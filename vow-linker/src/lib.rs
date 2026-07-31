use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

const RUNTIME_LIBRARY: &str = "libvow_runtime.a";
const RUNTIME_PATH_ENV: &str = "VOW_RUNTIME_PATH";
const SHIM_LIBRARY: &str = "libvow_clif_shim.a";
const SHIM_PATH_ENV: &str = "VOW_CLIF_SHIM_PATH";

/// A failure to invoke the native linker or a non-zero linker exit status.
#[derive(Debug)]
pub enum LinkError {
    Invoke(std::io::Error),
    Failed(ExitStatus),
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invoke(error) => write!(f, "failed to invoke cc: {error}"),
            Self::Failed(status) => write!(f, "cc exited with status {status}"),
        }
    }
}

impl std::error::Error for LinkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Invoke(error) => Some(error),
            Self::Failed(_) => None,
        }
    }
}

/// Links native `inputs` into an executable at `output` using the system C
/// compiler and the libraries required by the Vow runtime.
pub fn link_executable<'a>(
    inputs: impl IntoIterator<Item = &'a Path>,
    output: &Path,
) -> Result<(), LinkError> {
    link_executable_with_reproducibility(inputs, output, false)
}

/// Links native `inputs` into an executable whose platform metadata is stable
/// across output paths. This is used by the self-hosted compiler's fixed-point
/// build; it is currently distinct from [`link_executable`] only on macOS.
pub fn link_reproducible_executable<'a>(
    inputs: impl IntoIterator<Item = &'a Path>,
    output: &Path,
) -> Result<(), LinkError> {
    link_executable_with_reproducibility(inputs, output, true)
}

fn link_executable_with_reproducibility<'a>(
    inputs: impl IntoIterator<Item = &'a Path>,
    output: &Path,
    reproducible: bool,
) -> Result<(), LinkError> {
    let mut command = Command::new("cc");
    command.args(inputs);
    command.arg("-o").arg(output);
    command.args(platform_link_args_for(std::env::consts::OS));
    add_reproducibility_args(&mut command, reproducible);

    let status = command.status().map_err(LinkError::Invoke)?;
    if status.success() {
        Ok(())
    } else {
        Err(LinkError::Failed(status))
    }
}

#[cfg(target_os = "macos")]
fn add_reproducibility_args(command: &mut Command, reproducible: bool) {
    if reproducible {
        // Stabilise LC_UUID and CDHash across different -o names; see #500.
        command.args(["-Wl,-reproducible", "-Wl,-final_output,vow"]);
    }
}

#[cfg(not(target_os = "macos"))]
fn add_reproducibility_args(_command: &mut Command, _reproducible: bool) {}

/// Returns the native libraries needed to link the Vow runtime on `os`.
pub fn platform_link_args_for(os: &str) -> &'static [&'static str] {
    match os {
        "linux" => &["-lpthread", "-ldl", "-lm"],
        "macos" => &["-lpthread", "-lm"],
        _ => &["-lpthread", "-lm"],
    }
}

/// Finds the compiled Vow runtime library using environment, installed-prefix,
/// and Cargo build-tree locations in precedence order.
pub fn find_runtime_lib() -> Option<PathBuf> {
    find_library(RUNTIME_LIBRARY, RUNTIME_PATH_ENV)
}

/// Finds the compiled self-hosted Cranelift shim using environment,
/// installed-prefix, and Cargo build-tree locations in precedence order.
pub fn find_shim_lib() -> Option<PathBuf> {
    find_library(SHIM_LIBRARY, SHIM_PATH_ENV)
}

fn find_library(name: &str, env_var: &str) -> Option<PathBuf> {
    let executable = std::env::current_exe().ok();
    find_library_from_parts(name, std::env::var_os(env_var), executable.as_deref())
}

fn find_library_from_parts(
    name: &str,
    env_value: Option<std::ffi::OsString>,
    executable: Option<&Path>,
) -> Option<PathBuf> {
    let target_dir = cargo_target_dir();
    find_library_from_parts_with_target_dir(name, env_value, executable, &target_dir)
}

fn find_library_from_parts_with_target_dir(
    name: &str,
    env_value: Option<std::ffi::OsString>,
    executable: Option<&Path>,
    target_dir: &Path,
) -> Option<PathBuf> {
    if let Some(path) = env_value.map(PathBuf::from)
        && path.exists()
    {
        return Some(path);
    }

    if let Some(executable) = executable
        && let Some(path) = find_installed_library(name, executable)
    {
        return Some(path);
    }

    find_library_in_cargo_target(name, target_dir)
}

fn find_installed_library(name: &str, executable: &Path) -> Option<PathBuf> {
    let executable_dir = executable.parent();
    let prefix_dir = executable_dir.and_then(Path::parent);
    // Preserve the legacy adjacent-to-executable lookup before prefix paths so
    // manual installs that co-locate the static libraries with vowc keep
    // working.
    let candidates = [
        executable_dir.map(|dir| dir.join(name)),
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

fn find_library_in_cargo_target(name: &str, target_dir: &Path) -> Option<PathBuf> {
    // Development fallback only: env overrides and installed-prefix libraries
    // are checked first. Prefer `release` over `debug`: bootstrap builds release
    // archives, so a stale debug archive must not shadow a newly built runtime.
    ["release", "debug"]
        .into_iter()
        .map(|profile| target_dir.join(profile).join(name))
        .find(|candidate| candidate.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_library_in_installed_lib_vow_dir() {
        let root = tempfile::TempDir::new().unwrap();
        let bin_dir = root.path().join("bin");
        let lib_dir = root.path().join("lib").join("vow");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::create_dir_all(&lib_dir).unwrap();
        let executable = bin_dir.join("vowc");
        let library = lib_dir.join(RUNTIME_LIBRARY);
        std::fs::write(&executable, b"").unwrap();
        std::fs::write(&library, b"").unwrap();

        let found = find_library_from_parts_with_target_dir(
            RUNTIME_LIBRARY,
            None,
            Some(&executable),
            &root.path().join("target"),
        );
        assert_eq!(found.as_deref(), Some(library.as_path()));
    }

    #[test]
    fn finds_library_in_installed_lib_dir() {
        let root = tempfile::TempDir::new().unwrap();
        let bin_dir = root.path().join("bin");
        let lib_dir = root.path().join("lib");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::create_dir_all(&lib_dir).unwrap();
        let executable = bin_dir.join("vowc");
        let library = lib_dir.join(RUNTIME_LIBRARY);
        std::fs::write(&executable, b"").unwrap();
        std::fs::write(&library, b"").unwrap();

        let found = find_library_from_parts_with_target_dir(
            RUNTIME_LIBRARY,
            None,
            Some(&executable),
            &root.path().join("target"),
        );
        assert_eq!(found.as_deref(), Some(library.as_path()));
    }

    #[test]
    fn env_override_does_not_require_current_executable() {
        let root = tempfile::TempDir::new().unwrap();
        let library = root.path().join(RUNTIME_LIBRARY);
        std::fs::write(&library, b"").unwrap();

        let found = find_library_from_parts(
            RUNTIME_LIBRARY,
            Some(library.clone().into_os_string()),
            None,
        );
        assert_eq!(found.as_deref(), Some(library.as_path()));
    }

    #[test]
    fn cargo_target_fallback_does_not_require_current_executable() {
        let root = tempfile::TempDir::new().unwrap();
        let debug_dir = root.path().join("debug");
        std::fs::create_dir_all(&debug_dir).unwrap();
        let library = debug_dir.join(RUNTIME_LIBRARY);
        std::fs::write(&library, b"").unwrap();

        let found =
            find_library_from_parts_with_target_dir(RUNTIME_LIBRARY, None, None, root.path());
        assert_eq!(found.as_deref(), Some(library.as_path()));
    }

    #[test]
    fn cargo_target_fallback_accepts_release_when_debug_missing() {
        let root = tempfile::TempDir::new().unwrap();
        let release_dir = root.path().join("release");
        std::fs::create_dir_all(&release_dir).unwrap();
        let library = release_dir.join(RUNTIME_LIBRARY);
        std::fs::write(&library, b"").unwrap();

        let found =
            find_library_from_parts_with_target_dir(RUNTIME_LIBRARY, None, None, root.path());
        assert_eq!(found.as_deref(), Some(library.as_path()));
    }

    #[test]
    fn cargo_target_fallback_prefers_release_before_debug() {
        let root = tempfile::TempDir::new().unwrap();
        let debug_dir = root.path().join("debug");
        let release_dir = root.path().join("release");
        std::fs::create_dir_all(&debug_dir).unwrap();
        std::fs::create_dir_all(&release_dir).unwrap();
        let debug_library = debug_dir.join(RUNTIME_LIBRARY);
        let release_library = release_dir.join(RUNTIME_LIBRARY);
        std::fs::write(&debug_library, b"debug").unwrap();
        std::fs::write(&release_library, b"release").unwrap();

        let found =
            find_library_from_parts_with_target_dir(RUNTIME_LIBRARY, None, None, root.path());
        assert_eq!(found.as_deref(), Some(release_library.as_path()));
    }

    #[test]
    fn cargo_target_fallback_accepts_debug_when_release_missing() {
        let root = tempfile::TempDir::new().unwrap();
        let debug_dir = root.path().join("debug");
        std::fs::create_dir_all(&debug_dir).unwrap();
        let library = debug_dir.join(RUNTIME_LIBRARY);
        std::fs::write(&library, b"debug").unwrap();

        let found =
            find_library_from_parts_with_target_dir(RUNTIME_LIBRARY, None, None, root.path());
        assert_eq!(found.as_deref(), Some(library.as_path()));
    }
}
