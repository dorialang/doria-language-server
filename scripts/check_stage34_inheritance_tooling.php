#!/usr/bin/env php
<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$expectedCompiler = 'b2e2b2b7768e14904cf137d4ca5d9d0f223c646a';

function stage34_text(string $path): string
{
    $text = file_get_contents($path);
    if ($text === false) {
        fwrite(STDERR, "ERROR: could not read {$path}.\n");
        exit(1);
    }
    return $text;
}

function stage34_require(bool $condition, string $message): void
{
    if (!$condition) {
        fwrite(STDERR, "ERROR: {$message}\n");
        exit(1);
    }
}

$manifest = stage34_text($root . '/Cargo.toml');
$lock = stage34_text($root . '/Cargo.lock');
$analysis = stage34_text($root . '/server/src/analysis.rs');
$index = stage34_text($root . '/server/src/workspace_index.rs');
$server = stage34_text($root . '/server/src/lib.rs');
$vscodeGrammar = stage34_text($root . '/editors/vscode/doria/syntaxes/doria.tmLanguage.json');
$vscodeTest = stage34_text($root . '/editors/vscode/doria/test/inheritance-grammar.test.js');
$intellijTest = stage34_text($root . '/editors/intellij/doria/src/test/kotlin/dev/doria/intellij/highlighting/DoriaLexerTest.kt');
$fixture = stage34_text($root . '/editors/fixtures/latest-tokens.doria');
$docs = stage34_text($root . '/README.md')
    . stage34_text($root . '/CHANGELOG.md')
    . stage34_text($root . '/docs/architecture.md')
    . stage34_text($root . '/docs/semantic-hover.md')
    . stage34_text($root . '/server/README.md')
    . stage34_text($root . '/editors/vscode/doria/README.md')
    . stage34_text($root . '/editors/intellij/doria/README.md');

preg_match('/doriac\s*=\s*\{[^\n]*\brev\s*=\s*"([0-9a-f]{40})"/', $manifest, $pin);
stage34_require(
    ($pin[1] ?? null) === $expectedCompiler,
    'Cargo.toml must pin the final Stage 34 compiler commit.',
);
stage34_require(
    substr_count($lock, 'rev=' . $expectedCompiler . '#' . $expectedCompiler) >= 3,
    'Cargo.lock must resolve every compiler package to the final Stage 34 commit.',
);

foreach ([
    'info.class_hierarchy',
    'info.method_hierarchy',
    'info.method_call_targets',
    'HierarchyClass',
    'HierarchyMember',
    'virtual_root',
    'overridden_declaration',
    'hierarchy_semantic_tokens',
] as $fact) {
    stage34_require(str_contains($analysis, $fact), "compiler hierarchy projection is missing {$fact}.");
}

foreach ([
    'open_class_completions',
    'override_completions',
    'parent_completions',
    'member_occurrence_matches_target',
    'incomplete_packages',
] as $fact) {
    stage34_require(str_contains($index, $fact), "hierarchy index support is missing {$fact}.");
}

foreach ([
    'stage34_completion_uses_compiler_owned_open_class_facts',
    'stage34_override_completion_preserves_contract_and_omits_defaults',
    'stage34_parent_completion_respects_instance_static_and_lifecycle_contexts',
    'stage34_hover_explains_hierarchy_and_direct_parent_dispatch',
    'stage34_definition_keeps_exact_and_inherited_targets_distinct',
    'stage34_virtual_references_and_rename_cover_the_complete_slot_family',
    'stage34_virtual_rename_preserves_graph_and_source_edit_safety',
    'stage34_cross_file_hierarchy_uses_one_compiler_graph',
    'stage34_cross_package_hierarchy_preserves_navigation_and_edit_safety',
    'stage34_semantic_tokens_and_utf16_diagnostics_remain_compiler_owned',
    'stage34_compiler_diagnostics_and_incremental_refresh_do_not_go_stale',
    'SourceEditPolicy::Generated',
    'SourceEditPolicy::DependencyCache',
] as $coverage) {
    stage34_require(str_contains($server, $coverage), "Stage 34 tooling coverage is missing {$coverage}.");
}

$production = explode("\n#[cfg(test)]\nmod tests", $analysis, 2)[0]
    . explode("\n#[cfg(test)]\nmod tests", $server, 2)[0]
    . $index;
foreach (['parse_inheritance', 'build_class_hierarchy', 'validate_override', 'resolve_virtual_slot'] as $forbidden) {
    stage34_require(
        !str_contains($production, $forbidden),
        "the language server must not implement a second hierarchy authority: {$forbidden}.",
    );
}

foreach (['open', 'override', 'extends', 'parent'] as $word) {
    stage34_require(str_contains($fixture, $word), "shared Stage 34 fixture is missing {$word}.");
    stage34_require(str_contains($vscodeTest, '"' . $word . '"'), "VS Code Stage 34 test is missing {$word}.");
}
stage34_require(
    str_contains($vscodeGrammar, 'storage.modifier.inheritance.doria'),
    'VS Code must present open and override as inheritance modifiers.',
);
stage34_require(
    str_contains($intellijTest, 'testStage34InheritancePresentationUsesAcceptedKeywordAndTypeTokens'),
    'IntelliJ must retain Stage 34 inheritance presentation coverage.',
);

foreach ([
    'Stage 34 single class inheritance is complete',
    'Stage 35 interfaces and traits are next',
    'compiler-owned class hierarchy',
    'does not parse or check inheritance independently',
] as $fact) {
    stage34_require(str_contains($docs, $fact), "Stage 34 tooling documentation is missing {$fact}.");
}
foreach (['Stage 34 single class inheritance is next', 'Stage 34 is next'] as $stale) {
    stage34_require(!str_contains($docs, $stale), "stale Stage 34 boundary remains: {$stale}.");
}

fwrite(STDOUT, "Stage 34 inheritance tooling guard passed.\n");
