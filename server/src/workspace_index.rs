use std::collections::{HashMap, HashSet};

use doriac::names::{
    GlobalReferenceRole, GlobalSymbolId, GlobalSymbolKind, GlobalSymbolOwner, PackageIdentity,
};
use doriac::source::Span;

use crate::analysis::AnalysisSnapshot;

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
}

#[derive(Debug, Clone)]
struct DocumentSummary {
    package: PackageIdentity,
    declarations: Vec<(GlobalSymbolId, String, GlobalSymbolKind)>,
    imports: Vec<(String, GlobalSymbolId)>,
    compiler_known: Vec<(String, GlobalSymbolId, GlobalSymbolKind)>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct OpenDocumentIndex {
    occurrences: Vec<IndexedOccurrence>,
    declaration_counts: HashMap<GlobalSymbolId, usize>,
    symbol_kinds: HashMap<GlobalSymbolId, GlobalSymbolKind>,
    implicit_imports: HashSet<GlobalSymbolId>,
    documents: HashMap<String, DocumentSummary>,
}

impl OpenDocumentIndex {
    pub(crate) fn rebuild<'a>(
        documents: impl Iterator<Item = (&'a str, &'a AnalysisSnapshot)>,
    ) -> Self {
        let mut index = Self::default();
        for (uri, snapshot) in documents {
            index.add_document(uri, snapshot);
        }
        index.occurrences.sort_by(|left, right| {
            (&left.uri, left.span.start, left.span.end).cmp(&(
                &right.uri,
                right.span.start,
                right.span.end,
            ))
        });
        index
    }

    fn add_document(&mut self, uri: &str, snapshot: &AnalysisSnapshot) {
        let facts = snapshot.global_symbols();
        let package = snapshot.compilation_context().package.clone();
        let mut summary = DocumentSummary {
            package: package.clone(),
            declarations: Vec::new(),
            imports: Vec::new(),
            compiler_known: Vec::new(),
        };

        for declaration in &facts.declarations {
            *self
                .declaration_counts
                .entry(declaration.id.clone())
                .or_default() += 1;
            self.symbol_kinds
                .entry(declaration.id.clone())
                .or_insert(declaration.kind);
            summary.declarations.push((
                declaration.id.clone(),
                declaration.source_name.clone(),
                declaration.kind,
            ));
            self.occurrences.push(IndexedOccurrence {
                uri: uri.to_string(),
                span: declaration.name_span,
                symbol: declaration.id.clone(),
                role: IndexedRole::Declaration,
                source_spelling: declaration.source_name.clone(),
                alias: None,
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
                    .map(|reference| reference.symbol_id.clone())
                    .unwrap_or_else(|| package_symbol(&package, &import.target));
                (import, target)
            })
            .collect::<Vec<_>>();

        for (import, target) in &imports {
            summary.imports.push((import.alias.clone(), target.clone()));
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
            });
        }

        for unresolved in &facts.unresolved {
            let target = package_symbol(&package, &unresolved.source_spelling);
            let authored = snapshot
                .authored_qualified_name(unresolved.source_span)
                .unwrap_or(&unresolved.source_spelling);
            let alias = if unresolved.role == GlobalReferenceRole::ImportTarget {
                None
            } else {
                unresolved
                    .import_alias
                    .as_ref()
                    .map(|import_alias| AliasIdentity {
                        uri: uri.to_string(),
                        target: target.clone(),
                        alias: import_alias.clone(),
                    })
            };
            self.occurrences.push(IndexedOccurrence {
                uri: uri.to_string(),
                span: unresolved.source_span,
                symbol: target,
                role: if unresolved.role == GlobalReferenceRole::ImportTarget {
                    IndexedRole::ImportTarget
                } else if alias.is_some() {
                    IndexedRole::AliasUse
                } else {
                    IndexedRole::Reference
                },
                source_spelling: authored.to_string(),
                alias,
            });
        }

        self.documents.insert(uri.to_string(), summary);
    }

    pub(crate) fn target_at(&self, uri: &str, offset: usize) -> Option<SymbolTarget> {
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
        occurrence
            .alias
            .clone()
            .map(SymbolTarget::Alias)
            .or_else(|| Some(SymbolTarget::Canonical(occurrence.symbol.clone())))
    }

    pub(crate) fn references(
        &self,
        target: &SymbolTarget,
        include_declaration: bool,
    ) -> Vec<IndexedLocation> {
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
            })
            .map(|occurrence| IndexedLocation {
                uri: occurrence.uri.clone(),
                span: occurrence.span,
            })
            .collect()
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
                if self.declaration_counts.get(symbol) != Some(&1)
                    || self.implicit_imports.contains(symbol)
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
        }
    }

    pub(crate) fn hover(&self, uri: &str, offset: usize) -> Option<IndexedHover> {
        let target = self.target_at(uri, offset)?;
        let (symbol, alias) = match target {
            SymbolTarget::Canonical(symbol) => (symbol, None),
            SymbolTarget::Alias(alias) => (alias.target, Some(alias.alias)),
        };
        let kind = self.symbol_kinds.get(&symbol)?;
        let occurrence = self.occurrences.iter().find(|occurrence| {
            occurrence.uri == uri && span_contains_offset(occurrence.span, offset)
        })?;
        let mut markdown = format!("{} `{}`", kind_name(*kind), symbol.qualified_name);
        if let Some(alias) = alias {
            markdown.push_str(&format!("\n\nImported As `{alias}`"));
        }
        Some(IndexedHover {
            span: occurrence.span,
            markdown,
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
        for (alias, target) in &document.imports {
            let kind = self
                .symbol_kinds
                .get(target)
                .copied()
                .unwrap_or(GlobalSymbolKind::Class);
            completions.insert(
                alias.clone(),
                IndexedCompletion {
                    label: alias.clone(),
                    kind: completion_kind(kind),
                    detail: format!("Imported {} `{}`", kind_name(kind), target.qualified_name),
                },
            );
        }
        for (name, symbol, kind) in &document.compiler_known {
            completions
                .entry(name.clone())
                .or_insert_with(|| completion(name.clone(), *kind, &symbol.qualified_name));
        }
        for summary in self.documents.values() {
            if summary.package != document.package {
                continue;
            }
            for (symbol, _, kind) in &summary.declarations {
                completions
                    .entry(symbol.qualified_name.clone())
                    .or_insert_with(|| {
                        completion(symbol.qualified_name.clone(), *kind, &symbol.qualified_name)
                    });
            }
        }
        let mut completions = completions.into_values().collect::<Vec<_>>();
        completions.sort_by(|left, right| left.label.cmp(&right.label));
        completions
    }
}

fn package_symbol(package: &PackageIdentity, qualified_name: &str) -> GlobalSymbolId {
    GlobalSymbolId {
        owner: GlobalSymbolOwner::Package(package.clone()),
        qualified_name: qualified_name.to_string(),
    }
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
    }
}

fn completion_kind(kind: GlobalSymbolKind) -> u32 {
    match kind {
        GlobalSymbolKind::Class => 7,
        GlobalSymbolKind::Enum => 13,
        GlobalSymbolKind::Interface | GlobalSymbolKind::Trait => 8,
        GlobalSymbolKind::Function | GlobalSymbolKind::CompilerKnownIntrinsic => 3,
        GlobalSymbolKind::Constant => 21,
        GlobalSymbolKind::CompilerKnownType => 25,
    }
}

fn completion(label: String, kind: GlobalSymbolKind, qualified_name: &str) -> IndexedCompletion {
    IndexedCompletion {
        label,
        kind: completion_kind(kind),
        detail: format!("{} `{qualified_name}`", kind_name(kind)),
    }
}
