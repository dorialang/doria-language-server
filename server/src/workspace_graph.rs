use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use doriac::build_plan::{
    BuildPlan, BuildPlanDocument, CompilerOptions, CompilerTarget, Package, SelectedTarget, Source,
    SourceOrigin, SourceScope, TargetKind, BUILD_PLAN_SCHEMA_VERSION,
};
use doriac::compilation_graph::{
    compilation_context, GraphCompleteness, GraphLoadOptions, ProjectStructureAuthority,
};
use doriac::diagnostics::{Diagnostic, DiagnosticSource, LabelRole};
use doriac::incremental::{CompilationSession, IncrementalFacts};
use doriac::names::{GlobalSymbolFacts, SourceIdentity};
use doriac::source::SourceId;
use doriac::source_provider::{
    normalize_relative_path, FileSystemSourceProvider, InMemorySourceProvider, IncludeRequest,
    ProvidedSource, SourceProvider, SourceProviderError, SourceRequest,
};

use crate::analysis::AnalysisSnapshot;
use crate::file_uri::{file_uri_to_path, path_to_file_uri};
use crate::project::{ProjectDocument, SourceEditPolicy};

#[derive(Debug, Clone)]
pub(crate) struct OpenSource<'a> {
    pub(crate) uri: &'a str,
    pub(crate) relative_path: String,
    pub(crate) text: &'a str,
}

#[derive(Debug, Clone)]
pub(crate) struct GraphDocument {
    pub(crate) text: String,
    pub(crate) analysis: AnalysisSnapshot,
    pub(crate) source_id: SourceId,
    pub(crate) source_identity: SourceIdentity,
    pub(crate) display_path: String,
    pub(crate) edit_policy: SourceEditPolicy,
}

#[derive(Debug, Clone)]
pub(crate) struct ProjectGraphAnalysis {
    pub(crate) documents: HashMap<String, GraphDocument>,
    pub(crate) source_uris: HashMap<String, String>,
    pub(crate) incremental: IncrementalFacts,
    pub(crate) include_edges: Vec<(SourceIdentity, SourceIdentity)>,
}

#[derive(Debug, Clone)]
pub(crate) struct OpenGraphAnalysis {
    pub(crate) documents: HashMap<String, GraphDocument>,
    pub(crate) source_uris: HashMap<String, String>,
    pub(crate) incremental: IncrementalFacts,
    pub(crate) include_edges: Vec<(SourceIdentity, SourceIdentity)>,
}

#[derive(Debug, Clone)]
pub(crate) struct OpenGraphFailure {
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) source_uris: HashMap<String, String>,
}

pub(crate) fn analyze_open_graph(
    package_name: &str,
    sources: &[OpenSource<'_>],
    session: &mut CompilationSession,
) -> Result<OpenGraphAnalysis, OpenGraphFailure> {
    let mut provider = InMemorySourceProvider::new();
    let mut planned_sources = Vec::with_capacity(sources.len());
    let mut uri_by_identity = HashMap::new();
    let mut source_uris = HashMap::new();
    for source in sources {
        let identity = format!("{package_name}:{}", source.relative_path);
        provider.insert(package_name, &source.relative_path, source.text);
        uri_by_identity.insert(identity.clone(), source.uri.to_string());
        source_uris.insert(identity.clone(), source.uri.to_string());
        planned_sources.push(Source {
            identity,
            path: source.relative_path.clone(),
            scope: SourceScope::Main,
            origin: SourceOrigin::Explicit,
            generated_for: None,
        });
    }
    planned_sources.sort_by(|left, right| left.identity.cmp(&right.identity));

    let plan = BuildPlan {
        schema_version: BUILD_PLAN_SCHEMA_VERSION,
        edition: "2026".to_string(),
        root_package: package_name.to_string(),
        selected_target: SelectedTarget {
            package: package_name.to_string(),
            name: "open-documents".to_string(),
            kind: TargetKind::Library,
            entry_source: None,
            active_scopes: vec![SourceScope::Main],
        },
        packages: vec![Package {
            identity: package_name.to_string(),
            root: ".".to_string(),
            namespace_mappings: Vec::new(),
            sources: planned_sources,
            dependencies: Vec::new(),
        }],
        compiler: CompilerOptions {
            target: CompilerTarget::Debug,
            native_profile: None,
            target_triple: None,
        },
    };
    let text =
        doriac::build_plan::encode_build_plan(&plan).map_err(|diagnostics| OpenGraphFailure {
            diagnostics,
            source_uris: source_uris.clone(),
        })?;
    let document = BuildPlanDocument {
        path: format!("<doria-lsp:{package_name}>"),
        directory: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        text,
        plan,
    };
    let update = session
        .load_graph_with_options(
            &document,
            &provider,
            GraphLoadOptions {
                completeness: GraphCompleteness::Partial,
                project_structure: ProjectStructureAuthority::Unavailable,
            },
        )
        .map_err(|diagnostics| OpenGraphFailure {
            diagnostics,
            source_uris: source_uris.clone(),
        })?;
    let graph = update.graph;
    let analysis = session.analyze_graph(&graph);
    let mut documents = HashMap::new();
    for source in graph.sources.values() {
        let Some(uri) = uri_by_identity.get(&source.identity.0) else {
            continue;
        };
        source_uris.insert(source.display_path.clone(), uri.clone());
        let diagnostics = analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic_belongs_to_source(diagnostic, source.id, &source.display_path)
            })
            .cloned()
            .collect();
        let facts = source_symbol_facts(&analysis.semantic_info.global_symbols, &source.identity);
        let snapshot = AnalysisSnapshot::from_graph_source(
            &source.source.text,
            source.id,
            compilation_context(source),
            analysis
                .authored_sources
                .get(&source.identity.0)
                .unwrap_or(&source.authored),
            &analysis.semantic_info,
            diagnostics,
            facts,
        );
        documents.insert(
            uri.clone(),
            GraphDocument {
                text: source.source.text.clone(),
                analysis: snapshot,
                source_id: source.id,
                source_identity: source.identity.clone(),
                display_path: source.display_path.clone(),
                edit_policy: SourceEditPolicy::Editable,
            },
        );
    }

    Ok(OpenGraphAnalysis {
        documents,
        source_uris,
        incremental: update.facts,
        include_edges: graph
            .include_edges
            .iter()
            .map(|edge| (edge.including.clone(), edge.included.clone()))
            .collect(),
    })
}

pub(crate) fn analyze_project_graph(
    project: &ProjectDocument,
    open_sources: &[OpenSource<'_>],
    session: &mut CompilationSession,
) -> Result<ProjectGraphAnalysis, OpenGraphFailure> {
    let mut overlays = HashMap::new();
    let mut open_uri_by_path = HashMap::new();
    let source_paths = project
        .packages
        .iter()
        .flat_map(|package| package.sources.iter())
        .map(|source| (normalized_absolute(&source.path), source.identity.clone()))
        .collect::<HashMap<_, _>>();
    for source in open_sources {
        let Some(path) = file_uri_to_path(source.uri) else {
            continue;
        };
        let Some(identity) = source_paths.get(&normalized_absolute(&path)) else {
            continue;
        };
        open_uri_by_path.insert(normalized_absolute(&path), source.uri.to_string());
        overlays.insert(identity.clone(), source.text.to_string());
    }
    let provider = OverlaySourceProvider {
        filesystem: FileSystemSourceProvider,
        overlays,
    };
    let plan = project.tooling_build_plan.clone();
    let text =
        doriac::build_plan::encode_build_plan(&plan).map_err(|diagnostics| OpenGraphFailure {
            diagnostics,
            source_uris: HashMap::new(),
        })?;
    let document = BuildPlanDocument {
        path: project
            .project_root()
            .join("Baton.toml")
            .display()
            .to_string(),
        directory: project.project_root().to_path_buf(),
        text,
        plan,
    };
    let update = session
        .load_graph_with_options(
            &document,
            &provider,
            GraphLoadOptions {
                completeness: GraphCompleteness::Complete,
                project_structure: ProjectStructureAuthority::Unavailable,
            },
        )
        .map_err(|diagnostics| OpenGraphFailure {
            diagnostics,
            source_uris: HashMap::new(),
        })?;
    let graph = update.graph;
    let analysis = session.analyze_graph(&graph);
    let mut documents = HashMap::new();
    let mut source_uris = HashMap::new();
    for source in graph.sources.values() {
        let Some(path) = source.canonical_path.as_ref() else {
            continue;
        };
        let canonical_path = normalized_absolute(path);
        let uri = open_uri_by_path
            .get(&canonical_path)
            .cloned()
            .unwrap_or_else(|| path_to_file_uri(&canonical_path));
        source_uris.insert(source.identity.0.clone(), uri.clone());
        source_uris.insert(source.display_path.clone(), uri.clone());
        let diagnostics = analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic_belongs_to_source(diagnostic, source.id, &source.display_path)
            })
            .cloned()
            .collect();
        let facts = source_symbol_facts(&analysis.semantic_info.global_symbols, &source.identity);
        let snapshot = AnalysisSnapshot::from_graph_source(
            &source.source.text,
            source.id,
            compilation_context(source),
            analysis
                .authored_sources
                .get(&source.identity.0)
                .unwrap_or(&source.authored),
            &analysis.semantic_info,
            diagnostics,
            facts,
        );
        documents.insert(
            uri,
            GraphDocument {
                text: source.source.text.clone(),
                analysis: snapshot,
                source_id: source.id,
                source_identity: source.identity.clone(),
                display_path: source.display_path.clone(),
                edit_policy: project.source_policy(&source.identity.0),
            },
        );
    }

    Ok(ProjectGraphAnalysis {
        documents,
        source_uris,
        incremental: update.facts,
        include_edges: graph
            .include_edges
            .iter()
            .map(|edge| (edge.including.clone(), edge.included.clone()))
            .collect(),
    })
}

struct OverlaySourceProvider {
    filesystem: FileSystemSourceProvider,
    overlays: HashMap<String, String>,
}

impl SourceProvider for OverlaySourceProvider {
    fn read_source(
        &self,
        request: SourceRequest<'_>,
    ) -> Result<ProvidedSource, SourceProviderError> {
        let identity = format!("{}:{}", request.package.identity, request.source.path);
        let mut provided = self.filesystem.read_source(request)?;
        self.overlay(&identity, &mut provided);
        Ok(provided)
    }

    fn read_included_source(
        &self,
        request: IncludeRequest<'_>,
    ) -> Result<ProvidedSource, SourceProviderError> {
        let including = Path::new(request.including_relative_path);
        let parent = including.parent().unwrap_or_else(|| Path::new(""));
        let relative = normalize_relative_path(&parent.join(request.include_path)).ok();
        let identity = relative
            .as_ref()
            .map(|path| format!("{}:{path}", request.package.identity));
        let mut provided = self.filesystem.read_included_source(request)?;
        if let Some(identity) = identity {
            self.overlay(&identity, &mut provided);
        }
        Ok(provided)
    }
}

impl OverlaySourceProvider {
    fn overlay(&self, identity: &str, provided: &mut ProvidedSource) {
        let Some(text) = self.overlays.get(identity) else {
            return;
        };
        provided.text.clone_from(text);
        provided.content_fingerprint = content_fingerprint(text);
    }
}

fn normalized_absolute(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn content_fingerprint(text: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    format!("lsp-overlay-{:016x}", hasher.finish())
}

fn source_symbol_facts(facts: &GlobalSymbolFacts, source: &SourceIdentity) -> GlobalSymbolFacts {
    let namespace_declaration = facts
        .namespaces
        .iter()
        .find(|namespace| namespace.source_identity == *source)
        .cloned();
    GlobalSymbolFacts {
        namespaces: namespace_declaration.iter().cloned().collect(),
        namespace: namespace_declaration
            .as_ref()
            .map(|namespace| namespace.name.canonical()),
        namespace_declaration,
        declarations: facts
            .declarations
            .iter()
            .filter(|declaration| declaration.source_identity == *source)
            .cloned()
            .collect(),
        references: facts
            .references
            .iter()
            .filter(|reference| reference.source_identity == *source)
            .cloned()
            .collect(),
        imports: facts
            .imports
            .iter()
            .filter(|import| import.source_identity == *source)
            .cloned()
            .collect(),
        compiler_known: facts.compiler_known.clone(),
        unresolved: facts
            .unresolved
            .iter()
            .filter(|reference| reference.source_identity == *source)
            .cloned()
            .collect(),
    }
}

fn diagnostic_belongs_to_source(
    diagnostic: &Diagnostic,
    source_id: SourceId,
    display_path: &str,
) -> bool {
    let primary = diagnostic
        .labels
        .iter()
        .find(|label| label.role == LabelRole::Primary);
    let span = primary.map_or(diagnostic.span, |label| label.span);
    let source = primary.map(|label| &label.source);
    span.source == source_id
        || matches!(source, Some(DiagnosticSource::Path(path)) if path == display_path)
}
