#!/usr/bin/env php
<?php

declare(strict_types=1);

require_once __DIR__ . '/compiler_pin.php';

$root = dirname(__DIR__);

function indexed_foreach_text(string $path): string
{
    $text = file_get_contents($path);
    if ($text === false) {
        fwrite(STDERR, "ERROR: could not read {$path}.\n");
        exit(1);
    }
    return $text;
}

function indexed_foreach_require(bool $condition, string $message): void
{
    if (!$condition) {
        fwrite(STDERR, "ERROR: {$message}\n");
        exit(1);
    }
}

$manifest = indexed_foreach_text($root . '/Cargo.toml');
$lock = indexed_foreach_text($root . '/Cargo.lock');
$analysis = indexed_foreach_text($root . '/server/src/analysis.rs');
$server = indexed_foreach_text($root . '/server/src/lib.rs');
$lspTests = indexed_foreach_text($root . '/server/tests/lsp_tests.rs');
$accepted = indexed_foreach_text($root . '/editors/fixtures/indexed-foreach.doria');
$rejected = indexed_foreach_text($root . '/editors/fixtures/indexed-foreach-rejected.doria');
$vscodeGrammar = indexed_foreach_text($root . '/editors/vscode/doria/syntaxes/doria.tmLanguage.json');
$vscodeTest = indexed_foreach_text($root . '/editors/vscode/doria/test/indexed-foreach-grammar.test.js');
$intellijTest = indexed_foreach_text(
    $root . '/editors/intellij/doria/src/test/kotlin/dev/doria/intellij/highlighting/DoriaLexerTest.kt',
);
$docs = indexed_foreach_text($root . '/README.md')
    . indexed_foreach_text($root . '/CHANGELOG.md')
    . indexed_foreach_text($root . '/docs/architecture.md')
    . indexed_foreach_text($root . '/docs/semantic-hover.md')
    . indexed_foreach_text($root . '/server/README.md')
    . indexed_foreach_text($root . '/editors/vscode/doria/README.md')
    . indexed_foreach_text($root . '/editors/intellij/doria/README.md');

$expectedCompiler = doria_compiler_revision($manifest);
indexed_foreach_require(
    $expectedCompiler !== null,
    'Cargo.toml must pin an exact 40-character compiler revision.',
);
indexed_foreach_require(
    doria_lock_resolves_revision($lock, $expectedCompiler),
    'Cargo.lock must resolve every Doria package to the current manifest revision.',
);

foreach ([
    'foreach_loops',
    'ForeachIterationKind::SequenceIndex',
    'ForeachIterationKind::DictionaryKey',
    'ForeachValueAccess::Readonly',
    'ForeachValueAccess::Writable',
    'BindingKind::ForeachFirst',
    'BindingKind::ForeachValue',
    'foreach_binding_hovers',
    'foreach_binding_semantic_tokens',
    'Zero-Based Sequence Index',
    'Dictionary Key',
] as $fact) {
    indexed_foreach_require(str_contains($analysis, $fact), "compiler-fact projection is missing {$fact}.");
}

foreach ([
    'indexed_foreach_protocol_preserves_roles_hovers_and_variable_tokens',
    'indexed_foreach_diagnostics_ranges_and_actions_remain_compiler_owned',
    'indexed_foreach_property_facts_refresh_across_unsaved_files',
    'scalar_display_tooling_preserves_materialization_without_invented_apis',
    'stage34_cross_package_hierarchy_preserves_navigation_and_edit_safety',
] as $coverage) {
    indexed_foreach_require(str_contains($server, $coverage), "tooling coverage is missing {$coverage}.");
}
indexed_foreach_require(
    str_contains($lspTests, 'explicit_foreach_binding_types_forward_utf16_safe_compiler_fixes'),
    'tooling coverage must forward Decision 0133 diagnostics and fixes through the public LSP boundary.',
);

$production = explode("\n#[cfg(test)]\nmod tests", $analysis, 2)[0]
    . explode("\n#[cfg(test)]\nmod tests", $server, 2)[0];
foreach (['parse_foreach_binding', 'infer_foreach_role', 'validate_foreach_binding'] as $forbidden) {
    indexed_foreach_require(
        !str_contains($production, $forbidden),
        "the language server must not implement a second foreach checker: {$forbidden}.",
    );
}
indexed_foreach_require(
    !str_contains($production, 'M1101'),
    'valid indexed List and typed-array tooling must not maintain an M1101 path.',
);
foreach (['Int::toString', 'Float::toString', 'String::from(int', 'String::from(float'] as $invented) {
    indexed_foreach_require(
        !str_contains($analysis . $server, $invented),
        "tooling must not invent scalar conversion API {$invented}.",
    );
}

foreach ([
    'foreach ($this->contents as int $line => string $content)',
    'foreach ($contents as int $index => string $content)',
    'foreach ($counts as string $name => int $count)',
] as $fixture) {
    indexed_foreach_require(str_contains($accepted, $fixture), "accepted fixture is missing {$fixture}.");
}
foreach ([
    'foreach ($labels as int $index => string $label)',
    'foreach (0..<2 as int $index => int $value)',
] as $fixture) {
    indexed_foreach_require(str_contains($rejected, $fixture), "rejected fixture is missing {$fixture}.");
}
indexed_foreach_require(
    str_contains($vscodeTest, 'keeps indexed foreach roles in compiler semantics')
        && !str_contains($vscodeGrammar, 'Zero-Based Sequence Index'),
    'VS Code must test fixtures without encoding foreach semantics in TextMate.',
);
indexed_foreach_require(
    str_contains($intellijTest, 'testIndexedForeachFixturesKeepRolesInCompilerSemantics'),
    'IntelliJ must test accepted and semantic-invalid indexed foreach fixtures.',
);

foreach ([
    'post-Stage-34 indexed-foreach and scalar-display correction is implemented',
    'Decision 0133',
    'every foreach binding',
    'explicitly typed',
    'compiler-owned foreach semantic facts',
    'Zero-Based Sequence Index',
    'Dictionary Key',
    'no primitive `toString` or scalar-cast completion',
    'Stage 35 remains next',
    'property hooks remain future work',
] as $fact) {
    indexed_foreach_require(str_contains($docs, $fact), "tooling documentation is missing {$fact}.");
}

indexed_foreach_require(
    str_contains($server . $lspTests, 'E0748')
        && !str_contains($accepted, 'foreach ($contents as $index => $content)'),
    'omitted foreach binding types must remain rejected with compiler-owned E0748 fixes.',
);

foreach (glob($root . '/scripts/check_*.php') ?: [] as $guardPath) {
    indexed_foreach_require(
        preg_match('/\b[0-9a-f]{40}\b/', indexed_foreach_text($guardPath)) !== 1,
        basename($guardPath) . ' must derive the current compiler revision instead of hard-coding a stale pin.',
    );
}

indexed_foreach_require(
    !str_contains($server, 'fn stage35_'),
    'Stage 35 implementation must remain absent during this corrective beat.',
);
indexed_foreach_require(
    !str_contains($production, 'PropertyHook'),
    'property-hook implementation must remain absent during this corrective beat.',
);

fwrite(STDOUT, "Indexed foreach and scalar display tooling guard passed.\n");
