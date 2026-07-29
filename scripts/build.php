#!/usr/bin/env php
<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$target = $argv[1] ?? 'help';
$compilerPath = null;
for ($index = 2; $index < count($argv); $index++) {
    if ($argv[$index] !== '--compiler-path') {
        usage_error("unknown option '{$argv[$index]}'");
    }
    if ($compilerPath !== null) {
        usage_error('--compiler-path may only be specified once');
    }
    $index++;
    if (!isset($argv[$index]) || $argv[$index] === '') {
        usage_error('--compiler-path requires a Doria repository or doriac crate path');
    }
    $compilerPath = $argv[$index];
}

if (
    $compilerPath !== null
    && !in_array(
        $target,
        ['server', 'server-release', 'install-server', 'vscode', 'editors', 'all'],
        true,
    )
) {
    usage_error(
        '--compiler-path is supported by the server, server-release, install-server, '
            . 'vscode, editors, and all targets'
    );
}

try {
    match ($target) {
        'server' => build_server($root, false, $compilerPath),
        'server-release' => build_server($root, true, $compilerPath),
        'install-server' => install_server($root, $compilerPath),
        'vscode' => build_vscode($root, $compilerPath),
        'intellij' => build_intellij($root),
        'editors' => build_editors($root, $compilerPath),
        'all' => build_all($root, $compilerPath),
        'help', '--help', '-h' => print_usage(),
        default => usage_error("unknown target '{$target}'"),
    };
} catch (Throwable $error) {
    fwrite(STDERR, "build: {$error->getMessage()}\n");
    exit(1);
}

function build_server(string $root, bool $release, ?string $compilerPath = null): void
{
    if ($compilerPath !== null) {
        build_server_with_local_compiler($root, $release, $compilerPath);
        return;
    }

    remove_local_server_override($root, $release);
    $command = ['cargo', 'build', '--locked', '--bin', 'doria-lsp'];
    if ($release) {
        $command[] = '--release';
    }

    run_command($command, $root);

    require_artifact(server_artifact($root, $release), 'language-server executable');
}

function build_server_with_local_compiler(
    string $root,
    bool $release,
    string $compilerPath
): void {
    $compilerSource = resolve_compiler_source($root, $compilerPath);
    $compiler = $compilerSource['crate'];
    $runner = $root . '/target/local-doria-lsp-runner';
    $sourceDirectory = $runner . '/src';
    ensure_directory($sourceDirectory);

    $manifest = <<<TOML
[package]
name = "doria-local-lsp-runner"
version = "0.0.0"
edition = "2021"
publish = false

[[bin]]
name = "doria-lsp"
path = "src/main.rs"

[dependencies]
doria-language-server = { path = %s }

[patch."https://github.com/dorialang/doria"]
doriac = { path = %s }

[workspace]
TOML;
    write_generated_file(
        $runner . '/Cargo.toml',
        sprintf($manifest, toml_string($root . '/server'), toml_string($compiler)) . "\n"
    );
    write_generated_file(
        $sourceDirectory . '/main.rs',
        <<<'RUST'
fn main() -> std::process::ExitCode {
    doria_language_server::run_cli(std::env::args().skip(1))
}
RUST
        . "\n"
    );

    $command = [
        'cargo',
        'build',
        '--manifest-path',
        $runner . '/Cargo.toml',
        '--target-dir',
        $root . '/target/local-doria-lsp',
        '--bin',
        'doria-lsp',
    ];
    if ($release) {
        $command[] = '--release';
    }
    run_command($command, $root);

    $profile = $release ? 'release' : 'debug';
    $executable = PHP_OS_FAMILY === 'Windows' ? 'doria-lsp.exe' : 'doria-lsp';
    $localArtifact = $root . "/target/local-doria-lsp/{$profile}/{$executable}";
    require_artifact($localArtifact, 'local-compiler language-server executable');

    $artifact = $root . "/target/{$profile}/{$executable}";
    install_executable($localArtifact, $artifact);
    write_generated_file(local_server_marker($root, $release), $compiler . "\n");

    require_artifact($artifact, 'language-server executable');
    fwrite(STDOUT, "local compiler crate: {$compiler}\n");
}

function remove_local_server_override(string $root, bool $release): void
{
    $marker = local_server_marker($root, $release);
    if (!is_file($marker)) {
        return;
    }

    $profile = $release ? 'release' : 'debug';
    $executable = PHP_OS_FAMILY === 'Windows' ? 'doria-lsp.exe' : 'doria-lsp';
    $artifact = $root . "/target/{$profile}/{$executable}";
    if (is_file($artifact) && !unlink($artifact)) {
        throw new RuntimeException("could not remove local-compiler language server: {$artifact}");
    }
    if (!unlink($marker)) {
        throw new RuntimeException("could not remove local-compiler marker: {$marker}");
    }
}

function local_server_marker(string $root, bool $release): string
{
    $profile = $release ? 'release' : 'debug';
    return $root . "/target/{$profile}/doria-lsp.local-compiler";
}

function install_server(string $root, ?string $compilerPath = null): void
{
    if ($compilerPath !== null) {
        build_server_with_local_compiler($root, true, $compilerPath);
        $destination = cargo_install_root()
            . '/bin/'
            . (PHP_OS_FAMILY === 'Windows' ? 'doria-lsp.exe' : 'doria-lsp');
        install_executable(server_artifact($root, true), $destination);
        require_artifact($destination, 'globally installed language server');
        fwrite(STDOUT, "\nInstalled compiler-matched doria-lsp: {$destination}\n");
        return;
    }

    run_command(
        ['cargo', 'install', '--path', $root . '/server', '--locked', '--force'],
        $root
    );

    $executable = PHP_OS_FAMILY === 'Windows' ? 'doria-lsp.exe' : 'doria-lsp';
    require_artifact(
        cargo_install_root() . '/bin/' . $executable,
        'globally installed language server',
    );

    fwrite(STDOUT, "\nVerify the installation with: doria-lsp --version\n");
}

function cargo_install_root(): string
{
    foreach (['CARGO_INSTALL_ROOT', 'CARGO_HOME'] as $name) {
        $value = getenv($name);
        if ($value !== false && $value !== '') {
            return rtrim($value, '/\\');
        }
    }
    $home = getenv(PHP_OS_FAMILY === 'Windows' ? 'USERPROFILE' : 'HOME');
    if ($home === false || $home === '') {
        throw new RuntimeException(
            'could not determine Cargo install root; set CARGO_INSTALL_ROOT or CARGO_HOME'
        );
    }

    return rtrim($home, '/\\') . '/.cargo';
}

function build_vscode(string $root, ?string $compilerPath = null): void
{
    $editor = $root . '/editors/vscode/doria';
    $dist = $root . '/dist';
    ensure_directory($dist);

    build_server($root, true, $compilerPath);
    $server = server_artifact($root, true);
    $bundledServer = $editor . '/bin/' . basename($server);
    remove_stale_artifacts($editor . '/bin/doria-lsp*');
    install_executable($server, $bundledServer);

    $vsix = $dist . '/doria-language-support.vsix';
    // Remove the prior artifact so a packaging step that does not overwrite (or
    // no-ops) can never leave a stale file that require_artifact would report as
    // freshly built.
    remove_stale_artifacts($vsix);

    run_tool('npm', ['ci', '--ignore-scripts'], $editor);
    run_tool(
        'npm',
        ['run', 'package', '--', '--target', vscode_target(), '--out', $vsix],
        $editor
    );

    require_artifact($bundledServer, 'bundled VS Code language server');
    require_artifact($vsix, 'VS Code extension');
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
    if (count($artifacts) !== 1) {
        throw new RuntimeException(
            'IntelliJ plugin build must produce exactly one ZIP; found ' . count($artifacts)
        );
    }

    require_artifact($artifacts[0], 'IntelliJ plugin');
}

function build_editors(string $root, ?string $compilerPath = null): void
{
    build_vscode($root, $compilerPath);
    build_intellij($root);
}

function build_all(string $root, ?string $compilerPath = null): void
{
    build_server($root, false, $compilerPath);
    build_editors($root, $compilerPath);
}

function server_artifact(string $root, bool $release): string
{
    $metadata = json_decode(
        capture_command(['cargo', 'metadata', '--format-version', '1', '--no-deps'], $root),
        true,
        512,
        JSON_THROW_ON_ERROR
    );
    $profile = $release ? 'release' : 'debug';
    $executable = PHP_OS_FAMILY === 'Windows' ? 'doria-lsp.exe' : 'doria-lsp';

    return ($metadata['target_directory'] ?? $root . '/target') . "/{$profile}/{$executable}";
}

function vscode_target(): string
{
    $platform = match (PHP_OS_FAMILY) {
        'Windows' => 'win32',
        'Darwin' => 'darwin',
        'Linux' => 'linux',
        default => throw new RuntimeException(
            'VS Code packaging is unsupported on ' . PHP_OS_FAMILY
        ),
    };
    $machine = strtolower(php_uname('m'));
    $architecture = match ($machine) {
        'x86_64', 'amd64' => 'x64',
        'aarch64', 'arm64' => 'arm64',
        'armv7l', 'armv7' => 'armhf',
        default => throw new RuntimeException(
            "VS Code packaging is unsupported on architecture {$machine}"
        ),
    };
    if ($platform !== 'linux' && $architecture === 'armhf') {
        throw new RuntimeException("VS Code packaging does not support {$platform}-armhf");
    }

    return "{$platform}-{$architecture}";
}

/**
 * @return array{crate: string, workspace: ?string}
 */
function resolve_compiler_source(string $root, string $path): array
{
    $candidate = is_absolute_path($path) ? $path : $root . '/' . $path;
    $resolved = realpath($candidate);
    if ($resolved === false || !is_dir($resolved)) {
        throw new RuntimeException("compiler path does not exist: {$candidate}");
    }

    $repositoryCrate = $resolved . '/crates/doriac';
    if (is_file($repositoryCrate . '/Cargo.toml')) {
        return ['crate' => $repositoryCrate, 'workspace' => $resolved];
    }
    if (is_file($resolved . '/Cargo.toml')) {
        $workspace = $resolved;
        $possibleWorkspace = dirname(dirname($resolved));
        if (
            basename(dirname($resolved)) === 'crates'
            && is_file($possibleWorkspace . '/Cargo.toml')
        ) {
            $workspace = $possibleWorkspace;
        }
        return ['crate' => $resolved, 'workspace' => $workspace];
    }

    throw new RuntimeException(
        "compiler path must contain crates/doriac/Cargo.toml or be the doriac crate: {$resolved}"
    );
}

function install_executable(string $source, string $destination): void
{
    ensure_directory(dirname($destination));
    $temporary = $destination . '.install-' . getmypid();
    if (!copy($source, $temporary)) {
        throw new RuntimeException(
            "could not stage language-server executable: {$temporary}"
        );
    }
    if (PHP_OS_FAMILY !== 'Windows' && !chmod($temporary, 0755)) {
        @unlink($temporary);
        throw new RuntimeException("could not make language-server executable: {$temporary}");
    }
    if (PHP_OS_FAMILY === 'Windows' && is_file($destination) && !unlink($destination)) {
        @unlink($temporary);
        throw new RuntimeException(
            "could not replace the running language server at {$destination}; "
                . 'stop the IDE or language-server process and retry'
        );
    }
    if (!rename($temporary, $destination)) {
        @unlink($temporary);
        throw new RuntimeException("could not install language-server executable: {$destination}");
    }
}

function is_absolute_path(string $path): bool
{
    return str_starts_with($path, '/')
        || str_starts_with($path, '\\\\')
        || preg_match('/^[A-Za-z]:[\\\\\/]/', $path) === 1;
}

function toml_string(string $value): string
{
    return '"' . addcslashes($value, "\\\"") . '"';
}

function write_generated_file(string $path, string $contents): void
{
    if (file_put_contents($path, $contents) === false) {
        throw new RuntimeException("could not write generated build file: {$path}");
    }
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

/**
 * Delete a prior build artifact (an exact path or a glob pattern) so the build
 * tool must regenerate it and require_artifact can only ever find a file this
 * run produced. A missing artifact is not an error.
 */
function remove_stale_artifacts(string $pattern): void
{
    foreach (glob($pattern) ?: [] as $file) {
        if (is_file($file) && !unlink($file)) {
            throw new RuntimeException("could not remove stale artifact: {$file}");
        }
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
  php scripts/build.php <target> [--compiler-path <path>]

Targets:
  server          Build the debug doria-lsp executable
  server-release  Build the optimized doria-lsp executable
  install-server  Build and install doria-lsp into Cargo's global bin directory
  vscode          Build and bundle doria-lsp, then package a platform-specific VSIX
  intellij        Package the JetBrains plugin ZIP
  editors         Package both editor extensions
  all             Build the debug server and both editor extensions
  help            Show this help

Options:
  --compiler-path Build or install doria-lsp against a local Doria repository or doriac crate.
                  Supported by server, server-release, vscode, editors, and all.
                  This development mode leaves Cargo.toml and Cargo.lock unchanged.

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
