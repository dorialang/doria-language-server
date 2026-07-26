# Doria Language Support

This extension provides `.doria` language registration, TextMate syntax highlighting, attribute highlighting, editor bracket/comment behavior, and diagnostics from `doria-lsp`.

Editor releases track the Doria toolchain CalVer. The target toolchain release is `2026.03.1-canary`; the VS Code Marketplace manifest encodes it as the SemVer-compatible `2026.3.1-canary` because its required `version` field does not permit a zero-padded numeric month.

Syntax colors depend on the active VS Code theme. This extension improves Doria's TextMate scopes for cleaner highlighting, but it does not ship a custom color theme yet.

The extension uses the canonical Doria logo for `.doria` file icons.

New lines inside paired delimiters use VS Code's active indentation settings, including spaces versus tabs and the configured tab size.

Double-quoted interpolation uses the ordinary Doria expression grammar, so expressions such as `{left() + right()}` receive normal token scopes inside the string. Literal opening braces use `\{`; single-quoted strings remain non-interpolating.

## Install the VS Code extension

From the repository root, package the extension with the shared target-based build command:

```bash
php scripts/build.php vscode
code --install-extension dist/doria-language-support.vsix --force
```

Reload VS Code after installation. If the `code` shell command is unavailable, open the Extensions view, choose the `...` menu, select **Install from VSIX...**, and select `dist/doria-language-support.vsix`.

The platform-specific VSIX includes the matching optimized `doria-lsp` executable, so installing the extension enables language-server features without a separate server installation.

## Override the bundled language server

For language-server development, build a repository-local server and point the extension at it:

```bash
php scripts/build.php server
export DORIA_LSP_PATH="/absolute/path/to/doria-language-server/target/debug/doria-lsp"
```

The executable is under the repository-level `target/debug/` directory, not under `server/`. On Windows, use `target\debug\doria-lsp.exe`.

To install the server globally instead:

```bash
php scripts/build.php install-server
doria-lsp --version
```

If `doria-lsp` is on `PATH`, no environment variable is required. GUI-launched VS Code may not inherit shell environment changes until it is restarted. The most deterministic fallback is **Settings → Extensions → Doria → Language Server: Path**, or this workspace setting:

```json
{
  "doria.languageServer.path": "/absolute/path/to/doria-language-server/target/debug/doria-lsp"
}
```

Only existing configured and environment paths are used. Stale paths are ignored so they cannot prevent the bundled server from starting. The extension resolves the server from:

```text
1. doria.languageServer.path
2. DORIA_LSP_PATH
3. target/debug/doria-lsp in the open workspace
4. doria-lsp bundled in the installed extension
5. doria-lsp on PATH
```

The `vscode` build target compiles the optimized server for the host platform, copies it into the extension, and creates a platform-specific VSIX. It then runs the pinned npm install and packaging commands.

After changing the TextMate grammar, reload the VS Code window or restart the Extension Development Host so VS Code reads the updated grammar.

Keep this TextMate grammar aligned with the IntelliJ / JetBrains highlighter under `editors/intellij/doria`. From the repository root, run:

```bash
php scripts/check_editor_highlighting.php
```

Files under `editors/fixtures/` are shared syntax-highlighting smoke fixtures and are excluded from `doria-lsp` diagnostics.

Doria uses distinct spellings for imports and trait composition: file/namespace-scope `use` imports names from namespaces, while class-body or trait-body `uses` composes traits. The TextMate grammar keeps these scopes separate as import use and trait-composition uses.
