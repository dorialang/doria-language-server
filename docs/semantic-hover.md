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

The lexical fallback describes the accepted Pre-Stage-30 closure grammar. `fn`
declares an explicitly typed arrow closure, `with` introduces an explicit capture
list, and contextual `function(T): R` is the accepted function-type spelling.
The pinned compiler still reports `E0641` for semantic use: capture checking,
callable compatibility, lowering, and execution land in Stage 30. The server does
not infer free variables, automatic capture, `$this` capture, closure effects, or
environment layout.

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
`Doria\Std\Io\InvalidUtf8Error`. `R1000` remains a runtime outcome for an error that
escapes the entry point, so it is not published as an editor diagnostic.

The server never guesses a symbol from spelling alone. Ambiguous or unresolved
identifiers return no semantic hover.

## Signature help

`textDocument/signatureHelp` uses the same compiler-resolved callable records as
hover. Free functions, instance methods, static methods, and constructors show
parameter and return types together with any declared `throws` entries in source
order. The server does not reconstruct or alphabetize the compiler's checked-error
effects.

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
