# Doria IntelliJ Language Support

This directory contains first-pass Doria support for IntelliJ-based IDEs.

Plugin releases track the Doria toolchain CalVer. The target release is `2026.03.1-canary`.

It provides:

- `.doria` file recognition.
- Basic syntax highlighting for Doria keywords, variables, types, attributes, strings, string interpolation, comments, numbers, operators, punctuation, accepted OOP declaration vocabulary, namespace/import/include/directive vocabulary, and rejected strict-comparison/preprocessor spellings.
- Doria code-style settings and formatting for tabs, indentation, continuation indentation, spacing, braces, and preserved blank lines.
- A Doria settings page for configuring the language server path.
- `doria-lsp` integration through the IntelliJ Platform LSP API.

The initial plugin targets IntelliJ Platform `2025.2.1+`, where JetBrains exposes the LSP module as `com.intellij.modules.lsp`.

This is first-pass Doria support for IntelliJ / JetBrains IDEs. Local syntax highlighting and formatting do not provide semantic inspections or refactors. Compiler-backed diagnostics, completion, and hover remain separate and come from `doria-lsp` when the language server is configured and available.

The plugin registers the lower-case `doria` language id so Markdown fenced blocks using the `doria` info string can resolve to the Doria highlighter where the JetBrains Markdown plugin performs language injection. Planned keywords are highlighted for documentation readability only; compiler support still follows the staged plan.

## Build the language server

From the root of this repository:

```bash
php scripts/build.php server
```

Point the plugin setting or `DORIA_LSP_PATH` at an explicit development build when needed. Normally, install the compiled server into Cargo's bin directory with `php scripts/build.php install-server`; the plugin resolves that platform-neutral location before falling back to `PATH`.

## Build the plugin

From the repository root:

```bash
php scripts/build.php intellij
```

The target uses the checked-in Gradle wrapper and prints the absolute path of the generated ZIP. The equivalent low-level commands are:

```bash
cd editors/intellij/doria
./gradlew buildPlugin
```

On Windows, use `.\gradlew.bat buildPlugin`. Use the checked-in wrapper instead of a system Gradle installation so local builds and CI use the same pinned Gradle distribution.

The packaged plugin will be written under:

```text
build/distributions/doria-intellij-plugin-<version>.zip
```

That is the only local build artifact to select in **Install Plugin from
Disk**. Files under `build/libs/` are Gradle intermediates, not installable
plugin packages. Every `buildPlugin` invocation removes obsolete distribution
ZIPs first, so `build/distributions/` contains exactly one current plugin ZIP.

GitHub Actions always wraps retained artifacts in a download ZIP. After
downloading the `doria-intellij-plugin` Actions artifact, extract that outer
container once and install the versioned plugin ZIP inside it. A plugin ZIP
attached directly to a GitHub release can be installed without that extraction.

## Enable in RustRover or another JetBrains IDE

Install the packaged plugin from disk:

```text
Settings/Preferences -> Plugins -> gear icon -> Install Plugin from Disk...
```

Select the ZIP from `build/distributions/`, then restart the IDE when prompted. After restart, `.doria` files should be associated with the Doria file type and use the Doria syntax highlighter automatically.

If a `.doria` file still opens without highlighting, check:

```text
Settings/Preferences -> Editor -> File Types
```

Make sure `*.doria` is listed under `Doria`, and remove it from `Text` or `Plain Text` if the IDE previously learned that association.

The syntax highlighter, file type registration, comments, and settings page only require the IntelliJ Platform module. `doria-lsp` integration is enabled when the IDE also provides JetBrains' LSP module.

Double-quoted interpolation uses the ordinary Doria expression grammar, so expressions such as `{left() + right()}` receive normal token scopes inside the string. Literal opening braces use `\{`; single-quoted strings remain non-interpolating.

VS Code and IntelliJ / JetBrains highlighting should stay aligned. The shared smoke fixture is:

```text
editors/fixtures/latest-tokens.doria
```

After changing editor highlighting, run this from the repository root:

```bash
php scripts/check_editor_highlighting.php
```

Files under `editors/fixtures/` are syntax-highlighting smoke fixtures. The IntelliJ LSP adapter keeps them out of `doria-lsp` diagnostics so highlighting can be exercised independently of language-server diagnostics.

Doria uses distinct spellings for imports and trait composition: file/namespace-scope `use` imports names from namespaces, while class-body or trait-body `uses` composes traits. The IntelliJ highlighter keeps these scopes separate as import use and trait-composition uses.

## Run in a sandbox IDE

```bash
./gradlew runIde
```

On Windows PowerShell or Command Prompt:

```powershell
.\gradlew.bat runIde
```

## Language server path resolution

The plugin looks for `doria-lsp` in this order:

```text
1. Doria settings: Language server path
2. DORIA_LSP_PATH environment variable
3. The version-matched native server bundled with the plugin
4. Cargo's installed bin directory (development fallback)
5. doria-lsp on PATH (development fallback)
```

On Windows, the executable name is `doria-lsp.exe`.

Normal plugin installations use the bundled server automatically. The settings,
environment, Cargo, and PATH forms are retained for contributors testing a local
compiler or language-server build; end users do not configure a compiler or
language-server path.

The settings path also accepts `$PROJECT_DIR$`, for example:

```text
$PROJECT_DIR$/target/debug/doria-lsp
```

## Notes

This plugin intentionally reuses the existing `doria-lsp` binary instead of duplicating compiler diagnostics in IntelliJ. Syntax highlighting is local and lightweight; diagnostics, completion, and hover come from the language server.
