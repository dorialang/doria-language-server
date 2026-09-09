"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const grammar = JSON.parse(fs.readFileSync(path.join(__dirname, "..", "syntaxes", "doria.tmLanguage.json"), "utf8"));
const fixture = fs.readFileSync(path.join(__dirname, "..", "..", "..", "fixtures", "stage35-contracts.doria"), "utf8");

test("retained iterator sources use the approved borrow modifier", () => {
  const source = fs.readFileSync(path.join(__dirname, "..", "..", "..", "fixtures", "stage35-core-iteration.doria"), "utf8");
  const modifier = grammar.repository.keywords.patterns.find(pattern => pattern.name === "storage.modifier.mutability.doria");
  assert.match("borrow", new RegExp(modifier.match));
  assert.doesNotMatch("borrowed", new RegExp(modifier.match));
  assert.match(source, /__construct\(borrow List<Book> \$source\)/);
  assert.match(source, /function getCurrent\(\): Book/);
});

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

test("interface runtime fixture uses existing shared, narrowing, and call syntax", () => {
  const source = fs.readFileSync(path.join(__dirname, "..", "..", "..", "fixtures", "stage35-interface-runtime.doria"), "utf8");
  const patterns = Object.values(grammar.repository).flatMap(entry => entry.patterns || []);
  for (const name of ["SharedReference", "WeakReference", "WritableSharedReference", "WritableWeakReference", "ReadonlySharedReferenceAccess", "WritableSharedReferenceAccess"]) {
    assert.ok(source.includes(`${name}<`));
    assert.ok(patterns.some(pattern => pattern.match && new RegExp(pattern.match).test(name)), name);
  }
  assert.match(source, /if \(\$boxed is Readable\)/);
  assert.match(source, /\$write->rename\("after"\)/);
  assert.match(source, /interface Failure extends Error/);
});
