<?php

declare(strict_types=1);

/**
 * `scripts/build.php` starts the tool it names, with the arguments it gives.
 *
 * The build script starts `npm` by name and `gradlew` by path. On Windows both
 * are batch files, and a batch file reads its own directory from how it was
 * started. This check puts a small tool in a directory whose name has a space,
 * starts it the way the build script does from a different working directory,
 * and requires the tool to report its own directory and each argument whole.
 * When `npm` is installed it must also answer `--version` from that directory;
 * `--require-npm` makes a missing `npm` a failure, for CI.
 *
 * Usage: php scripts/check_build_tool_launching.php [--require-npm]
 */

require __DIR__ . '/build.php';

$requireNpm = in_array('--require-npm', array_slice($argv, 1), true);

$windows = PHP_OS_FAMILY === 'Windows';
$base = sys_get_temp_dir() . DIRECTORY_SEPARATOR . 'doria build tools ' . bin2hex(random_bytes(4));
$toolDirectory = $base . DIRECTORY_SEPARATOR . 'tool home';
$workingDirectory = $base . DIRECTORY_SEPARATOR . 'working directory';
$toolName = 'doria-echo-tool';
$toolFile = $toolDirectory . DIRECTORY_SEPARATOR . $toolName . ($windows ? '.cmd' : '');
$errors = [];

try {
    foreach ([$toolDirectory, $workingDirectory] as $directory) {
        if (!mkdir($directory, 0777, true)) {
            throw new RuntimeException("could not create {$directory}");
        }
    }
    file_put_contents(
        $toolFile,
        $windows
            ? "@ECHO OFF\r\nECHO dir=%~dp0\r\nECHO first=%~1\r\nECHO second=%~2\r\n"
            : "#!/bin/sh\necho \"dir=$(cd \"$(dirname \"$0\")\" && pwd -P)\"\n"
                . "echo \"first=$1\"\necho \"second=$2\"\n"
    );
    chmod($toolFile, 0755);

    $arguments = ['one two', 'key=value with space'];
    $expected = [
        'dir' => comparable_directory($toolDirectory),
        'first' => 'one two',
        'second' => 'key=value with space',
    ];

    $previousPath = getenv('PATH');
    putenv('PATH=' . $toolDirectory . PATH_SEPARATOR . ($previousPath === false ? '' : $previousPath));

    $cases = [
        'found on PATH by name' => $toolName,
        'given by path with forward slashes' => str_replace('\\', '/', $toolFile),
    ];
    foreach ($cases as $label => $tool) {
        $reported = [];
        foreach (preg_split('/\R/', capture_command(tool_command($tool, $arguments), $workingDirectory)) as $line) {
            if (str_contains($line, '=')) {
                [$key, $value] = explode('=', $line, 2);
                $reported[$key] = rtrim($value);
            }
        }
        $reported['dir'] = comparable_directory($reported['dir'] ?? '');
        foreach ($expected as $key => $value) {
            if (($reported[$key] ?? null) !== $value) {
                $errors[] = "{$label}: the tool reported {$key}=" . var_export($reported[$key] ?? null, true)
                    . ', expected ' . var_export($value, true);
            }
        }
    }

    try {
        resolve_tool('doria-tool-that-is-not-installed');
        $errors[] = 'a tool that is not on PATH was resolved';
    } catch (RuntimeException $error) {
        $message = $error->getMessage();
        if (!str_contains($message, 'doria-tool-that-is-not-installed') || !str_contains($message, 'PATH')) {
            $errors[] = "the missing-tool error does not name the tool and PATH: {$message}";
        }
    }

    $npm = null;
    try {
        $npm = tool_command('npm', ['--version']);
    } catch (RuntimeException $error) {
        if ($requireNpm) {
            throw $error;
        }
        echo "npm is not installed here; its launch was not checked.\n";
    }
    if ($npm !== null) {
        $version = trim(capture_command($npm, $workingDirectory));
        if (preg_match('/^\d+\.\d+\.\d+/', $version) !== 1) {
            $errors[] = 'npm --version printed ' . var_export($version, true);
        }
    }
} catch (Throwable $error) {
    $errors[] = $error->getMessage();
} finally {
    foreach ([$toolFile, $toolDirectory, $workingDirectory, $base] as $path) {
        if (is_file($path)) {
            unlink($path);
        } elseif (is_dir($path)) {
            rmdir($path);
        }
    }
}

if ($errors !== []) {
    fwrite(STDERR, "Build tool launching check failed:\n");
    foreach ($errors as $error) {
        fwrite(STDERR, "  - {$error}\n");
    }
    exit(1);
}

echo "Build tool launching check passed.\n";

/** A directory as one spelling: resolved, without a trailing separator, and lower case on Windows. */
function comparable_directory(string $directory): string
{
    $resolved = realpath(rtrim($directory, '/\\'));
    if ($resolved === false) {
        return $directory;
    }

    return PHP_OS_FAMILY === 'Windows' ? strtolower($resolved) : $resolved;
}
