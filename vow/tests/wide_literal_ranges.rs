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

fn assert_literal_out_of_range(name: &str) {
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
        error_codes.contains(&"LiteralOutOfRange"),
        "{name} must report LiteralOutOfRange\nerror codes: {error_codes:?}\njson: {json}"
    );
    assert!(
        !output.exists(),
        "{name} must not produce an executable after a frontend error"
    );
}

#[test]
fn implicit_i128_return_rejects_out_of_range_literal() {
    assert_literal_out_of_range("i128_body_literal_out_of_range.vow");
}

#[test]
fn explicit_i128_return_rejects_out_of_range_literal() {
    assert_literal_out_of_range("i128_return_literal_out_of_range.vow");
}

#[test]
fn u128_assignment_rejects_negative_literal() {
    assert_literal_out_of_range("u128_assignment_literal_below_range.vow");
}

#[test]
fn default_i64_return_rejects_wide_magnitude() {
    assert_literal_out_of_range("i64_body_wide_literal_out_of_range.vow");
}

#[test]
fn u128_context_rejects_negative_literal_inside_binary_marker() {
    assert_literal_out_of_range("u128_binary_literal_below_range.vow");
}

#[test]
fn i128_context_rejects_overflowing_literal_inside_binary_marker() {
    assert_literal_out_of_range("i128_binary_wide_literal_out_of_range.vow");
}

#[test]
fn u128_struct_field_rejects_negative_literal() {
    assert_literal_out_of_range("u128_struct_field_literal_below_range.vow");
}

#[test]
fn i128_enum_payload_rejects_overflowing_literal() {
    assert_literal_out_of_range("i128_enum_payload_literal_out_of_range.vow");
}

#[test]
fn i128_match_arm_rejects_overflowing_literal() {
    assert_literal_out_of_range("i128_match_literal_out_of_range.vow");
}

#[test]
fn i64_match_arm_rejects_wide_literal_before_compatibility_storage() {
    assert_literal_out_of_range("i64_match_wide_literal_out_of_range.vow");
}

#[test]
fn i64_loop_break_rejects_wide_literal_before_compatibility_storage() {
    assert_literal_out_of_range("i64_loop_break_wide_literal_out_of_range.vow");
}

#[test]
fn i128_option_payload_rejects_overflowing_literal() {
    assert_literal_out_of_range("i128_option_payload_literal_out_of_range.vow");
}

#[test]
fn i64_const_rejects_wide_literal_before_compatibility_storage() {
    assert_literal_out_of_range("i64_const_wide_literal_out_of_range.vow");
}
