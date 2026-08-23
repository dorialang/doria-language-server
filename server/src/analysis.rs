use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use doriac::ast::{
    Block, ClassDecl, ClassMember, ControlFlowFinally, DoWhileStmt, ElseBranch, EnumDecl, Expr,
    ForIncrement, ForInitializer, FunctionDecl, GivenPrelude, IfStmt, Item, MatchMode, MatchOrigin,
    MatchPattern, MemberAccess, Param, Program, StaticQualifier, Stmt, TryStmt, VarDecl,
    WhenExpression, WhileStmt,
};
use doriac::diagnostics::Diagnostic;
use doriac::enums::{EnumBackingType, EnumBackingValue};
use doriac::lexer::{Token, TokenKind};
use doriac::ownership::{
    CaptureAcquisitionKind, ClosureBorrowRoot, ClosureEscapeClassification, ClosureValueProvenance,
    InvocationConsumption,
};
use doriac::semantics::{CallableTarget, EnumSemanticInfo, SemanticInfo};
use doriac::source::Span;
use doriac::symbols::{BindingKind, BindingOwnership};
use doriac::types::{
    FunctionInvocationMode, ResolvedType, SharedHandleKind, TypeRef, TypeRegistry,
};

use crate::string_surface::{string_companion_method, string_property};

#[derive(Debug, Clone)]
pub(crate) struct SemanticHover {
    pub(crate) span: Span,
    pub(crate) markdown: String,
    priority: SemanticHoverPriority,
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
        let tokens = doriac::lex_source(path.to_string(), text.to_string()).unwrap_or_default();
        let (program, analysis) =
            match doriac::analyze_source_for_ide(path.to_string(), text.to_string()) {
                Ok(analysis) => analysis,
                Err(diagnostics) => {
                    return Self {
                        diagnostics,
                        ..Self::default()
                    };
                }
            };

        SnapshotBuilder::new(text, &tokens, Some(&analysis.info), analysis.diagnostics)
            .build(&program)
    }

    pub(crate) fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
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

    pub(crate) fn semantic_token_spans(&self) -> Vec<(Span, u32)> {
        let mut spans = self
            .occurrences
            .iter()
            .filter_map(|occurrence| {
                let symbol = self.symbols.get(occurrence.symbol)?;
                semantic_token_type(symbol).map(|token_type| (occurrence.span, token_type))
            })
            .collect::<Vec<_>>();
        spans.sort_by_key(|(span, _)| (span.start, span.end));
        spans.dedup_by_key(|(span, _)| (span.start, span.end));
        spans
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

struct SnapshotBuilder<'a> {
    text: &'a str,
    tokens: &'a [Token],
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
    when_depth: usize,
}

impl<'a> SnapshotBuilder<'a> {
    fn new(
        text: &'a str,
        tokens: &'a [Token],
        semantic_info: Option<&'a SemanticInfo>,
        diagnostics: Vec<Diagnostic>,
    ) -> Self {
        Self {
            text,
            tokens,
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
            when_depth: 0,
        }
    }

    fn build(mut self, program: &Program) -> AnalysisSnapshot {
        self.collect_declarations(program);
        self.collect_semantic_only_declarations();
        self.collect_references(program);
        self.collect_semantic_hovers();
        AnalysisSnapshot {
            diagnostics: self.diagnostics,
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
        }
    }

    fn collect_semantic_hovers(&mut self) {
        let Some(info) = self.semantic_info else {
            return;
        };
        let mut hovers = Vec::new();

        let mut function_types = info.function_types_by_span.iter().collect::<Vec<_>>();
        function_types.sort_by_key(|(span, _)| **span);
        for ((start, end), semantic) in function_types {
            hovers.push(SemanticHover::new(
                Span::new(*start, *end),
                format!(
                    "```doria\n{}\n```\n\nCanonical semantic function type.",
                    display_function_type_with_effects(
                        &semantic.ty,
                        &semantic.authored_checked_effects,
                    )
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
                matches!(ty, ResolvedType::Function(_)).then_some((
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
                .filter_map(|((start, end), resolved)| {
                    (*resolved == binding_id).then_some(Span::new(*start, *end))
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
        for closure in closures {
            let ownership = info.closure_ownership.get(&closure.closure_id);
            let signature = display_function_type_with_effects(
                &closure.function_type,
                &closure.inferred_checked_effects,
            );
            let effects = if closure.inferred_checked_effects.is_empty() {
                "none".to_string()
            } else {
                closure
                    .inferred_checked_effects
                    .iter()
                    .map(display_resolved_type)
                    .collect::<Vec<_>>()
                    .join(", ")
            };
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
                Span::new(closure.closure_id.start, closure.closure_id.end),
                format!(
                    "```doria\n{signature}\n```\n\n**Inferred invocation mode:** `{}`\n\n**Inferred checked effects:** {effects}\n\n**Ownership:** {ownership_summary}\n\n**Invocation:** {invocation}\n\n**Escape:** {escape}\n\n**Captures:** {captures}\n\nClosure execution remains behind the Stage 30d HIR/MIR/runtime boundary.",
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
                let markdown = format!(
                    "```doria\n{} {}\n```\n\n{} capture of `{name}`.",
                    display_resolved_type(&capture.source_type),
                    name,
                    acquisition.map_or("Compiler-resolved", |acquisition| {
                        capture_acquisition_name(acquisition.kind)
                    }),
                );
                hovers.push(SemanticHover::capture(
                    capture.declaration_span,
                    markdown.clone(),
                ));
                hovers.extend(
                    capture
                        .use_spans
                        .iter()
                        .map(|span| SemanticHover::capture(*span, markdown.clone())),
                );
            }
        }

        let mut callable_calls = info.callable_value_calls.iter().collect::<Vec<_>>();
        callable_calls.sort_by_key(|(span, _)| **span);
        for ((start, end), call) in callable_calls {
            hovers.push(SemanticHover::new(
                Span::new(*start, *end),
                format!(
                    "```doria\n{}\n```\n\nSemantically checked callable-value invocation returning `{}`. Execution remains behind the Stage 30d HIR/MIR/runtime boundary.",
                    display_function_type_with_effects(&call.function_type, &call.checked_effects),
                    display_resolved_type(&call.return_type),
                ),
            ));
        }

        self.semantic_hovers.extend(hovers);
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
                    let symbol = self.add_declaration_symbol(
                        selection_span,
                        function_signature(function, None),
                        phpdoc_before(self.text, function.span.start),
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
                ClassMember::Constant(_) => {}
            }
        }
    }

    fn collect_method(&mut self, class_name: &str, method: &FunctionDecl) {
        let selection_span =
            self.declaration_name_span(method.span, &method.name, TokenKind::Function);
        let symbol = self.add_declaration_symbol(
            selection_span,
            function_signature(method, Some(class_name)),
            phpdoc_before(self.text, method.span.start),
            SymbolKind::Plain,
        );
        self.record_callable_parameters(symbol, method);
        self.methods
            .insert((class_name.to_string(), method.name.clone()), symbol);
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
            Stmt::Expr { expr, .. } => self.visit_expr(expr, current_class, parent_class),
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
                    .and_then(|info| {
                        info.catch_error_types
                            .get(&(catch.span.start, catch.span.end))
                    })
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
                    "Runs once after the protected block or selected catch. Checked errors may not escape this finalizer."
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
            .and_then(|info| info.given_preludes.get(&(given.span.start, given.span.end)))
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
                "Runs exactly once when the attached control-flow construct leaves normally or through a structured transfer. Fatal panic bypasses it."
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
                    }) if method_name == method => Some(class_type.name.as_str()),
                    _ if matches!(object.as_ref(), Expr::This { .. }) => current_class,
                    _ => None,
                };
                if let Some(class_name) = resolved_class {
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
                    }) if method_name == method => Some(class_type.name.as_str()),
                    _ if matches!(qualifier, StaticQualifier::SelfType) => current_class,
                    _ if matches!(qualifier, StaticQualifier::Parent) => parent_class,
                    _ => None,
                };
                if let Some(class_name) = class_name {
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
                self.record_enum_static_reference(qualifier, *qualifier_span, member, *member_span);
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
                    .and_then(|info| info.matches.get(&(span.start, span.end)))
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
                .and_then(|semantic| semantic.whens.get(&(when.span.start, when.span.end))),
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
    documentation: &'static str,
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
        documentation,
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
        ClosureValueProvenance::Owned => "Owned closure".to_string(),
        ClosureValueProvenance::BorrowBound(roots) => {
            let roots = closure_root_names(info, roots);
            if roots.is_empty() {
                "Borrow-bound closure".to_string()
            } else {
                format!("Borrow-bound closure tied to {}", roots.join(", "))
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
                |(span, token_type)| *span == Span::new(binding_offset, binding_offset + 6)
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
            .any(|(span, token_type)| span.start == prepared && *token_type == 0));

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
                    tokens
                        .iter()
                        .any(|(token_span, token_type)| *token_span == span && *token_type == 3),
                    "method `{name}` at {span:?} must use the function token type"
                );
            }
        }

        for (start, _) in source.match_indices("Ready") {
            let span = Span::new(start, start + "Ready".len());
            assert!(
                tokens
                    .iter()
                    .any(|(token_span, token_type)| *token_span == span && *token_type == 2),
                "enum case at {span:?} must retain the enum-member token type"
            );
        }
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
            .any(|(span, token_type)| span.start == throws_offset && *token_type == 4));

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
            .any(|(span, token_type)| span.start == caught && *token_type == 0));

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
        assert!(!hover.markdown.contains("throws"));
    }

    #[test]
    fn capture_metadata_wins_ties_with_function_typed_binding_hovers() {
        let source = r#"let $callback = fn() => 1;
let $wrapper = fn() with ($callback) => $callback();
"#;
        let snapshot = AnalysisSnapshot::analyze("capture-hover.doria", source);
        assert!(
            snapshot
                .diagnostics()
                .iter()
                .all(|diagnostic| diagnostic.code == "E0641"),
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
            "Borrow-bound closure",
            "Readonly capture of `$read`",
            "Writable capture of `$write`",
            "Owned taking capture of `$copy`",
            "Writable Repeatable",
            "Nonescaping",
            "Stage 30d HIR/MIR/runtime boundary",
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
        assert!(once.markdown.contains("Owned closure"));
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
        let source = r#"function keep(function(): int $borrowed, take function(): int $owned): void {}
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
