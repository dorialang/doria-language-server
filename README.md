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

## Build the language server

Build the standalone server from this repository:

```bash
cargo build --locked --bin doria-lsp
```

The executable is written to `target/debug/doria-lsp` (`target\debug\doria-lsp.exe` on Windows). Set `DORIA_LSP_PATH` to that file, install a release archive's executable on `PATH`, or choose it in the editor's Doria language-server setting.

Confirm the server and compatible compiler versions with:

```bash
doria-lsp --version
```

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
