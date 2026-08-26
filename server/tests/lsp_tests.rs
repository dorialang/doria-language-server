use serde_json::Value;

use doria_language_server::{
    byte_offset_to_position, code_actions_for_document, diagnostics_for_document,
    position_to_byte_offset,
};
use doriac::diagnostics::{DiagnosticSeverity, LabelRole};

fn assert_lsp_diagnostic_matches_compiler(name: &str, source: &str, code: &str) {
    let compiler_diagnostics =
        doriac::check_source(name, source).expect_err("fixture must produce a diagnostic");
    let compiler_diagnostic = compiler_diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("compiler did not report {code}: {compiler_diagnostics:#?}"));
    let compiler_span = compiler_diagnostic
        .labels
        .iter()
        .find(|label| label.role == LabelRole::Primary)
        .or_else(|| compiler_diagnostic.labels.first())
        .map_or(compiler_diagnostic.span, |label| label.span);

    let uri = format!("file:///{name}");
    let lsp_diagnostics = diagnostics_for_document(&uri, source);
    let lsp_diagnostic = lsp_diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == code)
        .unwrap_or_else(|| panic!("language server did not publish {code}: {lsp_diagnostics:#?}"));
    let expected_cause_id = compiler_diagnostic
        .cause_id
        .as_ref()
        .map_or(Value::Null, |cause_id| Value::String(cause_id.clone()));
    let expected_start = byte_offset_to_position(source, compiler_span.start);
    let expected_end = byte_offset_to_position(source, compiler_span.end);

    assert_eq!(lsp_diagnostic["source"], "doriac");
    assert_eq!(
        lsp_diagnostic["severity"],
        match compiler_diagnostic.severity {
            DiagnosticSeverity::Error => 1,
            DiagnosticSeverity::Warning => 2,
            DiagnosticSeverity::Note => 3,
        }
    );
    assert!(lsp_diagnostic["message"]
        .as_str()
        .is_some_and(|message| message.starts_with(&compiler_diagnostic.title)));
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
        .count();
    let lsp_related = lsp_diagnostic["relatedInformation"]
        .as_array()
        .map_or(0, Vec::len);
    assert_eq!(lsp_related, compiler_related);
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
    let compiler = doriac::check_source(name, source)
        .expect_err("reusing the consumed mixed owner must fail checking");
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
        let compiler = doriac::check_source(name, source)
            .expect_err("Stage 30g diagnostic fixture must fail checking");
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
            "borrow-bound-storage.doria",
            "function main(): void { let $value = 1; let $borrowed = fn() with ($value) => $value; List<function(): int> $items = [$borrowed]; }",
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
        let compiler = doriac::check_source(name, source)
            .expect_err("Stage 30c diagnostic fixture must fail checking");
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
            "function main(): void { let $value = 1; let $borrowed = fn() with (/* keep */ $value) => $value; List<function(): int> $items = [$borrowed]; }",
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
    let compiler = doriac::check_source("stage30a-order.doria", source)
        .expect_err("fixture must report the permanent type error");
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
fn preserves_compiler_owned_checked_effect_diagnostics_for_ordinary_callables() {
    let source = "function greet(): void { echo \"value\"; } function main(): void {}";
    assert_lsp_diagnostic_matches_compiler("ordinary-uncovered-io.doria", source, "E0631");
}

#[test]
fn preserves_compiler_owned_checked_effect_diagnostics_for_incomplete_main_clauses() {
    let source = r#"function main(): void throws Doria\Std\Io\IoError
{
    let $line = read_line();
}"#;
    assert_lsp_diagnostic_matches_compiler("incomplete-main-effects.doria", source, "E0631");
}

#[test]
fn publishes_lifecycle_and_finalizer_io_effect_diagnostics() {
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
    let finalizer_diagnostic = finalizer
        .iter()
        .find(|diagnostic| diagnostic["code"] == "E0632")
        .unwrap_or_else(|| panic!("missing finalizer checked-effect diagnostic: {finalizer:#?}"));
    assert!(finalizer_diagnostic["message"]
        .as_str()
        .is_some_and(|message| message.contains("Doria\\Std\\Io\\IoError")));
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
    Doria\Std\Io\InvalidUtf8Error $utf8
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
fn two_clock_static_qualifiers_publish_semantic_not_parser_diagnostics() {
    let parent = diagnostics_for_document(
        "file:///parent.doria",
        "class Child { function save(): void { parent::save(); } }",
    );
    assert_eq!(parent.len(), 1);
    assert_eq!(parent[0]["code"], "E0496");
    assert!(parent[0]["message"]
        .as_str()
        .expect("message")
        .contains("Stage 34"));

    let trait_diagnostics = diagnostics_for_document(
        "file:///trait.doria",
        "trait UsesLimit { function limit(): int { return self::MAX_DEPTH; } }",
    );
    assert_eq!(trait_diagnostics.len(), 1);
    assert_eq!(trait_diagnostics[0]["code"], "E0493");
    assert!(trait_diagnostics[0]["message"]
        .as_str()
        .expect("message")
        .contains("Stage 35"));
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
            .filter(|diagnostic| diagnostic["code"] == "E0671")
            .count(),
        2,
        "both unresolved qualified inheritance names must retain the Stage 31 Slice 2 boundary: {diagnostics:#?}",
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
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == "E0632")
        .unwrap_or_else(|| panic!("missing finalizer diagnostic: {diagnostics:#?}"));
    let finally_offset = finalizer.rfind("finally").expect("finally keyword");
    let expected = byte_offset_to_position(&finalizer, finally_offset);
    assert_eq!(diagnostic["range"]["start"]["line"], expected.line);
    assert_eq!(
        diagnostic["range"]["start"]["character"],
        expected.character
    );
}
