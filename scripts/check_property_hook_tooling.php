#!/usr/bin/env php
<?php

declare(strict_types=1);

$root = dirname(__DIR__);

function property_hook_text(string $path): string
{
    global $root;
    $text = file_get_contents($root . '/' . $path);
    if ($text === false) {
        fwrite(STDERR, "ERROR: could not read {$path}.\n");
        exit(1);
    }
    return $text;
}

function property_hook_require(bool $condition, string $message): void
{
    if (!$condition) {
        fwrite(STDERR, "ERROR: {$message}\n");
        exit(1);
    }
}

$analysis = property_hook_text('server/src/analysis.rs');
$hooks = property_hook_text('server/src/analysis/property_hooks.rs');
$server = property_hook_text('server/src/lib.rs');
$index = property_hook_text('server/src/workspace_index.rs');
$tests = property_hook_text('server/src/tests/property_hooks.rs');

// These source checks complement, rather than execute, the Rust regressions.
foreach ([
    [$analysis, ['mod property_hooks;', 'self.collect_property_hooks(',
        'member_callables(member, PropertyHookContext::Class)',
        'member_callables(member, PropertyHookContext::Trait)',
        'doriac::property_hooks::declaration_facts(', 'PropertyHookContext::Interface',
        'info.member_receiver_access(object.span())', 'info.property_backing_fields',
        'facts.constrained_member_surfaces.get(&receiver_span)',
        'CallableValueTargetKind::Property', 'call.callee_span',
        'ReturnBorrowKind::Value', 'ReturnBorrowKind::Retained',
        'return_documentation_preserves_compiler_loan_kind_source_and_access']],
    [$hooks, ['declaration_facts(property, context)', 'facts.callables()',
        'info.property_accessor_calls', 'PropertyHooksSurface',
        'call.return_borrow', 'call.checked_effects']],
    [$index, ['fn property_accessor_definitions(']],
    [$server, ['mod property_hooks;', '.property_accessor_definitions(']],
    [$hooks, ['shared_hook_fixture_is_accepted_and_preserves_compiler_facts',
        'property_operations_navigate_to_accessors_but_backing_access_does_not',
        'interface_and_constrained_hovers_keep_result_borrow_and_specialized_types',
        'class_completions_use_accessor_receiver_contracts_not_property_writability',
        'callback_hovers_and_completions_distinguish_owned_carriers_from_lent_values',
        'constrained_completions_preserve_lexical_scopes_and_specialized_types',
        'constrained_completions_use_compiler_intersection_selection',
        'constrained_completions_preserve_access_and_forwarding_wrapper_members',
        'backing_in_lexical_closures_and_trait_override_navigation_use_compiler_identity',
        'snapshot.diagnostics().is_empty()', 'compiler.diagnostics.is_empty()',
        'parent then child, without invoking setters',
        'A child initializer replaces the inherited value']],
    [$tests, ['property_hook_protocol_preserves_cross_file_origins_and_unsaved_updates',
        'property_hook_protocol_preserves_merged_interface_origins_without_method_names',
        'setter_type_diagnostics_preserve_resolved_aliases_and_utf16_ranges',
        'blocking_hook_diagnostics_preserve_compiler_ranges_and_related_operations',
        'diagnostics.is_empty()', 'compiler.diagnostics.is_empty()',
        'diagnostics_to_lsp(uri, source, &compiler.diagnostics)']],
] as [$text, $facts]) {
    foreach ($facts as $fact) {
        property_hook_require(str_contains($text, $fact), "property-hook projection or coverage is missing {$fact}.");
    }
}

foreach ([$analysis, $hooks, $server, $index, $tests] as $text) {
    property_hook_require(
        !str_contains($text, 'E0764'),
        'the language server and its tests must not suppress or allow the retired E0764 gate.',
    );
}

$readme = property_hook_text('README.md');
foreach (['compiler-owned property-hook metadata', 'accessor origins',
    'result ownership', 'forwards compiler diagnostics unchanged'] as $fact) {
    property_hook_require(str_contains($readme, $fact), "property-hook capability documentation is missing {$fact}.");
}
foreach (['README.md', 'server/README.md', 'editors/vscode/doria/README.md',
    'editors/intellij/doria/README.md'] as $path) {
    foreach (preg_split('/\R\s*\R/', property_hook_text($path)) as $paragraph) {
        if (preg_match('/property[- ]hooks?/i', $paragraph) !== 1) {
            continue;
        }
        property_hook_require(
            preg_match('/\b(?:E0764|staged|Stage\s+\d+|not yet|pin(?:ned)?)\b/i', $paragraph) === 0,
            "{$path} must keep interim hook implementation status out of product copy.",
        );
    }
}
foreach (['README.md', 'server/README.md', 'docs/architecture.md', 'docs/semantic-hover.md',
    'editors/vscode/doria/README.md', 'editors/intellij/doria/README.md'] as $path) {
    property_hook_require(
        preg_match('/property hooks\s+remain\s+(?:separate\s+)?(?:future\s+)?work/i', property_hook_text($path)) === 0,
        "{$path} must describe hook capabilities rather than obsolete absence.",
    );
}

fwrite(STDOUT, "Property hook tooling guard passed (source contracts; runtime validation is separate).\n");
