#!/usr/bin/env php
<?php

declare(strict_types=1);

require_once __DIR__ . '/compiler_pin.php';

$root = dirname(__DIR__);

function constructor_role_text(string $path): string
{
    $text = file_get_contents($path);
    if ($text === false) {
        fwrite(STDERR, "ERROR: could not read {$path}.\n");
        exit(1);
    }
    return $text;
}

function constructor_role_require(bool $condition, string $message): void
{
    if (!$condition) {
        fwrite(STDERR, "ERROR: {$message}\n");
        exit(1);
    }
}

$manifest = constructor_role_text($root . '/Cargo.toml');
$lock = constructor_role_text($root . '/Cargo.lock');
$analysis = constructor_role_text($root . '/server/src/analysis.rs');
$server = constructor_role_text($root . '/server/src/lib.rs');
$index = constructor_role_text($root . '/server/src/workspace_index.rs');
$fixture = constructor_role_text($root . '/editors/fixtures/latest-tokens.doria');
$rejectedFixture = constructor_role_text($root . '/editors/fixtures/rejected-syntax.doria');
$vscodeGrammar = constructor_role_text($root . '/editors/vscode/doria/syntaxes/doria.tmLanguage.json');
$vscodeTest = constructor_role_text($root . '/editors/vscode/doria/test/inheritance-grammar.test.js');
$intellijLexer = constructor_role_text($root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/highlighting/DoriaLexer.kt');
$intellijTest = constructor_role_text($root . '/editors/intellij/doria/src/test/kotlin/dev/doria/intellij/highlighting/DoriaLexerTest.kt');
$docs = constructor_role_text($root . '/README.md')
    . constructor_role_text($root . '/CHANGELOG.md')
    . constructor_role_text($root . '/docs/architecture.md')
    . constructor_role_text($root . '/docs/semantic-hover.md')
    . constructor_role_text($root . '/server/README.md')
    . constructor_role_text($root . '/editors/vscode/doria/README.md')
    . constructor_role_text($root . '/editors/intellij/doria/README.md');

$expectedCompiler = doria_compiler_revision($manifest);
constructor_role_require(
    $expectedCompiler !== null,
    'Cargo.toml must pin an exact 40-character compiler revision.',
);
constructor_role_require(
    doria_compiler_revision_is_authoritative($expectedCompiler),
    'Cargo.toml must retain the centrally recorded compiler authority revision.',
);
constructor_role_require(
    doria_lock_resolves_revision($lock, $expectedCompiler),
    'Cargo.lock must resolve every compiler package to the current manifest revision.',
);

foreach ([
    'ConstructorParameterSemanticRole',
    'semantic_info.constructor_parameters',
    'property_families',
    'collect_constructor_parameter_roles',
    'constructor_parameter_documentation',
    'named_argument_completions_at_offset',
    'resolve_named_argument_references',
] as $fact) {
    constructor_role_require(str_contains($analysis, $fact), "compiler-fact projection is missing {$fact}.");
}

foreach ([
    'constructor_parameter_role_completion_context',
    'constructor_parameter_roles_drive_tokens_hovers_signatures_and_completion',
    'constructor_parameter_roles_keep_local_and_property_identities_distinct',
    'constructor_parameter_role_diagnostics_actions_and_utf16_ranges_are_compiler_owned',
    'SourceEditPolicy::Generated',
    'SourceEditPolicy::DependencyCache',
] as $coverage) {
    constructor_role_require(str_contains($server, $coverage), "constructor-role tooling coverage is missing {$coverage}.");
}
constructor_role_require(
    str_contains($index, 'relationship_only') && str_contains($index, 'return None;'),
    'property-family rename must refuse a relationship-only partial edit.',
);

$production = explode("\n#[cfg(test)]\nmod tests", $analysis, 2)[0]
    . explode("\n#[cfg(test)]\nmod tests", $server, 2)[0]
    . $index;
foreach (['validate_constructor_role', 'infer_constructor_promotion', 'build_property_family'] as $forbidden) {
    constructor_role_require(
        !str_contains($production, $forbidden),
        "the language server must not add a second constructor-role checker: {$forbidden}.",
    );
}

foreach (['override string $title', 'parameter string $rawTitle'] as $accepted) {
    constructor_role_require(str_contains($fixture, $accepted), "accepted editor fixture is missing {$accepted}.");
}
foreach (['parameter parameter', 'internal parameter', 'function invalidParameterRole(parameter'] as $rejected) {
    constructor_role_require(str_contains($rejectedFixture, $rejected), "rejected editor fixture is missing {$rejected}.");
}
constructor_role_require(
    str_contains($vscodeGrammar, 'storage.modifier.parameter-role.doria')
        && str_contains($vscodeTest, 'constructor parameter roles'),
    'VS Code must present and test the accepted parameter role.',
);
constructor_role_require(
    str_contains($intellijLexer, '"parameter"')
        && str_contains($intellijTest, 'testConstructorParameterRolesUseAcceptedKeywordTokens'),
    'IntelliJ must present and test the accepted parameter role.',
);

foreach ([
    'post-Stage-34 constructor-parameter-role correction',
    'compiler-owned constructor-role and property-family facts',
    'does not infer property promotion',
    'Stage 35 interfaces and traits are next',
] as $fact) {
    constructor_role_require(str_contains($docs, $fact), "constructor-role tooling documentation is missing {$fact}.");
}
constructor_role_require(
    !str_contains($server, 'fn stage35_'),
    'Stage 35 implementation must remain absent during this corrective beat.',
);

fwrite(STDOUT, "Constructor parameter roles tooling guard passed.\n");
