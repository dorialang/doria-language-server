use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use doriac::ast::{
    Block, ClassDecl, ClassMember, ControlFlowFinally, DoWhileStmt, ElseBranch, EnumDecl, Expr,
    ForIncrement, ForInitializer, FunctionDecl, GivenPrelude, IfStmt, Item, MatchMode, MatchOrigin,
    MatchPattern, MemberAccess, Param, Program, StaticQualifier, Stmt, TryStmt, VarDecl,
    WhenExpression, WhileStmt,
};
use doriac::attributes::{
    AttributeApplication, AttributeClassIdentity, AttributeClassSchema, AttributeSemanticInfo,
    AttributeTarget, AttributeValue, AttributeValueKind,
};
use doriac::diagnostics::{Diagnostic, DiagnosticSeverity, LabelRole};
use doriac::enums::{EnumBackingType, EnumBackingValue};
use doriac::lexer::{Token, TokenKind};
use doriac::names::{
    CompilationContext, CompilerSymbolIdentity, GlobalReferenceRole, GlobalSymbolFacts,
    GlobalSymbolId, GlobalSymbolKind, GlobalSymbolOwner, SourceIdentity,
};
use doriac::ownership::{
    CaptureAcquisitionKind, ClosureBorrowRoot, ClosureEscapeClassification, ClosureValueProvenance,
    InvocationConsumption,
};
use doriac::semantics::{
    CallableTarget, EnumSemanticInfo, ListAlgorithmCallInfo, ListAlgorithmKind, ListCallbackAccess,
    SemanticInfo,
};
use doriac::source::{SourceFile, SourceId, Span};
use doriac::symbols::{BindingKind, BindingOwnership};
use doriac::testing::{SourceSemanticContext, TestSemanticFacts};
use doriac::types::{
    FunctionInvocationMode, ResolvedType, SharedHandleKind, TypeRef, TypeRegistry,
};

use crate::string_surface::{string_companion_method, string_property};

const SEMANTIC_TOKEN_FUNCTION: u32 = 3;
const SEMANTIC_TOKEN_TYPE: u32 = 1;
const SEMANTIC_TOKEN_PROPERTY: u32 = 2;
const SEMANTIC_TOKEN_STRING: u32 = 6;
const SEMANTIC_TOKEN_DECLARATION: u32 = 1;

pub(crate) type SemanticTokenSpan = (Span, u32, u32);

#[derive(Debug, Clone)]
pub(crate) struct SemanticHover {
    pub(crate) span: Span,
    pub(crate) markdown: String,
    priority: SemanticHoverPriority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocumentationCommentTemplate {
    pub(crate) tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportCandidate {
    pub(crate) target: String,
    pub(crate) alias: String,
    pub(crate) reference_span: Span,
    pub(crate) requires_import: bool,
    pub(crate) class_like: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SemanticImport {
    pub(crate) target: String,
    pub(crate) alias: String,
    pub(crate) class_like: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MissingCallable {
    pub(crate) name: String,
    pub(crate) name_span: Span,
    pub(crate) parameters: Vec<GeneratedParameter>,
    pub(crate) return_type: String,
    pub(crate) target: MissingCallableTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClassDeclaration {
    name: String,
    span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GeneratedParameter {
    pub(crate) name: String,
    pub(crate) ty: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MissingCallableTarget {
    Function,
    Method { class_name: String, is_static: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnresolvedImportReference {
    pub(crate) spelling: String,
    pub(crate) span: Span,
    pub(crate) role: GlobalReferenceRole,
}

pub(crate) fn documentation_comment_template(
    text: &str,
    comment_start: usize,
    cursor_offset: usize,
) -> Option<DocumentationCommentTemplate> {
    let recovered = recover_unclosed_comment(text, comment_start, cursor_offset)?;
    let program = doriac::parse_source("documentation-comment.doria", recovered).ok()?;
    documentation_targets(&program)
        .into_iter()
        .filter(|target| target.start >= cursor_offset)
        .min_by_key(|target| target.start)
        .map(|target| DocumentationCommentTemplate { tags: target.tags })
}

fn recover_unclosed_comment(
    text: &str,
    comment_start: usize,
    cursor_offset: usize,
) -> Option<String> {
    if comment_start > cursor_offset || cursor_offset > text.len() {
        return None;
    }
    if text[comment_start..].contains("*/") {
        return Some(text.to_string());
    }

    let mut recovered = text.as_bytes().to_vec();
    for byte in &mut recovered[comment_start..cursor_offset] {
        if *byte != b'\n' && *byte != b'\r' {
            *byte = b' ';
        }
    }
    String::from_utf8(recovered).ok()
}

#[derive(Debug)]
struct DocumentationTarget {
    start: usize,
    tags: Vec<String>,
}

fn documentation_targets(program: &Program) -> Vec<DocumentationTarget> {
    let mut targets = Vec::new();
    for item in &program.items {
        match item {
            Item::Function(function) => targets.push(function_documentation_target(function)),
            Item::Class(class) => {
                targets.push(DocumentationTarget {
                    start: class.span.start,
                    tags: template_tags(&class.type_params),
                });
                member_documentation_targets(&class.members, &mut targets);
            }
            Item::Trait(trait_decl) => {
                targets.push(DocumentationTarget {
                    start: trait_decl.span.start,
                    tags: Vec::new(),
                });
                member_documentation_targets(&trait_decl.members, &mut targets);
            }
            Item::Enum(enum_decl) => targets.push(DocumentationTarget {
                start: enum_decl.span.start,
                tags: template_tags(&enum_decl.type_params),
            }),
            Item::Interface(interface) => targets.push(DocumentationTarget {
                start: interface.span.start,
                tags: Vec::new(),
            }),
            Item::Constant(constant) => targets.push(DocumentationTarget {
                start: constant.span.start,
                tags: constant
                    .ty
                    .as_ref()
                    .map(|ty| vec![format!("@var {ty}")])
                    .unwrap_or_default(),
            }),
            Item::Statement(_) => {}
        }
    }
    targets
}

fn member_documentation_targets(members: &[ClassMember], targets: &mut Vec<DocumentationTarget>) {
    for member in members {
        match member {
            ClassMember::Method(function) => {
                targets.push(function_documentation_target(function));
            }
            ClassMember::Property(property) => targets.push(DocumentationTarget {
                start: property.span.start,
                tags: vec![format!("@var {}", property.ty)],
            }),
            ClassMember::Constant(constant) => targets.push(DocumentationTarget {
                start: constant.span.start,
                tags: constant
                    .ty
                    .as_ref()
                    .map(|ty| vec![format!("@var {ty}")])
                    .unwrap_or_default(),
            }),
        }
    }
}

fn function_documentation_target(function: &FunctionDecl) -> DocumentationTarget {
    let mut tags = template_tags(&function.type_params);
    tags.extend(
        function
            .params
            .iter()
            .map(|parameter| format!("@param {}", documentation_parameter_signature(parameter))),
    );
    if let Some(return_type) = &function.return_type {
        tags.push(format!("@return {return_type}"));
    }
    if let Some(throws) = &function.throws {
        tags.extend(
            throws
                .entries
                .iter()
                .map(|entry| format!("@throws {}", entry.ty)),
        );
    }
    DocumentationTarget {
        start: function.span.start,
        tags,
    }
}

fn template_tags(parameters: &[doriac::ast::TypeParamDecl]) -> Vec<String> {
    parameters
        .iter()
        .map(|parameter| format!("@template {}", parameter.name))
        .collect()
}

fn documentation_parameter_signature(parameter: &Param) -> String {
    let mut parts = Vec::new();
    if matches!(parameter.promoted_access, Some(MemberAccess::Internal)) {
        parts.push("internal".to_string());
    }
    if parameter.take {
        parts.push("take".to_string());
    }
    if parameter.writable {
        parts.push("writable".to_string());
    }
    parts.push(parameter.ty.to_string());
    parts.push(format!("${}", parameter.name));
    parts.join(" ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SemanticHoverPriority {
    General,
    Capture,
}

impl SemanticHover {
    fn new(span: Span, markdown: String) -> Self {
        Self {
            span,
            markdown,
            priority: SemanticHoverPriority::General,
        }
    }

    fn capture(span: Span, markdown: String) -> Self {
        Self {
            span,
            markdown,
            priority: SemanticHoverPriority::Capture,
        }
    }

    fn selection_key(&self) -> (usize, Reverse<SemanticHoverPriority>) {
        (
            self.span.end.saturating_sub(self.span.start),
            Reverse(self.priority),
        )
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct AnalysisSnapshot {
    diagnostics: Vec<Diagnostic>,
    source_id: SourceId,
    compilation_context: CompilationContext,
    global_symbols: GlobalSymbolFacts,
    directive_semantic_tokens: Vec<(Span, u32)>,
    attribute_semantic_tokens: Vec<(Span, u32)>,
    assertion_semantic_tokens: Vec<SemanticTokenSpan>,
    test_semantics: TestSemanticFacts,
    source_semantic_context: Option<SourceSemanticContext>,
    attribute_info: AttributeSemanticInfo,
    attribute_parameter_occurrences: Vec<AttributeParameterOccurrence>,
    symbols: Vec<Symbol>,
    occurrences: Vec<Occurrence>,
    member_receivers: Vec<MemberReceiver>,
    static_receivers: Vec<StaticReceiver>,
    class_members: HashMap<String, Vec<ClassMemberCompletion>>,
    class_parents: HashMap<String, String>,
    enum_case_completions: HashMap<String, Vec<SemanticCompletion>>,
    enum_member_completions: HashMap<String, Vec<SemanticCompletion>>,
    local_visibilities: Vec<LocalVisibility>,
    call_signatures: Vec<CallSignatureContext>,
    semantic_hovers: Vec<SemanticHover>,
    missing_callables: Vec<MissingCallable>,
    class_declarations: Vec<ClassDeclaration>,
    member_occurrences: Vec<MemberOccurrence>,
    member_parents: Vec<MemberParent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct AttributeParameterIdentity {
    pub(crate) class: GlobalSymbolId,
    pub(crate) index: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct AttributeParameterOccurrence {
    pub(crate) identity: AttributeParameterIdentity,
    pub(crate) name: String,
    pub(crate) span: Span,
    pub(crate) declaration: bool,
    pub(crate) spelling: AttributeParameterSpelling,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MemberKind {
    Method,
    Property,
    Constant,
    EnumCase,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct MemberIdentity {
    pub(crate) owner: GlobalSymbolId,
    pub(crate) name: String,
    pub(crate) kind: MemberKind,
}

#[derive(Debug, Clone)]
pub(crate) struct MemberOccurrence {
    pub(crate) identity: MemberIdentity,
    pub(crate) span: Span,
    pub(crate) declaration: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct MemberParent {
    pub(crate) child: GlobalSymbolId,
    pub(crate) parent: GlobalSymbolId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttributeParameterSpelling {
    Variable,
    Label,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SemanticCompletion {
    pub(crate) label: String,
    pub(crate) kind: u32,
    pub(crate) detail: String,
    pub(crate) documentation: Option<String>,
}

#[derive(Debug, Clone)]
struct Symbol {
    signature: String,
    documentation: Option<String>,
    local_name: Option<String>,
    parameter_names: Vec<String>,
    kind: SymbolKind,
}

#[derive(Debug, Clone, Copy)]
struct Occurrence {
    span: Span,
    symbol: usize,
    role: OccurrenceRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SymbolKind {
    Plain,
    Variable,
    Keyword,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OccurrenceRole {
    Declaration,
    Reference,
}

#[derive(Debug, Clone)]
struct MemberReceiver {
    span: Span,
    receiver: ResolvedType,
    current_class: Option<String>,
    writable_payload_access: bool,
}

#[derive(Debug, Clone)]
struct StaticReceiver {
    span: Span,
    enum_name: String,
}

#[derive(Debug, Clone)]
struct ClassMemberCompletion {
    completion: SemanticCompletion,
    writable: bool,
    internal: bool,
    is_static: bool,
}

#[derive(Debug, Clone, Copy)]
struct LocalVisibility {
    symbol: usize,
    start: usize,
    end: usize,
    depth: usize,
}

#[derive(Debug, Clone)]
struct CallSignatureContext {
    span: Span,
    arguments: Vec<CallArgumentContext>,
    symbol: usize,
}

#[derive(Debug, Clone, Copy)]
struct CallArgumentContext {
    span: Span,
    parameter: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SignatureHelp {
    pub(crate) label: String,
    pub(crate) active_parameter: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalCompletion {
    pub(crate) label: String,
    pub(crate) detail: String,
}

impl AnalysisSnapshot {
    pub(crate) fn analyze(path: &str, text: &str) -> Self {
        Self::analyze_with_context(path, text, CompilationContext::standalone(path))
    }

    pub(crate) fn analyze_with_context(
        path: &str,
        text: &str,
        context: CompilationContext,
    ) -> Self {
        let tokens = doriac::lex_source(path.to_string(), text.to_string()).unwrap_or_default();
        let (program, analysis) = match doriac::analyze_source_for_ide_with_context(
            path.to_string(),
            text.to_string(),
            context.clone(),
        ) {
            Ok(analysis) => analysis,
            Err(diagnostics) => {
                return Self {
                    diagnostics,
                    compilation_context: context,
                    ..Self::default()
                };
            }
        };
        let global_symbols = analysis.info.global_symbols.clone();
        let directive_semantic_tokens = directive_semantic_tokens(&program);
        let mut snapshot = SnapshotBuilder::new(
            text,
            &tokens,
            SourceId::default(),
            Some(&analysis.info),
            analysis.diagnostics,
        )
        .build(&program);
        snapshot.compilation_context = context;
        snapshot.global_symbols = global_symbols;
        snapshot.directive_semantic_tokens = directive_semantic_tokens;
        snapshot
    }

    pub(crate) fn from_graph_source(
        text: &str,
        source_id: SourceId,
        context: CompilationContext,
        program: &Program,
        semantic_info: &SemanticInfo,
        diagnostics: Vec<Diagnostic>,
        global_symbols: GlobalSymbolFacts,
    ) -> Self {
        let source = SourceFile::with_id(source_id, context.source.0.clone(), text.to_string());
        let tokens = doriac::lexer::Lexer::new(&source).lex().unwrap_or_default();
        let directive_semantic_tokens = directive_semantic_tokens(program);
        let mut snapshot =
            SnapshotBuilder::new(text, &tokens, source_id, Some(semantic_info), diagnostics)
                .build(program);
        snapshot.compilation_context = context;
        snapshot.global_symbols = global_symbols;
        snapshot.directive_semantic_tokens = directive_semantic_tokens;
        snapshot
    }

    pub(crate) fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub(crate) fn extend_diagnostics(&mut self, diagnostics: Vec<Diagnostic>) {
        self.diagnostics.extend(diagnostics);
    }

    pub(crate) fn compilation_context(&self) -> &CompilationContext {
        &self.compilation_context
    }

    pub(crate) fn global_symbols(&self) -> &GlobalSymbolFacts {
        &self.global_symbols
    }

    #[cfg(test)]
    pub(crate) fn test_semantics(&self) -> &TestSemanticFacts {
        &self.test_semantics
    }

    #[cfg(test)]
    pub(crate) fn source_semantic_context(&self) -> Option<&SourceSemanticContext> {
        self.source_semantic_context.as_ref()
    }

    pub(crate) fn member_occurrences(&self) -> &[MemberOccurrence] {
        &self.member_occurrences
    }

    pub(crate) fn member_parents(&self) -> &[MemberParent] {
        &self.member_parents
    }

    pub(crate) fn missing_callable_at_offset(&self, offset: usize) -> Option<&MissingCallable> {
        self.missing_callables
            .iter()
            .filter(|callable| span_contains(callable.name_span, offset))
            .min_by_key(|callable| {
                callable
                    .name_span
                    .end
                    .saturating_sub(callable.name_span.start)
            })
    }

    pub(crate) fn class_declaration_span(&self, name: &str) -> Option<Span> {
        let mut declarations = self
            .class_declarations
            .iter()
            .filter(|declaration| declaration.name == name);
        let declaration = declarations.next()?;
        declarations.next().is_none().then_some(declaration.span)
    }

    pub(crate) fn import_candidate_at_offset(
        &self,
        source: &SourceIdentity,
        offset: usize,
    ) -> Option<ImportCandidate> {
        let resolved = self
            .global_symbols
            .references
            .iter()
            .filter(|reference| {
                reference.source_identity == *source
                    && importable_reference_role(reference.role)
                    && reference.source_spelling.contains('\\')
                    && span_contains(reference.source_span, offset)
            })
            .map(|reference| {
                (
                    reference.symbol_id.qualified_name.clone(),
                    reference.source_span,
                )
            });
        let unresolved = self
            .global_symbols
            .unresolved
            .iter()
            .filter(|reference| {
                reference.source_identity == *source
                    && importable_reference_role(reference.role)
                    && reference.source_spelling.contains('\\')
                    && span_contains(reference.source_span, offset)
            })
            .map(|reference| (reference.source_spelling.clone(), reference.source_span));
        let (target, reference_span) = resolved
            .chain(unresolved)
            .min_by_key(|(_, span)| span.end.saturating_sub(span.start))?;

        self.import_candidate_for_target(source, target, reference_span, None)
    }

    pub(crate) fn unresolved_short_import_at_offset(
        &self,
        text: &str,
        source: &SourceIdentity,
        offset: usize,
    ) -> Option<UnresolvedImportReference> {
        if let Some(reference) = self
            .global_symbols
            .unresolved
            .iter()
            .filter(|reference| {
                reference.source_identity == *source
                    && importable_reference_role(reference.role)
                    && !reference.source_spelling.contains('\\')
                    && span_contains(reference.source_span, offset)
            })
            .min_by_key(|reference| {
                reference
                    .source_span
                    .end
                    .saturating_sub(reference.source_span.start)
            })
            .map(|reference| UnresolvedImportReference {
                spelling: reference.source_spelling.clone(),
                span: reference.source_span,
                role: reference.role,
            })
        {
            return Some(reference);
        }

        let diagnostic = self.diagnostics.iter().find(|diagnostic| {
            unresolved_import_role(diagnostic.code).is_some()
                && (diagnostic
                    .labels
                    .iter()
                    .find(|label| label.role == LabelRole::Primary)
                    .is_some_and(|label| span_contains(label.span, offset))
                    || (diagnostic.labels.is_empty() && span_contains(diagnostic.span, offset)))
        })?;
        let role = unresolved_import_role(diagnostic.code)?;
        let token = doriac::lex_source("import-candidate.doria", text.to_string())
            .ok()?
            .into_iter()
            .find(|token| span_contains(token.span, offset))?;
        let TokenKind::Identifier(spelling) = token.kind else {
            return None;
        };
        Some(UnresolvedImportReference {
            spelling,
            span: token.span,
            role,
        })
    }

    pub(crate) fn import_candidate_for_target(
        &self,
        source: &SourceIdentity,
        target: String,
        reference_span: Span,
        class_like: Option<bool>,
    ) -> Option<ImportCandidate> {
        let class_like =
            class_like.unwrap_or_else(|| self.import_target_is_class_like(source, &target));
        let imports = self
            .global_symbols
            .imports
            .iter()
            .filter(|import| import.source_identity == *source)
            .collect::<Vec<_>>();
        if let Some(existing) = imports.iter().find(|import| import.target == target) {
            return Some(ImportCandidate {
                class_like,
                target,
                alias: existing.alias.clone(),
                reference_span,
                requires_import: false,
            });
        }

        let alias = target.rsplit('\\').next()?.to_string();
        let namespace = self
            .global_symbols
            .namespaces
            .iter()
            .find(|namespace| namespace.source_identity == *source)
            .map(|namespace| namespace.name.canonical());
        if namespace
            .as_ref()
            .is_some_and(|namespace| format!("{namespace}\\{alias}") == target)
        {
            return None;
        }
        let alias_conflicts =
            imports
                .iter()
                .any(|import| import.alias == alias && import.target != target)
                || self.global_symbols.declarations.iter().any(|declaration| {
                    declaration.qualified_name
                        == namespace.as_ref().map_or_else(
                            || alias.clone(),
                            |namespace| format!("{namespace}\\{alias}"),
                        )
                })
                || self.global_symbols.compiler_known.iter().any(|symbol| {
                    symbol.source_name == alias && symbol.id.qualified_name != target
                });
        if alias_conflicts {
            return None;
        }

        Some(ImportCandidate {
            class_like,
            target,
            alias,
            reference_span,
            requires_import: true,
        })
    }

    pub(crate) fn semantic_imports(&self, source: &SourceIdentity) -> Vec<SemanticImport> {
        self.global_symbols
            .imports
            .iter()
            .filter(|import| import.source_identity == *source)
            .map(|import| SemanticImport {
                target: import.target.clone(),
                alias: import.alias.clone(),
                class_like: self.import_target_is_class_like(source, &import.target),
            })
            .collect()
    }

    fn import_target_is_class_like(&self, source: &SourceIdentity, target: &str) -> bool {
        if let Some(kind) = self
            .global_symbols
            .declarations
            .iter()
            .find(|declaration| declaration.qualified_name == target)
            .map(|declaration| declaration.kind)
            .or_else(|| {
                self.global_symbols
                    .compiler_known
                    .iter()
                    .find(|symbol| symbol.id.qualified_name == target)
                    .map(|symbol| symbol.kind)
            })
        {
            return class_like_symbol_kind(kind);
        }

        self.global_symbols.references.iter().any(|reference| {
            reference.source_identity == *source
                && reference.symbol_id.qualified_name == target
                && class_like_reference_role(reference.role)
        }) || self.global_symbols.unresolved.iter().any(|reference| {
            reference.source_identity == *source
                && reference.source_spelling == target
                && class_like_reference_role(reference.role)
        })
    }

    pub(crate) fn attribute_info(&self) -> &AttributeSemanticInfo {
        &self.attribute_info
    }

    pub(crate) fn attribute_parameter_occurrences(&self) -> &[AttributeParameterOccurrence] {
        &self.attribute_parameter_occurrences
    }

    pub(crate) fn namespace_hover_at_offset(&self, offset: usize) -> Option<SemanticHover> {
        let namespace = self.global_symbols.namespace_declaration.as_ref()?;
        let contains_offset = span_contains(namespace.keyword_span, offset)
            || namespace
                .name
                .segments
                .iter()
                .any(|segment| span_contains(segment.span, offset))
            || namespace
                .name
                .separator_spans
                .iter()
                .any(|span| span_contains(*span, offset));
        if !contains_offset {
            return None;
        }
        Some(SemanticHover::new(
            namespace.name.span,
            format!("Namespace `{}`", namespace.name.canonical()),
        ))
    }

    pub(crate) fn signature_help_for_incomplete_call(
        path: &str,
        text: &str,
        offset: usize,
    ) -> Option<SignatureHelp> {
        let prefix = text.get(..offset)?;
        if !matches!(prefix.trim_end().chars().last(), Some('(' | ',')) {
            return None;
        }

        let tokens = doriac::lex_source(path.to_string(), prefix.to_string()).ok()?;
        let unmatched_parens = tokens
            .iter()
            .fold(0_usize, |depth, token| match token.kind {
                TokenKind::LeftParen => depth + 1,
                TokenKind::RightParen => depth.saturating_sub(1),
                _ => depth,
            });
        if unmatched_parens == 0 {
            return None;
        }

        let suffix = text.get(offset..)?;
        let trimmed_suffix = suffix.trim_start();
        let existing_closers = trimmed_suffix
            .chars()
            .take_while(|character| *character == ')')
            .count();
        let mut insertion = "0".to_string();
        insertion.push_str(&")".repeat(unmatched_parens.saturating_sub(existing_closers)));
        if !trimmed_suffix.starts_with(')') && !trimmed_suffix.starts_with(';') {
            insertion.push(';');
        }

        let mut recovered = text.to_string();
        recovered.insert_str(offset, &insertion);
        Self::analyze(path, &recovered).signature_help_at_offset(offset)
    }

    #[cfg(test)]
    pub(crate) fn from_diagnostics(diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            diagnostics,
            ..Self::default()
        }
    }

    pub(crate) fn hover_at_offset(&self, offset: usize) -> Option<SemanticHover> {
        let occurrence_hover = self
            .occurrences
            .iter()
            .filter(|occurrence| span_contains(occurrence.span, offset))
            .min_by_key(|occurrence| occurrence.span.end.saturating_sub(occurrence.span.start))
            .and_then(|occurrence| {
                let symbol = self.symbols.get(occurrence.symbol)?;
                let mut markdown = format!("```doria\n{}\n```", symbol.signature);
                if let Some(documentation) = &symbol.documentation {
                    markdown.push_str("\n\n");
                    markdown.push_str(documentation);
                }
                Some(SemanticHover::new(occurrence.span, markdown))
            });
        let semantic_hover = self
            .semantic_hovers
            .iter()
            .filter(|hover| span_contains(hover.span, offset))
            .min_by_key(|hover| hover.selection_key())
            .cloned();

        match (occurrence_hover, semantic_hover) {
            (Some(occurrence), Some(semantic)) => {
                let occurrence_width = occurrence.span.end.saturating_sub(occurrence.span.start);
                let semantic_width = semantic.span.end.saturating_sub(semantic.span.start);
                Some(if semantic_width <= occurrence_width {
                    semantic
                } else {
                    occurrence
                })
            }
            (Some(hover), None) | (None, Some(hover)) => Some(hover),
            (None, None) => None,
        }
    }

    pub(crate) fn member_completions_at_offset(
        &self,
        offset: usize,
    ) -> Option<Vec<SemanticCompletion>> {
        let context = self
            .member_receivers
            .iter()
            .filter(|context| context.span.start <= offset && offset <= context.span.end)
            .min_by_key(|context| context.span.end.saturating_sub(context.span.start))?;
        Some(self.member_completions(context))
    }

    fn member_completions(&self, context: &MemberReceiver) -> Vec<SemanticCompletion> {
        use SharedHandleKind::*;

        let receiver = non_nullable_type(&context.receiver);
        if let ResolvedType::Enum(enum_type) = receiver {
            return self
                .enum_member_completions
                .get(&enum_type.name)
                .cloned()
                .unwrap_or_default();
        }
        if let ResolvedType::Class(class) = receiver {
            return self.class_member_completions(
                &class.name,
                context.writable_payload_access,
                context,
            );
        }
        if let Some(completions) = list_algorithm_completions(receiver) {
            return completions;
        }
        let ResolvedType::SharedHandle(kind, payload) = receiver else {
            return Vec::new();
        };
        let mut completions = match kind {
            SharedReference => vec![
                shared_method_completion(&context.receiver, "share"),
                shared_method_completion(&context.receiver, "createWeakReference"),
                SemanticCompletion {
                    label: "referencedValue".to_string(),
                    kind: 10,
                    detail: format!("{} $referencedValue", display_resolved_type(payload)),
                    documentation: Some("Readonly, allocation-free projection to the payload for resolving wrapper/member name collisions. It does not change either ownership count.".to_string()),
                },
            ],
            WeakReference => vec![shared_method_completion(&context.receiver, "acquire")],
            WritableSharedReference => vec![
                shared_method_completion(&context.receiver, "share"),
                shared_method_completion(&context.receiver, "createWeakReference"),
                shared_method_completion(&context.receiver, "acquireReadonlyAccess"),
                shared_method_completion(&context.receiver, "acquireWritableAccess"),
            ],
            WritableWeakReference => {
                vec![shared_method_completion(&context.receiver, "acquire")]
            }
            ReadonlySharedReferenceAccess | WritableSharedReferenceAccess => Vec::new(),
        };

        let forwards_payload = matches!(
            kind,
            SharedReference | ReadonlySharedReferenceAccess | WritableSharedReferenceAccess
        );
        if !forwards_payload {
            return completions;
        }
        let writable = *kind == WritableSharedReferenceAccess;
        let ResolvedType::Class(class) = payload.as_ref() else {
            return completions;
        };
        completions.extend(self.class_member_completions(&class.name, writable, context));
        let mut labels = HashSet::new();
        completions.retain(|completion| labels.insert(completion.label.clone()));
        completions
    }

    pub(crate) fn static_completions_at_offset(
        &self,
        offset: usize,
    ) -> Option<Vec<SemanticCompletion>> {
        let context = self
            .static_receivers
            .iter()
            .filter(|context| context.span.start <= offset && offset <= context.span.end)
            .min_by_key(|context| context.span.end.saturating_sub(context.span.start))?;
        Some(
            self.enum_case_completions
                .get(&context.enum_name)
                .cloned()
                .unwrap_or_default(),
        )
    }

    fn class_member_completions(
        &self,
        class_name: &str,
        writable: bool,
        context: &MemberReceiver,
    ) -> Vec<SemanticCompletion> {
        let mut completions = Vec::new();
        let mut current = Some(class_name);
        let mut visited = HashSet::new();
        while let Some(class_name) = current {
            if !visited.insert(class_name) {
                break;
            }
            if let Some(members) = self.class_members.get(class_name) {
                completions.extend(
                    members
                        .iter()
                        .filter(|member| {
                            !member.is_static
                                && (writable || !member.writable)
                                && (!member.internal
                                    || context.current_class.as_deref() == Some(class_name))
                        })
                        .map(|member| member.completion.clone()),
                );
            }
            current = self.class_parents.get(class_name).map(String::as_str);
        }
        let mut labels = HashSet::new();
        completions.retain(|completion| labels.insert(completion.label.clone()));
        completions
    }

    pub(crate) fn local_completions_at_offset(&self, offset: usize) -> Vec<LocalCompletion> {
        let mut visible = self
            .local_visibilities
            .iter()
            .filter(|visibility| visibility.start <= offset && offset <= visibility.end)
            .collect::<Vec<_>>();
        visible.sort_by_key(|visibility| (visibility.depth, visibility.start));

        let mut by_name = HashMap::new();
        for visibility in visible {
            let Some(symbol) = self.symbols.get(visibility.symbol) else {
                continue;
            };
            let Some(name) = &symbol.local_name else {
                continue;
            };
            by_name.insert(
                name.clone(),
                LocalCompletion {
                    label: format!("${name}"),
                    detail: symbol.signature.clone(),
                },
            );
        }
        let mut completions = by_name.into_values().collect::<Vec<_>>();
        completions.sort_by(|left, right| left.label.cmp(&right.label));
        completions
    }

    pub(crate) fn signature_help_at_offset(&self, offset: usize) -> Option<SignatureHelp> {
        let context = self
            .call_signatures
            .iter()
            .filter(|context| context.span.start <= offset && offset <= context.span.end)
            .min_by_key(|context| context.span.end.saturating_sub(context.span.start))?;
        let symbol = self.symbols.get(context.symbol)?;
        let active_parameter = context
            .arguments
            .iter()
            .find(|argument| offset <= argument.span.end)
            .map(|argument| argument.parameter)
            .unwrap_or(context.arguments.len());
        Some(SignatureHelp {
            label: symbol.signature.clone(),
            active_parameter,
        })
    }

    pub(crate) fn reference_spans_at_offset(
        &self,
        offset: usize,
        include_declaration: bool,
    ) -> Vec<Span> {
        let Some(symbol) = self.symbol_at_offset(offset) else {
            return Vec::new();
        };
        let mut spans = self
            .occurrences
            .iter()
            .filter(|occurrence| {
                occurrence.symbol == symbol
                    && (include_declaration || occurrence.role != OccurrenceRole::Declaration)
            })
            .map(|occurrence| occurrence.span)
            .collect::<Vec<_>>();
        spans.sort_by_key(|span| (span.start, span.end));
        spans
    }

    pub(crate) fn declaration_span_at_offset(&self, offset: usize) -> Option<Span> {
        let symbol = self.symbol_at_offset(offset)?;
        self.occurrences
            .iter()
            .find(|occurrence| {
                occurrence.symbol == symbol && occurrence.role == OccurrenceRole::Declaration
            })
            .map(|occurrence| occurrence.span)
    }

    pub(crate) fn semantic_token_spans(&self) -> Vec<SemanticTokenSpan> {
        let test_semantic_tokens = self
            .source_semantic_context
            .as_ref()
            .filter(|context| context.is_development())
            .map_or_else(Vec::new, |_| {
                test_semantic_tokens(&self.test_semantics, self.source_id)
            });
        let mut spans = self
            .occurrences
            .iter()
            .filter_map(|occurrence| {
                let symbol = self.symbols.get(occurrence.symbol)?;
                semantic_token_type(symbol).map(|token_type| (occurrence.span, token_type))
            })
            .collect::<Vec<_>>();
        spans.retain(|(span, _)| {
            !self
                .attribute_semantic_tokens
                .iter()
                .any(|(attribute_span, _)| spans_overlap(*span, *attribute_span))
        });
        spans.extend(self.directive_semantic_tokens.iter().copied());
        spans.extend(self.attribute_semantic_tokens.iter().copied());
        spans.retain(|(span, _)| {
            !test_semantic_tokens
                .iter()
                .any(|(test_span, _, _)| span == test_span)
                && !self
                    .assertion_semantic_tokens
                    .iter()
                    .any(|(assertion_span, _, _)| span == assertion_span)
        });
        let mut tokens = spans
            .into_iter()
            .map(|(span, token_type)| (span, token_type, 0))
            .chain(test_semantic_tokens)
            .chain(self.assertion_semantic_tokens.iter().copied())
            .collect::<Vec<_>>();
        tokens.sort_by_key(|(span, _, _)| (span.start, span.end));
        tokens.dedup_by_key(|(span, _, _)| (span.start, span.end));
        tokens
    }

    pub(crate) fn rename_replacement_at_offset(
        &self,
        offset: usize,
        new_name: &str,
    ) -> Option<String> {
        let symbol = self.symbols.get(self.symbol_at_offset(offset)?)?;
        Some(match symbol.kind {
            SymbolKind::Plain => new_name.to_string(),
            SymbolKind::Variable if new_name.starts_with('$') => new_name.to_string(),
            SymbolKind::Variable => format!("${new_name}"),
            SymbolKind::Keyword => return None,
        })
    }

    fn symbol_at_offset(&self, offset: usize) -> Option<usize> {
        self.occurrences
            .iter()
            .filter(|occurrence| span_contains(occurrence.span, offset))
            .min_by_key(|occurrence| occurrence.span.end.saturating_sub(occurrence.span.start))
            .map(|occurrence| occurrence.symbol)
    }
}

fn importable_reference_role(role: GlobalReferenceRole) -> bool {
    !matches!(
        role,
        GlobalReferenceRole::ImportTarget
            | GlobalReferenceRole::ImportAliasUse
            | GlobalReferenceRole::Include
    )
}

fn unresolved_import_role(code: &str) -> Option<GlobalReferenceRole> {
    match code {
        "E0401" => Some(GlobalReferenceRole::Type),
        "E0305" => Some(GlobalReferenceRole::StaticQualifier),
        "E0309" => Some(GlobalReferenceRole::FunctionCall),
        "E0491" => Some(GlobalReferenceRole::Value),
        "E0686" => Some(GlobalReferenceRole::AttributeClass),
        _ => None,
    }
}

fn class_like_symbol_kind(kind: GlobalSymbolKind) -> bool {
    matches!(
        kind,
        GlobalSymbolKind::Class
            | GlobalSymbolKind::Enum
            | GlobalSymbolKind::Interface
            | GlobalSymbolKind::Trait
            | GlobalSymbolKind::CompilerKnownType
            | GlobalSymbolKind::CompilerKnownAttribute
    )
}

fn is_member_owner_kind(kind: Option<GlobalSymbolKind>) -> bool {
    kind.is_some_and(class_like_symbol_kind)
}

fn class_like_reference_role(role: GlobalReferenceRole) -> bool {
    matches!(
        role,
        GlobalReferenceRole::Type
            | GlobalReferenceRole::Constructor
            | GlobalReferenceRole::StaticQualifier
            | GlobalReferenceRole::Extends
            | GlobalReferenceRole::Implements
            | GlobalReferenceRole::Throws
            | GlobalReferenceRole::Catch
            | GlobalReferenceRole::TypeTest
            | GlobalReferenceRole::MatchPattern
            | GlobalReferenceRole::AttributeClass
    )
}

fn directive_semantic_tokens(program: &Program) -> Vec<(Span, u32)> {
    const TYPE: u32 = 1;
    const KEYWORD: u32 = 4;
    const NAMESPACE: u32 = 5;
    const STRING: u32 = 6;

    let mut tokens = Vec::new();
    if let Some(namespace) = &program.namespace {
        tokens.push((namespace.keyword_span, KEYWORD));
        tokens.extend(
            namespace
                .name
                .segments
                .iter()
                .map(|segment| (segment.span, NAMESPACE)),
        );
        tokens.extend(
            namespace
                .name
                .separator_spans
                .iter()
                .copied()
                .map(|span| (span, NAMESPACE)),
        );
    }
    for import in &program.imports {
        tokens.push((import.keyword_span, KEYWORD));
        if let Some(prefix) = &import.prefix {
            tokens.extend(
                prefix
                    .segments
                    .iter()
                    .map(|segment| (segment.span, NAMESPACE)),
            );
            tokens.extend(
                prefix
                    .separator_spans
                    .iter()
                    .copied()
                    .map(|span| (span, NAMESPACE)),
            );
        }
        for entry in &import.entries {
            tokens.extend(
                entry
                    .target
                    .segments
                    .iter()
                    .map(|segment| (segment.span, TYPE)),
            );
            tokens.extend(
                entry
                    .target
                    .separator_spans
                    .iter()
                    .copied()
                    .map(|span| (span, NAMESPACE)),
            );
            if let Some(span) = entry.as_span {
                tokens.push((span, KEYWORD));
            }
            if let Some(alias) = &entry.alias {
                tokens.push((alias.span, TYPE));
            }
        }
    }
    for include in &program.includes {
        tokens.push((include.keyword_span, KEYWORD));
        tokens.push((include.literal_span, STRING));
    }
    tokens
}

fn semantic_token_type(symbol: &Symbol) -> Option<u32> {
    if symbol.kind == SymbolKind::Keyword {
        return Some(4);
    }
    if symbol.kind == SymbolKind::Variable {
        return Some(0);
    }
    if symbol.signature.starts_with("enum ")
        || symbol.signature.starts_with("class ")
        || symbol.signature.starts_with("interface ")
        || symbol.signature.starts_with("trait ")
    {
        return Some(1);
    }
    if symbol.signature.starts_with("function ") || symbol.signature.contains(" function ") {
        return Some(3);
    }
    if symbol.signature.contains("::") {
        return Some(2);
    }
    if symbol.signature.starts_with("match (...): ")
        || symbol.signature.starts_with("when (...): ")
        || symbol.signature == "return expression;"
        || symbol.signature == "finally { ... }"
    {
        return Some(4);
    }
    None
}

fn test_semantic_tokens(facts: &TestSemanticFacts, source_id: SourceId) -> Vec<SemanticTokenSpan> {
    let mut tokens = Vec::new();
    for suite in facts
        .suites
        .iter()
        .filter(|suite| suite.call_name_span.source == source_id)
    {
        tokens.push((
            suite.call_name_span,
            SEMANTIC_TOKEN_FUNCTION,
            SEMANTIC_TOKEN_DECLARATION,
        ));
        tokens.push((suite.description_span, SEMANTIC_TOKEN_STRING, 0));
    }
    for test in facts.tests.iter().filter(|test| {
        test.origin == doriac::testing::TestOrigin::Behavioral
            && test.call_name_span.source == source_id
    }) {
        tokens.push((
            test.call_name_span,
            SEMANTIC_TOKEN_FUNCTION,
            SEMANTIC_TOKEN_DECLARATION,
        ));
        tokens.push((test.description_span, SEMANTIC_TOKEN_STRING, 0));
    }
    tokens.sort_by_key(|(span, token_type, modifiers)| {
        (span.start, span.end, *token_type, *modifiers)
    });
    tokens.dedup();
    tokens
}

fn compiler_assertion_symbol_tokens(
    facts: &GlobalSymbolFacts,
    source_id: SourceId,
    tokens: &[Token],
) -> Vec<SemanticTokenSpan> {
    let mut tokens = facts
        .references
        .iter()
        .filter(|reference| reference.source_span.source == source_id)
        .filter_map(|reference| {
            let GlobalSymbolOwner::CompilerKnown(CompilerSymbolIdentity::StandardTest(name)) =
                &reference.symbol_id.owner
            else {
                return None;
            };
            let token_type = match name.as_str() {
                doriac::compiler_known_test::EXPECT | doriac::compiler_known_test::FAIL => {
                    SEMANTIC_TOKEN_FUNCTION
                }
                doriac::compiler_known_test::ASSERTION_ERROR => SEMANTIC_TOKEN_TYPE,
                _ => return None,
            };
            let source_name = reference
                .source_spelling
                .rsplit('\\')
                .next()
                .unwrap_or(&reference.source_spelling);
            let span = find_identifier_span(tokens, reference.source_span, source_name)
                .unwrap_or(reference.source_span);
            Some((span, token_type, 0))
        })
        .collect::<Vec<_>>();
    tokens.sort_by_key(|(span, token_type, modifiers)| {
        (span.start, span.end, *token_type, *modifiers)
    });
    tokens.dedup();
    tokens
}

struct SnapshotBuilder<'a> {
    text: &'a str,
    tokens: &'a [Token],
    source_id: SourceId,
    semantic_info: Option<&'a SemanticInfo>,
    diagnostics: Vec<Diagnostic>,
    symbols: Vec<Symbol>,
    occurrences: Vec<Occurrence>,
    classes: HashMap<String, usize>,
    error_classes: HashSet<String>,
    enums: HashMap<String, usize>,
    enum_cases: HashMap<(String, String), usize>,
    class_parents: HashMap<String, String>,
    class_members: HashMap<String, Vec<ClassMemberCompletion>>,
    class_property_symbols: HashMap<(String, String), usize>,
    class_constant_symbols: HashMap<(String, String), usize>,
    enum_case_completions: HashMap<String, Vec<SemanticCompletion>>,
    enum_member_completions: HashMap<String, Vec<SemanticCompletion>>,
    methods: HashMap<(String, String), usize>,
    functions: HashMap<String, usize>,
    member_receivers: Vec<MemberReceiver>,
    static_receivers: Vec<StaticReceiver>,
    local_scopes: Vec<HashMap<String, usize>>,
    local_scope_ends: Vec<usize>,
    local_visibilities: Vec<LocalVisibility>,
    call_signatures: Vec<CallSignatureContext>,
    semantic_hovers: Vec<SemanticHover>,
    attribute_semantic_tokens: Vec<(Span, u32)>,
    assertion_semantic_tokens: Vec<SemanticTokenSpan>,
    attribute_parameter_occurrences: Vec<AttributeParameterOccurrence>,
    missing_callables: Vec<MissingCallable>,
    member_occurrences: Vec<MemberOccurrence>,
    member_parents: Vec<MemberParent>,
    statement_expression_span: Option<Span>,
    when_depth: usize,
}

impl<'a> SnapshotBuilder<'a> {
    fn new(
        text: &'a str,
        tokens: &'a [Token],
        source_id: SourceId,
        semantic_info: Option<&'a SemanticInfo>,
        diagnostics: Vec<Diagnostic>,
    ) -> Self {
        Self {
            text,
            tokens,
            source_id,
            semantic_info,
            diagnostics,
            symbols: Vec::new(),
            occurrences: Vec::new(),
            classes: HashMap::new(),
            error_classes: HashSet::new(),
            enums: HashMap::new(),
            enum_cases: HashMap::new(),
            class_parents: HashMap::new(),
            class_members: HashMap::new(),
            class_property_symbols: HashMap::new(),
            class_constant_symbols: HashMap::new(),
            enum_case_completions: HashMap::new(),
            enum_member_completions: HashMap::new(),
            methods: HashMap::new(),
            functions: HashMap::new(),
            member_receivers: Vec::new(),
            static_receivers: Vec::new(),
            local_scopes: Vec::new(),
            local_scope_ends: Vec::new(),
            local_visibilities: Vec::new(),
            call_signatures: Vec::new(),
            semantic_hovers: Vec::new(),
            attribute_semantic_tokens: Vec::new(),
            assertion_semantic_tokens: Vec::new(),
            attribute_parameter_occurrences: Vec::new(),
            missing_callables: Vec::new(),
            member_occurrences: Vec::new(),
            member_parents: Vec::new(),
            statement_expression_span: None,
            when_depth: 0,
        }
    }

    fn build(mut self, program: &Program) -> AnalysisSnapshot {
        self.collect_declarations(program);
        self.collect_semantic_only_declarations();
        self.collect_references(program);
        self.collect_attribute_facts(program);
        self.collect_semantic_hovers();
        let attribute_info = self
            .semantic_info
            .map_or_else(AttributeSemanticInfo::default, |info| {
                info.attributes.clone()
            });
        let test_semantics = self
            .semantic_info
            .map_or_else(TestSemanticFacts::default, |info| {
                info.test_semantics.clone()
            });
        if let Some(info) = self.semantic_info {
            self.assertion_semantic_tokens
                .extend(compiler_assertion_symbol_tokens(
                    &info.global_symbols,
                    self.source_id,
                    self.tokens,
                ));
        }
        self.assertion_semantic_tokens
            .sort_by_key(|(span, token_type, modifiers)| {
                (span.start, span.end, *token_type, *modifiers)
            });
        self.assertion_semantic_tokens.dedup();
        let source_semantic_context = self
            .semantic_info
            .and_then(|info| info.source_semantic_contexts.get(&self.source_id).cloned());
        let class_declarations = program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Class(class) => Some(ClassDeclaration {
                    name: class.name.clone(),
                    span: class.span,
                }),
                _ => None,
            })
            .collect();
        AnalysisSnapshot {
            diagnostics: self.diagnostics,
            source_id: self.source_id,
            compilation_context: CompilationContext::default(),
            global_symbols: GlobalSymbolFacts::default(),
            directive_semantic_tokens: Vec::new(),
            attribute_semantic_tokens: self.attribute_semantic_tokens,
            assertion_semantic_tokens: self.assertion_semantic_tokens,
            test_semantics,
            source_semantic_context,
            attribute_info,
            attribute_parameter_occurrences: self.attribute_parameter_occurrences,
            symbols: self.symbols,
            occurrences: self.occurrences,
            member_receivers: self.member_receivers,
            static_receivers: self.static_receivers,
            class_members: self.class_members,
            class_parents: self.class_parents,
            enum_case_completions: self.enum_case_completions,
            enum_member_completions: self.enum_member_completions,
            local_visibilities: self.local_visibilities,
            call_signatures: self.call_signatures,
            semantic_hovers: self.semantic_hovers,
            missing_callables: self.missing_callables,
            class_declarations,
            member_occurrences: self.member_occurrences,
            member_parents: self.member_parents,
        }
    }

    fn collect_attribute_facts(&mut self, program: &Program) {
        const VARIABLE: u32 = 0;
        const TYPE: u32 = 1;
        const KEYWORD: u32 = 4;
        const NAMESPACE: u32 = 5;

        for attachment in &program.attributes {
            for group in &attachment.groups {
                self.attribute_semantic_tokens
                    .push((group.open_span, KEYWORD));
                self.attribute_semantic_tokens
                    .push((group.close_span, KEYWORD));
                for attribute in &group.attributes {
                    let last_segment = attribute.name.segments.len().saturating_sub(1);
                    for (index, segment) in attribute.name.segments.iter().enumerate() {
                        self.attribute_semantic_tokens.push((
                            segment.span,
                            if index == last_segment {
                                TYPE
                            } else {
                                NAMESPACE
                            },
                        ));
                    }
                    self.attribute_semantic_tokens.extend(
                        attribute
                            .name
                            .separator_spans
                            .iter()
                            .copied()
                            .map(|span| (span, NAMESPACE)),
                    );
                    if let Some(arguments) = &attribute.argument_list {
                        for argument in &arguments.arguments {
                            if let Some(name) = &argument.name {
                                self.attribute_semantic_tokens.push((name.span, VARIABLE));
                            }
                            self.visit_expr(&argument.value, None, None);
                        }
                    }
                    if attribute.name.canonical() == "Attribute" {
                        self.semantic_hovers.push(SemanticHover::new(
                            attribute.name.span,
                            "**Attribute - Compiler-Known Attribute Metadata**\n\nMarks a readonly, non-generic class as a typed attribute schema. Attribute constructors are not executed while metadata is produced.\n\n**Metadata Only**\n\n**No Runtime Reflection**".to_string(),
                        ));
                    }
                }
            }
        }

        let Some(info) = self.semantic_info else {
            return;
        };

        for schema in &info.attributes.schemas {
            let AttributeClassIdentity::User(class) = &schema.identity else {
                continue;
            };
            let Some(declaration) = info
                .global_symbols
                .declarations
                .iter()
                .find(|declaration| declaration.id == *class)
                .filter(|declaration| declaration.name_span.source == self.source_id)
            else {
                continue;
            };
            if let Some(occurrence) = self.occurrences.iter().find(|occurrence| {
                occurrence.role == OccurrenceRole::Declaration
                    && occurrence.span == declaration.name_span
            }) {
                let documentation = attribute_schema_documentation(schema);
                append_documentation(
                    &mut self.symbols[occurrence.symbol].documentation,
                    &documentation,
                );
            }

            for parameter in &schema.parameters {
                if parameter.span.source != self.source_id {
                    continue;
                }
                let Some(name_span) = self.tokens.iter().find_map(|token| {
                    (parameter.span.start <= token.span.start
                        && token.span.end <= parameter.span.end
                        && matches!(&token.kind, TokenKind::Variable(name) if name == &parameter.name))
                    .then_some(token.span)
                }) else {
                    continue;
                };
                let identity = AttributeParameterIdentity {
                    class: class.clone(),
                    index: parameter.index,
                };
                let symbol = self
                    .occurrences
                    .iter()
                    .find(|occurrence| {
                        occurrence.role == OccurrenceRole::Declaration
                            && occurrence.span == name_span
                    })
                    .map(|occurrence| occurrence.symbol);
                if let Some(symbol) = symbol {
                    self.attribute_parameter_occurrences.extend(
                        self.occurrences
                            .iter()
                            .filter(|occurrence| occurrence.symbol == symbol)
                            .map(|occurrence| AttributeParameterOccurrence {
                                identity: identity.clone(),
                                name: parameter.name.clone(),
                                span: occurrence.span,
                                declaration: occurrence.role == OccurrenceRole::Declaration,
                                spelling: AttributeParameterSpelling::Variable,
                            }),
                    );
                } else {
                    self.attribute_parameter_occurrences
                        .push(AttributeParameterOccurrence {
                            identity,
                            name: parameter.name.clone(),
                            span: name_span,
                            declaration: true,
                            spelling: AttributeParameterSpelling::Variable,
                        });
                }
            }
        }

        for application in info
            .attributes
            .applications
            .iter()
            .filter(|application| application.span.source == self.source_id)
        {
            let authored = program
                .attributes
                .iter()
                .flat_map(|attachment| &attachment.groups)
                .flat_map(|group| &group.attributes)
                .find(|attribute| attribute.span == application.span);
            let Some(authored) = authored else {
                continue;
            };
            self.semantic_hovers.push(SemanticHover::new(
                authored.name.span,
                attribute_application_documentation(application),
            ));

            let AttributeClassIdentity::User(class) = &application.class_identity else {
                continue;
            };
            let Some(arguments) = &authored.argument_list else {
                continue;
            };
            for authored_argument in &application.authored_arguments {
                let Some(parameter_name) = authored_argument.name.as_ref() else {
                    continue;
                };
                let Some(name_span) = arguments
                    .arguments
                    .get(authored_argument.index)
                    .and_then(|argument| argument.name.as_ref())
                    .map(|name| name.span)
                else {
                    continue;
                };
                self.attribute_parameter_occurrences
                    .push(AttributeParameterOccurrence {
                        identity: AttributeParameterIdentity {
                            class: class.clone(),
                            index: authored_argument.bound_parameter_index,
                        },
                        name: parameter_name.clone(),
                        span: name_span,
                        declaration: false,
                        spelling: AttributeParameterSpelling::Label,
                    });
            }
        }
    }

    fn collect_semantic_hovers(&mut self) {
        let Some(info) = self.semantic_info else {
            return;
        };
        let mut hovers = Vec::new();

        let mut function_types = info.function_types_by_span.iter().collect::<Vec<_>>();
        function_types.sort_by_key(|(span, _)| **span);
        for (span, semantic) in function_types
            .into_iter()
            .filter(|(span, _)| span.source == self.source_id)
        {
            let ambient = ambient_effect_documentation(&semantic.ambient_checked_effects);
            hovers.push(SemanticHover::new(
                *span,
                format!(
                    "```doria\n{}\n```\n\nCanonical semantic function type.{ambient}",
                    display_function_type_with_effects(
                        &semantic.ty,
                        &semantic.authored_checked_effects,
                    )
                ),
            ));
        }

        let mut narrowed_function_uses = info
            .expression_types
            .keys()
            .filter(|span| span.source == self.source_id)
            .filter_map(|span| narrowed_function_type_for_use(info, span).map(|ty| (span, ty)))
            .collect::<Vec<_>>();
        narrowed_function_uses.sort_by_key(|(span, _)| **span);
        for (span, ty) in narrowed_function_uses {
            let span = *span;
            let target_capabilities = self.target_capabilities_for(span);
            hovers.push(SemanticHover::new(
                span,
                format!(
                    "```doria\n{}\n```\n\nCompiler-resolved function value after flow narrowing.{target_capabilities}",
                    display_resolved_type(ty),
                ),
            ));
        }

        let mut bindings = info
            .binding_resolution
            .declarations_by_id
            .values()
            .filter_map(|declaration| {
                let span = declaration.span?;
                let ty = declaration.source_type.as_ref()?;
                (span.source == self.source_id
                    && matches!(non_nullable_type(ty), ResolvedType::Function(_)))
                .then_some((
                    declaration.id,
                    span,
                    ty,
                    declaration.kind,
                    declaration.ownership,
                    declaration.name.as_str(),
                ))
            })
            .collect::<Vec<_>>();
        bindings.sort_by_key(|(id, span, _, _, _, _)| (span.start, span.end, *id));
        for (binding_id, declaration_span, ty, kind, ownership, declaration_name) in bindings {
            let name = binding_source_name(info, binding_id);
            let mut markdown = format!(
                "```doria\n{} {name}\n```\n\nSemantically resolved function-typed binding.",
                display_resolved_type(ty),
            );
            if matches!(
                kind,
                BindingKind::FunctionParameter
                    | BindingKind::MethodParameter
                    | BindingKind::ClosureParameter
            ) {
                markdown.push_str(match ownership {
                    BindingOwnership::Owned => "\n\n**Ownership:** Owned Callback Parameter",
                    BindingOwnership::ReadonlyBorrow | BindingOwnership::WritableBorrow => {
                        "\n\n**Ownership:** Nonescaping Callback Parameter"
                    }
                });
            }
            let hover_span = self
                .tokens
                .iter()
                .find(|token| {
                    declaration_span.start <= token.span.start
                        && token.span.end <= declaration_span.end
                        && matches!(&token.kind, TokenKind::Variable(name) if name == declaration_name)
                })
                .map_or(declaration_span, |token| token.span);
            hovers.push(SemanticHover::new(hover_span, markdown.clone()));
            let mut use_spans = info
                .binding_resolution
                .uses_by_span
                .iter()
                .filter_map(|(span, resolved)| {
                    (*resolved == binding_id && span.source == self.source_id).then_some(*span)
                })
                .collect::<Vec<_>>();
            use_spans.sort_by_key(|span| (span.start, span.end));
            hovers.extend(
                use_spans
                    .into_iter()
                    .map(|span| SemanticHover::new(span, markdown.clone())),
            );
        }

        let mut closures = info.closures.values().collect::<Vec<_>>();
        closures.sort_by_key(|closure| (closure.closure_id.start, closure.closure_id.end));
        for closure in closures
            .into_iter()
            .filter(|closure| closure.closure_id.source == self.source_id)
        {
            let closure_span = Span::in_source(
                closure.closure_id.source,
                closure.closure_id.start,
                closure.closure_id.end,
            );
            let target_capabilities = self.target_capabilities_for(closure_span);
            let ownership = info.closure_ownership.get(&closure.closure_id);
            let signature = display_function_type_with_effects(
                &closure.function_type,
                &closure.required_checked_effects,
            );
            let effects = effect_list(&closure.required_checked_effects);
            let ambient = ambient_effect_documentation(&closure.ambient_checked_effects);
            let captures = if closure.captures.is_empty() {
                "none".to_string()
            } else {
                closure
                    .captures
                    .iter()
                    .map(|capture| {
                        let acquisition = ownership.and_then(|ownership| {
                            ownership.acquisitions.iter().find(|acquisition| {
                                acquisition.environment_binding_id == capture.environment_binding_id
                            })
                        });
                        format!(
                            "{} capture of `{}`",
                            acquisition.map_or("Capture", |acquisition| {
                                capture_acquisition_name(acquisition.kind)
                            }),
                            binding_source_name(info, capture.source_binding_id)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let ownership_summary = ownership.map_or_else(
                || "Not available because ownership checking did not complete.".to_string(),
                |ownership| closure_provenance_summary(info, &ownership.provenance),
            );
            let invocation = ownership.map_or("Not available", |ownership| {
                closure_invocation_summary(
                    closure.inferred_invocation_mode,
                    ownership.invocation_consumption,
                )
            });
            let escape = ownership.map_or_else(
                || "Not available".to_string(),
                |ownership| closure_escape_summary(info, ownership.escape, &ownership.provenance),
            );
            hovers.push(SemanticHover::new(
                closure_span,
                format!(
                    "```doria\n{signature}\n```\n\n**Inferred invocation mode:** `{}`\n\n**Required checked effects:** {effects}{ambient}\n\n**Ownership:** {ownership_summary}\n\n**Invocation:** {invocation}\n\n**Escape:** {escape}\n\n**Captures:** {captures}{target_capabilities}",
                    invocation_mode_name(closure.inferred_invocation_mode),
                ),
            ));

            for capture in &closure.captures {
                let name = binding_source_name(info, capture.source_binding_id);
                let acquisition = ownership.and_then(|ownership| {
                    ownership.acquisitions.iter().find(|acquisition| {
                        acquisition.environment_binding_id == capture.environment_binding_id
                    })
                });
                let capture_kind = acquisition.map_or("Compiler-resolved", |acquisition| {
                    capture_acquisition_name(acquisition.kind)
                });
                let declaration_markdown = format!(
                    "```doria\n{} {}\n```\n\n{} capture of `{name}`.",
                    display_resolved_type(&capture.source_type),
                    name,
                    capture_kind,
                );
                hovers.push(SemanticHover::capture(
                    capture.declaration_span,
                    declaration_markdown,
                ));
                for span in &capture.use_spans {
                    let narrowed = narrowed_function_type_for_use(info, span);
                    let ty = narrowed.unwrap_or(&capture.source_type);
                    let mut markdown = format!(
                        "```doria\n{} {}\n```\n\n{} capture of `{name}`.",
                        display_resolved_type(ty),
                        name,
                        capture_kind,
                    );
                    if narrowed.is_some() {
                        markdown
                            .push_str("\n\nCompiler-resolved function value after flow narrowing.");
                        markdown.push_str(&self.target_capabilities_for(*span));
                    }
                    hovers.push(SemanticHover::capture(*span, markdown));
                }
            }
        }

        let mut callable_calls = info.callable_value_calls.iter().collect::<Vec<_>>();
        callable_calls.sort_by_key(|(span, _)| **span);
        for (span, call) in callable_calls
            .into_iter()
            .filter(|(span, _)| span.source == self.source_id)
        {
            let call_span = *span;
            let target_capabilities = self.target_capabilities_for(call_span);
            let ambient = ambient_effect_documentation(&call.ambient_checked_effects);
            hovers.push(SemanticHover::new(
                call_span,
                format!(
                    "```doria\n{}\n```\n\nSemantically checked callable-value invocation returning `{}`.{ambient}{target_capabilities}",
                    display_function_type_with_effects(
                        &call.function_type,
                        &call.required_checked_effects,
                    ),
                    display_resolved_type(&call.return_type),
                ),
            ));
        }

        self.semantic_hovers.extend(hovers);
    }

    fn target_capabilities_for(&self, span: Span) -> String {
        // Partial facts remain useful, but an error within this exact semantic
        // construct means the compiler has not proved that construct executable.
        if self.diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Error
                && diagnostic_overlaps_span(diagnostic, span)
        }) {
            String::new()
        } else {
            format!("\n\n{}", closure_target_capabilities())
        }
    }

    fn collect_declarations(&mut self, program: &Program) {
        for item in &program.items {
            match item {
                Item::Class(class) => self.collect_class(class),
                Item::Enum(enum_decl) => self.collect_enum(enum_decl),
                Item::Trait(trait_decl) => {
                    for member in &trait_decl.members {
                        if let ClassMember::Method(method) = member {
                            self.collect_method(&trait_decl.name, method);
                        }
                    }
                }
                Item::Function(function) => {
                    let selection_span = self.declaration_name_span(
                        function.span,
                        &function.name,
                        TokenKind::Function,
                    );
                    let mut documentation = phpdoc_before(self.text, function.span.start);
                    self.append_callable_effect_documentation(&mut documentation, function.span);
                    let symbol = self.add_declaration_symbol(
                        selection_span,
                        function_signature(function, None),
                        documentation,
                        SymbolKind::Plain,
                    );
                    self.record_callable_parameters(symbol, function);
                    self.functions.insert(function.name.clone(), symbol);
                }
                _ => {}
            }
        }
    }

    fn collect_semantic_only_declarations(&mut self) {
        let Some(info) = self.semantic_info else {
            return;
        };
        let enums = info
            .enums
            .iter()
            .filter(|info| !self.enums.contains_key(&info.name))
            .cloned()
            .collect::<Vec<_>>();
        let classes = info
            .classes
            .iter()
            .filter(|info| !self.classes.contains_key(&info.declaration_name))
            .cloned()
            .collect::<Vec<_>>();

        for info in enums {
            let symbol = self.add_metadata_symbol(
                format!("enum {}", info.name),
                Some(enum_documentation(&info)),
                SymbolKind::Plain,
            );
            self.enums.insert(info.name.clone(), symbol);

            let mut completions = Vec::new();
            for case in &info.cases {
                let signature = enum_case_signature(&info.name, &case.name, Some(case));
                let documentation = enum_case_documentation(Some(&info), Some(case));
                let case_symbol = self.add_metadata_symbol(
                    signature.clone(),
                    documentation.clone(),
                    SymbolKind::Plain,
                );
                self.enum_cases
                    .insert((info.name.clone(), case.name.clone()), case_symbol);
                completions.push(SemanticCompletion {
                    label: case.name.clone(),
                    kind: 20,
                    detail: signature,
                    documentation,
                });
            }
            self.enum_case_completions
                .insert(info.name.clone(), completions);

            if let Some(backing_type) = info.backing_type {
                let backing = enum_backing_name(backing_type).to_string();
                self.enum_member_completions.insert(
                    info.name,
                    vec![SemanticCompletion {
                        label: "value".to_string(),
                        kind: 10,
                        detail: format!("{backing} $value"),
                        documentation: Some(
                            "Readonly backing value associated with this enum case.".to_string(),
                        ),
                    }],
                );
            }
        }

        for info in classes {
            let class_name = info.declaration_name;
            let symbol = self.add_metadata_symbol(
                format!("class {class_name}"),
                Some("Compiler-known Doria class supplied by semantic analysis.".to_string()),
                SymbolKind::Plain,
            );
            self.classes.insert(class_name.clone(), symbol);

            for property in info.properties {
                let detail = format!("{} ${}", display_resolved_type(&property.ty), property.name);
                let mutability = if property.writable {
                    "writable"
                } else {
                    "readonly"
                };
                let documentation = Some(format!("Compiler-known {mutability} Doria property."));
                let property_symbol = self.add_metadata_symbol(
                    detail.clone(),
                    documentation.clone(),
                    SymbolKind::Variable,
                );
                self.class_property_symbols
                    .insert((class_name.clone(), property.name.clone()), property_symbol);
                self.class_members
                    .entry(class_name.clone())
                    .or_default()
                    .push(ClassMemberCompletion {
                        completion: SemanticCompletion {
                            label: property.name,
                            kind: 10,
                            detail,
                            documentation,
                        },
                        writable: false,
                        internal: false,
                        is_static: false,
                    });
            }
        }
    }

    fn collect_enum(&mut self, declaration: &EnumDecl) {
        let semantic = self
            .semantic_info
            .and_then(|info| info.enums.iter().find(|info| info.name == declaration.name))
            .cloned();
        let symbol = self.add_declaration_symbol(
            declaration.name_span,
            format!("enum {}", declaration.name),
            semantic.as_ref().map(enum_documentation),
            SymbolKind::Plain,
        );
        self.enums.insert(declaration.name.clone(), symbol);

        let mut completions = Vec::new();
        for case in &declaration.cases {
            let semantic_case = semantic
                .as_ref()
                .and_then(|info| info.cases.iter().find(|info| info.name == case.name));
            let signature = enum_case_signature(&declaration.name, &case.name, semantic_case);
            let documentation = enum_case_documentation(semantic.as_ref(), semantic_case);
            let case_symbol = self.add_declaration_symbol(
                case.name_span,
                signature.clone(),
                documentation.clone(),
                SymbolKind::Plain,
            );
            self.enum_cases
                .insert((declaration.name.clone(), case.name.clone()), case_symbol);
            self.record_member_occurrence(
                &declaration.name,
                &case.name,
                MemberKind::EnumCase,
                case.name_span,
                true,
            );
            completions.push(SemanticCompletion {
                label: case.name.clone(),
                kind: 20,
                detail: signature,
                documentation,
            });

            if let Some(semantic_case) = semantic_case {
                for (field, semantic_field) in case.payload.iter().zip(&semantic_case.payload) {
                    let field_span = find_variable_span(self.tokens, field.span, &field.name)
                        .unwrap_or(field.span);
                    self.add_declaration_symbol(
                        field_span,
                        format!(
                            "{} ${}",
                            display_resolved_type(&semantic_field.ty),
                            field.name
                        ),
                        Some("Readonly enum payload field.".to_string()),
                        SymbolKind::Variable,
                    );
                }
            }
        }
        self.enum_case_completions
            .insert(declaration.name.clone(), completions);

        if let Some(backing_type) = semantic.as_ref().and_then(|info| info.backing_type) {
            let backing = enum_backing_name(backing_type).to_string();
            self.enum_member_completions.insert(
                declaration.name.clone(),
                vec![SemanticCompletion {
                    label: "value".to_string(),
                    kind: 10,
                    detail: format!("{backing} $value"),
                    documentation: Some(
                        "Readonly backing value associated with this enum case.".to_string(),
                    ),
                }],
            );
        }
    }

    fn collect_class(&mut self, class: &ClassDecl) {
        let selection_span = self.declaration_name_span(class.span, &class.name, TokenKind::Class);
        let conforms_to_error = class
            .implements
            .iter()
            .any(|interface| interface == "Error");
        let mut documentation = phpdoc_before(self.text, class.span.start);
        if conforms_to_error {
            append_documentation(
                &mut documentation,
                "Explicitly conforms to the compiler-known `Error` interface. Its externally accessible readonly `string $message` property describes the checked error.",
            );
            self.error_classes.insert(class.name.clone());
        }
        let symbol = self.add_declaration_symbol(
            selection_span,
            class_signature(class),
            documentation,
            SymbolKind::Plain,
        );
        self.classes.insert(class.name.clone(), symbol);
        if let Some(parent) = &class.parent {
            self.class_parents
                .insert(class.name.clone(), parent.clone());
            if let (Some(child), Some(parent)) = (
                self.member_owner(&class.name, class.name_span, true),
                class
                    .parent_span
                    .and_then(|span| self.member_owner(parent, span, false)),
            ) {
                self.member_parents.push(MemberParent { child, parent });
            }
        }

        for member in &class.members {
            match member {
                ClassMember::Method(method) => {
                    self.collect_method(&class.name, method);
                    self.class_members
                        .entry(class.name.clone())
                        .or_default()
                        .push(ClassMemberCompletion {
                            completion: SemanticCompletion {
                                label: method.name.clone(),
                                kind: 2,
                                detail: function_signature(method, Some(&class.name)),
                                documentation: phpdoc_before(self.text, method.span.start),
                            },
                            writable: method.writable_this,
                            internal: matches!(method.access, MemberAccess::Internal),
                            is_static: method.is_static,
                        });
                }
                ClassMember::Property(property) => {
                    let selection_span =
                        find_variable_span(self.tokens, property.span, &property.name)
                            .unwrap_or(property.span);
                    let documentation = if conforms_to_error && property.name == "message" {
                        Some("Required externally accessible readonly message for the compiler-known `Error` contract.".to_string())
                    } else {
                        phpdoc_before(self.text, property.span.start)
                    };
                    let property_symbol = self.add_declaration_symbol(
                        selection_span,
                        format!("{} ${}", property.ty, property.name),
                        documentation.clone(),
                        SymbolKind::Variable,
                    );
                    self.class_property_symbols
                        .insert((class.name.clone(), property.name.clone()), property_symbol);
                    self.record_member_occurrence(
                        &class.name,
                        &property.name,
                        MemberKind::Property,
                        selection_span,
                        true,
                    );
                    self.class_members
                        .entry(class.name.clone())
                        .or_default()
                        .push(ClassMemberCompletion {
                            completion: SemanticCompletion {
                                label: property.name.clone(),
                                kind: 10,
                                detail: format!("{} ${}", property.ty, property.name),
                                documentation,
                            },
                            // Property completion represents a read. Mutability matters only
                            // when the property is used as an assignment place.
                            writable: false,
                            internal: matches!(property.access, MemberAccess::Internal),
                            is_static: property.is_static,
                        });
                }
                ClassMember::Constant(constant) => {
                    let constant_symbol = self.add_declaration_symbol(
                        constant.name_span,
                        format!("{}::{}", class.name, constant.name),
                        phpdoc_before(self.text, constant.span.start),
                        SymbolKind::Plain,
                    );
                    self.class_constant_symbols
                        .insert((class.name.clone(), constant.name.clone()), constant_symbol);
                    self.record_member_occurrence(
                        &class.name,
                        &constant.name,
                        MemberKind::Constant,
                        constant.name_span,
                        true,
                    );
                }
            }
        }
    }

    fn collect_method(&mut self, class_name: &str, method: &FunctionDecl) {
        let selection_span =
            self.declaration_name_span(method.span, &method.name, TokenKind::Function);
        let mut documentation = phpdoc_before(self.text, method.span.start);
        self.append_callable_effect_documentation(&mut documentation, method.span);
        let symbol = self.add_declaration_symbol(
            selection_span,
            function_signature(method, Some(class_name)),
            documentation,
            SymbolKind::Plain,
        );
        self.record_callable_parameters(symbol, method);
        self.methods
            .insert((class_name.to_string(), method.name.clone()), symbol);
        self.record_member_occurrence(
            class_name,
            &method.name,
            MemberKind::Method,
            selection_span,
            true,
        );
    }

    fn append_callable_effect_documentation(
        &self,
        documentation: &mut Option<String>,
        declaration: Span,
    ) {
        let Some(effects) = self
            .semantic_info
            .and_then(|info| info.callable_effective_checked_effects.get(&declaration))
        else {
            return;
        };
        let profile = doriac::CheckedEffectProfile::classify(effects.iter().cloned());
        let required = required_effect_documentation(&profile.required);
        let ambient = ambient_effect_documentation(&profile.ambient);
        let test_assertion = test_assertion_effect_documentation(&profile.test_assertion);
        if required.is_empty() && ambient.is_empty() && test_assertion.is_empty() {
            return;
        }
        let effects = [required, ambient, test_assertion]
            .into_iter()
            .filter(|section| !section.is_empty())
            .map(|section| section.trim().to_string())
            .collect::<Vec<_>>()
            .join("\n\n");
        append_documentation(documentation, &effects);
    }

    fn add_declaration_symbol(
        &mut self,
        selection_span: Span,
        signature: String,
        documentation: Option<String>,
        kind: SymbolKind,
    ) -> usize {
        self.add_symbol(
            selection_span,
            signature,
            documentation,
            kind,
            OccurrenceRole::Declaration,
        )
    }

    fn add_metadata_symbol(
        &mut self,
        signature: String,
        documentation: Option<String>,
        kind: SymbolKind,
    ) -> usize {
        let symbol = self.symbols.len();
        self.symbols.push(Symbol {
            signature,
            documentation,
            local_name: None,
            parameter_names: Vec::new(),
            kind,
        });
        symbol
    }

    fn add_reference_symbol(
        &mut self,
        selection_span: Span,
        signature: String,
        documentation: Option<String>,
        kind: SymbolKind,
    ) -> usize {
        self.add_symbol(
            selection_span,
            signature,
            documentation,
            kind,
            OccurrenceRole::Reference,
        )
    }

    fn add_symbol(
        &mut self,
        selection_span: Span,
        signature: String,
        documentation: Option<String>,
        kind: SymbolKind,
        role: OccurrenceRole,
    ) -> usize {
        let symbol = self.symbols.len();
        self.symbols.push(Symbol {
            signature,
            documentation,
            local_name: None,
            parameter_names: Vec::new(),
            kind,
        });
        self.occurrences.push(Occurrence {
            span: selection_span,
            symbol,
            role,
        });
        symbol
    }

    fn record_reference(&mut self, span: Span, symbol: usize) {
        self.occurrences.push(Occurrence {
            span,
            symbol,
            role: OccurrenceRole::Reference,
        });
    }

    fn record_member_occurrence(
        &mut self,
        owner_name: &str,
        name: &str,
        kind: MemberKind,
        span: Span,
        declaration: bool,
    ) {
        let Some(owner) = self.member_owner(owner_name, span, declaration) else {
            return;
        };
        let kind = if kind == MemberKind::Constant
            && self.semantic_info.is_some_and(|info| {
                info.global_symbols.declarations.iter().any(|declaration| {
                    declaration.id == owner && declaration.kind == GlobalSymbolKind::Enum
                })
            }) {
            MemberKind::EnumCase
        } else {
            kind
        };
        self.member_occurrences.push(MemberOccurrence {
            identity: MemberIdentity {
                owner,
                name: name.to_string(),
                kind,
            },
            span,
            declaration,
        });
    }

    fn member_owner(
        &self,
        owner_name: &str,
        span: Span,
        declaration: bool,
    ) -> Option<GlobalSymbolId> {
        let facts = &self.semantic_info?.global_symbols;
        if !declaration {
            if let Some(reference) = facts.references.iter().find(|reference| {
                reference.source_span.source == span.source
                    && (span_contains(span, reference.source_span.start)
                        || (reference.role == GlobalReferenceRole::StaticQualifier
                            && reference.source_span.end <= span.start
                            && span.start.saturating_sub(reference.source_span.end) <= 2))
                    && is_member_owner_kind(
                        facts
                            .declarations
                            .iter()
                            .find(|candidate| candidate.id == reference.symbol_id)
                            .map(|candidate| candidate.kind),
                    )
            }) {
                return Some(reference.symbol_id.clone());
            }
        }
        facts
            .declarations
            .iter()
            .filter(|candidate| is_member_owner_kind(Some(candidate.kind)))
            .find(|candidate| {
                candidate.qualified_name == owner_name
                    || (candidate.name_span.source == self.source_id
                        && candidate.source_name == owner_name)
            })
            .map(|candidate| candidate.id.clone())
            .or_else(|| {
                facts
                    .compiler_known
                    .iter()
                    .find(|candidate| candidate.id.qualified_name == owner_name)
                    .map(|candidate| candidate.id.clone())
            })
    }

    fn record_callable_parameters(&mut self, symbol: usize, function: &FunctionDecl) {
        self.symbols[symbol].parameter_names = function
            .params
            .iter()
            .map(|parameter| parameter.name.clone())
            .collect();
    }

    fn record_call_signature(&mut self, span: Span, args: &[doriac::ast::Argument], symbol: usize) {
        let parameter_names = &self.symbols[symbol].parameter_names;
        let parameter_name_refs = parameter_names
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let argument_names = args
            .iter()
            .map(|argument| argument.name.as_ref().map(|name| name.text.as_str()))
            .collect::<Vec<_>>();
        let bound = doriac::arg_binding::bind_arguments(
            &parameter_name_refs,
            &vec![false; parameter_name_refs.len()],
            &argument_names,
        );
        self.call_signatures.push(CallSignatureContext {
            span,
            arguments: args
                .iter()
                .enumerate()
                .map(|(index, argument)| CallArgumentContext {
                    span: argument.span,
                    parameter: bound.arg_to_param[index].unwrap_or(index),
                })
                .collect(),
            symbol,
        });
    }

    fn declaration_name_span(
        &self,
        declaration_span: Span,
        name: &str,
        keyword: TokenKind,
    ) -> Span {
        let mut saw_keyword = false;
        for token in tokens_in_span(self.tokens, declaration_span) {
            if same_token_variant(&token.kind, &keyword) {
                saw_keyword = true;
                continue;
            }
            if saw_keyword && identifier_is(token, name) {
                return token.span;
            }
        }
        find_identifier_span(self.tokens, declaration_span, name).unwrap_or(declaration_span)
    }

    fn collect_references(&mut self, program: &Program) {
        self.push_local_scope(self.text.len());
        for item in &program.items {
            match item {
                Item::Class(class) => {
                    if class
                        .implements
                        .iter()
                        .any(|interface| interface == "Error")
                    {
                        if let Some(span) = find_identifier_span(self.tokens, class.span, "Error") {
                            self.add_error_type_reference(span);
                        }
                    }
                    for member in &class.members {
                        if let ClassMember::Method(method) = member {
                            self.visit_function_body(
                                method,
                                Some(&class.name),
                                class.parent.as_deref(),
                            );
                        }
                        if let ClassMember::Property(property) = member {
                            if let Some(initializer) = &property.initializer {
                                self.visit_expr(
                                    initializer,
                                    Some(&class.name),
                                    class.parent.as_deref(),
                                );
                            }
                        }
                        if let ClassMember::Constant(constant) = member {
                            self.visit_expr(
                                &constant.initializer,
                                Some(&class.name),
                                class.parent.as_deref(),
                            );
                        }
                    }
                }
                Item::Enum(enum_decl) => {
                    for case in &enum_decl.cases {
                        if let Some(backing_value) = &case.backing_value {
                            self.visit_expr(backing_value, None, None);
                        }
                    }
                }
                Item::Trait(trait_decl) => {
                    for member in &trait_decl.members {
                        if let ClassMember::Method(method) = member {
                            self.visit_function_body(method, Some(&trait_decl.name), None);
                        }
                    }
                }
                Item::Function(function) => self.visit_function_body(function, None, None),
                Item::Constant(constant) => self.visit_expr(&constant.initializer, None, None),
                Item::Statement(statement) => self.visit_stmt(statement, None, None),
                _ => {}
            }
        }
        self.pop_local_scope();
    }

    fn visit_function_body(
        &mut self,
        function: &FunctionDecl,
        current_class: Option<&str>,
        parent_class: Option<&str>,
    ) {
        let block = &function.body;
        self.push_local_scope(block.span.end);
        if let Some(throws) = &function.throws {
            self.add_reference_symbol(
                throws.keyword_span,
                "throws checked errors".to_string(),
                Some("Declares the checked errors that may leave this callable.".to_string()),
                SymbolKind::Keyword,
            );
            for entry in &throws.entries {
                self.record_type_reference(&entry.ty, entry.span);
            }
        }
        for parameter in &function.params {
            self.declare_parameter(parameter, block.span.start, current_class);
        }
        for statement in &block.statements {
            self.visit_stmt(statement, current_class, parent_class);
        }
        self.pop_local_scope();
    }

    fn visit_block(
        &mut self,
        block: &Block,
        current_class: Option<&str>,
        parent_class: Option<&str>,
    ) {
        self.push_local_scope(block.span.end);
        for statement in &block.statements {
            self.visit_stmt(statement, current_class, parent_class);
        }
        self.pop_local_scope();
    }

    fn visit_stmt(
        &mut self,
        statement: &Stmt,
        current_class: Option<&str>,
        parent_class: Option<&str>,
    ) {
        match statement {
            Stmt::VarDecl(declaration) => {
                self.visit_local_declaration(declaration, current_class, parent_class)
            }
            Stmt::Assignment(assignment) => {
                self.visit_expr(&assignment.target, current_class, parent_class);
                self.visit_expr(&assignment.value, current_class, parent_class);
            }
            Stmt::Echo { expr, .. } => self.visit_expr(expr, current_class, parent_class),
            Stmt::Return { expr, span } => {
                if self.when_depth > 0 {
                    if let Some(keyword) = tokens_in_span(self.tokens, *span)
                        .find(|token| matches!(token.kind, TokenKind::Return))
                    {
                        self.add_reference_symbol(
                            keyword.span,
                            "return expression;".to_string(),
                            Some(
                                "Yields a value from the nearest enclosing `when` expression."
                                    .to_string(),
                            ),
                            SymbolKind::Plain,
                        );
                    }
                }
                if let Some(expr) = expr {
                    self.visit_expr(expr, current_class, parent_class);
                }
            }
            Stmt::Throw(statement) => {
                self.add_reference_symbol(
                    statement.keyword_span,
                    "throw Error;".to_string(),
                    Some(
                        "Transfers ownership of one explicit `Error` value to the current callable's checked-error effect."
                            .to_string(),
                    ),
                    SymbolKind::Plain,
                );
                self.visit_expr(&statement.expr, current_class, parent_class);
            }
            Stmt::Try(try_statement) => {
                self.visit_try_statement(try_statement, current_class, parent_class)
            }
            Stmt::If(if_statement) => {
                self.visit_if_statement(if_statement, current_class, parent_class)
            }
            Stmt::While(while_statement) => {
                self.visit_while_statement(while_statement, current_class, parent_class)
            }
            Stmt::DoWhile(do_while) => {
                self.visit_do_while_statement(do_while, current_class, parent_class)
            }
            Stmt::For(for_statement) => {
                self.push_local_scope(for_statement.span.end);
                if let Some(initializer) = &for_statement.initializer {
                    if let ForInitializer::VarDecl(declaration) = initializer {
                        self.visit_local_declaration(declaration, current_class, parent_class);
                    }
                    if let ForInitializer::Assignment(assignment) = initializer {
                        self.visit_expr(&assignment.target, current_class, parent_class);
                        self.visit_expr(&assignment.value, current_class, parent_class);
                    }
                }
                if let Some(condition) = &for_statement.condition {
                    self.visit_expr(condition, current_class, parent_class);
                }
                if let Some(increment) = &for_statement.increment {
                    if let ForIncrement::Increment(increment) = increment {
                        self.visit_expr(&increment.target, current_class, parent_class);
                    }
                    if let ForIncrement::Assignment(assignment) = increment {
                        self.visit_expr(&assignment.target, current_class, parent_class);
                        self.visit_expr(&assignment.value, current_class, parent_class);
                    }
                }
                self.visit_block(&for_statement.body, current_class, parent_class);
                self.pop_local_scope();
            }
            Stmt::Foreach(foreach) => {
                self.visit_expr(&foreach.iterable, current_class, parent_class);
                self.visit_block(&foreach.body, current_class, parent_class);
            }
            Stmt::Block(block) => self.visit_block(block, current_class, parent_class),
            Stmt::Increment(increment) => {
                self.visit_expr(&increment.target, current_class, parent_class)
            }
            Stmt::Expr { expr, .. } => {
                let previous = self.statement_expression_span.replace(expr.span());
                self.visit_expr(expr, current_class, parent_class);
                self.statement_expression_span = previous;
            }
            _ => {}
        }
    }

    fn visit_try_statement(
        &mut self,
        statement: &TryStmt,
        current_class: Option<&str>,
        parent_class: Option<&str>,
    ) {
        self.add_reference_symbol(
            statement.keyword_span,
            "try { ... } catch (...) { ... }".to_string(),
            Some(
                "Protects operations with checked effects. Source-ordered catches subtract only errors from the protected block."
                    .to_string(),
            ),
            SymbolKind::Plain,
        );
        self.visit_block(&statement.body, current_class, parent_class);

        for catch in &statement.catches {
            self.add_reference_symbol(
                catch.keyword_span,
                format!("catch ({}) {{ ... }}", catch.ty),
                Some(
                    "Handles this exact checked-error type from the protected block. `catch (Error)` is the catch-all form."
                        .to_string(),
                ),
                SymbolKind::Plain,
            );
            self.record_type_reference(&catch.ty, catch.ty_span);
            self.push_local_scope(catch.body.span.end);
            if let Some(binding) = &catch.binding {
                let ty = self
                    .semantic_info
                    .and_then(|info| info.catch_error_types.get(&catch.span))
                    .map(display_resolved_type)
                    .unwrap_or_else(|| catch.ty.to_string());
                self.declare_local_binding_with_documentation(
                    &binding.name,
                    binding.span,
                    format!("{ty} ${}", binding.name),
                    catch.body.span.start,
                    Some(
                        "Readonly owned checked-error value available only inside this catch body."
                            .to_string(),
                    ),
                );
            }
            for statement in &catch.body.statements {
                self.visit_stmt(statement, current_class, parent_class);
            }
            self.pop_local_scope();
        }

        if let Some(finalizer) = &statement.finally {
            self.add_reference_symbol(
                finalizer.keyword_span,
                "finally { ... }".to_string(),
                Some(
                    "Runs once after the protected block or selected catch. Checked errors propagate beyond this construct; sibling catches do not consume finalizer errors."
                        .to_string(),
                ),
                SymbolKind::Plain,
            );
            self.visit_block(&finalizer.body, current_class, parent_class);
        }
    }

    fn visit_if_statement(
        &mut self,
        if_statement: &IfStmt,
        current_class: Option<&str>,
        parent_class: Option<&str>,
    ) {
        self.push_local_scope(if_statement.span.end);
        if let Some(given) = &if_statement.given {
            self.visit_given_prelude(given, current_class, parent_class);
        }
        self.visit_expr(&if_statement.condition, current_class, parent_class);
        self.visit_block(&if_statement.then_block, current_class, parent_class);
        if let Some(branch) = &if_statement.else_branch {
            self.visit_else_branch(branch, current_class, parent_class);
        }
        if let Some(finalizer) = &if_statement.finally {
            self.visit_finally(finalizer, current_class, parent_class);
        }
        self.pop_local_scope();
    }

    fn visit_while_statement(
        &mut self,
        while_statement: &WhileStmt,
        current_class: Option<&str>,
        parent_class: Option<&str>,
    ) {
        self.push_local_scope(while_statement.span.end);
        if let Some(given) = &while_statement.given {
            self.visit_given_prelude(given, current_class, parent_class);
        }
        self.visit_expr(&while_statement.condition, current_class, parent_class);
        self.visit_block(&while_statement.body, current_class, parent_class);
        if let Some(finalizer) = &while_statement.finally {
            self.visit_finally(finalizer, current_class, parent_class);
        }
        self.pop_local_scope();
    }

    fn visit_do_while_statement(
        &mut self,
        do_while: &DoWhileStmt,
        current_class: Option<&str>,
        parent_class: Option<&str>,
    ) {
        self.visit_block(&do_while.body, current_class, parent_class);
        self.visit_expr(&do_while.condition, current_class, parent_class);
        self.add_reference_symbol(
            do_while.condition.span(),
            "do ... while condition: bool".to_string(),
            Some("Boolean condition evaluated after each completed loop body.".to_string()),
            SymbolKind::Plain,
        );
        if let Some(finalizer) = &do_while.finally {
            self.visit_finally(finalizer, current_class, parent_class);
        }
    }

    fn visit_given_prelude(
        &mut self,
        given: &GivenPrelude,
        current_class: Option<&str>,
        parent_class: Option<&str>,
    ) {
        let predicate_indices = self
            .semantic_info
            .and_then(|info| info.given_preludes.get(&given.span))
            .map(|info| {
                info.predicate_statement_indices
                    .iter()
                    .copied()
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        for (index, statement) in given.block.statements.iter().enumerate() {
            self.visit_stmt(statement, current_class, parent_class);
            if predicate_indices.contains(&index) {
                if let Stmt::Expr { expr, .. } = statement {
                    self.add_reference_symbol(
                        expr.span(),
                        "given predicate: bool".to_string(),
                        Some(
                            "Boolean gate evaluated in source order before the attached condition."
                                .to_string(),
                        ),
                        SymbolKind::Plain,
                    );
                }
            }
        }
    }

    fn visit_finally(
        &mut self,
        finalizer: &ControlFlowFinally,
        current_class: Option<&str>,
        parent_class: Option<&str>,
    ) {
        self.add_reference_symbol(
            finalizer.keyword_span,
            "finally { ... }".to_string(),
            Some(
                "Runs exactly once when the attached control-flow construct leaves normally or through a structured transfer. A checked error supersedes the pending nonfatal outcome and propagates outward; fatal panic bypasses the finalizer."
                    .to_string(),
            ),
            SymbolKind::Plain,
        );
        self.visit_block(&finalizer.block, current_class, parent_class);
    }

    fn visit_local_declaration(
        &mut self,
        declaration: &VarDecl,
        current_class: Option<&str>,
        parent_class: Option<&str>,
    ) {
        self.visit_expr(&declaration.initializer, current_class, parent_class);
        let ty = declaration
            .ty
            .as_ref()
            .map(ToString::to_string)
            .or_else(|| {
                self.semantic_info
                    .and_then(|info| info.expression_type(declaration.initializer.span()))
                    .map(display_resolved_type)
            })
            .unwrap_or_else(|| "Unknown".to_string());
        let prefix = if declaration.writable {
            "writable "
        } else {
            ""
        };
        for binding in &declaration.bindings {
            self.declare_local_binding(
                &binding.name,
                binding.span,
                format!("{prefix}{ty} ${}", binding.name),
                declaration.span.end,
            );
        }
    }

    fn declare_parameter(
        &mut self,
        parameter: &Param,
        visibility_start: usize,
        current_class: Option<&str>,
    ) {
        let selection_span = find_variable_span(self.tokens, parameter.span, &parameter.name)
            .unwrap_or(parameter.span);
        let documentation = (parameter.name == "message"
            && parameter.promoted_access.is_some()
            && current_class.is_some_and(|class| self.error_classes.contains(class)))
        .then(|| {
            "Promoted externally accessible readonly message required by the compiler-known `Error` contract."
                .to_string()
        });
        self.declare_local_binding_with_documentation(
            &parameter.name,
            selection_span,
            parameter_signature(parameter),
            visibility_start,
            documentation,
        );
    }

    fn declare_local_binding(
        &mut self,
        name: &str,
        selection_span: Span,
        signature: String,
        visibility_start: usize,
    ) {
        self.declare_local_binding_with_documentation(
            name,
            selection_span,
            signature,
            visibility_start,
            None,
        );
    }

    fn declare_local_binding_with_documentation(
        &mut self,
        name: &str,
        selection_span: Span,
        signature: String,
        visibility_start: usize,
        documentation: Option<String>,
    ) {
        let symbol = self.add_declaration_symbol(
            selection_span,
            signature,
            documentation,
            SymbolKind::Variable,
        );
        self.symbols[symbol].local_name = Some(name.to_string());
        self.local_visibilities.push(LocalVisibility {
            symbol,
            start: visibility_start,
            end: self
                .local_scope_ends
                .last()
                .copied()
                .unwrap_or(self.text.len()),
            depth: self.local_scopes.len(),
        });
        if let Some(scope) = self.local_scopes.last_mut() {
            scope.insert(name.to_string(), symbol);
        }
    }

    fn push_local_scope(&mut self, end: usize) {
        self.local_scopes.push(HashMap::new());
        self.local_scope_ends.push(end);
    }

    fn pop_local_scope(&mut self) {
        self.local_scopes.pop();
        self.local_scope_ends.pop();
    }

    fn resolve_local(&self, name: &str) -> Option<usize> {
        self.local_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    fn visit_else_branch(
        &mut self,
        branch: &ElseBranch,
        current_class: Option<&str>,
        parent_class: Option<&str>,
    ) {
        if let ElseBranch::If(if_statement) = branch {
            self.visit_if_statement(if_statement, current_class, parent_class);
        }
        if let ElseBranch::Block(block) = branch {
            self.visit_block(block, current_class, parent_class);
        }
    }

    fn visit_expr(
        &mut self,
        expression: &Expr,
        current_class: Option<&str>,
        parent_class: Option<&str>,
    ) {
        self.collect_assertion_expression_tokens(expression);
        match expression {
            Expr::Variable { name, span } => {
                if let Some(symbol) = self.resolve_local(name) {
                    self.record_reference(*span, symbol);
                }
            }
            Expr::MethodCall {
                object,
                method,
                args,
                span,
                null_safe,
                ..
            } => {
                self.visit_expr(object, current_class, parent_class);
                for argument in args {
                    self.visit_expr(&argument.value, current_class, parent_class);
                }
                let method_span =
                    self.member_name_span(Span::new(object.span().end, span.end), method);
                self.record_member_receiver(method_span, object, current_class);
                let builtin_hover = self.semantic_info.and_then(|info| {
                    if let Some(algorithm) = info.list_algorithm_calls.get(span) {
                        return Some(list_algorithm_hover(algorithm));
                    }
                    let receiver = info.expression_type(object.span())?;
                    compiler_known_method_hover(
                        receiver,
                        method,
                        *null_safe,
                        info.expression_type(*span),
                    )
                });
                if let (Some(method_span), Some(hover)) = (method_span, builtin_hover) {
                    self.add_reference_symbol(
                        method_span,
                        hover.signature,
                        Some(hover.documentation.to_string()),
                        SymbolKind::Plain,
                    );
                    return;
                }

                let target = self.semantic_info.and_then(|info| info.call_target(*span));
                let resolved_class = match target {
                    Some(CallableTarget::Method {
                        class_type,
                        method_name,
                    }) if method_name == method => Some(class_type.name.clone()),
                    _ if matches!(object.as_ref(), Expr::This { .. }) => {
                        current_class.map(ToOwned::to_owned)
                    }
                    _ => None,
                };
                if target.is_none() && self.has_diagnostic("E0304", *span) {
                    if let (Some(method_span), Some(class_name)) = (
                        method_span,
                        self.method_receiver_class_name(object, current_class),
                    ) {
                        self.record_missing_callable(
                            method,
                            method_span,
                            args,
                            *span,
                            MissingCallableTarget::Method {
                                class_name,
                                is_static: false,
                            },
                        );
                    }
                }
                if let (Some(class_name), Some(method_span)) =
                    (resolved_class.as_deref(), method_span)
                {
                    self.record_member_occurrence(
                        class_name,
                        method,
                        MemberKind::Method,
                        method_span,
                        false,
                    );
                }
                if let Some(class_name) = resolved_class.as_deref() {
                    if let Some(symbol) = self.resolve_method(class_name, method) {
                        if let Some(method_span) = method_span {
                            self.record_reference(method_span, symbol);
                        }
                        self.record_call_signature(*span, args, symbol);
                    }
                }
            }
            Expr::FunctionCall { name, args, span } => {
                for argument in args {
                    self.visit_expr(&argument.value, current_class, parent_class);
                }
                let resolved = matches!(
                    self.semantic_info.and_then(|info| info.call_target(*span)),
                    Some(CallableTarget::Function { name: resolved }) if resolved == name
                );
                if !resolved && self.has_diagnostic("E0309", *span) {
                    if let Some(name_span) = find_identifier_span(self.tokens, *span, name) {
                        self.record_missing_callable(
                            name,
                            name_span,
                            args,
                            *span,
                            MissingCallableTarget::Function,
                        );
                    }
                }
                if resolved {
                    let Some(symbol) = self.functions.get(name).copied() else {
                        return;
                    };
                    if let Some(name_span) = find_identifier_span(self.tokens, *span, name) {
                        self.record_reference(name_span, symbol);
                    }
                    self.record_call_signature(*span, args, symbol);
                }
            }
            Expr::StaticCall {
                qualifier,
                qualifier_span,
                method,
                member_span,
                args,
                span,
                ..
            } => {
                for argument in args {
                    self.visit_expr(&argument.value, current_class, parent_class);
                }
                if self.record_enum_static_reference(
                    qualifier,
                    *qualifier_span,
                    method,
                    *member_span,
                ) {
                    return;
                }
                if matches!(qualifier, StaticQualifier::Class(class) if class == "String") {
                    if let (Some(member), Some(method_span)) = (
                        string_companion_method(method),
                        self.member_name_span(Span::new(qualifier_span.end, span.end), method),
                    ) {
                        self.add_reference_symbol(
                            method_span,
                            member.signature.to_string(),
                            Some(member.documentation.to_string()),
                            SymbolKind::Plain,
                        );
                        return;
                    }
                }
                let target = self.semantic_info.and_then(|info| info.call_target(*span));
                let class_name = match target {
                    Some(CallableTarget::Method {
                        class_type,
                        method_name,
                    }) if method_name == method => Some(class_type.name.clone()),
                    _ if matches!(qualifier, StaticQualifier::SelfType) => {
                        current_class.map(ToOwned::to_owned)
                    }
                    _ if matches!(qualifier, StaticQualifier::Parent) => {
                        parent_class.map(ToOwned::to_owned)
                    }
                    _ => None,
                };
                if target.is_none() && self.has_diagnostic("E0304", *span) {
                    let class_name = match qualifier {
                        StaticQualifier::Class(name) => Some(name.clone()),
                        StaticQualifier::SelfType => current_class.map(ToOwned::to_owned),
                        StaticQualifier::Parent => parent_class.map(ToOwned::to_owned),
                        StaticQualifier::InvalidStatic => None,
                    };
                    if let (Some(method_span), Some(class_name)) = (
                        self.member_name_span(Span::new(qualifier_span.end, span.end), method),
                        class_name,
                    ) {
                        self.record_missing_callable(
                            method,
                            method_span,
                            args,
                            *span,
                            MissingCallableTarget::Method {
                                class_name,
                                is_static: true,
                            },
                        );
                    }
                }
                let method_span =
                    self.member_name_span(Span::new(qualifier_span.end, span.end), method);
                if let (Some(class_name), Some(method_span)) = (class_name.as_deref(), method_span)
                {
                    self.record_member_occurrence(
                        class_name,
                        method,
                        MemberKind::Method,
                        method_span,
                        false,
                    );
                }
                if let Some(class_name) = class_name.as_deref() {
                    if let Some(symbol) = self.resolve_method(class_name, method) {
                        if let Some(method_span) =
                            self.member_name_span(Span::new(qualifier_span.end, span.end), method)
                        {
                            self.record_reference(method_span, symbol);
                        }
                        self.record_call_signature(*span, args, symbol);
                    }
                }
            }
            Expr::New {
                class_type,
                args,
                span,
                ..
            } => {
                for argument in args {
                    self.visit_expr(&argument.value, current_class, parent_class);
                }
                if let Some(symbol) = self.classes.get(&class_type.name).copied() {
                    if let Some(name_span) =
                        find_identifier_span(self.tokens, *span, &class_type.name)
                    {
                        self.record_reference(name_span, symbol);
                    }
                }
                if let Some(symbol) = self.resolve_method(&class_type.name, "__construct") {
                    self.record_call_signature(*span, args, symbol);
                }
            }
            Expr::PropertyAccess {
                object,
                property,
                span,
                ..
            } => {
                self.visit_expr(object, current_class, parent_class);
                let property_span =
                    self.member_name_span(Span::new(object.span().end, span.end), property);
                self.record_member_receiver(property_span, object, current_class);
                let receiver = self
                    .semantic_info
                    .and_then(|info| info.expression_type(object.span()));
                if let (Some(ResolvedType::Enum(enum_type)), Some(property_span), "value") = (
                    receiver.map(non_nullable_type),
                    property_span,
                    property.as_str(),
                ) {
                    if let Some(backing_type) = self.semantic_info.and_then(|info| {
                        info.enums
                            .iter()
                            .find(|info| info.id == enum_type.id)
                            .and_then(|info| info.backing_type)
                    }) {
                        self.add_reference_symbol(
                            property_span,
                            format!("{} $value", enum_backing_name(backing_type)),
                            Some(
                                "Readonly backing value associated with this enum case."
                                    .to_string(),
                            ),
                            SymbolKind::Variable,
                        );
                    }
                    return;
                }
                if matches!(
                    receiver.map(non_nullable_type),
                    Some(ResolvedType::SharedHandle(
                        SharedHandleKind::SharedReference,
                        _
                    ))
                ) && property == "referencedValue"
                {
                    let Some(ResolvedType::SharedHandle(_, payload)) =
                        receiver.map(non_nullable_type)
                    else {
                        unreachable!("shared-reference receiver checked above");
                    };
                    if let Some(property_span) = property_span {
                        self.add_reference_symbol(
                            property_span,
                            format!("{} $referencedValue", display_resolved_type(payload)),
                            Some("Readonly, allocation-free projection to the payload for resolving wrapper/member name collisions. It does not change either ownership count.".to_string()),
                            SymbolKind::Variable,
                        );
                    }
                    return;
                }
                if let (Some(class_name), Some(property_span)) =
                    (receiver.and_then(member_receiver_class_name), property_span)
                {
                    if self
                        .semantic_info
                        .is_some_and(|info| info.expression_type(*span).is_some())
                    {
                        self.record_member_occurrence(
                            class_name,
                            property,
                            MemberKind::Property,
                            property_span,
                            false,
                        );
                    }
                    if let Some(symbol) = self.resolve_property(class_name, property) {
                        self.record_reference(property_span, symbol);
                    }
                }
                let is_string = self
                    .semantic_info
                    .and_then(|info| info.expression_type(object.span()))
                    .is_some_and(|ty| matches!(non_nullable_type(ty), ResolvedType::String));
                if is_string {
                    if let (Some(member), Some(property_span)) =
                        (string_property(property), property_span)
                    {
                        let return_type = self
                            .semantic_info
                            .and_then(|info| info.expression_type(*span))
                            .map(display_resolved_type)
                            .unwrap_or_else(|| {
                                member
                                    .signature
                                    .split_once(' ')
                                    .map_or("unknown", |(ty, _)| ty)
                                    .to_string()
                            });
                        self.add_reference_symbol(
                            property_span,
                            format!("{return_type} ${property}"),
                            Some(member.documentation.to_string()),
                            SymbolKind::Variable,
                        );
                    }
                    return;
                }
                if let (Some(receiver), Some(property_span)) = (receiver, property_span) {
                    if let Some(documentation) = collection_property(receiver, property) {
                        let return_type = self
                            .semantic_info
                            .and_then(|info| info.expression_type(*span))
                            .map(display_resolved_type)
                            .unwrap_or_else(|| "unknown".to_string());
                        self.add_reference_symbol(
                            property_span,
                            format!("{return_type} ${property}"),
                            Some(documentation.to_string()),
                            SymbolKind::Variable,
                        );
                    }
                }
            }
            Expr::StaticMember {
                qualifier,
                qualifier_span,
                member,
                member_span,
                ..
            } => {
                if !self.record_enum_static_reference(
                    qualifier,
                    *qualifier_span,
                    member,
                    *member_span,
                ) {
                    let owner = match qualifier {
                        StaticQualifier::Class(name) => Some(name.as_str()),
                        StaticQualifier::SelfType => current_class,
                        StaticQualifier::Parent => parent_class,
                        StaticQualifier::InvalidStatic => None,
                    };
                    if let Some(owner) = owner {
                        self.record_member_occurrence(
                            owner,
                            member,
                            MemberKind::Constant,
                            *member_span,
                            false,
                        );
                        if let Some(symbol) = self.resolve_constant(owner, member) {
                            self.record_reference(*member_span, symbol);
                        }
                    }
                }
            }
            Expr::IsType { expr, .. } | Expr::Grouped { expr, .. } | Expr::Unary { expr, .. } => {
                self.visit_expr(expr, current_class, parent_class)
            }
            Expr::Binary { left, right, .. }
            | Expr::Range {
                start: left,
                end: right,
                ..
            } => {
                self.visit_expr(left, current_class, parent_class);
                self.visit_expr(right, current_class, parent_class);
            }
            Expr::Array { elements, .. } => {
                for element in elements {
                    if let Some(key) = &element.key {
                        self.visit_expr(key, current_class, parent_class);
                    }
                    self.visit_expr(&element.value, current_class, parent_class);
                }
            }
            Expr::InterpolatedString { parts, .. } => {
                for part in parts {
                    if let doriac::ast::InterpolatedStringPart::Expr(expr) = part {
                        self.visit_expr(expr, current_class, parent_class);
                    }
                }
            }
            // An index expression carries symbol-bearing children on both
            // sides, so hover and definition need them traversed rather than
            // absorbed by the catch-all below.
            Expr::Index {
                collection, index, ..
            } => {
                self.visit_expr(collection, current_class, parent_class);
                self.visit_expr(index, current_class, parent_class);
            }
            Expr::Match {
                scrutinee,
                mode,
                arms,
                origin,
                span,
            } => {
                self.visit_expr(scrutinee, current_class, parent_class);
                let match_info = self
                    .semantic_info
                    .and_then(|info| info.matches.get(span))
                    .cloned();
                if *origin == MatchOrigin::Match {
                    if let (Some(info), Some(keyword)) = (
                        match_info.as_ref(),
                        tokens_in_span(self.tokens, *span)
                            .find(|token| matches!(token.kind, TokenKind::Match)),
                    ) {
                        self.add_reference_symbol(
                            keyword.span,
                            format!("match (...): {}", display_resolved_type(&info.result_type)),
                            Some("Exhaustive match expression result.".to_string()),
                            SymbolKind::Plain,
                        );
                    }
                }
                if let MatchMode::Consumed { take_span } = mode {
                    self.add_reference_symbol(
                        *take_span,
                        "match (take $value)".to_string(),
                        Some(
                            "Gives the whole Move value to this match. A matching guard sees readonly payload views; the selected arm receives owned Move payload bindings."
                                .to_string(),
                        ),
                        SymbolKind::Plain,
                    );
                }
                for (index, arm) in arms.iter().enumerate() {
                    self.push_local_scope(arm.span.end);
                    let visibility_start = arm.guard.as_ref().map_or_else(
                        || arm.value.span().start,
                        |guard| guard.condition.span().start,
                    );
                    match &arm.pattern {
                        MatchPattern::Expression(pattern) => {
                            self.visit_expr(pattern, current_class, parent_class)
                        }
                        MatchPattern::EnumCase {
                            qualifier,
                            qualifier_span,
                            case,
                            case_span,
                            bindings,
                            ..
                        } => {
                            self.record_enum_static_reference(
                                &StaticQualifier::Class(qualifier.clone()),
                                *qualifier_span,
                                case,
                                *case_span,
                            );
                            if let (Some(bindings), Some(arm_info)) = (
                                bindings.as_ref(),
                                match_info.as_ref().and_then(|info| info.arms.get(index)),
                            ) {
                                self.declare_match_bindings(
                                    bindings,
                                    arm_info.bindings.iter(),
                                    visibility_start,
                                );
                            }
                        }
                        MatchPattern::TypeBinding { ty, binding, span } => {
                            self.record_match_type_reference(ty, *span);
                            if let Some(binding_info) = match_info
                                .as_ref()
                                .and_then(|info| info.arms.get(index))
                                .and_then(|arm| arm.bindings.first())
                            {
                                self.declare_match_bindings(
                                    std::slice::from_ref(binding),
                                    std::iter::once(binding_info),
                                    visibility_start,
                                );
                            }
                        }
                        MatchPattern::Default { .. } => {}
                    }
                    if let Some(guard) = &arm.guard {
                        self.visit_expr(&guard.condition, current_class, parent_class);
                    }
                    self.visit_expr(&arm.value, current_class, parent_class);
                    self.pop_local_scope();
                }
            }
            Expr::When(when) => self.visit_when_expression(when, current_class, parent_class),
            // IDE analysis is best-effort across compiler feature branches. New
            // expression forms remain diagnostic-safe until their symbol-bearing
            // children need explicit traversal here.
            _ => {}
        }
    }

    fn collect_assertion_expression_tokens(&mut self, expression: &Expr) {
        let Some(info) = self
            .semantic_info
            .and_then(|info| info.assertions.get(&expression.span()))
            .filter(|info| info.terminal_span.source == self.source_id)
        else {
            return;
        };

        self.assertion_semantic_tokens
            .push((info.member_span, SEMANTIC_TOKEN_FUNCTION, 0));
        if !info.negated {
            return;
        }
        let Expr::MethodCall { object, .. } = expression else {
            return;
        };
        if let Expr::PropertyAccess {
            property,
            member_span,
            ..
        } = object.as_ref()
        {
            if property == "not" {
                self.assertion_semantic_tokens
                    .push((*member_span, SEMANTIC_TOKEN_PROPERTY, 0));
            }
        }
    }

    fn visit_when_expression(
        &mut self,
        when: &WhenExpression,
        current_class: Option<&str>,
        parent_class: Option<&str>,
    ) {
        self.push_local_scope(when.span.end);
        if let Some(given) = &when.given {
            self.visit_given_prelude(given, current_class, parent_class);
        }
        if let (Some(info), Some(keyword)) = (
            self.semantic_info
                .and_then(|semantic| semantic.whens.get(&when.span)),
            tokens_in_span(self.tokens, when.span)
                .find(|token| matches!(token.kind, TokenKind::When)),
        ) {
            self.add_reference_symbol(
                keyword.span,
                format!("when (...): {}", display_resolved_type(&info.result_type)),
                Some("Exhaustive conditional expression result.".to_string()),
                SymbolKind::Plain,
            );
        }
        for branch in &when.branches {
            if let Some(condition) = &branch.condition {
                self.visit_expr(condition, current_class, parent_class);
            }
            self.when_depth += 1;
            self.visit_block(&branch.block, current_class, parent_class);
            self.when_depth -= 1;
        }
        if let Some(finalizer) = &when.finally {
            self.visit_finally(finalizer, current_class, parent_class);
        }
        self.pop_local_scope();
    }

    fn declare_match_bindings<'b>(
        &mut self,
        bindings: &'b [doriac::ast::MatchBinding],
        semantics: impl Iterator<Item = &'b doriac::semantics::MatchBindingSemanticInfo>,
        visibility_start: usize,
    ) {
        for (binding, semantic) in bindings.iter().zip(semantics) {
            self.declare_local_binding_with_documentation(
                &binding.name,
                binding.span,
                format!(
                    "{} ${}",
                    display_resolved_type(&semantic.ty),
                    binding.name
                ),
                visibility_start,
                Some(if !semantic.borrowed {
                    "Readonly while the guard is evaluated; owns the selected Move payload in the arm."
                        .to_string()
                } else {
                    "Readonly pattern binding available in this arm's guard and result."
                        .to_string()
                }),
            );
        }
    }

    fn record_type_reference(&mut self, ty: &TypeRef, span: Span) {
        let Some(type_span) = find_identifier_span(self.tokens, span, &ty.name) else {
            return;
        };
        if ty.name == "Error" {
            self.add_error_type_reference(type_span);
            return;
        }
        if let Some(symbol) = self
            .enums
            .get(&ty.name)
            .or_else(|| self.classes.get(&ty.name))
            .copied()
        {
            self.record_reference(type_span, symbol);
        }
    }

    fn record_match_type_reference(&mut self, ty: &TypeRef, span: Span) {
        self.record_type_reference(ty, span);
    }

    fn add_error_type_reference(&mut self, span: Span) {
        self.add_reference_symbol(
            span,
            "interface Error\n{\n    string $message;\n}".to_string(),
            Some(
                "Compiler-known checked-error contract. Conforming classes explicitly declare `implements Error` and expose an externally accessible readonly `string $message` property."
                    .to_string(),
            ),
            SymbolKind::Plain,
        );
    }

    fn resolve_method(&self, class_name: &str, method: &str) -> Option<usize> {
        let mut current = Some(class_name);
        let mut visited = HashSet::new();
        while let Some(class_name) = current {
            if !visited.insert(class_name) {
                return None;
            }
            if let Some(symbol) = self
                .methods
                .get(&(class_name.to_string(), method.to_string()))
            {
                return Some(*symbol);
            }
            current = self.class_parents.get(class_name).map(String::as_str);
        }
        None
    }

    fn resolve_property(&self, class_name: &str, property: &str) -> Option<usize> {
        let mut current = Some(class_name);
        let mut visited = HashSet::new();
        while let Some(class_name) = current {
            if !visited.insert(class_name) {
                break;
            }
            if let Some(symbol) = self
                .class_property_symbols
                .get(&(class_name.to_string(), property.to_string()))
            {
                return Some(*symbol);
            }
            current = self.class_parents.get(class_name).map(String::as_str);
        }
        None
    }

    fn resolve_constant(&self, class_name: &str, constant: &str) -> Option<usize> {
        let mut current = Some(class_name);
        let mut visited = HashSet::new();
        while let Some(class_name) = current {
            if !visited.insert(class_name) {
                break;
            }
            if let Some(symbol) = self
                .class_constant_symbols
                .get(&(class_name.to_string(), constant.to_string()))
            {
                return Some(*symbol);
            }
            current = self.class_parents.get(class_name).map(String::as_str);
        }
        None
    }

    fn record_enum_static_reference(
        &mut self,
        qualifier: &StaticQualifier,
        qualifier_span: Span,
        member: &str,
        member_span: Span,
    ) -> bool {
        let StaticQualifier::Class(enum_name) = qualifier else {
            return false;
        };
        let Some(enum_symbol) = self.enums.get(enum_name).copied() else {
            return false;
        };
        self.record_reference(qualifier_span, enum_symbol);
        self.static_receivers.push(StaticReceiver {
            span: member_span,
            enum_name: enum_name.clone(),
        });
        if let Some(case_symbol) = self
            .enum_cases
            .get(&(enum_name.clone(), member.to_string()))
            .copied()
        {
            self.record_reference(member_span, case_symbol);
        }
        self.record_member_occurrence(enum_name, member, MemberKind::EnumCase, member_span, false);
        true
    }

    fn member_name_span(&self, search_span: Span, name: &str) -> Option<Span> {
        find_identifier_span(self.tokens, search_span, name)
    }

    fn record_member_receiver(
        &mut self,
        member_span: Option<Span>,
        object: &Expr,
        current_class: Option<&str>,
    ) {
        let Some(span) = member_span else {
            return;
        };
        let Some(receiver) = self
            .semantic_info
            .and_then(|info| info.expression_type(object.span()))
            .cloned()
        else {
            return;
        };
        self.member_receivers.push(MemberReceiver {
            span,
            receiver,
            current_class: current_class.map(ToOwned::to_owned),
            writable_payload_access: !is_readonly_shared_projection(object, self.semantic_info),
        });
    }

    fn has_diagnostic(&self, code: &str, span: Span) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == code && diagnostic_overlaps_span(diagnostic, span))
    }

    fn method_receiver_class_name(
        &self,
        object: &Expr,
        current_class: Option<&str>,
    ) -> Option<String> {
        if let Some(class_name) = self
            .semantic_info
            .and_then(|info| info.expression_type(object.span()))
            .and_then(member_receiver_class_name)
        {
            return Some(class_name.to_string());
        }
        if matches!(object, Expr::This { .. }) {
            current_class.map(ToOwned::to_owned)
        } else {
            None
        }
    }

    fn record_missing_callable(
        &mut self,
        name: &str,
        name_span: Span,
        arguments: &[doriac::ast::Argument],
        call_span: Span,
        target: MissingCallableTarget,
    ) {
        let mut used_names = HashSet::new();
        let parameters = arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                let preferred_name = argument
                    .name
                    .as_ref()
                    .map(|name| name.text.clone())
                    .or_else(|| match &argument.value {
                        Expr::Variable { name, .. } => Some(name.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| format!("arg{}", index + 1));
                let name = unique_parameter_name(preferred_name, &mut used_names);
                let ty = self.generated_argument_type(&argument.value);
                GeneratedParameter { name, ty }
            })
            .collect();
        let return_type = if self.statement_expression_span == Some(call_span) {
            "void".to_string()
        } else {
            "mixed".to_string()
        };
        self.missing_callables.push(MissingCallable {
            name: name.to_string(),
            name_span,
            parameters,
            return_type,
            target,
        });
    }

    fn generated_argument_type(&self, expression: &Expr) -> String {
        if let Some(ty) = self
            .semantic_info
            .and_then(|info| info.expression_type(expression.span()))
        {
            return generated_parameter_type(ty);
        }
        match expression {
            Expr::Variable { name, .. } => self
                .resolve_local(name)
                .and_then(|symbol| self.symbols.get(symbol))
                .and_then(|symbol| symbol.signature.split_once(" $").map(|(ty, _)| ty))
                .and_then(|ty| ty.split_whitespace().last())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| "mixed".to_string()),
            Expr::String { .. } | Expr::InterpolatedString { .. } => "string".to_string(),
            Expr::Int { .. } => "int".to_string(),
            Expr::Float { .. } => "float".to_string(),
            Expr::Bool { .. } => "bool".to_string(),
            Expr::New { class_type, .. } => class_type.to_string(),
            _ => "mixed".to_string(),
        }
    }
}

fn unique_parameter_name(preferred: String, used: &mut HashSet<String>) -> String {
    if used.insert(preferred.clone()) {
        return preferred;
    }
    for suffix in 2.. {
        let candidate = format!("{preferred}{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("an unused parameter suffix always exists")
}

fn generated_parameter_type(ty: &ResolvedType) -> String {
    match ty {
        ResolvedType::Null | ResolvedType::Unsupported | ResolvedType::Void => "mixed".to_string(),
        _ => display_resolved_type(ty),
    }
}

fn is_readonly_shared_projection(expression: &Expr, semantic_info: Option<&SemanticInfo>) -> bool {
    let Expr::PropertyAccess {
        object, property, ..
    } = expression
    else {
        return false;
    };
    property == "referencedValue"
        && semantic_info
            .and_then(|info| info.expression_type(object.span()))
            .is_some_and(|receiver| {
                matches!(
                    non_nullable_type(receiver),
                    ResolvedType::SharedHandle(SharedHandleKind::SharedReference, _)
                )
            })
}

struct CompilerKnownMethodHover {
    signature: String,
    documentation: String,
}

fn list_algorithm_completions(receiver: &ResolvedType) -> Option<Vec<SemanticCompletion>> {
    let receiver = match receiver {
        ResolvedType::SharedHandle(kind, payload) if kind.is_access() => payload.as_ref(),
        receiver => receiver,
    };
    let ResolvedType::List(element) = receiver else {
        return None;
    };
    let element = display_resolved_type(element);
    Some(
        [
            (
                "map",
                format!("function map<U>(function({element}): U): List<U>"),
                "Transforms each element in insertion order and returns a new List. The source List remains unchanged.",
            ),
            (
                "filter",
                format!("function filter(function({element}): bool): List<{element}>"),
                "Returns a new List containing selected Copy elements in insertion order. The source List remains unchanged.",
            ),
            (
                "reduce",
                format!(
                    "function reduce<A>(take A, function(writable A, {element}): void): A"
                ),
                "Owns the initial accumulator, lends it writably to the reducer for each element, and returns the completed accumulator.",
            ),
        ]
        .into_iter()
        .map(|(label, detail, documentation)| SemanticCompletion {
            label: label.to_string(),
            kind: 2,
            detail,
            documentation: Some(documentation.to_string()),
        })
        .collect(),
    )
}

fn list_algorithm_hover(algorithm: &ListAlgorithmCallInfo) -> CompilerKnownMethodHover {
    let receiver = display_resolved_type(&algorithm.receiver_type);
    let callback = display_function_type_with_effects(
        &algorithm.callback_type,
        &algorithm.required_checked_effects,
    );
    let result = display_resolved_type(&algorithm.result_type);
    let parameters = match algorithm.kind {
        ListAlgorithmKind::Map | ListAlgorithmKind::Filter => callback,
        ListAlgorithmKind::Reduce => format!(
            "take {}, {callback}",
            display_resolved_type(
                algorithm
                    .accumulator_type
                    .as_ref()
                    .expect("checked reduce algorithm has an accumulator type"),
            )
        ),
    };
    let method = match algorithm.kind {
        ListAlgorithmKind::Map => "map",
        ListAlgorithmKind::Filter => "filter",
        ListAlgorithmKind::Reduce => "reduce",
    };
    let effects = effect_list(&algorithm.required_checked_effects);
    let ambient = ambient_effect_documentation(&algorithm.ambient_checked_effects);
    let (access, invocation) = match algorithm.callback_access {
        ListCallbackAccess::Readonly => ("readonly", "Readonly Repeatable"),
        ListCallbackAccess::Writable => ("writable", "Writable Repeatable"),
    };
    CompilerKnownMethodHover {
        signature: format!("{receiver}::{method}({parameters}): {result}"),
        documentation: format!(
            "**Callback access:** {access}\n\n**Callback invocation:** {invocation}\n\n**Required checked effects:** {effects}{ambient}\n\n**Source:** Unchanged\n\n**Result:** Owned `{result}`"
        ),
    }
}

fn shared_method_completion(receiver: &ResolvedType, method: &str) -> SemanticCompletion {
    let (parameters, return_type, documentation) = shared_ownership_method(receiver, method)
        .unwrap_or_else(|| panic!("missing shared-ownership method metadata for `{method}`"));
    SemanticCompletion {
        label: method.to_string(),
        kind: 2,
        detail: format!("function {method}({parameters}): {return_type}"),
        documentation: Some(documentation.to_string()),
    }
}

fn compiler_known_method_hover(
    receiver: &ResolvedType,
    method: &str,
    null_safe: bool,
    resolved_return: Option<&ResolvedType>,
) -> Option<CompilerKnownMethodHover> {
    let receiver_type = non_nullable_type(receiver);
    let (parameters, fallback_return, documentation) =
        shared_ownership_method(receiver_type, method)
            .or_else(|| collection_method(receiver_type, method))?;
    let return_type = resolved_return
        .filter(|ty| !matches!(ty, ResolvedType::Unsupported))
        .map(display_resolved_type)
        .unwrap_or_else(|| {
            if null_safe && fallback_return != "void" && !fallback_return.starts_with('?') {
                format!("?{fallback_return}")
            } else {
                fallback_return
            }
        });

    Some(CompilerKnownMethodHover {
        signature: format!(
            "function {}::{method}({parameters}): {return_type}",
            display_resolved_type(receiver)
        ),
        documentation: documentation.to_string(),
    })
}

fn shared_ownership_method(
    receiver: &ResolvedType,
    method: &str,
) -> Option<(String, String, &'static str)> {
    use SharedHandleKind::*;

    let ResolvedType::SharedHandle(kind, payload) = receiver else {
        return None;
    };
    let payload = display_resolved_type(payload);
    let (return_type, documentation) = match (*kind, method) {
        (SharedReference, "share") => (
            format!("SharedReference<{payload}>"),
            "Creates one additional owner in the receiver's readonly shared-ownership family.",
        ),
        (SharedReference, "createWeakReference") => (
            format!("WeakReference<{payload}>"),
            "Creates a non-owning reference in the receiver's readonly shared-ownership family.",
        ),
        (WritableSharedReference, "share") => (
            format!("WritableSharedReference<{payload}>"),
            "Creates one additional owner in the receiver's writable shared-ownership family.",
        ),
        (WritableSharedReference, "createWeakReference") => (
            format!("WritableWeakReference<{payload}>"),
            "Creates a non-owning reference in the receiver's writable shared-ownership family.",
        ),
        (WeakReference, "acquire") => (
            format!("?SharedReference<{payload}>"),
            "Attempts to create a readonly strong owner. Returns `null` after the payload has been destroyed.",
        ),
        (WritableWeakReference, "acquire") => (
            format!("?WritableSharedReference<{payload}>"),
            "Attempts to create a writable-family strong owner. Returns `null` after the payload has been destroyed.",
        ),
        (WritableSharedReference, "acquireReadonlyAccess") => (
            format!("ReadonlySharedReferenceAccess<{payload}>"),
            "Acquires owned readonly access to the payload. Multiple readonly accesses may coexist.",
        ),
        (WritableSharedReference, "acquireWritableAccess") => (
            format!("WritableSharedReferenceAccess<{payload}>"),
            "Acquires owned exclusive writable access to the payload.",
        ),
        _ => return None,
    };
    Some((String::new(), return_type, documentation))
}

fn collection_method(
    receiver: &ResolvedType,
    method: &str,
) -> Option<(String, String, &'static str)> {
    let collection = match receiver {
        ResolvedType::SharedHandle(kind, payload) if kind.is_access() => payload.as_ref(),
        receiver => receiver,
    };
    let (parameters, return_type, documentation) = match (collection, method) {
        (ResolvedType::List(value), "add") => (
            format!("{} $value", display_resolved_type(value)),
            "void".to_string(),
            "Appends a value to this writable list.",
        ),
        (ResolvedType::List(value), "insertAt") => (
            format!("int $index, {} $value", display_resolved_type(value)),
            "void".to_string(),
            "Inserts a value at the given index in this writable list.",
        ),
        (ResolvedType::List(value), "removeAt") => (
            "int $index".to_string(),
            display_resolved_type(value),
            "Removes and returns the value at the given index.",
        ),
        (ResolvedType::List(value), "pop") => (
            String::new(),
            format!("?{}", display_resolved_type(value)),
            "Removes and returns the final value, or `null` when the list is empty.",
        ),
        (ResolvedType::List(value), "contains") => (
            format!("{} $value", display_resolved_type(value)),
            "bool".to_string(),
            "Reports whether this list contains an equal value.",
        ),
        (ResolvedType::List(value), "indexOf") => (
            format!("{} $value", display_resolved_type(value)),
            "?int".to_string(),
            "Returns the first equal position, or `null` when the value is absent.",
        ),
        (ResolvedType::List(value), "remove") => (
            format!("{} $value", display_resolved_type(value)),
            "bool".to_string(),
            "Removes the first equal value from this writable list and reports whether it changed.",
        ),
        (ResolvedType::TypedArray(value), "contains") => (
            format!("{} $value", display_resolved_type(value)),
            "bool".to_string(),
            "Reports whether this array contains an equal value.",
        ),
        (ResolvedType::PriorityQueue(value), "contains") => (
            format!("{} $value", display_resolved_type(value)),
            "bool".to_string(),
            "Reports whether this queue contains an equal value.",
        ),
        (ResolvedType::Deque(value), "contains") => (
            format!("{} $value", display_resolved_type(value)),
            "bool".to_string(),
            "Reports whether this deque contains an equal value.",
        ),
        (
            ResolvedType::Dictionary(key, value) | ResolvedType::SortedDictionary(key, value),
            "set",
        ) => (
            format!(
                "{} $key, {} $value",
                display_resolved_type(key),
                display_resolved_type(value)
            ),
            "void".to_string(),
            "Stores a value for the key in this writable dictionary.",
        ),
        (
            ResolvedType::Dictionary(key, value) | ResolvedType::SortedDictionary(key, value),
            "get",
        ) => (
            format!("{} $key", display_resolved_type(key)),
            format!("?{}", display_resolved_type(value)),
            "Returns the value for the key, or `null` when the key is absent.",
        ),
        (
            ResolvedType::Dictionary(key, _) | ResolvedType::SortedDictionary(key, _),
            "containsKey",
        ) => (
            format!("{} $key", display_resolved_type(key)),
            "bool".to_string(),
            "Reports whether this dictionary contains the key.",
        ),
        (
            ResolvedType::Dictionary(_, value) | ResolvedType::SortedDictionary(_, value),
            "containsValue",
        ) => (
            format!("{} $value", display_resolved_type(value)),
            "bool".to_string(),
            "Reports whether this dictionary contains an equal value.",
        ),
        (
            ResolvedType::Dictionary(key, value) | ResolvedType::SortedDictionary(key, value),
            "remove",
        ) => (
            format!("{} $key", display_resolved_type(key)),
            format!("?{}", display_resolved_type(value)),
            "Removes and returns the value for the key, or `null` when the key is absent.",
        ),
        (ResolvedType::Set(value) | ResolvedType::SortedSet(value), "add") => (
            format!("{} $value", display_resolved_type(value)),
            "bool".to_string(),
            "Adds a value and reports whether the set changed.",
        ),
        (ResolvedType::Set(value) | ResolvedType::SortedSet(value), "remove") => (
            format!("{} $value", display_resolved_type(value)),
            "bool".to_string(),
            "Removes a value and reports whether the set changed.",
        ),
        (ResolvedType::Set(value) | ResolvedType::SortedSet(value), "contains") => (
            format!("{} $value", display_resolved_type(value)),
            "bool".to_string(),
            "Reports whether this set contains the value.",
        ),
        (ResolvedType::Set(value), method @ ("union" | "intersect" | "difference")) => (
            format!("Set<{}> $other", display_resolved_type(value)),
            format!("Set<{}>", display_resolved_type(value)),
            match method {
                "union" => "Returns a set containing values from either set.",
                "intersect" => "Returns a set containing values present in both sets.",
                "difference" => "Returns a set containing values absent from the other set.",
                _ => unreachable!(),
            },
        ),
        (ResolvedType::SortedSet(value), method @ ("union" | "intersect" | "difference")) => (
            format!("SortedSet<{}> $other", display_resolved_type(value)),
            format!("SortedSet<{}>", display_resolved_type(value)),
            match method {
                "union" => "Returns a sorted set containing values from either set.",
                "intersect" => "Returns a sorted set containing values present in both sets.",
                "difference" => "Returns a sorted set containing values absent from the other set.",
                _ => unreachable!(),
            },
        ),
        (ResolvedType::PriorityQueue(value), "push") => (
            format!("{} $value", display_resolved_type(value)),
            "void".to_string(),
            "Adds a value to this writable min-priority queue.",
        ),
        (ResolvedType::PriorityQueue(value), "pop") => (
            String::new(),
            format!("?{}", display_resolved_type(value)),
            "Removes and returns the minimum value, or `null` when the queue is empty.",
        ),
        (ResolvedType::Deque(value), "pushFront" | "pushBack") => (
            format!("{} $value", display_resolved_type(value)),
            "void".to_string(),
            "Adds a value at the selected end of this writable deque.",
        ),
        (ResolvedType::Deque(value), "popFront" | "popBack") => (
            String::new(),
            format!("?{}", display_resolved_type(value)),
            "Removes and returns the value at the selected end, or `null` when the deque is empty.",
        ),
        (
            ResolvedType::List(_)
            | ResolvedType::Dictionary(_, _)
            | ResolvedType::Set(_)
            | ResolvedType::SortedDictionary(_, _)
            | ResolvedType::SortedSet(_)
            | ResolvedType::PriorityQueue(_)
            | ResolvedType::Deque(_),
            "clear",
        ) => (
            String::new(),
            "void".to_string(),
            "Empties this writable collection in place while preserving the collection for reuse.",
        ),
        (ResolvedType::Bytes, "toArray") => (
            String::new(),
            "uint8[]".to_string(),
            "Copies this byte buffer into a fixed-length `uint8[]`.",
        ),
        _ => return None,
    };
    Some((parameters, return_type, documentation))
}

fn collection_property(receiver: &ResolvedType, property: &str) -> Option<&'static str> {
    let collection = match non_nullable_type(receiver) {
        ResolvedType::SharedHandle(kind, payload) if kind.is_access() => payload.as_ref(),
        receiver => receiver,
    };
    match (collection, property) {
        (
            ResolvedType::List(_)
            | ResolvedType::Dictionary(_, _)
            | ResolvedType::Set(_)
            | ResolvedType::SortedDictionary(_, _)
            | ResolvedType::SortedSet(_)
            | ResolvedType::PriorityQueue(_)
            | ResolvedType::Deque(_),
            "count",
        ) => Some("The number of values currently stored in this collection."),
        (
            ResolvedType::List(_)
            | ResolvedType::Dictionary(_, _)
            | ResolvedType::Set(_)
            | ResolvedType::SortedDictionary(_, _)
            | ResolvedType::SortedSet(_)
            | ResolvedType::PriorityQueue(_)
            | ResolvedType::Deque(_),
            "isEmpty",
        ) => Some("Reports whether this collection contains no values."),
        (ResolvedType::List(_) | ResolvedType::Set(_) | ResolvedType::SortedSet(_), "first") => {
            Some("The first value in collection iteration order, or `null` when empty.")
        }
        (ResolvedType::List(_) | ResolvedType::Set(_) | ResolvedType::SortedSet(_), "last") => {
            Some("The last value in collection iteration order, or `null` when empty.")
        }
        (ResolvedType::Dictionary(_, _) | ResolvedType::SortedDictionary(_, _), "keys") => {
            Some("A readonly projection of this dictionary's keys.")
        }
        (ResolvedType::Dictionary(_, _) | ResolvedType::SortedDictionary(_, _), "values") => {
            Some("A readonly projection of this dictionary's values.")
        }
        (ResolvedType::PriorityQueue(_), "peek") => {
            Some("The minimum value without removing it, or `null` when empty.")
        }
        (ResolvedType::Deque(_), "peekFront") => {
            Some("The front value without removing it, or `null` when empty.")
        }
        (ResolvedType::Deque(_), "peekBack") => {
            Some("The back value without removing it, or `null` when empty.")
        }
        _ => None,
    }
}

fn non_nullable_type(ty: &ResolvedType) -> &ResolvedType {
    match ty {
        ResolvedType::Nullable(inner) => inner,
        ty => ty,
    }
}

fn narrowed_function_type_for_use<'a>(
    info: &'a SemanticInfo,
    span: &Span,
) -> Option<&'a ResolvedType> {
    let ty = info.expression_types.get(span)?;
    if !matches!(ty, ResolvedType::Function(_)) {
        return None;
    }

    let binding = info.binding_resolution.uses_by_span.get(span)?;
    let declared = info
        .binding_resolution
        .declarations_by_id
        .get(binding)?
        .source_type
        .as_ref()?;
    match declared {
        ResolvedType::Mixed => Some(ty),
        ResolvedType::Nullable(inner)
            if matches!(
                inner.as_ref(),
                ResolvedType::Mixed | ResolvedType::Function(_)
            ) =>
        {
            Some(ty)
        }
        _ => None,
    }
}

fn member_receiver_class_name(ty: &ResolvedType) -> Option<&str> {
    match non_nullable_type(ty) {
        ResolvedType::Class(class) => Some(&class.name),
        ResolvedType::SharedHandle(
            SharedHandleKind::SharedReference
            | SharedHandleKind::ReadonlySharedReferenceAccess
            | SharedHandleKind::WritableSharedReferenceAccess,
            payload,
        ) => match non_nullable_type(payload) {
            ResolvedType::Class(class) => Some(&class.name),
            _ => None,
        },
        _ => None,
    }
}

fn enum_backing_name(backing: EnumBackingType) -> &'static str {
    match backing {
        EnumBackingType::Int => "int",
        EnumBackingType::String => "string",
    }
}

fn enum_case_signature(
    enum_name: &str,
    case_name: &str,
    case: Option<&doriac::semantics::EnumCaseSemanticInfo>,
) -> String {
    let parameters = case
        .map(|case| {
            case.payload
                .iter()
                .map(|field| format!("{} ${}", display_resolved_type(&field.ty), field.name))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if parameters.is_empty() {
        format!("{enum_name}::{case_name}: {enum_name}")
    } else {
        format!(
            "{enum_name}::{case_name}({}): {enum_name}",
            parameters.join(", ")
        )
    }
}

fn enum_case_documentation(
    enum_info: Option<&EnumSemanticInfo>,
    case: Option<&doriac::semantics::EnumCaseSemanticInfo>,
) -> Option<String> {
    let case = case?;
    if !case.payload.is_empty() {
        let ownership = enum_info.map_or("enum", |info| {
            if info.capabilities.copy {
                "Copy enum"
            } else {
                "Move enum"
            }
        });
        return Some(format!(
            "Constructs a {ownership} value with readonly payload fields."
        ));
    }
    match (
        &case.backing_value,
        enum_info.and_then(|info| info.backing_type),
    ) {
        (Some(EnumBackingValue::Int(value)), Some(EnumBackingType::Int)) => Some(format!(
            "Enum case with readonly `int` backing value `{value}`."
        )),
        (Some(EnumBackingValue::String(value)), Some(EnumBackingType::String)) => Some(format!(
            "Enum case with readonly `string` backing value `{:?}`.",
            value
        )),
        _ => Some("Unit enum case.".to_string()),
    }
}

fn enum_documentation(info: &EnumSemanticInfo) -> String {
    let ownership = if info.capabilities.copy {
        "Copy"
    } else {
        "Move"
    };
    format!("Nominal {ownership} enum. Each value records one declared case and that case's data.")
}

fn attribute_schema_documentation(schema: &AttributeClassSchema) -> String {
    let parameters = if schema.parameters.is_empty() {
        "None".to_string()
    } else {
        schema
            .parameters
            .iter()
            .map(|parameter| {
                let default = if parameter.has_default {
                    " = default"
                } else {
                    ""
                };
                format!(
                    "- `{}`: `{}`{default}",
                    parameter.name,
                    doriac::attributes::metadata_type_name(&parameter.ty)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "**Typed Attribute Schema**\n\nConstructor Parameters:\n{parameters}\n\n**Metadata Only**\n\nAttribute constructors are not executed while metadata is produced. Doria does not provide runtime reflection for attributes."
    )
}

fn attribute_application_documentation(application: &AttributeApplication) -> String {
    let arguments = if application.bound_arguments.is_empty() {
        "None".to_string()
    } else {
        application
            .bound_arguments
            .iter()
            .map(|argument| {
                let defaulted = if argument.defaulted { " (default)" } else { "" };
                format!(
                    "- `{}`: `{}` = `{}`{defaulted}",
                    argument.parameter_name,
                    doriac::attributes::metadata_type_name(&argument.ty),
                    attribute_value_display(&argument.value)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let (heading, compiler_note) = match application.canonical_class_name.as_str() {
        "Test" => (
            "Test - Compiler-Known Attribute Metadata".to_string(),
            "\n\n**Execution Lands In Stage 33 Baton Test Orchestration**",
        ),
        "PHPExport" => (
            "PHPExport - Compiler-Known Attribute Metadata".to_string(),
            "\n\n**Bridge Semantics Land In Stage 41**",
        ),
        _ => (
            format!("Attribute `{}`", application.canonical_class_name),
            "",
        ),
    };
    format!(
        "**{heading}**\n\nTarget: {}\n\nArguments:\n{arguments}\n\n**Metadata Only**\n\n**No Runtime Reflection**{compiler_note}",
        attribute_target_display(&application.target),
    )
}

fn attribute_target_display(target: &AttributeTarget) -> String {
    match target {
        AttributeTarget::GlobalDeclaration { declaration, kind } => {
            format!("{} `{}`", kind.protocol_name(), declaration.qualified_name)
        }
        AttributeTarget::ClassMember {
            class, kind, name, ..
        } => format!(
            "{} `{}::{name}`",
            kind.protocol_name(),
            class.qualified_name
        ),
        AttributeTarget::CallableParameter {
            callable,
            parameter_name,
            roles,
            ..
        } => format!(
            "{} `{callable}(${parameter_name})`",
            roles
                .iter()
                .map(|role| role.protocol_name())
                .collect::<Vec<_>>()
                .join("+")
        ),
        AttributeTarget::EnumCase {
            enumeration,
            case_name,
            ..
        } => format!("enum case `{}::{case_name}`", enumeration.qualified_name),
        AttributeTarget::EnumPayloadField {
            enumeration,
            case_index,
            field_name,
            ..
        } => format!(
            "enum payload field `{}::case#{case_index}(${field_name})`",
            enumeration.qualified_name,
        ),
    }
}

fn attribute_value_display(value: &AttributeValue) -> String {
    match &value.value {
        AttributeValueKind::Integer { value } | AttributeValueKind::Float { value } => {
            value.clone()
        }
        AttributeValueKind::String(value) => format!("\"{}\"", value.escape_default()),
        AttributeValueKind::Bool(value) => value.to_string(),
        AttributeValueKind::Null => "null".to_string(),
        AttributeValueKind::Enum { case } => case.clone(),
        AttributeValueKind::PayloadEnum { case, fields } => format!(
            "{}({})",
            case,
            fields
                .iter()
                .map(attribute_value_display)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn append_documentation(documentation: &mut Option<String>, addition: &str) {
    match documentation {
        Some(documentation) => {
            documentation.push_str("\n\n");
            documentation.push_str(addition);
        }
        None => *documentation = Some(addition.to_string()),
    }
}

fn display_resolved_type(ty: &ResolvedType) -> String {
    let mut types = TypeRegistry::new();
    let id = types.intern_resolved(ty);
    types.display(id)
}

fn display_function_type_with_effects(ty: &ResolvedType, effects: &[ResolvedType]) -> String {
    let mut source_ordered = ty.clone();
    if let ResolvedType::Function(function) = &mut source_ordered {
        function.checked_effects = effects.to_vec();
    }
    display_resolved_type(&source_ordered)
}

fn effect_list(effects: &[ResolvedType]) -> String {
    if effects.is_empty() {
        "none".to_string()
    } else {
        effects
            .iter()
            .map(display_resolved_type)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn required_effect_documentation(effects: &[ResolvedType]) -> String {
    if effects.is_empty() {
        String::new()
    } else {
        format!("**Required checked effects:** {}", effect_list(effects))
    }
}

fn ambient_effect_documentation(effects: &[ResolvedType]) -> String {
    if effects.is_empty() {
        return String::new();
    }
    let effects = effects
        .iter()
        .map(|effect| format!("- `{}`", display_resolved_type(effect)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "\n\n**Ambient I/O:**\n\n{effects}\n\nAmbient I/O uses checked runtime transport without requiring source `throws`."
    )
}

fn test_assertion_effect_documentation(effects: &[ResolvedType]) -> String {
    if effects.is_empty() {
        return String::new();
    }
    let effects = effects
        .iter()
        .map(|effect| format!("- `{}`", display_resolved_type(effect)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "\n\n**Test assertions:**\n\n{effects}\n\nAssertion failures propagate automatically in development tests and helpers without requiring source `throws`."
    )
}

fn closure_target_capabilities() -> &'static str {
    "**Execution capability:** Executable In Debug And Native Targets\n\n**PHP compatibility:** Explicit closure lowering is available when the program's value families and operations are supported by the PHP backend."
}

fn invocation_mode_name(mode: FunctionInvocationMode) -> &'static str {
    match mode {
        FunctionInvocationMode::Readonly => "readonly",
        FunctionInvocationMode::Writable => "writable",
        FunctionInvocationMode::Once => "once",
    }
}

fn capture_acquisition_name(kind: CaptureAcquisitionKind) -> &'static str {
    match kind {
        CaptureAcquisitionKind::ReadonlyLease => "Readonly",
        CaptureAcquisitionKind::WritableLease => "Writable",
        CaptureAcquisitionKind::CopyIntoEnvironment
        | CaptureAcquisitionKind::MoveIntoEnvironment => "Owned taking",
    }
}

fn closure_invocation_summary(
    mode: FunctionInvocationMode,
    consumption: InvocationConsumption,
) -> &'static str {
    match (mode, consumption) {
        (FunctionInvocationMode::Readonly, InvocationConsumption::Repeatable) => {
            "Readonly Repeatable"
        }
        (FunctionInvocationMode::Writable, InvocationConsumption::Repeatable) => {
            "Writable Repeatable"
        }
        (FunctionInvocationMode::Once, InvocationConsumption::Repeatable) => "Repeatable",
        (_, InvocationConsumption::Once) => "Consumes On Invocation",
    }
}

fn closure_provenance_summary(info: &SemanticInfo, provenance: &ClosureValueProvenance) -> String {
    match provenance {
        ClosureValueProvenance::Owned => "Owned Closure".to_string(),
        ClosureValueProvenance::BorrowBound(roots) => {
            let roots = closure_root_names(info, roots);
            if roots.is_empty() {
                "Borrow-Bound Closure".to_string()
            } else {
                format!("Borrow-Bound Closure tied to {}", roots.join(", "))
            }
        }
    }
}

fn closure_escape_summary(
    info: &SemanticInfo,
    escape: ClosureEscapeClassification,
    provenance: &ClosureValueProvenance,
) -> String {
    match escape {
        ClosureEscapeClassification::Local => "Nonescaping".to_string(),
        ClosureEscapeClassification::Owned => "Owned callback".to_string(),
        ClosureEscapeClassification::ReturnedBorrow => {
            let roots = match provenance {
                ClosureValueProvenance::BorrowBound(roots) => closure_root_names(info, roots),
                ClosureValueProvenance::Owned => Vec::new(),
            };
            if roots.is_empty() {
                "Returned closure with a compiler-checked borrow".to_string()
            } else {
                format!("Returned closure tied to {}", roots.join(", "))
            }
        }
    }
}

fn closure_root_names(info: &SemanticInfo, roots: &[ClosureBorrowRoot]) -> Vec<String> {
    let mut names = roots
        .iter()
        .map(|root| match root {
            ClosureBorrowRoot::Binding(binding) => binding_source_name(info, *binding),
            ClosureBorrowRoot::Receiver => "$this".to_string(),
            ClosureBorrowRoot::EnclosingEnvironment(_) => "the enclosing closure".to_string(),
            ClosureBorrowRoot::Temporary => "a temporary receiver".to_string(),
        })
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

fn binding_source_name(info: &SemanticInfo, binding_id: doriac::symbols::BindingId) -> String {
    let name = info
        .binding_resolution
        .declarations_by_id
        .get(&binding_id)
        .map_or("value", |declaration| declaration.name.as_str());
    format!("${name}")
}

fn function_signature(function: &FunctionDecl, container: Option<&str>) -> String {
    let mut modifiers = Vec::new();
    if matches!(function.access, MemberAccess::Internal) {
        modifiers.push("internal");
    }
    if function.writable_this {
        modifiers.push("writable");
    }
    if function.is_static {
        modifiers.push("static");
    }

    let prefix = if modifiers.is_empty() {
        String::new()
    } else {
        format!("{} ", modifiers.join(" "))
    };
    let name = container
        .map(|container| format!("{container}::{}", function.name))
        .unwrap_or_else(|| function.name.clone());
    let type_parameters = type_parameter_signature(&function.type_params);
    let parameters = function
        .params
        .iter()
        .map(parameter_signature)
        .collect::<Vec<_>>()
        .join(", ");
    let return_type = function
        .return_type
        .as_ref()
        .map(|return_type| format!(": {return_type}"))
        .unwrap_or_default();
    let throws = function
        .throws
        .as_ref()
        .map(|clause| {
            format!(
                " throws {}",
                clause
                    .entries
                    .iter()
                    .map(|entry| entry.ty.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .unwrap_or_default();

    format!("{prefix}function {name}{type_parameters}({parameters}){return_type}{throws}")
}

fn parameter_signature(parameter: &Param) -> String {
    let mut parts = Vec::new();
    if matches!(parameter.promoted_access, Some(MemberAccess::Internal)) {
        parts.push("internal".to_string());
    }
    if parameter.take {
        parts.push("take".to_string());
    }
    if parameter.writable {
        parts.push("writable".to_string());
    }
    parts.push(parameter.ty.to_string());
    parts.push(format!("${}", parameter.name));
    let mut rendered = parts.join(" ");
    if parameter.default.is_some() {
        rendered.push_str(" = ...");
    }
    rendered
}

fn type_parameter_signature(parameters: &[doriac::ast::TypeParamDecl]) -> String {
    if parameters.is_empty() {
        return String::new();
    }
    let parameters = parameters
        .iter()
        .map(|parameter| {
            let mut rendered = parameter.name.clone();
            if !parameter.constraints.is_empty() {
                rendered.push_str(" implements ");
                rendered.push_str(
                    &parameter
                        .constraints
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", "),
                );
            }
            if let Some(default) = &parameter.default_type {
                rendered.push_str(&format!(" = {default}"));
            }
            rendered
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("<{parameters}>")
}

fn class_signature(class: &ClassDecl) -> String {
    let mut signature = format!(
        "class {}{}",
        class.name,
        type_parameter_signature(&class.type_params)
    );
    if let Some(parent) = &class.parent {
        signature.push_str(&format!(" extends {parent}"));
    }
    if !class.implements.is_empty() {
        signature.push_str(&format!(" implements {}", class.implements.join(", ")));
    }
    signature
}

fn phpdoc_before(text: &str, declaration_start: usize) -> Option<String> {
    let prefix = text.get(..declaration_start)?.trim_end();
    if !prefix.ends_with("*/") {
        return None;
    }
    let start = prefix.rfind("/**")?;
    let raw = prefix.get(start + 3..prefix.len().saturating_sub(2))?;
    let lines = raw
        .lines()
        .map(|line| {
            let line = line.trim();
            line.strip_prefix('*').unwrap_or(line).trim().to_string()
        })
        .collect::<Vec<_>>();

    render_phpdoc(&lines)
}

fn render_phpdoc(lines: &[String]) -> Option<String> {
    let mut narrative = Vec::new();
    let mut parameters = Vec::new();
    let mut returns = Vec::new();
    let mut throws = Vec::new();
    let mut tags = Vec::new();

    for line in lines {
        if let Some(rest) = line.strip_prefix("@param ") {
            parameters.push(render_doc_tag(rest, 2));
        } else if let Some(rest) = line.strip_prefix("@return ") {
            returns.push(render_doc_tag(rest, 1));
        } else if let Some(rest) = line.strip_prefix("@throws ") {
            throws.push(render_doc_tag(rest, 1));
        } else if line.starts_with('@') {
            tags.push(format!("`{line}`"));
        } else {
            narrative.push(line.clone());
        }
    }

    let mut sections = Vec::new();
    let narrative = trim_blank_lines(&narrative).join("\n");
    if !narrative.is_empty() {
        sections.push(narrative);
    }
    push_doc_section(&mut sections, "Parameters", &parameters);
    push_doc_section(&mut sections, "Returns", &returns);
    push_doc_section(&mut sections, "Throws", &throws);
    push_doc_section(&mut sections, "Tags", &tags);

    (!sections.is_empty()).then(|| sections.join("\n\n"))
}

fn render_doc_tag(value: &str, code_parts: usize) -> String {
    let mut parts = value.split_whitespace();
    let mut code = Vec::new();
    for _ in 0..code_parts {
        if let Some(part) = parts.next() {
            code.push(part);
        }
    }
    let description = parts.collect::<Vec<_>>().join(" ");
    if description.is_empty() {
        format!("- `{}`", code.join(" "))
    } else {
        format!("- `{}`: {description}", code.join(" "))
    }
}

fn push_doc_section(sections: &mut Vec<String>, title: &str, entries: &[String]) {
    if !entries.is_empty() {
        sections.push(format!("**{title}**\n\n{}", entries.join("\n")));
    }
}

fn trim_blank_lines(lines: &[String]) -> &[String] {
    let start = lines
        .iter()
        .position(|line| !line.is_empty())
        .unwrap_or(lines.len());
    let end = lines
        .iter()
        .rposition(|line| !line.is_empty())
        .map(|index| index + 1)
        .unwrap_or(start);
    &lines[start..end]
}

fn tokens_in_span(tokens: &[Token], span: Span) -> impl Iterator<Item = &Token> {
    tokens
        .iter()
        .filter(move |token| token.span.start >= span.start && token.span.end <= span.end)
}

fn find_identifier_span(tokens: &[Token], span: Span, name: &str) -> Option<Span> {
    tokens_in_span(tokens, span)
        .find(|token| identifier_is(token, name))
        .map(|token| token.span)
}

fn find_variable_span(tokens: &[Token], span: Span, name: &str) -> Option<Span> {
    tokens_in_span(tokens, span)
        .find(|token| matches!(&token.kind, TokenKind::Variable(variable) if variable == name))
        .map(|token| token.span)
}

fn identifier_is(token: &Token, name: &str) -> bool {
    matches!(&token.kind, TokenKind::Identifier(identifier) if identifier == name)
}

fn same_token_variant(left: &TokenKind, right: &TokenKind) -> bool {
    std::mem::discriminant(left) == std::mem::discriminant(right)
}

fn span_contains(span: Span, offset: usize) -> bool {
    span.start <= offset && offset <= span.end
}

fn spans_overlap(left: Span, right: Span) -> bool {
    left.start < right.end && right.start < left.end
}

fn diagnostic_overlaps_span(diagnostic: &Diagnostic, span: Span) -> bool {
    // `Diagnostic::span` is the current-source primary range. Secondary labels
    // can identify enclosing or causally related constructs and must not make
    // an otherwise valid closure inherit their capability suppression.
    spans_overlap(diagnostic.span, span)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hover(source: &str, needle: &str, occurrence: usize) -> SemanticHover {
        let offset = source
            .match_indices(needle)
            .nth(occurrence)
            .unwrap_or_else(|| panic!("missing occurrence {occurrence} of `{needle}`"))
            .0;
        let snapshot = AnalysisSnapshot::analyze("test.doria", source);
        snapshot.hover_at_offset(offset).unwrap_or_else(|| {
            panic!(
                "expected semantic hover for occurrence {occurrence} of `{needle}`; diagnostics: {:?}",
                snapshot.diagnostics()
            )
        })
    }

    #[test]
    fn enum_hovers_and_completions_use_compiler_semantic_metadata() {
        let source = r#"enum Status { case Draft; case Published; }
enum Priority: int { case Low = 1; case High = 2; }
enum Shape { case Circle(float $radius); }
class Document {}
enum LoadResult { case Loaded(Document $document); }

function main(): void
{
    Status $status = Status::Draft;
    Priority $priority = Priority::High;
    ?Shape $maybe = Shape::Circle(2.5);
    echo $priority->value;
}
"#;
        let snapshot = AnalysisSnapshot::analyze("test.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );

        assert!(hover(source, "Status", 0).markdown.contains("enum Status"));
        assert!(hover(source, "Draft", 0)
            .markdown
            .contains("Status::Draft: Status"));
        assert!(hover(source, "High", 0)
            .markdown
            .contains("Priority::High: Priority"));
        assert!(hover(source, "Circle", 0)
            .markdown
            .contains("Shape::Circle(float $radius): Shape"));
        assert!(hover(source, "Circle", 0)
            .markdown
            .contains("Constructs a Copy enum value"));
        assert!(hover(source, "Loaded", 0)
            .markdown
            .contains("Constructs a Move enum value"));
        assert!(hover(source, "$radius", 0)
            .markdown
            .contains("float $radius"));
        assert!(hover(source, "$document", 0)
            .markdown
            .contains("Document $document"));
        assert!(hover(source, "$maybe", 0)
            .markdown
            .contains("?Shape $maybe"));
        assert!(hover(source, "value", 0).markdown.contains("int $value"));

        let draft_offset = source.rfind("Draft").expect("enum case reference");
        let static_labels = snapshot
            .static_completions_at_offset(draft_offset)
            .expect("enum static completion context")
            .into_iter()
            .map(|completion| completion.label)
            .collect::<HashSet<_>>();
        assert_eq!(
            static_labels,
            HashSet::from(["Draft".to_string(), "Published".to_string()])
        );

        let value_offset = source.rfind("value").expect("backed value property");
        let members = snapshot
            .member_completions_at_offset(value_offset)
            .expect("backed enum member completion context");
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].label, "value");
        assert_eq!(members[0].detail, "int $value");
    }

    #[test]
    fn compiler_known_semantic_declarations_drive_member_completion_and_hover() {
        let source = r#"function inspect(Doria\Std\Io\IoError $error): void
{
    let $emoji = "😀";
    Doria\Std\Io\IoErrorReason $reason = $error->reason;
    Doria\Std\Io\IoOperation $operation = Doria\Std\Io\IoOperation::Read;
}
"#;
        let snapshot = AnalysisSnapshot::analyze("compiler-known.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );

        let property_offset = source.rfind("reason").expect("I/O error property");
        let members = snapshot
            .member_completions_at_offset(property_offset)
            .expect("compiler-known class member completion context");
        let details = members
            .into_iter()
            .map(|completion| (completion.label, completion.detail))
            .collect::<HashMap<_, _>>();
        assert_eq!(
            details.get("message").map(String::as_str),
            Some("string $message")
        );
        assert_eq!(
            details.get("operation").map(String::as_str),
            Some("Doria\\Std\\Io\\IoOperation $operation")
        );
        assert_eq!(
            details.get("target").map(String::as_str),
            Some("Doria\\Std\\Io\\IoTarget $target")
        );
        assert_eq!(
            details.get("reason").map(String::as_str),
            Some("Doria\\Std\\Io\\IoErrorReason $reason")
        );
        assert_eq!(
            details.get("systemCode").map(String::as_str),
            Some("?int $systemCode")
        );
        assert!(snapshot
            .hover_at_offset(property_offset)
            .expect("compiler-known property hover")
            .markdown
            .contains("Doria\\Std\\Io\\IoErrorReason $reason"));

        let case_offset = source.rfind("Read").expect("I/O operation case");
        let cases = snapshot
            .static_completions_at_offset(case_offset)
            .expect("compiler-known enum case completion context")
            .into_iter()
            .map(|completion| completion.label)
            .collect::<HashSet<_>>();
        assert_eq!(
            cases,
            HashSet::from([
                "Open".to_string(),
                "Read".to_string(),
                "Write".to_string(),
                "Append".to_string(),
                "Flush".to_string(),
            ])
        );
        assert!(snapshot
            .hover_at_offset(case_offset)
            .expect("compiler-known enum case hover")
            .markdown
            .contains("Doria\\Std\\Io\\IoOperation::Read: Doria\\Std\\Io\\IoOperation"));
    }

    #[test]
    fn executable_match_bindings_share_hover_reference_rename_and_scope_identity() {
        let payload = AnalysisSnapshot::analyze(
            "payload.doria",
            "enum Shape { case Circle(float $radius); } Shape $shape = Shape::Circle(2.5);",
        );
        assert!(
            payload.diagnostics().is_empty(),
            "{:?}",
            payload.diagnostics()
        );

        let matching_source = r#"enum Result { case Value(string $text); case Missing; }
function main(): void
{
    Result $result = Result::Value("ok");
    string $label = match ($result) {
        Result::Value($value) => "😀 {$value}",
        Result::Missing => "missing",
    };
}"#;
        let matching = AnalysisSnapshot::analyze("match.doria", matching_source);
        assert!(
            matching.diagnostics().is_empty(),
            "{:?}",
            matching.diagnostics()
        );
        assert!(hover(matching_source, "match", 0)
            .markdown
            .contains("match (...): string"));
        assert!(hover(matching_source, "$result", 1)
            .markdown
            .contains("Result $result"));
        assert!(hover(matching_source, "Value", 2)
            .markdown
            .contains("Result::Value(string $text): Result"));
        assert!(hover(matching_source, "$value", 0)
            .markdown
            .contains("string $value"));

        let binding_offset = matching_source.find("$value").expect("pattern binding");
        assert_eq!(
            matching
                .reference_spans_at_offset(binding_offset, true)
                .len(),
            2
        );
        assert_eq!(
            matching.rename_replacement_at_offset(binding_offset, "renamed"),
            Some("$renamed".to_string())
        );
        assert!(matching
            .semantic_token_spans()
            .iter()
            .any(
                |(span, token_type, _)| *span == Span::new(binding_offset, binding_offset + 6)
                    && *token_type == 0
            ));

        let leaked = AnalysisSnapshot::analyze(
            "leaked.doria",
            "enum Result { case Value(string $text); } function main(): void { Result $r = Result::Value(\"ok\"); string $v = match ($r) { Result::Value($inside) => $inside, }; echo $inside; }",
        );
        assert!(leaked
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == "E0101"));
    }

    #[test]
    fn control_flow_foundations_share_semantic_scope_hover_and_rename_identity() {
        let source = r#"function main(): void
{
    echo "😀"; given {
        /* 😀 */ let $prepared = true;
        /* 😀 */ true;
    } if ($prepared) {
        echo "{$prepared}";
    }

    echo "😀"; string $label = given {
        let $choice = true;
        true;
    } when ($choice): string {
        echo "😀"; let $branch = 1;
        echo "{$branch}";
        echo "😀"; return "selected";
    } else {
        return "fallback";
    };

    do {
        echo $label;
    } while (/* 😀 */ true);
}
"#;
        let snapshot = AnalysisSnapshot::analyze("control-flow.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );

        let prepared = source.find("$prepared").expect("given declaration");
        assert!(snapshot
            .hover_at_offset(prepared)
            .expect("given local hover")
            .markdown
            .contains("bool $prepared"));
        assert_eq!(
            snapshot.reference_spans_at_offset(prepared, true).len(),
            3,
            "given declaration, attached condition, and selected branch share one symbol"
        );
        assert_eq!(
            snapshot.rename_replacement_at_offset(prepared, "ready"),
            Some("$ready".to_string())
        );
        assert!(snapshot
            .semantic_token_spans()
            .iter()
            .any(|(span, token_type, _)| span.start == prepared && *token_type == 0));

        let if_body = source.find("echo \"{$prepared}\"").unwrap();
        assert!(snapshot
            .local_completions_at_offset(if_body)
            .iter()
            .any(|completion| completion.label == "$prepared"));
        let after_if = source.find("string $label").unwrap();
        assert!(!snapshot
            .local_completions_at_offset(after_if)
            .iter()
            .any(|completion| completion.label == "$prepared"));

        let predicate = source.find("true;\n    } if").unwrap();
        assert!(snapshot
            .hover_at_offset(predicate)
            .expect("given predicate hover")
            .markdown
            .contains("given predicate: bool"));
        assert!(hover(source, "when", 0)
            .markdown
            .contains("when (...): string"));
        assert!(hover(source, "return", 0)
            .markdown
            .contains("Yields a value"));
        assert!(hover(source, "true);", 0)
            .markdown
            .contains("do ... while condition: bool"));

        let branch = source.rfind("$branch").expect("when branch local use");
        assert!(snapshot
            .local_completions_at_offset(branch)
            .iter()
            .any(|completion| completion.label == "$branch"));
        let after_when = source.find("do {").unwrap();
        assert!(!snapshot
            .local_completions_at_offset(after_when)
            .iter()
            .any(|completion| completion.label == "$branch"));
    }

    #[test]
    fn semantic_tokens_keep_methods_distinct_from_enum_cases() {
        let source = r#"class Worker
{
    function operate(): void {}
    static function build(): void {}
}

enum Status { case Ready; }

function inspect(): void
{
    let $worker = new Worker();
    $worker->operate();
    Worker::build();
    Status $status = Status::Ready;
}
"#;
        let snapshot = AnalysisSnapshot::analyze("methods.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );
        let tokens = snapshot.semantic_token_spans();

        for name in ["operate", "build"] {
            let occurrences = source
                .match_indices(name)
                .map(|(start, _)| Span::new(start, start + name.len()))
                .collect::<Vec<_>>();
            assert_eq!(occurrences.len(), 2);
            for span in occurrences {
                assert!(
                    tokens.iter().any(|(token_span, token_type, _)| {
                        *token_span == span && *token_type == 3
                    }),
                    "method `{name}` at {span:?} must use the function token type"
                );
            }
        }

        for (start, _) in source.match_indices("Ready") {
            let span = Span::new(start, start + "Ready".len());
            assert!(
                tokens
                    .iter()
                    .any(|(token_span, token_type, _)| { *token_span == span && *token_type == 2 }),
                "enum case at {span:?} must retain the enum-member token type"
            );
        }
    }

    #[test]
    fn semantic_tokens_traverse_attribute_argument_values() {
        let source = r#"enum HttpMethod { case Get; case Post; }
#[Attribute]
class Route
{
    function __construct(HttpMethod $method) {}
}
#[Route(method: HttpMethod::Post)]
function main(): void {}
"#;
        let snapshot = AnalysisSnapshot::analyze("attributes.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );
        let tokens = snapshot.semantic_token_spans();
        let value_start = source.rfind("HttpMethod::Post").unwrap();
        let type_span = Span::new(value_start, value_start + "HttpMethod".len());
        let case_start = value_start + "HttpMethod::".len();
        let case_span = Span::new(case_start, case_start + "Post".len());

        assert!(tokens
            .iter()
            .any(|(span, token_type, _)| *span == type_span && *token_type == 1));
        assert!(tokens
            .iter()
            .any(|(span, token_type, _)| *span == case_span && *token_type == 2));
    }

    #[test]
    fn executable_finally_keeps_ast_hover_without_a_compiler_diagnostic() {
        let source = r#"function main(): void
{
    if (true) {
        let $body = "body";
    } /* 😀 */ finally {
        let $cleanup = "cleanup";
    }
}
"#;
        let snapshot = AnalysisSnapshot::analyze("finally.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );
        assert!(snapshot
            .hover_at_offset(source.find("finally").unwrap())
            .expect("finally hover")
            .markdown
            .contains("Runs exactly once"));
    }

    #[test]
    fn guarded_consuming_match_keeps_one_source_binding_identity() {
        let source = r#"class Document
{
    function __construct(string $name) {}
    function isReady(): bool { return true; }
}
enum LoadResult { case Loaded(Document $document); case Missing; }
function main(): void
{
    echo "😀";
    LoadResult $result = LoadResult::Loaded(new Document("ready"));
    Document $selected = match (take $result) {
        LoadResult::Loaded($document) if $document->isReady() => $document,
        LoadResult::Loaded($document) => $document,
        LoadResult::Missing => new Document("fallback"),
    };
}"#;
        let snapshot = AnalysisSnapshot::analyze("guarded-take.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );

        let take_offset = source.find("take").expect("take modifier");
        let take_hover = snapshot
            .hover_at_offset(take_offset)
            .expect("consuming-match hover");
        assert!(take_hover.markdown.contains("whole Move value"));

        let match_offset = source.find("match (take").expect("consuming match");
        let binding_offset = source[match_offset..]
            .find("LoadResult::Loaded($document)")
            .map(|offset| match_offset + offset + "LoadResult::Loaded(".len())
            .expect("payload binding");
        let guard_offset = source[binding_offset + 1..]
            .find("$document")
            .map(|offset| binding_offset + 1 + offset)
            .expect("guard reference");
        let arm_offset = source[guard_offset + 1..]
            .find("$document")
            .map(|offset| guard_offset + 1 + offset)
            .expect("arm reference");
        for offset in [binding_offset, guard_offset, arm_offset] {
            let hover = snapshot
                .hover_at_offset(offset)
                .expect("guarded binding hover");
            assert!(hover.markdown.contains("Document $document"));
            assert!(hover.markdown.contains("Readonly while the guard"));
            assert!(hover.markdown.contains("owns the selected Move payload"));
        }
        assert_eq!(
            snapshot
                .reference_spans_at_offset(binding_offset, true)
                .len(),
            3
        );
        assert_eq!(
            snapshot.rename_replacement_at_offset(guard_offset, "payload"),
            Some("$payload".to_string())
        );

        let second_binding = source[arm_offset + 1..]
            .find("$document")
            .map(|offset| arm_offset + 1 + offset)
            .expect("second-arm binding");
        assert_eq!(
            snapshot
                .reference_spans_at_offset(second_binding, true)
                .len(),
            2,
            "each arm owns a separate lexical binding"
        );
    }

    #[test]
    fn copy_pattern_binding_masks_an_outer_move_symbol() {
        let source = r#"class Box {}
enum Number { case Value(int $item); }
function consume(take Box $item): void {}
function main(): void
{
    Box $item = new Box();
    consume($item);
    int $value = match (Number::Value(42)) {
        Number::Value($item) => $item,
    };
}"#;
        let snapshot = AnalysisSnapshot::analyze("copy-shadow.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        let pattern_offset = source.rfind("Number::Value($item)").unwrap() + "Number::Value(".len();
        let pattern_hover = snapshot
            .hover_at_offset(pattern_offset)
            .expect("Copy pattern binding hover");
        assert!(pattern_hover.markdown.contains("int $item"));
        assert_eq!(
            snapshot
                .reference_spans_at_offset(pattern_offset, true)
                .len(),
            2,
            "the Copy pattern binding must not resolve to the outer Box"
        );
        assert_eq!(
            snapshot.rename_replacement_at_offset(pattern_offset, "number"),
            Some("$number".to_string())
        );
        let outer_offset = source.find("$item = new Box").expect("outer binding");
        let outer_hover = snapshot
            .hover_at_offset(outer_offset)
            .expect("outer Move binding hover");
        assert!(outer_hover.markdown.contains("Box $item"));
        assert_eq!(
            snapshot.reference_spans_at_offset(outer_offset, true).len(),
            2,
            "the moved outer symbol remains independent"
        );
    }

    #[test]
    fn exact_type_match_binding_hovers_with_its_narrowed_type() {
        let source = r#"class Document {}
function label(mixed $value): string
{
    return match ($value) {
        Document $document => "document",
        string $text => $text,
        default => "other",
    };
}"#;
        let snapshot = AnalysisSnapshot::analyze("types.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        assert!(hover(source, "$document", 0)
            .markdown
            .contains("Document $document"));
        assert!(hover(source, "$text", 0).markdown.contains("string $text"));
        assert!(hover(source, "Document", 1)
            .markdown
            .contains("class Document"));
    }

    #[test]
    fn mixed_function_narrowing_hovers_with_the_compiler_resolved_identity() {
        let source = r#"function main(): void
{
    mixed $value = fn(int $number) => $number + 1;

    if ($value is function(int): int) {
        int $result = $value(41);
    }
}"#;
        let snapshot = AnalysisSnapshot::analyze("mixed-function-hover.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );

        let declaration = snapshot
            .hover_at_offset(source.find("$value").expect("mixed declaration"))
            .expect("mixed declaration hover");
        assert!(declaration.markdown.contains("mixed $value"));
        assert!(!declaration.markdown.contains("Execution capability"));

        let narrowed_offset = source.rfind("$value").expect("narrowed callable use");
        let narrowed = snapshot
            .hover_at_offset(narrowed_offset)
            .expect("narrowed function hover");
        assert!(narrowed.markdown.contains("function(int): int"));
        assert!(narrowed
            .markdown
            .contains("Compiler-resolved function value after flow narrowing"));
        assert!(narrowed
            .markdown
            .contains("Executable In Debug And Native Targets"));
        assert!(!narrowed.markdown.contains("mixed $value"));

        let invocation = snapshot
            .hover_at_offset(source.rfind("41").expect("call argument"))
            .expect("narrowed invocation hover");
        assert!(invocation.markdown.contains("function(int): int"));
        assert!(invocation
            .markdown
            .contains("Semantically checked callable-value invocation"));
    }

    #[test]
    fn structural_function_hovers_preserve_every_identity_dimension() {
        let source = r#"class ParseError implements Error
{
    function __construct(string $message) {}
}

function inspect(
    function(): int $plain,
    function writable(writable int): string throws ParseError $writable,
    function once(take string): int $once,
    ?function(): int $nullable,
    List<function(int): int> $callbacks,
    Dictionary<string, function(): int throws ParseError> $handlers
): void {}
"#;
        let snapshot = AnalysisSnapshot::analyze("function-identities.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );

        for (needle, expected) in [
            ("function(): int $plain", "function(): int"),
            (
                "function writable(writable int): string throws ParseError",
                "function writable(writable int): string throws ParseError",
            ),
            (
                "function once(take string): int",
                "function once(take string): int",
            ),
            ("?function(): int", "function(): int"),
            ("function(int): int", "function(int): int"),
            (
                "function(): int throws ParseError",
                "function(): int throws ParseError",
            ),
        ] {
            let offset = source
                .find(needle)
                .unwrap_or_else(|| panic!("missing `{needle}`"));
            let hover = snapshot
                .hover_at_offset(offset + usize::from(needle.starts_with('?')))
                .unwrap_or_else(|| panic!("missing hover for `{needle}`"));
            assert!(
                hover.markdown.contains(expected),
                "`{expected}` missing from {}",
                hover.markdown
            );
            assert!(!hover.markdown.contains("callable\n"));
        }

        let nullable_binding = snapshot
            .hover_at_offset(source.find("$nullable").expect("nullable binding"))
            .expect("nullable binding hover");
        assert!(nullable_binding
            .markdown
            .contains("?function(): int $nullable"));
    }

    #[test]
    fn wrong_mixed_function_identity_stays_distinct_from_the_actual_value() {
        let source = r#"function main(): void
{
    mixed $value = fn(int $number) => $number;
    if ($value is function(): int) {
        echo "wrong";
    }
    if ($value is function(int): int) {
        int $result = $value(42);
    }
}"#;
        let snapshot = AnalysisSnapshot::analyze("wrong-function-identity.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );

        let value = snapshot
            .hover_at_offset(source.find("fn(").expect("closure"))
            .expect("closure hover");
        assert!(value.markdown.contains("function(int): int"));

        let wrong = snapshot
            .hover_at_offset(source.find("function(): int").expect("wrong type test"))
            .expect("wrong type-test hover");
        assert!(wrong.markdown.contains("function(): int"));
        assert!(!wrong.markdown.contains("function(int): int"));

        let exact_use = snapshot
            .hover_at_offset(source.rfind("$value").expect("exact narrowed use"))
            .expect("exact narrowed hover");
        assert!(exact_use.markdown.contains("function(int): int"));
    }

    #[test]
    fn nullable_function_values_through_mixed_hover_only_after_exact_narrowing() {
        let source = r#"function main(): void
{
    ?function(): int $present = fn() => 7;
    mixed $boxedPresent = $present;
    ?function(): int $absent = null;
    mixed $boxedAbsent = $absent;

    if ($boxedPresent is function(): int) {
        int $presentValue = $boxedPresent();
    }
    if ($boxedAbsent is function(): int) {
        int $absentValue = $boxedAbsent();
    }
}"#;
        let snapshot = AnalysisSnapshot::analyze("nullable-mixed-functions.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );

        let boxed_declaration = snapshot
            .hover_at_offset(source.find("$boxedPresent").expect("boxed declaration"))
            .expect("boxed declaration hover");
        assert!(boxed_declaration.markdown.contains("mixed $boxedPresent"));
        assert!(!boxed_declaration.markdown.contains("Execution capability"));

        for variable in ["$boxedPresent", "$boxedAbsent"] {
            let narrowed_offset = source.rfind(variable).expect("narrowed callable use");
            let narrowed = snapshot
                .hover_at_offset(narrowed_offset)
                .expect("narrowed nullable function hover");
            assert!(narrowed.markdown.contains("function(): int"));
            assert!(!narrowed.markdown.contains("?function(): int"));
            assert!(narrowed.markdown.contains("Execution capability"));
        }
    }

    #[test]
    fn method_declaration_and_typed_call_share_the_same_signature() {
        let source = r#"class Greeter
{
    function greet(string $name): string
    {
        return "Hello";
    }
}

function main(): void
{
    let $greeter = new Greeter();
    echo $greeter->greet("Doria");
}
"#;

        let declaration = hover(source, "greet", 0);
        let call = AnalysisSnapshot::analyze("test.doria", source)
            .hover_at_offset(source.rfind("greet").expect("method call"))
            .expect("method call should have semantic hover");

        assert_eq!(declaration.markdown, call.markdown);
        assert!(call
            .markdown
            .contains("function Greeter::greet(string $name): string"));
        assert_eq!(&source[call.span.start..call.span.end], "greet");
    }

    #[test]
    fn this_calls_resolve_even_when_an_unrelated_semantic_error_exists() {
        let source = r#"class Greeter
{
    function greet(): string
    {
        return "Hello";
    }

    function run(): void
    {
        $this->greet();
        missing();
    }
}
"#;

        let snapshot = AnalysisSnapshot::analyze("test.doria", source);
        assert!(!snapshot.diagnostics().is_empty());
        let offset = source.rfind("greet").expect("method call");
        let method = snapshot
            .hover_at_offset(offset)
            .expect("the current class resolves independently of expression types");
        assert!(method
            .markdown
            .contains("function Greeter::greet(): string"));
    }

    #[test]
    fn phpdoc_is_attached_to_declaration_and_call_hovers() {
        let source = r#"class Greeter
{
    /**
     * Creates a greeting.
     *
     * @param string $name Person to greet.
     * @return string The greeting.
     * @throws GreetingError When greeting fails.
     */
    function greet(string $name): string
    {
        return "Hello";
    }

    function run(): void
    {
        echo $this->greet("Doria");
    }
}
"#;

        let call = AnalysisSnapshot::analyze("test.doria", source)
            .hover_at_offset(source.rfind("greet").expect("method call"))
            .expect("method call should have semantic hover");
        assert!(call.markdown.contains("Creates a greeting."));
        assert!(call.markdown.contains("**Parameters**"));
        assert!(call.markdown.contains("`string $name`: Person to greet."));
        assert!(call.markdown.contains("**Returns**"));
        assert!(call.markdown.contains("`string`: The greeting."));
        assert!(call.markdown.contains("**Throws**"));
    }

    #[test]
    fn receiver_type_selects_the_correct_same_named_method() {
        let source = r#"class Alpha
{
    function value(): int
    {
        return 1;
    }
}

class Beta
{
    function value(): string
    {
        return "two";
    }
}

function main(): void
{
    let $beta = new Beta();
    echo $beta->value();
}
"#;

        let call = hover(source, "value", 2);
        assert!(call.markdown.contains("function Beta::value(): string"));
        assert!(!call.markdown.contains("Alpha::value"));
    }

    #[test]
    fn free_and_static_calls_use_compiler_resolved_targets() {
        let source = r#"function increment(int $value): int
{
    return $value + 1;
}

class Counter
{
    static function next(int $value): int
    {
        return increment($value);
    }
}

function main(): int
{
    return Counter::next(41);
}
"#;
        let snapshot = AnalysisSnapshot::analyze("test.doria", source);
        assert!(snapshot.diagnostics().is_empty());

        let free_call = snapshot
            .hover_at_offset(source.rfind("increment").expect("free function call"))
            .expect("free function call should have semantic hover");
        assert!(free_call
            .markdown
            .contains("function increment(int $value): int"));

        let static_call = snapshot
            .hover_at_offset(source.rfind("next").expect("static method call"))
            .expect("static method call should have semantic hover");
        assert!(static_call
            .markdown
            .contains("static function Counter::next(int $value): int"));
    }

    #[test]
    fn standalone_blocks_preserve_semantic_method_hovers() {
        let source = r#"class Counter
{
    function ping(): void
    {
    }
}

function main(): void
{
    let $counter = new Counter();
    {
        $counter->ping();
    }
}
"#;
        let snapshot = AnalysisSnapshot::analyze("test.doria", source);
        assert!(snapshot.diagnostics().is_empty());

        let call = snapshot
            .hover_at_offset(source.rfind("ping").expect("method call"))
            .expect("method call inside standalone block should have semantic hover");
        assert!(call.markdown.contains("function Counter::ping(): void"));
    }

    #[test]
    fn generic_function_hovers_include_type_parameters_without_false_diagnostics() {
        let source = r#"function identity<T>(T $value): T
{
    return $value;
}

function main(): int
{
    return identity(42);
}
"#;
        let snapshot = AnalysisSnapshot::analyze("test.doria", source);
        assert!(snapshot.diagnostics().is_empty());

        let call = snapshot
            .hover_at_offset(source.rfind("identity").expect("generic function call"))
            .expect("generic function call should have semantic hover");
        assert!(call.markdown.contains("function identity<T>(T $value): T"));
    }

    #[test]
    fn generic_class_hovers_include_constraints_without_false_diagnostics() {
        let source = r#"class Box<T implements Displayable>
{
    function __construct(take T $value) {}
}

function main(): void
{
    let $box = new Box<int>(42);
}
"#;
        let snapshot = AnalysisSnapshot::analyze("test.doria", source);
        assert!(snapshot.diagnostics().is_empty());

        let declaration = snapshot
            .hover_at_offset(source.find("Box").expect("generic class declaration"))
            .expect("generic class declaration should have semantic hover");
        assert!(declaration
            .markdown
            .contains("class Box<T implements Displayable>"));

        let construction = snapshot
            .hover_at_offset(source.rfind("Box").expect("generic class construction"))
            .expect("generic class construction should resolve to the class");
        assert!(construction
            .markdown
            .contains("class Box<T implements Displayable>"));
    }

    #[test]
    fn string_intrinsic_hovers_use_the_canonical_surface() {
        let source = r#"function main(): void
{
    string $text = "Straße";
    int $length = $text->length;
    bool $found = String::containsIgnoreCase($text, "STRASSE");
    string $title = String::upperFirst("doria");
}
"#;
        let snapshot = AnalysisSnapshot::analyze("test.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "canonical String calls should not produce diagnostics: {:?}",
            snapshot.diagnostics()
        );

        let length = snapshot
            .hover_at_offset(source.rfind("length").expect("length property"))
            .expect("String length should have semantic hover");
        assert!(length.markdown.contains("int $length"));
        assert!(length.markdown.contains("extended grapheme clusters"));

        let contains = snapshot
            .hover_at_offset(
                source
                    .find("containsIgnoreCase")
                    .expect("String companion call"),
            )
            .expect("String companion call should have semantic hover");
        assert!(contains
            .markdown
            .contains("String::containsIgnoreCase(string $text, string $needle): bool"));
        assert!(contains
            .markdown
            .contains("full default Unicode case folding"));
    }

    #[test]
    fn compiler_known_method_hovers_substitute_concrete_return_types() {
        let source = r#"class Theme
{
    function __construct(string $name) {}
}

function releasedTheme(): WeakReference<Theme>
{
    let $theme = shared new Theme("dark");
    return $theme->createWeakReference();
}

function main(): void
{
    let $observer = releasedTheme();
    let $released = $observer->acquire();

    let $settings = new WritableSharedReference(new Theme("light"));
    let $writableObserver = $settings->createWeakReference();
    let $writableReleased = $writableObserver->acquire();

    writable Dictionary<string, int> $scores = ["Ada" => 3];
    let $score = $scores->get("Ada");
    let $containsScore = $scores->containsValue(3);
    writable SortedDictionary<string, int> $sortedScores = SortedDictionary::from(["Ada" => 3]);
    let $sortedScore = $sortedScores->get("Ada");
    writable SortedSet<int> $numbers = SortedSet::from([1, 2]);
    let $combined = $numbers->union(SortedSet::from([3]));
    let $first = $numbers->first;
    writable List<string> $colors = ["red", "blue"];
    let $blue = $colors->indexOf("blue");
    let $removed = $colors->remove("red");
    writable PriorityQueue<int> $work = PriorityQueue::from([2, 1]);
    let $next = $work->pop();
    writable Deque<string> $line = Deque::from(["middle"]);
    $line->pushFront("first");
}
"#;
        let snapshot = AnalysisSnapshot::analyze("test.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );

        let readonly = snapshot
            .hover_at_offset(source.find("$observer->acquire").unwrap() + "$observer->".len())
            .expect("readonly acquire should provide semantic hover");
        assert!(readonly
            .markdown
            .contains("function WeakReference<Theme>::acquire(): ?SharedReference<Theme>"));

        let writable = snapshot
            .hover_at_offset(
                source.find("$writableObserver->acquire").unwrap() + "$writableObserver->".len(),
            )
            .expect("writable acquire should provide semantic hover");
        assert!(writable.markdown.contains(
            "function WritableWeakReference<Theme>::acquire(): ?WritableSharedReference<Theme>"
        ));

        let dictionary = snapshot
            .hover_at_offset(source.find("$scores->get").unwrap() + "$scores->".len())
            .expect("dictionary get should provide semantic hover");
        assert!(dictionary
            .markdown
            .contains("function Dictionary<string, int>::get(string $key): ?int"));

        let contains_value = snapshot
            .hover_at_offset(source.find("$scores->containsValue").unwrap() + "$scores->".len())
            .expect("dictionary containsValue should provide semantic hover");
        assert!(contains_value
            .markdown
            .contains("function Dictionary<string, int>::containsValue(int $value): bool"));

        let sorted_dictionary = snapshot
            .hover_at_offset(source.find("$sortedScores->get").unwrap() + "$sortedScores->".len())
            .expect("sorted dictionary get should provide semantic hover");
        assert!(sorted_dictionary
            .markdown
            .contains("function SortedDictionary<string, int>::get(string $key): ?int"));

        let sorted_set = snapshot
            .hover_at_offset(source.find("$numbers->union").unwrap() + "$numbers->".len())
            .expect("sorted set union should provide semantic hover");
        assert!(sorted_set
            .markdown
            .contains("function SortedSet<int>::union(SortedSet<int> $other): SortedSet<int>"));

        let first = snapshot
            .hover_at_offset(source.find("$numbers->first").unwrap() + "$numbers->".len())
            .expect("sorted set first should provide semantic hover");
        assert!(first.markdown.contains("?int $first"));

        let index_of = snapshot
            .hover_at_offset(source.find("$colors->indexOf").unwrap() + "$colors->".len())
            .expect("list indexOf should provide semantic hover");
        assert!(index_of
            .markdown
            .contains("function List<string>::indexOf(string $value): ?int"));

        let list_remove = snapshot
            .hover_at_offset(source.find("$colors->remove").unwrap() + "$colors->".len())
            .expect("list remove should provide semantic hover");
        assert!(list_remove
            .markdown
            .contains("function List<string>::remove(string $value): bool"));

        let priority_queue = snapshot
            .hover_at_offset(source.find("$work->pop").unwrap() + "$work->".len())
            .expect("priority queue pop should provide semantic hover");
        assert!(priority_queue
            .markdown
            .contains("function PriorityQueue<int>::pop(): ?int"));

        let deque = snapshot
            .hover_at_offset(source.find("$line->pushFront").unwrap() + "$line->".len())
            .expect("deque pushFront should provide semantic hover");
        assert!(deque
            .markdown
            .contains("function Deque<string>::pushFront(string $value): void"));
    }

    #[test]
    fn decision_0113_hovers_cover_slice_three_and_in_place_clear() {
        let source = r#"function main(): void
{
    echo "😀";
    writable List<int> $list = [1];
    let $position = $list->indexOf(1);
    $list->remove(1);
    $list->clear();
    writable Dictionary<string, int> $dictionary = ["one" => 1];
    echo $dictionary->containsValue(1);
    $dictionary->clear();
    writable Set<int> $set = Set::from([1]);
    let $first = $set->first;
    let $last = $set->last;
    $set->clear();
    writable SortedDictionary<int, int> $sortedDictionary = SortedDictionary::from([1 => 1]);
    $sortedDictionary->clear();
    writable SortedSet<int> $sortedSet = SortedSet::from([1]);
    let $sortedFirst = $sortedSet->first;
    let $sortedLast = $sortedSet->last;
    $sortedSet->clear();
    writable PriorityQueue<int> $queue = PriorityQueue::from([1]);
    $queue->clear();
    writable Deque<int> $deque = Deque::from([1]);
    $deque->clear();
}
"#;
        let snapshot = AnalysisSnapshot::analyze("decision-0113.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );

        for (needle, expected) in [
            (
                "$list->indexOf",
                "function List<int>::indexOf(int $value): ?int",
            ),
            (
                "$list->remove",
                "function List<int>::remove(int $value): bool",
            ),
            ("$list->clear", "function List<int>::clear(): void"),
            (
                "$dictionary->containsValue",
                "function Dictionary<string, int>::containsValue(int $value): bool",
            ),
            (
                "$dictionary->clear",
                "function Dictionary<string, int>::clear(): void",
            ),
            ("$set->clear", "function Set<int>::clear(): void"),
            (
                "$sortedDictionary->clear",
                "function SortedDictionary<int, int>::clear(): void",
            ),
            (
                "$sortedSet->clear",
                "function SortedSet<int>::clear(): void",
            ),
            (
                "$queue->clear",
                "function PriorityQueue<int>::clear(): void",
            ),
            ("$deque->clear", "function Deque<int>::clear(): void"),
        ] {
            let offset = source.find(needle).unwrap() + needle.find("->").unwrap() + 2;
            let hover = snapshot
                .hover_at_offset(offset)
                .unwrap_or_else(|| panic!("missing hover for {needle}"));
            assert!(hover.markdown.contains(expected), "{}", hover.markdown);
            assert!(
                hover.markdown.contains("writable") || !needle.ends_with("clear"),
                "clear hover must explain writable mutation: {}",
                hover.markdown
            );
        }

        for (needle, expected) in [
            ("$set->first", "?int $first"),
            ("$set->last", "?int $last"),
            ("$sortedSet->first", "?int $first"),
            ("$sortedSet->last", "?int $last"),
        ] {
            let offset = source.find(needle).unwrap() + needle.find("->").unwrap() + 2;
            let hover = snapshot
                .hover_at_offset(offset)
                .unwrap_or_else(|| panic!("missing hover for {needle}"));
            assert!(hover.markdown.contains(expected), "{}", hover.markdown);
        }
    }

    #[test]
    fn stage_30g_list_completion_is_receiver_scoped() {
        let source = r#"function main(): void
{
    List<int> $list = [1];
    Set<int> $set = Set::from([1]);
    let $mapped = $list->map(fn(int $value) => $value);
    echo $set->contains(1);
}
"#;
        let snapshot = AnalysisSnapshot::analyze("stage30g-completion.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );

        let list_offset = source.find("$list->map").unwrap() + "$list->".len();
        let completions = snapshot
            .member_completions_at_offset(list_offset)
            .expect("List call should retain its resolved receiver");
        let labels = completions
            .iter()
            .map(|completion| completion.label.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(labels, HashSet::from(["map", "filter", "reduce"]));
        for (label, expected) in [
            (
                "map",
                "Transforms each element in insertion order and returns a new List. The source List remains unchanged.",
            ),
            (
                "filter",
                "Returns a new List containing selected Copy elements in insertion order. The source List remains unchanged.",
            ),
            (
                "reduce",
                "Owns the initial accumulator, lends it writably to the reducer for each element, and returns the completed accumulator.",
            ),
        ] {
            let completion = completions
                .iter()
                .find(|completion| completion.label == label)
                .expect("algorithm completion");
            assert_eq!(completion.documentation.as_deref(), Some(expected));
            assert!(!completion.detail.contains("effects("));
        }

        let set_offset = source.find("$set->contains").unwrap() + "$set->".len();
        let other = snapshot
            .member_completions_at_offset(set_offset)
            .expect("Set call should retain its resolved receiver");
        assert!(other
            .iter()
            .all(|completion| !matches!(completion.label.as_str(), "map" | "filter" | "reduce")));

        let int = Box::new(ResolvedType::Integer(doriac::numeric::IntegerType::Int64));
        for receiver in [
            ResolvedType::TypedArray(int.clone()),
            ResolvedType::Dictionary(Box::new(ResolvedType::String), int.clone()),
            ResolvedType::SortedDictionary(Box::new(ResolvedType::String), int.clone()),
            ResolvedType::Set(int.clone()),
            ResolvedType::SortedSet(int.clone()),
            ResolvedType::PriorityQueue(int.clone()),
            ResolvedType::Deque(int),
        ] {
            assert!(
                list_algorithm_completions(&receiver).is_none(),
                "non-List receiver received Stage 30g algorithms: {receiver:?}",
            );
        }
    }

    #[test]
    fn stage_30g_hover_uses_the_concrete_compiler_algorithm_plan() {
        let source = r#"class Failure implements Error
{
    function __construct(string $message) {}
}

function transform(): void throws Failure
{
    List<int> $values = [1, 2];
    let writable $calls = 0;
    let writable $callback = function (int $value): string with (writable $calls) {
        $calls += 1;
        if ($value == 2) { throw new Failure("stop"); }
        return "{$value}";
    };
    List<string> $mapped = $values->map($callback);
    int $total = $values->reduce(0, function (writable int $sum, int $value): void {
        $sum += $value;
    });
}

function main(): void {}
"#;
        let snapshot = AnalysisSnapshot::analyze("stage30g-hover.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );

        let map_offset = source.find("$values->map").unwrap() + "$values->".len();
        let map = snapshot
            .hover_at_offset(map_offset)
            .expect("map should have a semantic hover");
        assert!(
            map.markdown.contains(
                "List<int>::map(function writable(int): string throws Failure): List<string>"
            ),
            "{}",
            map.markdown
        );
        for expected in [
            "**Callback access:** writable",
            "**Callback invocation:** Writable Repeatable",
            "**Required checked effects:** Failure",
            "**Ambient I/O:**",
            "Doria\\Std\\Io\\IoError",
            "Doria\\Std\\Io\\InvalidUtf8Error",
            "**Source:** Unchanged",
            "**Result:** Owned `List<string>`",
        ] {
            assert!(map.markdown.contains(expected), "{}", map.markdown);
        }
        for internal in ["FunctionTypeId", "plan ID", "descriptor", "__doria"] {
            assert!(!map.markdown.contains(internal), "{}", map.markdown);
        }

        let reduce_offset = source.find("$values->reduce").unwrap() + "$values->".len();
        let reduce = snapshot
            .hover_at_offset(reduce_offset)
            .expect("reduce should have a semantic hover");
        assert!(
            reduce
                .markdown
                .contains("List<int>::reduce(take int, function(writable int, int): void): int"),
            "{}",
            reduce.markdown
        );
        assert!(reduce
            .markdown
            .contains("**Required checked effects:** none"));
        assert!(reduce.markdown.contains("**Ambient I/O:**"));
    }

    #[test]
    fn ambient_effect_hovers_preserve_source_signatures_and_runtime_behavior() {
        let source = r#"function helper(): void
{
    echo "helper";
}

function explicit(): void throws Doria\Std\Io\IoError
{
    echo "explicit";
}

function main(): void
{
    let $callback = function (): void { echo "closure"; };
    $callback( );
    helper();
}
"#;
        let snapshot = AnalysisSnapshot::analyze("ambient-hover.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );

        let helper = snapshot
            .hover_at_offset(source.find("helper").expect("helper declaration"))
            .expect("helper hover");
        assert!(helper.markdown.contains("function helper(): void"));
        assert!(!helper.markdown.contains("function helper(): void throws"));
        assert!(helper.markdown.contains("**Ambient I/O:**"));
        assert!(helper.markdown.contains("Doria\\Std\\Io\\IoError"));

        let explicit = snapshot
            .hover_at_offset(source.find("explicit").expect("explicit declaration"))
            .expect("explicit hover");
        assert!(explicit
            .markdown
            .contains("function explicit(): void throws Doria\\Std\\Io\\IoError"));
        assert!(explicit.markdown.contains("**Ambient I/O:**"));

        let closure_offset = source.find("function (): void").expect("closure");
        let closure = snapshot
            .hover_at_offset(closure_offset)
            .expect("closure hover");
        assert!(closure.markdown.contains("function(): void"));
        assert!(!closure.markdown.contains("function(): void throws"));
        assert!(closure
            .markdown
            .contains("**Required checked effects:** none"));
        assert!(closure.markdown.contains("**Ambient I/O:**"));

        let call_offset = source.find("$callback( );").expect("indirect call") + "$callback(".len();
        let call = snapshot
            .hover_at_offset(call_offset)
            .expect("indirect call hover");
        assert!(call.markdown.contains("function(): void"));
        assert!(!call.markdown.contains("function(): void throws"));
        assert!(call.markdown.contains("**Ambient I/O:**"));
    }

    #[test]
    fn grouped_locals_have_independent_symbols_hovers_completions_and_references() {
        let source = r#"function main(): void
{
    let writable $left, $right = 10;
    int $minimum, $maximum = 5;
    $left = 20;
    echo "{$left}:{$right}:{$minimum}:{$maximum}";
}
"#;
        let snapshot = AnalysisSnapshot::analyze("test.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );

        for name in ["$left", "$right"] {
            let declaration = source.find(name).expect("grouped binding");
            let hover = snapshot
                .hover_at_offset(declaration)
                .expect("each grouped binding should have a hover");
            assert!(hover.markdown.contains(&format!("writable int {name}")));
            assert_eq!(&source[hover.span.start..hover.span.end], name);
        }
        for name in ["$minimum", "$maximum"] {
            let declaration = source.find(name).expect("explicit grouped binding");
            let hover = snapshot
                .hover_at_offset(declaration)
                .expect("each explicit grouped binding should have a hover");
            assert!(hover.markdown.contains(&format!("int {name}")));
        }

        let completion_offset = source.find("echo").expect("completion point");
        let completions = snapshot.local_completions_at_offset(completion_offset);
        assert!(completions.iter().any(|item| item.label == "$left"));
        assert!(completions.iter().any(|item| item.label == "$right"));
        assert!(completions.iter().any(|item| item.label == "$minimum"));
        assert!(completions.iter().any(|item| item.label == "$maximum"));

        let left = snapshot.reference_spans_at_offset(source.find("$left").unwrap(), true);
        let right = snapshot.reference_spans_at_offset(source.find("$right").unwrap(), true);
        assert_eq!(left.len(), 3, "declaration, assignment, and interpolation");
        assert_eq!(right.len(), 2, "declaration and interpolation");
        assert!(left
            .iter()
            .all(|span| &source[span.start..span.end] == "$left"));
        assert!(right
            .iter()
            .all(|span| &source[span.start..span.end] == "$right"));
    }

    #[test]
    fn grouped_local_scope_and_shadowing_keep_symbols_distinct() {
        let source = r#"function main(): void
{
    let $value, $peer = 1;
    {
        let $value, $inner = 2;
        echo "{$value}:{$inner}";
    }
    echo "{$value}:{$peer}";
}
"#;
        let snapshot = AnalysisSnapshot::analyze("test.doria", source);
        assert!(snapshot.diagnostics().is_empty());
        let inner_declaration = source.match_indices("$value").nth(1).unwrap().0;
        let inner = snapshot.reference_spans_at_offset(inner_declaration, true);
        assert_eq!(inner.len(), 2);
        let outer = snapshot.reference_spans_at_offset(source.find("$value").unwrap(), true);
        assert_eq!(outer.len(), 2);
        assert!(inner.iter().all(|span| !outer.contains(span)));
    }

    #[test]
    fn function_and_method_parameters_share_the_scoped_binding_index() {
        let source = r#"class Counter
{
    function add(writable int $amount): int
    {
        $amount += 1;
        return $amount;
    }
}

function greet(string $name): void throws Doria\Std\Io\IoError
{
    echo $name;
}
"#;
        let snapshot = AnalysisSnapshot::analyze("test.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );

        for (name, expected_references, signature) in [
            ("$amount", 3, "writable int $amount"),
            ("$name", 2, "string $name"),
        ] {
            let declaration = source.find(name).expect("parameter declaration");
            let references = snapshot.reference_spans_at_offset(declaration, true);
            assert_eq!(references.len(), expected_references);
            assert_eq!(
                snapshot.reference_spans_at_offset(declaration, false).len(),
                expected_references - 1
            );
            assert!(references
                .iter()
                .all(|span| &source[span.start..span.end] == name));
            assert!(snapshot
                .hover_at_offset(declaration)
                .expect("parameter hover")
                .markdown
                .contains(signature));
            assert_eq!(
                snapshot.rename_replacement_at_offset(declaration, "renamed"),
                Some("$renamed".to_string())
            );
        }

        let method_body = source.find("$amount +=").expect("method body");
        assert!(snapshot
            .local_completions_at_offset(method_body)
            .iter()
            .any(|completion| completion.label == "$amount"));
        let function_body = source.rfind("echo $name").expect("function body");
        assert!(snapshot
            .local_completions_at_offset(function_body)
            .iter()
            .any(|completion| completion.label == "$name"));
    }

    #[test]
    fn instance_completions_exclude_static_members() {
        let source = r#"class Factory
{
    static int $instances = 0;
    int $value = 0;

    static function create(): Factory { return new Factory(); }
    function run(): void {}
}

function main(): void
{
    let $factory = new Factory();
    $factory->run();
}
"#;
        let snapshot = AnalysisSnapshot::analyze("test.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );
        let offset = source.rfind("run").expect("instance method call");
        let labels = snapshot
            .member_completions_at_offset(offset)
            .expect("instance completion context")
            .into_iter()
            .map(|completion| completion.label)
            .collect::<HashSet<_>>();
        assert!(labels.contains("run"));
        assert!(labels.contains("value"));
        assert!(!labels.contains("create"));
        assert!(!labels.contains("instances"));
    }

    #[test]
    fn shared_member_completion_respects_wrapper_family_and_payload_access() {
        let source = r#"class Counter
{
    writable int $value = 0;
    string $name = "counter";

    function inspect(): int { return $this->value; }
    writable function increment(): void { $this->value++; }
    function share(): void {}
}

function main(): void
{
    let $readonly = shared new Counter();
    $readonly->share();

    let $owner = new WritableSharedReference(new Counter());
    $owner->acquireReadonlyAccess();
    let $read = $owner->acquireReadonlyAccess();
    $read->inspect();
    let writable $write = $owner->acquireWritableAccess();
    $write->increment();
}
"#;
        let snapshot = AnalysisSnapshot::analyze("test.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );

        let labels_at = |needle: &str| {
            let offset = source.find(needle).expect("completion needle") + needle.len() - 1;
            snapshot
                .member_completions_at_offset(offset)
                .expect("member completion context")
                .into_iter()
                .map(|completion| completion.label)
                .collect::<Vec<_>>()
        };

        let readonly = labels_at("$readonly->share");
        assert!(readonly.contains(&"share".to_string()));
        assert!(readonly.contains(&"createWeakReference".to_string()));
        assert!(readonly.contains(&"referencedValue".to_string()));
        assert!(readonly.contains(&"inspect".to_string()));
        assert!(!readonly.contains(&"increment".to_string()));
        assert_eq!(readonly.iter().filter(|label| *label == "share").count(), 1);

        let owner = labels_at("$owner->acquireReadonlyAccess");
        assert!(owner.contains(&"acquireWritableAccess".to_string()));
        assert!(!owner.contains(&"inspect".to_string()));
        assert!(!owner.contains(&"referencedValue".to_string()));

        let read = labels_at("$read->inspect");
        assert!(read.contains(&"inspect".to_string()));
        assert!(!read.contains(&"increment".to_string()));
        assert!(read.contains(&"share".to_string()));
        assert!(!read.contains(&"referencedValue".to_string()));

        let write = labels_at("$write->increment");
        assert!(write.contains(&"inspect".to_string()));
        assert!(write.contains(&"increment".to_string()));
        assert!(write.contains(&"share".to_string()));
        assert!(!write.contains(&"referencedValue".to_string()));
    }

    #[test]
    fn referenced_value_hover_uses_the_concrete_payload_type() {
        let source = r#"class Counter
{
    string $referencedValue = "payload";
}

function main(): void
{
    let $counter = shared new Counter();
    echo $counter->referencedValue->referencedValue;
}
"#;
        let snapshot = AnalysisSnapshot::analyze("test.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );
        let offset = source.find("referencedValue").expect("payload declaration");
        let wrapper_offset = source[offset + 1..]
            .find("referencedValue")
            .expect("wrapper projection")
            + offset
            + 1;
        let hover = snapshot
            .hover_at_offset(wrapper_offset)
            .expect("referencedValue hover");
        assert!(hover.markdown.contains("Counter $referencedValue"));
        assert!(hover.markdown.contains("allocation-free"));
        assert!(hover
            .markdown
            .contains("does not change either ownership count"));
    }

    #[test]
    fn checked_error_symbols_and_signatures_share_compiler_semantic_identity() {
        let source = r#"class FirstError implements Error
{
    function __construct(string $message) {}
}

class SecondError implements Error
{
    string $message = "second";
}

class Worker
{
    function __construct(int $id) throws FirstError {}

    function load(int $id, string $path): string throws FirstError, SecondError
    {
        return $path;
    }

    static function open(string $path): string throws SecondError
    {
        return $path;
    }
}

function find(int $id, string $path): string throws FirstError, SecondError
{
    return $path;
}

function fail(take FirstError $failure): void throws FirstError
{
    throw $failure;
}

function inspect(take FirstError $failure): void throws Error
{
    let $worker = new Worker(1);
    find(1, "record");
    find(path: "named", id: 4);
    $worker->load(2, "method");
    Worker::open("static");

    try {
        fail($failure);
    } catch (FirstError $caught) {
        echo $caught->message;
    }
}
"#;
        let snapshot = AnalysisSnapshot::analyze("checked-errors.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );

        let error_contract = snapshot
            .hover_at_offset(
                source
                    .find("implements Error")
                    .expect("Error contract reference")
                    + "implements ".len(),
            )
            .expect("Error contract hover");
        assert!(error_contract.markdown.contains("interface Error"));
        assert!(error_contract.markdown.contains("string $message"));

        let class = snapshot
            .hover_at_offset(source.find("FirstError").expect("Error class declaration"))
            .expect("Error class hover");
        assert!(class.markdown.contains("class FirstError implements Error"));
        assert!(class.markdown.contains("Explicitly conforms"));

        let promoted_message = snapshot
            .hover_at_offset(source.find("$message").expect("promoted message"))
            .expect("promoted message hover");
        assert!(promoted_message.markdown.contains("string $message"));
        assert!(promoted_message.markdown.contains("Error` contract"));

        for (needle, occurrence, expected) in [
            (
                "find",
                0,
                "function find(int $id, string $path): string throws FirstError, SecondError",
            ),
            (
                "load",
                0,
                "function Worker::load(int $id, string $path): string throws FirstError, SecondError",
            ),
            (
                "__construct",
                1,
                "function Worker::__construct(int $id) throws FirstError",
            ),
        ] {
            assert!(hover(source, needle, occurrence)
                .markdown
                .contains(expected));
        }
        let throw_offset = source.find("throw $failure").expect("throw statement");
        assert!(snapshot
            .hover_at_offset(throw_offset)
            .expect("throw statement hover")
            .markdown
            .contains("Transfers ownership"));
        let throws_offset = source.find("throws FirstError").expect("throws keyword");
        let throws_hover = snapshot
            .hover_at_offset(throws_offset)
            .expect("throws keyword hover");
        assert!(throws_hover.markdown.contains("throws checked errors"));
        assert_eq!(
            snapshot.rename_replacement_at_offset(throws_offset, "renamed"),
            None,
            "throws is a keyword, not a callable reference"
        );
        assert!(snapshot
            .semantic_token_spans()
            .iter()
            .any(|(span, token_type, _)| span.start == throws_offset && *token_type == 4));

        let caught = source.find("$caught").expect("catch binding");
        let caught_hover = snapshot
            .hover_at_offset(caught)
            .expect("catch binding hover");
        assert!(caught_hover.markdown.contains("FirstError $caught"));
        assert!(caught_hover.markdown.contains("Readonly owned"));
        assert_eq!(snapshot.reference_spans_at_offset(caught, true).len(), 2);
        assert_eq!(
            snapshot.rename_replacement_at_offset(caught, "handled"),
            Some("$handled".to_string())
        );
        assert!(snapshot
            .semantic_token_spans()
            .iter()
            .any(|(span, token_type, _)| span.start == caught && *token_type == 0));

        let call_expectations = [
            (
                "new Worker(1)",
                "1",
                "function Worker::__construct(int $id) throws FirstError",
                0,
            ),
            (
                "find(1, \"record\")",
                "\"record\"",
                "function find(int $id, string $path): string throws FirstError, SecondError",
                1,
            ),
            (
                "find(path: \"named\", id: 4)",
                "\"named\"",
                "function find(int $id, string $path): string throws FirstError, SecondError",
                1,
            ),
            (
                "find(path: \"named\", id: 4)",
                "4",
                "function find(int $id, string $path): string throws FirstError, SecondError",
                0,
            ),
            (
                "$worker->load(2, \"method\")",
                "\"method\"",
                "function Worker::load(int $id, string $path): string throws FirstError, SecondError",
                1,
            ),
            (
                "Worker::open(\"static\")",
                "\"static\"",
                "static function Worker::open(string $path): string throws SecondError",
                0,
            ),
        ];
        for (call, argument, label, active_parameter) in call_expectations {
            let call_start = source
                .find(call)
                .unwrap_or_else(|| panic!("missing `{call}`"));
            let offset = call_start + call.find(argument).expect("argument in call");
            assert_eq!(
                snapshot.signature_help_at_offset(offset),
                Some(SignatureHelp {
                    label: label.to_string(),
                    active_parameter,
                })
            );
        }

        let omitted = AnalysisSnapshot::analyze(
            "omitted-catch.doria",
            r#"class Failure implements Error { function __construct(string $message) {} }
function fail(): void throws Failure { throw new Failure("x"); }
function handle(): void { try { fail(); } catch (Failure) {} }"#,
        );
        assert!(
            omitted.diagnostics().is_empty(),
            "{:#?}",
            omitted.diagnostics()
        );
        assert!(omitted
            .symbols
            .iter()
            .all(|symbol| symbol.local_name.as_deref() != Some("caught")));

        let leaked = AnalysisSnapshot::analyze(
            "catch-scope.doria",
            r#"class Failure implements Error { function __construct(string $message) {} }
function fail(): void throws Failure { throw new Failure("x"); }
function handle(): void { try { fail(); } catch (Failure $caught) { echo $caught->message; } echo $caught->message; }"#,
        );
        assert!(leaked
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == "E0101"));
    }

    #[test]
    fn inferred_main_effects_do_not_rewrite_source_signature_hover() {
        let source = r#"function main(): void
{
    echo "value";
}
"#;
        let snapshot = AnalysisSnapshot::analyze("inferred-main-effects.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );

        let hover = snapshot
            .hover_at_offset(source.find("main").expect("main declaration"))
            .expect("main declaration hover");
        assert!(hover.markdown.contains("function main(): void"));
        assert!(!hover.markdown.contains("function main(): void throws"));
        assert!(hover.markdown.contains("**Ambient I/O:**"));
    }

    #[test]
    fn capture_metadata_wins_ties_with_function_typed_binding_hovers() {
        let source = r#"let $callback = fn() => 1;
let $wrapper = fn() with ($callback) => $callback();
"#;
        let snapshot = AnalysisSnapshot::analyze("capture-hover.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );

        let declaration_offset = source.find("($callback)").expect("capture declaration") + 1;
        let declaration = snapshot
            .hover_at_offset(declaration_offset)
            .expect("capture declaration hover");
        assert!(declaration
            .markdown
            .contains("Readonly capture of `$callback`"));
        assert!(!declaration
            .markdown
            .contains("Semantically resolved function-typed binding"));

        let use_offset = source.rfind("$callback").expect("captured use");
        let usage = snapshot
            .hover_at_offset(use_offset)
            .expect("captured use hover");
        assert!(usage.markdown.contains("Readonly capture of `$callback`"));
    }

    #[test]
    fn captured_narrowed_functions_combine_flow_identity_and_capture_metadata() {
        let source = r#"function main(): void
{
    mixed $mixed = fn() => 41;
    ?function(): int $nullable = fn() => 1;
    let $wrapper = function (): int with ($mixed, $nullable) {
        let writable $result = 0;
        if ($mixed is function(): int) {
            $result += $mixed();
        }
        if ($nullable != null) {
            $result += $nullable();
        }
        return $result;
    };
    echo "{$wrapper()}";
}
"#;
        let snapshot = AnalysisSnapshot::analyze("captured-narrowing-hover.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );

        let mixed_capture = snapshot
            .hover_at_offset(source.find("($mixed").expect("mixed capture") + 1)
            .expect("mixed capture hover");
        assert!(mixed_capture.markdown.contains("mixed $mixed"));
        assert!(mixed_capture
            .markdown
            .contains("Readonly capture of `$mixed`"));

        let mixed_use = snapshot
            .hover_at_offset(source.find("$mixed();").expect("narrowed mixed use"))
            .expect("narrowed mixed capture hover");
        assert!(mixed_use.markdown.contains("function(): int $mixed"));
        assert!(mixed_use.markdown.contains("Readonly capture of `$mixed`"));
        assert!(mixed_use
            .markdown
            .contains("Compiler-resolved function value after flow narrowing"));
        assert!(mixed_use.markdown.contains("Execution capability"));
        assert!(!mixed_use.markdown.contains("mixed $mixed"));

        let nullable_capture = snapshot
            .hover_at_offset(
                source
                    .find("$nullable) {")
                    .expect("nullable capture declaration"),
            )
            .expect("nullable capture hover");
        assert!(nullable_capture
            .markdown
            .contains("?function(): int $nullable"));

        let nullable_use = snapshot
            .hover_at_offset(source.find("$nullable();").expect("narrowed nullable use"))
            .expect("narrowed nullable capture hover");
        assert!(nullable_use.markdown.contains("function(): int $nullable"));
        assert!(nullable_use
            .markdown
            .contains("Readonly capture of `$nullable`"));
        assert!(nullable_use
            .markdown
            .contains("Compiler-resolved function value after flow narrowing"));
        assert!(!nullable_use.markdown.contains("?function(): int $nullable"));
    }

    #[test]
    fn closure_hovers_present_compiler_owned_ownership_without_internal_identity() {
        let source = r#"class Payload {}
function main(): void
{
    let $read = 1;
    let writable $write = 2;
    let $copy = "copy";
    let writable $operation = function (): void with ($read, writable $write, take $copy) {
        echo "{$read}{$copy}";
        $write += 1;
    };
    $operation();

    let $payload = new Payload();
    let $once = function (): Payload with (take $payload) { return $payload; };
}
"#;
        let snapshot = AnalysisSnapshot::analyze("ownership-hover.doria", source);
        let operation = snapshot
            .hover_at_offset(
                source
                    .find("function (): void")
                    .expect("repeatable closure"),
            )
            .expect("repeatable closure hover");
        for expected in [
            "Borrow-Bound Closure",
            "Readonly capture of `$read`",
            "Writable capture of `$write`",
            "Owned taking capture of `$copy`",
            "Writable Repeatable",
            "Nonescaping",
            "Executable In Debug And Native Targets",
            "Explicit closure lowering is available when the program's value families and operations are supported by the PHP backend",
        ] {
            assert!(
                operation.markdown.contains(expected),
                "missing `{expected}` from {}",
                operation.markdown
            );
        }

        let once = snapshot
            .hover_at_offset(source.rfind("function (): Payload").expect("once closure"))
            .expect("once closure hover");
        assert!(once.markdown.contains("Owned Closure"));
        assert!(once.markdown.contains("Owned taking capture of `$payload`"));
        assert!(once.markdown.contains("Consumes On Invocation"));

        for forbidden in [
            "BindingId",
            "ClosureId",
            "OwnershipSlotId",
            "environment offset",
            "descriptor address",
            "CaptureAcquisitionKind",
        ] {
            assert!(!operation.markdown.contains(forbidden));
            assert!(!once.markdown.contains(forbidden));
        }
    }

    #[test]
    fn closure_hovers_distinguish_callback_and_return_escape_contracts() {
        let source = r#"function keep(
    function(): int $borrowed,
    take function(): int $owned,
    ?function(): int $nullableBorrowed,
    take ?function(): int $nullableOwned
): void {}
function bind(int $value): function(): int
{
    return fn() with ($value) => $value;
}
class Box
{
    int $value = 1;
    function bind(): function(): int
    {
        return fn() with ($this) => $this->value;
    }
}
function main(): void
{
    List<function(): int> $callbacks = [fn() => 1];
}
"#;
        let snapshot = AnalysisSnapshot::analyze("escape-hover.doria", source);

        let borrowed_parameter = snapshot
            .hover_at_offset(
                source
                    .find("$borrowed")
                    .expect("borrowed callback parameter"),
            )
            .expect("borrowed callback parameter hover");
        assert!(borrowed_parameter
            .markdown
            .contains("Nonescaping Callback Parameter"));
        let owned_parameter = snapshot
            .hover_at_offset(source.find("$owned").expect("owned callback parameter"))
            .expect("owned callback parameter hover");
        assert!(owned_parameter
            .markdown
            .contains("Owned Callback Parameter"));
        let nullable_borrowed = snapshot
            .hover_at_offset(
                source
                    .find("$nullableBorrowed")
                    .expect("nullable borrowed callback parameter"),
            )
            .expect("nullable borrowed callback parameter hover");
        assert!(nullable_borrowed
            .markdown
            .contains("?function(): int $nullableBorrowed"));
        assert!(nullable_borrowed
            .markdown
            .contains("Nonescaping Callback Parameter"));
        let nullable_owned = snapshot
            .hover_at_offset(
                source
                    .find("$nullableOwned")
                    .expect("nullable owned callback parameter"),
            )
            .expect("nullable owned callback parameter hover");
        assert!(nullable_owned
            .markdown
            .contains("?function(): int $nullableOwned"));
        assert!(nullable_owned.markdown.contains("Owned Callback Parameter"));

        let parameter_return = snapshot
            .hover_at_offset(
                source
                    .find("fn() with ($value)")
                    .expect("parameter closure"),
            )
            .expect("parameter-rooted closure hover");
        assert!(parameter_return
            .markdown
            .contains("Returned closure tied to $value"));

        let receiver_return = snapshot
            .hover_at_offset(source.find("fn() with ($this)").expect("receiver closure"))
            .expect("receiver-rooted closure hover");
        assert!(receiver_return
            .markdown
            .contains("Returned closure tied to $this"));

        let owned_callback = snapshot
            .hover_at_offset(source.rfind("fn() => 1").expect("stored callback"))
            .expect("stored callback hover");
        assert!(owned_callback.markdown.contains("Owned callback"));
    }

    #[test]
    fn signature_help_recovers_only_advertised_incomplete_call_triggers() {
        for (call, active_parameter) in [("lookup(", 0), ("lookup(1,", 1)] {
            let source = format!(
                "function lookup(int $id, string $name): void {{}}\nfunction main(): void {{\n    {call}\n}}"
            );
            let offset = source.rfind(call).expect("incomplete call") + call.len();
            assert_eq!(
                AnalysisSnapshot::signature_help_for_incomplete_call(
                    "incomplete-call.doria",
                    &source,
                    offset,
                ),
                Some(SignatureHelp {
                    label: "function lookup(int $id, string $name): void".to_string(),
                    active_parameter,
                })
            );
        }

        let source = "function main(): void { echo \"not a call\"; }";
        assert_eq!(
            AnalysisSnapshot::signature_help_for_incomplete_call(
                "complete.doria",
                source,
                source.len(),
            ),
            None
        );
    }
}
