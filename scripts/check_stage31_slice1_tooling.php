#!/usr/bin/env php
<?php

declare(strict_types=1);

const COMPILER_REVISION = '8cfa433aa92dfdd1e3c6f1f7267f936ab5db04ee';

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
$graph = read_required($root . '/server/src/workspace_graph.rs');
$index = read_required($root . '/server/src/workspace_index.rs');
$accepted = read_required($root . '/editors/fixtures/latest-tokens.doria');
$rejected = read_required($root . '/editors/fixtures/rejected-syntax.doria');
$vscode = read_required($root . '/editors/vscode/doria/syntaxes/doria.tmLanguage.json');
$vscodeTests = read_required($root . '/editors/vscode/doria/test/internal-declarations.test.js');
$intellij = read_required($root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/highlighting/DoriaLexer.kt');
$intellijTests = read_required($root . '/editors/intellij/doria/src/test/kotlin/dev/doria/intellij/highlighting/DoriaLexerTest.kt');
$docs = read_required($root . '/README.md')
    . read_required($root . '/server/README.md')
    . read_required($root . '/docs/architecture.md')
    . read_required($root . '/docs/semantic-hover.md');

require_fact(
    str_contains($manifest, 'rev = "' . COMPILER_REVISION . '"'),
    'Cargo.toml must pin the final Stage 31 compiler commit.',
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

foreach (['from_graph_source', 'CompilationContext', 'global_symbols', 'directive_semantic_tokens'] as $fact) {
    require_fact(str_contains($analysis, $fact), "analysis snapshot is missing {$fact}.");
}
foreach ([
    'CompilationSession',
    'OpenDocumentIndex',
    'max_by_key',
    'did_change_workspace_folders',
    'reanalyze_documents',
    'publish_all_diagnostics',
    'graph_diagnostic_to_lsp',
    'textDocument/definition',
] as $fact) {
    require_fact(str_contains($server, $fact), "server is missing {$fact}.");
}
foreach ([
    'GraphLoadOptions',
    'GraphCompleteness::Partial',
    'ProjectStructureAuthority::Unavailable',
    'InMemorySourceProvider',
    'load_graph_with_options',
    'analyze_graph',
    'IncrementalFacts',
    'include_edges',
] as $fact) {
    require_fact(str_contains($graph, $fact), "open-document graph is missing {$fact}.");
}
foreach ([
    'GlobalSymbolId',
    'AliasIdentity',
    'AliasDeclaration',
    'implicit_imports',
    'declaration_counts',
    'incomplete_packages',
    'definition',
] as $fact) {
    require_fact(str_contains($index, $fact), "open-document index is missing {$fact}.");
}
require_fact(
    !str_contains($index, 'package_symbol')
        && !str_contains($index, 'GlobalSymbolOwner::Package(package.clone())'),
    'the presentation index must not fabricate identities for unresolved symbols.',
);
require_fact(
    !str_contains($server . $analysis . $index . $graph, 'Baton.toml')
        && !str_contains($server . $analysis . $index . $graph, 'read_dir(')
        && !str_contains($server . $analysis . $index . $graph, 'WalkDir'),
    'Stage 31 tooling must not read manifests or discover unopened files.',
);

foreach ([
    'namespace Acme\\Editor;',
    'use Acme\\Model\\User;',
    'use Acme\\Http\\Client as HttpClient;',
    'use Doria\\Std\\Math\\{',
    'Vector3 as Position3,',
    'include "generated/routes.doria";',
    'internal class PackageHelper',
    'internal enum PackageState',
    'internal interface PackageContract',
    'internal trait PackageSupport',
    'internal function packageHelper',
    'internal const int PACKAGE_LIMIT',
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
foreach (['meta.import.doria', 'punctuation.section.import.group.begin.doria', 'storage.modifier.visibility.doria'] as $scope) {
    require_fact(str_contains($vscode, $scope), "VS Code grammar is missing {$scope}.");
}
require_fact(
    str_contains($vscodeTests, 'top-level internal declarations'),
    'VS Code must test top-level internal declarations.',
);
foreach (['NAMESPACE_DECLARATION_LINE', 'isAcceptedImportStatement', 'IMPORT_GROUP_ENTRY'] as $fact) {
    require_fact(str_contains($intellij, $fact), "IntelliJ lexer is missing {$fact}.");
}
require_fact(
    str_contains($intellijTests, 'testTopLevelInternalDeclarationsKeepModifierAndDeclarationHighlighting'),
    'IntelliJ must test top-level internal declarations.',
);

foreach ([
    'stage31_open_document_index_uses_canonical_compiler_identity',
    'stage31_cross_document_ranges_are_utf16_safe',
    'stage31_slice2_graph_drives_cross_file_definition_and_global_kinds',
    'stage31_slice2_duplicate_diagnostics_publish_both_sources_and_decline_rename',
    'stage31_slice2_session_invalidates_changes_and_removes_closed_sources',
    'stage31_slice2_open_graph_tracks_includes_without_scanning_or_baton',
    'stage31_slice2_cross_file_checked_effects_and_closure_types_are_semantic',
    'stage31_slice2_partial_inputs_remain_compiler_facts_not_index_guesses',
    'stage31_internal_declaration_hover_preserves_access_and_signature',
] as $test) {
    require_fact(str_contains($server, $test), "Stage 31 test coverage is missing {$test}.");
}

require_fact(
    str_contains($docs, 'Stage 31 is complete')
        && str_contains($docs, 'Stage 32 attributes are next')
        && str_contains($docs, 'Stage 33 project integration remains scheduled'),
    'tooling documents must preserve the final Stage 31 status and boundaries.',
);

fwrite(STDOUT, "Stage 31 tooling guard passed.\n");
