# Doria language server

This crate builds the standalone `doria-lsp` executable. It owns LSP transport, document state, UTF-16 position mapping, compiler-diagnostic adaptation, completion, hover, and code actions.

Language behavior comes from the exact `doriac` revision pinned by the workspace manifest and lockfile. Do not copy compiler parsing or semantic rules into this crate.

The pinned compiler supplies Stage 30g List algorithm facts plus function-value
ownership, lifetime, escape, diagnostic, fix, and hover facts. The server only
adapts those structured facts to LSP. On resolved `List<T>` receivers it offers
`map`, `filter`, and `reduce` completion and renders concrete compiler-specialized
call hovers. Other collection families do not receive those algorithms.
Ordinary editor analysis is target-neutral: it identifies guaranteed debug/native
execution for valid closures and describes PHP lowering as conditional on the
program's independently supported PHP surface. It does not publish `E0641` for
valid closures. PHP remains a secondary compatibility backend with independent
limitations; Stage 30h is next, and Stage 30 remains incomplete.

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
