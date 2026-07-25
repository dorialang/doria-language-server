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

if ($compilerPath !== null && !in_array($target, ['server', 'server-release', 'all'], true)) {
    usage_error('--compiler-path is supported by the server, server-release, and all targets');
}

try {
    match ($target) {
        'server' => build_server($root, false, $compilerPath),
        'server-release' => build_server($root, true, $compilerPath),
        'install-server' => install_server($root),
        'vscode' => build_vscode($root),
        'intellij' => build_intellij($root),
        'editors' => build_editors($root),
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
    if ($compilerSource['workspace'] !== null) {
        $workspaceArtifact =
            $compilerSource['workspace'] . "/target/{$profile}/{$executable}";
        install_executable($localArtifact, $workspaceArtifact);
        require_artifact($workspaceArtifact, 'compiler-workspace language-server executable');
    }
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

    $vsix = $dist . '/doria-language-support.vsix';
    // Remove the prior artifact so a packaging step that does not overwrite (or
    // no-ops) can never leave a stale file that require_artifact would report as
    // freshly built.
    remove_stale_artifacts($vsix);

    run_tool('npm', ['ci', '--ignore-scripts'], $editor);
    run_tool('npm', ['run', 'package', '--', '--out', $vsix], $editor);

    require_artifact($vsix, 'VS Code extension');
}

function build_intellij(string $root): void
{
    $editor = $root . '/editors/intellij/doria';
    // Remove prior plugin ZIPs first. Gradle's buildPlugin is incremental: when
    // the plugin inputs are unchanged it reports the Zip task UP-TO-DATE and does
    // not rewrite the version-stamped ZIP, so a stale artifact would linger and
    // the newest-by-mtime glob below would report it as this run's output.
    // Deleting the ZIP also makes the Zip task's output missing, so Gradle
    // regenerates it.
    remove_stale_artifacts($editor . '/build/distributions/*.zip');

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

function build_all(string $root, ?string $compilerPath = null): void
{
    build_server($root, false, $compilerPath);
    build_editors($root);
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
    if (!copy($source, $destination)) {
        throw new RuntimeException(
            "could not install local-compiler language server: {$destination}"
        );
    }
    if (PHP_OS_FAMILY !== 'Windows' && !chmod($destination, 0755)) {
        throw new RuntimeException("could not make language-server executable: {$destination}");
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
  vscode          Package dist/doria-language-support.vsix
  intellij        Package the JetBrains plugin ZIP
  editors         Package both editor extensions
  all             Build the debug server and both editor extensions
  help            Show this help

Options:
  --compiler-path Build doria-lsp against a local Doria repository or doriac crate.
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
