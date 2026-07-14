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
6. Build and test the IntelliJ plugin with the Gradle wrapper.
7. Once the server lives here, run its complete Rust validation and build release binaries for supported platforms.
8. Smoke-test diagnostics, completion, hover, fixits, function-call highlighting, and interpolation in both editor families.
9. Tag and publish only after all artifacts report compatible versions.

Publishing Marketplace extensions, JetBrains plugins, binaries, tags, or GitHub releases is always an explicit maintainer action.
