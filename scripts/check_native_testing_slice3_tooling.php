#!/usr/bin/env php
<?php

declare(strict_types=1);

require_once __DIR__ . '/compiler_pin.php';

$root = dirname(__DIR__);

function slice3Text(string $path): string
{
    $text = file_get_contents($path);
    if ($text === false) {
        fwrite(STDERR, "ERROR: could not read {$path}.\n");
        exit(1);
    }
    return $text;
}

function slice3Require(bool $condition, string $message): void
{
    if (!$condition) {
        fwrite(STDERR, "ERROR: {$message}\n");
        exit(1);
    }
}

function slice3Production(string $text, string $path): string
{
    $matched = preg_match('/^#\[cfg\(test\)\]\Rmod tests\s*\{/m', $text, $match, PREG_OFFSET_CAPTURE);
    slice3Require($matched === 1, "{$path} must retain a top-level unit-test module.");
    return substr($text, 0, $match[0][1]);
}

$manifest = slice3Text($root . '/Cargo.toml');
$lock = slice3Text($root . '/Cargo.lock');
$analysis = slice3Text($root . '/server/src/analysis.rs');
$server = slice3Text($root . '/server/src/lib.rs');
$index = slice3Text($root . '/server/src/workspace_index.rs');
$production = slice3Production($analysis, 'server/src/analysis.rs')
    . slice3Production($server, 'server/src/lib.rs')
    . $index;
$fixture = slice3Text($root . '/editors/fixtures/native-testing-slice3.doria');
$rejected = slice3Text($root . '/editors/fixtures/native-testing-slice3-rejected.doria');
$vscode = slice3Text($root . '/editors/vscode/doria/test/native-testing-grammar.test.js');
$intellij = slice3Text(
    $root . '/editors/intellij/doria/src/test/kotlin/dev/doria/intellij/highlighting/DoriaLexerTest.kt',
);
$docs = slice3Text($root . '/README.md')
    . slice3Text($root . '/CHANGELOG.md')
    . slice3Text($root . '/server/README.md')
    . slice3Text($root . '/docs/architecture.md')
    . slice3Text($root . '/docs/semantic-hover.md')
    . slice3Text($root . '/editors/vscode/doria/README.md')
    . slice3Text($root . '/editors/intellij/doria/README.md');

$expectedCompiler = doria_compiler_revision($manifest);
slice3Require($expectedCompiler !== null, 'Cargo.toml must pin an exact 40-character compiler revision.');
slice3Require(
    doria_compiler_revision_is_authoritative($expectedCompiler),
    'Cargo.toml must retain the centrally recorded compiler authority revision.',
);
slice3Require(
    doria_lock_resolves_revision($lock, $expectedCompiler),
    'Cargo.lock must resolve every Doria package to the current manifest revision.',
);

foreach ([
    'info.assertion_completions',
    'IMPLEMENTED_MEMBERS',
    'matcher.expected_operand()',
    'matcher.stable_complexity()',
    'SourceSemanticContext::is_development',
    'analyze_source_for_ide_with_source_context',
    'textDocument/documentSymbol',
    'workspace/symbol',
    'test_semantics()',
    'package.display_name()',
] as $fact) {
    slice3Require(str_contains($production, $fact), "compiler-owned Slice-3 projection is missing {$fact}.");
}

foreach ([
    'parse_expectation',
    'parse_matcher',
    'matcher_capability_matrix',
    'contains("/tests/")',
    'ends_with(".test.doria")',
    'codeLensProvider',
] as $forbidden) {
    slice3Require(!str_contains($production, $forbidden), "forbidden tooling behavior is present: {$forbidden}.");
}
foreach (['"toHaveCount"', '"toHaveKey"', '"toHaveValue"', '"toThrow"'] as $matcher) {
    slice3Require(!str_contains($production, $matcher), "production tooling must not own matcher spelling {$matcher}.");
}

foreach ([
    'native_testing_matcher_completion_uses_compiler_typed_candidates',
    'native_testing_import_completion_respects_compiler_source_scope',
    'native_testing_hovers_explain_compiler_owned_testing_contracts',
    'native_testing_symbols_and_navigation_preserve_authored_identity',
    'List<Bytes>',
    'expect as verify',
    'Doria\\\\Std\\\\Test\\\\expect',
    'Value::Null',
    '🧪 suite',
] as $coverage) {
    slice3Require(str_contains($server, $coverage), "Slice-3 server coverage is missing {$coverage}.");
}

foreach (['toBeEmpty', 'toHaveCount', 'toContain', 'toHaveKey', 'toHaveValue', 'toThrow'] as $matcher) {
    slice3Require(str_contains($fixture, $matcher), "accepted Slice-3 fixture is missing {$matcher}.");
}
foreach (['expect($bytes)->toContain', 'expect($values)->toContain', 'expect($items)->toHaveKey'] as $case) {
    slice3Require(str_contains($rejected, $case), "rejected Slice-3 fixture is missing {$case}.");
}
slice3Require(str_contains($vscode, 'presents Slice 3 collection and Error expectations'), 'VS Code Slice-3 fixture coverage is missing.');
slice3Require(str_contains($intellij, 'testNativeTestingSlice3KeepsMatcherSemanticsInTheLanguageServer'), 'IntelliJ Slice-3 fixture coverage is missing.');

foreach (['Native Testing Foundation are complete', 'Stage 34 single class inheritance is complete', 'Stage 35 interfaces and traits are next'] as $fact) {
    slice3Require(str_contains($docs, $fact), "final foundation documentation is missing {$fact}.");
}
foreach (['Slice 3 is next', 'foundation remains in progress', 'wait for Slice 3', 'remains blocked until the foundation'] as $stale) {
    slice3Require(!str_contains($docs, $stale), "stale foundation status remains: {$stale}.");
}

fwrite(STDOUT, "Native Testing Foundation Slice 3 tooling guard passed.\n");
