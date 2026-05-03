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
    let exe = std::env::current_exe().ok()?;
    find_lib_for_exe(name, env_var, &exe)
}

fn find_lib_for_exe(name: &str, env_var: &str, exe: &Path) -> Option<PathBuf> {
    if let Ok(p) = std::env::var(env_var) {
        let path = PathBuf::from(p);
        if path.exists() {
            return Some(path);
        }
    }

    let exe_dir = exe.parent();
    let prefix_dir = exe_dir.and_then(|dir| dir.parent());
    let candidates = [
        exe_dir.map(|dir| dir.join(name)),
        prefix_dir.map(|prefix| prefix.join("lib").join("vow").join(name)),
        prefix_dir.map(|prefix| prefix.join("lib").join(name)),
        Some(PathBuf::from(format!(
            "{}/{name}",
            concat!(env!("CARGO_MANIFEST_DIR"), "/../target/debug")
        ))),
        Some(PathBuf::from(format!(
            "{}/{name}",
            concat!(env!("CARGO_MANIFEST_DIR"), "/../target/release")
        ))),
    ];

    candidates
        .into_iter()
        .flatten()
        .find(|candidate| candidate.exists())
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
    // Needed when linking a Rust staticlib that uses std
    cmd.args(["-lpthread", "-ldl", "-lm"]);

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

        let found = find_lib_for_exe("libvow_runtime.a", "VOW_TEST_RUNTIME_PATH", &exe);
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

        let found = find_lib_for_exe("libvow_runtime.a", "VOW_TEST_RUNTIME_PATH", &exe);
        assert_eq!(found.as_deref(), Some(lib.as_path()));
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
