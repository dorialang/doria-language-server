# Architecture

## Purpose

The Doria Language Server repository provides one IDE-facing layer for all supported editors while keeping language semantics in the compiler.

```text
VS Code client --------\
                        -> doria-lsp -> reusable doriac services
JetBrains client ------/                 lexer / parser / checker / diagnostics

TextMate grammar ------> syntax presentation only
JetBrains highlighter -> syntax presentation only
```

## Ownership boundaries

### `doriac`

The compiler owns tokenization, parsing, semantic and type checking, diagnostic codes and spans, machine-applicable fixes, and the truth about whether a language feature is implemented.

### `doria-lsp`

The server owns LSP transport, document state, UTF-16/UTF-8 position conversion, protocol capability negotiation, and conversion of compiler results into LSP diagnostics, completion, hover, and code actions.

The server may organize IDE-friendly data but must not create a second semantic checker.

### Editor clients

Clients start and supervise `doria-lsp`, translate native editor APIs to LSP where necessary, and provide local file registration and lightweight syntax presentation. Client-specific fallback behavior must remain presentation-only.

### Syntax highlighters

The TextMate grammar and IntelliJ lexer are deliberately local and fast. They classify tokens using syntactic context for visual presentation, including arbitrary function and method calls. They are not a substitute for semantic tokens or compiler diagnostics.

## Shared fixtures

`editors/fixtures/latest-tokens.doria` exercises accepted and planned presentation vocabulary. `editors/fixtures/rejected-syntax.doria` ensures rejected PHP-shaped or preprocessor syntax does not accidentally look accepted.

Both editor implementations must be checked against the same fixtures and token inventory.

## Compiler dependency

The standalone server depends on `doriac` at an exact Git commit recorded in `Cargo.toml` and `Cargo.lock`. Default compiler runtime bundling is disabled because the language server needs reusable frontend services, not native runtime artifacts.

Compiler updates are deliberate compatibility changes: update the pinned revision, run the complete server and editor validation, and confirm the advertised Doria toolchain version before release. The server source may adapt compiler diagnostics to LSP structures, but compiler-owned syntax and semantic behavior must stay in `doriac`.
