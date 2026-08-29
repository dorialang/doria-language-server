"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const extension = fs.readFileSync(
  path.join(__dirname, "..", "extension.js"),
  "utf8"
);

test("bridges compiler-backed Doria code actions into VS Code workspace edits", () => {
  assert.match(extension, /registerCodeActionsProvider/);
  assert.match(extension, /textDocument\/codeAction/);
  assert.match(extension, /providedCodeActionKinds:\s*\[vscode\.CodeActionKind\.QuickFix\]/);
  assert.match(extension, /new vscode\.WorkspaceEdit\(\)/);
  assert.match(extension, /vscode\.TextEdit\.replace\(toRange\(change\.range\), change\.newText\)/);
});

test("keeps server-owned callable generation actions generic across documents", () => {
  assert.match(extension, /for \(const \[uri, changes\] of Object\.entries\(action\.edit\.changes\)\)/);
  assert.match(extension, /edit\.set\(\s*vscode\.Uri\.parse\(uri\)/);
  assert.doesNotMatch(extension, /Generate (?:method|function)/);
});
