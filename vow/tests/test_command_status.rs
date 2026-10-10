//! Integration coverage for `vow test` verdicts that never depend on a test
//! binary passing: a nonexistent path and a hung test must both fail closed
//! (#415).

use std::path::PathBuf;
use std::process::Command;

fn vow_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_vow"))
}

fn parse_json(stdout: &str) -> serde_json::Value {
    serde_json::from_str(stdout)
        .unwrap_or_else(|e| panic!("invalid JSON from vow: {e}\nstdout: {stdout}"))
}

#[test]
fn missing_test_path_reports_tests_failed_with_one_failed_entry() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does_not_exist.vow");

    let out = Command::new(vow_bin())
        .arg("test")
        .arg(&missing)
        .output()
        .expect("failed to run vow");

    assert_eq!(out.status.code(), Some(1));
    let json = parse_json(&String::from_utf8_lossy(&out.stdout));
    assert_eq!(json["status"], "TestsFailed");
    assert_eq!(json["total"], 1);
    assert_eq!(json["passed"], 0);
    assert_eq!(json["failed"], 1);
    assert_eq!(json["tests"].as_array().map(Vec::len), Some(1));
    assert_eq!(json["tests"][0]["status"], "failed");
    assert_eq!(json["tests"][0]["file"], missing.to_string_lossy().as_ref());
    assert!(
        json["tests"][0]["stderr"]
            .as_str()
            .is_some_and(|s| s.contains("does_not_exist.vow"))
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("does not exist"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn hung_test_is_reported_as_failed_not_passed() {
    let dir = tempfile::tempdir().unwrap();
    let hang = dir.path().join("test_hang.vow");
    std::fs::write(
        &hang,
        "module M\nfn main() -> i32 [io] {\n    while true {\n    }\n    0\n}\n",
    )
    .unwrap();

    let out = Command::new(vow_bin())
        .args(["test", "--timeout", "1000"])
        .arg(&hang)
        .current_dir(dir.path())
        .output()
        .expect("failed to run vow");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let json = parse_json(&stdout);
    if json["tests"][0]["status"] == "compile_error"
        && json["tests"][0]["stderr"]
            .as_str()
            .is_some_and(|m| m.contains("libvow_runtime.a"))
    {
        return;
    }
    assert_eq!(out.status.code(), Some(1), "stdout: {stdout}");
    assert_eq!(json["status"], "TestsFailed");
    assert_eq!(json["tests"][0]["status"], "timeout");
    assert_eq!(json["failed"], 1);
}
