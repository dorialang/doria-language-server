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
compatibility backend with independent limitations. Stage 30 and Stage 31 are
complete. Stage 32 attributes are next; Stage 33 project integration remains
scheduled and unimplemented.

For namespace-aware tooling, each longest-matching workspace root receives a
stable synthetic package and one compiler `CompilationSession`. Open documents
become an in-memory partial build plan and are checked as one compiler graph.
Cross-file definitions, references, conservative rename, rich hover, completion,
multi-source diagnostics, fixes, include edges, and incremental invalidation all
consume compiler-owned identities and facts. The server does not scan files,
read Baton manifests, or claim completeness for unopened sources; Stage 33 owns
that project inventory integration.

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
