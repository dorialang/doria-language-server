use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use serde_json::{json, Value};

use doriac::builtins::Builtin;
use doriac::diagnostics::{
    prepare_diagnostics, Diagnostic, DiagnosticFix, DiagnosticSeverity, DiagnosticSource,
    FixApplicability, FixEdit, LabelRole,
};
use doriac::lexer::{Token, TokenKind};
use doriac::names::{CompilationContext, Edition, PackageIdentity, SourceIdentity};
use doriac::source::Span;

mod analysis;
mod string_surface;
mod workspace_index;

use analysis::{AnalysisSnapshot, SemanticCompletion};
use string_surface::{STRING_COMPANION_METHODS, STRING_PROPERTIES};
use workspace_index::OpenDocumentIndex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}

pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

const BUILTINS: &[Builtin] = &[
    Builtin::Panic,
    Builtin::ReadLine,
    Builtin::Sprintf,
    Builtin::Printf,
    Builtin::ReadFile,
    Builtin::WriteFile,
    Builtin::AppendFile,
    Builtin::WriteStderr,
    Builtin::ReadFileBytes,
    Builtin::WriteFileBytes,
    Builtin::AppendFileBytes,
    Builtin::ReadStdinBytes,
    Builtin::WriteStdoutBytes,
    Builtin::WriteStderrBytes,
];

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
    #[cfg(test)]
    fn new(uri: &str, text: String, version: Option<i64>) -> Self {
        Self::with_context(uri, text, version, CompilationContext::standalone(uri))
    }

    fn with_context(
        uri: &str,
        text: String,
        version: Option<i64>,
        context: CompilationContext,
    ) -> Self {
        let analysis = AnalysisSnapshot::analyze_with_context(uri, &text, context);
        Self {
            text,
            version,
            analysis,
        }
    }
}

#[derive(Debug, Clone)]
struct WorkspaceRoot {
    uri: String,
    package: PackageIdentity,
}

#[derive(Default)]
struct Server {
    documents: HashMap<String, Document>,
    workspace_roots: Vec<WorkspaceRoot>,
    document_index: OpenDocumentIndex,
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
        || edits_overlap(&fix.edits)
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

fn edits_overlap(edits: &[FixEdit]) -> bool {
    let mut spans = edits.iter().map(|edit| edit.span).collect::<Vec<_>>();
    spans.sort_by_key(|span| (span.start, span.end));
    spans.windows(2).any(|pair| {
        pair[1].start < pair[0].end
            || (pair[0].start == pair[0].end
                && pair[1].start == pair[1].end
                && pair[0].start == pair[1].start)
    })
}

impl Server {
    fn handle_message<W: Write>(&mut self, message: Value, writer: &mut W) -> Result<bool, String> {
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return Ok(true);
        };

        let id = message.get("id").cloned();
        match method {
            "initialize" => {
                self.configure_workspace_roots(message.get("params"));
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
            "workspace/didChangeWorkspaceFolders" => {
                self.did_change_workspace_folders(message.get("params"), writer)?
            }
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
            "textDocument/signatureHelp" => {
                if let Some(id) = id {
                    send_response(writer, id, self.signature_help(message.get("params")))?;
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
            "textDocument/semanticTokens/full" => {
                if let Some(id) = id {
                    send_response(writer, id, self.semantic_tokens(message.get("params")))?;
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

        let context = self.compilation_context(uri);
        self.documents.insert(
            uri.to_string(),
            Document::with_context(uri, text.to_string(), version, context),
        );
        self.rebuild_document_index();
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

        let context = self.compilation_context(uri);
        self.documents.insert(
            uri.to_string(),
            Document::with_context(uri, text.to_string(), version, context),
        );
        self.rebuild_document_index();
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
            let context = self.compilation_context(uri);
            self.documents.insert(
                uri.to_string(),
                Document::with_context(uri, text.to_string(), version, context),
            );
            self.rebuild_document_index();
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
        self.rebuild_document_index();
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
        if let Some(hover) = self.document_index.hover(uri, offset) {
            return Some(json!({
                "contents": {
                    "kind": "markdown",
                    "value": hover.markdown,
                },
                "range": span_to_range(&document.text, hover.span),
            }));
        }
        if let Some(hover) = document.analysis.namespace_hover_at_offset(offset) {
            return Some(json!({
                "contents": {
                    "kind": "markdown",
                    "value": hover.markdown,
                },
                "range": span_to_range(&document.text, hover.span),
            }));
        }
        hover_at_offset_with_analysis(&document.text, offset, &document.analysis)
    }

    fn signature_help(&self, params: Option<&Value>) -> Value {
        let Some((uri, document, offset)) = self.uri_document_and_offset(params) else {
            return Value::Null;
        };
        let help = document
            .analysis
            .signature_help_at_offset(offset)
            .or_else(|| {
                AnalysisSnapshot::signature_help_for_incomplete_call(&uri, &document.text, offset)
            });
        let Some(help) = help else {
            return Value::Null;
        };
        json!({
            "signatures": [{ "label": help.label }],
            "activeSignature": 0,
            "activeParameter": help.active_parameter,
        })
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
            let suffix = document.text[offset..].trim_start();
            let insertion = if suffix.starts_with(',') || suffix.starts_with('}') {
                format!("{PLACEHOLDER} => 0")
            } else {
                PLACEHOLDER.to_string()
            };
            source.insert_str(offset, &insertion);
            let analysis = AnalysisSnapshot::analyze(&uri, &source);
            if let Some(completions) = analysis.static_completions_at_offset(offset) {
                return semantic_completion_items(completions);
            }
        }
        let mut completion = completion_items_with_analysis(&document.analysis, offset);
        if let Some(items) = completion.get_mut("items").and_then(Value::as_array_mut) {
            let mut labels = items
                .iter()
                .filter_map(|item| item.get("label").and_then(Value::as_str))
                .map(str::to_string)
                .collect::<std::collections::HashSet<_>>();
            for candidate in self.document_index.completions(&uri) {
                if labels.insert(candidate.label.clone()) {
                    items.push(json!({
                        "label": candidate.label,
                        "kind": candidate.kind,
                        "detail": candidate.detail,
                    }));
                }
            }
        }
        completion
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
        if let Some(target) = self.document_index.target_at(&uri, offset) {
            return Value::Array(
                self.document_index
                    .references(&target, include_declaration)
                    .into_iter()
                    .filter_map(|location| {
                        let target_document = self.documents.get(&location.uri)?;
                        Some(json!({
                            "uri": location.uri,
                            "range": span_to_range(&target_document.text, location.span),
                        }))
                    })
                    .collect(),
            );
        }
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
        if let Some(target) = self.document_index.target_at(&uri, offset) {
            let Some(edits) = self.document_index.rename(&target, new_name) else {
                return Value::Null;
            };
            let mut changes = serde_json::Map::new();
            for edit in edits {
                let Some(target_document) = self.documents.get(&edit.uri) else {
                    return Value::Null;
                };
                changes
                    .entry(edit.uri)
                    .or_insert_with(|| Value::Array(Vec::new()))
                    .as_array_mut()
                    .expect("workspace rename changes are arrays")
                    .push(json!({
                        "range": span_to_range(&target_document.text, edit.span),
                        "newText": edit.replacement,
                    }));
            }
            return json!({ "changes": changes });
        }
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

    fn semantic_tokens(&self, params: Option<&Value>) -> Value {
        let Some(uri) = params
            .and_then(|params| params.get("textDocument"))
            .and_then(|text_document| text_document.get("uri"))
            .and_then(Value::as_str)
        else {
            return json!({ "data": [] });
        };
        let Some(document) = self.documents.get(uri) else {
            return json!({ "data": [] });
        };

        let mut data = Vec::new();
        let mut previous_line = 0;
        let mut previous_start = 0;
        for (span, token_type) in document.analysis.semantic_token_spans() {
            let start = byte_offset_to_position(&document.text, span.start);
            let end = byte_offset_to_position(&document.text, span.end);
            if start.line != end.line {
                continue;
            }
            let delta_line = start.line - previous_line;
            let delta_start = if delta_line == 0 {
                start.character - previous_start
            } else {
                start.character
            };
            data.extend([
                delta_line,
                delta_start,
                end.character - start.character,
                token_type,
                0,
            ]);
            previous_line = start.line;
            previous_start = start.character;
        }
        json!({ "data": data })
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

    fn configure_workspace_roots(&mut self, params: Option<&Value>) {
        let mut uris = params
            .and_then(|params| params.get("workspaceFolders"))
            .and_then(Value::as_array)
            .map(|folders| {
                folders
                    .iter()
                    .filter_map(|folder| folder.get("uri").and_then(Value::as_str))
                    .map(normalize_root_uri)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if uris.is_empty() {
            if let Some(uri) = params
                .and_then(|params| params.get("rootUri"))
                .and_then(Value::as_str)
            {
                uris.push(normalize_root_uri(uri));
            }
        }
        uris.sort();
        uris.dedup();
        self.workspace_roots = uris
            .into_iter()
            .map(|uri| WorkspaceRoot {
                package: PackageIdentity::SyntheticTooling(format!("lsp-workspace:{uri}")),
                uri,
            })
            .collect();
        self.reanalyze_documents();
    }

    fn did_change_workspace_folders<W: Write>(
        &mut self,
        params: Option<&Value>,
        writer: &mut W,
    ) -> Result<(), String> {
        let Some(event) = params.and_then(|params| params.get("event")) else {
            return Ok(());
        };
        if let Some(removed) = event.get("removed").and_then(Value::as_array) {
            let removed = removed
                .iter()
                .filter_map(|folder| folder.get("uri").and_then(Value::as_str))
                .map(normalize_root_uri)
                .collect::<std::collections::HashSet<_>>();
            self.workspace_roots
                .retain(|root| !removed.contains(&root.uri));
        }
        if let Some(added) = event.get("added").and_then(Value::as_array) {
            for uri in added
                .iter()
                .filter_map(|folder| folder.get("uri").and_then(Value::as_str))
                .map(normalize_root_uri)
            {
                if self.workspace_roots.iter().any(|root| root.uri == uri) {
                    continue;
                }
                self.workspace_roots.push(WorkspaceRoot {
                    package: PackageIdentity::SyntheticTooling(format!("lsp-workspace:{uri}")),
                    uri,
                });
            }
        }
        self.workspace_roots
            .sort_by(|left, right| left.uri.cmp(&right.uri));
        self.reanalyze_documents();
        for uri in self.documents.keys() {
            self.publish_diagnostics(uri, writer)?;
        }
        Ok(())
    }

    fn compilation_context(&self, uri: &str) -> CompilationContext {
        let package = self
            .workspace_roots
            .iter()
            .filter(|root| uri_is_within(uri, &root.uri))
            .max_by_key(|root| root.uri.len())
            .map(|root| root.package.clone())
            .unwrap_or_else(|| PackageIdentity::SyntheticTooling(format!("lsp-standalone:{uri}")));
        CompilationContext {
            edition: Edition::Doria2026,
            package,
            source: SourceIdentity(uri.to_string()),
        }
    }

    fn reanalyze_documents(&mut self) {
        let documents = std::mem::take(&mut self.documents);
        self.documents = documents
            .into_iter()
            .map(|(uri, document)| {
                let context = self.compilation_context(&uri);
                let document =
                    Document::with_context(&uri, document.text, document.version, context);
                (uri, document)
            })
            .collect();
        self.rebuild_document_index();
    }

    fn rebuild_document_index(&mut self) {
        self.document_index = OpenDocumentIndex::rebuild(
            self.documents
                .iter()
                .map(|(uri, document)| (uri.as_str(), &document.analysis)),
        );
    }
}

fn normalize_root_uri(uri: &str) -> String {
    uri.trim_end_matches('/').to_string()
}

fn uri_is_within(uri: &str, root: &str) -> bool {
    uri == root
        || uri
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('/'))
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
            "signatureHelpProvider": {
                "triggerCharacters": ["(", ","]
            },
            "referencesProvider": true,
            "renameProvider": true,
            "semanticTokensProvider": {
                "legend": {
                    "tokenTypes": ["variable", "type", "enumMember", "function", "keyword", "namespace", "string"],
                    "tokenModifiers": []
                },
                "full": true
            },
            "codeActionProvider": true,
            "workspace": {
                "workspaceFolders": {
                    "supported": true,
                    "changeNotifications": true
                }
            }
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
        "once",
    ];
    let planned_keywords = [
        "interface",
        "async",
        "await",
        "unsafe",
        "extern",
        "open",
        "override",
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
        match keyword {
            "fn" => {
                item["detail"] = json!("Doria arrow-closure keyword");
                item["documentation"] = json!("Declares an arrow closure. Parameters are explicitly typed and the return type is inferred from the expression body. Enclosing locals must be listed explicitly in a `with` clause. The compiler checks closure signatures, captures, ownership, invocation mode, checked effects, and escape.");
            }
            "with" => {
                item["detail"] = json!("Doria closure-capture keyword");
                item["documentation"] = json!("Introduces an explicit closure capture list. A bare capture is readonly, `writable` requests exclusive writable access, and `take` transfers ownership. The compiler validates capture names, modes, ownership, lifetimes, and escape.");
            }
            "once" => {
                item["detail"] = json!("Doria function-type invocation modifier");
                item["documentation"] = json!("Marks a structural function type as consuming and one-shot. Calling a value of this type consumes the function value. The compiler infers and enforces closure invocation modes.");
            }
            _ if planned => {
                item["documentation"] =
                    json!("Accepted planned Doria syntax; compiler support lands in a later stage.");
            }
            _ => {}
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
        "label": "Error",
        "kind": 8,
        "detail": "compiler-known Doria interface",
        "documentation": "`interface Error` is the compiler-known checked-error contract. A conforming class explicitly declares `implements Error` and exposes an externally accessible readonly `string $message` property.",
    }));
    items.push(json!({
        "label": "toString",
        "kind": 2,
        "detail": "function toString(): string",
        "documentation": "The exact readonly instance method required by the compiler-known `Displayable` contract.",
    }));
    items.extend(BUILTINS.iter().copied().map(|builtin| {
        json!({
            "label": builtin.name(),
            "kind": 3,
            "detail": builtin.signature(),
            "documentation": builtin_documentation(builtin),
        })
    }));
    items.extend(
        doriac::compiler_known_io::CANONICAL_TYPES
            .into_iter()
            .map(|name| {
                json!({
                    "label": name,
                    "kind": 25,
                    "detail": "compiler-known Doria standard-library type",
                    "documentation": compiler_known_io_documentation(name),
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
    if let Some((description, span)) = compiler_known_io_hover_at(&tokens, token_index) {
        return Some(json!({
            "contents": {
                "kind": "markdown",
                "value": description,
            },
            "range": span_to_range(text, span),
        }));
    }
    let description = string_companion_hover_at(&tokens, token_index)
        .or_else(|| compiler_builtin_hover_at(&tokens, token_index))
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

fn compiler_builtin_hover_at(tokens: &[Token], token_index: usize) -> Option<String> {
    let TokenKind::Identifier(name) = &tokens.get(token_index)?.kind else {
        return None;
    };
    let builtin = Builtin::from_name(name)?;
    Some(format!(
        "```doria\n{}\n```\n\n{}",
        builtin.signature(),
        builtin_documentation(builtin)
    ))
}

fn compiler_known_io_hover_at(tokens: &[Token], token_index: usize) -> Option<(String, Span)> {
    if token_index < 6 {
        return None;
    }
    let segment = &tokens[token_index - 6..=token_index];
    let [Token {
        kind: TokenKind::Identifier(root),
        ..
    }, Token {
        kind: TokenKind::Backslash,
        ..
    }, Token {
        kind: TokenKind::Identifier(standard),
        ..
    }, Token {
        kind: TokenKind::Backslash,
        ..
    }, Token {
        kind: TokenKind::Identifier(io),
        ..
    }, Token {
        kind: TokenKind::Backslash,
        ..
    }, Token {
        kind: TokenKind::Identifier(name),
        ..
    }] = segment
    else {
        return None;
    };
    if root != "Doria" || standard != "Std" || io != "Io" {
        return None;
    }
    let qualified = format!("Doria\\Std\\Io\\{name}");
    let documentation = compiler_known_io_documentation(&qualified)?;
    Some((
        format!("```doria\n{qualified}\n```\n\n{documentation}"),
        Span::new(segment[0].span.start, segment[6].span.end),
    ))
}

fn compiler_known_io_documentation(name: &str) -> Option<&'static str> {
    match name {
        doriac::compiler_known_io::IO_OPERATION => Some(
            "Compiler-known I/O operation enum with `Open`, `Read`, `Write`, `Append`, and `Flush` cases.",
        ),
        doriac::compiler_known_io::IO_TARGET => Some(
            "Compiler-known I/O target enum with `File(string $path)`, `StandardInput`, `StandardOutput`, and `StandardError` cases.",
        ),
        doriac::compiler_known_io::IO_ERROR_REASON => Some(
            "Compiler-known I/O reason enum with stable portable categories; host-specific codes remain in `IoError::$systemCode`.",
        ),
        doriac::compiler_known_io::UTF8_INPUT_SOURCE => Some(
            "Compiler-known UTF-8 input-source enum with `File(string $path)` and `StandardInput` cases.",
        ),
        doriac::compiler_known_io::IO_ERROR => Some(
            "Compiler-known checked `Error` carrying readonly `message`, `operation`, `target`, `reason`, and nullable `systemCode` properties.",
        ),
        doriac::compiler_known_io::INVALID_UTF8_ERROR => Some(
            "Compiler-known checked `Error` carrying readonly `message`, `source`, `validByteCount`, and nullable `invalidByteCount` properties.",
        ),
        _ => None,
    }
}

fn builtin_documentation(builtin: Builtin) -> &'static str {
    match builtin {
        Builtin::Panic => {
            "Terminates execution with a fatal panic, Doria stack trace, and status 101. Panics are not checked errors and are not catchable."
        }
        Builtin::ReadLine => {
            "Writes and flushes the prompt, then reads one UTF-8 line. Returns `null` only at EOF and `\"\"` for a blank line. I/O and invalid UTF-8 failures are checked errors."
        }
        Builtin::Sprintf => {
            "Formats values with a compile-time-checked literal format string without performing I/O."
        }
        Builtin::Printf => {
            "Writes a compile-time-checked format with no added newline. Output failure is a checked I/O error."
        }
        Builtin::ReadFile => {
            "Reads a complete UTF-8 text file. I/O and invalid UTF-8 failures are distinct checked errors."
        }
        Builtin::WriteFile => {
            "Creates or truncates a UTF-8 text file and writes exact bytes. Failure is a checked I/O error."
        }
        Builtin::AppendFile => {
            "Creates or appends exact UTF-8 bytes. Failure is a checked I/O error."
        }
        Builtin::WriteStderr => {
            "Writes exact UTF-8 bytes to stderr without adding a newline. Failure is a checked I/O error."
        }
        Builtin::ReadFileBytes => {
            "Reads a whole file as raw bytes without UTF-8 validation. Failure is a checked I/O error."
        }
        Builtin::WriteFileBytes => {
            "Creates or truncates a file with exact bytes. Failure is a checked I/O error."
        }
        Builtin::AppendFileBytes => {
            "Creates or appends exact bytes. Failure is a checked I/O error."
        }
        Builtin::ReadStdinBytes => {
            "Reads all standard input as raw bytes, returning empty `Bytes` at EOF. Failure is a checked I/O error."
        }
        Builtin::WriteStdoutBytes => {
            "Writes exact bytes to standard output. Failure is a checked I/O error."
        }
        Builtin::WriteStderrBytes => {
            "Writes exact bytes to standard error. Failure is a checked I/O error."
        }
    }
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
            "Declares an interface. The compiler currently provides the compiler-known `Displayable` and `Error` contracts.",
        ),
        TokenKind::Implements => Some(
            "Declares nominal conformance to a compiler-known contract such as `Displayable` or `Error`.",
        ),
        TokenKind::Function => Some(
            "Declares a named function or method, an anonymous block closure, or a structural function type according to context. Function types preserve readonly, writable, or once invocation; parameter ownership; and checked effects. The compiler checks structural callable compatibility.",
        ),
        TokenKind::Fn => Some(
            "Declares an arrow closure. Parameters are explicitly typed and the return type is inferred from the expression body. Enclosing locals must be listed explicitly in a `with` clause. The compiler checks closure signatures, captures, ownership, invocation mode, checked effects, and escape.",
        ),
        TokenKind::With => Some(
            "Introduces an explicit closure capture list. A bare capture is readonly, `writable` requests exclusive writable access, and `take` transfers ownership. The compiler validates capture names, modes, ownership, lifetimes, and escape.",
        ),
        TokenKind::Let => Some("Declares a local binding with an inferred type."),
        TokenKind::Take => Some(
            "Gives ownership of a move-type argument or structural function value to this parameter. Call sites remain unmarked. Consuming invocation is written `function once(...)`, not `function take(...)`.",
        ),
        TokenKind::Writable => Some(
            "Marks a binding, property, parameter, or method receiver as mutable. In a structural function type it may independently mark writable invocation or a writable parameter borrow.",
        ),
        TokenKind::Once => Some(
            "Marks a structural function type as consuming and one-shot. Calling a value of this type consumes the function value. The compiler infers and enforces closure invocation modes.",
        ),
        TokenKind::Internal => {
            Some("Marks a class member as hidden from the external object surface.")
        }
        TokenKind::Readonly => Some("Reserved for explicit readonly syntax."),
        TokenKind::Return => Some("Returns a value from the current function."),
        TokenKind::If => Some("Selects statement control flow when its condition is true."),
        TokenKind::Else => Some("Selects the next branch when preceding conditions are false."),
        TokenKind::While => Some("Repeats a statement block while its condition remains true."),
        TokenKind::Do => Some("Runs a statement block before testing its `while` condition."),
        TokenKind::When => Some(
            "Begins an exhaustive conditional expression whose selected branch yields a value.",
        ),
        TokenKind::Given => Some(
            "Runs scoped setup and ordered boolean predicates before an attached `if`, `when`, or `while` condition.",
        ),
        TokenKind::Finally => Some(
            "Runs exactly once when the attached control-flow construct leaves normally or through a structured transfer. Fatal panic bypasses it.",
        ),
        TokenKind::Try => Some(
            "Protects operations with checked effects so source-ordered `catch` clauses can handle them.",
        ),
        TokenKind::Catch => Some(
            "Handles one exact checked-error type from the protected `try` block. `catch (Error)` is the catch-all form.",
        ),
        TokenKind::Throw => Some(
            "Transfers ownership of one explicit `Error` value as a checked effect.",
        ),
        TokenKind::Throws => Some(
            "Declares a reusable callable's source-ordered checked-error effect set. The selected program entrypoint may omit it and infer escaping effects.",
        ),
        TokenKind::Echo => Some(
            "`echo value;` writes the displayed value and has the checked `Doria\\Std\\Io\\IoError` effect.",
        ),
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
            "Begins an exhaustive expression that selects one value through enum, constant, null, exact-type, or ordered bool patterns.",
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
        TokenKind::Identifier(name) if Builtin::from_name(name).is_some() => {
            Builtin::from_name(name).map(builtin_documentation)
        }
        TokenKind::Identifier(name) => match name.as_str() {
            "Error" => Some("`interface Error` is the compiler-known checked-error contract. Conforming classes explicitly declare `implements Error` and expose an externally accessible readonly `string $message` property."),
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
            "mixed" => Some("The dynamic boundary type: a boxed runtime value that accepts any type but rejects every operation until narrowed with the exact `is` type-test operator or an exact `match` type-binding pattern. `?mixed` adds nullability."),
            "resource" => Some("Reserved for future PHP interop; not a usable core type."),
            companion @ ("Int" | "Int8" | "Int16" | "Int32" | "Int64" | "UInt8"
            | "UInt16" | "UInt32" | "UInt64") => integer_conversion_description(companion),
            "Bytes" => Some("`Bytes` is the owned, mutable byte-buffer move type: `Bytes::fromArray` / `->toArray` (copying), the `length` property, byte indexing with in-place writes, and byte-wise equality. It converts to and from `uint8[]` only explicitly."),
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
        "developmentOnly": diagnostic.development_only,
        "causeId": diagnostic.cause_id,
        "fixes": diagnostic.fixes.iter().map(|fix| json!({
            "title": fix.title,
            "applicability": fix.applicability.as_str(),
            "edits": fix.edits.iter().filter_map(|edit| {
                let edit_uri = diagnostic_source_uri(uri, &edit.source)?;
                Some(json!({
                    "uri": edit_uri,
                    "range": span_to_range(
                        if matches!(&edit.source, DiagnosticSource::Current) { text } else { "" },
                        edit.span,
                    ),
                    "newText": edit.replacement,
                }))
            }).collect::<Vec<_>>(),
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
        .filter_map(|label| {
            let related_uri = diagnostic_source_uri(uri, &label.source)?;
            Some(json!({
                "location": {
                    "uri": related_uri,
                    "range": span_to_range(
                        if matches!(&label.source, DiagnosticSource::Current) { text } else { "" },
                        label.span,
                    ),
                },
                "message": label.message,
            }))
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

fn diagnostic_source_uri(current_uri: &str, source: &DiagnosticSource) -> Option<String> {
    match source {
        DiagnosticSource::Current => Some(current_uri.to_string()),
        DiagnosticSource::Path(path) if path.contains("://") => Some(path.clone()),
        DiagnosticSource::Path(path) => Some(format!("file://{path}")),
        DiagnosticSource::Unavailable => None,
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
    fn completion_and_hover_describe_current_closure_behavior_and_target_capabilities() {
        let arrow = completion_item("fn");
        assert_eq!(arrow["detail"], "Doria arrow-closure keyword");
        assert!(arrow["documentation"]
            .as_str()
            .is_some_and(|text| text.contains("The compiler checks closure signatures")));
        assert!(arrow["documentation"]
            .as_str()
            .is_some_and(|text| !text.contains("target")));

        let capture = completion_item("with");
        assert_eq!(capture["detail"], "Doria closure-capture keyword");
        assert!(capture["documentation"]
            .as_str()
            .is_some_and(|text| text.contains("`take` transfers ownership")));

        let once = completion_item("once");
        assert_eq!(once["detail"], "Doria function-type invocation modifier");
        assert!(once["documentation"]
            .as_str()
            .is_some_and(|text| text.contains("consuming and one-shot")));

        assert!(hover_description(&TokenKind::Fn)
            .is_some_and(|text| text.contains("The compiler checks closure signatures")));
        assert!(hover_description(&TokenKind::Fn).is_some_and(|text| !text.contains("target")));
        assert!(hover_description(&TokenKind::With).is_some_and(|text| text
            .contains("validates capture names, modes, ownership, lifetimes, and escape")));

        let source = "let $minimum = 70; let $f = fn(int $value) with ($minimum) => $value; function accept(take function once(): int $callback): void { $callback(); }";
        let closure = hover_at_offset(source, source.find("fn(").unwrap()).expect("closure hover");
        let closure_text = closure["contents"]["value"].as_str().unwrap();
        assert!(closure_text.contains("function(int): int"));
        assert!(closure_text.contains("Inferred invocation mode"));
        assert!(closure_text.contains("Readonly capture of `$minimum`"));
        assert!(closure_text.contains("Executable In Debug And Native Targets"));
        assert!(closure_text.contains(
            "Explicit closure lowering is available when the program's value families and operations are supported by the PHP backend"
        ));
        assert!(!closure_text.contains("BindingId"));
        assert!(!closure_text.contains("ClosureId"));

        let capture_offset = source.find("($minimum)").unwrap() + 1;
        let capture = hover_at_offset(source, capture_offset).expect("capture occurrence hover");
        assert!(capture["contents"]["value"]
            .as_str()
            .is_some_and(|text| text.contains("Readonly capture of `$minimum`")));

        let function_type =
            hover_at_offset(source, source.find("function once").expect("function type"))
                .expect("semantic function-type hover");
        assert!(function_type["contents"]["value"]
            .as_str()
            .is_some_and(|text| text.contains("function once(): int")));

        let callback_binding = source.find("$callback").expect("function parameter");
        let binding = hover_at_offset(source, callback_binding).expect("function binding hover");
        assert!(binding["contents"]["value"]
            .as_str()
            .is_some_and(|text| text.contains("function once(): int $callback")));

        let callback_use = source.rfind("$callback").expect("callable use") + "$callback".len() + 1;
        let call = hover_at_offset(source, callback_use).expect("callable-value hover");
        assert!(call["contents"]["value"]
            .as_str()
            .is_some_and(|text| text.contains("Semantically checked callable-value invocation")));
    }

    #[test]
    fn mixed_function_narrowing_hover_preserves_exact_compiler_identity() {
        let source = r#"function main(): void
{
    mixed $value = fn(int $number) => $number + 1;
    if ($value is function(int): int) {
        int $result = $value(41);
    }
}"#;

        let declaration = hover_at_offset(source, source.find("$value").unwrap())
            .expect("mixed declaration hover");
        let declaration_text = declaration["contents"]["value"].as_str().unwrap();
        assert!(declaration_text.contains("mixed $value"));
        assert!(!declaration_text.contains("Execution capability"));

        let narrowed_offset = source.rfind("$value").unwrap();
        let narrowed = hover_at_offset(source, narrowed_offset).expect("narrowed function hover");
        let narrowed_text = narrowed["contents"]["value"].as_str().unwrap();
        assert!(narrowed_text.contains("function(int): int"));
        assert!(narrowed_text.contains("Compiler-resolved function value after flow narrowing"));
        assert!(narrowed_text.contains("Executable In Debug And Native Targets"));
        assert!(!narrowed_text.contains("mixed $value"));

        let call = hover_at_offset(source, source.rfind("41").unwrap())
            .expect("narrowed callable invocation hover");
        assert!(call["contents"]["value"]
            .as_str()
            .is_some_and(|text| text.contains("function(int): int")
                && text.contains("Semantically checked callable-value invocation")));
    }

    #[test]
    fn captured_mixed_function_hover_preserves_narrowing_and_capture_facts() {
        let source = r#"function main(): void
{
    mixed $value = fn() => 42;
    let $wrapper = function (): int with ($value) {
        if ($value is function(): int) {
            return $value();
        }
        return 0;
    };
    echo "{$wrapper()}";
}
"#;

        let capture =
            hover_at_offset(source, source.find("($value").unwrap() + 1).expect("capture hover");
        let capture_text = capture["contents"]["value"].as_str().unwrap();
        assert!(capture_text.contains("mixed $value"));
        assert!(capture_text.contains("Readonly capture of `$value`"));

        let use_offset = source.find("$value();").unwrap();
        let narrowed = hover_at_offset(source, use_offset).expect("narrowed capture hover");
        let narrowed_text = narrowed["contents"]["value"].as_str().unwrap();
        assert!(narrowed_text.contains("function(): int $value"));
        assert!(narrowed_text.contains("Readonly capture of `$value`"));
        assert!(narrowed_text.contains("Compiler-resolved function value after flow narrowing"));
        assert!(narrowed_text.contains("Execution capability"));
        assert!(!narrowed_text.contains("mixed $value"));
    }

    #[test]
    fn invalid_closure_hover_does_not_claim_target_execution() {
        let source = "let $outside = 1; let $invalid = fn() => $outside;";
        let diagnostics = diagnostics_for_document("file:///invalid-hover.doria", source);
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic["code"] == "E0642"));
        let hover = hover_at_offset(source, source.find("fn()").expect("invalid closure"))
            .expect("lexical closure hover remains available");
        let text = hover["contents"]["value"].as_str().unwrap();

        assert!(!text.contains("**Execution capability:**"));
        assert!(!text.contains("**PHP compatibility:**"));
    }

    #[test]
    fn unrelated_closure_errors_do_not_hide_valid_target_capabilities() {
        let source =
            "let $outside = 1; let $valid = fn() => 42; let $invalid = fn() => $outside; $valid();";
        let diagnostics = diagnostics_for_document("file:///mixed-hover.doria", source);
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic["code"] == "E0642"));

        let valid = hover_at_offset(source, source.find("fn() => 42").expect("valid closure"))
            .expect("valid closure hover");
        let valid_text = valid["contents"]["value"].as_str().unwrap();
        assert!(valid_text.contains("Executable In Debug And Native Targets"));
        assert!(valid_text.contains("**PHP compatibility:**"));

        let call_offset = source.find("$valid()").expect("valid call") + "$valid".len() + 1;
        let call = hover_at_offset(source, call_offset).expect("valid callable-value hover");
        let call_text = call["contents"]["value"].as_str().unwrap();
        assert!(
            call_text.contains("Executable In Debug And Native Targets")
                && call_text.contains("**PHP compatibility:**"),
            "{diagnostics:#?}\n{call_text}"
        );

        let invalid = hover_at_offset(
            source,
            source.find("fn() => $outside").expect("invalid closure"),
        )
        .expect("invalid closure hover");
        let invalid_text = invalid["contents"]["value"].as_str().unwrap();
        assert!(
            !invalid_text.contains("**Execution capability:**")
                && !invalid_text.contains("**PHP compatibility:**")
        );
    }

    #[test]
    fn completions_and_hover_expose_active_control_flow_foundations() {
        for (keyword, kind) in [
            ("when", TokenKind::When),
            ("given", TokenKind::Given),
            ("finally", TokenKind::Finally),
            ("do", TokenKind::Do),
        ] {
            assert_eq!(completion_item(keyword)["detail"], "Doria keyword");
            assert!(hover_description(&kind).is_some());
        }
    }

    #[test]
    fn completions_and_hover_expose_checked_error_foundations() {
        for (keyword, kind) in [
            ("try", TokenKind::Try),
            ("catch", TokenKind::Catch),
            ("throw", TokenKind::Throw),
            ("throws", TokenKind::Throws),
            ("finally", TokenKind::Finally),
        ] {
            assert_eq!(completion_item(keyword)["detail"], "Doria keyword");
            assert!(hover_description(&kind).is_some());
        }
        assert_eq!(
            completion_item("Error")["detail"],
            "compiler-known Doria interface"
        );
        assert!(hover_description(&TokenKind::Identifier("Error".to_string())).is_some());
    }

    #[test]
    fn take_completion_describes_active_ownership_syntax() {
        assert_eq!(completion_item("take")["detail"], "Doria keyword");
        assert!(hover_description(&TokenKind::Take).is_some());
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
        assert!(mixed_hover.contains("exact `match` type-binding pattern"));

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
    fn enum_case_completion_works_in_match_pattern_position() {
        let uri = "file:///match-completion.doria";
        let source = r#"enum Status { case Draft; case Published; }
function main(): void
{
    Status $status = Status::Draft;
    string $label = match ($status) { Status::, };
}
"#;
        let offset = source.find("::,").expect("pattern accessor") + 2;
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
            .collect::<HashSet<_>>();
        assert_eq!(labels, HashSet::from(["Draft", "Published"]));
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
            Some(Builtin::Panic.signature())
        );
        let source = "panic(\"boom\");";
        let hover = hover_at_offset(source, 1).expect("panic should have hover text")["contents"]
            ["value"]
            .as_str()
            .expect("panic hover should be markdown")
            .to_string();
        assert!(hover.contains(Builtin::Panic.signature()));
        assert!(hover.contains("status 101"));
        assert!(hover.contains("not catchable"));
    }

    #[test]
    fn completions_and_hover_expose_compiler_owned_builtin_contracts() {
        for (builtin, required_hover) in [
            (Builtin::ReadLine, "only at EOF"),
            (Builtin::Sprintf, "literal format"),
            (Builtin::Printf, "checked I/O error"),
            (Builtin::ReadFile, "invalid UTF-8"),
            (Builtin::WriteFile, "checked I/O error"),
            (Builtin::AppendFile, "checked I/O error"),
            (Builtin::WriteStderr, "without adding a newline"),
            (Builtin::ReadFileBytes, "without UTF-8 validation"),
            (Builtin::WriteFileBytes, "checked I/O error"),
            (Builtin::AppendFileBytes, "checked I/O error"),
            (Builtin::ReadStdinBytes, "empty `Bytes` at EOF"),
            (Builtin::WriteStdoutBytes, "standard output"),
            (Builtin::WriteStderrBytes, "standard error"),
        ] {
            assert_eq!(
                completion_detail(builtin.name()).as_deref(),
                Some(builtin.signature())
            );
            let source = format!("{}();", builtin.name());
            let hover = hover_at_offset(&source, 1)
                .expect("compiler-owned builtin should have hover text")["contents"]["value"]
                .as_str()
                .expect("builtin hover should be markdown")
                .to_string();
            assert!(hover.contains(builtin.signature()));
            assert!(
                hover.contains(required_hover),
                "{}: {hover}",
                builtin.name()
            );
        }
        assert!(!completion_labels().contains(&"print".to_string()));
    }

    #[test]
    fn compiler_known_io_types_have_qualified_utf16_safe_hovers_only() {
        let labels = completion_labels();
        for qualified in doriac::compiler_known_io::CANONICAL_TYPES {
            assert!(labels.contains(&qualified.to_string()), "{qualified}");
        }

        let source = "let $emoji = \"😀\"; Doria\\Std\\Io\\IoError $error;";
        let qualified = doriac::compiler_known_io::IO_ERROR;
        let start = source.find(qualified).expect("qualified I/O type");
        let hover = hover_at_offset(source, start + qualified.len() - 1)
            .expect("qualified I/O type should hover");
        let markdown = hover["contents"]["value"]
            .as_str()
            .expect("qualified I/O hover should be markdown");
        assert!(markdown.contains(qualified));
        assert!(markdown.contains("readonly `message`"));
        assert_eq!(
            hover["range"]["start"]["character"],
            byte_offset_to_position(source, start).character
        );
        assert_eq!(
            hover["range"]["end"]["character"],
            byte_offset_to_position(source, start + qualified.len()).character
        );

        let short = "IoError $error;";
        assert!(hover_at_offset(short, 1).is_none());
        assert!(!labels.contains(&"IoError".to_string()));
    }

    #[test]
    fn echo_hover_names_its_checked_io_effect() {
        let hover = hover_description(&TokenKind::Echo).expect("echo hover");
        assert!(hover.contains(doriac::compiler_known_io::IO_ERROR));
    }

    #[test]
    fn read_line_hover_shows_the_prompt_parameter() {
        let source = "read_line();";
        let hover = hover_at_offset(source, 1).expect("read_line should have hover text")
            ["contents"]["value"]
            .as_str()
            .expect("read_line hover should be markdown")
            .to_string();
        assert!(hover.contains("string $prompt"));
    }

    #[test]
    fn control_flow_protocol_ranges_remain_utf16_safe() {
        let uri = "file:///control-flow-utf16.doria";
        let text = r#"function main(): void
{
    echo "😀"; given {
        /* 😀 */ let $prepared = true;
        /* 😀 */ true;
    } if ($prepared) {
        echo "{$prepared}";
        let $branchOnly = true;
    } /* 😀 */ finally {
        let $cleanup = $prepared;
    }

    echo "😀"; string $label = given {
        let $choice = true;
        true;
    } when (/* 😀 */ $choice): string {
        echo "😀"; return "selected";
    } else {
        return "fallback";
    };

    do {
        echo $label;
    } while (/* 😀 */ true);

}
"#;
        let mut server = Server::default();
        server.documents.insert(
            uri.to_string(),
            Document::new(uri, text.to_string(), Some(1)),
        );

        let position_at = |offset: usize| {
            let position = byte_offset_to_position(text, offset);
            json!({ "line": position.line, "character": position.character })
        };
        let params_at = |offset: usize| {
            json!({
                "textDocument": { "uri": uri },
                "position": position_at(offset),
            })
        };
        let utf16_character = |offset: usize| {
            let line_start = text[..offset].rfind('\n').map_or(0, |index| index + 1);
            text[line_start..offset].encode_utf16().count() as u64
        };
        let hover_cases = [
            (text.find("given {").unwrap(), "scoped setup"),
            (text.find("$prepared").unwrap(), "bool $prepared"),
            (
                text.find("/* 😀 */ true;").unwrap() + "/* 😀 */ ".len(),
                "given predicate: bool",
            ),
            (text.find("$choice): string").unwrap(), "bool $choice"),
            (text.find("return \"selected\"").unwrap(), "Yields a value"),
            (text.find("true);").unwrap(), "do ... while condition: bool"),
            (text.find("finally").unwrap(), "Runs exactly once"),
        ];
        for (offset, expected) in hover_cases {
            let hover = server
                .hover(Some(&params_at(offset)))
                .unwrap_or_else(|| panic!("missing hover at {offset}"));
            assert!(
                hover["contents"]["value"]
                    .as_str()
                    .is_some_and(|value| value.contains(expected)),
                "{hover:#}"
            );
            assert_eq!(
                hover["range"]["start"]["character"],
                utf16_character(offset)
            );
        }

        let prepared = text.find("$prepared").unwrap();
        let reference_params = json!({
            "textDocument": { "uri": uri },
            "position": position_at(prepared),
            "context": { "includeDeclaration": true },
        });
        let references = server.references(Some(&reference_params));
        assert_eq!(references.as_array().map(Vec::len), Some(4));

        let rename_params = json!({
            "textDocument": { "uri": uri },
            "position": position_at(prepared),
            "newName": "ready",
        });
        let rename = server.rename(Some(&rename_params));
        let edits = rename["changes"][uri]
            .as_array()
            .expect("given rename edits");
        assert_eq!(edits.len(), 4);
        assert!(edits.iter().all(|edit| edit["newText"] == "$ready"));

        let semantic = server.semantic_tokens(Some(&json!({
            "textDocument": { "uri": uri },
        })));
        let data = semantic["data"].as_array().expect("semantic token data");
        let prepared_position = byte_offset_to_position(text, prepared);
        let mut line = 0_u64;
        let mut character = 0_u64;
        let mut found_prepared = false;
        for token in data.as_chunks::<5>().0 {
            let delta_line = token[0].as_u64().unwrap();
            let delta_start = token[1].as_u64().unwrap();
            line += delta_line;
            character = if delta_line == 0 {
                character + delta_start
            } else {
                delta_start
            };
            if line == prepared_position.line as u64
                && character == prepared_position.character as u64
                && token[2] == 9
                && token[3] == 0
            {
                found_prepared = true;
            }
        }
        assert!(found_prepared, "given local needs a UTF-16 semantic token");

        assert_eq!(diagnostics_for_document(uri, text), Vec::<Value>::new());

        let finalizer_body = text.find("let $cleanup = $prepared;").unwrap();
        assert!(!server
            .completion(Some(&params_at(finalizer_body)))
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["label"] == "$branchOnly")));
    }

    #[test]
    fn read_line_hover_shows_the_empty_string_default() {
        let source = "read_line();";
        let hover = hover_at_offset(source, 1).expect("read_line should have hover text")
            ["contents"]["value"]
            .as_str()
            .expect("read_line hover should be markdown")
            .to_string();
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

        let overlapping = Diagnostic::new("E0201", "overlapping fix", use_span)
            .with_structured_fix(
                "Overlapping Edits",
                FixApplicability::MachineApplicable,
                vec![
                    FixEdit {
                        source: DiagnosticSource::Current,
                        span: Span::new(10, 14),
                        replacement: "first".to_string(),
                    },
                    FixEdit {
                        source: DiagnosticSource::Current,
                        span: Span::new(12, 16),
                        replacement: "second".to_string(),
                    },
                ],
            );
        assert!(code_action_for_fix(uri, text, &overlapping, &overlapping.fixes[0]).is_none());
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
    fn match_binding_lsp_features_share_utf16_safe_symbol_identity() {
        let uri = "file:///match-bindings.doria";
        let text = r#"enum Result { case Value(string $text); case Missing; }
function main(): void
{
    Result $result = Result::Value("ok");
    string $label = match ($result) {
        /* 😀 */ Result::Value($payload) => $payload,
        Result::Missing => "missing",
    };
}
"#;
        let mut server = Server::default();
        server.documents.insert(
            uri.to_string(),
            Document::new(uri, text.to_string(), Some(1)),
        );

        let binding_offset = text.find("$payload").expect("payload binding");
        let binding_position = byte_offset_to_position(text, binding_offset);
        let line_start = text[..binding_offset]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        assert_ne!(
            binding_position.character as usize,
            binding_offset - line_start,
            "fixture must distinguish UTF-16 columns from byte columns"
        );
        let params = json!({
            "textDocument": { "uri": uri },
            "position": {
                "line": binding_position.line,
                "character": binding_position.character,
            }
        });

        let hover = server.hover(Some(&params)).expect("payload binding hover");
        assert!(hover["contents"]["value"]
            .as_str()
            .is_some_and(|contents| contents.contains("string $payload")));
        assert_eq!(
            hover["range"]["start"]["character"],
            binding_position.character
        );

        let references = server.references(Some(&params));
        let locations = references.as_array().expect("payload references");
        assert_eq!(locations.len(), 2);
        assert!(locations.iter().all(|location| {
            let line = location["range"]["start"]["line"].as_u64().unwrap() as u32;
            let character = location["range"]["start"]["character"].as_u64().unwrap() as u32;
            text[position_to_byte_offset(text, line, character)..].starts_with("$payload")
        }));

        let rename = server.rename(Some(&json!({
            "textDocument": { "uri": uri },
            "position": {
                "line": binding_position.line,
                "character": binding_position.character,
            },
            "newName": "value",
        })));
        let edits = rename["changes"][uri].as_array().expect("rename edits");
        assert_eq!(edits.len(), 2);
        assert!(edits.iter().all(|edit| edit["newText"] == "$value"));

        let semantic_tokens = server.semantic_tokens(Some(&json!({
            "textDocument": { "uri": uri },
        })));
        let data = semantic_tokens["data"].as_array().expect("semantic tokens");
        let mut line = 0_u32;
        let mut character = 0_u32;
        let mut found_binding = false;
        for token in data.as_chunks::<5>().0 {
            let delta_line = token[0].as_u64().unwrap() as u32;
            let delta_start = token[1].as_u64().unwrap() as u32;
            if delta_line == 0 {
                character += delta_start;
            } else {
                line += delta_line;
                character = delta_start;
            }
            if line == binding_position.line && character == binding_position.character {
                assert_eq!(token[2], "$payload".encode_utf16().count());
                assert_eq!(token[3], 0);
                found_binding = true;
            }
        }
        assert!(
            found_binding,
            "payload binding semantic token was not published"
        );
    }

    #[test]
    fn guarded_consuming_match_lsp_ranges_are_utf16_safe() {
        let uri = "file:///guarded-take.doria";
        let text = r#"class Document
{
    function __construct(string $name) {}
    function isReady(): bool { return true; }
}
enum LoadResult { case Loaded(Document $document); case Missing; }
function main(): void
{
    LoadResult $result = LoadResult::Loaded(new Document("ready"));
    Document $selected = match (take $result) {
        /* 😀 */ LoadResult::Loaded($document) if $document->isReady() => $document,
        LoadResult::Loaded($document) => $document,
        LoadResult::Missing => new Document("fallback"),
    };
    let $emoji = "😀"; LoadResult $again = $result;
}"#;
        let diagnostics = diagnostics_for_document(uri, text);
        let moved = diagnostics
            .iter()
            .find(|diagnostic| diagnostic["code"] == "E0470")
            .unwrap_or_else(|| panic!("missing consuming-match move diagnostic: {diagnostics:#?}"));
        let moved_offset = text.rfind("$result;").expect("moved source use");
        let moved_position = byte_offset_to_position(text, moved_offset);
        assert_eq!(moved["range"]["start"]["line"], moved_position.line);
        assert_eq!(
            moved["range"]["start"]["character"], moved_position.character,
            "moved-source diagnostics must use UTF-16 columns"
        );
        let mut server = Server::default();
        server.documents.insert(
            uri.to_string(),
            Document::new(uri, text.to_string(), Some(1)),
        );

        let match_offset = text.find("match (take").expect("consuming match");
        let binding_offset = text[match_offset..]
            .find("LoadResult::Loaded($document)")
            .map(|offset| match_offset + offset + "LoadResult::Loaded(".len())
            .expect("payload binding");
        let guard_offset = text[binding_offset + 1..]
            .find("$document")
            .map(|offset| binding_offset + 1 + offset)
            .expect("guard binding reference");
        let binding_position = byte_offset_to_position(text, binding_offset);
        let line_start = text[..binding_offset]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        assert_ne!(
            binding_position.character as usize,
            binding_offset - line_start,
            "fixture must distinguish UTF-16 columns from byte columns"
        );

        let params = json!({
            "textDocument": { "uri": uri },
            "position": {
                "line": binding_position.line,
                "character": binding_position.character,
            }
        });
        let hover = server.hover(Some(&params)).expect("guarded binding hover");
        assert!(hover["contents"]["value"]
            .as_str()
            .is_some_and(|value| value.contains("owns the selected Move payload")));

        let references = server.references(Some(&params));
        assert_eq!(references.as_array().expect("references").len(), 3);
        let rename = server.rename(Some(&json!({
            "textDocument": { "uri": uri },
            "position": {
                "line": byte_offset_to_position(text, guard_offset).line,
                "character": byte_offset_to_position(text, guard_offset).character,
            },
            "newName": "payload",
        })));
        let edits = rename["changes"][uri].as_array().expect("rename edits");
        assert_eq!(edits.len(), 3);
        assert!(edits.iter().all(|edit| edit["newText"] == "$payload"));

        let take_offset = text.find("take").expect("take keyword");
        let take_position = byte_offset_to_position(text, take_offset);
        let take_hover = server
            .hover(Some(&json!({
                "textDocument": { "uri": uri },
                "position": {
                    "line": take_position.line,
                    "character": take_position.character,
                }
            })))
            .expect("match take hover");
        assert!(take_hover["contents"]["value"]
            .as_str()
            .is_some_and(|value| value.contains("whole Move value")));

        let tokens = server.semantic_tokens(Some(&json!({
            "textDocument": { "uri": uri },
        })));
        let data = tokens["data"].as_array().expect("semantic tokens");
        let mut line = 0_u32;
        let mut character = 0_u32;
        let mut found = false;
        for token in data.as_chunks::<5>().0 {
            let delta_line = token[0].as_u64().unwrap() as u32;
            let delta_start = token[1].as_u64().unwrap() as u32;
            if delta_line == 0 {
                character += delta_start;
            } else {
                line += delta_line;
                character = delta_start;
            }
            if line == binding_position.line && character == binding_position.character {
                assert_eq!(token[2], "$document".encode_utf16().count());
                assert_eq!(token[3], 0);
                found = true;
            }
        }
        assert!(
            found,
            "guarded payload binding semantic token was not published"
        );
    }

    #[test]
    fn initialize_advertises_the_semantic_token_legend() {
        let provider = &initialize_result()["capabilities"]["semanticTokensProvider"];
        assert_eq!(provider["full"], true);
        assert_eq!(
            provider["legend"]["tokenTypes"],
            json!([
                "variable",
                "type",
                "enumMember",
                "function",
                "keyword",
                "namespace",
                "string"
            ])
        );
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

    #[test]
    fn checked_error_protocol_features_preserve_utf16_ranges_and_source_order() {
        let uri = "file:///checked-errors.doria";
        let text = r#"class Failure implements Error
{
    function __construct(string $message) {}
}

class Service
{
    function __construct(int $id) throws Failure {}
    function load(int $id, string $path): string throws Failure { return $path; }
    static function open(string $path): string throws Failure { return $path; }
}

function lookup(int $id, string $path): string throws Failure
{
    return $path;
}

function fail(take Failure $failure): void throws Failure
{
    echo "😀"; throw $failure;
}

function main(): void
{
    try {
        let $service = new Service(1);
        lookup(2, "free");
        $service->load(3, "method");
        Service::open("static");
        fail(new Failure("failure"));
    } catch (/* 😀 */ Failure $caught) {
        echo $caught->message;
    }
}

function relay(take Failure $failure): void throws Failure
{
    try {
        throw $failure;
    } catch (/* 😀 */ Failure $relayError) {
        echo "😀"; throw $relayError;
    } /* 😀 */ finally {
        echo "done";
    }
}
"#;
        let mut server = Server::default();
        server.documents.insert(
            uri.to_string(),
            Document::new(uri, text.to_string(), Some(1)),
        );

        let params_at = |offset: usize| {
            let position = byte_offset_to_position(text, offset);
            json!({
                "textDocument": { "uri": uri },
                "position": {
                    "line": position.line,
                    "character": position.character,
                },
            })
        };

        let catch_binding = text.find("$caught").expect("catch binding");
        let binding_position = byte_offset_to_position(text, catch_binding);
        let line_start = text[..catch_binding]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        assert_ne!(
            binding_position.character as usize,
            catch_binding - line_start,
            "fixture must distinguish UTF-16 columns from byte columns"
        );

        let hover = server
            .hover(Some(&params_at(catch_binding)))
            .expect("catch binding hover");
        assert!(hover["contents"]["value"]
            .as_str()
            .is_some_and(|value| value.contains("Readonly owned")));
        assert_eq!(
            hover["range"]["start"]["character"],
            binding_position.character
        );

        let references = server.references(Some(&json!({
            "textDocument": { "uri": uri },
            "position": {
                "line": binding_position.line,
                "character": binding_position.character,
            },
            "context": { "includeDeclaration": true },
        })));
        assert_eq!(references.as_array().map(Vec::len), Some(2));

        let rename = server.rename(Some(&json!({
            "textDocument": { "uri": uri },
            "position": {
                "line": binding_position.line,
                "character": binding_position.character,
            },
            "newName": "handled",
        })));
        let edits = rename["changes"][uri]
            .as_array()
            .expect("catch rename edits");
        assert_eq!(edits.len(), 2);
        assert!(edits.iter().all(|edit| edit["newText"] == "$handled"));
        assert_eq!(
            edits[0]["range"]["start"]["character"],
            binding_position.character
        );

        let signatures = [
            (
                "new Service(1)",
                "1",
                "function Service::__construct(int $id) throws Failure",
                0,
            ),
            (
                "lookup(2, \"free\")",
                "\"free\"",
                "function lookup(int $id, string $path): string throws Failure",
                1,
            ),
            (
                "$service->load(3, \"method\")",
                "\"method\"",
                "function Service::load(int $id, string $path): string throws Failure",
                1,
            ),
            (
                "Service::open(\"static\")",
                "\"static\"",
                "static function Service::open(string $path): string throws Failure",
                0,
            ),
        ];
        for (call, argument, expected, active_parameter) in signatures {
            let call_start = text
                .find(call)
                .unwrap_or_else(|| panic!("missing `{call}`"));
            let offset = call_start + call.find(argument).expect("argument in call");
            let signature = server.signature_help(Some(&params_at(offset)));
            assert_eq!(signature["signatures"][0]["label"], expected);
            assert_eq!(signature["activeParameter"], active_parameter);
        }

        let throw_offset = text.find("throw $failure").expect("throw statement");
        let throw_hover = server
            .hover(Some(&params_at(throw_offset)))
            .expect("throw hover");
        assert!(throw_hover["contents"]["value"]
            .as_str()
            .is_some_and(|value| value.contains("Transfers ownership")));

        for (needle, expected_text) in [
            ("throw $relayError", "Transfers ownership"),
            ("finally {", "Runs once"),
        ] {
            let offset = text.rfind(needle).expect("checked-error keyword");
            let position = byte_offset_to_position(text, offset);
            let line_start = text[..offset].rfind('\n').map_or(0, |index| index + 1);
            assert_ne!(
                position.character as usize,
                offset - line_start,
                "fixture must distinguish UTF-16 columns before `{needle}`"
            );
            let hover = server
                .hover(Some(&params_at(offset)))
                .unwrap_or_else(|| panic!("missing hover for `{needle}`"));
            assert_eq!(hover["range"]["start"]["line"], position.line);
            assert_eq!(hover["range"]["start"]["character"], position.character);
            assert!(hover["contents"]["value"]
                .as_str()
                .is_some_and(|value| value.contains(expected_text)));
        }

        let semantic = server.semantic_tokens(Some(&json!({
            "textDocument": { "uri": uri },
        })));
        let data = semantic["data"].as_array().expect("semantic token data");
        let mut line = 0_u64;
        let mut character = 0_u64;
        let mut found_binding = false;
        for token in data.as_chunks::<5>().0 {
            let delta_line = token[0].as_u64().unwrap();
            let delta_start = token[1].as_u64().unwrap();
            line += delta_line;
            character = if delta_line == 0 {
                character + delta_start
            } else {
                delta_start
            };
            if line == binding_position.line as u64
                && character == binding_position.character as u64
            {
                assert_eq!(token[2], "$caught".encode_utf16().count());
                assert_eq!(token[3], 0);
                found_binding = true;
            }
        }
        assert!(found_binding, "catch binding needs a UTF-16 semantic token");

        let throws_offset = text.find("throws Failure").expect("throws keyword");
        let throws_params = params_at(throws_offset);
        let throws_hover = server
            .hover(Some(&throws_params))
            .expect("throws keyword hover");
        assert!(throws_hover["contents"]["value"]
            .as_str()
            .is_some_and(|value| value.contains("throws checked errors")));
        let mut rename_params = throws_params;
        rename_params["newName"] = json!("renamed");
        assert_eq!(server.rename(Some(&rename_params)), Value::Null);
    }

    #[test]
    fn signature_help_survives_incomplete_parenthesis_and_comma_triggers() {
        for (call, active_parameter) in [("lookup(", 0), ("lookup(1,", 1)] {
            let uri = format!("file:///incomplete-{active_parameter}.doria");
            let text = format!(
                "function lookup(int $id, string $name): void {{}}\nfunction main(): void {{\n    {call}\n}}"
            );
            let offset = text.rfind(call).expect("incomplete call") + call.len();
            let position = byte_offset_to_position(&text, offset);
            let mut server = Server::default();
            server
                .documents
                .insert(uri.clone(), Document::new(&uri, text, Some(1)));
            let signature = server.signature_help(Some(&json!({
                "textDocument": { "uri": uri },
                "position": {
                    "line": position.line,
                    "character": position.character,
                },
            })));
            assert_eq!(
                signature["signatures"][0]["label"],
                "function lookup(int $id, string $name): void"
            );
            assert_eq!(signature["activeParameter"], active_parameter);
        }
    }

    fn stage31_server(roots: &[&str]) -> Server {
        let mut server = Server::default();
        server.configure_workspace_roots(Some(&json!({
            "workspaceFolders": roots
                .iter()
                .map(|uri| json!({ "uri": uri, "name": uri }))
                .collect::<Vec<_>>(),
        })));
        server
    }

    fn open_stage31_document(server: &mut Server, uri: &str, source: &str) {
        let context = server.compilation_context(uri);
        server.documents.insert(
            uri.to_string(),
            Document::with_context(uri, source.to_string(), Some(1), context),
        );
        server.rebuild_document_index();
    }

    fn params_at(uri: &str, source: &str, offset: usize) -> Value {
        let position = byte_offset_to_position(source, offset);
        json!({
            "textDocument": { "uri": uri },
            "position": {
                "line": position.line,
                "character": position.character,
            },
        })
    }

    #[test]
    fn stage31_open_document_index_uses_canonical_compiler_identity() {
        let root = "file:///workspace";
        let declaration_uri = "file:///workspace/model.doria";
        let consumer_uri = "file:///workspace/app.doria";
        let declaration = "namespace Acme\\Model; class User {}";
        let consumer = r#"namespace Acme\App;
use Acme\Model\User as ModelUser;
function inspect(ModelUser $user): void {}
function main(): void { let $user = new ModelUser(); inspect($user); }
"#;
        let mut server = stage31_server(&[root]);
        open_stage31_document(&mut server, declaration_uri, declaration);
        open_stage31_document(&mut server, consumer_uri, consumer);

        let declaration_offset = declaration.find("User").unwrap();
        let references = server.references(Some(&json!({
            "textDocument": { "uri": declaration_uri },
            "position": params_at(declaration_uri, declaration, declaration_offset)["position"].clone(),
            "context": { "includeDeclaration": true },
        })));
        let locations = references.as_array().expect("cross-document references");
        assert_eq!(locations.len(), 4, "{locations:#?}");
        assert!(locations
            .iter()
            .any(|location| location["uri"] == consumer_uri));

        let alias_use = consumer.rfind("ModelUser").unwrap();
        let hover = server
            .hover(Some(&params_at(consumer_uri, consumer, alias_use)))
            .expect("import alias hover");
        let markdown = hover["contents"]["value"].as_str().unwrap();
        assert!(markdown.contains("Class `Acme\\Model\\User`"));
        assert!(markdown.contains("Imported As `ModelUser`"));

        let alias_declaration = consumer.find("ModelUser").unwrap();
        let alias_rename = server.rename(Some(&json!({
            "textDocument": { "uri": consumer_uri },
            "position": params_at(consumer_uri, consumer, alias_declaration)["position"].clone(),
            "newName": "Person",
        })));
        let alias_edits = alias_rename["changes"][consumer_uri]
            .as_array()
            .expect("alias-local rename");
        assert_eq!(alias_edits.len(), 3);
        assert!(alias_edits.iter().all(|edit| edit["newText"] == "Person"));
        assert!(alias_rename["changes"].get(declaration_uri).is_none());

        let canonical_rename = server.rename(Some(&json!({
            "textDocument": { "uri": declaration_uri },
            "position": params_at(declaration_uri, declaration, declaration_offset)["position"].clone(),
            "newName": "Account",
        })));
        assert_eq!(
            canonical_rename["changes"][declaration_uri]
                .as_array()
                .map(Vec::len),
            Some(1)
        );
        let import_edits = canonical_rename["changes"][consumer_uri]
            .as_array()
            .expect("canonical import-target edit");
        assert_eq!(import_edits.len(), 1);
        assert_eq!(import_edits[0]["newText"], "Acme\\Model\\Account");
    }

    #[test]
    fn stage31_index_isolates_namespaces_workspace_roots_and_local_bindings() {
        let mut server = stage31_server(&["file:///one", "file:///two"]);
        let first_uri = "file:///one/first.doria";
        let second_uri = "file:///one/second.doria";
        let other_root_uri = "file:///two/first.doria";
        let first = "namespace First; class User {}";
        let second = "namespace Second; class User {}";
        let other_root = "namespace First; class User {}";
        open_stage31_document(&mut server, first_uri, first);
        open_stage31_document(&mut server, second_uri, second);
        open_stage31_document(&mut server, other_root_uri, other_root);

        for (uri, source) in [
            (first_uri, first),
            (second_uri, second),
            (other_root_uri, other_root),
        ] {
            let offset = source.find("User").unwrap();
            let references = server.references(Some(&params_at(uri, source, offset)));
            assert_eq!(references.as_array().map(Vec::len), Some(1));
            assert_eq!(references[0]["uri"], uri);
        }

        let local_uri = "file:///one/local.doria";
        let local = r#"namespace First;
class Value {}
function main(): void { let $Value = 1; echo $Value; let $item = new Value(); }
"#;
        open_stage31_document(&mut server, local_uri, local);
        let local_offset = local.find("$Value").unwrap();
        let local_references = server.references(Some(&params_at(local_uri, local, local_offset)));
        assert_eq!(local_references.as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn stage31_index_tracks_classes_functions_and_constants_by_canonical_identity() {
        let declarations_uri = "file:///workspace/support.doria";
        let consumer_uri = "file:///workspace/app.doria";
        let declarations = r#"namespace Acme\Support;
class Formatter {}
function formatName(): void {}
const int VERSION = 31;
"#;
        let consumer = r#"namespace Acme\App;
function main(): void {
    let $formatter = new Acme\Support\Formatter();
    Acme\Support\formatName();
    echo Acme\Support\VERSION;
}
"#;
        let mut server = stage31_server(&["file:///workspace"]);
        open_stage31_document(&mut server, declarations_uri, declarations);
        open_stage31_document(&mut server, consumer_uri, consumer);

        for name in ["Formatter", "formatName", "VERSION"] {
            let declaration = declarations.find(name).unwrap();
            let references = server.references(Some(&params_at(
                declarations_uri,
                declarations,
                declaration,
            )));
            let locations = references.as_array().expect("global references");
            assert_eq!(locations.len(), 2, "{name}: {locations:#?}");
            assert!(locations
                .iter()
                .any(|location| location["uri"] == consumer_uri));
        }
    }

    #[test]
    fn stage31_workspace_context_uses_the_longest_root_and_never_the_namespace() {
        let mut server = stage31_server(&["file:///workspace", "file:///workspace/vendor"]);
        let nested_uri = "file:///workspace/vendor/model.doria";
        let outside_uri = "file:///outside/model.doria";
        open_stage31_document(&mut server, nested_uri, "namespace Same; class Model {}");
        open_stage31_document(&mut server, outside_uri, "namespace Same; class Model {}");

        assert_eq!(
            server.documents[nested_uri]
                .analysis
                .compilation_context()
                .package,
            PackageIdentity::SyntheticTooling("lsp-workspace:file:///workspace/vendor".to_string())
        );
        assert_eq!(
            server.documents[outside_uri]
                .analysis
                .compilation_context()
                .package,
            PackageIdentity::SyntheticTooling(format!("lsp-standalone:{outside_uri}"))
        );

        let nested_name = server.documents[nested_uri]
            .analysis
            .global_symbols()
            .declarations[0]
            .id
            .qualified_name
            .clone();
        let outside_name = server.documents[outside_uri]
            .analysis
            .global_symbols()
            .declarations[0]
            .id
            .qualified_name
            .clone();
        assert_eq!(nested_name, r"Same\Model");
        assert_eq!(outside_name, nested_name);
        assert_ne!(
            server.documents[nested_uri]
                .analysis
                .compilation_context()
                .package,
            server.documents[outside_uri]
                .analysis
                .compilation_context()
                .package
        );
    }

    #[test]
    fn stage31_rename_declines_ambiguous_and_implicit_alias_edits() {
        let root = "file:///workspace";
        let mut server = stage31_server(&[root]);
        let one_uri = "file:///workspace/one.doria";
        let two_uri = "file:///workspace/two.doria";
        let import_uri = "file:///workspace/import.doria";
        let declaration = "namespace Acme; class User {}";
        let implicit = "namespace App; use Acme\\User; function inspect(User $user): void {}";
        open_stage31_document(&mut server, one_uri, declaration);
        open_stage31_document(&mut server, import_uri, implicit);

        let declaration_offset = declaration.find("User").unwrap();
        assert_eq!(
            server.rename(Some(&json!({
                "textDocument": { "uri": one_uri },
                "position": params_at(one_uri, declaration, declaration_offset)["position"].clone(),
                "newName": "Person",
            }))),
            Value::Null,
            "implicit alias preservation requires a nontrivial insertion"
        );
        let implicit_use = implicit.rfind("User").unwrap();
        assert_eq!(
            server.rename(Some(&json!({
                "textDocument": { "uri": import_uri },
                "position": params_at(import_uri, implicit, implicit_use)["position"].clone(),
                "newName": "Person",
            }))),
            Value::Null,
            "rename from an implicit alias use must not produce a partial edit"
        );

        open_stage31_document(&mut server, two_uri, declaration);
        assert_eq!(
            server.rename(Some(&json!({
                "textDocument": { "uri": one_uri },
                "position": params_at(one_uri, declaration, declaration_offset)["position"].clone(),
                "newName": "Person",
            }))),
            Value::Null,
            "duplicate canonical declarations must make rename unavailable"
        );
    }

    #[test]
    fn stage31_reindexing_removes_changed_and_closed_occurrences() {
        let uri = "file:///workspace/value.doria";
        let source = "namespace Acme; class Value {} function use(Value $value): void {}";
        let changed = "namespace Acme; class Value {} function use(int $value): void {}";
        let mut server = stage31_server(&["file:///workspace"]);
        open_stage31_document(&mut server, uri, source);
        let declaration = source.find("Value").unwrap();
        assert_eq!(
            server
                .references(Some(&params_at(uri, source, declaration)))
                .as_array()
                .map(Vec::len),
            Some(2)
        );

        let mut output = Vec::new();
        server
            .did_change(
                Some(&json!({
                    "textDocument": { "uri": uri, "version": 2 },
                    "contentChanges": [{ "text": changed }],
                })),
                &mut output,
            )
            .unwrap();
        let changed_declaration = changed.find("Value").unwrap();
        assert_eq!(
            server
                .references(Some(&params_at(uri, changed, changed_declaration)))
                .as_array()
                .map(Vec::len),
            Some(1)
        );

        let saved = "namespace Acme; class Value {} function use(Value $value): void {}";
        server
            .did_save(
                Some(&json!({
                    "textDocument": { "uri": uri },
                    "text": saved,
                })),
                &mut output,
            )
            .unwrap();
        let saved_declaration = saved.find("Value").unwrap();
        assert_eq!(
            server
                .references(Some(&params_at(uri, saved, saved_declaration)))
                .as_array()
                .map(Vec::len),
            Some(2)
        );

        server
            .did_close(
                Some(&json!({ "textDocument": { "uri": uri } })),
                &mut output,
            )
            .unwrap();
        assert!(!server.documents.contains_key(uri));
        assert!(server.document_index.completions(uri).is_empty());
    }

    #[test]
    fn stage31_hover_completion_tokens_and_boundaries_remain_compiler_owned() {
        let uri = "file:///workspace/app.doria";
        let source = r#"namespace Acme\App;
use Other\Model\User as ExternalUser;
include "generated/routes.doria";
function local(): void { List<int> $values = []; }
"#;
        let mut server = stage31_server(&["file:///workspace"]);
        open_stage31_document(&mut server, uri, source);

        let namespace = source.find("Acme").unwrap();
        let namespace_hover = server
            .hover(Some(&params_at(uri, source, namespace)))
            .expect("namespace hover");
        assert!(namespace_hover["contents"]["value"]
            .as_str()
            .is_some_and(|value| value.contains("Namespace `Acme\\App`")));

        let prelude = source.find("List").unwrap();
        let prelude_hover = server
            .hover(Some(&params_at(uri, source, prelude)))
            .expect("prelude hover");
        assert!(prelude_hover["contents"]["value"]
            .as_str()
            .is_some_and(|value| value.contains("Compiler-Known Type `List`")));

        let completion = server.completion(Some(&params_at(uri, source, source.len())));
        let labels = completion["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["label"].as_str())
            .collect::<std::collections::HashSet<_>>();
        assert!(labels.contains("ExternalUser"));
        assert!(labels.contains("local"));
        assert!(labels.contains("List"));

        let diagnostics = &server.documents[uri].analysis.diagnostics();
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0671"));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0672"));
        assert!(diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.code.starts_with('P')));

        let semantic = server.semantic_tokens(Some(&json!({
            "textDocument": { "uri": uri },
        })));
        let types = semantic["data"]
            .as_array()
            .unwrap()
            .as_chunks::<5>()
            .0
            .iter()
            .filter_map(|token| token[3].as_u64())
            .collect::<std::collections::HashSet<_>>();
        assert!(types.contains(&5), "namespace semantic token missing");
        assert!(types.contains(&6), "include-path string token missing");
    }

    #[test]
    fn stage31_cross_document_ranges_are_utf16_safe() {
        let declaration_uri = "file:///workspace/model.doria";
        let consumer_uri = "file:///workspace/use.doria";
        let declaration = "namespace Acme; class User {}";
        let consumer = "namespace App; use Acme\\User as Person; function use(): void { echo \"😀\"; let $user = new Person(); }";
        let mut server = stage31_server(&["file:///workspace"]);
        open_stage31_document(&mut server, declaration_uri, declaration);
        open_stage31_document(&mut server, consumer_uri, consumer);
        let expected =
            byte_offset_to_position(consumer, consumer.rfind("Person").unwrap()).character;
        assert_ne!(
            expected as usize,
            consumer.rfind("Person").unwrap(),
            "the emoji must make the UTF-16 column differ from the byte offset"
        );

        let alias_position = params_at(consumer_uri, consumer, consumer.rfind("Person").unwrap());
        let alias_references = server.references(Some(&alias_position));
        let runtime_use = alias_references
            .as_array()
            .unwrap()
            .iter()
            .find(|location| {
                location["range"]["start"]["character"].as_u64() == Some(expected as u64)
            })
            .expect("UTF-16 alias use range");
        assert_eq!(runtime_use["range"]["start"]["character"], expected);

        let rename = server.rename(Some(&json!({
            "textDocument": { "uri": consumer_uri },
            "position": alias_position["position"].clone(),
            "newName": "Account",
        })));
        let runtime_edit = rename["changes"][consumer_uri]
            .as_array()
            .unwrap()
            .iter()
            .find(|edit| edit["range"]["start"]["character"].as_u64() == Some(expected as u64))
            .expect("UTF-16 alias rename edit");
        assert_eq!(runtime_edit["newText"], "Account");
    }

    #[test]
    fn stage31_open_document_index_scales_structurally_and_deterministically() {
        let mut server = stage31_server(&["file:///workspace"]);
        for index in 0..128 {
            let uri = format!("file:///workspace/model-{index}.doria");
            let source = format!("namespace Package{index}; class SharedName {{}}");
            open_stage31_document(&mut server, &uri, &source);
        }

        let target_uri = "file:///workspace/model-73.doria";
        let source = &server.documents[target_uri].text;
        let offset = source.find("SharedName").unwrap();
        let references = server.references(Some(&params_at(target_uri, source, offset)));
        assert_eq!(references.as_array().map(Vec::len), Some(1));

        let first = server.document_index.completions(target_uri);
        let second = server.document_index.completions(target_uri);
        assert_eq!(first, second);
        assert_eq!(
            first
                .iter()
                .filter(|completion| completion.label.ends_with("\\SharedName"))
                .count(),
            128
        );
        assert!(!first.iter().any(|completion| {
            completion.label == "SharedName" && completion.detail.contains("Package72\\SharedName")
        }));
    }
}
