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

## Server extraction

The current server implementation remains in the compiler repository during the split. The extraction should move only the protocol layer and its tests. Compiler services stay in `doriac` and are consumed through a deliberate library dependency or stable compiler-service boundary.

The coordinated compiler change should remove the old `doria-lsp` binary only after this repository builds, tests, and packages its replacement.
