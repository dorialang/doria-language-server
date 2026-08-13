use std::collections::{HashMap, HashSet};

use doriac::ast::{
    Block, ClassDecl, ClassMember, ControlFlowFinally, DoWhileStmt, ElseBranch, EnumDecl, Expr,
    ForIncrement, ForInitializer, FunctionDecl, GivenPrelude, IfStmt, Item, MatchMode, MatchOrigin,
    MatchPattern, MemberAccess, Param, Program, StaticQualifier, Stmt, VarDecl, WhenExpression,
    WhileStmt,
};
use doriac::diagnostics::Diagnostic;
use doriac::enums::{EnumBackingType, EnumBackingValue};
use doriac::lexer::{Token, TokenKind};
use doriac::semantics::{CallableTarget, EnumSemanticInfo, SemanticInfo};
use doriac::source::Span;
use doriac::types::{ResolvedType, SharedHandleKind};

use crate::string_surface::{string_companion_method, string_property};

#[derive(Debug, Clone)]
pub(crate) struct SemanticHover {
    pub(crate) span: Span,
    pub(crate) markdown: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalCompletion {
    pub(crate) label: String,
    pub(crate) detail: String,
}

impl AnalysisSnapshot {
    pub(crate) fn analyze(path: &str, text: &str) -> Self {
        let tokens = doriac::lex_source(path.to_string(), text.to_string()).unwrap_or_default();
        let program = match doriac::parse_source(path.to_string(), text.to_string()) {
            Ok(program) => program,
            Err(diagnostics) => {
                return Self {
                    diagnostics,
                    ..Self::default()
                };
            }
        };

        let analysis = doriac::semantics::analyze_program_for_ide_with_source(&program, Some(text));

        SnapshotBuilder::new(text, &tokens, Some(&analysis.info), analysis.diagnostics)
            .build(&program)
    }

    pub(crate) fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    #[cfg(test)]
    pub(crate) fn from_diagnostics(diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            diagnostics,
            ..Self::default()
        }
    }

    pub(crate) fn hover_at_offset(&self, offset: usize) -> Option<SemanticHover> {
        let occurrence = self
            .occurrences
            .iter()
            .filter(|occurrence| span_contains(occurrence.span, offset))
            .min_by_key(|occurrence| occurrence.span.end.saturating_sub(occurrence.span.start))?;
        let symbol = self.symbols.get(occurrence.symbol)?;
        let mut markdown = format!("```doria\n{}\n```", symbol.signature);
        if let Some(documentation) = &symbol.documentation {
            markdown.push_str("\n\n");
            markdown.push_str(documentation);
        }

        Some(SemanticHover {
            span: occurrence.span,
            markdown,
        })
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
    if symbol.signature.contains("::") {
        return Some(2);
    }
    if symbol.signature.starts_with("function ") {
        return Some(3);
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
    enums: HashMap<String, usize>,
    enum_cases: HashMap<(String, String), usize>,
    class_parents: HashMap<String, String>,
    class_members: HashMap<String, Vec<ClassMemberCompletion>>,
    enum_case_completions: HashMap<String, Vec<SemanticCompletion>>,
    enum_member_completions: HashMap<String, Vec<SemanticCompletion>>,
    methods: HashMap<(String, String), usize>,
    functions: HashMap<String, usize>,
    member_receivers: Vec<MemberReceiver>,
    static_receivers: Vec<StaticReceiver>,
    local_scopes: Vec<HashMap<String, usize>>,
    local_scope_ends: Vec<usize>,
    local_visibilities: Vec<LocalVisibility>,
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
            enums: HashMap::new(),
            enum_cases: HashMap::new(),
            class_parents: HashMap::new(),
            class_members: HashMap::new(),
            enum_case_completions: HashMap::new(),
            enum_member_completions: HashMap::new(),
            methods: HashMap::new(),
            functions: HashMap::new(),
            member_receivers: Vec::new(),
            static_receivers: Vec::new(),
            local_scopes: Vec::new(),
            local_scope_ends: Vec::new(),
            local_visibilities: Vec::new(),
            when_depth: 0,
        }
    }

    fn build(mut self, program: &Program) -> AnalysisSnapshot {
        self.collect_declarations(program);
        self.collect_references(program);
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
                    let symbol = self.add_declaration_symbol(
                        selection_span,
                        function_signature(function, None),
                        phpdoc_before(self.text, function.span.start),
                        SymbolKind::Plain,
                    );
                    self.functions.insert(function.name.clone(), symbol);
                }
                _ => {}
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
        let symbol = self.add_declaration_symbol(
            selection_span,
            class_signature(class),
            phpdoc_before(self.text, class.span.start),
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
                    self.class_members
                        .entry(class.name.clone())
                        .or_default()
                        .push(ClassMemberCompletion {
                            completion: SemanticCompletion {
                                label: property.name.clone(),
                                kind: 10,
                                detail: format!("{} ${}", property.ty, property.name),
                                documentation: phpdoc_before(self.text, property.span.start),
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
        for parameter in &function.params {
            self.declare_parameter(parameter, block.span.start);
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
                "Accepted control-flow finalizer syntax. Execution currently reports the compiler's pending-finalizer diagnostic."
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

    fn declare_parameter(&mut self, parameter: &Param, visibility_start: usize) {
        let selection_span = find_variable_span(self.tokens, parameter.span, &parameter.name)
            .unwrap_or(parameter.span);
        self.declare_local_binding(
            &parameter.name,
            selection_span,
            parameter_signature(parameter),
            visibility_start,
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

    fn record_match_type_reference(&mut self, ty: &doriac::types::TypeRef, span: Span) {
        let Some(type_span) = find_identifier_span(self.tokens, span, &ty.name) else {
            return;
        };
        if let Some(symbol) = self
            .enums
            .get(&ty.name)
            .or_else(|| self.classes.get(&ty.name))
            .copied()
        {
            self.record_reference(type_span, symbol);
        }
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

fn display_resolved_type(ty: &ResolvedType) -> String {
    match ty {
        ResolvedType::Void => "void".to_string(),
        ResolvedType::Integer(integer) => integer.to_string(),
        ResolvedType::Float(float) => float.to_string(),
        ResolvedType::String => "string".to_string(),
        ResolvedType::Bytes => "Bytes".to_string(),
        ResolvedType::Bool => "bool".to_string(),
        ResolvedType::Null => "null".to_string(),
        ResolvedType::Mixed => "mixed".to_string(),
        ResolvedType::TypeParameter(name) => name.clone(),
        ResolvedType::Nullable(inner) => format!("?{}", display_resolved_type(inner)),
        ResolvedType::Enum(enum_type) => enum_type.name.clone(),
        ResolvedType::Class(class) => {
            if class.arguments.is_empty() {
                class.name.clone()
            } else {
                format!(
                    "{}<{}>",
                    class.name,
                    class
                        .arguments
                        .iter()
                        .map(display_resolved_type)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }
        ResolvedType::TypedArray(element) => format!("{}[]", display_resolved_type(element)),
        ResolvedType::List(element) => format!("List<{}>", display_resolved_type(element)),
        ResolvedType::Dictionary(key, value) => format!(
            "Dictionary<{}, {}>",
            display_resolved_type(key),
            display_resolved_type(value)
        ),
        ResolvedType::SortedDictionary(key, value) => format!(
            "SortedDictionary<{}, {}>",
            display_resolved_type(key),
            display_resolved_type(value)
        ),
        ResolvedType::Set(element) => format!("Set<{}>", display_resolved_type(element)),
        ResolvedType::SortedSet(element) => {
            format!("SortedSet<{}>", display_resolved_type(element))
        }
        ResolvedType::PriorityQueue(element) => {
            format!("PriorityQueue<{}>", display_resolved_type(element))
        }
        ResolvedType::Deque(element) => format!("Deque<{}>", display_resolved_type(element)),
        ResolvedType::SharedHandle(kind, payload) => {
            format!("{}<{}>", kind.source_name(), display_resolved_type(payload))
        }
        ResolvedType::Unsupported => "Unknown".to_string(),
    }
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

    format!("{prefix}function {name}{type_parameters}({parameters}){return_type}")
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
    fn pending_finally_keeps_ast_hover_and_compiler_diagnostic() {
        let source = r#"function main(): void
{
    if (true) {
        echo "body";
    } /* 😀 */ finally {
        echo "cleanup";
    }
}
"#;
        let snapshot = AnalysisSnapshot::analyze("finally.doria", source);
        assert!(snapshot
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == "E0611"));
        assert!(snapshot
            .hover_at_offset(source.find("finally").unwrap())
            .expect("finally hover")
            .markdown
            .contains("pending-finalizer diagnostic"));
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

function greet(string $name): void
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
}
