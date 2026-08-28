"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const root = path.join(__dirname, "..");
const grammar = JSON.parse(fs.readFileSync(path.join(root, "syntaxes", "doria.tmLanguage.json"), "utf8"));
const configuration = JSON.parse(fs.readFileSync(path.join(root, "language-configuration.json"), "utf8"));
const manifest = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
const extension = fs.readFileSync(path.join(root, "extension.js"), "utf8");

test("highlights Doria parameter modifiers inside PHPDoc-compatible tags", () => {
  const documentation = grammar.repository.comments.patterns.find(
    (pattern) => pattern.name === "comment.block.documentation.doria"
  );
  const parameter = documentation.patterns.find((pattern) => pattern.match.startsWith("(@param)"));
  const match = new RegExp(parameter.match).exec(
    "@param internal writable List<string> $items Items to render."
  );

  assert.ok(match);
  assert.equal(match[2], "internal writable ");
  assert.equal(match[3], "List<string>");
  assert.equal(match[4], "$items");
  assert.equal(parameter.captures[2].name, "storage.modifier.parameter.documentation.doria");

  const structural = new RegExp(parameter.match).exec(
    "@param take function writable(Dictionary<string, List<int>>): ?Result<string> throws ParseError $transform"
  );
  assert.ok(structural);
  assert.equal(
    structural[3],
    "function writable(Dictionary<string, List<int>>): ?Result<string> throws ParseError"
  );
  assert.equal(structural[4], "$transform");
});

test("continues ordinary and documentation block comments on Enter", () => {
  const blockPair = configuration.autoClosingPairs.find((pair) => pair.open === "/*");
  const docPair = configuration.autoClosingPairs.find((pair) => pair.open === "/**");
  assert.equal(blockPair.close, " */");
  assert.equal(docPair.close, " */");

  const openers = configuration.onEnterRules.map((rule) => new RegExp(rule.beforeText));
  assert.ok(openers.some((pattern) => pattern.test("/*")));
  assert.ok(openers.some((pattern) => pattern.test("/**")));
});

test("requests compiler-backed documentation tags when format-on-type is enabled", () => {
  assert.equal(manifest.contributes.configurationDefaults["[doria]"]["editor.formatOnType"], true);
  assert.match(extension, /registerOnTypeFormattingEditProvider/);
  assert.match(extension, /textDocument\/onTypeFormatting/);
  assert.match(extension, /provideOnTypeFormattingEdits/);
});
