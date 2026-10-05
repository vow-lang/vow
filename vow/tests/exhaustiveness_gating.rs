//! Regression tests: the exhaustiveness-pass error must gate the build.
//!
//! `Checker::check_expr`'s `ExprKind::Match` arm calls
//! `exhaustiveness::check_exhaustive` directly with the raw emitter, so a
//! `NonExhaustiveMatch` diagnostic reaches `diagnostics[]` without touching
//! the checker's `error_count`. Before the fix, `has_errors()` stayed false,
//! so `vow build` exited 0 (`Unverified`) and produced a binary from a
//! program with a non-exhaustive `match` — a fail-open. These tests pin the
//! fail-closed behavior.

use std::path::PathBuf;
use std::process::Command;

fn vow_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_vow"))
}

/// Compile `src` with `vow build --no-verify` and return (exit_code, json).
fn build_no_verify(src: &str) -> (i32, serde_json::Value) {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("m.vow");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(vow_bin())
        .args([
            "build",
            "--no-verify",
            path.to_str().unwrap(),
            "-o",
            dir.path().join("out").to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON from build: {e}\nstdout: {stdout}"));
    (out.status.code().unwrap_or(-1), json)
}

fn error_codes(json: &serde_json::Value) -> Vec<String> {
    json["diagnostics"]
        .as_array()
        .map(|xs| {
            xs.iter()
                .filter(|x| x["severity"] == "error")
                .filter_map(|x| x["error_code"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// `build --no-verify` of a well-typed program still runs codegen + link, which
/// needs `libvow_runtime.a`. Run standalone (no prior `cargo build --all` or
/// `VOW_RUNTIME_PATH`) the archive is absent and the build reports a link-only
/// `CompileFailed`. The frontend ran to completion regardless; the sibling
/// `effect_gating.rs` tests tolerate this same case.
fn runtime_link_failure(json: &serde_json::Value) -> bool {
    json["status"] == "CompileFailed"
        && json["message"]
            .as_str()
            .is_some_and(|m| m.contains("libvow_runtime.a"))
}

const ENUM_DEF: &str = "enum Color {\n    Red,\n    Green,\n    Blue,\n}\n";

/// Asserts `src` compiles: either the frontend succeeded, or (standalone test
/// runs only) the build got as far as a `libvow_runtime.a`-only link failure.
/// See `runtime_link_failure` for why the latter counts as success here.
fn assert_compiles(src: &str, context: &str) {
    let (exit, json) = build_no_verify(src);
    let status = json["status"].as_str();
    let frontend_success = exit == 0 && matches!(status, Some("Verified" | "Unverified"));
    let link_only_failure = exit != 0 && runtime_link_failure(&json);
    assert!(
        frontend_success || link_only_failure,
        "{context}\nexit: {exit}\njson: {json}"
    );
}

#[test]
fn non_exhaustive_enum_match_fails_build() {
    let src = format!(
        "module M\n\
         {ENUM_DEF}\
         fn describe(c: Color) -> i64 {{\n\
             match c {{\n\
                 Color::Red => 1,\n\
                 Color::Green => 2,\n\
             }}\n\
         }}\n\
         fn main() -> i32 {{ 0 }}\n"
    );
    let (exit, json) = build_no_verify(&src);
    assert_eq!(exit, 1, "a non-exhaustive match must fail the build");
    assert_eq!(json["status"], "CompileFailed");
    assert!(error_codes(&json).contains(&"NonExhaustiveMatch".to_string()));
}

#[test]
fn wildcard_arm_suppresses_non_exhaustive_check() {
    let src = format!(
        "module M\n\
         {ENUM_DEF}\
         fn describe(c: Color) -> i64 {{\n\
             match c {{\n\
                 Color::Red => 1,\n\
                 Color::Green => 2,\n\
                 _ => 3,\n\
             }}\n\
         }}\n\
         fn main() -> i32 {{ 0 }}\n"
    );
    assert_compiles(
        &src,
        "a trailing wildcard arm must suppress NonExhaustiveMatch",
    );
}

#[test]
fn exhaustive_enum_match_compiles() {
    let src = format!(
        "module M\n\
         {ENUM_DEF}\
         fn describe(c: Color) -> i64 {{\n\
             match c {{\n\
                 Color::Red => 1,\n\
                 Color::Green => 2,\n\
                 Color::Blue => 3,\n\
             }}\n\
         }}\n\
         fn main() -> i32 {{ 0 }}\n"
    );
    assert_compiles(&src, "an exhaustive match over all variants must compile");
}

#[test]
fn non_exhaustive_option_match_fails_build() {
    let src = "module M\n\
               fn unwrap_or_zero(value: Option<i64>) -> i64 {\n\
                   match value {\n\
                       Option::Some(inner) => inner,\n\
                   }\n\
               }\n\
               fn main() -> i32 { 0 }\n";
    let (exit, json) = build_no_verify(src);
    assert_eq!(exit, 1, "a non-exhaustive Option match must fail the build");
    assert_eq!(json["status"], "CompileFailed");
    assert!(error_codes(&json).contains(&"NonExhaustiveMatch".to_string()));
}

#[test]
fn option_wildcard_suppresses_non_exhaustive_check() {
    let src = "module M\n\
               fn unwrap_or_zero(value: Option<i64>) -> i64 {\n\
                   match value {\n\
                       Option::Some(inner) => inner,\n\
                       _ => 0,\n\
                   }\n\
               }\n\
               fn main() -> i32 { 0 }\n";
    assert_compiles(
        src,
        "a trailing wildcard arm must suppress NonExhaustiveMatch for Option",
    );
}

#[test]
fn exhaustive_option_match_compiles() {
    let src = "module M\n\
               fn unwrap_or_zero(value: Option<i64>) -> i64 {\n\
                   match value {\n\
                       Option::Some(inner) => inner,\n\
                       Option::None => 0,\n\
                   }\n\
               }\n\
               fn main() -> i32 { 0 }\n";
    assert_compiles(src, "an exhaustive Option match must compile");
}

#[test]
fn non_exhaustive_result_match_fails_build() {
    let src = "module M\n\
               fn unwrap_or_zero(value: Result<i64, i64>) -> i64 {\n\
                   match value {\n\
                       Result::Ok(inner) => inner,\n\
                   }\n\
               }\n\
               fn main() -> i32 { 0 }\n";
    let (exit, json) = build_no_verify(src);
    assert_eq!(exit, 1, "a non-exhaustive Result match must fail the build");
    assert_eq!(json["status"], "CompileFailed");
    assert!(error_codes(&json).contains(&"NonExhaustiveMatch".to_string()));
}

#[test]
fn result_wildcard_suppresses_non_exhaustive_check() {
    let src = "module M\n\
               fn unwrap_or_zero(value: Result<i64, i64>) -> i64 {\n\
                   match value {\n\
                       Result::Ok(inner) => inner,\n\
                       _ => 0,\n\
                   }\n\
               }\n\
               fn main() -> i32 { 0 }\n";
    assert_compiles(
        src,
        "a trailing wildcard arm must suppress NonExhaustiveMatch for Result",
    );
}

#[test]
fn exhaustive_result_match_compiles() {
    let src = "module M\n\
               fn unwrap_or_zero(value: Result<i64, i64>) -> i64 {\n\
                   match value {\n\
                       Result::Ok(inner) => inner,\n\
                       Result::Err(e) => e,\n\
                   }\n\
               }\n\
               fn main() -> i32 { 0 }\n";
    assert_compiles(src, "an exhaustive Result match must compile");
}
