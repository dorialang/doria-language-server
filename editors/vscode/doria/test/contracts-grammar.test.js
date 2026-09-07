"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const grammar = JSON.parse(fs.readFileSync(path.join(__dirname, "..", "syntaxes", "doria.tmLanguage.json"), "utf8"));
const fixture = fs.readFileSync(path.join(__dirname, "..", "..", "..", "fixtures", "stage35-contracts.doria"), "utf8");

test("presents generic composition and adaptations without checking conformance", () => {
  const control = grammar.repository.keywords.patterns.find(pattern => pattern.name === "keyword.control.doria");
  assert.match("insteadof", new RegExp(control.match));
  assert.doesNotMatch("insteadoffer", new RegExp(control.match));
  const patterns = Object.values(grammar.repository).flatMap(entry => entry.patterns || []);
  const composition = patterns.find(pattern => pattern.name === "meta.trait-composition.doria");
  assert.match("    uses Format<int>, Alternative<int> {", new RegExp(composition.begin));
  assert.match("{", new RegExp(composition.end));
  assert.match(fixture, /Format<int>::render insteadof Alternative<int>;/);
  assert.match(fixture, /function required\(\): int;/);
});
