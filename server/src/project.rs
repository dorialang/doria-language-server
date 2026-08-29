use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use doriac::build_plan::{
    validate_build_plan, BuildPlan, SelectedTarget, SourceOrigin, TargetKind,
};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProjectDocument {
    pub(crate) schema_version: u32,
    pub(crate) baton_version: String,
    pub(crate) workspace: Option<ProjectWorkspace>,
    pub(crate) selection: ProjectSelection,
    pub(crate) packages: Vec<ProjectPackage>,
    pub(crate) tooling_build_plan: BuildPlan,
    pub(crate) generated_sources: Vec<GeneratedSource>,
    pub(crate) fingerprints: ProjectFingerprints,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProjectWorkspace {
    pub(crate) root: PathBuf,
    pub(crate) manifest: PathBuf,
    pub(crate) lock: ProjectLock,
    pub(crate) members: Vec<ProjectMember>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProjectLock {
    pub(crate) path: PathBuf,
    pub(crate) sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProjectMember {
    pub(crate) package: String,
    pub(crate) compiler_package: String,
    pub(crate) root: PathBuf,
    pub(crate) manifest: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProjectSelection {
    pub(crate) kind: SelectionKind,
    pub(crate) package: Option<String>,
    pub(crate) development: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SelectionKind {
    Workspace,
    Package,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProjectPackage {
    pub(crate) package: String,
    pub(crate) compiler_package: String,
    pub(crate) root: PathBuf,
    pub(crate) manifest: PathBuf,
    pub(crate) manifest_fingerprint: String,
    pub(crate) source: PackageSource,
    pub(crate) dependencies: Vec<ProjectDependency>,
    pub(crate) sources: Vec<ProjectSource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PackageSource {
    Workspace,
    Path,
    Git,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProjectDependency {
    pub(crate) package: String,
    pub(crate) kind: doriac::build_plan::DependencyKind,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProjectSource {
    pub(crate) identity: String,
    pub(crate) path: PathBuf,
    pub(crate) scope: doriac::build_plan::SourceScope,
    pub(crate) origin: SourceOrigin,
    pub(crate) generated_for: Option<doriac::build_plan::GeneratedFor>,
    pub(crate) producer: Option<String>,
    pub(crate) sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GeneratedSource {
    pub(crate) identity: String,
    pub(crate) package: String,
    pub(crate) processor: String,
    pub(crate) path: PathBuf,
    pub(crate) generated_for: String,
    pub(crate) sha256: String,
    pub(crate) compiler_revision: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProjectFingerprints {
    pub(crate) workspace: String,
    pub(crate) lock: String,
    pub(crate) inventory: String,
    pub(crate) build_plan: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceEditPolicy {
    Editable,
    Generated,
    DependencyCache,
}

impl ProjectDocument {
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let document = serde_json::from_str::<Self>(text)
            .map_err(|error| format!("invalid Baton project JSON: {error}"))?;
        document.validate()?;
        Ok(document)
    }

    pub(crate) fn project_root(&self) -> &Path {
        self.workspace.as_ref().map_or_else(
            || self.packages[0].root.as_path(),
            |workspace| workspace.root.as_path(),
        )
    }

    pub(crate) fn source_policy(&self, identity: &str) -> SourceEditPolicy {
        let generated = self
            .generated_sources
            .iter()
            .any(|source| source.identity == identity);
        if generated {
            return SourceEditPolicy::Generated;
        }
        self.packages
            .iter()
            .find(|package| identity.starts_with(&format!("{}:", package.compiler_package)))
            .map_or(SourceEditPolicy::DependencyCache, |package| {
                match package.source {
                    PackageSource::Git => SourceEditPolicy::DependencyCache,
                    PackageSource::Workspace | PackageSource::Path => SourceEditPolicy::Editable,
                }
            })
    }

    pub(crate) fn analysis_plans(&self) -> Vec<(String, BuildPlan)> {
        if self.selection.kind == SelectionKind::Package {
            return vec![(
                self.tooling_build_plan.root_package.clone(),
                self.tooling_build_plan.clone(),
            )];
        }

        let packages = self
            .tooling_build_plan
            .packages
            .iter()
            .map(|package| (package.identity.as_str(), package))
            .collect::<HashMap<_, _>>();
        let mut roots = self
            .workspace
            .as_ref()
            .into_iter()
            .flat_map(|workspace| workspace.members.iter())
            .map(|member| member.compiler_package.clone())
            .collect::<Vec<_>>();
        roots.sort();
        roots.dedup();
        roots
            .into_iter()
            .map(|root| {
                let mut included = BTreeSet::from([root.clone()]);
                let mut pending = vec![root.clone()];
                while let Some(identity) = pending.pop() {
                    let package = packages
                        .get(identity.as_str())
                        .expect("validated workspace member is present in tooling plan");
                    for dependency in &package.dependencies {
                        if included.insert(dependency.package.clone()) {
                            pending.push(dependency.package.clone());
                        }
                    }
                }
                let plan_packages = self
                    .tooling_build_plan
                    .packages
                    .iter()
                    .filter(|package| included.contains(&package.identity))
                    .cloned()
                    .collect();
                let plan = BuildPlan {
                    schema_version: self.tooling_build_plan.schema_version,
                    edition: self.tooling_build_plan.edition.clone(),
                    root_package: root.clone(),
                    selected_target: SelectedTarget {
                        package: root.clone(),
                        name: "baton-tooling".to_string(),
                        kind: TargetKind::Library,
                        entry_source: None,
                        active_scopes: self
                            .tooling_build_plan
                            .selected_target
                            .active_scopes
                            .clone(),
                    },
                    packages: plan_packages,
                    compiler: self.tooling_build_plan.compiler.clone(),
                };
                (root, plan)
            })
            .collect()
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err(format!(
                "unsupported Baton project schema version `{}`; expected `1`",
                self.schema_version
            ));
        }
        if self.baton_version.trim().is_empty() {
            return Err("Baton project document has an empty batonVersion".to_string());
        }
        validate_build_plan(&self.tooling_build_plan)
            .map_err(|diagnostics| format!("invalid tooling build plan: {diagnostics:#?}"))?;
        if self.packages.is_empty() {
            return Err("Baton project document contains no packages".to_string());
        }
        match (
            self.selection.kind,
            self.selection.package.as_ref(),
            self.workspace.as_ref(),
        ) {
            (SelectionKind::Workspace, None, Some(_)) | (SelectionKind::Package, Some(_), _) => {}
            (SelectionKind::Workspace, _, None) => {
                return Err("workspace selection requires workspace metadata".to_string())
            }
            (SelectionKind::Workspace, Some(_), _) => {
                return Err("workspace selection must not name one package".to_string())
            }
            (SelectionKind::Package, None, _) => {
                return Err("package selection must name the selected package".to_string())
            }
        }

        let mut package_names = HashSet::new();
        let mut compiler_packages = HashSet::new();
        let mut source_identities = HashSet::new();
        let mut sources = HashMap::new();
        for package in &self.packages {
            require_absolute(&package.root, "package root")?;
            require_absolute(&package.manifest, "package manifest")?;
            require_digest(&package.manifest_fingerprint, "manifest fingerprint")?;
            if !package_names.insert(&package.package) {
                return Err(format!("duplicate project package `{}`", package.package));
            }
            if !compiler_packages.insert(package.compiler_package.as_str()) {
                return Err(format!(
                    "duplicate compiler package `{}`",
                    package.compiler_package
                ));
            }
            for dependency in &package.dependencies {
                if dependency.package.trim().is_empty() {
                    return Err("project dependency package cannot be empty".to_string());
                }
                let _ = dependency.kind;
            }
            for source in &package.sources {
                require_absolute(&source.path, "source path")?;
                require_digest(&source.sha256, "source digest")?;
                if !source
                    .identity
                    .starts_with(&format!("{}:", package.compiler_package))
                {
                    return Err(format!(
                        "source `{}` does not belong to compiler package `{}`",
                        source.identity, package.compiler_package
                    ));
                }
                if !source_identities.insert(&source.identity) {
                    return Err(format!("duplicate source identity `{}`", source.identity));
                }
                let _ = (
                    source.scope,
                    source.origin,
                    source.generated_for,
                    &source.producer,
                );
                sources.insert(source.identity.as_str(), source.path.as_path());
            }
        }

        let plan_packages = self
            .tooling_build_plan
            .packages
            .iter()
            .map(|package| (package.identity.as_str(), package))
            .collect::<HashMap<_, _>>();
        if plan_packages.keys().copied().collect::<HashSet<_>>() != compiler_packages {
            return Err("project packages and tooling build-plan packages differ".to_string());
        }
        for package in &self.packages {
            let plan_package = plan_packages[package.compiler_package.as_str()];
            let mut project_dependencies = package
                .dependencies
                .iter()
                .map(|dependency| (dependency.package.as_str(), dependency.kind))
                .collect::<Vec<_>>();
            project_dependencies.sort();
            let mut plan_dependencies = plan_package
                .dependencies
                .iter()
                .map(|dependency| (dependency.package.as_str(), dependency.kind))
                .collect::<Vec<_>>();
            plan_dependencies.sort();
            if project_dependencies != plan_dependencies {
                return Err(format!(
                    "project dependencies for `{}` differ from the tooling build plan",
                    package.compiler_package
                ));
            }
            let project_sources = package
                .sources
                .iter()
                .map(|source| (source.identity.as_str(), source))
                .collect::<HashMap<_, _>>();
            if project_sources.len() != plan_package.sources.len() {
                return Err(format!(
                    "project sources for `{}` differ from the tooling build plan",
                    package.compiler_package
                ));
            }
            for plan_source in &plan_package.sources {
                let Some(source) = project_sources.get(plan_source.identity.as_str()) else {
                    return Err(format!(
                        "tooling build-plan source `{}` is absent from project inventory",
                        plan_source.identity
                    ));
                };
                if source.scope != plan_source.scope
                    || source.origin != plan_source.origin
                    || source.generated_for != plan_source.generated_for
                {
                    return Err(format!(
                        "project source `{}` disagrees with the tooling build plan",
                        source.identity
                    ));
                }
            }
        }
        for generated in &self.generated_sources {
            require_absolute(&generated.path, "generated source path")?;
            require_digest(&generated.sha256, "generated source digest")?;
            if sources.get(generated.identity.as_str()) != Some(&generated.path.as_path()) {
                return Err(format!(
                    "generated source `{}` is absent from its package inventory",
                    generated.identity
                ));
            }
            if generated.package.trim().is_empty()
                || generated.processor.trim().is_empty()
                || generated.generated_for.trim().is_empty()
                || generated.compiler_revision.trim().is_empty()
            {
                return Err(format!(
                    "generated source `{}` has incomplete provenance",
                    generated.identity
                ));
            }
            if generated.compiler_revision != doriac::BUILD_COMMIT {
                return Err(format!(
                    "generated source `{}` was produced by compiler revision `{}`; expected `{}`",
                    generated.identity,
                    generated.compiler_revision,
                    doriac::BUILD_COMMIT
                ));
            }
        }
        if let Some(workspace) = &self.workspace {
            require_absolute(&workspace.root, "workspace root")?;
            require_absolute(&workspace.manifest, "workspace manifest")?;
            require_absolute(&workspace.lock.path, "workspace lock")?;
            require_digest(&workspace.lock.sha256, "workspace lock digest")?;
            for member in &workspace.members {
                require_absolute(&member.root, "workspace member root")?;
                require_absolute(&member.manifest, "workspace member manifest")?;
                if member.package.trim().is_empty() || member.compiler_package.trim().is_empty() {
                    return Err("workspace member identities cannot be empty".to_string());
                }
                if !compiler_packages.contains(member.compiler_package.as_str()) {
                    return Err(format!(
                        "workspace member `{}` is absent from the tooling build plan",
                        member.compiler_package
                    ));
                }
            }
        }
        for (name, digest) in [
            ("workspace", &self.fingerprints.workspace),
            ("lock", &self.fingerprints.lock),
            ("inventory", &self.fingerprints.inventory),
            ("build plan", &self.fingerprints.build_plan),
        ] {
            require_digest(digest, name)?;
        }
        let _ = self.selection.development;
        Ok(())
    }
}

fn require_absolute(path: &Path, description: &str) -> Result<(), String> {
    if path.is_absolute() {
        Ok(())
    } else {
        Err(format!(
            "{description} must be an absolute path: {}",
            path.display()
        ))
    }
}

fn require_digest(value: &str, description: &str) -> Result<(), String> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(format!("{description} must be a SHA-256 digest"))
    }
}

#[cfg(test)]
pub(crate) fn test_project(
    root: &Path,
    relative_paths: &[&str],
    package_source: PackageSource,
    generated_paths: &[&str],
) -> ProjectDocument {
    use doriac::build_plan::{
        CompilerOptions, CompilerTarget, Package, SelectedTarget, Source, SourceScope, TargetKind,
        BUILD_PLAN_SCHEMA_VERSION,
    };

    let compiler_package = "acme/app";
    let plan_sources = relative_paths
        .iter()
        .map(|path| Source {
            identity: format!("{compiler_package}:{path}"),
            path: (*path).to_string(),
            scope: SourceScope::Main,
            origin: SourceOrigin::Explicit,
            generated_for: None,
        })
        .collect::<Vec<_>>();
    let project_sources = relative_paths
        .iter()
        .map(|path| ProjectSource {
            identity: format!("{compiler_package}:{path}"),
            path: root.join(path),
            scope: SourceScope::Main,
            origin: SourceOrigin::Explicit,
            generated_for: None,
            producer: None,
            sha256: "0".repeat(64),
        })
        .collect::<Vec<_>>();
    ProjectDocument {
        schema_version: 1,
        baton_version: "test".to_string(),
        workspace: None,
        selection: ProjectSelection {
            kind: SelectionKind::Package,
            package: Some(compiler_package.to_string()),
            development: true,
        },
        packages: vec![ProjectPackage {
            package: compiler_package.to_string(),
            compiler_package: compiler_package.to_string(),
            root: root.to_path_buf(),
            manifest: root.join("Baton.toml"),
            manifest_fingerprint: "0".repeat(64),
            source: package_source,
            dependencies: Vec::new(),
            sources: project_sources,
        }],
        tooling_build_plan: BuildPlan {
            schema_version: BUILD_PLAN_SCHEMA_VERSION,
            edition: "2026".to_string(),
            root_package: compiler_package.to_string(),
            selected_target: SelectedTarget {
                package: compiler_package.to_string(),
                name: "baton-tooling".to_string(),
                kind: TargetKind::Library,
                entry_source: None,
                active_scopes: vec![SourceScope::Main],
            },
            packages: vec![Package {
                identity: compiler_package.to_string(),
                root: root.display().to_string(),
                namespace_mappings: Vec::new(),
                sources: plan_sources,
                dependencies: Vec::new(),
            }],
            compiler: CompilerOptions {
                target: CompilerTarget::Debug,
                native_profile: None,
                target_triple: None,
            },
        },
        generated_sources: generated_paths
            .iter()
            .map(|path| GeneratedSource {
                identity: format!("{compiler_package}:{path}"),
                package: compiler_package.to_string(),
                processor: "acme/generator".to_string(),
                path: root.join(path),
                generated_for: "main".to_string(),
                sha256: "0".repeat(64),
                compiler_revision: doriac::BUILD_COMMIT.to_string(),
            })
            .collect(),
        fingerprints: ProjectFingerprints {
            workspace: "0".repeat(64),
            lock: "0".repeat(64),
            inventory: "0".repeat(64),
            build_plan: "0".repeat(64),
        },
    }
}

#[cfg(test)]
mod tests {
    use doriac::build_plan::{
        CompilerOptions, CompilerTarget, Package, SelectedTarget, Source, SourceScope, TargetKind,
        BUILD_PLAN_SCHEMA_VERSION,
    };
    use serde_json::{json, Value};

    use super::*;

    fn project_value(root: &Path) -> Value {
        let source_path = root.join("src/main.doria");
        let manifest = root.join("Baton.toml");
        let digest = "0".repeat(64);
        let plan = BuildPlan {
            schema_version: BUILD_PLAN_SCHEMA_VERSION,
            edition: "2026".to_string(),
            root_package: "acme/app".to_string(),
            selected_target: SelectedTarget {
                package: "acme/app".to_string(),
                name: "baton-tooling".to_string(),
                kind: TargetKind::Library,
                entry_source: None,
                active_scopes: vec![SourceScope::Main],
            },
            packages: vec![Package {
                identity: "acme/app".to_string(),
                root: root.display().to_string(),
                namespace_mappings: Vec::new(),
                sources: vec![Source {
                    identity: "acme/app:src/main.doria".to_string(),
                    path: "src/main.doria".to_string(),
                    scope: SourceScope::Main,
                    origin: SourceOrigin::Explicit,
                    generated_for: None,
                }],
                dependencies: Vec::new(),
            }],
            compiler: CompilerOptions {
                target: CompilerTarget::Debug,
                native_profile: None,
                target_triple: None,
            },
        };
        json!({
            "schemaVersion": 1,
            "batonVersion": "2026.3.1-canary",
            "workspace": null,
            "selection": {
                "kind": "package",
                "package": "acme/app",
                "development": true
            },
            "packages": [{
                "package": "acme/app",
                "compilerPackage": "acme/app",
                "root": root,
                "manifest": manifest,
                "manifestFingerprint": digest,
                "source": "path",
                "dependencies": [],
                "sources": [{
                    "identity": "acme/app:src/main.doria",
                    "path": source_path,
                    "scope": "main",
                    "origin": "explicit",
                    "generatedFor": null,
                    "producer": null,
                    "sha256": "0".repeat(64)
                }]
            }],
            "toolingBuildPlan": plan,
            "generatedSources": [],
            "fingerprints": {
                "workspace": "0".repeat(64),
                "lock": "0".repeat(64),
                "inventory": "0".repeat(64),
                "buildPlan": "0".repeat(64)
            }
        })
    }

    #[test]
    fn parses_strict_schema_one_project_documents() {
        let root = std::env::temp_dir().join("doria-lsp-project-schema");
        let value = project_value(&root);
        let project = ProjectDocument::parse(&value.to_string()).unwrap();
        assert_eq!(project.project_root(), root);
        assert_eq!(
            project.source_policy("acme/app:src/main.doria"),
            SourceEditPolicy::Editable
        );

        let mut unknown = value;
        unknown["unexpected"] = json!(true);
        assert!(ProjectDocument::parse(&unknown.to_string())
            .unwrap_err()
            .contains("unknown field"));
    }

    #[test]
    fn generated_and_git_sources_are_read_only() {
        let root = std::env::temp_dir().join("doria-lsp-project-policy");
        let mut value = project_value(&root);
        value["generatedSources"] = json!([{
            "identity": "acme/app:src/main.doria",
            "package": "acme/app",
            "processor": "acme/generator",
            "path": root.join("src/main.doria"),
            "generatedFor": "main",
            "sha256": "0".repeat(64),
            "compilerRevision": doriac::BUILD_COMMIT
        }]);
        let project = ProjectDocument::parse(&value.to_string()).unwrap();
        assert_eq!(
            project.source_policy("acme/app:src/main.doria"),
            SourceEditPolicy::Generated
        );

        value["generatedSources"] = json!([]);
        value["packages"][0]["source"] = json!("git");
        let project = ProjectDocument::parse(&value.to_string()).unwrap();
        assert_eq!(
            project.source_policy("acme/app:src/main.doria"),
            SourceEditPolicy::DependencyCache
        );
    }

    #[test]
    fn rejects_project_facts_that_disagree_with_the_compiler_plan() {
        let root = std::env::temp_dir().join("doria-lsp-project-consistency");
        let mut value = project_value(&root);
        value["packages"][0]["sources"][0]["scope"] = json!("development");
        assert!(ProjectDocument::parse(&value.to_string())
            .unwrap_err()
            .contains("disagrees with the tooling build plan"));

        let mut value = project_value(&root);
        value["generatedSources"] = json!([{
            "identity": "acme/app:src/main.doria",
            "package": "acme/app",
            "processor": "acme/generator",
            "path": root.join("src/main.doria"),
            "generatedFor": "main",
            "sha256": "0".repeat(64),
            "compilerRevision": "stale"
        }]);
        assert!(ProjectDocument::parse(&value.to_string())
            .unwrap_err()
            .contains("expected"));
    }

    #[test]
    fn workspace_analysis_plans_isolate_members_and_keep_real_dependency_closures() {
        let root = std::env::temp_dir().join("doria-lsp-project-workspace-plans");
        let app_root = root.join("apps/app");
        let support_root = root.join("packages/support");
        let processor_root = root.join("tools/processor");
        let digest = "0".repeat(64);
        let mut value = project_value(&app_root);
        value["workspace"] = json!({
            "root": root,
            "manifest": root.join("Baton.toml"),
            "lock": { "path": root.join("Baton.lock"), "sha256": digest },
            "members": [
                {
                    "package": "acme/app",
                    "compilerPackage": "acme/app",
                    "root": app_root,
                    "manifest": app_root.join("Baton.toml")
                },
                {
                    "package": "acme/processor",
                    "compilerPackage": "acme/processor",
                    "root": processor_root,
                    "manifest": processor_root.join("Baton.toml")
                },
                {
                    "package": "acme/support",
                    "compilerPackage": "acme/support",
                    "root": support_root,
                    "manifest": support_root.join("Baton.toml")
                }
            ]
        });
        value["selection"] = json!({
            "kind": "workspace",
            "package": null,
            "development": true
        });
        value["packages"][0]["dependencies"] =
            json!([{ "package": "acme/support", "kind": "normal" }]);
        value["toolingBuildPlan"]["packages"][0]["dependencies"] =
            json!([{ "package": "acme/support", "kind": "normal" }]);
        value["packages"].as_array_mut().unwrap().extend([
            json!({
                "package": "acme/processor",
                "compilerPackage": "acme/processor",
                "root": processor_root,
                "manifest": processor_root.join("Baton.toml"),
                "manifestFingerprint": digest,
                "source": "workspace",
                "dependencies": [],
                "sources": [{
                    "identity": "acme/processor:src/main.doria",
                    "path": processor_root.join("src/main.doria"),
                    "scope": "main",
                    "origin": "explicit",
                    "generatedFor": null,
                    "producer": null,
                    "sha256": digest
                }]
            }),
            json!({
                "package": "acme/support",
                "compilerPackage": "acme/support",
                "root": support_root,
                "manifest": support_root.join("Baton.toml"),
                "manifestFingerprint": digest,
                "source": "workspace",
                "dependencies": [],
                "sources": [{
                    "identity": "acme/support:src/Support.doria",
                    "path": support_root.join("src/Support.doria"),
                    "scope": "main",
                    "origin": "autoload",
                    "generatedFor": null,
                    "producer": null,
                    "sha256": digest
                }]
            }),
        ]);
        value["toolingBuildPlan"]["packages"]
            .as_array_mut()
            .unwrap()
            .extend([
                json!({
                    "identity": "acme/processor",
                    "root": processor_root,
                    "namespaceMappings": [],
                    "sources": [{
                        "identity": "acme/processor:src/main.doria",
                        "path": "src/main.doria",
                        "scope": "main",
                        "origin": "explicit",
                        "generatedFor": null
                    }],
                    "dependencies": []
                }),
                json!({
                    "identity": "acme/support",
                    "root": support_root,
                    "namespaceMappings": [],
                    "sources": [{
                        "identity": "acme/support:src/Support.doria",
                        "path": "src/Support.doria",
                        "scope": "main",
                        "origin": "autoload",
                        "generatedFor": null
                    }],
                    "dependencies": []
                }),
            ]);

        let project = ProjectDocument::parse(&value.to_string()).unwrap();
        let plans = project.analysis_plans();
        assert_eq!(
            plans
                .iter()
                .map(|(root, _)| root.as_str())
                .collect::<Vec<_>>(),
            ["acme/app", "acme/processor", "acme/support"]
        );
        assert_eq!(
            plans[0]
                .1
                .packages
                .iter()
                .map(|package| package.identity.as_str())
                .collect::<Vec<_>>(),
            ["acme/app", "acme/support"]
        );
        assert_eq!(
            plans[1]
                .1
                .packages
                .iter()
                .map(|package| package.identity.as_str())
                .collect::<Vec<_>>(),
            ["acme/processor"]
        );
        for (_, plan) in plans {
            validate_build_plan(&plan).unwrap();
        }
    }
}
