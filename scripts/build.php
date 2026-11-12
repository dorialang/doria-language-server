#!/usr/bin/env php
<?php

declare(strict_types=1);

// Checks include this file for its functions; only a direct run builds.
if (PHP_SAPI !== 'cli' || realpath($_SERVER['SCRIPT_FILENAME'] ?? '') !== __FILE__) {
    return;
}

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
        ['server', 'server-release', 'install-server', 'vscode', 'intellij', 'editors', 'all'],
        true,
    )
) {
    usage_error(
        '--compiler-path is supported by the server, server-release, install-server, '
            . 'vscode, intellij, editors, and all targets'
    );
}

try {
    match ($target) {
        'server' => build_server($root, false, $compilerPath),
        'server-release' => build_server($root, true, $compilerPath),
        'install-server' => install_server($root, $compilerPath),
        'vscode' => build_vscode($root, $compilerPath),
        'intellij' => build_intellij($root, $compilerPath),
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
): string {
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

[profile.dev]
debug = "line-tables-only"

[profile.dev.package."*"]
debug = false

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
    seed_local_runner_lock($root . '/Cargo.lock', $runner . '/Cargo.lock');
    $update = [
        'cargo',
        'update',
        '--manifest-path',
        $runner . '/Cargo.toml',
        '--package',
        'doriac',
    ];
    if (getenv('CARGO_NET_OFFLINE') === 'true') {
        $update[] = '--offline';
    }
    run_command($update, $root);

    $targetDirectory = local_compiler_target_directory($root);
    $command = [
        'cargo',
        'build',
        '--manifest-path',
        $runner . '/Cargo.toml',
        '--target-dir',
        $targetDirectory,
        '--bin',
        'doria-lsp',
    ];
    if ($release) {
        $command[] = '--release';
    }
    run_command($command, $root);

    $profile = $release ? 'release' : 'debug';
    $executable = PHP_OS_FAMILY === 'Windows' ? 'doria-lsp.exe' : 'doria-lsp';
    $localArtifact = $targetDirectory . "/{$profile}/{$executable}";
    require_artifact($localArtifact, 'local-compiler language-server executable');

    $artifact = $root . "/target/{$profile}/{$executable}";
    install_executable($localArtifact, $artifact);
    write_generated_file(local_server_marker($root, $release), $compiler . "\n");

    require_artifact($artifact, 'language-server executable');
    fwrite(STDOUT, "local compiler crate: {$compiler}\n");

    return $artifact;
}

function local_compiler_target_directory(string $root): string
{
    $configured = getenv('CARGO_TARGET_DIR');
    if ($configured === false || $configured === '') {
        return $root . '/target/local-doria-lsp';
    }

    return is_absolute_path($configured)
        ? rtrim($configured, '/\\')
        : $root . '/' . rtrim($configured, '/\\');
}

function seed_local_runner_lock(string $source, string $destination): void
{
    if (!is_file($source)) {
        throw new RuntimeException("canonical Cargo lockfile was not found at {$source}");
    }
    if (!copy($source, $destination)) {
        throw new RuntimeException("could not seed local runner lockfile: {$destination}");
    }
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
        $artifact = build_server_with_local_compiler($root, true, $compilerPath);
        $destination = cargo_install_root()
            . '/bin/'
            . (PHP_OS_FAMILY === 'Windows' ? 'doria-lsp.exe' : 'doria-lsp');
        install_executable($artifact, $destination);
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
    // Named before anything is built, so a server this package cannot carry
    // is reported before the build is paid for.
    $target = vscode_target(server_target_triple($root));
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
        ['run', 'package', '--', '--target', $target, '--out', $vsix],
        $editor
    );

    require_artifact($bundledServer, 'bundled VS Code language server');
    require_artifact($vsix, 'VS Code extension');
}

function build_intellij(string $root, ?string $compilerPath = null): void
{
    $editor = $root . '/editors/intellij/doria';
    $platform = intellij_platform_key(server_target_triple($root));
    build_server($root, true, $compilerPath);
    $server = server_artifact($root, true);
    $bundle = $root . '/target/intellij-lsp-bundle';
    $executable = PHP_OS_FAMILY === 'Windows' ? 'doria-lsp.exe' : 'doria-lsp';
    remove_stale_artifacts($bundle . '/*/doria-lsp*');
    install_executable($server, "{$bundle}/{$platform}/{$executable}");

    $gradle = PHP_OS_FAMILY === 'Windows' ? $editor . '/gradlew.bat' : $editor . '/gradlew';
    $arguments = [
        'buildPlugin',
        '--no-daemon',
        "-PdoriaLspBundleDir={$bundle}",
    ];
    run_tool($gradle, $arguments, $editor);

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
    build_intellij($root, $compilerPath);
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

/**
 * The Rust target `doria-lsp` is built for: the host of the toolchain this
 * repository pins.
 *
 * An editor package is named for the server it carries, so the platform is
 * asked of the toolchain that built that server. The PHP running this script
 * says nothing about it: a 32-bit PHP on 64-bit Windows reports `i586` while
 * Cargo builds an x86_64 server.
 */
function server_target_triple(string $root): string
{
    $requested = getenv('CARGO_BUILD_TARGET');
    if ($requested !== false && $requested !== '') {
        throw new RuntimeException(
            "CARGO_BUILD_TARGET is set to '{$requested}', but this script builds and packages "
                . 'the server for the toolchain host. Unset CARGO_BUILD_TARGET, then run this command again.'
        );
    }

    $version = capture_command(['rustc', '-vV'], $root);
    if (preg_match('/^host:\s*(\S+)\s*$/m', $version, $match) !== 1) {
        throw new RuntimeException(
            "`rustc -vV` did not report a host target. Run `rustc -vV` in {$root} and check the Rust installation."
        );
    }

    return $match[1];
}

/**
 * The system and processor a Rust target names, in the target's own words.
 *
 * @return array{system: string, processor: string}
 */
function target_platform(string $triple): array
{
    $system = match (true) {
        str_contains($triple, '-windows-') => 'windows',
        str_contains($triple, '-apple-darwin') => 'macos',
        str_contains($triple, '-linux-') => 'linux',
        default => throw new RuntimeException(
            "Editor packaging does not support the Rust target {$triple}: only Windows, macOS and Linux "
                . 'servers are packaged. Build on one of those systems.'
        ),
    };

    return ['system' => $system, 'processor' => explode('-', $triple, 2)[0]];
}

function vscode_target(string $triple): string
{
    ['system' => $system, 'processor' => $processor] = target_platform($triple);
    $platform = match ($system) {
        'windows' => 'win32',
        'macos' => 'darwin',
        'linux' => 'linux',
    };
    $architecture = match (true) {
        $processor === 'x86_64' => 'x64',
        $processor === 'aarch64' => 'arm64',
        $system === 'linux' && str_starts_with($processor, 'armv7') => 'armhf',
        default => throw new RuntimeException(
            "VS Code packaging does not support the Rust target {$triple}. " . supported_hosts_advice($system)
        ),
    };

    return "{$platform}-{$architecture}";
}

function intellij_platform_key(string $triple): string
{
    ['system' => $system, 'processor' => $processor] = target_platform($triple);
    if (!in_array($processor, ['x86_64', 'aarch64'], true)) {
        throw new RuntimeException(
            "IntelliJ packaging does not support the Rust target {$triple}. " . supported_hosts_advice($system)
        );
    }

    return "{$system}-{$processor}";
}

function supported_hosts_advice(string $system): string
{
    $vendor = match ($system) {
        'windows' => 'pc-windows-msvc',
        'macos' => 'apple-darwin',
        'linux' => 'unknown-linux-gnu',
    };

    return "Install a Rust toolchain whose host is x86_64-{$vendor} or aarch64-{$vendor} "
        . '(`rustup set default-host <host>`, then `rustup toolchain install`), then run this command again.';
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
    run_command(tool_command($tool, $arguments), $workingDirectory);
}

/**
 * The command that starts `$tool`, a name to find on PATH or a path to a
 * file, with `$arguments`.
 *
 * The file is found here and started by its full path. On Windows `npm` and
 * `gradlew` are batch files. A batch file that the command processor finds on
 * PATH from a quoted name reads its own directory (`%~dp0`) as the working
 * directory, so `npm.cmd` looked for npm inside the directory being built.
 *
 * The result is a list of arguments for a program, or on Windows a whole
 * command line for a batch file; see `windows_batch_command_line`.
 *
 * @param list<string> $arguments
 * @return list<string>|string
 */
function tool_command(string $tool, array $arguments): array|string
{
    $executable = resolve_tool($tool);
    if (PHP_OS_FAMILY !== 'Windows') {
        return [$executable, ...$arguments];
    }

    $executable = str_replace('/', '\\', $executable);
    $extension = strtolower(pathinfo($executable, PATHINFO_EXTENSION));

    return in_array($extension, ['bat', 'cmd'], true)
        ? windows_batch_command_line($executable, $arguments)
        : [$executable, ...$arguments];
}

/**
 * The file a tool name stands for: the first match on PATH, tried with each
 * PATHEXT extension on Windows when the name has none. A tool given with a
 * directory is the caller's choice of file and is returned unchanged. The
 * working directory is never searched.
 */
function resolve_tool(string $tool): string
{
    if (is_absolute_path($tool) || strpbrk($tool, '/\\') !== false) {
        return $tool;
    }

    $windows = PHP_OS_FAMILY === 'Windows';
    $extensions = [''];
    if ($windows && pathinfo($tool, PATHINFO_EXTENSION) === '') {
        $configured = getenv('PATHEXT');
        if ($configured === false || $configured === '') {
            $configured = '.COM;.EXE;.BAT;.CMD';
        }
        $extensions = array_values(array_filter(explode(';', $configured)));
    }

    $path = getenv('PATH');
    foreach (explode(PATH_SEPARATOR, $path === false ? '' : $path) as $directory) {
        $directory = rtrim(trim($directory, '"'), '/\\');
        if ($directory === '') {
            continue;
        }
        foreach ($extensions as $extension) {
            $candidate = $directory . DIRECTORY_SEPARATOR . $tool . $extension;
            if (is_file($candidate) && ($windows || is_executable($candidate))) {
                return $candidate;
            }
        }
    }

    throw new RuntimeException(
        "'{$tool}' was not found on PATH. Install it, or add its directory to PATH "
            . 'and open a new terminal, then run this command again.'
    );
}

/**
 * The Windows command line that runs a batch file with `$arguments`.
 *
 * A batch file runs inside the command processor, which reads one line of
 * text, so this function writes that line itself: the file and every argument
 * in double quotes, the whole line in one more pair, after `/s /c`. With `/s`
 * the command processor removes exactly that outer pair and runs the rest
 * unchanged.
 *
 * Handing the command processor a list of separately quoted arguments does
 * not work. Without `/s` it removes the first and the last quote of the line,
 * so the last argument loses its closing quote and takes in whatever the batch
 * file wrote after `%*`. `gradlew.bat` ends its Java command with
 * `%* & call :exitWithErrorLevel`, and that text became part of the last
 * Gradle argument.
 *
 * @param list<string> $arguments
 */
function windows_batch_command_line(string $batchFile, array $arguments): string
{
    $commandProcessor = getenv('COMSPEC');
    if ($commandProcessor === false || $commandProcessor === '') {
        $commandProcessor = 'cmd.exe';
    }

    $line = implode(' ', array_map(windows_batch_argument(...), [$batchFile, ...$arguments]));

    return '"' . $commandProcessor . '" /d /s /c "' . $line . '"';
}

/**
 * One argument in double quotes for a batch file, which hands it on to a
 * program. Backslashes at the end are doubled, because the program reads a
 * backslash before the closing quote as part of the quote.
 *
 * A double quote, a percent sign or a line break cannot be passed through the
 * command processor unchanged, so an argument that has one is refused.
 */
function windows_batch_argument(string $argument): string
{
    if (preg_match('/["%\r\n\0]/', $argument) === 1) {
        throw new RuntimeException(
            'cannot pass ' . var_export($argument, true) . ' to a Windows batch file: the command processor '
                . 'rewrites double quotes, percent signs and line breaks. Remove that character from the '
                . 'path or value, then run this command again.'
        );
    }

    $trailing = strlen($argument) - strlen(rtrim($argument, '\\'));

    return '"' . $argument . str_repeat('\\', $trailing) . '"';
}

/**
 * Starts a command: a list of arguments for a program, or a whole Windows
 * command line, which is passed on exactly as written.
 *
 * @param list<string>|string $command
 * @param array<int, mixed> $descriptors
 * @param-out array<int, resource> $pipes
 * @return resource|false
 */
function start_command(array|string $command, array $descriptors, &$pipes, string $workingDirectory)
{
    return proc_open(
        $command,
        $descriptors,
        $pipes,
        $workingDirectory,
        null,
        is_string($command) ? ['bypass_shell' => true] : null
    );
}

/** @param list<string>|string $command */
function run_command(array|string $command, string $workingDirectory): void
{
    fwrite(STDOUT, "\n> " . display_command($command) . "\n");
    $process = start_command(
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

/** @param list<string>|string $command */
function capture_command(array|string $command, string $workingDirectory): string
{
    $process = start_command(
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

/** @param list<string>|string $command */
function display_command(array|string $command): string
{
    if (is_string($command)) {
        return $command;
    }

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
  intellij        Build doria-lsp and bundle it in the JetBrains plugin ZIP
  editors         Package both editor extensions
  all             Build the debug server and both editor extensions
  help            Show this help

Options:
  --compiler-path Build or install doria-lsp against a local Doria repository or doriac crate.
                  Supported by server, server-release, vscode, intellij, editors, and all.
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
