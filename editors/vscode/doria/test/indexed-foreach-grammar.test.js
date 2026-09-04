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
  path.join(__dirname, "..", "..", "..", "fixtures", "indexed-foreach.doria"),
  "utf8"
);
const rejected = fs.readFileSync(
  path.join(
    __dirname,
    "..",
    "..",
    "..",
    "fixtures",
    "indexed-foreach-rejected.doria"
  ),
  "utf8"
);

test("keeps indexed foreach roles in compiler semantics", () => {
  assert.ok(
    accepted.includes(
      "foreach ($this->contents as int $line => string $content)"
    )
  );
  assert.ok(
    accepted.includes("foreach ($contents as int $index => string $content)")
  );
  assert.ok(accepted.includes("foreach ($counts as string $name => int $count)"));
  assert.ok(rejected.includes("foreach ($labels as int $index => string $label)"));
  assert.ok(rejected.includes("foreach (0..<2 as int $index => int $value)"));

  const serialized = JSON.stringify(grammar);
  for (const semanticRole of [
    "Zero-Based Sequence Index",
    "Dictionary Key",
    "Sequence Index Binding Must Be Int"
  ]) {
    assert.equal(serialized.includes(semanticRole), false);
  }
});
