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

This includes checked-effect contracts and classification. Ordinary reusable
callables declare escaping required effects, while the selected program
entrypoint may infer its effective escaping set. Canonical I/O effects are
ambient: they keep checked runtime transport without creating a source
catch-or-declare obligation. Source finalizers may replace a pending nonfatal
outcome with a checked Error, which then flows to an outer context. `E0632` is
historical and reserved. The server consumes the compiler's required/ambient
profiles and does not reproduce that analysis or suppress its diagnostics.

### `doria-lsp`

The server owns LSP transport, document state, UTF-16/UTF-8 position conversion, protocol capability negotiation, and conversion of compiler results into LSP diagnostics, completion, hover, and code actions.

That conversion is loss-aware. The compiler's primary diagnostic label supplies
the LSP range; secondary labels are exported as related information with UTF-16
positions; explanations and Help remain readable diagnostic detail; stable
codes, severity, kind, development-only status, cause identity, documentation
links, and structured fixes are retained. Duplicate and cause grouping stay compiler-owned. Only
Machine Applicable fixes become automatic code actions, so clients never guess
at a semantic correction. Equivalent diagnostic titles and primary-label text
are collapsed at this boundary so every client receives one concise message.

Import actions also remain compiler-backed. The server uses canonical global
symbol identity and reference roles to shorten a qualified occurrence, reject
alias collisions, and organize existing imports into class-like and non-class
blocks. Editor clients only present and apply the returned LSP workspace edit.

The server may organize IDE-friendly data but must not create a second semantic checker.
Each source has one compiler-backed analysis snapshot, with open documents also
retaining their editor version. Snapshots contain
diagnostics, symbols, and resolved source occurrences. Diagnostics and semantic
features consume that shared snapshot so an individual hover request does not
re-parse or re-check the document.

Stage 31 uses one compiler `CompilationSession` for each stable synthetic
workspace package. The longest matching root wins; documents outside all roots
receive isolated standalone packages, and namespace text never determines
package identity. Currently open text is supplied through an in-memory partial
build plan with project-layout authority explicitly unavailable. The compiler
graph owns canonical identities, cross-file resolution, include and semantic
dependency edges, diagnostics, and incremental invalidation. The bounded index
only projects those facts into definition, references, conservative rename,
hover, completion, and source-aware fixes. Open, change, save, close, and
workspace-folder events reanalyze the relevant open-source graph and republish
affected URIs. This partial mode remains the bounded fallback when authoritative
project discovery is unavailable.

Stage 33 adds complete project authority without a second project parser. Baton
runs asynchronously with offline project-discovery arguments and returns strict
schema-1 JSON containing its source inventory and compiler tooling build plan.
The server overlays unsaved buffers and, for an aggregate workspace, derives one
package-rooted dependency closure per member from that supplied plan. Each closure
uses a reusable complete compiler graph session, so unrelated member entrypoints
and declarations remain isolated while real dependency navigation remains intact.
The server indexes supplied unopened sources. Generated sources
and Git cache sources are navigation-only; workspace and path sources may receive
safe compiler edits. Editor file watchers debounce structural refreshes. Baton is
never invoked by hover, completion, or another per-request path, and the server
does not parse `Baton.toml` or `Baton.lock`.

Stage 32 attribute tooling is another projection of that compiler graph. The
server indexes compiler-owned attribute schemas, canonical class references,
constructor-parameter identities, bound constant values, targets, and source
spans. It does not parse or bind attributes independently and does not evaluate
their expressions. Completion therefore lists only visible marked schemas and
their remaining constructor parameters; hover presents typed metadata; and
definition, references, and rename retain the same Stage 31 graph-safety rules.
Compiler-known markers have no invented source definition and cannot be renamed.
Attributes remain metadata only: there is no runtime reflection, `#[Test]` does
not execute tests, and `#[PHPExport]` does not activate a bridge.

Native Testing Foundation Slices 1 and 2 follow the same ownership boundary.
`doriac` recognizes compiler-known `Doria\Std\Test` declarations, validates
their development-source scope, and supplies typed suite/test facts with exact
source spans. The server retains those facts and the source semantic context
from the Baton/compiler project graph, then projects declaration and description
semantic tokens. It does not parse behavioral declarations, recognize them by
raw spelling, or infer source scope from paths. Compiler-owned expectation symbols
are projected only when the compiler reports the exact canonical identity.
Assertion roots, `not`, terminal matchers, diagnostics, and automatic
`TestAssertion` effects likewise come from compiler facts rather than an LSP
testing parser. User-defined same-name symbols remain ordinary. Collection/Error
matchers and final testing completion, hover, and navigation wait for Native
Testing Foundation Slice 3.

### Editor clients

Clients start and supervise `doria-lsp`, translate native editor APIs to LSP
where necessary, and provide local file registration and lightweight syntax
presentation. The VS Code client also maps project launch profiles onto
`baton run`; Baton remains responsible for manifest discovery, entry-point
selection, builds, and toolchain selection. Direct `doriac run` is reserved for
an explicit standalone-file profile. Client-specific fallback behavior must
remain presentation-only.

The official clients likewise do not implement testing semantics. They render
the server's compiler-backed semantic tokens and diagnostics; local TextMate and
IntelliJ tokenization remains presentation-only.

Released editor packages bundle the `doria-lsp` built from the same repository
revision. VS Code packages one native server per platform-specific VSIX; the
JetBrains plugin packages all supported native servers and selects by host OS and
architecture. Users never select a compiler: the compiler is an implementation
dependency embedded in the bundled server. Explicit paths and environment
overrides exist only for language-server and compiler development.

Both official clients expose one Baton path override. It is passed as
`DORIA_BATON_PATH`; otherwise the server uses the version-matched installed
component or `PATH`. Clients watch manifests, locks, Doria source creation and
deletion, generated output, and private inventory changes, and they expose an
explicit project refresh command without performing discovery on the UI thread.
After discovery, the server maintains bounded watchers rooted at the exact
package paths supplied by Baton; it never watches the global dependency cache as
one recursive root.

### Syntax highlighters

The TextMate grammar and IntelliJ lexer are deliberately local and fast. They classify tokens using syntactic context for visual presentation, including arbitrary function and method calls. They are not a substitute for semantic tokens or compiler diagnostics.

## Shared fixtures

`editors/fixtures/latest-tokens.doria` exercises accepted and planned presentation vocabulary, including namespace, individual/aliased/grouped imports, and include syntax. `editors/fixtures/stage32-attributes.doria` and `stage32-attributes-rejected.doria` preserve the accepted attribute/comment boundary and malformed or semantically rejected examples. `editors/fixtures/native-testing-slice2.doria` and `native-testing-slice2-rejected.doria` preserve the compiler-owned expectation surface and its rejected boundaries without adding editor-side semantics. `editors/fixtures/rejected-syntax.doria` ensures rejected PHP-shaped, preprocessor, wildcard-import, malformed-group, computed-include, and nested-directive syntax does not accidentally look accepted.

Both editor implementations must be checked against the same fixtures and token inventory.

## Compiler dependency

The standalone server depends on `doriac` at an exact Git commit recorded in `Cargo.toml` and `Cargo.lock`. Default compiler runtime bundling is disabled because the language server needs reusable frontend services, not native runtime artifacts.

Compiler updates are deliberate compatibility changes: update the pinned revision, run the complete server and editor validation, and confirm the advertised Doria toolchain version before release. The server source may adapt compiler diagnostics to LSP structures, but compiler-owned syntax and semantic behavior must stay in `doriac`.

See [semantic-hover.md](semantic-hover.md) for the hover payload, fallback behavior,
and the first semantic-navigation slice.
