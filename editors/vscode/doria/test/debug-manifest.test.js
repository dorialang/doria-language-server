"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const { defaultDebugConfiguration } = require("../debug-support");

const manifest = JSON.parse(
  fs.readFileSync(path.join(__dirname, "..", "package.json"), "utf8")
);

test("activates for Doria debugging and contributes the Doria debugger type", () => {
  assert.ok(manifest.activationEvents.includes("onDebug:doria"));

  const debuggerContribution = manifest.contributes.debuggers.find(
    (debugger_) => debugger_.type === "doria"
  );
  assert.ok(debuggerContribution);
  assert.ok(debuggerContribution.languages.includes("doria"));
  assert.deepEqual(
    debuggerContribution.initialConfigurations,
    [defaultDebugConfiguration()]
  );
});

test("exposes project and standalone launch modes with explicit tool paths", () => {
  const debuggerContribution = manifest.contributes.debuggers.find(
    (debugger_) => debugger_.type === "doria"
  );
  const modes = debuggerContribution.configurationAttributes.launch
    .properties.mode.enum;
  const snippets = debuggerContribution.configurationSnippets;

  assert.deepEqual(modes, ["project", "standalone"]);
  assert.ok(snippets.some(({ body }) => body.mode === "project"));
  assert.ok(snippets.some(({ body }) => body.mode === "standalone"));
  assert.ok(manifest.contributes.configuration.properties["doria.baton.path"]);
  assert.ok(manifest.contributes.configuration.properties["doria.compiler.path"]);
});
