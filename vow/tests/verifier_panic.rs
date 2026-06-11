//! Regression: a panicked verifier worker must fail the build *closed* (#413).
//!
//! `run_pipeline_from_frontend` runs verification on a worker thread. A panic
//! there makes `join()` return `Err`; the old `.unwrap_or((Skipped, _))` turned
//! that into `BuildStatus::Unverified` — a success with exit 0 and a linked
//! binary, hiding a verifier crash. The build now reports `VerifyFailed`, drops
//! the executable, and exits non-zero. The panic is injected via the
//! test-only `VOW_TEST_VERIFIER_PANIC` env var.

use std::path::PathBuf;
use std::process::Command;

fn vow_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_vow"))
}

#[test]
fn verifier_thread_panic_fails_build_closed() {
    let dir = tempfile::TempDir::new().unwrap();
    let src = dir.path().join("m.vow");
    // A satisfiable contract so the build reaches the verifier join site.
    std::fs::write(
        &src,
        "module M\n\
         fn f(x: i64) -> i64 vow { ensures: result == x } { x }\n\
         fn main() -> i32 [io] { 0 }\n",
    )
    .unwrap();
    let out_path = dir.path().join("out");

    let out = Command::new(vow_bin())
        .args([
            "build",
            src.to_str().unwrap(),
            "-o",
            out_path.to_str().unwrap(),
        ])
        .env("VOW_TEST_VERIFIER_PANIC", "1")
        .output()
        .expect("failed to run vow");

    // Fail-closed: non-zero exit, a failing status, and no binary handed back.
    assert_ne!(
        out.status.code(),
        Some(0),
        "a verifier-thread panic must not exit 0"
    );
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("build JSON");
    assert_eq!(json["status"], "VerifyFailed");
    assert_eq!(json["verify_status"], "panicked");
    assert!(
        json["executable"].is_null(),
        "no executable on a panicked build"
    );
    assert!(
        !out_path.exists(),
        "the linked binary must be removed on a panicked build"
    );
}

#[test]
fn normal_build_is_unaffected_by_the_injection_hook() {
    // Without the env var, the hook is inert and a clean build still succeeds
    // (Unverified here because ESBMC may be absent in CI; the point is exit 0).
    let dir = tempfile::TempDir::new().unwrap();
    let src = dir.path().join("m.vow");
    std::fs::write(&src, "module M\nfn main() -> i32 [io] { 0 }\n").unwrap();
    let out = Command::new(vow_bin())
        .args([
            "build",
            "--no-verify",
            src.to_str().unwrap(),
            "-o",
            dir.path().join("out").to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    assert_eq!(out.status.code(), Some(0));
}
