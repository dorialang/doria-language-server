"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const root = path.join(__dirname, "..");
const extension = fs.readFileSync(path.join(root, "extension.js"), "utf8");
const manifest = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));

test("project discovery receives one explicit Baton override", () => {
  assert.match(extension, /DORIA_BATON_PATH: batonPath/);
  assert.match(extension, /initializationOptions:\s*\{\s*batonPath: batonPath \|\| null/);
  assert.match(extension, /environmentPath: process\.env\.DORIA_BATON_PATH/);
  assert.doesNotMatch(extension, /process\.env\.BATON_PATH/);
  assert.match(
    manifest.contributes.configuration.properties["doria.baton.path"].description,
    /DORIA_BATON_PATH/
  );
});

test("project structure watchers and manual refresh share the LSP boundary", () => {
  for (const pattern of [
    "Baton.toml",
    "Baton.lock",
    "**/*.doria",
    ".doria/build/**",
    "build/.baton/**"
  ]) {
    assert.match(extension, new RegExp(pattern.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  }
  assert.match(extension, /workspace\/didChangeWatchedFiles/);
  assert.match(extension, /client\/registerCapability/);
  assert.match(extension, /client\/unregisterCapability/);
  assert.match(extension, /dynamicRegistration: true/);
  assert.match(extension, /new vscode\.RelativePattern\(vscode\.Uri\.parse\(glob\.baseUri\)/);
  assert.match(extension, /dynamicProjectWatchers/);
  assert.match(extension, /workspace\/executeCommand/);
  assert.equal(
    manifest.contributes.commands.some(({ command }) => command === "doria.refreshProject"),
    true
  );
});
