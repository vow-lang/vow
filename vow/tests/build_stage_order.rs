//! `vow build` finishes codegen and linking before any ESBMC process starts
//! (#179), so the in-process Cranelift working set never overlaps the
//! verifier's child processes.
//!
//! A fake `esbmc` first on `PATH` records whether the output executable
//! already exists when it is launched; it receives ESBMC's arguments rather
//! than `build`'s `-o`, so the executable path travels via `VOW_TEST_EXE`.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn vow_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_vow"))
}

fn fake_esbmc(dir: &Path) -> PathBuf {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = bin.join("esbmc");
    std::fs::write(
        &script,
        "#!/bin/sh\n\
         if [ -e \"$VOW_TEST_EXE\" ]; then echo exe=present >> \"$VOW_TEST_LOG\"; \
         else echo exe=absent >> \"$VOW_TEST_LOG\"; fi\n\
         echo 'VERIFICATION SUCCESSFUL'\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

fn build_with_fake_esbmc(dir: &Path, src: &Path, out: &Path, log: &Path) -> Output {
    let path = format!(
        "{}:{}",
        fake_esbmc(dir).display(),
        std::env::var("PATH").unwrap_or_default()
    );
    Command::new(vow_bin())
        .args([
            "build",
            "--no-cache",
            src.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .env("PATH", path)
        .env("VOW_TEST_EXE", out)
        .env("VOW_TEST_LOG", log)
        .output()
        .expect("failed to run vow")
}

#[test]
fn verification_starts_only_after_the_executable_is_linked() {
    let dir = tempfile::TempDir::new().unwrap();
    let src = dir.path().join("m.vow");
    std::fs::write(
        &src,
        "module M\n\
         fn f(x: i64) -> i64 vow { ensures: result == x } { x }\n\
         fn main() -> i32 [io] { 0 }\n",
    )
    .unwrap();
    let out_path = dir.path().join("out");
    let log = dir.path().join("log");

    let out = build_with_fake_esbmc(dir.path(), &src, &out_path, &log);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("build JSON");
    assert_eq!(json["status"], "Verified", "stdout: {stdout}");
    assert!(out_path.exists(), "the executable must be kept");
    let launches = std::fs::read_to_string(&log).expect("fake esbmc was never launched");
    assert!(!launches.is_empty());
    assert!(
        launches.lines().all(|l| l == "exe=present"),
        "ESBMC started before codegen+link finished: {launches:?}"
    );
}

#[test]
fn codegen_failure_never_starts_verification() {
    let dir = tempfile::TempDir::new().unwrap();
    let src = dir.path().join("m.vow");
    std::fs::write(
        &src,
        "module M\n\
         fn f(x: i64) -> i64 vow { ensures: result == x } { x }\n\
         fn remainder(a: f64, b: f64) -> f64 { a % b }\n\
         fn main() -> i32 [io] { 0 }\n",
    )
    .unwrap();
    let out_path = dir.path().join("out");
    let log = dir.path().join("log");

    let out = build_with_fake_esbmc(dir.path(), &src, &out_path, &log);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("build JSON");
    assert_eq!(json["status"], "CompileFailed", "stdout: {stdout}");
    assert!(
        stdout.contains("CodegenUnsupported"),
        "expected the codegen diagnostic: {stdout}"
    );
    assert!(
        !log.exists(),
        "verification must not start after a codegen failure: {:?}",
        std::fs::read_to_string(&log)
    );
}
