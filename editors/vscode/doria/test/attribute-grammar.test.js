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
  path.join(__dirname, "..", "..", "..", "fixtures", "stage32-attributes.doria"),
  "utf8"
);
const rejected = fs.readFileSync(
  path.join(
    __dirname,
    "..",
    "..",
    "..",
    "fixtures",
    "stage32-attributes-rejected.doria"
  ),
  "utf8"
);

test("presents accepted Stage 32 attributes without swallowing hash comments", () => {
  const attribute = grammar.repository.attributes.patterns[0];
  const comment = grammar.repository.comments.patterns.find(
    (pattern) => pattern.name === "comment.line.number-sign.doria"
  );

  assert.match("#[Attribute]", new RegExp(attribute.begin));
  assert.match("#[Acme\\Metadata\\Route(path: \"/\")]", new RegExp(attribute.begin));
  assert.doesNotMatch("#[]", new RegExp(attribute.begin));
  assert.match("# ordinary hash comment", new RegExp(comment.match));
  assert.match("# [Test]", new RegExp(comment.match));
  assert.doesNotMatch("#[Test]", new RegExp(comment.match));
  for (const source of ["#[Attribute]", "#[Test]", "#[PHPExport]", "#[Route(path:"]) {
    assert.ok(accepted.includes(source), `missing accepted fixture: ${source}`);
  }
  assert.ok(rejected.includes("#[]"));
});

test("keeps semantic policy out of the TextMate grammar", () => {
  const serialized = JSON.stringify(grammar.repository.attributes);
  for (const semanticRule of [
    "class not marked",
    "constant expression",
    "invalid target",
    "missing argument"
  ]) {
    assert.equal(serialized.includes(semanticRule), false);
  }
});
