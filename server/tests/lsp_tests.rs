use serde_json::Value;

use doria_language_server::{
    byte_offset_to_position, code_actions_for_document, diagnostics_for_document,
    position_to_byte_offset,
};
use doriac::diagnostics::LabelRole;

fn assert_stage_30_closure_boundary(name: &str, source: &str) {
    doriac::parse_source(name, source).expect("accepted closure grammar must parse");
    let compiler_diagnostics =
        doriac::check_source(name, source).expect_err("closure semantics remain a boundary");
    assert_eq!(
        compiler_diagnostics.len(),
        1,
        "compiler diagnostic cascade for {name}: {compiler_diagnostics:#?}"
    );
    let compiler_diagnostic = &compiler_diagnostics[0];
    assert_eq!(compiler_diagnostic.code, "E0641");
    let compiler_span = compiler_diagnostic
        .labels
        .iter()
        .find(|label| label.role == LabelRole::Primary)
        .map_or(compiler_diagnostic.span, |label| label.span);

    let uri = format!("file:///{name}");
    let diagnostics = diagnostics_for_document(&uri, source);
    assert_eq!(
        diagnostics.len(),
        1,
        "LSP diagnostic cascade for {name}: {diagnostics:#?}"
    );
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic["code"], "E0641");
    assert_eq!(diagnostic["data"]["kind"], "unsupportedDevelopmentSurface");
    assert_eq!(diagnostic["data"]["developmentOnly"], true);
    assert!(diagnostic["message"]
        .as_str()
        .is_some_and(|message| message.starts_with("Closure Semantics Await Stage 30")));
    let expected_start = byte_offset_to_position(source, compiler_span.start);
    let expected_end = byte_offset_to_position(source, compiler_span.end);
    assert_eq!(diagnostic["range"]["start"]["line"], expected_start.line);
    assert_eq!(
        diagnostic["range"]["start"]["character"],
        expected_start.character
    );
    assert_eq!(diagnostic["range"]["end"]["line"], expected_end.line);
    assert_eq!(
        diagnostic["range"]["end"]["character"],
        expected_end.character
    );
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
fn publishes_one_structured_boundary_for_every_accepted_closure_form() {
    let cases = [
        (
            "arrow.doria",
            "let $double = fn(int $value) => $value * 2; let $after = 1;",
        ),
        (
            "readonly-capture.doria",
            "let $minimum = 70; let $passes = fn(int $score) with ($minimum) => $score >= $minimum;",
        ),
        (
            "block.doria",
            "let $positive = function (int $value): bool { return $value > 0; };",
        ),
        (
            "block-capture.doria",
            "let $minimum = 70; let $passes = function (int $score): bool with ($minimum) { return $score >= $minimum; };",
        ),
        (
            "function-type.doria",
            "function accept(function(int): int $callback): void {}",
        ),
        (
            "nested.doria",
            "let $nested = fn(int $outer) => fn(int $inner) => $outer + $inner;",
        ),
        (
            "argument.doria",
            "function consume(mixed $value): void {} consume(fn(string $label) => $label);",
        ),
    ];

    for (name, source) in cases {
        assert_stage_30_closure_boundary(name, source);
    }
}

#[test]
fn closure_boundary_suppresses_body_cascades_and_stays_off_following_source() {
    let source = "let $closure = fn(int $value) => $missing + $value; let $after = 1;";
    assert_stage_30_closure_boundary("closure-body-boundary.doria", source);

    let diagnostic = diagnostics_for_document("file:///closure-body-boundary.doria", source)
        .into_iter()
        .next()
        .expect("closure boundary");
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
        r#"function main(): void throws Doria\Std\Io\IoError
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
        "function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { let $line = read_line(); }",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn accepts_prompted_read_line_without_false_diagnostics() {
    let diagnostics = diagnostics_for_document(
        "file:///input.doria",
        "function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { let $line = read_line(\"Name: \"); }",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn publishes_canonical_checked_io_effect_diagnostics() {
    let echo = diagnostics_for_document(
        "file:///unchecked-echo.doria",
        "function main(): void { echo \"value\"; }",
    );
    let echo_diagnostic = echo
        .iter()
        .find(|diagnostic| diagnostic["code"] == "E0631")
        .unwrap_or_else(|| panic!("missing uncovered echo effect: {echo:#?}"));
    assert!(echo_diagnostic["message"]
        .as_str()
        .is_some_and(|message| message.contains("Doria\\Std\\Io\\IoError")));

    let input = diagnostics_for_document(
        "file:///unchecked-input.doria",
        "function main(): void { let $line = read_line(); }",
    );
    let input_diagnostic = input
        .iter()
        .find(|diagnostic| diagnostic["code"] == "E0631")
        .unwrap_or_else(|| panic!("missing uncovered input effects: {input:#?}"));
    let message = input_diagnostic["message"]
        .as_str()
        .expect("input diagnostic message");
    assert!(message.contains("Doria\\Std\\Io\\IoError"));
    assert!(message.contains("Doria\\Std\\Io\\InvalidUtf8Error"));
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
        r#"function main(): void throws Doria\Std\Io\IoError
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
        r#"function main(): void throws Doria\Std\Io\IoError
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
    let source = r#"function main(): void throws Doria\Std\Io\IoError
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

function main(): void throws Doria\Std\Io\IoError
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
    function main(): void throws Doria\Std\Io\IoError
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
        r#"function main(): void throws Doria\Std\Io\IoError
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

function main(): void throws Doria\Std\Io\IoError
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

function main(): void throws Doria\Std\Io\IoError
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

function main(): void throws Doria\Std\Io\IoError
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
        r#"function main(): void throws Doria\Std\Io\IoError
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
        r#"function main(): void throws Doria\Std\Io\IoError
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
    for code in ["E0475", "E0476", "E0464"] {
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic["code"] == code));
    }
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
