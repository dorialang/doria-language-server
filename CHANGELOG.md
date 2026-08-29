# Changelog

All notable changes to the Doria Language Server and official editor integrations are recorded here.

This project follows the Doria toolchain CalVer.

## Unreleased

- Completed Stage 33 project tooling and Phase F by consuming Baton's strict
  schema-1 project document asynchronously, loading complete compiler graphs with
  unsaved overlays, indexing unopened and generated sources, refusing edits to
  generated and Git-cache sources, and retaining bounded partial-graph fallback.
  Added debounced project watchers, manual refresh, and one Baton path override
  to both official editor integrations. Stage 34 single class inheritance is next.

- Added compiler-resolved go-to-definition and conservative rename for local
  bindings and cross-file methods, properties, class constants, and enum cases,
  with standard LSP navigation and workspace edits exposed in both official
  editor integrations.
- Preserved compiler error severity for unresolved function and method calls,
  and added explicit LSP refactor actions that generate conservative free,
  instance, and static callable stubs in VS Code and JetBrains editors without
  presenting inferred edits as compiler-approved automatic fixes.
- Completed Stage 32 tooling by pinning the typed-attribute compiler and using
  its schemas, canonical identities, bound constant values, and source spans for
  scoped marker/schema and named-argument completion, typed metadata hover,
  semantic tokens, cross-file navigation/references, and conservative class and
  constructor-parameter rename. Added accepted/rejected shared fixtures and
  presentation-only VS Code and IntelliJ regressions. `Test`, `PHPExport`,
  processor execution, and runtime reflection remain deliberately inactive.
- Aligned diagnostics and semantic hover with the ambient canonical I/O and
  fallible-finalizer correction. Canonical I/O retains checked runtime transport
  without source `throws`; required effects remain explicit, finalizer Errors
  flow to enclosing contexts, and `E0632` remains historical and reserved.
- Completed Stage 31 tooling integration by pinning the final build-plan
  compiler, analyzing each open workspace package through a reusable partial
  compilation graph, and adding compiler-backed cross-file definition,
  references, conservative rename, rich hover, same-namespace completion,
  multi-source diagnostics and fixes, include facts, incremental invalidation,
  and stale-result clearing. This partial-graph path remains the Stage 33 fallback
  when authoritative Baton project discovery is unavailable.
- Completed Stage 31 Slice 1 editor alignment: pinned the namespace-aware
  compiler, assigned synthetic package identities by longest workspace root,
  indexed compiler-owned canonical symbols across open documents, and added
  namespace/import/include hover, completion, references, conservative rename,
  semantic tokens, highlighting, diagnostics, and UTF-16 regression coverage.
  This established the open-document index that Slice 2 subsequently moved onto
  the compiler's partial compilation graph.
- Pinned `doria-lsp` to the final integrated Stage 30 compiler and preserved
  exact structural function identity through `mixed` and nullable narrowing in
  semantic hover, including captured bindings and compiler-owned ownership
  diagnostics.
- Aligned `doria-lsp` with Stage 30g `List<T>::map`, `filter`, and `reduce`,
  including receiver-scoped completion, compiler-specialized semantic hovers,
  and compiler-owned algorithm diagnostics. Other collection families do not
  receive these algorithms.
- Aligned `doria-lsp` with Stage 30f PHP compatibility closure execution and the
  final Stage 30e compiler corrections. Valid closures are diagnostic-free in
  ordinary target-neutral editor analysis; semantic hovers identify guaranteed
  debug/native execution and describe PHP closure lowering without claiming that
  target-neutral analysis proved every surrounding operation PHP-compatible.
- Re-seed local-compiler runner lockfiles from the canonical workspace lock so
  installed-toolchain refreshes cannot reuse an incompatible generated lock.
- Aligned `doria-lsp` with Stage 30c ownership, lifetime, and escape analysis,
  including compiler-owned diagnostics and review-only fixes plus semantic
  closure ownership, capture acquisition, invocation, and escape hovers.
- Pinned `doria-lsp` to the constructor-rooted writable-path and owned-property
  correction, including compiler-owned diagnostics for readonly or uninitialized
  paths, borrowed move values, and overlapping transfers.
- Aligned `doria-lsp` with Stage 30b semantic function types, capture diagnostics
  and safe fixes, semantic callable hovers, and the narrowed execution-only
  `E0641` boundary while leaving closure execution unavailable.
- Completed Stage 30a editor alignment for structural function invocation modes,
  parameter ownership, checked effects, grouped nested types, and callable-value
  calls while preserving the compiler-owned `E0641` semantic boundary.
- Established the standalone Doria Language Server repository structure.
- Moved the VS Code and IntelliJ editor clients and shared syntax fixtures under one owner.
- Added cross-editor highlighting guardrails, CI, contribution guidance, architecture documentation, and release guidance.
- Migrated `doria-lsp` and its protocol tests from the compiler repository onto a commit-pinned `doriac` dependency.
- Added Linux, macOS, and Windows CI plus native release archives for x64 and arm64, with packaged VS Code and IntelliJ artifacts.
- Added functional IntelliJ formatting for language-specific indentation, tabs, spacing, braces, and preserved blank lines, with platform regression tests.
- Kept nested list and dictionary expressions inside attribute highlighting in both VS Code and JetBrains editors, with shared fixture and lexer regression coverage.
- Added one target-based build command that prints server, VS Code, and IntelliJ artifact paths and can install `doria-lsp` globally through Cargo.
- Bundled the native `doria-lsp` executable in platform-specific VS Code packages, with stale path overrides ignored when resolving the server.
- Added the canonical Doria logo as the `.doria` file icon in VS Code.
- Fixed VS Code delimiter Enter rules so indentation follows the editor's spaces, tabs, and tab-size settings instead of inserting a literal `\t`.
- Added VS Code Run and Debug launch profiles that default to `baton run` for
  projects, with explicit standalone-file execution through `doriac run`.
- Removed IntelliJ's token-only unused-variable guess so referenced class
  properties are no longer incorrectly dimmed.
- Made IntelliJ builds produce exactly one clearly named installable plugin ZIP,
  including builds invoked directly through Gradle.
