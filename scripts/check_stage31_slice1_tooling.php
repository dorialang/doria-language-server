#!/usr/bin/env php
<?php

declare(strict_types=1);

const COMPILER_REVISION = '48d8351d364864640fda1871ec9cd45ba5c5d65e';

$root = dirname(__DIR__);

function read_required(string $path): string
{
    $text = file_get_contents($path);
    if ($text === false) {
        fwrite(STDERR, "ERROR: could not read {$path}.\n");
        exit(1);
    }
    return $text;
}

function require_fact(bool $condition, string $message): void
{
    if (!$condition) {
        fwrite(STDERR, "ERROR: {$message}\n");
        exit(1);
    }
}

$manifest = read_required($root . '/Cargo.toml');
$lock = read_required($root . '/Cargo.lock');
$analysis = read_required($root . '/server/src/analysis.rs');
$server = read_required($root . '/server/src/lib.rs');
$index = read_required($root . '/server/src/workspace_index.rs');
$accepted = read_required($root . '/editors/fixtures/latest-tokens.doria');
$rejected = read_required($root . '/editors/fixtures/rejected-syntax.doria');
$vscode = read_required($root . '/editors/vscode/doria/syntaxes/doria.tmLanguage.json');
$intellij = read_required($root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/highlighting/DoriaLexer.kt');
$docs = read_required($root . '/README.md')
    . read_required($root . '/server/README.md')
    . read_required($root . '/docs/architecture.md')
    . read_required($root . '/docs/semantic-hover.md');

require_fact(
    str_contains($manifest, 'rev = "' . COMPILER_REVISION . '"'),
    'Cargo.toml must pin the final Stage 31 Slice 1 compiler commit.',
);
preg_match_all(
    '/github\.com\/dorialang\/doria\?rev=([0-9a-f]{40})#([0-9a-f]{40})/',
    $lock,
    $sources,
    PREG_SET_ORDER,
);
require_fact(count($sources) >= 3, 'Cargo.lock must contain the compiler-owned git packages.');
foreach ($sources as $source) {
    require_fact(
        $source[1] === COMPILER_REVISION && $source[2] === COMPILER_REVISION,
        'Cargo.lock must resolve every compiler-owned package to the exact pin.',
    );
}

foreach ([
    'analyze_source_for_ide_with_context',
    'CompilationContext',
    'global_symbols',
    'directive_semantic_tokens',
] as $fact) {
    require_fact(str_contains($analysis, $fact), "analysis snapshot is missing {$fact}.");
}
foreach ([
    'OpenDocumentIndex',
    'SyntheticTooling',
    'max_by_key',
    'did_change_workspace_folders',
    'rebuild_document_index',
] as $fact) {
    require_fact(str_contains($server, $fact), "server is missing {$fact}.");
}
foreach ([
    'GlobalSymbolId',
    'AliasIdentity',
    'AliasDeclaration',
    'implicit_imports',
    'declaration_counts',
] as $fact) {
    require_fact(str_contains($index, $fact), "open-document index is missing {$fact}.");
}
require_fact(
    !str_contains($server . $analysis . $index, 'Baton.toml')
        && !str_contains($server . $analysis . $index, 'read_dir(')
        && !str_contains($server . $analysis . $index, 'WalkDir'),
    'Stage 31 Slice 1 tooling must not read manifests or discover unopened files.',
);

foreach ([
    'namespace Acme\\Editor;',
    'use Acme\\Model\\User;',
    'use Acme\\Http\\Client as HttpClient;',
    'use Doria\\Std\\Math\\{',
    'Vector3 as Position3,',
    'include "generated/routes.doria";',
] as $snippet) {
    require_fact(str_contains($accepted, $snippet), "accepted fixture is missing {$snippet}.");
}
foreach ([
    'namespace Rejected {',
    'use \\Acme\\Http\\Client;',
    'use Acme\\*;',
    'use Acme\\{};',
    'use function Acme\\makeValue;',
    'use const Acme\\LIMIT;',
    'include $path;',
    'include "generated/{$name}.doria";',
] as $snippet) {
    require_fact(str_contains($rejected, $snippet), "rejected fixture is missing {$snippet}.");
}
foreach ([
    'meta.import.doria',
    'punctuation.section.import.group.begin.doria',
    'punctuation.section.import.group.end.doria',
] as $scope) {
    require_fact(str_contains($vscode, $scope), "VS Code grammar is missing {$scope}.");
}
foreach ([
    'NAMESPACE_DECLARATION_LINE',
    'isAcceptedImportStatement',
    'IMPORT_GROUP_ENTRY',
] as $fact) {
    require_fact(str_contains($intellij, $fact), "IntelliJ lexer is missing {$fact}.");
}

foreach ([
    'stage31_open_document_index_uses_canonical_compiler_identity',
    'stage31_index_tracks_classes_functions_and_constants_by_canonical_identity',
    'stage31_workspace_context_uses_the_longest_root_and_never_the_namespace',
    'stage31_rename_declines_ambiguous_and_implicit_alias_edits',
    'stage31_reindexing_removes_changed_and_closed_occurrences',
    'stage31_cross_document_ranges_are_utf16_safe',
    'stage31_open_document_index_scales_structurally_and_deterministically',
] as $test) {
    require_fact(str_contains($server, $test), "Stage 31 test coverage is missing {$test}.");
}

require_fact(
    str_contains($docs, 'Stage 31 Slice 1 is complete')
        && str_contains($docs, 'Stage 31 Slice 2 is next')
        && str_contains($docs, 'Stage 31 remains in progress'),
    'tooling documents must preserve the exact Stage 31 status.',
);

fwrite(STDOUT, "Stage 31 Slice 1 tooling guard passed.\n");
