# Changelog

All notable changes to the Doria Language Server and official editor integrations are recorded here.

This project follows the Doria toolchain CalVer.

## Unreleased

- Aligned the server and both editor highlighters with accepted Pre-Stage-30
  closure grammar, preserving the compiler-owned `E0641` semantic boundary.
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
