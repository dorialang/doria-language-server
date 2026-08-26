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

test("keeps top-level internal declarations in modifier and declaration scopes", () => {
  const modifier = grammar.repository.keywords.patterns.find(
    ({ name }) => name === "storage.modifier.visibility.doria"
  );
  assert.ok(modifier);
  assert.match("internal", new RegExp(modifier.match));

  const declarations = grammar.repository.declarations.patterns;
  for (const [keyword, scope] of [
    ["class", "entity.name.type.class.doria"],
    ["enum", "entity.name.type.enum.doria"],
    ["interface", "entity.name.type.interface.doria"],
    ["trait", "entity.name.type.trait.doria"],
    ["function", "entity.name.function.doria"]
  ]) {
    const pattern = declarations.find(({ captures }) =>
      Object.values(captures || {}).some(({ name }) => name === scope)
    );
    assert.ok(pattern, `missing ${keyword} declaration scope`);
    assert.match(`${keyword} Name`, new RegExp(pattern.match));
  }

  const accepted = fs.readFileSync(
    path.join(__dirname, "..", "..", "..", "fixtures", "latest-tokens.doria"),
    "utf8"
  );
  for (const declaration of [
    "internal class PackageHelper",
    "internal enum PackageState",
    "internal interface PackageContract",
    "internal trait PackageSupport",
    "internal function packageHelper",
    "internal const int PACKAGE_LIMIT"
  ]) {
    assert.ok(accepted.includes(declaration), `missing fixture: ${declaration}`);
  }
});
