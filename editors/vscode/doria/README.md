# Doria Language Support

This extension provides `.doria` language registration, TextMate syntax highlighting, attribute highlighting, editor bracket/comment behavior, and diagnostics from `doria-lsp`.

Editor releases track the Doria toolchain CalVer. The target toolchain release is `2026.03.1-canary`; the VS Code Marketplace manifest encodes it as the SemVer-compatible `2026.3.1-canary` because its required `version` field does not permit a zero-padded numeric month.

Syntax colors depend on the active VS Code theme. This extension improves Doria's TextMate scopes for cleaner highlighting, but it does not ship a custom color theme yet.

The extension uses the canonical Doria logo for `.doria` file icons and
contributes a Doria launch profile to VS Code's Run and Debug view.

New lines inside paired delimiters use VS Code's active indentation settings, including spaces versus tabs and the configured tab size.

Double-quoted interpolation uses the ordinary Doria expression grammar, so expressions such as `{left() + right()}` receive normal token scopes inside the string. Literal opening braces use `\{`; single-quoted strings remain non-interpolating.

The TextMate grammar recognizes accepted Stage 30a callable syntax: `fn` arrow
closures, anonymous `function` blocks, explicit `with` capture lists, structural
function invocation modes, parameter ownership, checked effects, grouped nested
types, and callable-value calls. `once` is an accepted invocation modifier;
`writable` and `take` retain their context-sensitive modifier scopes. This is
presentation only; `doria-lsp` publishes the compiler's structured `E0641`
execution boundary together with Stage 30b semantic diagnostics and safe capture
fixes. The grammar does not implement capture or callable semantics, and closure
execution remains unavailable.

## Install the VS Code extension

From the repository root, package the extension with the shared target-based build command:

```bash
php scripts/build.php vscode
code --install-extension dist/doria-language-support.vsix --force
```

Reload VS Code after installation. If the `code` shell command is unavailable, open the Extensions view, choose the `...` menu, select **Install from VSIX...**, and select `dist/doria-language-support.vsix`.

The platform-specific VSIX includes the matching optimized `doria-lsp` executable, so installing the extension enables language-server features without a separate server installation.

## Run a Doria project

Open a project containing `Baton.toml`, open any project file, and use **Run and
Debug** or press `F5`. The generated **Run Doria project** profile finds the
manifest from the active file or workspace and runs:

```bash
baton run
```

Unsaved Doria project files are saved before launch. Program output and input use
VS Code's integrated terminal. Set `release` to `true` for `baton run --release`;
values in `args` are forwarded after `--`.

No `.vscode/launch.json` is required. To keep an explicit profile:

```json
{
  "version": "0.2.0",
  "configurations": [
    {
      "type": "doria",
      "request": "launch",
      "name": "Run Doria project",
      "mode": "project",
      "cwd": "${workspaceFolder}",
      "args": [],
      "release": false,
      "noDebug": true
    }
  ]
}
```

Baton is resolved from `doria.baton.path`, then `BATON_PATH`, then `PATH`.
Standalone source files can use the **Doria: Run standalone file** snippet,
which runs the selected `program` through `doriac run`; that opt-in mode resolves
the compiler from `doria.compiler.path`, `DORIAC_PATH`, Cargo's installed bin
directory, then `PATH`.

These are execution profiles. Source-level breakpoints and stepping will remain
disabled until the Doria toolchain exposes a debugger protocol.

## Override the bundled language server

For language-server development, build a repository-local server and point the extension at it:

```bash
php scripts/build.php server
export DORIA_LSP_PATH="/absolute/path/to/doria-language-server/target/debug/doria-lsp"
```

The executable is under the repository-level `target/debug/` directory, not under `server/`. Use this mutable build only through an explicit setting or `DORIA_LSP_PATH`; on Windows, its name is `doria-lsp.exe`.

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
3. doria-lsp bundled in the installed extension
4. Cargo's installed bin directory
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
