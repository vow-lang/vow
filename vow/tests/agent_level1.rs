#[cfg(target_os = "linux")]
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
#[cfg(target_os = "linux")]
use std::process::Stdio;
use std::sync::OnceLock;

static SUPPORT_LIBS_BUILT: OnceLock<()> = OnceLock::new();

fn vow_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_vow"))
}

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("examples")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn support_lib(name: &str) -> PathBuf {
    build_support_libs();

    find_support_lib(name)
        .unwrap_or_else(|| panic!("missing {name} after building support libraries"))
}

fn build_support_libs() {
    SUPPORT_LIBS_BUILT.get_or_init(|| {
        let status = Command::new("cargo")
            .args(["build", "-p", "vow-runtime", "-p", "vow-clif-shim"])
            .status()
            .expect("failed to invoke cargo to build support libraries");
        assert!(
            status.success(),
            "cargo build -p vow-runtime -p vow-clif-shim failed"
        );
    });
}

fn find_support_lib(name: &str) -> Option<PathBuf> {
    let root = workspace_root();
    for profile in ["debug", "release"] {
        let path = root.join("target").join(profile).join(name);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

#[test]
fn help_json_valid() {
    let out = Command::new(vow_bin())
        .args(["--help"])
        .output()
        .expect("failed to run vow");
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON from --help: {e}\nstdout: {stdout}"));
    assert!(json.get("tool").is_some(), "expected 'tool' key in JSON");
}

#[test]
fn build_help_json_valid() {
    let out = Command::new(vow_bin())
        .args(["build", "--help"])
        .output()
        .expect("failed to run vow");
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON from build --help: {e}\nstdout: {stdout}"));
    assert!(json.get("tool").is_some(), "expected 'tool' key in JSON");
}

#[test]
fn installed_prefix_can_build_and_run_program_without_env_paths() {
    let dir = tempfile::TempDir::new().unwrap();
    let prefix = dir.path().join("prefix");
    let bin_dir = prefix.join("bin");
    let lib_dir = prefix.join("lib").join("vow");
    std::fs::create_dir_all(&bin_dir).unwrap();
    std::fs::create_dir_all(&lib_dir).unwrap();

    let installed_vowc = bin_dir.join("vowc");
    std::fs::copy(vow_bin(), &installed_vowc).unwrap();
    std::fs::copy(
        support_lib("libvow_runtime.a"),
        lib_dir.join("libvow_runtime.a"),
    )
    .unwrap();
    std::fs::copy(
        support_lib("libvow_clif_shim.a"),
        lib_dir.join("libvow_clif_shim.a"),
    )
    .unwrap();

    let out_path = dir.path().join("hello");
    let result = Command::new(&installed_vowc)
        .args([
            "build",
            "--no-verify",
            "--no-cache",
            examples_dir().join("hello.vow").to_str().unwrap(),
            "-o",
            out_path.to_str().unwrap(),
        ])
        .env_remove("VOW_RUNTIME_PATH")
        .env_remove("VOW_CLIF_SHIM_PATH")
        .output()
        .expect("failed to run installed vowc");
    assert_eq!(
        result.status.code(),
        Some(0),
        "expected installed vowc build to succeed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );

    let run = Command::new(&out_path)
        .output()
        .expect("failed to run installed-compiler output");
    assert_eq!(run.status.code(), Some(0), "compiled binary should exit 0");
}

#[test]
fn compile_success() {
    let dir = tempfile::TempDir::new().unwrap();
    let out_path = dir.path().join("hello");
    let result = Command::new(vow_bin())
        .args([
            "build",
            "--no-verify",
            "--no-cache",
            examples_dir().join("hello.vow").to_str().unwrap(),
            "-o",
            out_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    assert_eq!(result.status.code(), Some(0), "expected exit 0");
    let stdout = String::from_utf8_lossy(&result.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON: {e}\nstdout: {stdout}"));
    assert_eq!(json["status"], "Unverified");
    assert!(out_path.exists(), "expected executable at {out_path:?}");

    let run = Command::new(&out_path)
        .output()
        .expect("failed to run compiled binary");
    assert_eq!(run.status.code(), Some(0), "compiled binary should exit 0");
}

#[cfg(target_os = "linux")]
#[test]
fn stdin_read_line_processes_large_stream_under_memory_cap() {
    build_support_libs();

    let dir = tempfile::TempDir::new().unwrap();
    let src_path = dir.path().join("stdin_large.vow");
    let out_path = dir.path().join("stdin_large");
    let src = r#"module StdinLarge

fn main() -> i32 [read, io] {
    let mut count: u64 = 0;
    let mut line: String = stdin_read_line();
    while line.len() > 0 {
        count = count + 1;
        line = stdin_read_line();
    }
    print_str(String::from("total: "));
    print_u64(count);
    print_str(String::from("\n"));
    0
}
"#;
    std::fs::write(&src_path, src).unwrap();

    let result = Command::new(vow_bin())
        .args([
            "build",
            "--no-verify",
            "--no-cache",
            src_path.to_str().unwrap(),
            "-o",
            out_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    assert_eq!(
        result.status.code(),
        Some(0),
        "expected build success\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );

    let mut child = Command::new("sh")
        .arg("-c")
        .arg("ulimit -v 131072; exec \"$1\"")
        .arg("sh")
        .arg(out_path.to_str().unwrap())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run compiled binary under memory cap");

    {
        let stdin = child.stdin.as_mut().expect("child stdin");
        let payload = vec![b'x'; 128 * 1024 - 1];
        for _ in 0..2000 {
            stdin
                .write_all(&payload)
                .expect("failed to write line payload");
            stdin.write_all(b"\n").expect("failed to write newline");
        }
    }

    let run = child.wait_with_output().expect("failed to wait for child");
    assert_eq!(
        run.status.code(),
        Some(0),
        "expected bounded line processing to finish\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "total: 2000\n");
}

#[test]
fn checked_arithmetic_returns_integer() {
    let dir = tempfile::TempDir::new().unwrap();
    let src_path = dir.path().join("checked_arithmetic.vow");
    let out_path = dir.path().join("checked_arithmetic");
    let src = r#"module CheckedArithmetic

fn checked_add(a: i64, b: i64) -> i64 {
    a +! b
}

fn main() -> i32 {
    if checked_add(3, 4) != 7 {
        return 1;
    }
    0
}
"#;
    std::fs::write(&src_path, src).unwrap();

    let result = Command::new(vow_bin())
        .args([
            "build",
            "--no-verify",
            "--no-cache",
            src_path.to_str().unwrap(),
            "-o",
            out_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    assert_eq!(result.status.code(), Some(0), "expected exit 0");
    let stdout = String::from_utf8_lossy(&result.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON: {e}\nstdout: {stdout}"));
    assert_eq!(json["status"], "Unverified");
    assert!(
        json["diagnostics"].as_array().unwrap().is_empty(),
        "expected no diagnostics, got {stdout}"
    );
}

#[test]
fn compile_type_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let bad_src = "module Bad\nfn main() -> i32 { true }";
    let src_path = dir.path().join("bad.vow");
    std::fs::write(&src_path, bad_src).unwrap();

    let result = Command::new(vow_bin())
        .args([
            "build",
            "--no-verify",
            "--no-cache",
            src_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    assert_eq!(result.status.code(), Some(1), "expected exit 1");
    let stdout = String::from_utf8_lossy(&result.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON: {e}\nstdout: {stdout}"));
    assert_eq!(json["status"], "CompileFailed");
    let diags = json["diagnostics"].as_array().expect("diagnostics array");
    assert!(!diags.is_empty(), "expected non-empty diagnostics");
}

#[test]
#[ignore]
fn verify_success() {
    let result = Command::new(vow_bin())
        .args([
            "build",
            "--no-cache",
            examples_dir().join("cegis_fixed.vow").to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    assert_eq!(result.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&result.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON: {e}\nstdout: {stdout}"));
    assert_eq!(json["status"], "Verified");
}

#[test]
#[ignore]
fn verify_failure_with_counterexample() {
    let result = Command::new(vow_bin())
        .args([
            "build",
            "--no-cache",
            examples_dir().join("cegis_broken.vow").to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    assert_eq!(result.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&result.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON: {e}\nstdout: {stdout}"));
    assert_eq!(json["status"], "VerifyFailed");
    let ces = json["counterexamples"]
        .as_array()
        .expect("counterexamples array");
    assert!(!ces.is_empty(), "expected non-empty counterexamples");
    let first = &ces[0];
    assert!(
        first.get("values").is_some(),
        "expected values in counterexample"
    );
}

#[test]
fn debug_trace_emits_json_lines() {
    let dir = tempfile::TempDir::new().unwrap();
    let out_path = dir.path().join("hello_trace");
    let result = Command::new(vow_bin())
        .args([
            "build",
            "--debug-trace=calls",
            "--no-verify",
            "--no-cache",
            examples_dir().join("hello.vow").to_str().unwrap(),
            "-o",
            out_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    assert_eq!(result.status.code(), Some(0), "expected exit 0");
    assert!(out_path.exists(), "expected compiled binary");

    let run = Command::new(&out_path)
        .output()
        .expect("failed to run traced binary");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(
        stderr.contains(r#""event":"enter""#),
        "expected enter trace in stderr, got: {stderr}"
    );
    assert!(
        stderr.contains(r#""event":"exit""#),
        "expected exit trace in stderr, got: {stderr}"
    );
}
