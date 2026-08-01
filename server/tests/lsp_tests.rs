use serde_json::Value;

use doria_language_server::{
    byte_offset_to_position, code_actions_for_document, diagnostics_for_document,
    position_to_byte_offset,
};

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
    let source = r#"function main(): void { echo "😀"; String::contains("text", 1); }"#;
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

function inspect(SharedReference<Node> $node): void
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
    echo "😀";
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
        r#"let writable $count = 0;

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
"#,
    );

    assert_eq!(diagnostics, Vec::<Value>::new());
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
