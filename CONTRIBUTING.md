# Contributing

Thank you for helping improve Doria's IDE experience.

## Source of truth

The Doria compiler and its accepted specification and decisions define language behavior. This repository presents that behavior to editors; it must not invent a second parser, type system, diagnostic meaning, or feature-status authority.

## Before making a change

- Check whether the change belongs to the language server, both editor clients, or only one editor-specific adapter.
- For language syntax changes, confirm the corresponding compiler specification or accepted decision first.
- Keep VS Code and IntelliJ highlighting aligned unless a platform limitation is documented.
- Add or update a shared fixture under `editors/fixtures/` for syntax changes.
- Keep client code thin: diagnostics, completion meaning, hover meaning, and fixits should come from `doria-lsp` wherever the protocol supports them.

## Validation

Run the repository guardrails:

```bash
php scripts/check_editor_highlighting.php
npm --prefix editors/vscode/doria ci --ignore-scripts
npm --prefix editors/vscode/doria run check
```

Validate the standalone server:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
```

Build and test the JetBrains plugin:

```bash
cd editors/intellij/doria
./gradlew test buildPlugin
```

When changing the pinned `doriac` revision, run all Rust and editor checks and verify `doria-lsp --version` reports the intended canonical Doria toolchain release.

## Pull requests

Keep pull requests focused and include:

- the user-visible IDE behavior being changed;
- the Doria language version or decision that supports it;
- tests or fixtures covering the behavior;
- which editor clients were exercised;
- screenshots only when the visual result cannot be verified adequately by tests.

Release and version changes must update the VS Code manifest, IntelliJ plugin version, documentation, and changelog together.
