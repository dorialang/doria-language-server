#!/usr/bin/env php
<?php

declare(strict_types=1);

$root = dirname(__DIR__);

function ambient_read(string $path): string
{
    $contents = file_get_contents($path);
    if ($contents === false) {
        fwrite(STDERR, "ERROR: could not read {$path}.\n");
        exit(1);
    }
    return $contents;
}

function ambient_require(bool $condition, string $message): void
{
    if (!$condition) {
        fwrite(STDERR, "ERROR: {$message}\n");
        exit(1);
    }
}

$analysis = ambient_read($root . '/server/src/analysis.rs');
$server = ambient_read($root . '/server/src/lib.rs');
$tests = ambient_read($root . '/server/tests/lsp_tests.rs');
$docs = ambient_read($root . '/README.md')
    . ambient_read($root . '/server/README.md')
    . ambient_read($root . '/docs/architecture.md')
    . ambient_read($root . '/docs/semantic-hover.md');

foreach ([
    'CheckedEffectProfile::classify',
    'required_checked_effects',
    'ambient_checked_effects',
    'ambient_effect_documentation',
] as $fact) {
    ambient_require(str_contains($analysis, $fact), "semantic hover is missing compiler-owned {$fact}.");
}

ambient_require(
    str_contains($server, 'builtin_documentation_with_effects')
        && str_contains($server, 'builtin.ambient_error_types()'),
    'builtin hover must use compiler-owned ambient-effect metadata.',
);

foreach ([
    'accepts_ambient_io_in_ordinary_helpers_without_source_contracts',
    'ambient_closures_and_list_callbacks_need_no_source_contract',
    'preserves_destructor_boundary_and_accepts_ambient_finalizers',
    'fallible_finalizers_flow_to_outer_context_but_not_same_try_catches',
    'missing_nonambient_callable_contract_remains_compiler_owned',
] as $test) {
    ambient_require(str_contains($tests, $test), "protocol coverage is missing {$test}.");
}

ambient_require(
    !preg_match('/["\']code["\']\s*[:=]>?\s*["\']E0632["\']/', $tests),
    'E0632 must not be expected as a live protocol diagnostic.',
);

foreach ([
    'Ambient I/O',
    'finalizer-precedence',
    'E0632',
    'historical',
    'Stage 31 is complete',
    'Stage 32 is complete',
] as $fact) {
    ambient_require(str_contains($docs, $fact), "tooling documentation is missing {$fact}.");
}

fwrite(STDOUT, "Ambient I/O and fallible-finalizer tooling guard passed.\n");
