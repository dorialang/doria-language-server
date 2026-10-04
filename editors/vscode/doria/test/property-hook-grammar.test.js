"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { before, test } = require("node:test");
const { Registry, parseRawGrammar, INITIAL } = require("vscode-textmate");
const { loadWASM, OnigScanner, OnigString } = require("vscode-oniguruma");

const fixture = fs.readFileSync(path.join(__dirname, "..", "..", "..", "fixtures", "stage36-property-hooks.doria"), "utf8");
let grammar;
let withoutHooks;
before(async () => {
  const wasm = fs.readFileSync(require.resolve("vscode-oniguruma/release/onig.wasm"));
  await loadWASM(wasm.buffer.slice(wasm.byteOffset, wasm.byteOffset + wasm.byteLength));
  const definition = parseRawGrammar(fs.readFileSync(path.join(__dirname, "..", "syntaxes", "doria.tmLanguage.json"), "utf8"), "doria.json");
  const load = async definition => new Registry({
    onigLib: Promise.resolve({
      createOnigScanner: patterns => new OnigScanner(patterns),
      createOnigString: text => new OnigString(text),
    }),
    loadGrammar: async () => definition,
  }).loadGrammar("source.doria");
  grammar = await load(definition);
  withoutHooks = await load({ ...definition, patterns: definition.patterns.filter(pattern => pattern.include !== "#propertyHooks") });
});

function tokenize(source, tokenizer = grammar) {
  let state = INITIAL;
  const tokens = [];
  for (const line of source.split("\n")) {
    const result = tokenizer.tokenizeLine(line, state);
    state = result.ruleStack;
    tokens.push(...result.tokens.map(token => ({ text: line.slice(token.startIndex, token.endIndex), scopes: token.scopes })));
  }
  return tokens;
}

const hookTokens = tokens => tokens.filter(token => token.scopes.includes("keyword.declaration.property-hook.doria"));

test("tokenizes the shared Stage 36 fixture with callable get/set names intact", () => {
  const tokens = tokenize(fixture);
  assert.equal(hookTokens(tokens).length, 13);
  const names = tokens.filter(token => ["get", "set"].includes(token.text));
  const count = scope => names.filter(token => token.scopes.includes(scope)).length;
  assert.equal(count("entity.name.function.doria"), 4);
  assert.equal(count("entity.name.function.call.doria"), 3);
  assert.equal(count("entity.name.function.method.call.doria"), 4);
  const borrowed = tokens.filter(token => token.text === "borrowed");
  const borrowedCount = scope => borrowed.filter(token => token.scopes.includes(scope)).length;
  assert.equal(borrowedCount("storage.modifier.ownership.property-hook.doria"), 2);
  assert.equal(borrowedCount("entity.name.function.doria"), 2);
  assert.equal(borrowedCount("entity.name.function.call.doria"), 1);
  assert.equal(borrowedCount("entity.name.function.method.call.doria"), 1);
  for (const source of ["interface Named", "trait HasName", "writable get", "borrowed get;", "writable borrowed get", "set (take List<string> $value) throws", "open int $reading = 1", "override int $reading = 2"]) {
    assert.ok(fixture.includes(source), source);
  }
});

test("keeps multiline hook headers and nested expressions in their lexical scopes", () => {
  const source = [
    "class Value {",
    "    writable List<string> $items /* property */",
    "    {",
    "        writable borrowed /* result */",
    "            get",
    "            throws Failure",
    "        {",
    "            if (true) { set(value: get()); }",
    "            return $this->items;",
    "        }",
    "        set /* setter */ (",
    "            take List<string> $value",
    "        ) throws Failure",
    "        {",
    "            $this->items = $value;",
    "        }",
    "    }",
    "    int $next { get => get(); }",
    "    function get(): int { return 1; }",
    "    function set(int $value): void {}",
    "}",
  ].join("\n");
  assert.deepEqual(hookTokens(tokenize(source)).map(token => token.text), ["get", "set", "get"]);
  assert.equal(tokenize(source).filter(token => token.scopes.includes("storage.modifier.ownership.property-hook.doria")).length, 1);
});

test("keeps borrowed names callable outside hook headers", () => {
  const source = [
    "borrowed; borrowedValue; borrowed(); Accessors::borrowed();",
    "$object->borrowed(); $object->borrowed; Accessors::borrowed;",
    "function borrowed(): void {}",
    "class Example { List<string> $items { get => borrowed();",
    "set (take List<string> $value) { borrowed(); } }",
    "function borrowed(): void {} }",
    '"borrowed get"; // borrowed get',
  ].join("\n");
  const tokens = tokenize(source);
  assert.ok(!tokens.some(token => token.scopes.includes("storage.modifier.ownership.property-hook.doria")));
  const names = tokens.filter(token => token.text === "borrowed");
  const count = scope => names.filter(token => token.scopes.includes(scope)).length;
  assert.equal(count("entity.name.function.call.doria"), 3);
  assert.equal(count("entity.name.function.doria"), 2);
  assert.equal(count("entity.name.function.method.call.doria"), 1);
  assert.equal(count("entity.name.function.static.doria"), 1);
});

test("rejects borrow get without changing retained-source parameters", () => {
  const tokens = tokenize("class Example { List<string> $items { borrow /* wrong spelling */\n get; } " +
    "function __construct(borrow List<string> $source) {} }");
  const modifiers = tokens.filter(token => token.text === "borrow");
  assert.equal(modifiers.length, 2);
  assert.ok(modifiers[0].scopes.includes("invalid.illegal.modifier.property-hook.doria"));
  assert.ok(modifiers[1].scopes.includes("storage.modifier.mutability.doria"));
});

test("does not reserve identifiers outside hooks or read comments and strings as hooks", () => {
  const source = [
    "get; set; getter; setter; forget; reset;",
    "get(); set(1); set($value); set(value: get()); set(fn(int $value) => $value);",
    "$values->get($key); $values->set($key, $value);",
    "Accessors::get(); Accessors::set($value);",
    "$object->get; $object->set; Accessors::get;",
    "function get(): int { return 1; }",
    "function set(int $value): void {}",
    "// $property { get; set(int $value); }",
    "\"$property { get; set(int $value); }\";",
    "class Example { int $value { /* get; set(int $value); */ get => get(); } }",
  ].join("\n");
  assert.deepEqual(hookTokens(tokenize(source)).map(token => token.text), ["get"]);
});

test("separates backing initializers from hooks without confusing expression bodies", () => {
  for (const initializer of [
    '""', '"{ get; set(int $value); }"',
    '["value" => [get(), 2]]', 'Factory::make(get())',
    'fn(): int => get()',
    'function(): int { set(value: get()); return get(); }',
    'match (get()) { true => get(), false => 0 }',
    'when (true): int { set(value: get()); return 1; } else { return 0; }',
    'given { let $initial = get(); } when (true): int { return $initial; } else { return 0; }',
    'Factory::make(function(): int { return get(); })',
  ]) {
    for (const separator of ["", "\n/* hooks follow */\n"]) {
      const source = `class Example { writable mixed $value = ${initializer}${separator}{
        get => $this->value;
        set (mixed $value) { $this->value = $value; }
      } function get(): int { return 1; } }`;
      const tokens = tokenize(source);
      assert.deepEqual(hookTokens(tokens).map(token => token.text), ["get", "set"], source);
      assert.ok(tokens.some(token => token.text === "get" && token.scopes.includes("entity.name.function.doria")));
    }
    assert.deepEqual(hookTokens(tokenize(`let $value = ${initializer}; set(value: get());`)), [], initializer);
  }
});

test("preserves existing token colors outside property hooks", () => {
  const source = [
    'let $value = ["name" => [1, get()]];',
    'let $value = function(take Value $item): int { return $item->get(); };',
    'let $value = fn(int $item): int => $item + 1;',
    'let $value = match (true) { true => get(), false => 0 };',
    'let $value = when (true): int { return 1; } else { return 0; };',
    'return $this',
    ';',
    'string $value = "{ get; set(int $value); }";',
    'let $value = function take (): void {};',
    '// $property = function() { get; set; }',
  ].join("\n");
  const colors = tokens => tokens.flatMap(token => [...token.text].map(character => [character, token.scopes.at(-1)]));
  assert.deepEqual(colors(tokenize(source)), colors(tokenize(source, withoutHooks)));
});
