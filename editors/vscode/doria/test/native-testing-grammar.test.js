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
const accepted = fs.readFileSync(
  path.join(__dirname, "..", "..", "..", "fixtures", "native-testing-slice2.doria"),
  "utf8"
);
const rejected = fs.readFileSync(
  path.join(
    __dirname,
    "..",
    "..",
    "..",
    "fixtures",
    "native-testing-slice2-rejected.doria"
  ),
  "utf8"
);

test("presents Slice 2 assertions through ordinary call and member scopes", () => {
  const call = new RegExp(grammar.repository.calls.patterns[0].match);
  const method = new RegExp(grammar.repository.accessors.patterns[1].match);
  const property = new RegExp(grammar.repository.accessors.patterns[2].match);

  assert.match("expect(", call);
  assert.match("fail(", call);
  assert.match("->toEqual(", method);
  assert.match("->not", property);
  assert.ok(accepted.includes('expect(add(20, 22))->toEqual(42)'));
  assert.ok(rejected.includes("expect(1)->not->not->toEqual(2)"));
});

test("keeps matcher semantics out of the TextMate grammar", () => {
  const serialized = JSON.stringify(grammar);
  for (const matcher of ["toEqual", "toBeNull", "toContain", "toThrow"]) {
    assert.equal(serialized.includes(matcher), false);
  }
});
