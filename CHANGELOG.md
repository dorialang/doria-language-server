# Changelog

All notable changes to the Doria Language Server and official editor integrations are recorded here.

This project follows the Doria toolchain CalVer.

## Unreleased

- Established the standalone Doria Language Server repository structure.
- Moved the VS Code and IntelliJ editor clients and shared syntax fixtures under one owner.
- Added cross-editor highlighting guardrails, CI, contribution guidance, architecture documentation, and release guidance.
- Migrated `doria-lsp` and its protocol tests from the compiler repository onto a commit-pinned `doriac` dependency.
- Added Linux, macOS, and Windows CI plus native release archives for x64 and arm64, with packaged VS Code and IntelliJ artifacts.
- Added functional IntelliJ formatting for language-specific indentation, tabs, spacing, braces, and preserved blank lines, with platform regression tests.
