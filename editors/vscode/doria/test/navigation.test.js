"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const extension = fs.readFileSync(
  path.join(__dirname, "..", "extension.js"),
  "utf8"
);

test("bridges compiler-backed definition locations into VS Code", () => {
  assert.match(extension, /registerDefinitionProvider/);
  assert.match(extension, /textDocument\/definition/);
  assert.match(extension, /new vscode\.Location\(/);
  assert.match(extension, /vscode\.Uri\.parse\(location\.uri\)/);
});

test("bridges compiler-backed cross-document rename edits into VS Code", () => {
  assert.match(extension, /registerRenameProvider/);
  assert.match(extension, /textDocument\/rename/);
  assert.match(extension, /newName/);
  assert.match(extension, /toWorkspaceEdit\(edit\)/);
});

test("exposes the exact compiler-backed reference set used by rename", () => {
  assert.match(extension, /registerReferenceProvider/);
  assert.match(extension, /textDocument\/references/);
  assert.match(extension, /includeDeclaration: context\.includeDeclaration/);
  assert.match(extension, /\(locations \?\? \[\]\)\.map\(toLocation\)/);
});
