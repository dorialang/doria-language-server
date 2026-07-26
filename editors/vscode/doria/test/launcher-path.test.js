"use strict";

const assert = require("node:assert/strict");
const path = require("node:path");
const test = require("node:test");
const {
  resolveBatonPath,
  resolveCompilerPath
} = require("../launcher-path");

function resolver(existingPaths, overrides = {}) {
  const existing = new Set(existingPaths.map((candidate) => path.resolve(candidate)));
  return resolveCompilerPath({
    configuredPath: "",
    environmentPath: "",
    workspaceRoot: "/workspace",
    platform: "linux",
    pathExists: (candidate) => existing.has(path.resolve(candidate)),
    ...overrides
  });
}

test("uses an existing configured compiler first", () => {
  const result = resolver(
    ["/configured/doriac", "/workspace/target/debug/doriac"],
    { configuredPath: "/configured/doriac" }
  );

  assert.equal(result.command, path.resolve("/configured/doriac"));
  assert.equal(result.source, "setting");
});

test("resolves a relative compiler path from the workspace", () => {
  const result = resolver(
    ["/workspace/tools/doriac"],
    { configuredPath: "tools/doriac" }
  );

  assert.equal(result.command, path.resolve("/workspace/tools/doriac"));
});

test("ignores stale overrides and uses a workspace compiler", () => {
  const result = resolver(
    ["/workspace/target/debug/doriac"],
    {
      configuredPath: "/old/doriac",
      environmentPath: "/old/environment/doriac"
    }
  );

  assert.equal(result.command, path.resolve("/workspace/target/debug/doriac"));
  assert.equal(result.source, "workspace");
  assert.deepEqual(
    result.rejectedPaths.map(({ source }) => source),
    ["setting", "environment"]
  );
});

test("falls back to doriac on PATH", () => {
  const result = resolver([]);

  assert.equal(result.command, "doriac");
  assert.equal(result.source, "PATH");
});

test("uses the Windows compiler executable name", () => {
  const result = resolver([], { platform: "win32" });

  assert.equal(result.command, "doriac.exe");
});

test("resolves Baton from its setting, environment, then PATH", () => {
  const configured = resolveBatonPath({
    configuredPath: "/toolchain/baton",
    environmentPath: "/environment/baton",
    workspaceRoot: "/workspace",
    platform: "linux",
    pathExists: (candidate) => candidate === "/toolchain/baton"
  });
  assert.equal(configured.command, "/toolchain/baton");
  assert.equal(configured.source, "setting");

  const fallback = resolveBatonPath({
    configuredPath: "",
    environmentPath: "",
    workspaceRoot: "/workspace",
    platform: "win32",
    pathExists: () => false
  });
  assert.equal(fallback.command, "baton");
  assert.equal(fallback.source, "PATH");
});
