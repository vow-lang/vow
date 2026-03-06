use std::path::PathBuf;
use std::process::Command;

fn vow_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_vow"))
}

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("examples")
}

#[test]
fn help_json_valid() {
    let out = Command::new(vow_bin())
        .args(["--help"])
        .output()
        .expect("failed to run vow");
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON from --help: {e}\nstdout: {stdout}"));
    assert!(json.get("tool").is_some(), "expected 'tool' key in JSON");
}

#[test]
fn build_help_json_valid() {
    let out = Command::new(vow_bin())
        .args(["build", "--help"])
        .output()
        .expect("failed to run vow");
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON from build --help: {e}\nstdout: {stdout}"));
    assert!(json.get("tool").is_some(), "expected 'tool' key in JSON");
}

#[test]
fn compile_success() {
    let dir = tempfile::TempDir::new().unwrap();
    let out_path = dir.path().join("hello");
    let result = Command::new(vow_bin())
        .args([
            "build",
            "--no-verify",
            "--no-cache",
            examples_dir().join("hello.vow").to_str().unwrap(),
            "-o",
            out_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    assert_eq!(result.status.code(), Some(0), "expected exit 0");
    let stdout = String::from_utf8_lossy(&result.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON: {e}\nstdout: {stdout}"));
    assert_eq!(json["status"], "Unverified");
    assert!(out_path.exists(), "expected executable at {out_path:?}");

    let run = Command::new(&out_path)
        .output()
        .expect("failed to run compiled binary");
    assert_eq!(run.status.code(), Some(0), "compiled binary should exit 0");
}

#[test]
fn compile_type_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let bad_src = "module Bad\nfn main() -> i32 { true }";
    let src_path = dir.path().join("bad.vow");
    std::fs::write(&src_path, bad_src).unwrap();

    let result = Command::new(vow_bin())
        .args([
            "build",
            "--no-verify",
            "--no-cache",
            src_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    assert_eq!(result.status.code(), Some(1), "expected exit 1");
    let stdout = String::from_utf8_lossy(&result.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON: {e}\nstdout: {stdout}"));
    assert_eq!(json["status"], "CompileFailed");
    let diags = json["diagnostics"].as_array().expect("diagnostics array");
    assert!(!diags.is_empty(), "expected non-empty diagnostics");
}

#[test]
#[ignore]
fn verify_success() {
    let result = Command::new(vow_bin())
        .args([
            "build",
            "--no-cache",
            examples_dir().join("cegis_fixed.vow").to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    assert_eq!(result.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&result.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON: {e}\nstdout: {stdout}"));
    assert_eq!(json["status"], "Verified");
}

#[test]
#[ignore]
fn verify_failure_with_counterexample() {
    let result = Command::new(vow_bin())
        .args([
            "build",
            "--no-cache",
            examples_dir().join("cegis_broken.vow").to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    assert_eq!(result.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&result.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON: {e}\nstdout: {stdout}"));
    assert_eq!(json["status"], "VerifyFailed");
    let ces = json["counterexamples"]
        .as_array()
        .expect("counterexamples array");
    assert!(!ces.is_empty(), "expected non-empty counterexamples");
    let first = &ces[0];
    assert!(
        first.get("values").is_some(),
        "expected values in counterexample"
    );
}

#[test]
fn debug_trace_emits_json_lines() {
    let dir = tempfile::TempDir::new().unwrap();
    let out_path = dir.path().join("hello_trace");
    let result = Command::new(vow_bin())
        .args([
            "build",
            "--debug-trace=calls",
            "--no-verify",
            "--no-cache",
            examples_dir().join("hello.vow").to_str().unwrap(),
            "-o",
            out_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    assert_eq!(result.status.code(), Some(0), "expected exit 0");
    assert!(out_path.exists(), "expected compiled binary");

    let run = Command::new(&out_path)
        .output()
        .expect("failed to run traced binary");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(
        stderr.contains(r#""event":"enter""#),
        "expected enter trace in stderr, got: {stderr}"
    );
    assert!(
        stderr.contains(r#""event":"exit""#),
        "expected exit trace in stderr, got: {stderr}"
    );
}
