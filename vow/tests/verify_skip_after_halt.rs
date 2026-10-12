//! A `VerificationSkipped` warning is a static fact about the program: every
//! non-modelable vowed function is reported even when another function halts
//! verification, whatever `--verify-jobs` says (#1561).
//!
//! A fake `esbmc` that fails at once stands in for the solver, so the halt
//! lands long before a worker could claim the later skip indices — the exact
//! timing a warm verify cache produces.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const SOURCE: &str = "module M\n\
    fn wrong(x: i64) -> i64 vow { ensures: result == x + 1 } { x }\n\
    fn push_a(g: Vec<Vec<i64>>, r: Vec<i64>) -> i64 vow { ensures: result >= 0 } \
    { g.push(r); g.len() as i64 }\n\
    fn push_b(g: Vec<Vec<i64>>, r: Vec<i64>) -> i64 vow { ensures: result >= 0 } \
    { g.push(r); g.len() as i64 }\n\
    fn main() -> i32 [io] { 0 }\n";

fn vow_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_vow"))
}

fn failing_esbmc(dir: &Path) -> PathBuf {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = bin.join("esbmc");
    std::fs::write(&script, "#!/bin/sh\necho 'VERIFICATION FAILED'\nexit 1\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

fn skipped_functions(json: &serde_json::Value) -> Vec<String> {
    let mut names: Vec<String> = json["diagnostics"]
        .as_array()
        .expect("diagnostics array")
        .iter()
        .filter(|d| d["error_code"] == "VerificationSkipped" && d["severity"] == "warning")
        .filter_map(|d| d["message"].as_str())
        .filter_map(|m| m.split('`').nth(1))
        .map(str::to_string)
        .collect();
    names.sort();
    names
}

#[test]
fn skips_are_reported_after_a_failure_for_any_job_count() {
    let dir = tempfile::TempDir::new().unwrap();
    let src = dir.path().join("m.vow");
    std::fs::write(&src, SOURCE).unwrap();
    let path = format!(
        "{}:{}",
        failing_esbmc(dir.path()).display(),
        std::env::var("PATH").unwrap_or_default()
    );

    for jobs in ["1", "4"] {
        let out = Command::new(vow_bin())
            .args(["verify", "--no-cache", "--verify-jobs", jobs])
            .arg(&src)
            .env("PATH", &path)
            .output()
            .expect("failed to run vow");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let json: serde_json::Value = serde_json::from_str(&stdout).expect("verify JSON");

        assert_eq!(json["status"], "VerifyFailed", "jobs={jobs}: {stdout}");
        assert_eq!(
            skipped_functions(&json),
            vec!["push_a", "push_b"],
            "jobs={jobs}: {stdout}"
        );
    }
}
