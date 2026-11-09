<?php

declare(strict_types=1);

$root = sys_get_temp_dir() . '/doria-target-size-' . bin2hex(random_bytes(8));
$environment = getenv();
unset($environment['CARGO_TARGET_DIR']);
$checks = 0;
$failed = false;

try {
    foreach (['scripts', 'target', 'configured cache', 'override cache'] as $directory) {
        if (!mkdir($root . '/' . $directory, 0700, true)) {
            throw new RuntimeException("could not create test directory: {$directory}");
        }
    }
    $script = $root . '/scripts/check_cargo_target_size.php';
    if (!copy(__DIR__ . '/check_cargo_target_size.php', $script)) {
        throw new RuntimeException('could not copy checker into isolated fixture');
    }
    $root = realpath($root) ?: throw new RuntimeException('could not resolve fixture root');
    foreach (['target' => 4096, 'configured cache' => 8192, 'override cache' => 16384] as $directory => $size) {
        $file = $root . '/' . $directory . '/artifact';
        if (file_put_contents($file, str_repeat('x', $size)) !== $size) {
            throw new RuntimeException("could not write test artifact: {$file}");
        }
    }
    foreach ([
        [$environment, [], 'target'],
        [array_replace($environment, ['CARGO_TARGET_DIR' => '']), [], 'target'],
        [array_replace($environment, ['CARGO_TARGET_DIR' => $root . '/configured cache']), [], 'configured cache'],
        [array_replace($environment, ['CARGO_TARGET_DIR' => './configured cache']), [], 'configured cache'],
        [array_replace($environment, ['CARGO_TARGET_DIR' => $root . '/configured cache']), ['--target-dir', 'override cache'], 'override cache'],
    ] as [$env, $arguments, $directory]) {
        [$status, $output, $error] = run_checker($script, $env, $arguments);
        $metadata = stat($root . '/' . $directory . '/artifact');
        $bytes = isset($metadata['blocks']) ? $metadata['blocks'] * 512 : $metadata['size'];
        $expectedSize = sprintf('%.2f KiB', $bytes / 1024);
        verify($status === 0 && $error === '', "unexpected result: {$output}{$error}");
        verify(str_contains($output, $root . DIRECTORY_SEPARATOR . $directory . ';'), "wrong target: {$output}");
        verify(str_contains($output, "size: {$expectedSize} ("), "wrong allocation: {$output}");
        $checks++;
    }

    [$status, $output, $error] = run_checker($script, $environment, ['--target-dir', 'missing/nested']);
    verify($status === 0 && $error === '' && str_contains($output, 'size: 0 B ('), "missing target: {$output}{$error}");
    verify(str_contains($output, $root . DIRECTORY_SEPARATOR . 'missing' . DIRECTORY_SEPARATOR . 'nested; absent;'), "missing path: {$output}");
    $checks++;

    rename($root . '/target', $root . '/saved-target');
    [$status, $output, $error] = run_checker($script, $environment, []);
    verify($status === 0 && $error === '' && str_contains($output, 'size: 0 B ('), "absent default: {$output}{$error}");
    verify(str_contains($output, $root . DIRECTORY_SEPARATOR . 'target; absent;'), "default path: {$output}");
    rename($root . '/saved-target', $root . '/target');
    $checks++;

    foreach ([
        ['--target-dir'], ['--target-dir', ''], ['--unknown'],
        ['--target-dir', 'target', '--target-dir', 'override cache'],
        ['--target-dir', 'target/artifact'], ['--target-dir', 'C:relative'], ['--target-dir', 'C:'],
        ['--target-dir', 'target/artifact/child'],
    ] as $arguments) {
        [$status, $output, $error] = run_checker($script, $environment, $arguments);
        verify($status === 1 && $output === '' && str_starts_with($error, 'ERROR:'), "failure was not reported: {$output}{$error}");
        $checks++;
    }

    if (PHP_OS_FAMILY !== 'Windows') {
        foreach (['unreadable' => 'unreadable', 'target/unreadable' => 'target'] as $directory => $target) {
            $unreadable = $root . '/' . $directory;
            mkdir($unreadable, 0700);
            chmod($unreadable, 0000);
            clearstatcache();
            try {
                if (!is_readable($unreadable)) {
                    [$status, $output, $error] = run_checker($script, $environment, ['--target-dir', $target]);
                    verify($status === 1 && $output === '' && str_starts_with($error, 'ERROR:'), "unreadable target: {$output}{$error}");
                    $checks++;
                } else {
                    fwrite(STDOUT, "SKIP: current user can read permission-restricted directories.\n");
                }
            } finally {
                chmod($unreadable, 0700);
            }
        }
    }

    $artifact = $root . '/target/artifact';
    [$status, $before] = run_checker($script, $environment, []);
    if (stat($artifact)['ino'] !== 0 && @link($artifact, $root . '/target/hard-link')) {
        [$status, $after, $error] = run_checker($script, $environment, []);
        verify($status === 0 && $error === '' && $before === $after, 'hard link was counted twice');
        $checks++;
    } else {
        fwrite(STDOUT, "SKIP: filesystem does not expose hard-link identities or permit hard links.\n");
    }
    if (@symlink($root . '/configured cache', $root . '/target/directory-link')) {
        if (!symlink($root . '/configured cache/artifact', $root . '/target/file-link')) {
            throw new RuntimeException('could not create file symlink fixture');
        }
        [$status, $after, $error] = run_checker($script, $environment, []);
        verify($status === 0 && $error === '' && $before === $after, 'symlink target was counted');
        $checks++;
    } else {
        fwrite(STDOUT, "SKIP: filesystem does not support creating symlinks.\n");
    }
    fwrite(STDOUT, "Cargo target size checks passed ({$checks} filesystem cases).\n");
} catch (Throwable $error) {
    fwrite(STDERR, "ERROR: {$error->getMessage()}\n");
    $failed = true;
} finally {
    if (is_dir($root)) {
        $entries = new RecursiveIteratorIterator(
            new RecursiveDirectoryIterator($root, FilesystemIterator::SKIP_DOTS),
            RecursiveIteratorIterator::CHILD_FIRST,
        );
        foreach ($entries as $entry) {
            if ($entry->isDir() && !$entry->isLink()) {
                rmdir($entry->getPathname());
            } else {
                unlink($entry->getPathname());
            }
        }
        rmdir($root);
    }
}
if ($failed) {
    exit(1);
}

/** @return array{int, string, string} */
function run_checker(string $script, array $environment, array $arguments): array
{
    $process = proc_open(
        [PHP_BINARY, $script, ...$arguments],
        [0 => ['pipe', 'r'], 1 => ['pipe', 'w'], 2 => ['pipe', 'w']],
        $pipes,
        sys_get_temp_dir(),
        $environment,
    );
    if (!is_resource($process)) {
        throw new RuntimeException('could not start checker');
    }
    fclose($pipes[0]);
    $output = stream_get_contents($pipes[1]);
    $error = stream_get_contents($pipes[2]);
    fclose($pipes[1]);
    fclose($pipes[2]);
    return [proc_close($process), $output, $error];
}

function verify(bool $condition, string $message): void
{
    if (!$condition) {
        throw new RuntimeException($message);
    }
}
