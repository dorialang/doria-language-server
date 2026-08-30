#!/usr/bin/env php
<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$expectedCompiler = '71ca767a4fa5f813258d89c7aa46a0600e1212f9';

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
    'Cargo.toml must pin the final Slice-1 compiler authority commit.',
);
native_testing_require(
    substr_count($lock, 'rev=' . $expectedCompiler . '#' . $expectedCompiler) >= 3,
    'Cargo.lock must resolve every compiler package to the final Slice-1 commit.',
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

$productionServer = explode('#[cfg(test)]', $server, 2)[0];
$productionAnalysis = explode('#[cfg(test)]', $analysis, 2)[0];
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

foreach (['Native Testing Foundation Slice 1', 'Slice 2 is next', 'Expectations are not yet available', 'final testing completion, hover, and navigation wait'] as $fact) {
    native_testing_require(str_contains($docs, $fact), "native testing tooling documentation is missing {$fact}.");
}

fwrite(STDOUT, "Native Testing Foundation Slice 1 tooling guard passed.\n");
