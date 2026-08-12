# Releasing

## Version policy

Language-server and editor releases track the compatible Doria toolchain CalVer, currently `2026.03.1-canary`.

Use the canonical zero-padded form in user-facing Doria version text. When a package ecosystem requires SemVer numeric components, use the equivalent unpadded form and retain the canonical toolchain version in separate metadata where supported.

## Release checklist

1. Confirm the supported Doria compiler version and protocol behavior.
2. Update `CHANGELOG.md`.
3. Update the VS Code extension version and `doriaToolchainVersion`.
4. Update the IntelliJ plugin version.
5. Run the cross-editor guardrails and JavaScript syntax check.
6. Build and test the IntelliJ plugin with the Gradle wrapper; verify its ZIP contains all six native `doria-lsp` binaries.
7. Run Rust formatting, Clippy with warnings denied, tests, and a locked release build.
8. Smoke-test diagnostics, completion, hover, fixits, function-call highlighting, and interpolation in both editor families.
9. Push a `v*` tag only after all manifests report compatible versions.
10. Confirm the GitHub release contains server archives and matching platform-specific VSIX packages for Linux, macOS, and Windows on x64 and arm64, plus the universal IntelliJ ZIP and `SHA256SUMS`.

Publishing Marketplace extensions, JetBrains plugins, binaries, tags, or GitHub releases is always an explicit maintainer action.

`workflow_dispatch` builds and retains every artifact without publishing a GitHub release. A pushed `v*` tag builds the same matrix and publishes those artifacts after every platform job succeeds.

The IntelliJ packaging job waits for all six server jobs, assembles their exact
binaries under `bin/<os>-<architecture>/`, and fails if any supported platform is
missing. This makes the plugin and compiler-backed server one versioned release
unit; Marketplace users do not install Rust, Cargo, `doriac`, or `doria-lsp`.

GitHub Actions wraps the retained `doria-intellij-plugin` artifact in its own
download ZIP. Extract that outer container to obtain the installable
`doria-intellij-plugin-<version>.zip`. Tagged GitHub releases attach the
installable plugin ZIP directly.
