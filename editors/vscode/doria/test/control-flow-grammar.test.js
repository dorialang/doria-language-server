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

test("scopes every accepted Stage 28a finalizer attachment without inventing elseif", () => {
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

  const acceptedAttachments = [
    "if (true) {} finally {}",
    "given { true; } if (true) {} finally {}",
    "let $value = when (true): int { return 1; } else { return 0; } finally {};",
    "let $value = given { true; } when (true): int { return 1; } else { return 0; } finally {};",
    "while (true) {} finally {}",
    "given { true; } while (true) {} finally {}",
    "do {} while (false) finally {}"
  ];
  for (const source of acceptedAttachments) {
    assert.match(source, /\bfinally\b/);
    for (const keyword of source.match(/[A-Za-z]+/g) || []) {
      if (["if", "else", "while", "return", "when", "given", "finally", "do"].includes(keyword)) {
        assert.match(keyword, new RegExp(control.match));
      }
    }
  }
});
