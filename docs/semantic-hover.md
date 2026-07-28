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

If a document has semantic errors, declarations that still parse remain available.
References are returned only when the compiler can resolve them safely. Existing
keyword, primitive type, and compiler-intrinsic hover remains available as a lexical
fallback.

The server never guesses a symbol from spelling alone. Ambiguous or unresolved
identifiers return no semantic hover.

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

Workspace indexing, imported declarations, inheritance across files, signature help,
definition navigation, references, and rename build on the same symbol/occurrence
snapshot rather than adding client-specific resolution rules.
