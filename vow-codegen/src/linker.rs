use std::path::{Path, PathBuf};
use std::process::Command;

use crate::CodegenError;

/// Find the compiled vow-runtime static library.
/// Checks VOW_RUNTIME_PATH env var first, then searches the cargo target tree
/// relative to the vowc executable (for installed use) and workspace root
/// (for development use).
pub fn find_runtime_lib() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("VOW_RUNTIME_PATH") {
        let path = PathBuf::from(p);
        if path.exists() {
            return Some(path);
        }
    }

    // During development: workspace root / target / {profile} / libvow_runtime.a
    let candidates = [
        // relative to current exe (vowc installed alongside the runtime)
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("libvow_runtime.a"))),
        // cargo debug build
        Some(PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../target/debug/libvow_runtime.a"
        ))),
        // cargo release build
        Some(PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../target/release/libvow_runtime.a"
        ))),
    ];

    candidates
        .into_iter()
        .flatten()
        .find(|candidate| candidate.exists())
}

/// Link one or more object files together with the vow runtime into an
/// executable. Uses the system C compiler as the linker driver.
pub fn link(objects: &[&Path], runtime_lib: &Path, output: &Path) -> Result<(), CodegenError> {
    let mut cmd = Command::new("cc");
    for obj in objects {
        cmd.arg(obj);
    }
    cmd.arg(runtime_lib);
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
    fn find_runtime_returns_some_in_dev_build() {
        // After `cargo build -p vow-runtime`, the debug staticlib should exist.
        // This test will pass in a workspace dev environment.
        let found = find_runtime_lib();
        assert!(
            found.is_some(),
            "could not find libvow_runtime.a; run `cargo build -p vow-runtime` first"
        );
    }
}
