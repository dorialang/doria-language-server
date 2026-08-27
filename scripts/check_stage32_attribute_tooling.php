#!/usr/bin/env php
<?php

declare(strict_types=1);

$root = dirname(__DIR__);

function stage32_read(string $path): string
{
    $contents = file_get_contents($path);
    if ($contents === false) {
        fwrite(STDERR, "ERROR: could not read {$path}.\n");
        exit(1);
    }
    return $contents;
}

function stage32_require(bool $condition, string $message): void
{
    if (!$condition) {
        fwrite(STDERR, "ERROR: {$message}\n");
        exit(1);
    }
}

$compilerCommit = '5adddf0d283f20f171444518e6f2d889ca59437f';
$manifest = stage32_read($root . '/Cargo.toml');
$lock = stage32_read($root . '/Cargo.lock');
$analysis = stage32_read($root . '/server/src/analysis.rs');
$server = stage32_read($root . '/server/src/lib.rs');
$index = stage32_read($root . '/server/src/workspace_index.rs');
$accepted = stage32_read($root . '/editors/fixtures/stage32-attributes.doria');
$rejected = stage32_read($root . '/editors/fixtures/stage32-attributes-rejected.doria');
$grammar = stage32_read($root . '/editors/vscode/doria/syntaxes/doria.tmLanguage.json');
$vscodeTests = stage32_read($root . '/editors/vscode/doria/test/attribute-grammar.test.js');
$intellijLexer = stage32_read($root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/highlighting/DoriaLexer.kt');
$intellijTests = stage32_read($root . '/editors/intellij/doria/src/test/kotlin/dev/doria/intellij/highlighting/DoriaLexerTest.kt');
$docs = stage32_read($root . '/README.md')
    . stage32_read($root . '/server/README.md')
    . stage32_read($root . '/docs/architecture.md')
    . stage32_read($root . '/docs/semantic-hover.md');

stage32_require(
    str_contains($manifest, 'rev = "' . $compilerCommit . '"'),
    'Cargo.toml must pin the final Stage 32 compiler commit.',
);
preg_match_all(
    '/github\.com\/dorialang\/doria\?rev=([0-9a-f]{40})#([0-9a-f]{40})/',
    $lock,
    $sources,
    PREG_SET_ORDER,
);
stage32_require(count($sources) >= 3, 'Cargo.lock must contain compiler-owned git packages.');
foreach ($sources as $source) {
    stage32_require(
        $source[1] === $compilerCommit && $source[2] === $compilerCommit,
        'every compiler-owned lockfile package must resolve to the Stage 32 pin.',
    );
}

foreach ([
    'AttributeSemanticInfo',
    'info.attributes.clone()',
    'collect_attribute_facts',
    'attribute_application_documentation',
    'attribute_schema_documentation',
] as $fact) {
    stage32_require(str_contains($analysis, $fact), "analysis is missing compiler-backed {$fact}.");
}
foreach ([
    'AttributeParameterIdentity',
    'attribute_completions',
    'attribute_argument_completions',
    'GlobalReferenceRole::AttributeClass',
    'CompilerKnownAttribute',
] as $fact) {
    stage32_require(str_contains($index, $fact), "workspace index is missing {$fact}.");
}
foreach ([
    'stage32_attribute_completion_context_uses_compiler_lexing_boundaries',
    'stage32_completion_is_scoped_to_visible_attribute_schemas_and_parameters',
    'stage32_hovers_navigation_references_and_rename_share_compiler_identities',
    'stage32_semantic_tokens_and_diagnostics_remain_compiler_owned_and_incremental',
] as $test) {
    stage32_require(str_contains($server, $test), "server coverage is missing {$test}.");
}
stage32_require(
    str_contains($server, 'doriac::lexer::Lexer::new')
        && !str_contains($server . $analysis . $index, 'fn parse_attribute')
        && !str_contains($server . $analysis . $index, 'struct AttributeParser')
        && !str_contains($server . $analysis . $index, 'fn evaluate_attribute'),
    'tooling must consume compiler lexing and facts without a second parser or evaluator.',
);

foreach (['#[Attribute]', '#[Test]', '#[PHPExport]', '# ordinary hash comment', '# [Test]'] as $source) {
    stage32_require(str_contains($accepted, $source), "accepted fixture is missing {$source}.");
}
foreach (['#[]', '#[Missing]', '#[Ordinary]'] as $source) {
    stage32_require(str_contains($rejected, $source), "rejected fixture is missing {$source}.");
}
stage32_require(
    str_contains($grammar, 'meta.attribute.doria')
        && str_contains($grammar, 'comment.line.number-sign.doria')
        && str_contains($vscodeTests, 'doesNotMatch("#[Test]"'),
    'VS Code must preserve the adjacent attribute/hash-comment boundary.',
);
stage32_require(
    str_contains($intellijLexer, 'MODE_ATTRIBUTE')
        && str_contains($intellijTests, 'testStage32AttributePresentationPreservesHashCommentsAndMalformedBoundaries')
        && !str_contains($intellijLexer, 'AttributeParser'),
    'IntelliJ must remain a presentation-only attribute client.',
);

foreach ([
    'Stage 32 is complete',
    'Stage 33 project integration is next',
    'compiler-owned',
    'no runtime reflection',
] as $fact) {
    stage32_require(str_contains(strtolower($docs), strtolower($fact)), "documentation is missing {$fact}.");
}

fwrite(STDOUT, "Stage 32 attribute tooling guard passed.\n");
