#!/usr/bin/env php
<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$expectedCompiler = 'f229e07297e1677afb2e2d3af0c25d8cf306b36b';

function native_testing_text(string $path): string
{
    $text = file_get_contents($path);
    if ($text === false) {
        fwrite(STDERR, "ERROR: could not read {$path}.\n");
        exit(1);
    }
    return $text;
}

function native_testing_require(bool $condition, string $message): void
{
    if (!$condition) {
        fwrite(STDERR, "ERROR: {$message}\n");
        exit(1);
    }
}

function native_testing_without_unit_test_module(string $text, string $path): string
{
    $matched = preg_match(
        '/^#\[cfg\(test\)\]\Rmod tests\s*\{/m',
        $text,
        $match,
        PREG_OFFSET_CAPTURE,
    );
    native_testing_require($matched === 1, "{$path} must retain a top-level unit-test module.");

    return substr($text, 0, $match[0][1]);
}

$manifest = native_testing_text($root . '/Cargo.toml');
$lock = native_testing_text($root . '/Cargo.lock');
$analysis = native_testing_text($root . '/server/src/analysis.rs');
$server = native_testing_text($root . '/server/src/lib.rs');
$index = native_testing_text($root . '/server/src/workspace_index.rs');
$project = native_testing_text($root . '/server/src/project.rs');
$discovery = native_testing_text($root . '/server/src/baton_discovery.rs');
$docs = native_testing_text($root . '/README.md')
    . native_testing_text($root . '/server/README.md')
    . native_testing_text($root . '/docs/architecture.md')
    . native_testing_text($root . '/docs/semantic-hover.md')
    . native_testing_text($root . '/editors/vscode/doria/README.md')
    . native_testing_text($root . '/editors/intellij/doria/README.md');

preg_match('/doriac\s*=\s*\{[^\n]*\brev\s*=\s*"([0-9a-f]{40})"/', $manifest, $pin);
native_testing_require(
    ($pin[1] ?? null) === $expectedCompiler,
    'Cargo.toml must pin the final Native Testing Foundation compiler while preserving Slice 1.',
);
native_testing_require(
    substr_count($lock, 'rev=' . $expectedCompiler . '#' . $expectedCompiler) >= 3,
    'Cargo.lock must resolve every compiler package to the final foundation commit.',
);

foreach (['TestSemanticFacts', 'SourceSemanticContext', 'test_semantics', 'source_semantic_context', 'is_development()', 'test_semantic_tokens'] as $fact) {
    native_testing_require(str_contains($analysis, $fact), "compiler test-fact projection is missing {$fact}.");
}
foreach (['CompilerKnownTestDeclaration', 'GlobalReferenceRole::TestDeclaration', 'is_future_member'] as $fact) {
    native_testing_require(str_contains($index, $fact), "compiler-known test identity handling is missing {$fact}.");
}
foreach (['native_test_declarations_follow_compiler_facts_and_project_source_scope', 'SourceScope::Development', 'SourceScope::Main', 'semantic_token_records', '"tokenModifiers": ["declaration"]'] as $coverage) {
    native_testing_require(str_contains($server, $coverage), "native testing tooling coverage is missing {$coverage}.");
}
foreach (['analyze_project_graph', 'tooling_build_plan', 'baton project', 'never parses `Baton.toml`'] as $fact) {
    native_testing_require(
        str_contains($server . $project . $discovery . $docs, $fact),
        "Baton/compiler project integration is missing {$fact}.",
    );
}

$scanFixture = <<<'RUST'
fn before_test_helper() {}
#[cfg(test)]
fn test_helper() {}
fn production_after_test_helper() { parse_behavioral(); }
#[cfg(test)]
mod tests {}
RUST;
native_testing_require(
    str_contains(
        native_testing_without_unit_test_module($scanFixture, 'guard scan fixture'),
        'parse_behavioral',
    ),
    'item-level test configuration must not hide later production code from the tooling guard.',
);

$productionServer = native_testing_without_unit_test_module($server, 'server/src/lib.rs');
$productionAnalysis = native_testing_without_unit_test_module($analysis, 'server/src/analysis.rs');
$production = $productionServer . $productionAnalysis . $index;
foreach (['parse_behavioral', 'parse_test_declaration', 'infer_test_from_path', 'contains("/tests/")', 'ends_with(".test.doria")'] as $forbidden) {
    native_testing_require(
        !str_contains($production, $forbidden),
        "language-server-authored testing inference is forbidden: {$forbidden}.",
    );
}
native_testing_require(
    !str_contains($production, '"describe"')
        && !str_contains($production, '"Doria\\\\Std\\\\Test"'),
    'production tooling must consume compiler identities instead of recognizing raw test spellings.',
);

foreach (['All three Native Testing Foundation', 'Native Testing Foundation are complete', 'Stage 34 single class inheritance is next'] as $fact) {
    native_testing_require(str_contains($docs, $fact), "native testing tooling documentation is missing {$fact}.");
}

fwrite(STDOUT, "Native Testing Foundation Slice 1 tooling guard passed.\n");
