#!/usr/bin/env php
<?php

declare(strict_types=1);

$root = dirname(__DIR__);

function stage33_text(string $path): string
{
    $text = file_get_contents($path);
    if ($text === false) {
        fwrite(STDERR, "ERROR: could not read {$path}.\n");
        exit(1);
    }
    return $text;
}

function stage33_require(bool $condition, string $message): void
{
    if (!$condition) {
        fwrite(STDERR, "ERROR: {$message}\n");
        exit(1);
    }
}

$manifest = stage33_text($root . '/Cargo.toml');
$lock = stage33_text($root . '/Cargo.lock');
$server = stage33_text($root . '/server/src/lib.rs');
$discovery = stage33_text($root . '/server/src/baton_discovery.rs');
$project = stage33_text($root . '/server/src/project.rs');
$graph = stage33_text($root . '/server/src/workspace_graph.rs');
$vscode = stage33_text($root . '/editors/vscode/doria/extension.js');
$vscodeManifest = stage33_text($root . '/editors/vscode/doria/package.json');
$intellijSettings = stage33_text($root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/settings/DoriaSettings.kt');
$intellijDescriptor = stage33_text($root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/lsp/DoriaLspServerDescriptor.kt');
$docs = stage33_text($root . '/README.md')
    . stage33_text($root . '/server/README.md')
    . stage33_text($root . '/docs/architecture.md')
    . stage33_text($root . '/docs/semantic-hover.md');

$compiler = 'f619d3dc175c1a671504fea3aff3613c61b05151';
stage33_require(str_contains($manifest, 'rev = "' . $compiler . '"'), 'doriac must use the final green Stage 33 compiler pin.');
stage33_require(substr_count($lock, 'rev=' . $compiler . '#' . $compiler) >= 3, 'Cargo.lock must resolve every compiler-owned package to the final pin.');

foreach (['"project"', '"--json"', '"--workspace"', '"--development"', '"--offline"'] as $argument) {
    stage33_require(str_contains($discovery, $argument), "Baton discovery is missing {$argument}.");
}
foreach (['thread::spawn', 'recv_timeout', 'cancelled', 'PROJECT_TIMEOUT', 'MAX_STDOUT', 'MAX_STDERR'] as $fact) {
    stage33_require(str_contains($discovery . $server, $fact), "asynchronous discovery is missing {$fact}.");
}
stage33_require(
    str_contains($discovery, 'Workspace Package Selection Is Unavailable')
        && str_contains($discovery, 'DORIA_BATON_PATH')
        && str_contains($discovery, 'current_executable'),
    'Baton fallback and installed-component resolution must remain exact.',
);
stage33_require(
    str_contains($project, 'deny_unknown_fields')
        && str_contains($project, 'schema_version != 1')
        && str_contains($project, 'doriac::BUILD_COMMIT')
        && str_contains($project, 'differ from the tooling build plan')
        && str_contains($project, 'SourceEditPolicy::Generated')
        && str_contains($project, 'PackageSource::Git => SourceEditPolicy::DependencyCache'),
    'strict project schema and readonly source policy are incomplete.',
);
stage33_require(
    str_contains($graph, 'GraphCompleteness::Complete')
        && str_contains($graph, 'OverlaySourceProvider')
        && str_contains($graph, 'open_uri_by_path')
        && str_contains($graph, 'FileSystemSourceProvider'),
    'complete project graph or unsaved overlay support is missing.',
);
foreach (['project_documents', 'analyze_project_graph', 'source_is_editable', 'doria.refreshProject', 'didChangeWatchedFiles'] as $fact) {
    stage33_require(str_contains($server, $fact), "server project integration is missing {$fact}.");
}
stage33_require(
    str_contains($server, 'published_diagnostics')
        && str_contains($server, '.all_documents()')
        && str_contains($server, 'replace_project_watchers')
        && str_contains($server, 'client/unregisterCapability')
        && str_contains($server, '"baseUri": base_uri'),
    'unopened diagnostics or exact inventory-root watcher lifecycle is missing.',
);
stage33_require(
    preg_match('/\btoml\s*=/', $manifest) !== 1
        && !str_contains($project . $discovery, 'toml::')
        && !str_contains($project . $discovery, 'tomlj'),
    'the language server must consume Baton JSON instead of parsing manifests.',
);
stage33_require(
    str_contains($vscode, 'DORIA_BATON_PATH: batonPath')
        && str_contains($vscode, 'createFileSystemWatcher')
        && str_contains($vscode, 'dynamicProjectWatchers')
        && str_contains($vscode, 'dynamicRegistration: true')
        && str_contains($vscode, 'client/unregisterCapability')
        && str_contains($vscodeManifest, 'doria.refreshProject'),
    'VS Code Baton override, watcher, or refresh command is missing.',
);
stage33_require(
    str_contains($intellijSettings, 'batonPath')
        && str_contains($intellijDescriptor, 'DORIA_BATON_PATH'),
    'IntelliJ Baton override is missing.',
);
foreach (['Stage 33 and Phase F are complete', 'Stage 34 single class inheritance', 'never parses `Baton.toml`'] as $fact) {
    stage33_require(str_contains($docs, $fact), "Stage 33 documentation is missing {$fact}.");
}
foreach (['stage33_project_graph_indexes_unopened_sources_and_overlays_open_buffers', 'project_inventory_replaces_watchers_with_exact_package_roots', 'generated_and_git_sources_are_read_only', 'scheduling_is_non_blocking_debounced_and_cancels_superseded_work'] as $coverage) {
    stage33_require(str_contains($server . $project . $discovery, $coverage), "Stage 33 regression coverage is missing {$coverage}.");
}

fwrite(STDOUT, "Stage 33 project tooling guard passed.\n");
