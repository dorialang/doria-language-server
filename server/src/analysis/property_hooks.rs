use super::*;
use doriac::ast::{PropertyDecl, PropertyHookKind};
use doriac::property_hooks::{declaration_facts, PropertyBackingField, PropertyHookStorage};

// Accessor keywords are navigation targets, never property-rename occurrences.
#[derive(Debug, Clone, Default)]
pub(crate) struct PropertyAccessorNavigation {
    pub(crate) declarations: HashMap<Span, Span>,
    pub(crate) references: HashMap<Span, Vec<Span>>,
}

impl SnapshotBuilder<'_> {
    pub(super) fn collect_property_hooks(
        &mut self,
        owner: &str,
        property: &PropertyDecl,
        context: PropertyHookContext,
        documentation: &mut Option<String>,
    ) {
        let Some(facts) = declaration_facts(property, context) else {
            return;
        };
        let storage = storage_documentation(
            facts.storage,
            self.semantic_info
                .and_then(|info| info.property_backing_fields.get(&property.span)),
        );
        append_documentation(documentation, &storage);
        for span in [property.open_span, property.override_span]
            .into_iter()
            .flatten()
        {
            self.hierarchy_semantic_tokens
                .push((span, SEMANTIC_TOKEN_KEYWORD, 0));
        }
        for (accessor, callable) in facts.accessors().zip(facts.callables()) {
            let hook = accessor.hook;
            self.property_accessors
                .declarations
                .insert(hook.span, hook.keyword_span);
            for span in [
                Some(hook.keyword_span),
                hook.writable_span,
                hook.borrowed_span,
            ]
            .into_iter()
            .flatten()
            {
                self.hierarchy_semantic_tokens
                    .push((span, SEMANTIC_TOKEN_KEYWORD, 0));
            }
            let mut signature = format!("{owner}::${} {{ ", property.name);
            // Setters already imply a writable receiver; only getters spell it.
            if accessor.receiver_mode == ReceiverMode::Writable
                && hook.kind == PropertyHookKind::Get
            {
                signature.push_str("writable ");
            }
            if hook.borrowed_span.is_some() {
                signature.push_str("borrowed ");
            }
            match hook.kind {
                PropertyHookKind::Get => signature.push_str(&format!("get: {}", property.ty)),
                PropertyHookKind::Set => {
                    signature.push_str(&format!(
                        "set({}): void",
                        hook.parameter
                            .as_ref()
                            .map(parameter_signature_without_default)
                            .unwrap_or_default()
                    ));
                }
            }
            signature.push_str(" }");
            let mut docs = Some(format!(
                "{} receiver.",
                receiver_label(accessor.receiver_mode)
            ));
            if hook.body.as_block().is_none() {
                append_documentation(
                    &mut docs,
                    "Declared accessor requirement, not an executable body.",
                );
            }
            self.append_callable_effect_documentation(&mut docs, hook.span);
            if let Some(info) = self.semantic_info {
                if let Some(signature) = info.callable_signatures.get(&hook.span) {
                    append_documentation(
                        &mut docs,
                        &callable_return_documentation(
                            signature,
                            info.return_borrows.get(&hook.span).copied(),
                        ),
                    );
                }
                if let Some(hierarchy) = info.method_hierarchy.get(&hook.span) {
                    append_documentation(&mut docs, &method_hierarchy_documentation(hierarchy));
                }
            }
            let markdown = format!(
                "```doria\n{signature}\n```\n\n{}",
                docs.as_deref().unwrap_or_default()
            );
            for span in [
                Some(hook.keyword_span),
                hook.writable_span,
                hook.borrowed_span,
            ]
            .into_iter()
            .flatten()
            {
                self.semantic_hovers
                    .push(SemanticHover::new(span, format!("{markdown}\n\n{storage}")));
            }
            append_documentation(documentation, &markdown);
            let symbol = self.add_metadata_symbol(signature, docs, SymbolKind::Plain);
            self.record_callable_parameters(symbol, &callable);
            self.callable_declarations.insert(hook.span, symbol);
        }
    }

    pub(super) fn has_composed_callable(&self, declaration: Span) -> bool {
        self.semantic_info.is_some_and(|info| {
            info.composition
                .origins
                .iter()
                .filter(|origin| {
                    origin.authored_declaration.source == declaration.source
                        && origin.authored_declaration.start <= declaration.start
                        && origin.authored_declaration.end >= declaration.end
                })
                .filter_map(|origin| info.composition.class(&origin.composing_class))
                .any(|class| {
                    class
                        .members
                        .iter()
                        .flat_map(|member| member_callables(member, PropertyHookContext::Class))
                        .any(|callable| {
                            callable.span != declaration && callable.span.authored() == declaration
                        })
                })
        })
    }

    pub(super) fn record_property_accessor_reference(
        &mut self,
        access_span: Span,
        member_span: Span,
        name: &str,
    ) {
        let Some(info) = self.semantic_info else {
            return;
        };
        let Some(calls) = info.property_accessor_calls.get(&access_span) else {
            return;
        };
        let mut declarations = Vec::new();
        let mut sections = Vec::new();
        for (kind, call) in [
            (PropertyHookKind::Get, &calls.getter),
            (PropertyHookKind::Set, &calls.setter),
        ] {
            let Some(call) = call else {
                continue;
            };
            declarations.push(call.declaration);
            let signature = CallableSignatureSemanticInfo {
                generic_parameter_count: 0,
                parameters: call.parameter.iter().cloned().collect(),
                return_type: call.return_type.clone(),
            };
            let detail = accessor_signature(
                name,
                kind,
                &signature,
                call.receiver_mode,
                &call.checked_effects,
                &HashMap::new(),
            );
            let mut docs = format!(
                "{} receiver on `{}`.",
                receiver_label(call.receiver_mode),
                display_resolved_type(&call.declaring_type)
            );
            docs.push_str(&callable_return_documentation(
                &signature,
                call.return_borrow,
            ));
            if call.virtual_root.is_some() {
                docs.push_str("\n\nVirtual property accessor selected by the compiler.");
            }
            sections.push(format!("```doria\n{detail}\n```\n\n{docs}"));
        }
        // Interface/constrained calls may have several merged contract origins.
        let origins = info
            .contracts
            .member_references
            .iter()
            .filter(|reference| reference.span == member_span)
            .flat_map(|reference| reference.origins.iter().copied())
            .collect::<Vec<_>>();
        if !origins.is_empty() {
            declarations = origins;
        }
        if declarations.is_empty() {
            return;
        }
        self.property_accessors
            .references
            .insert(member_span, declarations);
        self.hierarchy_semantic_tokens
            .push((member_span, SEMANTIC_TOKEN_PROPERTY, 0));
        self.semantic_hovers.push(SemanticHover::new(
            member_span,
            sections.join("\n\n---\n\n"),
        ));
    }
}

fn receiver_label(mode: ReceiverMode) -> &'static str {
    if mode == ReceiverMode::Writable {
        "Writable"
    } else {
        "Readonly"
    }
}

pub(super) fn accessor_signature(
    name: &str,
    kind: PropertyHookKind,
    signature: &CallableSignatureSemanticInfo,
    receiver: ReceiverMode,
    effects: &[ResolvedType],
    bindings: &HashMap<String, ResolvedType>,
) -> String {
    let display = |ty: &ResolvedType| {
        display_resolved_type(&doriac::types::substitute_resolved_type(ty, bindings))
    };
    let parameters = signature
        .parameters
        .iter()
        .map(|parameter| {
            let mode = if parameter.take {
                "take "
            } else if parameter.borrow {
                "borrow "
            } else if parameter.writable {
                "writable "
            } else {
                ""
            };
            format!("{mode}{} ${}", display(&parameter.r#type), parameter.name)
        })
        .collect::<Vec<_>>()
        .join(", ");
    let accessor = match kind {
        PropertyHookKind::Get => format!(
            "{}get: {}",
            if receiver == ReceiverMode::Writable {
                "writable "
            } else {
                ""
            },
            display(&signature.return_type)
        ),
        PropertyHookKind::Set => format!("set({parameters}): void"),
    };
    let throws = if effects.is_empty() {
        String::new()
    } else {
        format!(
            " throws {}",
            effects.iter().map(display).collect::<Vec<_>>().join(", ")
        )
    };
    format!("${name} {{ {accessor}{throws} }}")
}

pub(super) fn surface_documentation(
    name: &str,
    hooks: &doriac::semantics::composition::PropertyHooksSurface,
    bindings: &HashMap<String, ResolvedType>,
    backing_field: Option<&PropertyBackingField>,
) -> String {
    let mut sections = vec![storage_documentation(hooks.storage, backing_field)];
    for (kind, accessor) in [
        (PropertyHookKind::Get, &hooks.getter),
        (PropertyHookKind::Set, &hooks.setter),
    ] {
        let Some(accessor) = accessor else {
            continue;
        };
        let signature = accessor_signature(
            name,
            kind,
            &accessor.signature,
            accessor.receiver_mode,
            &accessor.checked_effects,
            bindings,
        );
        let docs = callable_details_documentation(
            Some(&accessor.signature),
            accessor.return_borrow,
            &accessor.automatic_effects,
        );
        sections.push(format!(
            "```doria\n{signature}\n```\n\n{} receiver.{}",
            receiver_label(accessor.receiver_mode),
            docs.map(|docs| format!("\n\n{docs}")).unwrap_or_default()
        ));
    }
    sections.join("\n\n")
}

fn storage_documentation(
    storage: Option<PropertyHookStorage>,
    backing_field: Option<&PropertyBackingField>,
) -> String {
    let mut documentation = match storage {
        Some(PropertyHookStorage::Backed) => "Automatically backed property. Direct access to this property on `$this` within its own hook or lexical closure uses backing storage. Backing initializers run once per object, parent then child, without invoking setters. A child initializer replaces the inherited value in the reused field.",
        Some(PropertyHookStorage::Computed) if backing_field.is_some() => "Computed property; inherited backing storage is retained.",
        Some(PropertyHookStorage::Computed) => "Computed property; no backing storage slot declared here.",
        None => "Interface property contract; no instance storage.",
    }.to_string();
    if let Some(field) = backing_field {
        documentation.push_str(&format!(
            "\n\nBacking field: `{}::${}`.",
            field.declaring_class, field.property_name
        ));
    }
    documentation
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_index::OpenDocumentIndex;

    fn analyze(source: &str) -> AnalysisSnapshot {
        let snapshot = AnalysisSnapshot::analyze("hooks.doria", source);
        assert!(
            snapshot.diagnostics().is_empty(),
            "unexpected compiler diagnostic: {:?}",
            snapshot.diagnostics()
        );
        snapshot
    }

    fn hover(snapshot: &AnalysisSnapshot, source: &str, needle: &str) -> String {
        snapshot
            .hover_at_offset(source.find(needle).unwrap())
            .unwrap()
            .markdown
    }

    #[test]
    fn shared_hook_fixture_is_accepted_and_preserves_compiler_facts() {
        let source = include_str!("../../../editors/fixtures/stage36-property-hooks.doria");
        let snapshot = analyze(source);
        let (program, compiler) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
        assert!(
            compiler.diagnostics.is_empty(),
            "{:?}",
            compiler.diagnostics
        );
        for item in &program.items {
            let properties = match item {
                Item::Class(class) => class
                    .members
                    .iter()
                    .filter_map(|member| match member {
                        ClassMember::Property(property) => Some(property),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                Item::Trait(declaration) => declaration
                    .members
                    .iter()
                    .filter_map(|member| match member {
                        ClassMember::Property(property) => Some(property),
                        _ => None,
                    })
                    .collect(),
                Item::Interface(declaration) => declaration.properties.iter().collect(),
                _ => Vec::new(),
            };
            for property in properties {
                for hook in &property.hooks {
                    for span in [
                        Some(hook.keyword_span),
                        hook.writable_span,
                        hook.borrowed_span,
                    ]
                    .into_iter()
                    .flatten()
                    {
                        assert!(
                            snapshot.semantic_token_spans().contains(&(
                                span,
                                SEMANTIC_TOKEN_KEYWORD,
                                0
                            )),
                            "{span:?}"
                        );
                    }
                    assert!(!snapshot
                        .member_occurrences
                        .iter()
                        .any(|occurrence| occurrence.declaration
                            && occurrence.span == hook.keyword_span));
                }
            }
        }
        assert!(hover(&snapshot, source, "$fahrenheit {").contains("Computed property"));
        assert!(hover(&snapshot, source, "$name =")
            .contains("parent then child, without invoking setters"));
        let inherited = hover(&snapshot, source, "$reading = 2");
        assert!(
            inherited.contains("Backing field: `Editor\\Hooks\\Gauge::$reading`"),
            "{inherited}"
        );
        assert!(
            inherited.contains("A child initializer replaces the inherited value"),
            "{inherited}"
        );
        assert!(hover(&snapshot, source, "writable get {").contains("Writable receiver"));
        assert!(hover(&snapshot, source, "get throws HookFailure").contains("HookFailure"));
        assert!(hover(&snapshot, source, "get(): int").contains("function"));
    }

    #[test]
    fn property_operations_navigate_to_accessors_but_backing_access_does_not() {
        let source = r#"
class Counter {
    writable int $value = 0 {
        get => $this->value;
        set (int $next) => $this->value = $next;
    }
}
function useCounter(writable Counter $counter): void {
    echo $counter->value;
    $counter->value = 1;
    $counter->value += 2;
    $counter->value++;
}
"#;
        let snapshot = analyze(source);
        let index = OpenDocumentIndex::rebuild(std::iter::once((
            "graph".to_string(),
            "hooks.doria",
            &snapshot,
        )));
        let get = source.find("get =>").unwrap();
        let set = source.find("set (int").unwrap();
        for (usage, expected) in [
            ("$counter->value;", vec![get]),
            ("$counter->value =", vec![set]),
            ("$counter->value +=", vec![get, set]),
            ("$counter->value++", vec![get, set]),
        ] {
            let offset = source.find(usage).unwrap() + "$counter->".len();
            let mut targets = index
                .property_accessor_definitions("hooks.doria", offset)
                .unwrap()
                .iter()
                .map(|location| location.span.start)
                .collect::<Vec<_>>();
            targets.sort_unstable();
            assert_eq!(targets, expected, "{usage}");
            let tokens = snapshot.semantic_token_spans();
            let member_tokens = tokens
                .iter()
                .filter(|(span, _, _)| span.start == offset)
                .collect::<Vec<_>>();
            assert_eq!(member_tokens.len(), 1, "{usage}: {member_tokens:?}");
            assert_eq!(member_tokens[0].1, SEMANTIC_TOKEN_PROPERTY, "{usage}");
        }
        let backing = source.find("$this->value").unwrap() + "$this->".len();
        assert!(index
            .property_accessor_definitions("hooks.doria", backing)
            .is_none());
        assert_eq!(
            index.definition("hooks.doria", backing).unwrap().span.start,
            source.find("$value =").unwrap()
        );
        let parameter = source.rfind("$next").unwrap();
        assert_eq!(
            snapshot
                .declaration_span_at_offset(parameter)
                .unwrap()
                .start,
            source.find("$next").unwrap()
        );
    }

    #[test]
    fn interface_and_constrained_hovers_keep_result_borrow_and_specialized_types() {
        let source = r#"
class Book {}
interface Shelf<T> { T $book { borrowed get; } }
interface Factory { Book $book { get; } }
function read(Shelf<Book> $shelf): Book { return $shelf->book; }
function generic<T implements Shelf<Book>>(T $shelf): Book { return $shelf->book; }
function create(Factory $factory): Book { return $factory->book; }
"#;
        let snapshot = analyze(source);
        for (offset, _) in source.match_indices("$shelf->book") {
            let offset = offset + "$shelf->".len();
            let text = snapshot.hover_at_offset(offset).unwrap().markdown;
            assert!(text.contains("get: Book"), "{text}");
            assert!(
                text.contains("Returns a readonly borrow rooted in the receiver"),
                "{text}"
            );
        }
        let owned = source.find("$factory->book").unwrap() + "$factory->".len();
        let text = snapshot.hover_at_offset(owned).unwrap().markdown;
        assert!(text.contains("get: Book"), "{text}");
        assert!(!text.contains("Returns a readonly borrow"), "{text}");
        let completion = snapshot.member_completions_at_offset(owned).unwrap();
        assert_eq!(
            completion
                .iter()
                .filter(|item| item.label == "book")
                .count(),
            1
        );
        assert_eq!(
            completion
                .iter()
                .find(|item| item.label == "book")
                .unwrap()
                .kind,
            10
        );
    }

    #[test]
    fn class_completions_use_accessor_receiver_contracts_not_property_writability() {
        let source = r#"
class Panel {
    writable int $stored = 0;
    writable int $readWrite = 0 {
        get => $this->readWrite;
        set (int $next) => $this->readWrite = $next;
    }
    int $cache { writable get => 42; }
    writable int $sink { set (int $next) {} }
}
function inspect(Panel $panel): int { return $panel->stored; }
function edit(writable Panel $panel): int { return $panel->stored; }
"#;
        let snapshot = analyze(source);
        let accesses = source
            .match_indices("$panel->stored")
            .map(|(offset, _)| offset + "$panel->".len())
            .collect::<Vec<_>>();
        for (offset, writable) in [(accesses[0], false), (accesses[1], true)] {
            let completions = snapshot.member_completions_at_offset(offset).unwrap();
            for name in ["stored", "readWrite"] {
                assert!(
                    completions.iter().any(|item| item.label == name),
                    "{completions:?}"
                );
            }
            for name in ["cache", "sink"] {
                assert_eq!(
                    completions.iter().any(|item| item.label == name),
                    writable,
                    "{completions:?}"
                );
            }
            let property = completions
                .iter()
                .find(|item| item.label == "readWrite")
                .unwrap();
            assert_eq!(property.kind, 10);
            let docs = property.documentation.as_deref().unwrap();
            assert!(docs.contains("get: int"), "{docs}");
            assert!(docs.contains("set(int $next): void"), "{docs}");
            assert!(!completions
                .iter()
                .any(|item| item.label.contains("::get") || item.label.contains("::set")));
        }
    }

    #[test]
    fn callback_hovers_and_completions_distinguish_owned_carriers_from_lent_values() {
        let source = r#"
interface Reader<T> { function(): T $callback { get; } }
interface Loan<T> { function(): T $callback { borrowed get; } }
class Source implements Reader<int> {
    int $value = 42;
    function(): int $callback { get => fn() with ($this) => $this->value; }
    ?function(): int $optional { get => fn() with ($this) => $this->value; }
}
class Stored implements Loan<int> {
    function __construct(take function(): int $stored) {}
    function(): int $callback { borrowed get => $this->stored; }
}
function concrete(Source $source): int { return $source->callback(); }
function grouped(Source $source): int { return ($source->callback)(); }
function erased(Reader<int> $reader): int { return $reader->callback(); }
function generic<T implements Reader<int>>(T $reader): int { return $reader->callback(); }
function lent(Loan<int> $loan): int { return $loan->callback(); }
function existing(Stored $stored): int { return $stored->callback(); }
function optional(Source $source): void { let $callback = $source->optional; }
"#;
        let snapshot = analyze(source);
        let retained =
            "Returns an owned value that retains a readonly borrow rooted in the receiver";
        let lent = "Returns a readonly borrow rooted in the receiver";
        for needle in ["get => fn()", "$optional {"] {
            let text = hover(&snapshot, source, needle);
            assert!(text.contains(retained), "{text}");
            assert!(!text.contains(lent), "{text}");
        }
        assert!(hover(&snapshot, source, "borrowed get =>").contains(lent));
        for (receiver, name, borrowed) in [
            ("$source->", "callback", false),
            ("$reader->", "callback", false),
            ("$source->", "optional", false),
            ("$loan->", "callback", true),
            ("$stored->", "callback", true),
        ] {
            for (start, _) in source.match_indices(&format!("{receiver}{name}")) {
                let offset = start + receiver.len();
                let text = snapshot.hover_at_offset(offset).unwrap().markdown;
                assert!(
                    snapshot
                        .property_accessors
                        .references
                        .keys()
                        .any(|span| { span.start == offset && span.end == offset + name.len() }),
                    "missing accessor reference at {offset} for {receiver}{name}: {text}"
                );
                assert!(
                    snapshot
                        .semantic_token_spans()
                        .iter()
                        .any(|(span, kind, _)| {
                            span.start == offset
                                && span.end == offset + name.len()
                                && *kind == SEMANTIC_TOKEN_PROPERTY
                        }),
                    "{text}"
                );
                let completions = snapshot.member_completions_at_offset(offset).unwrap();
                let docs = completions
                    .iter()
                    .find(|item| item.label == name)
                    .unwrap_or_else(|| {
                        panic!(
                            "missing completion at {offset} for {receiver}{name}: {completions:?}"
                        )
                    })
                    .documentation
                    .as_deref()
                    .unwrap();
                for presentation in [&text, docs] {
                    assert!(
                        presentation.contains(if borrowed { lent } else { retained }),
                        "{presentation}"
                    );
                    assert!(
                        !presentation.contains(if borrowed { retained } else { lent }),
                        "{presentation}"
                    );
                }
            }
        }
    }

    #[test]
    fn constrained_completions_preserve_lexical_scopes_and_specialized_types() {
        let source = r#"
interface Reader<T> {
    function read(): T;
    function(): T $callback { get; }
}
interface Fault extends Error { function code(): int; }
function number<T implements Reader<int>>(T $number): void { $number->read(); }
function text<T implements Reader<string>>(T $text): void { $text->read(); }
function failure<T implements Fault>(T $failure): void { $failure->code(); }
class Container<T implements Reader<bool>> {
    function outer(T $outer): void { $outer->read(); }
    function inner<U implements Reader<float>>(U $inner): void { $inner->read(); }
}
"#;
        let snapshot = analyze(source);
        for (receiver, ty) in [
            ("number", "int"),
            ("text", "string"),
            ("outer", "bool"),
            ("inner", "float"),
        ] {
            let needle = format!("${receiver}->");
            let offset = source.find(&needle).unwrap() + needle.len();
            let completions = snapshot.member_completions_at_offset(offset).unwrap();
            assert_eq!(completions.len(), 2, "{completions:?}");
            for (name, kind, signature) in [
                ("read", 2, format!("function read(): {ty}")),
                ("callback", 10, format!("get: function(): {ty}")),
            ] {
                let item = completions.iter().find(|item| item.label == name).unwrap();
                assert_eq!(item.kind, kind);
                assert!(item.detail.contains(&signature), "{item:?}");
            }
        }
        let offset = source.find("$failure->").unwrap() + "$failure->".len();
        let completions = snapshot.member_completions_at_offset(offset).unwrap();
        assert_eq!(completions.len(), 2, "{completions:?}");
        assert!(completions
            .iter()
            .any(|item| item.label == "code" && item.kind == 2));
        let message = completions
            .iter()
            .find(|item| item.label == "message")
            .unwrap();
        assert_eq!(message.kind, 10);
        assert_eq!(message.detail, "string $message");
    }

    #[test]
    fn constrained_completions_use_compiler_intersection_selection() {
        for constraints in ["Read, Cached, Conflict", "Conflict, Cached, Read"] {
            let source = format!(
                r#"
interface Read {{
    int $value {{ get; }}
    function read(): int;
    function clash(): int;
}}
interface Cached {{
    int $value {{ writable get; }}
    writable function read(): int;
}}
interface Conflict {{ function clash(): string; }}
function inspect<T implements {constraints}>(T $receiver): int {{ return $receiver->value; }}
"#
            );
            let snapshot = analyze(&source);
            let start = source.find("$receiver->").unwrap();
            let offset = start + "$receiver->".len();
            let completions = snapshot.member_completions_at_offset(offset).unwrap();
            assert_eq!(completions.len(), 2, "{completions:?}");
            let surface = &snapshot.contracts.constrained_member_surfaces
                [&Span::new(start, start + "$receiver".len())];
            for (name, signature) in [
                ("read", "function read(): int"),
                ("value", "$value { get: int }"),
            ] {
                let item = completions.iter().find(|item| item.label == name).unwrap();
                assert_eq!(item.detail, signature);
                let requirement = surface
                    .requirements
                    .iter()
                    .find(|item| item.name == name)
                    .unwrap();
                assert_eq!(requirement.origins.len(), 2);
                assert_eq!(
                    item.documentation.as_deref(),
                    Some(interface_requirement_documentation(requirement).as_str())
                );
            }
            assert!(!completions.iter().any(|item| item.label == "clash"));
        }
    }

    #[test]
    fn constrained_completions_preserve_access_and_forwarding_wrapper_members() {
        let source = r#"
interface Reader {
    function read(): int;
    writable function change(): void;
    int $cache { writable get; }
    writable int $sink { set (int $value); }
}
function inspect<T implements Reader>(T $readonly, writable T $writable,
    SharedReference<T> $shared, ReadonlySharedReferenceAccess<T> $readAccess,
    WritableSharedReferenceAccess<T> $writeAccess): void {
    $readonly->read();
    $writable->read();
    $shared->read();
    $readAccess->read();
    $writeAccess->read();
}
"#;
        let snapshot = analyze(source);
        for (receiver, writable) in [
            ("readonly", false),
            ("writable", true),
            ("shared", false),
            ("readAccess", false),
            ("writeAccess", true),
        ] {
            let needle = format!("${receiver}->");
            let offset = source.find(&needle).unwrap() + needle.len();
            let completions = snapshot.member_completions_at_offset(offset).unwrap();
            assert!(
                completions.iter().any(|item| item.label == "read"),
                "{completions:?}"
            );
            for name in ["change", "cache", "sink"] {
                assert_eq!(
                    completions.iter().any(|item| item.label == name),
                    writable,
                    "{receiver}: {completions:?}"
                );
            }
            if receiver == "shared" {
                for name in ["share", "createWeakReference", "referencedValue"] {
                    assert!(
                        completions.iter().any(|item| item.label == name),
                        "{completions:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn setter_hovers_preserve_specialized_property_types_and_parameter_ownership() {
        let source = r#"
interface Sink<T> { writable T $items { set (take T $value); } }
trait Items {
    writable List<int> $items = [1] {
        get => $this->items;
        set (take List<int> $value) => $this->items = $value;
    }
}
class Collection { uses Items; }
function store(writable Sink<List<int>> $sink, take List<int> $values): void {
    $sink->items = $values;
}
function replace(writable Collection $collection, take List<int> $values): void {
    $collection->items = $values;
}
"#;
        let snapshot = analyze(source);
        assert!(hover(&snapshot, source, "set (take T").contains("set(take T $value): void"));
        assert!(hover(&snapshot, source, "set (take List<int>")
            .contains("set(take List<int> $value): void"));
        for receiver in ["$sink->", "$collection->"] {
            let offset = source.find(receiver).unwrap() + receiver.len();
            let text = snapshot.hover_at_offset(offset).unwrap().markdown;
            assert!(text.contains("set(take List<int> $value): void"), "{text}");
            let completions = snapshot.member_completions_at_offset(offset).unwrap();
            let item = completions
                .iter()
                .find(|item| item.label == "items")
                .unwrap();
            let presentation = format!(
                "{}\n{}",
                item.detail,
                item.documentation.as_deref().unwrap_or_default()
            );
            assert!(
                presentation.contains("set(take List<int> $value): void"),
                "{presentation}"
            );
        }
    }

    #[test]
    fn backing_in_lexical_closures_and_trait_override_navigation_use_compiler_identity() {
        let source = r#"
trait Value { int $value = 1 { get => $this->value; } }
class Composed { uses Value; }
open class Base { open int $value { get => 2; } }
class Child extends Base { override int $value { get => 3; } }
class Lexical {
    int $value = 4 { get { let $read = fn() with ($this) => $this->value; return $read(); } }
}
open class StoredBase { open int $stored = 5 { get => $this->stored; } }
class StoredChild extends StoredBase {
    override int $stored = 6 { get => $this->stored + 1; }
}
class ComputedChild extends StoredBase {
    override int $stored { get => 7; }
}
function composed(Composed $object): int { return $object->value; }
function inherited(Child $child): int { return $child->value; }
function inheritedStorage(StoredChild $child): int { return $child->stored; }
function computedStorage(ComputedChild $computed): int { return $computed->stored; }
"#;
        let snapshot = analyze(source);
        assert!(hover(&snapshot, source, "$value = 4").contains("Automatically backed"));
        assert!(hover(&snapshot, source, "$stored = 6").contains("Automatically backed"));
        assert!(hover(&snapshot, source, "$stored = 6")
            .contains("Backing field: `StoredBase::$stored`"));
        assert!(hover(&snapshot, source, "$stored = 6")
            .contains("A child initializer replaces the inherited value"));
        let computed = hover(&snapshot, source, "get => 7");
        assert!(
            computed.contains("Computed property; inherited backing storage is retained"),
            "{computed}"
        );
        assert!(
            computed.contains("Backing field: `StoredBase::$stored`"),
            "{computed}"
        );
        assert!(!computed.contains("no backing storage slot"), "{computed}");
        for (usage, field) in [
            ("$child->stored", "StoredBase::$stored"),
            ("$computed->stored", "StoredBase::$stored"),
            ("$object->value", "Composed::$value"),
        ] {
            let offset = source.find(usage).unwrap() + usage.find("->").unwrap() + 2;
            let name = usage.split("->").nth(1).unwrap();
            let completions = snapshot.member_completions_at_offset(offset).unwrap();
            let docs = completions
                .iter()
                .find(|item| item.label == name)
                .unwrap()
                .documentation
                .as_deref()
                .unwrap();
            assert!(
                docs.contains(&format!("Backing field: `{field}`")),
                "{docs}"
            );
            assert!(!docs.contains("no backing storage slot"), "{docs}");
        }
        let index = OpenDocumentIndex::rebuild(std::iter::once((
            "graph".to_string(),
            "hooks.doria",
            &snapshot,
        )));
        for (usage, getter) in [
            ("$object->value", "get => $this"),
            ("$child->value", "get => 3"),
            ("$child->stored", "get => $this->stored + 1"),
            ("$computed->stored", "get => 7"),
        ] {
            let offset = source.find(usage).unwrap() + usage.find("->").unwrap() + 2;
            let targets = index
                .property_accessor_definitions("hooks.doria", offset)
                .unwrap();
            assert_eq!(targets.len(), 1, "{targets:?}");
            assert_eq!(targets[0].span.start, source.find(getter).unwrap());
        }
    }
}
