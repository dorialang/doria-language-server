#!/usr/bin/env php
<?php

declare(strict_types=1);

/**
 * Check that Doria editor highlighters cover the accepted token vocabulary.
 */

$root = dirname(__DIR__);

$cargoManifest = $root . '/Cargo.toml';
$readme = $root . '/README.md';
$vscodePackage = $root . '/editors/vscode/doria/package.json';
$vscodeGrammar = $root . '/editors/vscode/doria/syntaxes/doria.tmLanguage.json';
$vscodeLanguageConfiguration = $root . '/editors/vscode/doria/language-configuration.json';
$vscodeExtension = $root . '/editors/vscode/doria/extension.js';
$vscodeDebugSupport = $root . '/editors/vscode/doria/debug-support.js';
$vscodeLauncherPath = $root . '/editors/vscode/doria/launcher-path.js';
$vscodeServerPath = $root . '/editors/vscode/doria/server-path.js';
$vscodeIcon = $root . '/editors/vscode/doria/icons/doria.svg';
$buildScript = $root . '/scripts/build.php';
$intellijLexer = $root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/highlighting/DoriaLexer.kt';
$intellijLanguage = $root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/DoriaLanguage.kt';
$intellijBuildGradle = $root . '/editors/intellij/doria/build.gradle';
$intellijTokenTypes = $root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/highlighting/DoriaTokenTypes.kt';
$intellijSyntaxHighlighter = $root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/highlighting/DoriaSyntaxHighlighter.kt';
$intellijLexerTest = $root . '/editors/intellij/doria/src/test/kotlin/dev/doria/intellij/highlighting/DoriaLexerTest.kt';
$intellijCodeStyleProvider = $root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/codestyle/DoriaLanguageCodeStyleSettingsProvider.kt';
$intellijFormatter = $root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/codestyle/DoriaFormattingModelBuilder.kt';
$intellijParser = $root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/psi/DoriaParserDefinition.kt';
$intellijFormatterTest = $root . '/editors/intellij/doria/src/test/kotlin/dev/doria/intellij/codestyle/DoriaFormattingModelBuilderTest.kt';
$intellijLspFiles = $root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/lsp/DoriaLspFiles.kt';
$intellijLspResolver = $root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/lsp/DoriaLspServerPathResolver.kt';
$intellijPluginXml = $root . '/editors/intellij/doria/src/main/resources/META-INF/plugin.xml';
$intellijPluginIcon = $root . '/editors/intellij/doria/src/main/resources/META-INF/pluginIcon.svg';
$intellijCreateClassAction = $root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/actions/DoriaCreateClassAction.kt';
$doriaLogo = $root . '/res/images/doria-app-icon-warm.svg';
$releaseWorkflow = $root . '/.github/workflows/release.yml';
$lspAnalysis = $root . '/server/src/analysis.rs';
$lspServer = $root . '/server/src/lib.rs';
$lspTests = $root . '/server/tests/lsp_tests.rs';
$fixture = $root . '/editors/fixtures/latest-tokens.doria';
$rejectedFixture = $root . '/editors/fixtures/rejected-syntax.doria';

$groupedLocalForms = [
    'let $left, $right = 0;',
    'let writable $red, $green, $blue = 0;',
    'int $minimum, $maximum = 0;',
    'writable int $x, $y = 0;',
];

$acceptedKeywords = [
    'class',
    'interface',
    'trait',
    'extends',
    'implements',
    'function',
    'const',
    'static',
    'self',
    'parent',
    'let',
    'take',
    'writable',
    'readonly',
    'internal',
    'namespace',
    'use',
    'uses',
    'as',
    'include',
    'declare',
    'echo',
    'return',
    'if',
    'else',
    'while',
    'for',
    'foreach',
    'break',
    'continue',
    'is',
    'true',
    'false',
    'null',
    'new',
    'throw',
    'throws',
    'try',
    'catch',
    'finally',
    'enum',
    'case',
    'match',
    'when',
    'given',
    'default',
    'do',
    'fn',
    'get',
    'set',
    'insteadof',
    'shared',
    'spawn',
    'scope',
    'async',
    'await',
    'unsafe',
    'extern',
    'open',
    'override',
    'with',
    'take',
    'once',
];

$plannedKeywords = [
    'async',
    'await',
    'unsafe',
    'extern',
    'open',
    'override',
    'get',
    'set',
    'insteadof',
    'spawn',
    'scope',
];
$primitiveTypes = [
    'void',
    'int',
    'int8',
    'int16',
    'int32',
    'int64',
    'uint8',
    'uint16',
    'uint32',
    'uint64',
    'float',
    'float32',
    'float64',
    'string',
    'bool',
    'mixed',
];

$implementedIntegerTypes = [
    'int',
    'int8',
    'int16',
    'int32',
    'int64',
    'uint8',
    'uint16',
    'uint32',
    'uint64',
];

$implementedStage14ScalarTypes = [
    'float',
    'float32',
    'float64',
    'bool',
];

$reservedTypes = [
    'resource',
];

$plannedTypes = [
    'SharedReference',
    'WeakReference',
    'WritableSharedReference',
    'WritableWeakReference',
    'ReadonlySharedReferenceAccess',
    'WritableSharedReferenceAccess',
    'Sendable',
    'Shareable',
    'Ptr',
    'MutPtr',
    'Bytes',
    'List',
    'Dictionary',
    'Set',
    'SortedDictionary',
    'SortedSet',
    'PriorityQueue',
    'Deque',
];

$rejectedTypes = [
    'array',
    'object',
    'never',
];

$stage28GuardForms = [
    'Option::Some($value) if $value > 0 => $value,',
    'match (take $loadResult)',
];

$stage28aControlFlowForms = [
    'given {',
    '} if ($prepared) {',
    '} when ($prepared): string {',
    '} while ($remaining > 0) {',
    'do {',
    '} while ($count < 2);',
    '} finally {',
];

$lspSupportedTypes = [
    'void',
    'int',
    'int8',
    'int16',
    'int32',
    'int64',
    'uint8',
    'uint16',
    'uint32',
    'uint64',
    'float',
    'float32',
    'float64',
    'string',
    'bool',
    'mixed',
    'List',
    'Dictionary',
    'Set',
    'SortedDictionary',
    'SortedSet',
    'PriorityQueue',
    'Deque',
    'Bytes',
    'SharedReference',
    'WeakReference',
    'WritableSharedReference',
    'WritableWeakReference',
    'ReadonlySharedReferenceAccess',
    'WritableSharedReferenceAccess',
];
$wordOperators = ['not', 'and', 'or', 'xor'];
$stage13SymbolOperators = [
    '-',
    '~',
    '+',
    '*',
    '/',
    '%',
    '<<',
    '>>',
    '&',
    '|',
    '^',
    '==',
    '!=',
    '<',
    '<=',
    '>',
    '>=',
    '++',
    '--',
    '+=',
    '-=',
    '*=',
    '/=',
    '%=',
    '<<=',
    '>>=',
    '&=',
    '|=',
    '^=',
];
$booleanSymbolOperators = ['!', '&&', '||'];
$rejectedPreprocessor = [
    'include',
    'define',
    'undef',
    'if',
    'ifdef',
    'ifndef',
    'elif',
    'else',
    'endif',
    'warning',
    'error',
];
$rejectedKeywords = ['goto', 'require', 'require_once', 'include_once', 'print', 'instanceof'];
$strictComparison = ['===', '!=='];
$notKeywords = ['public', 'private', 'protected', 'Result', 'object'];
function fail_check(string $message): never
{
    fwrite(STDERR, "editor highlighting check failed: {$message}\n");
    exit(1);
}

function require_check(bool $condition, string $message): void
{
    if (!$condition) {
        fail_check($message);
    }
}

function any_match(iterable $items, callable $predicate): bool
{
    foreach ($items as $item) {
        if ($predicate($item)) {
            return true;
        }
    }

    return false;
}

function relative_path(string $path): string
{
    global $root;

    $prefix = $root . '/';
    if (str_starts_with($path, $prefix)) {
        return substr($path, strlen($prefix));
    }

    return $path;
}

function read_text(string $path): string
{
    $text = @file_get_contents($path);
    if ($text === false) {
        fail_check(relative_path($path) . ' could not be read');
    }

    return $text;
}

function load_json(string $path): mixed
{
    try {
        return json_decode(read_text($path), true, 512, JSON_THROW_ON_ERROR);
    } catch (JsonException $error) {
        fail_check(relative_path($path) . ' is not valid JSON: ' . $error->getMessage());
    }
}

/** @return Generator<array<string, mixed>> */
function walk_patterns(mixed $node): Generator
{
    if (!is_array($node)) {
        return;
    }

    if (array_key_exists('match', $node) || array_key_exists('begin', $node) || array_key_exists('name', $node)) {
        yield $node;
    }

    foreach ($node as $value) {
        yield from walk_patterns($value);
    }
}

function regex_matches(string $pattern, string $subject): bool
{
    $regex = '~' . str_replace('~', '\\~', $pattern) . '~';
    $result = @preg_match($regex, $subject);
    if ($result === false) {
        fail_check("TextMate regex {$pattern} could not be evaluated by PHP PCRE: " . preg_last_error_msg());
    }

    return $result === 1;
}

function regex_fully_matches(string $pattern, string $subject): bool
{
    return regex_matches('\\A(?:' . $pattern . ')\\z', $subject);
}

function check_vscode_package(): void
{
    global $vscodePackage, $vscodeIcon, $doriaLogo;

    $package = load_json($vscodePackage);
    require_check(
        ($package['version'] ?? null) === '2026.3.1-canary' &&
            ($package['doriaToolchainVersion'] ?? null) === '2026.03.1-canary',
        'VS Code package must carry the pre-1.0 canary encoding of Doria CalVer 2026.03.1-canary'
    );
    $grammars = $package['contributes']['grammars'] ?? [];
    require_check(
        any_match(
            $grammars,
            static fn (mixed $grammar): bool => is_array($grammar)
                && ($grammar['language'] ?? null) === 'doria'
                && ($grammar['scopeName'] ?? null) === 'source.doria'
                && ($grammar['path'] ?? null) === './syntaxes/doria.tmLanguage.json'
        ),
        'VS Code package.json must map doria/source.doria to ./syntaxes/doria.tmLanguage.json'
    );
    $languages = $package['contributes']['languages'] ?? [];
    require_check(
        any_match(
            $languages,
            static fn (mixed $language): bool => is_array($language)
                && ($language['id'] ?? null) === 'doria'
                && ($language['icon']['light'] ?? null) === './icons/doria.svg'
                && ($language['icon']['dark'] ?? null) === './icons/doria.svg'
        ),
        'VS Code must associate .doria files with the canonical Doria icon'
    );
    require_check(is_file($vscodeIcon), 'VS Code must package icons/doria.svg');
    require_check(
        rtrim(read_text($vscodeIcon)) === rtrim(read_text($doriaLogo)),
        'VS Code file icon must use the canonical Doria README SVG'
    );
    $debuggers = $package['contributes']['debuggers'] ?? [];
    require_check(
        any_match(
            $debuggers,
            static fn (mixed $debugger): bool => is_array($debugger)
                && ($debugger['type'] ?? null) === 'doria'
                && in_array('doria', $debugger['languages'] ?? [], true)
                && ($debugger['initialConfigurations'][0]['mode'] ?? null) === 'project'
                && ($debugger['initialConfigurations'][0]['noDebug'] ?? null) === true
        )
        && in_array('onDebug:doria', $package['activationEvents'] ?? [], true)
        && isset($package['contributes']['configuration']['properties']['doria.baton.path'])
        && isset($package['contributes']['configuration']['properties']['doria.compiler.path']),
        'VS Code must contribute a Doria launch profile that defaults to Baton project execution'
    );

    $defaults = $package['contributes']['configurationDefaults']['[doria]'] ?? [];
    require_check(
        ($defaults['editor.insertSpaces'] ?? null) === true &&
            ($defaults['editor.tabSize'] ?? null) === 4 &&
            ($defaults['editor.detectIndentation'] ?? null) === true &&
            ($defaults['editor.wordWrapColumn'] ?? null) === 120,
        'VS Code must provide the PHP-shaped Doria editor defaults without overriding user or workspace settings'
    );
}

function check_vscode_language_server_packaging(): void
{
    global $vscodeExtension, $vscodeDebugSupport, $vscodeLauncherPath, $vscodeServerPath, $buildScript;

    $extension = read_text($vscodeExtension);
    $debugSupport = read_text($vscodeDebugSupport);
    $launcherResolver = read_text($vscodeLauncherPath);
    $resolver = read_text($vscodeServerPath);
    $builder = read_text($buildScript);

    require_check(
        str_contains($extension, 'require("./server-path")')
            && str_contains($extension, 'resolution.rejectedPaths'),
        'VS Code client must use the tested server resolver and report ignored stale overrides'
    );
    require_check(
        str_contains($resolver, 'path.join(extensionPath, "bin", executable)')
            && str_contains($resolver, 'pathExists(resolved)')
            && str_contains($resolver, 'source: "bundled"'),
        'VS Code server resolution must ignore missing overrides and load the bundled native server'
    );
    require_check(
        str_contains($builder, 'build_server($root, true, $compilerPath)')
            && str_contains($builder, "install_executable(\$server, \$bundledServer)")
            && str_contains($builder, "'--target', vscode_target()"),
        'VS Code packaging must build, bundle, and platform-target doria-lsp'
    );
    require_check(
        str_contains(
            $builder,
            "seed_local_runner_lock(\$root . '/Cargo.lock', \$runner . '/Cargo.lock')"
        ) && str_contains($builder, 'function seed_local_runner_lock('),
        'local compiler builds must reseed the disposable runner from the canonical Cargo lockfile'
    );
    require_check(
        str_contains($extension, 'registerDebugConfigurationProvider')
            && str_contains($extension, 'registerDebugAdapterDescriptorFactory')
            && str_contains($extension, 'resolveBatonPath')
            && str_contains($debugSupport, 'findBatonProjectRoot')
            && str_contains($launcherResolver, 'executableName: "baton"'),
        'VS Code must resolve Baton projects and register the Doria launch adapter'
    );
}

function check_intellij_language_server_packaging(): void
{
    global $intellijBuildGradle, $intellijLspResolver, $releaseWorkflow, $buildScript;

    $gradle = read_text($intellijBuildGradle);
    $resolver = read_text($intellijLspResolver);
    $release = read_text($releaseWorkflow);
    $builder = read_text($buildScript);

    foreach ([
        'linux-x86_64',
        'linux-aarch64',
        'macos-x86_64',
        'macos-aarch64',
        'windows-x86_64',
        'windows-aarch64',
    ] as $platform) {
        require_check(
            str_contains($gradle, $platform) && str_contains($release, $platform),
            "IntelliJ packaging is missing the {$platform} language server"
        );
    }
    require_check(
        str_contains($resolver, 'resolve("bin").resolve(platform).resolve(executable)')
            && strpos($resolver, 'bundledCandidate(') < strpos($resolver, 'cargoBinCandidates('),
        'IntelliJ must resolve its bundled compiler-matched server before Cargo and PATH fallbacks'
    );
    require_check(
        str_contains($gradle, "into 'bin'")
            && str_contains($gradle, 'requireUniversalDoriaLspBundle')
            && str_contains($release, 'needs: server-binaries'),
        'IntelliJ release packaging must fail unless all native language servers are bundled'
    );
    require_check(
        str_contains($builder, 'build_intellij($root, $compilerPath)')
            && str_contains($builder, 'intellij_platform_key()')
            && str_contains($builder, '-PdoriaLspBundleDir='),
        'The developer IntelliJ build must bundle the compiler-matched host server'
    );
}

function check_vscode_language_configuration(): void
{
    global $vscodeLanguageConfiguration;

    $config = load_json($vscodeLanguageConfiguration);
    foreach (['brackets', 'autoClosingPairs', 'surroundingPairs'] as $key) {
        $json = json_encode($config[$key] ?? null, JSON_THROW_ON_ERROR);
        require_check(str_contains($json, '#[') && str_contains($json, ']'), 'VS Code language configuration must include #[...] behavior in ' . $key);
    }

    require_check(
        isset($config['indentationRules']['increaseIndentPattern'], $config['indentationRules']['decreaseIndentPattern']),
        'VS Code language configuration must define Doria indentation rules'
    );
    require_check(
        any_match(
            $config['onEnterRules'] ?? [],
            static fn (mixed $rule): bool => is_array($rule) &&
                ($rule['action']['appendText'] ?? null) === ' * '
        ),
        'VS Code language configuration must continue PHP-style documentation comments'
    );
    foreach ([
        ['^.*\\([^\\)]*$', '^\\s*\\).*$'],
        ['^.*\\{[^\\}]*$', '^\\s*\\}.*$'],
        ['^.*\\[[^\\]]*$', '^\\s*\\].*$'],
    ] as [$beforeText, $afterText]) {
        require_check(
            any_match(
                $config['onEnterRules'] ?? [],
                static fn (mixed $rule): bool => is_array($rule)
                    && ($rule['beforeText'] ?? null) === $beforeText
                    && ($rule['afterText'] ?? null) === $afterText
                    && ($rule['action']['indent'] ?? null) === 'indentOutdent'
                    && !array_key_exists('appendText', $rule['action'])
            ),
            'VS Code delimiter Enter rules must defer indentation text to editor settings'
        );
    }
    require_check(
        !any_match(
            $config['onEnterRules'] ?? [],
            static fn (mixed $rule): bool => is_array($rule)
                && ($rule['action']['appendText'] ?? null) === '\\t'
        ),
        'VS Code Enter rules must not insert a literal escaped tab'
    );
}

function check_vscode_grammar(): void
{
    global $acceptedKeywords, $primitiveTypes, $reservedTypes, $plannedTypes, $wordOperators, $stage13SymbolOperators, $booleanSymbolOperators;
    global $notKeywords, $strictComparison, $rejectedPreprocessor, $rejectedKeywords, $rejectedTypes, $vscodeGrammar;

    $grammar = load_json($vscodeGrammar);
    $grammarText = json_encode($grammar, JSON_THROW_ON_ERROR);
    $patterns = iterator_to_array(walk_patterns($grammar), false);

    $interpolationPatterns = array_values(array_filter(
        $patterns,
        static fn (array $pattern): bool => ($pattern['name'] ?? null) === 'meta.interpolation.expression.doria'
    ));
    require_check($interpolationPatterns !== [], 'VS Code grammar must define interpolation scopes');
    $interpolationJson = json_encode($interpolationPatterns, JSON_THROW_ON_ERROR);
    require_check(
        str_contains($interpolationJson, '#interpolationExpression') &&
            str_contains($interpolationJson, '#strings') &&
            str_contains($interpolationJson, '#calls') &&
            str_contains($interpolationJson, '#operators') &&
            !str_contains($interpolationJson, '(?=\\$)'),
        'VS Code interpolation must embed balanced ordinary Doria expressions rather than a variable-only grammar'
    );
    require_check(
        str_contains($grammarText, 'keyword.declaration.implements.doria'),
        'VS Code must scope active implements syntax as a declaration keyword'
    );
    require_check(
        str_contains($grammarText, 'keyword.operator.type-test.doria'),
        'VS Code must scope the Stage 22 is operator as a type-test operator'
    );
    require_check(
        str_contains($grammarText, 'storage.modifier.mutability.doria') &&
            str_contains($grammarText, 'variable.other.property.doria'),
        'VS Code must scope shared construction as a modifier and referencedValue as a property'
    );
    foreach ([
        'keyword.declaration.enum.doria',
        'entity.name.type.enum.doria',
        'keyword.declaration.enum.case.doria',
        'constant.other.enum.case.doria',
    ] as $scope) {
        require_check(str_contains($grammarText, $scope), "VS Code grammar is missing accepted enum scope '{$scope}'");
    }
    foreach ([
        'keyword.declaration.constant.doria',
        'constant.other.doria',
        'constant.other.class.doria',
        'storage.modifier.static.doria',
        'variable.language.class-context.doria',
        'variable.other.property.static.doria',
        'entity.name.function.static.doria',
        'invalid.illegal.sigil.static-access.doria',
    ] as $scope) {
        require_check(str_contains($grammarText, $scope), "VS Code grammar is missing Stage 20 scope '{$scope}'");
    }
    $staticAccessorPatterns = $grammar['repository']['accessors']['patterns'] ?? [];
    $staticMethodPatterns = [];
    $classConstantPatterns = [];
    $staticPropertyPatterns = [];
    $enumCasePatterns = [];
    foreach ($staticAccessorPatterns as $pattern) {
        $capture = $pattern['captures']['2']['name'] ?? null;
        if ($capture === 'entity.name.function.static.doria') {
            $staticMethodPatterns[] = (string) ($pattern['match'] ?? '');
        } elseif ($capture === 'constant.other.class.doria') {
            $classConstantPatterns[] = (string) ($pattern['match'] ?? '');
        } elseif ($capture === 'variable.other.property.static.doria') {
            $staticPropertyPatterns[] = (string) ($pattern['match'] ?? '');
        } elseif ($capture === 'constant.other.enum.case.doria') {
            $enumCasePatterns[] = (string) ($pattern['match'] ?? '');
        }
    }
    require_check(
        any_match($enumCasePatterns, static fn (string $match): bool => regex_matches($match, '::Draft')) &&
            any_match($enumCasePatterns, static fn (string $match): bool => regex_matches($match, '::Circle(')) &&
            !any_match($enumCasePatterns, static fn (string $match): bool => regex_matches($match, '::MAX_DEPTH')),
        'VS Code must classify unit and payload enum-case access without treating every PascalCase identifier as a case'
    );
    require_check(
        any_match($staticMethodPatterns, static fn (string $match): bool => regex_matches($match, '::nextId(')),
        'VS Code must classify a parenthesized :: member as a static method'
    );
    require_check(
        any_match($classConstantPatterns, static fn (string $match): bool => regex_matches($match, '::MAX_DEPTH')) &&
            !any_match($classConstantPatterns, static fn (string $match): bool => regex_matches($match, '::nextId(')),
        'VS Code must classify non-call :: members as class constants without swallowing static methods'
    );
    require_check(
        any_match($staticPropertyPatterns, static fn (string $match): bool => regex_matches($match, '::next')) &&
            any_match($staticPropertyPatterns, static fn (string $match): bool => regex_matches($match, '::age')) &&
            !any_match($staticPropertyPatterns, static fn (string $match): bool => regex_matches($match, '::MAX_DEPTH')) &&
            !any_match($staticPropertyPatterns, static fn (string $match): bool => regex_matches($match, '::$age')),
        'VS Code must classify lowercase ::name as static-property access without swallowing class constants'
    );

    $invalidStaticSigilMatches = [];
    foreach ($patterns as $pattern) {
        if (($pattern['captures']['3']['name'] ?? null) === 'invalid.illegal.sigil.static-access.doria') {
            $invalidStaticSigilMatches[] = (string) ($pattern['match'] ?? '');
        }
    }
    require_check(
        any_match($invalidStaticSigilMatches, static fn (string $match): bool => regex_matches($match, '::$age')),
        'VS Code must mark only the PHP-style static-property sigil invalid'
    );

    $tokens = array_unique([...$acceptedKeywords, ...$primitiveTypes, ...$reservedTypes, ...$plannedTypes, ...$wordOperators]);
    sort($tokens);
    foreach ($tokens as $token) {
        require_check(str_contains($grammarText, $token), "VS Code grammar is missing '{$token}'");
    }

    foreach ($rejectedTypes as $type) {
        require_check(!str_contains($grammarText, $type), "VS Code grammar must not highlight rejected type '{$type}'");
    }

    $primitiveTypeMatches = [];
    foreach ($patterns as $pattern) {
        if (($pattern['name'] ?? null) === 'storage.type.primitive.doria') {
            $primitiveTypeMatches[] = (string) ($pattern['match'] ?? '');
        }
    }
    require_check($primitiveTypeMatches !== [], 'VS Code grammar must define primitive type highlighting');
    foreach ($primitiveTypes as $type) {
        require_check(
            any_match($primitiveTypeMatches, static fn (string $match): bool => regex_fully_matches($match, $type)),
            "VS Code grammar must classify '{$type}' as a primitive type"
        );
    }

    sort($notKeywords);
    foreach ($notKeywords as $token) {
        require_check(!str_contains($grammarText, $token), "VS Code grammar must not treat '{$token}' as a keyword");
    }

    $normalOperatorMatches = [];
    foreach ($patterns as $pattern) {
        if (($pattern['name'] ?? null) === 'keyword.operator.doria') {
            $normalOperatorMatches[] = (string) ($pattern['match'] ?? '');
        }
    }
    require_check($normalOperatorMatches !== [], 'VS Code grammar must define normal operator highlighting');
    foreach (array_unique([...$stage13SymbolOperators, ...$booleanSymbolOperators]) as $operator) {
        require_check(
            any_match($normalOperatorMatches, static fn (string $match): bool => regex_fully_matches($match, $operator)),
            "VS Code grammar must highlight '{$operator}' as one complete operator token"
        );
    }

    $wordOperatorMatches = [];
    foreach ($patterns as $pattern) {
        if (($pattern['name'] ?? null) === 'keyword.operator.logical.word.doria') {
            $wordOperatorMatches[] = (string) ($pattern['match'] ?? '');
        }
    }
    foreach ($wordOperators as $operator) {
        require_check(
            any_match($wordOperatorMatches, static fn (string $match): bool => regex_fully_matches($match, $operator)),
            "VS Code grammar must classify '{$operator}' as a word operator"
        );
    }

    $logicalSymbolMatches = [];
    foreach ($patterns as $pattern) {
        if (($pattern['name'] ?? null) === 'keyword.operator.logical.symbol.doria') {
            $logicalSymbolMatches[] = (string) ($pattern['match'] ?? '');
        }
    }
    foreach ($booleanSymbolOperators as $operator) {
        require_check(
            any_match($logicalSymbolMatches, static fn (string $match): bool => regex_fully_matches($match, $operator)),
            "VS Code grammar must classify '{$operator}' as a logical operator"
        );
    }
    require_check(
        !any_match($logicalSymbolMatches, static fn (string $match): bool => regex_matches($match, '!=')),
        "VS Code logical-operator patterns must not split the accepted '!=' operator"
    );

    foreach ($strictComparison as $operator) {
        foreach ($normalOperatorMatches as $match) {
            require_check(
                !str_contains($match, $operator),
                "VS Code grammar must not highlight '{$operator}' as a normal operator"
            );
        }
    }

    $invalidOperatorPatterns = [];
    foreach ($patterns as $pattern) {
        if (($pattern['name'] ?? null) === 'invalid.illegal.operator.strict-comparison.doria') {
            $invalidOperatorPatterns[] = (string) ($pattern['match'] ?? '');
        }
    }
    foreach ($strictComparison as $operator) {
        require_check(
            any_match($invalidOperatorPatterns, static fn (string $match): bool => regex_fully_matches($match, $operator)),
            "VS Code grammar must mark '{$operator}' invalid"
        );
    }
    foreach (['&', '|'] as $operator) {
        require_check(
            !any_match($invalidOperatorPatterns, static fn (string $match): bool => regex_fully_matches($match, $operator)),
            "VS Code grammar must not mark single '{$operator}' invalid"
        );
    }

    $invalidPreprocessorPatterns = [];
    foreach ($patterns as $pattern) {
        if (($pattern['name'] ?? null) === 'invalid.illegal.preprocessor.doria') {
            $invalidPreprocessorPatterns[] = (string) ($pattern['match'] ?? '');
        }
    }
    require_check($invalidPreprocessorPatterns !== [], 'VS Code grammar must define invalid preprocessor highlighting');
    sort($rejectedPreprocessor);
    foreach ($rejectedPreprocessor as $directive) {
        require_check(
            any_match($invalidPreprocessorPatterns, static fn (string $match): bool => str_contains($match, $directive)),
            "VS Code grammar must mark #{$directive} invalid or unsupported"
        );
    }

    $invalidKeywordPatterns = [];
    foreach ($patterns as $pattern) {
        if (($pattern['name'] ?? null) === 'invalid.illegal.keyword.rejected.doria') {
            $invalidKeywordPatterns[] = (string) ($pattern['match'] ?? '');
        }
    }
    require_check($invalidKeywordPatterns !== [], 'VS Code grammar must define rejected keyword highlighting');
    foreach ($rejectedKeywords as $keyword) {
        require_check(
            any_match($invalidKeywordPatterns, static fn (string $match): bool => str_contains($match, $keyword)),
            'VS Code grammar must mark ' . $keyword . ' invalid or unsupported'
        );
    }
    require_check(
        any_match($invalidKeywordPatterns, static fn (string $match): bool => str_contains($match, 'use') && str_contains($match, '(?=')),
        'VS Code grammar must mark closure use capture invalid or unsupported'
    );
    $importPatterns = array_values(array_filter(
        $patterns,
        static fn (array $pattern): bool => ($pattern['name'] ?? null) === 'meta.import.doria'
    ));
    $traitPatterns = array_values(array_filter(
        $patterns,
        static fn (array $pattern): bool => ($pattern['name'] ?? null) === 'meta.trait-composition.doria'
    ));
    require_check($importPatterns !== [], 'VS Code grammar must define a distinct import-use scope');
    require_check($traitPatterns !== [], 'VS Code grammar must define a distinct trait-composition scope');

    $importBegin = (string) ($importPatterns[0]['begin'] ?? '');
    $traitBegin = (string) ($traitPatterns[0]['begin'] ?? '');
    require_check(regex_matches($importBegin, 'use App\\Models\\User;'), 'VS Code import pattern must match namespace imports');
    require_check(!regex_matches($importBegin, '    uses HasSlug;'), 'VS Code import pattern must not match class-body trait composition');
    require_check(regex_matches($traitBegin, '    uses HasSlug;'), 'VS Code trait-composition pattern must match class-body uses');
    require_check(!regex_matches($traitBegin, 'use App\\Models\\User;'), 'VS Code trait-composition pattern must not match namespace imports');
    require_check(!regex_matches($traitBegin, '    use HasSlug;'), 'VS Code trait-composition pattern must not accept legacy class-body use');

    foreach ([
        'keyword.control.import.doria',
        'keyword.operator.alias.doria',
        'keyword.other.trait-uses.doria',
        'invalid.illegal.keyword.trait-use-old-spelling.doria',
        'entity.name.type.trait.doria',
    ] as $scope) {
        require_check(str_contains($grammarText, $scope), "VS Code grammar is missing '{$scope}'");
    }

    $callPatterns = $grammar['repository']['calls']['patterns'] ?? [];
    $functionCallMatches = [];
    foreach ($callPatterns as $pattern) {
        if (($pattern['name'] ?? null) === 'entity.name.function.call.doria') {
            $functionCallMatches[] = (string) ($pattern['match'] ?? '');
        }
    }
    require_check($functionCallMatches !== [], 'VS Code grammar must define contextual function-call highlighting');
    foreach (['read_file', 'calculateReport', 'saveReport', 'formatReport'] as $call) {
        require_check(
            any_match($functionCallMatches, static fn (string $match): bool => regex_matches($match, $call . ' (')),
            "VS Code grammar must highlight arbitrary call name '{$call}' before an opening parenthesis"
        );
    }
    require_check(
        !any_match($functionCallMatches, static fn (string $match): bool => regex_matches($match, 'calculateReport;')),
        'VS Code grammar must not classify a bare identifier as a function call'
    );

    $attributePatterns = $grammar['repository']['attributes']['patterns'] ?? [];
    require_check($attributePatterns !== [], 'VS Code grammar must define attribute highlighting');
    $attributeBegin = (string) ($attributePatterns[0]['begin'] ?? '');
    require_check(regex_matches($attributeBegin, '#[Module]'), 'VS Code attribute pattern must match simple attributes');
    require_check(
        regex_matches($attributeBegin, '#[App\\Routing\\Route(path: "/parser")]'),
        'VS Code attribute pattern must match namespaced attributes'
    );
    foreach ([
        'punctuation.definition.attribute.begin.doria',
        'punctuation.definition.attribute.end.doria',
        'entity.name.type.attribute.doria',
        'variable.parameter.attribute.doria',
    ] as $scope) {
        require_check(str_contains($grammarText, $scope), "VS Code grammar is missing '{$scope}'");
    }
    $attributeExpressionPatterns = $grammar['repository']['attributeExpression']['patterns'] ?? [];
    $attributeIncludes = [];
    foreach ($attributeExpressionPatterns as $pattern) {
        if (is_array($pattern) && array_key_exists('include', $pattern)) {
            $attributeIncludes[] = $pattern['include'];
        }
    }
    require_check(
        any_match(
            $attributePatterns[0]['patterns'] ?? [],
            static fn (mixed $pattern): bool => is_array($pattern) &&
                ($pattern['include'] ?? null) === '#attributeExpression'
        ),
        'VS Code attributes must use the shared attribute-expression grammar'
    );
    require_check(
        in_array('#attributeBrackets', $attributeIncludes, true),
        'VS Code attribute expressions must recognize nested list and dictionary brackets'
    );
    $attributeBracketPatterns = $grammar['repository']['attributeBrackets'] ?? [];
    require_check(
        ($attributeBracketPatterns['begin'] ?? null) === '\\[' &&
            ($attributeBracketPatterns['end'] ?? null) === '\\]' &&
            any_match(
                $attributeBracketPatterns['patterns'] ?? [],
                static fn (mixed $pattern): bool => is_array($pattern) &&
                    ($pattern['include'] ?? null) === '#attributeExpression'
            ),
        'VS Code nested attribute brackets must recursively reuse the attribute-expression grammar'
    );
    $invalidIndex = array_search('#invalid', $attributeIncludes, true);
    $operatorsIndex = array_search('#operators', $attributeIncludes, true);
    require_check($invalidIndex !== false, 'VS Code attribute context must include invalid syntax patterns');
    require_check($operatorsIndex !== false, 'VS Code attribute context must include operator syntax patterns');
    require_check(
        $invalidIndex < $operatorsIndex,
        'VS Code attribute context must check invalid syntax before normal operators'
    );
}

function check_intellij_lexer(): void
{
    global $acceptedKeywords, $primitiveTypes, $reservedTypes, $plannedTypes, $wordOperators, $stage13SymbolOperators, $booleanSymbolOperators;
    global $notKeywords, $strictComparison, $rejectedPreprocessor, $rejectedKeywords, $rejectedTypes;
    global $buildScript, $intellijLexer, $intellijLanguage, $intellijBuildGradle, $intellijTokenTypes, $intellijSyntaxHighlighter, $intellijLexerTest, $intellijCodeStyleProvider, $intellijFormatter, $intellijParser, $intellijFormatterTest, $intellijPluginXml, $intellijPluginIcon, $doriaLogo;

    $intellijBuildText = read_text($intellijBuildGradle);
    require_check(
        str_contains($intellijBuildText, "version = '2026.03.1-canary'"),
        'IntelliJ package must carry the pre-1.0 Doria CalVer canary suffix'
    );
    require_check(
        str_contains($intellijBuildText, "tasks.register('cleanPluginDistributions', Delete)")
            && str_contains($intellijBuildText, 'dependsOn cleanPluginDistributions')
            && str_contains($intellijBuildText, 'doria-intellij-plugin-${project.version}.zip'),
        'IntelliJ buildPlugin must remove stale distributions and produce one clearly named plugin ZIP'
    );
    require_check(
        str_contains(read_text($buildScript), 'if (count($artifacts) !== 1)'),
        'repository build wrapper must reject ambiguous IntelliJ plugin ZIP output'
    );

    $lexerText = read_text($intellijLexer);
    $intellijHighlightingText = implode("\n", [
        $lexerText,
        read_text($intellijTokenTypes),
        read_text($intellijSyntaxHighlighter),
    ]);

    $tokens = array_unique([...$acceptedKeywords, ...$primitiveTypes, ...$reservedTypes, ...$plannedTypes, ...$wordOperators]);
    sort($tokens);
    foreach ($tokens as $token) {
        require_check(str_contains($lexerText, '"' . $token . '"'), "IntelliJ lexer is missing '{$token}'");
    }

    require_check(
        preg_match('/private val PRIMITIVE_TYPES = setOf\\((.*?)\\n\\s*\\)/s', $lexerText, $primitiveMatches) === 1,
        'IntelliJ primitive type classification could not be found'
    );
    $primitiveTypeList = $primitiveMatches[1];
    foreach ($primitiveTypes as $type) {
        require_check(
            str_contains($primitiveTypeList, '"' . $type . '"'),
            "IntelliJ lexer must classify '{$type}' as a primitive type"
        );
    }

    foreach ($rejectedTypes as $type) {
        require_check(!str_contains($lexerText, chr(34) . $type . chr(34)), "IntelliJ lexer must not highlight rejected type '{$type}'");
    }

    sort($notKeywords);
    foreach ($notKeywords as $token) {
        require_check(!str_contains($lexerText, '"' . $token . '"'), "IntelliJ lexer must not treat '{$token}' as a keyword");
    }

    foreach ($strictComparison as $operator) {
        require_check(str_contains($lexerText, '"' . $operator . '"'), "IntelliJ lexer must recognize '{$operator}'");
    }
    require_check(
        preg_match('/private val STRICT_COMPARISON_OPERATORS = setOf\\((.*?)\\)/s', $lexerText, $strictOperatorMatches) === 1,
        'IntelliJ invalid strict-comparison classification could not be found'
    );
    $strictOperatorList = $strictOperatorMatches[1];
    foreach (['&', '|'] as $operator) {
        require_check(
            !str_contains($strictOperatorList, '"' . $operator . '"'),
            "IntelliJ lexer must not mark single '{$operator}' invalid"
        );
    }
    require_check(
        str_contains($lexerText, 'STRICT_COMPARISON_OPERATORS') && str_contains($lexerText, 'DoriaTokenTypes.INVALID'),
        'IntelliJ lexer must route strict comparison operators to invalid highlighting'
    );

    require_check(
        preg_match('/private val THREE_CHAR_OPERATORS =\\s*(.*?)\\n\\s*private val TWO_CHAR_OPERATORS/s', $lexerText, $threeCharacterMatches) === 1,
        'IntelliJ three-character operator classification could not be found'
    );
    require_check(
        preg_match('/private val TWO_CHAR_OPERATORS = setOf\\((.*?)\\n\\s*\\)/s', $lexerText, $twoCharacterMatches) === 1,
        'IntelliJ two-character operator classification could not be found'
    );
    $threeCharacterOperatorList = $threeCharacterMatches[1];
    $twoCharacterOperatorList = $twoCharacterMatches[1];
    foreach (array_unique([...$stage13SymbolOperators, ...$booleanSymbolOperators]) as $operator) {
        $operatorLength = strlen($operator);
        if ($operatorLength === 1) {
            continue;
        }

        $operatorList = $operatorLength === 3 ? $threeCharacterOperatorList : $twoCharacterOperatorList;
        require_check(
            str_contains($operatorList, '"' . $operator . '"'),
            "IntelliJ lexer must tokenize '{$operator}' as one complete operator token"
        );
    }
    require_check(
        str_contains($lexerText, 'three in THREE_CHAR_OPERATORS -> three') &&
            str_contains($lexerText, 'two in TWO_CHAR_OPERATORS -> two'),
        'IntelliJ lexer must prefer three-character operators before two-character operators'
    );

    foreach ($rejectedKeywords as $keyword) {
        require_check(
            str_contains($lexerText, chr(34) . $keyword . chr(34)) && str_contains($lexerText, 'INVALID_KEYWORDS'),
            'IntelliJ lexer must mark ' . $keyword . ' invalid'
        );
    }
    sort($rejectedPreprocessor);
    foreach ($rejectedPreprocessor as $directive) {
        require_check(str_contains($lexerText, '"' . $directive . '"'), "IntelliJ lexer must recognize #{$directive} as unsupported");
    }
    require_check(
        str_contains($lexerText, 'firstNonWhitespace != tokenStart'),
        'IntelliJ preprocessor check must require # to be the first non-whitespace character on the line'
    );
    require_check(
        str_contains($lexerText, 'TRAIT_USES_LINE') && str_contains($lexerText, 'DoriaTokenTypes.TRAIT_USES_KEYWORD'),
        'IntelliJ lexer must recognize trait-composition uses'
    );
    require_check(
        str_contains($lexerText, 'LEGACY_TRAIT_USE_LINE') && str_contains($lexerText, 'isLegacyTraitUseLine() -> DoriaTokenTypes.INVALID'),
        'IntelliJ lexer must mark legacy class-body trait use invalid'
    );
    require_check(
        str_contains($lexerText, 'LEGACY_CLOSURE_USE_LINE') && str_contains($lexerText, 'isLegacyClosureUseLine() -> DoriaTokenTypes.INVALID'),
        'IntelliJ lexer must mark legacy closure use capture invalid'
    );
    require_check(
        str_contains($lexerText, 'MODE_ATTRIBUTE') && str_contains($lexerText, 'scanAttributeToken'),
        'IntelliJ lexer must define an attribute scanning mode'
    );
    require_check(
        str_contains($lexerText, 'MODE_INTERPOLATION_DOUBLE_STRING') &&
            str_contains($lexerText, 'MODE_INTERPOLATION_SINGLE_STRING') &&
            str_contains($lexerText, 'scanCodeToken(doubleQuoteStartsInterpolatedString = false)') &&
            str_contains($lexerText, 'interpolationBraceDepth++') &&
            str_contains($lexerText, 'interpolationBraceDepth--') &&
            str_contains($lexerText, 'decodeInterpolationBraceDepth'),
        'IntelliJ interpolation must reuse ordinary code scanning and preserve balanced braces and nested quoted strings'
    );
    require_check(
        str_contains($lexerText, 'DoriaTokenTypes.ATTRIBUTE_DELIMITER') &&
            str_contains($lexerText, 'DoriaTokenTypes.ATTRIBUTE_NAME') &&
            str_contains($lexerText, 'DoriaTokenTypes.ATTRIBUTE_ARGUMENT'),
        'IntelliJ lexer must emit dedicated attribute tokens'
    );
    require_check(
        str_contains($lexerText, 'isEnumDeclarationName() -> DoriaTokenTypes.ENUM_DECLARATION') &&
            str_contains($lexerText, 'isEnumCaseName(text) -> DoriaTokenTypes.ENUM_CASE') &&
            str_contains($intellijHighlightingText, 'DORIA_ENUM_DECLARATION') &&
            str_contains($intellijHighlightingText, 'DORIA_ENUM_CASE'),
        'IntelliJ must distinguish enum declarations and enum cases contextually'
    );
    $lexerTestText = read_text($intellijLexerTest);
    require_check(
        str_contains($lexerTestText, 'testNestedAttributeBracketsDoNotCloseTheAttributeEarly') &&
            str_contains($lexerTestText, 'tokens.count { it.type == DoriaTokenTypes.BRACKET }'),
        'IntelliJ lexer tests must preserve nested attribute brackets until the outer attribute delimiter'
    );
    require_check(
        str_contains($lexerText, 'isConstantName(text) -> DoriaTokenTypes.CLASS_CONSTANT') &&
            str_contains($lexerText, 'isConstantDeclaration()') &&
            str_contains($lexerText, 'CONSTANT_REFERENCE_NAME.matches(text)'),
        'IntelliJ lexer must distinguish class/top-level constants from static calls and type names'
    );
    require_check(
        str_contains($lexerText, 'isStaticPropertyName(text) -> DoriaTokenTypes.STATIC_PROPERTY') &&
            str_contains($lexerText, 'previousAccessor() == "::" && !isCallName()') &&
            str_contains($lexerText, 'isStaticPropertyDeclaration() -> DoriaTokenTypes.STATIC_PROPERTY') &&
            str_contains($lexerText, 'DoriaTokenTypes.STATIC_PROPERTY'),
        'IntelliJ lexer must distinguish static properties from ordinary variables'
    );
    require_check(
        str_contains($lexerText, 'previousAccessor() == "::" -> DoriaTokenTypes.INVALID'),
        'IntelliJ lexer must mark the PHP static-access sigil invalid'
    );
    require_check(
        str_contains($lexerText, "private fun isCallName(): Boolean = nextNonWhitespace(tokenEnd) == '('") &&
            str_contains($lexerText, 'isCallName() -> callableTokenType()') &&
            str_contains($lexerText, 'callableTokenType()') &&
            str_contains($lexerText, '"->" -> DoriaTokenTypes.METHOD_CALL') &&
            str_contains($lexerText, '"::" -> DoriaTokenTypes.STATIC_METHOD_CALL') &&
            str_contains($lexerText, 'else -> DoriaTokenTypes.FUNCTION_CALL'),
        'IntelliJ lexer must classify calls from parenthesis and accessor context rather than a name list'
    );
    require_check(
        str_contains($lexerText, 'isConstructorTypeName() -> DoriaTokenTypes.TYPE_NAME') &&
            str_contains($lexerText, "buffer[cursor] == '\\\\'") &&
            str_contains($lexerText, 'toString() == "new"') &&
            strpos($lexerText, 'isConstructorTypeName() -> DoriaTokenTypes.TYPE_NAME') <
                strpos($lexerText, 'isCallName() -> callableTokenType()') &&
            strpos($lexerText, 'isCallName() -> callableTokenType()') <
                strpos($lexerText, 'text.first().isUpperCase() -> DoriaTokenTypes.TYPE_NAME'),
        'IntelliJ lexer must preserve constructor type names while prioritizing other call syntax over capitalization'
    );
    foreach (['FUNCTION_CALL', 'METHOD_CALL', 'STATIC_METHOD_CALL'] as $callStyle) {
        require_check(
            preg_match(
                '/val ' . $callStyle . ': TextAttributesKey = TextAttributesKey\.createTextAttributesKey\(\s*"DORIA_' . $callStyle . '",\s*DefaultLanguageHighlighterColors\.FUNCTION_DECLARATION,/s',
                $intellijHighlightingText
            ) === 1,
            'IntelliJ ' . $callStyle . ' must inherit the visible function-declaration color instead of a theme-white call style'
        );
    }

    $pluginXml = read_text($intellijPluginXml);
    require_check(
        str_contains($pluginXml, 'language=' . chr(34) . 'doria' . chr(34)),
        'IntelliJ plugin must register the lowercase doria language id for Markdown fences'
    );
    require_check(
        !str_contains($pluginXml, '<annotator')
            && !str_contains($intellijHighlightingText, 'DORIA_UNUSED_VARIABLE'),
        'IntelliJ syntax presentation must not guess unused symbols without semantic resolution'
    );
    $languageText = read_text($intellijLanguage);
    require_check(
        str_contains($languageText, 'Language("doria")') &&
            str_contains($languageText, 'getDisplayName(): String = "Doria"'),
        'IntelliJ must keep the lowercase doria language id while presenting the Doria display name'
    );
    require_check(
        str_contains($pluginXml, '<langCodeStyleSettingsProvider') &&
            str_contains($pluginXml, 'dev.doria.intellij.codestyle.DoriaLanguageCodeStyleSettingsProvider'),
        'IntelliJ plugin must register the Doria code-style settings provider'
    );
    $codeStyleText = read_text($intellijCodeStyleProvider);
    foreach ([
        'getLanguageName(): String = "Doria"',
        'getIndentOptionsEditor(): IndentOptionsEditor = SmartIndentOptionsEditor()',
        'override fun createConfigurable(',
        'TabbedLanguageCodeStylePanel(',
        'INDENT_SIZE = 4',
        'CONTINUATION_INDENT_SIZE = 4',
        'TAB_SIZE = 4',
        'USE_TAB_CHARACTER = false',
        'RIGHT_MARGIN = 120',
        'CLASS_BRACE_STYLE = CommonCodeStyleSettings.NEXT_LINE',
        'METHOD_BRACE_STYLE = CommonCodeStyleSettings.NEXT_LINE',
    ] as $default) {
        require_check(
            str_contains($codeStyleText, $default),
            "IntelliJ Doria code-style defaults are missing '{$default}'"
        );
    }
    require_check(
        str_contains($pluginXml, '<lang.formatter') &&
            str_contains($pluginXml, 'dev.doria.intellij.codestyle.DoriaFormattingModelBuilder'),
        'IntelliJ plugin must register the Doria formatting model'
    );
    $formatterText = read_text($intellijFormatter);
    foreach ([
        'getCommonSettings(DoriaLanguage)',
        'getLanguageIndentOptions(DoriaLanguage)',
        'SPACE_WITHIN_METHOD_CALL_PARENTHESES',
        'SPACE_WITHIN_METHOD_PARENTHESES',
        'SPACE_BEFORE_COMMA',
        'SPACE_BEFORE_SEMICOLON',
        'DoriaElementTypes.BLOCK',
        'DoriaElementTypes.PARENTHESIZED',
        'DoriaElementTypes.BRACKETED',
    ] as $formatterBehavior) {
        require_check(
            str_contains($formatterText, $formatterBehavior),
            "IntelliJ formatter is missing '{$formatterBehavior}'"
        );
    }
    $parserText = read_text($intellijParser);
    require_check(
        str_contains($parserText, 'builder.tokenType == closingType') &&
            str_contains($parserText, 'builder.tokenText == closing'),
        'IntelliJ delimiter grouping must match token type and text so interpolation braces cannot close code blocks'
    );
    $formatterTestText = read_text($intellijFormatterTest);
    foreach ([
        'testIndentOptionsDriveReformatting',
        'testSpacingAndBraceSettingsDriveReformatting',
        'testTabsAndLanguageBoundariesRemainSemantic',
    ] as $formatterTest) {
        require_check(
            str_contains($formatterTestText, $formatterTest),
            "IntelliJ formatter regression coverage is missing '{$formatterTest}'"
        );
    }
    require_check(is_file($intellijPluginIcon), 'IntelliJ plugin must package META-INF/pluginIcon.svg');
    require_check(
        rtrim(read_text($intellijPluginIcon)) === rtrim(read_text($doriaLogo)),
        'IntelliJ plugin icon must use the canonical Doria README SVG'
    );

    foreach ([
        'DORIA_IMPORT_USE_KEYWORD',
        'DORIA_IMPORT_PATH',
        'DORIA_IMPORT_ALIAS_KEYWORD',
        'DORIA_IMPORT_ALIAS',
        'DORIA_TRAIT_USES_KEYWORD',
        'DORIA_TRAIT_NAME',
        'DORIA_ATTRIBUTE_DELIMITER',
        'DORIA_ATTRIBUTE_NAME',
        'DORIA_ATTRIBUTE_ARGUMENT',
        'DORIA_ENUM_DECLARATION',
        'DORIA_ENUM_CASE',
        'DORIA_TYPE_TEST_OPERATOR',
        'DORIA_LOGICAL_OPERATOR',
        'DORIA_PROPERTY',
        'DORIA_FUNCTION_CALL',
        'DORIA_METHOD_CALL',
        'DORIA_STATIC_METHOD_CALL',
        'DORIA_CLASS_CONSTANT',
        'DORIA_STATIC_PROPERTY',
    ] as $tokenType) {
        require_check(str_contains($intellijHighlightingText, $tokenType), "IntelliJ highlighting is missing {$tokenType}");
    }
}

function check_lsp_completion_vocabulary(): void
{
    global $acceptedKeywords, $plannedKeywords, $primitiveTypes, $reservedTypes, $plannedTypes, $wordOperators, $notKeywords, $lspServer, $lspSupportedTypes, $rejectedTypes;
    global $implementedIntegerTypes, $implementedStage14ScalarTypes;

    $lspText = read_text($lspServer);
    $tokens = array_unique([...$acceptedKeywords, ...$wordOperators]);
    sort($tokens);
    foreach ($tokens as $token) {
        require_check(str_contains($lspText, chr(34) . $token . chr(34)), 'LSP completion list is missing ' . $token);
    }

    foreach ($plannedKeywords as $keyword) {
        require_check(str_contains($lspText, chr(34) . $keyword . chr(34)), 'LSP completion list is missing planned keyword ' . $keyword);
    }
    require_check(str_contains($lspText, 'planned Doria keyword'), 'LSP planned keyword completions must be clearly marked as planned');
    require_check(
        str_contains($lspText, 'Accepted planned Doria syntax; compiler support lands in a later stage.'),
        'LSP planned keyword completions must explain that compiler support lands later'
    );

    require_check(
        preg_match('/let types = \[(.*?)\];/s', $lspText, $matches) === 1,
        'LSP completion type list could not be found'
    );
    $lspTypeList = $matches[1];

    require_check(
        preg_match('/let reserved_types = \[(.*?)\];/s', $lspText, $reservedMatches) === 1,
        'LSP completion reserved type list could not be found'
    );
    $lspReservedTypeList = $reservedMatches[1];
    require_check(
        preg_match('/let planned_keywords = \[(.*?)\];/s', $lspText, $plannedMatches) === 1,
        'LSP planned keyword list could not be found'
    );
    foreach (['try', 'catch', 'throw', 'throws'] as $keyword) {
        require_check(
            !str_contains($plannedMatches[1], chr(34) . $keyword . chr(34)),
            "LSP must classify {$keyword} as implemented checked-error syntax"
        );
    }
    require_check(
        str_contains($lspText, '"label": "Error"'),
        'LSP completion is missing the compiler-known Error interface'
    );

    sort($lspSupportedTypes);
    foreach ($lspSupportedTypes as $type) {
        require_check(str_contains($lspTypeList, chr(34) . $type . chr(34)), 'LSP completion type list is missing ' . $type);
    }

    foreach ($implementedIntegerTypes as $type) {
        require_check(
            str_contains($lspTypeList, chr(34) . $type . chr(34)),
            'LSP must classify Stage 13 integer type ' . $type . ' as implemented'
        );
    }
    require_check(
        str_contains($lspText, '`int` is an exact alias for `int64`'),
        'LSP completion or hover text must document int as the exact int64 alias'
    );

    foreach ($implementedStage14ScalarTypes as $type) {
        require_check(
            str_contains($lspTypeList, chr(34) . $type . chr(34)),
            'LSP must classify Stage 14 scalar type ' . $type . ' as implemented'
        );
    }
    require_check(
        str_contains($lspText, 'exact alias of `float64`'),
        'LSP completion or hover text must document float as the exact float64 alias'
    );

    sort($reservedTypes);
    foreach ($reservedTypes as $type) {
        require_check(str_contains($lspReservedTypeList, chr(34) . $type . chr(34)), 'LSP completion reserved type list is missing ' . $type);
        require_check(!str_contains($lspTypeList, chr(34) . $type . chr(34)), 'LSP completion type list must not advertise reserved type ' . $type);
    }

    foreach ($rejectedTypes as $type) {
        require_check(!str_contains($lspTypeList, chr(34) . $type . chr(34)), 'LSP completion type list must not advertise rejected type ' . $type);
        require_check(!str_contains($lspReservedTypeList, chr(34) . $type . chr(34)), 'LSP completion reserved type list must not advertise rejected type ' . $type);
    }

    $unsupportedTypes = array_diff(array_unique([...$primitiveTypes, ...$plannedTypes]), $lspSupportedTypes);
    sort($unsupportedTypes);
    foreach ($unsupportedTypes as $type) {
        require_check(!str_contains($lspTypeList, chr(34) . $type . chr(34)), 'LSP completion type list must not advertise unsupported type ' . $type);
    }

    foreach (['Int', 'Int8', 'Int16', 'Int32', 'Int64', 'UInt8', 'UInt16', 'UInt32', 'UInt64'] as $companion) {
        require_check(
            str_contains($lspText, chr(34) . $companion . '::from' . chr(34)),
            'LSP completion list is missing ' . $companion . '::from'
        );
    }
    require_check(
        str_contains($lspText, 'Doria integer conversion intrinsic') &&
            str_contains($lspText, 'Out-of-range conversion panics'),
        'LSP must provide completion and hover details for explicit integer conversions'
    );
    foreach (['Int::toFloat', 'Float::toInt'] as $intrinsic) {
        require_check(
            str_contains($lspText, chr(34) . $intrinsic . chr(34)),
            'LSP completion list is missing ' . $intrinsic
        );
    }
    require_check(
        str_contains($lspText, 'Doria scalar conversion intrinsic'),
        'LSP must provide completion and hover details for Stage 14 scalar conversions'
    );

    sort($notKeywords);
    foreach ($notKeywords as $token) {
        require_check(!str_contains($lspText, chr(34) . $token . chr(34)), 'LSP completion list must not advertise ' . $token);
    }
}

function check_intellij_class_name_vocabulary(): void
{
    global $acceptedKeywords, $plannedKeywords, $primitiveTypes, $reservedTypes, $wordOperators, $rejectedKeywords;
    global $intellijCreateClassAction;

    $actionText = read_text($intellijCreateClassAction);
    require_check(
        preg_match('/DORIA_RESERVED_NAME_SEGMENTS = setOf\((.*?)\n\s*\)/s', $actionText, $segmentMatches) === 1,
        'IntelliJ class workflow reserved-name vocabulary could not be found'
    );
    require_check(
        preg_match('/DORIA_RESERVED_CLASS_NAMES = setOf\((.*?)\n\s*\)/s', $actionText, $classMatches) === 1,
        'IntelliJ class workflow reserved class-name vocabulary could not be found'
    );

    $reservedSegments = array_unique([
        ...$acceptedKeywords,
        ...$plannedKeywords,
        ...$primitiveTypes,
        ...$reservedTypes,
        ...$wordOperators,
        ...$rejectedKeywords,
        'static',
        'spawn',
        'scope',
        'is',
        'array',
        'object',
    ]);
    foreach ($reservedSegments as $name) {
        require_check(
            str_contains($segmentMatches[1], '"' . $name . '"'),
            'IntelliJ class workflow must reject reserved Doria name ' . $name
        );
    }

    foreach ([
        'Int', 'Int8', 'Int16', 'Int32', 'Int64',
        'UInt8', 'UInt16', 'UInt32', 'UInt64',
        'Float', 'Float32', 'Float64', 'Bool', 'Displayable',
    ] as $name) {
        require_check(
            str_contains($classMatches[1], '"' . $name . '"'),
            'IntelliJ class workflow must reject compiler-reserved class name ' . $name
        );
    }
    require_check(
        str_contains($actionText, "fun isDoriaQualifiedClassName")
            && str_contains($actionText, "isDoriaClassName(segments.last())"),
        'IntelliJ class workflow must validate the terminal qualified type-name segment as a class name'
    );
    require_check(
        str_contains($actionText, "if (!isDoriaQualifiedInterfaceName(interfaceName))")
            && str_contains($actionText, 'value == "Displayable" || isDoriaQualifiedClassName(value)'),
        'IntelliJ class workflow must accept Displayable in the interface picker without allowing it as a class name'
    );
}

function check_editor_fixture_diagnostics_are_skipped(): void
{
    global $vscodeExtension, $intellijLspFiles;

    $vscodeText = read_text($vscodeExtension);
    $intellijText = read_text($intellijLspFiles);

    require_check(
        str_contains($vscodeText, '/editors/fixtures/') && str_contains($vscodeText, 'isDoriaSource'),
        'VS Code client must keep editor fixtures out of doria-lsp diagnostics'
    );
    require_check(
        str_contains($intellijText, '/editors/fixtures/') && str_contains($intellijText, 'isDoriaSourceFile'),
        'IntelliJ LSP adapter must keep editor fixtures out of doria-lsp diagnostics'
    );
}

function check_fixture(): void
{
    global $fixture, $rejectedFixture, $strictComparison, $rejectedKeywords, $groupedLocalForms, $stage28GuardForms, $stage28aControlFlowForms;

    $fixtureText = read_text($fixture);
    $requiredSnippets = [
        'internal',
        'const DEFAULT_LIMIT = 25;',
        'const int HARD_LIMIT = 100;',
        'internal const MAX_DEPTH = DEFAULT_LIMIT * 4;',
        'static int $initial = 0;',
        'static writable int $next = 1;',
        'internal static string $label = "parser";',
        'self::next += self::MAX_DEPTH;',
        'ParserLimits::nextId()',
        'ParserLimits::next;',
        'ParserLimits::MAX_DEPTH',
        'uses HasSlug, TracksChanges;',
        'with ($minimum)',
        'with (take $message)',
        'open class Model',
        'override function save',
        'throws StorageError',
        'enum Option',
        'enum Status',
        'enum Priority: int',
        'case Some',
        'case Draft',
        'case Low = 1;',
        'match ($option)',
        'Option::Some($value)',
        'default => -1',
        'unsafe',
        'extern',
        '#[PhpExport]',
        '#[App\Routing\Route(path: "/parser", name: "parser.show")]',
        'imports: [',
        'ORMModule::forRoot(',
        'entities: [],',
        '0..<10',
        '0..10',
        'float32 $delta = 0.016;',
        'float $ratio = 1.0 / 3.0;',
        'Int::toFloat($whole)',
        'Float::toInt($wideAlias)',
        'let $both = $ready and not false;',
        'bool $symbolNegated = !$input;',
        'bool $symbolBoth = $ready && $symbolNegated;',
        'bool $symbolEither = $symbolBoth || $different;',
        'echo "Profile: {$this->profile->displayName}";',
        'echo "Count: {$count}";',
        'echo "sum: {left() + right()}";',
        'echo "nested string: {sprintf("value={left() + right()}")}";',
        'echo "commented: {left() /* } { */ + right()}";',
        'echo "literal brace: \{ and bare }";',
        'implements Displayable',
        'function toString(): string',
        "echo 'Literal {\$name}';",
        'read_file("input.txt")',
        'calculateReport($text)',
        '$repository->saveReport($customResult)',
        'ReportFormatter::formatReport($customResult)',
        'new App\Report()',
        '?User',
        'if ($metadata is string)',
        '$maybeUser?->name ?? "anonymous"',
        '\n\t\r\s',
        'use App\Repositories\UserRepository;',
        'get_time()',
        'String::startsWith($name, "Dor")',
        'Int::wrappingAdd(1, 2)',
        '$name->isEmpty()',
        '$message->retryAfter(seconds: 30)',
        // Stage 23a named arguments across all four callable forms.
        'scheduleDelivery(recipient: $name, attempts: 3)',
        '$message->deliver(attempts: 2, recipient: $name)',
        'MessageFactory::create(recipient: $name, attempts: 1)',
        'new Message(attempts: 5, recipient: $name)',
        // Stage 23b program entry arguments.
        'function main(List<string> $args): void',
        // Stage 23c sequence fill literal.
        'let $runtimeSizedFlags = [true; $args->count];',
        '$message->tenantId',
        '$repository->findById($id)',
        'let $reference = shared new SharedEditorPayload();',
        '$reference->referencedValue->name',
        'let $score = match ($option)',
        'Option::Some($value) => 0',
        'Option::None => 0',
    ];
    foreach ($requiredSnippets as $snippet) {
        require_check(str_contains($fixtureText, $snippet), 'shared editor fixture is missing ' . $snippet);
    }
    foreach ($groupedLocalForms as $snippet) {
        require_check(
            str_contains($fixtureText, $snippet),
            'shared editor fixture is missing grouped local form ' . $snippet,
        );
    }
    foreach ($stage28GuardForms as $snippet) {
        require_check(
            str_contains($fixtureText, $snippet),
            'shared editor fixture is missing Stage 28 form ' . $snippet,
        );
    }
    foreach ($stage28aControlFlowForms as $snippet) {
        require_check(
            str_contains($fixtureText, $snippet),
            'shared editor fixture is missing Stage 28a control-flow form ' . $snippet,
        );
    }
    require_check(
        !str_contains($fixtureText, ' where ') && !str_contains($fixtureText, ' when true =>'),
        'shared editor fixture must not publish where/when aliases for match guards',
    );
    require_check(
        !str_contains($fixtureText, 'str_starts_with('),
        'shared editor fixture must not publish the removed Doria str_* String surface',
    );

    $stage13TypeSnippets = [
        'int8 $minimumInt8 = -128;',
        'int16 $maximumInt16 = 32767;',
        'int32 $maximumInt32 = 2147483647;',
        'int64 $maximumInt64 = 9223372036854775807;',
        'uint8 $maximumUInt8 = 255;',
        'uint16 $maximumUInt16 = 65535;',
        'uint32 $maximumUInt32 = 4294967295;',
        'uint64 $maximumUInt64 = 18446744073709551615;',
    ];
    foreach ($stage13TypeSnippets as $snippet) {
        require_check(str_contains($fixtureText, $snippet), 'shared editor fixture is missing Stage 13 type snippet ' . $snippet);
    }

    foreach (['Int', 'Int8', 'Int16', 'Int32', 'Int64', 'UInt8', 'UInt16', 'UInt32', 'UInt64'] as $companion) {
        require_check(
            str_contains($fixtureText, $companion . '::from('),
            'shared editor fixture is missing Stage 13 conversion ' . $companion . '::from'
        );
    }

    $stage13OperatorSnippets = [
        'let $negated = -$defaultInt;',
        'let $complemented = ~$defaultInt;',
        'let $sum = $defaultInt + 2;',
        'let $difference = $defaultInt - 2;',
        'let $product = $defaultInt * 2;',
        'let $quotient = $defaultInt / 2;',
        'let $remainder = $defaultInt % 2;',
        'let $shiftedLeft = $defaultInt << 1;',
        'let $shiftedRight = $defaultInt >> 1;',
        'let $masked = $defaultInt & 255;',
        'let $combined = $defaultInt | 1;',
        'let $toggled = $defaultInt ^ 1;',
        'let $equal = $defaultInt == 42;',
        'let $notEqual = $defaultInt != 0;',
        'let $less = $defaultInt < 100;',
        'let $lessOrEqual = $defaultInt <= 42;',
        'let $greater = $defaultInt > 0;',
        'let $greaterOrEqual = $defaultInt >= 42;',
        'let $booleanXor = true xor false;',
        '$accumulator += 3;',
        '$accumulator -= 2;',
        '$accumulator *= 2;',
        '$accumulator /= 2;',
        '$accumulator %= 5;',
        '$accumulator <<= 1;',
        '$accumulator >>= 1;',
        '$accumulator &= 7;',
        '$accumulator |= 8;',
        '$accumulator ^= 3;',
        '$accumulator++;',
        '$accumulator--;',
    ];
    foreach ($stage13OperatorSnippets as $snippet) {
        require_check(str_contains($fixtureText, $snippet), 'shared editor fixture is missing Stage 13 operator snippet ' . $snippet);
    }

    require_check(is_file($rejectedFixture), 'negative editor fixture must exist');
    $rejectedText = read_text($rejectedFixture);
    foreach (['public', 'private', 'protected', 'use HasSlug;', 'use ($x)', '#define', '#include'] as $snippet) {
        require_check(str_contains($rejectedText, $snippet), 'negative editor fixture is missing ' . $snippet);
    }
    foreach ($strictComparison as $operator) {
        require_check(str_contains($rejectedText, $operator), 'negative editor fixture is missing ' . $operator);
    }
    foreach ($rejectedKeywords as $keyword) {
        require_check(str_contains($rejectedText, $keyword), 'negative editor fixture is missing ' . $keyword);
    }
}

function check_stage30f_callable_alignment(): void
{
    global $cargoManifest, $readme, $vscodeGrammar, $intellijLexer, $intellijLexerTest;
    global $lspAnalysis, $lspServer, $lspTests, $fixture, $rejectedFixture;

    $compilerCommit = 'd395cd135399ce1803d1be7c5a2a9ded2053c738';
    require_check(
        preg_match(
            '/doriac\s*=\s*\{[^}]*\brev\s*=\s*"' . $compilerCommit . '"/',
            read_text($cargoManifest),
        ) === 1,
        "root doriac dependency must pin compiler commit {$compilerCommit}",
    );

    $fixtureText = read_text($fixture);
    $acceptedFacts = [
        'arrow closure' => '/\bfn\s*\([^)]*\$[A-Za-z_][A-Za-z0-9_]*\)\s*=>/',
        'anonymous block closure' => '/\bfunction\s*\([^)]*\)\s*:\s*[^\{]+\{/',
        'readonly capture' => '/\bwith\s*\(\s*\$[A-Za-z_][A-Za-z0-9_]*\s*\)/',
        'writable capture' => '/\bwith\s*\([^)]*\bwritable\s+\$[A-Za-z_][A-Za-z0-9_]*/',
        'taking capture' => '/\bwith\s*\([^)]*\btake\s+\$[A-Za-z_][A-Za-z0-9_]*/',
        'function type' => '/\bfunction\s*\([^$)]*\)\s*:\s*\??[A-Za-z_][A-Za-z0-9_]*/',
        'writable invocation' => '/\bfunction\s+writable\s*\(/',
        'once invocation' => '/\bfunction\s+once\s*\(/',
        'function-type writable parameter' => '/\bfunction\s*\(\s*writable\s+[A-Za-z_]/',
        'function-type take parameter' => '/\bfunction\s*\(\s*take\s+[A-Za-z_]/',
        'function-type checked effects' => '/\bfunction\s*\([^)]*\)\s*:\s*[^;=\n]+\bthrows\s+[A-Za-z_]/',
        'grouped nested function type' => '/\(\s*function\s*\(\s*\)\s*:\s*int\s+throws\s+ParseError\s*\)/',
        'callable-value invocation' => '/\$identity\s*\(\s*1\s*\)/',
    ];
    foreach ($acceptedFacts as $fact => $pattern) {
        require_check(
            preg_match($pattern, $fixtureText) === 1,
            "shared accepted fixture must cover {$fact}",
        );
    }
    require_check(
        str_contains($fixtureText, 'Stage 30f PHP Compatibility') &&
            str_contains($fixtureText, 'executable in debug, native, and PHP compatibility targets') &&
            !str_contains($fixtureText, 'Planned/future: closure'),
        'shared fixture must describe the implemented Stage 30f target surface',
    );

    $rejectedText = read_text($rejectedFixture);
    foreach ([
        'PHP closure use' => '/\buse\s*\(\s*\$[A-Za-z_]/',
        'empty capture list' => '/\bwith\s*\(\s*\)/',
        'reference capture' => '/\bwith\s*\([^)]*&\s*\$[A-Za-z_]/',
        'readonly capture modifier' => '/\bwith\s*\([^)]*\breadonly\s+\$[A-Za-z_]/',
        'take invocation spelling' => '/\bfunction\s+take\s*\(\s*\)/',
        'explicit readonly invocation' => '/\bfunction\s+readonly\s*\(/',
        'conflicting parameter ownership' => '/\bfunction\s*\(\s*take\s+writable\s+/',
        'missing effect type' => '/\bfunction\s*\(\s*\)\s*:\s*void\s+throws\s+\$/',
        'tuple-like grouped type' => '/\(\s*int\s*,\s*string\s*\)/',
        'named callable argument' => '/\$callback\s*\(\s*value\s*:/',
    ] as $fact => $pattern) {
        require_check(
            preg_match($pattern, $rejectedText) === 1,
            "shared rejected fixture must cover {$fact}",
        );
    }
    $acceptedCaptureList = '/\bwith\s*\(\s*(?:(?:writable|take)\s+)?\$[A-Za-z_][A-Za-z0-9_]*(?:\s*,\s*(?:(?:writable|take)\s+)?\$[A-Za-z_][A-Za-z0-9_]*)*\s*\)/';
    require_check(
        preg_match($acceptedCaptureList, $rejectedText) !== 1,
        'shared rejected fixture must not contain an accepted capture list',
    );

    $grammar = load_json($vscodeGrammar);
    $grammarText = json_encode($grammar, JSON_THROW_ON_ERROR);
    foreach ([
        'keyword.declaration.function.arrow.doria',
        'keyword.declaration.function.anonymous.doria',
        'storage.type.function.doria',
        'keyword.control.closure.capture.doria',
        'storage.modifier.ownership.capture.doria',
        'variable.other.capture.doria',
        'keyword.operator.closure.arrow.doria',
        'storage.modifier.invocation.function.doria',
        'storage.modifier.ownership.parameter.function-type.doria',
        'meta.group.type.doria',
        'punctuation.definition.arguments.begin.callable.doria',
    ] as $scope) {
        require_check(str_contains($grammarText, $scope), "VS Code closure grammar is missing {$scope}");
    }
    require_check(
        !str_contains($grammarText, 'invalid.illegal.closure.capture.doria'),
        'VS Code must leave malformed closure-capture diagnostics to the compiler',
    );
    $closurePatterns = $grammar['repository']['closures']['patterns'] ?? [];
    $arrowPattern = null;
    $blockPattern = null;
    foreach ($closurePatterns as $pattern) {
        if (($pattern['beginCaptures']['1']['name'] ?? null) === 'keyword.declaration.function.arrow.doria') {
            $arrowPattern = $pattern;
        }
        if (($pattern['beginCaptures']['1']['name'] ?? null) === 'keyword.declaration.function.anonymous.doria') {
            $blockPattern = $pattern;
        }
    }
    require_check(
        is_array($arrowPattern) &&
            ($arrowPattern['end'] ?? null) === '(=>)' &&
            ($arrowPattern['endCaptures']['1']['name'] ?? null) === 'keyword.operator.closure.arrow.doria' &&
            any_match(
                $arrowPattern['patterns'] ?? [],
                static fn (array $pattern): bool => ($pattern['include'] ?? null) === '#closureCaptureList',
            ),
        'VS Code closure arrows must be scoped only by the bounded fn construct',
    );
    require_check(
        any_match(
            $grammar['repository']['closureCaptureList']['patterns'] ?? [],
            static fn (array $pattern): bool => ($pattern['include'] ?? null) === '#comments',
        ),
        'VS Code capture lists must tokenize comments before capture syntax',
    );
    require_check(
        !any_match(
            $closurePatterns,
            static fn (array $pattern): bool =>
                ($pattern['name'] ?? null) === 'keyword.operator.closure.arrow.doria',
        ),
        'VS Code must not expose an unconditional root closure-arrow matcher',
    );
    require_check(
        is_array($blockPattern) && regex_matches(
            (string) ($blockPattern['begin'] ?? ''),
            'function (int $value): Doria\\Std\\Io\\IoError {',
        ),
        'VS Code anonymous closures must recognize namespace-qualified return types',
    );
    require_check(
        any_match(
            $grammar['repository']['keywords']['patterns'] ?? [],
            static fn (array $pattern): bool =>
                ($pattern['name'] ?? null) === 'storage.modifier.invocation.function.doria' &&
                regex_matches((string) ($pattern['match'] ?? ''), 'once'),
        ),
        'VS Code must recognize once as an accepted invocation modifier',
    );
    require_check(
        isset($grammar['repository']['typeGrouping'], $grammar['repository']['callableCalls']),
        'VS Code must present grouped function types and explicit callable-value calls',
    );
    $reservedPattern = any_match(
        $grammar['repository']['keywords']['patterns'] ?? [],
        static fn (array $pattern): bool =>
            ($pattern['name'] ?? null) === 'keyword.other.reserved.doria' &&
            !regex_matches((string) ($pattern['match'] ?? ''), 'fn') &&
            !regex_matches((string) ($pattern['match'] ?? ''), 'with'),
    );
    require_check($reservedPattern, 'VS Code must not classify fn or with as reserved vocabulary');
    foreach (['fnord', 'withdraw', 'functionality', 'onceOnly', 'once_more', 'once1'] as $identifier) {
        require_check(
            !regex_matches('\\b(?:fn|with|function|once)\\b', $identifier),
            "closure keyword boundaries must preserve identifier {$identifier}",
        );
    }

    $lexerText = read_text($intellijLexer);
    $lexerTestText = read_text($intellijLexerTest);
    require_check(
        str_contains($lexerText, '"fn"') &&
            str_contains($lexerText, '"with"') &&
            str_contains($lexerText, '"once"') &&
            str_contains($lexerText, 'isRejectedFunctionInvocationModifier()') &&
            !str_contains($lexerText, 'withTokenType()') &&
            !str_contains($lexerText, 'hasInvalidClosureCaptureClause()'),
        'IntelliJ must recognize fn/with while leaving capture diagnostics to the compiler',
    );
    foreach ([
        'testAcceptedClosureGrammarUsesAlignedKeywordModifierAndVariableTokens',
        'testClosureKeywordsRespectIdentifierStringAndCommentBoundaries',
        'testCompilerOwnsMalformedCaptureDiagnosticsWithoutCommentFalsePositives',
        'testStage30aFunctionTypesAndCallableCallsUseAlignedPresentationTokens',
        'testOnceAndRejectedTakeInvocationRespectLexicalContext',
    ] as $test) {
        require_check(str_contains($lexerTestText, $test), "IntelliJ closure coverage is missing {$test}");
    }

    $lspText = read_text($lspAnalysis) . read_text($lspServer);
    $lspTestText = read_text($lspTests);
    require_check(
        str_contains($lspText, '"developmentOnly": diagnostic.development_only') &&
            str_contains($lspText, 'Doria arrow-closure keyword') &&
            str_contains($lspText, 'Doria closure-capture keyword') &&
            str_contains($lspText, 'Doria function-type invocation modifier') &&
            str_contains($lspText, 'collect_semantic_hovers') &&
            str_contains($lspText, 'Semantically checked callable-value invocation') &&
            str_contains($lspText, 'ClosureValueProvenance') &&
            str_contains($lspText, 'ClosureEscapeClassification') &&
            str_contains($lspText, 'Executable In Debug, Native, And PHP Compatibility Targets') &&
            !str_contains($lspText, 'PHP Compatibility Lands In Stage 30f') &&
            !str_contains($lspText, 'Stage 30b checks') &&
            !str_contains($lspText, 'Stage 30b infers'),
        'LSP must preserve compiler metadata and expose current closure target capabilities without stale stage claims',
    );
    foreach ([
        'valid_stage_30f_closure_documents_are_target_neutral',
        'missing_capture_diagnostic_stays_off_following_source',
        'malformed_capture_forms_remain_parser_diagnostics_not_stage_30_boundaries',
        'type_only_function_syntax_has_no_execution_boundary',
        'publishes_compiler_owned_stage_30b_diagnostics_without_redundant_boundaries',
        'distinguishes_function_shape_and_checked_effect_mismatches',
        'exposes_only_safe_compiler_owned_capture_fixes_as_code_actions',
        'publishes_the_complete_stage_30c_ownership_diagnostic_surface',
        'ownership_edits_remain_compiler_owned_and_review_only',
        'closure_hovers_present_compiler_owned_ownership_without_internal_identity',
        'invalid_closure_hover_does_not_claim_target_execution',
        'closure_hovers_distinguish_callback_and_return_escape_contracts',
        'malformed_stage_30a_forms_remain_parser_diagnostics',
        'stage_30a_does_not_change_named_function_method_or_static_calls',
        'stage_30_diagnostics_preserve_compiler_source_order_without_redundant_e0641',
        'developmentOnly',
    ] as $coverage) {
        require_check(
            str_contains($lspTestText . $lspText, $coverage),
            "LSP closure coverage is missing {$coverage}",
        );
    }

    $readmeText = read_text($readme);
    require_check(
        str_contains($readmeText, 'Stage 30f PHP compatibility closure execution is implemented') &&
            str_contains($readmeText, 'E0641') &&
            str_contains($readmeText, 'supported compatibility surface') &&
            str_contains($readmeText, 'Stage 30g') &&
            str_contains($readmeText, 'Stage 30 remains incomplete') &&
            !str_contains($readmeText, 'lowering remains the Stage 30f boundary'),
        'README must state Stage 30f compatibility execution and Stage 30g next',
    );
}

function check_inferred_main_effect_alignment(): void
{
    global $readme, $lspAnalysis, $lspServer, $lspTests;

    $serverText = read_text($lspAnalysis) . read_text($lspServer);
    $testText = read_text($lspAnalysis) . read_text($lspTests);
    $readmeText = read_text($readme);

    require_check(
        !str_contains($serverText, 'E0631'),
        'LSP implementation must not infer or suppress compiler checked-effect diagnostics',
    );
    foreach ([
        'accepts_compiler_inferred_checked_effects_for_selected_main',
        'preserves_compiler_owned_checked_effect_diagnostics_for_ordinary_callables',
        'preserves_compiler_owned_checked_effect_diagnostics_for_incomplete_main_clauses',
        'assert_lsp_diagnostic_matches_compiler',
        'inferred_main_effects_do_not_rewrite_source_signature_hover',
    ] as $coverage) {
        require_check(
            str_contains($testText, $coverage),
            "LSP inferred-main checked-effect coverage is missing {$coverage}",
        );
    }
    require_check(
        str_contains($readmeText, 'entrypoint may omit `throws`') &&
            str_contains($readmeText, 'server neither infers effects nor suppresses E0631'),
        'README must preserve compiler ownership of selected-main effect inference',
    );
}

function main(): int
{
    check_vscode_package();
    check_vscode_language_server_packaging();
    check_intellij_language_server_packaging();
    check_vscode_language_configuration();
    check_vscode_grammar();
    check_intellij_lexer();
    check_intellij_class_name_vocabulary();
    check_lsp_completion_vocabulary();
    check_editor_fixture_diagnostics_are_skipped();
    check_fixture();
    check_stage30f_callable_alignment();
    check_inferred_main_effect_alignment();
    echo "Doria editor highlighting checks passed.\n";
    return 0;
}

exit(main());
