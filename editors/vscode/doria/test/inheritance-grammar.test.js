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

test("presents Stage 34 inheritance words without semantic regexes", () => {
  const patterns = grammar.repository.keywords.patterns;
  const inheritance = patterns.find(
    (pattern) => pattern.name === "storage.modifier.inheritance.doria"
  );
  const control = patterns.find(
    (pattern) => pattern.name === "keyword.control.doria"
  );
  const context = patterns.find(
    (pattern) => pattern.name === "variable.language.class-context.doria"
  );
  const reserved = patterns.find(
    (pattern) => pattern.name === "keyword.other.reserved.doria"
  );

  assert.match("open", new RegExp(inheritance.match));
  assert.match("override", new RegExp(inheritance.match));
  assert.match("extends", new RegExp(control.match));
  assert.match("parent", new RegExp(context.match));
  assert.doesNotMatch("open", new RegExp(reserved.match));
  assert.doesNotMatch("override", new RegExp(reserved.match));
  assert.doesNotMatch("opened", new RegExp(inheritance.match));
  assert.doesNotMatch("overridden", new RegExp(inheritance.match));
});

test("presents constructor parameter roles without accepting an alias", () => {
  const patterns = grammar.repository.keywords.patterns;
  const role = patterns.find(
    (pattern) => pattern.name === "storage.modifier.parameter-role.doria"
  );
  const inheritance = patterns.find(
    (pattern) => pattern.name === "storage.modifier.inheritance.doria"
  );

  assert.match("parameter", new RegExp(role.match));
  assert.doesNotMatch("param", new RegExp(role.match));
  assert.doesNotMatch("parameterized", new RegExp(role.match));
  assert.match("override", new RegExp(inheritance.match));
});
