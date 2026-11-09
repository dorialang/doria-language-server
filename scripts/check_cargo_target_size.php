<?php

declare(strict_types=1);

const TARGET_WARNING_BYTES = 15 * 1024 * 1024 * 1024;

try {
    $configured = getenv('CARGO_TARGET_DIR');
    $target = $configured !== false && $configured !== '' ? $configured : 'target';
    $explicit = false;
    for ($index = 1; $index < count($argv); $index++) {
        if (
            $argv[$index] !== '--target-dir' || $explicit
            || !isset($argv[$index + 1]) || $argv[$index + 1] === ''
            || str_starts_with($argv[$index + 1], '--')
        ) {
            throw new RuntimeException(
                'Usage: php scripts/check_cargo_target_size.php [--target-dir <path>]',
            );
        }
        $target = $argv[++$index];
        $explicit = true;
    }
    $target = resolve_target_directory($target);
    if (!is_dir($target)) {
        if (file_exists($target) || is_link($target)) {
            throw new RuntimeException("Cargo target is not a directory: {$target}");
        }
        fwrite(STDOUT, "Cargo target allocated size: 0 B ({$target}; absent; warning threshold: 15 GiB)\n");
        exit(0);
    }

    $bytes = 0;
    $seenFiles = [];
    $entries = new RecursiveIteratorIterator(
        new RecursiveDirectoryIterator($target, FilesystemIterator::SKIP_DOTS),
        RecursiveIteratorIterator::LEAVES_ONLY,
    );

    foreach ($entries as $entry) {
        $metadata = @lstat($entry->getPathname());
        if ($metadata === false) {
            throw new RuntimeException("could not measure {$entry->getPathname()}");
        }
        if (($metadata['mode'] & 0170000) !== 0100000) {
            continue;
        }

        $identity = "{$metadata['dev']}:{$metadata['ino']}";
        if ($metadata['ino'] !== 0 && isset($seenFiles[$identity])) {
            continue;
        }

        if ($metadata['ino'] !== 0) {
            $seenFiles[$identity] = true;
        }

        $bytes += array_key_exists('blocks', $metadata)
            ? $metadata['blocks'] * 512
            : $metadata['size'];
    }
} catch (Throwable $error) {
    fwrite(STDERR, "ERROR: {$error->getMessage()}\n");
    exit(1);
}

$size = format_bytes($bytes);
fwrite(STDOUT, "Cargo target allocated size: {$size} ({$target}; warning threshold: 15 GiB)\n");

if ($bytes > TARGET_WARNING_BYTES) {
    fwrite(
        STDERR,
        "WARNING: {$target} exceeds 15 GiB. Cargo does not garbage-collect obsolete project artifacts.\n"
        . 'Inspect the directory and use `cargo clean --target-dir '
        . escapeshellarg($target) . ' --dry-run` to preview cleanup. '
        . "Do not clean it without Andrew's approval.\n",
    );
    exit(2);
}

function resolve_target_directory(string $path): string
{
    if (preg_match('/^[A-Za-z]:(?:$|[^\\\\\/])/', $path) === 1) {
        throw new RuntimeException("drive-relative Cargo targets are not supported: {$path}");
    }
    $absolute = str_starts_with($path, '/')
        || str_starts_with($path, '\\\\')
        || preg_match('/^[A-Za-z]:[\\\\\/]/', $path) === 1;
    if (!$absolute) {
        // Build commands run from the repository root, including local runners.
        $path = dirname(__DIR__) . DIRECTORY_SEPARATOR . $path;
    }

    $missing = [];
    while (($resolved = realpath($path)) === false) {
        if (file_exists($path) || is_link($path) || dirname($path) === $path) {
            throw new RuntimeException("could not resolve Cargo target: {$path}");
        }
        $missing[] = basename($path);
        $path = dirname($path);
    }
    if ($missing !== [] && !is_dir($resolved)) {
        throw new RuntimeException("Cargo target ancestor is not a directory: {$resolved}");
    }
    if (is_dir($resolved) && !is_readable($resolved)) {
        throw new RuntimeException("could not read Cargo target directory: {$resolved}");
    }
    return $missing === []
        ? $resolved
        : rtrim($resolved, '/\\') . DIRECTORY_SEPARATOR
            . implode(DIRECTORY_SEPARATOR, array_reverse($missing));
}

function format_bytes(int $bytes): string
{
    $units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
    $value = (float) $bytes;
    $unit = 0;

    while ($value >= 1024 && $unit < count($units) - 1) {
        $value /= 1024;
        ++$unit;
    }

    return $unit === 0
        ? sprintf('%d %s', $bytes, $units[$unit])
        : sprintf('%.2f %s', $value, $units[$unit]);
}
