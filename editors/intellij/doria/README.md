# Doria IntelliJ Language Support

This directory contains first-pass Doria support for IntelliJ-based IDEs.

Plugin releases track the Doria toolchain CalVer. The target release is `2026.03.1-canary`.

It provides:

- `.doria` file recognition.
- Basic syntax highlighting for Doria keywords, variables, types, attributes, strings, string interpolation, comments, numbers, operators, punctuation, accepted OOP declaration vocabulary, namespace/import/include/directive vocabulary, and rejected strict-comparison/preprocessor spellings.
- Doria code-style settings and formatting for tabs, indentation, continuation indentation, spacing, braces, and preserved blank lines.
- PHPDoc-compatible documentation comments. Pressing Enter after `/*` continues a structured block; pressing Enter after `/**` asks `doria-lsp` to pre-fill declaration-aware `@template`, `@param`, `@return`, `@throws`, and `@var` tags. Doria parameter modifiers remain distinctly highlighted inside `@param` tags.
- Separate **New > Doria File** and **New > Doria Class** workflows. The class dialog can create a class, interface, trait, or enum and exposes class inheritance controls only for class templates. Namespace suggestions use the nearest `Baton.toml` `[autoload.namespaces]` and `[autoload-dev.namespaces]` roots. Without a matching mapping, the namespace remains editable for explicit entry. Moving a Doria file does not rewrite its namespace or references; automatic move retargeting waits for compiler-owned reference information.
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

Documentation-comment generation uses the running language server so its tags
stay aligned with the compiler's declaration grammar. The plugin's Enter handler
bridges the standard `textDocument/onTypeFormatting` request on the supported
2025.2 platform; newer platform-native on-type formatting can consume the same
server capability. Plain block comments are structured without inventing semantic
documentation.

Double-quoted interpolation uses the ordinary Doria expression grammar, so expressions such as `{left() + right()}` receive normal token scopes inside the string. Literal opening braces use `\{`; single-quoted strings remain non-interpolating.

The presentation lexer recognizes accepted Stage 30a callable syntax: `fn` arrow
closures, anonymous `function` blocks, explicit `with` capture lists, structural
function invocation modes, parameter ownership, checked effects, grouped nested
types, and callable-value calls. It recognizes `once` as an accepted keyword,
keeps parameter `writable` and `take` as modifiers, and leaves semantic authority
to the compiler. `doria-lsp` publishes compiler-owned closure ownership,
lifetime, escape diagnostics, review-only fixes, and semantic hovers, together
with Stage 30 List-algorithm diagnostics and semantic facts.
Valid closures are diagnostic-free in ordinary target-neutral editor analysis and
execute through the compiler's debug and native targets and its supported PHP
compatibility surface. Completion and concrete semantic hovers expose `map`,
`filter`, and `reduce` on `List<T>` only; diagnostics remain compiler-owned.
PHP remains secondary. Stages 30 through 33 and Phase F are complete. Native
Testing Foundation Slice 1 is complete and Slice 2 is next. `doria-lsp` consumes
compiler-owned behavioral declaration facts and Baton/compiler source scope to
publish declaration and description semantic tokens in development sources. The
plugin does not parse tests or infer them from paths. Expectations are not yet
available, and final testing completion, hover, and navigation wait for Slice 3.
Stage 34 single class inheritance remains blocked until the foundation is
complete. Attribute colors remain presentation-only, while
`doria-lsp` supplies compiler-owned schema completion, typed metadata hover,
semantic tokens, navigation, references, rename, and diagnostics. The plugin
does not parse attribute or testing semantics, provide runtime reflection, or
activate PHP exports. Top-level
`internal` declarations retain their type, function, enum, and constant
presentation with `internal` as a modifier. The presentation lexer does not
implement package visibility or other semantics.

Compiler-backed hover separates required source `throws` effects from the exact
ambient canonical I/O effects transported at runtime. Ambient I/O does not require
source declarations, including in source `finally` blocks; escaping finalizer
errors follow compiler-owned precedence rules. `E0632` is historical and reserved.
The presentation lexer performs no effect classification or finalizer analysis.

VS Code and IntelliJ / JetBrains highlighting should stay aligned. The shared smoke fixture is:

```text
editors/fixtures/latest-tokens.doria
```

After changing editor highlighting, run this from the repository root:

```bash
php scripts/check_editor_highlighting.php
```

Files under `editors/fixtures/` are syntax-highlighting smoke fixtures. The IntelliJ LSP adapter keeps them out of `doria-lsp` diagnostics so highlighting can be exercised independently of language-server diagnostics.

Doria uses distinct spellings for imports and trait composition: file/namespace-scope `use` imports individual, aliased, or grouped names from namespaces, while class-body or trait-body `uses` composes traits. The IntelliJ highlighter keeps these scopes separate as import use and trait-composition uses. It also presents namespace declarations and literal `include` directives; the compiler owns all resolution and diagnostics.

The intention menu offers `Use import for ...` through the bundled language
server for compiler-classified fully qualified names and unresolved short names
that match workspace declarations. Applying it shortens qualified occurrences
and maintains sorted `use` declarations: class-like imports first, followed by
alphabetized function and constant imports in a separate block. Existing aliases
are reused, ambiguous matches remain explicit choices, and alias collisions or
import blocks with interleaved comments are not rewritten.

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

The Doria settings page also provides one optional Baton path. It accepts
`$PROJECT_DIR$` and is passed to the language server as `DORIA_BATON_PATH`.
Otherwise `doria-lsp` resolves a version-matched sibling component or Baton on
`PATH`. The LSP client registers project-structure watchers with the IDE, and
all Baton discovery runs off the UI thread. The language server consumes Baton's
strict project JSON; the plugin does not parse manifests or locks.

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
