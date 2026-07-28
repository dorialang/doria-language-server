use std::process::Command;

#[test]
fn reports_server_and_canonical_toolchain_versions() {
    let output = Command::new(env!("CARGO_BIN_EXE_doria-lsp"))
        .arg("--version")
        .output()
        .expect("doria-lsp should run");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("version output should be UTF-8"),
        "doria-lsp 2026.3.1-canary (Doria 2026.03.1-canary)\n",
    );
}

#[test]
fn rejects_unknown_arguments_without_starting_stdio_transport() {
    let output = Command::new(env!("CARGO_BIN_EXE_doria-lsp"))
        .arg("--unknown")
        .output()
        .expect("doria-lsp should run");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(output.stderr).expect("error output should be UTF-8"),
        "unknown argument: --unknown\n",
    );
}
