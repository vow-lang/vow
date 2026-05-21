use std::path::{Path, PathBuf};
use std::process::Command;

use crate::CodegenError;

/// Find the compiled vow-runtime static library.
/// Checks VOW_RUNTIME_PATH env var first, then searches the cargo target tree
/// relative to the vowc executable (for installed use) and workspace root
/// (for development use).
pub fn find_runtime_lib() -> Option<PathBuf> {
    find_lib("libvow_runtime.a", "VOW_RUNTIME_PATH")
}

/// Find the compiled vow-clif-shim static library.
pub fn find_shim_lib() -> Option<PathBuf> {
    find_lib("libvow_clif_shim.a", "VOW_CLIF_SHIM_PATH")
}

fn find_lib(name: &str, env_var: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok();
    find_lib_from_parts(name, std::env::var_os(env_var), exe.as_deref())
}

fn find_lib_from_parts(
    name: &str,
    env_value: Option<std::ffi::OsString>,
    exe: Option<&Path>,
) -> Option<PathBuf> {
    let target_dir = cargo_target_dir();
    find_lib_from_parts_with_target_dir(name, env_value, exe, &target_dir)
}

fn find_lib_from_parts_with_target_dir(
    name: &str,
    env_value: Option<std::ffi::OsString>,
    exe: Option<&Path>,
    target_dir: &Path,
) -> Option<PathBuf> {
    if let Some(p) = env_value {
        let path = PathBuf::from(p);
        if path.exists() {
            return Some(path);
        }
    }

    if let Some(exe) = exe
        && let Some(path) = find_installed_lib_for_exe(name, exe)
    {
        return Some(path);
    }

    find_lib_in_cargo_target(name, target_dir)
}

fn find_installed_lib_for_exe(name: &str, exe: &Path) -> Option<PathBuf> {
    let exe_dir = exe.parent();
    let prefix_dir = exe_dir.and_then(|dir| dir.parent());
    // Preserve the legacy adjacent-to-exe lookup before prefix paths so manual
    // installs that co-locate the static libraries with vowc keep working.
    let candidates = [
        exe_dir.map(|dir| dir.join(name)),
        prefix_dir.map(|prefix| prefix.join("lib").join("vow").join(name)),
        prefix_dir.map(|prefix| prefix.join("lib").join(name)),
    ];

    candidates
        .into_iter()
        .flatten()
        .find(|candidate| candidate.exists())
}

fn cargo_target_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../target"))
}

fn find_lib_in_cargo_target(name: &str, target_dir: &Path) -> Option<PathBuf> {
    // Development fallback only: env overrides and installed-prefix libraries
    // are checked first, so prefer plain cargo build/test artifacts here while
    // still accepting release bootstrap artifacts.
    ["debug", "release"]
        .into_iter()
        .map(|profile| target_dir.join(profile).join(name))
        .find(|candidate| candidate.exists())
}

fn platform_link_args_for(os: &str) -> &'static [&'static str] {
    match os {
        "linux" => &["-lpthread", "-ldl", "-lm"],
        "macos" => &["-lpthread", "-lm"],
        _ => &["-lpthread", "-lm"],
    }
}

/// Link one or more object files together with the vow runtime into an
/// executable. Uses the system C compiler as the linker driver.
/// If `shim_lib` is provided, it is also included in the link.
pub fn link(
    objects: &[&Path],
    runtime_lib: &Path,
    shim_lib: Option<&Path>,
    output: &Path,
) -> Result<(), CodegenError> {
    let mut cmd = Command::new("cc");
    for obj in objects {
        cmd.arg(obj);
    }
    cmd.arg(runtime_lib);
    if let Some(shim) = shim_lib {
        cmd.arg(shim);
    }
    cmd.arg("-o").arg(output);
    // Needed when linking a Rust staticlib that uses std.
    // On macOS the dl* symbols live in libc (no separate libdl), so -ldl
    // would cause "library not found" — only pass it on platforms that
    // actually ship libdl as a standalone library.
    cmd.args(platform_link_args_for(std::env::consts::OS));

    let status = cmd
        .status()
        .map_err(|e| CodegenError::Link(format!("failed to invoke cc: {e}")))?;

    if !status.success() {
        return Err(CodegenError::Link(format!(
            "cc exited with status {status}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_lib_in_installed_lib_vow_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let bin_dir = dir.path().join("bin");
        let lib_dir = dir.path().join("lib").join("vow");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::create_dir_all(&lib_dir).unwrap();
        let exe = bin_dir.join("vowc");
        let lib = lib_dir.join("libvow_runtime.a");
        std::fs::write(&exe, b"").unwrap();
        std::fs::write(&lib, b"").unwrap();

        let found = find_lib_from_parts_with_target_dir(
            "libvow_runtime.a",
            None,
            Some(&exe),
            &dir.path().join("target"),
        );
        assert_eq!(found.as_deref(), Some(lib.as_path()));
    }

    #[test]
    fn finds_lib_in_installed_lib_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let bin_dir = dir.path().join("bin");
        let lib_dir = dir.path().join("lib");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::create_dir_all(&lib_dir).unwrap();
        let exe = bin_dir.join("vowc");
        let lib = lib_dir.join("libvow_runtime.a");
        std::fs::write(&exe, b"").unwrap();
        std::fs::write(&lib, b"").unwrap();

        let found = find_lib_from_parts_with_target_dir(
            "libvow_runtime.a",
            None,
            Some(&exe),
            &dir.path().join("target"),
        );
        assert_eq!(found.as_deref(), Some(lib.as_path()));
    }

    #[test]
    fn env_override_does_not_require_current_exe() {
        let dir = tempfile::TempDir::new().unwrap();
        let lib = dir.path().join("libvow_runtime.a");
        std::fs::write(&lib, b"").unwrap();

        let found =
            find_lib_from_parts("libvow_runtime.a", Some(lib.clone().into_os_string()), None);
        assert_eq!(found.as_deref(), Some(lib.as_path()));
    }

    #[test]
    fn cargo_target_fallback_does_not_require_current_exe() {
        let dir = tempfile::TempDir::new().unwrap();
        let debug_dir = dir.path().join("debug");
        std::fs::create_dir_all(&debug_dir).unwrap();
        let lib = debug_dir.join("libvow_runtime.a");
        std::fs::write(&lib, b"").unwrap();

        let found = find_lib_from_parts_with_target_dir("libvow_runtime.a", None, None, dir.path());
        assert_eq!(found.as_deref(), Some(lib.as_path()));
    }

    #[test]
    fn cargo_target_fallback_accepts_release_when_debug_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        let release_dir = dir.path().join("release");
        std::fs::create_dir_all(&release_dir).unwrap();
        let lib = release_dir.join("libvow_runtime.a");
        std::fs::write(&lib, b"").unwrap();

        let found = find_lib_from_parts_with_target_dir("libvow_runtime.a", None, None, dir.path());
        assert_eq!(found.as_deref(), Some(lib.as_path()));
    }

    #[test]
    fn cargo_target_fallback_prefers_debug_before_release() {
        let dir = tempfile::TempDir::new().unwrap();
        let debug_dir = dir.path().join("debug");
        let release_dir = dir.path().join("release");
        std::fs::create_dir_all(&debug_dir).unwrap();
        std::fs::create_dir_all(&release_dir).unwrap();
        let debug_lib = debug_dir.join("libvow_runtime.a");
        let release_lib = release_dir.join("libvow_runtime.a");
        std::fs::write(&debug_lib, b"debug").unwrap();
        std::fs::write(&release_lib, b"release").unwrap();

        let found = find_lib_from_parts_with_target_dir("libvow_runtime.a", None, None, dir.path());
        assert_eq!(found.as_deref(), Some(debug_lib.as_path()));
    }

    #[test]
    fn linux_link_args_include_dl() {
        assert_eq!(
            platform_link_args_for("linux"),
            ["-lpthread", "-ldl", "-lm"]
        );
    }

    #[test]
    fn macos_link_args_omit_dl() {
        assert_eq!(platform_link_args_for("macos"), ["-lpthread", "-lm"]);
    }

    #[test]
    fn other_link_args_omit_dl() {
        assert_eq!(platform_link_args_for("freebsd"), ["-lpthread", "-lm"]);
    }

    #[test]
    fn find_runtime_returns_some_in_dev_build() {
        let found = find_runtime_lib();
        assert!(
            found.is_some(),
            "could not find libvow_runtime.a; run `cargo build -p vow-runtime` first"
        );
    }
}
