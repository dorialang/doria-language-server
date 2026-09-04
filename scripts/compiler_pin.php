<?php

declare(strict_types=1);

const DORIA_COMPILER_AUTHORITY_REVISION = 'cb833dbd08cfefa307cabba04ddd3f9459e8d006';

function doria_compiler_revision(string $manifest): ?string
{
    $matched = preg_match(
        '/doriac\s*=\s*\{[^\n]*\brev\s*=\s*"([0-9a-f]{40})"/',
        $manifest,
        $pin,
    );

    return $matched === 1 ? $pin[1] : null;
}

function doria_lock_resolves_revision(string $lock, string $revision): bool
{
    return substr_count($lock, 'rev=' . $revision . '#' . $revision) >= 3;
}

function doria_compiler_revision_is_authoritative(?string $revision): bool
{
    return $revision !== null && hash_equals(DORIA_COMPILER_AUTHORITY_REVISION, $revision);
}
