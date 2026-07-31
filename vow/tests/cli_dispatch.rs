//! Integration coverage for `main()`'s CLI dispatch: verify-jobs/solver-config
//! resolution, subcommand routing, and the legacy no-subcommand path. This
//! glue only runs when the real `vow` binary parses real argv, so unlike most
//! tests in `src/main.rs` (which call the pipeline functions directly) it
//! cannot be exercised without spawning the built binary.

use std::path::PathBuf;
use std::process::Command;

fn vow_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_vow"))
}

const TRIVIAL_PROGRAM: &str = "module M\nfn main() -> i32 [io] { 0 }\n";

fn write_program(dir: &tempfile::TempDir, name: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, TRIVIAL_PROGRAM).unwrap();
    path
}

fn parse_json(stdout: &str) -> serde_json::Value {
    serde_json::from_str(stdout)
        .unwrap_or_else(|e| panic!("invalid JSON from vow: {e}\nstdout: {stdout}"))
}

/// A well-typed program still runs codegen + link, which needs
/// `libvow_runtime.a`. Tolerate a link-only `CompileFailed` when the archive
/// is absent (standalone `cargo test` without a prior `cargo build --all`),
/// matching the convention in `effect_gating.rs` / `region_summary_equivalence.rs`.
fn is_runtime_link_failure(status_code: Option<i32>, json: &serde_json::Value) -> bool {
    status_code == Some(1)
        && json["status"] == "CompileFailed"
        && json["message"]
            .as_str()
            .is_some_and(|m| m.contains("libvow_runtime.a"))
}

#[test]
fn verify_jobs_zero_is_rejected_before_any_subcommand_work() {
    // `--verify-jobs 0` is rejected by `cli::resolve_verify_jobs` and reported
    // via `unwrap_or_exit`'s Err arm before the source file is even read, so a
    // nonexistent path still exercises the dispatch-level error path.
    let out = Command::new(vow_bin())
        .args(["build", "--verify-jobs", "0", "does-not-need-to-exist.vow"])
        .output()
        .expect("failed to run vow");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("error: --verify-jobs must be >= 1"),
        "expected verify-jobs error in stderr, got: {stderr}"
    );
}

#[test]
fn verify_subcommand_resolves_jobs_and_solver_config() {
    // Exercises the Command::Verify dispatch arm's jobs/config resolution,
    // which only runs via the real CLI entry point. The verify outcome
    // depends on whether ESBMC is installed in this environment, so only the
    // JSON shape is checked here, not the verify status.
    let dir = tempfile::TempDir::new().unwrap();
    let source = write_program(&dir, "m.vow");
    let out = Command::new(vow_bin())
        .args(["verify", source.to_str().unwrap()])
        .output()
        .expect("failed to run vow");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json = parse_json(&stdout);
    assert!(
        json.get("status").is_some(),
        "expected 'status' key in verify JSON, got: {json}"
    );
}

#[test]
fn test_subcommand_resolves_jobs_on_an_empty_directory() {
    // Exercises the Command::Test dispatch arm's jobs resolution. An empty
    // directory has zero test files, so run_test_command reports total: 0 and
    // exits 0 without needing ESBMC or the runtime archive.
    let dir = tempfile::TempDir::new().unwrap();
    let out = Command::new(vow_bin())
        .args(["test", dir.path().to_str().unwrap()])
        .output()
        .expect("failed to run vow");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json = parse_json(&stdout);
    assert_eq!(out.status.code(), Some(0), "stdout: {stdout}");
    assert_eq!(json["total"], 0);
}

#[test]
fn legacy_no_subcommand_invocation_builds() {
    // Exercises the `None =>` dispatch arm: the pre-subcommand `vow <file>`
    // form, kept for backward compatibility. Mode/trace translation and
    // jobs/config resolution only run through this path when there is no
    // subcommand at all.
    let dir = tempfile::TempDir::new().unwrap();
    let source = write_program(&dir, "m.vow");
    let out_path = dir.path().join("out");
    let out = Command::new(vow_bin())
        .args([
            "--no-verify",
            source.to_str().unwrap(),
            "-o",
            out_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json = parse_json(&stdout);
    if is_runtime_link_failure(out.status.code(), &json) {
        eprintln!("SKIP: runtime archive not present in this environment");
        return;
    }
    assert_eq!(
        out.status.code(),
        Some(0),
        "expected legacy invocation to build successfully\nstdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(json["status"], "Unverified");
}
