use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn vow_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_vow"))
}

/// Linking a program needs `libvow_runtime.a`; see `wide_literal_aggregates.rs`.
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
                return;
            }
        }
        let status = Command::new(env!("CARGO"))
            .args(["build", "-p", "vow-runtime"])
            .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/.."))
            .status()
            .expect("spawn cargo to build vow-runtime");
        assert!(status.success(), "failed to build vow-runtime staticlib");
        let built = target_dir.join("debug").join("libvow_runtime.a");
        assert!(built.exists(), "no runtime archive at {}", built.display());
        // SAFETY: single-threaded `Once` initializer, before any test spawns a
        // child process that reads the environment.
        unsafe { std::env::set_var("VOW_RUNTIME_PATH", &built) };
    });
}

/// Text of one function in a `--dump-ir` listing.
fn ir_function<'a>(dump: &'a str, name: &str) -> &'a str {
    let header = format!("fn {name}(");
    let start = dump
        .find(&header)
        .unwrap_or_else(|| panic!("missing function `{name}` in:\n{dump}"));
    let rest = &dump[start..];
    let end = rest[1..].find("\nfn ").map_or(rest.len(), |i| i + 1);
    &rest[..end]
}

/// A `Vec<bool>` element read is the runtime's 8-byte slot typed `i64`; the
/// lowerer compares it with zero so every later use sees a `bool`, for an index
/// read and a `for` binding alike. Any other element type is left alone. Twin of
/// `compiler/tests/test_lower_vec_bool_reads.vow`.
#[test]
fn vec_bool_reads_are_narrowed_to_bool() {
    ensure_runtime_archive();
    let dir = tempfile::TempDir::new().unwrap();
    let source_path = dir.path().join("vec_bool_layout.vow");
    let output_path = dir.path().join("vec_bool_layout");
    fs::write(
        &source_path,
        fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/vec_bool_layout.vow"),
        )
        .unwrap(),
    )
    .unwrap();
    let output = Command::new(vow_bin())
        .args([
            "build",
            "--no-verify",
            "--no-cache",
            "--dump-ir",
            source_path.to_str().unwrap(),
            "-o",
            output_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    let dump = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);

    for name in ["read_flag", "count_set"] {
        let body = ir_function(&dump, name);
        assert_eq!(
            body.matches("extern:__vow_vec_get_val").count(),
            1,
            "`{name}` reads one element:\n{body}"
        );
        assert!(
            body.contains("= Ne[i64]("),
            "`{name}` must compare the slot with zero:\n{body}"
        );
    }
    let byte = ir_function(&dump, "read_byte");
    assert!(
        !byte.contains("Ne[i64]"),
        "a `Vec<u8>` read is not a bool comparison:\n{byte}"
    );
}
