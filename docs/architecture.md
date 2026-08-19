# Architecture

## Purpose

The Doria Language Server repository provides one IDE-facing layer for all supported editors while keeping language semantics in the compiler.

```text
VS Code client --------\
                        -> doria-lsp -> reusable doriac services
JetBrains client ------/                 lexer / parser / checker / diagnostics

TextMate grammar ------> syntax presentation only
JetBrains highlighter -> syntax presentation only

VS Code launch profile -> baton run -> project entry selected from Baton.toml
```

## Ownership boundaries

### `doriac`

The compiler owns tokenization, parsing, semantic and type checking, diagnostic codes and spans, machine-applicable fixes, and the truth about whether a language feature is implemented.

This includes checked-effect contracts. Ordinary reusable callables declare
their escaping effects, while the selected program entrypoint may infer its
effective escaping set. The server does not reproduce that analysis or suppress
its diagnostics.

### `doria-lsp`

The server owns LSP transport, document state, UTF-16/UTF-8 position conversion, protocol capability negotiation, and conversion of compiler results into LSP diagnostics, completion, hover, and code actions.

That conversion is loss-aware. The compiler's primary diagnostic label supplies
the LSP range; secondary labels are exported as related information with UTF-16
positions; explanations and Help remain readable diagnostic detail; stable
codes, severity, kind, development-only status, cause identity, documentation
links, and structured fixes are retained. Duplicate and cause grouping stay compiler-owned. Only
Machine Applicable fixes become automatic code actions, so clients never guess
at a semantic correction.

The server may organize IDE-friendly data but must not create a second semantic checker.
Each open document has one versioned compiler-backed analysis snapshot containing
diagnostics, symbols, and resolved source occurrences. Diagnostics and semantic
features consume that shared snapshot so an individual hover request does not
re-parse or re-check the document.

### Editor clients

Clients start and supervise `doria-lsp`, translate native editor APIs to LSP
where necessary, and provide local file registration and lightweight syntax
presentation. The VS Code client also maps project launch profiles onto
`baton run`; Baton remains responsible for manifest discovery, entry-point
selection, builds, and toolchain selection. Direct `doriac run` is reserved for
an explicit standalone-file profile. Client-specific fallback behavior must
remain presentation-only.

Released editor packages bundle the `doria-lsp` built from the same repository
revision. VS Code packages one native server per platform-specific VSIX; the
JetBrains plugin packages all supported native servers and selects by host OS and
architecture. Users never select a compiler: the compiler is an implementation
dependency embedded in the bundled server. Explicit paths and environment
overrides exist only for language-server and compiler development.

### Syntax highlighters

The TextMate grammar and IntelliJ lexer are deliberately local and fast. They classify tokens using syntactic context for visual presentation, including arbitrary function and method calls. They are not a substitute for semantic tokens or compiler diagnostics.

## Shared fixtures

`editors/fixtures/latest-tokens.doria` exercises accepted and planned presentation vocabulary. `editors/fixtures/rejected-syntax.doria` ensures rejected PHP-shaped or preprocessor syntax does not accidentally look accepted.

Both editor implementations must be checked against the same fixtures and token inventory.

## Compiler dependency

The standalone server depends on `doriac` at an exact Git commit recorded in `Cargo.toml` and `Cargo.lock`. Default compiler runtime bundling is disabled because the language server needs reusable frontend services, not native runtime artifacts.

Compiler updates are deliberate compatibility changes: update the pinned revision, run the complete server and editor validation, and confirm the advertised Doria toolchain version before release. The server source may adapt compiler diagnostics to LSP structures, but compiler-owned syntax and semantic behavior must stay in `doriac`.

See [semantic-hover.md](semantic-hover.md) for the hover payload, fallback behavior,
and the first semantic-navigation slice.
