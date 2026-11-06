"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { before, test } = require("node:test");
const { Registry, parseRawGrammar, INITIAL } = require("vscode-textmate");
const { loadWASM, OnigScanner, OnigString } = require("vscode-oniguruma");

let grammar;
before(async () => {
  const wasm = fs.readFileSync(require.resolve("vscode-oniguruma/release/onig.wasm"));
  await loadWASM(wasm.buffer.slice(wasm.byteOffset, wasm.byteOffset + wasm.byteLength));
  const definition = parseRawGrammar(fs.readFileSync(path.join(__dirname, "..", "syntaxes", "doria.tmLanguage.json"), "utf8"), "doria.json");
  grammar = await new Registry({
    onigLib: Promise.resolve({
      createOnigScanner: patterns => new OnigScanner(patterns),
      createOnigString: text => new OnigString(text),
    }),
    loadGrammar: async () => definition,
  }).loadGrammar("source.doria");
});

function tokenize(source) {
  let state = INITIAL;
  return source.split("\n").flatMap(line => {
    const result = grammar.tokenizeLine(line, state);
    state = result.ruleStack;
    return result.tokens.map(token => ({ text: line.slice(token.startIndex, token.endIndex), scopes: token.scopes }));
  });
}

test("reserved spellings use ordinary member scopes in declaration and access positions", () => {
  const fixture = fs.readFileSync(path.join(__dirname, "..", "..", "..", "fixtures", "stage36a-reserved-members.doria"), "utf8");
  const tokens = tokenize(fixture + "\nasync await spawn scope");
  for (const name of ["async", "await", "spawn", "scope"]) {
    const matches = tokens.filter(token => token.text === name);
    assert.ok(matches[0].scopes.includes("entity.name.function.doria"), name);
    const callScope = ["spawn", "scope"].includes(name)
      ? "entity.name.function.static.doria"
      : "entity.name.function.method.call.doria";
    assert.ok(matches.at(-2).scopes.includes(callScope), name);
    assert.ok(matches.at(-1).scopes.some(scope => scope.startsWith("keyword.")), name);
  }
});

test("consuming methods keep explicit callback ownership and invocation modifiers distinct", () => {
  const fixture = fs.readFileSync(path.join(__dirname, "..", "..", "..", "fixtures", "stage36a-consuming-methods.doria"), "utf8");
  const tokens = tokenize(fixture);
  const consuming = tokens.filter(token => token.text === "consuming");
  assert.equal(consuming.length, 2);
  assert.ok(consuming.every(token => token.scopes.includes("storage.modifier.mutability.doria")));
  const takes = tokens.filter(token => token.text === "take");
  assert.equal(takes.length, 3);
  assert.ok(takes.every(token => token.scopes.includes("storage.modifier.mutability.doria")));
  for (const name of ["once", "writable"]) {
    assert.ok(tokens.some(token => token.text === name && token.scopes.includes("storage.modifier.invocation.function.doria")), name);
  }
  for (const name of ["finish", "continueBuilding", "accept", "acceptOnce", "acceptWriter"]) {
    assert.ok(tokens.some(token => token.text === name && token.scopes.includes("entity.name.function.doria")), name);
  }
  assert.ok(!tokens.some(token => token.scopes.some(scope => scope.startsWith("invalid."))));
});

test("consuming respects comments, strings, identifier boundaries, and multiline declarations", () => {
  const tokens = tokenize([
    "internal consuming /* receiver */",
    "function finish(): void {}",
    "consumingValue consuming_more consuming1 $consuming;",
    '"consuming"; // consuming',
  ].join("\n"));
  const modifiers = tokens.filter(token => token.scopes.includes("storage.modifier.mutability.doria"));
  assert.deepEqual(modifiers.map(token => token.text), ["consuming"]);
  assert.ok(tokens.some(token => token.text === "finish" && token.scopes.includes("entity.name.function.doria")));
  assert.ok(tokens.some(token => token.text === "$consuming" && token.scopes.includes("variable.other.doria")));
});
