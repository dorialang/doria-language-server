use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};
use std::process::ExitCode;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use doriac::builtins::Builtin;
use doriac::diagnostics::{
    prepare_diagnostics, Diagnostic, DiagnosticFix, DiagnosticSeverity, DiagnosticSource,
    FixApplicability, FixEdit, LabelRole,
};
use doriac::incremental::{CompilationSession, IncrementalFacts};
use doriac::lexer::{Token, TokenKind};
use doriac::names::{
    CompilationContext, Edition, GlobalSymbolKind, PackageIdentity, SourceIdentity,
};
use doriac::source::{SourceId, Span};

mod analysis;
mod baton_discovery;
mod file_uri;
mod project;
mod string_surface;
mod workspace_graph;
mod workspace_index;

use analysis::{
    documentation_comment_template, AnalysisSnapshot, ImportCandidate, MissingCallable,
    MissingCallableTarget, SemanticCompletion, SemanticImport,
};
use baton_discovery::{DiscoveryRequest, ProjectDiscovery};
use file_uri::file_uri_to_path;
use project::{ProjectDocument, SourceEditPolicy};
use string_surface::{STRING_COMPANION_METHODS, STRING_PROPERTIES};
use workspace_graph::{analyze_open_graph, analyze_project_graph, GraphDocument, OpenSource};
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
    source_id: SourceId,
    source_identity: SourceIdentity,
    display_path: String,
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
        let source_identity = context.source.clone();
        let analysis = AnalysisSnapshot::analyze_with_context(uri, &text, context);
        Self {
            text,
            version,
            analysis,
            source_id: SourceId::default(),
            source_identity,
            display_path: uri.to_string(),
        }
    }

    fn from_graph(graph: GraphDocument, version: Option<i64>) -> Self {
        Self {
            text: graph.text,
            version,
            analysis: graph.analysis,
            source_id: graph.source_id,
            source_identity: graph.source_identity,
            display_path: graph.display_path,
        }
    }
}

#[derive(Debug, Clone)]
struct WorkspaceRoot {
    uri: String,
    package: String,
}

#[derive(Default)]
struct Server {
    documents: HashMap<String, Document>,
    project_documents: HashMap<String, Document>,
    workspace_roots: Vec<WorkspaceRoot>,
    workspace_sessions: HashMap<String, CompilationSession>,
    projects: HashMap<String, ProjectDocument>,
    source_edit_policies: HashMap<String, SourceEditPolicy>,
    baton_override: Option<String>,
    supports_dynamic_watching: bool,
    project_discovery: ProjectDiscovery,
    project_status_messages: HashMap<String, String>,
    project_watcher_registrations: HashMap<String, String>,
    published_diagnostics: HashMap<String, Value>,
    source_uris: HashMap<String, String>,
    incremental_facts: HashMap<String, IncrementalFacts>,
    include_edges: HashMap<String, Vec<(SourceIdentity, SourceIdentity)>>,
    document_index: OpenDocumentIndex,
}

pub fn run_stdio() -> Result<(), String> {
    let stdout = io::stdout();
    let mut writer = io::BufWriter::new(stdout.lock());
    let mut server = Server::default();

    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let stdin = io::stdin();
        let mut reader = io::BufReader::new(stdin.lock());
        loop {
            let message = read_message(&mut reader);
            let finished = matches!(message, Ok(None) | Err(_));
            if sender.send(message).is_err() || finished {
                break;
            }
        }
    });

    loop {
        match receiver.recv_timeout(Duration::from_millis(25)) {
            Ok(Ok(Some(body))) => {
                let message = serde_json::from_slice::<Value>(&body)
                    .map_err(|error| format!("failed to parse LSP message: {error}"))?;
                if !server.handle_message(message, &mut writer)? {
                    break;
                }
                writer
                    .flush()
                    .map_err(|error| format!("failed to flush LSP response: {error}"))?;
            }
            Ok(Ok(None)) => break,
            Ok(Err(error)) => return Err(error),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if server.poll_project_discovery(&mut writer)? {
                    writer.flush().map_err(|error| {
                        format!("failed to flush project discovery updates: {error}")
                    })?;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
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
        self.poll_project_discovery(writer)?;
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return Ok(true);
        };

        let id = message.get("id").cloned();
        match method {
            "initialize" => {
                self.supports_dynamic_watching = message
                    .get("params")
                    .and_then(|params| params.get("capabilities"))
                    .and_then(|capabilities| capabilities.get("workspace"))
                    .and_then(|workspace| workspace.get("didChangeWatchedFiles"))
                    .and_then(|watching| watching.get("dynamicRegistration"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                self.baton_override = message
                    .get("params")
                    .and_then(|params| params.get("initializationOptions"))
                    .and_then(|options| options.get("batonPath"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
                    .map(str::to_string);
                self.configure_workspace_roots(message.get("params"));
                self.schedule_all_project_refreshes(Duration::ZERO);
                if let Some(id) = id {
                    send_response(writer, id, initialize_result())?;
                }
            }
            "initialized" if self.supports_dynamic_watching => register_project_watchers(writer)?,
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
            "workspace/didChangeWatchedFiles" => {
                self.did_change_watched_files(message.get("params"))
            }
            "workspace/executeCommand" => {
                let command = message
                    .get("params")
                    .and_then(|params| params.get("command"))
                    .and_then(Value::as_str);
                if command == Some("doria.refreshProject") {
                    self.schedule_all_project_refreshes(Duration::ZERO);
                    if let Some(id) = id {
                        send_response(writer, id, Value::Null)?;
                    }
                } else if let Some(id) = id {
                    send_error(
                        writer,
                        id,
                        -32602,
                        "unknown Doria workspace command".to_string(),
                    )?;
                }
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
            "textDocument/onTypeFormatting" => {
                if let Some(id) = id {
                    send_response(writer, id, self.on_type_formatting(message.get("params")))?;
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
            "textDocument/definition" => {
                if let Some(id) = id {
                    send_response(writer, id, self.definition(message.get("params")))?;
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
        self.reanalyze_documents();
        self.publish_all_diagnostics(writer)
    }

    fn on_type_formatting(&self, params: Option<&Value>) -> Value {
        let Some(params) = params else {
            return json!([]);
        };
        if params.get("ch").and_then(Value::as_str) != Some("\n") {
            return json!([]);
        }
        let Some(uri) = params
            .get("textDocument")
            .and_then(|document| document.get("uri"))
            .and_then(Value::as_str)
        else {
            return json!([]);
        };
        let Some(document) = self.documents.get(uri) else {
            return json!([]);
        };
        let Some(line) = params
            .get("position")
            .and_then(|position| position.get("line"))
            .and_then(Value::as_u64)
        else {
            return json!([]);
        };
        let Some(character) = params
            .get("position")
            .and_then(|position| position.get("character"))
            .and_then(Value::as_u64)
        else {
            return json!([]);
        };
        let offset = position_to_byte_offset(&document.text, line as u32, character as u32);
        documentation_comment_edits(&document.text, offset)
            .into_iter()
            .map(|edit| {
                json!({
                    "range": span_to_range(&document.text, edit.span),
                    "newText": edit.new_text,
                })
            })
            .collect()
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
        self.reanalyze_documents();
        self.publish_all_diagnostics(writer)
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
        }

        self.reanalyze_documents();
        self.publish_all_diagnostics(writer)
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
        self.reanalyze_documents();
        self.publish_all_diagnostics(writer)
    }

    fn publish_diagnostics<W: Write>(&mut self, uri: &str, writer: &mut W) -> Result<(), String> {
        let Some(document) = self.document(uri) else {
            return Ok(());
        };

        let diagnostics = prepare_diagnostics(document.analysis.diagnostics())
            .iter()
            .map(|diagnostic| self.graph_diagnostic_to_lsp(uri, diagnostic))
            .collect::<Vec<_>>();
        let mut params = json!({
            "uri": uri,
            "diagnostics": diagnostics,
        });

        if let Some(version) = document.version {
            params["version"] = json!(version);
        }

        if self.published_diagnostics.get(uri) == Some(&params) {
            return Ok(());
        }
        send_notification(writer, "textDocument/publishDiagnostics", params.clone())?;
        self.published_diagnostics.insert(uri.to_string(), params);
        Ok(())
    }

    fn publish_all_diagnostics<W: Write>(&mut self, writer: &mut W) -> Result<(), String> {
        let mut uris = self
            .all_documents()
            .map(|(uri, _)| uri.clone())
            .collect::<Vec<_>>();
        uris.sort();
        let active = uris.iter().cloned().collect::<HashSet<_>>();
        let mut removed = self
            .published_diagnostics
            .keys()
            .filter(|uri| !active.contains(*uri))
            .cloned()
            .collect::<Vec<_>>();
        removed.sort();
        for uri in removed {
            send_notification(
                writer,
                "textDocument/publishDiagnostics",
                json!({ "uri": uri, "diagnostics": [] }),
            )?;
            self.published_diagnostics.remove(&uri);
        }
        for uri in uris {
            self.publish_diagnostics(&uri, writer)?;
        }
        Ok(())
    }

    fn graph_diagnostic_to_lsp(&self, uri: &str, diagnostic: &Diagnostic) -> Value {
        let selected = diagnostic
            .labels
            .iter()
            .enumerate()
            .find(|(_, label)| label.role == LabelRole::Primary)
            .or_else(|| diagnostic.labels.iter().enumerate().next());
        let primary_span = selected.map_or(diagnostic.span, |(_, label)| label.span);
        let primary_uri = selected
            .and_then(|(_, label)| self.diagnostic_location_uri(uri, &label.source, label.span))
            .unwrap_or_else(|| uri.to_string());
        let message = diagnostic_message(diagnostic);
        let text = self
            .document(&primary_uri)
            .map_or("", |document| document.text.as_str());
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
                    let edit_uri = self.diagnostic_location_uri(uri, &edit.source, edit.span)?;
                    if !self.source_is_editable(&edit_uri) {
                        return None;
                    }
                    let edit_text = self.document(&edit_uri)?.text.as_str();
                    Some(json!({
                        "uri": edit_uri,
                        "range": span_to_range(edit_text, edit.span),
                        "newText": edit.replacement,
                    }))
                }).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        });
        if let Some((edit_uri, edit)) = diagnostic.fixes.iter().find_map(|fix| {
            let [edit] = fix.edits.as_slice() else {
                return None;
            };
            if fix.applicability != FixApplicability::MachineApplicable {
                return None;
            }
            self.diagnostic_location_uri(uri, &edit.source, edit.span)
                .map(|edit_uri| (edit_uri, edit))
        }) {
            if let Some(edit_text) = self
                .source_is_editable(&edit_uri)
                .then(|| self.document(&edit_uri))
                .flatten()
                .map(|document| &document.text)
            {
                value["data"]["fix"] = json!({
                    "uri": edit_uri,
                    "range": span_to_range(edit_text, edit.span),
                    "newText": edit.replacement,
                });
            }
        }
        let related = diagnostic
            .labels
            .iter()
            .enumerate()
            .filter(|(index, _)| selected.map(|(selected, _)| selected) != Some(*index))
            .filter_map(|(_, label)| {
                let related_uri = self.diagnostic_location_uri(uri, &label.source, label.span)?;
                let related_text = self.document(&related_uri)?.text.as_str();
                Some(json!({
                    "location": {
                        "uri": related_uri,
                        "range": span_to_range(related_text, label.span),
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

    fn graph_code_action(
        &self,
        uri: &str,
        diagnostic: &Diagnostic,
        fix: &DiagnosticFix,
    ) -> Option<Value> {
        if fix.applicability != FixApplicability::MachineApplicable {
            return None;
        }
        let mut edits_by_uri = HashMap::<String, Vec<&FixEdit>>::new();
        for edit in &fix.edits {
            let edit_uri = self.diagnostic_location_uri(uri, &edit.source, edit.span)?;
            if !self.source_is_editable(&edit_uri) {
                return None;
            }
            self.document(&edit_uri)?;
            edits_by_uri.entry(edit_uri).or_default().push(edit);
        }
        if edits_by_uri.values().any(|edits| {
            edits_overlap(&edits.iter().map(|edit| (*edit).clone()).collect::<Vec<_>>())
        }) {
            return None;
        }
        let mut changes = serde_json::Map::new();
        for (edit_uri, edits) in edits_by_uri {
            let text = &self.document(&edit_uri)?.text;
            changes.insert(
                edit_uri,
                Value::Array(
                    edits
                        .into_iter()
                        .map(|edit| {
                            json!({
                                "range": span_to_range(text, edit.span),
                                "newText": edit.replacement,
                            })
                        })
                        .collect(),
                ),
            );
        }
        Some(json!({
            "title": fix.title,
            "kind": "quickfix",
            "diagnostics": [self.graph_diagnostic_to_lsp(uri, diagnostic)],
            "isPreferred": true,
            "edit": { "changes": changes },
        }))
    }

    fn diagnostic_location_uri(
        &self,
        current_uri: &str,
        source: &DiagnosticSource,
        span: Span,
    ) -> Option<String> {
        match source {
            DiagnosticSource::Current => Some(current_uri.to_string()),
            DiagnosticSource::Path(path) => self
                .source_uris
                .get(path)
                .cloned()
                .or_else(|| path.contains("://").then(|| path.clone()))
                .or_else(|| self.source_uri_in_current_package(current_uri, span.source)),
            DiagnosticSource::Unavailable => {
                self.source_uri_in_current_package(current_uri, span.source)
            }
        }
    }

    fn source_uri_in_current_package(
        &self,
        current_uri: &str,
        source_id: SourceId,
    ) -> Option<String> {
        let package = &self
            .document(current_uri)?
            .analysis
            .compilation_context()
            .package;
        self.all_documents().find_map(|(uri, document)| {
            (document.source_id == source_id
                && document.analysis.compilation_context().package == *package)
                .then(|| uri.clone())
        })
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
        let document = self.document(uri)?;
        let offset = position_to_byte_offset(&document.text, line, character);
        if let Some(hover) = document.analysis.namespace_hover_at_offset(offset) {
            return Some(json!({
                "contents": {
                    "kind": "markdown",
                    "value": hover.markdown,
                },
                "range": span_to_range(&document.text, hover.span),
            }));
        }
        let indexed = self.document_index.hover(uri, offset);
        if let Some(hover) = document.analysis.hover_at_offset(offset) {
            let mut markdown = hover.markdown;
            if let Some(indexed) = &indexed {
                if !markdown.contains(&indexed.markdown) {
                    markdown.push_str("\n\n---\n\n");
                    markdown.push_str(&indexed.markdown);
                }
            }
            return Some(json!({
                "contents": {
                    "kind": "markdown",
                    "value": markdown,
                },
                "range": span_to_range(&document.text, hover.span),
            }));
        }
        if let Some(hover) = indexed {
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
        if let Some(context) = attribute_completion_context(&document.text, offset) {
            let completions = match context {
                AttributeCompletionContext::Name => self.document_index.attribute_completions(&uri),
                AttributeCompletionContext::Arguments {
                    attribute,
                    positional_count,
                    supplied_names,
                    named_started,
                } => self.document_index.attribute_argument_completions(
                    &uri,
                    &attribute,
                    positional_count,
                    &supplied_names,
                    named_started,
                ),
            };
            return indexed_completion_items(completions);
        }
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
                        "documentation": candidate.documentation,
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
                        let target_document = self.document(&location.uri)?;
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

    fn definition(&self, params: Option<&Value>) -> Value {
        let Some((uri, document, offset)) = self.uri_document_and_offset(params) else {
            return Value::Null;
        };
        if let Some(location) = self.document_index.definition(&uri, offset) {
            let Some(target_document) = self.document(&location.uri) else {
                return Value::Null;
            };
            return json!({
                "uri": location.uri,
                "range": span_to_range(&target_document.text, location.span),
            });
        }
        let Some(span) = document.analysis.declaration_span_at_offset(offset) else {
            return Value::Null;
        };
        json!({
            "uri": uri,
            "range": span_to_range(&document.text, span),
        })
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
                if !self.source_is_editable(&edit.uri) {
                    return Value::Null;
                }
                let Some(target_document) = self.document(&edit.uri) else {
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
        let Some(document) = self.document(uri) else {
            return json!({ "data": [] });
        };

        let mut data = Vec::new();
        let mut previous_line = 0;
        let mut previous_start = 0;
        for (span, token_type, token_modifiers) in document.analysis.semantic_token_spans() {
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
                token_modifiers,
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
        let document = self.document(uri)?;
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
        let Some(document) = self.document(uri) else {
            return json!([]);
        };
        if !self.source_is_editable(uri) {
            return json!([]);
        }

        let mut actions = prepare_diagnostics(document.analysis.diagnostics())
            .iter()
            .flat_map(|diagnostic| {
                diagnostic
                    .fixes
                    .iter()
                    .filter_map(|fix| self.graph_code_action(uri, diagnostic, fix))
            })
            .collect::<Vec<_>>();
        if let Some(offset) = code_action_offset(params, &document.text) {
            let mut import_candidates = document
                .analysis
                .import_candidate_at_offset(&document.source_identity, offset)
                .into_iter()
                .collect::<Vec<_>>();
            if import_candidates.is_empty() {
                if let Some(reference) = document.analysis.unresolved_short_import_at_offset(
                    &document.text,
                    &document.source_identity,
                    offset,
                ) {
                    import_candidates.extend(
                        self.document_index
                            .import_candidates(uri, &reference.spelling, reference.role)
                            .into_iter()
                            .filter_map(|candidate| {
                                document.analysis.import_candidate_for_target(
                                    &document.source_identity,
                                    candidate.target,
                                    reference.span,
                                    Some(candidate.class_like),
                                )
                            }),
                    );
                }
            }
            for candidate in import_candidates {
                if let Some(action) = import_code_action(uri, document, &candidate) {
                    actions.push(action);
                }
            }
            if let Some(callable) = document.analysis.missing_callable_at_offset(offset) {
                if let Some(action) = self.missing_callable_code_action(uri, document, callable) {
                    actions.push(action);
                }
            }
        }
        Value::Array(actions)
    }

    fn missing_callable_code_action(
        &self,
        uri: &str,
        document: &Document,
        callable: &MissingCallable,
    ) -> Option<Value> {
        let (destination_uri, destination, edit_span, replacement, title) = match &callable.target {
            MissingCallableTarget::Function => {
                let edit_span = trailing_whitespace_span(&document.text);
                let replacement =
                    render_missing_function(&document.text, edit_span.start, callable);
                (
                    uri.to_string(),
                    document,
                    edit_span,
                    replacement,
                    format!("Generate function `{}`", callable.name),
                )
            }
            MissingCallableTarget::Method {
                class_name,
                is_static,
            } => {
                let (destination_uri, destination, declaration_span) =
                    self.class_declaration_document(document, class_name)?;
                if !self.source_is_editable(&destination_uri) {
                    return None;
                }
                let close = class_closing_brace(&destination.text, declaration_span)?;
                let edit_span =
                    Span::new(trailing_whitespace_start(&destination.text, close), close);
                let replacement = render_missing_method(
                    &destination.text,
                    declaration_span,
                    edit_span.start,
                    close,
                    callable,
                    *is_static,
                );
                (
                    destination_uri,
                    destination,
                    edit_span,
                    replacement,
                    format!("Generate method `{}`", callable.name),
                )
            }
        };

        let has_diagnostic = prepare_diagnostics(document.analysis.diagnostics())
            .into_iter()
            .any(|diagnostic| {
                matches!(diagnostic.code, "E0304" | "E0309")
                    && spans_overlap(diagnostic.span, callable.name_span)
            });
        if !has_diagnostic {
            return None;
        }
        let mut changes = serde_json::Map::new();
        changes.insert(
            destination_uri,
            json!([{
                "range": span_to_range(&destination.text, edit_span),
                "newText": replacement,
            }]),
        );
        Some(json!({
            "title": title,
            "kind": "refactor.rewrite",
            "edit": { "changes": changes },
        }))
    }

    fn class_declaration_document<'a>(
        &'a self,
        current: &Document,
        class_name: &str,
    ) -> Option<(String, &'a Document, Span)> {
        let imported_target = current
            .analysis
            .semantic_imports(&current.source_identity)
            .into_iter()
            .find(|import| import.alias == class_name)
            .map(|import| import.target);
        let class_name = imported_target.as_deref().unwrap_or(class_name);
        let current_package = &current.analysis.compilation_context().package;
        let mut exact = Vec::new();
        let mut short = Vec::new();
        for (uri, document) in self.all_documents() {
            if document.analysis.compilation_context().package != *current_package {
                continue;
            }
            for declaration in document
                .analysis
                .global_symbols()
                .declarations
                .iter()
                .filter(|declaration| {
                    declaration.kind == GlobalSymbolKind::Class
                        && declaration.source_identity == document.source_identity
                })
            {
                let Some(class_span) = document
                    .analysis
                    .class_declaration_span(&declaration.source_name)
                else {
                    continue;
                };
                let candidate = (uri.clone(), document, class_span);
                if declaration.qualified_name == class_name {
                    exact.push(candidate);
                } else if declaration.source_name == class_name
                    || declaration
                        .qualified_name
                        .strip_suffix(class_name)
                        .is_some_and(|prefix| prefix.ends_with('\\'))
                {
                    short.push(candidate);
                }
            }
        }
        let mut candidates = if exact.is_empty() { short } else { exact };
        (candidates.len() == 1).then(|| candidates.remove(0))
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
                package: tooling_package_name("workspace", &uri),
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
            for root in &removed {
                self.project_discovery.cancel(root);
                self.projects.remove(root);
                self.project_status_messages.remove(root);
                self.replace_project_watchers(root, None, writer)?;
            }
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
                    package: tooling_package_name("workspace", &uri),
                    uri,
                });
            }
        }
        self.workspace_roots
            .sort_by(|left, right| left.uri.cmp(&right.uri));
        self.schedule_all_project_refreshes(Duration::ZERO);
        self.reanalyze_documents();
        self.publish_all_diagnostics(writer)
    }

    fn did_change_watched_files(&mut self, params: Option<&Value>) {
        let Some(changes) = params
            .and_then(|params| params.get("changes"))
            .and_then(Value::as_array)
        else {
            return;
        };
        let mut roots = HashSet::new();
        for change in changes {
            let Some(uri) = change.get("uri").and_then(Value::as_str) else {
                continue;
            };
            if !project_file_change_requires_refresh(uri) {
                continue;
            }
            for root in &self.workspace_roots {
                let supplied_root = self.projects.get(&root.uri).is_some_and(|project| {
                    project.packages.iter().any(|package| {
                        let package_uri = file_uri::path_to_file_uri(&package.root);
                        uri_is_within(uri, &package_uri)
                    })
                });
                if uri_is_within(uri, &root.uri) || supplied_root {
                    roots.insert(root.uri.clone());
                }
            }
        }
        for root in roots {
            self.schedule_project_refresh(&root, Duration::from_millis(150));
        }
    }

    fn schedule_all_project_refreshes(&mut self, delay: Duration) {
        let roots = self
            .workspace_roots
            .iter()
            .map(|root| root.uri.clone())
            .collect::<Vec<_>>();
        for root in roots {
            self.schedule_project_refresh(&root, delay);
        }
    }

    fn schedule_project_refresh(&mut self, root_uri: &str, delay: Duration) {
        let Some(root_path) = file_uri_to_path(root_uri) else {
            return;
        };
        self.project_discovery.schedule(
            DiscoveryRequest {
                root_uri: root_uri.to_string(),
                root_path,
                baton_override: self.baton_override.clone(),
            },
            delay,
        );
    }

    fn poll_project_discovery<W: Write>(&mut self, writer: &mut W) -> Result<bool, String> {
        let updates = self.project_discovery.poll();
        if updates.is_empty() {
            return Ok(false);
        }
        let mut messages = Vec::new();
        for update in updates {
            match update.result {
                Ok(project) => {
                    self.replace_project_watchers(&update.root_uri, Some(&project), writer)?;
                    self.projects.insert(update.root_uri.clone(), project);
                    self.project_status_messages.remove(&update.root_uri);
                }
                Err(message) => {
                    self.projects.remove(&update.root_uri);
                    self.replace_project_watchers(&update.root_uri, None, writer)?;
                    let bounded = message.chars().take(500).collect::<String>();
                    let changed =
                        self.project_status_messages.get(&update.root_uri) != Some(&bounded);
                    if changed {
                        self.project_status_messages
                            .insert(update.root_uri.clone(), bounded.clone());
                        messages.push(format!(
                            "Doria project discovery unavailable for {}: {}. Open documents remain available in partial-project mode.",
                            update.root_uri, bounded
                        ));
                    }
                }
            }
        }
        self.reanalyze_documents();
        self.publish_all_diagnostics(writer)?;
        for message in messages {
            send_notification(
                writer,
                "window/logMessage",
                json!({ "type": 3, "message": message }),
            )?;
        }
        Ok(true)
    }

    fn replace_project_watchers<W: Write>(
        &mut self,
        root_uri: &str,
        project: Option<&ProjectDocument>,
        writer: &mut W,
    ) -> Result<(), String> {
        if !self.supports_dynamic_watching {
            return Ok(());
        }
        if let Some(registration_id) = self.project_watcher_registrations.remove(root_uri) {
            send_message(
                writer,
                &json!({
                    "jsonrpc": "2.0",
                    "id": format!("unregister-{registration_id}"),
                    "method": "client/unregisterCapability",
                    "params": {
                        "unregisterations": [{
                            "id": registration_id,
                            "method": "workspace/didChangeWatchedFiles"
                        }]
                    }
                }),
            )?;
        }
        let Some(project) = project else {
            return Ok(());
        };

        let registration_id = format!("doria-project-{:016x}", stable_hash(root_uri));
        let mut package_roots = project
            .packages
            .iter()
            .map(|package| file_uri::path_to_file_uri(&package.root))
            .collect::<Vec<_>>();
        package_roots.sort();
        package_roots.dedup();
        let mut watchers = package_roots
            .into_iter()
            .map(|base_uri| {
                json!({
                    "globPattern": { "baseUri": base_uri, "pattern": "**/*.doria" },
                    "kind": 7
                })
            })
            .collect::<Vec<_>>();
        let project_root = file_uri::path_to_file_uri(project.project_root());
        for pattern in [".doria/build/**", "build/.baton/**"] {
            watchers.push(json!({
                "globPattern": { "baseUri": project_root.clone(), "pattern": pattern },
                "kind": 7
            }));
        }
        send_message(
            writer,
            &json!({
                "jsonrpc": "2.0",
                "id": format!("register-{registration_id}"),
                "method": "client/registerCapability",
                "params": {
                    "registrations": [{
                        "id": registration_id.clone(),
                        "method": "workspace/didChangeWatchedFiles",
                        "registerOptions": { "watchers": watchers }
                    }]
                }
            }),
        )?;
        self.project_watcher_registrations
            .insert(root_uri.to_string(), registration_id);
        Ok(())
    }

    fn compilation_context(&self, uri: &str) -> CompilationContext {
        let (_, package, relative_path) = self.graph_location(uri);
        CompilationContext {
            edition: Edition::Doria2026,
            package: PackageIdentity::Named(package.clone()),
            source: SourceIdentity(format!("{package}:{relative_path}")),
        }
    }

    fn reanalyze_documents(&mut self) {
        self.project_documents.clear();
        self.source_edit_policies.clear();
        self.source_uris.clear();
        self.incremental_facts.clear();
        self.include_edges.clear();

        let mut active_groups = HashSet::new();
        let mut consumed = HashSet::new();
        let projects = self
            .workspace_roots
            .iter()
            .filter_map(|root| {
                self.projects
                    .get(&root.uri)
                    .cloned()
                    .map(|project| (root.uri.clone(), project))
            })
            .collect::<Vec<_>>();
        for (root_uri, project) in projects {
            let sources = self
                .documents
                .iter()
                .filter(|(uri, _)| {
                    project.packages.iter().any(|package| {
                        uri_is_within(uri, &file_uri::path_to_file_uri(&package.root))
                    })
                })
                .map(|(uri, document)| (uri.clone(), document.text.clone()))
                .collect::<Vec<_>>();
            let open_sources = sources
                .iter()
                .map(|(uri, text)| OpenSource {
                    uri,
                    relative_path: workspace_relative_source(uri, &root_uri),
                    text,
                })
                .collect::<Vec<_>>();
            for (member, plan) in project.analysis_plans() {
                let group = format!("project:{root_uri}:{member}");
                active_groups.insert(group.clone());
                let session = self.workspace_sessions.entry(group.clone()).or_default();
                let Ok(graph) = analyze_project_graph(&project, &plan, &open_sources, session)
                else {
                    continue;
                };
                self.source_uris.extend(graph.source_uris);
                self.incremental_facts
                    .insert(group.clone(), graph.incremental);
                self.include_edges.insert(group, graph.include_edges);
                for (uri, graph_document) in graph.documents {
                    self.source_edit_policies
                        .insert(uri.clone(), graph_document.edit_policy);
                    if let Some(version) = self.documents.get(&uri).map(|document| document.version)
                    {
                        consumed.insert(uri.clone());
                        self.documents
                            .insert(uri, Document::from_graph(graph_document, version));
                    } else {
                        self.project_documents
                            .insert(uri, Document::from_graph(graph_document, None));
                    }
                }
            }
        }

        let mut groups = HashMap::<String, (String, Vec<(String, String, String)>)>::new();
        for (uri, document) in &self.documents {
            if consumed.contains(uri) {
                continue;
            }
            let (group, package, relative_path) = self.graph_location(uri);
            let group = if self.projects.contains_key(&group) {
                format!("partial:{group}")
            } else {
                group
            };
            groups
                .entry(group)
                .or_insert_with(|| (package, Vec::new()))
                .1
                .push((uri.clone(), relative_path, document.text.clone()));
        }
        for (_, sources) in groups.values_mut() {
            sources.sort_by(|left, right| left.0.cmp(&right.0));
        }

        for (group, (package, sources)) in groups {
            active_groups.insert(group.clone());
            let open_sources = sources
                .iter()
                .map(|(uri, relative_path, text)| OpenSource {
                    uri,
                    relative_path: relative_path.clone(),
                    text,
                })
                .collect::<Vec<_>>();
            let session = self.workspace_sessions.entry(group.clone()).or_default();
            match analyze_open_graph(&package, &open_sources, session) {
                Ok(graph) => {
                    self.source_uris.extend(graph.source_uris);
                    self.incremental_facts
                        .insert(group.clone(), graph.incremental);
                    self.include_edges.insert(group, graph.include_edges);
                    for (uri, graph_document) in graph.documents {
                        let Some(document) = self.documents.get_mut(&uri) else {
                            continue;
                        };
                        document.analysis = graph_document.analysis;
                        document.source_id = graph_document.source_id;
                        document.source_identity = graph_document.source_identity;
                        document.display_path = graph_document.display_path;
                        self.source_edit_policies
                            .insert(uri, SourceEditPolicy::Editable);
                    }
                }
                Err(failure) => {
                    self.source_uris.extend(failure.source_uris);
                    for (uri, relative_path, _) in &sources {
                        let display_path = format!("{package}:{relative_path}");
                        let context = CompilationContext {
                            edition: Edition::Doria2026,
                            package: PackageIdentity::Named(package.clone()),
                            source: SourceIdentity(display_path.clone()),
                        };
                        let Some(document) = self.documents.get_mut(uri) else {
                            continue;
                        };
                        let mut analysis = AnalysisSnapshot::analyze_with_context(
                            &display_path,
                            &document.text,
                            context,
                        );
                        analysis.extend_diagnostics(
                            failure
                                .diagnostics
                                .iter()
                                .filter(|diagnostic| {
                                    diagnostic_targets_path(diagnostic, &display_path)
                                        || !diagnostic.labels.iter().any(|label| {
                                            matches!(label.source, DiagnosticSource::Path(_))
                                        })
                                })
                                .cloned()
                                .collect(),
                        );
                        document.analysis = analysis;
                        document.source_id = SourceId::default();
                        document.source_identity = SourceIdentity(display_path.clone());
                        document.display_path = display_path.clone();
                        self.source_uris.insert(display_path, uri.clone());
                        self.source_edit_policies
                            .insert(uri.clone(), SourceEditPolicy::Editable);
                    }
                }
            }
        }
        self.workspace_sessions
            .retain(|group, _| active_groups.contains(group));
        self.rebuild_document_index();
    }

    fn graph_location(&self, uri: &str) -> (String, String, String) {
        if let Some(root) = self
            .workspace_roots
            .iter()
            .filter(|root| uri_is_within(uri, &root.uri))
            .max_by_key(|root| root.uri.len())
        {
            return (
                root.uri.clone(),
                root.package.clone(),
                workspace_relative_source(uri, &root.uri),
            );
        }
        let package = tooling_package_name("standalone", uri);
        (
            format!("standalone:{uri}"),
            package,
            format!("document-{:016x}.doria", stable_hash(uri)),
        )
    }

    fn rebuild_document_index(&mut self) {
        self.document_index = OpenDocumentIndex::rebuild(
            self.all_documents()
                .map(|(uri, document)| (uri.as_str(), &document.analysis)),
        );
    }

    fn document(&self, uri: &str) -> Option<&Document> {
        self.documents
            .get(uri)
            .or_else(|| self.project_documents.get(uri))
    }

    fn all_documents(&self) -> impl Iterator<Item = (&String, &Document)> {
        self.documents.iter().chain(self.project_documents.iter())
    }

    fn source_is_editable(&self, uri: &str) -> bool {
        self.source_edit_policies
            .get(uri)
            .copied()
            .unwrap_or(SourceEditPolicy::Editable)
            == SourceEditPolicy::Editable
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

fn code_action_offset(params: Option<&Value>, text: &str) -> Option<usize> {
    let start = params?.get("range")?.get("start")?;
    Some(position_to_byte_offset(
        text,
        start.get("line")?.as_u64()? as u32,
        start.get("character")?.as_u64()? as u32,
    ))
}

fn import_code_action(
    uri: &str,
    document: &Document,
    candidate: &ImportCandidate,
) -> Option<Value> {
    let mut imports = document
        .analysis
        .semantic_imports(&document.source_identity);
    if candidate.requires_import {
        imports.push(SemanticImport {
            target: candidate.target.clone(),
            alias: candidate.alias.clone(),
            class_like: candidate.class_like,
        });
    }
    sort_imports(&mut imports);
    imports.dedup_by(|left, right| left.target == right.target && left.alias == right.alias);

    let import_block = render_import_block(&imports, source_newline(&document.text));
    let import_edit = import_section_edit(&document.text, &import_block)?;
    let mut edits = Vec::new();
    if document.text[candidate.reference_span.start..candidate.reference_span.end]
        != candidate.alias
    {
        edits.push(json!({
            "range": span_to_range(&document.text, candidate.reference_span),
            "newText": candidate.alias,
        }));
    }
    if document.text[import_edit.span.start..import_edit.span.end] != import_edit.new_text {
        edits.push(json!({
            "range": span_to_range(&document.text, import_edit.span),
            "newText": import_edit.new_text,
        }));
    }

    let mut changes = serde_json::Map::new();
    changes.insert(uri.to_string(), Value::Array(edits));
    Some(json!({
        "title": format!("Use import for {}", candidate.target),
        "kind": "quickfix",
        "isPreferred": true,
        "edit": {
            "changes": changes,
        },
    }))
}

fn trailing_whitespace_span(text: &str) -> Span {
    Span::new(text.trim_end_matches(char::is_whitespace).len(), text.len())
}

fn trailing_whitespace_start(text: &str, end: usize) -> usize {
    text[..end].trim_end_matches(char::is_whitespace).len()
}

fn class_closing_brace(text: &str, declaration: Span) -> Option<usize> {
    let tokens = doriac::lex_source("generated-method.doria", text).ok()?;
    let opening = tokens.iter().position(|token| {
        token.kind == TokenKind::LeftBrace && token.span.start >= declaration.start
    })?;
    let mut depth = 0_usize;
    for token in &tokens[opening..] {
        match token.kind {
            TokenKind::LeftBrace => depth += 1,
            TokenKind::RightBrace => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(token.span.start);
                }
            }
            _ => {}
        }
    }
    None
}

fn class_opening_brace(text: &str, declaration: Span) -> Option<usize> {
    doriac::lex_source("generated-method.doria", text)
        .ok()?
        .into_iter()
        .find(|token| token.kind == TokenKind::LeftBrace && token.span.start >= declaration.start)
        .map(|token| token.span.start)
}

fn render_missing_function(text: &str, insertion: usize, callable: &MissingCallable) -> String {
    let newline = source_newline(text);
    let prefix = if text[..insertion].trim().is_empty() {
        String::new()
    } else {
        format!("{newline}{newline}")
    };
    format!(
        "{prefix}{}{}",
        render_callable(callable, false, "", "    ", newline),
        newline,
    )
}

fn render_missing_method(
    text: &str,
    declaration: Span,
    insertion: usize,
    close: usize,
    callable: &MissingCallable,
    is_static: bool,
) -> String {
    let newline = source_newline(text);
    let close_indent = line_indent(text, close);
    let member_indent = inferred_member_indent(text, declaration, insertion, close_indent);
    let indent_unit = member_indent
        .strip_prefix(close_indent)
        .filter(|indent| !indent.is_empty())
        .unwrap_or("    ");
    let body_start = class_opening_brace(text, declaration)
        .map(|brace| brace + 1)
        .unwrap_or(declaration.start);
    let body_has_members = !text[body_start..insertion].trim().is_empty();
    let separator = if body_has_members {
        format!("{newline}{newline}")
    } else {
        newline.to_string()
    };
    format!(
        "{separator}{}{newline}{close_indent}",
        render_callable(callable, is_static, &member_indent, indent_unit, newline),
    )
}

fn render_callable(
    callable: &MissingCallable,
    is_static: bool,
    indent: &str,
    indent_unit: &str,
    newline: &str,
) -> String {
    let parameters = callable
        .parameters
        .iter()
        .map(|parameter| format!("{} ${}", parameter.ty, parameter.name))
        .collect::<Vec<_>>()
        .join(", ");
    let static_modifier = if is_static { "static " } else { "" };
    let body = if callable.return_type == "void" {
        String::new()
    } else {
        format!("{indent}{indent_unit}return null;{newline}")
    };
    format!(
        "{indent}{static_modifier}function {}({parameters}): {}{newline}{indent}{{{newline}{body}{indent}}}",
        callable.name, callable.return_type,
    )
}

fn line_indent(text: &str, offset: usize) -> &str {
    let line_start = text[..offset].rfind('\n').map_or(0, |index| index + 1);
    let line = &text[line_start..offset];
    &line[..line
        .find(|character: char| !matches!(character, ' ' | '\t'))
        .unwrap_or(line.len())]
}

fn inferred_member_indent<'a>(
    text: &'a str,
    declaration: Span,
    insertion: usize,
    close_indent: &'a str,
) -> String {
    text[declaration.start..insertion]
        .lines()
        .filter_map(|line| {
            let content = line.trim();
            if content.is_empty() || matches!(content, "{" | "}") {
                return None;
            }
            let indent = &line[..line
                .find(|character: char| !matches!(character, ' ' | '\t'))
                .unwrap_or(line.len())];
            (indent.len() > close_indent.len() && indent.starts_with(close_indent))
                .then(|| indent.to_string())
        })
        .min_by_key(String::len)
        .unwrap_or_else(|| format!("{close_indent}    "))
}

fn sort_imports(imports: &mut [SemanticImport]) {
    imports.sort_by(|left, right| {
        (!left.class_like)
            .cmp(&!right.class_like)
            .then_with(|| left.target.to_lowercase().cmp(&right.target.to_lowercase()))
            .then_with(|| left.target.cmp(&right.target))
            .then_with(|| left.alias.cmp(&right.alias))
    });
}

fn render_import_block(imports: &[SemanticImport], newline: &str) -> String {
    let mut lines = Vec::new();
    let mut previous_class_like = imports.first().map(|import| import.class_like);
    for import in imports {
        if previous_class_like.is_some_and(|class_like| class_like != import.class_like) {
            lines.push(String::new());
        }
        let final_segment = import.target.rsplit('\\').next().unwrap_or(&import.target);
        if import.alias == final_segment {
            lines.push(format!("use {};", import.target));
        } else {
            lines.push(format!("use {} as {};", import.target, import.alias));
        }
        previous_class_like = Some(import.class_like);
    }
    lines.join(newline)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImportSectionEdit {
    span: Span,
    new_text: String,
}

fn import_section_edit(text: &str, import_block: &str) -> Option<ImportSectionEdit> {
    let program = doriac::parse_source("import-action.doria", text.to_string()).ok()?;
    let newline = source_newline(text);
    if let (Some(first), Some(last)) = (program.imports.first(), program.imports.last()) {
        if program.imports.windows(2).any(|pair| {
            text[pair[0].span.end..pair[1].span.start]
                .chars()
                .any(|character| !character.is_whitespace())
        }) {
            return None;
        }
        let end = following_whitespace_end(text, last.span.end);
        return Some(ImportSectionEdit {
            span: Span::new(first.span.start, end),
            new_text: format!("{import_block}{newline}{newline}"),
        });
    }

    let start = program
        .namespace
        .as_ref()
        .map_or(0, |namespace| namespace.semicolon_span.end);
    let end = following_whitespace_end(text, start);
    let prefix = if program.namespace.is_some() {
        format!("{newline}{newline}")
    } else {
        String::new()
    };
    Some(ImportSectionEdit {
        span: Span::new(start, end),
        new_text: format!("{prefix}{import_block}{newline}{newline}"),
    })
}

fn following_whitespace_end(text: &str, start: usize) -> usize {
    text[start..]
        .char_indices()
        .find_map(|(relative, character)| (!character.is_whitespace()).then_some(start + relative))
        .unwrap_or(text.len())
}

fn source_newline(text: &str) -> &str {
    if text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DocumentationCommentEdit {
    span: Span,
    new_text: String,
}

fn documentation_comment_edits(text: &str, offset: usize) -> Vec<DocumentationCommentEdit> {
    let Some(trigger) = documentation_comment_trigger(text, offset) else {
        return Vec::new();
    };
    let tags = if trigger.documentation {
        documentation_comment_template(text, trigger.comment_start, offset)
            .map(|template| template.tags)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };

    if trigger.before_enter {
        let replacement_end = same_line_comment_end(text, offset).unwrap_or(offset);
        let mut lines = vec![trigger.opener.to_string(), format!("{} * ", trigger.indent)];
        if !tags.is_empty() {
            lines.push(format!("{} *", trigger.indent));
            lines.extend(tags.iter().map(|tag| format!("{} * {tag}", trigger.indent)));
        }
        lines.push(format!("{} */", trigger.indent));
        return vec![DocumentationCommentEdit {
            span: Span::new(trigger.comment_start, replacement_end),
            new_text: lines.join(newline),
        }];
    }

    let mut edits = Vec::new();
    if !trigger.current_line_has_star {
        edits.push(DocumentationCommentEdit {
            span: Span::new(offset, offset),
            new_text: " * ".to_string(),
        });
    }
    if let Some(close_start) = following_comment_close(text, offset) {
        if !tags.is_empty() {
            let mut inserted = vec![format!("{} *", trigger.indent)];
            inserted.extend(tags.iter().map(|tag| format!("{} * {tag}", trigger.indent)));
            edits.push(DocumentationCommentEdit {
                span: Span::new(close_start, close_start),
                new_text: format!("{}{newline}", inserted.join(newline)),
            });
        }
    } else {
        let mut inserted = Vec::new();
        if !tags.is_empty() {
            inserted.push(format!("{} *", trigger.indent));
            inserted.extend(tags.iter().map(|tag| format!("{} * {tag}", trigger.indent)));
        }
        inserted.push(format!("{} */", trigger.indent));
        edits.push(DocumentationCommentEdit {
            span: Span::new(offset, offset),
            new_text: format!("{newline}{}", inserted.join(newline)),
        });
    }
    edits
}

#[derive(Debug)]
struct DocumentationCommentTrigger<'a> {
    comment_start: usize,
    opener: &'a str,
    indent: &'a str,
    documentation: bool,
    before_enter: bool,
    current_line_has_star: bool,
}

fn documentation_comment_trigger(
    text: &str,
    offset: usize,
) -> Option<DocumentationCommentTrigger<'_>> {
    if offset > text.len() || !text.is_char_boundary(offset) {
        return None;
    }
    let current_start = text[..offset].rfind('\n').map_or(0, |index| index + 1);
    let current_prefix = &text[current_start..offset];
    if let Some((indent, opener)) = comment_opener(current_prefix) {
        return Some(DocumentationCommentTrigger {
            comment_start: current_start + indent.len(),
            opener,
            indent,
            documentation: opener == "/**",
            before_enter: true,
            current_line_has_star: false,
        });
    }

    let previous_end = current_start.checked_sub(1)?;
    let previous_start = text[..previous_end]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let previous_line = text[previous_start..previous_end].trim_end_matches('\r');
    let (indent, opener) = comment_opener(previous_line)?;
    let current_trimmed = current_prefix.trim();
    if !current_trimmed.is_empty() && current_trimmed != "*" {
        return None;
    }
    Some(DocumentationCommentTrigger {
        comment_start: previous_start + indent.len(),
        opener,
        indent,
        documentation: opener == "/**",
        before_enter: false,
        current_line_has_star: current_trimmed == "*",
    })
}

fn comment_opener(line: &str) -> Option<(&str, &str)> {
    let indent_end = line.find(|character: char| !matches!(character, ' ' | '\t'))?;
    let indent = &line[..indent_end];
    let opener = line[indent_end..].trim_end();
    matches!(opener, "/*" | "/**").then_some((indent, opener))
}

fn same_line_comment_end(text: &str, offset: usize) -> Option<usize> {
    let line_end = text[offset..]
        .find(['\r', '\n'])
        .map_or(text.len(), |relative| offset + relative);
    let suffix = &text[offset..line_end];
    let leading = suffix.len() - suffix.trim_start().len();
    let after_whitespace = &suffix[leading..];
    after_whitespace
        .strip_prefix("*/")
        .filter(|rest| rest.trim().is_empty())
        .map(|_| offset + leading + 2)
}

fn following_comment_close(text: &str, offset: usize) -> Option<usize> {
    let mut line_start = text[offset..]
        .find('\n')
        .map(|relative| offset + relative + 1)?;
    while line_start <= text.len() {
        let line_end = text[line_start..]
            .find('\n')
            .map_or(text.len(), |relative| line_start + relative);
        let line = text[line_start..line_end].trim_end_matches('\r');
        if line.trim().is_empty() {
            line_start = line_end.saturating_add(1);
            continue;
        }
        return (line.trim() == "*/").then_some(line_start);
    }
    None
}

fn stable_hash(value: &str) -> u64 {
    value.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

fn tooling_package_name(kind: &str, identity: &str) -> String {
    format!("tooling/{kind}-{:016x}", stable_hash(identity))
}

fn workspace_relative_source(uri: &str, root: &str) -> String {
    uri.strip_prefix(root)
        .unwrap_or(uri)
        .trim_start_matches('/')
        .split(['?', '#'])
        .next()
        .filter(|path| !path.is_empty())
        .unwrap_or("document.doria")
        .to_string()
}

fn diagnostic_targets_path(diagnostic: &Diagnostic, path: &str) -> bool {
    diagnostic
        .labels
        .iter()
        .any(|label| matches!(&label.source, DiagnosticSource::Path(source) if source == path))
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
            "definitionProvider": true,
            "signatureHelpProvider": {
                "triggerCharacters": ["(", ","]
            },
            "documentOnTypeFormattingProvider": {
                "firstTriggerCharacter": "\n"
            },
            "referencesProvider": true,
            "renameProvider": true,
            "semanticTokensProvider": {
                "legend": {
                    "tokenTypes": ["variable", "type", "enumMember", "function", "keyword", "namespace", "string"],
                    "tokenModifiers": ["declaration"]
                },
                "full": true
            },
            "codeActionProvider": true,
            "executeCommandProvider": {
                "commands": ["doria.refreshProject"]
            },
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
            "documentation": builtin_documentation_with_effects(builtin),
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

fn indexed_completion_items(completions: Vec<workspace_index::IndexedCompletion>) -> Value {
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum AttributeCompletionContext {
    Name,
    Arguments {
        attribute: String,
        positional_count: usize,
        supplied_names: HashSet<String>,
        named_started: bool,
    },
}

fn attribute_completion_context(text: &str, offset: usize) -> Option<AttributeCompletionContext> {
    let prefix = text.get(..offset)?;
    let source = doriac::source::SourceFile::with_id(
        SourceId::default(),
        "<attribute-completion>",
        prefix.to_string(),
    );
    let tokens = doriac::lexer::Lexer::new(&source).lex().ok()?;
    let open = tokens
        .iter()
        .rposition(|token| token.kind == TokenKind::AttributeOpen)?;
    let mut application_start = open + 1;
    let mut paren_depth = 0_usize;
    for (index, token) in tokens.iter().enumerate().skip(application_start) {
        match token.kind {
            TokenKind::LeftParen => paren_depth += 1,
            TokenKind::RightParen => paren_depth = paren_depth.saturating_sub(1),
            TokenKind::RightBracket if paren_depth == 0 => return None,
            TokenKind::Comma if paren_depth == 0 => application_start = index + 1,
            _ => {}
        }
    }

    let application_tokens = tokens
        .get(application_start..)
        .unwrap_or_default()
        .iter()
        .filter(|token| token.kind != TokenKind::Eof)
        .collect::<Vec<_>>();
    let Some(left_paren) = application_tokens
        .iter()
        .position(|token| token.kind == TokenKind::LeftParen)
    else {
        return Some(AttributeCompletionContext::Name);
    };
    let name_tokens = &application_tokens[..left_paren];
    let first = name_tokens.first()?.span.start;
    let last = name_tokens.last()?.span.end;
    let attribute = prefix.get(first..last)?.to_string();

    let mut positional_count = 0_usize;
    let mut supplied_names = HashSet::new();
    let mut named_started = false;
    let mut argument_tokens = Vec::<&Token>::new();
    let mut depth = 1_usize;
    for token in application_tokens.iter().skip(left_paren + 1) {
        match token.kind {
            TokenKind::LeftParen | TokenKind::LeftBracket | TokenKind::LeftBrace => depth += 1,
            TokenKind::RightParen | TokenKind::RightBracket | TokenKind::RightBrace => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
            TokenKind::Comma if depth == 1 => {
                record_attribute_argument(
                    &argument_tokens,
                    &mut positional_count,
                    &mut supplied_names,
                    &mut named_started,
                    true,
                );
                argument_tokens.clear();
            }
            _ => argument_tokens.push(token),
        }
    }
    record_attribute_argument(
        &argument_tokens,
        &mut positional_count,
        &mut supplied_names,
        &mut named_started,
        false,
    );
    if !matches!(
        argument_tokens.as_slice(),
        [] | [Token {
            kind: TokenKind::Identifier(_),
            ..
        }]
    ) {
        return None;
    }
    Some(AttributeCompletionContext::Arguments {
        attribute,
        positional_count,
        supplied_names,
        named_started,
    })
}

fn record_attribute_argument(
    tokens: &[&Token],
    positional_count: &mut usize,
    supplied_names: &mut HashSet<String>,
    named_started: &mut bool,
    complete: bool,
) {
    if tokens.is_empty() {
        return;
    }
    if let [Token {
        kind: TokenKind::Identifier(name),
        ..
    }, Token {
        kind: TokenKind::Colon,
        ..
    }, ..] = tokens
    {
        supplied_names.insert(name.clone());
        *named_started = true;
    } else if (complete
        || !matches!(
            tokens,
            [Token {
                kind: TokenKind::Identifier(_),
                ..
            }]
        ))
        && !*named_started
    {
        *positional_count += 1;
    }
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
        builtin_documentation_with_effects(builtin)
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

fn builtin_documentation_with_effects(builtin: Builtin) -> String {
    let mut documentation = builtin_documentation(builtin).to_string();
    let ambient = builtin.ambient_error_types();
    if !ambient.is_empty() {
        documentation.push_str("\n\n**Ambient I/O:**\n\n");
        documentation.push_str(
            &ambient
                .iter()
                .map(|effect| format!("- `{effect}`"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        documentation.push_str(
            "\n\nThese checked runtime outcomes propagate without requiring source `throws`.",
        );
    }
    documentation
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
            "`echo value;` writes the displayed value. `Doria\\Std\\Io\\IoError` is an ambient checked runtime effect and does not require source `throws`.",
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
    let message = diagnostic_message(diagnostic);

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

fn diagnostic_message(diagnostic: &Diagnostic) -> String {
    let primary = diagnostic
        .labels
        .iter()
        .find(|label| label.role == LabelRole::Primary)
        .or_else(|| diagnostic.labels.first());
    let mut message = diagnostic.title.clone();
    if let Some(primary) = primary.filter(|label| !label.message.is_empty()) {
        if diagnostic_text_is_equivalent(&message, &primary.message) {
            message.clone_from(&primary.message);
        } else {
            message.push('\n');
            message.push_str(&primary.message);
        }
    }
    if let Some(explanation) = &diagnostic.explanation {
        message.push_str("\n\nWhy: ");
        message.push_str(explanation);
    }
    for help in &diagnostic.helps {
        message.push_str("\nHelp: ");
        message.push_str(help);
    }
    message
}

fn diagnostic_text_is_equivalent(left: &str, right: &str) -> bool {
    fn normalized(text: &str) -> String {
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .trim_end_matches(['.', ':', ';'])
            .to_lowercase()
    }

    normalized(left) == normalized(right)
}

fn lsp_severity(severity: DiagnosticSeverity) -> u8 {
    match severity {
        DiagnosticSeverity::Error => 1,
        DiagnosticSeverity::Warning => 2,
        DiagnosticSeverity::Note => 3,
    }
}

fn spans_overlap(left: Span, right: Span) -> bool {
    left.start < right.end && right.start < left.end
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

fn register_project_watchers<W: Write>(writer: &mut W) -> Result<(), String> {
    send_message(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "id": "doria-project-watchers",
            "method": "client/registerCapability",
            "params": {
                "registrations": [{
                    "id": "doria-project-files",
                    "method": "workspace/didChangeWatchedFiles",
                    "registerOptions": {
                        "watchers": [
                            { "globPattern": "**/Baton.toml", "kind": 7 },
                            { "globPattern": "**/Baton.lock", "kind": 7 },
                            { "globPattern": "**/*.doria", "kind": 7 },
                            { "globPattern": "**/.doria/build/**", "kind": 7 },
                            { "globPattern": "**/build/.baton/**", "kind": 7 }
                        ]
                    }
                }]
            }
        }),
    )
}

fn project_file_change_requires_refresh(uri: &str) -> bool {
    uri.ends_with("/Baton.toml")
        || uri.ends_with("/Baton.lock")
        || uri.contains("/.doria/build/")
        || uri.contains("/build/.baton/")
        || uri.ends_with(".doria")
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
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use doriac::diagnostics::{FixEdit, LabelRole};

    #[test]
    fn initialize_reports_canonical_toolchain_calver() {
        assert_eq!(
            initialize_result()["serverInfo"]["version"],
            doriac::TOOLCHAIN_VERSION
        );
        assert_eq!(doriac::TOOLCHAIN_VERSION, "2026.03.1-canary");
        assert_eq!(
            initialize_result()["capabilities"]["documentOnTypeFormattingProvider"]
                ["firstTriggerCharacter"],
            "\n",
        );
    }

    #[test]
    fn documentation_comment_generation_uses_compiler_authored_signatures() {
        let source = "/**\nfunction transform<T>(take Payload $payload, writable List<int> $items): ?Result<T> throws FirstError, SecondError {}\n";
        let edits = documentation_comment_edits(source, 3);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].span, Span::new(0, 3));
        assert_eq!(
            edits[0].new_text,
            "/**\n * \n *\n * @template T\n * @param take Payload $payload\n * @param writable List<int> $items\n * @return ?Result<T>\n * @throws FirstError\n * @throws SecondError\n */",
        );
    }

    #[test]
    fn documentation_comment_generation_inserts_tags_after_the_summary_caret() {
        let source = "/**\n * \n */\nfunction greet(string $name): string {}\n";
        let offset = source.find(" * \n").expect("summary line") + 3;
        let edits = documentation_comment_edits(source, offset);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].span.start, source.find(" */").unwrap());
        assert_eq!(
            edits[0].new_text,
            " *\n * @param string $name\n * @return string\n",
        );
    }

    #[test]
    fn ordinary_block_comment_generation_never_invents_documentation_tags() {
        let source = "    /*\n    function greet(string $name): string {}\n";
        let edits = documentation_comment_edits(source, 6);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "/*\n     * \n     */");
    }

    #[test]
    fn on_type_formatting_serves_documentation_edits_for_open_documents() {
        let uri = "file:///documentation.doria";
        let source = "/**\n * \n */\nfunction greet(string $name): string {}\n";
        let offset = source.find(" * \n").expect("summary line") + 3;
        let position = byte_offset_to_position(source, offset);
        let mut server = Server::default();
        server.documents.insert(
            uri.to_string(),
            Document::new(uri, source.to_string(), Some(1)),
        );

        let response = server.on_type_formatting(Some(&json!({
            "textDocument": { "uri": uri },
            "position": { "line": position.line, "character": position.character },
            "ch": "\n",
            "options": { "tabSize": 4, "insertSpaces": true },
        })));

        assert_eq!(response.as_array().map(Vec::len), Some(1));
        assert_eq!(
            response[0]["newText"],
            " *\n * @param string $name\n * @return string\n",
        );
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
        assert!(hover.contains("ambient"));
        assert!(hover.contains("does not require source `throws`"));
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
        assert!(hover.contains("**Ambient I/O:**"));
        assert!(hover.contains(doriac::compiler_known_io::IO_ERROR));
        assert!(hover.contains(doriac::compiler_known_io::INVALID_UTF8_ERROR));
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
    fn equivalent_diagnostic_title_and_primary_label_are_not_repeated() {
        let diagnostic = Diagnostic::new(
            "E0201",
            "cannot write through readonly object path",
            Span::new(0, 5),
        )
        .with_title("Cannot Write Through Readonly Object Path")
        .with_label(
            DiagnosticSource::Current,
            Span::new(0, 5),
            LabelRole::Primary,
            "cannot write through readonly object path",
        );

        let lsp = diagnostic_to_lsp("file:///main.doria", "$this", &diagnostic);
        assert_eq!(lsp["message"], "cannot write through readonly object path");
        let mut server = Server::default();
        server.documents.insert(
            "file:///main.doria".to_string(),
            Document::new("file:///main.doria", "$this".to_string(), Some(1)),
        );
        let graph_lsp = server.graph_diagnostic_to_lsp("file:///main.doria", &diagnostic);
        assert_eq!(
            graph_lsp["message"],
            "cannot write through readonly object path"
        );
        assert!(diagnostic_text_is_equivalent(
            "Cannot Write Through Readonly Object Path:",
            "cannot   write through readonly object path."
        ));
        assert!(!diagnostic_text_is_equivalent(
            "Cannot Write Through Readonly Object Path",
            "the receiver is readonly"
        ));
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
                source_id: SourceId::default(),
                source_identity: SourceIdentity(uri.to_string()),
                display_path: uri.to_string(),
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
        let initialize = initialize_result();
        assert_eq!(initialize["capabilities"]["definitionProvider"], true);
        let provider = &initialize["capabilities"]["semanticTokensProvider"];
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
        assert_eq!(provider["legend"]["tokenModifiers"], json!(["declaration"]));
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
        server.reanalyze_documents();
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

    fn semantic_token_records(value: &Value) -> Vec<(u32, u32, u32, u32, u32)> {
        let mut line = 0;
        let mut start = 0;
        let data = value["data"].as_array().expect("semantic token data");
        let (tokens, remainder) = data.as_slice().as_chunks::<5>();
        assert!(remainder.is_empty(), "incomplete semantic token record");
        tokens
            .iter()
            .map(|token| {
                let delta_line = token[0].as_u64().expect("delta line") as u32;
                let delta_start = token[1].as_u64().expect("delta start") as u32;
                line += delta_line;
                start = if delta_line == 0 {
                    start + delta_start
                } else {
                    delta_start
                };
                (
                    line,
                    start,
                    token[2].as_u64().expect("length") as u32,
                    token[3].as_u64().expect("token type") as u32,
                    token[4].as_u64().expect("token modifiers") as u32,
                )
            })
            .collect()
    }

    fn code_action_params_at(uri: &str, source: &str, offset: usize) -> Value {
        let position = params_at(uri, source, offset)["position"].clone();
        json!({
            "textDocument": { "uri": uri },
            "range": {
                "start": position,
                "end": position,
            },
            "context": { "diagnostics": [] },
        })
    }

    #[test]
    fn unresolved_calls_preserve_compiler_error_severity() {
        let source = r#"class Application
{
    function run(): void
    {
        $this->start();
    }
}

function main(): void
{
    launch();
}
"#;
        let diagnostics = diagnostics_for_document("file:///missing-callables.doria", source);
        for code in ["E0304", "E0309"] {
            let diagnostic = diagnostics
                .iter()
                .find(|diagnostic| diagnostic["code"] == code)
                .unwrap_or_else(|| panic!("missing {code}: {diagnostics:#?}"));
            assert_eq!(diagnostic["severity"], 1);
        }
        let compiler = doriac::check_source("missing-callables.doria", source)
            .expect_err("unresolved calls remain compiler errors");
        assert!(compiler.iter().any(|diagnostic| {
            diagnostic.code == "E0304" && diagnostic.severity == DiagnosticSeverity::Error
        }));
        assert!(compiler.iter().any(|diagnostic| {
            diagnostic.code == "E0309" && diagnostic.severity == DiagnosticSeverity::Error
        }));
    }

    #[test]
    fn missing_callable_actions_generate_typed_function_and_method_stubs() {
        let uri = "file:///workspace/app.doria";
        let source = r#"class Application
{
    function run(string $name): void
    {
        $this->start($name, retries: 2);
    }
}

function main(): void
{
    launch("Doria");
}
"#;
        let mut server = stage31_server(&["file:///workspace"]);
        open_stage31_document(&mut server, uri, source);

        let method_actions = server.code_actions(Some(&code_action_params_at(
            uri,
            source,
            source.find("start").unwrap(),
        )));
        let method = method_actions
            .as_array()
            .and_then(|actions| {
                actions
                    .iter()
                    .find(|action| action["title"] == "Generate method `start`")
            })
            .unwrap_or_else(|| panic!("missing method action: {method_actions:#?}"));
        assert_eq!(method["kind"], "refactor.rewrite");
        assert!(method.get("diagnostics").is_none());
        assert!(method.get("isPreferred").is_none());
        let method_text = method["edit"]["changes"][uri][0]["newText"]
            .as_str()
            .expect("generated method text");
        assert!(
            method_text.contains("function start(string $name, int $retries): void"),
            "{method_text:?}"
        );
        assert!(method_text.contains("\n    {\n    }\n"), "{method_text:?}");

        let function_actions = server.code_actions(Some(&code_action_params_at(
            uri,
            source,
            source.find("launch").unwrap(),
        )));
        let function = function_actions
            .as_array()
            .and_then(|actions| {
                actions
                    .iter()
                    .find(|action| action["title"] == "Generate function `launch`")
            })
            .unwrap_or_else(|| panic!("missing function action: {function_actions:#?}"));
        assert_eq!(function["kind"], "refactor.rewrite");
        assert!(function.get("diagnostics").is_none());
        assert!(function.get("isPreferred").is_none());
        let function_text = function["edit"]["changes"][uri][0]["newText"]
            .as_str()
            .expect("generated function text");
        assert!(function_text.contains("function launch(string $arg1): void"));
    }

    #[test]
    fn missing_method_action_edits_the_open_declaring_class() {
        let worker_uri = "file:///workspace/Worker.doria";
        let app_uri = "file:///workspace/app.doria";
        let worker = "namespace Acme\\Model;\nclass Worker\n{\n}\n";
        let app = r#"namespace Acme\App;
use Acme\Model\Worker as Runner;

function main(): void
{
    let $worker = new Runner();
    $worker->process(42);
    Runner::reset();
}
"#;
        let mut server = stage31_server(&["file:///workspace"]);
        open_stage31_document(&mut server, worker_uri, worker);
        open_stage31_document(&mut server, app_uri, app);

        for (name, expected) in [
            ("process", "function process(int $arg1): void"),
            ("reset", "static function reset(): void"),
        ] {
            let actions = server.code_actions(Some(&code_action_params_at(
                app_uri,
                app,
                app.find(name).unwrap(),
            )));
            let action = actions
                .as_array()
                .and_then(|actions| {
                    actions
                        .iter()
                        .find(|action| action["title"] == format!("Generate method `{name}`"))
                })
                .unwrap_or_else(|| panic!("missing {name} action: {actions:#?}"));
            assert!(action["edit"]["changes"].get(app_uri).is_none());
            let generated = action["edit"]["changes"][worker_uri][0]["newText"]
                .as_str()
                .expect("generated method text");
            assert!(generated.contains(expected), "{generated:?}");
        }
    }

    #[test]
    fn expression_call_generation_uses_a_valid_conservative_return() {
        let uri = "file:///workspace/value.doria";
        let source = "function main(): void\n{\n    let $value = calculate(1);\n}\n";
        let mut server = stage31_server(&["file:///workspace"]);
        open_stage31_document(&mut server, uri, source);
        let actions = server.code_actions(Some(&code_action_params_at(
            uri,
            source,
            source.find("calculate").unwrap(),
        )));
        let action = actions
            .as_array()
            .and_then(|actions| {
                actions
                    .iter()
                    .find(|action| action["title"] == "Generate function `calculate`")
            })
            .unwrap_or_else(|| panic!("missing function action: {actions:#?}"));
        let edit = &action["edit"]["changes"][uri][0];
        let start = position_to_byte_offset(
            source,
            edit["range"]["start"]["line"].as_u64().unwrap() as u32,
            edit["range"]["start"]["character"].as_u64().unwrap() as u32,
        );
        let end = position_to_byte_offset(
            source,
            edit["range"]["end"]["line"].as_u64().unwrap() as u32,
            edit["range"]["end"]["character"].as_u64().unwrap() as u32,
        );
        let mut generated = source.to_string();
        generated.replace_range(start..end, edit["newText"].as_str().unwrap());
        assert!(generated.contains("function calculate(int $arg1): mixed"));
        assert!(generated.contains("return null;"));
        doriac::check_source("value.doria", &generated)
            .unwrap_or_else(|diagnostics| panic!("generated source must check: {diagnostics:#?}"));
    }

    fn apply_workspace_edits(source: &str, action: &Value, uri: &str) -> String {
        let mut edits = action["edit"]["changes"][uri]
            .as_array()
            .expect("workspace edits")
            .iter()
            .map(|edit| {
                let range = &edit["range"];
                let start = position_to_byte_offset(
                    source,
                    range["start"]["line"].as_u64().unwrap() as u32,
                    range["start"]["character"].as_u64().unwrap() as u32,
                );
                let end = position_to_byte_offset(
                    source,
                    range["end"]["line"].as_u64().unwrap() as u32,
                    range["end"]["character"].as_u64().unwrap() as u32,
                );
                (start, end, edit["newText"].as_str().unwrap().to_string())
            })
            .collect::<Vec<_>>();
        edits.sort_by_key(|(start, end, _)| std::cmp::Reverse((*start, *end)));

        let mut result = source.to_string();
        for (start, end, replacement) in edits {
            result.replace_range(start..end, &replacement);
        }
        result
    }

    #[test]
    fn import_code_action_shortens_qualified_names_and_groups_sorted_imports() {
        let uri = "file:///workspace/app.doria";
        let source = r#"namespace App\Core;

use Vendor\zeta;
use Vendor\Zebra;

function run(Vendor\Render\WindowStyles $styles, Zebra $z): void
{
    zeta();
}
"#;
        let offset = source.find("Vendor\\Render\\WindowStyles").unwrap() + 8;
        let mut server = Server::default();
        server.documents.insert(
            uri.to_string(),
            Document::new(uri, source.to_string(), Some(1)),
        );

        let actions = server.code_actions(Some(&code_action_params_at(uri, source, offset)));
        let action = actions
            .as_array()
            .unwrap()
            .iter()
            .find(|action| action["title"] == "Use import for Vendor\\Render\\WindowStyles")
            .expect("qualified type import action");
        let updated = apply_workspace_edits(source, action, uri);

        assert_eq!(
            updated,
            r#"namespace App\Core;

use Vendor\Render\WindowStyles;
use Vendor\Zebra;

use Vendor\zeta;

function run(WindowStyles $styles, Zebra $z): void
{
    zeta();
}
"#
        );
    }

    #[test]
    fn import_code_action_reuses_an_existing_alias_without_duplicating_it() {
        let uri = "file:///workspace/app.doria";
        let source = "namespace App;\n\nuse Vendor\\Render\\WindowStyles as Styles;\n\nfunction run(Vendor\\Render\\WindowStyles $styles): void {}\n";
        let offset = source.rfind("Vendor\\Render\\WindowStyles").unwrap() + 8;
        let mut server = Server::default();
        server.documents.insert(
            uri.to_string(),
            Document::new(uri, source.to_string(), Some(1)),
        );

        let actions = server.code_actions(Some(&code_action_params_at(uri, source, offset)));
        let action = actions
            .as_array()
            .unwrap()
            .iter()
            .find(|action| action["title"] == "Use import for Vendor\\Render\\WindowStyles")
            .expect("existing alias action");
        let updated = apply_workspace_edits(source, action, uri);

        assert_eq!(
            updated.matches("use Vendor\\Render\\WindowStyles").count(),
            1
        );
        assert!(updated.contains("function run(Styles $styles): void"));
    }

    #[test]
    fn import_code_action_resolves_short_names_from_compiler_workspace_symbols() {
        let declaration_uri = "file:///workspace/src/Rendering/WindowStyles.doria";
        let consumer_uri = "file:///workspace/src/Core/Application.doria";
        let declaration = "namespace Doria\\Tui\\Rendering;\nclass WindowStyles {}\n";
        let consumer = "namespace Doria\\Tui\\Core;\n\nclass Application\n{\n    function loadStyle(WindowStyles $styles): void {}\n}\n";
        let mut server = stage31_server(&["file:///workspace"]);
        open_stage31_document(&mut server, declaration_uri, declaration);
        open_stage31_document(&mut server, consumer_uri, consumer);

        let offset = consumer.find("WindowStyles").unwrap() + 3;

        let actions =
            server.code_actions(Some(&code_action_params_at(consumer_uri, consumer, offset)));
        let action = actions
            .as_array()
            .unwrap()
            .iter()
            .find(|action| action["title"] == "Use import for Doria\\Tui\\Rendering\\WindowStyles")
            .expect("short-name workspace import action");
        let updated = apply_workspace_edits(consumer, action, consumer_uri);

        assert!(updated.contains("use Doria\\Tui\\Rendering\\WindowStyles;"));
        assert_eq!(updated.matches("WindowStyles $styles").count(), 1);
    }

    #[test]
    fn import_code_action_exposes_ambiguous_short_names_as_explicit_choices() {
        let consumer_uri = "file:///workspace/src/App.doria";
        let consumer = "namespace App;\nfunction render(Widget $widget): void {}\n";
        let mut server = stage31_server(&["file:///workspace"]);
        open_stage31_document(
            &mut server,
            "file:///workspace/src/First.doria",
            "namespace First; class Widget {}",
        );
        open_stage31_document(
            &mut server,
            "file:///workspace/src/Second.doria",
            "namespace Second; class Widget {}",
        );
        open_stage31_document(&mut server, consumer_uri, consumer);
        let offset = consumer.find("Widget").unwrap() + 2;

        let actions =
            server.code_actions(Some(&code_action_params_at(consumer_uri, consumer, offset)));
        let titles = actions
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|action| action["title"].as_str())
            .filter(|title| title.starts_with("Use import for "))
            .collect::<Vec<_>>();
        assert_eq!(
            titles,
            vec![
                "Use import for First\\Widget",
                "Use import for Second\\Widget"
            ]
        );
    }

    #[test]
    fn import_code_action_declines_alias_collisions_and_interleaved_comments() {
        let collision_uri = "file:///workspace/collision.doria";
        let collision = "namespace App;\n\nuse Other\\WindowStyles;\n\nfunction run(Vendor\\WindowStyles $styles, WindowStyles $other): void {}\n";
        let collision_offset = collision.find("Vendor\\WindowStyles").unwrap() + 4;
        let mut server = Server::default();
        server.documents.insert(
            collision_uri.to_string(),
            Document::new(collision_uri, collision.to_string(), Some(1)),
        );
        let collision_actions = server.code_actions(Some(&code_action_params_at(
            collision_uri,
            collision,
            collision_offset,
        )));
        assert!(!collision_actions
            .as_array()
            .unwrap()
            .iter()
            .any(|action| { action["title"] == "Use import for Vendor\\WindowStyles" }));

        let comment_uri = "file:///workspace/comment.doria";
        let comment = "use Vendor\\Zebra;\n// Keep this grouping note.\nuse Vendor\\zeta;\n\nfunction run(Vendor\\WindowStyles $styles): void {}\n";
        let comment_offset = comment.find("Vendor\\WindowStyles").unwrap() + 4;
        server.documents.insert(
            comment_uri.to_string(),
            Document::new(comment_uri, comment.to_string(), Some(1)),
        );
        let comment_actions = server.code_actions(Some(&code_action_params_at(
            comment_uri,
            comment,
            comment_offset,
        )));
        assert!(!comment_actions
            .as_array()
            .unwrap()
            .iter()
            .any(|action| { action["title"] == "Use import for Vendor\\WindowStyles" }));
    }

    #[test]
    fn import_section_edit_preserves_crlf_line_endings() {
        let source = "namespace App;\r\n\r\nfunction run(): void {}\r\n";
        let edit =
            import_section_edit(source, "use Vendor\\Window;").expect("namespace import insertion");
        assert_eq!(edit.new_text, "\r\n\r\nuse Vendor\\Window;\r\n\r\n");
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
        let alias_declaration = consumer.find("ModelUser").unwrap();
        let alias_definition =
            server.definition(Some(&params_at(consumer_uri, consumer, alias_use)));
        assert_eq!(alias_definition["uri"], consumer_uri);
        let alias_definition_start = &alias_definition["range"]["start"];
        assert_eq!(
            position_to_byte_offset(
                consumer,
                alias_definition_start["line"].as_u64().unwrap() as u32,
                alias_definition_start["character"].as_u64().unwrap() as u32,
            ),
            alias_declaration,
        );
        let canonical_definition =
            server.definition(Some(&params_at(consumer_uri, consumer, alias_declaration)));
        assert_eq!(canonical_definition["uri"], declaration_uri);

        let hover = server
            .hover(Some(&params_at(consumer_uri, consumer, alias_use)))
            .expect("import alias hover");
        let markdown = hover["contents"]["value"].as_str().unwrap();
        assert!(markdown.contains("Class `Acme\\Model\\User`"));
        assert!(markdown.contains("Imported As `ModelUser`"));

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
    fn stage31_index_augments_semantic_function_hover_without_replacing_it() {
        let uri = "file:///workspace/app.doria";
        let source = r#"namespace Acme;
/** Returns the supplied value. */
function inspect(int $value): int { return $value; }
function main(): void { inspect(42); }
"#;
        let mut server = stage31_server(&["file:///workspace"]);
        open_stage31_document(&mut server, uri, source);

        let call = source.find("inspect").unwrap();
        let hover = server
            .hover(Some(&params_at(uri, source, call)))
            .expect("indexed function hover");
        let markdown = hover["contents"]["value"].as_str().unwrap();
        assert!(markdown.contains("function inspect(int $value): int"));
        assert!(markdown.contains("Returns the supplied value."));
        assert!(markdown.contains("Function `Acme\\inspect`"));
    }

    #[test]
    fn stage31_internal_declaration_hover_preserves_access_and_signature() {
        let uri = "file:///workspace/app.doria";
        let source = r#"namespace Acme;
internal function helper(int $value): int { return $value; }
function main(): void { echo helper(31); }
"#;
        let mut server = stage31_server(&["file:///workspace"]);
        open_stage31_document(&mut server, uri, source);

        let hover = server
            .hover(Some(&params_at(
                uri,
                source,
                source.find("helper").unwrap(),
            )))
            .expect("internal declaration hover");
        let markdown = hover["contents"]["value"].as_str().unwrap();
        assert!(markdown.contains("internal function helper(int $value): int"));
        assert!(markdown.contains("Function `Acme\\helper`"));
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
            PackageIdentity::Named(tooling_package_name(
                "workspace",
                "file:///workspace/vendor"
            ))
        );
        assert_eq!(
            server.documents[outside_uri]
                .analysis
                .compilation_context()
                .package,
            PackageIdentity::Named(tooling_package_name("standalone", outside_uri))
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
            .all(|diagnostic| !matches!(diagnostic.code, "E0671" | "E0672")));
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
    fn stage31_slice2_graph_drives_cross_file_definition_and_global_kinds() {
        let root = "file:///workspace";
        let declarations_uri = "file:///workspace/domain.doria";
        let consumer_uri = "file:///workspace/app.doria";
        let declarations = r#"namespace Acme;
class User {}
enum State { case Ready; }
function makeUser(): User { return new User(); }
const int VERSION = 31;
"#;
        let consumer = r#"namespace Acme;
function acceptsUser(User $user): void {}
function inspect(): void {
    let $user = makeUser();
    State $state = State::Ready;
    echo VERSION;
    acceptsUser(1);
}
"#;
        let mut server = stage31_server(&[root]);
        open_stage31_document(&mut server, declarations_uri, declarations);
        open_stage31_document(&mut server, consumer_uri, consumer);

        assert!(server.documents[consumer_uri]
            .analysis
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.code != "E0681"));
        assert!(server.documents[consumer_uri]
            .analysis
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == "E0408"));
        for name in ["makeUser", "State", "VERSION"] {
            let offset = consumer.find(name).unwrap();
            let definition = server.definition(Some(&params_at(consumer_uri, consumer, offset)));
            assert_eq!(
                definition["uri"], declarations_uri,
                "{name}: {definition:#?}"
            );
        }
        let completion =
            server.completion(Some(&params_at(consumer_uri, consumer, consumer.len())));
        let labels = completion["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["label"].as_str())
            .collect::<std::collections::HashSet<_>>();
        for name in ["User", "State", "makeUser", "VERSION"] {
            assert!(
                labels.contains(name),
                "missing same-namespace completion {name}"
            );
        }
    }

    #[test]
    fn navigation_and_rename_follow_resolved_members_across_files() {
        let declarations_uri = "file:///workspace/domain.doria";
        let consumer_uri = "file:///workspace/app.doria";
        let declarations = r#"namespace Acme;
class Model
{
    int $identifier = 1;
    const int VERSION = 1;
    function render(): string { return "ready"; }
}
enum State { case Ready; }
"#;
        let consumer = r#"namespace Acme;
function inspect(Model $model): void
{
    echo $model->identifier;
    echo $model->render();
    echo Model::VERSION;
    State $state = State::Ready;
}
"#;
        let mut server = stage31_server(&["file:///workspace"]);
        open_stage31_document(&mut server, declarations_uri, declarations);
        open_stage31_document(&mut server, consumer_uri, consumer);
        assert!(server.documents[consumer_uri]
            .analysis
            .diagnostics()
            .is_empty());

        for (name, declaration_name) in [
            ("identifier", "$identifier"),
            ("render", "render"),
            ("VERSION", "VERSION"),
            ("Ready", "Ready"),
        ] {
            let offset = consumer.find(name).unwrap();
            let definition = server.definition(Some(&params_at(consumer_uri, consumer, offset)));
            assert_eq!(
                definition["uri"], declarations_uri,
                "{name}: {definition:#?}"
            );
            let expected =
                byte_offset_to_position(declarations, declarations.find(declaration_name).unwrap());
            assert_eq!(definition["range"]["start"]["line"], expected.line);
            assert_eq!(
                definition["range"]["start"]["character"],
                expected.character
            );
        }

        let render = consumer.find("render").unwrap();
        let rename = server.rename(Some(&json!({
            "textDocument": { "uri": consumer_uri },
            "position": params_at(consumer_uri, consumer, render)["position"].clone(),
            "newName": "display",
        })));
        assert_eq!(
            rename["changes"][declarations_uri].as_array().map(Vec::len),
            Some(1)
        );
        assert_eq!(
            rename["changes"][consumer_uri].as_array().map(Vec::len),
            Some(1)
        );
        assert!(rename["changes"][declarations_uri]
            .as_array()
            .unwrap()
            .iter()
            .chain(rename["changes"][consumer_uri].as_array().unwrap())
            .all(|edit| edit["newText"] == "display"));
    }

    #[test]
    fn definition_falls_back_to_same_document_local_bindings() {
        let uri = "file:///workspace/local.doria";
        let source = r#"function main(): void
{
    int $value = 1;
    echo $value;
}
"#;
        let mut server = Server::default();
        server.documents.insert(
            uri.to_string(),
            Document::new(uri, source.to_string(), Some(1)),
        );
        server.rebuild_document_index();

        let reference = source.rfind("$value").unwrap();
        let definition = server.definition(Some(&params_at(uri, source, reference)));
        let expected = byte_offset_to_position(source, source.find("$value").unwrap());
        assert_eq!(definition["uri"], uri);
        assert_eq!(definition["range"]["start"]["line"], expected.line);
        assert_eq!(
            definition["range"]["start"]["character"],
            expected.character
        );
    }

    #[test]
    fn stage31_slice2_duplicate_diagnostics_publish_once_with_related_source() {
        let first_uri = "file:///workspace/first.doria";
        let second_uri = "file:///workspace/second.doria";
        let first = "namespace Acme; /* 😀 */ class User {}";
        let second = "namespace Acme; class User {}";
        let mut server = stage31_server(&["file:///workspace"]);
        open_stage31_document(&mut server, first_uri, first);
        open_stage31_document(&mut server, second_uri, second);

        let published = [first_uri, second_uri]
            .into_iter()
            .filter_map(|uri| {
                server.documents[uri]
                    .analysis
                    .diagnostics()
                    .iter()
                    .find(|diagnostic| diagnostic.code == "E0684")
                    .map(|diagnostic| (uri, diagnostic))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            published.len(),
            1,
            "a compiler diagnostic belongs only to its primary source"
        );
        let (primary_uri, duplicate) = published[0];
        let related_uri = if primary_uri == first_uri {
            second_uri
        } else {
            first_uri
        };
        let lsp = server.graph_diagnostic_to_lsp(primary_uri, duplicate);
        assert_eq!(lsp["code"], "E0684");
        assert!(lsp["relatedInformation"]
            .as_array()
            .is_some_and(|related| related
                .iter()
                .any(|item| { item["location"]["uri"] == related_uri })));
        assert!(server.documents[related_uri]
            .analysis
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.code != "E0684"));

        let offset = first.rfind("User").unwrap();
        assert_eq!(
            server.rename(Some(&json!({
                "textDocument": { "uri": first_uri },
                "position": params_at(first_uri, first, offset)["position"].clone(),
                "newName": "Account",
            }))),
            Value::Null
        );
    }

    #[test]
    fn stage31_slice2_session_invalidates_changes_and_removes_closed_sources() {
        let root = "file:///workspace";
        let model_uri = "file:///workspace/model.doria";
        let consumer_uri = "file:///workspace/app.doria";
        let model = "namespace Acme; class User {}";
        let renamed = "namespace Acme; class Account {}";
        let consumer = "namespace Acme; function inspect(User $user): void {}";
        let mut server = stage31_server(&[root]);
        open_stage31_document(&mut server, model_uri, model);
        open_stage31_document(&mut server, consumer_uri, consumer);
        assert!(server.documents[consumer_uri]
            .analysis
            .diagnostics()
            .is_empty());

        let mut output = Vec::new();
        server
            .did_change(
                Some(&json!({
                    "textDocument": { "uri": model_uri, "version": 2 },
                    "contentChanges": [{ "text": renamed }],
                })),
                &mut output,
            )
            .unwrap();
        let facts = &server.incremental_facts[root];
        assert!(facts
            .changed_sources
            .iter()
            .any(|source| source.ends_with(":model.doria")));
        let consumer_diagnostics = server.documents[consumer_uri].analysis.diagnostics();
        assert!(
            !consumer_diagnostics.is_empty(),
            "changing a cross-file declaration must invalidate its consumer"
        );
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains(model_uri));
        assert!(output.contains(consumer_uri));

        let updated_consumer = "namespace Acme; function inspect(Account $user): void {}";
        let mut clear_output = Vec::new();
        server
            .did_change(
                Some(&json!({
                    "textDocument": { "uri": consumer_uri, "version": 2 },
                    "contentChanges": [{ "text": updated_consumer }],
                })),
                &mut clear_output,
            )
            .unwrap();
        assert!(server.documents[consumer_uri]
            .analysis
            .diagnostics()
            .is_empty());
        assert!(String::from_utf8(clear_output)
            .unwrap()
            .contains("\"diagnostics\":[]"));

        let mut close_output = Vec::new();
        server
            .did_close(
                Some(&json!({ "textDocument": { "uri": model_uri } })),
                &mut close_output,
            )
            .unwrap();
        assert!(!server.documents.contains_key(model_uri));
        assert!(server.incremental_facts[root]
            .removed_sources
            .iter()
            .any(|source| source.ends_with(":model.doria")));
        let use_offset = updated_consumer.find("Account").unwrap();
        assert_eq!(
            server.definition(Some(
                &params_at(consumer_uri, updated_consumer, use_offset,)
            )),
            Value::Null,
            "closed declarations must not remain as phantom graph facts"
        );
    }

    #[test]
    fn stage31_slice2_open_graph_tracks_includes_without_scanning_or_baton() {
        let root = "file:///workspace";
        let entry_uri = "file:///workspace/app.doria";
        let included_uri = "file:///workspace/generated/routes.doria";
        let entry = "namespace Acme; include \"generated/routes.doria\"; function use(Routes $routes): void {}";
        let included = "namespace Acme; class Routes {}";
        let mut server = stage31_server(&[root]);
        open_stage31_document(&mut server, included_uri, included);
        open_stage31_document(&mut server, entry_uri, entry);

        assert_eq!(server.include_edges[root].len(), 1);
        assert!(server.documents[entry_uri]
            .analysis
            .diagnostics()
            .iter()
            .all(|diagnostic| !matches!(diagnostic.code, "E0671" | "E0672")));
        assert!(server.source_uris.values().any(|uri| uri == included_uri));
    }

    #[test]
    fn stage31_slice2_cross_file_checked_effects_and_closure_types_are_semantic() {
        let support_uri = "file:///workspace/support.doria";
        let consumer_uri = "file:///workspace/app.doria";
        let support = r#"namespace Acme;
class Failure implements Error
{
    function message(): string { return "failure"; }
}
function fail(): void throws Failure { throw new Failure(); }
function apply(function(int): int $callback): int { return $callback(1); }
"#;
        let consumer = r#"namespace Acme;
function run(): int
{
    int $offset = 2;
    int $value = apply(fn(int $item) with ($offset) => $item + $offset);
    fail();
    return $value;
}
"#;
        let mut server = stage31_server(&["file:///workspace"]);
        open_stage31_document(&mut server, support_uri, support);
        open_stage31_document(&mut server, consumer_uri, consumer);

        let diagnostics = server.documents[consumer_uri].analysis.diagnostics();
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0631"));
        assert!(diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "E0681"));
        let capture = consumer.rfind("$offset").unwrap();
        let hover = server
            .hover(Some(&params_at(consumer_uri, consumer, capture)))
            .expect("cross-file closure capture hover");
        assert!(hover["contents"]["value"]
            .as_str()
            .is_some_and(|markdown| markdown.contains("int $offset")));
    }

    #[test]
    fn stage31_slice2_partial_inputs_remain_compiler_facts_not_index_guesses() {
        let uri = "file:///workspace/app.doria";
        let source = "namespace Acme; function inspect(Other\\Missing $value): void {}";
        let mut server = stage31_server(&["file:///workspace"]);
        open_stage31_document(&mut server, uri, source);

        let diagnostic = server.documents[uri]
            .analysis
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code == "E0681")
            .expect("partial graph compiler-input fact");
        assert_eq!(
            diagnostic.kind,
            doriac::diagnostics::DiagnosticKind::CompilerInput
        );
        let offset = source.find("Other").unwrap();
        assert_eq!(
            server.definition(Some(&params_at(uri, source, offset))),
            Value::Null
        );
        assert_eq!(
            server.rename(Some(&json!({
                "textDocument": { "uri": uri },
                "position": params_at(uri, source, offset)["position"].clone(),
                "newName": "Known",
            }))),
            Value::Null
        );
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
            127
        );
        assert!(first
            .iter()
            .any(|completion| completion.label == "SharedName"));
        assert!(!first.iter().any(|completion| {
            completion.label == "SharedName" && completion.detail.contains("Package72\\SharedName")
        }));
    }

    #[test]
    fn stage32_attribute_completion_context_uses_compiler_lexing_boundaries() {
        assert_eq!(
            attribute_completion_context("#[Rou", 5),
            Some(AttributeCompletionContext::Name)
        );
        assert!(attribute_completion_context("# [Route]", 9).is_none());
        assert!(attribute_completion_context("# #[Route]", 10).is_none());
        assert!(attribute_completion_context("\"#[Route]\"", 10).is_none());
        assert!(attribute_completion_context("#[Route]", 8).is_none());
        let value_prefix = "#[Route(method: HttpMethod::";
        assert!(attribute_completion_context(value_prefix, value_prefix.len()).is_none());
        let context = attribute_completion_context(
            "#[Route(path: \"/posts\", ",
            "#[Route(path: \"/posts\", ".len(),
        )
        .expect("attribute arguments remain active");
        let AttributeCompletionContext::Arguments {
            attribute,
            supplied_names,
            named_started,
            ..
        } = context
        else {
            panic!("expected argument completion context");
        };
        assert_eq!(attribute, "Route");
        assert_eq!(supplied_names, HashSet::from(["path".to_string()]));
        assert!(named_started);

        let uri = "file:///workspace/new.doria";
        let source = "#[";
        let mut server = stage31_server(&["file:///workspace"]);
        open_stage31_document(&mut server, uri, source);
        let completion = server.completion(Some(&params_at(uri, source, source.len())));
        let labels = completion["items"]
            .as_array()
            .expect("compiler-known completion in incomplete source")
            .iter()
            .filter_map(|item| item["label"].as_str())
            .collect::<HashSet<_>>();
        assert_eq!(labels, HashSet::from(["Attribute", "PHPExport", "Test"]));
    }

    #[test]
    fn stage32_completion_is_scoped_to_visible_attribute_schemas_and_parameters() {
        let root = "file:///workspace";
        let schema_uri = "file:///workspace/route.doria";
        let app_uri = "file:///workspace/app.doria";
        let schema = r#"namespace Acme\Metadata;
#[Attribute]
class Route
{
    function __construct(string $path, int $status = 200) {}
}
class Ordinary {}
"#;
        let app = r#"namespace Acme\App;
use Acme\Metadata\Route;
#[Route(path: "/posts", status: 201)]
function main(): void {}
"#;
        let mut server = stage31_server(&[root]);
        open_stage31_document(&mut server, schema_uri, schema);
        open_stage31_document(&mut server, app_uri, app);

        let name_offset = app.find("Route(path").unwrap() + "Ro".len();
        let names = server.completion(Some(&params_at(app_uri, app, name_offset)))["items"]
            .as_array()
            .expect("attribute completion items")
            .iter()
            .filter_map(|item| item["label"].as_str().map(str::to_string))
            .collect::<HashSet<_>>();
        assert!(names.contains("Attribute"));
        assert!(names.contains("Test"));
        assert!(names.contains("PHPExport"));
        assert!(names.contains("Route"));
        assert!(!names.contains("Ordinary"));

        let first_argument = app.find("Route(").unwrap() + "Route(".len();
        let arguments = server.completion(Some(&params_at(app_uri, app, first_argument)))["items"]
            .as_array()
            .expect("attribute argument completion")
            .iter()
            .filter_map(|item| item["label"].as_str().map(str::to_string))
            .collect::<HashSet<_>>();
        assert_eq!(
            arguments,
            HashSet::from(["path".to_string(), "status".to_string()])
        );

        let second_argument = app.find(", status").unwrap() + 2;
        let remaining = server.completion(Some(&params_at(app_uri, app, second_argument)))["items"]
            .as_array()
            .expect("remaining attribute arguments")
            .iter()
            .filter_map(|item| item["label"].as_str().map(str::to_string))
            .collect::<HashSet<_>>();
        assert_eq!(remaining, HashSet::from(["status".to_string()]));
    }

    #[test]
    fn stage32_attribute_values_retain_expression_completion() {
        let uri = "file:///workspace/route.doria";
        let source = r#"enum HttpMethod { case Get; case Post; }
#[Attribute]
class Route
{
    function __construct(HttpMethod $method) {}
}
#[Route(method: HttpMethod::Post)]
function main(): void {}
"#;
        let mut server = stage31_server(&["file:///workspace"]);
        open_stage31_document(&mut server, uri, source);

        let offset = source.find("HttpMethod::Post").unwrap() + "HttpMethod::".len();
        let completion = server.completion(Some(&params_at(uri, source, offset)));
        let labels = completion["items"]
            .as_array()
            .expect("enum case completions inside attribute value")
            .iter()
            .filter_map(|item| item["label"].as_str().map(str::to_string))
            .collect::<HashSet<_>>();
        assert_eq!(
            labels,
            HashSet::from(["Get".to_string(), "Post".to_string()])
        );
    }

    #[test]
    fn stage32_hovers_navigation_references_and_rename_share_compiler_identities() {
        let root = "file:///workspace";
        let schema_uri = "file:///workspace/route.doria";
        let app_uri = "file:///workspace/app.doria";
        let schema = r#"namespace Acme\Metadata;
#[Attribute]
class Route
{
    function __construct(string $path) { echo $path; }
}
"#;
        let app = r#"namespace Acme\App;
use Acme\Metadata\Route as WebRoute;
#[WebRoute(path: "/posts")]
#[Test]
#[PHPExport]
function main(): void {}
"#;
        let mut server = stage31_server(&[root]);
        open_stage31_document(&mut server, schema_uri, schema);
        open_stage31_document(&mut server, app_uri, app);

        let application = app.find("WebRoute(path").unwrap();
        let hover = server
            .hover(Some(&params_at(app_uri, app, application)))
            .expect("attribute application hover");
        let markdown = hover["contents"]["value"].as_str().unwrap();
        assert!(markdown.contains("Attribute `Acme\\Metadata\\Route`"));
        assert!(markdown.contains("`path`: `string` = `\"/posts\"`"));
        assert!(markdown.contains("No Runtime Reflection"));

        let definition = server.definition(Some(&params_at(app_uri, app, application)));
        assert_eq!(definition["uri"], schema_uri);
        let references = server.references(Some(&json!({
            "textDocument": { "uri": app_uri },
            "position": params_at(app_uri, app, application)["position"].clone(),
            "context": { "includeDeclaration": true },
        })));
        assert_eq!(references.as_array().map(Vec::len), Some(3));

        let class_rename = server.rename(Some(&json!({
            "textDocument": { "uri": schema_uri },
            "position": params_at(schema_uri, schema, schema.find("Route").unwrap())["position"].clone(),
            "newName": "Endpoint",
        })));
        let class_edits = class_rename["changes"]
            .as_object()
            .expect("class rename edits");
        assert!(class_edits.contains_key(schema_uri));
        assert!(class_edits.contains_key(app_uri));

        let named_argument = app.find("path:").unwrap();
        let parameter_rename = server.rename(Some(&json!({
            "textDocument": { "uri": app_uri },
            "position": params_at(app_uri, app, named_argument)["position"].clone(),
            "newName": "uri",
        })));
        let parameter_edits = parameter_rename["changes"]
            .as_object()
            .expect("parameter rename edits");
        assert_eq!(
            parameter_edits[schema_uri].as_array().map(Vec::len),
            Some(2)
        );
        assert_eq!(parameter_edits[app_uri].as_array().map(Vec::len), Some(1));
        assert!(parameter_edits[schema_uri]
            .as_array()
            .unwrap()
            .iter()
            .all(|edit| edit["newText"] == "$uri"));
        assert_eq!(parameter_edits[app_uri][0]["newText"], "uri");

        for marker in ["Test", "PHPExport"] {
            let offset = app.find(marker).unwrap();
            let marker_hover = server
                .hover(Some(&params_at(app_uri, app, offset)))
                .expect("compiler-known marker hover");
            assert!(marker_hover["contents"]["value"]
                .as_str()
                .is_some_and(|value| value.contains("Metadata Only")));
            assert_eq!(
                server.rename(Some(&json!({
                    "textDocument": { "uri": app_uri },
                    "position": params_at(app_uri, app, offset)["position"].clone(),
                    "newName": "Known",
                }))),
                Value::Null
            );
        }
    }

    #[test]
    fn stage32_semantic_tokens_and_diagnostics_remain_compiler_owned_and_incremental() {
        let uri = "file:///workspace/attributes.doria";
        let valid = "#[Test]\nfunction main(): void {}\n# comment\n";
        let invalid = "function main(): void {}\n#[Missing]\nfunction extra(): void {}\n";
        let mut server = stage31_server(&["file:///workspace"]);
        open_stage31_document(&mut server, uri, valid);
        assert!(server.documents[uri].analysis.diagnostics().is_empty());
        let semantic = server.semantic_tokens(Some(&json!({
            "textDocument": { "uri": uri },
        })));
        let data = semantic["data"].as_array().expect("semantic tokens");
        assert!(
            data.len() >= 15,
            "attribute delimiter, name, and target tokens"
        );

        let mut output = Vec::new();
        server
            .did_change(
                Some(&json!({
                    "textDocument": { "uri": uri, "version": 2 },
                    "contentChanges": [{ "text": invalid }],
                })),
                &mut output,
            )
            .unwrap();
        assert!(server.documents[uri]
            .analysis
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unknown attribute")));

        let mut clear_output = Vec::new();
        server
            .did_change(
                Some(&json!({
                    "textDocument": { "uri": uri, "version": 3 },
                    "contentChanges": [{ "text": valid }],
                })),
                &mut clear_output,
            )
            .unwrap();
        assert!(server.documents[uri].analysis.diagnostics().is_empty());
        assert!(String::from_utf8(clear_output)
            .unwrap()
            .contains("\"diagnostics\":[]"));
    }

    #[test]
    fn stage33_project_graph_indexes_unopened_sources_and_overlays_open_buffers() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("doria-lsp-stage33-{nonce}"));
        fs::create_dir_all(root.join("src")).unwrap();
        let declaration_path = root.join("src/Person.doria");
        let usage_path = root.join("src/main.doria");
        let broken_path = root.join("src/Broken.doria");
        fs::write(&declaration_path, "class Person {}\n").unwrap();
        fs::write(&usage_path, "function accept(Missing $value): void {}\n").unwrap();
        fs::write(&broken_path, "UnknownThing $value = 1;\n").unwrap();

        let root_uri = file_uri::path_to_file_uri(&root);
        let declaration_uri = file_uri::path_to_file_uri(&declaration_path.canonicalize().unwrap());
        let usage_uri = file_uri::path_to_file_uri(&usage_path);
        let broken_uri = file_uri::path_to_file_uri(&broken_path.canonicalize().unwrap());
        let source = "function accept(Person $value): void {}\n";
        let mut server = stage31_server(&[&root_uri]);
        let project = project::test_project(
            &root,
            &["src/Person.doria", "src/main.doria", "src/Broken.doria"],
            project::PackageSource::Path,
            &[],
        );
        let mut session = CompilationSession::default();
        analyze_project_graph(
            &project,
            &project.tooling_build_plan,
            &[OpenSource {
                uri: &usage_uri,
                relative_path: "src/main.doria".to_string(),
                text: source,
            }],
            &mut session,
        )
        .unwrap_or_else(|failure| panic!("project graph: {:?}", failure.diagnostics));
        server.projects.insert(root_uri.clone(), project);
        open_stage31_document(&mut server, &usage_uri, source);

        assert_eq!(server.documents[&usage_uri].text, source);
        assert!(
            server.documents[&usage_uri]
                .analysis
                .diagnostics()
                .is_empty(),
            "{:?}",
            server.documents[&usage_uri].analysis.diagnostics()
        );
        assert!(server.project_documents.contains_key(&declaration_uri));
        assert!(server.project_documents[&broken_uri]
            .analysis
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unknown type")));
        let mut diagnostic_output = Vec::new();
        server
            .publish_all_diagnostics(&mut diagnostic_output)
            .unwrap();
        let diagnostic_output = String::from_utf8(diagnostic_output).unwrap();
        assert!(diagnostic_output.contains(&broken_uri));
        assert!(diagnostic_output.contains("unknown type"));
        let offset = source.find("Person").unwrap();
        let definition = server.definition(Some(&params_at(&usage_uri, source, offset)));
        assert_eq!(definition["uri"], declaration_uri);
        let declaration_source = "class Person {}\n";
        let hover = server
            .hover(Some(&params_at(
                &declaration_uri,
                declaration_source,
                declaration_source.find("Person").unwrap(),
            )))
            .expect("unopened supplied source should support hover");
        assert!(hover["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("Person"));
        assert!(server.semantic_tokens(Some(&json!({
            "textDocument": { "uri": declaration_uri }
        })))["data"]
            .as_array()
            .is_some_and(|tokens| !tokens.is_empty()));
        let rename = server.rename(Some(&json!({
            "textDocument": { "uri": usage_uri },
            "position": params_at(&usage_uri, source, offset)["position"].clone(),
            "newName": "Customer"
        })));
        let changes = rename["changes"].as_object().unwrap();
        assert!(changes.contains_key(&declaration_uri));
        assert!(changes.contains_key(&usage_uri));

        server.projects.insert(
            root_uri.clone(),
            project::test_project(
                &root,
                &["src/Person.doria", "src/main.doria", "src/Broken.doria"],
                project::PackageSource::Git,
                &[],
            ),
        );
        server.reanalyze_documents();
        assert_eq!(
            server.rename(Some(&json!({
                "textDocument": { "uri": usage_uri },
                "position": params_at(&usage_uri, source, offset)["position"].clone(),
                "newName": "Customer"
            }))),
            Value::Null
        );

        server.projects.insert(
            root_uri,
            project::test_project(
                &root,
                &["src/Person.doria", "src/main.doria", "src/Broken.doria"],
                project::PackageSource::Path,
                &["src/Person.doria"],
            ),
        );
        server.reanalyze_documents();
        assert_eq!(
            server.rename(Some(&json!({
                "textDocument": { "uri": usage_uri },
                "position": params_at(&usage_uri, source, offset)["position"].clone(),
                "newName": "Customer"
            }))),
            Value::Null
        );

        server.projects.clear();
        server.reanalyze_documents();
        let mut cleared_output = Vec::new();
        server.publish_all_diagnostics(&mut cleared_output).unwrap();
        let cleared_output = String::from_utf8(cleared_output).unwrap();
        assert!(cleared_output.contains(&broken_uri));
        assert!(cleared_output.contains("\"diagnostics\":[]"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn native_test_declarations_follow_compiler_facts_and_project_source_scope() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("doria-lsp-native-tests-{nonce}"));
        fs::create_dir_all(root.join("tests")).unwrap();
        let root = root.canonicalize().unwrap();
        let source_path = root.join("tests/behavior.doria");
        let source = r#"use Doria\Std\Test\{
    AssertionError,
    describe,
    expect,
    fail,
    it,
    test,
};

function assertionHelper(): void {
    expect(42)->not->toEqual(0);
}

function consumeAssertion(AssertionError $error): void {}
function assertionFailure(): void { fail("caught"); }

describe("🧪 User", function (): void {
    describe("creation", function (): void {
        it("can be created", function (): void { assertionHelper(); });
    });
    test("can be removed", function (): void {});
});
"#;
        fs::write(&source_path, source).unwrap();
        let root_uri = file_uri::path_to_file_uri(&root);
        let source_uri = file_uri::path_to_file_uri(&source_path);
        let mut project = project::test_project(
            &root,
            &["tests/behavior.doria"],
            project::PackageSource::Path,
            &[],
        );
        project.packages[0].sources[0].scope = doriac::build_plan::SourceScope::Development;
        project.tooling_build_plan.packages[0].sources[0].scope =
            doriac::build_plan::SourceScope::Development;
        project.tooling_build_plan.selected_target.active_scopes = vec![
            doriac::build_plan::SourceScope::Main,
            doriac::build_plan::SourceScope::Development,
        ];

        let mut session = CompilationSession::default();
        let graph = analyze_project_graph(
            &project,
            &project.tooling_build_plan,
            &[OpenSource {
                uri: &source_uri,
                relative_path: "tests/behavior.doria".to_string(),
                text: source,
            }],
            &mut session,
        )
        .unwrap_or_else(|failure| panic!("native test project graph: {:?}", failure.diagnostics));
        assert!(graph.documents[&source_uri]
            .analysis
            .diagnostics()
            .is_empty());

        let mut server = stage31_server(&[&root_uri]);
        server.projects.insert(root_uri.clone(), project);
        open_stage31_document(&mut server, &source_uri, source);
        let analysis = &server.documents[&source_uri].analysis;
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        assert!(analysis
            .source_semantic_context()
            .is_some_and(doriac::testing::SourceSemanticContext::is_development));
        assert_eq!(analysis.test_semantics().suites.len(), 2);
        assert_eq!(analysis.test_semantics().tests.len(), 2);
        assert!(analysis
            .test_semantics()
            .suites
            .iter()
            .all(|suite| suite.source.0 == "acme/app:tests/behavior.doria"));
        assert!(analysis
            .test_semantics()
            .tests
            .iter()
            .all(|test| test.source.0 == "acme/app:tests/behavior.doria"));

        let test_imports = analysis
            .global_symbols()
            .references
            .iter()
            .filter(|reference| {
                reference.role == doriac::names::GlobalReferenceRole::ImportTarget
                    && doriac::compiler_known_test::is_declaration(
                        &reference.symbol_id.qualified_name,
                    )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            test_imports.len(),
            3,
            "{:?}",
            analysis.global_symbols().references
        );
        assert!(test_imports
            .iter()
            .all(|reference| { reference.source_identity.0 == "acme/app:tests/behavior.doria" }));

        let completion_labels = server
            .document_index
            .completions(&source_uri)
            .into_iter()
            .map(|completion| completion.label)
            .collect::<std::collections::HashSet<_>>();
        for implemented in ["describe", "it", "test", "expect", "fail", "AssertionError"] {
            assert!(completion_labels.contains(implemented));
        }

        let tokens = semantic_token_records(&server.semantic_tokens(Some(&json!({
            "textDocument": { "uri": source_uri }
        }))));
        for (name, offset) in source
            .match_indices("describe(")
            .map(|(offset, _)| ("describe", offset))
            .chain(
                source
                    .match_indices("it(")
                    .map(|(offset, _)| ("it", offset)),
            )
            .chain(
                source
                    .match_indices("test(")
                    .map(|(offset, _)| ("test", offset)),
            )
        {
            let position = byte_offset_to_position(source, offset);
            assert!(tokens.contains(&(
                position.line,
                position.character,
                name.encode_utf16().count() as u32,
                3,
                1,
            )));
        }
        for description in [
            "\"🧪 User\"",
            "\"creation\"",
            "\"can be created\"",
            "\"can be removed\"",
        ] {
            let offset = source.find(description).unwrap();
            let position = byte_offset_to_position(source, offset);
            assert!(tokens.contains(&(
                position.line,
                position.character,
                description.encode_utf16().count() as u32,
                6,
                0,
            )));
        }
        for (needle, name, token_type) in [
            ("expect(42)", "expect", 3),
            ("not->", "not", 2),
            ("toEqual(0)", "toEqual", 3),
            ("fail(\"caught\")", "fail", 3),
            ("AssertionError $error", "AssertionError", 1),
        ] {
            let offset = source.find(needle).unwrap();
            let position = byte_offset_to_position(source, offset);
            assert!(
                tokens.contains(&(
                    position.line,
                    position.character,
                    name.encode_utf16().count() as u32,
                    token_type,
                    0,
                )),
                "missing semantic token for {needle}: {tokens:?}"
            );
        }
        let helper_hover = analysis
            .hover_at_offset(source.find("assertionHelper").unwrap())
            .expect("assertion helper hover");
        assert!(helper_hover.markdown.contains("**Test assertions:**"));
        assert!(helper_hover
            .markdown
            .contains("Doria\\Std\\Test\\AssertionError"));
        assert!(!helper_hover.markdown.contains("throws AssertionError"));

        let ordinary = "function describe(string $name): void {}\nfunction main(): void { describe(\"ordinary\"); }\n";
        let ordinary_snapshot = AnalysisSnapshot::analyze("ordinary.doria", ordinary);
        let ordinary_call = ordinary.rfind("describe(").unwrap();
        assert!(ordinary_snapshot
            .semantic_token_spans()
            .iter()
            .filter(|(span, _, _)| span.start == ordinary_call)
            .all(|(_, _, modifiers)| *modifiers == 0));
        assert!(ordinary_snapshot.test_semantics().suites.is_empty());
        assert!(ordinary_snapshot.test_semantics().tests.is_empty());

        let ordinary_expect =
            "function expect(int $value): void {}\nfunction main(): void { expect(42); }\n";
        let ordinary_expect_snapshot =
            AnalysisSnapshot::analyze("ordinary-expect.doria", ordinary_expect);
        assert!(ordinary_expect_snapshot.diagnostics().is_empty());
        assert!(ordinary_expect_snapshot
            .global_symbols()
            .references
            .iter()
            .filter(|reference| reference.source_spelling == "expect")
            .all(|reference| !matches!(
                reference.symbol_id.owner,
                doriac::names::GlobalSymbolOwner::CompilerKnown(
                    doriac::names::CompilerSymbolIdentity::StandardTest(_)
                )
            )));

        for (variant, call_name) in [
            (
                "use Doria\\Std\\Test\\expect as verify;\nfunction helper(): void { verify(42)->toEqual(42); }\n",
                "verify",
            ),
            (
                "function helper(): void { Doria\\Std\\Test\\expect(42)->toEqual(42); }\n",
                "expect",
            ),
        ] {
            let mut variant_project = project::test_project(
                &root,
                &["tests/behavior.doria"],
                project::PackageSource::Path,
                &[],
            );
            variant_project.packages[0].sources[0].scope =
                doriac::build_plan::SourceScope::Development;
            variant_project.tooling_build_plan.packages[0].sources[0].scope =
                doriac::build_plan::SourceScope::Development;
            variant_project
                .tooling_build_plan
                .selected_target
                .active_scopes = vec![
                doriac::build_plan::SourceScope::Main,
                doriac::build_plan::SourceScope::Development,
            ];
            server.projects.insert(root_uri.clone(), variant_project);
            open_stage31_document(&mut server, &source_uri, variant);
            assert!(
                server.documents[&source_uri]
                    .analysis
                    .diagnostics()
                    .is_empty(),
                "{variant}: {:?}",
                server.documents[&source_uri].analysis.diagnostics()
            );
            let tokens = semantic_token_records(&server.semantic_tokens(Some(&json!({
                "textDocument": { "uri": source_uri }
            }))));
            let offset = variant.rfind(call_name).unwrap();
            let position = byte_offset_to_position(variant, offset);
            assert!(tokens.contains(&(
                position.line,
                position.character,
                call_name.encode_utf16().count() as u32,
                3,
                0,
            )));
        }

        let mut main_project = project::test_project(
            &root,
            &["tests/behavior.doria"],
            project::PackageSource::Path,
            &[],
        );
        main_project
            .tooling_build_plan
            .selected_target
            .active_scopes = vec![doriac::build_plan::SourceScope::Main];
        server.projects.insert(root_uri.clone(), main_project);
        open_stage31_document(&mut server, &source_uri, source);
        assert!(server.documents[&source_uri]
            .analysis
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == "E0701"));

        let malformed =
            "use Doria\\Std\\Test\\{ expect };\nfunction helper(): void { expect(1); }\n";
        let mut malformed_project = project::test_project(
            &root,
            &["tests/behavior.doria"],
            project::PackageSource::Path,
            &[],
        );
        malformed_project.packages[0].sources[0].scope =
            doriac::build_plan::SourceScope::Development;
        malformed_project.tooling_build_plan.packages[0].sources[0].scope =
            doriac::build_plan::SourceScope::Development;
        malformed_project
            .tooling_build_plan
            .selected_target
            .active_scopes = vec![
            doriac::build_plan::SourceScope::Main,
            doriac::build_plan::SourceScope::Development,
        ];
        server.projects.insert(root_uri.clone(), malformed_project);
        open_stage31_document(&mut server, &source_uri, malformed);
        let diagnostics = server.documents[&source_uri].analysis.diagnostics();
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "E0714")
                .count(),
            1
        );

        for (invalid, code) in [
            (
                "use Doria\\Std\\Test\\expect;\nfunction helper(): void { expect(value: 1)->toEqual(1); }\n",
                "E0713",
            ),
            (
                "use Doria\\Std\\Test\\expect;\nfunction helper(): void { expect(1)->not->not->toEqual(2); }\n",
                "E0716",
            ),
            (
                "use Doria\\Std\\Test\\expect;\nfunction helper(): void { expect(1)->not(); }\n",
                "E0716",
            ),
            (
                "use Doria\\Std\\Test\\expect;\nfunction helper(): void { echo \"😀\"; expect(1)->toEqual(); }\n",
                "E0717",
            ),
            (
                "use Doria\\Std\\Test\\expect;\nfunction helper(): void { expect(1)->toEqual(value: 1); }\n",
                "E0717",
            ),
            (
                "use Doria\\Std\\Test\\expect;\nfunction helper(): void { expect(1)->unknownMatcher(); }\n",
                "E0717",
            ),
            (
                "use Doria\\Std\\Test\\expect;\nfunction helper(): void { let $escaped = expect(1)->toEqual(1); }\n",
                "E0715",
            ),
            (
                "use Doria\\Std\\Test\\expect;\nfunction helper(): void { expect([1])->toContain(1); }\n",
                "E0718",
            ),
            (
                "use Doria\\Std\\Test\\expect;\nfunction helper(): void { expect(1)->toBeNull(); }\n",
                "E0720",
            ),
            (
                "use Doria\\Std\\Test\\expect;\nfunction helper(): void { expect(1)->toBeTrue(); }\n",
                "E0720",
            ),
            (
                "use Doria\\Std\\Test\\expect;\nfunction helper(): void { expect(1)->toContain(\"x\"); }\n",
                "E0720",
            ),
            (
                "use Doria\\Std\\Test\\expect;\nfunction helper(): void { expect(1)->toEqual(\"1\"); }\n",
                "E0420",
            ),
            (
                "use Doria\\Std\\Test\\expect;\nfunction helper(): void { expect(1)->toThrow(); }\n",
                "E0718",
            ),
            (
                "use Doria\\Std\\Test\\AssertionError;\nfunction helper(): void { let $error = new AssertionError(\"x\"); }\n",
                "E0719",
            ),
        ] {
            open_stage31_document(&mut server, &source_uri, invalid);
            let diagnostics = server.documents[&source_uri].analysis.diagnostics();
            assert!(
                diagnostics.iter().any(|diagnostic| diagnostic.code == code),
                "{invalid}: {diagnostics:#?}"
            );
        }

        let utf16 = "use Doria\\Std\\Test\\expect;\nfunction helper(): void { echo \"😀\"; expect(1)->toEqual(); }\n";
        open_stage31_document(&mut server, &source_uri, utf16);
        let diagnostic = server.documents[&source_uri]
            .analysis
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code == "E0717")
            .expect("UTF-16 matcher diagnostic");
        let lsp = diagnostic_to_lsp(&source_uri, utf16, diagnostic);
        let primary = diagnostic
            .labels
            .iter()
            .find(|label| label.role == LabelRole::Primary)
            .map_or(diagnostic.span, |label| label.span);
        let expected = byte_offset_to_position(utf16, primary.start);
        assert_ne!(primary.start, expected.character as usize);
        assert_eq!(lsp["range"]["start"]["line"], expected.line);
        assert_eq!(lsp["range"]["start"]["character"], expected.character);

        let repaired =
            "use Doria\\Std\\Test\\expect;\nfunction helper(): void { expect(1)->toEqual(1); }\n";
        open_stage31_document(&mut server, &source_uri, repaired);
        assert!(
            server.documents[&source_uri]
                .analysis
                .diagnostics()
                .is_empty(),
            "stale assertion diagnostics must clear after a valid overlay"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn native_assertion_effects_flow_through_cross_file_helpers() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("doria-lsp-assertion-graph-{nonce}"));
        fs::create_dir_all(root.join("tests")).unwrap();
        let root = root.canonicalize().unwrap();
        let helper_path = root.join("tests/assertions.doria");
        let consumer_path = root.join("tests/consumer.doria");
        let helper = "use Doria\\Std\\Test\\expect;\nfunction sharedAssertion(): void { expect(42)->toEqual(42); }\n";
        let consumer = "function consumer(): void { sharedAssertion(); }\n";
        fs::write(&helper_path, helper).unwrap();
        fs::write(&consumer_path, consumer).unwrap();

        let root_uri = file_uri::path_to_file_uri(&root);
        let helper_uri = file_uri::path_to_file_uri(&helper_path);
        let consumer_uri = file_uri::path_to_file_uri(&consumer_path);
        let mut project = project::test_project(
            &root,
            &["tests/assertions.doria", "tests/consumer.doria"],
            project::PackageSource::Path,
            &[],
        );
        for source in &mut project.packages[0].sources {
            source.scope = doriac::build_plan::SourceScope::Development;
        }
        for source in &mut project.tooling_build_plan.packages[0].sources {
            source.scope = doriac::build_plan::SourceScope::Development;
        }
        project.tooling_build_plan.selected_target.active_scopes = vec![
            doriac::build_plan::SourceScope::Main,
            doriac::build_plan::SourceScope::Development,
        ];

        let mut server = stage31_server(&[&root_uri]);
        server.projects.insert(root_uri, project);
        open_stage31_document(&mut server, &helper_uri, helper);
        open_stage31_document(&mut server, &consumer_uri, consumer);
        assert!(
            server.documents[&helper_uri]
                .analysis
                .diagnostics()
                .is_empty(),
            "{:?}",
            server.documents[&helper_uri].analysis.diagnostics()
        );
        assert!(
            server.documents[&consumer_uri]
                .analysis
                .diagnostics()
                .is_empty(),
            "{:?}",
            server.documents[&consumer_uri].analysis.diagnostics()
        );
        let hover = server
            .hover(Some(&params_at(
                &consumer_uri,
                consumer,
                consumer.find("sharedAssertion").unwrap(),
            )))
            .expect("cross-file assertion helper hover");
        let markdown = hover["contents"]["value"]
            .as_str()
            .expect("cross-file assertion helper markdown");
        assert!(markdown.contains("**Test assertions:**"));
        assert!(markdown.contains("Doria\\Std\\Test\\AssertionError"));
        assert!(!markdown.contains("throws AssertionError"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn initialized_registers_only_project_structure_watchers() {
        let mut output = Vec::new();
        register_project_watchers(&mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("client/registerCapability"));
        for path in [
            "Baton.toml",
            "Baton.lock",
            "*.doria",
            ".doria/build",
            "build/.baton",
        ] {
            assert!(output.contains(path));
        }
    }

    #[test]
    fn modified_doria_sources_schedule_project_rediscovery() {
        let root_uri = "file:///workspace".to_string();
        let mut server = Server {
            workspace_roots: vec![WorkspaceRoot {
                uri: root_uri.clone(),
                package: "workspace/project".to_string(),
            }],
            ..Server::default()
        };

        server.did_change_watched_files(Some(&json!({
            "changes": [{
                "uri": "file:///workspace/src/Changed.doria",
                "type": 2
            }]
        })));

        assert_eq!(
            server.project_discovery.pending_generation(&root_uri),
            Some(1)
        );
    }

    #[test]
    fn project_inventory_replaces_watchers_with_exact_package_roots() {
        let root = std::env::temp_dir().join("doria-lsp-stage33-watchers");
        let project =
            project::test_project(&root, &["src/main.doria"], project::PackageSource::Git, &[]);
        let root_uri = file_uri::path_to_file_uri(&root);
        let mut server = Server {
            supports_dynamic_watching: true,
            ..Server::default()
        };
        let mut output = Vec::new();
        server
            .replace_project_watchers(&root_uri, Some(&project), &mut output)
            .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("client/registerCapability"));
        assert!(output.contains(&root_uri));
        assert!(output.contains("**/*.doria"));

        let mut replacement = Vec::new();
        server
            .replace_project_watchers(&root_uri, None, &mut replacement)
            .unwrap();
        assert!(String::from_utf8(replacement)
            .unwrap()
            .contains("client/unregisterCapability"));
    }
}
