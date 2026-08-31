//! A checked operator's overflow abort is part of the operator's meaning, not a
//! debug-mode convenience: `grammar.md` says checked operators "abort with
//! `ArithmeticOverflow`", and `cli.md` reserves exit `134` for every runtime
//! abort so an agent can tell an abort from an application result.
//!
//! Regression coverage for the case where the `__vow_arithmetic_overflow` call
//! was gated on debug mode while the trailing `trap` was not, so a release
//! build — the default for `vow build` — died on a bare `ud2`: `SIGILL`, exit
//! `132`, and not one byte on stderr.
//!
//! These assert *parity*: release must match debug byte for byte. Asserting
//! only "release exits 134" would still pass if the two modes diverged in what
//! they printed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn vow_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_vow"))
}

/// These tests link real executables, which needs `libvow_runtime.a`. `cargo
/// test` builds only the crates under test, so on a clean checkout that
/// archive may not exist and every test here would fail on a link error rather
/// than on the behavior it means to check. Mirrors `wide_literal_aggregates`.
fn ensure_runtime_archive() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if std::env::var_os("VOW_RUNTIME_PATH").is_some_and(|p| PathBuf::from(p).exists()) {
            return;
        }
        let target_dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../target"));
        for profile in ["release", "debug"] {
            if target_dir.join(profile).join("libvow_runtime.a").exists() {
                return; // the linker's own search finds this unaided
            }
        }
        let status = Command::new(env!("CARGO"))
            .args(["build", "-p", "vow-runtime"])
            .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/.."))
            .status()
            .expect("spawn cargo to build vow-runtime");
        assert!(status.success(), "failed to build vow-runtime staticlib");
        let built = target_dir.join("debug").join("libvow_runtime.a");
        assert!(
            built.exists(),
            "cargo build -p vow-runtime did not produce {}",
            built.display()
        );
        // SAFETY: single-threaded `Once` initializer, before any test spawns a
        // child process that reads the environment.
        unsafe { std::env::set_var("VOW_RUNTIME_PATH", &built) };
    });
}

/// Exit status and stderr of the compiled program.
struct RunResult {
    exit: Option<i32>,
    stderr: String,
}

/// Compile `source` in `mode` and run it. `mode` of `None` passes no `--mode`
/// flag at all, which is the path a plain `vow build` takes.
fn build_and_run(dir: &Path, source: &str, mode: Option<&str>, tag: &str) -> RunResult {
    ensure_runtime_archive();
    let source_path = dir.join(format!("{tag}.vow"));
    let output_path = dir.join(tag);
    fs::write(&source_path, source).unwrap();

    let mut args = vec!["build".to_string(), "--no-verify".to_string()];
    if let Some(mode) = mode {
        args.push("--mode".to_string());
        args.push(mode.to_string());
    }
    args.push(source_path.to_str().unwrap().to_string());
    args.push("-o".to_string());
    args.push(output_path.to_str().unwrap().to_string());

    let build = Command::new(vow_bin())
        .args(&args)
        .output()
        .expect("failed to run vow");
    assert!(
        build.status.success() && output_path.exists(),
        "build failed for {tag}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    let run = Command::new(&output_path)
        .output()
        .unwrap_or_else(|e| panic!("failed to run compiled {tag}: {e}"));
    RunResult {
        exit: run.status.code(),
        stderr: String::from_utf8_lossy(&run.stderr).into_owned(),
    }
}

/// `vow-runtime` reports every runtime abort with this status (`cli.md`).
const ABORT_EXIT: i32 = 134;
const OVERFLOW_JSON: &str = r#"{"error":"ArithmeticOverflow"}"#;

fn overflow_program(ty: &str, seed: &str, op: &str, rhs: &str) -> String {
    // The operand comes through a call so the checked op is lowered to a real
    // overflow check rather than folded at compile time.
    format!(
        "module CheckedOverflow\n\
         fn seed() -> {ty} {{ {seed} }}\n\
         fn overflow() -> {ty} {{ seed() {op} {rhs} }}\n\
         fn main() -> i32 {{ overflow(); 0 }}\n"
    )
}

#[test]
fn release_mode_overflow_reports_the_same_diagnostic_as_debug() {
    let dir = tempfile::TempDir::new().unwrap();
    let source = overflow_program("i64", "9223372036854775807", "+!", "1");

    let debug = build_and_run(dir.path(), &source, Some("debug"), "dbg");
    let release = build_and_run(dir.path(), &source, Some("release"), "rel");

    assert_eq!(
        debug.exit,
        Some(ABORT_EXIT),
        "debug overflow must abort with {ABORT_EXIT}; stderr: {}",
        debug.stderr
    );
    assert_eq!(
        release.exit, debug.exit,
        "release must abort exactly as debug does, not on a bare trap\n\
         release stderr: {:?}",
        release.stderr
    );
    assert_eq!(
        release.stderr, debug.stderr,
        "release must emit the same diagnostic as debug, byte for byte"
    );
    assert!(
        release.stderr.contains(OVERFLOW_JSON),
        "release overflow must emit {OVERFLOW_JSON}; got: {:?}",
        release.stderr
    );
}

#[test]
fn default_build_mode_overflow_is_diagnosed() {
    // Release is the default, so a plain `vow build` is the path agents take.
    let dir = tempfile::TempDir::new().unwrap();
    let source = overflow_program("i64", "9223372036854775807", "+!", "1");
    let result = build_and_run(dir.path(), &source, None, "def");

    assert_eq!(
        result.exit,
        Some(ABORT_EXIT),
        "default-mode overflow must abort with {ABORT_EXIT}; stderr: {:?}",
        result.stderr
    );
    assert!(
        result.stderr.contains(OVERFLOW_JSON),
        "default-mode overflow must emit {OVERFLOW_JSON}; got: {:?}",
        result.stderr
    );
}

#[test]
fn checked_operators_are_diagnosed_in_release_at_every_width() {
    let dir = tempfile::TempDir::new().unwrap();
    let cases = [
        ("i64_add", "i64", "9223372036854775807", "+!", "1"),
        ("i64_sub", "i64", "-9223372036854775807", "-!", "2"),
        ("i64_mul", "i64", "9223372036854775807", "*!", "2"),
        ("u64_add", "u64", "18446744073709551615", "+!", "1"),
        ("u64_sub", "u64", "0", "-!", "1"),
        ("u64_mul", "u64", "18446744073709551615", "*!", "2"),
        ("i32_add", "i32", "2147483647", "+!", "1"),
        ("u32_sub", "u32", "0", "-!", "1"),
        (
            "i128_add",
            "i128",
            "170141183460469231731687303715884105727",
            "+!",
            "1",
        ),
        ("u128_sub", "u128", "0", "-!", "1"),
    ];

    for (tag, ty, seed, op, rhs) in cases {
        let source = overflow_program(ty, seed, op, rhs);
        let result = build_and_run(dir.path(), &source, Some("release"), tag);
        assert_eq!(
            result.exit,
            Some(ABORT_EXIT),
            "{tag}: release `{op}` must abort with {ABORT_EXIT}, not a bare trap; stderr: {:?}",
            result.stderr
        );
        assert!(
            result.stderr.contains(OVERFLOW_JSON),
            "{tag}: release `{op}` must emit {OVERFLOW_JSON}; got: {:?}",
            result.stderr
        );
    }
}
