use std::collections::{HashMap, HashSet};

use doriac::ast::MemberAccess;
use doriac::attributes::{AttributeClassIdentity, AttributeClassSchema, AttributeSchemaParameter};
use doriac::names::{GlobalReferenceRole, GlobalSymbolId, GlobalSymbolKind, PackageIdentity};
use doriac::source::Span;

use crate::analysis::{
    AnalysisSnapshot, AttributeParameterIdentity, AttributeParameterSpelling, HierarchyClass,
    HierarchyContext, HierarchyMember, MemberIdentity, MemberKind, MemberOccurrence,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct AliasIdentity {
    pub(crate) uri: String,
    pub(crate) target: GlobalSymbolId,
    pub(crate) alias: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SymbolTarget {
    Canonical(GlobalSymbolId),
    Alias(AliasIdentity),
    AttributeParameter(AttributeParameterIdentity),
    Member(MemberTarget),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemberTarget {
    graph: String,
    identity: MemberIdentity,
    exact_declaration: Option<Span>,
    virtual_root: Option<Span>,
}

#[derive(Debug, Clone)]
pub(crate) struct IndexedLocation {
    pub(crate) uri: String,
    pub(crate) span: Span,
}

#[derive(Debug, Clone)]
pub(crate) struct IndexedEdit {
    pub(crate) uri: String,
    pub(crate) span: Span,
    pub(crate) replacement: String,
}

#[derive(Debug, Clone)]
pub(crate) struct IndexedHover {
    pub(crate) span: Span,
    pub(crate) markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexedCompletion {
    pub(crate) label: String,
    pub(crate) kind: u32,
    pub(crate) detail: String,
    pub(crate) documentation: Option<String>,
    pub(crate) insert_text: Option<String>,
    pub(crate) snippet: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexedImportCandidate {
    pub(crate) target: String,
    pub(crate) class_like: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexedTestSymbol {
    pub(crate) name: String,
    pub(crate) uri: String,
    pub(crate) span: Span,
    pub(crate) suite: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndexedRole {
    Declaration,
    Reference,
    ImportTarget,
    AliasDeclaration,
    AliasUse,
}

#[derive(Debug, Clone)]
struct IndexedOccurrence {
    uri: String,
    span: Span,
    symbol: GlobalSymbolId,
    role: IndexedRole,
    source_spelling: String,
    alias: Option<AliasIdentity>,
    global_role: Option<GlobalReferenceRole>,
}

#[derive(Debug, Clone)]
struct IndexedAttributeParameterOccurrence {
    uri: String,
    span: Span,
    identity: AttributeParameterIdentity,
    name: String,
    declaration: bool,
    spelling: AttributeParameterSpelling,
}

#[derive(Debug, Clone)]
struct IndexedMemberOccurrence {
    graph: String,
    uri: String,
    occurrence: MemberOccurrence,
}

#[derive(Debug, Clone)]
struct IndexedHierarchyClass {
    uri: String,
    class: HierarchyClass,
}

#[derive(Debug, Clone)]
struct IndexedHierarchyMember {
    graph: String,
    member: HierarchyMember,
}

#[derive(Debug, Clone)]
struct DocumentSummary {
    graph: String,
    package: PackageIdentity,
    namespace: Option<String>,
    declarations: Vec<(GlobalSymbolId, String, GlobalSymbolKind)>,
    imports: Vec<(String, String, Option<GlobalSymbolId>)>,
    compiler_known: Vec<(String, GlobalSymbolId, GlobalSymbolKind)>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct OpenDocumentIndex {
    occurrences: Vec<IndexedOccurrence>,
    declaration_counts: HashMap<GlobalSymbolId, usize>,
    symbol_kinds: HashMap<GlobalSymbolId, GlobalSymbolKind>,
    implicit_imports: HashSet<GlobalSymbolId>,
    incomplete_packages: HashSet<PackageIdentity>,
    declaration_hovers: HashMap<GlobalSymbolId, String>,
    symbol_access: HashMap<GlobalSymbolId, MemberAccess>,
    attribute_schemas: HashMap<AttributeClassIdentity, AttributeClassSchema>,
    attribute_parameter_occurrences: Vec<IndexedAttributeParameterOccurrence>,
    member_occurrences: Vec<IndexedMemberOccurrence>,
    member_parents: HashMap<String, HashMap<GlobalSymbolId, GlobalSymbolId>>,
    hierarchy_classes: HashMap<String, HashMap<GlobalSymbolId, IndexedHierarchyClass>>,
    hierarchy_members: Vec<IndexedHierarchyMember>,
    test_symbols: Vec<IndexedTestSymbol>,
    documents: HashMap<String, DocumentSummary>,
}

impl OpenDocumentIndex {
    pub(crate) fn rebuild<'a>(
        documents: impl Iterator<Item = (String, &'a str, &'a AnalysisSnapshot)>,
    ) -> Self {
        let mut index = Self::default();
        for (graph, uri, snapshot) in documents {
            index.add_document(&graph, uri, snapshot);
        }
        index.occurrences.sort_by(|left, right| {
            (&left.uri, left.span.start, left.span.end).cmp(&(
                &right.uri,
                right.span.start,
                right.span.end,
            ))
        });
        index
            .attribute_parameter_occurrences
            .sort_by(|left, right| {
                (&left.uri, left.span.start, left.span.end).cmp(&(
                    &right.uri,
                    right.span.start,
                    right.span.end,
                ))
            });
        index.member_occurrences.sort_by(|left, right| {
            (
                &left.uri,
                left.occurrence.span.start,
                left.occurrence.span.end,
            )
                .cmp(&(
                    &right.uri,
                    right.occurrence.span.start,
                    right.occurrence.span.end,
                ))
        });
        index.test_symbols.sort_by(|left, right| {
            (&left.name, &left.uri, left.span.start).cmp(&(
                &right.name,
                &right.uri,
                right.span.start,
            ))
        });
        index
    }

    fn add_document(&mut self, graph: &str, uri: &str, snapshot: &AnalysisSnapshot) {
        let facts = snapshot.global_symbols();
        let package = snapshot.compilation_context().package.clone();
        let mut summary = DocumentSummary {
            graph: graph.to_string(),
            package: package.clone(),
            namespace: facts.namespace.clone(),
            declarations: Vec::new(),
            imports: Vec::new(),
            compiler_known: Vec::new(),
        };

        for suite in snapshot
            .test_semantics()
            .suites
            .iter()
            .filter(|suite| suite.call_name_span.source == snapshot.source_id())
        {
            self.test_symbols.push(IndexedTestSymbol {
                name: format!("{} :: {}", suite.package.display_name(), suite.display_name),
                uri: uri.to_string(),
                span: suite.call_name_span,
                suite: true,
            });
        }
        for test in snapshot
            .test_semantics()
            .tests
            .iter()
            .filter(|test| test.call_name_span.source == snapshot.source_id())
        {
            self.test_symbols.push(IndexedTestSymbol {
                name: format!("{} :: {}", test.package.display_name(), test.display_name),
                uri: uri.to_string(),
                span: test.call_name_span,
                suite: false,
            });
        }

        for declaration in &facts.declarations {
            *self
                .declaration_counts
                .entry(declaration.id.clone())
                .or_default() += 1;
            self.symbol_kinds
                .entry(declaration.id.clone())
                .or_insert(declaration.kind);
            self.symbol_access
                .entry(declaration.id.clone())
                .or_insert(declaration.access);
            summary.declarations.push((
                declaration.id.clone(),
                declaration.source_name.clone(),
                declaration.kind,
            ));
            if let Some(hover) = snapshot.hover_at_offset(declaration.name_span.start) {
                self.declaration_hovers
                    .entry(declaration.id.clone())
                    .or_insert(hover.markdown);
            }
            self.occurrences.push(IndexedOccurrence {
                uri: uri.to_string(),
                span: declaration.name_span,
                symbol: declaration.id.clone(),
                role: IndexedRole::Declaration,
                source_spelling: declaration.source_name.clone(),
                alias: None,
                global_role: None,
            });
        }

        for known in &facts.compiler_known {
            self.symbol_kinds
                .entry(known.id.clone())
                .or_insert(known.kind);
            summary
                .compiler_known
                .push((known.source_name.clone(), known.id.clone(), known.kind));
        }

        let imports = facts
            .imports
            .iter()
            .map(|import| {
                let target = facts
                    .references
                    .iter()
                    .find(|reference| {
                        reference.role == GlobalReferenceRole::ImportTarget
                            && reference.source_span == import.target_span
                    })
                    .map(|reference| reference.symbol_id.clone());
                (import, target)
            })
            .collect::<Vec<_>>();

        for (import, target) in &imports {
            summary
                .imports
                .push((import.alias.clone(), import.target.clone(), target.clone()));
            let Some(target) = target else {
                continue;
            };
            let explicit = !span_contains(import.target_span, import.alias_span);
            if explicit {
                self.occurrences.push(IndexedOccurrence {
                    uri: uri.to_string(),
                    span: import.alias_span,
                    symbol: target.clone(),
                    role: IndexedRole::AliasDeclaration,
                    source_spelling: import.alias.clone(),
                    alias: Some(AliasIdentity {
                        uri: uri.to_string(),
                        target: target.clone(),
                        alias: import.alias.clone(),
                    }),
                    global_role: None,
                });
            } else {
                self.implicit_imports.insert(target.clone());
            }
        }

        for reference in &facts.references {
            let alias = if reference.role == GlobalReferenceRole::ImportTarget {
                None
            } else {
                reference.import_alias.as_ref().map(|alias| AliasIdentity {
                    uri: uri.to_string(),
                    target: reference.symbol_id.clone(),
                    alias: alias.clone(),
                })
            };
            self.occurrences.push(IndexedOccurrence {
                uri: uri.to_string(),
                span: reference.source_span,
                symbol: reference.symbol_id.clone(),
                role: if reference.role == GlobalReferenceRole::ImportTarget {
                    IndexedRole::ImportTarget
                } else if alias.is_some() {
                    IndexedRole::AliasUse
                } else {
                    IndexedRole::Reference
                },
                source_spelling: reference.source_spelling.clone(),
                alias,
                global_role: Some(reference.role),
            });
        }

        for schema in &snapshot.attribute_info().schemas {
            self.attribute_schemas
                .entry(schema.identity.clone())
                .or_insert_with(|| schema.clone());
        }
        self.attribute_parameter_occurrences.extend(
            snapshot
                .attribute_parameter_occurrences()
                .iter()
                .map(|occurrence| IndexedAttributeParameterOccurrence {
                    uri: uri.to_string(),
                    span: occurrence.span,
                    identity: occurrence.identity.clone(),
                    name: occurrence.name.clone(),
                    declaration: occurrence.declaration,
                    spelling: occurrence.spelling,
                }),
        );
        self.member_occurrences
            .extend(
                snapshot
                    .member_occurrences()
                    .iter()
                    .cloned()
                    .map(|occurrence| IndexedMemberOccurrence {
                        graph: graph.to_string(),
                        uri: uri.to_string(),
                        occurrence,
                    }),
            );
        for parent in snapshot.member_parents() {
            self.member_parents
                .entry(graph.to_string())
                .or_default()
                .entry(parent.child.clone())
                .or_insert_with(|| parent.parent.clone());
        }
        for class in snapshot.hierarchy_classes() {
            self.hierarchy_classes
                .entry(graph.to_string())
                .or_default()
                .entry(class.identity.clone())
                .or_insert_with(|| IndexedHierarchyClass {
                    uri: uri.to_string(),
                    class: class.clone(),
                });
        }
        self.hierarchy_members
            .extend(snapshot.hierarchy_members().iter().cloned().map(|member| {
                IndexedHierarchyMember {
                    graph: graph.to_string(),
                    member,
                }
            }));

        if !facts.unresolved.is_empty() {
            self.incomplete_packages.insert(package);
        }

        self.documents.insert(uri.to_string(), summary);
    }

    pub(crate) fn target_at(&self, uri: &str, offset: usize) -> Option<SymbolTarget> {
        if let Some(occurrence) = self
            .attribute_parameter_occurrences
            .iter()
            .filter(|occurrence| {
                occurrence.uri == uri && span_contains_offset(occurrence.span, offset)
            })
            .min_by_key(|occurrence| occurrence.span.end.saturating_sub(occurrence.span.start))
        {
            return Some(SymbolTarget::AttributeParameter(
                occurrence.identity.clone(),
            ));
        }
        if let Some(occurrence) = self.member_occurrence_at(uri, offset) {
            return Some(SymbolTarget::Member(MemberTarget {
                graph: occurrence.graph.clone(),
                identity: self
                    .resolved_member_identity(&occurrence.graph, &occurrence.occurrence.identity),
                exact_declaration: occurrence.occurrence.exact_declaration,
                virtual_root: occurrence.occurrence.virtual_root,
            }));
        }
        let occurrence = self
            .occurrences
            .iter()
            .filter(|occurrence| {
                occurrence.uri == uri && span_contains_offset(occurrence.span, offset)
            })
            .min_by_key(|occurrence| {
                (
                    occurrence.span.end.saturating_sub(occurrence.span.start),
                    !matches!(
                        occurrence.role,
                        IndexedRole::AliasDeclaration | IndexedRole::AliasUse
                    ),
                )
            })?;
        if occurrence.global_role == Some(GlobalReferenceRole::AttributeClass) {
            return Some(SymbolTarget::Canonical(occurrence.symbol.clone()));
        }
        occurrence
            .alias
            .clone()
            .map(SymbolTarget::Alias)
            .or_else(|| Some(SymbolTarget::Canonical(occurrence.symbol.clone())))
    }

    fn member_occurrence_at(&self, uri: &str, offset: usize) -> Option<&IndexedMemberOccurrence> {
        self.member_occurrences
            .iter()
            .filter(|candidate| {
                candidate.uri == uri && span_contains_offset(candidate.occurrence.span, offset)
            })
            .min_by_key(|candidate| {
                candidate
                    .occurrence
                    .span
                    .end
                    .saturating_sub(candidate.occurrence.span.start)
            })
    }

    fn resolved_member_identity(&self, graph: &str, identity: &MemberIdentity) -> MemberIdentity {
        let mut resolved = identity.clone();
        let mut owner = identity.owner.clone();
        let mut visited = HashSet::new();
        while visited.insert(owner.clone()) {
            let candidate = MemberIdentity {
                owner: owner.clone(),
                name: identity.name.clone(),
                kind: identity.kind,
            };
            if self.member_occurrences.iter().any(|occurrence| {
                occurrence.graph == graph
                    && occurrence.occurrence.declaration
                    && occurrence.occurrence.identity == candidate
            }) {
                return candidate;
            }
            let Some(parent) = self
                .member_parents
                .get(graph)
                .and_then(|parents| parents.get(&owner))
            else {
                break;
            };
            owner = parent.clone();
            resolved.owner = owner.clone();
        }
        resolved
    }

    fn member_occurrence_matches_target(
        &self,
        occurrence: &IndexedMemberOccurrence,
        target: &MemberTarget,
    ) -> bool {
        if occurrence.graph != target.graph {
            return false;
        }
        if let Some(root) = target.virtual_root {
            return occurrence.occurrence.virtual_root == Some(root);
        }
        if let Some(declaration) = target.exact_declaration {
            return occurrence.occurrence.exact_declaration == Some(declaration);
        }
        self.resolved_member_identity(&occurrence.graph, &occurrence.occurrence.identity)
            == target.identity
    }

    pub(crate) fn references(
        &self,
        target: &SymbolTarget,
        include_declaration: bool,
    ) -> Vec<IndexedLocation> {
        if let SymbolTarget::AttributeParameter(identity) = target {
            return self
                .attribute_parameter_occurrences
                .iter()
                .filter(|occurrence| {
                    occurrence.identity == *identity
                        && (include_declaration || !occurrence.declaration)
                })
                .map(|occurrence| IndexedLocation {
                    uri: occurrence.uri.clone(),
                    span: occurrence.span,
                })
                .collect();
        }
        if let SymbolTarget::Member(target) = target {
            return self
                .member_occurrences
                .iter()
                .filter(|candidate| {
                    self.member_occurrence_matches_target(candidate, target)
                        && (include_declaration || !candidate.occurrence.declaration)
                })
                .map(|candidate| IndexedLocation {
                    uri: candidate.uri.clone(),
                    span: candidate.occurrence.span,
                })
                .collect();
        }
        self.occurrences
            .iter()
            .filter(|occurrence| match target {
                SymbolTarget::Canonical(symbol) => {
                    occurrence.symbol == *symbol
                        && occurrence.role != IndexedRole::AliasDeclaration
                        && (include_declaration || occurrence.role != IndexedRole::Declaration)
                }
                SymbolTarget::Alias(alias) => {
                    occurrence.alias.as_ref() == Some(alias)
                        && (include_declaration || occurrence.role != IndexedRole::AliasDeclaration)
                }
                SymbolTarget::AttributeParameter(_) => false,
                SymbolTarget::Member(_) => false,
            })
            .map(|occurrence| IndexedLocation {
                uri: occurrence.uri.clone(),
                span: occurrence.span,
            })
            .collect()
    }

    pub(crate) fn definition(&self, uri: &str, offset: usize) -> Option<IndexedLocation> {
        if let Some(SymbolTarget::Member(target)) = self.target_at(uri, offset) {
            if let Some(declaration) = target.exact_declaration {
                return self
                    .member_occurrences
                    .iter()
                    .find(|candidate| {
                        candidate.occurrence.declaration
                            && candidate.occurrence.exact_declaration == Some(declaration)
                    })
                    .map(|candidate| IndexedLocation {
                        uri: candidate.uri.clone(),
                        span: candidate.occurrence.span,
                    });
            }
            return self
                .member_occurrences
                .iter()
                .find(|candidate| {
                    candidate.occurrence.declaration
                        && candidate.occurrence.identity == target.identity
                })
                .map(|candidate| IndexedLocation {
                    uri: candidate.uri.clone(),
                    span: candidate.occurrence.span,
                });
        }
        if let Some(SymbolTarget::AttributeParameter(identity)) = self.target_at(uri, offset) {
            return self
                .attribute_parameter_occurrences
                .iter()
                .find(|candidate| candidate.identity == identity && candidate.declaration)
                .map(|candidate| IndexedLocation {
                    uri: candidate.uri.clone(),
                    span: candidate.span,
                });
        }
        let occurrence = self
            .occurrences
            .iter()
            .filter(|occurrence| {
                occurrence.uri == uri && span_contains_offset(occurrence.span, offset)
            })
            .min_by_key(|occurrence| occurrence.span.end.saturating_sub(occurrence.span.start))?;
        if occurrence.role == IndexedRole::AliasUse {
            let alias = occurrence.alias.as_ref()?;
            if occurrence.global_role == Some(GlobalReferenceRole::AttributeClass) {
                return self
                    .occurrences
                    .iter()
                    .find(|candidate| {
                        candidate.symbol == alias.target
                            && candidate.role == IndexedRole::Declaration
                    })
                    .map(indexed_location);
            }
            return self
                .occurrences
                .iter()
                .find(|candidate| {
                    candidate.role == IndexedRole::AliasDeclaration
                        && candidate.alias.as_ref() == Some(alias)
                })
                .or_else(|| {
                    self.occurrences.iter().find(|candidate| {
                        candidate.symbol == alias.target
                            && candidate.role == IndexedRole::Declaration
                    })
                })
                .map(indexed_location);
        }
        self.occurrences
            .iter()
            .find(|candidate| {
                candidate.symbol == occurrence.symbol && candidate.role == IndexedRole::Declaration
            })
            .map(indexed_location)
    }

    pub(crate) fn rename(&self, target: &SymbolTarget, new_name: &str) -> Option<Vec<IndexedEdit>> {
        if !is_identifier(new_name) {
            return None;
        }
        match target {
            SymbolTarget::Alias(alias) => {
                if !self.occurrences.iter().any(|occurrence| {
                    occurrence.alias.as_ref() == Some(alias)
                        && occurrence.role == IndexedRole::AliasDeclaration
                }) {
                    return None;
                }
                let edits = self
                    .occurrences
                    .iter()
                    .filter(|occurrence| occurrence.alias.as_ref() == Some(alias))
                    .map(|occurrence| IndexedEdit {
                        uri: occurrence.uri.clone(),
                        span: occurrence.span,
                        replacement: new_name.to_string(),
                    })
                    .collect::<Vec<_>>();
                (!edits.is_empty()).then_some(edits)
            }
            SymbolTarget::Canonical(symbol) => {
                let package = symbol_package(symbol)?;
                if self.declaration_counts.get(symbol) != Some(&1)
                    || self.implicit_imports.contains(symbol)
                    || self.incomplete_packages.contains(package)
                {
                    return None;
                }
                let mut edits = self
                    .occurrences
                    .iter()
                    .filter(|occurrence| occurrence.symbol == *symbol)
                    .filter_map(|occurrence| {
                        if matches!(
                            occurrence.role,
                            IndexedRole::AliasDeclaration | IndexedRole::AliasUse
                        ) {
                            return None;
                        }
                        let replacement = if occurrence.role == IndexedRole::Declaration {
                            new_name.to_string()
                        } else {
                            replace_final_segment(&occurrence.source_spelling, new_name)
                        };
                        Some(IndexedEdit {
                            uri: occurrence.uri.clone(),
                            span: occurrence.span,
                            replacement,
                        })
                    })
                    .collect::<Vec<_>>();
                edits.sort_by(|left, right| {
                    (&left.uri, left.span.start, left.span.end).cmp(&(
                        &right.uri,
                        right.span.start,
                        right.span.end,
                    ))
                });
                edits.dedup_by(|left, right| left.uri == right.uri && left.span == right.span);
                (!edits.is_empty()).then_some(edits)
            }
            SymbolTarget::AttributeParameter(identity) => {
                let package = symbol_package(&identity.class)?;
                if self.declaration_counts.get(&identity.class) != Some(&1)
                    || self.incomplete_packages.contains(package)
                {
                    return None;
                }
                let mut edits = self
                    .attribute_parameter_occurrences
                    .iter()
                    .filter(|occurrence| occurrence.identity == *identity)
                    .map(|occurrence| IndexedEdit {
                        uri: occurrence.uri.clone(),
                        span: occurrence.span,
                        replacement: match occurrence.spelling {
                            AttributeParameterSpelling::Variable => format!("${new_name}"),
                            AttributeParameterSpelling::Label => new_name.to_string(),
                        },
                    })
                    .collect::<Vec<_>>();
                edits.sort_by(|left, right| {
                    (&left.uri, left.span.start, left.span.end).cmp(&(
                        &right.uri,
                        right.span.start,
                        right.span.end,
                    ))
                });
                edits.dedup_by(|left, right| left.uri == right.uri && left.span == right.span);
                (!edits.is_empty()).then_some(edits)
            }
            SymbolTarget::Member(target) => {
                let declarations = self
                    .member_occurrences
                    .iter()
                    .filter(|candidate| candidate.occurrence.declaration)
                    .filter(|candidate| self.member_occurrence_matches_target(candidate, target))
                    .collect::<Vec<_>>();
                if declarations.is_empty()
                    || declarations.iter().any(|declaration| {
                        symbol_package(&declaration.occurrence.identity.owner)
                            .is_none_or(|package| self.incomplete_packages.contains(package))
                    })
                    || declarations.iter().any(|declaration| {
                        self.member_occurrences.iter().any(|candidate| {
                            candidate.occurrence.declaration
                                && candidate.occurrence.identity.owner
                                    == declaration.occurrence.identity.owner
                                && candidate.occurrence.identity.kind
                                    == declaration.occurrence.identity.kind
                                && candidate.occurrence.identity.name == new_name
                                && !self.member_occurrence_matches_target(candidate, target)
                        })
                    })
                {
                    return None;
                }
                let mut edits = self
                    .member_occurrences
                    .iter()
                    .filter(|candidate| self.member_occurrence_matches_target(candidate, target))
                    .map(|candidate| IndexedEdit {
                        uri: candidate.uri.clone(),
                        span: candidate.occurrence.span,
                        replacement: new_name.to_string(),
                    })
                    .collect::<Vec<_>>();
                edits.sort_by(|left, right| {
                    (&left.uri, left.span.start, left.span.end).cmp(&(
                        &right.uri,
                        right.span.start,
                        right.span.end,
                    ))
                });
                edits.dedup_by(|left, right| left.uri == right.uri && left.span == right.span);
                (!edits.is_empty()).then_some(edits)
            }
        }
    }

    pub(crate) fn hover(&self, uri: &str, offset: usize) -> Option<IndexedHover> {
        let target = self.target_at(uri, offset)?;
        let (symbol, alias) = match target {
            SymbolTarget::Canonical(symbol) => (symbol, None),
            SymbolTarget::Alias(alias) => (alias.target, Some(alias.alias)),
            SymbolTarget::AttributeParameter(identity) => {
                let occurrence =
                    self.attribute_parameter_occurrences
                        .iter()
                        .find(|occurrence| {
                            occurrence.uri == uri && span_contains_offset(occurrence.span, offset)
                        })?;
                let schema = self
                    .attribute_schemas
                    .get(&AttributeClassIdentity::User(identity.class.clone()))?;
                let parameter = schema.parameters.get(identity.index)?;
                return Some(IndexedHover {
                    span: occurrence.span,
                    markdown: format!(
                        "Attribute parameter `${}`: `{}`\n\nNamed attribute arguments bind to this constructor parameter by compiler identity.",
                        occurrence.name,
                        doriac::attributes::metadata_type_name(&parameter.ty)
                    ),
                });
            }
            SymbolTarget::Member(target) => {
                let occurrence = self.member_occurrence_at(uri, offset)?;
                let member = target
                    .exact_declaration
                    .and_then(|declaration| {
                        self.hierarchy_members.iter().find(|member| {
                            member.graph == target.graph && member.member.declaration == declaration
                        })
                    })
                    .or_else(|| {
                        self.hierarchy_members.iter().find(|member| {
                            member.graph == target.graph
                                && member.member.owner == target.identity.owner
                                && member.member.name == target.identity.name
                                && member.member.kind == target.identity.kind
                        })
                    })?;
                let mut markdown = format!("```doria\n{}\n```", member.member.detail);
                if let Some(documentation) = &member.member.documentation {
                    markdown.push_str("\n\n");
                    markdown.push_str(documentation);
                }
                if member.member.kind == MemberKind::Method {
                    let role = if member.member.is_override {
                        "Override"
                    } else if member.member.is_open {
                        "Open Virtual Root"
                    } else {
                        "Nonvirtual"
                    };
                    markdown.push_str(&format!(
                        "\n\n**Hierarchy Method Role:** {role}\n\n**Effective Receiver:** {}\n\n**Dispatch:** {}",
                        if member.member.writable_receiver {
                            "Writable"
                        } else {
                            "Readonly"
                        },
                        if occurrence.occurrence.direct_parent {
                            "Direct parent implementation (virtual dispatch bypassed)"
                        } else if member.member.virtual_root.is_some() {
                            "Virtual"
                        } else {
                            "Direct"
                        },
                    ));
                    if let Some(root) = member.member.virtual_root {
                        if let Some(label) = self.method_declaration_label(&target.graph, root) {
                            markdown
                                .push_str(&format!("\n\n**Root Virtual Declaration:** `{label}`"));
                        }
                    }
                    if let Some(parent) = member.member.overridden_declaration {
                        if let Some(label) = self.method_declaration_label(&target.graph, parent) {
                            markdown.push_str(&format!(
                                "\n\n**Nearest Overridden Declaration:** `{label}`"
                            ));
                        }
                        if let Some(parent_member) =
                            self.hierarchy_members.iter().find(|candidate| {
                                candidate.graph == target.graph
                                    && candidate.member.declaration == parent
                            })
                        {
                            if parent_member.member.detail.contains("= ...") {
                                markdown.push_str(&format!(
                                    "\n\n**Inherited Defaults:** `{}`",
                                    parent_member.member.detail
                                ));
                            }
                            if parent_member.member.detail != member.member.detail {
                                markdown.push_str(&format!(
                                    "\n\n**Override Contract:** `{}`\n\n**Effective Signature:** `{}`",
                                    parent_member.member.detail, member.member.detail
                                ));
                            }
                        }
                    }
                }
                return Some(IndexedHover {
                    span: occurrence.occurrence.span,
                    markdown,
                });
            }
        };
        let kind = self.symbol_kinds.get(&symbol)?;
        let occurrence = self.occurrences.iter().find(|occurrence| {
            occurrence.uri == uri && span_contains_offset(occurrence.span, offset)
        })?;
        let mut markdown = format!("{} `{}`", kind_name(*kind), symbol.qualified_name);
        if let Some(declaration) = self.declaration_hovers.get(&symbol) {
            if !declaration.is_empty() && !markdown.contains(declaration) {
                markdown.push_str("\n\n");
                markdown.push_str(declaration);
            }
        }
        if let Some(alias) = alias {
            markdown.push_str(&format!("\n\nImported As `{alias}`"));
        }
        Some(IndexedHover {
            span: occurrence.span,
            markdown,
        })
    }

    fn method_declaration_label(&self, graph: &str, declaration: Span) -> Option<String> {
        self.hierarchy_members
            .iter()
            .find(|member| {
                member.graph == graph
                    && member.member.kind == MemberKind::Method
                    && member.member.declaration == declaration
            })
            .and_then(|member| {
                self.hierarchy_classes
                    .get(graph)
                    .and_then(|classes| classes.get(&member.member.owner))
                    .map(|class| format!("{}::{}", class.class.qualified_name, member.member.name))
            })
    }

    pub(crate) fn completions(&self, uri: &str) -> Vec<IndexedCompletion> {
        let Some(document) = self.documents.get(uri) else {
            return Vec::new();
        };
        let mut completions = HashMap::<String, IndexedCompletion>::new();
        for (symbol, source_name, kind) in &document.declarations {
            completions.insert(
                source_name.clone(),
                completion(source_name.clone(), *kind, &symbol.qualified_name),
            );
        }
        for (alias, source_target, target) in &document.imports {
            let kind = target
                .as_ref()
                .and_then(|target| self.symbol_kinds.get(target))
                .copied()
                .unwrap_or(GlobalSymbolKind::Class);
            let detail = target.as_ref().map_or_else(
                || format!("Unresolved import `{source_target}`"),
                |target| format!("Imported {} `{}`", kind_name(kind), target.qualified_name),
            );
            completions.insert(
                alias.clone(),
                IndexedCompletion {
                    label: alias.clone(),
                    kind: completion_kind(kind),
                    detail,
                    documentation: None,
                    insert_text: None,
                    snippet: false,
                },
            );
        }
        for (name, symbol, kind) in &document.compiler_known {
            if *kind == GlobalSymbolKind::CompilerKnownAttribute
                || doriac::compiler_known_test::is_future_member(&symbol.qualified_name)
            {
                continue;
            }
            completions
                .entry(name.clone())
                .or_insert_with(|| completion(name.clone(), *kind, &symbol.qualified_name));
        }
        for summary in self.documents.values() {
            if summary.package != document.package {
                continue;
            }
            for (symbol, _, kind) in &summary.declarations {
                let label = if summary.namespace == document.namespace {
                    symbol
                        .qualified_name
                        .rsplit('\\')
                        .next()
                        .unwrap_or(&symbol.qualified_name)
                        .to_string()
                } else {
                    symbol.qualified_name.clone()
                };
                completions
                    .entry(label.clone())
                    .or_insert_with(|| completion(label, *kind, &symbol.qualified_name));
            }
        }
        let mut completions = completions.into_values().collect::<Vec<_>>();
        completions.sort_by(|left, right| left.label.cmp(&right.label));
        completions
    }

    pub(crate) fn open_class_completions(
        &self,
        uri: &str,
        context: Option<&HierarchyContext>,
        excluded_source_name: Option<&str>,
        recovered_namespace: Option<&str>,
        recovered_imports: &[(String, String)],
    ) -> Vec<IndexedCompletion> {
        let Some(document) = self.documents.get(uri) else {
            return Vec::new();
        };
        let current = context.and_then(|context| {
            self.hierarchy_classes
                .get(&document.graph)
                .and_then(|classes| classes.get(&context.class))
        });
        let mut completions = self
            .hierarchy_classes
            .get(&document.graph)
            .into_iter()
            .flat_map(|classes| classes.values())
            .filter(|candidate| candidate.class.is_open)
            .filter(|candidate| excluded_source_name != Some(candidate.class.source_name.as_str()))
            .filter(|candidate| {
                current.is_none_or(|current| {
                    candidate.class.identity != current.class.identity
                        && !candidate
                            .class
                            .ancestors
                            .contains(&current.class.qualified_name)
                })
            })
            .filter_map(|candidate| {
                let label = self.visible_class_label(
                    document,
                    candidate,
                    recovered_namespace,
                    recovered_imports,
                )?;
                let parameters = candidate.class.type_parameters.len();
                let insert_text = if candidate.class.type_parameters.is_empty() {
                    None
                } else {
                    Some(format!(
                        "{}<{}>",
                        label,
                        candidate
                            .class
                            .type_parameters
                            .iter()
                            .enumerate()
                            .map(|(index, parameter)| format!("${{{}:{parameter}}}", index + 1))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                };
                Some(IndexedCompletion {
                    label,
                    kind: 7,
                    detail: format!(
                        "Open class `{}` ({parameters} type parameter{})",
                        candidate.class.qualified_name,
                        if parameters == 1 { "" } else { "s" }
                    ),
                    documentation: Some("Compiler-validated inheritance candidate.".to_string()),
                    insert_text,
                    snippet: parameters > 0,
                })
            })
            .collect::<Vec<_>>();
        completions.sort_by(|left, right| left.label.cmp(&right.label));
        completions
    }

    pub(crate) fn override_completions(
        &self,
        graph: &str,
        context: &HierarchyContext,
    ) -> Vec<IndexedCompletion> {
        let implemented_roots = self
            .hierarchy_members
            .iter()
            .filter(|member| member.graph == graph)
            .filter(|member| member.member.owner == context.class)
            .filter_map(|member| member.member.virtual_root)
            .collect::<HashSet<_>>();
        let implemented_names = self
            .hierarchy_members
            .iter()
            .filter(|member| member.graph == graph)
            .filter(|member| member.member.owner == context.class)
            .map(|member| member.member.name.as_str())
            .collect::<HashSet<_>>();
        let parents = self.member_parents.get(graph);
        let mut owner = parents.and_then(|parents| parents.get(&context.class));
        let mut visited = HashSet::new();
        let mut completions = Vec::new();
        while let Some(parent) = owner {
            if !visited.insert(parent.clone()) {
                break;
            }
            completions.extend(
                self.hierarchy_members
                    .iter()
                    .filter(|candidate| candidate.graph == graph)
                    .filter(|candidate| candidate.member.owner == *parent)
                    .filter(|candidate| candidate.member.kind == MemberKind::Method)
                    .filter(|candidate| candidate.member.access == MemberAccess::External)
                    .filter(|candidate| candidate.member.virtual_root.is_some())
                    .filter(|candidate| {
                        candidate
                            .member
                            .virtual_root
                            .is_none_or(|root| !implemented_roots.contains(&root))
                            && !implemented_names.contains(candidate.member.name.as_str())
                    })
                    .map(|candidate| IndexedCompletion {
                        label: candidate.member.name.clone(),
                        kind: 2,
                        detail: format!("Override {}", candidate.member.detail),
                        documentation: candidate.member.documentation.clone(),
                        insert_text: candidate.member.override_stub.clone(),
                        snippet: candidate.member.override_stub.is_some(),
                    }),
            );
            owner = parents.and_then(|parents| parents.get(parent));
        }
        let mut labels = HashSet::new();
        completions.retain(|completion| labels.insert(completion.label.clone()));
        completions.sort_by(|left, right| left.label.cmp(&right.label));
        completions
    }

    pub(crate) fn parent_completions(
        &self,
        graph: &str,
        context: &HierarchyContext,
    ) -> Vec<IndexedCompletion> {
        let Some(parents) = self.member_parents.get(graph) else {
            return Vec::new();
        };
        let Some(parent) = parents.get(&context.class) else {
            return Vec::new();
        };
        let static_context = context.method.is_some_and(|method| method.is_static);
        let constructor_context = context.method.is_some_and(|method| method.constructor);
        let mut owner = Some(parent);
        let mut direct = true;
        let mut visited = HashSet::new();
        let mut names = HashSet::new();
        let mut completions = Vec::new();
        while let Some(current) = owner {
            if !visited.insert(current.clone()) {
                break;
            }
            completions.extend(
                self.hierarchy_members
                    .iter()
                    .filter(|candidate| candidate.graph == graph)
                    .filter(|candidate| candidate.member.owner == *current)
                    .filter(|candidate| candidate.member.access == MemberAccess::External)
                    .filter(|candidate| candidate.member.name != "__destruct")
                    .filter(|candidate| {
                        if candidate.member.name == "__construct" {
                            return direct && constructor_context;
                        }
                        match candidate.member.kind {
                            MemberKind::Method => !static_context || candidate.member.is_static,
                            MemberKind::Property => candidate.member.is_static,
                            MemberKind::Constant | MemberKind::EnumCase => true,
                        }
                    })
                    .filter(|candidate| names.insert(candidate.member.name.clone()))
                    .map(|candidate| IndexedCompletion {
                        label: candidate.member.name.clone(),
                        kind: match candidate.member.kind {
                            MemberKind::Method => 2,
                            MemberKind::Property => 10,
                            MemberKind::Constant | MemberKind::EnumCase => 21,
                        },
                        detail: format!("Parent member: {}", candidate.member.detail),
                        documentation: candidate.member.documentation.clone(),
                        insert_text: None,
                        snippet: false,
                    }),
            );
            owner = parents.get(current);
            direct = false;
        }
        completions.sort_by(|left, right| left.label.cmp(&right.label));
        completions
    }

    fn visible_class_label(
        &self,
        document: &DocumentSummary,
        candidate: &IndexedHierarchyClass,
        recovered_namespace: Option<&str>,
        recovered_imports: &[(String, String)],
    ) -> Option<String> {
        let candidate_package = symbol_package(&candidate.class.identity)?;
        if candidate.class.access == MemberAccess::Internal
            && candidate_package != &document.package
        {
            return None;
        }
        if let Some((_, alias)) = recovered_imports
            .iter()
            .find(|(target, _)| target == &candidate.class.qualified_name)
        {
            return Some(alias.clone());
        }
        if let Some(alias) = document.imports.iter().find_map(|(alias, _, target)| {
            (target.as_ref() == Some(&candidate.class.identity)).then(|| alias.clone())
        }) {
            return Some(alias);
        }
        if candidate_package == &document.package {
            let candidate_namespace = self
                .documents
                .get(&candidate.uri)
                .and_then(|document| document.namespace.as_ref());
            let current_namespace = recovered_namespace.or(document.namespace.as_deref());
            return Some(
                if candidate_namespace.map(String::as_str) == current_namespace {
                    candidate.class.source_name.clone()
                } else {
                    candidate.class.qualified_name.clone()
                },
            );
        }
        None
    }

    pub(crate) fn test_symbols(&self, query: &str) -> Vec<IndexedTestSymbol> {
        let query = query.to_ascii_lowercase();
        self.test_symbols
            .iter()
            .filter(|symbol| query.is_empty() || symbol.name.to_ascii_lowercase().contains(&query))
            .cloned()
            .collect()
    }

    pub(crate) fn import_candidates(
        &self,
        uri: &str,
        name: &str,
        role: GlobalReferenceRole,
    ) -> Vec<IndexedImportCandidate> {
        let Some(document) = self.documents.get(uri) else {
            return Vec::new();
        };
        let mut candidates = HashMap::<String, IndexedImportCandidate>::new();
        for summary in self.documents.values() {
            if summary.package != document.package || summary.namespace == document.namespace {
                continue;
            }
            for (symbol, source_name, kind) in &summary.declarations {
                if source_name != name
                    || !import_kind_matches_reference_role(*kind, role)
                    || self.declaration_counts.get(symbol) != Some(&1)
                {
                    continue;
                }
                candidates.insert(
                    symbol.qualified_name.clone(),
                    IndexedImportCandidate {
                        target: symbol.qualified_name.clone(),
                        class_like: is_class_like_import_kind(*kind),
                    },
                );
            }
        }
        let mut candidates = candidates.into_values().collect::<Vec<_>>();
        candidates.sort_by(|left, right| left.target.cmp(&right.target));
        candidates
    }

    pub(crate) fn attribute_completions(&self, uri: &str) -> Vec<IndexedCompletion> {
        let Some(document) = self.documents.get(uri) else {
            return Vec::new();
        };
        let mut completions = HashMap::<String, IndexedCompletion>::new();
        for name in doriac::names::COMPILER_KNOWN_ATTRIBUTES {
            completions.insert(
                name.to_string(),
                IndexedCompletion {
                    label: name.to_string(),
                    kind: 7,
                    detail: format!("Compiler-known attribute `{name}`"),
                    documentation: Some(compiler_known_attribute_documentation(name)),
                    insert_text: None,
                    snippet: false,
                },
            );
        }
        for schema in self.attribute_schemas.values() {
            let Some(label) = self.attribute_source_name(document, schema) else {
                continue;
            };
            completions
                .entry(label.clone())
                .or_insert_with(|| IndexedCompletion {
                    label,
                    kind: 7,
                    detail: format!("Typed attribute `{}`", schema.canonical_name),
                    documentation: Some(attribute_schema_completion_documentation(schema)),
                    insert_text: None,
                    snippet: false,
                });
        }
        let mut completions = completions.into_values().collect::<Vec<_>>();
        completions.sort_by(|left, right| left.label.cmp(&right.label));
        completions
    }

    pub(crate) fn attribute_argument_completions(
        &self,
        uri: &str,
        source_name: &str,
        positional_count: usize,
        supplied_names: &HashSet<String>,
        _named_started: bool,
    ) -> Vec<IndexedCompletion> {
        let Some(document) = self.documents.get(uri) else {
            return Vec::new();
        };
        let Some(schema) = self.attribute_schema_for_source_name(document, source_name) else {
            return Vec::new();
        };
        schema
            .parameters
            .iter()
            .filter(|parameter| parameter.index >= positional_count)
            .filter(|parameter| !supplied_names.contains(&parameter.name))
            .map(attribute_parameter_completion)
            .collect()
    }

    fn attribute_source_name(
        &self,
        document: &DocumentSummary,
        schema: &AttributeClassSchema,
    ) -> Option<String> {
        match &schema.identity {
            AttributeClassIdentity::CompilerKnown(_) => Some(schema.canonical_name.clone()),
            AttributeClassIdentity::User(symbol) => {
                if self.symbol_access.get(symbol) == Some(&MemberAccess::Internal)
                    && symbol_package(symbol) != Some(&document.package)
                {
                    return None;
                }
                if let Some((alias, _, _)) = document
                    .imports
                    .iter()
                    .find(|(_, _, target)| target.as_ref() == Some(symbol))
                {
                    return Some(alias.clone());
                }
                if schema.package != document.package {
                    return None;
                }
                let (namespace, short) = schema.canonical_name.rsplit_once('\\').map_or(
                    (None, schema.canonical_name.as_str()),
                    |(namespace, short)| (Some(namespace), short),
                );
                if namespace == document.namespace.as_deref() {
                    Some(short.to_string())
                } else {
                    Some(schema.canonical_name.clone())
                }
            }
        }
    }

    fn attribute_schema_for_source_name(
        &self,
        document: &DocumentSummary,
        source_name: &str,
    ) -> Option<&AttributeClassSchema> {
        self.attribute_schemas.values().find(|schema| {
            self.attribute_source_name(document, schema).as_deref() == Some(source_name)
                || schema.canonical_name == source_name
        })
    }
}

fn indexed_location(occurrence: &IndexedOccurrence) -> IndexedLocation {
    IndexedLocation {
        uri: occurrence.uri.clone(),
        span: occurrence.span,
    }
}

fn symbol_package(symbol: &GlobalSymbolId) -> Option<&PackageIdentity> {
    match &symbol.owner {
        doriac::names::GlobalSymbolOwner::Package(package) => Some(package),
        doriac::names::GlobalSymbolOwner::CompilerKnown(_) => None,
    }
}

fn import_kind_matches_reference_role(kind: GlobalSymbolKind, role: GlobalReferenceRole) -> bool {
    match role {
        GlobalReferenceRole::Type
        | GlobalReferenceRole::Extends
        | GlobalReferenceRole::Implements
        | GlobalReferenceRole::Throws
        | GlobalReferenceRole::Catch
        | GlobalReferenceRole::TypeTest
        | GlobalReferenceRole::MatchPattern => matches!(
            kind,
            GlobalSymbolKind::Class
                | GlobalSymbolKind::Enum
                | GlobalSymbolKind::Interface
                | GlobalSymbolKind::Trait
                | GlobalSymbolKind::CompilerKnownType
        ),
        GlobalReferenceRole::Constructor => kind == GlobalSymbolKind::Class,
        GlobalReferenceRole::StaticQualifier => matches!(
            kind,
            GlobalSymbolKind::Class
                | GlobalSymbolKind::Enum
                | GlobalSymbolKind::Interface
                | GlobalSymbolKind::Trait
                | GlobalSymbolKind::CompilerKnownType
        ),
        GlobalReferenceRole::FunctionCall => matches!(
            kind,
            GlobalSymbolKind::Function | GlobalSymbolKind::CompilerKnownIntrinsic
        ),
        GlobalReferenceRole::TestDeclaration => {
            kind == GlobalSymbolKind::CompilerKnownTestDeclaration
        }
        GlobalReferenceRole::Value => kind == GlobalSymbolKind::Constant,
        GlobalReferenceRole::AttributeClass => matches!(
            kind,
            GlobalSymbolKind::Class | GlobalSymbolKind::CompilerKnownAttribute
        ),
        GlobalReferenceRole::ImportTarget
        | GlobalReferenceRole::ImportAliasUse
        | GlobalReferenceRole::Include => false,
    }
}

fn is_class_like_import_kind(kind: GlobalSymbolKind) -> bool {
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

fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

fn span_contains_offset(span: Span, offset: usize) -> bool {
    span.start <= offset && offset < span.end
}

fn replace_final_segment(source: &str, new_name: &str) -> String {
    source.rsplit_once('\\').map_or_else(
        || new_name.to_string(),
        |(prefix, _)| format!("{prefix}\\{new_name}"),
    )
}

fn is_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    matches!(characters.next(), Some(first) if first == '_' || first.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn kind_name(kind: GlobalSymbolKind) -> &'static str {
    match kind {
        GlobalSymbolKind::Class => "Class",
        GlobalSymbolKind::Enum => "Enum",
        GlobalSymbolKind::Interface => "Interface",
        GlobalSymbolKind::Trait => "Trait",
        GlobalSymbolKind::Function => "Function",
        GlobalSymbolKind::Constant => "Constant",
        GlobalSymbolKind::CompilerKnownType => "Compiler-Known Type",
        GlobalSymbolKind::CompilerKnownIntrinsic => "Language Intrinsic",
        GlobalSymbolKind::CompilerKnownAttribute => "Compiler-Known Attribute",
        GlobalSymbolKind::CompilerKnownTestDeclaration => "Compiler-Known Test Declaration",
    }
}

fn completion_kind(kind: GlobalSymbolKind) -> u32 {
    match kind {
        GlobalSymbolKind::Class => 7,
        GlobalSymbolKind::Enum => 13,
        GlobalSymbolKind::Interface | GlobalSymbolKind::Trait => 8,
        GlobalSymbolKind::Function
        | GlobalSymbolKind::CompilerKnownIntrinsic
        | GlobalSymbolKind::CompilerKnownTestDeclaration => 3,
        GlobalSymbolKind::Constant => 21,
        GlobalSymbolKind::CompilerKnownType => 25,
        GlobalSymbolKind::CompilerKnownAttribute => 7,
    }
}

fn completion(label: String, kind: GlobalSymbolKind, qualified_name: &str) -> IndexedCompletion {
    IndexedCompletion {
        label,
        kind: completion_kind(kind),
        detail: format!("{} `{qualified_name}`", kind_name(kind)),
        documentation: None,
        insert_text: None,
        snippet: false,
    }
}

fn attribute_parameter_completion(parameter: &AttributeSchemaParameter) -> IndexedCompletion {
    let default = if parameter.has_default {
        " (default available)"
    } else {
        ""
    };
    IndexedCompletion {
        label: parameter.name.clone(),
        kind: 5,
        detail: format!(
            "Attribute argument `{}`: `{}`{default}",
            parameter.name,
            doriac::attributes::metadata_type_name(&parameter.ty)
        ),
        documentation: None,
        insert_text: None,
        snippet: false,
    }
}

fn attribute_schema_completion_documentation(schema: &AttributeClassSchema) -> String {
    let signature = schema
        .parameters
        .iter()
        .map(|parameter| {
            format!(
                "{} ${}{}",
                doriac::attributes::metadata_type_name(&parameter.ty),
                parameter.name,
                if parameter.has_default {
                    " = default"
                } else {
                    ""
                }
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "```doria\n#[{}({signature})]\n```\n\nMetadata only. Attribute constructors are not executed.",
        schema.canonical_name
    )
}

fn compiler_known_attribute_documentation(name: &str) -> String {
    match name {
        "Attribute" => "Marks a readonly, non-generic class as a typed attribute schema. Metadata only; constructors are not executed and no runtime reflection is provided.",
        "Test" => "Compiler-known test metadata. Execution lands in Stage 33 Baton test orchestration.",
        "PHPExport" => "Compiler-known bridge metadata. Bridge semantics land in Stage 41.",
        _ => "Compiler-known attribute metadata.",
    }
    .to_string()
}
