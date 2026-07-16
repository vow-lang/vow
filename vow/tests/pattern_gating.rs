use std::path::{Path, PathBuf};
use std::process::Command;

fn vow_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_vow"))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/error")
        .join(name)
}

fn assert_unsupported_pattern(name: &str) {
    let dir = tempfile::TempDir::new().unwrap();
    let output = dir.path().join("out");
    let command_output = Command::new(vow_bin())
        .args([
            "build",
            "--no-verify",
            fixture(name).to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    let stdout = String::from_utf8_lossy(&command_output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("invalid JSON from build: {error}\nstdout: {stdout}"));
    let error_codes: Vec<&str> = json["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|diagnostic| diagnostic["severity"] == "error")
        .filter_map(|diagnostic| diagnostic["error_code"].as_str())
        .collect();

    assert_eq!(
        command_output.status.code(),
        Some(1),
        "{name} must fail before codegen\njson: {json}"
    );
    assert_eq!(json["status"], "CompileFailed");
    assert!(
        error_codes.contains(&"UnsupportedPattern"),
        "{name} must report UnsupportedPattern\nerror codes: {error_codes:?}\njson: {json}"
    );
    assert!(
        !output.exists(),
        "{name} must not produce an executable after a frontend error"
    );
}

#[test]
fn literal_patterns_fail_closed() {
    assert_unsupported_pattern("match_integer_literal_pattern.vow");
    assert_unsupported_pattern("match_bool_literal_pattern.vow");
}

#[test]
fn parsed_unsupported_patterns_fail_closed() {
    assert_unsupported_pattern("match_parsed_unsupported_patterns.vow");
}

#[test]
fn scalar_catchall_patterns_fail_closed() {
    assert_unsupported_pattern("match_scalar_catchall_pattern.vow");
}
