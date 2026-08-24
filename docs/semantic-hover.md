# Semantic hover

## User contract

`doria-lsp` provides the same hover behavior to VS Code and JetBrains clients through
the standard `textDocument/hover` request.

For a compiler-resolved declaration or reference, hover returns:

- the canonical Doria declaration signature;
- the declaration's immediately preceding PHPDoc comment, when present;
- the exact source range of the hovered identifier.

The first vertical slice covers same-file classes, free functions, instance methods,
and static methods. Method references are resolved from compiler-produced expression
types, including nullable receivers and `self`/`parent` static qualifiers. Hovering a
declaration uses the same symbol record as hovering one of its references.

Enum declarations use the same compiler-owned snapshot. Unit and backed cases show
their nominal enum result type, payload cases show their declared field signature,
payload fields show their resolved type, and the readonly `value` projection shows
the backing type. Enum and payload-case documentation identifies whether the enum is
a Copy or Move value without exposing private layout offsets. Static completion after
`EnumType::` lists only cases declared by that resolved enum. The server does not infer
cases from PascalCase spelling.

If a document has semantic errors, declarations that still parse remain available.
References are returned only when the compiler can resolve them safely. Existing
keyword, primitive type, and compiler-intrinsic hover remains available as a lexical
fallback.

The lexical fallback describes the accepted Stage 30a callable grammar. Stage 30c
semantic hovers use compiler metadata instead: structural function-typed bindings
and parameters show their canonical semantic type and callback ownership contract;
closure expressions show inferred invocation mode and checked effects together with
owned or borrow-bound provenance, capture acquisition, invocation consumption, and
escape classification. Capture occurrences show their compiler-resolved acquisition
mode. Callable-value calls show the checked function signature where the existing
expression-hover route can identify the call. Authored effect order remains visible
on source function types. The server does not rediscover free variables, infer
captures, or calculate lifetimes from text.

The compiler publishes precise capture and callable diagnostics with safe capture
fixes. Ordinary language-server analysis is target-neutral, so valid closures and
type-only function syntax do not receive `E0641`; invalid closures receive their
specific semantic diagnostic without a redundant execution boundary. Stage 30f
PHP compatibility closure execution is implemented. Semantic closure hovers
identify debug and native execution for a valid closure. They describe explicit
PHP closure lowering separately and condition it on the program's value families
and operations being supported by the PHP backend; target-neutral analysis does
not claim per-closure PHP compatibility. PHP remains a secondary compatibility
backend with independent limitations. Stage 30g List algorithms are next, so
Stage 30 remains incomplete. Semantically invalid closures receive no execution
capability block.

Ownership hover text stays in Doria vocabulary: Owned Closure or Borrow-Bound Closure,
Readonly/Writable/Owned taking capture, Readonly/Writable Repeatable or Consumes On
Invocation, Nonescaping or Owned callback, and returned closures tied to a parameter
or `$this`. Compiler-private binding IDs, closure coordinates, pass slots, future
environment layout, and backend symbols are never presented.

Constructor-rooted writable paths and owned property writes use the same compiler
authority. Accepted nested writes, owned initialization, and writable replacement
remain diagnostic-free; readonly intermediates, uninitialized intermediates,
borrowed right-hand sides, and overlapping transfers retain the compiler's exact
codes, labels, severity, kind, and metadata. The server does not infer path
capability or constructor state, and it does not claim support for moving values
out of properties.

Compiler-known ownership and collection methods use the same signature-first
presentation. When the compiler resolves the receiver, hover substitutes its concrete
generic arguments and exact return type; for example, `WeakReference<Theme>::acquire()`
returns `?SharedReference<Theme>`. In incomplete code, the lexical fallback still shows
the generic callable signature rather than documentation alone.

Compiler-known built-ins use signatures and checked-error effects exported by `doriac`.
The six Stage 29 I/O types are completed and hovered only by their canonical qualified
identities under `Doria\Std\Io`; short aliases such as `IoError` are neither guessed nor
offered. Because these declarations are compiler-known rather than source-backed, the
server provides their contract documentation without inventing a definition location.

Live diagnostics include uncovered checked effects such as `Doria\Std\Io\IoError` and
`Doria\Std\Io\InvalidUtf8Error`. Ordinary reusable callables declare those effects
explicitly. When the selected program entrypoint omits `throws`, the compiler infers
its exact escaping set; the server does not perform inference or suppress E0631.
`R1000` remains a runtime outcome for an error that escapes the entry point, so it is
not published as an editor diagnostic.

The server never guesses a symbol from spelling alone. Ambiguous or unresolved
identifiers return no semantic hover.

## Signature help

`textDocument/signatureHelp` uses the same compiler-resolved callable records as
hover. Free functions, instance methods, static methods, and constructors show
parameter and return types together with any declared `throws` entries in source
order. The server does not reconstruct or alphabetize the compiler's checked-error
effects. A clause-free `main` hover remains clause-free rather than synthesizing
the compiler's inferred effective set as source syntax.

## PHPDoc presentation

Doria documentation comments use PHPDoc syntax:

```doria
/**
 * Creates a greeting.
 *
 * @param string $name Person to greet.
 * @return string The greeting.
 */
function greet(string $name): string
{
    return "Hello, {$name}!";
}
```

Hover preserves the summary and body text and presents `@param`, `@return`, and
`@throws` tags in dedicated Markdown sections. Unknown tags remain visible instead
of being discarded.

## Ownership boundary

The compiler remains the semantic authority. The server builds an IDE-oriented
snapshot from `doriac` parser and semantic-analysis results, maps byte spans to LSP
positions, caches the snapshot by document version, and renders protocol Markdown.
It does not infer a parallel Doria type system.

Workspace indexing, imported declarations, inheritance across files, definition
navigation, references, and rename build on the same symbol/occurrence snapshot
rather than adding client-specific resolution rules.
