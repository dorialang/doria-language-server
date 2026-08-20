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
const capture = grammar.repository.closureCaptureList;

function patternWithScope(scope) {
  return closures.find(
    (pattern) =>
      pattern.name === scope ||
      Object.values(pattern.beginCaptures || {}).some(
        (capture) => capture.name === scope
      ) ||
      Object.values(pattern.endCaptures || {}).some(
        (capture) => capture.name === scope
      )
  );
}

test("scopes every accepted Stage 30a callable grammar form", () => {
  const arrow = patternWithScope("keyword.declaration.function.arrow.doria");
  const block = patternWithScope("keyword.declaration.function.anonymous.doria");
  const functionType = patternWithScope("storage.type.function.doria");

  assert.ok(arrow);
  assert.ok(block);
  assert.ok(functionType);
  assert.ok(capture);
  assert.match("fn(int $value) => $value", new RegExp(arrow.begin));
  assert.match(
    "function (int $value): bool {",
    new RegExp(block.begin)
  );
  assert.match(
    "function (int $value): Doria\\Std\\Io\\IoError {",
    new RegExp(block.begin)
  );
  assert.match(
    "function (function(int): string $callback): bool {",
    new RegExp(block.begin)
  );
  assert.doesNotMatch(
    "function(int): bool $callback): void {",
    new RegExp(block.begin),
    "a nested function type must not become an anonymous closure because a later body exists"
  );
  assert.match("function(int): int", new RegExp(functionType.begin));
  assert.match("function writable(int): int", new RegExp(functionType.begin));
  assert.match("function once(): Payload", new RegExp(functionType.begin));
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
    capture.patterns.some((pattern) => pattern.include === "#comments"),
    "capture comments must stay comments rather than becoming capture syntax"
  );
  assert.ok(
    functionType.patterns.some((pattern) => pattern.include === "#closures"),
    "nested function types must reuse the bounded function-type presentation"
  );
  assert.ok(
    functionType.patterns.some(
      (pattern) =>
        pattern.name === "storage.modifier.ownership.parameter.function-type.doria" &&
        new RegExp(pattern.match).test("take") &&
        new RegExp(pattern.match).test("writable")
    ),
    "function-type parameters must preserve writable and take ownership presentation"
  );
  assert.equal(
    functionType.beginCaptures["2"].name,
    "storage.modifier.invocation.function.doria"
  );
  assert.equal(arrow.end, "(=>)");
  assert.equal(
    arrow.endCaptures["1"].name,
    "keyword.operator.closure.arrow.doria"
  );
  assert.ok(
    arrow.patterns.some((pattern) => pattern.include === "#closureCaptureList")
  );
  assert.equal(
    closures.some(
      (pattern) => pattern.name === "keyword.operator.closure.arrow.doria"
    ),
    false,
    "match and when arrows must not receive the closure-arrow scope"
  );
});

test("does not classify nearby identifiers or prose as closure keywords", () => {
  const arrow = patternWithScope("keyword.declaration.function.arrow.doria");
  const functionType = patternWithScope("storage.type.function.doria");
  const reserved = grammar.repository.keywords.patterns.find(
    (pattern) => pattern.name === "keyword.other.reserved.doria"
  );
  const once = grammar.repository.keywords.patterns.find(
    (pattern) => pattern.name === "storage.modifier.invocation.function.doria"
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
  assert.ok(rootIncludes.indexOf("#comments") < rootIncludes.indexOf("#keywords"));
  assert.ok(rootIncludes.indexOf("#strings") < rootIncludes.indexOf("#keywords"));
  assert.doesNotMatch("fn", new RegExp(reserved.match));
  assert.doesNotMatch("with", new RegExp(reserved.match));
  assert.match("once", new RegExp(once.match));
  for (const identifier of ["onceOnly", "once_more", "once1"]) {
    assert.doesNotMatch(identifier, new RegExp(once.match));
  }
  assert.ok(rootIncludes.indexOf("#variables") < rootIncludes.indexOf("#keywords"));
});

test("scopes Stage 30a effects grouping and callable punctuation without changing named calls", () => {
  const functionType = patternWithScope("storage.type.function.doria");
  const grouping = grammar.repository.typeGrouping;
  const callable = grammar.repository.callableCalls.patterns[0];
  const namedCall = grammar.repository.calls.patterns[0];
  const throwsKeyword = grammar.repository.keywords.patterns.find(
    (pattern) => pattern.name === "keyword.control.exception.doria"
  );

  assert.match("throws", new RegExp(throwsKeyword.match));
  assert.match("(function(): int throws ParseError)", new RegExp(grouping.begin));
  assert.ok(grouping.patterns.some((pattern) => pattern.include === "#closures"));
  assert.equal(
    grouping.beginCaptures["1"].name,
    "punctuation.definition.type.group.begin.doria"
  );
  assert.equal(
    grouping.endCaptures["1"].name,
    "punctuation.definition.type.group.end.doria"
  );
  assert.match("$callback(", new RegExp(callable.match));
  assert.equal(
    callable.captures["3"].name,
    "punctuation.definition.arguments.begin.callable.doria"
  );
  assert.match("transform(", new RegExp(namedCall.match));
  const rootIncludes = grammar.patterns.map((pattern) => pattern.include);
  assert.ok(rootIncludes.indexOf("#callableCalls") < rootIncludes.indexOf("#variables"));
  assert.ok(rootIncludes.indexOf("#callableCalls") < rootIncludes.indexOf("#calls"));
  assert.match(
    "function((function(): int throws FirstError), string): void",
    new RegExp(functionType.begin)
  );
});

test("leaves malformed capture diagnostics to the compiler without reading comments as syntax", () => {
  const invalidMatches = invalid
    .map((pattern) => pattern.match)
    .filter(Boolean)
    .map((pattern) => new RegExp(pattern));

  assert.ok(
    invalidMatches.some((pattern) =>
      pattern.test("fn(int $value) use ($outside) => $value")
    ),
    "legacy PHP closure use remains visibly rejected"
  );

  for (const source of [
    "fn(int $value) with () => $value",
    "fn(int $value) with (&$outside) => $value",
    "fn(int $value) with (writable &$outside) => $value",
    "fn(int $value) with (readonly $outside) => $value",
    "fn(int $value) with ($outside /* readonly & */) => $value",
  ]) {
    assert.equal(
      invalidMatches.some((pattern) => pattern.test(source)),
      false,
      `compiler-owned capture validation must not use raw editor matching for ${source}`
    );
  }

  assert.equal(
    invalid.some(
      (pattern) => pattern.name === "invalid.illegal.closure.capture.doria"
    ),
    false
  );

  const invalidInvocation = invalid.find(
    (pattern) =>
      Object.values(pattern.captures || {}).some(
        (capture) => capture.name === "invalid.illegal.modifier.function.invocation.doria"
      )
  );
  assert.ok(invalidInvocation);
  const invalidInvocationPattern = new RegExp(invalidInvocation.match);
  for (const source of [
    "function take(): Payload",
    "function /* comment */ take(): Payload",
    "function // comment\n take(): Payload",
    "function # comment\n take /* comment */ (): Payload",
  ]) {
    assert.match(source, invalidInvocationPattern);
  }
  assert.doesNotMatch(
    "function(take Payload): void",
    invalidInvocationPattern,
    "take remains valid as a function-type parameter ownership mode"
  );
});
