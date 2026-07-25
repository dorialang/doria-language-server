# Doria Zed Language Support

Language support for [Doria](https://github.com/dorialang/doria) in the Zed editor.

Extension releases track the Doria toolchain CalVer. The target release is
`2026.3.1-canary`.

## What it provides

- `.doria` file recognition (the **Doria** language).
- Editor language configuration: `//` and `/* */` comments, bracket matching and
  auto-close, and `$`-aware word boundaries for Doria's sigil variables.
- The **Doria Language Server** (`doria-lsp`): diagnostics, hover, completion, and
  everything else the server advertises.

## Requirements

The extension launches `doria-lsp`; it does not bundle it. Provide it one of two
ways:

1. **On `PATH`** — build and install the server from the `doria-language-server`
   repository:
   ```bash
   php scripts/build.php install-server
   # or: cargo install --path server
   ```
2. **A configured path** — set it in your Zed `settings.json`:
   ```json
   {
     "lsp": {
       "doria": { "binary": { "path": "/absolute/path/to/doria-lsp" } }
     }
   }
   ```

The extension prefers the configured path, then falls back to `doria-lsp` on
`PATH`.

## Status: syntax highlighting is pending a tree-sitter grammar

Zed **requires a tree-sitter grammar** to register a language, and the Doria
grammar (`tree-sitter-doria`) does not exist yet. Until it is authored, published,
and pinned by commit in `extension.toml` under `[grammars.doria]`, **this
extension will not load in Zed**. Everything else — the LSP wiring, the language
configuration, and the manifest — is complete and compiles.

Once the grammar repository exists, set its `repository` and `rev` in
`extension.toml` and add tree-sitter query files (`highlights.scm`, etc.) under
`src/languages/doria/`.

## Building and installing (dev)

```bash
# Compile the extension to WebAssembly:
cargo build --release --target wasm32-wasip1

# Then in Zed: Extensions -> Install Dev Extension -> select this directory
# (editors/zed/doria). Zed also builds the wasm itself on install.
```
