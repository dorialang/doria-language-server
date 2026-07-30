use std::collections::{HashMap, HashSet};

use doriac::ast::{
    Block, ClassDecl, ClassMember, ElseBranch, Expr, ForIncrement, ForInitializer, FunctionDecl,
    Item, MemberAccess, Program, StaticQualifier, Stmt,
};
use doriac::diagnostics::Diagnostic;
use doriac::lexer::{Token, TokenKind};
use doriac::semantics::{CallableTarget, SemanticInfo};
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
}

#[derive(Debug, Clone)]
struct Symbol {
    signature: String,
    documentation: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct Occurrence {
    span: Span,
    symbol: usize,
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

        let analysis = doriac::semantics::analyze_program_for_ide(&program);

        SnapshotBuilder::new(text, &tokens, Some(&analysis.info), analysis.diagnostics)
            .build(&program)
    }

    pub(crate) fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
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
}

struct SnapshotBuilder<'a> {
    text: &'a str,
    tokens: &'a [Token],
    semantic_info: Option<&'a SemanticInfo>,
    diagnostics: Vec<Diagnostic>,
    symbols: Vec<Symbol>,
    occurrences: Vec<Occurrence>,
    classes: HashMap<String, usize>,
    class_parents: HashMap<String, String>,
    methods: HashMap<(String, String), usize>,
    functions: HashMap<String, usize>,
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
            class_parents: HashMap::new(),
            methods: HashMap::new(),
            functions: HashMap::new(),
        }
    }

    fn build(mut self, program: &Program) -> AnalysisSnapshot {
        self.collect_declarations(program);
        self.collect_references(program);
        AnalysisSnapshot {
            diagnostics: self.diagnostics,
            symbols: self.symbols,
            occurrences: self.occurrences,
        }
    }

    fn collect_declarations(&mut self, program: &Program) {
        for item in &program.items {
            match item {
                Item::Class(class) => self.collect_class(class),
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
                    let symbol = self.add_symbol(
                        selection_span,
                        function_signature(function, None),
                        phpdoc_before(self.text, function.span.start),
                    );
                    self.functions.insert(function.name.clone(), symbol);
                }
                _ => {}
            }
        }
    }

    fn collect_class(&mut self, class: &ClassDecl) {
        let selection_span = self.declaration_name_span(class.span, &class.name, TokenKind::Class);
        let symbol = self.add_symbol(
            selection_span,
            class_signature(class),
            phpdoc_before(self.text, class.span.start),
        );
        self.classes.insert(class.name.clone(), symbol);
        if let Some(parent) = &class.parent {
            self.class_parents
                .insert(class.name.clone(), parent.clone());
        }

        for member in &class.members {
            if let ClassMember::Method(method) = member {
                self.collect_method(&class.name, method);
            }
        }
    }

    fn collect_method(&mut self, class_name: &str, method: &FunctionDecl) {
        let selection_span =
            self.declaration_name_span(method.span, &method.name, TokenKind::Function);
        let symbol = self.add_symbol(
            selection_span,
            function_signature(method, Some(class_name)),
            phpdoc_before(self.text, method.span.start),
        );
        self.methods
            .insert((class_name.to_string(), method.name.clone()), symbol);
    }

    fn add_symbol(
        &mut self,
        selection_span: Span,
        signature: String,
        documentation: Option<String>,
    ) -> usize {
        let symbol = self.symbols.len();
        self.symbols.push(Symbol {
            signature,
            documentation,
        });
        self.occurrences.push(Occurrence {
            span: selection_span,
            symbol,
        });
        symbol
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
        for item in &program.items {
            match item {
                Item::Class(class) => {
                    for member in &class.members {
                        if let ClassMember::Method(method) = member {
                            self.visit_block(
                                &method.body,
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
                Item::Trait(trait_decl) => {
                    for member in &trait_decl.members {
                        if let ClassMember::Method(method) = member {
                            self.visit_block(&method.body, Some(&trait_decl.name), None);
                        }
                    }
                }
                Item::Function(function) => self.visit_block(&function.body, None, None),
                Item::Constant(constant) => self.visit_expr(&constant.initializer, None, None),
                Item::Statement(statement) => self.visit_stmt(statement, None, None),
                _ => {}
            }
        }
    }

    fn visit_block(
        &mut self,
        block: &Block,
        current_class: Option<&str>,
        parent_class: Option<&str>,
    ) {
        for statement in &block.statements {
            self.visit_stmt(statement, current_class, parent_class);
        }
    }

    fn visit_stmt(
        &mut self,
        statement: &Stmt,
        current_class: Option<&str>,
        parent_class: Option<&str>,
    ) {
        match statement {
            Stmt::VarDecl(declaration) => {
                self.visit_expr(&declaration.initializer, current_class, parent_class)
            }
            Stmt::Assignment(assignment) => {
                self.visit_expr(&assignment.target, current_class, parent_class);
                self.visit_expr(&assignment.value, current_class, parent_class);
            }
            Stmt::Echo { expr, .. } => self.visit_expr(expr, current_class, parent_class),
            Stmt::Return {
                expr: Some(expr), ..
            } => self.visit_expr(expr, current_class, parent_class),
            Stmt::If(if_statement) => {
                self.visit_expr(&if_statement.condition, current_class, parent_class);
                self.visit_block(&if_statement.then_block, current_class, parent_class);
                if let Some(branch) = &if_statement.else_branch {
                    self.visit_else_branch(branch, current_class, parent_class);
                }
            }
            Stmt::While(while_statement) => {
                self.visit_expr(&while_statement.condition, current_class, parent_class);
                self.visit_block(&while_statement.body, current_class, parent_class);
            }
            Stmt::For(for_statement) => {
                if let Some(initializer) = &for_statement.initializer {
                    if let ForInitializer::VarDecl(declaration) = initializer {
                        self.visit_expr(&declaration.initializer, current_class, parent_class);
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

    fn visit_else_branch(
        &mut self,
        branch: &ElseBranch,
        current_class: Option<&str>,
        parent_class: Option<&str>,
    ) {
        if let ElseBranch::If(if_statement) = branch {
            self.visit_expr(&if_statement.condition, current_class, parent_class);
            self.visit_block(&if_statement.then_block, current_class, parent_class);
            if let Some(branch) = &if_statement.else_branch {
                self.visit_else_branch(branch, current_class, parent_class);
            }
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
            Expr::MethodCall {
                object,
                method,
                args,
                span,
                null_safe,
            } => {
                self.visit_expr(object, current_class, parent_class);
                for argument in args {
                    self.visit_expr(&argument.value, current_class, parent_class);
                }
                let method_span =
                    self.member_name_span(Span::new(object.span().end, span.end), method);
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
                    self.add_symbol(
                        method_span,
                        hover.signature,
                        Some(hover.documentation.to_string()),
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
                            self.occurrences.push(Occurrence {
                                span: method_span,
                                symbol,
                            });
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
                        self.occurrences.push(Occurrence {
                            span: name_span,
                            symbol,
                        });
                    }
                }
            }
            Expr::StaticCall {
                qualifier,
                qualifier_span,
                method,
                args,
                span,
                ..
            } => {
                for argument in args {
                    self.visit_expr(&argument.value, current_class, parent_class);
                }
                if matches!(qualifier, StaticQualifier::Class(class) if class == "String") {
                    if let (Some(member), Some(method_span)) = (
                        string_companion_method(method),
                        self.member_name_span(Span::new(qualifier_span.end, span.end), method),
                    ) {
                        self.add_symbol(
                            method_span,
                            member.signature.to_string(),
                            Some(member.documentation.to_string()),
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
                            self.occurrences.push(Occurrence {
                                span: method_span,
                                symbol,
                            });
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
                        self.occurrences.push(Occurrence {
                            span: name_span,
                            symbol,
                        });
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
                let is_string = self
                    .semantic_info
                    .and_then(|info| info.expression_type(object.span()))
                    .is_some_and(|ty| matches!(non_nullable_type(ty), ResolvedType::String));
                if is_string {
                    if let (Some(member), Some(property_span)) = (
                        string_property(property),
                        self.member_name_span(Span::new(object.span().end, span.end), property),
                    ) {
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
                        self.add_symbol(
                            property_span,
                            format!("{return_type} ${property}"),
                            Some(member.documentation.to_string()),
                        );
                    }
                }
            }
            Expr::StaticMember { .. } => {}
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
            // IDE analysis is best-effort across compiler feature branches. New
            // expression forms remain diagnostic-safe until their symbol-bearing
            // children need explicit traversal here.
            _ => {}
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

    fn member_name_span(&self, search_span: Span, name: &str) -> Option<Span> {
        find_identifier_span(self.tokens, search_span, name)
    }
}

struct CompilerKnownMethodHover {
    signature: String,
    documentation: &'static str,
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
        (ResolvedType::Dictionary(key, value), "set") => (
            format!(
                "{} $key, {} $value",
                display_resolved_type(key),
                display_resolved_type(value)
            ),
            "void".to_string(),
            "Stores a value for the key in this writable dictionary.",
        ),
        (ResolvedType::Dictionary(key, value), "get") => (
            format!("{} $key", display_resolved_type(key)),
            format!("?{}", display_resolved_type(value)),
            "Returns the value for the key, or `null` when the key is absent.",
        ),
        (ResolvedType::Dictionary(key, _), "has") => (
            format!("{} $key", display_resolved_type(key)),
            "bool".to_string(),
            "Reports whether this dictionary contains the key.",
        ),
        (ResolvedType::Dictionary(key, value), "remove") => (
            format!("{} $key", display_resolved_type(key)),
            format!("?{}", display_resolved_type(value)),
            "Removes and returns the value for the key, or `null` when the key is absent.",
        ),
        (ResolvedType::Set(value), "add") => (
            format!("{} $value", display_resolved_type(value)),
            "bool".to_string(),
            "Adds a value and reports whether the set changed.",
        ),
        (ResolvedType::Set(value), "remove") => (
            format!("{} $value", display_resolved_type(value)),
            "bool".to_string(),
            "Removes a value and reports whether the set changed.",
        ),
        (ResolvedType::Set(value), "contains") => (
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
        (ResolvedType::Bytes, "toArray") => (
            String::new(),
            "uint8[]".to_string(),
            "Copies this byte buffer into a fixed-length `uint8[]`.",
        ),
        _ => return None,
    };
    Some((parameters, return_type, documentation))
}

fn non_nullable_type(ty: &ResolvedType) -> &ResolvedType {
    match ty {
        ResolvedType::Nullable(inner) => inner,
        ty => ty,
    }
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
        ResolvedType::Set(element) => format!("Set<{}>", display_resolved_type(element)),
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
        .map(|parameter| {
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
        })
        .collect::<Vec<_>>()
        .join(", ");
    let return_type = function
        .return_type
        .as_ref()
        .map(|return_type| format!(": {return_type}"))
        .unwrap_or_default();

    format!("{prefix}function {name}{type_parameters}({parameters}){return_type}")
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
    }
}
