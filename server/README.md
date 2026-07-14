# Language-server migration target

This directory is reserved for the standalone `doria-lsp` implementation.

During the repository split, the active server remains in the Doria compiler repository at:

```text
crates/doriac/src/lsp.rs
crates/doriac/src/bin/doria-lsp.rs
```

The migration should move the LSP transport, document-state, position-mapping, completion, hover, diagnostics-adaptation, and code-action code here. Lexer, parser, semantic checker, type system, and diagnostic definitions remain compiler services and must not be copied into this repository.

Before deleting the compiler-owned binary, the standalone server must preserve its tests and successfully serve both editor clients from this repository.
