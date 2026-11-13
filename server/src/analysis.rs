use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use doriac::ast::{
    Block, ClassDecl, ClassMember, ConstructorParameterRole, ControlFlowFinally, DoWhileStmt,
    ElseBranch, EnumDecl, Expr, ForIncrement, ForInitializer, FunctionDecl, GivenPrelude, IfStmt,
    Item, MatchMode, MatchOrigin, MatchPattern, MemberAccess, NominalCondition, Param, Program,
    StaticQualifier, Stmt, TryStmt, VarDecl, WhenExpression, WhileStmt,
};
use doriac::attributes::{
    AttributeApplication, AttributeClassIdentity, AttributeClassSchema, AttributeSemanticInfo,
    AttributeTarget, AttributeValue, AttributeValueKind,
};
use doriac::byte_intrinsics::{self, ByteIntrinsic, ByteMemberKind};
use doriac::diagnostics::{Diagnostic, DiagnosticSeverity, LabelRole};
use doriac::enums::{EnumBackingType, EnumBackingValue};
use doriac::lexer::{Token, TokenKind};
use doriac::names::{
    CompilationContext, CompilerSymbolIdentity, GlobalReferenceRole, GlobalSymbolFacts,
    GlobalSymbolId, GlobalSymbolKind, GlobalSymbolOwner, SourceIdentity,
};
use doriac::ownership::{
    BorrowRoot, CaptureAcquisitionKind, ClosureEscapeClassification, InvocationConsumption,
    ValueProvenance,
};
use doriac::semantics::composition::{
    ClassMemberAvailability, ClassMemberSurface, ClassMemberSurfaceEntry, MemberAvailability,
};
use doriac::semantics::contracts::{
    ConformanceStatus, ContractFacts, InterfaceSpecializationFacts, RequirementFacts,
};
use doriac::semantics::{
    CallableSignatureSemanticInfo, CallableTarget, ConstructorParameterSemanticRole,
    EnumCaseDeclarationSemanticInfo, EnumDeclarationSemanticInfo, ForeachIterationKind,
    ForeachValueAccess, ListAlgorithmCallInfo, ListAlgorithmKind, ListCallbackAccess,
    ObjectPathAccess, SemanticInfo,
};
use doriac::source::{SourceFile, SourceId, Span};
use doriac::symbols::{
    BindingKind, BindingOwnership, MemberKind as CompilerMemberKind, ReceiverMode,
};
use doriac::testing::{SourceSemanticContext, TestSemanticFacts};
use doriac::types::{
    ClassType, FunctionInvocationMode, ResolvedType, SharedHandleKind, TypeRef, TypeRegistry,
};

use crate::string_surface::{string_companion_method, string_property, STRING_PROPERTIES};

mod property_hooks;
use doriac::property_hooks::{member_callables, PropertyHookContext};
use property_hooks::PropertyAccessorNavigation;

const SEMANTIC_TOKEN_FUNCTION: u32 = 3;
const SEMANTIC_TOKEN_TYPE: u32 = 1;
const SEMANTIC_TOKEN_PROPERTY: u32 = 2;
const SEMANTIC_TOKEN_STRING: u32 = 6;
const SEMANTIC_TOKEN_DECLARATION: u32 = 1;
const SEMANTIC_TOKEN_KEYWORD: u32 = 4;

pub(crate) type SemanticTokenSpan = (Span, u32, u32);

trait AstTypeName {
    fn ast_type_name(&self) -> &str;
}

impl AstTypeName for String {
    fn ast_type_name(&self) -> &str {
        self
    }
}

impl AstTypeName for TypeRef {
    fn ast_type_name(&self) -> &str {
        &self.name
    }
}

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
                    tags: template_tags(&trait_decl.type_params),
                });
                member_documentation_targets(&trait_decl.members, &mut targets);
            }
            Item::Enum(enum_decl) => targets.push(DocumentationTarget {
                start: enum_decl.span.start,
                tags: template_tags(&enum_decl.type_params),
            }),
            Item::Interface(interface) => {
                targets.push(DocumentationTarget {
                    start: interface.span.start,
                    tags: template_tags(&interface.type_params),
                });
                targets.extend(
                    interface
                        .requirements
                        .iter()
                        .map(function_documentation_target),
                );
            }
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
            ClassMember::Uses(_) => {}
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
    if parameter.borrow_span.is_some() {
        parts.push("borrow".to_string());
    }
    match parameter.constructor_role {
        ConstructorParameterRole::Promoted {
            access: MemberAccess::Internal,
            ..
        } => parts.push("internal".to_string()),
        ConstructorParameterRole::InheritedPropertyOverride { .. } => {
            parts.push("override".to_string());
        }
        ConstructorParameterRole::ConstructorOnly { .. } => {
            parts.push("parameter".to_string());
        }
        ConstructorParameterRole::Ordinary
        | ConstructorParameterRole::Promoted {
            access: MemberAccess::External,
            ..
        } => {}
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
    has_standalone_entry: bool,
    source_id: SourceId,
    compilation_context: CompilationContext,
    global_symbols: GlobalSymbolFacts,
    contracts: doriac::semantics::contracts::ContractFacts,
    composition_origins: Vec<doriac::trait_composition::EffectiveMemberOrigin>,
    composition_obligations: Vec<doriac::trait_composition::MethodObligation>,
    class_member_surfaces: Vec<ClassMemberSurface>,
    class_member_availability: HashMap<Span, ClassMemberAvailability>,
    property_backing_fields: HashMap<Span, doriac::property_hooks::PropertyBackingField>,
    trait_adaptations: Vec<doriac::ast::TraitAdaptation>,
    trait_adaptation_surfaces: Vec<doriac::semantics::composition::TraitAdaptationSurface>,
    composition_rename: doriac::semantics::composition_rename::CompositionRenameFacts,
    recovered_composition: Option<ComposedSourcePresentation>,
    directive_semantic_tokens: Vec<(Span, u32)>,
    attribute_semantic_tokens: Vec<(Span, u32)>,
    assertion_semantic_tokens: Vec<SemanticTokenSpan>,
    foreach_semantic_tokens: Vec<SemanticTokenSpan>,
    hierarchy_semantic_tokens: Vec<SemanticTokenSpan>,
    pub(crate) property_accessors: PropertyAccessorNavigation,
    assertion_completions: Vec<(Span, doriac::semantics::AssertionCompletionInfo)>,
    test_semantics: TestSemanticFacts,
    source_semantic_context: Option<SourceSemanticContext>,
    attribute_info: AttributeSemanticInfo,
    attribute_parameter_occurrences: Vec<AttributeParameterOccurrence>,
    symbols: Vec<Symbol>,
    occurrences: Vec<Occurrence>,
    /// Class-name span in a `new` expression, and the constructor it calls.
    constructor_references: Vec<(Span, usize)>,
    parameter_presentation: ParameterPresentation,
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
    hierarchy_classes: Vec<HierarchyClass>,
    hierarchy_members: Vec<HierarchyMember>,
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
    pub(crate) exact_declaration: Option<Span>,
    pub(crate) virtual_root: Option<Span>,
    pub(crate) direct_parent: bool,
    pub(crate) relationship_only: bool,
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SemanticCompletion {
    pub(crate) label: String,
    pub(crate) kind: u32,
    pub(crate) detail: String,
    pub(crate) documentation: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct HierarchyClass {
    pub(crate) identity: GlobalSymbolId,
    pub(crate) source_name: String,
    pub(crate) qualified_name: String,
    pub(crate) type_parameters: Vec<String>,
    pub(crate) is_open: bool,
    pub(crate) access: MemberAccess,
    pub(crate) ancestors: Vec<String>,
    pub(crate) body_span: Span,
}

#[derive(Debug, Clone)]
pub(crate) struct HierarchyMember {
    pub(crate) owner: GlobalSymbolId,
    pub(crate) name: String,
    pub(crate) kind: MemberKind,
    pub(crate) declaration: Span,
    pub(crate) detail: String,
    pub(crate) documentation: Option<String>,
    pub(crate) access: MemberAccess,
    pub(crate) is_static: bool,
    pub(crate) writable_receiver: bool,
    pub(crate) is_open: bool,
    pub(crate) is_override: bool,
    pub(crate) virtual_root: Option<Span>,
    pub(crate) overridden_declaration: Option<Span>,
    pub(crate) override_stub: Option<String>,
    pub(crate) body_span: Option<Span>,
}

#[derive(Debug, Clone)]
pub(crate) struct HierarchyContext {
    pub(crate) class: GlobalSymbolId,
    pub(crate) method: Option<HierarchyMethodContext>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HierarchyMethodContext {
    pub(crate) is_static: bool,
    pub(crate) constructor: bool,
}

#[derive(Debug, Clone)]
struct Symbol {
    signature: String,
    documentation: Option<String>,
    local_name: Option<String>,
    parameter_names: Vec<String>,
    parameter_symbols: Vec<Option<usize>>,
    kind: SymbolKind,
}

#[derive(Debug, Clone, Copy)]
struct PendingNamedArgumentReference {
    callable: usize,
    parameter: usize,
    span: Span,
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
    Type,
    TypeParameter,
    Variable,
    Keyword,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OccurrenceRole {
    Declaration,
    Reference,
    NamedArgument,
}

#[derive(Debug, Clone)]
struct MemberReceiver {
    span: Span,
    receiver_span: Span,
    receiver: ResolvedType,
    current_class: Option<String>,
    writable_payload_access: bool,
}

#[derive(Debug, Clone)]
struct StaticReceiver {
    span: Span,
    receiver_span: Span,
    receiver: StaticReceiverKind,
    current_class: Option<String>,
}

#[derive(Debug, Clone)]
enum StaticReceiverKind {
    Byte(ResolvedType),
    Enum(String),
    Class {
        receiver: ClassType<ResolvedType>,
        parent: bool,
    },
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
    signature: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct CallArgumentContext {
    span: Span,
    parameter: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SignatureHelp {
    pub(crate) label: String,
    pub(crate) active_parameter: usize,
}

// These are source-facing values, after graph-local expansion identity has been
// resolved. Keeping only this projection avoids retaining whole compiler snapshots
// for dependencies shared by several workspace members.
#[derive(Debug, Clone, Default)]
pub(crate) struct ComposedSourcePresentation {
    hovers: Vec<SemanticHover>,
    completions: Vec<(Span, Vec<SemanticCompletion>)>,
    calls: Vec<PresentedCall>,
}

#[derive(Debug, Clone)]
struct PresentedCall {
    span: Span,
    arguments: Vec<CallArgumentContext>,
    label: String,
    parameter_names: Vec<String>,
}

impl PresentedCall {
    fn signature_help(&self, offset: usize) -> SignatureHelp {
        SignatureHelp {
            label: self.label.clone(),
            active_parameter: self
                .arguments
                .iter()
                .find(|argument| offset <= argument.span.end)
                .map(|argument| argument.parameter)
                .unwrap_or(self.arguments.len()),
        }
    }

    fn named_arguments(&self, offset: usize) -> Vec<SemanticCompletion> {
        let supplied = self
            .arguments
            .iter()
            .filter(|argument| argument.span.start < offset)
            .map(|argument| argument.parameter)
            .collect::<HashSet<_>>();
        self.parameter_names
            .iter()
            .enumerate()
            .filter(|(index, _)| !supplied.contains(index))
            .map(|(_, name)| SemanticCompletion {
                label: format!("{name}:"),
                kind: 5,
                detail: format!("Named argument for {}", self.label),
                documentation: None,
            })
            .collect()
    }
}

impl ComposedSourcePresentation {
    pub(crate) fn add(&mut self, snapshot: &AnalysisSnapshot) {
        let contextual = |span: Span| span != span.authored();
        // SourceId and ExpansionId must not survive this presentation boundary.
        let display_span = |span: Span| Span::new(span.start, span.end);
        for hover in snapshot
            .semantic_hovers
            .iter()
            .filter(|hover| contextual(hover.span))
        {
            let Some(origin) = snapshot
                .composition_origins
                .iter()
                .find(|origin| origin.id == hover.span.expansion)
            else {
                continue;
            };
            self.hovers.push(SemanticHover {
                span: display_span(hover.span),
                markdown: format!(
                    "**Composed in `{}::{}` via `{}`**\n\n{}",
                    origin.composing_class, origin.name, origin.trait_type, hover.markdown
                ),
                priority: hover.priority,
            });
        }
        for context in snapshot
            .member_receivers
            .iter()
            .filter(|context| contextual(context.span))
        {
            self.completions.push((
                display_span(context.span),
                snapshot.member_completions(context),
            ));
        }
        for context in snapshot
            .static_receivers
            .iter()
            .filter(|context| contextual(context.span))
        {
            self.completions.push((
                display_span(context.span),
                snapshot.static_receiver_completions(context, None),
            ));
        }
        for context in snapshot
            .call_signatures
            .iter()
            .filter(|context| contextual(context.span))
        {
            if let Some(mut call) = snapshot.present_call(context) {
                call.span = display_span(call.span);
                for argument in &mut call.arguments {
                    argument.span = display_span(argument.span);
                }
                self.calls.push(call);
            }
        }
        for adaptation in &snapshot.trait_adaptations {
            if let Some(completions) =
                snapshot.trait_adaptation_completions_at_offset(adaptation.method.span.start)
            {
                self.completions.push((
                    Span::new(adaptation.separator_span.end, adaptation.method.span.end),
                    completions,
                ));
            }
        }
    }

    pub(crate) fn hover(&self, offset: usize) -> Option<SemanticHover> {
        let selected = self
            .hovers
            .iter()
            .filter(|hover| span_contains(hover.span, offset))
            .min_by_key(|hover| hover.selection_key())?;
        let mut sections = self
            .hovers
            .iter()
            .filter(|hover| hover.span == selected.span && hover.priority == selected.priority)
            .map(|hover| hover.markdown.clone())
            .collect::<Vec<_>>();
        sections.sort();
        sections.dedup();
        Some(SemanticHover {
            markdown: sections.join("\n\n---\n\n"),
            ..selected.clone()
        })
    }

    pub(crate) fn completions(&self, offset: usize) -> Option<Vec<SemanticCompletion>> {
        let span = self
            .completions
            .iter()
            .map(|(span, _)| *span)
            .filter(|span| span_contains(*span, offset))
            .min_by_key(|span| span.end - span.start)?;
        let mut items = self
            .completions
            .iter()
            .filter(|(candidate, _)| *candidate == span)
            .flat_map(|(_, items)| items.iter().cloned())
            .collect::<Vec<_>>();
        items.sort();
        items.dedup();
        Some(items)
    }

    fn calls_at(&self, offset: usize) -> Option<Vec<&PresentedCall>> {
        let span = self
            .calls
            .iter()
            .map(|call| call.span)
            .filter(|span| span_contains(*span, offset))
            .min_by_key(|span| span.end - span.start)?;
        Some(self.calls.iter().filter(|call| call.span == span).collect())
    }

    pub(crate) fn signatures(&self, offset: usize) -> Option<Vec<SignatureHelp>> {
        let mut items = self
            .calls_at(offset)?
            .iter()
            .map(|call| call.signature_help(offset))
            .collect::<Vec<_>>();
        items.sort();
        items.dedup();
        Some(items)
    }

    pub(crate) fn named_arguments(&self, offset: usize) -> Option<Vec<SemanticCompletion>> {
        let mut items = self
            .calls_at(offset)?
            .iter()
            .flat_map(|call| call.named_arguments(offset))
            .collect::<Vec<_>>();
        items.sort();
        items.dedup();
        Some(items)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalCompletion {
    pub(crate) label: String,
    pub(crate) detail: String,
}

impl AnalysisSnapshot {
    pub(crate) fn merge_recovered_compositions(snapshots: Vec<Self>) -> Option<Self> {
        let mut presentation = ComposedSourcePresentation::default();
        for snapshot in &snapshots {
            presentation.add(snapshot);
        }
        let mut primary = snapshots.into_iter().next()?;
        // Semantic identities stay in one graph. Only source-facing values from
        // other package views are retained during temporary editor recovery.
        primary.recovered_composition = Some(presentation);
        Some(primary)
    }

    pub(crate) fn analyze(path: &str, text: &str) -> Self {
        Self::analyze_with_context(path, text, CompilationContext::standalone(path))
    }

    pub(crate) fn analyze_with_context(
        path: &str,
        text: &str,
        context: CompilationContext,
    ) -> Self {
        Self::analyze_with_source_context(path, text, SourceSemanticContext::standalone(context))
    }

    pub(crate) fn analyze_with_source_context(
        path: &str,
        text: &str,
        source_context: SourceSemanticContext,
    ) -> Self {
        let context = source_context.compilation.clone();
        let tokens = doriac::lex_source(path.to_string(), text.to_string()).unwrap_or_default();
        let (program, analysis) = match doriac::analyze_source_for_ide_with_source_context(
            path.to_string(),
            text.to_string(),
            source_context.clone(),
        ) {
            Ok(analysis) => analysis,
            Err(diagnostics) => {
                return Self {
                    diagnostics,
                    compilation_context: context,
                    source_semantic_context: Some(source_context),
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

    pub(crate) fn is_self_contained_program(&self) -> bool {
        self.has_standalone_entry
            && self.global_symbols.unresolved.is_empty()
            && self.global_symbols.references.iter().all(|reference| {
                matches!(
                    reference.symbol_id.owner,
                    GlobalSymbolOwner::CompilerKnown(_)
                ) || self
                    .global_symbols
                    .declarations
                    .iter()
                    .any(|declaration| declaration.id == reference.symbol_id)
            })
    }

    pub(crate) fn extend_diagnostics(&mut self, diagnostics: Vec<Diagnostic>) {
        self.diagnostics.extend(diagnostics);
    }

    pub(crate) fn compilation_context(&self) -> &CompilationContext {
        &self.compilation_context
    }

    pub(crate) fn source_id(&self) -> SourceId {
        self.source_id
    }

    pub(crate) fn contracts(&self) -> &doriac::semantics::contracts::ContractFacts {
        &self.contracts
    }

    pub(crate) fn composition_origins(
        &self,
    ) -> &[doriac::trait_composition::EffectiveMemberOrigin] {
        &self.composition_origins
    }

    pub(crate) fn composition_rename(
        &self,
    ) -> &doriac::semantics::composition_rename::CompositionRenameFacts {
        &self.composition_rename
    }

    pub(crate) fn composition_obligations(&self) -> &[doriac::trait_composition::MethodObligation] {
        &self.composition_obligations
    }

    pub(crate) fn global_symbols(&self) -> &GlobalSymbolFacts {
        &self.global_symbols
    }

    pub(crate) fn test_semantics(&self) -> &TestSemanticFacts {
        &self.test_semantics
    }

    pub(crate) fn source_semantic_context(&self) -> Option<&SourceSemanticContext> {
        self.source_semantic_context.as_ref()
    }

    pub(crate) fn member_occurrences(&self) -> &[MemberOccurrence] {
        &self.member_occurrences
    }

    pub(crate) fn member_parents(&self) -> &[MemberParent] {
        &self.member_parents
    }

    pub(crate) fn hierarchy_classes(&self) -> &[HierarchyClass] {
        &self.hierarchy_classes
    }

    pub(crate) fn hierarchy_members(&self) -> &[HierarchyMember] {
        &self.hierarchy_members
    }

    pub(crate) fn hierarchy_context_at_offset(&self, offset: usize) -> Option<HierarchyContext> {
        let class = self
            .hierarchy_classes
            .iter()
            .filter(|class| span_contains(class.body_span, offset))
            .min_by_key(|class| class.body_span.end.saturating_sub(class.body_span.start))?;
        let method = self
            .hierarchy_members
            .iter()
            .filter(|member| member.owner == class.identity)
            .filter_map(|member| {
                member
                    .body_span
                    .filter(|span| span_contains(*span, offset))
                    .map(|_| HierarchyMethodContext {
                        is_static: member.is_static,
                        constructor: member.name == "__construct",
                    })
            })
            .next();
        Some(HierarchyContext {
            class: class.identity.clone(),
            method,
        })
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

    #[cfg(test)]
    pub(crate) fn signature_help_for_incomplete_call(
        path: &str,
        text: &str,
        offset: usize,
    ) -> Option<Vec<SignatureHelp>> {
        Self::analyze(
            path,
            &Self::recover_incomplete_call_source(path, text, offset)?,
        )
        .signature_help_at_offset(offset)
    }

    pub(crate) fn recover_incomplete_call_source(
        path: &str,
        text: &str,
        offset: usize,
    ) -> Option<String> {
        let prefix = text.get(..offset)?;
        let trimmed = prefix.trim_end();
        let partial_label = if matches!(trimmed.chars().last(), Some('(' | ',')) {
            None
        } else {
            if trimmed.len() != prefix.len() {
                return None;
            }
            let partial_label = trimmed
                .rsplit_once(['(', ','])
                .map(|(_, segment)| segment.trim())
                .filter(|segment| {
                    !segment.is_empty()
                        && segment
                            .chars()
                            .all(|character| character == '_' || character.is_ascii_alphanumeric())
                })?;
            debug_assert!(!partial_label.is_empty());
            Some((trimmed.len() - partial_label.len(), partial_label.len()))
        };

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
        let mut insertion = if partial_label.is_some() {
            String::new()
        } else {
            "0".to_string()
        };
        insertion.push_str(&")".repeat(unmatched_parens.saturating_sub(existing_closers)));
        if !trimmed_suffix.starts_with(')') && !trimmed_suffix.starts_with(';') {
            insertion.push(';');
        }

        let mut recovered = text.to_string();
        if let Some((start, length)) = partial_label {
            recovered.replace_range(start..start + length, &" ".repeat(length));
        }
        recovered.insert_str(offset, &insertion);
        Some(recovered)
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
                if let Some(constructor) = self.constructor_section(occurrence.span) {
                    markdown.push_str("\n\n---\n\n");
                    markdown.push_str(&constructor);
                }
                Some(SemanticHover::new(occurrence.span, markdown))
            });
        let semantic_hover = self.semantic_hover_at_offset(offset);

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

    /// The constructor a `new` expression calls, for the hover on its class
    /// name: the same signature and documentation the constructor's own hover
    /// shows.
    fn constructor_section(&self, class_name_span: Span) -> Option<String> {
        let (_, constructor) = self
            .constructor_references
            .iter()
            .find(|(span, _)| *span == class_name_span)?;
        let constructor = self.symbols.get(*constructor)?;
        let mut markdown = format!("```doria\n{}\n```", constructor.signature);
        if let Some(documentation) = &constructor.documentation {
            markdown.push_str("\n\n");
            markdown.push_str(documentation);
        }
        Some(markdown)
    }

    pub(crate) fn semantic_hover_at_offset(&self, offset: usize) -> Option<SemanticHover> {
        let selected = self
            .semantic_hovers
            .iter()
            .filter(|hover| span_contains(hover.span, offset))
            .min_by_key(|hover| hover.selection_key())?;
        let mut contextual = false;
        let mut sections = self
            .semantic_hovers
            .iter()
            .filter(|hover| {
                hover.span.authored() == selected.span.authored()
                    && hover.priority == selected.priority
            })
            .filter_map(|hover| {
                if hover.span == hover.span.authored() {
                    return Some(hover.markdown.clone());
                }
                contextual = true;
                let origin = self
                    .composition_origins
                    .iter()
                    .find(|origin| origin.id == hover.span.expansion)?;
                Some(format!(
                    "**Composed in `{}::{}` via `{}`**\n\n{}",
                    origin.composing_class, origin.name, origin.trait_type, hover.markdown
                ))
            })
            .collect::<Vec<_>>();
        if !contextual {
            return Some(selected.clone());
        }
        sections.sort();
        sections.dedup();
        (!sections.is_empty()).then(|| SemanticHover {
            span: selected.span.authored(),
            markdown: sections.join("\n\n---\n\n"),
            priority: selected.priority,
        })
    }

    pub(crate) fn member_completions_at_offset(
        &self,
        offset: usize,
    ) -> Option<Vec<SemanticCompletion>> {
        if let Some(items) = self
            .recovered_composition
            .as_ref()
            .and_then(|view| view.completions(offset))
        {
            return Some(items);
        }
        let context = self
            .member_receivers
            .iter()
            .filter(|context| context.span.start <= offset && offset <= context.span.end)
            .min_by_key(|context| context.span.end.saturating_sub(context.span.start))?;
        let mut completions = self
            .member_receivers
            .iter()
            .filter(|candidate| candidate.span.authored() == context.span.authored())
            .flat_map(|candidate| self.member_completions(candidate))
            .collect::<Vec<_>>();
        completions.sort();
        completions.dedup();
        Some(completions)
    }

    pub(crate) fn assertion_completions_at_offset(
        &self,
        offset: usize,
    ) -> Option<Vec<SemanticCompletion>> {
        let (_, completion) = self
            .assertion_completions
            .iter()
            .filter(|(span, _)| {
                span.source == self.source_id
                    && span.end <= offset
                    && offset.saturating_sub(span.end) <= 2
            })
            .max_by_key(|(span, _)| span.end)?;
        let mut items = completion
            .matchers
            .iter()
            .copied()
            .map(|matcher| assertion_matcher_completion(matcher, &completion.actual_type))
            .collect::<Vec<_>>();
        if !completion.negated {
            items.push(SemanticCompletion {
                label: "not".to_string(),
                kind: 10,
                detail: "Negated compiler-owned expectation".to_string(),
                documentation: Some(
                    "Property-shaped negation. It evaluates no value and may appear only once before a terminal matcher."
                        .to_string(),
                ),
            });
        }
        items.sort_by(|left, right| left.label.cmp(&right.label));
        items.dedup_by(|left, right| left.label == right.label);
        Some(items)
    }

    pub(crate) fn compiler_test_import_completions_at_offset(
        &self,
        text: &str,
        offset: usize,
    ) -> Option<Vec<SemanticCompletion>> {
        if !self
            .source_semantic_context()
            .is_some_and(SourceSemanticContext::is_development)
        {
            return None;
        }
        let prefix = doriac::compiler_known_test::import_completion_prefix(text, offset)?;
        Some(
            doriac::compiler_known_test::IMPLEMENTED_MEMBERS
                .iter()
                .filter_map(|canonical| {
                    let label = canonical.rsplit('\\').next()?.to_string();
                    if !label.starts_with(&prefix) {
                        return None;
                    }
                    let kind = if *canonical == doriac::compiler_known_test::ASSERTION_ERROR {
                        7
                    } else {
                        3
                    };
                    Some(SemanticCompletion {
                        label,
                        kind,
                        detail: format!("Compiler-owned `{canonical}`"),
                        documentation: compiler_test_member_hover(canonical).map(str::to_string),
                    })
                })
                .collect(),
        )
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
            return self.class_member_completions(class, context.writable_payload_access, context);
        }
        if let Some(completions) = interface_member_completions(
            &self.contracts,
            receiver,
            context.receiver_span,
            context.writable_payload_access,
        ) {
            return completions;
        }
        if let Some(completions) = list_algorithm_completions(receiver) {
            return completions;
        }
        if matches!(receiver, ResolvedType::String) {
            return string_property_completions();
        }
        if matches!(receiver, ResolvedType::Bytes | ResolvedType::ByteRegion(_)) {
            return byte_completions(receiver, context.writable_payload_access, false);
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
        let writable = context.writable_payload_access;
        if let ResolvedType::Class(class) = payload.as_ref() {
            completions.extend(self.class_member_completions(class, writable, context));
        } else if let Some(members) =
            interface_member_completions(&self.contracts, payload, context.receiver_span, writable)
        {
            completions.extend(members);
        } else if kind.is_access() {
            if matches!(payload.as_ref(), ResolvedType::String) {
                completions.extend(string_property_completions());
            } else {
                completions.extend(byte_completions(payload, writable, false));
            }
        }
        let mut labels = HashSet::new();
        completions.retain(|completion| labels.insert(completion.label.clone()));
        completions
    }

    fn trait_adaptation_completions_at_offset(
        &self,
        offset: usize,
    ) -> Option<Vec<SemanticCompletion>> {
        let adaptation = self.trait_adaptations.iter().find(|adaptation| {
            adaptation.separator_span.end <= offset && offset <= adaptation.method.span.end
        })?;
        let mut completions = self
            .trait_adaptation_surfaces
            .iter()
            .filter(|candidate| {
                candidate
                    .applicable_origins
                    .contains(&adaptation.origin_span)
            })
            .map(|candidate| {
                let prefix = format!(
                    "{}{}{}",
                    if candidate.access == MemberAccess::Internal {
                        "internal "
                    } else {
                        ""
                    },
                    if candidate.is_static { "static " } else { "" },
                    match candidate.receiver_mode {
                        Some(ReceiverMode::Writable) => "writable ",
                        Some(ReceiverMode::Consuming) => "consuming ",
                        Some(ReceiverMode::Readonly) | None => "",
                    }
                );
                let signature = semantic_callable_signature(
                    &format!("{}::{}", candidate.trait_type, candidate.name),
                    &candidate.generic_parameters,
                    &candidate.signature,
                    &candidate.checked_effects,
                    &HashMap::new(),
                    &self.parameter_presentation,
                );
                let mut documentation = callable_details_documentation(
                    Some(&candidate.signature),
                    candidate.return_borrow,
                    &candidate.automatic_effects,
                );
                append_documentation(
                    &mut documentation,
                    &format!("Composed in `{}`.", candidate.composing_class),
                );
                SemanticCompletion {
                    label: candidate.name.clone(),
                    kind: 2,
                    detail: format!("{prefix}{signature}"),
                    documentation,
                }
            })
            .collect::<Vec<_>>();
        completions.sort_by(|left, right| {
            (&left.label, &left.detail, &left.documentation).cmp(&(
                &right.label,
                &right.detail,
                &right.documentation,
            ))
        });
        completions.dedup();
        Some(completions)
    }

    pub(crate) fn static_completions_at_offset(
        &self,
        offset: usize,
    ) -> Option<Vec<SemanticCompletion>> {
        if let Some(items) = self
            .recovered_composition
            .as_ref()
            .and_then(|view| view.completions(offset))
        {
            return Some(items);
        }
        if let Some(completions) = self.trait_adaptation_completions_at_offset(offset) {
            return Some(completions);
        }
        let context = self
            .static_receivers
            .iter()
            .filter(|context| context.span.start <= offset && offset <= context.span.end)
            .min_by_key(|context| context.span.end.saturating_sub(context.span.start))?;
        let method = self
            .hierarchy_context_at_offset(offset)
            .and_then(|context| context.method);
        let mut completions = self
            .static_receivers
            .iter()
            .filter(|candidate| candidate.span.authored() == context.span.authored())
            .flat_map(|candidate| self.static_receiver_completions(candidate, method))
            .collect::<Vec<_>>();
        completions.sort();
        completions.dedup();
        Some(completions)
    }

    fn static_receiver_completions(
        &self,
        context: &StaticReceiver,
        method: Option<HierarchyMethodContext>,
    ) -> Vec<SemanticCompletion> {
        match &context.receiver {
            StaticReceiverKind::Byte(receiver) => byte_completions(receiver, false, true),
            StaticReceiverKind::Enum(name) => self
                .enum_case_completions
                .get(name)
                .cloned()
                .unwrap_or_default(),
            StaticReceiverKind::Class { receiver, parent } => self
                .class_member_surfaces
                .iter()
                .find(|surface| surface.receiver == *receiver)
                .into_iter()
                .filter(|surface| surface.valid)
                .flat_map(|surface| &surface.members)
                .filter(|member| {
                    if member.name == "__destruct" {
                        return false;
                    }
                    if member.name == "__construct" {
                        return *parent
                            && method.is_some_and(|method| method.constructor)
                            && member.declaring_class == *receiver
                            && member.access == MemberAccess::External;
                    }
                    (matches!(
                        member.kind,
                        CompilerMemberKind::StaticMethod
                            | CompilerMemberKind::StaticProperty
                            | CompilerMemberKind::Constant
                    ) || (*parent
                        && member.kind == CompilerMemberKind::InstanceMethod
                        && method.is_some_and(|method| !method.is_static)))
                        && (member.access == MemberAccess::External
                            || context.current_class.as_deref()
                                == Some(member.declaring_class.name.as_str()))
                })
                .filter_map(|member| {
                    self.available_member_completion(receiver, member, context.receiver_span)
                })
                .collect(),
        }
    }

    fn class_member_completions(
        &self,
        receiver: &ClassType<ResolvedType>,
        writable: bool,
        context: &MemberReceiver,
    ) -> Vec<SemanticCompletion> {
        if let Some(surface) = self
            .class_member_surfaces
            .iter()
            .find(|surface| surface.receiver == *receiver)
        {
            return surface
                .members
                .iter()
                .filter(|member| {
                    surface.valid
                        && !is_lifecycle_method(&member.name)
                        && matches!(
                            member.kind,
                            CompilerMemberKind::InstanceMethod
                                | CompilerMemberKind::InstanceProperty
                                | CompilerMemberKind::PromotedProperty
                        )
                        && (member.kind != CompilerMemberKind::InstanceMethod
                            || writable
                            || !member.writable)
                        && member.hooks.as_ref().is_none_or(|hooks| {
                            writable
                                || hooks.getter.as_ref().is_some_and(|getter| {
                                    getter.receiver_mode == Some(ReceiverMode::Readonly)
                                })
                        })
                        && (member.access == MemberAccess::External
                            || context.current_class.as_deref()
                                == Some(member.declaring_class.name.as_str()))
                })
                .filter_map(|member| {
                    self.available_member_completion(receiver, member, context.receiver_span)
                })
                .collect();
        }
        let mut completions = Vec::new();
        let mut current = Some(receiver.name.as_str());
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
                                && !is_lifecycle_method(&member.completion.label)
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

    fn available_member_completion(
        &self,
        receiver: &ClassType<ResolvedType>,
        member: &ClassMemberSurfaceEntry,
        receiver_span: Span,
    ) -> Option<SemanticCompletion> {
        let availability = self
            .class_member_availability
            .get(&receiver_span)
            .filter(|facts| &facts.receiver == receiver)
            .map_or(Some(member.availability), |facts| {
                facts.members.get(&member.name).copied()
            })?;
        if availability == MemberAvailability::Unproved {
            return None;
        }
        let mut completion = class_surface_member_completion(
            member,
            &HashMap::new(),
            self.property_backing_fields.get(&member.declaration),
            &self.parameter_presentation,
        );
        if availability == MemberAvailability::RequiresTypeArguments {
            let condition = self
                .contracts
                .conditions
                .get(&member.declaration)
                .or_else(|| {
                    self.contracts
                        .callable_conditions
                        .get(&member.declaration)
                        .and_then(|owner| self.contracts.conditions.get(owner))
                })?;
            let bounds = condition
                .obligations
                .iter()
                .map(|bound| {
                    format!(
                        "{} implements {}",
                        display_resolved_type(&bound.subject_type),
                        display_resolved_type(&ResolvedType::Interface(bound.interface.clone()))
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            completion.detail.push_str(&format!(" where ({bounds})"));
            append_documentation(
                &mut completion.documentation,
                "Availability depends on the inferred method type arguments. The condition must be proved for the invocation.",
            );
        }
        if let Some(documentation) = self
            .class_members
            .get(&member.declaring_class.name)
            .and_then(|members| {
                members
                    .iter()
                    .find(|candidate| candidate.completion.label == member.name)
            })
            .and_then(|member| member.completion.documentation.as_ref())
        {
            append_documentation(&mut completion.documentation, documentation);
        }
        if let Some(origin) = self
            .composition_origins
            .iter()
            .find(|origin| origin.id == member.declaration.expansion)
        {
            append_documentation(
                &mut completion.documentation,
                &format!(
                    "Composed from `{}` into `{}`.",
                    origin.trait_type, origin.composing_class
                ),
            );
        }
        Some(completion)
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

    pub(crate) fn signature_help_at_offset(&self, offset: usize) -> Option<Vec<SignatureHelp>> {
        if let Some(items) = self
            .recovered_composition
            .as_ref()
            .and_then(|view| view.signatures(offset))
        {
            return Some(items);
        }
        let mut signatures = self
            .call_contexts_at_offset(offset)?
            .into_iter()
            .filter_map(|context| self.present_call(context))
            .map(|call| call.signature_help(offset))
            .collect::<Vec<_>>();
        signatures.sort();
        signatures.dedup();
        (!signatures.is_empty()).then_some(signatures)
    }

    fn call_contexts_at_offset(&self, offset: usize) -> Option<Vec<&CallSignatureContext>> {
        let context = self
            .call_signatures
            .iter()
            .filter(|context| context.span.start <= offset && offset <= context.span.end)
            .min_by_key(|context| context.span.end.saturating_sub(context.span.start))?;
        Some(
            self.call_signatures
                .iter()
                .filter(|candidate| candidate.span.authored() == context.span.authored())
                .collect(),
        )
    }

    fn present_call(&self, context: &CallSignatureContext) -> Option<PresentedCall> {
        let symbol = self.symbols.get(context.symbol)?;
        Some(PresentedCall {
            span: context.span,
            arguments: context.arguments.clone(),
            label: context
                .signature
                .clone()
                .unwrap_or_else(|| symbol.signature.clone()),
            parameter_names: symbol.parameter_names.clone(),
        })
    }

    pub(crate) fn named_argument_completions_at_offset(
        &self,
        offset: usize,
    ) -> Option<Vec<SemanticCompletion>> {
        if let Some(items) = self
            .recovered_composition
            .as_ref()
            .and_then(|view| view.named_arguments(offset))
        {
            return Some(items);
        }
        let mut completions = Vec::new();
        for context in self.call_contexts_at_offset(offset)? {
            if let Some(call) = self.present_call(context) {
                completions.extend(call.named_arguments(offset));
            }
        }
        completions.sort();
        completions.dedup();
        Some(completions)
    }

    pub(crate) fn reference_spans_at_offset(
        &self,
        offset: usize,
        include_declaration: bool,
    ) -> Vec<Span> {
        let Some(symbol) = self.symbol_at_offset(offset) else {
            return Vec::new();
        };
        // Clause subjects do not enumerate every use of a generic parameter.
        if self.symbols[symbol].kind == SymbolKind::TypeParameter {
            return Vec::new();
        }
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
        // Explicit semantic classifications take precedence over generic symbol kinds.
        let mut tokens = test_semantic_tokens
            .into_iter()
            .chain(self.assertion_semantic_tokens.iter().copied())
            .chain(self.foreach_semantic_tokens.iter().copied())
            .chain(self.hierarchy_semantic_tokens.iter().copied())
            .chain(
                spans
                    .into_iter()
                    .map(|(span, token_type)| (span, token_type, 0)),
            )
            .collect::<Vec<_>>();
        tokens.sort_by_key(|(span, _, _)| (span.start, span.end));
        tokens.dedup_by_key(|(span, _, _)| (span.start, span.end));
        tokens
    }

    #[cfg(test)]
    pub(crate) fn rename_replacement_at_offset(
        &self,
        offset: usize,
        new_name: &str,
    ) -> Option<String> {
        let occurrence = self
            .occurrences
            .iter()
            .filter(|occurrence| span_contains(occurrence.span, offset))
            .min_by_key(|occurrence| occurrence.span.end.saturating_sub(occurrence.span.start))?;
        let symbol = self.symbols.get(occurrence.symbol)?;
        let bare_name = new_name.strip_prefix('$').unwrap_or(new_name);
        Some(match (symbol.kind, occurrence.role) {
            (SymbolKind::Plain | SymbolKind::Type, _) => new_name.to_string(),
            (SymbolKind::Variable, OccurrenceRole::NamedArgument) => bare_name.to_string(),
            (SymbolKind::Variable, _) => format!("${bare_name}"),
            (SymbolKind::Keyword | SymbolKind::TypeParameter, _) => return None,
        })
    }

    pub(crate) fn rename_edits_at_offset(
        &self,
        offset: usize,
        new_name: &str,
    ) -> Option<Vec<(Span, String)>> {
        let symbol = self.symbol_at_offset(offset)?;
        self.rename_edits_for_symbol(symbol, new_name)
    }

    pub(crate) fn local_rename_edits_at_offset(
        &self,
        offset: usize,
        new_name: &str,
    ) -> Option<Vec<(Span, String)>> {
        let symbol = self
            .occurrences
            .iter()
            .filter(|occurrence| span_contains(occurrence.span, offset))
            .filter(|occurrence| {
                self.symbols
                    .get(occurrence.symbol)
                    .is_some_and(|symbol| symbol.local_name.is_some())
            })
            .min_by_key(|occurrence| occurrence.span.end.saturating_sub(occurrence.span.start))?
            .symbol;
        self.rename_edits_for_symbol(symbol, new_name)
    }

    fn rename_edits_for_symbol(
        &self,
        symbol: usize,
        new_name: &str,
    ) -> Option<Vec<(Span, String)>> {
        let kind = self.symbols.get(symbol)?.kind;
        if matches!(kind, SymbolKind::Keyword | SymbolKind::TypeParameter) {
            return None;
        }
        let bare_name = new_name.strip_prefix('$').unwrap_or(new_name);
        Some(
            self.occurrences
                .iter()
                .filter(|occurrence| occurrence.symbol == symbol)
                .map(|occurrence| {
                    let replacement = match (kind, occurrence.role) {
                        (SymbolKind::Variable, OccurrenceRole::NamedArgument) => {
                            bare_name.to_string()
                        }
                        (SymbolKind::Variable, _) => format!("${bare_name}"),
                        _ => bare_name.to_string(),
                    };
                    (occurrence.span, replacement)
                })
                .collect(),
        )
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
            | GlobalReferenceRole::Uses
            | GlobalReferenceRole::TraitAdaptation
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
    if matches!(symbol.kind, SymbolKind::Type | SymbolKind::TypeParameter) {
        return Some(1);
    }
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

fn foreach_binding_semantic_tokens(
    info: &SemanticInfo,
    source_id: SourceId,
) -> Vec<SemanticTokenSpan> {
    const VARIABLE: u32 = 0;

    let foreach_bindings = info
        .binding_resolution
        .declarations_by_id
        .values()
        .filter(|declaration| {
            matches!(
                declaration.kind,
                BindingKind::ForeachFirst | BindingKind::ForeachValue
            )
        })
        .filter_map(|declaration| {
            declaration
                .span
                .filter(|span| span.source == source_id)
                .map(|span| (declaration.id, span))
        })
        .collect::<Vec<_>>();
    let binding_ids = foreach_bindings
        .iter()
        .map(|(binding, _)| *binding)
        .collect::<HashSet<_>>();
    let mut tokens = foreach_bindings
        .into_iter()
        .map(|(_, span)| (span, VARIABLE, SEMANTIC_TOKEN_DECLARATION))
        .collect::<Vec<_>>();
    tokens.extend(
        info.binding_resolution
            .uses_by_span
            .iter()
            .filter_map(|(span, binding)| {
                (span.source == source_id && binding_ids.contains(binding))
                    .then_some((*span, VARIABLE, 0))
            }),
    );
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
    constructor_references: Vec<(Span, usize)>,
    parameter_presentation: ParameterPresentation,
    classes: HashMap<String, usize>,
    contract_types: HashMap<String, usize>,
    error_classes: HashSet<String>,
    enums: HashMap<String, usize>,
    enum_cases: HashMap<(String, String), usize>,
    class_parents: HashMap<String, String>,
    class_members: HashMap<String, Vec<ClassMemberCompletion>>,
    class_property_symbols: HashMap<(String, String), usize>,
    class_static_property_symbols: HashMap<(String, String), usize>,
    class_constant_symbols: HashMap<(String, String), usize>,
    enum_case_completions: HashMap<String, Vec<SemanticCompletion>>,
    enum_member_completions: HashMap<String, Vec<SemanticCompletion>>,
    methods: HashMap<(String, String), usize>,
    functions: HashMap<String, usize>,
    callable_declarations: HashMap<Span, usize>,
    member_receivers: Vec<MemberReceiver>,
    static_receivers: Vec<StaticReceiver>,
    local_scopes: Vec<HashMap<String, usize>>,
    local_scope_ends: Vec<usize>,
    local_visibilities: Vec<LocalVisibility>,
    call_signatures: Vec<CallSignatureContext>,
    pending_named_argument_references: Vec<PendingNamedArgumentReference>,
    semantic_hovers: Vec<SemanticHover>,
    attribute_semantic_tokens: Vec<(Span, u32)>,
    assertion_semantic_tokens: Vec<SemanticTokenSpan>,
    foreach_semantic_tokens: Vec<SemanticTokenSpan>,
    hierarchy_semantic_tokens: Vec<SemanticTokenSpan>,
    property_accessors: PropertyAccessorNavigation,
    assertion_completions: Vec<(Span, doriac::semantics::AssertionCompletionInfo)>,
    attribute_parameter_occurrences: Vec<AttributeParameterOccurrence>,
    missing_callables: Vec<MissingCallable>,
    member_occurrences: Vec<MemberOccurrence>,
    member_parents: Vec<MemberParent>,
    hierarchy_classes: Vec<HierarchyClass>,
    hierarchy_members: Vec<HierarchyMember>,
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
            constructor_references: Vec::new(),
            parameter_presentation: semantic_info
                .map(ParameterPresentation::from_semantic_info)
                .unwrap_or_default(),
            classes: HashMap::new(),
            contract_types: HashMap::new(),
            error_classes: HashSet::new(),
            enums: HashMap::new(),
            enum_cases: HashMap::new(),
            class_parents: HashMap::new(),
            class_members: HashMap::new(),
            class_property_symbols: HashMap::new(),
            class_static_property_symbols: HashMap::new(),
            class_constant_symbols: HashMap::new(),
            enum_case_completions: HashMap::new(),
            enum_member_completions: HashMap::new(),
            methods: HashMap::new(),
            functions: HashMap::new(),
            callable_declarations: HashMap::new(),
            member_receivers: Vec::new(),
            static_receivers: Vec::new(),
            local_scopes: Vec::new(),
            local_scope_ends: Vec::new(),
            local_visibilities: Vec::new(),
            call_signatures: Vec::new(),
            pending_named_argument_references: Vec::new(),
            semantic_hovers: Vec::new(),
            attribute_semantic_tokens: Vec::new(),
            assertion_semantic_tokens: Vec::new(),
            foreach_semantic_tokens: Vec::new(),
            hierarchy_semantic_tokens: Vec::new(),
            property_accessors: PropertyAccessorNavigation::default(),
            assertion_completions: Vec::new(),
            attribute_parameter_occurrences: Vec::new(),
            missing_callables: Vec::new(),
            member_occurrences: Vec::new(),
            member_parents: Vec::new(),
            hierarchy_classes: Vec::new(),
            hierarchy_members: Vec::new(),
            statement_expression_span: None,
            when_depth: 0,
        }
    }

    fn build(mut self, program: &Program) -> AnalysisSnapshot {
        self.collect_declarations(program);
        self.collect_semantic_only_declarations();
        self.collect_references(program);
        self.collect_resolved_type_references();
        self.collect_condition_subject_references();
        self.collect_composed_references();
        self.resolve_named_argument_references();
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
            self.foreach_semantic_tokens
                .extend(foreach_binding_semantic_tokens(info, self.source_id));
            self.assertion_semantic_tokens
                .extend(compiler_assertion_symbol_tokens(
                    &info.global_symbols,
                    self.source_id,
                    self.tokens,
                ));
            self.assertion_completions.extend(
                info.assertion_completions
                    .iter()
                    .filter(|(span, _)| span.source == self.source_id)
                    .map(|(span, completion)| (*span, completion.clone())),
            );
            self.collect_compiler_test_hovers(info);
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
            has_standalone_entry: program.includes.is_empty()
                && program.items.iter().any(|item| {
                    matches!(item, Item::Function(function) if doriac::names::source_name_is(&function.name, "main"))
                }),
            contracts: self
                .semantic_info
                .map(|info| info.contracts.clone())
                .unwrap_or_default(),
            composition_origins: self
                .semantic_info
                .map(|info| info.composition.origins.clone())
                .unwrap_or_default(),
            composition_obligations: self.semantic_info.map(|info| info.composition.obligations.clone()).unwrap_or_default(),
            class_member_surfaces: self
                .semantic_info
                .map(|info| info.class_member_surfaces.clone())
                .unwrap_or_default(),
            class_member_availability: self
                .semantic_info
                .map(|info| info.class_member_availability.clone())
                .unwrap_or_default(),
            property_backing_fields: self.semantic_info.map(|info| info.property_backing_fields.clone()).unwrap_or_default(),
            trait_adaptations: program.items.iter().filter_map(|item| match item {
                Item::Class(class) => Some(&class.members),
                Item::Trait(declaration) => Some(&declaration.members),
                _ => None,
            }).flatten().filter_map(|member| match member {
                ClassMember::Uses(uses) => Some(&uses.adaptations),
                _ => None,
            }).flatten().filter(|adaptation| adaptation.method.span.source == self.source_id).cloned().collect(),
            trait_adaptation_surfaces: self.semantic_info.map(|info| info.trait_adaptation_surfaces.clone()).unwrap_or_default(),
            composition_rename: self.semantic_info.map(|info| info.composition_rename.clone()).unwrap_or_default(),
            recovered_composition: None,
            source_id: self.source_id,
            compilation_context: CompilationContext::default(),
            global_symbols: GlobalSymbolFacts::default(),
            directive_semantic_tokens: Vec::new(),
            attribute_semantic_tokens: self.attribute_semantic_tokens,
            assertion_semantic_tokens: self.assertion_semantic_tokens,
            foreach_semantic_tokens: self.foreach_semantic_tokens,
            hierarchy_semantic_tokens: self.hierarchy_semantic_tokens,
            property_accessors: self.property_accessors,
            assertion_completions: self.assertion_completions,
            test_semantics,
            source_semantic_context,
            attribute_info,
            attribute_parameter_occurrences: self.attribute_parameter_occurrences,
            symbols: self.symbols,
            occurrences: self.occurrences,
            constructor_references: self.constructor_references,
            parameter_presentation: self.parameter_presentation,
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
            hierarchy_classes: self.hierarchy_classes,
            hierarchy_members: self.hierarchy_members,
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
                            && self.symbols[occurrence.symbol].local_name.as_deref()
                                == Some(parameter.name.as_str())
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

        let mut foreach_loops = info
            .foreach_loops
            .values()
            .filter(|loop_info| loop_info.value_binding_span.source == self.source_id)
            .collect::<Vec<_>>();
        foreach_loops.sort_by_key(|loop_info| {
            loop_info
                .first_binding_span
                .unwrap_or(loop_info.value_binding_span)
        });
        for loop_info in foreach_loops {
            if let (Some(binding_span), Some(_), Some(ty)) = (
                loop_info.first_binding_span,
                loop_info.first_binding_type_span,
                loop_info.first_binding_type.as_ref(),
            ) {
                let role = match loop_info.iteration_kind {
                    ForeachIterationKind::SequenceIndex => "Zero-Based Sequence Index",
                    ForeachIterationKind::DictionaryKey => "Dictionary Key",
                    ForeachIterationKind::ValueOnly => continue,
                };
                hovers.extend(foreach_binding_hovers(
                    info,
                    binding_span,
                    BindingKind::ForeachFirst,
                    ty,
                    role,
                    "Readonly",
                    &loop_info.iterable_type,
                ));
            }

            let role = match loop_info.iteration_kind {
                ForeachIterationKind::SequenceIndex => "Sequence Element",
                ForeachIterationKind::DictionaryKey => "Dictionary Value",
                ForeachIterationKind::ValueOnly => "Iteration Value",
            };
            let access = match loop_info.value_access {
                ForeachValueAccess::Readonly => "Readonly",
                ForeachValueAccess::Writable => "Writable",
            };
            if loop_info.value_binding_type_span.is_some() {
                hovers.extend(foreach_binding_hovers(
                    info,
                    loop_info.value_binding_span,
                    BindingKind::ForeachValue,
                    &loop_info.value_binding_type,
                    role,
                    access,
                    &loop_info.iterable_type,
                ));
            }
        }

        let mut bindings = info
            .binding_resolution
            .declarations_by_id
            .values()
            .filter_map(|declaration| {
                let span = declaration.span?;
                let ty = declaration.source_type.as_ref()?;
                let documentation = doriac::compiler_known_stream::is_call_scoped_request(ty)
                    .then(|| PENDING_REQUEST_DOCUMENTATION.to_string())
                    .or_else(|| inline_copy_documentation(ty));
                (span.source == self.source_id
                    && (documentation.is_some()
                        || matches!(non_nullable_type(ty), ResolvedType::Function(_))))
                .then_some((
                    declaration.id,
                    span,
                    ty,
                    declaration.kind,
                    declaration.ownership,
                    declaration.name.as_str(),
                    documentation,
                ))
            })
            .collect::<Vec<_>>();
        bindings.sort_by_key(|(id, span, _, _, _, _, _)| (span.start, span.end, *id));
        for (binding_id, declaration_span, ty, kind, ownership, declaration_name, documentation) in
            bindings
        {
            let name = binding_source_name(info, binding_id);
            let mut markdown = if let Some(documentation) = documentation.as_deref() {
                let access = if ownership == BindingOwnership::WritableBorrow
                    || info
                        .binding_resolution
                        .declarations_by_id
                        .get(&binding_id)
                        .is_some_and(|declaration| declaration.writable)
                {
                    "writable "
                } else {
                    ""
                };
                format!(
                    "```doria\n{access}{} {name}\n```\n\n{documentation}",
                    display_resolved_type(ty),
                )
            } else {
                format!(
                    "```doria\n{} {name}\n```\n\nSemantically resolved function-typed binding.",
                    display_resolved_type(ty),
                )
            };
            if documentation.is_none()
                && matches!(
                    kind,
                    BindingKind::FunctionParameter
                        | BindingKind::MethodParameter
                        | BindingKind::ClosureParameter
                )
            {
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
                .map_or(declaration_span, |token| token.span.in_expansion(declaration_span.expansion));
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
        closures.sort_by_key(|closure| closure.closure_id);
        for closure in closures
            .into_iter()
            .filter(|closure| closure.closure_id.source == self.source_id)
        {
            let closure_span = Span::in_source(
                closure.closure_id.source,
                closure.closure_id.start,
                closure.closure_id.end,
            )
            .in_expansion(closure.closure_id.expansion);
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
        self.hierarchy_semantic_tokens
            .extend(self.tokens.iter().filter_map(|token| {
                matches!(token.kind, TokenKind::Uses | TokenKind::Insteadof).then_some((
                    token.span,
                    SEMANTIC_TOKEN_KEYWORD,
                    0,
                ))
            }));
        for item in &program.items {
            match item {
                Item::Class(class) => self.collect_class(class),
                Item::Enum(enum_decl) => self.collect_enum(enum_decl),
                Item::Trait(trait_decl) => {
                    self.collect_contract_declaration(
                        "trait",
                        &trait_decl.name,
                        trait_decl.name_span,
                        &trait_decl.type_params,
                        &[],
                    );
                    for member in &trait_decl.members {
                        match member {
                            ClassMember::Method(method) => {
                                self.collect_method(&trait_decl.name, method)
                            }
                            ClassMember::Property(property) => self.collect_authored_property(
                                &trait_decl.name,
                                property,
                                phpdoc_before(self.text, property.span.start),
                                PropertyHookContext::Trait,
                            ),
                            ClassMember::Constant(constant) => {
                                self.collect_authored_constant(&trait_decl.name, constant)
                            }
                            _ => {}
                        }
                    }
                }
                Item::Interface(interface) => {
                    self.collect_contract_declaration(
                        "interface",
                        &interface.name,
                        interface.name_span,
                        &interface.type_params,
                        &interface.parents,
                    );
                    for requirement in &interface.requirements {
                        self.collect_method(&interface.name, requirement);
                    }
                    for property in &interface.properties {
                        self.collect_authored_property(
                            &interface.name,
                            property,
                            phpdoc_before(self.text, property.span.start),
                            PropertyHookContext::Interface,
                        );
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
                    self.append_callable_source_documentation(&mut documentation, function);
                    let symbol = self.add_declaration_symbol(
                        selection_span,
                        function_signature(function, None, &self.parameter_presentation),
                        documentation,
                        SymbolKind::Plain,
                    );
                    self.record_callable_parameters(symbol, function);
                    self.callable_declarations.insert(function.span, symbol);
                    self.functions.insert(function.name.clone(), symbol);
                }
                _ => {}
            }
        }
    }

    fn collect_contract_declaration(
        &mut self,
        kind: &str,
        name: &str,
        span: Span,
        parameters: &[doriac::ast::TypeParamDecl],
        parents: &[TypeRef],
    ) {
        let signature = contract_signature(kind, name, parameters, parents);
        let documentation = if kind == "interface" {
            "Nominal interface declaration. Checked requirements govern owned and borrowed interface values, erased calls, narrowing, and shared payload views. Selected trait members can satisfy interface requirements; conformance is checked after composition."
        } else {
            "Compile-time trait declaration. Members are specialized in each composing class; aliases retain separate identities and requirements are checked against the selected implementation."
        };
        let symbol = self.add_declaration_symbol(
            span,
            signature,
            Some(documentation.to_string()),
            SymbolKind::Plain,
        );
        self.contract_types.insert(name.to_string(), symbol);
    }

    fn collect_semantic_only_declarations(&mut self) {
        let Some(info) = self.semantic_info else {
            return;
        };
        let enums = info
            .enum_declarations
            .iter()
            .filter(|info| !self.enums.contains_key(&info.source_type.name))
            .cloned()
            .collect::<Vec<_>>();
        let classes = info
            .classes
            .iter()
            .filter(|info| !self.classes.contains_key(&info.declaration_name))
            .cloned()
            .collect::<Vec<_>>();
        let interfaces = info
            .contracts
            .interfaces
            .iter()
            .filter(|fact| fact.valid && !self.contract_types.contains_key(&fact.name))
            .cloned()
            .collect::<Vec<_>>();
        let conformances = &info.contracts.conformances;

        for interface in interfaces {
            let parents = interface
                .parents
                .iter()
                .map(|parent| parent.authored_type.clone())
                .collect::<Vec<_>>();
            let symbol = self.add_metadata_symbol(
                contract_signature(
                    "interface",
                    &interface.name,
                    &interface.type_parameters,
                    &parents,
                ),
                Some(
                    crate::hover_description(&TokenKind::Identifier(interface.name.clone()))
                        .unwrap_or(
                            "Nominal interface contract supplied by compiler semantic analysis.",
                        )
                        .to_string(),
                ),
                SymbolKind::Plain,
            );
            self.contract_types.insert(interface.name, symbol);
        }

        for info in enums {
            let name = &info.source_type.name;
            let applied = display_resolved_type(&ResolvedType::Enum(info.source_type.clone()));
            let copy = self.enum_copy(&info.source_type);
            let symbol = self.add_metadata_symbol(
                format!("enum {applied}"),
                Some(enum_documentation(name, copy)),
                SymbolKind::Plain,
            );
            self.enums.insert(name.clone(), symbol);

            let mut completions = Vec::new();
            for case in &info.cases {
                let signature = enum_case_signature(name, &applied, &case.name, Some(case));
                let documentation = enum_case_documentation(Some(&info), Some(case), copy);
                let case_symbol = self.add_metadata_symbol(
                    signature.clone(),
                    documentation.clone(),
                    SymbolKind::Plain,
                );
                self.symbols[case_symbol].parameter_names = case
                    .payload
                    .iter()
                    .map(|field| field.name.clone())
                    .collect();
                self.enum_cases
                    .insert((name.clone(), case.name.clone()), case_symbol);
                completions.push(SemanticCompletion {
                    label: case.name.clone(),
                    kind: 20,
                    detail: signature,
                    documentation,
                });
            }
            self.enum_case_completions.insert(name.clone(), completions);

            if let Some(backing_type) = info.backing_type {
                let backing = enum_backing_name(backing_type).to_string();
                self.enum_member_completions.insert(
                    name.clone(),
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

        // Classes declared in project sources: they reach this file through the
        // package graph, but they are the project's own classes, not
        // compiler-known ones.
        let source_declared_classes = info
            .global_symbols
            .declarations
            .iter()
            .filter(|declaration| declaration.kind == GlobalSymbolKind::Class)
            .flat_map(|declaration| {
                [
                    declaration.qualified_name.clone(),
                    declaration.source_name.clone(),
                ]
            })
            .collect::<HashSet<_>>();
        for info in classes {
            let class_name = info.declaration_name;
            let inline_copy =
                ClassType::new(class_name.clone(), info.arguments.clone()).inline_copy_type();
            let mut documentation = match inline_copy {
                Some(inline_copy) => Some(inline_value_documentation(inline_copy)),
                None if source_declared_classes.contains(&class_name) => None,
                None => {
                    Some("Compiler-known Doria class supplied by semantic analysis.".to_string())
                }
            };
            let mut interfaces = conformances
                .iter()
                .filter(|fact| {
                    fact.status == ConformanceStatus::Checked
                        && matches!(&fact.implementing_type, ResolvedType::Class(class)
                            if class.name == class_name && class.arguments == info.arguments)
                })
                .map(|fact| {
                    format!(
                        "`{}`",
                        display_resolved_type(&ResolvedType::Interface(fact.interface.clone()))
                    )
                })
                .collect::<Vec<_>>();
            interfaces.sort();
            interfaces.dedup();
            if !interfaces.is_empty() {
                append_documentation(
                    &mut documentation,
                    &format!("**Implemented Interfaces:** {}", interfaces.join(", ")),
                );
            }
            if let Some(request) = pending_request_documentation(&class_name) {
                append_documentation(&mut documentation, request);
            }
            if class_name == doriac::compiler_known_stream::TRANSFER_GROUP {
                append_documentation(&mut documentation, TRANSFER_GROUP_CONTROL_DOCUMENTATION);
            }
            if let Some(position) = file_position_documentation(&class_name) {
                append_documentation(&mut documentation, position);
            }
            if let Some(writable) = writable_file_documentation(&class_name) {
                append_documentation(&mut documentation, writable);
            }
            if class_name == doriac::compiler_known_stream::STANDARD_STREAMS {
                append_documentation(&mut documentation, STANDARD_STREAM_DOCUMENTATION);
            }
            let symbol = self.add_metadata_symbol(
                if inline_copy.is_some() {
                    class_name.clone()
                } else {
                    format!("class {class_name}")
                },
                documentation,
                if inline_copy.is_some() {
                    SymbolKind::Type
                } else {
                    SymbolKind::Plain
                },
            );
            self.classes.insert(class_name.clone(), symbol);

            // These fallback tables are keyed by name, not by application.
            // Generic members must come from the checked receiver surface.
            if !info.arguments.is_empty() {
                continue;
            }
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

    fn enum_copy(&self, ty: &doriac::enums::EnumType) -> Option<bool> {
        self.semantic_info?
            .enums
            .iter()
            .find(|info| &info.source_type == ty)
            .map(|info| info.capabilities.copy)
    }

    fn collect_enum(&mut self, declaration: &EnumDecl) {
        let semantic = self
            .semantic_info
            .and_then(|info| {
                info.enum_declarations
                    .iter()
                    .find(|info| info.name_span == declaration.name_span)
            })
            .cloned();
        let copy = semantic
            .as_ref()
            .and_then(|info| self.enum_copy(&info.source_type));
        let applied = format!(
            "{}{}",
            declaration.name,
            type_parameter_signature(&declaration.type_params)
        );
        let symbol = self.add_declaration_symbol(
            declaration.name_span,
            format!("enum {applied}"),
            semantic
                .as_ref()
                .map(|_| enum_documentation(&declaration.name, copy)),
            SymbolKind::Plain,
        );
        self.enums.insert(declaration.name.clone(), symbol);

        let mut completions = Vec::new();
        for case in &declaration.cases {
            let semantic_case = semantic
                .as_ref()
                .and_then(|info| info.cases.iter().find(|info| info.name == case.name));
            let signature =
                enum_case_signature(&declaration.name, &applied, &case.name, semantic_case);
            let documentation = enum_case_documentation(semantic.as_ref(), semantic_case, copy);
            let case_symbol = self.add_declaration_symbol(
                case.name_span,
                signature.clone(),
                documentation.clone(),
                SymbolKind::Plain,
            );
            self.symbols[case_symbol].parameter_names = case
                .payload
                .iter()
                .map(|field| field.name.clone())
                .collect();
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

    fn collect_condition_subject_references(&mut self) {
        let Some(info) = self.semantic_info else {
            return;
        };
        let mut conditions = info.contracts.conditions.values().collect::<Vec<_>>();
        conditions.sort_by_key(|condition| condition.clause.start);
        let mut symbols = HashMap::new();
        let mut references = HashSet::new();
        for condition in conditions {
            for obligation in &condition.obligations {
                let parameter = &obligation.subject;
                if obligation.subject_span.source != self.source_id
                    || parameter.declaration.source != self.source_id
                {
                    continue;
                }
                let symbol = if let Some(symbol) = symbols.get(&parameter.declaration) {
                    *symbol
                } else {
                    let Some(selection) =
                        find_identifier_span(self.tokens, parameter.declaration, &parameter.name)
                    else {
                        continue;
                    };
                    let symbol = self.add_declaration_symbol(
                        selection,
                        format!("type parameter {}", parameter.name),
                        Some("Lexical type parameter selected by the compiler for this declaration condition. This predicate does not establish an active member or conformance witness.".to_string()),
                        SymbolKind::TypeParameter,
                    );
                    symbols.insert(parameter.declaration, symbol);
                    symbol
                };
                if references.insert(obligation.subject_span) {
                    self.record_reference(obligation.subject_span, symbol);
                }
            }
        }
    }

    fn collect_nominal_condition(&mut self, condition: Option<&NominalCondition>) {
        let Some(condition) = condition else {
            return;
        };
        self.hierarchy_semantic_tokens
            .push((condition.keyword_span, SEMANTIC_TOKEN_KEYWORD, 0));
        for bound in &condition.bounds {
            self.hierarchy_semantic_tokens
                .push((bound.implements_span, SEMANTIC_TOKEN_KEYWORD, 0));
        }
        self.semantic_hovers.push(SemanticHover::new(
            condition.keyword_span,
            "Local nominal capability condition. All bounds must hold after generic substitution before this conformance or member is available.".to_string(),
        ));
    }

    fn collect_class(&mut self, class: &ClassDecl) {
        for conformance in &class.implements {
            self.collect_nominal_condition(conformance.condition.as_ref());
        }
        let selection_span = self.declaration_name_span(class.span, &class.name, TokenKind::Class);
        if let Some(span) = class.open_span {
            self.hierarchy_semantic_tokens
                .push((span, SEMANTIC_TOKEN_KEYWORD, 0));
        }
        if let Some(span) = class.extends_span {
            self.hierarchy_semantic_tokens
                .push((span, SEMANTIC_TOKEN_KEYWORD, 0));
        }
        if let Some(span) = class.parent_span {
            self.hierarchy_semantic_tokens
                .push((span, SEMANTIC_TOKEN_TYPE, 0));
        }
        let conforms_to_error = class
            .implements
            .iter()
            .any(|interface| interface.condition.is_none() && interface.ty.name == "Error");
        let mut documentation = phpdoc_before(self.text, class.span.start);
        if let Some(hierarchy) = self
            .semantic_info
            .and_then(|info| info.class_hierarchy.get(&class.span))
        {
            append_documentation(
                &mut documentation,
                &class_hierarchy_documentation(hierarchy),
            );
        }
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
        if let (Some(identity), Some(hierarchy)) = (
            self.member_owner(&class.name, class.name_span, true),
            self.semantic_info
                .and_then(|info| info.class_hierarchy.get(&class.span)),
        ) {
            self.hierarchy_classes.push(HierarchyClass {
                identity,
                source_name: class.name.clone(),
                qualified_name: hierarchy.name.clone(),
                type_parameters: class
                    .type_params
                    .iter()
                    .map(|parameter| parameter.name.clone())
                    .collect(),
                is_open: hierarchy.is_open,
                access: class.access,
                ancestors: hierarchy.ancestors.clone(),
                body_span: class.span,
            });
        }
        if let Some(parent) = &class.parent {
            self.class_parents
                .insert(class.name.clone(), parent.ast_type_name().to_string());
            if let (Some(child), Some(parent)) = (
                self.member_owner(&class.name, class.name_span, true),
                class
                    .parent_span
                    .and_then(|span| self.member_owner(parent.ast_type_name(), span, false)),
            ) {
                self.member_parents.push(MemberParent { child, parent });
            }
        }

        for member in &class.members {
            match member {
                ClassMember::Uses(_) => {}
                ClassMember::Method(method) => {
                    self.collect_method(&class.name, method);
                    if method.availability.is_some() {
                        continue;
                    }
                    self.class_members
                        .entry(class.name.clone())
                        .or_default()
                        .push(ClassMemberCompletion {
                            completion: SemanticCompletion {
                                label: method.name.clone(),
                                kind: 2,
                                detail: function_signature(
                                    method,
                                    Some(&class.name),
                                    &self.parameter_presentation,
                                ),
                                documentation: phpdoc_before(self.text, method.span.start),
                            },
                            writable: method.writable_this,
                            internal: matches!(method.access, MemberAccess::Internal),
                            is_static: method.is_static,
                        });
                }
                ClassMember::Property(property) => {
                    let documentation = if conforms_to_error && property.name == "message" {
                        Some("Required externally accessible readonly message for the compiler-known `Error` contract.".to_string())
                    } else {
                        phpdoc_before(self.text, property.span.start)
                    };
                    self.collect_authored_property(
                        &class.name,
                        property,
                        documentation.clone(),
                        PropertyHookContext::Class,
                    );
                    if property.availability.is_some() {
                        continue;
                    }
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
                    if let Some(owner) = self.member_owner(&class.name, class.name_span, true) {
                        self.hierarchy_members.push(HierarchyMember {
                            owner,
                            name: property.name.clone(),
                            kind: MemberKind::Property,
                            declaration: property.span,
                            detail: format!("{} ${}", property.ty, property.name),
                            documentation: phpdoc_before(self.text, property.span.start),
                            access: property.access,
                            is_static: property.is_static,
                            writable_receiver: false,
                            is_open: false,
                            is_override: false,
                            virtual_root: None,
                            overridden_declaration: None,
                            override_stub: None,
                            body_span: None,
                        });
                    }
                }
                ClassMember::Constant(constant) => {
                    self.collect_authored_constant(&class.name, constant);
                    if let Some(owner) = self.member_owner(&class.name, class.name_span, true) {
                        self.hierarchy_members.push(HierarchyMember {
                            owner,
                            name: constant.name.clone(),
                            kind: MemberKind::Constant,
                            declaration: constant.span,
                            detail: format!("{}::{}", class.name, constant.name),
                            documentation: phpdoc_before(self.text, constant.span.start),
                            access: constant.access,
                            is_static: true,
                            writable_receiver: false,
                            is_open: false,
                            is_override: false,
                            virtual_root: None,
                            overridden_declaration: None,
                            override_stub: None,
                            body_span: None,
                        });
                    }
                }
            }
        }
    }

    fn collect_authored_constant(&mut self, owner: &str, constant: &doriac::ast::ConstDecl) {
        let symbol = self.add_declaration_symbol(
            constant.name_span,
            format!("{owner}::{}", constant.name),
            phpdoc_before(self.text, constant.span.start),
            SymbolKind::Plain,
        );
        self.class_constant_symbols
            .insert((owner.to_string(), constant.name.clone()), symbol);
        self.record_member_occurrence(
            owner,
            &constant.name,
            MemberKind::Constant,
            constant.name_span,
            true,
        );
        if let Some(occurrence) = self
            .member_occurrences
            .last_mut()
            .filter(|occurrence| occurrence.span == constant.name_span)
        {
            occurrence.exact_declaration = Some(constant.span);
        }
    }

    fn collect_authored_property(
        &mut self,
        owner: &str,
        property: &doriac::ast::PropertyDecl,
        mut documentation: Option<String>,
        context: PropertyHookContext,
    ) {
        self.collect_nominal_condition(property.availability.as_ref());
        self.collect_property_hooks(owner, property, context, &mut documentation);
        let selection_span =
            find_variable_span(self.tokens, property.span, &property.name).unwrap_or(property.span);
        let symbol = self.add_declaration_symbol(
            selection_span,
            format!(
                "{} ${}{}",
                property.ty,
                property.name,
                nominal_condition_signature(property.availability.as_ref())
            ),
            documentation,
            SymbolKind::Variable,
        );
        if property.availability.is_none() {
            self.class_property_symbols
                .insert((owner.to_string(), property.name.clone()), symbol);
            if property.is_static {
                self.class_static_property_symbols
                    .insert((owner.to_string(), property.name.clone()), symbol);
            }
        }
        self.record_member_occurrence(
            owner,
            &property.name,
            MemberKind::Property,
            selection_span,
            true,
        );
        if let Some(occurrence) = self
            .member_occurrences
            .last_mut()
            .filter(|occurrence| occurrence.span == selection_span)
        {
            occurrence.exact_declaration = Some(property.span);
        }
    }

    fn collect_method(&mut self, class_name: &str, method: &FunctionDecl) {
        self.collect_nominal_condition(method.availability.as_ref());
        let selection_span = method.name_span;
        if let Some(span) = method.open_span {
            self.hierarchy_semantic_tokens
                .push((span, SEMANTIC_TOKEN_KEYWORD, 0));
        }
        if let Some(span) = method.override_span {
            self.hierarchy_semantic_tokens
                .push((span, SEMANTIC_TOKEN_KEYWORD, 0));
        }
        self.collect_constructor_parameter_roles(class_name, method);
        let mut documentation = phpdoc_before(self.text, method.span.start);
        self.append_callable_effect_documentation(&mut documentation, method.span);
        self.append_callable_source_documentation(&mut documentation, method);
        if method.body.as_block().is_none() {
            append_documentation(
                &mut documentation,
                "Declared requirement, not an executable method body.",
            );
        }
        if let Some(hierarchy) = self
            .semantic_info
            .and_then(|info| info.method_hierarchy.get(&method.span))
        {
            append_documentation(
                &mut documentation,
                &method_hierarchy_documentation(hierarchy),
            );
        }
        let hierarchy_documentation = documentation.clone();
        let symbol = self.add_declaration_symbol(
            selection_span,
            function_signature(method, Some(class_name), &self.parameter_presentation),
            documentation,
            SymbolKind::Plain,
        );
        self.record_callable_parameters(symbol, method);
        self.callable_declarations.insert(method.span, symbol);
        if method.availability.is_none() {
            self.methods
                .insert((class_name.to_string(), method.name.clone()), symbol);
        }
        self.record_method_declaration_occurrence(class_name, method, selection_span);
        if let (Some(owner), Some(hierarchy)) = (
            self.member_owner(class_name, method.span, true),
            self.semantic_info
                .and_then(|info| info.method_hierarchy.get(&method.span)),
        ) {
            self.hierarchy_members.push(HierarchyMember {
                owner,
                name: method.name.clone(),
                kind: MemberKind::Method,
                declaration: method.span,
                detail: function_signature(method, Some(class_name), &self.parameter_presentation),
                documentation: hierarchy_documentation,
                access: hierarchy.access,
                is_static: hierarchy.is_static,
                writable_receiver: hierarchy.receiver_mode == Some(ReceiverMode::Writable),
                is_open: hierarchy.is_open,
                is_override: hierarchy.is_override,
                virtual_root: hierarchy.virtual_root,
                overridden_declaration: hierarchy.overridden_declaration,
                override_stub: hierarchy
                    .virtual_root
                    .is_some()
                    .then(|| override_stub(method)),
                body_span: method.body.as_block().map(|body| body.span),
            });
        }
    }

    fn collect_constructor_parameter_roles(&mut self, class_name: &str, method: &FunctionDecl) {
        if method.name != "__construct" {
            return;
        }

        for parameter in &method.params {
            let semantic_role = self
                .semantic_info
                .and_then(|info| info.constructor_parameters.get(&parameter.span))
                .map(|parameter| parameter.role.clone());
            match (parameter.constructor_role, semantic_role) {
                (
                    ConstructorParameterRole::Promoted { access, .. },
                    Some(ConstructorParameterSemanticRole::Promoted {
                        access: semantic_access,
                    }),
                ) if access == semantic_access => {
                    self.collect_promoted_property(class_name, parameter, access);
                }
                (
                    ConstructorParameterRole::InheritedPropertyOverride { override_span },
                    Some(ConstructorParameterSemanticRole::InheritedPropertyOverride {
                        declaring_class,
                        property_name,
                        declaration,
                    }),
                ) => {
                    let Some(family) = self
                        .semantic_info
                        .and_then(|info| info.property_families.get(&declaration))
                        .filter(|family| {
                            family.root_declaring_class == declaring_class
                                && family.root_property_name == property_name
                                && family.override_parameters.contains(&parameter.span)
                        })
                    else {
                        continue;
                    };
                    self.hierarchy_semantic_tokens
                        .push((override_span, SEMANTIC_TOKEN_KEYWORD, 0));
                    self.record_member_occurrence(
                        &declaring_class,
                        &property_name,
                        MemberKind::Property,
                        override_span,
                        false,
                    );
                    if let Some(occurrence) = self
                        .member_occurrences
                        .last_mut()
                        .filter(|occurrence| occurrence.span == override_span)
                    {
                        occurrence.exact_declaration = Some(declaration);
                        occurrence.relationship_only = true;
                    }
                    self.semantic_hovers.push(SemanticHover::new(
                        override_span,
                        format!(
                            "Inherited property override for `{}::{}`.\n\nThe constructor parameter reuses the inherited property and declares no new storage. The parent construction phase initializes the root property.",
                            family.root_declaring_class, family.root_property_name,
                        ),
                    ));
                }
                (
                    ConstructorParameterRole::ConstructorOnly { parameter_span },
                    Some(ConstructorParameterSemanticRole::ConstructorOnly),
                ) => {
                    self.hierarchy_semantic_tokens.push((
                        parameter_span,
                        SEMANTIC_TOKEN_KEYWORD,
                        0,
                    ));
                    self.semantic_hovers.push(SemanticHover::new(
                        parameter_span,
                        "Constructor-only parameter.\n\nThis input is available only in `__construct` and declares no property or object storage."
                            .to_string(),
                    ));
                }
                _ => {}
            }
        }
    }

    fn collect_promoted_property(
        &mut self,
        class_name: &str,
        parameter: &Param,
        access: MemberAccess,
    ) {
        let mut documentation = Some(promoted_property_documentation(
            class_name, parameter, access,
        ));
        if parameter.name == "message" && self.error_classes.contains(class_name) {
            append_documentation(
                &mut documentation,
                "Required externally accessible readonly message for the compiler-known `Error` contract.",
            );
        }
        let documentation = documentation.expect("promoted property documentation");
        let property_symbol = self.add_declaration_symbol(
            parameter.name_span,
            format!("{} ${}", parameter.ty, parameter.name),
            Some(documentation.clone()),
            SymbolKind::Variable,
        );
        self.class_property_symbols.insert(
            (class_name.to_string(), parameter.name.clone()),
            property_symbol,
        );
        self.record_member_occurrence(
            class_name,
            &parameter.name,
            MemberKind::Property,
            parameter.name_span,
            true,
        );
        if let Some(occurrence) = self
            .member_occurrences
            .last_mut()
            .filter(|occurrence| occurrence.span == parameter.name_span)
        {
            occurrence.exact_declaration = Some(parameter.span);
        }
        self.class_members
            .entry(class_name.to_string())
            .or_default()
            .push(ClassMemberCompletion {
                completion: SemanticCompletion {
                    label: parameter.name.clone(),
                    kind: 10,
                    detail: format!("{} ${}", parameter.ty, parameter.name),
                    documentation: Some(documentation.clone()),
                },
                writable: false,
                internal: access == MemberAccess::Internal,
                is_static: false,
            });
        if let Some(owner) = self.member_owner(class_name, parameter.span, true) {
            self.hierarchy_members.push(HierarchyMember {
                owner,
                name: parameter.name.clone(),
                kind: MemberKind::Property,
                declaration: parameter.span,
                detail: format!("{} ${}", parameter.ty, parameter.name),
                documentation: Some(documentation),
                access,
                is_static: false,
                writable_receiver: false,
                is_open: false,
                is_override: false,
                virtual_root: None,
                overridden_declaration: None,
                override_stub: None,
                body_span: None,
            });
        }
    }

    fn append_callable_source_documentation(
        &self,
        documentation: &mut Option<String>,
        function: &FunctionDecl,
    ) {
        if let Some(info) = self.semantic_info {
            if let Some(signature) = info.callable_signatures.get(&function.span) {
                let returned = callable_return_documentation(
                    signature,
                    info.return_borrows.get(&function.span).copied(),
                );
                if !returned.is_empty() {
                    append_documentation(documentation, returned.trim());
                }
            }
        }
        let Some(plan) = self
            .semantic_info
            .and_then(|info| info.retained_callables.get(&function.span))
        else {
            return;
        };
        for (label, sources) in [
            ("Returned Value", &plan.returns),
            ("Constructed Cursor", &plan.constructs),
        ] {
            for retained in sources {
                let source = match retained.source {
                    doriac::symbols::BorrowSource::Receiver => "the receiver".to_string(),
                    doriac::symbols::BorrowSource::Parameter(index) => function
                        .params
                        .get(index)
                        .map(|parameter| format!("`${}`", parameter.name))
                        .unwrap_or_else(|| "the checked parameter".to_string()),
                };
                let access = if retained.inherited {
                    "the existing source loans carried by"
                } else {
                    "a readonly source loan from"
                };
                append_documentation(
                    documentation,
                    &format!(
                        "**{label}:** Retains {access} {source}. {}",
                        retained_lifetime_documentation(retained.lifetime),
                    ),
                );
            }
        }
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
            parameter_symbols: Vec::new(),
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
            parameter_symbols: Vec::new(),
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
            exact_declaration: None,
            virtual_root: None,
            direct_parent: false,
            relationship_only: false,
        });
    }

    fn record_method_declaration_occurrence(
        &mut self,
        owner_name: &str,
        method: &FunctionDecl,
        name_span: Span,
    ) {
        self.record_member_occurrence(
            owner_name,
            &method.name,
            MemberKind::Method,
            name_span,
            true,
        );
        let hierarchy = self
            .semantic_info
            .and_then(|info| info.method_hierarchy.get(&method.span));
        if let Some(occurrence) = self
            .member_occurrences
            .last_mut()
            .filter(|occurrence| occurrence.span == name_span)
        {
            occurrence.exact_declaration = Some(method.span);
            occurrence.virtual_root = hierarchy.and_then(|method| method.virtual_root);
        }
    }

    fn record_method_call_occurrence(
        &mut self,
        owner_name: &str,
        method_name: &str,
        call_span: Span,
        name_span: Span,
    ) {
        self.record_member_occurrence(
            owner_name,
            method_name,
            MemberKind::Method,
            name_span,
            false,
        );
        let target = self
            .semantic_info
            .and_then(|info| info.method_call_targets.get(&call_span));
        if let Some(occurrence) = self
            .member_occurrences
            .last_mut()
            .filter(|occurrence| occurrence.span == name_span)
        {
            occurrence.exact_declaration = target.map(|target| target.declaration);
            occurrence.virtual_root = target.and_then(|target| target.virtual_root);
            occurrence.direct_parent = target.is_some_and(|target| target.direct_parent);
        }
        if let Some(target) = target.filter(|target| target.direct_parent) {
            let declaration = self.semantic_info.and_then(|info| {
                info.method_hierarchy
                    .get(&target.declaration)
                    .map(|method| format!("{}::{}", method.declaring_class, method.name))
            });
            self.semantic_hovers.push(SemanticHover::new(
                name_span,
                format!(
                    "Direct parent call to `{}`.\n\nThis call bypasses virtual dispatch and invokes the selected parent implementation.",
                    declaration.unwrap_or_else(|| method_name.to_string())
                ),
            ));
        }
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
        self.symbols[symbol].parameter_symbols = vec![None; function.params.len()];
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
        self.pending_named_argument_references
            .extend(args.iter().enumerate().filter_map(|(index, argument)| {
                Some(PendingNamedArgumentReference {
                    callable: symbol,
                    parameter: bound.arg_to_param[index]?,
                    span: argument.name.as_ref()?.span,
                })
            }));
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
            signature: None,
        });
    }

    fn record_byte_call(
        &mut self,
        member_span: Span,
        span: Span,
        args: &[doriac::ast::Argument],
        receiver: &ResolvedType,
        mut member: ByteIntrinsic,
    ) {
        if let Some(result) = self
            .semantic_info
            .and_then(|info| info.expression_type(span))
        {
            if !matches!(result, ResolvedType::Unsupported) {
                member.return_type = result.clone();
            }
        }
        let symbol = self.add_reference_symbol(
            member_span,
            byte_signature(receiver, &member),
            Some(member.documentation.to_string()),
            SymbolKind::Plain,
        );
        self.symbols[symbol].parameter_names = member
            .parameters
            .iter()
            .map(|parameter| parameter.name.to_string())
            .collect();
        self.record_call_signature(span, args, symbol);
    }

    fn record_contract_call(
        &mut self,
        span: Span,
        receiver_span: Span,
        member_span: Span,
        args: &[doriac::ast::Argument],
    ) -> bool {
        let Some(info) = self.semantic_info else {
            return false;
        };
        let (requirements, method_name, declaration) = match info.call_target(span) {
            Some(CallableTarget::InterfaceMethod {
                interface,
                method_name,
                requirement,
            }) => {
                let Some(fact) = info
                    .contracts
                    .interface_specializations
                    .iter()
                    .find(|fact| fact.valid && &fact.specialization == interface)
                else {
                    return false;
                };
                (&fact.requirements, method_name, requirement)
            }
            Some(CallableTarget::ConstrainedMethod {
                method_name,
                requirement,
                ..
            }) => {
                let Some(surface) = info
                    .contracts
                    .constrained_member_surfaces
                    .get(&receiver_span)
                else {
                    return false;
                };
                (&surface.requirements, method_name, requirement)
            }
            _ => return false,
        };
        let Some(requirement) = requirements
            .iter()
            .find(|required| required.accessor.is_none() && &required.name == method_name)
        else {
            return false;
        };
        let bindings = info
            .generic_call_specializations
            .get(&span)
            .map(|specialization| {
                requirement
                    .generic_parameters
                    .iter()
                    .zip(&specialization.arguments)
                    .map(|(parameter, argument)| {
                        let doriac::semantics::GenericArgument::Type(ty) = argument;
                        (parameter.name.clone(), ty.clone())
                    })
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        let signature = interface_requirement_signature(requirement, &bindings);
        let documentation = interface_requirement_documentation(requirement);
        let parameter_names = requirement
            .signature
            .parameters
            .iter()
            .map(|parameter| parameter.name.clone())
            .collect();
        let symbol = self
            .callable_declarations
            .get(declaration)
            .copied()
            .unwrap_or_else(|| {
                let symbol = self.add_metadata_symbol(
                    signature.clone(),
                    Some(documentation.clone()),
                    SymbolKind::Plain,
                );
                self.symbols[symbol].parameter_names = parameter_names;
                symbol
            });
        self.semantic_hovers.push(SemanticHover::new(
            member_span,
            format!("```doria\n{signature}\n```\n\n{documentation}"),
        ));
        self.record_call_signature(span, args, symbol);
        self.call_signatures
            .last_mut()
            .expect("recorded contract call")
            .signature = Some(signature);
        true
    }

    fn record_class_call(
        &mut self,
        span: Span,
        member_span: Option<Span>,
        args: &[doriac::ast::Argument],
    ) -> Option<usize> {
        let info = self.semantic_info?;
        let Some(CallableTarget::Method {
            class_type,
            method_name,
            ..
        }) = info.call_target(span)
        else {
            return None;
        };
        let member = info
            .class_member_surface(class_type)
            .filter(|surface| surface.valid)
            .and_then(|surface| {
                surface
                    .members
                    .iter()
                    .find(|member| &member.name == method_name)
            })?;
        let signature = member.signature.as_ref()?;
        if member_span.is_none()
            && class_type.inline_copy_type().is_some()
            && member.access == MemberAccess::Internal
        {
            return None;
        }
        let bindings = info
            .generic_call_specializations
            .get(&span)
            .map(|specialization| {
                member
                    .generic_parameters
                    .iter()
                    .zip(&specialization.arguments)
                    .map(|(parameter, argument)| {
                        let doriac::semantics::GenericArgument::Type(ty) = argument;
                        (parameter.name.clone(), ty.clone())
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut completion = class_surface_member_completion(
            member,
            &bindings,
            info.property_backing_fields.get(&member.declaration),
            &self.parameter_presentation,
        );
        if let Some(origin) = info.composition.origin(member.declaration) {
            append_documentation(
                &mut completion.documentation,
                &format!(
                    "Composed from `{}` into `{}`.",
                    origin.trait_type, origin.composing_class
                ),
            );
        }
        let parameter_names = signature
            .parameters
            .iter()
            .map(|parameter| parameter.name.clone())
            .collect();
        let symbol = self
            .callable_declarations
            .get(&member.declaration)
            .copied()
            .unwrap_or_else(|| {
                let symbol =
                    self.add_metadata_symbol(completion.detail.clone(), None, SymbolKind::Plain);
                self.symbols[symbol].parameter_names = parameter_names;
                symbol
            });
        let mut documentation = self.symbols[symbol].documentation.clone();
        if let Some(extra) = completion.documentation {
            if !documentation
                .as_ref()
                .is_some_and(|documentation| documentation.contains(&extra))
            {
                append_documentation(&mut documentation, &extra);
            }
        }
        let mut markdown = format!("```doria\n{}\n```", completion.detail);
        if let Some(documentation) = documentation {
            markdown.push_str(&format!("\n\n{documentation}"));
        }
        if let Some(member_span) = member_span {
            if let Some(parent_hover) = self.semantic_hovers.iter_mut().find(|hover| {
                hover.span == member_span
                    && info
                        .method_call_targets
                        .get(&span)
                        .is_some_and(|target| target.direct_parent)
            }) {
                markdown.push_str(&format!("\n\n{}", parent_hover.markdown));
                parent_hover.markdown = markdown;
            } else {
                self.semantic_hovers
                    .push(SemanticHover::new(member_span, markdown));
            }
            self.record_reference(member_span, symbol);
        }
        self.record_call_signature(span, args, symbol);
        self.call_signatures
            .last_mut()
            .expect("recorded class call")
            .signature = Some(completion.detail);
        Some(symbol)
    }

    fn record_class_value_reference(
        &mut self,
        receiver: &ClassType<ResolvedType>,
        name: &str,
        access_span: Span,
        member_span: Span,
    ) -> bool {
        let Some(info) = self.semantic_info else {
            return false;
        };
        if info.expression_type(access_span).is_none() {
            return false;
        }
        let Some(member) = info
            .class_member_surface(receiver)
            .filter(|surface| surface.valid)
            .and_then(|surface| {
                surface
                    .members
                    .iter()
                    .find(|member| member.name == name && member.signature.is_none())
            })
        else {
            return false;
        };
        let kind = if member.kind == CompilerMemberKind::Constant {
            MemberKind::Constant
        } else {
            MemberKind::Property
        };
        let completion = class_surface_member_completion(
            member,
            &HashMap::new(),
            info.property_backing_fields.get(&member.declaration),
            &self.parameter_presentation,
        );
        self.record_member_occurrence(&member.declaring_class.name, name, kind, member_span, false);
        if let Some(occurrence) = self
            .member_occurrences
            .last_mut()
            .filter(|occurrence| occurrence.span == member_span)
        {
            occurrence.exact_declaration = Some(member.declaration);
        }
        let mut markdown = format!("```doria\n{}\n```", completion.detail);
        if let Some(documentation) = completion.documentation {
            markdown.push_str(&format!("\n\n{documentation}"));
        }
        if let Some(origin) = info.composition.origin(member.declaration) {
            markdown.push_str(&format!(
                "\n\nComposed from `{}` into `{}`.",
                origin.trait_type, origin.composing_class
            ));
        }
        if !info.property_accessor_calls.contains_key(&access_span) {
            self.semantic_hovers
                .push(SemanticHover::new(member_span, markdown));
        }
        true
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
                    if class.implements.iter().any(|interface| {
                        interface.condition.is_none() && interface.ty.name == "Error"
                    }) {
                        if let Some(span) = find_identifier_span(self.tokens, class.span, "Error") {
                            self.add_error_type_reference(span);
                        }
                    }
                    for member in &class.members {
                        for method in member_callables(member, PropertyHookContext::Class) {
                            self.visit_function_body(
                                &method,
                                Some(&class.name),
                                class.parent.as_ref().map(AstTypeName::ast_type_name),
                            );
                        }
                        if let ClassMember::Property(property) = member {
                            if let Some(initializer) = &property.initializer {
                                self.visit_expr(
                                    initializer,
                                    Some(&class.name),
                                    class.parent.as_ref().map(AstTypeName::ast_type_name),
                                );
                            }
                        }
                        if let ClassMember::Constant(constant) = member {
                            self.visit_expr(
                                &constant.initializer,
                                Some(&class.name),
                                class.parent.as_ref().map(AstTypeName::ast_type_name),
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
                        for method in member_callables(member, PropertyHookContext::Trait) {
                            if !self.has_composed_callable(method.span) {
                                self.visit_function_body(&method, Some(&trait_decl.name), None);
                            }
                        }
                        if let ClassMember::Property(property) = member {
                            if let Some(initializer) = &property.initializer {
                                self.visit_expr(initializer, Some(&trait_decl.name), None);
                            }
                        }
                    }
                }
                Item::Interface(interface) => {
                    for requirement in &interface.requirements {
                        self.visit_function_body(requirement, Some(&interface.name), None);
                    }
                    for property in &interface.properties {
                        if let Some(facts) = doriac::property_hooks::declaration_facts(
                            property,
                            PropertyHookContext::Interface,
                        ) {
                            for callable in facts.callables() {
                                self.visit_function_body(&callable, Some(&interface.name), None);
                            }
                        }
                    }
                }
                Item::Function(function) => self.visit_function_body(function, None, None),
                Item::Constant(constant) => self.visit_expr(&constant.initializer, None, None),
                Item::Statement(statement) => self.visit_stmt(statement, None, None),
            }
        }
        self.pop_local_scope();
    }

    fn collect_composed_references(&mut self) {
        let Some(info) = self.semantic_info else {
            return;
        };
        let mut classes = info
            .composition
            .origins
            .iter()
            .filter(|origin| origin.authored_declaration.source == self.source_id)
            .map(|origin| origin.composing_class.as_str())
            .collect::<Vec<_>>();
        classes.sort();
        classes.dedup();
        for name in classes {
            let Some(class) = info.composition.class(name) else {
                continue;
            };
            let parent = class.parent.as_ref().map(AstTypeName::ast_type_name);
            for member in &class.members {
                for callable in member_callables(member, PropertyHookContext::Class) {
                    if callable.span.source == self.source_id
                        && callable.span != callable.span.authored()
                    {
                        self.visit_function_body(&callable, Some(name), parent);
                    }
                }
                match member {
                    ClassMember::Property(property)
                        if property.span.source == self.source_id
                            && property.span != property.span.authored() =>
                    {
                        if let Some(initializer) = &property.initializer {
                            self.visit_expr(initializer, Some(name), parent);
                        }
                    }
                    ClassMember::Constant(constant)
                        if constant.span.source == self.source_id
                            && constant.span != constant.span.authored() =>
                    {
                        self.visit_expr(&constant.initializer, Some(name), parent);
                    }
                    _ => {}
                }
            }
        }
    }

    fn visit_function_body(
        &mut self,
        function: &FunctionDecl,
        current_class: Option<&str>,
        parent_class: Option<&str>,
    ) {
        let block = &function.body;
        if let Some(signature) = self
            .semantic_info
            .and_then(|info| info.callable_signatures.get(&function.span))
        {
            for (parameter, checked) in function.params.iter().zip(&signature.parameters) {
                if let Some(span) =
                    find_identifier_span(self.tokens, parameter.type_span, &parameter.ty.name)
                {
                    self.record_applied_enum_hover(span, &checked.r#type);
                }
            }
            if let (Some(ty), Some(span)) =
                (&function.return_type, function.syntax.return_type_span)
            {
                if let Some(span) = find_identifier_span(self.tokens, span, &ty.name) {
                    self.record_applied_enum_hover(span, &signature.return_type);
                }
            }
        }
        self.push_local_scope(block.span().end);
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
        let callable = self.callable_declarations.get(&function.span).copied();
        for (index, parameter) in function.params.iter().enumerate() {
            let parameter_symbol =
                self.declare_parameter(parameter, block.span().start, current_class);
            if let Some(callable) = callable {
                if let Some(slot) = self.symbols[callable].parameter_symbols.get_mut(index) {
                    *slot = Some(parameter_symbol);
                }
            }
        }
        for statement in block.statements() {
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
    ) -> usize {
        let selection_span = find_variable_span(self.tokens, parameter.span, &parameter.name)
            .unwrap_or(parameter.span);
        let mut documentation =
            constructor_parameter_documentation(self.semantic_info, current_class, parameter);
        if parameter.name == "message"
            && parameter.constructor_role.is_promoted()
            && current_class.is_some_and(|class| self.error_classes.contains(class))
        {
            append_documentation(
                &mut documentation,
                "Required externally accessible readonly message for the compiler-known `Error` contract.",
            );
        }
        self.declare_local_binding_with_documentation(
            &parameter.name,
            selection_span,
            parameter_signature(parameter, &self.parameter_presentation),
            visibility_start,
            documentation,
        )
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
    ) -> usize {
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
        symbol
    }

    fn resolve_named_argument_references(&mut self) {
        let references = std::mem::take(&mut self.pending_named_argument_references);
        for reference in references {
            let Some(symbol) = self
                .symbols
                .get(reference.callable)
                .and_then(|callable| callable.parameter_symbols.get(reference.parameter))
                .copied()
                .flatten()
            else {
                continue;
            };
            self.occurrences.push(Occurrence {
                span: reference.span,
                symbol,
                role: OccurrenceRole::NamedArgument,
            });
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
                member_span,
                args,
                span,
                null_safe,
                ..
            } => {
                self.visit_expr(object, current_class, parent_class);
                for argument in args {
                    self.visit_expr(&argument.value, current_class, parent_class);
                }
                let method_span = Some(*member_span);
                self.record_member_receiver(method_span, object, current_class);
                if let (Some(member_span), Some(callee_span)) = (
                    method_span,
                    self.semantic_info
                        .and_then(|info| info.callable_value_calls.get(span))
                        .filter(|call| {
                            call.target_kind == doriac::semantics::CallableValueTargetKind::Property
                        })
                        .map(|call| call.callee_span),
                ) {
                    self.record_property_accessor_reference(callee_span, member_span, method);
                    let receiver = self
                        .semantic_info
                        .and_then(|info| info.expression_type(object.span()))
                        .and_then(member_receiver_class_type)
                        .cloned();
                    if let Some(receiver) = receiver {
                        self.record_class_value_reference(
                            &receiver,
                            method,
                            callee_span,
                            member_span,
                        );
                    }
                    return;
                }
                let byte_method = self.semantic_info.and_then(|info| {
                    let receiver = collection_receiver_type(info.expression_type(object.span())?);
                    let member = byte_intrinsics::member(receiver, method)?;
                    (member.kind == ByteMemberKind::Method).then(|| (receiver.clone(), member))
                });
                if let (Some(method_span), Some((receiver, member))) = (method_span, byte_method) {
                    self.record_byte_call(method_span, *span, args, &receiver, member);
                    return;
                }
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

                if method_span.is_some_and(|method_span| {
                    self.record_contract_call(*span, object.span(), method_span, args)
                }) {
                    return;
                }
                let target = self.semantic_info.and_then(|info| info.call_target(*span));
                let resolved_class = match target {
                    Some(CallableTarget::Method {
                        class_type,
                        method_name,
                        ..
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
                    self.record_method_call_occurrence(class_name, method, *span, method_span);
                }
                if method_span.is_some_and(|member_span| {
                    self.record_class_call(*span, Some(member_span), args)
                        .is_some()
                }) {
                    return;
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
            Expr::CallableCall { callee, args, .. } => {
                self.visit_expr(callee, current_class, parent_class);
                for argument in args {
                    self.visit_expr(&argument.value, current_class, parent_class);
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
                self.record_class_static_receiver(
                    qualifier,
                    *qualifier_span,
                    *member_span,
                    current_class,
                );
                if matches!(qualifier, StaticQualifier::Parent) {
                    self.hierarchy_semantic_tokens.push((
                        *qualifier_span,
                        SEMANTIC_TOKEN_KEYWORD,
                        0,
                    ));
                }
                for argument in args {
                    self.visit_expr(&argument.value, current_class, parent_class);
                }
                if self.record_enum_static_reference(
                    qualifier,
                    *qualifier_span,
                    method,
                    *member_span,
                    Some(*span),
                    Some((*span, args)),
                ) {
                    return;
                }
                if matches!(qualifier, StaticQualifier::Class(class) if class == "String") {
                    if let Some(member) = string_companion_method(method) {
                        self.add_reference_symbol(
                            *member_span,
                            member.signature.to_string(),
                            Some(member.documentation.to_string()),
                            SymbolKind::Plain,
                        );
                        return;
                    }
                }
                if let StaticQualifier::Class(name) = qualifier {
                    if let Some(receiver) = byte_intrinsics::type_from_source_name(name) {
                        if let Some(member) = byte_intrinsics::member(&receiver, method)
                            .filter(|member| member.kind == ByteMemberKind::StaticMethod)
                        {
                            self.record_byte_call(*member_span, *span, args, &receiver, member);
                            return;
                        }
                    }
                }
                let target = self.semantic_info.and_then(|info| info.call_target(*span));
                let class_name = match target {
                    Some(CallableTarget::Method {
                        class_type,
                        method_name,
                        ..
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
                    if let Some(class_name) = class_name {
                        self.record_missing_callable(
                            method,
                            *member_span,
                            args,
                            *span,
                            MissingCallableTarget::Method {
                                class_name,
                                is_static: true,
                            },
                        );
                    }
                }
                if let Some(class_name) = class_name.as_deref() {
                    self.record_method_call_occurrence(class_name, method, *span, *member_span);
                }
                if self
                    .record_class_call(*span, Some(*member_span), args)
                    .is_some()
                {
                    return;
                }
                if let Some(class_name) = class_name.as_deref() {
                    if let Some(symbol) = self.resolve_method(class_name, method) {
                        self.record_reference(*member_span, symbol);
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
                let name_span = find_identifier_span(self.tokens, *span, &class_type.name);
                if let (Some(symbol), Some(name_span)) =
                    (self.classes.get(&class_type.name).copied(), name_span)
                {
                    self.record_reference(name_span, symbol);
                }
                // A constructor declared in this file resolves directly; one
                // from another file of the package comes from the compiler's
                // class member surface. Either way it is the constructor this
                // `new` calls, which the hover on the class name shows.
                let constructor = match self.resolve_method(&class_type.name, "__construct") {
                    Some(symbol) => {
                        self.record_call_signature(*span, args, symbol);
                        Some(symbol)
                    }
                    None => self.record_class_call(*span, None, args),
                };
                if let (Some(name_span), Some(constructor)) = (name_span, constructor) {
                    self.constructor_references.push((name_span, constructor));
                }
            }
            Expr::PropertyAccess {
                object,
                property,
                member_span,
                span,
                ..
            } => {
                self.visit_expr(object, current_class, parent_class);
                let property_span = Some(*member_span);
                if let Some(property_span) = property_span {
                    self.record_property_accessor_reference(*span, property_span, property);
                }
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
                if let (Some(class), Some(property_span)) =
                    (receiver.and_then(member_receiver_class_type), property_span)
                {
                    if self.record_class_value_reference(class, property, *span, property_span) {
                        return;
                    }
                    let class_name = class.name.as_str();
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
                if property == "message" {
                    let payload = receiver
                        .map(non_nullable_type)
                        .map(|receiver| match receiver {
                            ResolvedType::SharedHandle(kind, payload)
                                if *kind == SharedHandleKind::SharedReference
                                    || kind.is_access() =>
                            {
                                payload.as_ref()
                            }
                            receiver => receiver,
                        });
                    if let (Some(info), Some(payload), Some(property_span)) =
                        (self.semantic_info, payload, property_span)
                    {
                        if interface_contract(&info.contracts, payload)
                            .is_some_and(interface_has_message)
                            && info.expression_type(*span).is_some()
                        {
                            self.add_reference_symbol(
                                property_span,
                                "string $message".to_string(),
                                Some(ERROR_MESSAGE_DOCUMENTATION.to_string()),
                                SymbolKind::Variable,
                            );
                            return;
                        }
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
                        return;
                    }
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
                span,
                ..
            } => {
                self.record_property_accessor_reference(*span, *member_span, member);
                self.record_class_static_receiver(
                    qualifier,
                    *qualifier_span,
                    *member_span,
                    current_class,
                );
                if matches!(qualifier, StaticQualifier::Parent) {
                    self.hierarchy_semantic_tokens.push((
                        *qualifier_span,
                        SEMANTIC_TOKEN_KEYWORD,
                        0,
                    ));
                }
                if !self.record_enum_static_reference(
                    qualifier,
                    *qualifier_span,
                    member,
                    *member_span,
                    Some(*span),
                    None,
                ) {
                    if let Some(receiver) = self
                        .semantic_info
                        .and_then(|info| info.static_receiver_types.get(qualifier_span))
                        .cloned()
                    {
                        if self.record_class_value_reference(&receiver, member, *span, *member_span)
                        {
                            return;
                        }
                    }
                    let owner = match qualifier {
                        StaticQualifier::Class(name) => Some(name.as_str()),
                        StaticQualifier::SelfType => current_class,
                        StaticQualifier::Parent => parent_class,
                        StaticQualifier::InvalidStatic => None,
                    };
                    if let Some(owner) = owner {
                        let static_property = self.resolve_static_property(owner, member);
                        let kind = if static_property.is_some() {
                            MemberKind::Property
                        } else {
                            MemberKind::Constant
                        };
                        self.record_member_occurrence(owner, member, kind, *member_span, false);
                        if let Some(symbol) =
                            static_property.or_else(|| self.resolve_constant(owner, member))
                        {
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
                                None,
                                None,
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

    fn collect_compiler_test_hovers(&mut self, info: &SemanticInfo) {
        for reference in info
            .global_symbols
            .references
            .iter()
            .filter(|reference| reference.source_span.source == self.source_id)
        {
            let GlobalSymbolOwner::CompilerKnown(CompilerSymbolIdentity::StandardTest(name)) =
                &reference.symbol_id.owner
            else {
                continue;
            };
            let source_name = reference
                .source_spelling
                .rsplit('\\')
                .next()
                .unwrap_or(&reference.source_spelling);
            let span = find_identifier_span(self.tokens, reference.source_span, source_name)
                .unwrap_or(reference.source_span);
            if let Some(markdown) = compiler_test_member_hover(name) {
                self.semantic_hovers
                    .push(SemanticHover::new(span, markdown.to_string()));
            }
            if reference
                .source_spelling
                .starts_with(doriac::compiler_known_test::NAMESPACE)
                && reference.source_span.start + doriac::compiler_known_test::NAMESPACE.len()
                    <= reference.source_span.end
            {
                self.semantic_hovers.push(SemanticHover::new(
                    Span::in_source(
                        self.source_id,
                        reference.source_span.start,
                        reference.source_span.start + doriac::compiler_known_test::NAMESPACE.len(),
                    )
                    .in_expansion(reference.source_span.expansion),
                    compiler_test_module_hover().to_string(),
                ));
            }
        }

        for assertion in info
            .assertions
            .values()
            .filter(|assertion| assertion.member_span.source == self.source_id)
        {
            let Some(actual) = assertion.actual_type.as_ref() else {
                continue;
            };
            self.semantic_hovers.push(SemanticHover::new(
                assertion.member_span,
                assertion_matcher_documentation(
                    assertion.matcher,
                    actual,
                    assertion.expected_type.as_ref(),
                ),
            ));
            if assertion.negated {
                let negation_span = tokens_in_span(self.tokens, assertion.terminal_span)
                    .find(|token| matches!(token.kind, TokenKind::Not))
                    .map(|token| token.span);
                if let Some(span) = negation_span {
                    self.semantic_hovers.push(SemanticHover::new(
                        span,
                        "`not`\n\nProperty-shaped expectation negation. It evaluates no value, may appear once, and preserves the matcher’s exact type and ownership rules."
                            .to_string(),
                    ));
                }
            }
        }

        let source_context = info.source_semantic_contexts.get(&self.source_id);
        for suite in info
            .test_semantics
            .suites
            .iter()
            .filter(|suite| suite.call_name_span.source == self.source_id)
        {
            self.semantic_hovers.push(SemanticHover::new(
                suite.call_name_span,
                behavioral_test_hover("Suite", &suite.display_name, source_context),
            ));
        }
        for test in info
            .test_semantics
            .tests
            .iter()
            .filter(|test| test.call_name_span.source == self.source_id)
        {
            let kind = match test.origin {
                doriac::testing::TestOrigin::Attribute => "Low-Level Test",
                doriac::testing::TestOrigin::Behavioral => "Behavioral Test",
            };
            self.semantic_hovers.push(SemanticHover::new(
                test.call_name_span,
                behavioral_test_hover(kind, &test.display_name, source_context),
            ));
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

    fn collect_resolved_type_references(&mut self) {
        let Some(info) = self.semantic_info else {
            return;
        };
        let source_id = self.source_id;
        let occupied = self
            .occurrences
            .iter()
            .map(|occurrence| occurrence.span)
            .collect::<HashSet<_>>();
        for reference in info.global_symbols.references.iter().filter(|reference| {
            reference.source_span.source == source_id
                && (class_like_reference_role(reference.role)
                    || reference.role == GlobalReferenceRole::ImportTarget)
                && !occupied.contains(&reference.source_span)
        }) {
            let name = &reference.symbol_id.qualified_name;
            if let Some(symbol) = self
                .enums
                .get(name)
                .or_else(|| self.classes.get(name))
                .or_else(|| self.contract_types.get(name))
                .copied()
            {
                self.record_reference(reference.source_span, symbol);
            }
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
            .or_else(|| self.contract_types.get(&ty.name))
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
                "Compiler-known checked-error contract. Conforming classes explicitly implement Error or an Error subinterface and expose an externally accessible readonly stored `string $message` property."
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

    fn resolve_static_property(&self, class_name: &str, property: &str) -> Option<usize> {
        let mut current = Some(class_name);
        let mut visited = HashSet::new();
        while let Some(class_name) = current {
            if !visited.insert(class_name) {
                break;
            }
            if let Some(symbol) = self
                .class_static_property_symbols
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
        expression_span: Option<Span>,
        call: Option<(Span, &[doriac::ast::Argument])>,
    ) -> bool {
        let StaticQualifier::Class(enum_name) = qualifier else {
            return false;
        };
        let enum_name = self
            .semantic_info
            .and_then(|info| {
                info.global_symbols.references.iter().find(|reference| {
                    reference.source_span == qualifier_span
                        && class_like_reference_role(reference.role)
                })
            })
            .map_or_else(
                || enum_name.clone(),
                |reference| reference.symbol_id.qualified_name.clone(),
            );
        let Some(enum_symbol) = self.enums.get(&enum_name).copied() else {
            return false;
        };
        self.record_reference(qualifier_span, enum_symbol);
        if let Some(ty) = expression_span.and_then(|span| self.semantic_info?.expression_type(span))
        {
            self.record_applied_enum_hover(qualifier_span, ty);
        }
        self.static_receivers.push(StaticReceiver {
            span: member_span,
            receiver_span: qualifier_span,
            receiver: StaticReceiverKind::Enum(enum_name.clone()),
            current_class: None,
        });
        if let Some(case_symbol) = self
            .enum_cases
            .get(&(enum_name.clone(), member.to_string()))
            .copied()
        {
            self.record_reference(member_span, case_symbol);
            if let Some((span, args)) = call {
                self.record_call_signature(span, args, case_symbol);
            }
        }
        self.record_member_occurrence(&enum_name, member, MemberKind::EnumCase, member_span, false);
        true
    }

    fn record_applied_enum_hover(&mut self, span: Span, ty: &ResolvedType) {
        let ResolvedType::Enum(enum_type) = non_nullable_type(ty) else {
            return;
        };
        if enum_type.arguments.is_empty() {
            return;
        }
        let applied = display_resolved_type(ty);
        let documentation = enum_documentation(&enum_type.name, self.enum_copy(enum_type));
        self.semantic_hovers.push(SemanticHover::new(
            span,
            format!("```doria\nenum {applied}\n```\n\n{documentation}"),
        ));
    }

    fn record_class_static_receiver(
        &mut self,
        qualifier: &StaticQualifier,
        qualifier_span: Span,
        member_span: Span,
        current_class: Option<&str>,
    ) {
        if let StaticQualifier::Class(name) = qualifier {
            if let Some(receiver) = byte_intrinsics::type_from_source_name(name) {
                self.static_receivers.push(StaticReceiver {
                    span: member_span,
                    receiver_span: qualifier_span,
                    receiver: StaticReceiverKind::Byte(receiver),
                    current_class: None,
                });
                return;
            }
        }
        if let Some(receiver) = self
            .semantic_info
            .and_then(|info| info.static_receiver_types.get(&qualifier_span))
        {
            let current_class = current_class
                .and_then(|name| self.member_owner(name, member_span, true))
                .map(|owner| owner.qualified_name);
            self.static_receivers.push(StaticReceiver {
                span: member_span,
                receiver_span: qualifier_span,
                receiver: StaticReceiverKind::Class {
                    receiver: receiver.clone(),
                    parent: matches!(qualifier, StaticQualifier::Parent),
                },
                current_class,
            });
        }
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
        let Some(info) = self.semantic_info else {
            return;
        };
        let Some(receiver) = info.expression_type(object.span()).cloned() else {
            return;
        };
        self.member_receivers.push(MemberReceiver {
            span,
            receiver_span: object.span(),
            receiver,
            current_class: current_class
                .and_then(|name| self.member_owner(name, span, true))
                .map(|owner| owner.qualified_name),
            writable_payload_access: info.member_receiver_access(object.span())
                == Some(ObjectPathAccess::Writable),
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

struct CompilerKnownMethodHover {
    signature: String,
    documentation: String,
}

/// The compiler rejects a direct call to a constructor or destructor
/// (E0414): an object is built with `new` and destroyed by the runtime, so
/// neither is offered after `->`.
fn is_lifecycle_method(name: &str) -> bool {
    matches!(name, "__construct" | "__destruct")
}

fn interface_member_completions(
    facts: &ContractFacts,
    receiver: &ResolvedType,
    receiver_span: Span,
    writable: bool,
) -> Option<Vec<SemanticCompletion>> {
    let (requirements, has_error_message) = match receiver {
        ResolvedType::TypeParameter(_) => {
            let surface = facts.constrained_member_surfaces.get(&receiver_span)?;
            (&surface.requirements, surface.has_error_message)
        }
        ResolvedType::Interface(_) | ResolvedType::Error => {
            let Some(fact) = interface_contract(facts, receiver) else {
                return Some(Vec::new());
            };
            (&fact.requirements, interface_has_message(fact))
        }
        _ => return None,
    };
    let mut completions = requirements
        .iter()
        .filter(|requirement| {
            requirement.accessor != Some(doriac::ast::PropertyHookKind::Set)
                || !requirements.iter().any(|other| {
                    other.name == requirement.name
                        && other.accessor == Some(doriac::ast::PropertyHookKind::Get)
                })
        })
        .filter(|requirement| writable || requirement.receiver_mode != ReceiverMode::Writable)
        .map(|requirement| SemanticCompletion {
            label: requirement.name.clone(),
            kind: if requirement.accessor.is_some() {
                10
            } else {
                2
            },
            detail: interface_requirement_signature(requirement, &HashMap::new()),
            documentation: Some(interface_requirement_documentation(requirement)),
        })
        .collect::<Vec<_>>();
    if has_error_message {
        completions.push(SemanticCompletion {
            label: "message".to_string(),
            kind: 10,
            detail: "string $message".to_string(),
            documentation: Some(ERROR_MESSAGE_DOCUMENTATION.to_string()),
        });
    }
    Some(completions)
}

const ERROR_MESSAGE_DOCUMENTATION: &str =
    "Externally accessible readonly stored message required by Error and its subinterfaces.";

fn interface_contract<'a>(
    facts: &'a ContractFacts,
    receiver: &ResolvedType,
) -> Option<&'a InterfaceSpecializationFacts> {
    facts.interface_specializations.iter().find(|fact| {
        fact.valid
            && match receiver {
                ResolvedType::Interface(interface) => fact.specialization == *interface,
                ResolvedType::Error => fact.specialization.name == "Error",
                _ => false,
            }
    })
}

fn interface_has_message(fact: &InterfaceSpecializationFacts) -> bool {
    fact.specialization.name == "Error"
        || fact
            .ancestors
            .iter()
            .any(|ancestor| ancestor.name == "Error")
}

fn interface_requirement_signature(
    requirement: &RequirementFacts,
    bindings: &HashMap<String, ResolvedType>,
) -> String {
    if let Some(kind) = requirement.accessor {
        return property_hooks::accessor_signature(
            &requirement.name,
            kind,
            &requirement.signature,
            Some(requirement.receiver_mode),
            &requirement.checked_effects,
            bindings,
        );
    }
    format!(
        "{}{}",
        match requirement.receiver_mode {
            ReceiverMode::Readonly => "",
            ReceiverMode::Writable => "writable ",
            ReceiverMode::Consuming => "consuming ",
        },
        semantic_callable_signature(
            &requirement.name,
            &requirement.generic_parameters,
            &requirement.signature,
            &requirement.checked_effects,
            bindings,
            &ParameterPresentation::default(),
        )
    )
}

/// How the compiler says each authored parameter reads, keyed by the
/// parameter's declaration span: its default as written, and for a
/// constructor its role. Signatures render parameters from this so that a
/// declaration in an unopened file reads the same as one in the open file.
#[derive(Debug, Clone, Default)]
pub(crate) struct ParameterPresentation {
    defaults: HashMap<Span, String>,
    roles: HashMap<Span, ConstructorParameterSemanticRole>,
}

impl ParameterPresentation {
    fn from_semantic_info(info: &SemanticInfo) -> Self {
        let defaults = info
            .callable_signatures
            .values()
            .flat_map(|signature| &signature.parameters)
            .chain(
                info.class_member_surfaces
                    .iter()
                    .flat_map(|surface| &surface.members)
                    .filter_map(|member| member.signature.as_ref())
                    .flat_map(|signature| &signature.parameters),
            )
            .filter_map(|parameter| Some((parameter.declaration?, parameter.default.clone()?)))
            .collect();
        let roles = info
            .constructor_parameters
            .iter()
            .map(|(span, parameter)| (*span, parameter.role.clone()))
            .collect();
        Self { defaults, roles }
    }

    fn default_of(&self, declaration: Span) -> Option<&str> {
        self.defaults.get(&declaration).map(String::as_str)
    }

    /// The constructor role keyword a parameter is declared with, if any.
    fn role_keyword(&self, declaration: Span) -> Option<&'static str> {
        match self.roles.get(&declaration)? {
            ConstructorParameterSemanticRole::Promoted {
                access: MemberAccess::Internal,
            } => Some("internal"),
            ConstructorParameterSemanticRole::Promoted {
                access: MemberAccess::External,
            } => None,
            ConstructorParameterSemanticRole::InheritedPropertyOverride { .. } => Some("override"),
            ConstructorParameterSemanticRole::ConstructorOnly => Some("parameter"),
        }
    }
}

fn semantic_callable_signature(
    name: &str,
    generic_parameters: &[doriac::ast::TypeParamDecl],
    signature: &CallableSignatureSemanticInfo,
    checked_effects: &[ResolvedType],
    bindings: &HashMap<String, ResolvedType>,
    presentation: &ParameterPresentation,
) -> String {
    let display = |ty: &ResolvedType| {
        display_resolved_type(&doriac::types::substitute_resolved_type(ty, bindings))
    };
    let parameters = signature
        .parameters
        .iter()
        .map(|parameter| {
            // The same order the declaration is written in, and the same
            // order the open-file signature renders.
            let mut parts = Vec::new();
            if parameter.borrow {
                parts.push("borrow".to_string());
            }
            if let Some(role) = parameter
                .declaration
                .and_then(|declaration| presentation.role_keyword(declaration))
            {
                parts.push(role.to_string());
            }
            if parameter.take {
                parts.push("take".to_string());
            }
            if parameter.writable {
                parts.push("writable".to_string());
            }
            parts.push(display(&parameter.r#type));
            parts.push(format!("${}", parameter.name));
            let mut rendered = parts.join(" ");
            if let Some(default) = &parameter.default {
                rendered.push_str(" = ");
                rendered.push_str(default);
            } else if parameter.has_default {
                rendered.push_str(" = ...");
            }
            rendered
        })
        .collect::<Vec<_>>()
        .join(", ");
    let effects = if checked_effects.is_empty() {
        String::new()
    } else {
        format!(
            " throws {}",
            checked_effects
                .iter()
                .map(display)
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    // A constructor is declared without a return type and always returns void,
    // so render it the way it is declared; the authored declaration hover shows
    // the same signature.
    let constructor = name == "__construct" || name.ends_with("::__construct");
    let return_type = if constructor && matches!(signature.return_type, ResolvedType::Void) {
        String::new()
    } else {
        format!(": {}", display(&signature.return_type))
    };
    format!(
        "function {name}{}({parameters}){return_type}{effects}",
        type_parameter_signature(generic_parameters),
    )
}

fn class_surface_member_completion(
    member: &ClassMemberSurfaceEntry,
    bindings: &HashMap<String, ResolvedType>,
    backing_field: Option<&doriac::property_hooks::PropertyBackingField>,
    presentation: &ParameterPresentation,
) -> SemanticCompletion {
    let mut modifiers = Vec::new();
    if member.access == MemberAccess::Internal {
        modifiers.push("internal");
    }
    if member.is_open {
        modifiers.push("open");
    }
    if member.is_override {
        modifiers.push("override");
    }
    if matches!(
        member.kind,
        CompilerMemberKind::StaticMethod | CompilerMemberKind::StaticProperty
    ) && member.hooks.is_none()
    {
        modifiers.push("static");
    }
    if member.receiver_mode == Some(ReceiverMode::Consuming) {
        modifiers.push("consuming");
    } else if member.writable {
        modifiers.push("writable");
    }
    let prefix = if modifiers.is_empty() {
        String::new()
    } else {
        format!("{} ", modifiers.join(" "))
    };
    let (kind, signature) = if let Some(signature) = &member.signature {
        let owner = display_resolved_type(&ResolvedType::Class(member.declaring_class.clone()));
        (
            2,
            semantic_callable_signature(
                &format!("{owner}::{}", member.name),
                &member.generic_parameters,
                signature,
                &member.checked_effects,
                bindings,
                presentation,
            ),
        )
    } else if let Some((kind, accessor)) = member.hooks.as_ref().and_then(|hooks| {
        hooks
            .getter
            .as_ref()
            .map(|getter| (doriac::ast::PropertyHookKind::Get, getter))
            .or_else(|| {
                hooks
                    .setter
                    .as_ref()
                    .map(|setter| (doriac::ast::PropertyHookKind::Set, setter))
            })
    }) {
        (
            10,
            property_hooks::accessor_signature(
                &member.name,
                kind,
                &accessor.signature,
                accessor.receiver_mode,
                &accessor.checked_effects,
                bindings,
            ),
        )
    } else {
        let ty = member
            .ty
            .as_ref()
            .map(display_resolved_type)
            .unwrap_or_default();
        if member.kind == CompilerMemberKind::Constant {
            (21, format!("const {ty} {}", member.name))
        } else {
            (10, format!("{ty} ${}", member.name))
        }
    };
    let mut documentation = callable_details_documentation(
        member.signature.as_ref(),
        member.return_borrow,
        &member.automatic_effects,
    );
    if member.hooks.is_none()
        && matches!(
            member.kind,
            CompilerMemberKind::InstanceProperty
                | CompilerMemberKind::PromotedProperty
                | CompilerMemberKind::StaticProperty
        )
    {
        append_documentation(
            &mut documentation,
            if member.writable {
                "This stored property is writable."
            } else {
                "This stored property is readonly."
            },
        );
    }
    if let Some(hooks) = &member.hooks {
        append_documentation(
            &mut documentation,
            &property_hooks::surface_documentation(&member.name, hooks, bindings, backing_field),
        );
    }
    SemanticCompletion {
        label: member.name.clone(),
        kind,
        detail: format!("{prefix}{signature}"),
        documentation,
    }
}

fn callable_details_documentation(
    signature: Option<&CallableSignatureSemanticInfo>,
    return_borrow: Option<doriac::symbols::ReturnBorrow>,
    automatic_effects: &[ResolvedType],
) -> Option<String> {
    let mut documentation = signature
        .map(|signature| {
            let mut documentation = callable_return_documentation(signature, return_borrow);
            let defaults = signature
                .parameters
                .iter()
                .filter(|parameter| parameter.has_default)
                .map(|parameter| format!("`${}`", parameter.name))
                .collect::<Vec<_>>();
            if !defaults.is_empty() {
                documentation.push_str(&format!(
                    "\n\nDefault arguments available for {}.",
                    defaults.join(", ")
                ));
            }
            documentation.trim().to_string()
        })
        .filter(|documentation| !documentation.is_empty());
    let profile = doriac::CheckedEffectProfile::classify(automatic_effects.iter().cloned());
    for effects in [
        ambient_effect_documentation(&profile.ambient),
        test_assertion_effect_documentation(&profile.test_assertion),
    ] {
        if !effects.is_empty() {
            append_documentation(&mut documentation, effects.trim());
        }
    }
    documentation
}

fn interface_requirement_documentation(requirement: &RequirementFacts) -> String {
    let mut documentation = "Declared requirement. Erased calls use this contract, not implementation-only defaults. Checked dispatch preserves automatic I/O and TestAssertion transport without adding authored throws.".to_string();
    documentation.push_str(&callable_return_documentation(
        &requirement.signature,
        requirement.return_borrow,
    ));
    documentation
}

const PENDING_REQUEST_DOCUMENTATION: &str = "**Lifetime:** Call-Scoped Request View\n\nBorrowed group-slot view. It cannot be constructed, consumed, retained, or escape its preparation/completion call.";

pub(crate) const INLINE_COPY_DOCUMENTATION: &str = "**Representation:** Inline Copy Value\n\nAssignment copies the value; the original remains usable. Its nullable form is also Copy. Copy capability does not imply absence of retained ownership or owned cleanup. Its internal representation and constructors are not public APIs. This does not make ordinary user-defined classes Copy.";

const NON_OWNING_INLINE_DOCUMENTATION: &str =
    "This value carries no heap ownership; copying it allocates nothing.";

pub(crate) const STANDARD_STREAM_DOCUMENTATION: &str = "**Ownership:** Process-Owned Standard Device\n\nStandardStreams exposes readonly static input, output, and error properties returning StandardInput, StandardOutput, and StandardError. These non-owning, nonclosable inline Copy views share the intrinsic device owner, redirection, injection, and access/mode coordination. Access and copying allocate no native handle, control block, buffer, or heap object. Copies grant no independent exclusive access; mode changes require exclusive control and a restoring guard.\n\n**Interface Borrowing:** Copy-to-interface conversion creates an independent owning erased snapshot. A returned borrow stored beyond the call statement requires a named interface owner. The original Copy binding remains independently usable and assignable. Nullable erasure follows the same compiler-owned lifetime rules.\n\n**Terminal:** Each view has readonly `isInteractive: bool`, true when that standard stream is attached to a terminal. It never throws and answers while a pending transfer holds the device.";

const CANCELLATION_TOKEN_DOCUMENTATION: &str = "**Ownership:** Shared Cancellation State\n\nEach independently owned token copy retains one shared state claim and releases it during ordinary cleanup. Borrowed arguments acquire no ownership claim. Token creation and copying allocate no second state or native signal. Tokens may outlive their source. Copying or dropping a token, and destroying its CancellationSource, do not request cancellation.\n\n**Equality:** `==` compares shared cancellation-state identity; `!=` is its inverse. Tokens and copies from the same source compare equal; distinct live states differ. Cancellation and source destruction do not change identity while tokens survive. Ordinary nullable equality applies. Borrowed comparison acquires no ownership claim and performs no signal or clock work. This grants no ordering, hashing, integer conversion, or address API.";

pub(crate) const DURATION_NUMERIC_DOCUMENTATION: &str = "**Numeric Domain:** Signed 64-bit (`int64`, canonically `int`) nanoseconds. Negative intervals are valid. Duration remains a distinct nominal value, not an integer. No implicit integer conversions.";

pub(crate) const DEADLINE_DOCUMENTATION: &str = "**Expiry:** Preserves the exact instant, including past instants. Distinct past deadlines remain distinguishable; expiry does not replace them with a shared expired marker. When used to bound a completion-observation wait, expiry stops only that wait, not the underlying transfer.";

pub(crate) const REGISTRATION_KEY_DOCUMENTATION: &str = "**Ownership:** Non-owning, type-preserving registration key. Copying a key does not copy or borrow the registered stream; the readiness set owns that stream. Its type argument remains part of its identity. Unregistering returns the original stream type.";

pub(crate) const READ_TRANSFER_KEY_DOCUMENTATION: &str = "**Identity:** Opaque, non-owning identifier for a read transfer, not a registered stream. Distinct from WriteTransferKey, RegistrationKey<T>, and integers; no integer conversions or public constructor. Copying it does not copy, borrow, or own a stream or buffer. Retrieved by TransferPlan's getReadKey(int $position), using zero-based positions across the entire mixed plan; an invalid or wrong-direction position throws IoError. Operation identity remains stable through consuming appends. The approved TransferGroup contract also exposes it through readonly observedReadKey: ?ReadTransferKey, without retaining group, storage, or source loans. Tooling validation and pinned-compiler integration remain pending.";

pub(crate) const WRITE_TRANSFER_KEY_DOCUMENTATION: &str = "**Identity:** Opaque, non-owning identifier for a write transfer, not a registered stream. Distinct from ReadTransferKey, RegistrationKey<T>, and integers; no integer conversions or public constructor. Copying it does not copy, borrow, or own a stream or buffer. Retrieved by TransferPlan's getWriteKey(int $position), using zero-based positions across the entire mixed plan; an invalid or wrong-direction position throws IoError. Operation identity remains stable through consuming appends. The approved TransferGroup contract also exposes it through readonly observedWriteKey: ?WriteTransferKey, without retaining group, storage, or source loans. Tooling validation and pinned-compiler integration remain pending.";

pub(crate) const WATCH_KEY_DOCUMENTATION: &str = "**Identity:** Opaque, non-owning identifier for one watch in a transfer plan, not the watched source. Distinct from ReadTransferKey, WriteTransferKey, RegistrationKey<T>, and integers; no integer conversions or public constructor. Copying it does not borrow the source or start a watch. Retrieved by TransferPlan's getWatchKey(int $position), using zero-based positions across the entire mixed plan; a position that is not a watch throws IoError. Carried by TransferCompletion::Ready and exposed through TransferGroup's readonly observedWatchKey: ?WatchKey. A watch reports once; resubmitWatch(WatchKey $key) arms it again after its Ready has been fetched.";

pub(crate) const WATCH_SOURCE_DOCUMENTATION: &str = "**Role:** Opaque value by which a Watchable source tells the runtime what to watch. It has no public constructor or member, and a program has no use for it: pass the source itself to TransferPlan::watch or andWatch. Copying it does not borrow the source or start a watch. Only the standard library implements Watchable; E0781 rejects a program's own implementation.";

pub(crate) const TERMINAL_SIZE_DOCUMENTATION: &str = "**Terminal Size:** The columns and rows of the terminal window, read through readonly `columns` and `rows`. Returned by Console::getSize() and carried by TerminalEvent::Resized.";

pub(crate) const TERMINAL_POSITION_DOCUMENTATION: &str = "**Terminal Position:** One cell of the terminal, column first and zero-based, with (0, 0) at the top left; read through readonly `column` and `row`. Returned by RawMode::getCursorPosition().";

pub(crate) const KEY_MODIFIERS_DOCUMENTATION: &str = "**Key Modifiers:** Which of Control, Alt, and Shift were held with a key, read through readonly `control`, `alt`, and `shift`. Carried by TerminalEvent::Key.";

pub(crate) const TERMINAL_STYLE_DOCUMENTATION: &str = "**Style:** How one Console write looks. Start from Style::plain and build with withForeground, withBackground, withBold, withDim, withItalic, withUnderline, withReverse, and withStrikethrough; each returns a changed copy. A style belongs to one write and leaves nothing set on the terminal. A color the terminal cannot show is drawn as the nearest it can, and redirected output receives the text alone.";

pub(crate) const TRANSFER_KEY_LOOKUP_FAILURE_DOCUMENTATION: &str = "**Lookup Failure:** Negative, out-of-range, or wrong-direction positions throw `Doria\\Std\\Io\\IoError` with `operation = IoOperation::Access`, `target = IoTarget::TransferPlan`, and `reason = IoErrorReason::InvalidInput`. The TransferPlan target has no payload. `systemCode` and `transferredByteCount` are `null`.";

pub(crate) const TRANSFER_GROUP_CONTROL_DOCUMENTATION: &str = "**Approved Group Lifetime Controls:** ReadinessSet::begin accepts optional named deadline: ?Doria\\Std\\Time\\Deadline and cancellation: ?Doria\\Std\\Concurrency\\CancellationToken parameters, both defaulting to null. Null omits that lifetime control; there is no TransferControl type. Controls persist for the group, distinct from getNextCompletion's wait-only budget. The group retains shared cancellation-token state, not a CancellationSource loan. Cancellation or group-deadline expiry drains outstanding transfers before storage release. Tooling validation and pinned-compiler integration remain pending.";

const FILE_POSITION_DOCUMENTATION: &str = "**Approved File Position Contract:** ReadableFile, WritableFile, and DuplexFile implement Doria\\Std\\Io\\SeekableStream. Writable seek(int $offset, SeekOrigin $origin): int returns the new position with checked IoError; origin is required and has Start, Current, and End cases. Owned files expose readonly getPosition(): int throws IoError and getLength(): int throws IoError methods, not position/length properties. Fresh inspection can block and remains forbidden in property hooks, including through helpers, under the unchanged nonblocking hook rule. Seek failures use IoOperation::Seek, appended after Access without changing earlier tags; inspection failures use Access. Both retain the opening-path IoTarget::File, null transferredByteCount, and an optional systemCode. Append writes still write at the then-current EOF despite seeking. Tooling validation and pinned-compiler integration remain pending.";

pub(crate) fn file_position_documentation(name: &str) -> Option<&'static str> {
    matches!(
        name,
        doriac::compiler_known_stream::READABLE_FILE
            | doriac::compiler_known_stream::WRITABLE_FILE
            | doriac::compiler_known_stream::DUPLEX_FILE
            | "Doria\\Std\\Io\\SeekableStream"
    )
    .then_some(FILE_POSITION_DOCUMENTATION)
}

const WRITABLE_FILE_DOCUMENTATION: &str = "**Approved File Synchronization Contract:** WritableFile and DuplexFile expose writable synchronizeData(): void throws IoError and synchronizeMetadata(): void throws IoError. Data synchronization synchronizes file data; metadata synchronization synchronizes full data plus metadata. ReadableFile exposes neither method. No synchronization interface or abbreviated alias is introduced. Failures use distinct IoOperation::SynchronizeData and SynchronizeMetadata cases appended after Seek, preserving earlier identities, with the original opening-path IoTarget::File, optional systemCode, and null transferredByteCount. Tooling validation and pinned-compiler integration remain pending.\n\n**Approved File Resize Contract:** WritableFile and DuplexFile expose writable setLength(int $length): void throws IoError, not a truncate alias or interface expansion. The required byte length is nonnegative. Shrinking discards the tail; supported growth exposes zero-readable extension. Resizing leaves the cursor unchanged and does not consume the owner. ReadableFile has no setLength method. Filesystem/platform failures use IoOperation::Resize appended after SynchronizeMetadata, preserving earlier tags, with the opening-path IoTarget::File, optional systemCode, and null transferredByteCount. Tooling validation and pinned-compiler integration remain pending.";

pub(crate) fn writable_file_documentation(name: &str) -> Option<&'static str> {
    matches!(
        name,
        doriac::compiler_known_stream::WRITABLE_FILE | doriac::compiler_known_stream::DUPLEX_FILE
    )
    .then_some(WRITABLE_FILE_DOCUMENTATION)
}

pub(crate) const TRANSFER_GROUP_RESUBMISSION_FAILURE_DOCUMENTATION: &str = "**Approved Resubmission Contract:** Invalid keys or ranges and active or unobserved operations passed to resubmitRead/resubmitWrite throw existing `Doria\\Std\\Io\\IoError` with `operation = IoOperation::Configure`, `target = IoTarget::TransferGroup`, and `reason = IoErrorReason::InvalidInput`. The TransferGroup target has no payload; `systemCode` and `transferredByteCount` are `null`. Failed validation submits no work and consumes no stored transfer Error. Valid reuse preserves the original directional key and retained buffer. Actual transfer failures propagate unchanged through getNextCompletion. Tooling validation and pinned-compiler integration remain pending.";

pub(crate) fn inline_value_documentation(ty: doriac::inline_value::InlineValueKind) -> String {
    let mut documentation = INLINE_COPY_DOCUMENTATION.to_string();
    if !ty.requires_owned_cleanup() {
        documentation.push_str("\n\n");
        documentation.push_str(NON_OWNING_INLINE_DOCUMENTATION);
    }
    documentation.push_str("\n\n");
    documentation.push_str(match ty {
        doriac::inline_value::InlineValueKind::Duration => DURATION_NUMERIC_DOCUMENTATION,
        doriac::inline_value::InlineValueKind::Deadline => DEADLINE_DOCUMENTATION,
        doriac::inline_value::InlineValueKind::RegistrationKey => REGISTRATION_KEY_DOCUMENTATION,
        doriac::inline_value::InlineValueKind::ReadTransferKey => READ_TRANSFER_KEY_DOCUMENTATION,
        doriac::inline_value::InlineValueKind::WriteTransferKey => WRITE_TRANSFER_KEY_DOCUMENTATION,
        doriac::inline_value::InlineValueKind::WatchKey => WATCH_KEY_DOCUMENTATION,
        doriac::inline_value::InlineValueKind::WatchSource => WATCH_SOURCE_DOCUMENTATION,
        doriac::inline_value::InlineValueKind::CancellationToken => {
            CANCELLATION_TOKEN_DOCUMENTATION
        }
        doriac::inline_value::InlineValueKind::Terminal(value) => match value {
            doriac::inline_value::TerminalValue::Size => TERMINAL_SIZE_DOCUMENTATION,
            doriac::inline_value::TerminalValue::Position => TERMINAL_POSITION_DOCUMENTATION,
            doriac::inline_value::TerminalValue::KeyModifiers => KEY_MODIFIERS_DOCUMENTATION,
            doriac::inline_value::TerminalValue::Style => TERMINAL_STYLE_DOCUMENTATION,
        },
        doriac::inline_value::InlineValueKind::StandardInput
        | doriac::inline_value::InlineValueKind::StandardOutput
        | doriac::inline_value::InlineValueKind::StandardError => STANDARD_STREAM_DOCUMENTATION,
    });
    if matches!(
        ty,
        doriac::inline_value::InlineValueKind::ReadTransferKey
            | doriac::inline_value::InlineValueKind::WriteTransferKey
    ) {
        documentation.push_str("\n\n");
        documentation.push_str(TRANSFER_KEY_LOOKUP_FAILURE_DOCUMENTATION);
        documentation.push_str("\n\n");
        documentation.push_str(TRANSFER_GROUP_RESUBMISSION_FAILURE_DOCUMENTATION);
    }
    if ty == doriac::inline_value::InlineValueKind::WatchKey {
        documentation.push_str("\n\n");
        documentation.push_str(TRANSFER_KEY_LOOKUP_FAILURE_DOCUMENTATION);
    }
    documentation
}

fn inline_copy_documentation(ty: &ResolvedType) -> Option<String> {
    match non_nullable_type(ty) {
        ResolvedType::Class(class) => class.inline_copy_type().map(inline_value_documentation),
        _ => None,
    }
}

pub(crate) fn pending_request_documentation(name: &str) -> Option<&'static str> {
    doriac::compiler_known_stream::is_pending_request(name).then_some(PENDING_REQUEST_DOCUMENTATION)
}

pub(crate) fn transfer_result_documentation(name: &str) -> Option<&'static str> {
    match name {
        doriac::compiler_known_stream::TRANSFER_COMPLETION => Some(
            "Identifies a retired read or write with its matching typed transfer key and nullable outcome, or with Ready a watch whose source has something the program can take without waiting. Ready carries only its WatchKey; the program asks the source, then calls resubmitWatch to watch again. A null outcome means no transfer outcome on that completed operation, not absence of a completion. Zero-byte success, WouldBlock, EndOfStream, and a read's TimedOut remain actual applicable outcomes. Construction and copying do not retire work, release loans, or grant buffer access.",
        ),
        doriac::compiler_known_stream::TRANSFER_WAIT_RESULT => Some(
            "Completed carries a TransferCompletion, distinct from the wait observations TimedOut and Exhausted. Neither observation permanently stops the group or prevents later resubmission. Transfer failures throw the original owned checked Error rather than returning an error case. If wait infrastructure fails alongside a preserved transfer completion or owned transfer error, the infrastructure error is reported first. The original completion or owned error remains available for a subsequent fetch; it is not discarded, relabeled, or reconstructed. The approved reusable owner is Doria\\Std\\Io\\TransferGroup, with readonly observedReadKey: ?ReadTransferKey and readonly observedWriteKey: ?WriteTransferKey. Only the fetched operation's direction is present; both are null before the first fetch and after timeout or exhaustion. The observed key preserves the consumed plan's original opaque stamp and mixed-plan operation position, published before an original owned checked Error propagates and preserved across resubmission. Key copies retain no group, storage, or source loans or resources. Observed keys do not replace successful completion payload keys. A retired read's known completed prefix may be available through getReadRegion before fetching; observed keys are published only by fetching. Fetching and cleanup do not repeat retired adapter callbacks. An original queued IoError remains owned until fetched or group destruction. Group setup through begin consumes the plan on success and failure. Its own plan-validation and native-setup IoError use IoOperation::Configure / IoTarget::TransferGroup, preserving reason and system-code details. An adapter's original owned checked Error propagates unchanged, not relabeled or reconstructed. Failed setup releases plan loans, leaves no active group, and submits no work. Construction and copying do not retire work, release loans, or grant buffer access. The compiler declares writable getNextCompletion(Wait $wait): TransferWaitResult, with ambient IoError transport. Tooling validation and pinned-compiler integration remain pending.",
        ),
        _ => None,
    }
}

fn callable_return_documentation(
    signature: &CallableSignatureSemanticInfo,
    borrow: Option<doriac::symbols::ReturnBorrow>,
) -> String {
    let mut documentation = String::new();
    if let ResolvedType::Enum(outcome) = &signature.return_type {
        if let Some(result) = flush_outcome_documentation(&outcome.name) {
            documentation.push_str("\n\n");
            documentation.push_str(result);
        }
    }
    if let Some(borrow) = borrow {
        let source = match borrow.source {
            doriac::symbols::BorrowSource::Receiver => "the receiver".to_string(),
            doriac::symbols::BorrowSource::Parameter(index) => signature
                .parameters
                .get(index)
                .map(|parameter| format!("`${}`", parameter.name))
                .unwrap_or_else(|| "the declared parameter".to_string()),
        };
        let access = if borrow.writable {
            "writable"
        } else {
            "readonly"
        };
        match borrow.kind {
            doriac::types::ReturnBorrowKind::Value => documentation.push_str(&format!(
                "\n\nReturns a {access} borrow rooted in {source}.",
            )),
            doriac::types::ReturnBorrowKind::Retained => documentation.push_str(&format!(
                "\n\nReturns an owned value that retains a {access} borrow rooted in {source}.",
            )),
        }
        documentation.push(' ');
        documentation.push_str(retained_lifetime_documentation(borrow.lifetime));
    } else if matches!(signature.return_type, ResolvedType::InterfaceSelf(_)) {
        documentation.push_str(
            "\n\nReturns a new owned value of the same exact dynamic implementing class.",
        );
    }
    documentation
}

fn retained_lifetime_documentation(lifetime: doriac::types::RetainedLifetime) -> &'static str {
    match lifetime {
        doriac::types::RetainedLifetime::LastUse =>
            "The source must remain available through the result's last use.",
        doriac::types::RetainedLifetime::Cleanup =>
            "The source must remain available through the result's cleanup, even after its last use.",
    }
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
                "Returns a new List containing selected Copy-or-Cloneable elements in insertion order. The source List remains unchanged.",
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
            display_nullable_resolved_type(value),
            "Removes and returns the final value, or `null` when the list is empty.",
        ),
        (ResolvedType::List(value), "get") => (
            "int $index".to_string(),
            display_nullable_resolved_type(value),
            "Returns the value at the given index, or `null` when the index is out of range. `$list[$index]` panics instead.",
        ),
        (ResolvedType::TypedArray(value), "get") => (
            "int $index".to_string(),
            display_nullable_resolved_type(value),
            "Returns the value at the given index, or `null` when the index is out of range. `$array[$index]` panics instead.",
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
            display_nullable_resolved_type(value),
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
            display_nullable_resolved_type(value),
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
            display_nullable_resolved_type(value),
            "Removes and returns the minimum value, or `null` when the queue is empty.",
        ),
        (ResolvedType::Deque(value), "pushFront" | "pushBack") => (
            format!("{} $value", display_resolved_type(value)),
            "void".to_string(),
            "Adds a value at the selected end of this writable deque.",
        ),
        (ResolvedType::Deque(value), "popFront" | "popBack") => (
            String::new(),
            display_nullable_resolved_type(value),
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
        _ => return None,
    };
    Some((parameters, return_type, documentation))
}

fn collection_property(receiver: &ResolvedType, property: &str) -> Option<&'static str> {
    let collection = collection_receiver_type(receiver);
    if let Some(member) = byte_intrinsics::member(collection, property)
        .filter(|member| member.kind == ByteMemberKind::Property)
    {
        return Some(member.documentation);
    }
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

fn collection_receiver_type(receiver: &ResolvedType) -> &ResolvedType {
    match non_nullable_type(receiver) {
        ResolvedType::SharedHandle(kind, payload) if kind.is_access() => payload.as_ref(),
        receiver => receiver,
    }
}

pub(crate) fn byte_signature(receiver: &ResolvedType, member: &ByteIntrinsic) -> String {
    let result = display_resolved_type(&member.return_type);
    if member.kind == ByteMemberKind::Property {
        return format!("{result} ${}", member.name);
    }
    let parameters = member
        .parameters
        .iter()
        .map(|parameter| {
            format!(
                "{} ${}",
                display_resolved_type(&parameter.ty),
                parameter.name
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let prefix = match member.receiver_mode {
        None => "static ",
        Some(ReceiverMode::Writable) => "writable ",
        Some(ReceiverMode::Consuming) => "consuming ",
        Some(ReceiverMode::Readonly) => "",
    };
    format!(
        "{prefix}function {}::{}({parameters}): {result}",
        display_resolved_type(receiver),
        member.name
    )
}

fn byte_completions(
    receiver: &ResolvedType,
    writable: bool,
    is_static: bool,
) -> Vec<SemanticCompletion> {
    let kinds: &[ByteMemberKind] = if is_static {
        &[ByteMemberKind::StaticMethod]
    } else {
        &[ByteMemberKind::Method, ByteMemberKind::Property]
    };
    kinds
        .iter()
        .flat_map(|kind| byte_intrinsics::members(receiver, *kind))
        .filter(|member| writable || member.receiver_mode != Some(ReceiverMode::Writable))
        .map(|member| SemanticCompletion {
            label: member.name.to_string(),
            kind: if member.kind == ByteMemberKind::Property {
                10
            } else {
                2
            },
            detail: byte_signature(receiver, &member),
            documentation: Some(member.documentation.to_string()),
        })
        .collect()
}

pub(crate) fn string_property_completions() -> Vec<SemanticCompletion> {
    byte_completions(&ResolvedType::String, false, false)
        .into_iter()
        .chain(STRING_PROPERTIES.iter().map(|property| SemanticCompletion {
            label: property.name.to_string(),
            kind: 10,
            detail: property.signature.to_string(),
            documentation: Some(property.documentation.to_string()),
        }))
        .collect()
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

fn foreach_binding_hovers(
    info: &SemanticInfo,
    binding_span: Span,
    kind: BindingKind,
    ty: &ResolvedType,
    role: &str,
    access: &str,
    iterable_type: &ResolvedType,
) -> Vec<SemanticHover> {
    let source_id = binding_span.source;
    let Some(declaration) = info
        .binding_resolution
        .declarations_by_id
        .values()
        .filter(|declaration| declaration.kind == kind)
        .filter_map(|declaration| declaration.span.map(|span| (declaration, span)))
        .filter(|(_, span)| binding_span.contains(*span))
        .min_by_key(|(declaration, span)| (span.start, span.end, declaration.id))
        .map(|(declaration, _)| declaration)
    else {
        return Vec::new();
    };
    let Some(declaration_span) = declaration.span else {
        return Vec::new();
    };
    let markdown = format!(
        "```doria\n{} ${}\n```\n\n**Role:** {role}\n\n**Access:** {access}\n\n**Iterable:** `{}`",
        display_resolved_type(ty),
        declaration.name,
        display_resolved_type(iterable_type),
    );

    let mut spans = vec![declaration_span];
    spans.extend(
        info.binding_resolution
            .uses_by_span
            .iter()
            .filter_map(|(span, binding)| {
                (*binding == declaration.id && span.source == source_id).then_some(*span)
            }),
    );
    spans.sort_by_key(|span| (span.start, span.end));
    spans.dedup();
    spans
        .into_iter()
        .map(|span| SemanticHover::new(span, markdown.clone()))
        .collect()
}

fn member_receiver_class_name(ty: &ResolvedType) -> Option<&str> {
    member_receiver_class_type(ty).map(|class| class.name.as_str())
}

fn member_receiver_class_type(ty: &ResolvedType) -> Option<&ClassType<ResolvedType>> {
    match non_nullable_type(ty) {
        ResolvedType::Class(class) => Some(class),
        ResolvedType::SharedHandle(
            SharedHandleKind::SharedReference
            | SharedHandleKind::ReadonlySharedReferenceAccess
            | SharedHandleKind::WritableSharedReferenceAccess,
            payload,
        ) => match non_nullable_type(payload) {
            ResolvedType::Class(class) => Some(class),
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
    result_type: &str,
    case_name: &str,
    case: Option<&EnumCaseDeclarationSemanticInfo>,
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
        format!("{enum_name}::{case_name}: {result_type}")
    } else {
        format!(
            "{enum_name}::{case_name}({}): {result_type}",
            parameters.join(", ")
        )
    }
}

fn enum_case_documentation(
    enum_info: Option<&EnumDeclarationSemanticInfo>,
    case: Option<&EnumCaseDeclarationSemanticInfo>,
    copy: Option<bool>,
) -> Option<String> {
    let case = case?;
    let mut documentation = if !case.payload.is_empty() {
        let ownership = match copy {
            Some(true) => "Copy enum",
            Some(false) => "Move enum",
            None => "enum",
        };
        format!("Constructs a {ownership} value with readonly payload fields.")
    } else {
        match (
            &case.backing_value,
            enum_info.and_then(|info| info.backing_type),
        ) {
            (Some(EnumBackingValue::Int(value)), Some(EnumBackingType::Int)) => {
                format!("Enum case with readonly `int` backing value `{value}`.")
            }
            (Some(EnumBackingValue::String(value)), Some(EnumBackingType::String)) => format!(
                "Enum case with readonly `string` backing value `{:?}`.",
                value
            ),
            _ => "Unit enum case.".to_string(),
        }
    };
    if let Some(result) = enum_info.and_then(|info| {
        transfer_result_documentation(&info.source_type.name)
            .or_else(|| flush_outcome_documentation(&info.source_type.name))
    }) {
        documentation.push_str("\n\n");
        documentation.push_str(result);
    }
    Some(documentation)
}

fn enum_documentation(name: &str, copy: Option<bool>) -> String {
    let ownership = match copy {
        Some(true) => "Copy ",
        Some(false) => "Move ",
        None => "",
    };
    let mut documentation = format!(
        "Nominal {ownership}enum. Each value records one declared case and that case's data."
    );
    if let Some(result) =
        transfer_result_documentation(name).or_else(|| flush_outcome_documentation(name))
    {
        documentation.push_str("\n\n");
        documentation.push_str(result);
    }
    documentation
}

fn flush_outcome_documentation(name: &str) -> Option<&'static str> {
    (name == "Doria\\Std\\Io\\FlushOutcome").then_some(
        "Complete means the flush completed. Pending preserves the unwritten remainder for a later flush call. Flushing does not implicitly wait for readiness; checked failures throw IoError. Both cases are payload-free Copy values.",
    )
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

fn display_nullable_resolved_type(ty: &ResolvedType) -> String {
    display_resolved_type(&ResolvedType::Nullable(Box::new(ty.clone())))
}

fn assertion_matcher_completion(
    matcher: doriac::assertions::AssertionMatcher,
    actual: &ResolvedType,
) -> SemanticCompletion {
    let signature = assertion_matcher_signature(matcher, actual, None);
    SemanticCompletion {
        label: matcher.source_name().to_string(),
        kind: 2,
        detail: signature.clone(),
        documentation: Some(assertion_matcher_documentation(matcher, actual, None)),
    }
}

fn assertion_matcher_signature(
    matcher: doriac::assertions::AssertionMatcher,
    actual: &ResolvedType,
    expected: Option<&ResolvedType>,
) -> String {
    use doriac::assertions::ExpectedOperandRule;
    let parameter = match matcher.expected_operand() {
        ExpectedOperandRule::None => String::new(),
        ExpectedOperandRule::SameType | ExpectedOperandRule::OrderedSameType => {
            format!("{} $expected", display_resolved_type(actual))
        }
        ExpectedOperandRule::String => "string $expected".to_string(),
        ExpectedOperandRule::ExactInt => "int $expected".to_string(),
        ExpectedOperandRule::CollectionElement => assertion_collection_element(actual)
            .map(|ty| format!("{} $expected", display_resolved_type(ty)))
            .unwrap_or_else(|| "$expected".to_string()),
        ExpectedOperandRule::DictionaryKey => assertion_dictionary_parts(actual)
            .map(|(key, _)| format!("{} $expected", display_resolved_type(key)))
            .unwrap_or_else(|| "$expected".to_string()),
        ExpectedOperandRule::DictionaryValue => assertion_dictionary_parts(actual)
            .map(|(_, value)| format!("{} $expected", display_resolved_type(value)))
            .unwrap_or_else(|| "$expected".to_string()),
        ExpectedOperandRule::OptionalErrorInspector => expected.map_or_else(
            || "[function(Error): void $inspector]".to_string(),
            |ty| format!("[{} $inspector]", display_resolved_type(ty)),
        ),
        ExpectedOperandRule::FailureMessage => "string $message".to_string(),
    };
    format!("{}({parameter}): void", matcher.source_name())
}

fn assertion_matcher_documentation(
    matcher: doriac::assertions::AssertionMatcher,
    actual: &ResolvedType,
    expected: Option<&ResolvedType>,
) -> String {
    let mut lines = vec![
        format!(
            "```doria\n{}\n```",
            assertion_matcher_signature(matcher, actual, expected)
        ),
        format!("**Actual type:** `{}`", display_resolved_type(actual)),
        format!("**Receiver family:** {}", assertion_receiver_family(actual)),
        "**Negation:** Supported through the property-shaped `not` modifier.".to_string(),
    ];
    if let Some(complexity) = matcher.stable_complexity() {
        lines.push(format!("**Complexity:** `{complexity}`"));
    }
    if matcher == doriac::assertions::AssertionMatcher::Throws {
        if let ResolvedType::Function(function) = actual {
            lines.push(format!(
                "Invokes the zero-parameter function subject once using its ordinary **{}** invocation mode.",
                invocation_mode_name(function.invocation_mode)
            ));
        }
        lines.push(
            "Fatal panic is not a checked Error and does not satisfy this matcher.".to_string(),
        );
    } else if !matches!(
        matcher.expected_operand(),
        doriac::assertions::ExpectedOperandRule::None
    ) {
        lines.push(
            "Expected operands use exact Doria types with no test-only coercion.".to_string(),
        );
    }
    lines.push(
        "Raises `AssertionError` with the automatic `TestAssertion` effect on failure.".to_string(),
    );
    lines.join("\n\n")
}

fn assertion_collection_element(actual: &ResolvedType) -> Option<&ResolvedType> {
    match actual {
        ResolvedType::TypedArray(element)
        | ResolvedType::List(element)
        | ResolvedType::Set(element)
        | ResolvedType::SortedSet(element)
        | ResolvedType::PriorityQueue(element)
        | ResolvedType::Deque(element) => Some(element),
        _ => None,
    }
}

fn assertion_dictionary_parts(actual: &ResolvedType) -> Option<(&ResolvedType, &ResolvedType)> {
    match actual {
        ResolvedType::Dictionary(key, value) | ResolvedType::SortedDictionary(key, value) => {
            Some((key, value))
        }
        _ => None,
    }
}

fn assertion_receiver_family(actual: &ResolvedType) -> &'static str {
    match actual {
        ResolvedType::TypedArray(_) => "Typed Array",
        ResolvedType::Bytes => "Bytes",
        ResolvedType::List(_) => "List",
        ResolvedType::Dictionary(_, _) => "Dictionary",
        ResolvedType::SortedDictionary(_, _) => "SortedDictionary",
        ResolvedType::Set(_) => "Set",
        ResolvedType::SortedSet(_) => "SortedSet",
        ResolvedType::PriorityQueue(_) => "PriorityQueue",
        ResolvedType::Deque(_) => "Deque",
        ResolvedType::Function(_) => "Function Value",
        ResolvedType::String => "String",
        ResolvedType::Nullable(_) | ResolvedType::Null | ResolvedType::Mixed => "Nullable Or Mixed",
        ResolvedType::Bool => "Bool",
        ResolvedType::Integer(_) | ResolvedType::Float(_) => "Numeric",
        _ => "Exact Doria Value",
    }
}

fn compiler_test_module_hover() -> &'static str {
    "`Doria\\Std\\Test`\n\nCompiler-owned development-source testing module. It provides behavioral declarations, expectations, explicit failure, and `AssertionError`; it is not part of the main-source prelude."
}

fn compiler_test_member_hover(name: &str) -> Option<&'static str> {
    match name {
        doriac::compiler_known_test::DESCRIBE => Some(
            "```doria\ndescribe(string $description, function(): void $body): void\n```\n\nDeclares a nested behavioral test suite in development source. The description is const-evaluated and the compiler preserves authored hierarchy and source ranges.",
        ),
        doriac::compiler_known_test::IT => Some(
            "```doria\nit(string $description, function(): void $body): void\n```\n\nDeclares a behavioral test in development source. The compiler generates direct test metadata and an isolated callable; no runtime suite registry is created.",
        ),
        doriac::compiler_known_test::TEST => Some(
            "```doria\ntest(string $description, function(): void $body): void\n```\n\nDeclares a behavioral test in development source. The compiler generates direct test metadata and an isolated callable; no runtime suite registry is created.",
        ),
        doriac::compiler_known_test::EXPECT => Some(
            "```doria\nexpect<T>(readonly T $actual)\n```\n\n**Compiler-Owned Ephemeral Expectation**\n\nDevelopment Source Only\n\nActual Evaluated Once\n\n`TestAssertion` On Failure",
        ),
        doriac::compiler_known_test::FAIL => Some(
            "```doria\nfail(string $message): void\n```\n\nRaises compiler-owned `AssertionError` with the automatic `TestAssertion` effect. The message is evaluated exactly once.",
        ),
        doriac::compiler_known_test::ASSERTION_ERROR => Some(
            "```doria\nclass AssertionError implements Error\n{\n    string $message;\n}\n```\n\nCompiler-owned catchable Error for failed expectations. `TestAssertion` propagates automatically, an unhandled assertion exits with status 70, and fatal panic remains a distinct runtime outcome. Private assertion payload fields are intentionally not exposed.",
        ),
        _ => None,
    }
}

fn behavioral_test_hover(
    kind: &str,
    display_name: &str,
    context: Option<&SourceSemanticContext>,
) -> String {
    let mut hover = format!(
        "**{kind}:** `{display_name}`\n\nCompiler-owned test metadata preserves the authored source range and suite hierarchy."
    );
    if context.is_some_and(|context| context.origin == doriac::build_plan::SourceOrigin::Generated)
    {
        hover.push_str("\n\n**Provenance:** Generated development source. Navigation is available, but editor writes and rename are disabled.");
    }
    hover
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

fn closure_provenance_summary(info: &SemanticInfo, provenance: &ValueProvenance) -> String {
    match provenance {
        ValueProvenance::Owned => "Owned Closure".to_string(),
        ValueProvenance::BorrowBound(roots) => {
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
    provenance: &ValueProvenance,
) -> String {
    match escape {
        ClosureEscapeClassification::Local => "Nonescaping".to_string(),
        ClosureEscapeClassification::Owned => "Owned callback".to_string(),
        ClosureEscapeClassification::ReturnedBorrow => {
            let roots = match provenance {
                ValueProvenance::BorrowBound(roots) => closure_root_names(info, roots),
                ValueProvenance::Owned => Vec::new(),
            };
            if roots.is_empty() {
                "Returned closure with a compiler-checked borrow".to_string()
            } else {
                format!("Returned closure tied to {}", roots.join(", "))
            }
        }
    }
}

fn closure_root_names(info: &SemanticInfo, roots: &[BorrowRoot]) -> Vec<String> {
    let mut names = roots
        .iter()
        .map(|root| match root {
            BorrowRoot::Binding(binding) => binding_source_name(info, *binding),
            BorrowRoot::Receiver => "$this".to_string(),
            BorrowRoot::EnclosingEnvironment(_) => "the enclosing closure".to_string(),
            BorrowRoot::Temporary => "a temporary receiver".to_string(),
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

fn nominal_condition_signature(condition: Option<&NominalCondition>) -> String {
    let Some(condition) = condition else {
        return String::new();
    };
    let bounds = condition
        .bounds
        .iter()
        .map(|bound| {
            format!(
                "{} implements {}",
                bound.subject.text,
                bound
                    .interfaces
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(" where ({bounds})")
}

fn function_signature(
    function: &FunctionDecl,
    container: Option<&str>,
    presentation: &ParameterPresentation,
) -> String {
    let mut modifiers = Vec::new();
    if matches!(function.access, MemberAccess::Internal) {
        modifiers.push("internal");
    }
    if function.is_open {
        modifiers.push("open");
    }
    if function.is_override {
        modifiers.push("override");
    }
    match ReceiverMode::from_declaration(function) {
        ReceiverMode::Writable => modifiers.push("writable"),
        ReceiverMode::Consuming => modifiers.push("consuming"),
        ReceiverMode::Readonly => {}
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
        .map(|parameter| parameter_signature(parameter, presentation))
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

    format!(
        "{prefix}function {name}{type_parameters}({parameters}){return_type}{throws}{}",
        nominal_condition_signature(function.availability.as_ref())
    )
}

fn override_stub(function: &FunctionDecl) -> String {
    let mut modifiers = vec!["override"];
    match ReceiverMode::from_declaration(function) {
        ReceiverMode::Writable => modifiers.push("writable"),
        ReceiverMode::Consuming => modifiers.push("consuming"),
        ReceiverMode::Readonly => {}
    }
    if function.is_static {
        modifiers.push("static");
    }
    let parameters = function
        .params
        .iter()
        .map(parameter_signature_without_default)
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
    format!(
        "{} function {}{}({parameters}){return_type}{throws}\n{{\n    ${{0}}\n}}",
        modifiers.join(" "),
        function.name,
        type_parameter_override_signature(&function.type_params),
    )
}

fn parameter_signature(parameter: &Param, presentation: &ParameterPresentation) -> String {
    let mut parts = Vec::new();
    if parameter.borrow_span.is_some() {
        parts.push("borrow".to_string());
    }
    match parameter.constructor_role {
        ConstructorParameterRole::Promoted {
            access: MemberAccess::Internal,
            ..
        } => parts.push("internal".to_string()),
        ConstructorParameterRole::InheritedPropertyOverride { .. } => {
            parts.push("override".to_string());
        }
        ConstructorParameterRole::ConstructorOnly { .. } => {
            parts.push("parameter".to_string());
        }
        ConstructorParameterRole::Ordinary
        | ConstructorParameterRole::Promoted {
            access: MemberAccess::External,
            ..
        } => {}
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
        rendered.push_str(" = ");
        // The compiler's text of the default; `...` only when the source
        // did not reach semantic analysis.
        rendered.push_str(presentation.default_of(parameter.span).unwrap_or("..."));
    }
    rendered
}

fn parameter_signature_without_default(parameter: &Param) -> String {
    let mut parts = Vec::new();
    if parameter.borrow_span.is_some() {
        parts.push("borrow".to_string());
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

fn constructor_parameter_documentation(
    semantic_info: Option<&SemanticInfo>,
    current_class: Option<&str>,
    parameter: &Param,
) -> Option<String> {
    let semantic_info = semantic_info?;
    let role = semantic_info.constructor_parameters.get(&parameter.span)?;
    Some(match &role.role {
        ConstructorParameterSemanticRole::Promoted { access } => {
            promoted_property_documentation(current_class.unwrap_or("class"), parameter, *access)
        }
        ConstructorParameterSemanticRole::ConstructorOnly => format!(
            "**Constructor-Only Parameter**\n\nDeclares No Property\n\nAvailable In `__construct` Only\n\n**Parameter Mode:** {}",
            parameter_mode_documentation(parameter),
        ),
        ConstructorParameterSemanticRole::InheritedPropertyOverride {
            declaring_class,
            property_name,
            ..
        } => {
            let root_mutability = semantic_info
                .classes
                .iter()
                .flat_map(|class| &class.properties)
                .find(|property| {
                    property.declaring_class == *declaring_class && property.name == *property_name
                })
                .map_or("Compiler-resolved", |property| {
                    if property.writable {
                        "Writable"
                    } else {
                        "Readonly"
                    }
                });
            format!(
                "**Inherited Property Override Parameter**\n\nReuses `{declaring_class}::{property_name}`\n\nDeclares No New Storage\n\nInitialized By The Parent Construction Phase\n\n**Parameter Mode:** {}\n\n**Inherited Property Mutability:** {root_mutability}",
                parameter_mode_documentation(parameter),
            )
        }
    })
}

fn promoted_property_documentation(
    class_name: &str,
    parameter: &Param,
    access: MemberAccess,
) -> String {
    if parameter.borrow_span.is_some() {
        return format!(
            "**Retained Readonly Source**\n\nDeclares `{class_name}::{}` as a source loan, not an owned field. The source stays with its owner. It cannot be mutated while the loan is active.",
            parameter.name,
        );
    }
    format!(
        "**Promoted Property**\n\n**Accessibility:** {}\n\n**Mutability:** {}\n\nDeclares `{class_name}::{}`",
        if access == MemberAccess::Internal {
            "Internal"
        } else {
            "External"
        },
        if parameter.writable {
            "Writable"
        } else {
            "Readonly"
        },
        parameter.name,
    )
}

fn parameter_mode_documentation(parameter: &Param) -> &'static str {
    if parameter.borrow_span.is_some() {
        "Retained Readonly Source (`borrow`)"
    } else if parameter.take {
        "Owned (`take`)"
    } else if parameter.writable {
        "Writable Borrow"
    } else {
        "Readonly Borrow"
    }
}

fn type_parameter_signature(parameters: &[doriac::ast::TypeParamDecl]) -> String {
    if parameters.is_empty() {
        return String::new();
    }
    let parameters = parameters
        .iter()
        .map(|parameter| {
            let mut rendered =
                doriac::trait_composition::authored_type_parameter_name(&parameter.name)
                    .to_string();
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

fn contract_signature(
    kind: &str,
    name: &str,
    parameters: &[doriac::ast::TypeParamDecl],
    parents: &[TypeRef],
) -> String {
    let mut signature = format!("{kind} {name}{}", type_parameter_signature(parameters));
    if !parents.is_empty() {
        signature.push_str(&format!(
            " extends {}",
            parents
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    signature
}

fn type_parameter_override_signature(parameters: &[doriac::ast::TypeParamDecl]) -> String {
    if parameters.is_empty() {
        return String::new();
    }
    let parameters = parameters
        .iter()
        .map(|parameter| {
            let mut rendered =
                doriac::trait_composition::authored_type_parameter_name(&parameter.name)
                    .to_string();
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
            rendered
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("<{parameters}>")
}

fn class_signature(class: &ClassDecl) -> String {
    let mut signature = format!(
        "{}class {}{}",
        if class.is_open { "open " } else { "" },
        class.name,
        type_parameter_signature(&class.type_params)
    );
    if let Some(parent) = &class.parent {
        signature.push_str(&format!(" extends {parent}"));
    }
    if !class.implements.is_empty() {
        signature.push_str(&format!(
            " implements {}",
            class
                .implements
                .iter()
                .map(|entry| format!(
                    "{}{}",
                    entry.ty,
                    nominal_condition_signature(entry.condition.as_ref())
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    signature
}

fn class_hierarchy_documentation(
    hierarchy: &doriac::semantics::ClassHierarchySemanticInfo,
) -> String {
    let parent = hierarchy
        .parent
        .as_ref()
        .map_or("none", |parent| parent.name.as_str());
    let role = if hierarchy.parent.is_none() {
        "Root"
    } else {
        "Derived"
    };
    format!(
        "**Direct Parent:** `{parent}`\n\n**Known Hierarchy Role:** {role}\n\n**Hierarchy Depth:** {}\n\n**Subclassing:** {}\n\n**Runtime Representation:** {}",
        hierarchy.ancestors.len(),
        if hierarchy.is_open {
            "Permitted"
        } else {
            "Closed"
        },
        if hierarchy.is_open {
            "Open Class"
        } else {
            "Closed Class"
        },
    )
}

fn method_hierarchy_documentation(
    hierarchy: &doriac::semantics::MethodHierarchySemanticInfo,
) -> String {
    let role = if hierarchy.is_override {
        "Override"
    } else if hierarchy.is_open {
        "Open Virtual Root"
    } else {
        "Nonvirtual"
    };
    let receiver = match hierarchy.receiver_mode {
        Some(ReceiverMode::Writable) => "Writable",
        Some(ReceiverMode::Readonly) | None => "Readonly",
        Some(ReceiverMode::Consuming) => "Consuming",
    };
    let dispatch = if hierarchy.virtual_root.is_some() {
        "Virtual"
    } else {
        "Direct"
    };
    format!(
        "**Hierarchy Method Role:** {role}\n\n**Effective Receiver:** {receiver}\n\n**Source Dispatch:** {dispatch}"
    )
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
        .map(|token| token.span.in_expansion(span.expansion))
}

fn find_variable_span(tokens: &[Token], span: Span, name: &str) -> Option<Span> {
    tokens_in_span(tokens, span)
        .find(|token| matches!(&token.kind, TokenKind::Variable(variable) if variable == name))
        .map(|token| token.span.in_expansion(span.expansion))
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
    left.source == right.source
        && left.expansion == right.expansion
        && left.start < right.end
        && right.start < left.end
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

    #[test]
    fn nullable_collection_method_fallbacks_use_compiler_canonical_types() {
        let int = ResolvedType::Integer(doriac::numeric::IntegerType::Int64);
        let nested = ResolvedType::List(Box::new(int.clone()));
        for (element, expected) in [
            (nested.clone(), "?List<int>"),
            (ResolvedType::Nullable(Box::new(nested)), "?List<int>"),
            (ResolvedType::Mixed, "?mixed"),
            (
                ResolvedType::Nullable(Box::new(ResolvedType::Mixed)),
                "?mixed",
            ),
        ] {
            let list = ResolvedType::List(Box::new(element.clone()));
            let dictionary =
                ResolvedType::Dictionary(Box::new(ResolvedType::String), Box::new(element.clone()));
            let sorted = ResolvedType::SortedDictionary(
                Box::new(ResolvedType::String),
                Box::new(element.clone()),
            );
            let deque = ResolvedType::Deque(Box::new(element));
            for (receiver, method) in [
                (&list, "pop"),
                (&list, "get"),
                (&dictionary, "get"),
                (&dictionary, "remove"),
                (&sorted, "get"),
                (&sorted, "remove"),
                (&deque, "popFront"),
                (&deque, "popBack"),
            ] {
                let hover = compiler_known_method_hover(receiver, method, false, None).unwrap();
                assert!(
                    hover.signature.ends_with(&format!(": {expected}")),
                    "{}",
                    hover.signature
                );
                assert!(!hover.signature.contains("??"));
            }
        }
        let queue = ResolvedType::PriorityQueue(Box::new(ResolvedType::Nullable(Box::new(int))));
        let hover = compiler_known_method_hover(&queue, "pop", false, None).unwrap();
        assert!(hover.signature.ends_with(": ?int"), "{}", hover.signature);
    }

    #[test]
    fn collection_return_hovers_distinguish_borrowed_reads_from_owned_removals() {
        let source =
            include_str!("../../editors/fixtures/stage36a-nullable-collection-returns.doria");
        let snapshot = AnalysisSnapshot::analyze("nullable-collection-returns.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        for name in [
            "firstNested",
            "lastNested",
            "firstNullableNested",
            "lastNullableNested",
            "getNested",
            "getNullableNested",
            "firstMixed",
            "lastMixed",
            "firstNullableMixed",
            "lastNullableMixed",
            "getMixed",
            "getNullableMixed",
            "peekFrontNested",
            "peekBackNested",
            "peekFrontNullableNested",
            "peekBackNullableNested",
            "peekFrontMixed",
            "peekBackMixed",
            "peekFrontNullableMixed",
            "peekBackNullableMixed",
        ] {
            let hover = snapshot
                .hover_at_offset(source.find(name).unwrap())
                .unwrap();
            assert!(
                hover
                    .markdown
                    .contains("Returns a readonly borrow rooted in `$source`."),
                "{name}: {}",
                hover.markdown
            );
            assert!(!hover.markdown.contains("??"));
        }
        for name in [
            "popNested",
            "popNullableNested",
            "removeNested",
            "removeNullableNested",
            "removeAtNested",
            "removeAtNullableNested",
            "popMixed",
            "popNullableMixed",
            "removeMixed",
            "removeNullableMixed",
            "removeAtMixed",
            "removeAtNullableMixed",
            "popFrontNested",
            "popBackNested",
            "popFrontNullableNested",
            "popBackNullableNested",
            "popFrontMixed",
            "popBackMixed",
            "popFrontNullableMixed",
            "popBackNullableMixed",
        ] {
            let hover = snapshot
                .hover_at_offset(source.find(name).unwrap())
                .unwrap();
            assert!(
                !hover.markdown.contains("borrow rooted"),
                "{name}: {}",
                hover.markdown
            );
            assert!(!hover.markdown.contains("??"));
        }
    }

    #[test]
    fn owned_source_carrier_hovers_do_not_claim_ordinary_borrowed_returns() {
        let source = include_str!("../../editors/fixtures/stage36a-owned-source-carriers.doria");
        let snapshot = AnalysisSnapshot::analyze("owned-source-carriers.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        for name in ["pass", "obtain", "popIterator", "popCallback", "popNested"] {
            let hover = snapshot
                .hover_at_offset(source.find(name).unwrap())
                .unwrap();
            assert!(
                !hover.markdown.contains("Returns a readonly borrow"),
                "{name}: {}",
                hover.markdown
            );
            assert!(
                !hover.markdown.contains("Returns a writable borrow"),
                "{name}: {}",
                hover.markdown
            );
            assert!(!hover.markdown.contains("??"), "{name}: {}", hover.markdown);
            if name == "obtain" {
                assert!(
                    hover
                        .markdown
                        .contains("Retains a readonly source loan from `$shelf`."),
                    "{}",
                    hover.markdown
                );
            }
        }
    }

    #[test]
    fn callable_source_documentation_reaches_function_and_method_references() {
        let source = r#"
/** Reads the first nested list. */
function first(List<List<int>> $source): ?List<int> { return $source->first; }
class Reader {
    function first(List<List<int>> $source): ?List<int> { return $source->first; }
}
function main(): void {
    List<List<int>> $source = [[1]];
    let $reader = new Reader();
    { let $value = first($source); }
    { let $value = $reader->first($source); }
}
"#;
        let snapshot = AnalysisSnapshot::analyze("callable-source-documentation.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        let occurrences = source.match_indices("first(").collect::<Vec<_>>();
        assert_eq!(occurrences.len(), 4);
        for (offset, _) in occurrences {
            let hover = snapshot.hover_at_offset(offset).unwrap();
            assert_eq!(
                hover
                    .markdown
                    .matches("Returns a readonly borrow rooted in `$source`.")
                    .count(),
                1,
                "{}",
                hover.markdown
            );
            assert!(hover
                .markdown
                .contains("The source must remain available through the result's last use."));
        }
        let hover = snapshot
            .hover_at_offset(source.find("first(").unwrap())
            .unwrap();
        assert!(hover.markdown.contains("Reads the first nested list."));
    }

    #[test]
    fn callable_calls_preserve_callee_and_argument_references() {
        let source = "function invoke(function(int): int $callback, int $value): int { return ($callback)($value); }";
        let snapshot = AnalysisSnapshot::analyze("callable-references.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        for name in ["$callback", "$value"] {
            assert_eq!(
                snapshot
                    .declaration_span_at_offset(source.rfind(name).unwrap())
                    .unwrap()
                    .start,
                source.find(name).unwrap(),
                "{name}"
            );
        }
    }

    #[test]
    fn return_documentation_preserves_compiler_loan_kind_source_access_and_lifetime() {
        use doriac::symbols::{BorrowSource, ReturnBorrow};
        use doriac::types::{RetainedLifetime, ReturnBorrowKind};

        let signature = CallableSignatureSemanticInfo {
            generic_parameter_count: 0,
            parameters: vec![doriac::semantics::CallableParameterSemanticInfo {
                name: "source".to_string(),
                r#type: ResolvedType::Class(ClassType::new("Source", Vec::new())),
                take: false,
                writable: true,
                borrow: false,
                has_default: false,
                default: None,
                declaration: None,
            }],
            return_type: ResolvedType::Class(ClassType::new("Result", Vec::new())),
        };
        for (source, label) in [
            (BorrowSource::Receiver, "the receiver"),
            (BorrowSource::Parameter(0), "`$source`"),
        ] {
            for writable in [false, true] {
                let access = if writable { "writable" } else { "readonly" };
                for kind in [ReturnBorrowKind::Value, ReturnBorrowKind::Retained] {
                    for (lifetime, obligation) in [
                        (
                            RetainedLifetime::LastUse,
                            "The source must remain available through the result's last use.",
                        ),
                        (
                            RetainedLifetime::Cleanup,
                            "The source must remain available through the result's cleanup, even after its last use.",
                        ),
                    ] {
                        let actual = callable_return_documentation(
                            &signature,
                            Some(ReturnBorrow {
                                source,
                                writable,
                                kind,
                                lifetime,
                            }),
                        );
                        let expected = match kind {
                            ReturnBorrowKind::Value => format!("Returns a {access} borrow rooted in {label}."),
                            ReturnBorrowKind::Retained => format!("Returns an owned value that retains a {access} borrow rooted in {label}."),
                        };
                        assert_eq!(actual.trim(), format!("{expected} {obligation}"));
                    }
                }
            }
        }
    }

    #[test]
    fn composed_completions_use_the_selected_specialized_member_surface() {
        let source = include_str!("../../editors/fixtures/stage35-contracts.doria");
        let snapshot = AnalysisSnapshot::analyze("contracts.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        for (call, parameter_type, has_alias) in [
            ("$report->render(1)", "int", true),
            ("$text->render(\"value\")", "string", false),
        ] {
            let offset = source.find(call).unwrap() + call.find("render").unwrap();
            let members = snapshot.member_completions_at_offset(offset).unwrap();
            let renders = members
                .iter()
                .filter(|member| member.label == "render")
                .collect::<Vec<_>>();
            assert_eq!(renders.len(), 1, "{members:?}");
            assert!(
                renders[0]
                    .detail
                    .contains(&format!("{parameter_type} $value")),
                "{renders:?}"
            );
            assert_eq!(
                has_alias,
                members
                    .iter()
                    .any(|member| member.label == "renderAlternative"),
                "{members:?}"
            );
            assert!(
                members.iter().any(|member| member.label == "count"),
                "{members:?}"
            );
            for hidden in ["debug", "traceAlternative", "typeLabel", "LABEL"] {
                assert!(
                    !members.iter().any(|member| member.label == hidden),
                    "{members:?}"
                );
            }
        }
    }

    #[test]
    fn composed_source_hovers_are_contextual_and_order_independent() {
        let source = "trait Values<T> { function value(T $value): T { return $value; } function again(T $value): T { return $this->value($value); } static function staticValue(T $value): T { return $value; } static function againStatic(T $value): T { return self::staticValue($value); } } class First { uses Values<int>; } class Second { uses Values<string>; }";
        let mut snapshot = AnalysisSnapshot::analyze("contexts.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        let offset = source.find("->value").unwrap() + 2;
        let first = snapshot.semantic_hover_at_offset(offset).unwrap();
        for expected in [
            "First::again",
            "Second::again",
            "int $value): int",
            "string $value): string",
        ] {
            assert!(first.markdown.contains(expected), "{first:?}");
        }
        assert_eq!(first.span.start, offset);
        assert_eq!(first.span, first.span.authored());
        let argument = offset + "value(".len();
        let signatures = snapshot.signature_help_at_offset(argument).unwrap();
        assert_eq!(signatures.len(), 2, "{signatures:?}");
        let completions = snapshot.member_completions_at_offset(offset).unwrap();
        let static_offset = source.find("self::staticValue").unwrap() + "self::".len();
        let static_completions = snapshot
            .static_completions_at_offset(static_offset)
            .unwrap();
        for ty in ["int", "string"] {
            let signature = format!("{ty} $value): {ty}");
            assert!(
                signatures
                    .iter()
                    .any(|item| item.label.contains(&signature)),
                "{signatures:?}"
            );
            assert!(
                completions
                    .iter()
                    .any(|item| item.label == "value" && item.detail.contains(&signature)),
                "{completions:?}"
            );
            assert!(
                static_completions
                    .iter()
                    .any(|item| item.label == "staticValue" && item.detail.contains(&signature)),
                "{static_completions:?}"
            );
        }
        snapshot.semantic_hovers.reverse();
        snapshot.call_signatures.reverse();
        snapshot.member_receivers.reverse();
        snapshot.static_receivers.reverse();
        snapshot
            .semantic_hovers
            .push(snapshot.semantic_hovers[0].clone());
        assert_eq!(
            first.markdown,
            snapshot.semantic_hover_at_offset(offset).unwrap().markdown
        );
        assert_eq!(
            signatures,
            snapshot.signature_help_at_offset(argument).unwrap()
        );
        assert_eq!(
            completions,
            snapshot.member_completions_at_offset(offset).unwrap()
        );
        assert_eq!(
            static_completions,
            snapshot
                .static_completions_at_offset(static_offset)
                .unwrap()
        );
    }

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
        assert_eq!(
            details.get("transferredByteCount").map(String::as_str),
            Some("?int $transferredByteCount")
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
        let empty = doriac::parse_source("io-inventory.doria", "").unwrap();
        let declarations = doriac::compiler_known_io::augment_program(&empty);
        let operation = declarations
            .items
            .iter()
            .find_map(|item| match item {
                Item::Enum(declaration)
                    if declaration.name == doriac::compiler_known_io::IO_OPERATION =>
                {
                    Some(declaration)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(
            cases,
            operation
                .cases
                .iter()
                .map(|case| case.name.clone())
                .collect()
        );
        assert!(snapshot
            .hover_at_offset(case_offset)
            .expect("compiler-known enum case hover")
            .markdown
            .contains("Doria\\Std\\Io\\IoOperation::Read: Doria\\Std\\Io\\IoOperation"));
        for (ty, case) in [
            (doriac::compiler_known_io::IO_OPERATION, "Register"),
            (doriac::compiler_known_io::IO_OPERATION, "Unregister"),
            (doriac::compiler_known_io::IO_OPERATION, "Wait"),
            (doriac::compiler_known_io::IO_OPERATION, "Configure"),
            (doriac::compiler_known_io::IO_OPERATION, "Access"),
            (doriac::compiler_known_io::IO_TARGET, "ReadinessSet"),
            (doriac::compiler_known_io::IO_TARGET, "TransferPlan"),
        ] {
            let source = format!("function inspect(): void {{ {ty} $value = {ty}::{case}; }}");
            let snapshot = AnalysisSnapshot::analyze("readiness-io-case.doria", &source);
            assert!(
                snapshot.diagnostics().is_empty(),
                "{:?}",
                snapshot.diagnostics()
            );
            let offset = source.rfind(case).unwrap();
            assert!(snapshot
                .static_completions_at_offset(offset)
                .unwrap()
                .iter()
                .any(|completion| completion.label == case));
            assert!(snapshot
                .hover_at_offset(offset)
                .unwrap()
                .markdown
                .contains(&format!("{ty}::{case}: {ty}")));
        }
    }

    #[test]
    fn compiler_known_errors_preserve_nominal_and_member_facts() {
        for name in [
            doriac::compiler_known_time::CLOCK_ERROR,
            doriac::compiler_known_concurrency::CANCELLATION_ERROR,
        ] {
            for (prefix, spelling) in [
                (String::new(), name),
                (format!("use {name} as TimeFailure; "), "TimeFailure"),
            ] {
                let source = format!(
                    "{prefix}function inspect({spelling} $view): void {{ \
                let $error = new {spelling}(\"failed\", systemCode: 0); \
                let $code = $error->systemCode; let $message = $error->message; }}"
                );
                let snapshot = AnalysisSnapshot::analyze("clock-error-help.doria", &source);
                assert!(
                    snapshot.diagnostics().is_empty(),
                    "{:?}",
                    snapshot.diagnostics()
                );
                let type_offset = source.find(&format!("{spelling} $view")).unwrap();
                let hover = snapshot.hover_at_offset(type_offset).unwrap();
                assert!(
                    hover.markdown.contains(&format!("class {name}")),
                    "{hover:?}"
                );
                assert!(hover.markdown.contains("`Error`"), "{hover:?}");
                assert!(snapshot.declaration_span_at_offset(type_offset).is_none());

                let offset = source.find("->systemCode").unwrap() + 2;
                let members = snapshot.member_completions_at_offset(offset).unwrap();
                let properties = members
                    .iter()
                    .filter(|member| member.kind == 10)
                    .map(|member| member.label.as_str())
                    .collect::<HashSet<_>>();
                assert_eq!(properties, HashSet::from(["message", "systemCode"]));
                for (property, signature) in [
                    ("message", "string $message"),
                    ("systemCode", "?int $systemCode"),
                ] {
                    let member = members
                        .iter()
                        .find(|member| member.label == property)
                        .unwrap();
                    assert_eq!(member.detail, signature);
                    assert!(member
                        .documentation
                        .as_deref()
                        .unwrap()
                        .contains("readonly"));
                    let offset = source.find(&format!("->{property}")).unwrap() + 2;
                    let hover = snapshot.hover_at_offset(offset).unwrap();
                    assert!(hover.markdown.contains(signature), "{hover:?}");
                    assert!(hover.markdown.contains("readonly"), "{hover:?}");
                    assert!(snapshot.declaration_span_at_offset(offset).is_none());
                }
                let help = snapshot
                    .signature_help_at_offset(source.find("systemCode:").unwrap())
                    .unwrap();
                assert!(help[0].label.contains(&format!("{name}::__construct(")));
                assert!(help[0].label.contains("string $message, ?int $systemCode"));
                assert_eq!(help[0].active_parameter, 1);
            }
        }
    }

    #[test]
    fn new_expression_class_hover_shows_the_constructor_declared_in_the_same_file() {
        let source = "class ApplicationConfig { function __construct(string $name, internal writable int $width = 120) {} }\n\
            class Application {\n    /** Builds the application from its configuration. */\n    function __construct(take ApplicationConfig $config) {}\n}\n\
            function main(): void { let $app = new Application(new ApplicationConfig(\"demo\")); }";

        let created = hover(source, "Application(new", 0).markdown;
        assert!(created.contains("class Application"), "{created}");
        assert!(
            created.contains("function Application::__construct(take ApplicationConfig $config)"),
            "{created}"
        );
        assert!(
            created.contains("Builds the application from its configuration."),
            "the constructor's documentation travels with it: {created}"
        );

        // A default reads as written, the same as from an unopened file.
        let nested = hover(source, "ApplicationConfig(\"demo", 0).markdown;
        assert!(
            nested.contains(
                "function ApplicationConfig::__construct(string $name, internal writable int $width = 120)"
            ),
            "{nested}"
        );

        let declaration = hover(source, "Application {", 0).markdown;
        assert!(
            !declaration.contains("__construct"),
            "the declaration itself is not a constructor call: {declaration}"
        );
    }

    #[test]
    fn compiler_known_constructors_share_signature_and_named_argument_help() {
        for (name, arguments, last_parameter, active_parameter) in [
            (doriac::compiler_known_time::CLOCK_ERROR, "\"failed\", systemCode: 0", "systemCode", 1),
            (doriac::compiler_known_concurrency::CANCELLATION_ERROR, "\"failed\", systemCode: 0", "systemCode", 1),
            (doriac::compiler_known_io::INVALID_UTF8_ERROR,
                "\"invalid\", Doria\\Std\\Io\\Utf8InputSource::StandardInput, validByteCount: 0", "validByteCount", 2),
            (doriac::compiler_known_io::IO_ERROR,
                "\"failed\", Doria\\Std\\Io\\IoOperation::Read, Doria\\Std\\Io\\IoTarget::StandardInput, reason: Doria\\Std\\Io\\IoErrorReason::Other", "reason", 3),
        ] {
            for (prefix, spelling) in [
                (String::new(), name),
                (format!("use {name} as Failure; "), "Failure"),
            ] {
                let source = format!("{prefix}function inspect(): void {{ let $error = new {spelling}({arguments}); }}");
                let snapshot = AnalysisSnapshot::analyze("constructor-help.doria", &source);
                assert!(snapshot.diagnostics().is_empty(), "{:?}", snapshot.diagnostics());
                let offset = source.find(&format!("{last_parameter}:")).unwrap();
                let help = snapshot.signature_help_at_offset(offset).unwrap();
                assert!(help[0].label.contains(&format!("{name}::__construct(")), "{help:?}");
                assert_eq!(help[0].active_parameter, active_parameter);
                assert!(snapshot.named_argument_completions_at_offset(offset).unwrap().iter()
                    .any(|completion| completion.label == format!("{last_parameter}:")));
                let class_offset = source.find(&format!("new {spelling}")).unwrap() + 4;
                assert!(snapshot.hover_at_offset(class_offset).unwrap().markdown.contains(&format!("class {name}")));
                assert!(snapshot.declaration_span_at_offset(class_offset).is_none());
            }
        }
    }

    #[test]
    fn duration_factory_and_getter_help_uses_compiler_member_facts() {
        let name = doriac::compiler_known_time::DURATION;
        let integer =
            display_resolved_type(&ResolvedType::Integer(doriac::numeric::IntegerType::Int64));
        for (prefix, spelling) in [
            (String::new(), name),
            (format!("use {name} as Interval; "), "Interval"),
        ] {
            let source = format!(
                "{prefix}function inspect(?{spelling} $optional): void {{ \
                let $interval = {spelling}::fromNanoseconds(nanoseconds: -1); \
                let $copy = $interval; let $number = $interval->nanoseconds; \
                let $maybe = $optional?->nanoseconds; }}"
            );
            let snapshot = AnalysisSnapshot::analyze("duration-members.doria", &source);
            assert!(
                snapshot.diagnostics().is_empty(),
                "{:?}",
                snapshot.diagnostics()
            );

            let offset = source.find("fromNanoseconds").unwrap();
            let members = snapshot.static_completions_at_offset(offset).unwrap();
            let factory = members
                .iter()
                .find(|member| member.label == "fromNanoseconds")
                .unwrap();
            assert_eq!(factory.kind, 2);
            assert!(
                factory.detail.contains(&format!(
                    "{name}::fromNanoseconds({integer} $nanoseconds): {name}"
                )),
                "{factory:?}"
            );
            assert!(!factory.detail.contains("throws"));
            assert!(!members.iter().any(|member| member.label == "__construct"));
            let hover = snapshot.hover_at_offset(offset).unwrap();
            assert!(hover.markdown.contains(&factory.detail), "{hover:?}");
            assert!(snapshot.declaration_span_at_offset(offset).is_none());

            let argument = source.find("nanoseconds:").unwrap();
            let help = snapshot.signature_help_at_offset(argument).unwrap();
            assert_eq!(help[0].label, factory.detail);
            assert_eq!(help[0].active_parameter, 0);
            assert!(snapshot
                .named_argument_completions_at_offset(argument)
                .unwrap()
                .iter()
                .any(|completion| completion.label == "nanoseconds:"));

            let offset = source.find("->nanoseconds").unwrap() + 2;
            let members = snapshot.member_completions_at_offset(offset).unwrap();
            let getter = members
                .iter()
                .find(|member| member.label == "nanoseconds")
                .unwrap();
            assert_eq!(getter.kind, 10);
            assert_eq!(getter.detail, format!("$nanoseconds {{ get: {integer} }}"));
            assert!(getter
                .documentation
                .as_deref()
                .unwrap()
                .contains("Readonly"));
            assert!(!getter.detail.contains("set("));
            for (start, _) in source.match_indices("->nanoseconds") {
                let hover = snapshot.hover_at_offset(start + 2).unwrap();
                assert!(
                    hover.markdown.contains(&format!("get: {integer}"))
                        || hover.markdown.contains(&format!("get: ?{integer}")),
                    "{hover:?}"
                );
                assert!(hover.markdown.contains("Readonly"), "{hover:?}");
                assert!(snapshot.declaration_span_at_offset(start + 2).is_none());
            }
        }
    }

    #[test]
    fn deadline_factory_help_uses_compiler_signature_and_checked_effects() {
        let name = doriac::compiler_known_time::DEADLINE;
        let duration = doriac::compiler_known_time::DURATION;
        let error = doriac::compiler_known_time::CLOCK_ERROR;
        for (prefix, spelling) in [
            (String::new(), name),
            (format!("use {name} as Instant; "), "Instant"),
        ] {
            let source = format!(
                "{prefix}function inspect({duration} $interval): void throws {error} {{ \
                let $deadline = {spelling}::fromNow(duration: $interval); \
                let $copy = $deadline; ?{spelling} $optional = $deadline; }}"
            );
            let snapshot = AnalysisSnapshot::analyze("deadline-factory.doria", &source);
            assert!(
                snapshot.diagnostics().is_empty(),
                "{:?}",
                snapshot.diagnostics()
            );
            let offset = source.find("fromNow").unwrap();
            let members = snapshot.static_completions_at_offset(offset).unwrap();
            let factory = members
                .iter()
                .find(|member| member.label == "fromNow")
                .unwrap();
            assert_eq!(factory.kind, 2);
            assert_eq!(
                factory.detail,
                format!(
                    "static function {name}::fromNow({duration} $duration): {name} throws {error}"
                )
            );
            assert!(!members.iter().any(|member| member.label == "__construct"));
            let hover = snapshot.hover_at_offset(offset).unwrap();
            assert!(hover.markdown.contains(&factory.detail), "{hover:?}");
            assert!(snapshot.declaration_span_at_offset(offset).is_none());
            let argument = source.find("duration:").unwrap();
            let help = snapshot.signature_help_at_offset(argument).unwrap();
            assert_eq!(help[0].label, factory.detail);
            assert_eq!(help[0].active_parameter, 0);
            assert!(snapshot
                .named_argument_completions_at_offset(argument)
                .unwrap()
                .iter()
                .any(|completion| completion.label == "duration:"));
            for binding in ["$deadline", "$copy", "$optional"] {
                let hover = snapshot
                    .hover_at_offset(source.rfind(binding).unwrap())
                    .unwrap();
                assert!(
                    hover.markdown.contains(INLINE_COPY_DOCUMENTATION),
                    "{hover:?}"
                );
                assert!(hover.markdown.contains(DEADLINE_DOCUMENTATION), "{hover:?}");
            }
        }
    }

    #[test]
    fn wait_enum_help_uses_compiler_copy_and_payload_facts() {
        let name = "Doria\\Std\\Io\\Wait";
        for (prefix, spelling) in [
            (String::new(), name),
            (format!("use {name} as Observation; "), "Observation"),
        ] {
            for (case, parameter, ty) in [
                ("For", "duration", doriac::compiler_known_time::DURATION),
                ("Until", "deadline", doriac::compiler_known_time::DEADLINE),
            ] {
                let source = format!(
                    "{prefix}function pass({spelling} $wait): {spelling} {{ return $wait; }} \
                    function inspect({ty} $value): void {{ \
                    let $wait = {spelling}::{case}({parameter}: $value); \
                    let $copy = pass($wait); ?{spelling} $optional = $wait; \
                    pass($wait); if ($optional != null) {{ pass($optional); }} }}"
                );
                let snapshot = AnalysisSnapshot::analyze("wait-help.doria", &source);
                assert!(
                    snapshot.diagnostics().is_empty(),
                    "{:?}",
                    snapshot.diagnostics()
                );
                let type_offset = source.find(&format!("{spelling} $wait")).unwrap();
                let hover = snapshot.hover_at_offset(type_offset).unwrap();
                assert!(
                    hover.markdown.contains(&format!("enum {name}")),
                    "{hover:?}"
                );
                assert!(hover.markdown.contains("Nominal Copy enum"), "{hover:?}");
                assert!(snapshot.declaration_span_at_offset(type_offset).is_none());

                let offset = source.find(&format!("::{case}")).unwrap() + 2;
                let signature = format!("{name}::{case}({ty} ${parameter}): {name}");
                let hover = snapshot.hover_at_offset(offset).unwrap();
                assert!(hover.markdown.contains(&signature), "{hover:?}");
                assert!(
                    hover.markdown.contains("Constructs a Copy enum value"),
                    "{hover:?}"
                );
                assert!(snapshot.declaration_span_at_offset(offset).is_none());
                let argument = source.find(&format!("{parameter}:")).unwrap();
                let help = snapshot.signature_help_at_offset(argument).unwrap();
                assert_eq!(help[0].label, signature);
                assert_eq!(help[0].active_parameter, 0);
                assert!(snapshot
                    .named_argument_completions_at_offset(argument)
                    .unwrap()
                    .iter()
                    .any(|completion| completion.label == format!("{parameter}:")));
                let cases = snapshot.static_completions_at_offset(offset).unwrap();
                let labels = cases
                    .iter()
                    .map(|item| item.label.as_str())
                    .collect::<HashSet<_>>();
                assert_eq!(
                    labels,
                    HashSet::from(["Immediate", "For", "Until", "Indefinite"])
                );
                assert!(cases.iter().all(|item| item.kind == 20));
            }
        }
        let source = include_str!("../../editors/fixtures/stage36a-wait-values.doria");
        let snapshot = AnalysisSnapshot::analyze("wait-values.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
    }

    #[test]
    fn transfer_group_observed_keys_preserve_readonly_nullable_copy_results() {
        use doriac::compiler_known_stream::{
            READ_TRANSFER_KEY, TRANSFER_GROUP, WRITE_TRANSFER_KEY,
        };
        for (property, key) in [
            ("observedReadKey", READ_TRANSFER_KEY),
            ("observedWriteKey", WRITE_TRANSFER_KEY),
        ] {
            for (prefix, group_name, key_name) in [
                (String::new(), TRANSFER_GROUP, key),
                (
                    format!("use {TRANSFER_GROUP} as Group; use {key} as Key;"),
                    "Group",
                    "Key",
                ),
            ] {
                let source = format!(
                    "{prefix} function observe({group_name} $group): ?{key_name} {{ \
                    let $key = $group->{property}; let $copy = $key; return $key; }}"
                );
                let snapshot = AnalysisSnapshot::analyze("observed-transfer-key.doria", &source);
                assert!(
                    snapshot.diagnostics().is_empty(),
                    "{:?}",
                    snapshot.diagnostics()
                );
                let offset = source.find(&format!("->{property}")).unwrap() + 2;
                let hover = snapshot.hover_at_offset(offset).unwrap();
                assert!(
                    hover
                        .markdown
                        .contains(&format!("${property} {{ get: ?{key} }}")),
                    "{hover:?}"
                );
                for variable in ["$key", "$copy"] {
                    let binding = snapshot
                        .hover_at_offset(source.rfind(variable).unwrap())
                        .unwrap();
                    assert!(
                        binding.markdown.contains(&format!("?{key} {variable}")),
                        "{binding:?}"
                    );
                    assert!(
                        binding.markdown.contains(INLINE_COPY_DOCUMENTATION),
                        "{binding:?}"
                    );
                }
                let members = snapshot.member_completions_at_offset(offset).unwrap();
                assert!(members.iter().any(|item| item.label == property));
                let invalid = source
                    .replace(
                        &format!("{group_name} $group"),
                        &format!("writable {group_name} $group"),
                    )
                    .replace(
                        &format!("let $key = $group->{property};"),
                        &format!("$group->{property} = null; let $key = $group->{property};"),
                    );
                let (_, compiler) =
                    doriac::analyze_source_for_ide("readonly-observed-key.doria", &invalid)
                        .unwrap();
                assert!(!compiler.diagnostics.is_empty());
                let rejected = AnalysisSnapshot::analyze("readonly-observed-key.doria", &invalid);
                assert_eq!(rejected.diagnostics(), compiler.diagnostics);
            }
        }
    }

    #[test]
    fn transfer_group_read_region_uses_compiler_owned_nullable_borrow() {
        let group = "Doria\\Std\\Io\\TransferGroup";
        let key = doriac::compiler_known_stream::READ_TRANSFER_KEY;
        for (prefix, group_name, key_name) in [
            (String::new(), group, key),
            (
                format!("use {group} as Group; use {key} as ReadKey;"),
                "Group",
                "ReadKey",
            ),
        ] {
            let source = format!(
                "{prefix} function inspect({group_name} $group, {key_name} $key): void throws Doria\\Std\\Io\\IoError {{ \
                let $region = $group->getReadRegion(key: $key); \
                if ($region != null) {{ echo $region->length; }} }}"
            );
            let snapshot = AnalysisSnapshot::analyze("transfer-group-read-region.doria", &source);
            assert!(
                snapshot.diagnostics().is_empty(),
                "{:?}",
                snapshot.diagnostics()
            );
            let offset = source.find("getReadRegion").unwrap();
            let member = snapshot
                .class_member_surfaces
                .iter()
                .find(|surface| surface.receiver.name == group)
                .unwrap()
                .members
                .iter()
                .find(|member| member.name == "getReadRegion")
                .unwrap();
            assert!(
                member.return_borrow.is_some(),
                "compiler member surface omitted getReadRegion's retained receiver loan: {member:?}"
            );
            let signature =
                format!("function {group}::getReadRegion({key} $key): ?Doria\\Std\\Io\\ByteRegion");
            let hover = snapshot.hover_at_offset(offset).unwrap();
            assert!(hover.markdown.contains(&signature), "{hover:?}");
            assert!(hover.markdown.contains("**Ambient I/O:**"), "{hover:?}");
            assert!(
                hover.markdown.contains(
                    "Returns an owned value that retains a readonly borrow rooted in the receiver."
                ),
                "{hover:?}"
            );
            assert!(
                hover
                    .markdown
                    .contains("The source must remain available through the result's last use."),
                "{hover:?}"
            );
            let members = snapshot.member_completions_at_offset(offset).unwrap();
            assert_eq!(
                members
                    .iter()
                    .find(|item| item.label == "getReadRegion")
                    .unwrap()
                    .detail,
                signature
            );
            let argument = source.find("key:").unwrap();
            let help = snapshot.signature_help_at_offset(argument).unwrap();
            assert_eq!(help[0].label, signature);
            assert_eq!(help[0].active_parameter, 0);
            assert!(snapshot
                .named_argument_completions_at_offset(argument)
                .unwrap()
                .iter()
                .any(|item| item.label == "key:"));
            let binding = snapshot
                .hover_at_offset(source.find("$region").unwrap())
                .unwrap();
            assert!(
                binding
                    .markdown
                    .contains("?Doria\\Std\\Io\\ByteRegion $region"),
                "{binding:?}"
            );
            let narrowed = snapshot
                .member_completions_at_offset(source.find("length").unwrap())
                .unwrap();
            assert!(narrowed.iter().any(|item| item.label == "length"));
            assert!(!narrowed
                .iter()
                .any(|item| item.label == "getWritableRegion"));
            let last_use = format!(
                "{prefix} function change(writable {group_name} $group): void {{}} \
                function region({group_name} $group, {key_name} $key): ?Doria\\Std\\Io\\ByteRegion throws Doria\\Std\\Io\\IoError {{ return $group->getReadRegion($key); }} \
                function inspect(writable {group_name} $group, {key_name} $key): void throws Doria\\Std\\Io\\IoError {{ \
                let $region = region($group, $key); if ($region != null) {{ echo $region->length; }} change($group); }}"
            );
            let snapshot = AnalysisSnapshot::analyze("read-region-last-use.doria", &last_use);
            assert!(
                snapshot.diagnostics().is_empty(),
                "{:?}",
                snapshot.diagnostics()
            );
            for invalid in [
                last_use.replace("if ($region != null) { echo $region->length; } change($group);", "change($group); if ($region != null) { echo $region->length; }"),
                format!("{prefix} function escape(take {group_name} $group, {key_name} $key): ?Doria\\Std\\Io\\ByteRegion throws Doria\\Std\\Io\\IoError {{ return $group->getReadRegion($key); }}"),
            ] {
                let (_, compiler) = doriac::analyze_source_for_ide("invalid-read-region-loan.doria", &invalid).unwrap();
                assert!(!compiler.diagnostics.is_empty(), "{invalid}");
                let rejected = AnalysisSnapshot::analyze("invalid-read-region-loan.doria", &invalid);
                assert_eq!(rejected.diagnostics(), compiler.diagnostics);
            }
        }
    }

    #[test]
    fn transfer_group_fetch_preserves_writable_signatures_and_region_loans() {
        use doriac::compiler_known_stream::{
            READ_TRANSFER_KEY, TRANSFER_GROUP, TRANSFER_WAIT_RESULT,
        };
        let wait = "Doria\\Std\\Io\\Wait";
        for (prefix, group, key, wait_name, result) in [
            (String::new(), TRANSFER_GROUP, READ_TRANSFER_KEY, wait, TRANSFER_WAIT_RESULT),
            (
                format!("use {TRANSFER_GROUP} as Group; use {READ_TRANSFER_KEY} as Key; use {wait} as Wait; use {TRANSFER_WAIT_RESULT} as Result;"),
                "Group", "Key", "Wait", "Result",
            ),
        ] {
            let source = format!("{prefix} function inspect(writable {group} $group, {key} $key): {result} throws Doria\\Std\\Io\\IoError {{ \
                if ($group->getReadRegion($key) == null || $group->getReadRegion($key) == null || $group->getNextCompletion({wait_name}::Immediate) == {result}::Exhausted) {{}} \
                if ($group->getReadRegion($key) != null && $group->getReadRegion($key) != null && $group->getNextCompletion({wait_name}::Immediate) != {result}::Exhausted) {{}} \
                let $completion = $group->getNextCompletion(wait: {wait_name}::Immediate); \
                $group->resubmitRead($key, 0, 1); return $completion; }}");
            let snapshot = AnalysisSnapshot::analyze("group-fetch.doria", &source);
            assert!(snapshot.diagnostics().is_empty(), "{:?}", snapshot.diagnostics());
            let offset = source.find("getNextCompletion").unwrap();
            let signature = format!("writable function {TRANSFER_GROUP}::getNextCompletion({wait} $wait): {TRANSFER_WAIT_RESULT}");
            assert!(snapshot.hover_at_offset(offset).unwrap().markdown.contains(&signature));
            assert_eq!(
                snapshot.member_completions_at_offset(offset).unwrap().iter()
                    .find(|item| item.label == "getNextCompletion").unwrap().detail,
                signature
            );
            let argument = source.find("wait:").unwrap();
            let help = snapshot.signature_help_at_offset(argument).unwrap();
            assert_eq!(help[0].label, signature);
            assert_eq!(help[0].active_parameter, 0);
            assert!(snapshot.named_argument_completions_at_offset(argument).unwrap().iter()
                .any(|item| item.label == "wait:"));
            assert!(snapshot.hover_at_offset(source.find("$completion").unwrap()).unwrap()
                .markdown.contains(&format!("{TRANSFER_WAIT_RESULT} $completion")));
            let readonly = source.replace(&format!("writable {group} $group"), &format!("{group} $group"));
            let filtered = AnalysisSnapshot::analyze("readonly-group-fetch.doria", &readonly)
                .member_completions_at_offset(readonly.find("getNextCompletion").unwrap()).unwrap();
            assert!(!filtered.iter().any(|item| item.label == "getNextCompletion"));
            let conflict = format!("{prefix} function inspect(writable {group} $group, {key} $key): {result} throws Doria\\Std\\Io\\IoError {{ \
                let $region = $group->getReadRegion($key); let $completion = $group->getNextCompletion({wait_name}::Immediate); \
                if ($region != null) {{ echo $region->length; }} return $completion; }}");
            let argument_conflict = format!("{prefix} function consume(?Doria\\Std\\Io\\ByteRegion $region, bool $done): int {{ \
                if ($region != null) {{ return $region->length; }} return 0; }} \
                function inspect(writable {group} $group, {key} $key): void throws Doria\\Std\\Io\\IoError {{ \
                consume($group->getReadRegion($key), $group->getReadRegion($key) == null || $group->getNextCompletion({wait_name}::Immediate) == {result}::Exhausted); }}");
            for invalid in [readonly, conflict, argument_conflict] {
                let (_, compiler) = doriac::analyze_source_for_ide("invalid-group-fetch.doria", &invalid).unwrap();
                assert!(!compiler.diagnostics.is_empty(), "{invalid}");
                assert_eq!(AnalysisSnapshot::analyze("invalid-group-fetch.doria", &invalid).diagnostics(), compiler.diagnostics);
            }
        }
    }

    #[test]
    fn transfer_group_resubmission_preserves_writable_directional_range_contracts() {
        use doriac::compiler_known_stream::{
            READ_TRANSFER_KEY, TRANSFER_GROUP, WRITE_TRANSFER_KEY,
        };
        for (method, key, wrong_key) in [
            ("resubmitRead", READ_TRANSFER_KEY, WRITE_TRANSFER_KEY),
            ("resubmitWrite", WRITE_TRANSFER_KEY, READ_TRANSFER_KEY),
        ] {
            for (prefix, group_name, key_name) in [
                (String::new(), TRANSFER_GROUP, key),
                (
                    format!("use {TRANSFER_GROUP} as Group; use {key} as Key;"),
                    "Group",
                    "Key",
                ),
            ] {
                let source = format!("{prefix} function inspect(writable {group_name} $group, {key_name} $key): void throws Doria\\Std\\Io\\IoError {{ \
                    $group->{method}(key: $key, offset: 0, length: 1); $group->{method}($key, 0, 1); }}");
                let snapshot = AnalysisSnapshot::analyze("group-resubmit.doria", &source);
                assert!(
                    snapshot.diagnostics().is_empty(),
                    "{:?}",
                    snapshot.diagnostics()
                );
                let offset = source.find(&format!("->{method}")).unwrap() + 2;
                let signature = format!("writable function {TRANSFER_GROUP}::{method}({key} $key, int $offset, int $length): void");
                let hover = snapshot.hover_at_offset(offset).unwrap();
                assert!(hover.markdown.contains(&signature), "{hover:?}");
                assert!(hover.markdown.contains("**Ambient I/O:**"), "{hover:?}");
                let members = snapshot.member_completions_at_offset(offset).unwrap();
                assert_eq!(
                    members
                        .iter()
                        .find(|item| item.label == method)
                        .unwrap()
                        .detail,
                    signature
                );
                for (name, active) in [("key:", 0), ("offset:", 1), ("length:", 2)] {
                    let argument = source.find(name).unwrap();
                    let help = snapshot.signature_help_at_offset(argument).unwrap();
                    assert_eq!(help[0].label, signature);
                    assert_eq!(help[0].active_parameter, active);
                    assert!(snapshot
                        .named_argument_completions_at_offset(argument)
                        .unwrap()
                        .iter()
                        .any(|item| item.label == name));
                }
                let readonly = source.replace(
                    &format!("writable {group_name} $group"),
                    &format!("{group_name} $group"),
                );
                let filtered =
                    AnalysisSnapshot::analyze("readonly-group-resubmit.doria", &readonly)
                        .member_completions_at_offset(
                            readonly.find(&format!("->{method}")).unwrap() + 2,
                        )
                        .unwrap();
                assert!(!filtered
                    .iter()
                    .any(|item| matches!(item.label.as_str(), "resubmitRead" | "resubmitWrite")));
                let conflict = format!("{prefix} function conflict(writable {group_name} $group, {key_name} $key, {READ_TRANSFER_KEY} $readKey): void throws Doria\\Std\\Io\\IoError {{ \
                    let $region = $group->getReadRegion($readKey); $group->{method}($key, 0, 1); if ($region != null) {{ echo $region->length; }} }}");
                for invalid in [
                    readonly,
                    source.replace(&format!("{key_name} $key"), &format!("{wrong_key} $key")),
                    source.replace("offset: 0", "offset: 0.5"),
                    conflict,
                ] {
                    let (_, compiler) =
                        doriac::analyze_source_for_ide("invalid-group-resubmit.doria", &invalid)
                            .unwrap();
                    assert!(!compiler.diagnostics.is_empty(), "{invalid}");
                    let rejected =
                        AnalysisSnapshot::analyze("invalid-group-resubmit.doria", &invalid);
                    assert_eq!(rejected.diagnostics(), compiler.diagnostics);
                }
            }
        }
    }

    #[test]
    fn writable_file_operations_help_preserves_compiler_access_and_effects() {
        let source = include_str!("../../editors/fixtures/stage36a-seekable-files.doria");
        let snapshot = AnalysisSnapshot::analyze("file-synchronization.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        for (method, parameters, arguments) in [
            ("synchronizeData", "", ""),
            ("synchronizeMetadata", "", ""),
            ("setLength", "int $length", "0"),
        ] {
            for (offset, _) in source.match_indices(&format!("->{method}")) {
                let offset = offset + 2;
                let members = snapshot.member_completions_at_offset(offset).unwrap();
                let operation = members.iter().find(|item| item.label == method).unwrap();
                assert_eq!(operation.kind, 2);
                assert!(
                    operation.detail.starts_with("writable function ")
                        && operation
                            .detail
                            .contains(&format!("::{method}({parameters}): void")),
                    "{operation:?}"
                );
                assert!(!members.iter().any(|item| item.label == "truncate"));
                let hover = snapshot.hover_at_offset(offset).unwrap();
                assert!(hover.markdown.contains(&operation.detail), "{hover:?}");
                assert!(
                    hover.markdown.contains("Doria\\Std\\Io\\IoError"),
                    "{hover:?}"
                );
                let help = snapshot
                    .signature_help_at_offset(offset + method.len() + 1)
                    .unwrap();
                assert_eq!(help[0].label, operation.detail);
                assert!(snapshot.declaration_span_at_offset(offset).is_none());
            }
            for file in ["ReadableFile", "WritableFile", "DuplexFile"] {
                let invalid = format!("use Doria\\Std\\Io\\{{{file}, IoError}}; function inspect({file} $file): void throws IoError {{ $file->{method}({arguments}); }}");
                let (_, compiler) =
                    doriac::analyze_source_for_ide("invalid-file-synchronization.doria", &invalid)
                        .unwrap();
                assert!(!compiler.diagnostics.is_empty(), "{invalid}");
                let snapshot =
                    AnalysisSnapshot::analyze("invalid-file-synchronization.doria", &invalid);
                assert_eq!(snapshot.diagnostics(), compiler.diagnostics);
                let members = snapshot
                    .member_completions_at_offset(invalid.find(&format!("->{method}")).unwrap() + 2)
                    .unwrap();
                assert!(!members.iter().any(|item| matches!(
                    item.label.as_str(),
                    "synchronizeData" | "synchronizeMetadata" | "setLength"
                )));
            }
        }
        let readable = snapshot
            .member_completions_at_offset(
                source.find("$read->getPosition").unwrap() + "$read->".len(),
            )
            .unwrap();
        assert!(!readable.iter().any(|item| matches!(
            item.label.as_str(),
            "synchronizeData" | "synchronizeMetadata" | "setLength"
        )));
        let argument = source.find("length:").unwrap();
        let help = snapshot.signature_help_at_offset(argument).unwrap();
        assert_eq!(help[0].active_parameter, 0);
        assert!(snapshot
            .named_argument_completions_at_offset(argument)
            .unwrap()
            .iter()
            .any(|item| item.label == "length:"));
        for file in ["WritableFile", "DuplexFile"] {
            for arguments in ["", "length: \"invalid\""] {
                let invalid = format!("use Doria\\Std\\Io\\{{{file}, IoError}}; function inspect(writable {file} $file): void throws IoError {{ $file->setLength({arguments}); }}");
                let (_, compiler) =
                    doriac::analyze_source_for_ide("invalid-file-length.doria", &invalid).unwrap();
                assert!(!compiler.diagnostics.is_empty(), "{invalid}");
                let snapshot = AnalysisSnapshot::analyze("invalid-file-length.doria", &invalid);
                assert_eq!(snapshot.diagnostics(), compiler.diagnostics);
            }
        }
    }

    #[test]
    fn seekable_file_help_preserves_compiler_access_effects_and_named_arguments() {
        let source = include_str!("../../editors/fixtures/stage36a-seekable-files.doria");
        let snapshot = AnalysisSnapshot::analyze("seekable-files.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        for (offset, _) in source.match_indices("->seek") {
            let offset = offset + 2;
            let members = snapshot.member_completions_at_offset(offset).unwrap();
            let seek = members.iter().find(|item| item.label == "seek").unwrap();
            assert!(seek.detail.starts_with("writable function "), "{seek:?}");
            assert!(
                seek.detail
                    .contains("int $offset, Doria\\Std\\Io\\SeekOrigin $origin): int"),
                "{seek:?}"
            );
            let hover = snapshot.hover_at_offset(offset).unwrap();
            assert!(hover.markdown.contains(&seek.detail), "{hover:?}");
            assert!(
                hover.markdown.contains("Doria\\Std\\Io\\IoError"),
                "{hover:?}"
            );
            assert!(snapshot.declaration_span_at_offset(offset).is_none());
        }
        for (name, active) in [("offset:", 0), ("origin:", 1)] {
            let offset = source.find(name).unwrap();
            let help = snapshot.signature_help_at_offset(offset).unwrap();
            assert_eq!(help[0].active_parameter, active);
            assert!(snapshot
                .named_argument_completions_at_offset(offset)
                .unwrap()
                .iter()
                .any(|item| item.label == name));
        }
        for method in ["getPosition", "getLength"] {
            for (offset, _) in source.match_indices(&format!("->{method}")) {
                let offset = offset + 2;
                let members = snapshot.member_completions_at_offset(offset).unwrap();
                let inspection = members.iter().find(|item| item.label == method).unwrap();
                assert_eq!(inspection.kind, 2);
                assert!(
                    inspection.detail.starts_with("function ")
                        && inspection.detail.contains(&format!("::{method}(): int")),
                    "{inspection:?}"
                );
                assert!(!members
                    .iter()
                    .any(|item| matches!(item.label.as_str(), "position" | "length")));
                let hover = snapshot.hover_at_offset(offset).unwrap();
                assert!(hover.markdown.contains(&inspection.detail), "{hover:?}");
                assert!(
                    hover.markdown.contains("Doria\\Std\\Io\\IoError"),
                    "{hover:?}"
                );
                let help = snapshot
                    .signature_help_at_offset(offset + method.len() + 1)
                    .unwrap();
                assert_eq!(help[0].label, inspection.detail);
                assert!(snapshot.declaration_span_at_offset(offset).is_none());
            }
        }
        for case in ["Start", "Current", "End"] {
            let offset = source.find(&format!("Origin::{case}")).unwrap() + "Origin::".len();
            let cases = snapshot.static_completions_at_offset(offset).unwrap();
            assert_eq!(
                cases
                    .iter()
                    .filter(|item| item.kind == 20)
                    .map(|item| item.label.as_str())
                    .collect::<HashSet<_>>(),
                HashSet::from(["Start", "Current", "End"])
            );
        }
        for file in ["ReadableFile", "WritableFile", "DuplexFile"] {
            for invalid in [
                format!("use Doria\\Std\\Io\\{{{file}, SeekOrigin, IoError}}; function inspect({file} $file): void throws IoError {{ $file->seek(0, SeekOrigin::Start); }}"),
                format!("use Doria\\Std\\Io\\{{{file}, IoError}}; function inspect({file} $file): void throws IoError {{ let $position = $file->position; }}"),
                format!("use Doria\\Std\\Io\\{{{file}, IoError}}; function inspect({file} $file): void throws IoError {{ let $length = $file->length; }}"),
                format!("use Doria\\Std\\Io\\{{{file}, IoError}}; function inspect(writable {file} $file): void throws IoError {{ $file->seek(0); }}"),
            ] {
                let (_, compiler) = doriac::analyze_source_for_ide("invalid-seekable-file.doria", &invalid).unwrap();
                assert!(!compiler.diagnostics.is_empty(), "{invalid}");
                let snapshot = AnalysisSnapshot::analyze("invalid-seekable-file.doria", &invalid);
                assert_eq!(snapshot.diagnostics(), compiler.diagnostics);
            }
        }
    }

    #[test]
    fn transfer_group_controls_preserve_compiler_signature_and_named_arguments() {
        let source = include_str!("../../editors/fixtures/stage36a-transfer-group-controls.doria");
        let snapshot = AnalysisSnapshot::analyze("transfer-group-controls.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        let signature = "writable function Doria\\Std\\Io\\ReadinessSet::begin(take Doria\\Std\\Io\\TransferPlan $plan, ?Doria\\Std\\Time\\Deadline $deadline = null, ?Doria\\Std\\Concurrency\\CancellationToken $cancellation = null): Doria\\Std\\Io\\TransferGroup";
        let offset = source.find("->begin").unwrap() + 2;
        let members = snapshot.member_completions_at_offset(offset).unwrap();
        let begin = members.iter().find(|item| item.label == "begin").unwrap();
        assert_eq!(begin.detail, signature);
        assert!(begin
            .documentation
            .as_deref()
            .unwrap()
            .contains("Default arguments available for `$deadline`, `$cancellation`."));
        assert!(snapshot
            .hover_at_offset(offset)
            .unwrap()
            .markdown
            .contains(signature));
        assert!(snapshot.declaration_span_at_offset(offset).is_none());
        for (name, active) in [("deadline:", 1), ("cancellation:", 2)] {
            let argument = source.find(name).unwrap();
            let help = snapshot.signature_help_at_offset(argument).unwrap();
            assert_eq!(help[0].label, signature);
            assert_eq!(help[0].active_parameter, active);
            assert!(snapshot
                .named_argument_completions_at_offset(argument)
                .unwrap()
                .iter()
                .any(|item| item.label == name));
        }
    }

    #[test]
    fn transfer_group_observation_help_preserves_approved_contract() {
        let documentation =
            transfer_result_documentation(doriac::compiler_known_stream::TRANSFER_WAIT_RESULT)
                .unwrap();
        for expected in [
            "Doria\\Std\\Io\\TransferGroup",
            "readonly observedReadKey: ?ReadTransferKey",
            "readonly observedWriteKey: ?WriteTransferKey",
            "Only the fetched operation's direction is present",
            "both are null before the first fetch and after timeout or exhaustion",
            "original opaque stamp and mixed-plan operation position",
            "published before an original owned checked Error propagates",
            "If wait infrastructure fails alongside a preserved transfer completion or owned transfer error, the infrastructure error is reported first",
            "The original completion or owned error remains available for a subsequent fetch; it is not discarded, relabeled, or reconstructed",
            "preserved across resubmission",
            "Key copies retain no group, storage, or source loans or resources",
            "Observed keys do not replace successful completion payload keys",
            "A retired read's known completed prefix may be available through getReadRegion before fetching; observed keys are published only by fetching",
            "Fetching and cleanup do not repeat retired adapter callbacks",
            "An original queued IoError remains owned until fetched or group destruction",
            "begin consumes the plan on success and failure",
            "plan-validation and native-setup IoError use IoOperation::Configure / IoTarget::TransferGroup",
            "preserving reason and system-code details",
            "An adapter's original owned checked Error propagates unchanged, not relabeled or reconstructed",
            "Failed setup releases plan loans, leaves no active group, and submits no work",
            "The compiler declares writable getNextCompletion(Wait $wait): TransferWaitResult",
            "Tooling validation and pinned-compiler integration remain pending",
        ] {
            assert!(documentation.contains(expected), "{documentation}");
        }
        for rejected in ["TransferIdentity", "TransferSession"] {
            assert!(!documentation.contains(rejected), "{documentation}");
        }
    }

    #[test]
    fn transfer_result_enum_help_preserves_payloads_copy_and_nullable_outcomes() {
        use doriac::compiler_known_stream::{
            READ_TRANSFER_KEY, TRANSFER_COMPLETION, TRANSFER_WAIT_RESULT, WRITE_TRANSFER_KEY,
        };

        for (case, key, outcome, success) in [
            (
                "Read",
                READ_TRANSFER_KEY,
                "Doria\\Std\\Io\\ReadOutcome",
                "Data",
            ),
            (
                "Write",
                WRITE_TRANSFER_KEY,
                "Doria\\Std\\Io\\WriteOutcome",
                "Accepted",
            ),
        ] {
            for (prefix, event, result) in [
                (String::new(), TRANSFER_COMPLETION, TRANSFER_WAIT_RESULT),
                (format!("use {TRANSFER_COMPLETION} as Event; use {TRANSFER_WAIT_RESULT} as Observation;"), "Event", "Observation"),
            ] {
                let other_case = if case == "Read" { "Write" } else { "Read" };
                let mut source = format!(
                    r#"{prefix}
function passEvent({event} $event): {event} {{ return $event; }}
function passResult({result} $result): {result} {{ return $result; }}
function matchesOutcome({event} $completion, ?{outcome} $expected): bool {{
    return match ($completion) {{
        {event}::{case}($ignoredKey, $actual) => $actual == $expected && !($actual != $expected),
        {event}::{other_case}($ignoredKey, $actual) => false,
        {event}::Ready($ignoredWatch) => false,
    }};
}}
function inspect({key} $key, ?{outcome} $outcome): void {{
    let $completion = {event}::{case}(key: $key, outcome: $outcome);
    let $empty = {event}::{case}($key, null);
    let $zero = {event}::{case}($key, {outcome}::{success}(0));
    let $blocked = {event}::{case}($key, {outcome}::WouldBlock);
    matchesOutcome($completion, $outcome); matchesOutcome($empty, null);
    let $eventCopy = passEvent($completion); passEvent($completion);
    ?{event} $maybeEvent = $empty;
    if ($maybeEvent != null) {{ passEvent($maybeEvent); }}
    let $completed = {result}::Completed(completion: $completion);
    let $copy = passResult($completed); passResult($completed);
    ?{result} $optional = $completed;
    if ($optional != null) {{ passResult($optional); }}
    let $timeout = {result}::TimedOut;
    let $exhausted = {result}::Exhausted;
}}
"#
                );
                if case == "Read" {
                    source.push_str(&format!(
                        "function terminal({key} $key): void {{ \
                        {event}::Read($key, {outcome}::EndOfStream); \
                        {event}::Read($key, {outcome}::TimedOut); }}"
                    ));
                }
                let snapshot = AnalysisSnapshot::analyze("transfer-result-help.doria", &source);
                assert!(snapshot.diagnostics().is_empty(), "{:?}", snapshot.diagnostics());
                for (spelling, name, parameter) in [(event, TRANSFER_COMPLETION, "$event"), (result, TRANSFER_WAIT_RESULT, "$result")] {
                    let offset = source.find(&format!("{spelling} {parameter}")).unwrap();
                    let hover = snapshot.hover_at_offset(offset).unwrap();
                    assert!(hover.markdown.contains(&format!("enum {name}")), "{hover:?}");
                    assert!(hover.markdown.contains("Nominal Copy enum"), "{hover:?}");
                    assert!(hover.markdown.contains(transfer_result_documentation(name).unwrap()), "{hover:?}");
                    assert!(snapshot.declaration_span_at_offset(offset).is_none());
                }
                for (name, case, fields, parameter, active, labels) in [
                    (TRANSFER_COMPLETION, case, format!("{key} $key, ?{outcome} $outcome"), "outcome:", 1, vec!["Read", "Write", "Ready"]),
                    (TRANSFER_WAIT_RESULT, "Completed", format!("{TRANSFER_COMPLETION} $completion"), "completion:", 0, vec!["Completed", "TimedOut", "Exhausted"]),
                ] {
                    let offset = source.find(&format!("::{case}(")).unwrap() + 2;
                    let signature = format!("{name}::{case}({fields}): {name}");
                    let hover = snapshot.hover_at_offset(offset).unwrap();
                    assert!(hover.markdown.contains(&signature), "{hover:?}");
                    assert!(hover.markdown.contains("Constructs a Copy enum value"), "{hover:?}");
                    assert!(hover.markdown.contains(transfer_result_documentation(name).unwrap()), "{hover:?}");
                    let argument = source.find(parameter).unwrap();
                    let help = snapshot.signature_help_at_offset(argument).unwrap();
                    assert_eq!(help[0].label, signature);
                    assert_eq!(help[0].active_parameter, active);
                    assert!(snapshot.named_argument_completions_at_offset(argument).unwrap().iter().any(|item| item.label == parameter));
                    let completions = snapshot.static_completions_at_offset(offset).unwrap();
                    assert_eq!(completions.iter().map(|item| item.label.as_str()).collect::<HashSet<_>>(), labels.into_iter().collect());
                    assert!(completions.iter().all(|item| item.kind == 20 && item.documentation.as_ref().unwrap().contains(transfer_result_documentation(name).unwrap())));
                    assert!(snapshot.declaration_span_at_offset(offset).is_none());
                }
                for case in ["TimedOut", "Exhausted"] {
                    let offset = source.find(&format!("{result}::{case}")).unwrap() + result.len() + 2;
                    let hover = snapshot.hover_at_offset(offset).unwrap();
                    assert!(hover.markdown.contains(&format!("{TRANSFER_WAIT_RESULT}::{case}: {TRANSFER_WAIT_RESULT}")), "{hover:?}");
                    assert!(hover.markdown.contains(transfer_result_documentation(TRANSFER_WAIT_RESULT).unwrap()), "{hover:?}");
                }
                for (variable, ty) in [
                    ("$completion", TRANSFER_COMPLETION.to_string()),
                    ("$eventCopy", TRANSFER_COMPLETION.to_string()),
                    ("$maybeEvent", format!("?{event}")),
                    ("$completed", TRANSFER_WAIT_RESULT.to_string()),
                    ("$copy", TRANSFER_WAIT_RESULT.to_string()),
                    ("$optional", format!("?{result}")),
                    ("$actual", format!("?{outcome}")),
                ] {
                    let offset = if variable == "$completion" {
                        source.find("let $completion").unwrap() + "let ".len()
                    } else {
                        source.find(variable).unwrap()
                    };
                    let hover = snapshot.hover_at_offset(offset).unwrap();
                    assert!(hover.markdown.contains(&format!("{ty} {variable}")), "{hover:?}");
                }
                for invalid in [
                    source.replace("key: $key", "key: 0"),
                    source.replace("outcome: $outcome", "outcome: 1"),
                    source.replace("completion: $completion", "completion: $key"),
                    source.replace(&format!("?{outcome} $expected"), &format!("?{TRANSFER_WAIT_RESULT} $expected")),
                ] {
                    let (_, compiler) = doriac::analyze_source_for_ide("invalid-transfer-result.doria", &invalid).unwrap();
                    assert!(!compiler.diagnostics.is_empty(), "{invalid}");
                    let snapshot = AnalysisSnapshot::analyze("invalid-transfer-result.doria", &invalid);
                    assert_eq!(snapshot.diagnostics(), compiler.diagnostics);
                }
            }
        }
        for name in [
            "App\\TransferCompletion",
            "App\\TransferWaitResult",
            "TransferCompletion",
            "TransferWaitResult",
        ] {
            assert!(transfer_result_documentation(name).is_none());
        }
    }

    #[test]
    fn cancellation_source_help_uses_compiler_declarations_and_member_facts() {
        use doriac::compiler_known_concurrency::{
            CANCELLATION_ERROR, CANCELLATION_SOURCE, CANCELLATION_TOKEN,
        };

        for (prefix, source_name, token_name, error_name) in [
            (String::new(), CANCELLATION_SOURCE, CANCELLATION_TOKEN, CANCELLATION_ERROR),
            (format!("use {CANCELLATION_SOURCE} as Source; use {CANCELLATION_TOKEN} as Token; use {CANCELLATION_ERROR} as Failure; "), "Source", "Token", "Failure"),
        ] {
            let source = format!(
                "{prefix}function inspect(writable {source_name} $source, {token_name} $view): {token_name} throws {error_name} {{ \
                let writable $local = new {source_name}(); \
                let $token = $source->token; let $copy = $token; \
                let $requested = $source->cancellationRequested; $source->cancel(); return $copy; }}"
            );
            let snapshot = AnalysisSnapshot::analyze("cancellation-help.doria", &source);
            assert!(snapshot.diagnostics().is_empty(), "{:?}", snapshot.diagnostics());
            for (needle, expected) in [
                (format!("{source_name} $source"), format!("class {CANCELLATION_SOURCE}")),
                (format!("{token_name} $view"), CANCELLATION_TOKEN_DOCUMENTATION.to_string()),
            ] {
                let offset = source.find(&needle).unwrap();
                let hover = snapshot.hover_at_offset(offset).unwrap();
                assert!(hover.markdown.contains(&expected), "{hover:?}");
                assert!(snapshot.declaration_span_at_offset(offset).is_none());
            }
            let offset = source.find("->cancel").unwrap() + 2;
            let members = snapshot.member_completions_at_offset(offset).unwrap();
            for (name, detail) in [
                ("token", format!("$token {{ get: {CANCELLATION_TOKEN} }}")),
                ("cancellationRequested", "$cancellationRequested { get: bool }".to_string()),
                ("cancel", format!("writable function {CANCELLATION_SOURCE}::cancel(): void throws {CANCELLATION_ERROR}")),
            ] {
                let member = members.iter().find(|member| member.label == name).unwrap();
                assert_eq!(member.detail, detail);
                let needle = if member.kind == 2 { format!("->{name}(") } else { format!("->{name}") };
                let offset = source.find(&needle).unwrap() + 2;
                let hover = snapshot.hover_at_offset(offset).unwrap();
                assert!(hover.markdown.contains(&detail), "{hover:?}");
                assert!(snapshot.declaration_span_at_offset(offset).is_none());
                if member.kind == 10 {
                    assert!(member.documentation.as_deref().unwrap().contains("Readonly"));
                    assert!(!member.detail.contains("set("));
                } else {
                    let help = snapshot.signature_help_at_offset(offset + name.len() + 1).unwrap();
                    assert_eq!(help[0].label, detail);
                }
            }
            for unapproved in ["create", "getToken", "requestCancellation"] {
                assert!(!members.iter().any(|member| member.label == unapproved));
            }
            let readonly = source.replace(&format!("writable {source_name} $source"), &format!("{source_name} $source"));
            let filtered = AnalysisSnapshot::analyze("readonly-cancellation-help.doria", &readonly)
                .member_completions_at_offset(readonly.find("->cancel").unwrap() + 2).unwrap();
            assert!(!filtered.iter().any(|member| member.label == "cancel"));
            let constructor = source.find(&format!("new {source_name}(")).unwrap() + 4 + source_name.len() + 1;
            let help = snapshot.signature_help_at_offset(constructor).unwrap();
            assert!(help[0].label.contains(&format!("{CANCELLATION_SOURCE}::__construct()")), "{help:?}");

            for invalid in [
                format!("{prefix}function inspect({source_name} $source): void throws {error_name} {{ $source->cancel(); }}"),
                format!("{prefix}function inspect(writable {source_name} $source): void {{ $source->cancellationRequested = false; }}"),
                format!("{prefix}function inspect(writable {source_name} $source, {token_name} $token): void {{ $source->token = $token; }}"),
                format!("{prefix}function inspect(): void {{ let $token = new {token_name}(); }}"),
            ] {
                let snapshot = AnalysisSnapshot::analyze("invalid-cancellation-help.doria", &invalid);
                let (_, compiler) = doriac::analyze_source_for_ide("invalid-cancellation-help.doria", &invalid).unwrap();
                assert!(!compiler.diagnostics.is_empty(), "{invalid}");
                assert_eq!(snapshot.diagnostics(), compiler.diagnostics);
            }
        }
    }

    #[test]
    fn cancellation_token_comparison_help_preserves_shared_state_identity() {
        let source = include_str!("../../editors/fixtures/stage36a-cancellation.doria");
        let snapshot = AnalysisSnapshot::analyze("cancellation-comparison-help.doria", source);
        for needle in [
            "$first == $second",
            "$token != $other",
            "$optional == $token",
        ] {
            let hover = snapshot
                .hover_at_offset(source.find(needle).unwrap())
                .unwrap();
            assert!(
                hover.markdown.contains(CANCELLATION_TOKEN_DOCUMENTATION),
                "{hover:?}"
            );
            assert!(
                !hover.markdown.contains(NON_OWNING_INLINE_DOCUMENTATION),
                "{hover:?}"
            );
        }
    }

    #[test]
    fn terminal_values_and_watch_keys_explain_themselves_on_hover() {
        let source = include_str!("../../editors/fixtures/stage46-terminal.doria");
        let snapshot = AnalysisSnapshot::analyze("terminal.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        for (needle, documentation) in [
            ("$size->rows - 1", TERMINAL_SIZE_DOCUMENTATION),
            ("$cursor->column", TERMINAL_POSITION_DOCUMENTATION),
            ("$modifiers->control", KEY_MODIFIERS_DOCUMENTATION),
            ("$title);", TERMINAL_STYLE_DOCUMENTATION),
            ("$watch = $plan", WATCH_KEY_DOCUMENTATION),
        ] {
            let hover = snapshot
                .hover_at_offset(source.find(needle).unwrap())
                .unwrap();
            assert!(
                hover.markdown.contains(documentation),
                "{needle}: {hover:?}"
            );
            assert!(
                hover.markdown.contains(NON_OWNING_INLINE_DOCUMENTATION),
                "{needle}: {hover:?}"
            );
        }
    }

    #[test]
    fn standard_stream_accessors_and_copy_help_use_compiler_metadata() {
        use doriac::compiler_known_stream::{STANDARD_ERROR, STANDARD_INPUT, STANDARD_OUTPUT};
        let source = include_str!("../../editors/fixtures/stage36a-standard-streams.doria");
        let snapshot = AnalysisSnapshot::analyze("standard-streams.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        for (property, ty) in [
            ("input", STANDARD_INPUT),
            ("output", STANDARD_OUTPUT),
            ("error", STANDARD_ERROR),
        ] {
            let offset = source.find(&format!("Devices::{property}")).unwrap() + "Devices::".len();
            let members = snapshot.static_completions_at_offset(offset).unwrap();
            let getter = members.iter().find(|item| item.label == property).unwrap();
            assert_eq!(getter.kind, 10);
            assert_eq!(getter.detail, format!("static ${property} {{ get: {ty} }}"));
            assert!(!getter.detail.contains("set("));
            assert!(getter
                .documentation
                .as_deref()
                .unwrap()
                .contains("Static accessor"));
            let hover = snapshot.hover_at_offset(offset).unwrap();
            assert!(hover.markdown.contains(&getter.detail), "{hover:?}");
            assert!(hover.markdown.contains("Static accessor"), "{hover:?}");
            assert!(!hover.markdown.contains("Readonly receiver"), "{hover:?}");
            assert!(snapshot.declaration_span_at_offset(offset).is_none());
        }
        for needle in ["$inputCopy =", "$output =", "$error =", "$optional ="] {
            let hover = snapshot
                .hover_at_offset(source.find(needle).unwrap())
                .unwrap();
            assert!(
                hover.markdown.contains(STANDARD_STREAM_DOCUMENTATION),
                "{hover:?}"
            );
            assert!(
                hover.markdown.contains(NON_OWNING_INLINE_DOCUMENTATION),
                "{hover:?}"
            );
            assert!(
                !hover.markdown.contains(CANCELLATION_TOKEN_DOCUMENTATION),
                "{hover:?}"
            );
        }
        for ty in [STANDARD_INPUT, STANDARD_OUTPUT, STANDARD_ERROR] {
            for invalid in [
                format!("function inspect(): void {{ new {ty}(); }}"),
                format!("function inspect(writable {ty} $view): void {{ $view->close(); }}"),
            ] {
                let (_, compiler) =
                    doriac::analyze_source_for_ide("invalid-standard-stream.doria", &invalid)
                        .unwrap();
                assert!(!compiler.diagnostics.is_empty(), "{invalid}");
                let rejected = AnalysisSnapshot::analyze("invalid-standard-stream.doria", &invalid);
                assert_eq!(rejected.diagnostics(), compiler.diagnostics);
            }
        }
    }

    #[test]
    fn inline_copy_help_distinguishes_managed_cancellation_from_non_owning_values() {
        use doriac::inline_value::InlineValueKind;

        for kind in [
            InlineValueKind::Duration,
            InlineValueKind::Deadline,
            InlineValueKind::RegistrationKey,
            InlineValueKind::ReadTransferKey,
            InlineValueKind::WriteTransferKey,
            InlineValueKind::WatchKey,
            InlineValueKind::WatchSource,
            InlineValueKind::CancellationToken,
            InlineValueKind::StandardInput,
            InlineValueKind::StandardOutput,
            InlineValueKind::StandardError,
            InlineValueKind::Terminal(doriac::inline_value::TerminalValue::Size),
            InlineValueKind::Terminal(doriac::inline_value::TerminalValue::Position),
            InlineValueKind::Terminal(doriac::inline_value::TerminalValue::KeyModifiers),
            InlineValueKind::Terminal(doriac::inline_value::TerminalValue::Style),
        ] {
            let documentation = inline_value_documentation(kind);
            assert!(documentation.contains(INLINE_COPY_DOCUMENTATION));
            assert_eq!(
                documentation.contains(NON_OWNING_INLINE_DOCUMENTATION),
                !kind.requires_owned_cleanup(),
                "{kind:?}: {documentation}"
            );
            assert_eq!(
                documentation.contains(CANCELLATION_TOKEN_DOCUMENTATION),
                kind == InlineValueKind::CancellationToken,
                "{kind:?}: {documentation}"
            );
            assert_eq!(
                documentation.contains(STANDARD_STREAM_DOCUMENTATION),
                matches!(
                    kind,
                    InlineValueKind::StandardInput
                        | InlineValueKind::StandardOutput
                        | InlineValueKind::StandardError
                ),
                "{kind:?}: {documentation}"
            );
            if matches!(
                kind,
                InlineValueKind::StandardInput
                    | InlineValueKind::StandardOutput
                    | InlineValueKind::StandardError
            ) {
                assert!(!kind.requires_owned_cleanup());
            }
        }
        assert!(InlineValueKind::CancellationToken.requires_owned_cleanup());
        for claim in [
            "retains one shared state claim",
            "releases it during ordinary cleanup",
            "Borrowed arguments acquire no ownership claim",
            "allocate no second state or native signal",
            "Tokens may outlive their source",
            "destroying its CancellationSource, do not request cancellation",
            "`==` compares shared cancellation-state identity; `!=` is its inverse",
            "distinct live states differ",
            "Ordinary nullable equality applies",
            "Borrowed comparison acquires no ownership claim",
            "no ordering, hashing, integer conversion, or address API",
        ] {
            assert!(CANCELLATION_TOKEN_DOCUMENTATION.contains(claim), "{claim}");
        }
    }

    #[test]
    fn user_cancellation_token_name_does_not_grant_standard_copy_semantics() {
        let source = "namespace App; class CancellationToken {} \
            function consume(take CancellationToken $value): void {} \
            function inspect(CancellationToken $token): void { \
            let $second = $token; consume($token); }";
        for namespace in ["App", "App\\Concurrency"] {
            let source = source.replace("namespace App;", &format!("namespace {namespace};"));
            let snapshot = AnalysisSnapshot::analyze("user-cancellation-token.doria", &source);
            let (_, compiler) =
                doriac::analyze_source_for_ide("user-cancellation-token.doria", &source).unwrap();
            assert!(!compiler.diagnostics.is_empty());
            assert_eq!(snapshot.diagnostics(), compiler.diagnostics);
            assert!(doriac::inline_value::InlineValueKind::for_name(&format!(
                "{namespace}\\CancellationToken"
            ))
            .is_none());
            let hover = snapshot
                .hover_at_offset(source.find("CancellationToken").unwrap())
                .unwrap();
            assert!(
                hover.markdown.contains("class CancellationToken"),
                "{hover:?}"
            );
            assert!(
                !hover.markdown.contains(INLINE_COPY_DOCUMENTATION),
                "{hover:?}"
            );
        }
    }

    #[test]
    fn inline_copy_help_preserves_nominal_identity_and_opaque_construction() {
        for name in [
            doriac::compiler_known_time::DURATION,
            doriac::compiler_known_time::DEADLINE,
            doriac::compiler_known_stream::READ_TRANSFER_KEY,
            doriac::compiler_known_stream::WRITE_TRANSFER_KEY,
        ] {
            for (prefix, spelling) in [
                (String::new(), name),
                (format!("use {name} as InlineValue; "), "InlineValue"),
            ] {
                let source = format!(
                    "{prefix}function pass({spelling} $value): {spelling} {{ return $value; }} \
                    function inspect({spelling} $original): void {{ \
                    let $copy = $original; pass($original); ?{spelling} $optional = $original; \
                    pass($original); if ($optional != null) {{ pass($optional); }} }}"
                );
                let snapshot = AnalysisSnapshot::analyze("inline-copy-help.doria", &source);
                assert!(
                    snapshot.diagnostics().is_empty(),
                    "{:?}",
                    snapshot.diagnostics()
                );
                let type_offset =
                    source.find(&format!("{spelling} $original")).unwrap() + spelling.len() - 1;
                let hover = snapshot.hover_at_offset(type_offset).unwrap();
                assert!(hover.markdown.contains(name), "{hover:?}");
                let documentation = inline_value_documentation(
                    doriac::inline_value::InlineValueKind::for_name(name).unwrap(),
                );
                assert!(hover.markdown.contains(&documentation), "{hover:?}");
                assert!(
                    hover.markdown.contains(INLINE_COPY_DOCUMENTATION),
                    "{hover:?}"
                );
                assert_eq!(
                    hover.markdown.contains(DURATION_NUMERIC_DOCUMENTATION),
                    name == doriac::compiler_known_time::DURATION,
                    "{hover:?}"
                );
                assert_eq!(
                    hover.markdown.contains(DEADLINE_DOCUMENTATION),
                    name == doriac::compiler_known_time::DEADLINE,
                    "{hover:?}"
                );
                assert!(!hover.markdown.contains("class "), "{hover:?}");
                assert!(snapshot.declaration_span_at_offset(type_offset).is_none());
                assert!(snapshot
                    .semantic_token_spans()
                    .iter()
                    .any(|(span, kind, _)| span_contains(*span, type_offset) && *kind == 1));
                for variable in ["$copy", "$original", "$optional"] {
                    let hover = snapshot
                        .hover_at_offset(source.rfind(variable).unwrap())
                        .unwrap();
                    assert!(hover.markdown.contains(&documentation), "{hover:?}");
                    assert!(
                        hover.markdown.contains(INLINE_COPY_DOCUMENTATION),
                        "{hover:?}"
                    );
                    assert_eq!(
                        hover.markdown.contains(DURATION_NUMERIC_DOCUMENTATION),
                        name == doriac::compiler_known_time::DURATION,
                        "{hover:?}"
                    );
                    assert_eq!(
                        hover.markdown.contains(DEADLINE_DOCUMENTATION),
                        name == doriac::compiler_known_time::DEADLINE,
                        "{hover:?}"
                    );
                }
                let source = format!("{prefix}function inspect(): void {{ new {spelling}(); }}");
                let snapshot = AnalysisSnapshot::analyze("opaque-copy-construction.doria", &source);
                assert!(
                    snapshot
                        .diagnostics()
                        .iter()
                        .any(|diagnostic| diagnostic.code == "E0307"),
                    "{:?}",
                    snapshot.diagnostics()
                );
                assert!(snapshot
                    .signature_help_at_offset(source.find("();").unwrap() + 1)
                    .is_none());
            }
            let short = name.rsplit('\\').next().unwrap();
            let source = format!(
                "namespace App; class {short} {{ function __construct(string $label) {{}} }} \
                function inspect(): void {{ let $value = new {short}(\"owned\"); }}"
            );
            let snapshot = AnalysisSnapshot::analyze("user-inline-copy-name.doria", &source);
            assert!(
                snapshot.diagnostics().is_empty(),
                "{:?}",
                snapshot.diagnostics()
            );
            let hover = snapshot
                .hover_at_offset(source.find(short).unwrap())
                .unwrap();
            assert!(
                hover.markdown.contains(&format!("class {short}")),
                "{hover:?}"
            );
            assert!(!hover.markdown.contains(INLINE_COPY_DOCUMENTATION));
            assert!(!hover.markdown.contains(DURATION_NUMERIC_DOCUMENTATION));
            assert!(snapshot
                .signature_help_at_offset(source.find("\"owned\"").unwrap())
                .is_some());
        }
    }

    #[test]
    fn transfer_plan_help_preserves_mixed_generic_signatures() {
        use doriac::compiler_known_stream::TRANSFER_PLAN;

        for (prefix, plan) in [
            (String::new(), TRANSFER_PLAN),
            (format!("use {TRANSFER_PLAN} as Plan;"), "Plan"),
        ] {
            let source = format!(
                r#"{prefix}
use Doria\Std\Io\{{PendingReadableStream, PendingWritableStream, RegistrationKey,
    WritableByteRegion, ByteRegion, IoError}};
function readThenWrite<R implements PendingReadableStream, W implements PendingWritableStream>(
    RegistrationKey<R> $readKey, writable WritableByteRegion $destination,
    RegistrationKey<W> $writeKey, ByteRegion $source
): {plan} throws IoError {{
    return {plan}::read(key: $readKey, destination: $destination)
        ->andWriteTo(key: $writeKey, source: $source);
}}
function writeThenRead<R implements PendingReadableStream, W implements PendingWritableStream>(
    RegistrationKey<R> $readKey, writable WritableByteRegion $destination,
    RegistrationKey<W> $writeKey, ByteRegion $source
): {plan} throws IoError {{
    return {plan}::writeTo(key: $writeKey, source: $source)
        ->andReadFrom(key: $readKey, destination: $destination);
}}
"#
            );
            let snapshot = AnalysisSnapshot::analyze("mixed-plan-help.doria", &source);
            assert!(
                snapshot.diagnostics().is_empty(),
                "{:?}",
                snapshot.diagnostics()
            );
            for (method, mode, bound, key_type, region, region_type) in [
                (
                    "read",
                    "static",
                    "PendingReadableStream",
                    "R",
                    "destination",
                    "writable Doria\\Std\\Io\\WritableByteRegion",
                ),
                (
                    "andReadFrom",
                    "consuming",
                    "PendingReadableStream",
                    "R",
                    "destination",
                    "writable Doria\\Std\\Io\\WritableByteRegion",
                ),
                (
                    "writeTo",
                    "static",
                    "PendingWritableStream",
                    "W",
                    "source",
                    "Doria\\Std\\Io\\ByteRegion",
                ),
                (
                    "andWriteTo",
                    "consuming",
                    "PendingWritableStream",
                    "W",
                    "source",
                    "Doria\\Std\\Io\\ByteRegion",
                ),
            ] {
                let offset = source.find(&format!("{method}(key:")).unwrap();
                let hover = snapshot.hover_at_offset(offset).unwrap();
                assert!(hover.markdown.contains(&format!("{mode} function {TRANSFER_PLAN}::{method}<T implements Doria\\Std\\Io\\{bound}>")), "{hover:?}");
                assert!(hover.markdown.contains(&format!("(Doria\\Std\\Io\\RegistrationKey<{key_type}> $key, {region_type} ${region}): {TRANSFER_PLAN}")), "{hover:?}");
                assert!(
                    hover.markdown.contains("Doria\\Std\\Io\\IoError"),
                    "{hover:?}"
                );
                let argument = offset + source[offset..].find(&format!("{region}:")).unwrap();
                let help = snapshot.signature_help_at_offset(argument).unwrap();
                assert!(hover.markdown.contains(&help[0].label), "{help:?}");
                assert_eq!(help[0].active_parameter, 1);
                assert!(snapshot
                    .named_argument_completions_at_offset(argument)
                    .unwrap()
                    .iter()
                    .any(|item| item.label == format!("{region}:")));
                let members = if mode == "static" {
                    snapshot.static_completions_at_offset(offset).unwrap()
                } else {
                    snapshot.member_completions_at_offset(offset).unwrap()
                };
                assert!(
                    members.iter().any(|item| item.label == method),
                    "{members:?}"
                );
                assert!(
                    !members.iter().any(|item| matches!(
                        item.label.as_str(),
                        "andRead" | "readFrom" | "write" | "andWrite"
                    )),
                    "{members:?}"
                );
                assert!(snapshot.declaration_span_at_offset(offset).is_none());
            }
            for (approved, rejected) in [
                ("::read(", "::readFrom("),
                ("->andReadFrom(", "->andRead("),
                ("::writeTo(", "::write("),
                ("->andWriteTo(", "->andWrite("),
            ] {
                let invalid = source.replace(approved, rejected);
                let (_, compiler) =
                    doriac::analyze_source_for_ide("rejected-plan-name.doria", &invalid).unwrap();
                assert!(!compiler.diagnostics.is_empty(), "{invalid}");
                let snapshot = AnalysisSnapshot::analyze("rejected-plan-name.doria", &invalid);
                assert_eq!(snapshot.diagnostics(), compiler.diagnostics);
            }
        }
    }

    #[test]
    fn transfer_plan_lookup_help_preserves_readonly_access_and_copy_results() {
        use doriac::compiler_known_stream::{READ_TRANSFER_KEY, TRANSFER_PLAN, WRITE_TRANSFER_KEY};

        for (method, key, documentation) in [
            (
                "getReadKey",
                READ_TRANSFER_KEY,
                READ_TRANSFER_KEY_DOCUMENTATION,
            ),
            (
                "getWriteKey",
                WRITE_TRANSFER_KEY,
                WRITE_TRANSFER_KEY_DOCUMENTATION,
            ),
        ] {
            for (prefix, plan, result) in [
                (String::new(), TRANSFER_PLAN, key),
                (
                    format!("use {TRANSFER_PLAN} as Plan; use {key} as OperationKey;"),
                    "Plan",
                    "OperationKey",
                ),
            ] {
                let source = format!(
                    "{prefix} function discard(take {plan} $plan): void {{}} \
                    function lookup(take {plan} $plan): {result} throws Doria\\Std\\Io\\IoError {{ \
                    let $key = $plan->{method}(position: 0); let $copy = $key; \
                    discard($plan); return $copy; }}"
                );
                let snapshot = AnalysisSnapshot::analyze("transfer-key-lookup.doria", &source);
                assert!(
                    snapshot.diagnostics().is_empty(),
                    "{:?}",
                    snapshot.diagnostics()
                );
                let offset = source.find(&format!("->{method}")).unwrap() + 2;
                let signature = format!("function {TRANSFER_PLAN}::{method}(int $position): {key}");
                let hover = snapshot.hover_at_offset(offset).unwrap();
                assert!(hover.markdown.contains(&signature), "{hover:?}");
                assert!(hover.markdown.contains("**Ambient I/O:**"), "{hover:?}");
                assert!(!hover.markdown.contains("borrow rooted in"), "{hover:?}");
                let members = snapshot.member_completions_at_offset(offset).unwrap();
                let completion = members.iter().find(|item| item.label == method).unwrap();
                assert_eq!(completion.detail, signature);
                let position = source.find("position:").unwrap();
                let help = snapshot.signature_help_at_offset(position).unwrap();
                assert_eq!(help[0].label, signature);
                assert_eq!(help[0].active_parameter, 0);
                assert!(snapshot
                    .named_argument_completions_at_offset(position)
                    .unwrap()
                    .iter()
                    .any(|item| item.label == "position:"));
                for variable in ["$key", "$copy"] {
                    let hover = snapshot
                        .hover_at_offset(source.rfind(variable).unwrap())
                        .unwrap();
                    assert!(hover.markdown.contains(key), "{hover:?}");
                    assert!(
                        hover.markdown.contains(INLINE_COPY_DOCUMENTATION),
                        "{hover:?}"
                    );
                    assert!(hover.markdown.contains(documentation), "{hover:?}");
                    assert!(
                        hover
                            .markdown
                            .contains(TRANSFER_KEY_LOOKUP_FAILURE_DOCUMENTATION),
                        "{hover:?}"
                    );
                    assert!(
                        !hover.markdown.contains(REGISTRATION_KEY_DOCUMENTATION),
                        "{hover:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn registration_key_binding_help_preserves_generic_copy_identity() {
        let source = include_str!("../../editors/fixtures/stage36a-registration-keys.doria");
        let snapshot = AnalysisSnapshot::analyze("registration-keys.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        for (variable, ty) in [
            (
                "$returned",
                "Doria\\Std\\Io\\RegistrationKey<Doria\\Std\\Io\\ReadableFile>",
            ),
            (
                "$optionalCopy",
                "?Doria\\Std\\Io\\RegistrationKey<Doria\\Std\\Io\\ReadableFile>",
            ),
            (
                "$writableCopy",
                "?Doria\\Std\\Io\\RegistrationKey<Doria\\Std\\Io\\WritableFile>",
            ),
        ] {
            let hover = snapshot
                .hover_at_offset(source.rfind(variable).unwrap())
                .unwrap();
            assert!(hover.markdown.contains(ty), "{hover:?}");
            assert!(
                hover.markdown.contains(INLINE_COPY_DOCUMENTATION),
                "{hover:?}"
            );
            assert!(
                hover.markdown.contains(REGISTRATION_KEY_DOCUMENTATION),
                "{hover:?}"
            );
            assert!(
                !hover.markdown.contains(DURATION_NUMERIC_DOCUMENTATION),
                "{hover:?}"
            );
        }
    }

    #[test]
    fn readiness_set_help_uses_compiler_generic_signatures_and_effects() {
        use doriac::compiler_known_stream::{
            READABLE_FILE, READINESS_SET, READINESS_STREAM, REGISTRATION_KEY,
        };

        for (prefix, spelling) in [
            (String::new(), READINESS_SET),
            (format!("use {READINESS_SET} as Registry; "), "Registry"),
        ] {
            let construction = format!("{prefix}function create(): void {{ new {spelling}(); }}");
            let snapshot =
                AnalysisSnapshot::analyze("readiness-set-construction.doria", &construction);
            assert!(
                snapshot.diagnostics().is_empty(),
                "{:?}",
                snapshot.diagnostics()
            );
            let constructor = snapshot
                .signature_help_at_offset(construction.find("();").unwrap() + 1)
                .unwrap();
            assert!(constructor[0]
                .label
                .contains(&format!("{READINESS_SET}::__construct()")));
            assert!(!constructor[0].label.contains("throws"), "{constructor:?}");
            let setup = snapshot
                .hover_at_offset(construction.find("create").unwrap())
                .unwrap();
            assert!(setup.markdown.contains("**Ambient I/O:**"), "{setup:?}");
            assert!(
                setup.markdown.contains("`Doria\\Std\\Io\\IoError`"),
                "{setup:?}"
            );

            let source = format!(
                "{construction} function relay(writable {spelling} $set, take {READABLE_FILE} $stream): {READABLE_FILE} {{ \
                let $key = $set->register(stream: $stream); let $copy = $key; \
                let $returned = $set->unregister(key: $copy); return $returned; }}"
            );
            let snapshot = AnalysisSnapshot::analyze("readiness-set-help.doria", &source);
            assert!(
                snapshot.diagnostics().is_empty(),
                "{:?}",
                snapshot.diagnostics()
            );

            for (method, parameter, parameter_type, result) in [
                (
                    "register",
                    "stream",
                    format!("take {READABLE_FILE}"),
                    format!("{REGISTRATION_KEY}<{READABLE_FILE}>"),
                ),
                (
                    "unregister",
                    "key",
                    format!("{REGISTRATION_KEY}<{READABLE_FILE}>"),
                    READABLE_FILE.to_string(),
                ),
            ] {
                let offset = source.find(&format!("->{method}")).unwrap() + 2;
                let members = snapshot.member_completions_at_offset(offset).unwrap();
                let completion = members
                    .iter()
                    .find(|member| member.label == method)
                    .unwrap();
                assert_eq!(completion.kind, 2);
                assert!(
                    completion
                        .detail
                        .starts_with(&format!("writable function {READINESS_SET}::{method}<T")),
                    "{completion:?}"
                );
                if method == "register" {
                    assert!(
                        completion
                            .detail
                            .contains(&format!("T implements {READINESS_STREAM}")),
                        "{completion:?}"
                    );
                }
                let hover = snapshot.hover_at_offset(offset).unwrap();
                assert!(
                    hover
                        .markdown
                        .contains(&format!("({parameter_type} ${parameter}): {result}")),
                    "{hover:?}"
                );
                assert!(hover.markdown.contains("**Ambient I/O:**"), "{hover:?}");
                assert!(
                    hover.markdown.contains("`Doria\\Std\\Io\\IoError`"),
                    "{hover:?}"
                );
                assert!(!completion.detail.contains("throws"), "{completion:?}");
                assert!(snapshot.declaration_span_at_offset(offset).is_none());
                let argument = source.find(&format!("{parameter}:")).unwrap();
                let help = snapshot.signature_help_at_offset(argument).unwrap();
                assert!(hover.markdown.contains(&help[0].label), "{help:?}");
                assert_eq!(help[0].active_parameter, 0);
                assert!(snapshot
                    .named_argument_completions_at_offset(argument)
                    .unwrap()
                    .iter()
                    .any(|item| item.label == format!("{parameter}:")));
            }
            for (variable, ty) in [
                ("$copy", format!("{REGISTRATION_KEY}<{READABLE_FILE}>")),
                ("$returned", READABLE_FILE.to_string()),
            ] {
                let hover = snapshot
                    .hover_at_offset(source.rfind(variable).unwrap())
                    .unwrap();
                assert!(hover.markdown.contains(&ty), "{hover:?}");
                assert_eq!(
                    hover.markdown.contains(INLINE_COPY_DOCUMENTATION),
                    variable == "$copy",
                    "{hover:?}"
                );
            }
            let readonly = source.replace("writable ", "");
            let snapshot = AnalysisSnapshot::analyze("readonly-set-help.doria", &readonly);
            let members = snapshot
                .member_completions_at_offset(readonly.find("->register").unwrap() + 2)
                .unwrap();
            assert!(!members
                .iter()
                .any(|member| matches!(member.label.as_str(), "register" | "unregister")));
        }
        let source = include_str!("../../editors/fixtures/stage36a-readiness-set.doria");
        let snapshot = AnalysisSnapshot::analyze("readiness-set-types.doria", source);
        let types = [READABLE_FILE, READINESS_STREAM, "T", "ReadyOnly"];
        for ((offset, _), ty) in source.match_indices("->register(").zip(types) {
            let hover = snapshot.hover_at_offset(offset + 2).unwrap();
            assert!(
                hover
                    .markdown
                    .contains(&format!("(take {ty} $stream): {REGISTRATION_KEY}<{ty}>")),
                "{hover:?}"
            );
        }
        for ((offset, _), ty) in source
            .match_indices("->unregister(")
            .zip(types.into_iter().chain([READABLE_FILE]))
        {
            let hover = snapshot.hover_at_offset(offset + 2).unwrap();
            assert!(
                hover
                    .markdown
                    .contains(&format!("({REGISTRATION_KEY}<{ty}> $key): {ty}")),
                "{hover:?}"
            );
        }
    }

    #[test]
    fn readiness_set_borrowing_help_preserves_type_access_and_receiver_loans() {
        use doriac::compiler_known_stream::{READABLE_FILE, READINESS_SET, REGISTRATION_KEY};

        let source = include_str!("../../editors/fixtures/stage36a-readiness-set-borrowing.doria");
        let snapshot = AnalysisSnapshot::analyze("readiness-set-borrowing-help.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        for (method, writable) in [("getStream", false), ("getWritableStream", true)] {
            let offset = source.find(&format!("->{method}(")).unwrap() + 2;
            let completion = snapshot
                .member_completions_at_offset(offset)
                .unwrap()
                .into_iter()
                .find(|member| member.label == method)
                .unwrap();
            assert_eq!(completion.kind, 2);
            assert!(
                completion
                    .detail
                    .contains(&format!("function {READINESS_SET}::{method}<T")),
                "{completion:?}"
            );
            assert_eq!(
                completion.detail.starts_with("writable "),
                writable,
                "{completion:?}"
            );
            let hover = snapshot.hover_at_offset(offset).unwrap();
            assert!(
                hover.markdown.contains(&format!(
                    "({REGISTRATION_KEY}<{READABLE_FILE}> $key): {READABLE_FILE}"
                )),
                "{hover:?}"
            );
            let access = if writable { "writable" } else { "readonly" };
            assert!(
                hover.markdown.contains(&format!(
                    "Returns a {access} borrow rooted in the receiver."
                )),
                "{hover:?}"
            );
            let argument = offset + method.len() + 1;
            let help = snapshot.signature_help_at_offset(argument).unwrap();
            assert!(hover.markdown.contains(&help[0].label), "{help:?}");
            assert_eq!(help[0].active_parameter, 0);
            assert!(snapshot
                .named_argument_completions_at_offset(argument)
                .unwrap()
                .iter()
                .any(|item| item.label == "key:"));
            assert!(snapshot.declaration_span_at_offset(offset).is_none());
        }
        let readonly = source.find("->getStream(").unwrap() + 2;
        let members = snapshot.member_completions_at_offset(readonly).unwrap();
        assert!(members.iter().any(|member| member.label == "getStream"));
        assert!(!members
            .iter()
            .any(|member| member.label == "getWritableStream"));
        for ((offset, _), ty) in source.match_indices("->getStream(").zip([
            READABLE_FILE,
            "Doria\\Std\\Io\\ReadinessStream",
            "T",
            READABLE_FILE,
        ]) {
            let hover = snapshot.hover_at_offset(offset + 2).unwrap();
            assert!(
                hover
                    .markdown
                    .contains(&format!("({REGISTRATION_KEY}<{ty}> $key): {ty}")),
                "{hover:?}"
            );
        }
        let read = source.find("$stream->read(").unwrap();
        assert!(snapshot
            .hover_at_offset(read)
            .unwrap()
            .markdown
            .contains(&format!("writable {READABLE_FILE} $stream")));
        assert!(snapshot
            .member_completions_at_offset(read + "$stream->".len())
            .unwrap()
            .iter()
            .any(|member| member.label == "read"));
    }

    #[test]
    fn borrowed_local_help_and_member_access_follow_compiler_binding_facts() {
        let source = include_str!("../../editors/fixtures/stage36a-writable-borrowed-locals.doria");
        let name = "writable-borrowed-locals-help.doria";
        let (_, compiler) = doriac::analyze_source_for_ide(name, source).unwrap();
        assert!(
            compiler.diagnostics.is_empty(),
            "{:?}",
            compiler.diagnostics
        );
        let snapshot = AnalysisSnapshot::analyze(name, source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        for name in [
            "method", "function", "alias", "generic", "property", "readonly",
        ] {
            let writable = !matches!(name, "readonly" | "property");
            let declaration = compiler
                .info
                .binding_resolution
                .declarations_by_id
                .values()
                .find(|binding| binding.kind == BindingKind::Local && binding.name == name)
                .unwrap();
            assert_eq!(
                declaration.ownership,
                if writable {
                    BindingOwnership::WritableBorrow
                } else {
                    BindingOwnership::ReadonlyBorrow
                }
            );
            let expected = format!("{}Counter ${name}", if writable { "writable " } else { "" });
            for (offset, _) in source.match_indices(&format!("${name}")) {
                let hover = snapshot.hover_at_offset(offset).unwrap();
                assert!(hover.markdown.contains(&expected), "{name}: {hover:?}");
                if !writable {
                    assert!(!hover.markdown.contains("writable Counter"), "{hover:?}");
                }
            }
            let member = source.find(&format!("${name}->")).unwrap() + name.len() + 3;
            let completions = snapshot.member_completions_at_offset(member).unwrap();
            assert_eq!(
                completions.iter().any(|member| member.label == "bump"),
                writable,
                "{name}: {completions:?}"
            );
        }
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
    fn conditional_clauses_preserve_declarations_without_unconditional_member_fallbacks() {
        let source = include_str!("../../editors/fixtures/stage36a-conditional-capabilities.doria");
        let snapshot = AnalysisSnapshot::analyze("conditional-capabilities.doria", source);
        let (_, compiler) =
            doriac::analyze_source_for_ide("conditional-capabilities.doria", source).unwrap();
        assert_eq!(snapshot.diagnostics(), compiler.diagnostics);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        assert_eq!(compiler.info.contracts.conditions.len(), 8);
        assert!(!compiler.info.callable_signatures.is_empty());
        for (offset, _) in source.match_indices("where (") {
            assert!(snapshot
                .semantic_token_spans()
                .iter()
                .any(|(span, kind, _)| {
                    span.start == offset
                        && span.end == offset + "where".len()
                        && *kind == SEMANTIC_TOKEN_KEYWORD
                }));
            assert!(snapshot
                .hover_at_offset(offset)
                .unwrap()
                .markdown
                .contains("Local nominal capability condition"));
        }
        for needle in [
            "function ready(): int where",
            "int $state where",
            "static function inspect<U>",
        ] {
            let offset = source.find(needle).unwrap();
            let name_offset = if needle.starts_with("int ") {
                offset + "int ".len()
            } else {
                offset + source[offset..].find("function ").unwrap() + "function ".len()
            };
            assert!(snapshot
                .hover_at_offset(name_offset)
                .unwrap()
                .markdown
                .contains("where ("));
        }
        let adapter = snapshot
            .class_members
            .iter()
            .find(|(name, _)| name.ends_with("Adapter"))
            .unwrap()
            .1;
        assert!(adapter.iter().all(|member| {
            !matches!(
                member.completion.label.as_str(),
                "ready" | "prepare" | "state" | "inspect"
            )
        }));
        assert!(snapshot.global_symbols.references.iter().any(|reference| {
            reference.import_alias.as_deref() == Some("Capable")
                && &source[reference.source_span.start..reference.source_span.end] == "Capable"
        }));
        assert!(snapshot
            .hover_at_offset(source.find("Adapter<T>").unwrap())
            .unwrap()
            .markdown
            .contains("where ("));
        let multiline = source.replace("where (", "where\n(");
        let multiline_snapshot =
            AnalysisSnapshot::analyze("multiline-conditions.doria", &multiline);
        assert!(
            multiline_snapshot.diagnostics().is_empty(),
            "{:?}",
            multiline_snapshot.diagnostics()
        );
        for (offset, _) in multiline.match_indices("where\n(") {
            assert!(multiline_snapshot
                .semantic_token_spans()
                .iter()
                .any(|(span, kind, _)| {
                    span.start == offset
                        && span.end == span.start + "where".len()
                        && *kind == SEMANTIC_TOKEN_KEYWORD
                }));
        }
        let contextual = "function where(int $where): int { return $where; } function main(): void { echo where(1); }";
        let ordinary = AnalysisSnapshot::analyze("contextual-where.doria", contextual);
        assert!(
            ordinary.diagnostics().is_empty(),
            "{:?}",
            ordinary.diagnostics()
        );
        assert!(!ordinary
            .semantic_token_spans()
            .iter()
            .any(|(span, kind, _)| {
                &contextual[span.start..span.end] == "where" && *kind == SEMANTIC_TOKEN_KEYWORD
            }));
    }

    #[test]
    fn conditional_subject_navigation_uses_compiler_lexical_parameter_origins() {
        let source = r#"namespace App;
use App\Ready as R;
interface Ready<T> {}
interface Other<T> {}
class Adapter<T> implements R<T> where (T implements R<T>) {
    static function inspect<U>(): int where (T implements R<List<U>>, Other<U>, U implements R<T>) { return 1; }
}
trait Inspection<T> {
    int $state where (T implements R<T>) { get { return 1; } }
}"#;
        let snapshot = AnalysisSnapshot::analyze("condition-subjects.doria", source);
        let (_, compiler) =
            doriac::analyze_source_for_ide("condition-subjects.doria", source).unwrap();
        assert_eq!(snapshot.diagnostics(), compiler.diagnostics);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        assert_eq!(compiler.info.contracts.conditions.len(), 3);
        assert!(compiler
            .info
            .contracts
            .conformances
            .iter()
            .all(|fact| fact.origin.source != SourceId::default()));
        let owner = source.find("Adapter<T>").unwrap() + "Adapter<".len();
        let method = source.find("inspect<U>").unwrap() + "inspect<".len();
        let trait_parameter = source.find("Inspection<T>").unwrap() + "Inspection<".len();
        for (offset, declaration, name) in [
            (source.find("T implements R<T>").unwrap(), owner, "T"),
            (source.find("T implements R<List<U>>").unwrap(), owner, "T"),
            (source.find("U implements R<T>").unwrap(), method, "U"),
            (
                source.rfind("T implements R<T>").unwrap(),
                trait_parameter,
                "T",
            ),
        ] {
            assert_eq!(
                snapshot.declaration_span_at_offset(offset),
                Some(Span::new(declaration, declaration + 1))
            );
            assert!(snapshot
                .hover_at_offset(offset)
                .unwrap()
                .markdown
                .contains(&format!("type parameter {name}")));
            assert!(snapshot.rename_edits_at_offset(offset, "V").is_none());
            assert!(snapshot.rename_replacement_at_offset(offset, "V").is_none());
            assert!(snapshot.reference_spans_at_offset(offset, true).is_empty());
        }
        for condition in compiler.info.contracts.conditions.values() {
            for obligation in &condition.obligations {
                let declaration = snapshot
                    .declaration_span_at_offset(obligation.subject_span.start)
                    .unwrap();
                assert!(declaration.start >= obligation.subject.declaration.start);
                assert!(declaration.end <= obligation.subject.declaration.end);
                assert_eq!(
                    &source[declaration.start..declaration.end],
                    obligation.subject.name
                );
            }
        }
        let invalid = source.replace("U implements R<T>", "Missing implements R<T>");
        let rejected = AnalysisSnapshot::analyze("invalid-condition-subject.doria", &invalid);
        assert!(rejected
            .diagnostics()
            .iter()
            .any(|finding| finding.code == "E0778"));
        for needle in ["T implements R<List<U>>", "Missing implements R<T>"] {
            assert!(rejected
                .declaration_span_at_offset(invalid.find(needle).unwrap())
                .is_none());
        }
    }

    #[test]
    fn conditional_member_features_follow_checked_receivers_and_authored_origins() {
        use crate::workspace_index::OpenDocumentIndex;

        let members = r#"
    function relay<U>(U $value): U where (T implements Ready) { return $value; }
    static function relayStatic<U>(U $value): U where (T implements Ready) { return $value; }
    int $state where (T implements Ready) { get => 1; }
    function probe(): int { return 0; }
    static function staticProbe(): int { return 0; }
"#;
        for composed in [false, true] {
            let owner = if composed {
                format!("trait Values<T> {{ {members} }} open class Adapter<T> {{ uses Values<T> {{ Values<T>::relay as renamed; }} }}")
            } else {
                format!("open class Adapter<T> {{ {members} }}")
            };
            let method = if composed { "renamed" } else { "relay" };
            for (argument, active) in [("Yes", true), ("No", false)] {
                let declarations = format!("interface Ready {{}} class Yes implements Ready {{}} class No {{}} {owner} class Selected extends Adapter<{argument}> {{}}");
                let calls = if active {
                    format!("echo $adapter->{method}(value: 7); echo Selected::relayStatic(value: 8); echo $adapter->state;")
                } else {
                    "echo $adapter->probe(); echo Selected::staticProbe();".to_string()
                };
                let caller = format!("function inspect(Selected $adapter): void {{ {calls} }}");
                for source in [
                    format!("{declarations}{caller}"),
                    format!("{caller}{declarations}"),
                ] {
                    let name = "conditional-editor-features.doria";
                    let snapshot = AnalysisSnapshot::analyze(name, &source);
                    assert!(
                        snapshot.diagnostics().is_empty(),
                        "{source}: {:?}",
                        snapshot.diagnostics()
                    );
                    let index = OpenDocumentIndex::rebuild(std::iter::once((
                        "graph".to_string(),
                        name,
                        &snapshot,
                    )));
                    let instance_offset = source.find("$adapter->").unwrap() + "$adapter->".len();
                    let instance = snapshot
                        .member_completions_at_offset(instance_offset)
                        .unwrap();
                    for label in ["relay", method, "state"] {
                        assert_eq!(
                            instance.iter().any(|item| item.label == label),
                            active,
                            "{source}: {instance:?}"
                        );
                    }
                    assert!(!instance.iter().any(|item| item.label == "relayStatic"));
                    let static_offset = source.find("Selected::").unwrap() + "Selected::".len();
                    let statics = snapshot
                        .static_completions_at_offset(static_offset)
                        .unwrap();
                    assert_eq!(
                        statics.iter().any(|item| item.label == "relayStatic"),
                        active,
                        "{source}: {statics:?}"
                    );
                    assert!(!statics
                        .iter()
                        .any(|item| item.label == "relay" || item.label == "state"));
                    if !active {
                        assert!(instance.iter().any(|item| item.label == "probe"));
                        assert!(statics.iter().any(|item| item.label == "staticProbe"));
                        continue;
                    }
                    for (offset, member, declaration, value) in [
                        (instance_offset, method, "relay", "7"),
                        (static_offset, "relayStatic", "relayStatic", "8"),
                    ] {
                        let signature = format!("{member}<U>(int $value): int");
                        let hover = snapshot.hover_at_offset(offset).unwrap();
                        assert!(hover.markdown.contains(&signature), "{source}: {hover:?}");
                        let argument =
                            source[offset..].find(&format!("value: {value}")).unwrap() + offset;
                        let help = snapshot.signature_help_at_offset(argument).unwrap();
                        assert_eq!(help.len(), 1, "{help:?}");
                        assert!(help[0].label.contains(&signature), "{help:?}");
                        assert_eq!(help[0].active_parameter, 0);
                        let targets =
                            index.contract_definitions(name, offset).unwrap_or_else(|| {
                                index.definition(name, offset).into_iter().collect()
                            });
                        let authored = source.find(&format!("function {declaration}<")).unwrap()
                            + "function ".len();
                        assert!(
                            targets
                                .iter()
                                .any(|target| target.uri == name && target.span.start == authored),
                            "{source}: {targets:?}"
                        );
                        if composed && member == "renamed" {
                            let alias = source.find("as renamed").unwrap() + "as ".len();
                            assert!(
                                targets
                                    .iter()
                                    .any(|target| target.uri == name && target.span.start == alias),
                                "{targets:?}"
                            );
                        }
                    }
                    let property = source.find("$adapter->state").unwrap() + "$adapter->".len();
                    let hover = snapshot.hover_at_offset(property).unwrap();
                    assert!(hover.markdown.contains("get: int"), "{hover:?}");
                    let targets = index.property_accessor_definitions(name, property).unwrap();
                    assert_eq!(targets.len(), 1, "{targets:?}");
                    assert_eq!(targets[0].uri, name);
                    assert_eq!(targets[0].span.start, source.find("get =>").unwrap());
                }
            }
        }
    }

    #[test]
    fn conditional_completions_respect_caller_premises_for_symbolic_receivers() {
        let declarations = r#"
interface Ready {}
class Adapter<T> {
    function inspect(): int where (T implements Ready) { return 1; }
    int $state where (T implements Ready) { get => 1; }
    function probe(): int { return 0; }
    static function inspectStatic(): int where (T implements Ready) { return 1; }
    static function staticProbe(): int { return 0; }
    function provedStatic(): int where (T implements Ready) { return self::staticProbe(); }
    function unprovedStatic(): int { return self::staticProbe(); }
}
"#;
        let callers = r#"
function proved<T implements Ready>(Adapter<T> $proved): int { return $proved->probe(); }
function unproved<T>(Adapter<T> $unproved): int { return $unproved->probe(); }
"#;
        for source in [
            format!("{declarations}{callers}"),
            format!("{callers}{declarations}"),
        ] {
            let snapshot =
                AnalysisSnapshot::analyze("conditional-caller-completion.doria", &source);
            assert!(
                snapshot.diagnostics().is_empty(),
                "{source}: {:?}",
                snapshot.diagnostics()
            );
            for (receiver, available) in [("$proved->", true), ("$unproved->", false)] {
                let offset = source.find(receiver).unwrap() + receiver.len();
                let members = snapshot.member_completions_at_offset(offset).unwrap();
                assert!(members.iter().any(|member| member.label == "probe"));
                for label in ["inspect", "state"] {
                    assert_eq!(
                        members.iter().any(|member| member.label == label),
                        available,
                        "{receiver}: {members:?}"
                    );
                }
            }
            for ((offset, _), available) in
                source.match_indices("self::staticProbe").zip([true, false])
            {
                let members = snapshot
                    .static_completions_at_offset(offset + "self::".len())
                    .unwrap();
                assert!(members.iter().any(|member| member.label == "staticProbe"));
                assert_eq!(
                    members.iter().any(|member| member.label == "inspectStatic"),
                    available,
                    "{members:?}"
                );
            }
        }
    }

    #[test]
    fn conditional_generic_candidates_keep_their_conditions_until_call_inference() {
        use crate::workspace_index::OpenDocumentIndex;

        let source = r#"
interface Ready {}
class Yes implements Ready {}
class Factory {
    function choose<U>(U $value): U where (U implements Ready) { return $value; }
    static function chooseStatic<U>(U $value): U where (U implements Ready) { return $value; }
    function probe(): int { return 0; }
    static function staticProbe(): int { return 0; }
}
function inspect(Factory $factory, Yes $ready): void {
    echo $factory->probe();
    echo Factory::staticProbe();
    let $first = $factory->choose(value: $ready);
    let $second = Factory::chooseStatic(value: $ready);
}
"#;
        let name = "conditional-method-inference.doria";
        let snapshot = AnalysisSnapshot::analyze(name, source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        let index =
            OpenDocumentIndex::rebuild(std::iter::once(("graph".to_string(), name, &snapshot)));
        let instance = snapshot
            .member_completions_at_offset(source.find("probe();").unwrap())
            .unwrap();
        let statics = snapshot
            .static_completions_at_offset(source.find("staticProbe();").unwrap())
            .unwrap();
        for (items, method, usage) in [
            (instance, "choose", "$factory->choose"),
            (statics, "chooseStatic", "Factory::chooseStatic"),
        ] {
            let candidate = items.iter().find(|item| item.label == method).unwrap();
            assert!(
                candidate.detail.ends_with("where (U implements Ready)"),
                "{candidate:?}"
            );
            assert!(candidate
                .documentation
                .as_deref()
                .unwrap()
                .contains("inferred method type arguments"));
            let offset = source.find(usage).unwrap() + usage.len() - method.len();
            let signature = format!("{method}<U>(Yes $value): Yes");
            let hover = snapshot.hover_at_offset(offset).unwrap();
            assert!(hover.markdown.contains(&signature), "{hover:?}");
            let help = snapshot
                .signature_help_at_offset(offset + method.len() + 1)
                .unwrap();
            assert_eq!(help.len(), 1, "{help:?}");
            assert!(help[0].label.contains(&signature), "{help:?}");
            assert_eq!(
                index.definition(name, offset).unwrap().span.start,
                source.find(&format!("function {method}<")).unwrap() + "function ".len()
            );
        }
    }

    #[test]
    fn semantic_tokens_classify_reserved_members_at_authored_name_spans() {
        let source = include_str!("../../editors/fixtures/stage36a-reserved-members.doria");
        let snapshot = AnalysisSnapshot::analyze("reserved-members.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );
        let tokens = snapshot.semantic_token_spans();
        for name in ["async", "await", "spawn", "scope"] {
            for (start, _) in source.match_indices(name) {
                let span = Span::new(start, start + name.len());
                assert!(
                    tokens.iter().any(|(candidate, kind, _)| {
                        *candidate == span && *kind == SEMANTIC_TOKEN_FUNCTION
                    }),
                    "{name} at {span:?}"
                );
            }
        }
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
    fn method_hovers_label_dispatch_as_a_source_contract() {
        let source = r#"open class Base
{
    open function value(): int { return 1; }
    function fixed(): int { return 2; }
}
class Child extends Base
{
    override function value(): int { return parent::value(); }
}
"#;
        let snapshot = AnalysisSnapshot::analyze("dispatch.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        for (declaration, prefix, dispatch) in [
            ("open function value", "open function ", "Virtual"),
            ("override function value", "override function ", "Virtual"),
            ("function fixed", "function ", "Direct"),
        ] {
            let offset = source.find(declaration).unwrap() + prefix.len();
            let hover = snapshot.hover_at_offset(offset).unwrap();
            assert!(
                hover
                    .markdown
                    .contains(&format!("**Source Dispatch:** {dispatch}")),
                "{}",
                hover.markdown
            );
            assert!(!hover.markdown.contains("**Dispatch:**"));
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
    let $view = $text->byteRegion;
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

        let offset = source.find("->byteRegion").unwrap() + 2;
        let hover = snapshot.hover_at_offset(offset).unwrap();
        let completions = snapshot.member_completions_at_offset(offset).unwrap();
        let view = completions
            .iter()
            .find(|item| item.label == "byteRegion")
            .unwrap();
        let intrinsic = byte_intrinsics::member(&ResolvedType::String, "byteRegion").unwrap();
        assert_eq!(view.kind, 10);
        assert_eq!(view.detail, "Doria\\Std\\Io\\ByteRegion $byteRegion");
        assert_eq!(view.documentation.as_deref(), Some(intrinsic.documentation));
        assert!(hover.markdown.contains(&view.detail));
        assert!(hover.markdown.contains(intrinsic.documentation));
        assert!(completions.iter().any(|item| item.label == "bytes"));

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
    fn buffered_adapter_members_follow_the_checked_source_tier() {
        for (direction, adapter, file, sync_method, region, parameter, outcome, result, request) in [
            (
                "Read",
                "BufferedReader",
                "ReadableFile",
                "read",
                "writable WritableByteRegion",
                "destination",
                "ReadOutcome",
                "EndOfStream",
                "ReadRequest",
            ),
            (
                "Write",
                "BufferedWriter",
                "WritableFile",
                "write",
                "ByteRegion",
                "source",
                "WriteOutcome",
                "WouldBlock",
                "WriteRequest",
            ),
        ] {
            for ownership in ["", "Borrowed"] {
                let adapter = format!("{ownership}{adapter}");
                let capability = if direction == "Read" {
                    "ReadableStream"
                } else {
                    "WritableStream"
                };
                let prepare = format!("prepare{direction}");
                let complete = format!("complete{direction}");
                let source = format!(
                    r#"use Doria\Std\Io\{{{adapter}, {file}, {capability}, {outcome}, {request}, ByteRegion, WritableByteRegion, IoError}};
class SyncOnly implements {capability} {{
    writable function {sync_method}({region} ${parameter}): {outcome} {{ return {outcome}::{result}; }}
}}
function inspectPending(writable {adapter}<{file}> $pending,
    {request} $request, {region} $region): void throws IoError {{
    {{ let $ready = $pending->readinessSource; }}
    $pending->{complete}($request, $region);
}}
function inspectSync(writable {adapter}<SyncOnly> $sync, {region} $region): void throws IoError {{
    $sync->{sync_method}($region);
}}
"#
                );
                let snapshot = AnalysisSnapshot::analyze("buffered-adapter-tiers.doria", &source);
                assert!(
                    snapshot.diagnostics().is_empty(),
                    "{source}\n{:?}",
                    snapshot.diagnostics()
                );
                for (receiver, available) in [("$pending->", true), ("$sync->", false)] {
                    let offset = source.find(receiver).unwrap() + receiver.len();
                    let items = snapshot.member_completions_at_offset(offset).unwrap();
                    for name in ["readinessSource", &prepare, &complete] {
                        assert_eq!(
                            items.iter().any(|item| item.label == name),
                            available,
                            "{adapter}: {items:?}"
                        );
                    }
                    assert!(items.iter().any(|item| item.label == sync_method));
                }
                for (name, ty) in [("readinessSource", "ReadinessSource"), (&complete, outcome)] {
                    let prefix = "$pending->";
                    let offset = source.find(&format!("{prefix}{name}")).unwrap() + prefix.len();
                    let hover = snapshot.hover_at_offset(offset).unwrap();
                    assert!(hover.markdown.contains(ty), "{hover:?}");
                }
            }
        }
    }

    #[test]
    fn reached_process_types_supply_member_hover_and_completion() {
        let source = r#"use Doria\Std\Process\Child as ProcessChild;
function inspect(writable ProcessChild $child): void {
    let $input = $child->getInput();
}
"#;
        let snapshot = AnalysisSnapshot::analyze("process-child.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );
        let alias = source.rfind("ProcessChild").unwrap();
        assert!(snapshot
            .hover_at_offset(alias)
            .unwrap()
            .markdown
            .contains("Doria\\Std\\Process\\Child"));
        let offset = source.find("getInput").unwrap();
        let hover = snapshot.hover_at_offset(offset).unwrap();
        assert!(
            hover.markdown.contains("Doria\\Std\\Process\\ChildInput"),
            "{hover:?}"
        );
        let items = snapshot.member_completions_at_offset(offset).unwrap();
        for name in ["getInput", "getOutput", "getErrors"] {
            assert!(items.iter().any(|item| item.label == name), "{name}");
        }
        for name in ["input", "output", "errors"] {
            assert!(!items.iter().any(|item| item.label == name), "{name}");
        }
    }

    #[test]
    fn region_bulk_copy_hovers_and_completions_follow_intrinsic_metadata() {
        let source = r#"use Doria\Std\Io\{ByteRegion, WritableByteRegion};
function copy(writable WritableByteRegion $target, ByteRegion $source): int {
    $target->copyWithin(0, 1, 1);
    return $target->copyFrom($source);
}
"#;
        let snapshot = AnalysisSnapshot::analyze("region-copy.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );
        for name in ["copyFrom", "copyWithin"] {
            let offset = source.find(name).unwrap();
            let hover = snapshot.hover_at_offset(offset).unwrap();
            let items = snapshot.member_completions_at_offset(offset).unwrap();
            let item = items.iter().find(|item| item.label == name).unwrap();
            assert!(hover.markdown.contains(&item.detail), "{hover:?}");
            assert!(hover
                .markdown
                .contains(item.documentation.as_deref().unwrap()));
            assert!(snapshot.declaration_span_at_offset(offset).is_none());
        }
    }

    #[test]
    fn byte_contracts_drive_hover_completion_and_named_signature_help() {
        let source = r#"use Doria\Std\Io\ByteRegion;
use Doria\Std\Io\WritableByteRegion;
function inspect(): void {
    writable Bytes $bytes = Bytes::allocate(length: 4);
    {
        ByteRegion $view = $bytes->getRegion(length: 2, offset: 1);
        let $inner = $view->getRegion(0, 1);
        echo $inner->length;
    }
    {
        writable WritableByteRegion $view = $bytes->getWritableRegion(0, 4);
        {
            let writable $inner = $view->getWritableRegion(0, 2);
            $inner[0] = 7;
        }
        let $readonly = $view->getRegion(0, 1);
        echo $readonly[0];
    }
    let $array = $bytes->toArray();
    let $copy = Bytes::fromArray($array);
    echo $copy->length;
}
"#;
        let snapshot = AnalysisSnapshot::analyze("bytes.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:#?}",
            snapshot.diagnostics()
        );
        for (needle, signature) in [
            ("allocate", "static function Bytes::allocate(int $length): Bytes"),
            ("getRegion(length", "function Bytes::getRegion(int $offset, int $length): Doria\\Std\\Io\\ByteRegion"),
            ("getWritableRegion", "writable function Bytes::getWritableRegion(int $offset, int $length): Doria\\Std\\Io\\WritableByteRegion"),
            ("toArray", "function Bytes::toArray(): uint8[]"),
            ("fromArray", "static function Bytes::fromArray(uint8[] $bytes): Bytes"),
            ("length;", "int $length"),
        ] {
            let hover = snapshot.hover_at_offset(source.find(needle).unwrap()).unwrap();
            assert!(hover.markdown.contains(signature), "{hover:?}");
        }
        for (needle, writable) in [
            ("$bytes->getRegion", true),
            ("$view->getRegion", false),
            ("$view->getWritableRegion", true),
        ] {
            let offset = source.find(needle).unwrap() + needle.find("->").unwrap() + 2;
            let completions = snapshot.member_completions_at_offset(offset).unwrap();
            assert!(completions.iter().any(|item| item.label == "getRegion"));
            assert!(completions.iter().any(|item| item.label == "length"));
            assert_eq!(
                completions
                    .iter()
                    .any(|item| item.label == "getWritableRegion"),
                writable
            );
        }
        let static_items = snapshot
            .static_completions_at_offset(source.find("allocate").unwrap())
            .unwrap();
        assert_eq!(
            static_items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            ["allocate", "fromArray"]
        );
        for (needle, parameter) in [("length: 2", 1), ("offset: 1", 0)] {
            let signatures = snapshot
                .signature_help_at_offset(source.find(needle).unwrap())
                .unwrap();
            assert_eq!(signatures.len(), 1);
            assert_eq!(signatures[0].active_parameter, parameter);
            assert!(signatures[0]
                .label
                .ends_with(": Doria\\Std\\Io\\ByteRegion"));
        }
        let incomplete = "function inspect(): void { let $bytes = Bytes::allocate(\n}";
        let offset = incomplete.find("allocate(").unwrap() + "allocate(".len();
        let help = AnalysisSnapshot::signature_help_for_incomplete_call(
            "incomplete-bytes.doria",
            incomplete,
            offset,
        )
        .unwrap();
        assert_eq!(
            help[0].label,
            "static function Bytes::allocate(int $length): Bytes"
        );
    }

    #[test]
    fn nullable_byte_members_preserve_compiler_results_and_receiver_access() {
        for (ty, writable_storage) in [
            ("Bytes", true),
            ("Doria\\Std\\Io\\ByteRegion", false),
            ("Doria\\Std\\Io\\WritableByteRegion", true),
        ] {
            let writable_call = if writable_storage {
                r#"{
        let writable $part = $value?->getWritableRegion(0, 0);
        if ($part != null) { echo $part->length; }
    }"#
            } else {
                ""
            };
            let source = format!(
                r#"function inspect(writable ?{ty} $value): void {{
    {{
        let $part = $value?->getRegion(length: 0, offset: 0);
        ?int $length = $value?->length;
        echo $length ?? 0;
    }}
    if ($value != null) {{
        let $part = $value->getRegion(0, 0);
        int $length = $value->length;
        echo $length;
    }}
    {writable_call}
}}
"#
            );
            let snapshot = AnalysisSnapshot::analyze("nullable-bytes.doria", &source);
            assert!(
                snapshot.diagnostics().is_empty(),
                "{ty}: {:#?}",
                snapshot.diagnostics()
            );
            for (needle, signature) in [
                ("getRegion(length", "): ?Doria\\Std\\Io\\ByteRegion"),
                ("?->length", "```doria\n?int $length\n```"),
                ("->getRegion(0", "): Doria\\Std\\Io\\ByteRegion"),
                ("$value->length", "```doria\nint $length\n```"),
            ] {
                let offset = source.find(needle).unwrap();
                let offset = offset + needle.find("->").map_or(0, |arrow| arrow + 2);
                let hover = snapshot.hover_at_offset(offset).unwrap();
                assert!(hover.markdown.contains(signature), "{ty}: {hover:?}");
                let items = snapshot.member_completions_at_offset(offset).unwrap();
                assert_eq!(
                    items.iter().any(|item| item.label == "getWritableRegion"),
                    writable_storage,
                    "{ty}: {items:?}"
                );
            }
            let help = snapshot
                .signature_help_at_offset(source.find("length: 0").unwrap())
                .unwrap();
            assert_eq!(help[0].active_parameter, 1);
            assert!(help[0].label.ends_with("): ?Doria\\Std\\Io\\ByteRegion"));
            if writable_storage {
                let hover = snapshot
                    .hover_at_offset(source.find("getWritableRegion").unwrap())
                    .unwrap();
                assert!(hover
                    .markdown
                    .contains("): ?Doria\\Std\\Io\\WritableByteRegion"));
                assert!(hover.markdown.contains("writable function"));
            }

            let readonly =
                format!("function inspect(?{ty} $value): ?int {{ return $value?->length; }}");
            let snapshot = AnalysisSnapshot::analyze("readonly-nullable-bytes.doria", &readonly);
            assert!(
                snapshot.diagnostics().is_empty(),
                "{ty}: {:#?}",
                snapshot.diagnostics()
            );
            let items = snapshot
                .member_completions_at_offset(readonly.find("length").unwrap())
                .unwrap();
            assert!(
                !items.iter().any(|item| item.label == "getWritableRegion"),
                "{ty}: {items:?}"
            );
        }
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
                "Returns a new List containing selected Copy-or-Cloneable elements in insertion order. The source List remains unchanged.",
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
        read_line();
        $calls += 1;
        if ($value == 2) { throw new Failure("stop"); }
        return "{$value}";
    };
    List<string> $mapped = $values->map($callback);
    int $total = $values->reduce(0, function (writable int $sum, int $value): void {
        read_line();
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

        let pure_source = source.replace("        read_line();\n", "");
        let pure = AnalysisSnapshot::analyze("stage30g-pure-hover.doria", &pure_source);
        assert!(pure.diagnostics().is_empty(), "{:?}", pure.diagnostics());
        for name in ["map", "reduce"] {
            let offset = pure_source.find(&format!("$values->{name}")).unwrap() + "$values->".len();
            let hover = pure.hover_at_offset(offset).unwrap();
            assert!(
                !hover.markdown.contains("**Ambient I/O:**"),
                "{}",
                hover.markdown
            );
        }
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
    fn member_completions_respect_compiler_receiver_access() {
        let source = r#"
interface CounterView {
    function read(): int;
    writable function change(): void;
}
class Counter implements CounterView {
    function read(): int { return 1; }
    writable function change(): void {}
    function inspect(): int { return $this->read(); }
    writable function edit(): int { return $this->read(); }
}
function inspect(Counter $readonly): int { return $readonly->read(); }
function edit(writable Counter $writable): int { return $writable->read(); }
function view(CounterView $view): int { return $view->read(); }
function editView(writable CounterView $editView): int { return $editView->read(); }
function locals(): void {
    let $readonlyLocal = new Counter();
    let writable $writableLocal = new Counter();
    $readonlyLocal->read();
    $writableLocal->read();
}
"#;
        let snapshot = AnalysisSnapshot::analyze("receiver-access.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        for (expression, writable) in [
            ("$readonly->read", false),
            ("$writable->read", true),
            ("$view->read", false),
            ("$editView->read", true),
            ("$readonlyLocal->read", false),
            ("$writableLocal->read", true),
        ] {
            let offset = source.find(expression).unwrap() + expression.find("->").unwrap() + 2;
            let completions = snapshot.member_completions_at_offset(offset).unwrap();
            assert!(completions.iter().any(|item| item.label == "read"));
            assert_eq!(
                completions.iter().any(|item| item.label == "change"),
                writable,
                "{expression}: {completions:?}"
            );
        }
        for (index, (offset, _)) in source.match_indices("$this->read").enumerate() {
            let completions = snapshot
                .member_completions_at_offset(offset + "$this->".len())
                .unwrap();
            assert_eq!(
                completions.iter().any(|item| item.label == "change"),
                index == 1,
                "{completions:?}"
            );
        }
    }

    #[test]
    fn stage35_interface_runtime_fixture_has_no_false_diagnostics() {
        let source = include_str!("../../editors/fixtures/stage35-interface-runtime.doria");
        let snapshot = AnalysisSnapshot::analyze("interface-runtime.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        for (expression, included, excluded) in [
            ("$again->read", "read", "rename"),
            ("$read->read", "read", "rename"),
            ("$write->rename", "rename", "share"),
            ("$owner->createWeakReference", "referencedValue", "rename"),
            ("$weak->acquire", "acquire", "read"),
            (
                "$mutable->createWeakReference",
                "acquireWritableAccess",
                "read",
            ),
        ] {
            let offset = source.find(expression).unwrap() + expression.find("->").unwrap() + 2;
            let members = snapshot.member_completions_at_offset(offset).unwrap();
            assert!(
                members.iter().any(|member| member.label == included),
                "{expression}: {members:?}"
            );
            assert!(
                !members.iter().any(|member| member.label == excluded),
                "{expression}: {members:?}"
            );
        }
    }

    #[test]
    fn interface_requirements_preserve_all_compiler_receiver_modes() {
        let source = r#"
interface Work {
    function read(): int;
    writable function update(): void;
    consuming function finish(): int;
}
function inspect(Work $view): int { return $view->read(); }
function edit(writable Work $view): void { $view->update(); }
function complete(take Work $view): int { return $view->finish(); }
"#;
        let snapshot = AnalysisSnapshot::analyze("interface-receivers.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        for (name, signature) in [
            ("read", "function read(): int"),
            ("update", "writable function update(): void"),
            ("finish", "consuming function finish(): int"),
        ] {
            let offset = source.find(&format!("$view->{name}")).unwrap() + "$view->".len();
            let members = snapshot.member_completions_at_offset(offset).unwrap();
            let member = members.iter().find(|member| member.label == name).unwrap();
            assert_eq!(member.detail, signature);
            let hover = snapshot.hover_at_offset(offset).unwrap();
            assert!(hover.markdown.contains(signature), "{hover:?}");
            if name == "read" {
                assert!(!members.iter().any(|member| member.label == "update"));
            }
        }
    }

    #[test]
    fn compiler_stream_requirements_share_hover_completion_and_signature_facts() {
        let source = r#"use Doria\Std\Io\ReadableStream as Reader;
use Doria\Std\Io\{WritableStream, DuplexStream, FlushableStream, WritableByteRegion, ByteRegion, IoError};
function inspect(writable Reader $reader, writable WritableStream $writer,
    writable DuplexStream $duplex, writable FlushableStream $flusher,
    writable WritableByteRegion $destination, ByteRegion $source): void throws IoError
{
    let $read = $reader->read(destination: $destination);
    let $write = $writer->write(source: $source);
    let $duplexRead = $duplex->read($destination);
    let $duplexWrite = $duplex->write($source);
    let $flush = $flusher->flush();
}
"#;
        let snapshot = AnalysisSnapshot::analyze("stream-help.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        for (receiver, method, signature, names) in [
            ("reader", "read", "writable function read(writable Doria\\Std\\Io\\WritableByteRegion $destination): Doria\\Std\\Io\\ReadOutcome throws Doria\\Std\\Io\\IoError", vec!["read"]),
            ("writer", "write", "writable function write(Doria\\Std\\Io\\ByteRegion $source): Doria\\Std\\Io\\WriteOutcome throws Doria\\Std\\Io\\IoError", vec!["write"]),
            ("duplex", "read", "writable function read(writable Doria\\Std\\Io\\WritableByteRegion $destination): Doria\\Std\\Io\\ReadOutcome throws Doria\\Std\\Io\\IoError", vec!["read", "write"]),
            ("duplex", "write", "writable function write(Doria\\Std\\Io\\ByteRegion $source): Doria\\Std\\Io\\WriteOutcome throws Doria\\Std\\Io\\IoError", vec!["read", "write"]),
            ("flusher", "flush", "writable function flush(): Doria\\Std\\Io\\FlushOutcome throws Doria\\Std\\Io\\IoError", vec!["flush"]),
        ] {
            let needle = format!("${receiver}->{method}(");
            let offset = source.find(&needle).unwrap() + receiver.len() + 3;
            let hover = snapshot.hover_at_offset(offset).unwrap();
            assert!(hover.markdown.contains(signature), "{hover:?}");
            let help = snapshot.signature_help_at_offset(offset + method.len() + 1).unwrap();
            assert_eq!(help[0].label, signature);
            assert_eq!(help[0].active_parameter, 0);
            let items = snapshot.member_completions_at_offset(offset).unwrap();
            let labels = items.iter().map(|item| item.label.as_str()).collect::<HashSet<_>>();
            assert_eq!(labels, names.into_iter().collect());
            assert_eq!(items.iter().find(|item| item.label == method).unwrap().detail, signature);
            assert!(snapshot.declaration_span_at_offset(offset).is_none());
            if method == "flush" {
                assert!(hover.markdown.contains("Pending preserves the unwritten remainder"));
                assert!(items.iter().find(|item| item.label == method).unwrap()
                    .documentation.as_deref().unwrap().contains("does not implicitly wait for readiness"));
            }
        }
        assert!(snapshot
            .hover_at_offset(source.find("$flush =").unwrap())
            .unwrap()
            .markdown
            .contains("Doria\\Std\\Io\\FlushOutcome $flush"));
    }

    #[test]
    fn flush_outcome_help_preserves_payload_free_copy_cases() {
        for case in ["Complete", "Pending"] {
            let source = format!("use Doria\\Std\\Io\\FlushOutcome; function outcome(): FlushOutcome {{ return FlushOutcome::{case}; }}");
            let snapshot = AnalysisSnapshot::analyze("flush-outcome-help.doria", &source);
            assert!(
                snapshot.diagnostics().is_empty(),
                "{:?}",
                snapshot.diagnostics()
            );
            let offset = source.find(&format!("::{case}")).unwrap() + 2;
            let hover = snapshot.hover_at_offset(offset).unwrap();
            assert!(
                hover.markdown.contains("Doria\\Std\\Io\\FlushOutcome"),
                "{hover:?}"
            );
            assert!(
                hover.markdown.contains("payload-free Copy values"),
                "{hover:?}"
            );
            let labels = snapshot
                .static_completions_at_offset(offset)
                .unwrap()
                .into_iter()
                .map(|item| item.label)
                .collect::<HashSet<_>>();
            assert_eq!(
                labels,
                ["Complete", "Pending"]
                    .into_iter()
                    .map(str::to_string)
                    .collect()
            );
            assert!(snapshot
                .signature_help_at_offset(offset + case.len())
                .is_none());
        }
    }

    #[test]
    fn constrained_method_help_uses_scoped_compiler_contracts() {
        let source = r#"
interface Transform<U> { function transform(U $value): U; }
function inspectInt<T implements Transform<int>>(T $processor, int $input): int {
    return $processor->transform(value: $input);
}
function inspectString<T implements Transform<string>>(T $processor, string $input): string {
    return $processor->transform(value: $input);
}
"#;
        let snapshot = AnalysisSnapshot::analyze("constrained-help.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        let offsets = source
            .match_indices("->transform(")
            .map(|(offset, _)| offset + 2)
            .collect::<Vec<_>>();
        assert_eq!(offsets.len(), 2);
        for (offset, ty) in offsets.into_iter().zip(["int", "string"]) {
            let expected = format!("function transform({ty} $value): {ty}");
            let hover = snapshot.hover_at_offset(offset).unwrap();
            assert!(hover.markdown.contains(&expected), "{hover:?}");
            let help = snapshot
                .signature_help_at_offset(offset + "transform(".len())
                .unwrap();
            assert_eq!(help[0].label, expected);
            assert_eq!(help[0].active_parameter, 0);
            let items = snapshot.member_completions_at_offset(offset).unwrap();
            assert_eq!(
                items
                    .iter()
                    .find(|item| item.label == "transform")
                    .unwrap()
                    .detail,
                expected
            );
            let arguments = snapshot
                .named_argument_completions_at_offset(offset + "transform(".len())
                .unwrap();
            assert_eq!(arguments.len(), 1);
            assert_eq!(arguments[0].label, "value:");
            assert!(arguments[0].detail.contains(&expected));
        }
    }

    #[test]
    fn pending_adapter_requirements_preserve_inherited_and_phase_contracts() {
        let adapters = include_str!("../../editors/fixtures/stage36a-pending-adapters.doria");
        let snapshot = AnalysisSnapshot::analyze("pending-adapters.doria", adapters);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        for (capability, request, region, prepare, complete, outcome, transfer) in [
            (
                "PendingReadableStream",
                "ReadRequest",
                "writable WritableByteRegion",
                "prepareRead",
                "completeRead",
                "ReadOutcome",
                "read",
            ),
            (
                "PendingWritableStream",
                "WriteRequest",
                "ByteRegion",
                "prepareWrite",
                "completeWrite",
                "WriteOutcome",
                "write",
            ),
        ] {
            for generic in [false, true] {
                let parameters = if generic {
                    format!("<T implements {capability}>")
                } else {
                    String::new()
                };
                let receiver = if generic { "T" } else { capability };
                if transfer == "read" {
                    let source = format!(
                        "use Doria\\Std\\Io\\{{{capability}, {request}, ByteRegion, WritableByteRegion, {outcome}, IoError}};\nfunction inspect{parameters}(writable {receiver} $stream, writable {request} $request, {region} $region): ?{outcome} throws IoError {{ $stream->{prepare}($request, $region); return $stream->{complete}($request, $region); }}"
                    );
                    let snapshot =
                        AnalysisSnapshot::analyze("pending-premature-completion.doria", &source);
                    assert!(
                        snapshot
                            .diagnostics()
                            .iter()
                            .any(|diagnostic| diagnostic.code == "E0774"),
                        "completion cannot reuse selected writable storage before retirement: {:?}",
                        snapshot.diagnostics()
                    );
                }
                let source = format!(
                    "use Doria\\Std\\Io\\{{{capability}, {request}, ByteRegion, WritableByteRegion, {outcome}, IoError}};\n\
                     function prepare{parameters}(writable {receiver} $stream, writable {request} $request, {region} $region): void throws IoError {{ $stream->{prepare}($request, $region); }}\n\
                     function complete{parameters}(writable {receiver} $stream, {request} $request, {region} $region): ?{outcome} throws IoError {{ return $stream->{complete}($request, $region); }}"
                );
                let snapshot = AnalysisSnapshot::analyze("pending-help.doria", &source);
                assert!(
                    snapshot.diagnostics().is_empty(),
                    "{source}\n{:?}",
                    snapshot.diagnostics()
                );
                for (method, request_mode, result) in
                    [(prepare, "writable ", "void"), (complete, "", outcome)]
                {
                    let offset = source.find(&format!("->{method}(")).unwrap() + 2;
                    let region_type = if transfer == "read" {
                        "writable Doria\\Std\\Io\\WritableByteRegion"
                    } else {
                        "Doria\\Std\\Io\\ByteRegion"
                    };
                    let result = if result == "void" {
                        result.to_string()
                    } else {
                        format!("?Doria\\Std\\Io\\{result}")
                    };
                    let region_name = if transfer == "read" {
                        "destination"
                    } else {
                        "source"
                    };
                    let expected = format!("writable function {method}({request_mode}Doria\\Std\\Io\\{request} $request, {region_type} ${region_name}): {result} throws Doria\\Std\\Io\\IoError");
                    let hover = snapshot.hover_at_offset(offset).unwrap_or_else(|| {
                        panic!(
                            "missing {capability} {method} hover (generic={generic}): {:#?}",
                            snapshot.semantic_hovers
                        )
                    });
                    assert!(hover.markdown.contains(&expected), "{hover:?}");
                    let help = snapshot
                        .signature_help_at_offset(offset + method.len() + 1)
                        .unwrap();
                    assert_eq!(help[0].label, expected);
                    let items = snapshot.member_completions_at_offset(offset).unwrap();
                    let labels = items
                        .iter()
                        .map(|item| item.label.as_str())
                        .collect::<HashSet<_>>();
                    assert_eq!(
                        labels,
                        [transfer, "readinessSource", prepare, complete]
                            .into_iter()
                            .collect()
                    );
                    assert_eq!(
                        items
                            .iter()
                            .find(|item| item.label == method)
                            .unwrap()
                            .detail,
                        expected
                    );
                    assert_eq!(
                        items
                            .iter()
                            .find(|item| item.label == transfer)
                            .unwrap()
                            .detail,
                        format!("writable function {transfer}({region_type} ${region_name}): Doria\\Std\\Io\\{outcome} throws Doria\\Std\\Io\\IoError")
                    );
                    assert!(snapshot.declaration_span_at_offset(offset).is_none());
                }
            }
        }
    }

    #[test]
    fn pending_request_members_preserve_readonly_outcomes_and_writable_completion() {
        for (request, outcome) in [
            ("ReadRequest", "ReadOutcome"),
            ("WriteRequest", "WriteOutcome"),
        ] {
            let source = format!("use Doria\\Std\\Io\\{{{request}, {outcome}}};\nfunction prepare(writable {request} $request): void {{ $request->complete({outcome}::WouldBlock); }}\nfunction completed({request} $request): ?{outcome} {{ return $request->outcome; }}");
            let snapshot = AnalysisSnapshot::analyze("pending-request-help.doria", &source);
            assert!(
                snapshot.diagnostics().is_empty(),
                "{source}\n{:?}",
                snapshot.diagnostics()
            );
            let offset = source.find("->complete(").unwrap() + 2;
            let expected = format!("writable function Doria\\Std\\Io\\{request}::complete(Doria\\Std\\Io\\{outcome} $outcome): void");
            assert!(snapshot
                .hover_at_offset(offset)
                .unwrap()
                .markdown
                .contains(&expected));
            assert_eq!(
                snapshot
                    .signature_help_at_offset(offset + "complete(".len())
                    .unwrap()[0]
                    .label,
                expected
            );
            let writable = snapshot.member_completions_at_offset(offset).unwrap();
            assert_eq!(
                writable
                    .iter()
                    .find(|item| item.label == "complete")
                    .unwrap()
                    .detail,
                expected
            );
            let offset = source.find("->outcome").unwrap() + 2;
            let expected = format!("$outcome {{ get: ?Doria\\Std\\Io\\{outcome} }}");
            let readonly = snapshot.member_completions_at_offset(offset).unwrap();
            assert!(!readonly.iter().any(|item| item.label == "complete"));
            let completion = readonly
                .iter()
                .find(|item| item.label == "outcome")
                .unwrap();
            assert_eq!(completion.detail, expected);
            assert!(completion
                .documentation
                .as_ref()
                .unwrap()
                .contains("**Ambient I/O:**"));
            let hover = snapshot.hover_at_offset(offset).unwrap().markdown;
            let signature = hover.split("```").nth(1).unwrap();
            assert!(
                signature.contains(&format!("get: ?Doria\\Std\\Io\\{outcome}")),
                "{hover}"
            );
            assert!(!signature.contains("throws"), "{hover}");
            assert!(hover.contains("**Ambient I/O:**"), "{hover}");
            assert!(snapshot.declaration_span_at_offset(offset).is_none());
        }
    }

    #[test]
    fn pending_request_binding_hovers_use_canonical_identity_access_and_call_scope() {
        for request in ["ReadRequest", "WriteRequest"] {
            let source = format!("use Doria\\Std\\Io\\{request} as RequestView;\nfunction inspect(RequestView $request): void {{ let $alias = $request; let $again = $alias; }}\nfunction prepare(writable RequestView $request): void {{ let $alias = $request; let $again = $alias; }}");
            let snapshot = AnalysisSnapshot::analyze("request-bindings.doria", &source);
            assert!(
                snapshot.diagnostics().is_empty(),
                "{:?}",
                snapshot.diagnostics()
            );
            let (_, compiler) =
                doriac::analyze_source_for_ide("request-bindings.doria", &source).unwrap();
            let mut count = 0;
            for (span, binding) in &compiler.info.binding_resolution.uses_by_span {
                if span.source != SourceId::default() {
                    continue;
                }
                let declaration = &compiler.info.binding_resolution.declarations_by_id[binding];
                let Some(ResolvedType::Class(class)) = declaration.source_type.as_ref() else {
                    continue;
                };
                assert!(doriac::compiler_known_stream::is_pending_request(
                    &class.name
                ));
                assert_ne!(declaration.ownership, BindingOwnership::Owned);
                let hover = snapshot.hover_at_offset(span.start).unwrap().markdown;
                let access = if declaration.ownership == BindingOwnership::WritableBorrow {
                    "writable "
                } else {
                    ""
                };
                assert!(
                    hover.contains(&format!(
                        "{access}Doria\\Std\\Io\\{request} ${}",
                        declaration.name
                    )),
                    "{hover}"
                );
                assert!(hover.contains("Call-Scoped Request View"), "{hover}");
                assert!(
                    !hover.contains("result's last use") && !hover.contains("result's cleanup"),
                    "{hover}"
                );
                count += 1;
            }
            assert_eq!(count, 4);
        }
        let source =
            "namespace App; class ReadRequest {} function inspect(ReadRequest $request): void {}";
        let snapshot = AnalysisSnapshot::analyze("ordinary-request-name.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        let hover = snapshot
            .hover_at_offset(source.find("$request").unwrap())
            .unwrap()
            .markdown;
        assert!(!hover.contains("Call-Scoped Request View"), "{hover}");
    }

    #[test]
    fn consuming_class_and_trait_signatures_preserve_compiler_receiver_modes() {
        for declarations in [
            "class Resource { consuming function finish(): int { return 42; } }",
            "trait Finishing { consuming function finish(): int { return 42; } } class Resource { uses Finishing { Finishing::finish as finishAlias; } }",
        ] {
            let source = format!("{declarations} function main(): void {{ let $resource = new Resource(); let $value = $resource->finish(); }}");
            let snapshot = AnalysisSnapshot::analyze("consuming-help.doria", &source);
            assert!(snapshot.diagnostics().is_empty(), "{source}\n{:?}", snapshot.diagnostics());
            let offset = source.find("->finish(").unwrap() + 2;
            let expected = "consuming function Resource::finish(): int";
            assert!(snapshot.hover_at_offset(offset).unwrap().markdown.contains(expected));
            assert_eq!(snapshot.signature_help_at_offset(offset + "finish(".len()).unwrap()[0].label, expected);
            let completions = snapshot.member_completions_at_offset(offset).unwrap();
            assert_eq!(completions.iter().find(|item| item.label == "finish").unwrap().detail, expected);
            if let Some(adaptation) = snapshot.trait_adaptations.first() {
                let items = snapshot.trait_adaptation_completions_at_offset(adaptation.method.span.start).unwrap();
                assert!(items.iter().any(|item| item.detail == "consuming function Finishing::finish(): int"), "{items:?}");
                assert_eq!(completions.iter().find(|item| item.label == "finishAlias").unwrap().detail, "consuming function Resource::finishAlias(): int");
            }
            let program = doriac::parse_source("consuming-help.doria", declarations).unwrap();
            let method = program.items.iter().find_map(|item| {
                let members = match item {
                    Item::Class(declaration) => &declaration.members,
                    Item::Trait(declaration) => &declaration.members,
                    _ => return None,
                };
                members.iter().find_map(|member| match member {
                    ClassMember::Method(method) => Some(method),
                    _ => None,
                })
            }).unwrap();
            assert_eq!(
                function_signature(method, Some("Resource"), &ParameterPresentation::default()),
                expected
            );
            assert!(override_stub(method).starts_with("override consuming function finish(): int"));
        }
    }

    #[test]
    fn generic_enum_hovers_separate_authored_identity_from_checked_applications() {
        let source = include_str!("../../editors/fixtures/stage36a-generic-enums.doria");
        let snapshot = AnalysisSnapshot::analyze("generic-enum-hover.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        let declaration = source.find("Optional<T>").unwrap();
        let declaration_hover = snapshot.hover_at_offset(declaration).unwrap();
        assert!(declaration_hover.markdown.contains("enum Optional<T>"));
        assert!(!declaration_hover.markdown.contains("Nominal Copy enum"));
        assert!(!declaration_hover.markdown.contains("Nominal Move enum"));

        for (expression, expected) in [
            ("Optional::Some(7)", "Optional<int>"),
            ("Optional::Some(new Token", "Optional<Token>"),
            ("Optional::None;", "Optional<int>"),
        ] {
            let offset = source.find(expression).unwrap();
            let hover = snapshot.hover_at_offset(offset).unwrap();
            assert!(hover.markdown.contains(expected), "{hover:?}");
            assert_eq!(
                snapshot.declaration_span_at_offset(offset).unwrap().start,
                declaration
            );
        }
        let parameter = source.find("Optional<int> $value").unwrap();
        let hover = snapshot.hover_at_offset(parameter).unwrap();
        assert!(hover.markdown.contains("enum Optional<int>"), "{hover:?}");
    }

    #[test]
    fn compiler_stream_type_and_outcome_hovers_use_nominal_metadata() {
        for name in [
            "PendingReadableStream",
            "PendingWritableStream",
            "ReadRequest",
            "WriteRequest",
        ] {
            assert!(doriac::compiler_known_stream::CANONICAL_TYPES
                .contains(&format!("Doria\\Std\\Io\\{name}").as_str()));
        }
        for name in doriac::compiler_known_stream::CANONICAL_TYPES {
            let inline_copy = doriac::inline_value::InlineValueKind::for_name(name);
            let mut arguments = inline_copy
                .filter(|kind| kind.argument_count() > 0)
                .map(|kind| {
                    format!(
                        "<{}>",
                        vec!["Doria\\Std\\Io\\ReadableFile"; kind.argument_count()].join(", ")
                    )
                })
                .unwrap_or_default();
            if let Some(arguments_for_new_type) = match name.rsplit('\\').next().unwrap() {
                "FinishOutcome" | "LockOutcome" => Some("<int, int>"),
                "BorrowedFinishOutcome" | "BorrowedLockOutcome" => Some("<int>"),
                "BufferedReader"
                | "BorrowedBufferedReader"
                | "LineReader"
                | "BorrowedLineReader"
                | "FileLock"
                | "LockedFile" => Some("<Doria\\Std\\Io\\ReadableFile>"),
                "BufferedWriter" | "BorrowedBufferedWriter" => {
                    Some("<Doria\\Std\\Io\\WritableFile>")
                }
                _ => None,
            } {
                arguments = arguments_for_new_type.to_string();
            }
            let kind = doriac::compiler_known_stream::declarations()
                .items
                .iter()
                .find_map(|item| match item {
                    Item::Enum(declaration) if declaration.name == name => Some("enum"),
                    Item::Interface(declaration) if declaration.name == name => Some("interface"),
                    Item::Class(declaration) if declaration.name == name => Some("class"),
                    _ => None,
                })
                .expect("canonical stream type must have a compiler declaration");
            for (prefix, spelling) in [
                (String::new(), name),
                (format!("use {name} as View; "), "View"),
            ] {
                let source =
                    format!("{prefix}function inspect({spelling}{arguments} $value): void {{}}");
                let snapshot = AnalysisSnapshot::analyze("stream-type.doria", &source);
                assert!(
                    snapshot.diagnostics().is_empty(),
                    "{:?}",
                    snapshot.diagnostics()
                );
                let offset = source
                    .rfind(&format!("{spelling}{arguments} $value"))
                    .unwrap()
                    + spelling.len()
                    - 1;
                let hover = snapshot.hover_at_offset(offset).unwrap();
                if let Some(inline_copy) = inline_copy {
                    assert!(
                        hover
                            .markdown
                            .contains(&inline_value_documentation(inline_copy)),
                        "{hover:?}"
                    );
                    assert!(!hover.markdown.contains("class "), "{hover:?}");
                } else {
                    assert!(
                        hover.markdown.contains(&format!("{kind} {name}")),
                        "{hover:?}"
                    );
                }
                if let Some(request) = pending_request_documentation(name) {
                    assert!(hover.markdown.contains(request), "{hover:?}");
                }
                if let Some(class) = doriac::compiler_known_stream::declarations()
                    .items
                    .iter()
                    .find_map(|item| match item {
                        Item::Class(class) if class.name == name => Some(class),
                        _ => None,
                    })
                {
                    for interface in &class.implements {
                        assert!(
                            hover.markdown.contains(&format!("`{}`", interface.ty)),
                            "{hover:?}"
                        );
                    }
                }
                assert!(snapshot.declaration_span_at_offset(offset).is_none());
            }
        }
        for (name, case, cases) in [
            (
                "ReadOutcome",
                "Data",
                vec!["Data", "WouldBlock", "EndOfStream", "TimedOut"],
            ),
            ("WriteOutcome", "Accepted", vec!["Accepted", "WouldBlock"]),
        ] {
            let qualified = format!("Doria\\Std\\Io\\{name}");
            let source = format!(
                "function inspect(): void {{ let $value = {qualified}::{case}(byteCount: 0); }}"
            );
            let snapshot = AnalysisSnapshot::analyze("stream-outcome.doria", &source);
            assert!(
                snapshot.diagnostics().is_empty(),
                "{:?}",
                snapshot.diagnostics()
            );
            let offset = source.find(&format!("::{case}")).unwrap() + 2;
            let signature = format!("{qualified}::{case}(int $byteCount): {qualified}");
            let hover = snapshot.hover_at_offset(offset).unwrap();
            assert!(hover.markdown.contains(&signature), "{hover:?}");
            assert!(hover.markdown.contains("Copy enum"), "{hover:?}");
            let help = snapshot
                .signature_help_at_offset(source.find("byteCount:").unwrap())
                .unwrap();
            assert_eq!(help[0].label, signature);
            assert_eq!(help[0].active_parameter, 0);
            let labels = snapshot
                .static_completions_at_offset(offset)
                .unwrap()
                .into_iter()
                .map(|item| item.label)
                .collect::<HashSet<_>>();
            assert_eq!(labels, cases.into_iter().map(str::to_string).collect());
        }
        let source = "function inspect(Doria\\Std\\Io\\IoError $error): ?int { return $error->transferredByteCount; }";
        let snapshot = AnalysisSnapshot::analyze("io-progress.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        let hover = snapshot
            .hover_at_offset(source.rfind("transferredByteCount").unwrap())
            .unwrap();
        assert!(
            hover.markdown.contains("?int $transferredByteCount"),
            "{hover:?}"
        );
        assert!(hover.markdown.contains("readonly"), "{hover:?}");
    }

    #[test]
    fn stage35_erased_and_narrowed_completions_use_only_the_requirement_view() {
        let source = r#"
interface Read<T> { function read(T $value): T; }
interface Change extends Read<int> { writable function write(int $value): void; }
class Box implements Change {
    function read(int $value = 1): int { return $value; }
    writable function write(int $value): void {}
    function implementationOnly(): void {}
}
function invoke(Read<int> $view): int { return $view->read(7); }
function narrow(writable Read<int> $view): void {
    if ($view is Change) { $view->write(2); }
}
"#;
        let snapshot = AnalysisSnapshot::analyze("interface-views.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        let call = source.find("$view->read").unwrap() + "$view->".len();
        let members = snapshot.member_completions_at_offset(call).unwrap();
        assert_eq!(
            members
                .iter()
                .map(|member| member.label.as_str())
                .collect::<Vec<_>>(),
            vec!["read"]
        );
        assert!(members[0].detail.contains("int $value"), "{members:?}");
        assert!(!members[0].detail.contains('='));
        let signature = snapshot
            .signature_help_at_offset(call + "read(".len())
            .unwrap();
        assert!(
            signature[0].label.contains("read(int $value): int"),
            "{signature:?}"
        );
        let narrowed = source.find("$view->write").unwrap() + "$view->".len();
        let members = snapshot.member_completions_at_offset(narrowed).unwrap();
        assert!(members
            .iter()
            .any(|member| member.label == "write" && member.detail.starts_with("writable")));
        assert!(!members
            .iter()
            .any(|member| member.label == "implementationOnly"));
    }

    #[test]
    fn stage35_interface_properties_and_results_preserve_the_declared_view() {
        let source = r#"
interface View { function read(): int; }
class Value implements View {
    function read(): int { return 1; }
    function implementationOnly(): void {}
}
class Holder { function __construct(take View $view) {} }
function make(): View { return new Value(); }
function inspect(Holder $holder): void {
    echo $holder->view->read();
    let $result = make();
    echo $result->read();
}
"#;
        let snapshot = AnalysisSnapshot::analyze("interface-storage.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        let property = source.find("$holder->view").unwrap() + "$holder->".len();
        let hover = snapshot.hover_at_offset(property).unwrap();
        assert!(hover.markdown.contains("View $view"), "{}", hover.markdown);
        for expression in ["$holder->view->read", "$result->read"] {
            let offset = source.find(expression).unwrap() + expression.rfind("read").unwrap();
            let members = snapshot.member_completions_at_offset(offset).unwrap();
            assert_eq!(
                members
                    .iter()
                    .map(|member| member.label.as_str())
                    .collect::<Vec<_>>(),
                vec!["read"]
            );
            let hover = snapshot.hover_at_offset(offset).unwrap();
            assert!(hover.markdown.contains("read(): int"), "{}", hover.markdown);
        }
    }

    #[test]
    fn stage35_generic_erased_signatures_use_compiler_substitutions() {
        let source = "interface Select<T> { function choose<U>(T $value, U $other): U; } function call(Select<int> $view): string { return $view->choose(other: \"chosen\", value: 2); }";
        let snapshot = AnalysisSnapshot::analyze("interface-generic.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        let call = source.rfind("choose").unwrap();
        let hover = snapshot.hover_at_offset(call).unwrap();
        assert!(
            hover.markdown.contains("int $value, string $other"),
            "{}",
            hover.markdown
        );
        assert!(hover.markdown.contains("): string"), "{}", hover.markdown);
        let signature = snapshot
            .signature_help_at_offset(source.find("\"chosen\"").unwrap())
            .unwrap();
        assert_eq!(signature[0].active_parameter, 1);
    }

    #[test]
    fn stage35_error_subinterface_members_and_effects_are_projected() {
        let source = "interface Failure extends Error { function code(): int; } interface Work { function run(): void throws Failure; } function call(Work $job): void { try { $job->run(); } catch (Failure $error) { echo $error->code(); echo $error->message; } }";
        let snapshot = AnalysisSnapshot::analyze("interface-effects.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        let error = source.find("$error->code").unwrap() + "$error->".len();
        let members = snapshot.member_completions_at_offset(error).unwrap();
        assert!(members.iter().any(|member| member.label == "message"));
        assert!(members.iter().any(|member| member.label == "code"));
        let message = snapshot
            .hover_at_offset(source.find("message").unwrap())
            .unwrap();
        assert!(
            message.markdown.contains("string $message"),
            "{}",
            message.markdown
        );
        assert!(
            message.markdown.contains("readonly stored"),
            "{}",
            message.markdown
        );
        let call = source.find("$job->run").unwrap() + "$job->".len();
        let hover = snapshot.hover_at_offset(call).unwrap();
        assert!(
            hover.markdown.contains("throws Failure"),
            "{}",
            hover.markdown
        );
    }

    #[test]
    fn stage35_retained_sources_and_core_execution_have_no_false_diagnostics() {
        let source = include_str!("../../editors/fixtures/stage35-core-iteration.doria");
        let snapshot = AnalysisSnapshot::analyze("iteration.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        let source_hover = snapshot
            .hover_at_offset(source.find("$source) {}").unwrap())
            .unwrap();
        assert!(
            source_hover.markdown.contains("Retained Readonly Source"),
            "{}",
            source_hover.markdown
        );
        assert!(
            source_hover.markdown.contains("List<Book> $source"),
            "{}",
            source_hover.markdown
        );
        let signature = snapshot
            .signature_help_at_offset(
                source.find("new BookCursor($books)").unwrap() + "new BookCursor(".len(),
            )
            .unwrap();
        assert!(
            signature[0].label.contains("borrow List<Book>"),
            "{}",
            signature[0].label
        );

        let erased = "interface CloneView extends Cloneable {} function inspect(CloneView $value, Iterator<Book> $cursor): void { $value->clone(); let $book = $cursor->getCurrent(); echo $book->title; } class Book { function __construct(string $title) {} }";
        let snapshot = AnalysisSnapshot::analyze("erased-core.doria", erased);
        assert!(
            snapshot.diagnostics().is_empty(),
            "{:?}",
            snapshot.diagnostics()
        );
        for (needle, expected) in [
            ("$value->clone", "same exact dynamic"),
            ("$cursor->getCurrent", "readonly borrow"),
        ] {
            let offset = erased.find(needle).unwrap() + needle.find("->").unwrap() + 2;
            let hover = snapshot.hover_at_offset(offset).unwrap();
            assert!(hover.markdown.contains(expected), "{}", hover.markdown);
        }
    }

    #[test]
    fn stage35_iterator_diagnostics_preserve_source_and_element_loans() {
        let cursor = "class Book { function __construct(string $title) {} } class Cursor implements Iterator<Book> { function __construct(borrow List<Book> $source) {} function hasCurrent(): bool { return true; } function getCurrent(): Book { return $this->source[0]; } writable function advance(): void {} }";
        for (body, code) in [
            (
                "let writable $source = [new Book(\"one\")]; let $cursor = new Cursor($source); $source->add(new Book(\"two\")); echo $cursor->getCurrent()->title;",
                "E0763",
            ),
            (
                "let $source = [new Book(\"one\")]; let writable $cursor = new Cursor($source); let $value = $cursor->getCurrent(); $cursor->advance(); echo $value->title;",
                "E0477",
            ),
        ] {
            let snapshot = AnalysisSnapshot::analyze(
                "loans.doria",
                &format!("{cursor} function main(): void {{ {body} }}"),
            );
            assert!(
                snapshot
                    .diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.code == code),
                "{body}: {:?}",
                snapshot.diagnostics()
            );
            assert!(
                !snapshot
                    .diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.code == "E0759")
            );
        }
    }

    #[test]
    fn stage35_interface_writability_and_composition_conflicts_remain_diagnostics() {
        for (source, code) in [
            (
                "interface Change { writable function write(): void; } function bad(Change $view): void { $view->write(); }",
                "E0203",
            ),
            (
                "trait First { function read(): int { return 1; } } trait Second { function read(): int { return 2; } } class Box { uses First, Second; }",
                "E0757",
            ),
        ] {
            let snapshot = AnalysisSnapshot::analyze("interface-boundary.doria", source);
            assert!(
                snapshot
                    .diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.code == code),
                "{source}: {:?}",
                snapshot.diagnostics()
            );
            assert!(
                !snapshot
                    .diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.code == "E0758")
            );
        }
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

    static function fetch(string $path): string throws SecondError
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
    Worker::fetch("static");

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
            assert!(
                hover(source, needle, occurrence)
                    .markdown
                    .contains(expected)
            );
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
                "Worker::fetch(\"static\")",
                "\"static\"",
                "static function Worker::fetch(string $path): string throws SecondError",
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
                Some(vec![SignatureHelp {
                    label: label.to_string(),
                    active_parameter,
                }])
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
    fn readiness_callback_hovers_preserve_compiler_owned_return_types() {
        for callback in [
            "fn(take ReadinessSet $input) => $input",
            "function (take ReadinessSet $input): ReadinessSet { return $input; }",
            "when ($choose) { return fn(take ReadinessSet $input) => $input; } else { return function (take ReadinessSet $input): ReadinessSet { return $input; }; }",
        ] {
            let source = format!(
                r#"use Doria\Std\Io\{{ReadinessSet, IoError}};
function exercise(bool $choose): void throws IoError {{
    let $forward = {callback};
    let $returned = $forward(
        new ReadinessSet()
    );
}}
function main(): void {{}}
"#
            );
            let (_, compiler) =
                doriac::analyze_source_for_ide("readiness-callback-hover.doria", &source).unwrap();
            assert!(
                compiler.diagnostics.is_empty(),
                "{:?}",
                compiler.diagnostics
            );
            let snapshot = AnalysisSnapshot::analyze("readiness-callback-hover.doria", &source);
            assert!(
                snapshot.diagnostics().is_empty(),
                "{:?}",
                snapshot.diagnostics()
            );
            assert_eq!(
                compiler.info.closures.len(),
                if callback.starts_with("when") { 2 } else { 1 }
            );
            for closure in compiler.info.closures.values() {
                let hover = snapshot.hover_at_offset(closure.closure_id.start).unwrap();
                assert!(
                    hover.markdown.contains(&display_function_type_with_effects(
                        &closure.function_type,
                        &closure.required_checked_effects,
                    )),
                    "{}",
                    hover.markdown
                );
                let ResolvedType::Function(function) = &closure.function_type else {
                    panic!("closure must have a function type");
                };
                assert!(function.return_borrow.is_none_or(|borrow|
                    borrow.kind != doriac::types::ReturnBorrowKind::Value
                ));
            }
            let call = compiler.info.callable_value_calls.values().next().unwrap();
            let offset = source.find("$forward(\n").unwrap() + "$forward(\n".len();
            let hover = snapshot.hover_at_offset(offset).unwrap();
            assert!(
                hover.markdown.contains(&display_resolved_type(&call.return_type)),
                "{}",
                hover.markdown
            );
            assert!(
                hover.markdown.contains("Semantically checked callable-value invocation"),
                "{}",
                hover.markdown
            );
            assert!(!hover.markdown.contains("Returns a readonly borrow"));
        }
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
    fn indexed_foreach_hovers_project_compiler_owned_roles_types_and_access() {
        let source = r#"
class Window
{
    internal writable List<string> $entries = ["alpha"];
    function render(): void
    {
        foreach ($this->entries as int $line => string $content) {
            echo "{$line}:{$content}";
        }
    }
}
function generic<T>(List<T> $items): void
{
    foreach ($items as int $index => T $item) { echo $index; }
}
function main(): void
{
    string[] $letters = ["a"];
    foreach ($letters as int $offset => string $letter) { echo $offset; }
    Dictionary<string, int> $map = ["left" => 1];
    foreach ($map as string $id => int $number) { echo $id; }
    SortedDictionary<int, string> $sorted = SortedDictionary::from([1 => "one"]);
    foreach ($sorted as int $rank => string $label) { echo $rank; }
    writable List<int> $numbers = [1];
    foreach ($numbers as int $position => writable int $value) { $value += $position; }
}
"#;
        let snapshot = AnalysisSnapshot::analyze("indexed-foreach-hover.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "accepted indexed foreach fixture: {:#?}",
            snapshot.diagnostics()
        );

        for (needle, role, ty, iterable) in [
            (
                "$line",
                "Zero-Based Sequence Index",
                "int $line",
                "List<string>",
            ),
            (
                "$index",
                "Zero-Based Sequence Index",
                "int $index",
                "List<T>",
            ),
            (
                "$offset",
                "Zero-Based Sequence Index",
                "int $offset",
                "string[]",
            ),
            (
                "$id",
                "Dictionary Key",
                "string $id",
                "Dictionary<string, int>",
            ),
            (
                "$rank",
                "Dictionary Key",
                "int $rank",
                "SortedDictionary<int, string>",
            ),
        ] {
            let markdown = hover(source, needle, 0).markdown;
            assert!(markdown.contains(ty), "{needle}: {markdown}");
            assert!(markdown.contains(role), "{needle}: {markdown}");
            assert!(
                markdown.contains("**Access:** Readonly"),
                "{needle}: {markdown}"
            );
            assert!(markdown.contains(iterable), "{needle}: {markdown}");
        }

        let content = hover(source, "$content", 0).markdown;
        assert!(content.contains("string $content"));
        assert!(content.contains("Sequence Element"));
        assert!(content.contains("**Access:** Readonly"));
        assert!(hover(source, "$number", 0)
            .markdown
            .contains("Dictionary Value"));
        let writable = hover(source, "$value", 0).markdown;
        assert!(writable.contains("int $value"));
        assert!(writable.contains("**Access:** Writable"));

        for needle in ["$line", "$content", "$position", "$value"] {
            assert!(hover(source, needle, 1).markdown.contains(needle));
        }
    }

    #[test]
    fn inferred_foreach_recovery_types_do_not_publish_hovers() {
        let source = r#"
function main(): void
{
    List<string> $items = ["a"];
    foreach ($items as $index => $value) {}
}
"#;
        let snapshot = AnalysisSnapshot::analyze("indexed-foreach-recovery.doria", source);
        assert_eq!(
            snapshot
                .diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.code == "E0748")
                .count(),
            2
        );
        for binding in ["$index", "$value"] {
            let offset = source.rfind(binding).expect("foreach binding");
            assert!(
                snapshot.hover_at_offset(offset).is_none(),
                "recovery-only type published a hover for {binding}"
            );
        }
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
                Some(vec![SignatureHelp {
                    label: "function lookup(int $id, string $name): void".to_string(),
                    active_parameter,
                }])
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
