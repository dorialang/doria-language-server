# Doria language server

This crate builds the standalone `doria-lsp` executable. It owns LSP transport, document state, UTF-16 position mapping, compiler-diagnostic adaptation, completion, hover, and code actions.

Language behavior comes from the exact `doriac` revision pinned by the workspace manifest and lockfile. Do not copy compiler parsing or semantic rules into this crate.

The pinned compiler supplies complete Stage 30 List algorithm facts plus function-value
ownership, lifetime, escape, diagnostic, fix, and hover facts. The server only
adapts those structured facts to LSP. On resolved `List<T>` receivers it offers
`map`, `filter`, and `reduce` completion and renders concrete compiler-specialized
call hovers. Other collection families do not receive those algorithms.
Flow-narrowed function values retain the compiler's exact structural identity
through `mixed` and nullable storage; the server displays that identity without
reconstructing tags, effects, invocation modes, or ownership from source text.
Ordinary editor analysis is target-neutral: it identifies guaranteed debug/native
execution for valid closures and describes PHP lowering as conditional on the
program's independently supported PHP surface. Valid closures receive no
`E0641` because the compiler has completed their accepted routes; the server does
not suppress that historical, reserved diagnostic. PHP remains a secondary
compatibility backend with independent limitations. Stage 30 is complete. Stage
31 is complete. Stage 32 is complete. Attribute completion, hover, navigation, references, rename, semantic
tokens, and diagnostics consume compiler-owned schemas, canonical identities,
bound constant values, and source spans. The server does not execute attribute
constructors, provide runtime reflection, run `#[Test]`, or activate
`#[PHPExport]`. Stage 33, Phase F, and the Native Testing Foundation are complete.
For development sources, the server
retains compiler-owned behavioral suite/test facts and Baton/compiler
source-scope context, then projects exact declaration and description semantic
tokens, typed completion, testing hovers, authored symbols, and safe navigation.
It also projects compiler-owned expectation roots, `fail`, `AssertionError`,
`not`, collection/Error matchers, diagnostics, and automatic `TestAssertion`
effect facts. It has no testing parser, matcher type checker, path-based test
inference, fake Test definitions, or generated-callable symbols. Stage 34 single
class inheritance is complete. The server consumes compiler-owned class
hierarchy, inherited-member, virtual-family, direct-parent, and callable-contract
facts for completion, hover, navigation, references, rename, and semantic tokens.
It does not parse or check inheritance independently, and it preserves incomplete,
generated, and dependency-source edit safety. Stage 35 interfaces and traits are
next.

The post-Stage-34 constructor-parameter-role correction is implemented through
compiler-owned constructor-role and property-family facts. Hover, signature and
named-argument completion, member completion, navigation, references, and rename
therefore distinguish promoted properties, inherited-property `override`
parameters, and constructor-only `parameter` inputs without a second promotion
or inheritance checker. Property-family rename is refused when the complete
atomic edit cannot be proven.

The post-Stage-34 indexed-foreach and scalar-display correction is implemented
through compiler-owned foreach semantic facts. Hover distinguishes readonly
Zero-Based Sequence Index bindings from Dictionary Key bindings and preserves
the value binding's resolved access; semantic tokens remain ordinary variables.
Decision 0133 requires an explicit type on every binding. Omitted types remain
compiler diagnostics with Machine Applicable insertion fixes; they are never
accepted as inferred declarations. Diagnostics and fixes are forwarded from the
compiler, with no second foreach checker. Scalar interpolation and `%s` add no primitive `toString` or scalar-cast
completion. Stage 35 remains next, and property hooks remain future work.

The compiler classifies checked effects into source-required effects and ambient
canonical I/O effects. Hovers keep source signatures focused on required
`throws` contracts and describe the exact ambient
`Doria\Std\Io\IoError`/`Doria\Std\Io\InvalidUtf8Error` runtime profile
separately. Ambient I/O needs no source declaration, including inside source
`finally` blocks. A checked error escaping a finalizer propagates under the
compiler's finalizer-precedence rules; `E0632` is historical and reserved. The
server consumes compiler-owned effect profiles and does not classify effects by
source spelling.

For project-aware tooling, Baton is resolved from one explicit editor override,
`DORIA_BATON_PATH`, a sibling installed component, or `PATH`. The server runs
`baton project --json --workspace --development --offline` asynchronously and
retries package selection only for Baton's exact workspace-selection diagnostic.
Its strict schema-1 project document supplies the complete compiler build plan,
source inventory, generated provenance, and edit policy. Aggregate workspaces use
one compiler session per member dependency closure rather than one flattened
semantic program. Open buffers overlay
matching sources without writing to disk; unopened sources participate in
navigation and diagnostics. Generated and Git-cache sources remain readonly.
Project watcher registrations are rooted at the exact package paths supplied by
Baton rather than at the global dependency cache.
Discovery failure falls back to the existing compiler-owned partial graph for
open documents. The server never parses Baton manifests or locks.

From the repository root:

```bash
php scripts/build.php server
./target/debug/doria-lsp --version
```

The wrapper runs `cargo build --locked --bin doria-lsp` and prints the executable's absolute path. The default output is `target/debug/doria-lsp` (`target\debug\doria-lsp.exe` on Windows), relative to the repository root rather than this crate directory.

Install the server into Cargo's global bin directory with:

```bash
php scripts/build.php install-server
doria-lsp --version
```

Run the server checks directly with Cargo:

```bash
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Without arguments, `doria-lsp` serves LSP over stdin/stdout. Use `doria-lsp --version` to inspect the server package and compatible canonical Doria toolchain versions.
