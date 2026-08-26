use std::process::Command;

const REQUIRED_COMPILER_COMMIT: &str = "8397e0e390003e7c91534a0a2fc802340df57225";

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
fn reports_machine_readable_compiler_identity() {
    let output = Command::new(env!("CARGO_BIN_EXE_doria-lsp"))
        .args(["--version", "--json"])
        .output()
        .expect("doria-lsp should run");

    assert!(output.status.success());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("version output should be JSON");
    assert_eq!(value["schema"], 1);
    assert_eq!(value["component"], "doria-lsp");
    assert_eq!(value["version"], "2026.3.1-canary");
    assert_eq!(value["toolchainVersion"], "2026.03.1-canary");
    assert_eq!(doriac::BUILD_COMMIT, REQUIRED_COMPILER_COMMIT);
    assert_eq!(value["compilerCommit"], REQUIRED_COMPILER_COMMIT);
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
