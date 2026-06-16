//! Regression tests: effect-pass and linear-pass errors must gate the build.
//!
//! `Checker::check_module` runs the effect and linear-usage passes
//! (`effects::check_fn_effects`, `linear::check_linear_usage`), which emit
//! diagnostics directly to the emitter without touching the checker's
//! `error_count`. Before the fix, those errors were reported but
//! `has_errors()` stayed false, so `vow build` exited 0 (`Unverified`) and
//! produced a binary from a program that violates Vow's effect discipline —
//! a fail-open. These tests pin the fail-closed behavior.

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
/// `region_summary_equivalence.rs` tests tolerate this same case.
fn runtime_link_failure(json: &serde_json::Value) -> bool {
    json["status"] == "CompileFailed"
        && json["message"]
            .as_str()
            .is_some_and(|m| m.contains("libvow_runtime.a"))
}

#[test]
fn effectful_call_from_pure_body_fails_build() {
    let (exit, json) = build_no_verify(
        "module M\n\
         fn se() -> i64 [io] { print_i64(0); 0 }\n\
         fn pure_caller() -> i64 { se() }\n\
         fn main() -> i32 [io] { 0 }\n",
    );
    assert_eq!(exit, 1, "an effect violation must fail the build");
    assert_eq!(json["status"], "CompileFailed");
    assert!(error_codes(&json).contains(&"EffectViolation".to_string()));
}

#[test]
fn impure_contract_clause_fails_build() {
    let (exit, json) = build_no_verify(
        "module M\n\
         fn se() -> i64 [io] { print_i64(0); 0 }\n\
         fn f(x: i64) -> i64 vow { requires: se() == 0 } { x }\n\
         fn main() -> i32 [io] { 0 }\n",
    );
    assert_eq!(exit, 1, "an impure contract clause must fail the build");
    assert_eq!(json["status"], "CompileFailed");
    assert!(error_codes(&json).contains(&"EffectViolation".to_string()));
}

#[test]
fn linear_double_consume_fails_build() {
    // `double_consume` passes the linear `h` to `use_handle` twice; the second
    // call is an "already consumed" linear violation. Before the fix the linear
    // pass emitted this to the raw emitter without touching `error_count`, so
    // the build exited 0. `use_handle` does not error: field access does not
    // consume, and the never-consumed backstop exempts linear params.
    let (exit, json) = build_no_verify(
        "module M\n\
         linear struct Handle { fd: i64 }\n\
         fn use_handle(h: Handle) -> i64 { h.fd }\n\
         fn double_consume(h: Handle) -> i64 { use_handle(h) + use_handle(h) }\n\
         fn main() -> i32 [io] { 0 }\n",
    );
    assert_eq!(exit, 1, "a linear double-consume must fail the build");
    assert_eq!(json["status"], "CompileFailed");
    assert!(error_codes(&json).contains(&"LinearTypeViolation".to_string()));
}

#[test]
fn valid_effect_discipline_compiles() {
    // No false positive: an [io] function may call an [io] function.
    let (exit, json) = build_no_verify(
        "module M\n\
         fn se() -> i64 [io] { print_i64(0); 0 }\n\
         fn io_caller() -> i64 [io] { se() }\n\
         fn main() -> i32 [io] { 0 }\n",
    );
    // A valid program proceeds through codegen + link; the effect pass passing
    // is what this test pins. Tolerate a link-only failure when the runtime
    // archive is absent (standalone `cargo test`), matching the convention in
    // region_summary_equivalence.rs, so the regression test is reliable
    // regardless of prior build state.
    let status = json["status"].as_str();
    let frontend_success = exit == 0 && matches!(status, Some("Verified" | "Unverified"));
    let link_only_failure = exit != 0 && runtime_link_failure(&json);
    assert!(
        frontend_success || link_only_failure,
        "well-typed effect usage must compile (or fail only on a missing runtime archive)\n\
         exit: {exit}\njson: {json}"
    );
}
