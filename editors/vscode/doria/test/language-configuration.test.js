"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const configuration = JSON.parse(
  fs.readFileSync(path.join(__dirname, "..", "language-configuration.json"), "utf8")
);

test("delimiter Enter rules defer indentation text to VS Code settings", () => {
  const delimiterRules = configuration.onEnterRules.filter(
    ({ action }) => action.indent === "indentOutdent"
  );

  assert.equal(delimiterRules.length, 4);
  for (const rule of delimiterRules) {
    assert.equal(
      rule.action.appendText,
      rule.beforeText.startsWith("^\\s*/\\*\\*") ? " * " : undefined
    );
  }
});

test("no Enter rule inserts a literal escaped tab", () => {
  for (const rule of configuration.onEnterRules) {
    assert.notEqual(rule.action.appendText, "\\t");
  }
});
