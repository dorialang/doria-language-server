#!/usr/bin/env php
<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$expectedCompiler = '7d9800c719366b9374921f8679c784d3a3e8d109';

function slice2_text(string $path): string
{
    $text = file_get_contents($path);
    if ($text === false) {
        fwrite(STDERR, "ERROR: could not read {$path}.\n");
        exit(1);
    }
    return $text;
}

function slice2_require(bool $condition, string $message): void
{
    if (!$condition) {
        fwrite(STDERR, "ERROR: {$message}\n");
        exit(1);
    }
}

function slice2_without_tests(string $text, string $path): string
{
    $matched = preg_match('/^#\[cfg\(test\)\]\Rmod tests\s*\{/m', $text, $match, PREG_OFFSET_CAPTURE);
    slice2_require($matched === 1, "{$path} must retain a top-level unit-test module.");
    return substr($text, 0, $match[0][1]);
}

$manifest = slice2_text($root . '/Cargo.toml');
$lock = slice2_text($root . '/Cargo.lock');
$analysis = slice2_text($root . '/server/src/analysis.rs');
$server = slice2_text($root . '/server/src/lib.rs');
$accepted = slice2_text($root . '/editors/fixtures/native-testing-slice2.doria');
$rejected = slice2_text($root . '/editors/fixtures/native-testing-slice2-rejected.doria');
$vscodeTest = slice2_text($root . '/editors/vscode/doria/test/native-testing-grammar.test.js');
$intellijLexer = slice2_text(
    $root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/highlighting/DoriaLexer.kt',
);
$intellijTest = slice2_text(
    $root . '/editors/intellij/doria/src/test/kotlin/dev/doria/intellij/highlighting/DoriaLexerTest.kt',
);
$docs = slice2_text($root . '/README.md')
    . slice2_text($root . '/CHANGELOG.md')
    . slice2_text($root . '/server/README.md')
    . slice2_text($root . '/docs/architecture.md')
    . slice2_text($root . '/docs/semantic-hover.md')
    . slice2_text($root . '/editors/vscode/doria/README.md')
    . slice2_text($root . '/editors/intellij/doria/README.md');

preg_match('/doriac\s*=\s*\{[^\n]*\brev\s*=\s*"([0-9a-f]{40})"/', $manifest, $pin);
slice2_require(($pin[1] ?? null) === $expectedCompiler, 'Cargo.toml must pin final foundation Doria.');
slice2_require(
    substr_count($lock, 'rev=' . $expectedCompiler . '#' . $expectedCompiler) >= 3,
    'Cargo.lock must resolve every Doria package to final Slice-2 Doria.',
);

foreach ([
    'assertion_semantic_tokens',
    'compiler_assertion_symbol_tokens',
    'info.assertions.get',
    'CompilerSymbolIdentity::StandardTest',
    'test_assertion_effect_documentation',
    'profile.test_assertion',
] as $projection) {
    slice2_require(str_contains($analysis, $projection), "compiler assertion projection is missing {$projection}.");
}

$production = slice2_without_tests($analysis, 'server/src/analysis.rs')
    . slice2_without_tests($server, 'server/src/lib.rs');
foreach ([
    'parse_expectation',
    'parse_matcher',
    'infer_test_from_path',
    'contains("/tests/")',
    'ends_with(".test.doria")',
] as $forbidden) {
    slice2_require(!str_contains($production, $forbidden), "tooling-owned assertion inference is forbidden: {$forbidden}.");
}
foreach ([
    '"toEqual"',
    '"toBeNull"',
    '"toBeTrue"',
    '"toContain"',
    '"toThrow"',
] as $matcher) {
    slice2_require(!str_contains($production, $matcher), "production tooling must not recognize matcher spelling {$matcher}.");
}

foreach ([
    'expect(add(20, 22))->toEqual(42)',
    'expect("Doria")->not->toEqual("PHP")',
    'AssertionError $error',
    'fail("explicit failure")',
] as $fixture) {
    slice2_require(str_contains($accepted, $fixture), "accepted Slice-2 fixture is missing {$fixture}.");
}
foreach (['not->not', 'toEqual()'] as $fixture) {
    slice2_require(str_contains($rejected, $fixture), "rejected Slice-2 fixture is missing {$fixture}.");
}

foreach ([
    'native_test_declarations_follow_compiler_facts_and_project_source_scope',
    'native_assertion_effects_flow_through_cross_file_helpers',
    'ordinary-expect.doria',
    'expect as verify',
    'Test\\\\expect(42)',
    'missing semantic token for',
    'UTF-16 matcher diagnostic',
    'stale assertion diagnostics must clear',
    'E0714',
    'E0716',
    'E0717',
    'E0719',
    'E0720',
    'E0420',
] as $coverage) {
    slice2_require(str_contains($server, $coverage), "Slice-2 tooling coverage is missing {$coverage}.");
}

slice2_require(
    str_contains($vscodeTest, 'presents Slice 2 assertions through ordinary call and member scopes')
        && str_contains($vscodeTest, 'keeps matcher semantics out of the TextMate grammar'),
    'VS Code must retain ordinary assertion-token and no-matcher-table coverage.',
);
slice2_require(
    str_contains($intellijTest, 'testNativeTestingSlice2UsesOrdinaryCallMemberAndTypeTokens'),
    'IntelliJ must retain ordinary assertion call/member/type token coverage.',
);
slice2_require(
    str_contains($intellijLexer, 'in WORD_OPERATORS -> if (previousAccessor() == "->")'),
    'IntelliJ word operators after member access must remain contextual member tokens.',
);

foreach ([
    'All three Native Testing Foundation',
    'Native Testing Foundation are complete',
    'collection/Error',
    'type-directed Test import and matcher completion',
    'Stage 34 single class inheritance is next',
] as $fact) {
    slice2_require(str_contains($docs, $fact), "Slice-2 tooling documentation is missing {$fact}.");
}
foreach (['Slice 2 is next', 'Expectations are not yet available', 'expectations remain Slice 2 work'] as $stale) {
    slice2_require(!str_contains($docs, $stale), "stale Slice-1 tooling claim remains: {$stale}.");
}

fwrite(STDOUT, "Native Testing Foundation Slice 2 tooling guard passed.\n");
