# Agent Guidance

## Repository role

This repository owns Doria's language server, syntax highlighting, shared editor fixtures, and IDE clients. The `doria` compiler repository remains the authority for language syntax, semantics, diagnostics, and staged implementation status.

**This repository restates language facts, so it goes stale whenever a compiler stage lands.** Hover text, descriptions, highlighting, and fixtures all encode claims the compiler owns. A stale language server tells the user that valid code is wrong — worse than shipping no language server at all. Two obligations follow:

- The `doriac` pin in the root `Cargo.toml` (`rev = "…"`) is part of the language surface, not just a build detail. A pin behind the compiler's landed work means hovers and diagnostics describe a language that no longer exists.
- Any claim scoped to a superseded stage is a defect to fix on sight, whether or not the current task caused it: "this compiler currently supports…", "represents the EOF result of…", or "alias" for a type that is now implemented with a real member surface. When compiler work lands a stage, `doria/AGENTS.md` ("Language-server sweep") requires that beat to update this repository too — the sweep is not optional follow-up.

## Guardrails

- Do not duplicate compiler parsing, semantic checking, or diagnostics in editor clients.
- Preserve the compiler's structured diagnostic model. Use its primary label for
  the LSP range, secondary labels for related information, explicit severity,
  and compiler-owned cause grouping. Do not regroup or suppress findings by
  matching prose.
- Publish automatic quick fixes only for compiler fixes marked Machine
  Applicable. Requires Review and Informational fixes may remain diagnostic
  detail but are not automatic code actions.
- Keep VS Code and IntelliJ / JetBrains syntax highlighting aligned.
- Treat TextMate and IntelliJ lexing as presentation only, never as compiler implementation.
- Planned keywords may be highlighted, but documentation and UI must not claim compiler support prematurely.
- Keep rejected syntax visibly rejected; do not highlight it as accepted Doria.
- Preserve `doria-lsp` as a thin protocol layer over reusable compiler services.
- Keep the VS Code and JetBrains clients thin and avoid client-specific language rules.
- Use the canonical logo under `res/images/` for editor branding.
- Track Doria's CalVer and retain any ecosystem-specific encoding explanation.
- Do not initialize Git, push, publish, sign, or create releases without explicit authorization.

## Build artifact storage

- Cargo does not garbage-collect old project artifacts. Run `php scripts/check_cargo_target_size.php` before and after the full Rust validation suite.
- The checker reports a problem when the repository's `target/` exceeds 15 GiB. It is diagnostic only and must never delete build artifacts.
- Never run `cargo clean` or remove `target/` automatically. Report the measured size and ask Andrew for approval before cleaning.
- Keep test debug information at line-table level and test incremental compilation disabled unless Andrew explicitly accepts the storage tradeoff.

## Required validation

For highlighting or editor-client changes, run:

```bash
php scripts/check_editor_highlighting.php
npm --prefix editors/vscode/doria ci --ignore-scripts
npm --prefix editors/vscode/doria run check
```

For IntelliJ plugin changes, also run:

```bash
cd editors/intellij/doria
./gradlew test buildPlugin
```

For server or compiler-dependency changes, run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
```
