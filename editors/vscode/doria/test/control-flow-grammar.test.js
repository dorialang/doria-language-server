"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const grammar = JSON.parse(
  fs.readFileSync(
    path.join(__dirname, "..", "syntaxes", "doria.tmLanguage.json"),
    "utf8"
  )
);

test("scopes the Stage 28a control-flow spellings without inventing elseif", () => {
  const control = grammar.repository.keywords.patterns.find(
    ({ name }) => name === "keyword.control.doria"
  );

  assert.ok(control);
  for (const keyword of [
    "if",
    "else",
    "while",
    "return",
    "when",
    "given",
    "finally",
    "do"
  ]) {
    assert.match(keyword, new RegExp(control.match));
  }
  assert.doesNotMatch("elseif", new RegExp(control.match));
});
