"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const {
  buildRunArguments,
  defaultDebugConfiguration,
  findBatonProjectRoot
} = require("../debug-support");

test("provides a Baton project launch profile by default", () => {
  assert.deepEqual(defaultDebugConfiguration(), {
    type: "doria",
    request: "launch",
    name: "Run Doria project",
    mode: "project",
    cwd: "${workspaceFolder}",
    args: [],
    release: false,
    noDebug: true
  });
});

test("builds a fast baton run command by default", () => {
  assert.deepEqual(
    buildRunArguments({
      request: "launch",
      mode: "project"
    }),
    ["run"]
  );
});

test("forwards Baton release mode and program arguments after the separator", () => {
  assert.deepEqual(
    buildRunArguments({
      request: "launch",
      mode: "project",
      release: true,
      args: ["--port", "8080", "two words"]
    }),
    [
      "run",
      "--release",
      "--",
      "--port",
      "8080",
      "two words"
    ]
  );
});

test("standalone mode runs an explicit source file through doriac", () => {
  assert.deepEqual(
    buildRunArguments({
      request: "launch",
      mode: "standalone",
      program: "/workspace/main.doria"
    }),
    ["run", "/workspace/main.doria"]
  );
});

test("rejects malformed launch configurations before starting the toolchain", () => {
  assert.throws(
    () => buildRunArguments({
      request: "launch",
      mode: "standalone",
      program: "",
      args: []
    }),
    /requires a Doria source file/
  );
  assert.throws(
    () => buildRunArguments({ request: "attach", program: "main.doria", args: [] }),
    /Unsupported Doria debug request/
  );
  assert.throws(
    () => buildRunArguments({ request: "launch", program: "main.doria", args: "one" }),
    /array of strings/
  );
});

test("finds a Baton project from a nested source directory", () => {
  const root = findBatonProjectRoot(
    "/workspace/project/src/deep",
    (candidate) => candidate === "/workspace/project/Baton.toml"
  );

  assert.equal(root, "/workspace/project");
});

test("returns undefined outside a Baton project", () => {
  assert.equal(findBatonProjectRoot("/workspace/standalone", () => false), undefined);
});
