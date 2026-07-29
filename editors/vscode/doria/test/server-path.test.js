"use strict";

const assert = require("node:assert/strict");
const path = require("node:path");
const test = require("node:test");
const { resolveServerPath } = require("../server-path");

function resolver(existingPaths, overrides = {}) {
  const existing = new Set(existingPaths.map((candidate) => path.resolve(candidate)));
  return resolveServerPath({
    configuredPath: "",
    environmentPath: "",
    workspaceRoot: "/workspace",
    extensionPath: "/extension",
    cargoHome: "/cargo",
    homeDirectory: "/home/test",
    platform: "linux",
    pathExists: (candidate) => existing.has(path.resolve(candidate)),
    ...overrides
  });
}

test("uses an existing configured server first", () => {
  const result = resolver(
    ["/configured/doria-lsp", "/extension/bin/doria-lsp"],
    { configuredPath: "/configured/doria-lsp" }
  );

  assert.equal(result.command, path.resolve("/configured/doria-lsp"));
  assert.equal(result.source, "setting");
});

test("resolves a relative configured path from the workspace", () => {
  const result = resolver(
    ["/workspace/tools/doria-lsp"],
    { configuredPath: "tools/doria-lsp" }
  );

  assert.equal(result.command, path.resolve("/workspace/tools/doria-lsp"));
  assert.equal(result.source, "setting");
});

test("ignores stale overrides and uses the bundled server", () => {
  const result = resolver(
    ["/extension/bin/doria-lsp"],
    {
      configuredPath: "/old/configured/doria-lsp",
      environmentPath: "/old/server/target/debug/doria-lsp"
    }
  );

  assert.equal(result.command, path.resolve("/extension/bin/doria-lsp"));
  assert.equal(result.source, "bundled");
  assert.deepEqual(
    result.rejectedPaths.map(({ source }) => source),
    ["setting", "environment"]
  );
});

test("ignores mutable workspace binaries and prefers the bundled server", () => {
  const result = resolver([
    "/workspace/target/debug/doria-lsp",
    "/extension/bin/doria-lsp"
  ]);

  assert.equal(result.command, path.resolve("/extension/bin/doria-lsp"));
  assert.equal(result.source, "bundled");
});

test("uses the Cargo-installed server when no bundle is available", () => {
  const result = resolver(["/cargo/bin/doria-lsp"]);

  assert.equal(result.command, path.resolve("/cargo/bin/doria-lsp"));
  assert.equal(result.source, "Cargo install");
});

test("falls back to PATH when no server file is available", () => {
  const result = resolver([]);

  assert.equal(result.command, "doria-lsp");
  assert.equal(result.source, "PATH");
});

test("uses the Windows executable name", () => {
  const result = resolver(
    ["/extension/bin/doria-lsp.exe"],
    { platform: "win32" }
  );

  assert.equal(result.command, path.resolve("/extension/bin/doria-lsp.exe"));
  assert.equal(result.source, "bundled");
});
