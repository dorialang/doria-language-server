#!/usr/bin/env php
<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$target = $argv[1] ?? 'help';

if (count($argv) > 2) {
    usage_error('expected exactly one target argument');
}

try {
    match ($target) {
        'server' => build_server($root, false),
        'server-release' => build_server($root, true),
        'install-server' => install_server($root),
        'vscode' => build_vscode($root),
        'intellij' => build_intellij($root),
        'editors' => build_editors($root),
        'all' => build_all($root),
        'help', '--help', '-h' => print_usage(),
        default => usage_error("unknown target '{$target}'"),
    };
} catch (Throwable $error) {
    fwrite(STDERR, "build: {$error->getMessage()}\n");
    exit(1);
}

function build_server(string $root, bool $release): void
{
    $command = ['cargo', 'build', '--locked', '--bin', 'doria-lsp'];
    if ($release) {
        $command[] = '--release';
    }

    run_command($command, $root);

    $metadata = json_decode(
        capture_command(['cargo', 'metadata', '--format-version', '1', '--no-deps'], $root),
        true,
        512,
        JSON_THROW_ON_ERROR
    );
    $profile = $release ? 'release' : 'debug';
    $executable = PHP_OS_FAMILY === 'Windows' ? 'doria-lsp.exe' : 'doria-lsp';
    $artifact = ($metadata['target_directory'] ?? $root . '/target') . "/{$profile}/{$executable}";
    require_artifact($artifact, 'language-server executable');
}

function install_server(string $root): void
{
    run_command(
        ['cargo', 'install', '--path', $root . '/server', '--locked', '--force'],
        $root
    );

    $cargoHome = getenv('CARGO_INSTALL_ROOT');
    if ($cargoHome === false || $cargoHome === '') {
        $cargoHome = getenv('CARGO_HOME');
    }
    if ($cargoHome === false || $cargoHome === '') {
        $userHome = PHP_OS_FAMILY === 'Windows' ? getenv('USERPROFILE') : getenv('HOME');
        $cargoHome = ($userHome === false || $userHome === '') ? null : $userHome . '/.cargo';
    }

    $executable = PHP_OS_FAMILY === 'Windows' ? 'doria-lsp.exe' : 'doria-lsp';
    if ($cargoHome !== null) {
        require_artifact($cargoHome . '/bin/' . $executable, 'globally installed language server');
    }

    fwrite(STDOUT, "\nVerify the installation with: doria-lsp --version\n");
}

function build_vscode(string $root): void
{
    $editor = $root . '/editors/vscode/doria';
    $dist = $root . '/dist';
    ensure_directory($dist);

    run_tool('npm', ['ci', '--ignore-scripts'], $editor);
    run_tool(
        'npm',
        ['run', 'package', '--', '--out', $dist . '/doria-language-support.vsix'],
        $editor
    );

    require_artifact($dist . '/doria-language-support.vsix', 'VS Code extension');
}

function build_intellij(string $root): void
{
    $editor = $root . '/editors/intellij/doria';
    $gradle = PHP_OS_FAMILY === 'Windows' ? $editor . '/gradlew.bat' : $editor . '/gradlew';
    $command = PHP_OS_FAMILY === 'Windows'
        ? windows_command($gradle, ['buildPlugin', '--no-daemon'])
        : [$gradle, 'buildPlugin', '--no-daemon'];

    run_command($command, $editor);

    $artifacts = glob($editor . '/build/distributions/*.zip') ?: [];
    usort(
        $artifacts,
        static fn (string $left, string $right): int => filemtime($right) <=> filemtime($left)
    );
    if ($artifacts === []) {
        throw new RuntimeException('IntelliJ plugin build completed without producing a ZIP');
    }

    require_artifact($artifacts[0], 'IntelliJ plugin');
}

function build_editors(string $root): void
{
    build_vscode($root);
    build_intellij($root);
}

function build_all(string $root): void
{
    build_server($root, false);
    build_editors($root);
}

/** @param list<string> $arguments */
function run_tool(string $tool, array $arguments, string $workingDirectory): void
{
    $command = PHP_OS_FAMILY === 'Windows'
        ? windows_command($tool, $arguments)
        : [$tool, ...$arguments];
    run_command($command, $workingDirectory);
}

/**
 * @param list<string> $arguments
 * @return list<string>
 */
function windows_command(string $executable, array $arguments): array
{
    $commandProcessor = getenv('COMSPEC');
    if ($commandProcessor === false || $commandProcessor === '') {
        $commandProcessor = 'cmd.exe';
    }

    return [$commandProcessor, '/d', '/c', $executable, ...$arguments];
}

/** @param list<string> $command */
function run_command(array $command, string $workingDirectory): void
{
    fwrite(STDOUT, "\n> " . display_command($command) . "\n");
    $process = proc_open(
        $command,
        [
            0 => ['file', 'php://stdin', 'r'],
            1 => ['file', 'php://stdout', 'w'],
            2 => ['file', 'php://stderr', 'w'],
        ],
        $pipes,
        $workingDirectory
    );
    if (!is_resource($process)) {
        throw new RuntimeException('could not start command: ' . display_command($command));
    }

    $status = proc_close($process);
    if ($status !== 0) {
        throw new RuntimeException("command failed with exit code {$status}: " . display_command($command));
    }
}

/** @param list<string> $command */
function capture_command(array $command, string $workingDirectory): string
{
    $process = proc_open(
        $command,
        [
            0 => ['file', 'php://stdin', 'r'],
            1 => ['pipe', 'w'],
            2 => ['pipe', 'w'],
        ],
        $pipes,
        $workingDirectory
    );
    if (!is_resource($process)) {
        throw new RuntimeException('could not start command: ' . display_command($command));
    }

    $output = stream_get_contents($pipes[1]);
    $error = stream_get_contents($pipes[2]);
    fclose($pipes[1]);
    fclose($pipes[2]);
    $status = proc_close($process);
    if ($status !== 0) {
        throw new RuntimeException(trim($error) ?: 'command failed: ' . display_command($command));
    }

    return $output;
}

/** @param list<string> $command */
function display_command(array $command): string
{
    return implode(' ', array_map(
        static fn (string $argument): string => preg_match('/^[A-Za-z0-9_\.\-\/:=]+$/', $argument) === 1
            ? $argument
            : escapeshellarg($argument),
        $command
    ));
}

function ensure_directory(string $directory): void
{
    if (!is_dir($directory) && !mkdir($directory, 0777, true) && !is_dir($directory)) {
        throw new RuntimeException("could not create artifact directory: {$directory}");
    }
}

function require_artifact(string $path, string $label): void
{
    if (!is_file($path)) {
        throw new RuntimeException("{$label} was not found at {$path}");
    }

    $resolved = realpath($path) ?: $path;
    fwrite(STDOUT, "\n{$label}: {$resolved}\n");
}

function print_usage(): void
{
    fwrite(STDOUT, <<<'USAGE'
Build Doria language-server and editor artifacts from the repository root.

Usage:
  php scripts/build.php <target>

Targets:
  server          Build the debug doria-lsp executable
  server-release  Build the optimized doria-lsp executable
  install-server  Build and install doria-lsp into Cargo's global bin directory
  vscode          Package dist/doria-language-support.vsix
  intellij        Package the JetBrains plugin ZIP
  editors         Package both editor extensions
  all             Build the debug server and both editor extensions
  help            Show this help

Every build target prints the absolute path of each generated artifact.
USAGE
    );
    fwrite(STDOUT, "\n");
}

function usage_error(string $message): never
{
    fwrite(STDERR, "build: {$message}\n\n");
    print_usage();
    exit(1);
}
