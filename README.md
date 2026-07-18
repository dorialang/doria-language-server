<div align="center">
    <img src="res/images/doria-app-icon-warm.svg" alt="Doria Logo" width="200" height="200" />
    <h1>Doria Language Server</h1>
    <p>Official editor tooling for the <a href="https://github.com/dorialang/doria">Doria programming language</a>.</p>
</div>

This repository is the home of `doria-lsp`, syntax highlighting, and IDE integrations for writing Doria programs. It keeps editor-specific presentation separate from the compiler while reusing `doriac` as the authority for parsing, type checking, and diagnostics.

## Repository status

This repository owns the standalone `doria-lsp` binary, editor clients, shared syntax fixtures, and their release artifacts. The server consumes a commit-pinned `doriac` library dependency; it does not duplicate the compiler's lexer, parser, semantic checker, or diagnostics.

Current editor support includes:

- VS Code language registration, TextMate highlighting, editor configuration, diagnostics, completion, hover, and fixits through `doria-lsp`.
- IntelliJ Platform support for RustRover, IntelliJ IDEA, PhpStorm, and compatible JetBrains IDEs, with local syntax highlighting and optional LSP integration.
- Shared accepted/planned and rejected-syntax fixtures used to keep both highlighters aligned.

Syntax highlighting is editor UX, not a language implementation. Planned vocabulary may be highlighted for documentation readability even when the current compiler correctly reports it as unsupported.

## Layout

```text
editors/
  fixtures/          Shared highlighting fixtures
  intellij/doria/    JetBrains plugin
  vscode/doria/      VS Code extension
res/images/          Canonical Doria artwork
scripts/             Repository guardrails
server/              Standalone language-server crate and tests
docs/                Architecture and release documentation
```

## Build and package artifacts

Use one command from the repository root instead of remembering each ecosystem's build invocation:

```bash
php scripts/build.php help
php scripts/build.php <target>
```

| Target | Result |
| --- | --- |
| `server` | Debug `doria-lsp` executable |
| `server-release` | Optimized `doria-lsp` executable |
| `install-server` | Install `doria-lsp` into Cargo's global bin directory |
| `vscode` | `dist/doria-language-support.vsix` |
| `intellij` | JetBrains plugin ZIP under `editors/intellij/doria/build/distributions/` |
| `editors` | Both editor packages |
| `all` | Debug server and both editor packages |

Every target prints the absolute path of each artifact it creates. PHP and Rust/Cargo are needed for the server target; the VS Code target additionally needs Node.js/npm, and the IntelliJ target needs Java 21.

## Build the language server step by step

1. Open a terminal at the repository root—the directory containing the top-level `Cargo.toml`.
2. Build the debug server:

   ```bash
   php scripts/build.php server
   ```

   The underlying Cargo command is `cargo build --locked --bin doria-lsp`.

3. Find the executable at:

   ```text
   Linux/macOS: target/debug/doria-lsp
   Windows:     target\debug\doria-lsp.exe
   ```

   `target/` is at the repository root, not inside `server/`. If `CARGO_TARGET_DIR` is configured, the wrapper reports the actual absolute output path.

4. Verify the local executable:

   ```bash
   ./target/debug/doria-lsp --version
   ```

   On Windows PowerShell:

   ```powershell
   .\target\debug\doria-lsp.exe --version
   ```

5. To make the server globally available, install it through Cargo:

   ```bash
   php scripts/build.php install-server
   doria-lsp --version
   ```

   Cargo normally installs it as `$HOME/.cargo/bin/doria-lsp` on Linux/macOS or `%USERPROFILE%\.cargo\bin\doria-lsp.exe` on Windows. Rustup normally adds that directory to `PATH`. If it did not, add the relevant directory to your shell or system `PATH`:

   ```bash
   export PATH="$HOME/.cargo/bin:$PATH"
   doria-lsp --version
   ```

   Add that `export` line to your shell startup file to keep it across terminals. On Windows, add `%USERPROFILE%\.cargo\bin` through **System Properties → Environment Variables → Path**. Restart the IDE after changing `PATH`.

For GUI-launched IDEs that do not inherit your shell environment, set the editor's explicit Doria language-server path or set `DORIA_LSP_PATH` to the absolute executable path. In VS Code this is the `doria.languageServer.path` setting; in JetBrains IDEs use **Settings → Languages & Frameworks → Doria → Language server path**. An explicit path is the most deterministic development setup.

CI builds and tests the server on Linux, macOS, and Windows. GitHub release workflows build native archives for all three operating systems on x64 and arm64, and package the VS Code extension and IntelliJ Platform plugin.

Both editor clients resolve the server in this order:

1. The editor's explicit Doria language-server setting.
2. `DORIA_LSP_PATH`.
3. `target/debug/doria-lsp` in the open project.
4. `doria-lsp` on `PATH`.

## Development

Run the cross-editor consistency checks from this repository root:

```bash
php scripts/check_editor_highlighting.php
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
npm --prefix editors/vscode/doria ci --ignore-scripts
npm --prefix editors/vscode/doria run check
```

Build the IntelliJ plugin with its checked-in Gradle wrapper:

```bash
cd editors/intellij/doria
./gradlew test buildPlugin
```

The packaged plugin is written to `editors/intellij/doria/build/distributions/`.

See [CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow, [docs/architecture.md](docs/architecture.md) for component boundaries, and [docs/releasing.md](docs/releasing.md) for CalVer release coordination.

## Versioning

Language-server and editor releases track the Doria toolchain CalVer. The current target is `2026.03.1-canary`. Ecosystems that require SemVer-compatible numeric components encode the same release without zero padding, for example `2026.3.1-canary` in the VS Code manifest.

## License

MIT. See [LICENSE](LICENSE).
