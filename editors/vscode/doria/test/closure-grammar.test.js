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

const closures = grammar.repository.closures.patterns;
const invalid = grammar.repository.invalid.patterns;

function patternWithScope(scope) {
  return closures.find(
    (pattern) =>
      pattern.name === scope ||
      Object.values(pattern.beginCaptures || {}).some(
        (capture) => capture.name === scope
      )
  );
}

test("scopes every accepted pre-Stage-30 closure form", () => {
  const arrow = patternWithScope("keyword.declaration.function.arrow.doria");
  const block = patternWithScope("keyword.declaration.function.anonymous.doria");
  const functionType = patternWithScope("storage.type.function.doria");
  const capture = patternWithScope("keyword.control.closure.capture.doria");

  assert.ok(arrow);
  assert.ok(block);
  assert.ok(functionType);
  assert.ok(capture);
  assert.match("fn(int $value) => $value", new RegExp(arrow.begin));
  assert.match(
    "function (int $value): bool {",
    new RegExp(block.begin)
  );
  assert.match("function(int): int", new RegExp(functionType.begin));
  assert.match("function(function(int): string): bool", new RegExp(functionType.begin));
  assert.match("with ($minimum)", new RegExp(capture.begin));
  assert.match("with (writable $count)", new RegExp(capture.begin));
  assert.match("with (take $message)", new RegExp(capture.begin));

  assert.ok(
    arrow.patterns.some(
      (pattern) => pattern.name === "variable.parameter.closure.doria"
    )
  );
  assert.ok(
    capture.patterns.some(
      (pattern) => pattern.name === "variable.other.capture.doria"
    )
  );
  assert.ok(
    capture.patterns.some(
      (pattern) => pattern.name === "storage.modifier.ownership.capture.doria"
    )
  );
  assert.ok(
    functionType.patterns.some((pattern) => pattern.include === "#closures"),
    "nested function types must reuse the bounded function-type presentation"
  );
  assert.ok(patternWithScope("keyword.operator.closure.arrow.doria"));
});

test("does not classify nearby identifiers or prose as closure keywords", () => {
  const arrow = patternWithScope("keyword.declaration.function.arrow.doria");
  const functionType = patternWithScope("storage.type.function.doria");
  const capture = patternWithScope("keyword.control.closure.capture.doria");
  const reserved = grammar.repository.keywords.patterns.find(
    (pattern) => pattern.name === "keyword.other.reserved.doria"
  );

  for (const identifier of ["fnord", "withdraw", "functionality"]) {
    assert.doesNotMatch(identifier, new RegExp(arrow.begin));
    assert.doesNotMatch(identifier, new RegExp(functionType.begin));
    assert.doesNotMatch(identifier, new RegExp(capture.begin));
    assert.doesNotMatch(identifier, new RegExp(reserved.match));
  }

  const rootIncludes = grammar.patterns.map((pattern) => pattern.include);
  assert.ok(rootIncludes.indexOf("#comments") < rootIncludes.indexOf("#closures"));
  assert.ok(rootIncludes.indexOf("#strings") < rootIncludes.indexOf("#closures"));
  assert.doesNotMatch("fn", new RegExp(reserved.match));
  assert.doesNotMatch("with", new RegExp(reserved.match));
});

test("keeps malformed capture syntax rejected", () => {
  const invalidMatches = invalid
    .map((pattern) => pattern.match)
    .filter(Boolean)
    .map((pattern) => new RegExp(pattern));

  for (const source of [
    "fn(int $value) use ($outside) => $value",
    "fn(int $value) with () => $value",
    "fn(int $value) with (&$outside) => $value",
    "fn(int $value) with (writable &$outside) => $value",
    "fn(int $value) with (readonly $outside) => $value",
  ]) {
    assert.ok(
      invalidMatches.some((pattern) => pattern.test(source)),
      `expected rejected closure presentation for ${source}`
    );
  }
});
