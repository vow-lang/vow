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
    let (exit, json) = build_no_verify(&src);
    let status = json["status"].as_str();
    let frontend_success = exit == 0 && matches!(status, Some("Verified" | "Unverified"));
    let link_only_failure = exit != 0 && runtime_link_failure(&json);
    assert!(
        frontend_success || link_only_failure,
        "a trailing wildcard arm must suppress NonExhaustiveMatch\n\
         exit: {exit}\njson: {json}"
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
    let (exit, json) = build_no_verify(&src);
    let status = json["status"].as_str();
    let frontend_success = exit == 0 && matches!(status, Some("Verified" | "Unverified"));
    let link_only_failure = exit != 0 && runtime_link_failure(&json);
    assert!(
        frontend_success || link_only_failure,
        "an exhaustive match over all variants must compile\n\
         exit: {exit}\njson: {json}"
    );
}
