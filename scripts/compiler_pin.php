<?php

declare(strict_types=1);

const DORIA_COMPILER_AUTHORITY_REVISION = '706e534c3fa6d0901ff63e4f7929c6186e378501';

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
