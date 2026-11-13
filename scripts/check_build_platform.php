<?php

declare(strict_types=1);

/**
 * `scripts/build.php` names an editor package for the server it carries.
 *
 * The platform in a VSIX target or an IntelliJ bundle directory is the Rust
 * target `doria-lsp` was built for. It is never the machine the PHP
 * interpreter reports: a 32-bit PHP on 64-bit Windows reports `i586` while
 * Cargo builds an x86_64 server. This check holds the mapping from Rust
 * targets to package names, requires the script to ask `rustc` for the host,
 * and forbids `php_uname` in the build script.
 *
 * Usage: php scripts/check_build_platform.php
 */

require __DIR__ . '/build.php';

$root = dirname(__DIR__);
$errors = [];

/** Rust target => [VS Code target, IntelliJ bundle directory]; null means refused. */
$targets = [
    'x86_64-pc-windows-msvc' => ['win32-x64', 'windows-x86_64'],
    'aarch64-pc-windows-msvc' => ['win32-arm64', 'windows-aarch64'],
    'x86_64-apple-darwin' => ['darwin-x64', 'macos-x86_64'],
    'aarch64-apple-darwin' => ['darwin-arm64', 'macos-aarch64'],
    'x86_64-unknown-linux-gnu' => ['linux-x64', 'linux-x86_64'],
    'aarch64-unknown-linux-gnu' => ['linux-arm64', 'linux-aarch64'],
    'armv7-unknown-linux-gnueabihf' => ['linux-armhf', null],
    'i686-pc-windows-msvc' => [null, null],
    'i686-unknown-linux-gnu' => [null, null],
    'x86_64-unknown-freebsd' => [null, null],
];

foreach ($targets as $triple => [$vscode, $intellij]) {
    foreach (['vscode_target' => $vscode, 'intellij_platform_key' => $intellij] as $function => $expected) {
        try {
            $actual = $function($triple);
        } catch (RuntimeException $error) {
            $actual = null;
            $message = $error->getMessage();
            if (!str_contains($message, $triple) || !str_contains($message, '. ')) {
                $errors[] = "{$function}({$triple}) refuses without naming the target and what to do: {$message}";
            }
        }
        if ($actual !== $expected) {
            $errors[] = "{$function}({$triple}) gave " . var_export($actual, true)
                . ', expected ' . var_export($expected, true);
        }
    }
}

try {
    $reported = capture_command(['rustc', '-vV'], $root);
    $host = server_target_triple($root);
    if (!preg_match('/^host:\s*' . preg_quote($host, '/') . '\s*$/m', $reported)) {
        $errors[] = "server_target_triple gave {$host}, which is not the host `rustc -vV` reports";
    }
    echo "Server target: {$host}\n";
} catch (Throwable $error) {
    $errors[] = $error->getMessage();
}

$previous = getenv('CARGO_BUILD_TARGET');
putenv('CARGO_BUILD_TARGET=wasm32-unknown-unknown');
try {
    server_target_triple($root);
    $errors[] = 'a CARGO_BUILD_TARGET override was accepted, though the script packages the host build';
} catch (RuntimeException $error) {
    if (!str_contains($error->getMessage(), 'Unset CARGO_BUILD_TARGET')) {
        $errors[] = 'the CARGO_BUILD_TARGET error does not say what to do: ' . $error->getMessage();
    }
} finally {
    putenv($previous === false ? 'CARGO_BUILD_TARGET' : "CARGO_BUILD_TARGET={$previous}");
}

if (preg_match('/\bphp_uname\s*\(/', (string) file_get_contents(__DIR__ . '/build.php')) === 1) {
    $errors[] = 'scripts/build.php calls php_uname: the interpreter does not own the packaged server\'s platform';
}

if ($errors !== []) {
    fwrite(STDERR, "Build platform check failed:\n");
    foreach ($errors as $error) {
        fwrite(STDERR, "  - {$error}\n");
    }
    exit(1);
}

echo "Build platform check passed.\n";
