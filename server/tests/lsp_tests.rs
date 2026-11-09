use serde_json::Value;

use doria_language_server::{
    byte_offset_to_position, code_actions_for_document, diagnostics_for_document,
    position_to_byte_offset,
};
use doriac::diagnostics::{
    prepare_diagnostics, Diagnostic, DiagnosticSeverity, DiagnosticSource, LabelRole,
};

const CONSUMING_RETAINED_RECEIVER: &str =
    include_str!("../../editors/fixtures/stage36a-consuming-retained-receiver.doria");
const READINESS_SOURCES: &str =
    include_str!("../../editors/fixtures/stage36a-readiness-sources.doria");
const PENDING_ADAPTERS: &str =
    include_str!("../../editors/fixtures/stage36a-pending-adapters.doria");
const SIGNED_DURATION: &str = include_str!("../../editors/fixtures/stage36a-signed-duration.doria");
const WAIT_VALUES: &str = include_str!("../../editors/fixtures/stage36a-wait-values.doria");
const REGISTRATION_KEYS: &str =
    include_str!("../../editors/fixtures/stage36a-registration-keys.doria");
const READINESS_SET: &str = include_str!("../../editors/fixtures/stage36a-readiness-set.doria");
const WRITABLE_BORROWED_LOCALS: &str =
    include_str!("../../editors/fixtures/stage36a-writable-borrowed-locals.doria");
const BORROWING_READINESS_ADAPTER: &str =
    include_str!("../../editors/fixtures/stage36a-borrowing-readiness-adapter.doria");
const READINESS_FORWARDING_CALLBACKS: [&str; 3] = [
    "fn(take ReadinessSet $input) => $input",
    "function (take ReadinessSet $input): ReadinessSet { return $input; }",
    "when ($choose) { return fn(take ReadinessSet $input) => $input; } else { return function (take ReadinessSet $input): ReadinessSet { return $input; }; }",
];

fn compiler_findings_for_publication(name: &str, source: &str) -> Vec<Diagnostic> {
    let diagnostics =
        doriac::check_source(name, source).expect_err("fixture must produce a diagnostic");
    prepare_diagnostics(&diagnostics)
}

fn assert_lsp_diagnostic_matches_compiler(name: &str, source: &str, code: &str) {
    let compiler_diagnostics = compiler_findings_for_publication(name, source);
    let compiler_diagnostic = compiler_diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("compiler did not report {code}: {compiler_diagnostics:#?}"));
    assert_lsp_finding_matches_compiler(name, source, compiler_diagnostic);
}

fn assert_lsp_finding_matches_compiler(name: &str, source: &str, compiler_diagnostic: &Diagnostic) {
    let compiler_span = compiler_diagnostic
        .labels
        .iter()
        .find(|label| label.role == LabelRole::Primary)
        .or_else(|| compiler_diagnostic.labels.first())
        .map_or(compiler_diagnostic.span, |label| label.span);

    let uri = format!("file:///{name}");
    let lsp_diagnostics = diagnostics_for_document(&uri, source);
    let expected_start = byte_offset_to_position(source, compiler_span.start);
    let expected_end = byte_offset_to_position(source, compiler_span.end);
    let lsp_diagnostic = lsp_diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic["code"] == compiler_diagnostic.code
                && diagnostic["range"]["start"]["line"] == expected_start.line
                && diagnostic["range"]["start"]["character"] == expected_start.character
                && diagnostic["range"]["end"]["line"] == expected_end.line
                && diagnostic["range"]["end"]["character"] == expected_end.character
        })
        .unwrap_or_else(|| {
            panic!("language server did not publish {compiler_diagnostic:#?}: {lsp_diagnostics:#?}")
        });
    let expected_cause_id = compiler_diagnostic
        .cause_id
        .as_ref()
        .map_or(Value::Null, |cause_id| Value::String(cause_id.clone()));

    assert_eq!(lsp_diagnostic["source"], "doriac");
    assert_eq!(
        lsp_diagnostic["severity"],
        match compiler_diagnostic.severity {
            DiagnosticSeverity::Error => 1,
            DiagnosticSeverity::Warning => 2,
            DiagnosticSeverity::Note => 3,
        }
    );
    let primary_message = compiler_diagnostic
        .labels
        .iter()
        .find(|label| label.role == LabelRole::Primary)
        .or_else(|| compiler_diagnostic.labels.first())
        .map(|label| label.message.as_str())
        .unwrap_or_default();
    let normalize = |text: &str| {
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .trim_end_matches(['.', ':', ';'])
            .to_lowercase()
    };
    let expected_prefix = if !primary_message.is_empty()
        && normalize(&compiler_diagnostic.title) == normalize(primary_message)
    {
        primary_message
    } else {
        &compiler_diagnostic.title
    };
    assert!(lsp_diagnostic["message"]
        .as_str()
        .is_some_and(|message| message.starts_with(expected_prefix)));
    assert_eq!(
        lsp_diagnostic["data"]["kind"],
        compiler_diagnostic.kind.as_str()
    );
    assert_eq!(
        lsp_diagnostic["data"]["developmentOnly"],
        compiler_diagnostic.development_only
    );
    assert_eq!(lsp_diagnostic["data"]["causeId"], expected_cause_id);
    assert_eq!(
        lsp_diagnostic["range"]["start"]["line"],
        expected_start.line
    );
    assert_eq!(
        lsp_diagnostic["range"]["start"]["character"],
        expected_start.character
    );
    assert_eq!(lsp_diagnostic["range"]["end"]["line"], expected_end.line);
    assert_eq!(
        lsp_diagnostic["range"]["end"]["character"],
        expected_end.character
    );
    let compiler_related = compiler_diagnostic
        .labels
        .iter()
        .filter(|label| label.role == LabelRole::Secondary)
        .collect::<Vec<_>>();
    let lsp_related = lsp_diagnostic["relatedInformation"]
        .as_array()
        .map_or(0, Vec::len);
    assert_eq!(lsp_related, compiler_related.len());
    for (index, label) in compiler_related.iter().enumerate() {
        let related = &lsp_diagnostic["relatedInformation"][index];
        assert_eq!(related["message"], label.message);
        if matches!(label.source, DiagnosticSource::Current) {
            let start = byte_offset_to_position(source, label.span.start);
            let end = byte_offset_to_position(source, label.span.end);
            assert_eq!(related["location"]["uri"], uri);
            assert_eq!(related["location"]["range"]["start"]["line"], start.line);
            assert_eq!(
                related["location"]["range"]["start"]["character"],
                start.character
            );
            assert_eq!(related["location"]["range"]["end"]["line"], end.line);
            assert_eq!(
                related["location"]["range"]["end"]["character"],
                end.character
            );
        }
    }
}

fn assert_stage_30_closure_is_valid(name: &str, source: &str) {
    doriac::parse_source(name, source).expect("accepted closure grammar must parse");
    doriac::check_source(name, source)
        .unwrap_or_else(|diagnostics| panic!("valid Stage 30 source: {diagnostics:#?}"));
    let uri = format!("file:///{name}");
    let diagnostics = diagnostics_for_document(&uri, source);
    assert!(diagnostics.is_empty(), "{name}: {diagnostics:#?}");
}

#[test]
fn maps_byte_offsets_to_utf16_lsp_positions() {
    let text = "let $name = \"Zoë\";\nlet $emoji = \"😀\";\n";

    let first_newline = text.find('\n').expect("fixture should contain newline");
    let emoji = text.find('😀').expect("fixture should contain emoji");

    assert_eq!(byte_offset_to_position(text, 0).line, 0);
    assert_eq!(byte_offset_to_position(text, first_newline + 1).line, 1);
    assert_eq!(byte_offset_to_position(text, emoji).character, 14);
    assert_eq!(
        byte_offset_to_position(text, emoji + "😀".len()).character,
        16
    );
}

#[test]
fn maps_utf16_lsp_positions_to_byte_offsets() {
    let text = "let $emoji = \"😀\";\n";
    let emoji = text.find('😀').expect("fixture should contain emoji");

    assert_eq!(position_to_byte_offset(text, 0, 14), emoji);
    assert_eq!(position_to_byte_offset(text, 0, 15), emoji);
    assert_eq!(position_to_byte_offset(text, 0, 16), emoji + "😀".len());
}

#[test]
fn exposes_compiler_diagnostics_as_lsp_diagnostics() {
    let diagnostics = diagnostics_for_document(
        "file:///test.doria",
        r#"let $count = 0;
$count = 1;
"#,
    );

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0]["source"],
        Value::String("doriac".to_string())
    );
    assert_eq!(diagnostics[0]["code"], Value::String("E0201".to_string()));
    assert_eq!(diagnostics[0]["range"]["start"]["line"], Value::from(1));
    assert_eq!(
        diagnostics[0]["range"]["start"]["character"],
        Value::from(0)
    );
    assert!(diagnostics[0]["message"]
        .as_str()
        .expect("message should be string")
        .contains("Cannot Write to Readonly Binding"));
}

#[test]
fn namespace_naming_diagnostics_remain_compiler_owned_and_utf16_safe() {
    let source = "// 😀\nnamespace acme\\HTTP; function main(): void {}";
    assert_lsp_diagnostic_matches_compiler("namespace-naming.doria", source, "E0675");

    let diagnostics = diagnostics_for_document("file:///namespace-naming.doria", source);
    let naming = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic["code"] == "E0675")
        .collect::<Vec<_>>();
    assert_eq!(naming.len(), 2, "{diagnostics:#?}");
    assert!(naming
        .iter()
        .all(|diagnostic| diagnostic["source"] == "doriac"));
}

#[test]
fn stage32_attribute_diagnostics_remain_compiler_owned_and_utf16_safe() {
    let schema = r#"// 😀 keeps UTF-16 conversion honest.
#[Attribute]
class Route { function __construct(string $path, int $status) {} }
class Ordinary {}
function runtimePath(): string { return "/runtime"; }
"#;
    let cases = [
        ("E0686", "#[Missing] function target(): void {}"),
        (
            "E0687",
            "#[Attribute] class Route {} class Ordinary {} #[Ordinary] function target(): void {}",
        ),
        ("E0688", "#[Attribute] function target(): void {}"),
        (
            "E0689",
            "#[Attribute(repeatable: true)] class MarkerArguments {}",
        ),
        ("E0690", "#[Attribute] class GenericAttribute<T> {}"),
        (
            "E0692",
            "#[Attribute] class CollectionAttribute { function __construct(List<int> $values) {} }",
        ),
        (
            "E0516",
            "#[Route(path: \"/\", unknown: 200)] function target(): void {}",
        ),
        (
            "E0517",
            "#[Route(path: \"/\", status: 200, status: 201)] function target(): void {}",
        ),
        ("E0518", "#[Route(path: \"/\")] function target(): void {}"),
        (
            "E0403",
            "#[Route(path: 1, status: 200)] function target(): void {}",
        ),
        (
            "E0693",
            "#[Route(path: runtimePath(), status: 200)] function target(): void {}",
        ),
    ];

    for (code, body) in cases {
        let source = if matches!(
            code,
            "E0686" | "E0687" | "E0688" | "E0689" | "E0690" | "E0692"
        ) {
            format!("// 😀 keeps UTF-16 conversion honest.\n{body}\n")
        } else {
            format!("{schema}\n{body}\n")
        };
        assert_lsp_diagnostic_matches_compiler("stage32-attributes.doria", &source, code);
    }
}

#[test]
fn accepted_stage_30h_closure_routes_are_compiler_owned_and_target_neutral() {
    let cases = [
        (
            "no-capture.doria",
            "function main(): void { let $double = fn(int $value) => $value * 2; int $result = $double(21); }",
        ),
        (
            "capture.doria",
            "function main(): void { let $minimum = 70; let $passes = fn(int $score) with ($minimum) => $score >= $minimum; bool $result = $passes(70); }",
        ),
        (
            "callable-value.doria",
            "function main(): void { let $identity = fn(int $value) => $value; int $result = $identity(42); }",
        ),
        (
            "callable-property.doria",
            "class Runner { writable function(int): int $callback = fn(int $value) => $value; function run(int $value): int { return $this->callback($value); } } function main(): void { let $runner = new Runner(); int $result = $runner->run(42); }",
        ),
        (
            "checked-callable.doria",
            "class Failure implements Error { function __construct(string $message) {} } function main(): void { let $fail = function (): int { throw new Failure(\"x\"); }; try { $fail(); } catch (Failure $error) {} }",
        ),
        (
            "function-property.doria",
            "class Runner { function(int): int $callback = fn(int $value) => $value + 1; } function main(): void { let $runner = new Runner(); int $result = $runner->callback(41); }",
        ),
        (
            "native-writable-capture.doria",
            "function bind(writable int $value): function writable(): int { return function (): int with (writable $value) { $value += 1; return $value; }; } function main(): void { let writable $value = 41; writable function writable(): int $callback = bind($value); int $result = $callback(); }",
        ),
        (
            "nullable-function.doria",
            "function maybe(): ?function(): int { return fn() => 42; } function main(): void { let $callback = maybe(); if ($callback != null) { int $result = $callback(); } }",
        ),
        (
            "mixed-function.doria",
            "function inspect(mixed $value): void {} function boxed(): mixed { return fn() => 13; } function main(): void { let $callback = fn() => 42; inspect($callback); int $result = $callback(); mixed $returned = boxed(); }",
        ),
        (
            "final-collection-storage.doria",
            "function main(): void { (function(): int)[] $fixed = [fn() => 10]; writable SortedDictionary<string, function(): int> $sorted = SortedDictionary::from([]); $sorted->set(\"value\", fn() => 11); writable Deque<function(): int> $queue = Deque::from([]); $queue->pushBack(fn() => 12); function(): int $front = $queue->popFront() ?? fn() => 0; int $result = $fixed[0]() + $sorted[\"value\"]() + $front(); }",
        ),
        (
            "payload-enum-storage.doria",
            "enum Work { case Run(function(): int $callback); } function execute(take Work $work): int { return match (take $work) { Work::Run($callback) => $callback() }; } function main(): void { let $work = Work::Run(fn() => 42); int $result = execute($work); }",
        ),
        (
            "invariant-generic-storage.doria",
            "class Holder<T> { function __construct(take T $value) {} } function main(): void { let $callback = fn(int $value) => $value + 1; let $holder = new Holder<function(int): int>($callback); }",
        ),
    ];

    for (name, source) in cases {
        assert_stage_30_closure_is_valid(name, source);
    }
}

#[test]
fn mixed_function_identity_routes_are_compiler_owned_and_diagnostic_free() {
    let cases = [
        (
            "exact-mixed-function.doria",
            r#"function main(): void
{
    mixed $value = fn(int $number) => $number + 1;
    if ($value is function(int): int) {
        int $result = $value(41);
    }
}"#,
        ),
        (
            "wrong-mixed-function-identity.doria",
            r#"function main(): void
{
    mixed $value = fn(int $number) => $number;
    if ($value is function(): int) { echo "wrong"; }
    if ($value is function(int): int) { int $result = $value(42); }
}"#,
        ),
        (
            "nullable-mixed-functions.doria",
            r#"function main(): void
{
    ?function(): int $present = fn() => 7;
    mixed $boxedPresent = $present;
    ?function(): int $absent = null;
    mixed $boxedAbsent = $absent;
    if ($boxedPresent is function(): int) { int $value = $boxedPresent(); }
    if ($boxedAbsent is function(): int) { int $value = $boxedAbsent(); }
}"#,
        ),
        (
            "mixed-function-modes-and-effects.doria",
            r#"class Payload {}
class ParseError implements Error { function __construct(string $message) {} }
function inspect(take function writable(): int $callback): void
{
    writable mixed $value = $callback;
    if ($value is function(): int) {}
    if ($value is function writable(): int) { int $result = $value(); }
}
function main(): void
{
    let $payload = new Payload();
    mixed $once = function (): Payload with (take $payload) { return $payload; };
    if ($once is function(): Payload) {}
    if ($once is function once(): Payload) {}

    mixed $readonlyParameter = fn(Payload $value) => 1;
    if ($readonlyParameter is function(take Payload): int) {}
    if ($readonlyParameter is function(Payload): int) {}

    mixed $plain = fn() => 1;
    if ($plain is function(): int throws ParseError) {}

    mixed $throwing = function (): int { throw new ParseError("failure"); };
    if ($throwing is function(): int) {}
    if ($throwing is function(): int throws ParseError) {}
}"#,
        ),
    ];

    for (name, source) in cases {
        assert_stage_30_closure_is_valid(name, source);
        assert!(diagnostics_for_document(&format!("file:///{name}"), source)
            .iter()
            .all(|diagnostic| diagnostic["code"] != "E0641"));
    }
}

#[test]
fn mixed_function_extraction_diagnostics_remain_compiler_owned() {
    let name = "mixed-function-move.doria";
    let source = r#"function consume(take mixed $value): int
{
    int $result = match (take $value) {
        function(): int $callback => $callback(),
        default => -1,
    };
    echo "{$value}";
    return $result;
}
"#;
    let compiler = compiler_findings_for_publication(name, source);
    assert!(!compiler.is_empty());
    assert!(compiler.iter().all(|diagnostic| diagnostic.code != "E0641"));

    let lsp = diagnostics_for_document(&format!("file:///{name}"), source);
    let compiler_codes = compiler
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();
    let lsp_codes = lsp
        .iter()
        .map(|diagnostic| diagnostic["code"].as_str().expect("diagnostic code"))
        .collect::<Vec<_>>();
    assert_eq!(lsp_codes, compiler_codes);
    for diagnostic in &compiler {
        assert_lsp_diagnostic_matches_compiler(name, source, diagnostic.code);
    }
}

#[test]
fn stage_30g_algorithm_diagnostics_remain_compiler_owned_and_source_ordered() {
    let cases = [
        (
            "once-list-callback.doria",
            "class Token {} function main(): void { List<int> $values = [1]; let $token = new Token(); function once(int): int $callback = function (int $value): int with (take $token) { let $owned = $token; return $value; }; let $mapped = $values->map($callback); }",
            "E0664",
        ),
        (
            "readonly-writable-list-callback.doria",
            "function main(): void { List<int> $values = [1]; let writable $calls = 0; let $callback = function (int $value): int with (writable $calls) { $calls += 1; return $value; }; let $mapped = $values->map($callback); }",
            "E0668",
        ),
        (
            "move-filter.doria",
            "class Item {} function main(): void { List<Item> $items = [new Item()]; let $filtered = $items->filter(fn(Item $item) => true); }",
            "E0666",
        ),
        (
            "borrowed-map-result.doria",
            "class Item {} function main(): void { List<Item> $items = [new Item()]; List<Item> $same = $items->map(fn(Item $item) => $item); }",
            "E0667",
        ),
        (
            "reduce-shape.doria",
            "function main(): void { List<int> $values = [1]; let $result = $values->reduce(0, fn(int $sum, int $value) => $sum + $value); }",
            "E0665",
        ),
        (
            "checked-list-callback.doria",
            "class Failure implements Error { function __construct(string $message) {} } function transform(): void { List<int> $values = [1]; let $mapped = $values->map(function (int $value): int { throw new Failure(\"stop\"); }); } function main(): void {}",
            "E0631",
        ),
        (
            "named-list-algorithm.doria",
            "function main(): void { List<int> $values = [1]; let $mapped = $values->map(transform: fn(int $value) => $value); }",
            "E0519",
        ),
        (
            "other-collection-algorithm.doria",
            "function main(): void { Set<int> $values = Set::from([1]); let $mapped = $values->map(fn(int $value) => $value); }",
            "E0521",
        ),
    ];

    for (name, source, code) in cases {
        assert_lsp_diagnostic_matches_compiler(name, source, code);
        let compiler = compiler_findings_for_publication(name, source);
        let lsp = diagnostics_for_document(&format!("file:///{name}"), source);
        assert_eq!(
            lsp.iter()
                .filter_map(|diagnostic| diagnostic["code"].as_str())
                .collect::<Vec<_>>(),
            compiler
                .iter()
                .map(|diagnostic| diagnostic.code)
                .collect::<Vec<_>>(),
            "diagnostic order drift for {name}",
        );
        assert_eq!(
            lsp.iter()
                .filter(|diagnostic| diagnostic["code"] == code)
                .count(),
            1,
            "algorithm diagnostic must not be duplicated for {name}: {lsp:#?}",
        );
        assert!(
            lsp.iter().all(|diagnostic| diagnostic["code"] != "E0641"),
            "valid Stage 30g syntax must not receive E0641: {lsp:#?}",
        );
    }
}

#[test]
fn type_only_function_syntax_has_no_execution_boundary() {
    let cases = [
        (
            "writable-invocation.doria",
            "function accept(function writable(int): int $callback): void {}",
        ),
        (
            "once-invocation.doria",
            "class Payload {} function accept(function once(): Payload $factory): void {}",
        ),
        (
            "parameter-ownership.doria",
            "class Counter {} class Payload {} function accept(function(writable Counter): void $writer, function(take Payload): void $consumer): void {}",
        ),
        (
            "checked-effects.doria",
            "class ParseError implements Error { function __construct(string $message) {} } class StorageError implements Error { function __construct(string $message) {} } function accept(function(string): int throws ParseError, StorageError $parser): void {}",
        ),
        (
            "grouped-nested.doria",
            "class ParseError implements Error { function __construct(string $message) {} } function accept(function((function(): int throws ParseError), string): void $callback): void {}",
        ),
        (
            "inner-outer-effects.doria",
            "class InnerError implements Error { function __construct(string $message) {} } class OuterError implements Error { function __construct(string $message) {} } function accept(function(): (function(): int throws InnerError) throws OuterError $callback): void {}",
        ),
    ];

    for (name, source) in cases {
        doriac::check_source(name, source).expect("type-only function syntax is semantic");
        let diagnostics = diagnostics_for_document(&format!("file:///{name}"), source);
        assert!(diagnostics.is_empty(), "{name}: {diagnostics:#?}");
    }
}

#[test]
fn publishes_compiler_owned_stage_30b_diagnostics_without_redundant_boundaries() {
    let cases = [
        (
            "missing.doria",
            "let $value = 1; let $f = fn() => $value;",
            "E0642",
        ),
        (
            "duplicate.doria",
            "let $value = 1; let $f = fn() with ($value, $value) => $value;",
            "E0643",
        ),
        (
            "unused.doria",
            "let $value = 1; let $f = fn() with ($value) => 1;",
            "E0646",
        ),
        (
            "writable.doria",
            "let $value = 1; let $f = fn() with (writable $value) => $value;",
            "E0645",
        ),
        ("this.doria", "let $f = fn() with ($this) => 1;", "E0644"),
        ("recursive.doria", "let $f = fn() with ($f) => 1;", "E0647"),
        (
            "return.doria",
            "function(): int $f = fn() => \"wrong\";",
            "E0648",
        ),
        (
            "nullable.doria",
            "function invoke(?function(): int $f): void { $f(); }",
            "E0650",
        ),
        (
            "access.doria",
            "function invoke(function writable(): int $f): void { $f(); }",
            "E0651",
        ),
        (
            "named.doria",
            "class Worker { function(int): int $format; function __construct(function(int): int $format) { $this->format = $format; } } function invoke(Worker $worker): void { $worker->format(value: 1); }",
            "E0652",
        ),
    ];

    for (name, source, code) in cases {
        assert_lsp_diagnostic_matches_compiler(name, source, code);
        let diagnostics = diagnostics_for_document(&format!("file:///{name}"), source);
        if code != "E0646" {
            assert!(
                diagnostics
                    .iter()
                    .all(|diagnostic| diagnostic["code"] != "E0641"),
                "invalid source must not also receive E0641: {diagnostics:#?}"
            );
        }
    }
}

#[test]
fn publishes_the_complete_stage_30c_ownership_diagnostic_surface() {
    let cases = [
        (
            "readonly-lease-conflict.doria",
            "function main(): void { let writable $value = 1; let $read = fn() with ($value) => $value; $value = 2; $read(); }",
            "E0654",
        ),
        (
            "writable-lease-conflict.doria",
            "function main(): void { let writable $value = 1; let writable $write = function (): void with (writable $value) { $value += 1; }; let $read = fn() with ($value) => $value; $write(); $read(); }",
            "E0654",
        ),
        (
            "taking-capture-use-after-move.doria",
            "class Payload {} function main(): void { let $value = new Payload(); let $first = fn() with (take $value) => $value; let $second = fn() with (take $value) => $value; }",
            "E0655",
        ),
        (
            "function-value-move.doria",
            "function main(): void { let $first = fn() => 1; let $second = $first; $first; $second(); }",
            "E0655",
        ),
        (
            "once-reuse.doria",
            "class Payload {} function main(): void { let $value = new Payload(); let $once = function (): Payload with (take $value) { return $value; }; $once(); $once(); }",
            "E0655",
        ),
        (
            "maybe-consumed-once.doria",
            "class Payload {} function main(bool $condition): void { let $value = new Payload(); let $once = function (): Payload with (take $value) { return $value; }; if ($condition) { $once(); } $once(); }",
            "E0655",
        ),
        (
            "borrowed-once-parameter.doria",
            "class Payload {} function invoke(function once(): Payload $factory): Payload { return $factory(); }",
            "E0656",
        ),
        (
            "stored-once-invocation.doria",
            "class Store { writable function once(): int $factory = fn() => 1; writable function make(): int { return $this->factory(); } }",
            "E0660",
        ),
        (
            "borrow-bound-collection-escape.doria",
            "function main(): void { writable List<function(): int> $items = []; { let $value = 1; let $borrowed = fn() with ($value) => $value; $items = [$borrowed]; } }",
            "E0762",
        ),
        (
            "nonescaping-collection-retention.doria",
            "function store(function(): int $callback): void { List<function(): int> $items = [$callback]; }",
            "E0657",
        ),
        (
            "borrow-bound-local-escape.doria",
            "function main(): void { writable function(): int $callback = fn() => 0; { let $value = 1; $callback = fn() with ($value) => $value; } }",
            "E0658",
        ),
        (
            "nonescaping-retention.doria",
            "function retain(function(): int $callback): function(): int { return $callback; }",
            "E0657",
        ),
        (
            "borrowed-property-retention.doria",
            "class Store { function(): int $callback; function __construct(function(): int $callback) { $this->callback = $callback; } }",
            "E0657",
        ),
        (
            "returned-local-borrow.doria",
            "function invalid(): function(): int { let $value = 1; return fn() with ($value) => $value; }",
            "E0658",
        ),
        (
            "multiple-return-roots.doria",
            "function invalid(int $left, int $right): function(): int { return fn() with ($left, $right) => $left + $right; }",
            "E0659",
        ),
        (
            "receiver-cycle.doria",
            "class Box { writable function(): int $callback = fn() => 0; writable function install(): void { $this->callback = fn() with ($this) => 1; } }",
            "E0658",
        ),
        (
            "incomplete-constructor-capture.doria",
            "class Box { int $value; function __construct() { let $read = fn() with ($this) => $this->value; $this->value = 1; } }",
            "E0503",
        ),
    ];

    for (name, source, code) in cases {
        assert_lsp_diagnostic_matches_compiler(name, source, code);
        let compiler = compiler_findings_for_publication(name, source);
        let compiler_codes = compiler
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<Vec<_>>();
        let lsp = diagnostics_for_document(&format!("file:///{name}"), source);
        let lsp_codes = lsp
            .iter()
            .filter_map(|diagnostic| diagnostic["code"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            lsp_codes, compiler_codes,
            "diagnostic order drift for {name}"
        );
        if code != "E0661" {
            let ownership_diagnostic = lsp
                .iter()
                .find(|diagnostic| diagnostic["code"] == code)
                .unwrap_or_else(|| panic!("missing {code}: {lsp:#?}"));
            assert!(
                ownership_diagnostic["data"]["developmentOnly"] == false,
                "ordinary ownership finding is not a development boundary: {ownership_diagnostic:#?}"
            );
        }
    }

    assert_stage_30_closure_is_valid(
        "stage30d-valid.doria",
        "function main(): void { let $callback = fn() => 1; }",
    );
}

#[test]
fn ownership_edits_remain_compiler_owned_and_review_only() {
    let uri = "file:///ownership-review.doria";
    let cases = [
        (
            "function invalid(): function(): int { let $value = 1; let $borrowed = fn() with (/* keep */ $value) => $value; return $borrowed; }",
            "E0658",
            "Capture Borrowed Values With Ownership",
        ),
        (
            "class Store { writable function(): int $callback = fn() => 0; writable function retain(/* keep */ function(): int $input): void { $this->callback = $input; } }",
            "E0657",
            "Accept Callback With Ownership",
        ),
    ];

    for (source, code, title) in cases {
        let diagnostics = diagnostics_for_document(uri, source);
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic["code"] == code)
            .unwrap_or_else(|| panic!("missing {code}: {diagnostics:#?}"));
        let fixes = diagnostic["data"]["fixes"]
            .as_array()
            .expect("compiler-owned structured fixes");
        assert_eq!(fixes.len(), 1, "{diagnostic:#?}");
        assert_eq!(fixes[0]["title"], title);
        assert_eq!(fixes[0]["applicability"], "requiresReview");
        let replacement = fixes[0]["edits"][0]["newText"]
            .as_str()
            .expect("fix replacement");
        assert_eq!(replacement, "take ");
        assert!(
            !replacement.contains("clone")
                && !replacement.contains("SharedReference")
                && !replacement.contains("lifetime")
        );
        assert!(
            code_actions_for_document(uri, source).is_empty(),
            "ownership-changing review edits must not become automatic actions"
        );
    }
}

#[test]
fn constructor_rooted_property_writes_follow_the_pinned_compiler() {
    let accepted = [
        (
            "constructor-nested-write.doria",
            r#"class Window
{
    writable string $title;
    function __construct(string $inputTitle) { $this->title = $inputTitle; }
}
class Application
{
    internal writable Window $window = new Window("");
    function __construct(string $inputTitle) { $this->window->title = $inputTitle; }
}"#,
        ),
        (
            "constructor-owned-initialize.doria",
            r#"class Window
{
    string $title;
    function __construct(string $inputTitle) { $this->title = $inputTitle; }
}
class Application
{
    internal writable Window $window;
    function __construct(string $inputTitle) { $this->window = new Window($inputTitle); }
}"#,
        ),
        (
            "owned-property-replace.doria",
            r#"class Window {}
class Application
{
    internal writable Window $window = new Window();
    writable function replace(take Window $window): void { $this->window = $window; }
}"#,
        ),
    ];

    for (name, source) in accepted {
        doriac::check_source(name, source).expect("the pinned compiler must accept the write");
        let diagnostics = diagnostics_for_document(&format!("file:///{name}"), source);
        assert!(diagnostics.is_empty(), "{name}: {diagnostics:#?}");
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic["code"] != "E0472"),
            "valid move-in must not publish historical E0472: {diagnostics:#?}"
        );
    }
}

#[test]
fn constructor_property_write_boundaries_preserve_compiler_diagnostics() {
    let cases = [
        (
            "readonly-intermediate.doria",
            r#"class Window { writable string $title = ""; }
class Application
{
    Window $window = new Window();
    function __construct() { $this->window->title = "Doria"; }
}"#,
            "E0201",
        ),
        (
            "uninitialized-intermediate.doria",
            r#"class Window { writable string $title = ""; }
class Application
{
    writable Window $window;
    function __construct() { $this->window->title = "Doria"; }
}"#,
            "E0501",
        ),
        (
            "borrowed-owned-property.doria",
            r#"class Window {}
function inspect(Window $window): Window { return $window; }
class Application
{
    writable Window $window = new Window();
    writable function replace(Window $candidate): void
    {
        $this->window = inspect($candidate);
    }
}"#,
            "E0478",
        ),
        (
            "overlapping-property-move.doria",
            r#"class Window {}
class Application
{
    writable Window $window = new Window();
    writable function replace(): void { $this->window = $this->window; }
}"#,
            "E0471",
        ),
    ];

    for (name, source, code) in cases {
        assert_lsp_diagnostic_matches_compiler(name, source, code);
    }

    assert_lsp_diagnostic_matches_compiler(
        "closure-capture-regression.doria",
        "let $value = 1; let $closure = fn() => $value;",
        "E0642",
    );
}

#[test]
fn distinguishes_function_shape_and_checked_effect_mismatches() {
    let shape = "function(int): int $f = fn(string $value) => 1;";
    assert_lsp_diagnostic_matches_compiler("shape-mismatch.doria", shape, "E0648");
    let shape_diagnostic = diagnostics_for_document("file:///shape-mismatch.doria", shape)
        .into_iter()
        .find(|diagnostic| diagnostic["code"] == "E0648")
        .expect("function-shape mismatch");
    assert!(shape_diagnostic["message"]
        .as_str()
        .is_some_and(|message| message.contains("parameter 1")));

    let effects = r#"class Failure implements Error { function __construct(string $message) {} }
function(): void $f = function (): void { throw new Failure("later"); };"#;
    assert_lsp_diagnostic_matches_compiler("effect-mismatch.doria", effects, "E0648");
    let effect_diagnostic = diagnostics_for_document("file:///effect-mismatch.doria", effects)
        .into_iter()
        .find(|diagnostic| diagnostic["code"] == "E0648")
        .expect("checked-effect mismatch");
    assert!(effect_diagnostic["message"]
        .as_str()
        .is_some_and(|message| message.contains("checked errors")));
}

#[test]
fn exposes_only_safe_compiler_owned_capture_fixes_as_code_actions() {
    let uri = "file:///capture-actions.doria";
    let readonly = "let $value = 1; let $f = fn() => $value;";
    let actions = code_actions_for_document(uri, readonly);
    assert_eq!(actions.len(), 1, "{actions:#?}");
    assert_eq!(actions[0]["title"], "Add Missing Closure Captures");
    assert_eq!(
        actions[0]["edit"]["changes"][uri][0]["newText"],
        " with ($value)"
    );

    let extended =
        "let $base = 1; let $minimum = 2; let $f = fn() with ($base) => $base + $minimum;";
    let actions = code_actions_for_document(uri, extended);
    assert_eq!(actions.len(), 1, "{actions:#?}");
    assert_eq!(
        actions[0]["edit"]["changes"][uri][0]["newText"],
        ", $minimum"
    );

    let writable =
        "let writable $total = 0; let $f = function (): int { $total += 1; return $total; };";
    let actions = code_actions_for_document(uri, writable);
    assert_eq!(actions.len(), 1, "{actions:#?}");
    assert_eq!(
        actions[0]["edit"]["changes"][uri][0]["newText"],
        " with (writable $total)"
    );

    let unused = "let $value = 1; let $f = fn() with ($value) => 1;";
    let actions = code_actions_for_document(uri, unused);
    assert_eq!(actions.len(), 1, "{actions:#?}");
    assert_eq!(actions[0]["edit"]["changes"][uri][0]["newText"], "");

    let one_unused =
        "let $used = 1; let $unused = 2; let $f = fn() with ($used, $unused) => $used;";
    let actions = code_actions_for_document(uri, one_unused);
    assert_eq!(actions.len(), 1, "{actions:#?}");
    assert_eq!(actions[0]["title"], "Remove the Unused Capture Entry");
    assert_eq!(actions[0]["edit"]["changes"][uri][0]["newText"], "");

    let taking = r#"class Payload {}
let $payload = new Payload();
let $f = function (): Payload { return $payload; };"#;
    let actions = code_actions_for_document(uri, taking);
    assert!(
        actions.iter().all(|action| {
            !action["edit"]["changes"][uri]
                .as_array()
                .into_iter()
                .flatten()
                .any(|edit| {
                    edit["newText"]
                        .as_str()
                        .is_some_and(|text| text.contains("take"))
                })
        }),
        "the server must not synthesize an ownership-transferring `take` fix: {actions:#?}"
    );

    let commented = "let $value = 1; let $f = fn() with (/* keep */ $value) => 1;";
    assert!(
        code_actions_for_document(uri, commented).is_empty(),
        "comment-owning capture removal requires review"
    );
}

#[test]
fn stage_30a_does_not_change_named_function_method_or_static_calls() {
    let source = r#"function identity(int $value): int { return $value; }
class Calculator
{
    function identity(int $value): int { return $value; }
    static function staticIdentity(int $value): int { return $value; }
}
function main(): void
{
    let $calculator = new Calculator();
    int $a = identity(1);
    int $b = $calculator->identity(2);
    int $c = Calculator::staticIdentity(3);
}"#;

    let diagnostics = diagnostics_for_document("file:///ordinary-calls.doria", source);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn malformed_stage_30a_forms_remain_parser_diagnostics() {
    let cases = [
        "class Payload {} function accept(function take(): Payload $callback): void {}",
        "function accept(function readonly(int): int $callback): void {}",
        "class Payload {} function accept(function(take writable Payload): void $callback): void {}",
        "function accept(function(): void throws $callback): void {}",
        "function accept((int, string) $pair): void {}",
        "let $callback = fn(int $value) => $value; $callback(value: 42);",
    ];

    for (index, source) in cases.into_iter().enumerate() {
        let name = format!("malformed-stage30a-{index}.doria");
        doriac::parse_source(&name, source).expect_err("malformed Stage 30a syntax must not parse");
        let diagnostics = diagnostics_for_document(&format!("file:///{name}"), source);
        assert!(
            !diagnostics.is_empty(),
            "missing parser diagnostic for {source}"
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic["code"] != "E0641"),
            "malformed syntax was misreported as the semantic boundary: {diagnostics:#?}"
        );
    }
}

#[test]
fn stage_30_diagnostics_preserve_compiler_source_order_without_redundant_e0641() {
    let source = r#"function main(): void
{
    int $bad = "not an integer";
    let $callback = fn(int $value) => $value;
    $callback(1);
}"#;
    let compiler = compiler_findings_for_publication("stage30a-order.doria", source);
    let lsp = diagnostics_for_document("file:///stage30a-order.doria", source);
    let compiler_codes = compiler
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();
    let lsp_codes = lsp
        .iter()
        .map(|diagnostic| diagnostic["code"].as_str().expect("diagnostic code"))
        .collect::<Vec<_>>();

    assert_eq!(lsp_codes, compiler_codes);
    assert_eq!(lsp_codes, ["E0403"]);
    assert!(lsp.windows(2).all(|pair| {
        pair[0]["range"]["start"]["line"].as_u64() <= pair[1]["range"]["start"]["line"].as_u64()
    }));
    assert!(lsp.iter().all(|diagnostic| diagnostic["code"] != "E0641"));
}

#[test]
fn missing_capture_diagnostic_stays_off_following_source() {
    let source =
        "let $outside = 1; let $closure = fn(int $value) => $outside + $value; let $after = 1;";
    assert_lsp_diagnostic_matches_compiler("closure-body-boundary.doria", source, "E0642");

    let diagnostic = diagnostics_for_document("file:///closure-body-boundary.doria", source)
        .into_iter()
        .find(|diagnostic| diagnostic["code"] == "E0642")
        .expect("missing capture");
    let after = byte_offset_to_position(source, source.find("let $after").unwrap());
    assert!(
        diagnostic["range"]["end"]["character"].as_u64().unwrap() < after.character as u64,
        "the closure diagnostic must not extend into the following statement: {diagnostic:#?}"
    );
}

#[test]
fn malformed_capture_forms_remain_parser_diagnostics_not_stage_30_boundaries() {
    let cases = [
        "let $closure = fn(int $value) use ($outside) => $value;",
        "let $closure = fn(int $value) with () => $value;",
        "let $closure = fn(int $value) with (&$outside) => $value;",
        "let $closure = fn(int $value) with (readonly $outside) => $value;",
    ];

    for (index, source) in cases.into_iter().enumerate() {
        let name = format!("malformed-closure-{index}.doria");
        doriac::parse_source(&name, source).expect_err("malformed closure must not parse");
        let diagnostics = diagnostics_for_document(&format!("file:///{name}"), source);
        assert!(
            !diagnostics.is_empty(),
            "missing parser diagnostic for {source}"
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic["code"] != "E0641"),
            "malformed syntax was misreported as the semantic boundary: {diagnostics:#?}"
        );
    }
}

#[test]
fn grouped_local_declarations_use_compiler_diagnostics_without_false_positives() {
    let accepted = diagnostics_for_document(
        "file:///grouped.doria",
        r#"function main(): void
{
    let $left, $right = 1;
    let writable $red, $blue = 2;
    int $minimum, $maximum = 3;
    writable string $first, $second = "value";
    echo "{$left}:{$right}:{$red}:{$blue}:{$minimum}:{$maximum}:{$first}:{$second}";
}
"#,
    );
    assert!(accepted.is_empty(), "{accepted:#?}");

    let duplicate = diagnostics_for_document(
        "file:///duplicate.doria",
        "function main(): void { let $value, $value = 1; }",
    );
    assert!(duplicate
        .iter()
        .any(|diagnostic| diagnostic["code"] == "E0103"));

    let owned = diagnostics_for_document(
        "file:///owned.doria",
        "class Token {} function main(): void { let $left, $right = new Token(); }",
    );
    assert!(owned.iter().any(|diagnostic| diagnostic["code"] == "E0551"));
}

#[test]
fn accepts_zero_argument_read_line_without_false_diagnostics() {
    let diagnostics = diagnostics_for_document(
        "file:///input.doria",
        "function main(): void { let $line = read_line(); }",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn accepts_prompted_read_line_without_false_diagnostics() {
    let diagnostics = diagnostics_for_document(
        "file:///input.doria",
        "function main(): void { let $line = read_line(\"Name: \"); }",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn accepts_compiler_inferred_checked_effects_for_selected_main() {
    for (name, source) in [
        (
            "inferred-direct-io.doria",
            "function main(): void { echo \"value\"; }",
        ),
        (
            "inferred-helper-io.doria",
            r#"function greet(): void throws Doria\Std\Io\IoError { echo "value"; }
function main(): void { greet(); }"#,
        ),
    ] {
        let diagnostics = diagnostics_for_document(&format!("file:///{name}"), source);
        assert!(diagnostics.is_empty(), "{name}: {diagnostics:#?}");
    }
}

#[test]
fn publishes_inferred_main_diagnostics_in_source_order() {
    let diagnostics = diagnostics_for_document(
        "file:///inferred-main-order.doria",
        r#"function earlier(): void
{
    int $value = "not an integer";
}
function main(): void
{
    int $value = "also not an integer";
}"#,
    );
    let assignment_lines = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic["code"] == "E0403")
        .map(|diagnostic| {
            diagnostic["range"]["start"]["line"]
                .as_u64()
                .expect("diagnostic start line")
        })
        .collect::<Vec<_>>();

    assert_eq!(assignment_lines, [2, 6]);
}

#[test]
fn accepts_ambient_io_in_ordinary_helpers_without_source_contracts() {
    for (name, source) in [
        (
            "echo-helper.doria",
            "function greet(): void { echo \"value\"; } function main(): void { greet(); }",
        ),
        (
            "read-helper.doria",
            "function read(): ?string { return read_line(); } function main(): void { read(); }",
        ),
    ] {
        let diagnostics = diagnostics_for_document(&format!("file:///{name}"), source);
        assert!(diagnostics.is_empty(), "{name}: {diagnostics:#?}");
    }
}

#[test]
fn accepts_explicit_ambient_throws_without_requiring_the_complete_ambient_set() {
    let source = r#"function main(): void throws Doria\Std\Io\IoError
{
    let $line = read_line();
}"#;
    let diagnostics = diagnostics_for_document("file:///explicit-ambient-effects.doria", source);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn preserves_destructor_boundary_and_accepts_ambient_finalizers() {
    let destructor = diagnostics_for_document(
        "file:///destructor-output.doria",
        r#"class Log
{
    function __destruct() { echo "drop"; }
}
"#,
    );
    let destructor_diagnostic = destructor
        .iter()
        .find(|diagnostic| diagnostic["code"] == "E0631")
        .unwrap_or_else(|| panic!("missing destructor checked-effect diagnostic: {destructor:#?}"));
    let message = destructor_diagnostic["message"]
        .as_str()
        .expect("destructor diagnostic message");
    assert!(message.contains("Destructors Cannot Throw Checked Errors"));
    assert!(message.contains("Doria\\Std\\Io\\IoError"));

    let finalizer = diagnostics_for_document(
        "file:///finalizer-output.doria",
        r#"function main(): void
{
    if (true) {} finally { echo "cleanup"; }
}
"#,
    );
    assert!(finalizer.is_empty(), "{finalizer:#?}");
}

#[test]
fn ambient_closures_and_list_callbacks_need_no_source_contract() {
    let source = r#"function main(): void
{
    let $callback = function (): void { echo "closure"; };
    $callback();
    List<int> $values = [1, 2];
    List<string> $mapped = $values->map(function (int $value): string {
        echo "{$value}";
        return "{$value}";
    });
}"#;
    let diagnostics = diagnostics_for_document("file:///ambient-callbacks.doria", source);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn fallible_finalizers_flow_to_outer_context_but_not_same_try_catches() {
    let error = r#"class CleanupError implements Error
{
    function __construct(string $message) {}
}
function cleanup(): void throws CleanupError
{
    throw new CleanupError("cleanup");
}
"#;
    let handled = format!(
        r#"{error}
function main(): void
{{
    try {{
        try {{ if (false) {{ cleanup(); }} }} catch (CleanupError) {{}} finally {{ cleanup(); }}
    }} catch (CleanupError) {{}}
}}
"#
    );
    let diagnostics = diagnostics_for_document("file:///outer-finalizer-catch.doria", &handled);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");

    let same_try_only = format!(
        r#"{error}
function run(): void
{{
    try {{ if (false) {{ cleanup(); }} }} catch (CleanupError) {{}} finally {{ cleanup(); }}
}}
function main(): void {{}}
"#
    );
    assert_lsp_diagnostic_matches_compiler(
        "same-try-finalizer-catch.doria",
        &same_try_only,
        "E0631",
    );
}

#[test]
fn missing_nonambient_callable_contract_remains_compiler_owned() {
    let source = r#"class Failure implements Error
{
    function __construct(string $message) {}
}
function fail(): void throws Failure { throw new Failure("x"); }
function caller(): void { fail(); }
function main(): void {}
"#;
    assert_lsp_diagnostic_matches_compiler("required-effect.doria", source, "E0631");
}

#[test]
fn retired_finalizer_diagnostic_remains_catalogued_but_is_never_published() {
    assert!(doriac::diagnostics::CATALOGUED_CODES.contains(&"E0632"));
    let diagnostics = diagnostics_for_document(
        "file:///fallible-finalizer.doria",
        r#"function main(): void { try {} finally { echo "cleanup"; } }"#,
    );
    assert!(diagnostics
        .iter()
        .all(|diagnostic| diagnostic["code"] != "E0632"));
}

#[test]
fn accepts_only_the_canonical_compiler_known_io_type_identities() {
    let diagnostics = diagnostics_for_document(
        "file:///canonical-io-types.doria",
        r#"function inspect(
    Doria\Std\Io\IoOperation $operation,
    Doria\Std\Io\IoTarget $target,
    Doria\Std\Io\IoErrorReason $reason,
    Doria\Std\Io\Utf8InputSource $source,
    Doria\Std\Io\IoError $io,
    Doria\Std\Io\InvalidUtf8Error $utf8,
    Doria\Std\Io\ByteRegion $region,
    Doria\Std\Io\WritableByteRegion $writableRegion
): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
}
"#,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");

    let short = diagnostics_for_document(
        "file:///short-io-type.doria",
        "function inspect(IoError $error): void {}",
    );
    assert!(
        short.iter().any(|diagnostic| diagnostic["code"] == "E0401"),
        "short aliases must remain unknown: {short:#?}"
    );
}

#[test]
fn stream_names_and_aliases_preserve_compiler_identity_diagnostics() {
    use doriac::ast::Item;

    let declarations = doriac::compiler_known_stream::declarations();
    for name in doriac::compiler_known_stream::CANONICAL_TYPES {
        let (namespace, short) = name.rsplit_once('\\').unwrap();
        let params = declarations
            .items
            .iter()
            .find_map(|item| match item {
                Item::Class(decl) if decl.name == name => Some(&decl.type_params),
                Item::Interface(decl) if decl.name == name => Some(&decl.type_params),
                Item::Enum(decl) if decl.name == name => Some(&decl.type_params),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing compiler declaration for {name}"));
        let (generics, arguments) = if params.is_empty() {
            (String::new(), String::new())
        } else {
            let mut bindings = Vec::new();
            let mut arguments = Vec::new();
            for param in params {
                let name = doriac::trait_composition::authored_type_parameter_name(&param.name);
                arguments.push(name);
                let constraints = param
                    .constraints
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                bindings.push(if constraints.is_empty() {
                    name.to_string()
                } else {
                    format!("{name} implements {constraints}")
                });
            }
            (
                format!("<{}>", bindings.join(", ")),
                format!("<{}>", arguments.join(", ")),
            )
        };
        for source in [
            format!("function inspect{generics}({name}{arguments} $value): void {{}}"),
            format!(
                "use {name} as View; function inspect{generics}(View{arguments} $value): void {{}}"
            ),
        ] {
            let diagnostics = diagnostics_for_document("file:///stream-identity.doria", &source);
            assert!(diagnostics.is_empty(), "{name}: {diagnostics:#?}");
        }
        let unimported = format!("function inspect({short} $value): void {{}}");
        assert_lsp_diagnostic_matches_compiler("unimported-stream.doria", &unimported, "E0401");
        let reserved = format!("namespace {namespace}; interface {short} {{}}");
        assert_lsp_diagnostic_matches_compiler("reserved-stream.doria", &reserved, "E0640");
    }
}

#[test]
fn file_creation_imports_and_exhaustiveness_preserve_compiler_contracts() {
    let body = r#"function describe(FileCreation $creation): string {
    /* 😀 */ return match ($creation) {
        FileCreation::OpenExisting => "open existing",
        FileCreation::OpenOrCreate => "open or create",
        FileCreation::CreateNew => "create new",
        FileCreation::TruncateExisting => "truncate existing",
        FileCreation::CreateOrTruncate => "create or truncate"
    };
}
"#;
    for (import, spelling) in [
        ("", "Doria\\Std\\Io\\FileCreation"),
        ("use Doria\\Std\\Io\\FileCreation;", "FileCreation"),
        ("use Doria\\Std\\Io\\FileCreation as Creation;", "Creation"),
        (
            "use Doria\\Std\\Io\\{FileCreation as Creation};",
            "Creation",
        ),
    ] {
        let source = format!("{import}\n{}", body.replace("FileCreation", spelling));
        let diagnostics = diagnostics_for_document("file:///file-creation.doria", &source);
        assert!(diagnostics.is_empty(), "{source}\n{diagnostics:#?}");
        let incomplete = source.replace(
            &format!("        {spelling}::CreateOrTruncate => \"create or truncate\""),
            "",
        );
        assert_lsp_diagnostic_matches_compiler(
            "file-creation-exhaustiveness.doria",
            &incomplete,
            "E0585",
        );
    }
    assert_lsp_diagnostic_matches_compiler(
        "reserved-file-creation.doria",
        "/* 😀 */ namespace Doria\\Std\\Io; enum FileCreation { case Custom; }",
        "E0640",
    );
}

#[test]
fn close_operation_preserves_canonical_io_and_exhaustiveness_diagnostics() {
    let source = r#"use Doria\Std\Io\{IoError, IoErrorReason, IoOperation, IoTarget};
function failClose(): void throws IoError {
    throw new IoError("closed", IoOperation::Close, IoTarget::File("fixture.bin"), IoErrorReason::Closed);
}
function describe(IoOperation $operation): string {
    /* 😀 */ return match ($operation) {
        IoOperation::Open => "open",
        IoOperation::Read => "read",
        IoOperation::Write => "write",
        IoOperation::Append => "append",
        IoOperation::Flush => "flush",
        IoOperation::Close => "close",
        IoOperation::Register => "register",
        IoOperation::Unregister => "unregister",
        IoOperation::Wait => "wait",
        IoOperation::Configure => "configure",
        IoOperation::Access => "access",
        IoOperation::Seek => "seek",
        IoOperation::SynchronizeData => "synchronizedata",
        IoOperation::SynchronizeMetadata => "synchronizemetadata",
        IoOperation::Resize => "resize",
        IoOperation::Lock => "lock",
        IoOperation::Unlock => "unlock",
        IoOperation::Spawn => "spawn",
        IoOperation::Terminate => "terminate",
    };
}
"#;
    let diagnostics = diagnostics_for_document("file:///close-operation.doria", source);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    for case in [
        "Close",
        "Register",
        "Unregister",
        "Wait",
        "Configure",
        "Access",
        "Seek",
        "SynchronizeData",
        "SynchronizeMetadata",
        "Resize",
        "Lock",
        "Unlock",
        "Spawn",
        "Terminate",
    ] {
        let arm = format!(
            "        IoOperation::{case} => \"{}\",",
            case.to_ascii_lowercase()
        );
        assert!(source.contains(&arm));
        let incomplete = source.replace(&arm, "");
        assert_lsp_diagnostic_matches_compiler(
            "close-operation-exhaustiveness.doria",
            &incomplete,
            "E0585",
        );
    }
}

#[test]
fn readiness_infrastructure_errors_preserve_canonical_io_contracts() {
    for operation in ["Register", "Unregister", "Wait"] {
        let source = format!(
            r#"use Doria\Std\Io\{{IoError, IoErrorReason, IoOperation, IoTarget}};
function fail(): void {{
    throw new IoError("readiness failure", IoOperation::{operation}, IoTarget::ReadinessSet, IoErrorReason::Other);
}}
function describe(IoTarget $target): string {{
    /* 😀 */ return match ($target) {{
        IoTarget::File($path) => $path,
        IoTarget::StandardInput => "stdin",
        IoTarget::StandardOutput => "stdout",
        IoTarget::StandardError => "stderr",
        IoTarget::ReadinessSet => "readiness",
        IoTarget::TransferPlan => "configuration",
        IoTarget::TransferGroup => "group",
        IoTarget::Process($program) => $program,
        IoTarget::ProcessInput($program) => $program,
        IoTarget::ProcessOutput($program) => $program,
        IoTarget::ProcessErrors($program) => $program,
    }};
}}
function main(): void {{
    try {{ fail(); }} catch (IoError $error) {{
        ?int $progress = $error->transferredByteCount;
        describe($error->target);
    }}
}}
"#
        );
        let name = "readiness-infrastructure-error.doria";
        doriac::check_source(name, &source)
            .unwrap_or_else(|diagnostics| panic!("{operation}: {diagnostics:#?}"));
        let diagnostics = diagnostics_for_document(&format!("file:///{name}"), &source);
        assert!(diagnostics.is_empty(), "{operation}: {diagnostics:#?}");
        for arm in [
            "        IoTarget::ReadinessSet => \"readiness\",",
            "        IoTarget::TransferPlan => \"configuration\",",
            "        IoTarget::TransferGroup => \"group\",",
            "        IoTarget::ProcessErrors($program) => $program,",
        ] {
            let incomplete = source.replace(arm, "");
            assert_lsp_diagnostic_matches_compiler(name, &incomplete, "E0585");
        }
    }
}

#[test]
fn readable_file_access_and_constructor_diagnostics_remain_compiler_owned() {
    let source = r#"use Doria\Std\Io\{File, ReadableStream, WritableByteRegion, ReadOutcome};
function readErased(writable ReadableStream $stream, writable WritableByteRegion $destination): ReadOutcome {
    return $stream->read($destination);
}
function inspect(): void {
    /* 😀 */ let writable $file = File::openRead("input.txt");
    writable Bytes $buffer = Bytes::allocate(1);
    {
        let writable $destination = $buffer->getWritableRegion(0, 1);
        let $direct = $file->read($destination);
        let $erased = readErased($file, $destination);
    }
    $file->close();
}
"#;
    let diagnostics = diagnostics_for_document("file:///readable-file.doria", source);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    assert_lsp_diagnostic_matches_compiler(
        "readonly-file.doria",
        &source.replace("let writable $file", "let $file"),
        "E0203",
    );
    assert_lsp_diagnostic_matches_compiler(
        "readonly-file-destination.doria",
        &source.replace("let writable $destination", "let $destination"),
        "E0479",
    );
    for name in ["File", "ReadableFile", "WritableFile", "DuplexFile"] {
        let source = format!("use Doria\\Std\\Io\\{name}; function inspect(): void {{ /* 😀 */ let $file = new {name}(); }}");
        assert_lsp_diagnostic_matches_compiler("internal-file-constructor.doria", &source, "E0307");
    }
}

#[test]
fn writable_and_duplex_file_diagnostics_remain_compiler_owned() {
    for factory in ["openWrite", "openReadWrite"] {
        let duplex_call = if factory == "openReadWrite" {
            "readWriteErased($file, $destination, $source);"
        } else {
            ""
        };
        let source = format!(
            r#"use Doria\Std\Io\{{File, FileCreation, FileWriteMode, WritableStream, DuplexStream, ByteRegion, WritableByteRegion, WriteOutcome}};
function writeErased(writable WritableStream $stream, ByteRegion $source): WriteOutcome {{
    return $stream->write($source);
}}
function readWriteErased(writable DuplexStream $stream, writable WritableByteRegion $destination, ByteRegion $source): void {{
    $stream->read($destination);
    $stream->write($source);
}}
function inspect(): void {{
    /* 😀 */ let writable $file = File::{factory}("input.txt", FileCreation::OpenOrCreate, FileWriteMode::Append);
    let $input = Bytes::allocate(1);
    let $source = $input->getRegion(0, 1);
    writable Bytes $buffer = Bytes::allocate(1);
    let writable $destination = $buffer->getWritableRegion(0, 1);
    writeErased($file, $source);
    {duplex_call}
    $file->close();
}}
"#
        );
        let diagnostics = diagnostics_for_document("file:///writable-file.doria", &source);
        assert!(diagnostics.is_empty(), "{source}\n{diagnostics:#?}");
        assert_lsp_diagnostic_matches_compiler(
            "readonly-writable-file.doria",
            &source.replace("let writable $file", "let $file"),
            "E0479",
        );
        assert_lsp_diagnostic_matches_compiler(
            "missing-file-mode.doria",
            &source.replace(", FileWriteMode::Append", ""),
            "E0409",
        );
        if factory == "openReadWrite" {
            assert_lsp_diagnostic_matches_compiler(
                "readonly-duplex-destination.doria",
                &source.replace("let writable $destination", "let $destination"),
                "E0479",
            );
        } else {
            assert_lsp_diagnostic_matches_compiler(
                "wrong-file-capability.doria",
                &source.replace("writeErased($file, $source);", "$file->read($destination);"),
                "E0304",
            );
        }
    }
}

#[test]
fn readable_file_reuse_after_close_preserves_compiler_move_diagnostics() {
    for close in [
        "$file->close();",
        "try { $file->close(); } catch (IoError) {}",
    ] {
        let source = format!(
            r#"use Doria\Std\Io\{{File, IoError}};
function inspect(): void {{
    let writable $file = File::openRead("input.txt");
    writable Bytes $buffer = Bytes::allocate(1);
    {close}
    let writable $destination = $buffer->getWritableRegion(0, 1);
    /* 😀 */ $file->read($destination);
}}
"#
        );
        assert_lsp_diagnostic_matches_compiler("consumed-readable-file.doria", &source, "E0470");
    }
}

#[test]
fn writable_and_duplex_file_reuse_after_close_preserves_compiler_move_diagnostics() {
    for factory in ["openWrite", "openReadWrite"] {
        for close in [
            "$file->close();",
            "try { $file->close(); } catch (IoError) {}",
        ] {
            let source = format!(
                r#"use Doria\Std\Io\{{File, FileCreation, FileWriteMode, IoError}};
function inspect(): void {{
    let writable $file = File::{factory}("input.txt", FileCreation::OpenExisting, FileWriteMode::Positioned);
    let $bytes = Bytes::allocate(1);
    let $source = $bytes->getRegion(0, 1);
    {close}
    /* 😀 */ $file->write($source);
}}
"#
            );
            let baseline = source.replace(close, "");
            let diagnostics =
                diagnostics_for_document("file:///writable-file-baseline.doria", &baseline);
            assert!(diagnostics.is_empty(), "{baseline}\n{diagnostics:#?}");
            assert_lsp_diagnostic_matches_compiler(
                "consumed-writable-file.doria",
                &source,
                "E0470",
            );
        }
    }
}

#[test]
fn stream_access_and_progress_errors_preserve_compiler_labels() {
    let valid = r#"use Doria\Std\Io\{ReadableStream, ReadOutcome, WritableByteRegion, IoError};
function inspect(writable ReadableStream $stream, writable WritableByteRegion $destination): ReadOutcome throws IoError {
    /* 😀 */ return $stream->read($destination);
}
"#;
    assert!(diagnostics_for_document("file:///stream-access.doria", valid).is_empty());
    for (source, code) in [
        (
            valid.replace("writable ReadableStream", "ReadableStream"),
            "E0203",
        ),
        (
            valid.replace("writable WritableByteRegion", "WritableByteRegion"),
            "E0479",
        ),
        (valid.replace("($destination);", "(0);"), "E0408"),
    ] {
        assert_lsp_diagnostic_matches_compiler("stream-access.doria", &source, code);
    }
    let progress_write = "function inspect(writable Doria\\Std\\Io\\IoError $error): void { /* 😀 */ $error->transferredByteCount = 0; }";
    assert_lsp_diagnostic_matches_compiler("readonly-progress.doria", progress_write, "E0202");
}

#[test]
fn taking_interface_arguments_preserve_compiler_move_diagnostics() {
    let source = r#"interface Capability { function inspect(): void; }
class Payload implements Capability { function inspect(): void {} }
class Sink { function consume(take Capability $value): void {} }
function inspect(Capability $value): void { $value->inspect(); }
function main(): void {
    let $value = new Payload();
    let $sink = new Sink();
    $sink->consume($value);
    /* 😀 */ inspect($value);
}
"#;
    assert_lsp_diagnostic_matches_compiler("taken-interface.doria", source, "E0470");
}

#[test]
fn writable_indexed_interface_calls_have_no_false_diagnostics() {
    let source = r#"use Doria\Std\Io\{ReadableStream, WritableByteRegion, ReadOutcome};
class Reader implements ReadableStream {
    writable function read(writable WritableByteRegion $destination): ReadOutcome {
        $destination[0] = 7;
        return ReadOutcome::Data(1);
    }
}
class Registry {
    internal writable List<ReadableStream> $streams = [new Reader()];
    writable function read(writable WritableByteRegion $destination): ReadOutcome {
        return $this->streams[0]->read($destination);
    }
}
function main(): void {
    let writable $registry = new Registry();
    writable Bytes $buffer = Bytes::allocate(1);
    {
        let writable $destination = $buffer->getWritableRegion(0, 1);
        $registry->read($destination);
    }
    echo $buffer[0];
}
"#;
    let diagnostics = diagnostics_for_document("file:///indexed-interface.doria", source);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");

    for readonly in [
        source.replace("internal writable", "internal"),
        source.replace(
            "writable function read(writable WritableByteRegion $destination): ReadOutcome {\n        return",
            "function read(writable WritableByteRegion $destination): ReadOutcome {\n        return",
        ),
    ] {
        assert_lsp_diagnostic_matches_compiler("readonly-indexed-interface.doria", &readonly, "E0203");
    }
}

#[test]
fn indexed_interface_arguments_preserve_compiler_access_diagnostics() {
    for (ty, values, index) in [
        ("List<ReadableStream>", "[new Reader()]", "0"),
        ("ReadableStream[]", "[new Reader()]", "0"),
        (
            "Dictionary<string, ReadableStream>",
            "[\"reader\" => new Reader()]",
            "\"reader\"",
        ),
        ("List<List<ReadableStream>>", "[[new Reader()]]", "0][0"),
    ] {
        for (callable, code) in [("readStream", "E0479"), ("$read", "E0651")] {
            let source = format!(
                r#"use Doria\Std\Io\{{ReadableStream, WritableByteRegion, ReadOutcome}};
class Reader implements ReadableStream {{
    writable function read(writable WritableByteRegion $destination): ReadOutcome {{
        $destination[0] = 7;
        return ReadOutcome::Data(1);
    }}
}}
function readStream(writable ReadableStream $stream, writable WritableByteRegion $destination): ReadOutcome {{
    return $stream->read($destination);
}}
class Registry {{
    internal writable {ty} $streams = {values};
    writable function read(writable WritableByteRegion $destination): ReadOutcome {{
        let $read = function (writable ReadableStream $stream, writable WritableByteRegion $destination): ReadOutcome {{
            return $stream->read($destination);
        }};
        /* 😀 */ return {callable}($this->streams[{index}], $destination);
    }}
}}
function main(): void {{
    let writable $registry = new Registry();
    writable Bytes $buffer = Bytes::allocate(1);
    {{
        let writable $destination = $buffer->getWritableRegion(0, 1);
        $registry->read($destination);
    }}
}}
"#
            );
            let diagnostics =
                diagnostics_for_document("file:///indexed-interface-arguments.doria", &source);
            assert!(diagnostics.is_empty(), "{ty}, {callable}: {diagnostics:#?}");

            for readonly in [
                source.replace("internal writable", "internal"),
                source.replace(
                    "writable function read(writable WritableByteRegion $destination): ReadOutcome {\n        let",
                    "function read(writable WritableByteRegion $destination): ReadOutcome {\n        let",
                ),
            ] {
                assert_lsp_diagnostic_matches_compiler("readonly-indexed-argument.doria", &readonly, code);
            }
        }
    }
}

#[test]
fn generic_owned_property_writes_have_no_false_diagnostics() {
    for (ty, first, replacement, read) in [
        ("Item", "new Item(1)", "new Item(2)", "$holder->value->id"),
        (
            "List<Item>",
            "[new Item(1)]",
            "[new Item(2)]",
            "$holder->value[0]->id",
        ),
        ("int", "1", "2", "$holder->value"),
    ] {
        let source = format!(
            r#"
class Item {{
    function __construct(int $id) {{}}
    function __destruct() {{ try {{ echo "drop"; }} catch (Error) {{}} }}
}}
class Holder<T> {{
    writable T $value;
    function __construct(parameter take T $value) {{ $this->value = $value; }}
    writable function replace(take T $value): void {{ $this->value = $value; }}
}}
function main(): void {{
    let writable $holder = new Holder<{ty}>({first});
    let $replacement = {replacement};
    $holder->replace($replacement);
    $holder->value = {first};
    int $result = {read};
}}
"#
        );
        let name = "generic-owned-property.doria";
        doriac::check_source(name, &source)
            .unwrap_or_else(|diagnostics| panic!("{ty}: {diagnostics:#?}"));
        let diagnostics = diagnostics_for_document(&format!("file:///{name}"), &source);
        assert!(diagnostics.is_empty(), "{ty}: {diagnostics:#?}");
    }
}

#[test]
fn hook_replacement_cleanup_preserves_compiler_diagnostics() {
    for (declaration, parameter, constructor) in [
        ("Store", "Item", "new Store()"),
        ("Store<T>", "T", "new Store<Item>()"),
    ] {
        for (hook, operation) in [
            (
                "writable borrowed get { $this->item = null; return $this->item; }".to_owned(),
                "$store->item;",
            ),
            (
                format!("borrowed get => $this->item; set (take ?{parameter} $next) => $this->item = $next;"),
                "$store->item = new Item();",
            ),
        ] {
            for cleanup in ["", "try { echo \"drop\"; } catch (Error) {}"] {
                let source = format!(
                    r#"// 😀 keeps diagnostic positions UTF-16 safe.
class Item {{ function __destruct() {{ {cleanup} }} }}
class {declaration} {{ writable ?{parameter} $item = null {{ {hook} }} }}
function main(): void {{ let writable $store = {constructor}; {operation} }}
"#
                );
                let name = "hook-replacement-cleanup.doria";
                let (_, analysis) = doriac::analyze_source_for_ide(name, &source)
                    .expect("hook cleanup fixture must parse");
                let diagnostics = diagnostics_for_document(&format!("file:///{name}"), &source);
                if cleanup.is_empty() {
                    assert!(analysis.diagnostics.is_empty(), "{source}\n{:?}", analysis.diagnostics);
                    assert!(diagnostics.is_empty(), "{source}\n{diagnostics:#?}");
                } else {
                    assert!(!analysis.diagnostics.is_empty(), "{source}");
                    assert!(analysis.diagnostics.iter().all(|diagnostic| diagnostic.code == "E0770"), "{source}\n{:?}", analysis.diagnostics);
                    assert!(!diagnostics.is_empty(), "{source}");
                    assert!(diagnostics.iter().all(|diagnostic| diagnostic["code"] == "E0770"), "{source}\n{diagnostics:#?}");
                    assert_lsp_diagnostic_matches_compiler(name, &source, "E0770");
                    assert!(code_actions_for_document(&format!("file:///{name}"), &source)
                        .iter().all(|action| action["edit"].is_null()));
                }
            }
        }
    }
}

#[test]
fn nested_concrete_stream_adapters_have_no_false_diagnostics() {
    let source = r#"use Doria\Std\Io\{File, FileCreation, FileWriteMode, WritableFile, ByteRegion, WritableStream, WriteOutcome, IoError};
class MeteredWriter<S implements WritableStream> implements WritableStream {
    internal writable S $inner;
    writable int $calls = 0;
    function __construct(parameter take S $inner) { $this->inner = $inner; }
    writable function write(ByteRegion $source): WriteOutcome throws IoError {
        $this->calls++;
        return $this->inner->write($source);
    }
}
function writeConcrete<S implements WritableStream>(writable S $stream, ByteRegion $source): WriteOutcome throws IoError {
    return $stream->write($source);
}
function writeErased(writable WritableStream $stream, ByteRegion $source): WriteOutcome throws IoError {
    return $stream->write($source);
}
function main(): void {
    writable Bytes $buffer = Bytes::allocate(4);
    let $payload = $buffer->getRegion(0, 4);
    let writable $writer = new MeteredWriter<MeteredWriter<WritableFile>>(new MeteredWriter<WritableFile>(
        File::openWrite("nested.bin", FileCreation::CreateNew, FileWriteMode::Positioned)
    ));
    writeConcrete($writer, $payload);
    writeErased($writer, $payload);
}
"#;
    let name = "nested-concrete-stream-adapters.doria";
    doriac::check_source(name, source).unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
    let diagnostics = diagnostics_for_document(&format!("file:///{name}"), source);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn owned_stream_temporary_receivers_have_no_false_diagnostics() {
    let source = r#"use Doria\Std\Io\{ReadableStream, WritableByteRegion, ReadOutcome};
class Reader implements ReadableStream {
    writable function read(writable WritableByteRegion $destination): ReadOutcome {
        $destination[0] = 9;
        return ReadOutcome::Data(1);
    }
}
interface ReaderFactory { function makeReader(): ReadableStream; }
class Factory implements ReaderFactory {
    function makeReader(): ReadableStream { return new Reader(); }
    static function makeStaticReader(): ReadableStream { return new Reader(); }
}
class Holder { writable ReadableStream $reader = new Reader(); }
function makeReader(): ReadableStream { return new Reader(); }
function readFromFactory(ReaderFactory $factory, writable WritableByteRegion $destination): void {
    $factory->makeReader()->read($destination);
}
function main(): void {
    writable Bytes $buffer = Bytes::allocate(1);
    let $factory = new Factory();
    let $make = function (): ReadableStream { return new Reader(); };
    {
        let writable $destination = $buffer->getWritableRegion(0, 1);
        makeReader()->read($destination);
        Factory::makeStaticReader()->read($destination);
        $factory->makeReader()->read($destination);
        $make()->read($destination);
        readFromFactory($factory, $destination);
        (new Holder())->reader->read($destination);
        writable List<Holder> $holders = [new Holder()];
        $holders[0]->reader->read($destination);
    }
}
"#;
    let diagnostics = diagnostics_for_document("file:///owned-stream-temporaries.doria", source);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn readonly_stream_results_preserve_compiler_receiver_diagnostics() {
    for receiver in ["borrowReader($reader)", "(new Holder())->reader"] {
        let source = format!(
            r#"use Doria\Std\Io\{{ReadableStream, WritableByteRegion, ReadOutcome}};
class Reader implements ReadableStream {{
    writable function read(writable WritableByteRegion $destination): ReadOutcome {{
        return ReadOutcome::Data(0);
    }}
}}
class Holder {{ ReadableStream $reader = new Reader(); }}
function borrowReader(ReadableStream $reader): ReadableStream {{ return $reader; }}
function main(): void {{
    let writable $reader = new Reader();
    writable Bytes $buffer = Bytes::allocate(1);
    let writable $destination = $buffer->getWritableRegion(0, 1);
    /* 😀 */ {receiver}->read($destination);
}}
"#
        );
        assert_lsp_diagnostic_matches_compiler("readonly-stream-result.doria", &source, "E0203");
    }
}

#[test]
fn owned_field_borrows_accept_disjoint_stored_paths() {
    let declarations = r#"
class Item { writable int $value = 0; }
class Pair { writable Item $left = new Item(); writable Item $right = new Item(); }
class Parent { writable Pair $first = new Pair(); writable Pair $second = new Pair(); }
function touch(writable Item $left, writable Item $right): void {}
function observe(Item $left, writable Item $right): void {}
function identity(Item $item): Item { return $item; }
"#;
    let declarations = format!("{declarations}\n{INHERITED_OWNED_FIELDS}");
    for body in [
        "touch($pair->left, $pair->right);",
        "touch($parent->first->left, $parent->first->right);",
        "touch($parent->first->left, $parent->second->left);",
        "let $alias = identity($pair->left); observe($alias, $pair->right); echo $alias->value;",
        "let $view = identity($parent->first->left); $parent->first->right->value = 1; echo $view->value;",
        "touch($inherited->left, $inherited->right);",
    ] {
        let source = format!("{declarations} function inspect(writable Pair $pair, writable Parent $parent, writable InheritedPair $inherited): void {{ {body} }}");
        doriac::check_source("owned-field-paths.doria", &source)
            .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
        let diagnostics = diagnostics_for_document("file:///owned-field-paths.doria", &source);
        assert!(diagnostics.is_empty(), "{source}\n{diagnostics:#?}");
    }
}

#[test]
fn owned_field_conflicts_preserve_compiler_diagnostics() {
    let declarations = r#"
class Item { writable int $value = 0; }
class Pair {
    writable Item $left = new Item();
    writable Item $right = new Item();
    Item $view { borrowed get => $this->left; }
}
class Parent { writable Pair $first = new Pair(); writable Pair $second = new Pair(); }
function touch(writable Item $left, writable Item $right): void {}
function inspect(Item $left, writable Item $right): void {}
function identity(Item $item): Item { return $item; }
function inspectParent(writable Pair $parent, Item $child): void {}
"#;
    let declarations = format!("{declarations}\n{INHERITED_OWNED_FIELDS}");
    let name = "owned-field-conflicts.doria";
    for body in [
        "touch($pair->left, $pair->left);",
        "let $alias = identity($pair->left); inspect($alias, $pair->left); echo $alias->value;",
        "touch($parent->first->left, $parent->first->left);",
        "inspectParent($pair, $pair->left);",
        "inspect($pair->view, $pair->right);",
        "writable List<Item> $items = [new Item(), new Item()]; touch($items[0], $items[1]);",
        "touch($inherited->left, $inherited->left);",
    ] {
        let source = format!("{declarations} function route(writable Pair $pair, writable Parent $parent, writable InheritedPair $inherited): void {{ /* \u{1f600} */ {body} }}");
        doriac::parse_source(name, &source).expect("ownership fixture must parse");
        assert_lsp_diagnostic_matches_compiler(name, &source, "E0477");
        let actions = code_actions_for_document(&format!("file:///{name}"), &source);
        assert!(
            actions.iter().all(|action| action["edit"].is_null()),
            "{source}\n{actions:#?}"
        );
    }
}

const INHERITED_OWNED_FIELDS: &str = r#"
open class Base<T> {
    writable T $left;
    T $fixed;
    function __construct(parameter take T $left, parameter take T $fixed) {
        $this->left = $left;
        $this->fixed = $fixed;
    }
}
class InheritedPair extends Base<Item> {
    writable Item $right = new Item();
    function __construct() { parent::__construct(new Item(), new Item()); }
}
"#;

#[test]
fn owned_field_borrows_preserve_readonly_inherited_access() {
    let name = "readonly-inherited-fields.doria";
    for (mode, selected) in [("", "left"), ("writable ", "fixed")] {
        let source = format!(
            "class Item {{}}\n{INHERITED_OWNED_FIELDS}\n\
             function touch(writable Item $left, writable Item $right): void {{}}\n\
             function route({mode}InheritedPair $owner): void {{ /* \u{1f600} */ touch($owner->{selected}, $owner->right); }}"
        );
        doriac::parse_source(name, &source).expect("inherited field fixture must parse");
        assert_lsp_diagnostic_matches_compiler(name, &source, "E0204");
    }
}

#[test]
fn owned_field_region_loans_protect_selected_storage_and_ancestors() {
    let declarations = r#"
class Buffers {
    writable Bytes $left = Bytes::allocate(2);
    writable Bytes $right = Bytes::allocate(2);
}
class Parent { writable Buffers $buffers = new Buffers(); }
function consume(take Parent $parent): void {}
"#;
    let name = "owned-field-storage-loan.doria";
    for operation in [
        "$parent->buffers->left[0] = 1;",
        "$parent->buffers->left = Bytes::allocate(2);",
        "$parent->buffers = new Buffers();",
        "$parent = new Parent();",
        "consume($parent);",
    ] {
        let source = format!("{declarations} function main(): void {{ let writable $parent = new Parent(); let $region = $parent->buffers->left->getRegion(0, 1); /* \u{1f600} */ {operation} echo $region[0]; }}");
        doriac::parse_source(name, &source).expect("ownership fixture must parse");
        assert_lsp_diagnostic_matches_compiler(name, &source, "E0763");
        let actions = code_actions_for_document(&format!("file:///{name}"), &source);
        assert!(
            actions.iter().all(|action| action["edit"].is_null()),
            "{source}\n{actions:#?}"
        );
    }
}

#[test]
fn byte_region_borrow_diagnostics_preserve_compiler_ranges_and_no_ownership_fix() {
    let source = r#"function inspect(): void {
    writable Bytes $bytes = Bytes::allocate(length: 4);
    let $region = $bytes->getRegion(offset: 0, length: 4);
    echo "😀"; $bytes[0] = 1;
    echo $region[0];
}
"#;
    assert_lsp_diagnostic_matches_compiler("byte-loan.doria", source, "E0763");
    assert!(code_actions_for_document("file:///byte-loan.doria", source)
        .iter()
        .all(|action| action["edit"].is_null()));
    let escape = r#"use Doria\Std\Io\ByteRegion;
function escape(): ByteRegion {
    let $bytes = Bytes::allocate(1);
    return $bytes->getRegion(0, 1);
}
"#;
    assert_lsp_diagnostic_matches_compiler("byte-escape.doria", escape, "E0762");
}

#[test]
fn readiness_source_adapter_forwarding_has_no_false_diagnostics() {
    doriac::check_source("readiness-source.doria", READINESS_SOURCES)
        .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
    let diagnostics = diagnostics_for_document("file:///readiness-source.doria", READINESS_SOURCES);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn readiness_source_ownership_rejections_preserve_compiler_diagnostics() {
    let name = "invalid-readiness-source.doria";
    let uri = format!("file:///{name}");
    for body in [
        "function main(): void { /* 😀 */ let $source = new ReadinessSource(); }",
        "function escape(): ReadinessSource { let $file = File::openRead(\"ready.txt\"); /* 😀 */ return $file->readinessSource; }",
        "function consume(take ReadinessSource $source): void {} function main(): void { let $file = File::openRead(\"ready.txt\"); /* 😀 */ consume($file->readinessSource); }",
        "function inspect(ReadinessSource $source): void {} function main(): void { let $file = File::openRead(\"ready.txt\"); let $source = $file->readinessSource; /* 😀 */ $file->close(); inspect($source); }",
    ] {
        let source = format!("use Doria\\Std\\Io\\{{File, ReadinessSource}}; {body}");
        let diagnostics = doriac::check_source(name, &source)
            .expect_err("opaque readiness source must preserve its owner loan");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error),
            "{diagnostics:#?}"
        );
        for diagnostic in &diagnostics {
            assert_lsp_diagnostic_matches_compiler(name, &source, diagnostic.code);
        }
        assert!(code_actions_for_document(&uri, &source)
            .iter()
            .all(|action| action["edit"].is_null()));
    }
}

#[test]
fn pending_adapter_forwarding_no_longer_reports_removed_execution_boundary() {
    doriac::check_source("pending-adapters.doria", PENDING_ADAPTERS).unwrap();
    assert_stored_source_diagnostics("pending-adapters.doria", PENDING_ADAPTERS, None);
}

#[test]
fn pending_selected_storage_accepts_group_regions_and_adapter_owned_buffers() {
    for (phase, argument, accessor) in [
        ("Read", "$destination", "getWritableRegion"),
        ("Write", "$source", "getRegion"),
    ] {
        for selected in [
            argument.to_owned(),
            format!("$this->buffer->{accessor}(0, 4)"),
        ] {
            let preparation =
                format!("$this->source->prepare{phase}($request, {selected}); $this->count += 1;");
            let source = pending_selected_storage_source(phase, argument, &preparation);
            let name = "safe-pending-storage.doria";
            doriac::check_source(name, &source).unwrap();
            assert_stored_source_diagnostics(name, &source, None);
        }
    }
}

fn pending_selected_storage_source(phase: &str, argument: &str, preparation: &str) -> String {
    let source = PENDING_ADAPTERS
        .replace(
            "internal writable PendingReadableStream $source;",
            "internal writable ReadableFile $source;",
        )
        .replace(
            "parameter take PendingReadableStream $source",
            "parameter take ReadableFile $source",
        )
        .replace(
            "internal writable PendingWritableStream $source;",
            "internal writable WritableFile $source;",
        )
        .replace(
            "parameter take PendingWritableStream $source",
            "parameter take WritableFile $source",
        );
    let source = source.replace(
        "    function __construct(",
        "    internal writable Bytes $buffer = Bytes::allocate(8);\n    internal writable int $count = 0;\n    function __construct(",
    );
    let original = format!("$this->source->prepare{phase}($request, {argument});");
    assert!(source.contains(&original));
    source.replace(&original, preparation)
}

#[test]
fn pending_selected_storage_rejections_preserve_compiler_diagnostics() {
    let name = "unsafe-pending-storage.doria";
    let uri = format!("file:///{name}");
    for (phase, argument, accessor) in [
        ("Read", "$destination", "getWritableRegion"),
        ("Write", "$source", "getRegion"),
    ] {
        let selection =
            format!("$this->source->prepare{phase}($request, $this->buffer->{accessor}(0, 4));");
        for (preparation, code) in [
            (format!("writable Bytes $local = Bytes::allocate(4); /* \u{1f600} */ $this->source->prepare{phase}($request, $local->{accessor}(0, 4));"), "E0773"),
            (format!("{selection} /* \u{1f600} */ $this->buffer = Bytes::allocate(8);"), "E0774"),
            (format!("if ($this->count == 0) {{ {selection} }} $this->buffer = Bytes::allocate(8);"), "E0774"),
            (format!("try {{ {selection} }} catch (IoError) {{}} $this->buffer = Bytes::allocate(8);"), "E0774"),
            (format!("{selection} $request->complete({phase}Outcome::WouldBlock); $this->buffer = Bytes::allocate(8);"), "E0774"),
            (format!("{{ let $selected = $this->buffer->{accessor}(0, 4); $this->source->prepare{phase}($request, $selected); }} $this->buffer = Bytes::allocate(8);"), "E0774"),
        ] {
            let source = pending_selected_storage_source(phase, argument, &preparation);
            assert_lsp_diagnostic_matches_compiler(name, &source, code);
            let actions = code_actions_for_document(&uri, &source);
            assert!(actions.iter().all(|action| action["edit"].is_null()), "{source}\n{actions:#?}");
        }
    }
}

#[test]
fn pending_request_return_loans_and_local_callbacks_stay_call_scoped() {
    for request in ["ReadRequest", "WriteRequest"] {
        let source = format!(
            r#"use Doria\Std\Io\{request};
function identity({request} $request): {request} {{ return $request; }}
function identityGeneric<T>(T $value): T {{ return $value; }}
function inspect({request} $request): void {{}}
function prepare(writable {request} $request): void {{
    let $alias = identity($request);
    let $generic = identityGeneric($request);
    inspect($alias);
    inspect($generic);
    let $callback = function(): void with ($alias) {{ inspect($alias); }};
    $callback();
}}
"#
        );
        doriac::check_source("pending-request-loans.doria", &source)
            .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
        let diagnostics = diagnostics_for_document("file:///pending-request-loans.doria", &source);
        assert!(diagnostics.is_empty(), "{source}\n{diagnostics:#?}");
    }
}

#[test]
fn pending_request_escape_rejections_preserve_compiler_diagnostics() {
    let name = "pending-request-escape.doria";
    for request in ["ReadRequest", "WriteRequest"] {
        let prefix = format!("use Doria\\Std\\Io\\{request};\n");
        let valid = format!("{prefix}function inspect({request} $request): void {{}}");
        doriac::check_source(name, &valid).unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
        for (body, code) in [
            (format!("function inspect(): void {{ /* 😀 */ let $request = new {request}(); }}"), "E0307"),
            (format!("function consume(take {request} $request): void {{}}"), "E0772"),
            (format!("interface Factory {{ /* 😀 */ function create(): {request}; }}"), "E0772"),
            (format!("class Holder {{ ?{request} $saved = null; writable function retain({request} $request): void {{ /* 😀 */ $this->saved = $request; }} }}"), "E0772"),
            (format!("function inspect({request} $request): void {{}} function capture({request} $request): function(): void {{ /* 😀 */ return function(): void with ($request) {{ inspect($request); }}; }}"), "E0772"),
            (format!("function inspect({request} $request): void {{}} function capture({request} $request): void {{ /* 😀 */ let $callback = function(): void with (take $request) {{ inspect($request); }}; }}"), "E0645"),
            (format!("function collect({request} $request): void {{ /* 😀 */ List<{request}> $requests = [$request]; }}"), "E0772"),
            (format!("function erase({request} $request): mixed {{ /* 😀 */ return $request; }}"), "E0772"),
        ] {
            let source = format!("{prefix}{body}");
            let diagnostics = doriac::check_source(name, &source)
                .expect_err("opaque group-slot requests must not gain independent ownership or escape");
            assert!(diagnostics.iter().any(|diagnostic| diagnostic.code == code && diagnostic.severity == DiagnosticSeverity::Error), "{source}\n{diagnostics:#?}");
            for diagnostic in &diagnostics {
                assert_lsp_diagnostic_matches_compiler(name, &source, diagnostic.code);
            }
            let actions = code_actions_for_document(&format!("file:///{name}"), &source);
            assert!(actions.iter().all(|action| action["edit"].is_null()), "{source}\n{actions:#?}");
        }
    }
}

#[test]
fn pending_adapter_nonblocking_phases_preserve_compiler_diagnostics() {
    for (original, blocking) in [
        ("$this->source->prepareRead($request, $destination);", "/* 😀 */ $this->source->read($destination);"),
        ("$this->source->prepareWrite($request, $source);", "/* 😀 */ $this->source->write($source);"),
        ("return $this->source->completeRead($request, $destination);", "/* 😀 */ echo \"blocking\"; return $this->source->completeRead($request, $destination);"),
        ("return $this->source->completeWrite($request, $source);", "/* 😀 */ echo \"blocking\"; return $this->source->completeWrite($request, $source);"),
    ] {
        let name = "blocking-pending-adapter.doria";
        let source = PENDING_ADAPTERS.replace(original, blocking);
        assert_ne!(source, PENDING_ADAPTERS);
        let diagnostics = doriac::check_source(name, &source)
            .expect_err("pending preparation and completion must remain nonblocking");
        assert!(diagnostics.iter().any(|diagnostic| diagnostic.code == "E0771"), "{diagnostics:#?}");
        for diagnostic in &diagnostics {
            assert_lsp_diagnostic_matches_compiler(name, &source, diagnostic.code);
        }
    }
}

#[test]
fn nullable_byte_member_errors_remain_compiler_owned() {
    for ty in [
        "Bytes",
        "Doria\\Std\\Io\\ByteRegion",
        "Doria\\Std\\Io\\WritableByteRegion",
    ] {
        for member in ["length", "getRegion(0, 0)"] {
            let source = format!("function inspect(?{ty} $value): void {{ echo \"😀\"; let $result = $value->{member}; }}");
            assert_lsp_diagnostic_matches_compiler("nullable-byte-member.doria", &source, "E0506");
            let fixed = source.replace("$value->", "$value?->");
            let diagnostics =
                diagnostics_for_document("file:///nullable-byte-member.doria", &fixed);
            assert!(diagnostics.is_empty(), "{ty}::{member}: {diagnostics:#?}");
        }
    }
    for ty in ["Bytes", "Doria\\Std\\Io\\WritableByteRegion"] {
        let source = format!("function inspect(?{ty} $value): void {{ let $part = $value?->getWritableRegion(0, 0); }}");
        assert_lsp_diagnostic_matches_compiler("readonly-byte-member.doria", &source, "E0201");
    }
    for (source, expected) in [
        (
            "function inspect(?int $value): void { let $part = $value?->length; }",
            "property access requires a value with members",
        ),
        (
            "function inspect(Bytes $value): void { let $part = $value?->length; }",
            "null-safe access requires a nullable receiver with members",
        ),
    ] {
        assert_lsp_diagnostic_matches_compiler("invalid-null-safe.doria", source, "E0507");
        let diagnostics = diagnostics_for_document("file:///invalid-null-safe.doria", source);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic["code"] == "E0507"
                    && diagnostic["message"]
                        .as_str()
                        .is_some_and(|message| message.contains(expected))),
            "{diagnostics:#?}"
        );
    }
}

#[test]
fn ambiguous_byte_region_returns_preserve_compiler_diagnostics() {
    for (parameter, result, access, method) in [
        ("Bytes", "ByteRegion", "->", "getRegion"),
        (
            "writable Bytes",
            "WritableByteRegion",
            "->",
            "getWritableRegion",
        ),
        ("?ByteRegion", "?ByteRegion", "?->", "getRegion"),
        (
            "writable ?WritableByteRegion",
            "?WritableByteRegion",
            "?->",
            "getWritableRegion",
        ),
    ] {
        let left = format!("$left{access}{method}(0, 1)");
        let right = format!("$right{access}{method}(1, 1)");
        for body in [
            format!("if ($first) {{ return {left}; }} return {right};"),
            format!("{result} $view = when ($first) {{ return {left}; }} else {{ return {right}; }}; return $view;"),
        ] {
            let source = format!(r#"use Doria\Std\Io\ByteRegion;
use Doria\Std\Io\WritableByteRegion;
function choose(bool $first, {parameter} $left, {parameter} $right): {result} {{
    echo "😀"; {body}
}}
"#);
            assert_lsp_diagnostic_matches_compiler("ambiguous-byte-return.doria", &source, "E0478");
            assert!(code_actions_for_document("file:///ambiguous-byte-return.doria", &source)
                .iter().all(|action| action["edit"].is_null()));
            let fixed = source.replace(&format!("$right{access}"), &format!("$left{access}"));
            let diagnostics = diagnostics_for_document("file:///ambiguous-byte-return.doria", &fixed);
            assert!(diagnostics.is_empty(), "{fixed}\n{diagnostics:#?}");
        }
    }
    let local_union = r#"use Doria\Std\Io\ByteRegion;
function localJoin(bool $first, Bytes $left, Bytes $right): uint8 {
    ByteRegion $view = when ($first) { return $left->getRegion(0, 1); }
        else { return $right->getRegion(0, 1); };
    return $view[0];
}
"#;
    let diagnostics = diagnostics_for_document("file:///local-byte-union.doria", local_union);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn consumed_receiver_byte_views_publish_compiler_escape_diagnostics() {
    for (result, body, code) in [
        ("ByteRegion", "return $this->bytes->getRegion(0, 1);", "E0762"),
        ("?ByteRegion", "return $this->optional?->getRegion(0, 1);", "E0762"),
        ("function(): int", "let $view = $this->bytes->getRegion(0, 1); return fn() with (take $view) => $view->length;", "E0658"),
    ] {
        let source = format!(r#"use Doria\Std\Io\ByteRegion;
class Packet {{
    Bytes $bytes = Bytes::allocate(2);
    ?Bytes $optional = Bytes::allocate(2);
    consuming function view(): {result} {{ {body} }}
}}
"#);
        assert_lsp_diagnostic_matches_compiler("consumed-byte-view.doria", &source, code);
        assert!(code_actions_for_document("file:///consumed-byte-view.doria", &source)
            .iter().all(|action| action["edit"].is_null()));
    }
    let borrowed_argument = r#"use Doria\Std\Io\ByteRegion;
class Packet {
    consuming function view(Bytes $source): ByteRegion { return $source->getRegion(0, 1); }
}
"#;
    let diagnostics =
        diagnostics_for_document("file:///borrowed-byte-source.doria", borrowed_argument);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn byte_region_last_use_releases_storage_without_losing_live_aliases() {
    let source = r#"use Doria\Std\Io\ByteRegion;
function inspect(): void {
    writable Bytes $bytes = Bytes::allocate(2);
    let $view = $bytes->getRegion(0, 1);
    echo $view[0];
    $bytes[0] = 7;
    let writable $write = $bytes->getWritableRegion(0, 2);
    let writable $part = $write->getWritableRegion(0, 1);
    $part[0] = 2;
    $write[1] = 3;
    $bytes[0] = 4;
    ?ByteRegion $optional = $bytes->getRegion(0, 1);
    echo $optional?->length ?? 0;
    $bytes[0] = 5;
}
"#;
    let diagnostics = diagnostics_for_document("file:///last-byte-use.doria", source);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    for (body, code) in [
        ("let $view = $bytes->getRegion(0, 1); let $alias = $view; echo $view[0]; $bytes[0] = 7; echo $alias[0];", "E0763"),
        ("let $view = $bytes->getRegion(0, 1); let $read = fn() with (take $view) => $view[0]; $bytes[0] = 7; echo $read();", "E0654"),
    ] {
        let source = format!("function inspect(): void {{ writable Bytes $bytes = Bytes::allocate(2); {body} }}");
        assert_lsp_diagnostic_matches_compiler("live-byte-alias.doria", &source, code);
    }
}

#[test]
fn runtime_io_outcomes_are_not_published_as_live_diagnostics() {
    let diagnostics = diagnostics_for_document(
        "file:///runtime-only-io.doria",
        r#"function main(): void
{
    echo "value";
}
"#,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    assert!(diagnostics
        .iter()
        .all(|diagnostic| diagnostic["code"] != "R1000"));
}

#[test]
fn publishes_the_compiler_diagnostic_for_a_non_string_prompt() {
    let diagnostics = diagnostics_for_document(
        "file:///input.doria",
        "function main(): void { let $line = read_line(1); }",
    );
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == "E0453")
        .expect("the compiler prompt-type diagnostic should be published");
    assert!(diagnostic["message"]
        .as_str()
        .expect("diagnostic message")
        .contains("expects `string`"));
}

#[test]
fn io_diagnostic_ranges_remain_utf16_safe_after_non_ascii_text() {
    let source = "function main(): void { let $emoji = \"😀\"; let $line = read_line(1); }";
    let diagnostics = diagnostics_for_document("file:///input-utf16.doria", source);
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == "E0453")
        .unwrap_or_else(|| panic!("missing prompt-type diagnostic: {diagnostics:#?}"));
    let argument = source.find("read_line(1)").expect("read_line call") + "read_line(".len();
    let expected = byte_offset_to_position(source, argument);
    assert_eq!(diagnostic["range"]["start"]["line"], expected.line);
    assert_eq!(
        diagnostic["range"]["start"]["character"], expected.character,
        "compiler byte spans must be converted to UTF-16 positions"
    );
}

#[test]
fn preserves_php_readline_migration_guidance() {
    let diagnostics = diagnostics_for_document(
        "file:///input.doria",
        "function main(): void { let $line = readline(); }",
    );
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == "E0461")
        .expect("the PHP spelling diagnostic should be published");
    assert!(diagnostic["message"]
        .as_str()
        .expect("diagnostic message")
        .contains("read_line"));
}

#[test]
fn exposes_literal_brace_fix_data_at_original_source_span() {
    let text = "echo \"literal {word}\";";
    let diagnostics = diagnostics_for_document("file:///brace.doria", text);
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == "P0002")
        .expect("literal brace diagnostic should be published");

    assert_eq!(diagnostic["data"]["fix"]["newText"], "\\{");
    assert_eq!(diagnostic["data"]["fix"]["range"]["start"]["line"], 0);
    assert_eq!(
        diagnostic["data"]["fix"]["range"]["start"]["character"],
        text.find('{').expect("opening brace")
    );
}

#[test]
fn exposes_literal_brace_fix_as_a_preferred_code_action() {
    let uri = "file:///brace.doria";
    let text = "echo \"literal {word}\";";
    let actions = code_actions_for_document(uri, text);

    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["kind"], "quickfix");
    assert_eq!(actions[0]["isPreferred"], true);
    assert_eq!(actions[0]["edit"]["changes"][uri][0]["newText"], "\\{");
    assert_eq!(
        actions[0]["edit"]["changes"][uri][0]["range"]["start"]["character"],
        text.find('{').expect("opening brace")
    );
}

#[test]
fn exposes_writable_constructor_removal_as_a_preferred_code_action() {
    let uri = "file:///lifecycle.doria";
    let text = "class Person { writable function __construct() {} }";
    let actions = code_actions_for_document(uri, text);
    let action = actions
        .iter()
        .find(|action| {
            action["title"]
                .as_str()
                .is_some_and(|title| title.contains("Construction Grants `__construct`"))
        })
        .expect("writable lifecycle diagnostic should expose a quick fix");

    assert_eq!(action["kind"], "quickfix");
    assert_eq!(action["isPreferred"], true);
    assert_eq!(action["edit"]["changes"][uri][0]["newText"], "");
    assert_eq!(
        action["edit"]["changes"][uri][0]["range"]["start"]["character"],
        text.find("writable").expect("writable modifier")
    );
}

#[test]
fn exposes_collection_diagnostic_fixes_as_utf16_safe_code_actions() {
    let cases = [
        (
            "has",
            r#"function main(): void { echo "😀"; Dictionary<string, int> $values = []; echo $values->has("alpha"); }"#,
            "containsKey",
        ),
        (
            "isEmpty",
            r#"function main(): void { echo "😀"; List<int> $values = []; echo $values->isEmpty(); }"#,
            "",
        ),
        (
            "List::from",
            r#"function main(): void { echo "😀"; List<int> $values = List::from([1, 2]); }"#,
            "[1, 2]",
        ),
        (
            "Dictionary::from",
            r#"function main(): void { echo "😀"; Dictionary<string, int> $values = Dictionary::from(["alpha" => 1]); }"#,
            "[\"alpha\" => 1]",
        ),
    ];

    for (label, text, replacement) in cases {
        let uri = format!("file:///collection-{label}.doria");
        let actions = code_actions_for_document(&uri, text);
        assert_eq!(actions.len(), 1, "{label}: {actions:#?}");
        assert_eq!(actions[0]["kind"], "quickfix");
        assert_eq!(actions[0]["isPreferred"], true);

        let edits = actions[0]["edit"]["changes"][&uri]
            .as_array()
            .expect("quick fix edits");
        assert!(edits.iter().any(|edit| edit["newText"] == replacement));

        let first_edit = &edits[0];
        let byte_start = match label {
            "has" => text.find("has").unwrap(),
            "isEmpty" => text.find("isEmpty").unwrap() + "isEmpty".len(),
            "List::from" => text.find("List::from").unwrap(),
            "Dictionary::from" => text.rfind("Dictionary::from").unwrap(),
            _ => unreachable!(),
        };
        let expected = byte_offset_to_position(text, byte_start);
        assert_eq!(first_edit["range"]["start"]["line"], expected.line);
        assert_eq!(
            first_edit["range"]["start"]["character"], expected.character,
            "{label} must convert compiler byte offsets to UTF-16"
        );
    }
}

#[test]
fn accepts_the_complete_decision_0113_surface_after_non_ascii_text() {
    let source = r#"function main(): void
{
    echo "😀";
    writable List<int> $list = [1];
    let $position = $list->indexOf(1);
    $list->remove(1);
    $list->clear();
    writable Dictionary<string, int> $dictionary = ["one" => 1];
    echo $dictionary->containsValue(1);
    $dictionary->clear();
    writable Set<int> $set = Set::from([1]);
    let $first = $set->first;
    let $last = $set->last;
    $set->clear();
    writable SortedDictionary<int, int> $sortedDictionary = SortedDictionary::from([1 => 1]);
    echo $sortedDictionary->containsValue(1);
    $sortedDictionary->clear();
    writable SortedSet<int> $sortedSet = SortedSet::from([1]);
    let $sortedFirst = $sortedSet->first;
    let $sortedLast = $sortedSet->last;
    $sortedSet->clear();
    writable PriorityQueue<int> $queue = PriorityQueue::from([1]);
    $queue->clear();
    writable Deque<int> $deque = Deque::from([1]);
    $deque->clear();
}
"#;
    let diagnostics = diagnostics_for_document("file:///decision-0113.doria", source);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn preserves_the_compiler_readonly_clear_diagnostic_after_non_ascii_text() {
    let source =
        "function main(): void { let $emoji = \"😀\"; List<int> $values = [1]; $values->clear(); }";
    let diagnostics = diagnostics_for_document("file:///readonly-clear.doria", source);
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == "E0201")
        .unwrap_or_else(|| panic!("missing readonly clear diagnostic: {diagnostics:#?}"));
    let expected = byte_offset_to_position(source, source.find("$values->clear").unwrap());
    assert_eq!(diagnostic["range"]["start"]["line"], expected.line);
    assert_eq!(
        diagnostic["range"]["start"]["character"],
        expected.character
    );
}

#[test]
fn checked_error_execution_boundaries_remain_compiler_owned() {
    let handled = diagnostics_for_document(
        "file:///handled-errors.doria",
        r#"class Failure implements Error
{
    function __construct(string $message) {}
}

function fail(): void throws Failure
{
    throw new Failure("handled");
}

function main(): void
{
    try {
        fail();
    } catch (Failure $error) {
        echo $error->message;
    }
}
"#,
    );
    assert!(handled.is_empty(), "{handled:#?}");

    let escaping = diagnostics_for_document(
        "file:///escaping-error.doria",
        r#"class Failure implements Error
{
    function __construct(string $message) {}
}

function main(): void throws Failure
{
    throw new Failure("escaping");
}
"#,
    );
    assert!(
        escaping
            .iter()
            .all(|diagnostic| diagnostic["code"] != "B2902"),
        "document analysis must not fabricate the backend-only Slice 3 boundary: {escaping:#?}"
    );
}

#[test]
fn payload_enums_and_core_match_execution_come_from_the_compiler() {
    let accepted = diagnostics_for_document(
        "file:///enums.doria",
        r#"enum Status { case Draft; }
enum Priority: int { case High = 2; }
enum Transport: string { case Rail = "rail"; }
    function main(): void
{
    Status $status = Status::Draft;
    Priority $priority = Priority::High;
    Transport $transport = Transport::Rail;
    echo "{$status == Status::Draft}:{$priority->value}:{$transport->value}";
}
"#,
    );
    assert!(accepted.is_empty(), "{accepted:#?}");

    let payload = diagnostics_for_document(
        "file:///payload-enum.doria",
        "enum Shape { case Circle(float $radius); } Shape $shape = Shape::Circle(2.5);",
    );
    assert!(payload.is_empty(), "{payload:#?}");

    let matching = diagnostics_for_document(
        "file:///match.doria",
        "enum Status { case Draft; } let $label = match (Status::Draft) { Status::Draft => 1, };",
    );
    assert!(matching.is_empty(), "{matching:#?}");

    let missing = diagnostics_for_document(
        "file:///missing-match-case.doria",
        "enum Status { case Draft; case Published; } let $label = match (Status::Draft) { Status::Draft => 1, };",
    );
    assert!(missing
        .iter()
        .any(|diagnostic| diagnostic["code"] == "E0585"));
}

#[test]
fn match_payload_guard_and_ternary_semantics_remain_compiler_owned() {
    let payload = diagnostics_for_document(
        "file:///payload-pattern.doria",
        r#"enum Pair { case Values(int $left, int $right); }
function main(): void
{
    echo "😀";
    Pair $pair = Pair::Values(1, 2);
    int $value = match ($pair) { Pair::Values($left) => $left, };
}
"#,
    );
    assert!(payload
        .iter()
        .any(|diagnostic| diagnostic["code"] == "E0590"));

    let guard = diagnostics_for_document(
        "file:///match-guard.doria",
        "enum State { case Ready; } function f(State $state, bool $enabled): string { return match ($state) { State::Ready if $enabled => \"ready\", State::Ready => \"disabled\", }; }",
    );
    assert!(guard.is_empty(), "{guard:#?}");

    let non_bool_guard = diagnostics_for_document(
        "file:///match-guard-type.doria",
        "enum State { case Ready; } function f(State $state): string { return match ($state) { State::Ready if 1 => \"ready\", State::Ready => \"fallback\", }; }",
    );
    assert!(non_bool_guard
        .iter()
        .any(|diagnostic| diagnostic["code"] == "E0597"));

    let ternary_source =
        "function f(int $value): string { let $emoji = \"😀\"; return $value ? \"yes\" : \"no\"; }";
    let ternary = diagnostics_for_document("file:///ternary.doria", ternary_source);
    let diagnostic = ternary
        .iter()
        .find(|diagnostic| diagnostic["code"] == "E0595")
        .unwrap_or_else(|| panic!("missing strict ternary diagnostic: {ternary:#?}"));
    let expected = byte_offset_to_position(
        ternary_source,
        ternary_source.rfind("$value").expect("ternary condition"),
    );
    assert_eq!(diagnostic["range"]["start"]["line"], expected.line);
    assert_eq!(
        diagnostic["range"]["start"]["character"],
        expected.character
    );

    let elvis = diagnostics_for_document(
        "file:///elvis.doria",
        "function f(?string $value): string { return $value ?: \"fallback\"; }",
    );
    assert_eq!(elvis.len(), 1, "{elvis:#?}");
    let message = elvis[0]["message"].as_str().expect("Elvis diagnostic");
    assert!(message.contains("short ternary"));
    assert!(message.contains("`??`"));
    assert!(message.contains("full `? :`"));
}

#[test]
fn contextual_match_results_use_the_final_compiler_rules_in_every_call_form() {
    let diagnostics = diagnostics_for_document(
        "file:///contextual-match.doria",
        r#"class Sink
{
    mixed $property = match (1 == 2) { true => 1, false => "text", };

    function __construct(take mixed $value) {}
    function accept(mixed $value): void {}
    static function acceptStatic(mixed $value): void {}
}

function accept(mixed $value): void {}
function mixedResult(bool $condition): mixed
{
    return match ($condition) { true => 1, false => "text", };
}
function nullResult(bool $condition): ?string
{
    return match ($condition) { true => null, false => null, };
}

function main(): void
{
    bool $condition = true;
    mixed $local = match ($condition) { true => 1, false => "text", };
    writable mixed $assigned = 0;
    $assigned = match (false) { true => 1, false => "text", };
    accept(match (false) { true => 1, false => "text", });
    let $sink = new Sink(match (false) { true => 1, false => "text", });
    $sink->accept(match (false) { true => 1, false => "text", });
    Sink::acceptStatic(match (false) { true => 1, false => "text", });
    mixed $nested = match ($condition) {
        true => match (false) { true => 1, false => "nested", },
        false => false,
    };
    mixed $ternary = true ? match (false) { true => 1, false => "ternary", } : false;
    mixed $mixed = mixedResult(true);
    ?string $nullable = nullResult(false);
}"#,
    );

    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn enum_case_fixes_remain_machine_applicable_and_utf16_safe() {
    let uri = "file:///enum-fixes.doria";
    let source = r#"enum Status { case Draft; }
function main(): void { let $emoji = "😀"; Status $status = Status::Draft(); }
"#;
    let diagnostics = diagnostics_for_document(uri, source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0]["code"], "E0575");

    let actions = code_actions_for_document(uri, source);
    assert_eq!(actions.len(), 1, "{actions:#?}");
    let edit = &actions[0]["edit"]["changes"][uri][0];
    assert_eq!(edit["newText"], "");
    let parentheses = source.rfind("()").expect("unit-case parentheses");
    let expected = byte_offset_to_position(source, parentheses);
    assert_eq!(edit["range"]["start"]["line"], expected.line);
    assert_eq!(edit["range"]["start"]["character"], expected.character);

    let suggestion_uri = "file:///enum-suggestion.doria";
    let suggestion_source = "enum Status { case Draft; } Status $status = Status::Draf;";
    let suggestion = code_actions_for_document(suggestion_uri, suggestion_source);
    assert_eq!(suggestion.len(), 1, "{suggestion:#?}");
    assert_eq!(
        suggestion[0]["edit"]["changes"][suggestion_uri][0]["newText"],
        "Draft"
    );
}

#[test]
fn payload_enum_diagnostics_keep_compiler_codes_and_utf16_ranges() {
    let cases = [
        (
            "construction",
            r#"enum Shape { case Circle(float $radius); }
function main(): void { let $emoji = "😀"; Shape $shape = Shape::Circle("wide"); }"#,
            "E0408",
            "\"wide\"",
        ),
        (
            "named argument",
            r#"enum Coordinate { case Point(int $x, int $y); }
function main(): void { let $emoji = "😀"; let $point = Coordinate::Point(z: 1, y: 2); }"#,
            "E0516",
            "z:",
        ),
        (
            "recursive layout",
            r#"let $emoji = "😀"; enum Node { case Next(Node $next); }"#,
            "E0581",
            "Node $next",
        ),
        (
            "move",
            r#"class Document {}
enum LoadResult { case Loaded(Document $document); }
function main(): void { let $emoji = "😀"; Document $document = new Document(); LoadResult $result = LoadResult::Loaded($document); let $again = $document; }"#,
            "E0470",
            "$document; }",
        ),
        (
            "equality",
            r#"enum Bucket { case Values(List<int> $values); }
function main(): void { let $emoji = "😀"; Bucket $left = Bucket::Values([1]); Bucket $right = Bucket::Values([1]); bool $same = $left == $right; }"#,
            "E0584",
            "$left ==",
        ),
        (
            "match",
            r#"enum Status { case Draft; case Published; }
function main(): void { let $emoji = "😀"; let $label = match (Status::Draft) { Status::Draft => 1, }; }"#,
            "E0585",
            "match",
        ),
    ];

    for (label, source, code, needle) in cases {
        let diagnostics = diagnostics_for_document(&format!("file:///{label}.doria"), source);
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic["code"] == code)
            .unwrap_or_else(|| panic!("missing {code} for {label}: {diagnostics:#?}"));
        let expected = byte_offset_to_position(source, source.find(needle).expect("range needle"));
        assert_eq!(
            diagnostic["range"]["start"]["line"], expected.line,
            "{label}"
        );
        assert_eq!(
            diagnostic["range"]["start"]["character"], expected.character,
            "{label} must convert compiler byte offsets to UTF-16"
        );
    }
}

#[test]
fn accepts_completed_decision_0113_clear_without_false_diagnostics() {
    let source = "function main(): void { writable List<int> $v = []; $v->clear(); }";
    let uri = "file:///completed-collection.doria";
    let diagnostics = diagnostics_for_document(uri, source);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    assert!(code_actions_for_document(uri, source).is_empty());
}

#[test]
fn exposes_static_identity_fixes_without_rewriting_the_member() {
    let uri = "file:///statics.doria";
    let sigil_text =
        "class Foo { static int $prop = 1; function read(): int { return Foo::$prop; } }";
    let sigil_diagnostics = diagnostics_for_document(uri, sigil_text);
    let sigil = sigil_diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == "E0494")
        .expect("sigil diagnostic");
    assert_eq!(sigil["data"]["fix"]["newText"], "");
    assert_eq!(
        sigil["data"]["fix"]["range"]["start"]["character"],
        sigil_text.rfind("$prop").expect("access sigil")
    );
    let sigil_actions = code_actions_for_document(uri, sigil_text);
    assert_eq!(sigil_actions.len(), 1);
    assert_eq!(sigil_actions[0]["edit"]["changes"][uri][0]["newText"], "");

    let static_text = "class Foo { static function create(): int { return 1; } function read(): int { return static::create(); } }";
    let static_diagnostics = diagnostics_for_document(uri, static_text);
    let late_static = static_diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == "E0495")
        .expect("late-static diagnostic");
    assert_eq!(late_static["data"]["fix"]["newText"], "self");
    assert!(!late_static["message"]
        .as_str()
        .expect("message")
        .contains("Stage"));
    let static_actions = code_actions_for_document(uri, static_text);
    assert_eq!(static_actions.len(), 1);
    assert_eq!(
        static_actions[0]["edit"]["changes"][uri][0]["newText"],
        "self"
    );
}

#[test]
fn signed_duration_factory_and_getter_have_no_false_diagnostics() {
    let diagnostics = diagnostics_for_document("file:///signed-duration.doria", SIGNED_DURATION);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn wait_copy_values_and_named_payloads_have_no_false_diagnostics() {
    let diagnostics = diagnostics_for_document("file:///wait-values.doria", WAIT_VALUES);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn registration_key_copies_have_no_false_diagnostics() {
    let diagnostics =
        diagnostics_for_document("file:///registration-keys.doria", REGISTRATION_KEYS);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn readiness_set_construction_and_owned_storage_have_no_false_diagnostics() {
    let source = include_str!("../../editors/fixtures/stage36a-readiness-set-construction.doria");
    let diagnostics = diagnostics_for_document("file:///readiness-set-construction.doria", source);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn readiness_set_accepted_surface_has_no_false_diagnostics() {
    assert_stored_source_diagnostics("readiness-set.doria", READINESS_SET, None);
}

#[test]
fn readiness_set_borrowing_and_plan_configuration_preserve_compiler_facts() {
    for (name, source) in [
        (
            "readiness-set-borrowing.doria",
            include_str!("../../editors/fixtures/stage36a-readiness-set-borrowing.doria"),
        ),
        (
            "transfer-plan-errors.doria",
            include_str!("../../editors/fixtures/stage36a-transfer-plan-errors.doria"),
        ),
    ] {
        assert_stored_source_diagnostics(name, source, None);
    }
}

#[test]
fn writable_borrowed_locals_preserve_compiler_permissions_and_rejections() {
    let name = "writable-borrowed-locals.doria";
    assert_stored_source_diagnostics(name, WRITABLE_BORROWED_LOCALS, None);
    for (before, after, code) in [
        ("lend($owner)", "peek($owner)", "E0478"),
        (
            "let $property = $holder->current;",
            "let writable $property = $holder->current;",
            "E0478",
        ),
        ("$alias->bump();", "consume($alias);", "E0474"),
        (
            "let writable $function = lend($owner);",
            "let $function = lend($owner);",
            "E0478",
        ),
        (
            "$method->bump();",
            "$owner->bump(); $method->bump();",
            "E0477",
        ),
        (
            "let $readonly = peek($owner);",
            "let writable $readonly = peek($owner);",
            "E0478",
        ),
    ] {
        let invalid = WRITABLE_BORROWED_LOCALS.replacen(before, after, 1);
        assert_lsp_diagnostic_matches_compiler(name, &invalid, code);
    }
}

fn assert_stored_source_diagnostics(name: &str, source: &str, conflict: Option<(&str, &str)>) {
    let (_, analysis) = doriac::analyze_source_for_ide(name, source)
        .unwrap_or_else(|diagnostics| panic!("{name}: {diagnostics:#?}"));
    let conflicts = &analysis.diagnostics;
    match conflict {
        Some((code, mutation)) => {
            assert_eq!(conflicts.len(), 1, "{name}: {conflicts:#?}");
            assert_eq!(conflicts[0].code, code);
            assert_eq!(conflicts[0].span.start, source.find(mutation).unwrap());
        }
        None => assert!(conflicts.is_empty(), "{name}: {conflicts:#?}"),
    }
    let compiler = prepare_diagnostics(&analysis.diagnostics);
    let published = diagnostics_for_document(&format!("file:///{name}"), source);
    assert_eq!(published.len(), compiler.len(), "{published:#?}");
    for diagnostic in &compiler {
        assert_lsp_finding_matches_compiler(name, source, diagnostic);
    }
}

#[test]
fn readiness_set_rejections_preserve_compiler_diagnostics() {
    for body in [
        "function invalid(ReadinessSet $set, take ReadableFile $stream): void { $set->register($stream); }",
        "function invalid(writable ReadinessSet $set, RegistrationKey<ReadableFile> $key): WritableFile { return $set->unregister($key); }",
        "class NotReady {} function invalid(writable ReadinessSet $set, take NotReady $stream): void { $set->register($stream); }",
        "function invalid(writable ReadinessSet $set, take ReadableFile $stream): void { $set->register($stream); $set->register($stream); }",
        "function invalid(ReadinessSet $set, RegistrationKey<ReadableFile> $key): void { $set->getWritableStream($key); }",
        "function invalid(ReadinessSet $set, RegistrationKey<ReadableFile> $key): WritableFile { return $set->getStream($key); }",
        "function invalid(writable ReadinessSet $set, RegistrationKey<ReadableFile> $key, writable WritableByteRegion $destination): void throws IoError { let writable $stream = $set->getStream($key); $stream->read($destination); }",
    ] {
        let source = format!("use Doria\\Std\\Io\\{{ReadinessSet, ReadableFile, WritableFile, RegistrationKey, WritableByteRegion, IoError}}; {body}");
        let diagnostics = doriac::check_source("invalid-readiness-set.doria", &source)
            .expect_err("invalid readiness-set access must remain rejected");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind != doriac::diagnostics::DiagnosticKind::UnsupportedDevelopmentSurface
        }), "{diagnostics:#?}");
        for diagnostic in diagnostics {
            assert_lsp_diagnostic_matches_compiler("invalid-readiness-set.doria", &source, diagnostic.code);
        }
    }
}

#[test]
fn callbacks_returning_readiness_sets_preserve_stored_source_diagnostics() {
    for callback in READINESS_FORWARDING_CALLBACKS {
        let source = format!(
            r#"{BORROWING_READINESS_ADAPTER}
function exercise(bool $choose): void throws IoError {{
    writable List<int> $source = [1];
    {{
        writable ReadinessSet $set = new ReadinessSet();
        $set->register(new BorrowingReady(File::openRead("input"), $source));
        let $forward = {callback};
        let $returned = $forward($set);
        $source->add(2);
    }}
    $source->add(3);
}}
function main(): void {{}}
"#
        );
        let name = "readiness-callback-return.doria";
        let valid = source.replace("        $source->add(2);", "");
        assert_stored_source_diagnostics(name, &valid, None);
        assert_stored_source_diagnostics(name, &source, Some(("E0763", "$source->add(2)")));
    }
}

#[test]
fn readiness_callback_dispatch_preserves_independent_member_removal_diagnostics() {
    for callback in READINESS_FORWARDING_CALLBACKS {
        let source = format!(
            r#"{BORROWING_READINESS_ADAPTER}
function release(take BorrowingReady $stream): void {{}}
function exercise(bool $choose): void throws IoError {{
    writable List<int> $first = [1];
    writable List<int> $second = [2];
    writable ReadinessSet $set = new ReadinessSet();
    let $firstKey = $set->register(new BorrowingReady(File::openRead("input"), $first));
    let $secondKey = $set->register(new BorrowingReady(File::openRead("input"), $second));
    let $forward = {callback};
    let writable $returned = $forward($set);
    release($returned->unregister($firstKey));
    $first->add(3);
    release($returned->unregister($secondKey));
    $second->add(4);
}}
function main(): void {{}}
"#
        );
        let name = "readiness-callback-member-removal.doria";
        assert_stored_source_diagnostics(name, &source, None);
        let invalid = source.replace("$first->add(3);", "$first->add(3); $second->add(3);");
        assert_stored_source_diagnostics(name, &invalid, Some(("E0763", "$second->add(3)")));
    }
}

#[test]
fn captured_readiness_sets_preserve_source_conflicts_until_callback_cleanup() {
    for callback in [
        "fn() with (take $input) => inspect($input)",
        "function (): void with (take $input) { inspect($input); }",
    ] {
        let source = format!(
            r#"{BORROWING_READINESS_ADAPTER}
function inspect(ReadinessSet $set): void {{}}
function keep(take ReadinessSet $input): function(): void {{ return {callback}; }}
function exercise(): void throws IoError {{
    writable List<int> $source = [1];
    {{
        writable ReadinessSet $set = new ReadinessSet();
        $set->register(new BorrowingReady(File::openRead("input"), $source));
        let $recover = keep($set);
        $recover();
        $source->add(2);
    }}
    $source->add(3);
}}
function main(): void {{}}
"#
        );
        let name = "captured-readiness-storage.doria";
        let valid = source.replacen("        $source->add(2);", "", 1);
        assert_stored_source_diagnostics(name, &valid, None);
        assert_stored_source_diagnostics(name, &source, Some(("E0654", "$source->add(2)")));
    }
}

#[test]
fn readiness_sets_in_owned_collections_preserve_sources_through_forwarding() {
    let owner = r#"
class Owner {
    writable List<ReadinessSet> $sets;
    function __construct() { $this->sets = [new ReadinessSet()]; }
}
"#;
    for (ty, initial, setup, member) in [
        ("List<ReadinessSet>", "[new ReadinessSet()]", "", "$sets[0]"),
        ("ReadinessSet[]", "[new ReadinessSet()]", "", "$sets[0]"),
        (
            "List<List<ReadinessSet>>",
            "[[new ReadinessSet()]]",
            "",
            "$sets[0][0]",
        ),
        (
            "Dictionary<string, ReadinessSet>",
            "[\"slot\" => new ReadinessSet()]",
            "",
            "$sets[\"slot\"]",
        ),
        (
            "SortedDictionary<string, ReadinessSet>",
            "SortedDictionary::from([])",
            "$sets->set(\"slot\", new ReadinessSet());",
            "$sets[\"slot\"]",
        ),
        ("Owner", "new Owner()", "", "$sets->sets[0]"),
    ] {
        for collection in ["$sets", "forward($sets)", "forward(forward($sets))"] {
            let source = format!(
                r#"{BORROWING_READINESS_ADAPTER}{owner}
function forward(take {ty} $sets): {ty} {{ return $sets; }}
function exercise(): void throws IoError {{
    writable List<int> $source = [1];
    {{
        writable {ty} $sets = {initial};
        {setup}
        {member}->register(new BorrowingReady(File::openRead("input"), $source));
        let $returned = {collection};
        $source->add(2);
    }}
    $source->add(3);
}}
function main(): void {{}}
"#
            );
            let name = "forwarded-collection-storage.doria";
            let valid = source.replacen("        $source->add(2);", "", 1);
            assert_stored_source_diagnostics(name, &valid, None);
            assert_stored_source_diagnostics(name, &source, Some(("E0763", "$source->add(2)")));
        }
    }
}

#[test]
fn collection_extraction_helpers_preserve_stored_source_diagnostics() {
    for (ty, initial, member, result, extraction) in [
        (
            "List<ReadinessSet>",
            "[new ReadinessSet()]",
            "$sets[0]",
            "ReadinessSet",
            "$sets->removeAt(0)",
        ),
        (
            "List<ReadinessSet>",
            "[new ReadinessSet()]",
            "$sets[0]",
            "?ReadinessSet",
            "$sets->pop()",
        ),
        (
            "Dictionary<string, ReadinessSet>",
            "[\"slot\" => new ReadinessSet()]",
            "$sets[\"slot\"]",
            "?ReadinessSet",
            "$sets->remove(\"slot\")",
        ),
        (
            "List<List<ReadinessSet>>",
            "[[new ReadinessSet()]]",
            "$sets[0][0]",
            "?List<ReadinessSet>",
            "$sets->pop()",
        ),
    ] {
        for body in [
            format!("return {extraction};"),
            format!("let $removed = {extraction}; return relay($removed);"),
        ] {
            let source = format!(
                r#"{BORROWING_READINESS_ADAPTER}
function relay(take {result} $value): {result} {{ return $value; }}
function extract(take {ty} $incoming): {result} {{
    let writable $sets = $incoming;
    {body}
}}
function exercise(): void throws IoError {{
    writable List<int> $source = [1];
    {{
        writable {ty} $sets = {initial};
        {member}->register(new BorrowingReady(File::openRead("input"), $source));
        let $removed = extract($sets);
        $source->add(2);
    }}
    $source->add(3);
}}
function main(): void {{}}
"#
            );
            let name = "extracted-collection-storage.doria";
            let valid = source.replace("        $source->add(2);", "");
            assert_stored_source_diagnostics(name, &valid, None);
            assert_stored_source_diagnostics(name, &source, Some(("E0763", "$source->add(2)")));
        }
    }
}

#[test]
fn collection_clear_retires_only_the_selected_stored_sources() {
    for clearing in [
        "$owner->first->clear();",
        "emptySets($owner->first);",
        "relay($owner->first);",
    ] {
        let source = format!(
            r#"{BORROWING_READINESS_ADAPTER}
class Owner {{
    writable List<ReadinessSet> $first;
    writable List<ReadinessSet> $second;
    function __construct() {{
        $this->first = [new ReadinessSet()];
        $this->second = [new ReadinessSet()];
    }}
}}
function emptySets(writable List<ReadinessSet> $sets): void {{ $sets->clear(); }}
function relay(writable List<ReadinessSet> $sets): void {{ emptySets($sets); }}
function exercise(): void throws IoError {{
    writable List<int> $firstSource = [1];
    writable List<int> $secondSource = [2];
    let writable $owner = new Owner();
    $owner->first[0]->register(new BorrowingReady(File::openRead("input"), $firstSource));
    $owner->second[0]->register(new BorrowingReady(File::openRead("input"), $secondSource));
    {clearing}
    $firstSource->add(3);
    $secondSource->add(4);
}}
function main(): void {{}}
"#
        );
        let name = "cleared-collection-storage.doria";
        let valid = source.replace("    $secondSource->add(4);", "");
        assert_stored_source_diagnostics(name, &valid, None);
        assert_stored_source_diagnostics(name, &source, Some(("E0763", "$secondSource->add(4)")));
    }
}

#[test]
fn collection_literals_and_extraction_preserve_source_lifetimes() {
    let declarations = r#"
class Cursor implements Iterator<int> {
    writable bool $available = true;
    function __construct(borrow List<int> $source) {}
    function hasCurrent(): bool { return $this->available; }
    function getCurrent(): int { return $this->source[0]; }
    writable function advance(): void { $this->available = false; }
}
function capture(List<int> $source): function(): int {
    let $cursor = new Cursor($source);
    return fn() with (take $cursor) => $cursor->getCurrent();
}
"#;
    for (element, initial, result, observe, conflict_code) in [
        (
            "Iterator<int>",
            "new Cursor($source)",
            "?Iterator<int>",
            "if ($removed != null) { echo $removed->getCurrent(); }",
            "E0763",
        ),
        (
            "?function(): int",
            "capture($source)",
            "?function(): int",
            "if ($removed != null) { echo $removed(); }",
            "E0654",
        ),
        (
            "List<Iterator<int>>",
            "[new Cursor($source)]",
            "?List<Iterator<int>>",
            "if ($removed != null) { echo $removed[0]->getCurrent(); }",
            "E0763",
        ),
    ] {
        let escaping = format!(
            r#"{declarations}
function main(): void {{
    writable List<{element}> $escaped = [];
    {{
        writable List<int> $source = [7];
        $escaped = [{initial}];
    }}
    echo $escaped->count;
}}
"#
        );
        let name = "collection-literal-escape.doria";
        let diagnostics = compiler_findings_for_publication(name, &escaping);
        assert!(
            diagnostics.iter().any(|finding| finding.code == "E0762"),
            "{diagnostics:#?}"
        );
        let published = diagnostics_for_document(&format!("file:///{name}"), &escaping);
        assert_eq!(published.len(), diagnostics.len(), "{published:#?}");
        for diagnostic in diagnostics {
            assert_lsp_finding_matches_compiler(name, &escaping, &diagnostic);
        }
        for construction in [
            format!("[{initial}]"),
            "make($source)".to_string(),
            "forward(make($source))".to_string(),
        ] {
            let source = format!(
                r#"{declarations}
function make(List<int> $source): List<{element}> {{ return [{initial}]; }}
function forward(take List<{element}> $values): List<{element}> {{ return $values; }}
function relay(take {result} $value): {result} {{ return $value; }}
function extract(take List<{element}> $incoming): {result} {{
    let writable $values = $incoming;
    return relay($values->pop());
}}
function main(): void {{
    writable List<int> $source = [7];
    {{
        writable List<{element}> $values = {construction};
        let $removed = extract($values);
        $source->add(8);
        {observe}
    }}
    $source->add(9);
}}
"#
            );
            let name = "collection-literal-source-lifetimes.doria";
            let valid = source.replace("        $source->add(8);", "");
            doriac::check_source(name, &valid).unwrap();
            assert_stored_source_diagnostics(name, &valid, None);
            assert_stored_source_diagnostics(
                name,
                &source,
                Some((conflict_code, "$source->add(8)")),
            );
        }
    }
}

#[test]
fn conditional_capability_grammar_accepts_clauses_and_rejects_invalid_placement() {
    let source = include_str!("../../editors/fixtures/stage36a-conditional-capabilities.doria");
    doriac::parse_source("conditional-capabilities.doria", source).unwrap();
    assert_stored_source_diagnostics("conditional-capabilities.doria", source, None);
    let (_, analysis) =
        doriac::analyze_source_for_ide("conditional-capabilities.doria", source).unwrap();
    assert_eq!(analysis.info.contracts.conditions.len(), 8);
    assert!(!analysis.info.callable_signatures.is_empty());
    assert!(analysis
        .info
        .contracts
        .conformances
        .iter()
        .all(|fact| fact.origin.source != doriac::source::SourceId::default()));
    for invalid in [
        "class C<T> { int $stored where (T implements Ready); }",
        "class C<T> { function __construct() where (T implements Ready) {} }",
        "function inspect<T>(): int where (T implements Ready) { return 1; }",
    ] {
        assert_lsp_diagnostic_matches_compiler("conditional-placement.doria", invalid, "E0777");
    }
}

#[test]
fn conditional_getters_preserve_nonblocking_contract_diagnostics() {
    let source = r#"
interface Ready { function ready(): int; }
class Adapter<T> {
    function __construct(parameter take T $source) { $this->source = $source; }
    T $source;
    int $state where (T implements Ready) { get { return $this->source->ready(); } }
}
"#;
    assert_conditional_body_publication(
        "conditional-blocking-getter.doria",
        source,
        &[("E0770", 1)],
    );
}

#[test]
fn void_returning_flush_does_not_satisfy_the_outcome_contract() {
    let source = r#"
use Doria\Std\Io\{FlushableStream, IoError};
class Old implements FlushableStream {
    writable function flush(): void throws IoError {}
}
"#;
    assert_lsp_diagnostic_matches_compiler("void-flush-contract.doria", source, "E0755");
}

#[test]
fn conditional_declaration_errors_preserve_subject_identity_and_exact_diagnostics() {
    for subject in ["Missing", "Ready", "Concrete"] {
        let source = format!("interface Ready {{}} class Concrete {{}} class C<T> {{ function inspect(): int where ({subject} implements Ready) {{ return 1; }} }}");
        let (_, analysis) =
            doriac::analyze_source_for_ide("condition-subject.doria", &source).unwrap();
        let finding = analysis
            .diagnostics
            .iter()
            .find(|finding| finding.code == "E0778")
            .unwrap();
        let start = source
            .find(&format!("where ({subject} implements"))
            .unwrap()
            + "where (".len();
        assert_eq!(
            finding.span,
            doriac::source::Span::new(start, start + subject.len())
        );
        assert_eq!(finding.kind, doriac::diagnostics::DiagnosticKind::Language);
        assert!(analysis.info.contracts.conditions.is_empty());
        assert_lsp_finding_matches_compiler("condition-subject.doria", &source, finding);
    }
    for (interface, code) in [
        ("Concrete", "E0750"),
        ("?Ready<T>", "E0750"),
        ("Ready", "E0402"),
        ("Ready<T, int>", "E0402"),
        ("Ready<1>", "E0536"),
    ] {
        let source = format!("interface Ready<T> {{}} class Concrete {{}} class C<T> {{ function inspect(): int where (T implements {interface}) {{ return 1; }} }}");
        assert_lsp_diagnostic_matches_compiler("condition-interface.doria", &source, code);
    }
    for (declaration, code) in [
        ("class C<T> { function inspect<U>(): void where (T implements Ready<U>) {} function inspect(): void {} }", "E0481"),
        ("trait C<T> { function inspect<U>(): void where (T implements Ready<U>) {} function inspect(): void {} }", "E0309"),
        ("class C<T> { function inspect<T>(): void where (T implements Ready<T>) {} }", "E0530"),
        ("trait C<T> { function inspect<T>(): void where (T implements Ready<T>) {} }", "E0530"),
    ] {
        let source = format!("interface Ready<T> {{}} {declaration}");
        assert_lsp_diagnostic_matches_compiler("condition-declaration.doria", &source, code);
    }
}

#[test]
fn conditional_constraint_diagnostics_are_independent_of_declaration_order() {
    let declarations = "interface Ready {} class Full implements Ready {} class Plain {} class Adapter<T> implements Ready where (T implements Ready) {} class Required<T implements Ready> {}";
    for (source_type, constraint_errors) in [("Full", 0), ("Plain", 1)] {
        let consumer = format!("class Consumer {{ function inspect(Required<Adapter<{source_type}>> $value): void {{}} }}");
        for source in [
            format!("{declarations} {consumer}"),
            format!("{consumer} {declarations}"),
        ] {
            let name = "conditional-constraint-order.doria";
            let (_, analysis) = doriac::analyze_source_for_ide(name, &source).unwrap();
            assert_conditional_analysis_publication(
                name,
                &source,
                &analysis,
                &[("E0535", constraint_errors)],
            );
            assert_eq!(analysis.info.contracts.conditions.len(), 1);
        }
    }
}

#[test]
fn primitive_constraint_evidence_does_not_allow_interface_valued_returns() {
    let source = "interface Boxed { function value(): Equatable<int>; } class Invalid implements Boxed { function value(): int { return 1; } } function main(): void {}";
    assert_lsp_diagnostic_matches_compiler("primitive-interface-result.doria", source, "E0755");
}

#[test]
fn availability_implication_diagnostics_preserve_clauses_and_accessor_ownership() {
    let check = |source: &str, expected: Option<(&str, &str)>| {
        let name = "availability-implication.doria";
        let (program, analysis) = doriac::analyze_source_for_ide(name, source).unwrap();
        let findings = prepare_diagnostics(&analysis.diagnostics);
        if let Some((code, clause)) = expected {
            assert!(
                findings.iter().any(|finding| finding.code == code),
                "{findings:#?}"
            );
            let start = source.rfind(clause).unwrap();
            let clause_span = doriac::source::Span::new(start, start + clause.len());
            for finding in &findings {
                assert!(matches!(finding.code, "E0755" | "E0729"), "{finding:#?}");
                assert_eq!(finding.span, clause_span);
                assert_eq!(finding.kind, doriac::diagnostics::DiagnosticKind::Language);
                assert!(!finding.development_only);
                assert!(finding
                    .labels
                    .iter()
                    .any(|label| label.role == LabelRole::Secondary));
            }
        } else {
            assert!(findings.is_empty(), "{findings:#?}");
        }
        for item in &program.items {
            let doriac::ast::Item::Class(class) = item else {
                continue;
            };
            for member in &class.members {
                match member {
                    doriac::ast::ClassMember::Method(method) if method.availability.is_some() => {
                        assert_eq!(
                            analysis
                                .info
                                .contracts
                                .callable_conditions
                                .get(&method.span),
                            Some(&method.span)
                        );
                    }
                    doriac::ast::ClassMember::Property(property)
                        if property.availability.is_some() =>
                    {
                        assert!(analysis
                            .info
                            .contracts
                            .conditions
                            .contains_key(&property.span));
                        for hook in &property.hooks {
                            assert_eq!(
                                analysis.info.contracts.callable_conditions.get(&hook.span),
                                Some(&property.span)
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        let published = diagnostics_for_document(&format!("file:///{name}"), source);
        assert_eq!(published.len(), findings.len());
        for finding in &findings {
            assert_lsp_finding_matches_compiler(name, source, finding);
        }
        analysis
    };
    let interfaces = "interface Ready { function read(): int; } interface Pending extends Ready {}";
    for (parameters, premise, guard, expected) in [
        (
            "T",
            "where (T implements Pending)",
            "where (T implements Ready)",
            None,
        ),
        ("T implements Ready", "", "where (T implements Ready)", None),
        (
            "T",
            "where (T implements Ready)",
            "where (T implements Pending)",
            Some(("E0755", "where (T implements Pending)")),
        ),
    ] {
        check(&format!("{interfaces} class C<{parameters}> implements Ready {premise} {{ function read(): int {guard} {{ return 1; }} }}"), expected);
    }
    for (guard, expected) in [
        ("", None),
        (
            "where (U implements Pending)",
            Some(("E0729", "where (U implements Pending)")),
        ),
    ] {
        check(&format!("{interfaces} open class Parent<T> {{ open function read(): int where (T implements Ready) {{ return 1; }} }} class Child<U> extends Parent<U> {{ override function read(): int {guard} {{ return 2; }} }}"), expected);
    }
    let value = "interface Value { writable int $value { get; set (int $next); } }";
    for (premise, guard, expected) in [
        ("Pending", "Ready", None),
        (
            "Ready",
            "Pending",
            Some(("E0755", "where (T implements Pending)")),
        ),
    ] {
        let analysis = check(&format!("{interfaces} {value} class C<T> implements Value where (T implements {premise}) {{ writable int $value where (T implements {guard}) {{ get => 1; set (int $next) {{}} }} }}"), expected);
        assert_eq!(analysis.info.contracts.callable_conditions.len(), 2);
        if expected.is_some() {
            assert_eq!(
                analysis
                    .diagnostics
                    .iter()
                    .filter(|finding| finding.code == "E0755")
                    .count(),
                1
            );
        }
    }
    for (parent, child, expected) in [
        ("Pending", "Ready", None),
        (
            "Ready",
            "Pending",
            Some(("E0729", "where (U implements Pending)")),
        ),
    ] {
        check(&format!("{interfaces} {value} open class Parent<T> {{ open writable int $value where (T implements {parent}) {{ get => 1; set (int $next) {{}} }} }} class Child<U> extends Parent<U> {{ override writable int $value where (U implements {child}) {{ get => 2; set (int $next) {{}} }} }}"), expected);
    }
    for (required, owner, implementation, expected) in [
        ("T implements Ready", "T", "V implements Ready", None),
        (
            "T",
            "T implements Ready",
            "V",
            Some(("E0755", "where (V implements Ready)")),
        ),
    ] {
        check(&format!("interface Ready {{}} interface Work {{ function inspect<{required}>(T $value): int; }} class C<{owner}> implements Work {{ function inspect<{implementation}>(V $value): int where (V implements Ready) {{ return 1; }} }}"), expected);
    }
    for (parent, expected) in [
        ("T", None),
        ("U", Some(("E0729", "where (U implements Ready)"))),
    ] {
        check(&format!("interface Ready {{}} open class Parent<T> {{ open function inspect<U>(U $value): int where ({parent} implements Ready) {{ return 1; }} }} class Child<U> extends Parent<U> {{ override function inspect<W>(W $value): int where (U implements Ready) {{ return 2; }} }}"), expected);
    }
    for (guard, expected) in [
        ("Ready", None),
        ("Pending", Some(("E0755", "where (U implements Pending)"))),
    ] {
        check(&format!("{interfaces} open class Parent<T> implements Ready where (T implements Ready) {{ open function read(): int where (T implements Ready) {{ return 1; }} }} class Child<U> extends Parent<U> {{ override function read(): int where (U implements {guard}) {{ return 2; }} }}"), expected);
    }
    let borrowed = "interface Ready {} class Item {} interface View { Item $item { borrowed get; } } class C<T> implements View where (T implements Ready) { Item $stored; function __construct(parameter take Item $stored) { $this->stored = $stored; } Item $item where (T implements Ready) { borrowed get { return $this->stored; } } }";
    let analysis = check(borrowed, None);
    assert_eq!(analysis.info.contracts.callable_conditions.len(), 1);
}

fn assert_conditional_body_publication(
    name: &str,
    source: &str,
    expected_errors: &[(&str, usize)],
) -> Vec<Diagnostic> {
    let (_, analysis) = doriac::analyze_source_for_ide(name, source).unwrap();
    assert_conditional_analysis_publication(name, source, &analysis, expected_errors)
}

fn assert_conditional_analysis_publication(
    name: &str,
    source: &str,
    analysis: &doriac::semantics::SemanticAnalysis,
    expected_errors: &[(&str, usize)],
) -> Vec<Diagnostic> {
    assert!(
        analysis.diagnostics.iter().all(|finding| {
            expected_errors
                .iter()
                .any(|(code, _)| finding.code == *code)
        }),
        "{source}: {:#?}",
        analysis.diagnostics
    );
    for (code, count) in expected_errors {
        assert_eq!(
            analysis
                .diagnostics
                .iter()
                .filter(|finding| finding.code == *code)
                .count(),
            *count,
            "{source}: {:#?}",
            analysis.diagnostics
        );
    }
    let findings = prepare_diagnostics(&analysis.diagnostics);
    let published = diagnostics_for_document(&format!("file:///{name}"), source);
    assert_eq!(published.len(), findings.len(), "{published:#?}");
    for finding in &findings {
        assert_eq!(finding.kind, doriac::diagnostics::DiagnosticKind::Language);
        assert!(!finding.development_only);
        if finding.code == "E0779" {
            assert_eq!(
                finding.title,
                "Member Availability Condition Is Not Satisfied"
            );
            let primary = finding
                .labels
                .iter()
                .find(|label| label.role == LabelRole::Primary)
                .unwrap();
            assert_eq!(primary.span, finding.span);
            assert!(
                finding.labels.iter().any(|label| {
                    label.role == LabelRole::Secondary
                        && matches!(label.source, DiagnosticSource::Current)
                        && analysis
                            .info
                            .contracts
                            .conditions
                            .values()
                            .any(|condition| {
                                let clause = condition.clause;
                                (clause.source, clause.start, clause.end)
                                    == (label.span.source, label.span.start, label.span.end)
                            })
                }),
                "{finding:#?}"
            );
        }
        assert_lsp_finding_matches_compiler(name, source, finding);
    }
    findings
}

#[test]
fn guarded_bodies_preserve_scoped_premises_and_language_diagnostics() {
    for (name, source, errors) in [
        (
            "guarded-body-premises.doria",
            r#"
interface Ready { function read(): int; int $state { get; } }
function inspect<U implements Ready>(U $source): int { return $source->read(); }
class Adapter<T> {
    T $source;
    function __construct(parameter take T $source) { $this->source = $source; }
    function read(): int where (T implements Ready) { return inspect($this->source); }
    int $state where (T implements Ready) { get { return $this->source->state; } }
    static function inspect<U>(U $source): int where (U implements Ready) { return $source->read(); }
}
trait Inspection<T> {
    function inspect(T $source): int where (T implements Ready) { return $source->read(); }
}
"#,
            &[][..],
        ),
        (
            "guarded-premise-isolation.doria",
            r#"
interface Ready { function read(): int; }
class Adapter<T> {
    function guarded(T $source): int where (T implements Ready) { return $source->read(); }
    function unguarded(T $source): int { return $source->read(); }
}
function unguarded<T>(T $source): int { return $source->read(); }
"#,
            &[("E0537", 2)][..],
        ),
        (
            "guarded-core-bounds.doria",
            r#"
class Inspector {
    static function display<T>(T $value): string where (T implements Displayable) { return "{$value}"; }
    static function equal<T>(T $left, T $right): bool where (T implements Equatable<T>) { return $left == $right; }
    static function before<T>(T $left, T $right): bool where (T implements Comparable<T>) { return $left < $right; }
}
"#,
            &[][..],
        ),
    ] {
        assert_conditional_body_publication(name, source, errors);
    }
}

#[test]
fn guarded_calls_preserve_caller_context_and_exact_locations_in_both_source_orders() {
    for (declarations, callers, unavailable_calls) in [
        (
            "interface Ready {} class Adapter<T> { function read(): int where (T implements Ready) { return 1; } }",
            "function good<T implements Ready>(Adapter<T> $value): int { return $value->read(); } function bad<T>(Adapter<T> $value): int { return $value->read(); }",
            &["$value->read()"] as &[&str],
        ),
        (
            "interface Ready {} class Yes implements Ready {} class No {} class Adapter<T> { function read(): int where (T implements Ready) { return 1; } }",
            "function good(Adapter<Yes> $value): int { return $value->read(); } function bad(Adapter<No> $value): int { return $value->read(); } function nullable(?Adapter<No> $value): ?int { return $value?->read(); }",
            &["$value->read()", "$value?->read()"],
        ),
        (
            "interface Ready {} class Yes implements Ready {} class No {} class Factory { static function read<T>(T $source): int where (T implements Ready) { return 1; } static function make<T>(): ?T where (T implements Ready) { return null; } }",
            "function good<T implements Ready>(T $source): int { return Factory::read(source: $source); } function bad<T>(T $source): int { return Factory::read(source: $source); } function goodResult(): ?Yes { return Factory::make(); } function badResult(): ?No { return Factory::make(); }",
            &["Factory::read(source: $source)", "Factory::make()"],
        ),
        (
            "interface Ready {} class Adapter<T> { writable int $value where (T implements Ready) { get => 1; set (int $next) {} } }",
            "function good<T implements Ready>(writable Adapter<T> $value): void { $value->value += 1; } function bad<T>(writable Adapter<T> $value): void { $value->value += 1; }",
            &["$value->value"],
        ),
    ] {
        for source in [
            format!("/* \u{1f680} */ {declarations} {callers}"),
            format!("/* \u{1f680} */ {callers} {declarations}"),
        ] {
            let findings = assert_conditional_body_publication(
                "guarded-call-order.doria", &source, &[("E0779", unavailable_calls.len())],
            );
            let mut actual = findings.iter().filter(|finding| finding.code == "E0779")
                .map(|finding| (finding.span.start, finding.span.end)).collect::<Vec<_>>();
            let mut expected = unavailable_calls.iter().map(|call| {
                let start = source.rfind(call).unwrap();
                (start, start + call.len())
            }).collect::<Vec<_>>();
            actual.sort_unstable();
            expected.sort_unstable();
            assert_eq!(actual, expected, "{source}");
        }
    }
}

#[test]
fn composed_guarded_calls_preserve_authored_caller_diagnostics_in_both_orders() {
    let alias_declarations = r#"
interface Ready { function read(): int; }
class Yes implements Ready { function read(): int { return 1; } }
class No {}
trait Root<T> {
    function read(T $value): int where (T implements Ready) { return $value->read(); }
}
trait Middle<U> { uses Root<U> { Root<U>::read as inspect; } }
class Adapter<V> { uses Middle<V>; }
class Inactive { uses Root<No>; }
"#;
    let proved_callers = r#"
function inspect<T implements Ready>(Adapter<T> $adapter, T $source): int {
    return $adapter->inspect($source) + $adapter->read($source);
}
function concrete(Adapter<Yes> $adapter, Yes $source): int { return $adapter->read($source); }
"#;
    for (name, declarations, callers, unavailable_call) in [
        ("composed-proved-alias.doria", alias_declarations, proved_callers.to_string(), None),
        ("composed-unproved-alias.doria", alias_declarations,
            format!("{proved_callers} function unproved<T>(Adapter<T> $adapter, T $source): int {{ return $adapter->inspect($source); }}"),
            Some("$adapter->inspect($source)")),
        ("composed-inactive-specialization.doria", r#"
interface Ready { function read(): int; }
class No {}
trait Inspection<T> {
    function read(T $value): int where (T implements Ready) { return $value->read(); }
}
class Adapter { uses Inspection<No>; }
"#, "function inspect(Adapter $adapter, No $source): int { return $adapter->read($source); }".to_string(), Some("$adapter->read($source)")),
        ("composing-class-guarded-forward.doria", r#"
interface Ready { function read(): int; }
class No {}
trait Read<T> {
    function read(T $source): int where (T implements Ready) { return $source->read(); }
}
class Adapter<U> {
    uses Read<U>;
    function forward(U $source): int where (U implements Ready) { return $this->read($source); }
}
class Inactive { uses Read<No>; }
"#, "function forward<T implements Ready>(Adapter<T> $adapter, T $source): int { return $adapter->forward($source); }".to_string(), None),
        ("composed-property-update.doria", r#"
interface Ready {}
class Yes implements Ready {}
class No {}
trait State<T> {
    writable int $state where (T implements Ready) { get => 1; set (int $next) {} }
}
trait Middle<U> { uses State<U>; }
class Adapter<V> { uses Middle<V>; }
"#, r#"
function good<T implements Ready>(writable Adapter<T> $adapter): void { $adapter->state += 1; }
function bad(writable Adapter<No> $adapter): void { $adapter->state += 1; }
"#.to_string(), Some("$adapter->state")),
    ] {
        for source in [format!("{declarations}{callers}"), format!("{callers}{declarations}")] {
            let findings = assert_conditional_body_publication(
                name, &source, &[("E0779", usize::from(unavailable_call.is_some()))],
            );
            if let Some(call) = unavailable_call {
                let finding = findings.iter().find(|finding| finding.code == "E0779").unwrap();
                let start = source.rfind(call).unwrap();
                assert_eq!(finding.span, doriac::source::Span::new(start, start + call.len()));
            }
        }
    }
}

#[test]
fn authored_trait_imported_calls_preserve_unproved_guards_in_both_orders() {
    for (name, declarations, callers, bad_call) in [
        (
            "trait-body-imported-method.doria",
            r#"
interface Ready {}
trait Read<T> { function read(): int where (T implements Ready) { return 1; } }
"#,
            r#"
trait Forward<U> {
    uses Read<U>;
    function good(): int where (U implements Ready) { return $this->read(); }
    function bad(): int { return $this->read(); }
}
class Adapter<V> { uses Forward<V>; }
"#,
            "$this->read()",
        ),
        (
            "trait-body-imported-getter.doria",
            r#"
interface Ready {}
trait State<T> { int $state where (T implements Ready) { get => 1; } }
"#,
            r#"
trait Forward<U> {
    uses State<U>;
    function good(): int where (U implements Ready) { return $this->state; }
    function bad(): int { return $this->state; }
}
class Adapter<V> { uses Forward<V>; }
"#,
            "$this->state",
        ),
        (
            "trait-body-imported-alias.doria",
            r#"
interface Ready {}
trait Left<T> { function read(): int where (T implements Ready) { return 1; } }
trait Right<T> { function read(): int where (T implements Ready) { return 2; } }
"#,
            r#"
trait Forward<U> {
    uses Left<U>, Right<U> { Left<U>::read insteadof Right<U>; Right<U>::read as alternative; }
    function good(): int where (U implements Ready) { return $this->read() + $this->alternative(); }
    function bad(): int { return $this->alternative(); }
}
class Adapter<V> { uses Forward<V>; }
"#,
            "$this->alternative()",
        ),
        (
            "trait-body-imported-static.doria",
            r#"
interface Ready {}
trait Factory<T> {
    static function make<U>(U $value): U where (T implements Ready) { return $value; }
}
"#,
            r#"
trait Forward<U> {
    uses Factory<U>;
    static function good(int $value): int where (U implements Ready) { return self::make($value); }
    static function bad(int $value): int { return self::make($value); }
}
class Adapter<V> { uses Forward<V>; }
"#,
            "self::make($value)",
        ),
    ] {
        for source in [
            format!("{declarations}{callers}"),
            format!("{callers}{declarations}"),
        ] {
            let findings = assert_conditional_body_publication(name, &source, &[("E0779", 1)]);
            let finding = findings
                .iter()
                .find(|finding| finding.code == "E0779")
                .unwrap();
            let start = source.rfind(bad_call).unwrap();
            assert_eq!(
                finding.span,
                doriac::source::Span::new(start, start + bad_call.len())
            );
            let clause = "where (T implements Ready)";
            let clause_start =
                source.find(declarations).unwrap() + declarations.rfind(clause).unwrap();
            assert!(
                finding.labels.iter().any(|label| {
                    label.role == LabelRole::Secondary
                        && matches!(label.source, DiagnosticSource::Current)
                        && (label.span.start, label.span.end)
                            == (clause_start, clause_start + clause.len())
                }),
                "{finding:#?}"
            );
        }
    }
}

#[test]
fn interface_diamonds_and_conditional_conflicts_preserve_authored_publication() {
    for edges in [
        "Root<int>, Left<int>, Right<int>",
        "Left<int>, Right<int>, Root<int>",
    ] {
        let source = format!("interface Root<T> {{}} interface Left<T> extends Root<T> {{}} interface Right<T> extends Root<T> {{}} class Value implements {edges} {{}}");
        assert_stored_source_diagnostics("interface-diamond-order.doria", &source, None);
    }
    for (name, declarations, caller, primary, related_tokens, check_unused) in [
        ("active-iteration-conformance-conflict.doria", r#"
interface Ready {}
interface Writable {}
interface Left<E> extends Iterable<E> {}
interface Right<E> extends Iterable<E> {}
class Both implements Ready, Writable {}
class Adapter<T> implements Left<int> where (T implements Ready), Right<string> where (T implements Writable) {}
"#, "function inspect(Adapter<Both> $values): void { foreach ($values as int $value) {} }",
            "Right<string>", &["Ready", "Writable"] as &[&str], false),
        ("inactive-duplicate-conformance.doria",
            "interface Ready {} interface Writable {} class Neither {} \
        class Adapter<T> implements Iterable<int> where (T implements Ready), Iterable<int> where (T implements Writable) {}",
            "function inspect(Adapter<Neither> $values): void { foreach ($values as int $value) {} }",
            "Iterable<int>", &["Iterable<int>"], true),
    ] {
        let mut sources = vec![format!("{declarations}{caller}"), format!("{caller}{declarations}")];
        if check_unused {
            sources.push(declarations.to_string());
        }
        for source in sources {
            let findings = assert_conditional_body_publication(name, &source, &[("E0752", 1)]);
            let finding = findings.iter().find(|finding| finding.code == "E0752").unwrap();
            let start = source.rfind(primary).unwrap();
            assert_eq!(finding.span, doriac::source::Span::new(start, start + primary.len()));
            for &token in related_tokens {
                let start = if token == primary { source.find(token) } else { source.rfind(token) }.unwrap();
                assert!(finding.labels.iter().any(|label| {
                    label.role == LabelRole::Secondary && matches!(label.source, DiagnosticSource::Current)
                        && (label.span.start, label.span.end) == (start, start + token.len())
                }), "{finding:#?}");
            }
            if primary == "Iterable<int>" {
                assert_eq!(finding.labels.iter().filter(|label| {
                    label.role == LabelRole::Secondary && matches!(label.source, DiagnosticSource::Current)
                        && &source[label.span.start..label.span.end] == primary
                }).count(), 1);
            }
        }
    }
}

#[test]
fn conditional_coherence_covers_formed_types_and_inferred_results_in_both_orders() {
    let declarations = r#"
interface Ready {}
interface Writable {}
interface BothCaps extends Ready, Writable {}
interface Root<E> {}
class Both implements Ready, Writable {}
class ReadyOnly implements Ready {}
class WritableOnly implements Writable {}
class Neither {}
class Adapter<T> implements Root<int> where (T implements Ready), Root<string> where (T implements Writable) {}
"#;
    for (name, intermediate, caller, conflict, invalid_predicate) in [
        (
            "concrete-parameter-coherence.doria", "",
            "function inspect(Adapter<Both> $value): void {}", true, false,
        ),
        (
            "symbolic-caller-coherence.doria", "",
            "function good<T implements Ready>(Adapter<T> $value): void {}", false, false,
        ),
        (
            "inferred-result-coherence.doria",
            "function make<T>(T $value): ?Adapter<T> { return null; }",
            "function inspect(Both $value): void { let $result = make($value); }", true, false,
        ),
        (
            "incoherent-predicate-argument.doria", "",
            "interface Uses<E> {} trait Gate<T> { function inspect(): void where (T implements Uses<Adapter<Both>>) {} }", true, true,
        ),
    ] {
        for source in [
            format!("{declarations}{intermediate}{caller}"),
            format!("{caller}{intermediate}{declarations}"),
        ] {
            let (program, analysis) = doriac::analyze_source_for_ide(name, &source).unwrap();
            let findings = assert_conditional_analysis_publication(
                name, &source, &analysis, &[("E0752", usize::from(conflict))],
            );
            if conflict {
                let finding = findings.iter().find(|finding| finding.code == "E0752").unwrap();
                let primary = "Root<string>";
                let start = source.find(primary).unwrap();
                assert_eq!(finding.span, doriac::source::Span::new(start, start + primary.len()));
                for guard in ["Ready", "Writable"] {
                    let start = source.find(declarations).unwrap() + declarations.rfind(guard).unwrap();
                    assert!(finding.labels.iter().any(|label| {
                        label.role == LabelRole::Secondary && matches!(label.source, DiagnosticSource::Current)
                            && (label.span.start, label.span.end) == (start, start + guard.len())
                    }), "{finding:#?}");
                }
            }
            if invalid_predicate {
                let method = program.items.iter().find_map(|item| match item {
                    doriac::ast::Item::Trait(declaration) if declaration.name == "Gate" => {
                        declaration.members.iter().find_map(|member| match member {
                            doriac::ast::ClassMember::Method(method) if method.name == "inspect" => Some(method),
                            _ => None,
                        })
                    }
                    _ => None,
                }).unwrap();
                assert!(!analysis.info.contracts.conditions.contains_key(&method.span));
                assert_eq!(analysis.info.contracts.conditions.len(), 2);
            }
        }
    }
}

#[test]
fn concrete_stored_type_coherence_is_published_without_member_access_in_both_orders() {
    let declarations = r#"
interface Ready {}
interface Writable {}
interface Root<E> {}
class Both implements Ready, Writable {}
class ReadyOnly implements Ready {}
class Adapter<T> implements Root<int> where (T implements Ready), Root<string> where (T implements Writable) {}
"#;
    for (name, storage, argument, conflict) in [
        (
            "direct-stored-type-coherence.doria",
            "class Owner<T> { ?Adapter<T> $value = null; }", "Both", true,
        ),
        (
            "nested-trait-stored-type-coherence.doria",
            "trait Storage<T> { ?Adapter<T> $value = null; } class Inner<T> { uses Storage<T>; } class Owner<T> { ?Inner<T> $inner = null; }", "Both", true,
        ),
        (
            "nested-ready-only-stored-types.doria",
            "class Inner<T> { ?Adapter<T> $value = null; } class Owner<T> { ?Inner<T> $inner = null; }", "ReadyOnly", false,
        ),
        (
            "stable-recursive-stored-type-coherence.doria",
            "class Owner<T> { ?Owner<T> $next = null; ?Adapter<T> $value = null; }", "Both", true,
        ),
    ] {
        let caller = format!("function inspect(Owner<{argument}> $owner): void {{}}");
        for source in [
            format!("{declarations}{storage}{caller}"),
            format!("{caller}{storage}{declarations}"),
        ] {
            let findings = assert_conditional_body_publication(
                name, &source, &[("E0752", usize::from(conflict))],
            );
            if conflict {
                let finding = findings.iter().find(|finding| finding.code == "E0752").unwrap();
                let primary = "Root<string>";
                let start = source.find(primary).unwrap();
                assert_eq!(finding.span, doriac::source::Span::new(start, start + primary.len()));
                for guard in ["Ready", "Writable"] {
                    let start = source.find(declarations).unwrap() + declarations.rfind(guard).unwrap();
                    assert!(finding.labels.iter().any(|label| {
                        label.role == LabelRole::Secondary && matches!(label.source, DiagnosticSource::Current)
                            && (label.span.start, label.span.end) == (start, start + guard.len())
                    }), "{finding:#?}");
                }
            }
        }
    }
}

#[test]
fn active_conformance_targets_preserve_nested_coherence_diagnostics_in_both_orders() {
    let declarations = r#"
interface Ready {}
interface Writable {}
interface Root<E> {}
class Both implements Ready, Writable {}
class ReadyOnly implements Ready {}
class Neither {}
class Adapter<T> implements Root<int> where (T implements Ready), Root<string> where (T implements Writable) {}
"#;
    for (name, owner) in [
        (
            "direct-conformance-target-coherence.doria",
            "interface Uses<E> {} class Owner<T> implements Uses<Adapter<T>> where (T implements Ready) {}",
        ),
        (
            "inherited-conformance-target-coherence.doria",
            "interface Uses<E> {} interface Derived<T> extends Uses<Adapter<T>> {} class Owner<T> implements Derived<T> where (T implements Ready) {}",
        ),
        (
            "conformance-target-stored-type-coherence.doria",
            "interface Uses<E> {} class Inner<T> { ?Adapter<T> $value = null; } class Owner<T> implements Uses<Inner<T>> where (T implements Ready) {}",
        ),
    ] {
        for (argument, conflict) in [("Both", true), ("ReadyOnly", false), ("Neither", false)] {
            let caller = format!("function inspect(Owner<{argument}> $value): void {{}}");
            for source in [
                format!("{declarations}{owner}{caller}"),
                format!("{caller}{owner}{declarations}"),
            ] {
                let findings = assert_conditional_body_publication(
                    name, &source, &[("E0752", usize::from(conflict))],
                );
                if conflict {
                    let finding = findings.iter().find(|finding| finding.code == "E0752").unwrap();
                    let primary = "Root<string>";
                    let start = source.find(primary).unwrap();
                    assert_eq!(finding.span, doriac::source::Span::new(start, start + primary.len()));
                    for guard in ["Ready", "Writable"] {
                        let start = source.find(declarations).unwrap() + declarations.rfind(guard).unwrap();
                        assert!(finding.labels.iter().any(|label| {
                            label.role == LabelRole::Secondary && matches!(label.source, DiagnosticSource::Current)
                                && (label.span.start, label.span.end) == (start, start + guard.len())
                        }), "{finding:#?}");
                    }
                }
            }
        }
    }
}

#[test]
fn guarded_result_without_inference_preserves_e0531() {
    let source = "interface Ready {} class Factory { static function make<T>(): ?T where (T implements Ready) { return null; } } function inspect(): void { Factory::make(); }";
    assert_conditional_body_publication("guarded-unresolved-result.doria", source, &[("E0531", 1)]);
}

#[test]
fn symbolic_generic_calls_require_bounds_without_later_instantiation() {
    let source = "interface Ready {} function required<T implements Ready>(T $value): int { return 1; } function good<U implements Ready>(U $value): int { return required($value); } function bad<U>(U $value): int { return required($value); }";
    let name = "symbolic-call-bounds.doria";
    let (_, analysis) = doriac::analyze_source_for_ide(name, source).unwrap();
    assert!(analysis.info.contracts.conditions.is_empty());
    assert!(analysis.info.contracts.callable_conditions.is_empty());
    assert_eq!(analysis.diagnostics.len(), 1, "{:#?}", analysis.diagnostics);
    let finding = &analysis.diagnostics[0];
    assert_eq!(finding.code, "E0535");
    assert_eq!(finding.kind, doriac::diagnostics::DiagnosticKind::Language);
    let start = source.rfind("required($value)").unwrap();
    assert_eq!(
        finding.span,
        doriac::source::Span::new(start, start + "required($value)".len())
    );
    let findings = prepare_diagnostics(&analysis.diagnostics);
    let published = diagnostics_for_document(&format!("file:///{name}"), source);
    assert_eq!(published.len(), findings.len());
    for finding in &findings {
        assert_lsp_finding_matches_compiler(name, source, finding);
    }
}

#[test]
fn conditional_display_publication_uses_exact_specializations_in_both_source_orders() {
    let declarations = r#"
interface Enabled {}
class Yes implements Enabled {}
class No {}
class Adapter<T> implements Displayable where (T implements Enabled) {
    function toString(): string { return "adapter"; }
}
"#;
    let callers = r#"
function good(Adapter<Yes> $value): string {
    echo $value;
    printf("%s", $value);
    return "{$value}" . $value . sprintf("%s", $value);
}
function badEcho(Adapter<No> $value): void { echo $value; }
function badConcat(Adapter<No> $value): string { return "value:" . $value; }
function badInterpolation(Adapter<No> $value): string { return "{$value}"; }
function badFormat(Adapter<No> $value): string { return sprintf("%s", $value); }
"#;
    for source in [
        format!("{declarations}{callers}"),
        format!("{callers}{declarations}"),
    ] {
        let name = "conditional-display.doria";
        let findings =
            assert_conditional_body_publication(name, &source, &[("E0462", 4), ("E0404", 1)]);
        let program = doriac::parse_source(name, &source).unwrap();
        for function in program.items.iter().filter_map(|item| match item {
            doriac::ast::Item::Function(function) if function.name.starts_with("bad") => {
                Some(function)
            }
            _ => None,
        }) {
            let errors = findings
                .iter()
                .filter(|finding| {
                    finding.code == "E0462"
                        && finding.span.start >= function.span.start
                        && finding.span.end <= function.span.end
                })
                .collect::<Vec<_>>();
            assert_eq!(errors.len(), 1, "{}: {findings:#?}", function.name);
            assert_eq!(&source[errors[0].span.start..errors[0].span.end], "$value");
        }
        let consequence = findings
            .iter()
            .find(|finding| finding.code == "E0404")
            .unwrap();
        assert_eq!(
            &source[consequence.span.start..consequence.span.end],
            "\"value:\" . $value"
        );
    }
}

#[test]
fn conditional_core_bound_publication_rejects_only_inactive_specializations() {
    let declarations = r#"
interface Enabled {}
class Yes implements Enabled {}
class No {}
class Adapter<T> implements Displayable where (T implements Enabled), Error where (T implements Enabled) {
    function __construct(string $message) {}
    function toString(): string { return "adapter"; }
}
function display<T implements Displayable>(T $value): string { return "{$value}"; }
function message<T implements Error>(T $value): string { return $value->message; }
"#;
    let callers = r#"
function good(Adapter<Yes> $value): string { return display($value) . message($value); }
function bad(Adapter<No> $value): string { return display($value) . message($value); }
"#;
    for source in [
        format!("{declarations}{callers}"),
        format!("{callers}{declarations}"),
    ] {
        let findings = assert_conditional_body_publication(
            "conditional-core-bounds.doria",
            &source,
            &[("E0535", 2)],
        );
        let mut actual = findings
            .iter()
            .filter(|finding| finding.code == "E0535")
            .map(|finding| (finding.span.start, finding.span.end))
            .collect::<Vec<_>>();
        let mut expected = ["display($value)", "message($value)"].map(|call| {
            let start = source.rfind(call).unwrap();
            (start, start + call.len())
        });
        actual.sort_unstable();
        expected.sort_unstable();
        assert_eq!(actual, expected, "{source}");
    }
}

#[test]
fn guarded_error_body_publication_accepts_message_named_property_and_throw() {
    let source = r#"
interface Named { string $name { get; } }
class Inspector {
    static function message<T>(T $error): string where (T implements Error) { return $error->message; }
    static function nullable<T>(?T $error): ?string where (T implements Error) { return $error?->message; }
    static function name<T>(T $error): string where (T implements Error, T implements Named) { return $error->name; }
    static function propagate<T>(take T $error): void throws T where (T implements Error) { throw $error; }
}
"#;
    assert_conditional_body_publication("guarded-error-body.doria", source, &[]);
}

#[test]
fn conditional_error_admission_publication_preserves_throws_catch_and_throw_locations() {
    let source = r#"
interface Enabled {}
class Yes implements Enabled {}
class No {}
class Failure<T> implements Error where (T implements Enabled) { function __construct(string $message) {} }
function good(take Failure<Yes> $error): void throws Failure<Yes> { throw $error; }
function bad(take Failure<No> $error): void throws Failure<No> { throw $error; }
function goodCatch(take Failure<Yes> $error): void { try { throw $error; } catch (Failure<Yes>) {} }
function badCatch(): void { try {} catch (Failure<No>) {} }
"#;
    let findings = assert_conditional_body_publication(
        "conditional-error-admission.doria",
        source,
        &[("E0618", 1), ("E0623", 1), ("E0626", 1)],
    );
    let bad = source.find("function bad(").unwrap();
    for (code, start, token) in [
        (
            "E0618",
            source.find("throws Failure<No>").unwrap() + "throws ".len(),
            "Failure<No>",
        ),
        (
            "E0623",
            bad + source[bad..].find("throw $error;").unwrap() + "throw ".len(),
            "$error",
        ),
        ("E0626", source.rfind("Failure<No>").unwrap(), "Failure<No>"),
    ] {
        let finding = findings
            .iter()
            .find(|finding| finding.code == code)
            .unwrap();
        assert_eq!(
            finding.span,
            doriac::source::Span::new(start, start + token.len()),
            "{finding:#?}"
        );
    }
}

#[test]
fn inactive_core_conditions_preserve_authored_shape_diagnostics() {
    let source = r#"
interface Enabled {}
class No {}
class BadDisplay<T> implements Displayable where (T implements Enabled) { function toString(): int { return 1; } }
class BadError<T> implements Error where (T implements Enabled) {}
function inspect(BadDisplay<No> $display, BadError<No> $error): void {}
"#;
    let name = "conditional-core-shapes.doria";
    let findings = assert_conditional_body_publication(name, source, &[("E0463", 1), ("E0613", 1)]);
    let program = doriac::parse_source(name, source).unwrap();
    for (code, class_name) in [("E0463", "BadDisplay"), ("E0613", "BadError")] {
        let class = program
            .items
            .iter()
            .find_map(|item| match item {
                doriac::ast::Item::Class(class) if class.name == class_name => Some(class),
                _ => None,
            })
            .unwrap();
        let finding = findings
            .iter()
            .find(|finding| finding.code == code)
            .unwrap();
        assert_eq!(finding.span, class.span);
    }
}

#[test]
fn nullable_symbolic_property_publication_requires_a_scoped_nominal_member() {
    let source = r#"
interface Ready { int $value { get; } }
class Inspector {
    static function good<T>(?T $source): ?int where (T implements Ready) { return $source?->value; }
    static function bad<T>(?T $source): ?int { return $source?->value; }
}
"#;
    let findings = assert_conditional_body_publication(
        "nullable-symbolic-property.doria",
        source,
        &[("E0537", 1)],
    );
    let finding = findings
        .iter()
        .find(|finding| finding.code == "E0537")
        .unwrap();
    let start = source.rfind("value").unwrap();
    assert_eq!(
        finding.span,
        doriac::source::Span::new(start, start + "value".len())
    );
    assert!(findings.iter().all(|finding| finding.code != "E0507"));
}

#[test]
fn nongeneric_trait_method_parameters_do_not_capture_composing_class_parameters() {
    let source = r#"trait Identity {
    function identity<U>(U $value): U { return $value; }
}
class Box<U> { uses Identity; }
class Plain { uses Identity; }

function main(): void {
    let $box = new Box<int>();
    echo $box->identity("box;");
    echo $box->identity(7);
    echo ";";
    let $plain = new Plain();
    echo $plain->identity("plain;");
}
"#;
    assert_stored_source_diagnostics("nongeneric-trait-method-parameters.doria", source, None);
}

#[test]
fn conditional_trait_composition_accepts_checked_declarations() {
    let root = "interface Ready<T> {} trait Root<T, U> { function inspect(T $value): int where (T implements Ready<U>, U implements Ready<T>) { return 1; } }";
    let middle = "trait Middle<X, Y> { uses Root<List<Y>, X>; }";
    let record = "class Record<A, B> { uses Middle<B, A>; }";
    for (name, source) in [
        ("conditional-nested-traits-before.doria", format!("{root} {middle} {record}")),
        ("conditional-nested-traits-after.doria", format!("{record} {middle} {root}")),
        ("conditional-trait-method-generics.doria", r#"
interface Ready<T> {}
trait Inspection<T> {
    static function inspect<U>(U $value): int where (T implements Ready<List<U>>, U implements Ready<T>) { return 1; }
}
class Record<U> { uses Inspection<U>; }
"#.to_string()),
        ("conditional-trait-alias.doria", r#"
interface Ready {}
trait Inspection<T> {
    function inspect(T $value): int where (T implements Ready) { return 1; }
}
class Record<U> { uses Inspection<U> { Inspection<U>::inspect as other; } }
"#.to_string()),
        ("conditional-trait-property.doria", r#"
interface Ready {}
trait Inspection<T> {
    writable int $state where (T implements Ready) { get => 1; set (int $next) {} }
}
class Record<U> { uses Inspection<U>; }
"#.to_string()),
    ] {
        assert_conditional_body_publication(name, &source, &[]);
    }
}

#[test]
fn false_trait_guards_preserve_duplicate_body_conflicts_and_authored_locations() {
    let source = r#"
interface Ready {}
class No {}
trait First<T> { function inspect(): int where (T implements Ready) { return 1; } }
trait Second<T> { function inspect(): int where (T implements Ready) { return 2; } }
class Record { uses First<No>, Second<No>; }
"#;
    let name = "conditional-trait-conflict.doria";
    let findings = assert_conditional_body_publication(name, source, &[("E0757", 1)]);
    let program = doriac::parse_source(name, source).unwrap();
    let methods = program
        .items
        .iter()
        .filter_map(|item| match item {
            doriac::ast::Item::Trait(declaration) => {
                declaration.members.iter().find_map(|member| match member {
                    doriac::ast::ClassMember::Method(method) => Some(method),
                    _ => None,
                })
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(methods.len(), 2);
    let finding = findings
        .iter()
        .find(|finding| finding.code == "E0757")
        .unwrap();
    assert_eq!(
        (finding.span.start, finding.span.end),
        (methods[1].span.start, methods[1].span.end)
    );
    assert!(finding.labels.iter().any(|label| {
        label.role == LabelRole::Secondary
            && matches!(label.source, DiagnosticSource::Current)
            && (label.span.start, label.span.end) == (methods[0].span.start, methods[0].span.end)
    }));
}

#[test]
fn symbolic_type_formation_preserves_source_bounds_and_rejected_witnesses() {
    let declarations =
        "interface Ready {} class Box<T implements Ready> {} interface View<T implements Ready> {}";
    let uses = "function good<U implements Ready>(Box<U> $box, View<U> $view): void {} function bad<U>(Box<U> $box, View<U> $view): void {}";
    for (name, source, errors) in [
        (
            "symbolic-types-before.doria",
            format!("{declarations} {uses}"),
            2,
        ),
        (
            "symbolic-types-after.doria",
            format!("{uses} {declarations}"),
            2,
        ),
        (
            "symbolic-parent-trait.doria",
            r#"
interface Ready {}
open class Base<T implements Ready> {}
trait Members<T implements Ready> {}
class Good<U implements Ready> extends Base<U> { uses Members<U>; }
class BadParent<U> extends Base<U> {}
class BadTrait<U> { uses Members<U>; }
"#
            .to_string(),
            2,
        ),
        (
            "malformed-conformance-type.doria",
            r#"
interface Ready {}
interface Needs<T implements Ready> {}
class No {}
class Bad<T> implements Needs<T> {}
function inspect(Bad<No> $value): void {}
"#
            .to_string(),
            1,
        ),
        (
            "nested-nominal-substitution.doria",
            r#"
interface Ready {}
class Item implements Ready {}
class Other implements Ready {}
class Box<V implements Ready> {}
interface Wrap<V> {}
class Accept<T implements Wrap<Box<U>>, U implements Ready> {}
class Source implements Wrap<Box<Item>> {}
function good<X implements Wrap<Box<Y>>, Y implements Ready>(Accept<X, Y> $value): void {}
function concrete(Accept<Source, Item> $value): void {}
function mismatch(Accept<Source, Other> $value): void {}
"#
            .to_string(),
            1,
        ),
    ] {
        let (_, analysis) = doriac::analyze_source_for_ide(name, &source).unwrap();
        assert_eq!(
            analysis.diagnostics.len(),
            errors,
            "{name}: {:#?}",
            analysis.diagnostics
        );
        assert!(
            analysis.diagnostics.iter().all(|finding| {
                finding.code == "E0535"
                    && finding.kind == doriac::diagnostics::DiagnosticKind::Language
                    && !finding.development_only
            }),
            "{name}: {:#?}",
            analysis.diagnostics
        );
        assert!(analysis.info.contracts.conditions.is_empty());
        assert!(analysis.info.contracts.callable_conditions.is_empty());
        assert!(analysis.info.contracts.conformances.iter().filter(|fact| {
            matches!(&fact.implementing_type, doriac::types::ResolvedType::Class(class) if class.name == "Bad")
        }).all(|fact| fact.status != doriac::semantics::contracts::ConformanceStatus::Checked));
        let findings = prepare_diagnostics(&analysis.diagnostics);
        let published = diagnostics_for_document(&format!("file:///{name}"), &source);
        assert_eq!(published.len(), findings.len());
        for finding in &findings {
            assert_lsp_finding_matches_compiler(name, &source, finding);
        }
    }
}

#[test]
fn guarded_type_formation_preserves_local_scopes_and_rejects_invalid_predicates() {
    let mut cases = vec![
        ("guarded-signature-types.doria", r#"
interface Ready {}
class Box<T implements Ready> {}
interface View<T implements Ready> {}
class Inspector<T> {
    function accept(Box<T> $box): int where (T implements Ready) { return 1; }
    static function inspect<U>(Box<U> $box): int where (U implements Ready) { return 1; }
    function nested<U implements View<T>>(U $value): int where (T implements Ready) { return 1; }
    Box<T> $box where (T implements Ready) { get { return new Box<T>(); } }
}
trait Inspection<T> {
    function accept(Box<T> $box): int where (T implements Ready) { return 1; }
    Box<T> $box where (T implements Ready) { get { return new Box<T>(); } }
}
"#.to_string(), 0, false),
        ("guarded-storage-signature-isolation.doria", r#"
interface Ready {}
class Box<T implements Ready> {}
class Inspector<T> {
    Box<T> $box where (T implements Ready) { get { return new Box<T>(); } }
    ?Box<T> $stored = null;
    function unguarded(Box<T> $box): int { return 1; }
}
"#.to_string(), 2, false),
        ("guarded-conformance-target.doria", r#"
interface Ready {}
interface Needs<T implements Ready> {}
class Good<T> implements Needs<T> where (T implements Ready) {}
class Bad<T> implements Needs<T> {}
"#.to_string(), 1, false),
        ("malformed-inactive-predicate.doria", r#"
interface Ready {}
interface Needs<U implements Ready> {}
class No {}
class Inspector<T> { function inspect(): int where (T implements Needs<No>) { return 1; } }
"#.to_string(), 1, true),
        ("nested-collection-predicates.doria", r#"
interface Wrap<V> {}
class Inspector<T, U> {
    function hashed(): void where (T implements Wrap<Set<U>>, U implements Hashable) {}
    function sorted(): void where (T implements Wrap<SortedSet<U>>, U implements Comparable<U>) {}
    function queued(): void where (T implements Wrap<PriorityQueue<U>>, U implements Comparable<U>) {}
    function dictionary(): void where (T implements Wrap<Dictionary<U, int>>, U implements Hashable) {}
    function sortedDictionary(): void where (T implements Wrap<SortedDictionary<U, int>>, U implements Comparable<U>) {}
    function bad(): void where (T implements Wrap<Set<U>>) {}
}
"#.to_string(), 1, false),
        ("nullable-payload-not-hashable.doria", r#"
interface Wrap<V> {}
class Inspector<T, U> {
    function bad(): void where (T implements Wrap<Set<?U>>, U implements Hashable) {}
}
"#.to_string(), 1, true),
        ("primitive-hashable-predicates.doria", r#"
interface Wrap<V> {}
class Inspector<T> {
    function good(): void where (T implements Wrap<Set<int>>) {}
    function bad(): void where (T implements Wrap<Set<float>>) {}
}
"#.to_string(), 1, false),
        ("guarded-explicit-return-scopes.doria", r#"
interface Ready {}
class Box<T implements Ready> {}
class Inspector<T> {
    function make(): Box<T> where (T implements Ready) { return new Box<T>(); }
    static function makeOther<U>(): Box<U> where (U implements Ready) { return new Box<U>(); }
    function borrowValue(Box<T> $value): Box<T> where (T implements Ready) { return $value; }
    function forward(Box<T> $value): Box<T> where (T implements Ready) { return $this->borrowValue($value); }
}
"#.to_string(), 0, false),
    ];
    for (name, clause) in [
        (
            "predicate-conjunction-first.doria",
            "T implements Needs<U>, U implements Ready",
        ),
        (
            "predicate-conjunction-last.doria",
            "U implements Ready, T implements Needs<U>",
        ),
    ] {
        cases.push((name, format!("interface Ready {{}} interface Needs<U implements Ready> {{}} class Inspector<T, U> {{ function inspect(): int where ({clause}) {{ return 1; }} }}"), 0, false));
    }
    for (name, source, errors, invalid_predicate) in cases {
        let (_, analysis) = doriac::analyze_source_for_ide(name, &source).unwrap();
        assert_conditional_analysis_publication(name, &source, &analysis, &[("E0535", errors)]);
        assert!(analysis
            .diagnostics
            .iter()
            .all(|finding| finding.code != "E0454"));
        if invalid_predicate {
            assert!(analysis.info.contracts.conditions.is_empty());
        }
    }
}

#[test]
fn recursive_type_expansion_preserves_e0539_and_accepts_finite_recursion() {
    for (name, source, errors) in [
        (
            "direct-type-expansion.doria",
            "class Node<T> { ?Node<List<T>> $next = null; } function main(): void { let $node = new Node<int>(); }",
            1,
        ),
        (
            "mutual-type-expansion.doria",
            "class First<T> { ?Second<List<T>> $next = null; } class Second<T> { ?First<T> $next = null; } function main(): void { let $node = new First<int>(); }",
            1,
        ),
        (
            "stable-type-recursion.doria",
            "class Node<T> { ?Node<T> $next = null; } function main(): void { let $node = new Node<int>(); }",
            0,
        ),
        (
            "finite-type-collapse.doria",
            "class Node<T> { ?Node<int> $next = null; } function main(): void { let $node = new Node<bool>(); }",
            0,
        ),
    ] {
        let (_, analysis) = doriac::analyze_source_for_ide(name, source).unwrap();
        assert_eq!(analysis.diagnostics.len(), errors, "{name}: {:#?}", analysis.diagnostics);
        assert!(analysis.diagnostics.iter().all(|finding| {
            finding.code == "E0539" && finding.kind == doriac::diagnostics::DiagnosticKind::Language
                && !finding.development_only
        }), "{name}: {:#?}", analysis.diagnostics);
        let findings = prepare_diagnostics(&analysis.diagnostics);
        let published = diagnostics_for_document(&format!("file:///{name}"), source);
        assert!(published.iter().all(|finding| finding["code"] != "B0001"));
        assert_eq!(published.len(), findings.len(), "{name}: {published:#?}");
        for finding in &findings {
            assert_lsp_finding_matches_compiler(name, source, finding);
        }
    }
}

#[test]
fn missing_named_return_types_preserve_e0617_with_effects_and_guards() {
    for effects in ["", " throws Failure"] {
        for (declaration, guarded) in [
            (format!("function missing(){effects} {{}}"), false),
            (format!("class Worker {{ function missing(){effects} {{}} }}"), false),
            (format!("trait Worker {{ function missing(){effects} {{}} }}"), false),
            (format!("class Worker<T> {{ function missing(){effects} where (T implements Ready) {{}} }}"), true),
            (format!("class Worker<T> {{ static function missing(){effects} where (T implements Ready) {{}} }}"), true),
            (format!("trait Worker<T> {{ function missing(){effects} where (T implements Ready) {{}} }}"), true),
        ] {
            let source = format!("interface Ready {{}} class Failure implements Error {{ function __construct(string $message) {{}} }} {declaration}");
            let name = "missing-named-return.doria";
            let (_, analysis) = doriac::analyze_source_for_ide(name, &source).unwrap();
            if guarded {
                assert_conditional_analysis_publication(name, &source, &analysis, &[("E0617", 1)]);
            } else {
                assert_eq!(analysis.diagnostics.len(), 1, "{source}: {:#?}", analysis.diagnostics);
                assert_eq!(analysis.diagnostics[0].code, "E0617");
                let findings = prepare_diagnostics(&analysis.diagnostics);
                let published = diagnostics_for_document(&format!("file:///{name}"), &source);
                assert_eq!(published.len(), findings.len());
                for finding in &findings {
                    assert_lsp_finding_matches_compiler(name, &source, finding);
                }
            }
            let finding = analysis.diagnostics.iter().find(|finding| finding.code == "E0617").unwrap();
            assert_eq!(finding.kind, doriac::diagnostics::DiagnosticKind::Language);
            assert!(!finding.development_only);
            assert!(source[finding.span.start..finding.span.end].contains("function missing("));
        }
    }
}

#[test]
fn ordinary_generic_contracts_preserve_owner_and_method_parameter_identities() {
    for (owner_type, method_type, valid) in [
        ("U", "W", true),
        ("W", "U", false),
        ("W", "W", false),
        ("U", "U", false),
    ] {
        for (base, implementation, modifier, code) in [
            (
                "interface First<T> { function first<U>(T $owner, U $value): int; }",
                "class Value<U> implements First<U>",
                "",
                "E0755",
            ),
            (
                "open class Parent<T> { open function first<U>(T $owner, U $value): int { return 1; } }",
                "class Value<U> extends Parent<U>",
                "override ",
                "E0729",
            ),
        ] {
            let source = format!("{base} {implementation} {{ {modifier}function first<W>({owner_type} $owner, {method_type} $value): int {{ return 1; }} }}");
            let name = "ordinary-generic-identities.doria";
            let (program, analysis) = doriac::analyze_source_for_ide(name, &source).unwrap();
            assert!(analysis.diagnostics.iter().all(|finding| finding.code != "E0776"));
            assert!(analysis.info.contracts.conditions.is_empty());
            assert!(analysis.info.contracts.callable_conditions.is_empty());
            let findings = prepare_diagnostics(&analysis.diagnostics);
            let published = diagnostics_for_document(&format!("file:///{name}"), &source);
            assert_eq!(published.len(), findings.len());
            if valid {
                assert!(findings.is_empty(), "{source}: {findings:#?}");
                continue;
            }
            let method = program.items.iter().find_map(|item| match item {
                doriac::ast::Item::Class(class) if class.name == "Value" => {
                    class.members.iter().find_map(|member| match member {
                        doriac::ast::ClassMember::Method(method) if method.name == "first" => Some(method),
                        _ => None,
                    })
                }
                _ => None,
            }).unwrap();
            assert!(!findings.is_empty(), "{source}");
            for finding in &findings {
                assert_eq!(finding.code, code);
                assert_eq!(finding.kind, doriac::diagnostics::DiagnosticKind::Language);
                assert_eq!(finding.span, method.span);
                assert!(finding.labels.iter().any(|label| label.role == LabelRole::Secondary));
                assert_lsp_finding_matches_compiler(name, &source, finding);
            }
        }
    }
}

#[test]
fn inherited_generic_calls_preserve_owner_and_method_inference_without_false_diagnostics() {
    let source = r#"open class Parent<T> {
    function first<U>(T $owner, U $value): T { return $owner; }
    function second<U>(T $owner, U $value): U { return $value; }
}
class Child<U> extends Parent<U> {
    function inspect(U $owner): U { return $this->first($owner, 7); }
    function inspectSecond(U $owner): int { return $this->second($owner, 7); }
}
function main(): void {
    let $value = new Child<string>();
    echo $value->inspect("owner");
    echo $value->inspectSecond("owner");
}
"#;
    let name = "inherited-generic-identities.doria";
    doriac::check_source(name, source).unwrap();
    assert_stored_source_diagnostics(name, source, None);
}

#[test]
fn stage36a_enum_and_buffered_ownership_fixtures_have_no_false_diagnostics() {
    for (name, source) in [
        (
            "stage36a-generic-enums.doria",
            include_str!("../../editors/fixtures/stage36a-generic-enums.doria"),
        ),
        (
            "stage36a-buffered-ownership.doria",
            include_str!("../../editors/fixtures/stage36a-buffered-ownership.doria"),
        ),
    ] {
        doriac::check_source(name, source).unwrap();
        assert_stored_source_diagnostics(name, source, None);
    }
}

#[test]
fn owned_field_extraction_preserves_the_compiler_finding() {
    let source = r#"
class Source {}
class Owner {
    function __construct(take Source $source) {}
    function release(): Source { return take $this->source; }
}
"#;
    assert_lsp_diagnostic_matches_compiler("invalid-field-extraction.doria", source, "E0780");
}

#[test]
fn standard_stream_copy_transport_and_owned_interface_borrows_have_no_false_diagnostics() {
    let source = include_str!("../../editors/fixtures/stage36a-standard-streams.doria");
    let name = "standard-streams.doria";
    doriac::check_source(name, source).unwrap();
    assert_stored_source_diagnostics(name, source, None);
}

#[test]
fn copy_to_interface_temporary_borrows_preserve_compiler_diagnostics() {
    for property in ["input", "output", "error"] {
        let source = format!(
            r#"use Doria\Std\Io\{{StandardStreams, ReadinessStream}};
function borrowed(ReadinessStream $owner): ReadinessStream {{ return $owner; }}
function inspect(): void {{
    let $original = StandardStreams::{property};
    let $view = borrowed($original);
}}
"#
        );
        assert_lsp_diagnostic_matches_compiler(
            "standard-interface-temporary.doria",
            &source,
            "E0478",
        );
    }
}

#[test]
fn file_inspection_methods_remain_forbidden_in_property_hooks() {
    for file in ["ReadableFile", "WritableFile", "DuplexFile"] {
        for method in ["getPosition", "getLength"] {
            for expression in [
                format!("$this->file->{method}()"),
                "inspect($this->file)".to_string(),
            ] {
                let source = format!(
                    r#"use Doria\Std\Io\{{{file}, IoError}};
function inspect({file} $file): int throws IoError {{ return $file->{method}(); }}
class Inspector {{
    function __construct(take {file} $file) {{}}
    int $value {{
        get {{
            try {{ return {expression}; }}
            catch (IoError $error) {{ return 0; }}
        }}
    }}
}}
"#
                );
                assert_lsp_diagnostic_matches_compiler(
                    "file-inspection-hook.doria",
                    &source,
                    "E0770",
                );
            }
        }
    }
}

#[test]
fn seekable_file_fixture_preserves_compiler_diagnostics() {
    let source = include_str!("../../editors/fixtures/stage36a-seekable-files.doria");
    let name = "seekable-files.doria";
    doriac::check_source(name, source).unwrap();
    assert_stored_source_diagnostics(name, source, None);
}

#[test]
fn cancellation_source_and_copy_tokens_have_no_false_diagnostics() {
    let source = include_str!("../../editors/fixtures/stage36a-cancellation.doria");
    let name = "cancellation.doria";
    doriac::check_source(name, source).unwrap();
    assert_stored_source_diagnostics(name, source, None);
}

#[test]
fn iterator_factories_and_public_foreach_have_no_false_diagnostics() {
    let source = include_str!("../../editors/fixtures/stage36a-owned-source-carriers.doria");
    let name = "owned-source-carriers.doria";
    doriac::check_source(name, source).unwrap();
    assert_stored_source_diagnostics(name, source, None);
    let (_, analysis) = doriac::analyze_source_for_ide(name, source).unwrap();
    for ty in [
        doriac::types::ResolvedType::Integer(doriac::types::IntegerType::Int64),
        doriac::types::ResolvedType::Class(doriac::types::ClassType::new("Item", vec![])),
    ] {
        let signatures = analysis
            .info
            .closures
            .values()
            .filter_map(|closure| match &closure.function_type {
                doriac::types::ResolvedType::Function(function) if function.return_type == ty => {
                    Some(function)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(!signatures.is_empty(), "missing callback for {ty:?}");
        assert!(signatures
            .iter()
            .all(|function| function.return_borrow.is_none()));
    }
    for observation in [
        "echo $typedCursor->getCurrent();",
        "echo $typedCursors[0]->getCurrent();",
        "echo $propertyCursor->getCurrent();",
        "echo $propertyCursors[0]->getCurrent();",
        "echo $genericCursor->getCurrent();",
        "echo $genericCursors[0]->getCurrent();",
        "echo $relayedCursor->getCurrent();",
        "echo $relayedCursors[0]->getCurrent();",
        "if ($nullableCursor != null) { echo $nullableCursor->getCurrent(); }",
    ] {
        assert_eq!(source.matches(observation).count(), 1);
        let conflicting = source.replace(observation, &format!("$source->add(9); {observation}"));
        assert_stored_source_diagnostics(name, &conflicting, Some(("E0763", "$source->add(9)")));
    }
}

#[test]
fn forwarded_region_callbacks_preserve_source_diagnostics() {
    for (ty, take, getter, observe) in [
        ("ByteRegion", "", "getRegion", "echo $view[0];"),
        (
            "?ByteRegion",
            "",
            "getRegion",
            "if ($view != null) { echo $view[0]; }",
        ),
        (
            "WritableByteRegion",
            "take ",
            "getWritableRegion",
            "$view[0] = 29; echo $view[0];",
        ),
        (
            "?WritableByteRegion",
            "take ",
            "getWritableRegion",
            "if ($view != null) { echo $view[0]; }",
        ),
    ] {
        let source = format!(
            r#"use Doria\Std\Io\{{ByteRegion, WritableByteRegion}};
function apply(function({take}{ty}): {ty} $call, {take}{ty} $input): {ty} {{ return $call($input); }}
function main(): void {{
    writable Bytes $bytes = Bytes::allocate(1);
    $bytes[0] = 17;
    {{
        let $call = fn({take}{ty} $input) => $input;
        let writable $view = apply($call, $bytes->{getter}(0, 1));
        $bytes[0] = 31;
        {observe}
    }}
    $bytes[0] = 37;
}}
"#
        );
        let name = "forwarded-region-callback.doria";
        let valid = source.replace("        $bytes[0] = 31;", "");
        doriac::check_source(name, &valid).unwrap();
        assert_stored_source_diagnostics(name, &valid, None);
        assert_stored_source_diagnostics(name, &source, Some(("E0763", "$bytes[0] = 31")));
    }
}

#[test]
fn taking_source_carriers_into_returned_callbacks_preserves_cleanup_diagnostics() {
    for callback in [
        "fn() with (take $input) => $input->getCurrent()",
        "function (): int with (take $input) { return $input->getCurrent(); }",
    ] {
        let source = format!(
            r#"
class Cursor implements Iterator<int> {{
    function __construct(borrow List<int> $source) {{}}
    function hasCurrent(): bool {{ return true; }}
    function getCurrent(): int {{ return $this->source[0]; }}
    writable function advance(): void {{}}
    function __destruct() {{
        try {{ echo "cursor{{$this->source[0]}};"; }} catch (Error) {{}}
    }}
}}
function keep(take Cursor $input): function(): int {{ return {callback}; }}
function main(): void {{
    writable List<int> $source = [7];
    {{
        let $callback = keep(new Cursor($source));
        echo $callback();
        echo "alive;";
    }}
    $source->add(8);
    echo $source->count;
}}
"#
        );
        let name = "returned-carrier-callback.doria";
        doriac::check_source(name, &source).unwrap();
        assert_stored_source_diagnostics(name, &source, None);
        let invalid = source.replace("        echo \"alive;\";", "        $source->add(9);");
        assert_stored_source_diagnostics(name, &invalid, Some(("E0654", "$source->add(9)")));
    }
}

#[test]
fn nullable_indexed_callback_views_preserve_types_and_source_diagnostics() {
    for (ty, initializer, value) in [
        ("List<Book>", "[new Book(7)]", "$source[0]"),
        ("Book[]", "[new Book(7)]", "$source[0]"),
        (
            "Dictionary<string, Book>",
            "[\"slot\" => new Book(7)]",
            "$source[\"slot\"]",
        ),
        ("List<?Book>", "[new Book(7)]", "$source[0]"),
    ] {
        let source = format!(
            r#"
class Book {{
    function __construct(int $value) {{}}
    function __destruct() {{ try {{ echo "book{{$this->value}};"; }} catch (Error) {{}} }}
}}
function first({ty} $source): ?Book {{ return {value}; }}
function main(): void {{
    writable {ty} $books = {initializer};
    let $callback = fn({ty} $source) => first($source);
    let $view = $callback($books);
    if ($view != null) {{ echo "{{$view->value}};"; }}
}}
"#
        );
        let name = "nullable-indexed-callback.doria";
        doriac::check_source(name, &source).unwrap();
        assert_stored_source_diagnostics(name, &source, None);
        let mutation = format!("{} = new Book(8);", value.replace("$source", "$books"));
        let invalid = source.replace("    if ($view", &format!("    {mutation}\n    if ($view"));
        assert_stored_source_diagnostics(name, &invalid, Some(("E0477", &mutation)));
    }
}

#[test]
fn nested_and_mixed_collection_read_and_removal_returns_have_no_false_diagnostics() {
    let source = include_str!("../../editors/fixtures/stage36a-nullable-collection-returns.doria");
    let name = "nullable-collection-returns.doria";
    doriac::check_source(name, source).unwrap();
    let diagnostics = diagnostics_for_document(&format!("file:///{name}"), source);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn registration_key_rejections_preserve_compiler_diagnostics() {
    for body in [
        "function invalid(Key<WritableFile> $key): Key<ReadableFile> { return $key; }",
        "function invalid(Key<WritableFile> $key): void { identity($key); }",
        "function invalid(Key<ReadableFile> $key): int { return $key; }",
        "function invalid(): void { new Key<ReadableFile>(); }",
        "function invalid(?Key<ReadableFile> $key = 1): void {}",
        "function invalid(Key $key): void {}",
    ] {
        let source = format!(
            "use Doria\\Std\\Io\\{{RegistrationKey as Key, ReadableFile, WritableFile}}; \
            function identity(Key<ReadableFile> $key): Key<ReadableFile> {{ return $key; }} {body}"
        );
        let diagnostics = doriac::check_source("invalid-registration-key.doria", &source)
            .expect_err("invalid registration key use must remain rejected");
        for diagnostic in diagnostics {
            assert_lsp_diagnostic_matches_compiler(
                "invalid-registration-key.doria",
                &source,
                diagnostic.code,
            );
        }
    }
}

#[test]
fn deadline_factory_rejections_preserve_compiler_diagnostics() {
    for source in [
        "use Doria\\Std\\Time\\{Deadline, ClockError}; function invalid(): void throws ClockError { Deadline::fromNow(1); }",
        "use Doria\\Std\\Time\\{Deadline, Duration}; function invalid(): void { Deadline::fromNow(Duration::fromNanoseconds(-1)); }",
    ] {
        let diagnostics = doriac::check_source("invalid-deadline.doria", source)
            .expect_err("invalid Deadline factory use must remain rejected");
        for diagnostic in diagnostics {
            assert_lsp_diagnostic_matches_compiler("invalid-deadline.doria", source, diagnostic.code);
        }
    }
}

#[test]
fn duration_member_rejections_preserve_compiler_diagnostics() {
    for body in [
        "Interval::fromNanoseconds(1.0);",
        "Interval $invalid = 1;",
        "int64 $invalid = $interval;",
        "$interval->nanoseconds = 1;",
        "$interval->nanoseconds++;",
    ] {
        let source = format!(
            "use Doria\\Std\\Time\\Duration as Interval; \
            function invalid(writable Interval $interval): void {{ echo \"non-ASCII: 😀\"; {body} }}"
        );
        let diagnostics = doriac::check_source("invalid-duration.doria", &source)
            .expect_err("invalid Duration operation must remain rejected");
        for diagnostic in diagnostics {
            assert_lsp_diagnostic_matches_compiler(
                "invalid-duration.doria",
                &source,
                diagnostic.code,
            );
        }
    }
}

#[test]
fn two_clock_static_qualifiers_publish_semantic_not_parser_diagnostics() {
    let parent = diagnostics_for_document(
        "file:///parent.doria",
        "class Child { function save(): void { parent::save(); } }",
    );
    assert_eq!(parent.len(), 1);
    assert_eq!(parent[0]["code"], "E0731");
    assert!(!parent[0]["message"]
        .as_str()
        .expect("message")
        .contains("Stage 34"));

    let trait_diagnostics = diagnostics_for_document(
        "file:///trait.doria",
        "trait UsesLimit { function limit(): int { return self::MAX_DEPTH; } }",
    );
    assert!(trait_diagnostics.is_empty(), "{trait_diagnostics:#?}");

    let composition = diagnostics_for_document(
        "file:///composition.doria",
        "trait UsesLimit { function limit(): int { return self::MAX_DEPTH; } } \
         class Limit { const int MAX_DEPTH = 10; uses UsesLimit; }",
    );
    assert!(composition.is_empty(), "{composition:#?}");
}

#[test]
fn accepted_self_and_sigil_free_static_forms_have_no_false_diagnostics() {
    let diagnostics = diagnostics_for_document(
        "file:///self.doria",
        r#"
class Counter
{
    const STEP = 1;
    static writable int $value = 1;
    static function next(): int
    {
        self::value = self::value + self::STEP;
        return self::value;
    }
}
"#,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn executable_string_surface_has_no_false_diagnostics() {
    let diagnostics = diagnostics_for_document(
        "file:///strings.doria",
        r#"function main(): void
{
    string $text = String::trim("  Straße 👍🏾  ");
    int $characters = $text->length;
    int $bytes = $text->byteLength;
    bool $found = String::containsIgnoreCase($text, "STRASSE");
    int $count = String::countOccurrences("ha ha", "ha");
    string $title = String::upperFirst("doria");
    echo "{$characters}:{$bytes}:{$found}:{$count}:{$title}\n";
}
"#,
    );

    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn string_diagnostics_keep_utf16_positions_after_emoji() {
    let source = r#"function main(): void { let $emoji = "😀"; String::contains("text", 1); }"#;
    let diagnostics = diagnostics_for_document("file:///strings.doria", source);
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == "E0408")
        .expect("wrong String argument type should be reported");
    let argument = source.rfind('1').expect("invalid argument");
    let expected = byte_offset_to_position(source, argument);

    assert_eq!(diagnostic["range"]["start"]["line"], expected.line);
    assert_eq!(
        diagnostic["range"]["start"]["character"],
        expected.character
    );
}

#[test]
fn readonly_shared_ownership_has_no_false_diagnostics() {
    let diagnostics = diagnostics_for_document(
        "file:///shared.doria",
        r#"
class Node
{
    function __construct(string $name) {}
}

function inspect(SharedReference<Node> $node): void throws Doria\Std\Io\IoError
{
    echo $node->name;
}

function choose(
    take ?SharedReference<Node> $left,
    take ?SharedReference<Node> $right,
): ?SharedReference<Node>
{
    return $left ?? $right;
}

function main(): void
{
    let $root = shared new Node("root");
    let $weak = $root->createWeakReference();
    inspect($root->share());
    let $live = choose($weak->acquire(), null);
    if ($live != null) {
        inspect($live);
    }
}
"#,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn writable_shared_ownership_has_no_false_diagnostics() {
    let diagnostics = diagnostics_for_document(
        "file:///writable-shared.doria",
        r#"
class Counter
{
    writable int $value = 0;
}

function update(WritableSharedReference<Counter> $counter): void
{
    let writable $write = $counter->acquireWritableAccess();
    $write->value++;
}

function main(): void
{
    let $counter = new WritableSharedReference(new Counter());
    let $second = $counter->share();
    let $weak = $counter->createWeakReference();
    update($counter);

    let $live = $weak->acquire();
    if ($live != null) {
        let $read = $second->acquireReadonlyAccess();
        echo $read->value;
    }
}
"#,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn nullable_shared_factory_results_preserve_compiler_payload_access() {
    for (kind, initializer, writable) in [
        ("SharedReference", "shared new Node()", false),
        (
            "ReadonlySharedReferenceAccess",
            "(new WritableSharedReference(new Node()))->acquireReadonlyAccess()",
            false,
        ),
        (
            "WritableSharedReferenceAccess",
            "(new WritableSharedReference(new Node()))->acquireWritableAccess()",
            true,
        ),
    ] {
        let declarations = format!(
            r#"class Node {{
    writable int $count = 0;
    function read(): int {{ return $this->count; }}
    writable function update(): void {{}}
}}
function make(bool $present): ?{kind}<Node> {{
    if ($present) {{ return {initializer}; }}
    return null;
}}
"#
        );
        for present in [true, false] {
            let reads = format!(
                "{declarations}function main(): void {{ let $count = make({present})?->count; let $read = make({present})?->read(); }}"
            );
            let diagnostics =
                diagnostics_for_document("file:///nullable-shared-factory.doria", &reads);
            assert!(
                diagnostics.is_empty(),
                "{kind}, {present}: {diagnostics:#?}"
            );

            let mutation = format!(
                "{declarations}function main(): void {{ /* 😀 */ make({present})?->update(); }}"
            );
            if writable {
                let diagnostics =
                    diagnostics_for_document("file:///nullable-writable-factory.doria", &mutation);
                assert!(
                    diagnostics.is_empty(),
                    "{kind}, {present}: {diagnostics:#?}"
                );
            } else {
                assert_lsp_diagnostic_matches_compiler(
                    "nullable-readonly-factory.doria",
                    &mutation,
                    "E0203",
                );
            }
        }
    }
}

#[test]
fn standalone_blocks_have_no_false_diagnostics() {
    let diagnostics = diagnostics_for_document(
        "file:///standalone-block.doria",
        r#"
class Counter
{
    writable int $value = 0;
}

function main(): void
{
    let $counter = new WritableSharedReference(new Counter());

    {
        let writable $access = $counter->acquireWritableAccess();
        $access->value++;
    }

    let $access = $counter->acquireReadonlyAccess();
    echo "{$access->value}\n";
}
"#,
    );

    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn writable_shared_diagnostics_come_from_the_compiler_surface() {
    let cases = [
        (
            "file:///direct-access.doria",
            r#"
class Counter { writable int $value = 0; }
let $counter = new WritableSharedReference(new Counter());
echo $counter->value;
"#,
            "E0548",
        ),
        (
            "file:///readonly-access-write.doria",
            r#"
class Counter { writable int $value = 0; }
let $counter = new WritableSharedReference(new Counter());
let $read = $counter->acquireReadonlyAccess();
$read->value = 1;
"#,
            "E0201",
        ),
        (
            "file:///family-crossing.doria",
            r#"
class Counter {}
let $counter = new WritableSharedReference(new Counter());
SharedReference<Counter> $wrong = $counter;
"#,
            "E0403",
        ),
        (
            "file:///direct-access-construction.doria",
            r#"
class Counter {}
let $bad = new ReadonlySharedReferenceAccess<Counter>();
"#,
            "E0543",
        ),
        (
            "file:///use-after-move.doria",
            r#"
class Counter {}
let $counter = new WritableSharedReference(new Counter());
let $moved = $counter;
let $bad = $counter->share();
"#,
            "E0470",
        ),
    ];

    for (uri, source, expected_code) in cases {
        let diagnostics = diagnostics_for_document(uri, source);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic["code"] == expected_code),
            "{uri} should report {expected_code}: {diagnostics:#?}"
        );
    }
}

#[test]
fn shared_operation_diagnostics_preserve_utf16_ranges_after_emoji() {
    let source = r#"class Counter {}
function main(): void
{
    let $emoji = "😀";
    let $counter = new WritableSharedReference(new Counter());
    let $moved = $counter;
    let $bad = $counter->share();
}
"#;
    let diagnostics = diagnostics_for_document("file:///utf16-shared.doria", source);
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == "E0470")
        .expect("use-after-move diagnostic");
    let moved_use = source
        .rfind("$counter->share")
        .expect("moved shared operation");
    let expected = byte_offset_to_position(source, moved_use);
    assert_eq!(diagnostic["range"]["start"]["line"], expected.line);
    assert_eq!(
        diagnostic["range"]["start"]["character"],
        expected.character
    );
}

#[test]
fn duplicate_member_diagnostics_publish_the_original_declaration() {
    let uri = "file:///duplicate.doria";
    let text = "class Example { const FOO = 1; static int $FOO = 2; }";
    let diagnostics = diagnostics_for_document(uri, text);
    let duplicate = diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == "E0481")
        .expect("duplicate member diagnostic");

    assert_eq!(duplicate["relatedInformation"][0]["location"]["uri"], uri);
    assert_eq!(
        duplicate["relatedInformation"][0]["location"]["range"]["start"]["character"],
        text.find("const FOO").expect("original declaration")
    );
    assert!(duplicate["relatedInformation"][0]["message"]
        .as_str()
        .expect("related message")
        .contains("original class constant"));
}

#[test]
fn accepts_boolean_word_operators_without_lsp_diagnostics() {
    let diagnostics = diagnostics_for_document(
        "file:///operators.doria",
        r#"let $a = true and false;
let $b = false or true;
let $c = not false;
let $d = true xor false;
"#,
    );

    assert_eq!(diagnostics, Vec::<Value>::new());
}

#[test]
fn accepts_stage22_is_narrowing_without_lsp_diagnostics() {
    let diagnostics = diagnostics_for_document(
        "file:///narrowing.doria",
        r#"function describePayload(mixed $payload): string
{
    if ($payload is string) {
        return $payload;
    }

    return "other payload";
}
"#,
    );

    assert_eq!(diagnostics, Vec::<Value>::new());
}

#[test]
fn accepts_control_flow_without_lsp_diagnostics() {
    let diagnostics = diagnostics_for_document(
        "file:///control_flow.doria",
        r#"function main(): void
{
let writable $count = 0;

while ($count < 3) {
    if ($count == 0) {
        echo "zero";
    } else if ($count == 1) {
        echo "one";
    } else {
        echo "many";
    }

    echo "\n";
    $count += 1;
}
}
"#,
    );

    assert_eq!(diagnostics, Vec::<Value>::new());
}

#[test]
fn accepts_stage28a_executable_control_flow_without_lsp_diagnostics() {
    let diagnostics = diagnostics_for_document(
        "file:///stage28a.doria",
        r#"function main(): void
{
    let writable $count = 0;
    given {
        let $prepared = true;
        true;
    } if ($prepared) {
        echo "prepared";
    } finally {
        let $ifCleanup = "if cleanup";
    }
    string $label = given {
        let $prefix = "label";
        true;
    } when ($count == 0): string {
        return "zero";
    } else {
        return "other";
    } finally {
        let $whenCleanup = $prefix;
    };
    while ($count < 1) {
        $count++;
    } finally {
        let $whileCleanup = "while cleanup";
    }
    do {
        $count++;
    } while ($count < 2) finally {
        let $doCleanup = "do cleanup";
    }
    echo $label;
}
"#,
    );
    assert_eq!(diagnostics, Vec::<Value>::new());
}

#[test]
fn accepts_executable_finalizers_without_syntax_or_semantic_noise() {
    let diagnostics = diagnostics_for_document(
        "file:///finally.doria",
        "function main(): void { if (true) {} finally {} }",
    );
    assert_eq!(diagnostics, Vec::<Value>::new());
}

#[test]
fn finalizer_scope_and_transfer_diagnostics_remain_compiler_owned_and_utf16_safe() {
    for (name, source, transfer) in [
        (
            "return",
            "function leave(): void { if (true) {} finally { let $emoji = \"😀\"; return; } }",
            "return",
        ),
        (
            "break",
            "function leave(): void { while (true) { if (true) {} finally { let $emoji = \"😀\"; break; } break; } }",
            "break",
        ),
        (
            "continue",
            "function leave(): void { while (true) { if (true) {} finally { let $emoji = \"😀\"; continue; } break; } }",
            "continue",
        ),
    ] {
        let uri = format!("file:///{name}.doria");
        let diagnostics = diagnostics_for_document(&uri, source);
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic["code"] == "E0612")
            .unwrap_or_else(|| panic!("missing E0612 for {name}: {diagnostics:#?}"));
        let transfer_offset = source.find(transfer).expect("escaping transfer");
        let expected = byte_offset_to_position(source, transfer_offset);
        assert_eq!(diagnostic["range"]["start"]["line"], expected.line);
        assert_eq!(
            diagnostic["range"]["start"]["character"],
            expected.character
        );
        assert!(diagnostic["message"]
            .as_str()
            .is_some_and(|message| message.contains("cannot leave a `finally` block")));
    }

    let contained = diagnostics_for_document(
        "file:///contained-finalizer-control.doria",
        r#"function main(): void
{
    if (true) {} finally {
        let $emoji = "😀";
        string $label = when (true): string {
            return "ready";
        } else {
            return "waiting";
        };
        while (true) {
            break;
        }
        let $copy = $label;
    }
}
"#,
    );
    assert_eq!(contained, Vec::<Value>::new());

    let branch_local = diagnostics_for_document(
        "file:///branch-local-finalizer.doria",
        "function main(): void { if (true) { let $branch = 1; } finally { let $copy = $branch; } }",
    );
    assert!(branch_local
        .iter()
        .any(|diagnostic| diagnostic["code"] == "E0101"));

    let given_local = diagnostics_for_document(
        "file:///given-local-finalizer.doria",
        "function main(): void { given { let $prepared = true; } if ($prepared) {} finally { let $inside = $prepared; } let $outside = $prepared; }",
    );
    assert_eq!(
        given_local
            .iter()
            .filter(|diagnostic| diagnostic["code"] == "E0101")
            .count(),
        1,
        "the given local is valid through finally and unavailable afterward: {given_local:#?}"
    );
}

#[test]
fn accepts_builtin_panic_without_lsp_diagnostics() {
    let diagnostics = diagnostics_for_document(
        "file:///main_explicit_panic.doria",
        r#"function main(): void
{
    panic("explicit panic");
}
"#,
    );

    assert_eq!(diagnostics, Vec::<Value>::new());
}

#[test]
fn publishes_stable_semantic_diagnostics_for_class_workflow_syntax() {
    let diagnostics = diagnostics_for_document(
        "file:///Child.doria",
        r#"namespace Vendor\App;
interface Printable {}
class Child extends Vendor\Base implements Vendor\Contracts\Printable {}
"#,
    );

    assert!(diagnostics.iter().all(|diagnostic| {
        !diagnostic["code"]
            .as_str()
            .is_some_and(|code| code.starts_with('P'))
    }));
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic["code"] == "E0681"
                    && diagnostic["data"]["kind"] == "compilerInput"
                    && diagnostic["message"]
                        .as_str()
                        .is_some_and(|message| message.contains("partial compilation graph"))
            })
            .count(),
        2,
        "both unresolved qualified inheritance names must retain the compiler-owned partial-graph diagnostic: {diagnostics:#?}",
    );
}

#[test]
fn checked_error_diagnostics_keep_compiler_fixes_and_utf16_ranges() {
    let error_class = r#"class Failure implements Error
{
    function __construct(string $message) {}
}

"#;

    for (source, code, fix_title) in [
        (
            format!("{error_class}function fail(): void throws /* 😀 */ Failure, Failure {{}}"),
            "E0620",
            "Remove Duplicate Throws Entry",
        ),
        (
            format!("{error_class}function fail(): void throws /* 😀 */ Error, Failure {{}}"),
            "E0621",
            "Remove Redundant Throws Entry",
        ),
    ] {
        let uri = format!("file:///{code}.doria");
        let diagnostics = diagnostics_for_document(&uri, &source);
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic["code"] == code)
            .unwrap_or_else(|| panic!("missing {code}: {diagnostics:#?}"));
        let entry = source.rfind("Failure").expect("last throws entry");
        let expected = byte_offset_to_position(&source, entry);
        assert_eq!(diagnostic["range"]["start"]["line"], expected.line);
        assert_eq!(
            diagnostic["range"]["start"]["character"], expected.character,
            "{code} must preserve UTF-16 columns after the emoji"
        );
        assert_eq!(diagnostic["data"]["fixes"][0]["title"], fix_title);
        assert_eq!(
            diagnostic["data"]["fixes"][0]["applicability"],
            "machineApplicable"
        );

        let actions = code_actions_for_document(&uri, &source);
        let action = actions
            .iter()
            .find(|action| action["title"] == fix_title)
            .unwrap_or_else(|| panic!("missing {fix_title}: {actions:#?}"));
        assert_eq!(action["isPreferred"], true);
        assert_eq!(action["edit"]["changes"][&uri][0]["newText"], "");
    }

    let other_error = r#"class OtherError implements Error
{
    function __construct(string $message) {}
}
"#;
    let unreachable = format!(
        r#"{error_class}{other_error}
function fail(): void throws Failure {{ throw new Failure("x"); }}
function handle(): void
{{
    try {{ fail(); }} catch (/* 😀 */ OtherError $caught) {{ let $message = $caught->message; }}
}}
"#
    );
    let diagnostics = diagnostics_for_document("file:///unreachable-catch.doria", &unreachable);
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == "E0629")
        .unwrap_or_else(|| panic!("missing unreachable-catch diagnostic: {diagnostics:#?}"));
    let catch_type = unreachable.find("OtherError $caught").expect("catch type");
    let expected = byte_offset_to_position(&unreachable, catch_type);
    assert_eq!(diagnostic["range"]["start"]["line"], expected.line);
    assert_eq!(
        diagnostic["range"]["start"]["character"],
        expected.character
    );

    let moved = format!(
        r#"{error_class}
function relay(take Failure $failure): void throws Doria\Std\Io\IoError
{{
    echo "😀"; try {{ throw $failure; }} catch (Failure) {{}}
    echo $failure->message;
}}
"#
    );
    let diagnostics = diagnostics_for_document("file:///moved-error.doria", &moved);
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic["code"] == "E0470"));

    let uncovered = format!(
        r#"{error_class}
function fail(): void throws Failure {{ throw new Failure("x"); }}
function caller(): void throws Doria\Std\Io\IoError {{ echo "😀"; fail(); }}
"#
    );
    let diagnostics = diagnostics_for_document("file:///uncovered-error.doria", &uncovered);
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == "E0631")
        .unwrap_or_else(|| panic!("missing catch-or-declare diagnostic: {diagnostics:#?}"));
    assert!(diagnostic["message"]
        .as_str()
        .is_some_and(|message| message.contains("catch each error or add these exact types")));

    let finalizer = format!(
        r#"{error_class}
function fail(): void throws Failure {{ throw new Failure("x"); }}
function cleanup(): void throws Failure, Doria\Std\Io\IoError
{{
    echo "😀"; try {{}} finally {{ fail(); }}
}}
"#
    );
    let diagnostics = diagnostics_for_document("file:///finally-error.doria", &finalizer);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn nullable_collection_constraints_forward_compiler_diagnostics() {
    let declarations = "class Key implements Hashable, Comparable<Key> { function hash(): uint64 { return 0; } function compare(Key $other): Ordering { return Ordering::Equal; } }";
    for (ty, contract) in [
        ("Set<?Key>", "Hashable"),
        ("Dictionary<?Key, int>", "Hashable"),
        ("SortedSet<?Key>", "Comparable"),
        ("SortedDictionary<?Key, int>", "Comparable"),
        ("PriorityQueue<?Key>", "Comparable"),
    ] {
        let source = format!("{declarations} function inspect({ty} $items): void {{}}");
        let diagnostics = diagnostics_for_document("file:///nullable-key.doria", &source);
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic["code"] == "E0523"
                    && diagnostic["message"].as_str().is_some_and(|message| {
                        message.contains("?Key") && message.contains(contract)
                    })
            }),
            "{ty}: {diagnostics:#?}"
        );

        let valid = source.replace("?Key", "Key");
        let diagnostics = diagnostics_for_document("file:///key.doria", &valid);
        assert!(diagnostics.is_empty(), "{ty}: {diagnostics:#?}");
    }
}

#[test]
fn retained_iterator_loan_diagnostics_keep_utf16_ranges_and_no_ownership_fix() {
    let uri = "file:///iterator-loan.doria";
    let source = "class Cursor implements Iterator<int> { function __construct(borrow List<int> $source) {} function hasCurrent(): bool { return true; } function getCurrent(): int { return $this->source[0]; } writable function advance(): void {} } function main(): void { /* 😀 */ let writable $source = [1]; let $cursor = new Cursor($source); $source->add(2); echo $cursor->getCurrent(); }";
    assert_lsp_diagnostic_matches_compiler("iterator-loan.doria", source, "E0763");
    let diagnostics = diagnostics_for_document(uri, source);
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == "E0763")
        .expect("source-loan conflict");
    let expected = byte_offset_to_position(source, source.rfind("$source->add").unwrap());
    assert_eq!(diagnostic["range"]["start"]["line"], expected.line);
    assert_eq!(
        diagnostic["range"]["start"]["character"],
        expected.character
    );
    assert!(
        code_actions_for_document(uri, source)
            .iter()
            .all(|action| action["edit"].is_null()),
        "ownership changes must not be automatic fixes"
    );
}

#[test]
fn retained_value_escape_presentation_follows_the_compiler() {
    let source = r#"class Cursor implements Iterator<int> {
    function __construct(borrow List<int> $source) {}
    function hasCurrent(): bool { return true; }
    function getCurrent(): int { return $this->source[0]; }
    writable function advance(): void {}
}
function escape(): Cursor {
    let $source = [1];
    let $cursor = new Cursor($source);
    return $cursor;
}
"#;
    assert_lsp_diagnostic_matches_compiler("retained-escape.doria", source, "E0762");
    assert!(
        code_actions_for_document("file:///retained-escape.doria", source)
            .iter()
            .all(|action| action["edit"].is_null()),
        "a lifetime escape must not receive an automatic ownership fix"
    );
}

#[test]
fn consuming_retained_receiver_forwarding_has_no_false_diagnostics() {
    doriac::check_source("consuming-retained.doria", CONSUMING_RETAINED_RECEIVER)
        .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
    let diagnostics = diagnostics_for_document(
        "file:///consuming-retained.doria",
        CONSUMING_RETAINED_RECEIVER,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn consuming_retained_receiver_preserves_both_source_loan_diagnostics() {
    let name = "consuming-retained-loan.doria";
    let uri = format!("file:///{name}");
    for mutation in ["$left[0] = 0;", "$right[0] = 0;"] {
        let source = CONSUMING_RETAINED_RECEIVER
            .replace("echo \"value", &format!("/* 😀 */ {mutation} echo \"value"));
        assert_lsp_diagnostic_matches_compiler(name, &source, "E0763");
        let diagnostics = diagnostics_for_document(&uri, &source);
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic["code"] == "E0763")
            .expect("forwarded iterator must retain each source loan");
        let expected = byte_offset_to_position(&source, source.find(mutation).unwrap());
        assert_eq!(diagnostic["range"]["start"]["line"], expected.line);
        assert_eq!(
            diagnostic["range"]["start"]["character"],
            expected.character
        );
        assert!(code_actions_for_document(&uri, &source)
            .iter()
            .all(|action| action["edit"].is_null()));
    }
}

#[test]
fn consuming_retained_receiver_cannot_return_new_loans_of_owned_storage() {
    let source = CONSUMING_RETAINED_RECEIVER.replace(
        "return $this;",
        "return new Cursor($this->owned, $this->right);",
    );
    assert_lsp_diagnostic_matches_compiler("consuming-retained-escape.doria", &source, "E0762");
    assert!(
        code_actions_for_document("file:///consuming-retained-escape.doria", &source)
            .iter()
            .all(|action| action["edit"].is_null())
    );
}

#[test]
fn explicit_foreach_binding_types_forward_utf16_safe_compiler_fixes() {
    let uri = "file:///explicit-foreach-bindings.doria";
    let source = "function main(): void { /* 😀 */ List<string> $values = [\"a\"]; foreach ($values as $index => $value) {} }";
    let diagnostics = diagnostics_for_document(uri, source);
    let missing_types = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic["code"] == "E0748")
        .collect::<Vec<_>>();
    assert_eq!(missing_types.len(), 2, "{diagnostics:#?}");

    let actions = code_actions_for_document(uri, source);
    for (binding, replacement) in [("$index", "int "), ("$value", "string ")] {
        let binding_offset = source.rfind(binding).expect("foreach binding");
        let expected = byte_offset_to_position(source, binding_offset);
        let diagnostic = missing_types
            .iter()
            .find(|diagnostic| diagnostic["range"]["start"]["character"] == expected.character)
            .unwrap_or_else(|| panic!("missing diagnostic for {binding}: {diagnostics:#?}"));
        assert_eq!(diagnostic["range"]["start"]["line"], expected.line);
        assert_eq!(
            diagnostic["data"]["fixes"][0]["applicability"],
            "machineApplicable"
        );

        let action = actions
            .iter()
            .find(|action| action["edit"]["changes"][uri][0]["newText"] == replacement)
            .unwrap_or_else(|| panic!("missing {replacement:?} action: {actions:#?}"));
        assert_eq!(action["isPreferred"], true);
        assert_eq!(
            action["edit"]["changes"][uri][0]["range"]["start"]["line"],
            expected.line
        );
        assert_eq!(
            action["edit"]["changes"][uri][0]["range"]["start"]["character"],
            expected.character
        );
    }
}
