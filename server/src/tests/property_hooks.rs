use super::*;

#[test]
fn blocking_hook_diagnostics_preserve_compiler_ranges_and_related_operations() {
    let uri = "file:///workspace/blocking-hooks.doria";
    for (source, accessor, blocks) in [
        (
            r#"class Value { int $number { get { echo "blocked"; return 1; } } }"#,
            "get",
            true,
        ),
        (
            r#"class Value { writable int $number { set (int $value) { try { write_stderr("blocked"); } catch (Error) {} } } }"#,
            "set",
            true,
        ),
        (
            r#"function output(): int { try { echo "blocked"; } catch (Error) {} return 1; }
            function helper(): int { return output(); }
            class Value { int $number { get => helper(); } }"#,
            "get",
            true,
        ),
        (
            r#"function output(): int { echo "later"; return 1; }
            class Value { function(): int $action { get => fn() => output(); } }"#,
            "get",
            false,
        ),
    ] {
        let (_, compiler) = doriac::analyze_source_for_ide(uri, source).unwrap();
        let diagnostics = diagnostics_for_document(uri, source);
        assert_eq!(
            diagnostics,
            diagnostics_to_lsp(uri, source, &compiler.diagnostics)
        );
        assert!(
            compiler
                .diagnostics
                .iter()
                .all(|finding| finding.code == "E0770"),
            "{:?}",
            compiler.diagnostics
        );
        let findings = compiler
            .diagnostics
            .iter()
            .filter(|finding| finding.code == "E0770")
            .collect::<Vec<_>>();
        assert_eq!(
            findings.len(),
            usize::from(blocks),
            "{source}: {:?}",
            compiler.diagnostics
        );
        for finding in findings {
            assert_eq!(finding.title, "Property Hook May Block");
            assert_eq!(finding.span.start, source.find(accessor).unwrap());
            let diagnostic = diagnostics
                .iter()
                .find(|finding| finding["code"] == "E0770")
                .unwrap();
            assert_eq!(diagnostic["range"], span_to_range(source, finding.span));
            assert!(
                diagnostic["relatedInformation"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|related| {
                        related["message"] == "Potentially Blocking Operation"
                            && related["location"]["uri"] == uri
                            && related["location"]["range"] != diagnostic["range"]
                    }),
                "{diagnostic}"
            );
        }
    }
}

#[test]
fn setter_type_diagnostics_preserve_resolved_aliases_and_utf16_ranges() {
    let uri = "file:///workspace/setter-types.doria";
    for owner in ["class", "trait", "interface"] {
        for (property_type, parameter_type, accepted) in [
            ("float", "string", false),
            ("?int", "int", false),
            ("int8", "int16", false),
            ("T", "int", false),
            ("int", "int64", true),
            ("float", "float64", true),
            ("T", "T", true),
        ] {
            let body = if owner == "interface" { ";" } else { " {}" };
            let parameter = format!("{parameter_type} $value");
            let source = format!(
                "/* \u{1f600} */ {owner} Sample<T> {{ writable {property_type} $amount {{ set ({parameter}){body} }} }}"
            );
            let (_, compiler) = doriac::analyze_source_for_ide(uri, &source).unwrap();
            let diagnostics = diagnostics_for_document(uri, &source);
            assert_eq!(
                diagnostics,
                diagnostics_to_lsp(uri, &source, &compiler.diagnostics),
                "{source}"
            );
            if accepted {
                assert!(
                    compiler.diagnostics.is_empty(),
                    "{source}: {:?}",
                    compiler.diagnostics
                );
                continue;
            }
            assert!(
                compiler
                    .diagnostics
                    .iter()
                    .all(|finding| finding.code == "E0769"),
                "{source}: {:?}",
                compiler.diagnostics
            );
            let finding = compiler
                .diagnostics
                .iter()
                .find(|finding| finding.code == "E0769")
                .unwrap_or_else(|| panic!("{source}: {:?}", compiler.diagnostics));
            assert_eq!(finding.title, "Setter Input Type Must Match The Property");
            assert_eq!(&source[finding.span.start..finding.span.end], parameter);
            let diagnostic = diagnostics
                .iter()
                .find(|finding| finding["code"] == "E0769")
                .unwrap();
            let start = source.find(&parameter).unwrap();
            assert_eq!(diagnostic["range"]["start"]["line"], 0);
            assert_eq!(
                diagnostic["range"]["start"]["character"],
                source[..start].encode_utf16().count()
            );
            assert_eq!(
                diagnostic["range"]["end"]["character"],
                source[..start + parameter.len()].encode_utf16().count()
            );
            let property = source.find("$amount").unwrap();
            assert!(diagnostic["relatedInformation"]
                .as_array()
                .unwrap()
                .iter()
                .any(|related| {
                    related["location"]["uri"] == uri
                        && related["location"]["range"]
                            == span_to_range(
                                &source,
                                Span::new(property, property + "$amount".len()),
                            )
                }));
        }
    }
}

#[test]
fn borrowed_callback_getter_rejects_a_new_owned_result_with_compiler_diagnostics() {
    let source = r#"
class Example {
    int $value = 1;
    function(): int $callback {
        borrowed get => fn() with ($this) => $this->value;
    }
}
"#;
    let uri = "file:///workspace/callback-hook.doria";
    let (_, compiler) = doriac::analyze_source_for_ide(uri, source).unwrap();
    assert!(
        compiler
            .diagnostics
            .iter()
            .all(|finding| finding.code == "E0766"),
        "{:?}",
        compiler.diagnostics
    );
    let diagnostic = compiler
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "E0766")
        .expect("borrowed get lends an existing callback, not a newly owned carrier");
    assert_eq!(
        &source[diagnostic.span.start..diagnostic.span.end],
        "borrowed"
    );
    assert_eq!(
        diagnostics_for_document(uri, source),
        diagnostics_to_lsp(uri, source, &compiler.diagnostics)
    );
}

#[test]
fn property_hook_protocol_preserves_cross_file_origins_and_unsaved_updates() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("doria-lsp-property-hooks-{nonce}"));
    fs::create_dir_all(root.join("src")).unwrap();
    let declaration_path = root.join("src/Counter.doria");
    let usage_path = root.join("src/main.doria");
    let declaration = "class Counter { writable int $value = 0 { get => $this->value; set (int $next) => $this->value = $next; } }\n";
    let usage = "function inspect(writable Counter $counter): void { echo $counter->value; $counter->value += 1; }\n";
    fs::write(&declaration_path, declaration).unwrap();
    fs::write(&usage_path, usage).unwrap();
    let root_uri = file_uri::path_to_file_uri(&root);
    let declaration_uri = file_uri::path_to_file_uri(&declaration_path.canonicalize().unwrap());
    let usage_uri = file_uri::path_to_file_uri(&usage_path.canonicalize().unwrap());
    let mut server = stage31_server(&[&root_uri]);
    server.projects.insert(
        root_uri,
        project::test_project(
            &root,
            &["src/Counter.doria", "src/main.doria"],
            project::PackageSource::Path,
            &[],
        ),
    );
    open_stage31_document(&mut server, &usage_uri, usage);
    assert!(server.project_documents.contains_key(&declaration_uri));
    let read = usage.find("value").unwrap();
    let update = usage.rfind("value").unwrap();
    for (offset, expected) in [(read, vec!["get =>"]), (update, vec!["get =>", "set (int"])] {
        let definition = server.definition(Some(&params_at(&usage_uri, usage, offset)));
        let locations = definition.as_array().expect("accessor locations");
        assert_eq!(locations.len(), expected.len(), "{definition}");
        for needle in expected {
            let start = params_at(
                &declaration_uri,
                declaration,
                declaration.find(needle).unwrap(),
            )["position"]
                .clone();
            assert!(
                locations
                    .iter()
                    .any(|location| location["uri"] == declaration_uri
                        && location["range"]["start"] == start),
                "{definition}"
            );
        }
    }
    let hover = server
        .hover(Some(&params_at(&usage_uri, usage, update)))
        .unwrap();
    let markdown = hover["contents"]["value"].as_str().unwrap();
    assert!(markdown.contains("get: int"), "{markdown}");
    assert!(markdown.contains("set(int $next): void"), "{markdown}");
    for uri in [&declaration_uri, &usage_uri] {
        let diagnostics = server.document(uri).unwrap().analysis.diagnostics();
        assert!(diagnostics.is_empty(), "{uri}: {diagnostics:?}");
    }

    // Changing only the unsaved declaration must refresh the other document.
    let changed = declaration
        .replace("int $value = 0", "string $value = \"zero\"")
        .replace("int $next", "string $next");
    open_stage31_document(&mut server, &declaration_uri, &changed);
    let hover = server
        .hover(Some(&params_at(&usage_uri, usage, read)))
        .unwrap();
    assert!(
        hover["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("get: string"),
        "{hover}"
    );
    let definition = server.definition(Some(&params_at(&usage_uri, usage, read)));
    assert_eq!(
        definition[0]["range"]["start"],
        params_at(&declaration_uri, &changed, changed.find("get =>").unwrap())["position"]
    );
    open_stage31_document(&mut server, &declaration_uri, declaration);
    let restored = server
        .hover(Some(&params_at(&usage_uri, usage, read)))
        .unwrap();
    assert!(restored["contents"]["value"]
        .as_str()
        .unwrap()
        .contains("get: int"));
    for uri in [&declaration_uri, &usage_uri] {
        let diagnostics = server.document(uri).unwrap().analysis.diagnostics();
        assert!(diagnostics.is_empty(), "{uri}: {diagnostics:?}");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn property_hook_protocol_preserves_merged_interface_origins_without_method_names() {
    let source = r#"
interface Left<T> { T $value { get; } }
interface Right<T> { T $value { get; } }
interface Both extends Left<int>, Right<int> {}
function read(Both $both): int { return $both->value; }
function generic<T implements Both>(T $both): int { return $both->value; }
"#;
    let uri = "file:///workspace/property-hooks.doria";
    let mut server = stage31_server(&["file:///workspace"]);
    open_stage31_document(&mut server, uri, source);
    let diagnostics = server.document(uri).unwrap().analysis.diagnostics();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    for (offset, _) in source.match_indices("$both->value") {
        let offset = offset + "$both->".len();
        let definition = server.definition(Some(&params_at(uri, source, offset)));
        let locations = definition.as_array().unwrap();
        assert_eq!(locations.len(), 2, "{definition}");
        for (getter, _) in source.match_indices("get;") {
            assert!(
                locations.iter().any(|location| location["uri"] == uri
                    && location["range"]["start"] == params_at(uri, source, getter)["position"]),
                "{definition}"
            );
        }
        let hover = server.hover(Some(&params_at(uri, source, offset))).unwrap();
        assert!(
            hover["contents"]["value"]
                .as_str()
                .unwrap()
                .contains("get: int"),
            "{hover}"
        );
    }
    for (offset, _) in source.match_indices("get;") {
        assert_eq!(
            server.rename(Some(&json!({
                "textDocument": { "uri": uri },
                "position": params_at(uri, source, offset)["position"],
                "newName": "renamed"
            }))),
            Value::Null,
            "an accessor keyword must not be renamed"
        );
    }
}
