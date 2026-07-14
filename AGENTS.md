# Agent Guidance

## Repository role

This repository owns Doria's language server, syntax highlighting, shared editor fixtures, and IDE clients. The `doria` compiler repository remains the authority for language syntax, semantics, diagnostics, and staged implementation status.

## Guardrails

- Do not duplicate compiler parsing, semantic checking, or diagnostics in editor clients.
- Keep VS Code and IntelliJ / JetBrains syntax highlighting aligned.
- Treat TextMate and IntelliJ lexing as presentation only, never as compiler implementation.
- Planned keywords may be highlighted, but documentation and UI must not claim compiler support prematurely.
- Keep rejected syntax visibly rejected; do not highlight it as accepted Doria.
- Preserve `doria-lsp` as a thin protocol layer over reusable compiler services.
- Keep the VS Code and JetBrains clients thin and avoid client-specific language rules.
- Use the canonical logo under `res/images/` for editor branding.
- Track Doria's CalVer and retain any ecosystem-specific encoding explanation.
- Do not initialize Git, push, publish, sign, or create releases without explicit authorization.

## Required validation

For highlighting or editor-client changes, run:

```bash
php scripts/check_editor_highlighting.php
node --check editors/vscode/doria/extension.js
```

For IntelliJ plugin changes, also run:

```bash
cd editors/intellij/doria
./gradlew test buildPlugin
```

Once the Rust server source is migrated here, run formatting, Clippy with warnings denied, build, and tests for every server change.
