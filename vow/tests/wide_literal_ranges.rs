use std::fs;
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

fn assert_compile_error_code(name: &str, expected_error_code: &str) {
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
        error_codes.contains(&expected_error_code),
        "{name} must report {expected_error_code}\nerror codes: {error_codes:?}\njson: {json}"
    );
    assert!(
        !output.exists(),
        "{name} must not produce an executable after a frontend error"
    );
}

fn assert_literal_out_of_range(name: &str) {
    assert_compile_error_code(name, "LiteralOutOfRange");
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
fn i64_nested_match_rejects_wide_literal_before_compatibility_storage() {
    assert_literal_out_of_range("i64_nested_match_wide_literal_out_of_range.vow");
}

#[test]
fn i64_nested_loop_rejects_wide_literal_before_compatibility_storage() {
    assert_literal_out_of_range("i64_nested_loop_break_wide_literal_out_of_range.vow");
}

#[test]
fn i128_option_payload_rejects_overflowing_literal() {
    assert_literal_out_of_range("i128_option_payload_literal_out_of_range.vow");
}

#[test]
fn i64_option_payload_rejects_wide_literal_before_compatibility_storage() {
    assert_literal_out_of_range("i64_option_payload_wide_literal_out_of_range.vow");
}

#[test]
fn i64_const_rejects_wide_literal_before_compatibility_storage() {
    assert_literal_out_of_range("i64_const_wide_literal_out_of_range.vow");
}

#[test]
fn inferred_option_payload_defaults_to_i64_before_storage() {
    assert_literal_out_of_range("i64_inferred_option_wide_literal_out_of_range.vow");
}

#[test]
fn vec_truncate_rejects_wide_i64_argument_before_lowering() {
    assert_literal_out_of_range("i64_vec_truncate_wide_literal_out_of_range.vow");
}

#[test]
fn literal_only_comparison_defaults_operands_to_i64() {
    assert_literal_out_of_range("i64_literal_comparison_wide_literal_out_of_range.vow");
}

#[test]
fn index_rejects_wide_i64_operand_before_lowering() {
    assert_literal_out_of_range("i64_index_wide_literal_out_of_range.vow");
}

#[test]
fn string_match_rejects_wide_i64_position_before_lowering() {
    assert_literal_out_of_range("i64_string_match_wide_literal_out_of_range.vow");
}

#[test]
fn string_raw_parts_rejects_wide_i64_operand_before_lowering() {
    assert_literal_out_of_range("i64_string_raw_parts_wide_literal_out_of_range.vow");
}

#[test]
fn vec_raw_parts_rejects_wide_i64_operand_before_lowering() {
    assert_literal_out_of_range("i64_vec_raw_parts_wide_literal_out_of_range.vow");
}

#[test]
fn u128_context_rejects_compound_unary_negation() {
    assert_compile_error_code("u128_compound_negation.vow", "TypeMismatch");
}

#[test]
fn i128_let_binding_rejects_out_of_range_literal() {
    assert_literal_out_of_range("i128_literal_out_of_range.vow");
}

#[test]
fn i128_let_binding_rejects_literal_one_below_the_minimum() {
    assert_literal_out_of_range("i128_literal_below_range.vow");
}

#[test]
fn u128_let_binding_rejects_negative_literal() {
    assert_literal_out_of_range("u128_literal_out_of_range.vow");
}

/// The full-width admission boundaries: `i128::MIN` is one larger in magnitude
/// than `i128::MAX`, and `u128::MAX` occupies both limbs. A bound that models
/// the negative side as `i128::MAX` (or the positive side as anything narrower)
/// rejects these, so assert they reach the IR intact.
#[test]
fn full_width_boundary_literals_are_admitted() {
    let dir = tempfile::TempDir::new().unwrap();
    let source_path = dir.path().join("wide_boundaries.vow");
    fs::write(
        &source_path,
        r#"module WideBoundaries

fn i128_minimum() -> i128 {
    -170141183460469231731687303715884105728
}

fn i128_maximum() -> i128 {
    170141183460469231731687303715884105727
}

fn u128_maximum() -> u128 {
    340282366920938463463374607431768211455
}

fn u64_above_i64_max() -> u64 {
    18446744073709551615
}

fn i64_minimum() -> i64 {
    -9223372036854775808
}
"#,
    )
    .unwrap();

    let output = Command::new(vow_bin())
        .args([
            "build",
            "--no-verify",
            "--dump-ir",
            source_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "full-width boundary literals must be admitted\nstdout: {stdout}\nstderr: {stderr}"
    );
    for expected in [
        "ConstI128[-170141183460469231731687303715884105728i128]",
        "ConstI128[170141183460469231731687303715884105727i128]",
        "ConstU128[340282366920938463463374607431768211455u128]",
    ] {
        assert!(
            stdout.contains(expected),
            "missing {expected} in IR:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains("LiteralOutOfRange") && !stderr.contains("LiteralOutOfRange"),
        "boundary literals must not be range-rejected\nstdout: {stdout}\nstderr: {stderr}"
    );
}
