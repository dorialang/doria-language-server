use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use serde_json::{json, Value};

use doriac::diagnostics::{
    prepare_diagnostics, Diagnostic, DiagnosticFix, DiagnosticSeverity, DiagnosticSource,
    FixApplicability, LabelRole,
};
use doriac::lexer::{Token, TokenKind};
use doriac::source::Span;

mod analysis;
mod string_surface;

use analysis::{AnalysisSnapshot, SemanticCompletion};
use string_surface::{STRING_COMPANION_METHODS, STRING_PROPERTIES};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}

pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

const READ_LINE_SIGNATURE: &str = "read_line(string $prompt = \"\"): ?string";
const READ_LINE_DOCUMENTATION: &str = "`read_line(string $prompt = \"\"): ?string` writes the prompt exactly with no added newline, flushes stdout before reading one UTF-8 line, returns `null` only at EOF, and returns `\"\"` for a blank line.";

pub fn toolchain_version() -> &'static str {
    doriac::TOOLCHAIN_VERSION
}

pub fn run_cli<I, S>(arguments: I) -> ExitCode
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let arguments = arguments
        .into_iter()
        .map(|argument| argument.as_ref().to_string())
        .collect::<Vec<_>>();
    match arguments.as_slice() {
        [argument] if argument == "--version" || argument == "-V" => {
            println!(
                "doria-lsp {} (Doria {})",
                SERVER_VERSION,
                toolchain_version()
            );
            ExitCode::SUCCESS
        }
        [argument, format] if argument == "--version" && format == "--json" => {
            println!(
                "{}",
                json!({
                    "schema": 1,
                    "component": "doria-lsp",
                    "version": SERVER_VERSION,
                    "toolchainVersion": toolchain_version(),
                    "compilerCommit": doriac::BUILD_COMMIT,
                })
            );
            ExitCode::SUCCESS
        }
        [argument] if argument == "--help" || argument == "-h" => {
            println!(
                "doria-lsp [--version [--json]]\n\nWithout arguments, starts the Doria language server over stdio."
            );
            ExitCode::SUCCESS
        }
        [argument, ..] => {
            eprintln!("unknown argument: {argument}");
            ExitCode::from(2)
        }
        [] => match run_stdio() {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("{message}");
                ExitCode::FAILURE
            }
        },
    }
}

#[derive(Debug, Clone)]
struct Document {
    text: String,
    version: Option<i64>,
    analysis: AnalysisSnapshot,
}

impl Document {
    fn new(uri: &str, text: String, version: Option<i64>) -> Self {
        let analysis = AnalysisSnapshot::analyze(uri, &text);
        Self {
            text,
            version,
            analysis,
        }
    }
}

#[derive(Default)]
struct Server {
    documents: HashMap<String, Document>,
}

pub fn run_stdio() -> Result<(), String> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = io::BufReader::new(stdin.lock());
    let mut writer = io::BufWriter::new(stdout.lock());
    let mut server = Server::default();

    while let Some(body) = read_message(&mut reader)? {
        let message = serde_json::from_slice::<Value>(&body)
            .map_err(|error| format!("failed to parse LSP message: {error}"))?;
        if !server.handle_message(message, &mut writer)? {
            break;
        }
        writer
            .flush()
            .map_err(|error| format!("failed to flush LSP response: {error}"))?;
    }

    Ok(())
}

pub fn byte_offset_to_position(text: &str, offset: usize) -> LspPosition {
    let clamped = offset.min(text.len());
    let mut line = 0_u32;
    let mut character = 0_u32;

    for (byte_index, ch) in text.char_indices() {
        if byte_index >= clamped {
            break;
        }

        if ch == '\n' {
            line += 1;
            character = 0;
        } else {
            character += ch.len_utf16() as u32;
        }
    }

    LspPosition { line, character }
}

pub fn position_to_byte_offset(text: &str, line: u32, character: u32) -> usize {
    let mut current_line = 0_u32;
    let mut current_character = 0_u32;

    for (byte_index, ch) in text.char_indices() {
        if current_line == line && current_character >= character {
            return byte_index;
        }

        if ch == '\n' {
            if current_line == line {
                return byte_index;
            }
            current_line += 1;
            current_character = 0;
            continue;
        }

        if current_line == line {
            let next_character = current_character + ch.len_utf16() as u32;
            if next_character > character {
                return byte_index;
            }
            current_character = next_character;
        }
    }

    text.len()
}

pub fn diagnostics_for_document(uri: &str, text: &str) -> Vec<Value> {
    let snapshot = AnalysisSnapshot::analyze(uri, text);
    diagnostics_to_lsp(uri, text, snapshot.diagnostics())
}

fn diagnostics_to_lsp(uri: &str, text: &str, diagnostics: &[Diagnostic]) -> Vec<Value> {
    prepare_diagnostics(diagnostics)
        .iter()
        .map(|diagnostic| diagnostic_to_lsp(uri, text, diagnostic))
        .collect()
}

pub fn code_actions_for_document(uri: &str, text: &str) -> Vec<Value> {
    prepare_diagnostics(AnalysisSnapshot::analyze(uri, text).diagnostics())
        .iter()
        .flat_map(|diagnostic| {
            diagnostic
                .fixes
                .iter()
                .filter_map(move |fix| code_action_for_fix(uri, text, diagnostic, fix))
        })
        .collect()
}

fn code_action_for_fix(
    uri: &str,
    text: &str,
    diagnostic: &Diagnostic,
    fix: &DiagnosticFix,
) -> Option<Value> {
    if fix.applicability != FixApplicability::MachineApplicable
        || fix
            .edits
            .iter()
            .any(|edit| !matches!(&edit.source, DiagnosticSource::Current))
    {
        return None;
    }
    let mut changes = serde_json::Map::new();
    for edit in &fix.edits {
        let lsp_edit = json!({
            "range": span_to_range(text, edit.span),
            "newText": edit.replacement,
        });
        changes
            .entry(uri.to_string())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .expect("code-action change is always an array")
            .push(lsp_edit);
    }

    Some(json!({
        "title": fix.title,
        "kind": "quickfix",
        "diagnostics": [diagnostic_to_lsp(uri, text, diagnostic)],
        "isPreferred": true,
        "edit": {
            "changes": changes,
        },
    }))
}

impl Server {
    fn handle_message<W: Write>(&mut self, message: Value, writer: &mut W) -> Result<bool, String> {
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return Ok(true);
        };

        let id = message.get("id").cloned();
        match method {
            "initialize" => {
                if let Some(id) = id {
                    send_response(writer, id, initialize_result())?;
                }
            }
            "initialized" => {}
            "shutdown" => {
                if let Some(id) = id {
                    send_response(writer, id, Value::Null)?;
                }
            }
            "exit" => return Ok(false),
            "textDocument/didOpen" => self.did_open(message.get("params"), writer)?,
            "textDocument/didChange" => self.did_change(message.get("params"), writer)?,
            "textDocument/didSave" => self.did_save(message.get("params"), writer)?,
            "textDocument/didClose" => self.did_close(message.get("params"), writer)?,
            "textDocument/completion" => {
                if let Some(id) = id {
                    send_response(writer, id, self.completion(message.get("params")))?;
                }
            }
            "textDocument/hover" => {
                if let Some(id) = id {
                    let hover = self.hover(message.get("params"));
                    send_response(writer, id, hover.unwrap_or(Value::Null))?;
                }
            }
            "textDocument/codeAction" => {
                if let Some(id) = id {
                    let actions = self.code_actions(message.get("params"));
                    send_response(writer, id, actions)?;
                }
            }
            "textDocument/references" => {
                if let Some(id) = id {
                    send_response(writer, id, self.references(message.get("params")))?;
                }
            }
            "textDocument/rename" => {
                if let Some(id) = id {
                    send_response(writer, id, self.rename(message.get("params")))?;
                }
            }
            _ => {
                if let Some(id) = id {
                    send_error(
                        writer,
                        id,
                        -32601,
                        format!("method `{method}` is not supported"),
                    )?;
                }
            }
        }

        Ok(true)
    }

    fn did_open<W: Write>(&mut self, params: Option<&Value>, writer: &mut W) -> Result<(), String> {
        let Some(text_document) = params.and_then(|params| params.get("textDocument")) else {
            return Ok(());
        };
        let Some(uri) = text_document.get("uri").and_then(Value::as_str) else {
            return Ok(());
        };
        let Some(text) = text_document.get("text").and_then(Value::as_str) else {
            return Ok(());
        };
        let version = text_document.get("version").and_then(Value::as_i64);

        self.documents.insert(
            uri.to_string(),
            Document::new(uri, text.to_string(), version),
        );
        self.publish_diagnostics(uri, writer)
    }

    fn did_change<W: Write>(
        &mut self,
        params: Option<&Value>,
        writer: &mut W,
    ) -> Result<(), String> {
        let Some(params) = params else {
            return Ok(());
        };
        let Some(text_document) = params.get("textDocument") else {
            return Ok(());
        };
        let Some(uri) = text_document.get("uri").and_then(Value::as_str) else {
            return Ok(());
        };
        let version = text_document.get("version").and_then(Value::as_i64);
        let Some(changes) = params.get("contentChanges").and_then(Value::as_array) else {
            return Ok(());
        };
        let Some(text) = changes
            .last()
            .and_then(|change| change.get("text"))
            .and_then(Value::as_str)
        else {
            return Ok(());
        };

        self.documents.insert(
            uri.to_string(),
            Document::new(uri, text.to_string(), version),
        );
        self.publish_diagnostics(uri, writer)
    }

    fn did_save<W: Write>(&mut self, params: Option<&Value>, writer: &mut W) -> Result<(), String> {
        let Some(text_document) = params.and_then(|params| params.get("textDocument")) else {
            return Ok(());
        };
        let Some(uri) = text_document.get("uri").and_then(Value::as_str) else {
            return Ok(());
        };

        if let Some(text) = params
            .and_then(|params| params.get("text"))
            .and_then(Value::as_str)
        {
            let version = self
                .documents
                .get(uri)
                .and_then(|document| document.version);
            self.documents.insert(
                uri.to_string(),
                Document::new(uri, text.to_string(), version),
            );
        }

        self.publish_diagnostics(uri, writer)
    }

    fn did_close<W: Write>(
        &mut self,
        params: Option<&Value>,
        writer: &mut W,
    ) -> Result<(), String> {
        let Some(text_document) = params.and_then(|params| params.get("textDocument")) else {
            return Ok(());
        };
        let Some(uri) = text_document.get("uri").and_then(Value::as_str) else {
            return Ok(());
        };

        self.documents.remove(uri);
        send_notification(
            writer,
            "textDocument/publishDiagnostics",
            json!({
                "uri": uri,
                "diagnostics": [],
            }),
        )
    }

    fn publish_diagnostics<W: Write>(&self, uri: &str, writer: &mut W) -> Result<(), String> {
        let Some(document) = self.documents.get(uri) else {
            return Ok(());
        };

        let diagnostics = diagnostics_to_lsp(uri, &document.text, document.analysis.diagnostics());
        let mut params = json!({
            "uri": uri,
            "diagnostics": diagnostics,
        });

        if let Some(version) = document.version {
            params["version"] = json!(version);
        }

        send_notification(writer, "textDocument/publishDiagnostics", params)
    }

    fn hover(&self, params: Option<&Value>) -> Option<Value> {
        let params = params?;
        let uri = params
            .get("textDocument")
            .and_then(|text_document| text_document.get("uri"))
            .and_then(Value::as_str)?;
        let line = params
            .get("position")
            .and_then(|position| position.get("line"))
            .and_then(Value::as_u64)? as u32;
        let character = params
            .get("position")
            .and_then(|position| position.get("character"))
            .and_then(Value::as_u64)? as u32;
        let document = self.documents.get(uri)?;
        let offset = position_to_byte_offset(&document.text, line, character);
        hover_at_offset_with_analysis(&document.text, offset, &document.analysis)
    }

    fn completion(&self, params: Option<&Value>) -> Value {
        let Some((uri, document, offset)) = self.uri_document_and_offset(params) else {
            return completion_items();
        };
        if let Some(completions) = document.analysis.member_completions_at_offset(offset) {
            return semantic_completion_items(completions);
        }
        if let Some(completions) = document.analysis.static_completions_at_offset(offset) {
            return semantic_completion_items(completions);
        }

        // An accessor with no member name is incomplete source, so preserve the
        // compiler as the semantic authority by analyzing a temporary property
        // token at the cursor instead of guessing from nearby text.
        if document.text[..offset].ends_with("->") {
            const PLACEHOLDER: &str = "__doria_completion";
            let mut source = document.text.clone();
            source.insert_str(offset, PLACEHOLDER);
            let analysis = AnalysisSnapshot::analyze(&uri, &source);
            if let Some(completions) = analysis.member_completions_at_offset(offset) {
                return semantic_completion_items(completions);
            }
        }
        if document.text[..offset].ends_with("::") {
            const PLACEHOLDER: &str = "__doria_completion";
            let mut source = document.text.clone();
            source.insert_str(offset, PLACEHOLDER);
            let analysis = AnalysisSnapshot::analyze(&uri, &source);
            if let Some(completions) = analysis.static_completions_at_offset(offset) {
                return semantic_completion_items(completions);
            }
        }
        completion_items_with_analysis(&document.analysis, offset)
    }

    fn references(&self, params: Option<&Value>) -> Value {
        let Some((uri, document, offset)) = self.uri_document_and_offset(params) else {
            return json!([]);
        };
        let include_declaration = params
            .and_then(|params| params.get("context"))
            .and_then(|context| context.get("includeDeclaration"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        Value::Array(
            document
                .analysis
                .reference_spans_at_offset(offset, include_declaration)
                .into_iter()
                .map(|span| json!({ "uri": &uri, "range": span_to_range(&document.text, span) }))
                .collect(),
        )
    }

    fn rename(&self, params: Option<&Value>) -> Value {
        let Some((uri, document, offset)) = self.uri_document_and_offset(params) else {
            return Value::Null;
        };
        let Some(new_name) = params
            .and_then(|params| params.get("newName"))
            .and_then(Value::as_str)
        else {
            return Value::Null;
        };
        let Some(replacement) = document
            .analysis
            .rename_replacement_at_offset(offset, new_name)
        else {
            return Value::Null;
        };
        let edits = document
            .analysis
            .reference_spans_at_offset(offset, true)
            .into_iter()
            .map(|span| json!({ "range": span_to_range(&document.text, span), "newText": replacement }))
            .collect::<Vec<_>>();
        if edits.is_empty() {
            Value::Null
        } else {
            let mut changes = serde_json::Map::new();
            changes.insert(uri, Value::Array(edits));
            json!({ "changes": changes })
        }
    }

    fn uri_document_and_offset<'a>(
        &'a self,
        params: Option<&Value>,
    ) -> Option<(String, &'a Document, usize)> {
        let params = params?;
        let uri = params
            .get("textDocument")
            .and_then(|text_document| text_document.get("uri"))
            .and_then(Value::as_str)?;
        let line = params
            .get("position")
            .and_then(|position| position.get("line"))
            .and_then(Value::as_u64)? as u32;
        let character = params
            .get("position")
            .and_then(|position| position.get("character"))
            .and_then(Value::as_u64)? as u32;
        let document = self.documents.get(uri)?;
        let offset = position_to_byte_offset(&document.text, line, character);
        Some((uri.to_string(), document, offset))
    }

    fn code_actions(&self, params: Option<&Value>) -> Value {
        let Some(uri) = params
            .and_then(|params| params.get("textDocument"))
            .and_then(|text_document| text_document.get("uri"))
            .and_then(Value::as_str)
        else {
            return json!([]);
        };
        let Some(document) = self.documents.get(uri) else {
            return json!([]);
        };

        Value::Array(code_actions_for_document(uri, &document.text))
    }
}

fn initialize_result() -> Value {
    json!({
        "capabilities": {
            "textDocumentSync": {
                "openClose": true,
                "change": 1,
                "save": {
                    "includeText": false
                }
            },
            "completionProvider": {
                "triggerCharacters": ["$", ">", ":"]
            },
            "hoverProvider": true,
            "referencesProvider": true,
            "renameProvider": true,
            "codeActionProvider": true
        },
        "serverInfo": {
            "name": "doria-lsp",
            "version": doriac::TOOLCHAIN_VERSION
        }
    })
}

fn completion_items() -> Value {
    let keywords = [
        "class",
        "interface",
        "trait",
        "extends",
        "implements",
        "function",
        "let",
        "take",
        "writable",
        "readonly",
        "internal",
        "return",
        "echo",
        "new",
        "namespace",
        "use",
        "uses",
        "as",
        "include",
        "declare",
        "foreach",
        "if",
        "else",
        "while",
        "for",
        "break",
        "continue",
        "static",
        "self",
        "parent",
        "const",
        "not",
        "and",
        "or",
        "xor",
        "true",
        "false",
        "null",
        "throw",
        "throws",
        "try",
        "catch",
        "finally",
        "when",
        "given",
        "is",
        "default",
        "do",
        "fn",
        "get",
        "set",
        "insteadof",
        "shared",
        "spawn",
        "scope",
        "enum",
        "case",
        "match",
        "async",
        "await",
        "unsafe",
        "extern",
        "open",
        "override",
        "with",
        "take",
    ];
    let planned_keywords = [
        "interface",
        "async",
        "await",
        "unsafe",
        "extern",
        "open",
        "override",
        "with",
        "take",
        "throw",
        "throws",
        "try",
        "catch",
        "finally",
        "when",
        "given",
        "do",
        "fn",
        "get",
        "set",
        "insteadof",
        "spawn",
        "scope",
    ];
    let types = [
        "void",
        "int",
        "int8",
        "int16",
        "int32",
        "int64",
        "uint8",
        "uint16",
        "uint32",
        "uint64",
        "float",
        "float32",
        "float64",
        "string",
        "bool",
        "mixed",
        "List",
        "Dictionary",
        "Set",
        "SortedDictionary",
        "SortedSet",
        "PriorityQueue",
        "Deque",
        "Bytes",
        "SharedReference",
        "WeakReference",
        "WritableSharedReference",
        "WritableWeakReference",
        "ReadonlySharedReferenceAccess",
        "WritableSharedReferenceAccess",
    ];
    let reserved_types = ["resource"];
    let integer_conversions = [
        ("Int::from", "int (the int64 alias)"),
        ("Int8::from", "int8"),
        ("Int16::from", "int16"),
        ("Int32::from", "int32"),
        ("Int64::from", "int64"),
        ("UInt8::from", "uint8"),
        ("UInt16::from", "uint16"),
        ("UInt32::from", "uint32"),
        ("UInt64::from", "uint64"),
    ];

    let mut items = Vec::new();
    items.extend(keywords.into_iter().map(|keyword| {
        let planned = planned_keywords.contains(&keyword);
        let mut item = json!({
            "label": keyword,
            "kind": 14,
            "detail": if planned { "planned Doria keyword" } else { "Doria keyword" },
        });
        if planned {
            item["documentation"] =
                json!("Accepted planned Doria syntax; compiler support lands in a later stage.");
        }
        item
    }));
    items.extend(types.into_iter().map(|ty| {
        let mut item = json!({
            "label": ty,
            "kind": 25,
            "detail": "Doria type",
        });
        if let Some(documentation) = integer_type_description(ty) {
            item["detail"] = json!("implemented Doria integer type");
            item["documentation"] = json!(documentation);
        }
        if let Some(documentation) = scalar_runtime_type_description(ty) {
            item["detail"] = json!("implemented Doria scalar type");
            item["documentation"] = json!(documentation);
        }
        if let Some(documentation) = shared_ownership_type_description(ty) {
            item["detail"] = json!("compiler-known Doria shared-ownership type");
            item["documentation"] = json!(documentation);
        }
        item
    }));
    items.extend(reserved_types.into_iter().map(|ty| {
        json!({
            "label": ty,
            "kind": 25,
            "detail": "Reserved Doria type name",
        })
    }));
    items.push(json!({
        "label": "Displayable",
        "kind": 8,
        "detail": "compiler-known Doria interface",
        "documentation": "`interface Displayable` requires an explicit `implements Displayable` declaration and exactly `function toString(): string`. It controls interpolation, echo, concatenation, and `%s`. Other interfaces are not supported by this compiler.",
    }));
    items.push(json!({
        "label": "toString",
        "kind": 2,
        "detail": "function toString(): string",
        "documentation": "The exact readonly instance method required by the compiler-known `Displayable` contract.",
    }));
    items.push(json!({
        "label": "panic",
        "kind": 3,
        "detail": "Doria built-in function",
        "documentation": "Terminates execution with a fatal panic, Doria stack trace, and status 101.",
    }));
    items.extend(
        [
            ("read_line", READ_LINE_SIGNATURE, READ_LINE_DOCUMENTATION),
            (
                "sprintf",
                "sprintf(string $format, ...): string",
                "Formats values with a compile-time-checked literal format string.",
            ),
            (
                "printf",
                "printf(string $format, ...): void",
                "Writes a compile-time-checked format with no added newline and returns void.",
            ),
            (
                "read_file",
                "read_file(string $path): string",
                "Reads a complete UTF-8 text file or panics on failure.",
            ),
            (
                "write_file",
                "write_file(string $path, string $contents): void",
                "Creates or truncates a UTF-8 text file and writes exact bytes.",
            ),
            (
                "write_stderr",
                "write_stderr(string $value): void",
                "Writes exact UTF-8 bytes to stderr without adding a newline.",
            ),
        ]
        .into_iter()
        .map(|(label, detail, documentation)| {
            json!({
                "label": label,
                "kind": 3,
                "detail": detail,
                "documentation": documentation,
            })
        }),
    );
    items.extend(integer_conversions.into_iter().map(|(label, target)| {
        json!({
            "label": label,
            "kind": 3,
            "detail": "Doria integer conversion intrinsic",
            "documentation": format!(
                "Compiler-known explicit conversion to `{target}`. Accepts exactly one integer expression and panics when the value is out of range."
            ),
        })
    }));
    items.extend(STRING_PROPERTIES.iter().map(|property| {
        json!({
            "label": property.name,
            "kind": 10,
            "detail": property.signature,
            "documentation": property.documentation,
        })
    }));
    items.extend(STRING_COMPANION_METHODS.iter().map(|method| {
        json!({
            "label": format!("String::{}", method.name),
            "kind": 3,
            "detail": method.signature,
            "documentation": method.documentation,
        })
    }));
    items.extend(SHARED_OWNERSHIP_METHODS.iter().map(|method| {
        json!({
            "label": method.name,
            "kind": 2,
            "detail": method.signature,
            "documentation": method.documentation,
        })
    }));
    items.extend([
        json!({
            "label": "Int::toFloat",
            "kind": 3,
            "detail": "Doria scalar conversion intrinsic",
            "documentation": "Converts canonical `int`/`int64` to canonical `float`/`float64` with IEEE 754 round-to-nearest, ties-to-even. This conversion does not panic.",
        }),
        json!({
            "label": "Float::toInt",
            "kind": 3,
            "detail": "Doria scalar conversion intrinsic",
            "documentation": "Truncates canonical `float`/`float64` toward zero to canonical `int`/`int64`; NaN, infinity, and out-of-range values panic.",
        }),
    ]);

    json!({
        "isIncomplete": false,
        "items": items,
    })
}

fn semantic_completion_items(completions: Vec<SemanticCompletion>) -> Value {
    let items = completions
        .into_iter()
        .map(|completion| {
            json!({
                "label": completion.label,
                "kind": completion.kind,
                "detail": completion.detail,
                "documentation": completion.documentation,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "isIncomplete": false,
        "items": items,
    })
}

fn completion_items_with_analysis(analysis: &AnalysisSnapshot, offset: usize) -> Value {
    let mut completions = completion_items();
    let Some(items) = completions.get_mut("items").and_then(Value::as_array_mut) else {
        return completions;
    };
    items.extend(
        analysis
            .local_completions_at_offset(offset)
            .into_iter()
            .map(|completion| {
                json!({
                    "label": completion.label,
                    "kind": 6,
                    "detail": completion.detail,
                })
            }),
    );
    completions
}

fn scalar_runtime_type_description(name: &str) -> Option<&'static str> {
    match name {
        "float" => Some("Implemented canonical IEEE 754 binary64 scalar type; exact alias of `float64`."),
        "float64" => Some("Implemented IEEE 754 binary64 scalar type; exact alias of `float`."),
        "float32" => Some("Implemented distinct IEEE 754 binary32 scalar type."),
        "bool" => Some("Implemented Copy scalar type with runtime locals, parameters, returns, calls, and short-circuit operators."),
        _ => None,
    }
}

fn shared_ownership_type_description(name: &str) -> Option<&'static str> {
    match name {
        "SharedReference" => Some("`SharedReference<T>` is a non-thread-safe owning move value for a readonly shared class payload. Construct it with `shared new T(...)`; ownership is duplicated only by explicit `share()`. It never converts to the writable family, and readonly shared collection, scalar, string, and `mixed` payload execution remain unsupported."),
        "WeakReference" => Some("`WeakReference<T>` is a non-thread-safe non-owning move value created from `SharedReference<T>`. `acquire()` returns `?SharedReference<T>` while the class payload is alive and `null` after final strong release; it never crosses ownership families."),
        "WritableSharedReference" => Some("`WritableSharedReference<T>` is a non-thread-safe owning move value in the writable shared family. Ownership is duplicated only by explicit `share()`; payload access requires a lifetime-owning object obtained with `acquireReadonlyAccess()` or `acquireWritableAccess()`. It never converts to `SharedReference<T>`. Class, generic class, typed-array, List, Dictionary, Set, and Bytes payloads execute; scalar, string, and `mixed` composition remain deferred."),
        "WritableWeakReference" => Some("`WritableWeakReference<T>` is the non-thread-safe non-owning move value for the writable family. `acquire()` returns `?WritableSharedReference<T>` while the payload is alive and never crosses into the readonly family."),
        "ReadonlySharedReferenceAccess" => Some("`ReadonlySharedReferenceAccess<T>` is a non-thread-safe owned move value that keeps a writable-family payload alive for its full lifetime and forwards readonly properties, methods, indexing, and iteration. It cannot be shared, weakened, copied, or converted between families."),
        "WritableSharedReferenceAccess" => Some("`WritableSharedReferenceAccess<T>` is a non-thread-safe owned move value that keeps exclusive writable payload access for its full lifetime. Its binding must be `writable` to mutate through it; it cannot be shared, weakened, copied, or converted between families."),
        _ => None,
    }
}

fn integer_type_description(name: &str) -> Option<&'static str> {
    match name {
        "int" => {
            Some("Implemented signed 64-bit integer type. `int` is an exact alias for `int64`.")
        }
        "int8" => Some("Implemented signed 8-bit integer type."),
        "int16" => Some("Implemented signed 16-bit integer type."),
        "int32" => Some("Implemented signed 32-bit integer type."),
        "int64" => {
            Some("Implemented signed 64-bit integer type; the same canonical type as `int`.")
        }
        "uint8" => Some("Implemented unsigned 8-bit integer type."),
        "uint16" => Some("Implemented unsigned 16-bit integer type."),
        "uint32" => Some("Implemented unsigned 32-bit integer type."),
        "uint64" => Some("Implemented unsigned 64-bit integer type."),
        _ => None,
    }
}

#[cfg(test)]
fn hover_at_offset(text: &str, offset: usize) -> Option<Value> {
    let analysis = AnalysisSnapshot::analyze("<lsp>", text);
    hover_at_offset_with_analysis(text, offset, &analysis)
}

fn hover_at_offset_with_analysis(
    text: &str,
    offset: usize,
    analysis: &AnalysisSnapshot,
) -> Option<Value> {
    if let Some(hover) = analysis.hover_at_offset(offset) {
        return Some(json!({
            "contents": {
                "kind": "markdown",
                "value": hover.markdown,
            },
            "range": span_to_range(text, hover.span),
        }));
    }

    let tokens = doriac::lex_source("<lsp>", text.to_string()).ok()?;
    let token_index = tokens.iter().position(|token| {
        !matches!(token.kind, TokenKind::Eof)
            && token.span.start <= offset
            && offset < token.span.end
    })?;
    let token = &tokens[token_index];
    let description = string_companion_hover_at(&tokens, token_index)
        .or_else(|| integer_conversion_hover_at(&tokens, token_index).map(ToOwned::to_owned))
        .or_else(|| builtin_method_hover_at(&tokens, token_index))
        .or_else(|| hover_description(&token.kind).map(ToOwned::to_owned))?;

    Some(json!({
        "contents": {
            "kind": "markdown",
            "value": description,
        },
        "range": span_to_range(text, token.span),
    }))
}

fn string_companion_hover_at(tokens: &[Token], token_index: usize) -> Option<String> {
    if token_index < 2 || !matches!(tokens[token_index - 1].kind, TokenKind::DoubleColon) {
        return None;
    }
    let TokenKind::Identifier(companion) = &tokens[token_index - 2].kind else {
        return None;
    };
    let TokenKind::Identifier(method) = &tokens[token_index].kind else {
        return None;
    };
    if companion != "String" {
        return None;
    }
    let member = string_surface::string_companion_method(method)?;
    Some(format!(
        "```doria\n{}\n```\n\n{}",
        member.signature, member.documentation
    ))
}

struct BuiltinMethod {
    name: &'static str,
    signature: &'static str,
    documentation: &'static str,
}

const SHARED_OWNERSHIP_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod {
        name: "share",
        signature: "function share(): SharedReference<T> | WritableSharedReference<T>",
        documentation:
            "Creates one additional owner in the receiver's existing shared-ownership family.",
    },
    BuiltinMethod {
        name: "createWeakReference",
        signature:
            "function createWeakReference(): WeakReference<T> | WritableWeakReference<T>",
        documentation:
            "Creates a non-owning reference in the receiver's existing shared-ownership family.",
    },
    BuiltinMethod {
        name: "acquire",
        signature: "function acquire(): ?SharedReference<T> | ?WritableSharedReference<T>",
        documentation: "Attempts to create a strong owner from a weak reference without changing ownership families. Returns `null` after the payload has been destroyed.",
    },
    BuiltinMethod {
        name: "acquireReadonlyAccess",
        signature: "function acquireReadonlyAccess(): ReadonlySharedReferenceAccess<T>",
        documentation: "Acquires owned readonly access to a writable shared payload. Multiple readonly accesses may coexist.",
    },
    BuiltinMethod {
        name: "acquireWritableAccess",
        signature: "function acquireWritableAccess(): WritableSharedReferenceAccess<T>",
        documentation:
            "Acquires owned exclusive writable access to a writable shared payload.",
    },
];

fn builtin_method_hover_at(tokens: &[Token], token_index: usize) -> Option<String> {
    if token_index == 0
        || !matches!(
            tokens[token_index - 1].kind,
            TokenKind::Arrow | TokenKind::QuestionArrow
        )
    {
        return None;
    }
    let TokenKind::Identifier(name) = &tokens[token_index].kind else {
        return None;
    };
    let method = SHARED_OWNERSHIP_METHODS
        .iter()
        .find(|method| method.name == name)?;
    Some(format!(
        "```doria\n{}\n```\n\n{}",
        method.signature, method.documentation
    ))
}

fn integer_conversion_hover_at(tokens: &[Token], token_index: usize) -> Option<&'static str> {
    let TokenKind::Identifier(name) = &tokens[token_index].kind else {
        return None;
    };

    if let Some(description) = integer_conversion_description(name) {
        return Some(description);
    }

    if token_index < 2 {
        return None;
    }
    if !matches!(tokens[token_index - 1].kind, TokenKind::DoubleColon) {
        return None;
    }

    let TokenKind::Identifier(companion) = &tokens[token_index - 2].kind else {
        return None;
    };
    match (companion.as_str(), name.as_str()) {
        ("Int", "toFloat") => cross_kind_conversion_description("Int::toFloat"),
        ("Float", "toInt") => cross_kind_conversion_description("Float::toInt"),
        (_, "from") => integer_conversion_description(companion),
        _ => None,
    }
}

fn cross_kind_conversion_description(name: &str) -> Option<&'static str> {
    match name {
        "Int::toFloat" => Some("`Int::toFloat(value)` converts canonical `int`/`int64` to canonical `float`/`float64` using IEEE 754 round-to-nearest, ties-to-even, without panicking."),
        "Float::toInt" => Some("`Float::toInt(value)` truncates canonical `float`/`float64` toward zero to canonical `int`/`int64`; NaN, infinity, and out-of-range values panic."),
        _ => None,
    }
}

fn integer_conversion_description(companion: &str) -> Option<&'static str> {
    match companion {
        "Int" => Some("`Int::from(value)` explicitly converts one integer expression to `int`, the exact `int64` alias. Out-of-range conversion panics."),
        "Int8" => Some("`Int8::from(value)` explicitly converts one integer expression to `int8`. Out-of-range conversion panics."),
        "Int16" => Some("`Int16::from(value)` explicitly converts one integer expression to `int16`. Out-of-range conversion panics."),
        "Int32" => Some("`Int32::from(value)` explicitly converts one integer expression to `int32`. Out-of-range conversion panics."),
        "Int64" => Some("`Int64::from(value)` explicitly converts one integer expression to `int64`, the same canonical type as `int`. Out-of-range conversion panics."),
        "UInt8" => Some("`UInt8::from(value)` explicitly converts one integer expression to `uint8`. Out-of-range conversion panics."),
        "UInt16" => Some("`UInt16::from(value)` explicitly converts one integer expression to `uint16`. Out-of-range conversion panics."),
        "UInt32" => Some("`UInt32::from(value)` explicitly converts one integer expression to `uint32`. Out-of-range conversion panics."),
        "UInt64" => Some("`UInt64::from(value)` explicitly converts one integer expression to `uint64`. Out-of-range conversion panics."),
        _ => None,
    }
}

fn hover_description(kind: &TokenKind) -> Option<&'static str> {
    match kind {
        TokenKind::Class => Some("Declares a Doria class."),
        TokenKind::Interface => Some(
            "Declares an interface. This compiler currently provides only the compiler-known `Displayable` contract.",
        ),
        TokenKind::Implements => Some(
            "Declares nominal conformance. This compiler currently supports only the compiler-known `Displayable` contract.",
        ),
        TokenKind::Function => Some("Declares a function or method."),
        TokenKind::Let => Some("Declares a local binding with an inferred type."),
        TokenKind::Take => Some(
            "Gives ownership of a move-type argument to this parameter. Call sites remain unmarked.",
        ),
        TokenKind::Writable => {
            Some("Marks a binding, property, parameter, or method receiver as mutable.")
        }
        TokenKind::Internal => {
            Some("Marks a class member as hidden from the external object surface.")
        }
        TokenKind::Readonly => Some("Reserved for explicit readonly syntax."),
        TokenKind::Return => Some("Returns a value from the current function."),
        TokenKind::Echo => Some("Emits a value through the current backend."),
        TokenKind::New => Some("Constructs an instance of a class."),
        TokenKind::Foreach => Some("Iterates over a list or dictionary value."),
        TokenKind::As => Some("Separates a `foreach` iterable from its binding."),
        TokenKind::Static => Some("Declares a static method or property."),
        TokenKind::SelfType => Some(
            "Reserved declaring-class qualifier and type: `self::member` or a `self` return type.",
        ),
        TokenKind::Parent => Some(
            "Reserved parent-implementation qualifier. Its semantics land with inheritance in Stage 34.",
        ),
        TokenKind::Trait => Some(
            "Declares accepted trait syntax. Trait composition semantics land in Stage 35.",
        ),
        TokenKind::Const => Some("Declares a compile-time-evaluated constant."),
        TokenKind::Enum => Some("Declares a nominal Doria enum type."),
        TokenKind::Case => Some("Declares a case inside an enum."),
        TokenKind::Match => Some(
            "Begins an expression-position `match`. Its grammar is accepted; semantic execution lands in Stage 28.",
        ),
        TokenKind::Default => Some("Declares the fallback arm of a `match` expression."),
        TokenKind::Not => Some("Boolean NOT operator; exact synonym for `!`."),
        TokenKind::And => Some("Boolean AND operator; exact synonym for `&&`."),
        TokenKind::Or => Some("Boolean OR operator; exact synonym for `||`."),
        TokenKind::Xor => Some("Bool-only exclusive OR operator."),
        TokenKind::Is => Some(
            "Exact type-test operator. `value is Type` narrows `mixed` and nullable values on the true branch.",
        ),
        TokenKind::Void => Some("The `void` return type."),
        TokenKind::IntType => integer_type_description("int"),
        TokenKind::Int8Type => integer_type_description("int8"),
        TokenKind::Int16Type => integer_type_description("int16"),
        TokenKind::Int32Type => integer_type_description("int32"),
        TokenKind::Int64Type => integer_type_description("int64"),
        TokenKind::UInt8Type => integer_type_description("uint8"),
        TokenKind::UInt16Type => integer_type_description("uint16"),
        TokenKind::UInt32Type => integer_type_description("uint32"),
        TokenKind::UInt64Type => integer_type_description("uint64"),
        TokenKind::FloatType => scalar_runtime_type_description("float"),
        TokenKind::Float32Type => scalar_runtime_type_description("float32"),
        TokenKind::Float64Type => scalar_runtime_type_description("float64"),
        TokenKind::StringType => Some("The immutable UTF-8 `string` primitive type. Its nullable form `?string` is one instance of the general `?T` nullable model (`??`, `?->`, and `!= null` / `is` narrowing); `read_line` returning `?string` at EOF is just one producer.") ,
        TokenKind::BoolType => scalar_runtime_type_description("bool"),
        TokenKind::True | TokenKind::False => Some("Boolean literal."),
        TokenKind::Null => Some("The `null` literal: the absent value of any nullable type `?T`. Assign it to a `?T` binding or compare with `== null` / `!= null`; a `!= null` guard narrows `?T` to `T`."),
        TokenKind::Reserved(_) => Some("Reserved for future Doria syntax."),
        TokenKind::Identifier(name) => match name.as_str() {
            "Displayable" => Some("`interface Displayable` is the compiler-known display contract. A class must explicitly declare `implements Displayable` and provide `function toString(): string`. It controls interpolation, echo, concatenation, and `%s`. Other interfaces are not supported by this compiler."),
            "toString" => Some("`function toString(): string` is the exact externally accessible readonly instance method required by `Displayable`."),
            "List" => Some("`List<T>` is the growable, insertion-ordered sequence: `add`, `insertAt`, `removeAt`, `pop`, `contains`, `first`/`last`, and the `count`/`isEmpty` properties (decision 0100). An owned move type."),
            "Dictionary" => Some("`Dictionary<K, V>` is the insertion-ordered map: `get` (`?V`), `set`, `remove` (`?V`), `has`, the `keys`/`values` projections, and `count`/`isEmpty` (decision 0100). Keys require `Hashable`. An owned move type."),
            "Set" => Some("`Set<T>` is the insertion-ordered unique-element collection: `Set::from`, `add`, `remove`, `contains`, `union`/`intersect`/`difference`, and `count`/`isEmpty` (decision 0100). Elements require `Hashable`. An owned move type."),
            "SortedDictionary" => Some("`SortedDictionary<K, V>` is a key-ordered map with the `Dictionary` member surface. Keys and the `keys`/`values` projections use ascending `Comparable<K>` order. An owned move type."),
            "SortedSet" => Some("`SortedSet<T>` is an ascending-order unique-element collection with the `Set` member surface. Elements require `Comparable<T>`. An owned move type."),
            "PriorityQueue" => Some("`PriorityQueue<T>` is a min-priority queue: `push`, `pop`, `peek`, `count`, and `isEmpty`. It deliberately has no `foreach` order. Elements require `Comparable<T>`. An owned move type."),
            "Deque" => Some("`Deque<T>` is a double-ended queue: `pushFront`/`pushBack`, `popFront`/`popBack`, `peekFront`/`peekBack`, `count`, and `isEmpty`. Iteration runs front to back. An owned move type."),
            "String" => Some("`String` is the companion for canonical string operations. Text indices and lengths use Unicode extended grapheme clusters unless the API explicitly says bytes."),
            name @ ("SharedReference" | "WeakReference" | "WritableSharedReference"
            | "WritableWeakReference" | "ReadonlySharedReferenceAccess"
            | "WritableSharedReferenceAccess") => shared_ownership_type_description(name),
            "mixed" => Some("The dynamic boundary type: a boxed runtime value that accepts any type but rejects every operation until narrowed with the exact `is` type-test operator. `?mixed` adds nullability."),
            "resource" => Some("Reserved for future PHP interop; not a usable core type."),
            companion @ ("Int" | "Int8" | "Int16" | "Int32" | "Int64" | "UInt8"
            | "UInt16" | "UInt32" | "UInt64") => integer_conversion_description(companion),
            "panic" => Some(
                "Built-in fatal runtime function: `panic(\"message\");`. Panics are not catchable and exit with status 101.",
            ),
            "read_line" => Some(READ_LINE_DOCUMENTATION),
            "sprintf" => Some("`sprintf(string $format, ...): string` uses a compile-time-checked literal format string."),
            "printf" => Some("`printf(string $format, ...): void` uses the same checked formatter as `sprintf`, adds no newline, and returns void."),
            "read_file" => Some("`read_file(string $path): string` reads complete UTF-8 text and panics on failure."),
            "write_file" => Some("`write_file(string $path, string $contents): void` creates or truncates a UTF-8 text file and writes exact bytes."),
            "write_stderr" => Some("`write_stderr(string $value): void` writes exact bytes to stderr without adding a newline."),
            "Bytes" => Some("`Bytes` is the owned, mutable byte-buffer move type: `Bytes::fromArray` / `->toArray` (copying), the `length` property, byte indexing with in-place writes, and byte-wise equality. It converts to and from `uint8[]` only explicitly."),
            "append_file" => Some("`append_file(string $path, string $contents): void` creates or appends exact UTF-8 bytes; `write_file` stays truncate-only (decision 0091)."),
            "read_file_bytes" => Some("`read_file_bytes(string $path): Bytes` reads a whole file as raw bytes, with no UTF-8 validation or newline normalization."),
            "write_file_bytes" => Some("`write_file_bytes(string $path, Bytes $contents): void` creates or truncates a file with exact bytes."),
            "append_file_bytes" => Some("`append_file_bytes(string $path, Bytes $contents): void` creates or appends exact bytes."),
            "read_stdin_bytes" => Some("`read_stdin_bytes(): Bytes` reads all of stdin as raw bytes (empty on EOF), with no UTF-8 validation (decision 0101)."),
            "write_stdout_bytes" => Some("`write_stdout_bytes(Bytes $contents): void` writes exact bytes to stdout, with no newline or console translation (decision 0101)."),
            "write_stderr_bytes" => Some("`write_stderr_bytes(Bytes $contents): void` writes exact bytes to stderr, with no newline or console translation (decision 0101)."),
            _ => None,
        },
        TokenKind::Variable(_) => Some("Doria variable. Variables must be declared before use."),
        _ => None,
    }
}

fn diagnostic_to_lsp(uri: &str, text: &str, diagnostic: &Diagnostic) -> Value {
    let primary = diagnostic
        .labels
        .iter()
        .find(|label| label.role == LabelRole::Primary)
        .or_else(|| diagnostic.labels.first());
    let primary_span = primary.map_or(diagnostic.span, |label| label.span);
    let mut message = diagnostic.title.clone();
    if let Some(primary) = primary.filter(|label| !label.message.is_empty()) {
        message.push('\n');
        message.push_str(&primary.message);
    }
    if let Some(explanation) = &diagnostic.explanation {
        message.push_str("\n\nWhy: ");
        message.push_str(explanation);
    }
    for help in &diagnostic.helps {
        message.push_str("\nHelp: ");
        message.push_str(help);
    }

    let mut value = json!({
        "range": span_to_range(text, primary_span),
        "severity": lsp_severity(diagnostic.severity),
        "code": diagnostic.code,
        "source": "doriac",
        "message": message,
        "codeDescription": diagnostic.documentation.as_ref().and_then(|docs| {
            docs.url.as_ref().map(|url| json!({ "href": url }))
        }),
    });
    value["data"] = json!({
        "kind": diagnostic.kind.as_str(),
        "causeId": diagnostic.cause_id,
        "fixes": diagnostic.fixes.iter().map(|fix| json!({
            "title": fix.title,
            "applicability": fix.applicability.as_str(),
            "edits": fix.edits.iter().map(|edit| json!({
                "uri": diagnostic_source_uri(uri, &edit.source),
                "range": span_to_range(
                    if matches!(&edit.source, DiagnosticSource::Current) { text } else { "" },
                    edit.span,
                ),
                "newText": edit.replacement,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    if let Some(fix) = diagnostic
        .fixes
        .iter()
        .find_map(|fix| match fix.edits.as_slice() {
            [edit]
                if fix.applicability == FixApplicability::MachineApplicable
                    && matches!(&edit.source, DiagnosticSource::Current) =>
            {
                Some(edit)
            }
            _ => None,
        })
    {
        value["data"]["fix"] = json!({
            "range": span_to_range(text, fix.span),
            "newText": fix.replacement,
        });
    }
    let related = diagnostic
        .labels
        .iter()
        .filter(|label| label.role == LabelRole::Secondary)
        .map(|label| {
            json!({
                "location": {
                    "uri": diagnostic_source_uri(uri, &label.source),
                    "range": span_to_range(
                        if matches!(&label.source, DiagnosticSource::Current) { text } else { "" },
                        label.span,
                    ),
                },
                "message": label.message,
            })
        })
        .collect::<Vec<_>>();
    if !related.is_empty() {
        value["relatedInformation"] = Value::Array(related);
    }
    value
}

fn lsp_severity(severity: DiagnosticSeverity) -> u8 {
    match severity {
        DiagnosticSeverity::Error => 1,
        DiagnosticSeverity::Warning => 2,
        DiagnosticSeverity::Note => 3,
    }
}

fn diagnostic_source_uri(current_uri: &str, source: &DiagnosticSource) -> String {
    match source {
        DiagnosticSource::Current => current_uri.to_string(),
        DiagnosticSource::Path(path) if path.contains("://") => path.clone(),
        DiagnosticSource::Path(path) => format!("file://{path}"),
    }
}

fn span_to_range(text: &str, span: Span) -> Value {
    let start = byte_offset_to_position(text, span.start);
    let end = byte_offset_to_position(text, span.end);
    json!({
        "start": {
            "line": start.line,
            "character": start.character,
        },
        "end": {
            "line": end.line,
            "character": end.character,
        },
    })
}

fn read_message<R: BufRead>(reader: &mut R) -> Result<Option<Vec<u8>>, String> {
    let mut content_length = None::<usize>;

    loop {
        let mut line = String::new();
        let bytes_read = reader
            .read_line(&mut line)
            .map_err(|error| format!("failed to read LSP header: {error}"))?;

        if bytes_read == 0 {
            return if content_length.is_some() {
                Err("unexpected EOF while reading LSP headers".to_string())
            } else {
                Ok(None)
            };
        }

        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }

        if trimmed.to_ascii_lowercase().starts_with("content-length:") {
            let (_, value) = trimmed
                .split_once(':')
                .ok_or_else(|| "malformed Content-Length header".to_string())?;
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|error| format!("invalid Content-Length header: {error}"))?,
            );
        }
    }

    let length = content_length.ok_or_else(|| "missing Content-Length header".to_string())?;
    let mut body = vec![0_u8; length];
    reader
        .read_exact(&mut body)
        .map_err(|error| format!("failed to read LSP body: {error}"))?;
    Ok(Some(body))
}

fn send_response<W: Write>(writer: &mut W, id: Value, result: Value) -> Result<(), String> {
    send_message(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }),
    )
}

fn send_error<W: Write>(
    writer: &mut W,
    id: Value,
    code: i64,
    message: String,
) -> Result<(), String> {
    send_message(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": code,
                "message": message,
            },
        }),
    )
}

fn send_notification<W: Write>(writer: &mut W, method: &str, params: Value) -> Result<(), String> {
    send_message(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }),
    )
}

fn send_message<W: Write>(writer: &mut W, message: &Value) -> Result<(), String> {
    let body = serde_json::to_vec(message)
        .map_err(|error| format!("failed to encode LSP message: {error}"))?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())
        .map_err(|error| format!("failed to write LSP header: {error}"))?;
    writer
        .write_all(&body)
        .map_err(|error| format!("failed to write LSP body: {error}"))
}
#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use doriac::diagnostics::{FixEdit, LabelRole};

    #[test]
    fn initialize_reports_canonical_toolchain_calver() {
        assert_eq!(
            initialize_result()["serverInfo"]["version"],
            doriac::TOOLCHAIN_VERSION
        );
        assert_eq!(doriac::TOOLCHAIN_VERSION, "2026.03.1-canary");
    }

    fn completion_item(label: &str) -> Value {
        completion_items()["items"]
            .as_array()
            .expect("completion items should be an array")
            .iter()
            .find(|item| item["label"] == label)
            .unwrap_or_else(|| panic!("completion item `{label}` should exist"))
            .clone()
    }
    fn completion_labels() -> Vec<String> {
        completion_items()["items"]
            .as_array()
            .expect("completion items should be an array")
            .iter()
            .map(|item| {
                item["label"]
                    .as_str()
                    .expect("completion item labels should be strings")
                    .to_string()
            })
            .collect()
    }

    fn completion_detail(label: &str) -> Option<String> {
        completion_items()["items"]
            .as_array()
            .expect("completion items should be an array")
            .iter()
            .find(|item| item["label"].as_str() == Some(label))
            .and_then(|item| item["detail"].as_str())
            .map(ToOwned::to_owned)
    }

    #[test]
    fn completions_mark_accepted_planned_keywords() {
        for keyword in [
            "async",
            "await",
            "unsafe",
            "extern",
            "open",
            "override",
            "with",
            "take",
            "throw",
            "throws",
            "try",
            "catch",
            "finally",
            "when",
            "given",
            "do",
            "fn",
            "get",
            "set",
            "insteadof",
            "spawn",
            "scope",
        ] {
            let item = completion_item(keyword);
            assert_eq!(item["detail"], "planned Doria keyword");
            assert_eq!(
                item["documentation"],
                "Accepted planned Doria syntax; compiler support lands in a later stage."
            );
        }
    }

    #[test]
    fn enum_and_match_keywords_are_active_compiler_syntax() {
        for (keyword, kind) in [
            ("enum", TokenKind::Enum),
            ("case", TokenKind::Case),
            ("match", TokenKind::Match),
            ("default", TokenKind::Default),
        ] {
            assert_eq!(completion_item(keyword)["detail"], "Doria keyword");
            assert!(hover_description(&kind).is_some());
        }
    }

    #[test]
    fn completions_and_hover_expose_stage20_declarations() {
        for (keyword, kind, hover) in [
            (
                "static",
                TokenKind::Static,
                "Declares a static method or property.",
            ),
            (
                "const",
                TokenKind::Const,
                "Declares a compile-time-evaluated constant.",
            ),
        ] {
            assert_eq!(completion_item(keyword)["detail"], "Doria keyword");
            assert_eq!(hover_description(&kind), Some(hover));
        }
    }

    #[test]
    fn completions_keep_rejected_syntax_out() {
        let labels = completion_labels();
        let rejected = [
            ["pub", "lic"].concat(),
            ["pri", "vate"].concat(),
            ["pro", "tected"].concat(),
            ["requ", "ire"].concat(),
            ["requ", "ire_once"].concat(),
            ["include", "_once"].concat(),
            ["=", "=="].concat(),
            ["!", "=="].concat(),
            ["#de", "fine"].concat(),
            ["#inc", "lude"].concat(),
            ["ar", "ray"].concat(),
            ["instance", "of"].concat(),
        ];
        for rejected in rejected {
            assert!(
                !labels.iter().any(|label| label == &rejected),
                "rejected syntax `{rejected}` must not be an active LSP completion"
            );
        }
    }

    #[test]
    fn invalid_empty_enum_reports_the_specific_compiler_diagnostic() {
        let diagnostics = diagnostics_for_document(
            "test.doria",
            r#"enum Option
{
}
"#,
        );

        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0]["code"], "E0562");
    }

    #[test]
    fn named_arguments_are_not_reported_as_editor_errors() {
        // Stage 23a (decision 0098) makes `name: value` a real call form for
        // free functions, instance methods, static methods, and constructors.
        // The editor must not mark any of them as invalid code.
        let diagnostics = diagnostics_for_document(
            "test.doria",
            r#"class Message
{
    function __construct(string $recipient, int $attempts = 1)
    {
    }

    function deliver(string $recipient, int $attempts = 1): void
    {
    }

    static function create(string $recipient, int $attempts = 1): void
    {
    }
}

function scheduleDelivery(string $recipient, int $attempts = 1): void
{
}

function main(): void
{
    let $name = "inbox";
    scheduleDelivery(recipient: $name, attempts: 3);
    scheduleDelivery($name, attempts: 3);
    scheduleDelivery(attempts: 3, recipient: $name);
    let $message = new Message(attempts: 5, recipient: $name);
    $message->deliver(attempts: 2, recipient: $name);
    Message::create(recipient: $name);
}
"#,
        );

        assert!(
            diagnostics.is_empty(),
            "named arguments must not surface as editor errors: {diagnostics:?}"
        );
    }

    #[test]
    fn a_positional_argument_after_a_named_argument_is_reported() {
        // The ordering rule is a real compile error, so the editor should show it.
        let diagnostics = diagnostics_for_document(
            "test.doria",
            r#"function scheduleDelivery(string $recipient, int $attempts = 1): void
{
}

function main(): void
{
    scheduleDelivery(recipient: "inbox", 3);
}
"#,
        );

        assert!(
            !diagnostics.is_empty(),
            "a positional argument after a named argument must be reported"
        );
    }

    #[test]
    fn the_entry_argument_list_is_not_reported_as_an_editor_error() {
        // Stage 23b (decision 0099) adds `main(List<string> $args)` alongside
        // the parameterless forms. None of them may be marked as invalid code.
        for source in [
            r#"function main(List<string> $args): int
{
    printf("count=%d\n", $args->count);
    return 0;
}
"#,
            r#"function main(List<string> $args): void
{
    foreach ($args as $argument) {
        echo $argument;
    }
}
"#,
            r#"function main(): int
{
    return 0;
}
"#,
            r#"function main(): void
{
}
"#,
        ] {
            let diagnostics = diagnostics_for_document("test.doria", source);
            assert!(
                diagnostics.is_empty(),
                "accepted entry form must not surface as an editor error: {diagnostics:?}"
            );
        }
    }

    #[test]
    fn sequence_fill_literals_are_not_reported_as_editor_errors() {
        let diagnostics = diagnostics_for_document(
            "test.doria",
            r#"function main(List<string> $args): void
{
    bool[] $flags = [true; $args->count];
    let $counts = [0; $args->count];
    echo "{$flags->length}:{$counts->count}";
}
"#,
        );

        assert!(
            diagnostics.is_empty(),
            "Stage 23c sequence fills must not surface as editor errors: {diagnostics:?}"
        );
    }

    #[test]
    fn a_separate_argument_count_parameter_is_reported() {
        // Decision 0099 rejects `argc`; the editor should show that.
        let diagnostics = diagnostics_for_document(
            "test.doria",
            r#"function main(string[] $argv, int $argc): int
{
    return 0;
}
"#,
        );

        assert!(
            !diagnostics.is_empty(),
            "a separate argument count must be reported"
        );
    }

    #[test]
    fn writable_and_consuming_entry_argument_lists_are_reported() {
        // The entry glue owns the list and lends a readonly view to `main`.
        for source in [
            "function main(writable List<string> $args): void {}",
            "function main(take List<string> $args): void {}",
        ] {
            let diagnostics = diagnostics_for_document("test.doria", source);
            assert!(
                !diagnostics.is_empty(),
                "an owning or writable entry parameter must be reported"
            );
        }
    }

    #[test]
    fn completion_and_hover_document_the_narrow_displayable_contract() {
        let completion = completion_item("Displayable");
        let documentation = completion["documentation"]
            .as_str()
            .expect("Displayable completion should have documentation");
        assert!(documentation.contains("interface Displayable"));
        assert!(documentation.contains("function toString(): string"));
        assert!(documentation.contains("interpolation, echo, concatenation, and `%s`"));
        assert!(documentation.contains("Other interfaces are not supported by this compiler"));

        let source = "class Label implements Displayable {}";
        let hover = hover_at_offset(
            source,
            source.find("Displayable").expect("Displayable token"),
        )
        .expect("Displayable should provide hover information");
        let text = hover["contents"]["value"]
            .as_str()
            .expect("hover contents should be markdown");
        assert!(text.contains("interface Displayable"));
        assert!(text.contains("function toString(): string"));
        assert!(text.contains("Other interfaces are not supported by this compiler"));
    }

    #[test]
    fn completions_and_hover_expose_the_executable_string_surface() {
        for method in STRING_COMPANION_METHODS {
            let item = completion_item(&format!("String::{}", method.name));
            assert_eq!(item["detail"], method.signature);
            assert_eq!(item["documentation"], method.documentation);
        }
        for property in STRING_PROPERTIES {
            let item = completion_item(property.name);
            assert_eq!(item["detail"], property.signature);
            assert_eq!(item["documentation"], property.documentation);
        }

        let labels = completion_labels();
        assert!(labels.contains(&"String::containsIgnoreCase".to_string()));
        assert!(labels.contains(&"String::countOccurrences".to_string()));
        assert!(!labels.contains(&"trim".to_string()));

        let source = r#"function main(): void
{
    echo String::indexOfIgnoreCase("Straße", "STRASSE") ?? -1;
}
"#;
        let hover = hover_at_offset(
            source,
            source
                .find("indexOfIgnoreCase")
                .expect("String companion call"),
        )
        .expect("String companion should provide hover information");
        let markdown = hover["contents"]["value"]
            .as_str()
            .expect("hover should be Markdown");
        assert!(markdown.contains("String::indexOfIgnoreCase(string $text, string $needle): ?int"));
        assert!(markdown.contains("original grapheme sequence"));
    }

    #[test]
    fn hover_help_tracks_stage22_narrowing() {
        let null_hover = hover_description(&TokenKind::Null).expect("null should have hover text");
        // The hover documents the general `?T` model rather than singling out
        // `read_line`, which is only one producer of a `?string`.
        assert!(null_hover.contains("`?T`"));
        assert!(null_hover.contains("!= null"));
        assert!(null_hover.contains("narrows"));
        assert!(!null_hover.contains("Stage "));

        let mixed_hover = hover_description(&TokenKind::Identifier("mixed".to_string()))
            .expect("mixed should have hover text");
        assert!(mixed_hover.contains("exact `is` type-test operator"));
        assert!(!mixed_hover.contains("`match`"));

        let is_hover = hover_description(&TokenKind::Is).expect("is should have hover text");
        assert!(is_hover.contains("Exact type-test operator"));
        assert!(is_hover.contains("narrows `mixed` and nullable values"));
    }

    #[test]
    fn completions_and_hovers_track_stage25a_shared_ownership() {
        for name in [
            "SharedReference",
            "WeakReference",
            "WritableSharedReference",
            "WritableWeakReference",
            "ReadonlySharedReferenceAccess",
            "WritableSharedReferenceAccess",
        ] {
            assert_eq!(
                completion_item(name)["detail"],
                "compiler-known Doria shared-ownership type"
            );
            assert!(
                hover_description(&TokenKind::Identifier(name.to_string())).is_some(),
                "{name} should provide hover information"
            );
        }
        let shared = hover_description(&TokenKind::Identifier("SharedReference".to_string()))
            .expect("SharedReference hover");
        assert!(shared.contains("`shared new T(...)`"));
        assert!(shared.contains("readonly"));

        let weak = hover_description(&TokenKind::Identifier("WeakReference".to_string()))
            .expect("WeakReference hover");
        assert!(weak.contains("`acquire()`"));
        assert!(weak.contains("`?SharedReference<T>`"));

        let writable = hover_description(&TokenKind::Identifier(
            "WritableSharedReference".to_string(),
        ))
        .expect("WritableSharedReference hover");
        assert!(writable.contains("`acquireReadonlyAccess()`"));
        assert!(writable.contains("`acquireWritableAccess()`"));
        assert!(!writable.contains("next Stage 25a"));

        for member in [
            "share",
            "createWeakReference",
            "acquire",
            "acquireReadonlyAccess",
            "acquireWritableAccess",
        ] {
            assert_eq!(completion_item(member)["kind"], 2);
            let source = format!("$value->{member}();");
            let hover = hover_at_offset(&source, source.find(member).unwrap())
                .unwrap_or_else(|| panic!("{member} should provide member-call hover information"));
            assert!(hover["contents"]["value"]
                .as_str()
                .is_some_and(|contents| contents.contains("function")));
        }

        let source = "$value?->acquire();";
        let acquire = hover_at_offset(source, source.find("acquire").unwrap())
            .expect("acquire should provide fallback hover");
        let acquire = acquire["contents"]["value"]
            .as_str()
            .expect("hover contents should be markdown");
        assert!(acquire
            .contains("function acquire(): ?SharedReference<T> | ?WritableSharedReference<T>"));
        assert!(acquire.contains("Returns `null`"));
        assert!(
            hover_at_offset("share;", 1).is_none(),
            "an unrelated identifier must not receive ownership-method hover"
        );
    }

    #[test]
    fn incomplete_member_completion_uses_the_resolved_shared_receiver() {
        let uri = "file:///completion.doria";
        let source = r#"class Counter
{
    function inspect(): int { return 1; }
    writable function increment(): void {}
}

function main(): void
{
    let $counter = shared new Counter();
    $counter->;
}
"#;
        let offset = source.find("->;").expect("completion accessor") + 2;
        let position = byte_offset_to_position(source, offset);
        let mut server = Server::default();
        server.documents.insert(
            uri.to_string(),
            Document::new(uri, source.to_string(), Some(1)),
        );
        let response = server.completion(Some(&json!({
            "textDocument": { "uri": uri },
            "position": { "line": position.line, "character": position.character },
        })));
        let labels = response["items"]
            .as_array()
            .expect("completion items")
            .iter()
            .filter_map(|item| item["label"].as_str())
            .collect::<Vec<_>>();
        assert!(labels.contains(&"share"));
        assert!(labels.contains(&"createWeakReference"));
        assert!(labels.contains(&"referencedValue"));
        assert!(labels.contains(&"inspect"));
        assert!(!labels.contains(&"increment"));
    }

    #[test]
    fn incomplete_static_completion_uses_compiler_enum_metadata() {
        let uri = "file:///enum-completion.doria";
        let source = r#"enum Status { case Draft; case Published; }
function main(): void { Status::; }
"#;
        let offset = source.find("::;").expect("static accessor") + 2;
        let position = byte_offset_to_position(source, offset);
        let mut server = Server::default();
        server.documents.insert(
            uri.to_string(),
            Document::new(uri, source.to_string(), Some(1)),
        );
        let response = server.completion(Some(&json!({
            "textDocument": { "uri": uri },
            "position": { "line": position.line, "character": position.character },
        })));
        let items = response["items"].as_array().expect("completion items");
        let labels = items
            .iter()
            .filter_map(|item| item["label"].as_str())
            .collect::<HashSet<_>>();
        assert_eq!(labels, HashSet::from(["Draft", "Published"]));
        assert!(items.iter().all(|item| item["kind"] == 20));
    }

    #[test]
    fn completions_do_not_offer_unrelated_future_types() {
        let labels = completion_labels();
        for unsupported in [
            "never",
            "Shared",
            "Weak",
            "SharedMut",
            "Sendable",
            "Shareable",
            "Ptr",
            "MutPtr",
        ] {
            assert!(
                !labels.iter().any(|label| label == unsupported),
                "unsupported future type `{unsupported}` must not be an active LSP completion"
            );
        }
    }

    #[test]
    fn completions_keep_supported_types() {
        let labels = completion_labels();
        for supported in [
            "void",
            "int",
            "int8",
            "int16",
            "int32",
            "int64",
            "uint8",
            "uint16",
            "uint32",
            "uint64",
            "float",
            "float32",
            "float64",
            "string",
            "bool",
            "mixed",
            "resource",
            "List",
            "Dictionary",
            "Set",
            "SortedDictionary",
            "SortedSet",
            "PriorityQueue",
            "Deque",
            "Bytes",
            "SharedReference",
            "WeakReference",
            "WritableSharedReference",
            "WritableWeakReference",
            "ReadonlySharedReferenceAccess",
            "WritableSharedReferenceAccess",
        ] {
            assert!(
                labels.iter().any(|label| label == supported),
                "supported or reserved type `{supported}` should remain an LSP completion"
            );
        }
    }

    #[test]
    fn integer_type_completions_and_hover_mark_stage_13_coverage() {
        let integer_types = [
            ("int", TokenKind::IntType),
            ("int8", TokenKind::Int8Type),
            ("int16", TokenKind::Int16Type),
            ("int32", TokenKind::Int32Type),
            ("int64", TokenKind::Int64Type),
            ("uint8", TokenKind::UInt8Type),
            ("uint16", TokenKind::UInt16Type),
            ("uint32", TokenKind::UInt32Type),
            ("uint64", TokenKind::UInt64Type),
        ];

        for (name, kind) in integer_types {
            let item = completion_item(name);
            assert_eq!(item["detail"], "implemented Doria integer type");
            assert!(item["documentation"]
                .as_str()
                .expect("integer type completion should have documentation")
                .contains("Implemented"));

            let hover = hover_description(&kind).expect("integer type should have hover text");
            assert!(hover.contains("Implemented"));
        }

        let int_documentation = completion_item("int")["documentation"]
            .as_str()
            .expect("int completion should have documentation")
            .to_string();
        assert!(int_documentation.contains("exact alias for `int64`"));
        let int_hover = hover_description(&TokenKind::IntType).expect("int should have hover text");
        assert!(int_hover.contains("exact alias for `int64`"));
    }

    #[test]
    fn float_and_bool_completions_and_hover_mark_stage_14_runtime_coverage() {
        for (name, kind) in [
            ("float", TokenKind::FloatType),
            ("float32", TokenKind::Float32Type),
            ("float64", TokenKind::Float64Type),
            ("bool", TokenKind::BoolType),
        ] {
            let item = completion_item(name);
            assert_eq!(item["detail"], "implemented Doria scalar type");
            assert!(item["documentation"]
                .as_str()
                .expect("scalar completion should have documentation")
                .contains("Implemented"));

            let hover = hover_description(&kind).expect("scalar should have hover text");
            assert!(hover.contains("Implemented"));
        }
        assert!(completion_item("float")["documentation"]
            .as_str()
            .unwrap()
            .contains("alias of `float64`"));
    }

    #[test]
    fn integer_conversion_completions_and_hover_are_exposed() {
        for companion in [
            "Int", "Int8", "Int16", "Int32", "Int64", "UInt8", "UInt16", "UInt32", "UInt64",
        ] {
            let label = format!("{companion}::from");
            let item = completion_item(&label);
            assert_eq!(item["detail"], "Doria integer conversion intrinsic");
            assert!(item["documentation"]
                .as_str()
                .expect("conversion completion should have documentation")
                .contains("panics"));

            let hover = hover_description(&TokenKind::Identifier(companion.to_string()))
                .expect("conversion companion should have hover text");
            assert!(hover.contains(&label));
        }

        let source = "let $converted = UInt8::from($value);";
        let from_offset = source.find("from").expect("source should contain from") + 1;
        let hover = hover_at_offset(source, from_offset)
            .expect("from in a conversion intrinsic should have contextual hover text");
        let hover_text = hover["contents"]["value"]
            .as_str()
            .expect("hover contents should be text");
        assert!(hover_text.contains("`UInt8::from(value)`"));
        assert!(hover_text.contains("Out-of-range conversion panics"));
    }

    #[test]
    fn cross_kind_conversion_completions_and_hover_are_exposed() {
        for (label, source, method) in [
            ("Int::toFloat", "Int::toFloat($value)", "toFloat"),
            ("Float::toInt", "Float::toInt($value)", "toInt"),
        ] {
            let item = completion_item(label);
            assert_eq!(item["detail"], "Doria scalar conversion intrinsic");
            let offset = source.find(method).unwrap() + 1;
            let hover = hover_at_offset(source, offset).expect("intrinsic should have hover");
            assert!(hover["contents"]["value"].as_str().unwrap().contains(label));
        }

        for name in ["toFloat", "toInt"] {
            let source = format!("function {name}(): int {{ return 42; }}");
            let offset = source.find(name).unwrap() + 1;
            let hover =
                hover_at_offset(&source, offset).expect("user function declaration should hover");
            let text = hover["contents"]["value"]
                .as_str()
                .expect("hover contents should be markdown");
            assert!(text.contains(&format!("function {name}(): int")));
            assert!(
                !text.contains("converts canonical"),
                "unqualified user function {name} must not receive intrinsic hover text"
            );
        }
    }

    #[test]
    fn completion_marks_resource_as_reserved() {
        assert_eq!(
            completion_detail("resource").as_deref(),
            Some("Reserved Doria type name")
        );
    }

    #[test]
    fn completion_and_hover_expose_panic_as_a_builtin_function() {
        assert_eq!(
            completion_detail("panic").as_deref(),
            Some("Doria built-in function")
        );
        let hover = hover_description(&TokenKind::Identifier("panic".to_string()))
            .expect("panic should have hover text");
        assert!(hover.contains("status 101"));
        assert!(hover.contains("not catchable"));
    }

    #[test]
    fn completions_and_hover_expose_stage17_builtins() {
        for (name, signature, required_hover) in [
            ("read_line", READ_LINE_SIGNATURE, "only at EOF"),
            (
                "sprintf",
                "sprintf(string $format, ...): string",
                "literal format",
            ),
            (
                "printf",
                "printf(string $format, ...): void",
                "adds no newline",
            ),
            ("read_file", "read_file(string $path): string", "UTF-8"),
            (
                "write_file",
                "write_file(string $path, string $contents): void",
                "UTF-8",
            ),
            (
                "write_stderr",
                "write_stderr(string $value): void",
                "without adding a newline",
            ),
        ] {
            assert_eq!(completion_detail(name).as_deref(), Some(signature));
            let hover = hover_description(&TokenKind::Identifier(name.to_string()))
                .expect("Stage 17 builtin should have hover text");
            assert!(hover.contains(required_hover), "{name}: {hover}");
        }
        assert!(!completion_labels().contains(&"print".to_string()));
    }

    #[test]
    fn read_line_hover_shows_the_prompt_parameter() {
        let hover = hover_description(&TokenKind::Identifier("read_line".to_string()))
            .expect("read_line should have hover text");
        assert!(hover.contains("string $prompt"));
    }

    #[test]
    fn read_line_hover_shows_the_empty_string_default() {
        let hover = hover_description(&TokenKind::Identifier("read_line".to_string()))
            .expect("read_line should have hover text");
        assert!(hover.contains("$prompt = \"\""));
    }

    #[test]
    fn completions_do_not_offer_camel_case_read_line() {
        assert!(!completion_labels().contains(&"readLine".to_string()));
    }

    #[test]
    fn completions_do_not_offer_the_php_readline_spelling() {
        assert!(!completion_labels().contains(&"readline".to_string()));
    }

    #[test]
    fn structured_diagnostics_preserve_severity_related_utf16_and_fix_safety() {
        let uri = "file:///main.doria";
        let text = "let $emoji = \"😀\";\n$x = 1;\n";
        let use_span = Span::new(text.find("$x").unwrap(), text.find("$x").unwrap() + 2);
        let emoji = text.find('😀').unwrap();
        let diagnostic = Diagnostic::new("E0201", "example warning", use_span)
            .with_title("Example Warning")
            .with_severity(DiagnosticSeverity::Warning)
            .with_label(
                DiagnosticSource::Current,
                Span::new(emoji, emoji + "😀".len()),
                LabelRole::Secondary,
                "wide character here",
            )
            .with_structured_fix(
                "Review Both Files",
                FixApplicability::RequiresReview,
                vec![
                    FixEdit {
                        source: DiagnosticSource::Current,
                        span: use_span,
                        replacement: "$value".to_string(),
                    },
                    FixEdit {
                        source: DiagnosticSource::Path("/other.doria".to_string()),
                        span: Span::new(0, 1),
                        replacement: "x".to_string(),
                    },
                ],
            );

        let lsp = diagnostic_to_lsp(uri, text, &diagnostic);
        assert_eq!(lsp["severity"], 2);
        assert_eq!(
            lsp["relatedInformation"][0]["location"]["range"]["start"]["character"],
            14
        );
        assert_eq!(
            lsp["relatedInformation"][0]["location"]["range"]["end"]["character"],
            16
        );
        assert_eq!(lsp["data"]["fixes"][0]["applicability"], "requiresReview");
        assert!(
            code_action_for_fix(uri, text, &diagnostic, &diagnostic.fixes[0]).is_none(),
            "requires-review fixes must never become automatic code actions"
        );

        let cross_file = Diagnostic::new("E0201", "cross-file fix", use_span).with_structured_fix(
            "Edit Both Files",
            FixApplicability::MachineApplicable,
            vec![
                FixEdit {
                    source: DiagnosticSource::Current,
                    span: use_span,
                    replacement: "$value".to_string(),
                },
                FixEdit {
                    source: DiagnosticSource::Path("/other.doria".to_string()),
                    span: Span::new(8, 12),
                    replacement: "name".to_string(),
                },
            ],
        );
        assert!(
            code_action_for_fix(uri, text, &cross_file, &cross_file.fixes[0]).is_none(),
            "cross-file fixes need target text before they can become automatic actions"
        );
        assert!(
            diagnostic_to_lsp(uri, text, &cross_file)["data"]["fix"].is_null(),
            "the legacy single-edit field must not advertise part of a multi-file fix"
        );
    }

    #[test]
    fn live_diagnostics_use_compiler_owned_duplicate_and_cause_grouping() {
        let uri = "file:///main.doria";
        let text = "$missing;\n$other;\n";
        let root = Diagnostic::new("E0201", "unknown identifier `$missing`", Span::new(0, 8))
            .with_cause("missing");
        let duplicate = root.clone();
        let consequence =
            Diagnostic::new("E0403", "cannot determine the value type", Span::new(0, 8))
                .with_cause("missing")
                .as_consequence();
        let independent =
            Diagnostic::new("E0201", "unknown identifier `$other`", Span::new(10, 16));

        let mut server = Server::default();
        server.documents.insert(
            uri.to_string(),
            Document {
                text: text.to_string(),
                version: Some(7),
                analysis: AnalysisSnapshot::from_diagnostics(vec![
                    root,
                    duplicate,
                    consequence,
                    independent,
                ]),
            },
        );
        let mut output = Vec::new();
        server.publish_diagnostics(uri, &mut output).unwrap();
        let body_start = output
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("LSP header terminator")
            + 4;
        let notification: Value =
            serde_json::from_slice(&output[body_start..]).expect("diagnostic notification");
        let diagnostics = notification["params"]["diagnostics"]
            .as_array()
            .expect("published diagnostics should be an array");

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics[0]["relatedInformation"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(diagnostics[1]["range"]["start"]["line"], 1);
        assert_eq!(notification["params"]["version"], 7);
    }

    #[test]
    fn grouped_local_completion_references_and_rename_use_utf16_ranges() {
        let uri = "file:///grouped.doria";
        let text = r#"function main(): void
{
    echo "😀";
    let $left, $right = 10;
    echo "{$left}:{$right}";
}
"#;
        let mut server = Server::default();
        server.documents.insert(
            uri.to_string(),
            Document::new(uri, text.to_string(), Some(1)),
        );

        let completion_offset = text.rfind("echo").unwrap();
        let completion_position = byte_offset_to_position(text, completion_offset);
        let completion = server.completion(Some(&json!({
            "textDocument": { "uri": uri },
            "position": {
                "line": completion_position.line,
                "character": completion_position.character,
            }
        })));
        let items = completion["items"].as_array().unwrap();
        assert!(items.iter().any(|item| item["label"] == "$left"));
        assert!(items.iter().any(|item| item["label"] == "$right"));

        let right_offset = text.find("$right").unwrap();
        let right_position = byte_offset_to_position(text, right_offset);
        let params = json!({
            "textDocument": { "uri": uri },
            "position": {
                "line": right_position.line,
                "character": right_position.character,
            }
        });
        let references = server.references(Some(&params));
        assert_eq!(references.as_array().unwrap().len(), 2);
        assert!(references.as_array().unwrap().iter().all(|location| {
            let line = location["range"]["start"]["line"].as_u64().unwrap() as u32;
            let character = location["range"]["start"]["character"].as_u64().unwrap() as u32;
            let offset = position_to_byte_offset(text, line, character);
            text[offset..].starts_with("$right")
        }));

        let mut references_without_declaration = params.clone();
        references_without_declaration["context"] = json!({ "includeDeclaration": false });
        let references = server.references(Some(&references_without_declaration));
        let locations = references.as_array().unwrap();
        assert_eq!(locations.len(), 1);
        let line = locations[0]["range"]["start"]["line"].as_u64().unwrap() as u32;
        let character = locations[0]["range"]["start"]["character"]
            .as_u64()
            .unwrap() as u32;
        assert_eq!(
            position_to_byte_offset(text, line, character),
            text.rfind("$right").unwrap()
        );

        let mut rename_params = params;
        rename_params["newName"] = json!("renamed");
        let rename = server.rename(Some(&rename_params));
        let edits = rename["changes"][uri].as_array().unwrap();
        assert_eq!(edits.len(), 2);
        assert!(edits.iter().all(|edit| edit["newText"] == "$renamed"));
    }

    #[test]
    fn rename_preserves_plain_symbol_spelling_for_classes_functions_and_methods() {
        let uri = "file:///rename.doria";
        let text = r#"class Widget
{
    function render(): void {}
}

function invoke(Widget $widget): void
{
    $widget->render();
}

function main(): void
{
    let $widget = new Widget();
    invoke($widget);
}
"#;
        let mut server = Server::default();
        server.documents.insert(
            uri.to_string(),
            Document::new(uri, text.to_string(), Some(1)),
        );

        for (offset, replacement, expected_edits) in [
            (text.find("Widget").unwrap(), "Replacement", 2),
            (text.find("invoke").unwrap(), "dispatch", 2),
            (text.find("render").unwrap(), "display", 2),
        ] {
            let position = byte_offset_to_position(text, offset);
            let rename = server.rename(Some(&json!({
                "textDocument": { "uri": uri },
                "position": {
                    "line": position.line,
                    "character": position.character,
                },
                "newName": replacement,
            })));
            let edits = rename["changes"][uri].as_array().unwrap();
            assert_eq!(edits.len(), expected_edits);
            assert!(edits.iter().all(|edit| edit["newText"] == replacement));
        }
    }
}
