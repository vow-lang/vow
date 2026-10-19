//! The `vow skill` module: the embedded skill documentation (the `--help`
//! machine JSON, the human `--help` text, and the installable SKILL.md
//! entrypoint + reference/schema support files) together with the logic that
//! installs that tree into a project or the user's home.
//!
//! main.rs sees only the small interface below — `json`, `human`,
//! `entrypoint_markdown`, `bundle_markdown`, `install`, `maybe_auto_install`.
//! The ~10k lines of embedded documentation and the install mechanics stay
//! behind that seam.
//!
//! The embedded text lives in `vow/src/skill_gen/`, whole files that
//! `scripts/generate_help.py` writes from `docs/spec/`; do not edit them by hand.

use std::path::{Path, PathBuf};

// --- interface: embedded documentation ---------------------------------------

/// Machine-readable `--help` output (schema_version 2 tool_help JSON).
pub(crate) fn json() -> String {
    skill_json()
}

/// Human-readable `--help` output.
pub(crate) fn human() -> String {
    skill_human()
}

/// The concise installed `SKILL.md` entrypoint.
pub(crate) fn entrypoint_markdown() -> String {
    skill_entrypoint_markdown()
}

/// The full single-file skill bundle (`vow skill print --bundle`).
pub(crate) fn bundle_markdown() -> String {
    skill_bundle_markdown()
}

// --- install mechanics -------------------------------------------------------

const RESERVE_ATTEMPTS: u32 = 8;

type RenameFn<'a> = &'a dyn Fn(&Path, &Path) -> std::io::Result<()>;

fn io_ctx(op: &str, path: &Path, e: std::io::Error) -> std::io::Error {
    std::io::Error::new(e.kind(), format!("cannot {op} {}: {e}", path.display()))
}

fn unique_token(attempt: u32) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{}-{nanos}-{attempt}", std::process::id())
}

/// Exclusively create `<parent>/.<stem>-<token>`: `create_dir` fails when the
/// name is taken, so two concurrent installers never share a directory.
fn reserve_unique_dir(
    parent: &Path,
    stem: &str,
    token: &dyn Fn(u32) -> String,
) -> std::io::Result<PathBuf> {
    for attempt in 0..RESERVE_ATTEMPTS {
        let candidate = parent.join(format!(".{stem}-{}", token(attempt)));
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(io_ctx("create", &candidate, e)),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        format!(
            "cannot create {}: no unused name after {RESERVE_ATTEMPTS} attempts",
            parent.join(stem).display()
        ),
    ))
}

/// Writes every support file, then the entrypoint, so a directory that holds
/// `SKILL.md` always holds the files it links to.
fn write_skill_files(dir: &Path, entrypoint: &str, files: &[(&str, &str)]) -> std::io::Result<()> {
    for (relative_path, contents) in files {
        let path = dir.join(relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_ctx("create", parent, e))?;
        }
        std::fs::write(&path, contents).map_err(|e| io_ctx("write", &path, e))?;
    }
    let path = dir.join("SKILL.md");
    std::fs::write(&path, entrypoint).map_err(|e| io_ctx("write", &path, e))
}

/// Copies every entry of `from` that `into` does not already hold, so files the
/// installer does not own (local notes, stale files in owned directories) survive
/// the swap while owned files always come from the freshly written stage.
fn carry_over_foreign(from: &Path, into: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(from).map_err(|e| io_ctx("read", from, e))? {
        let entry = entry.map_err(|e| io_ctx("read", from, e))?;
        let (src, dst) = (entry.path(), into.join(entry.file_name()));
        let kind = entry.file_type().map_err(|e| io_ctx("read", &src, e))?;
        if dst.symlink_metadata().is_ok() {
            if kind.is_dir() && dst.is_dir() {
                carry_over_foreign(&src, &dst)?;
            }
        } else if kind.is_dir() {
            std::fs::create_dir(&dst).map_err(|e| io_ctx("create", &dst, e))?;
            carry_over_foreign(&src, &dst)?;
        } else if kind.is_symlink() {
            let link = std::fs::read_link(&src).map_err(|e| io_ctx("read", &src, e))?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(&link, &dst).map_err(|e| io_ctx("write", &dst, e))?;
            #[cfg(not(unix))]
            let _ = link;
        } else {
            std::fs::copy(&src, &dst).map_err(|e| io_ctx("write", &dst, e))?;
        }
    }
    Ok(())
}

/// Moves a fully written `stage` to `target`. An existing `target` is swapped
/// out through a backup, since a directory cannot be renamed over a non-empty
/// one, and put back when the final rename fails.
fn commit_stage(
    parent: &Path,
    stage: &Path,
    target: &Path,
    token: &dyn Fn(u32) -> String,
    rename: RenameFn,
) -> std::io::Result<()> {
    if target.symlink_metadata().is_err() {
        return rename(stage, target).map_err(|e| io_ctx("install", target, e));
    }
    let backup = reserve_unique_dir(parent, "vow-backup", token)?;
    if let Err(e) = rename(target, &backup) {
        let _ = std::fs::remove_dir(&backup);
        return Err(io_ctx("install", target, e));
    }
    if let Err(e) = rename(stage, target) {
        return Err(match rename(&backup, target) {
            Ok(()) => io_ctx("install", target, e),
            Err(_) => std::io::Error::new(
                e.kind(),
                format!(
                    "cannot install {}: {e}; previous tree kept at {}",
                    target.display(),
                    backup.display()
                ),
            ),
        });
    }
    let _ = std::fs::remove_dir_all(&backup);
    Ok(())
}

fn install_tree_with(
    root: &Path,
    entrypoint: &str,
    files: &[(&str, &str)],
    token: &dyn Fn(u32) -> String,
    rename: RenameFn,
) -> std::io::Result<PathBuf> {
    let parent = root.join(".claude/skills");
    let target = parent.join("vow");
    std::fs::create_dir_all(&parent).map_err(|e| io_ctx("create", &parent, e))?;

    let is_link = target
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink());
    if is_link {
        write_skill_files(&target, entrypoint, files)?;
        return Ok(target.join("SKILL.md"));
    }

    let stage = reserve_unique_dir(&parent, "vow-install", token)?;
    let committed = write_skill_files(&stage, entrypoint, files)
        .and_then(|()| {
            if target.is_dir() {
                carry_over_foreign(&target, &stage)
            } else {
                Ok(())
            }
        })
        .and_then(|()| commit_stage(&parent, &stage, &target, token, rename));
    if let Err(e) = committed {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(e);
    }
    Ok(target.join("SKILL.md"))
}

/// Builds the skill tree in a sibling staging directory and renames it into
/// `<root>/.claude/skills/vow`, so a failure never leaves a `SKILL.md` that
/// links to missing files. A symlinked `vow` directory is written in place.
fn install_tree_atomic(
    root: &Path,
    entrypoint: &str,
    files: &[(&str, &str)],
) -> std::io::Result<PathBuf> {
    install_tree_with(root, entrypoint, files, &unique_token, &|from, to| {
        std::fs::rename(from, to)
    })
}

fn install_skill_tree_to(root: &Path) -> std::io::Result<PathBuf> {
    install_tree_atomic(root, &skill_entrypoint_markdown(), skill_support_files())
}

fn run_skill_install_scoped<R: std::io::BufRead, W: std::io::Write>(
    cwd: &Path,
    home: Option<&Path>,
    local: bool,
    global: bool,
    stdin: &mut R,
    stderr: &mut W,
) -> Result<PathBuf, String> {
    if local && global {
        return Err("vow skill install: --local and --global are mutually exclusive".to_string());
    }

    let mut use_local = local;
    let mut use_global = global;
    if !use_local && !use_global {
        stderr
            .write_all(b"Install Vow skill locally (./.claude) or globally (~/.claude)? [l/g]: ")
            .map_err(|e| format!("vow skill install: cannot write prompt: {e}"))?;
        let mut answer = String::new();
        stdin
            .read_line(&mut answer)
            .map_err(|e| format!("vow skill install: cannot read answer: {e}"))?;
        let normalized = answer.trim().to_ascii_lowercase();
        if normalized == "l" || normalized == "local" {
            use_local = true;
        } else if normalized == "g" || normalized == "global" {
            use_global = true;
        } else {
            return Err(
                "vow skill install: stdin closed or unrecognised answer; pass --local or --global"
                    .to_string(),
            );
        }
    }

    let root = if use_local {
        if !cwd.join(".claude").is_dir() {
            return Err(
                "vow skill install: --local requires a .claude/ directory in the current project"
                    .to_string(),
            );
        }
        if !cwd.join(".git").exists() {
            return Err(
                "vow skill install: --local requires the current project to be a git checkout"
                    .to_string(),
            );
        }
        cwd.to_path_buf()
    } else if use_global {
        home.filter(|path| !path.as_os_str().is_empty())
            .ok_or_else(|| "vow skill install: --global requires $HOME".to_string())?
            .to_path_buf()
    } else {
        return Err("vow skill install: no install target selected".to_string());
    };

    install_skill_tree_to(&root).map_err(|e| format!("vow skill install: {e}"))
}

pub(crate) fn install(local: bool, global: bool) {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    let stderr = std::io::stderr();
    let mut stderr = stderr.lock();
    match run_skill_install_scoped(
        &cwd,
        home.as_deref(),
        local,
        global,
        &mut stdin,
        &mut stderr,
    ) {
        Ok(path) => eprintln!("installed skill to {}", path.display()),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

/// First-run auto-install: when `.claude/` already exists in the working
/// directory but the Vow skill is not yet installed, drop a fresh copy in
/// place. Silent on success, silent on failure — must never fail the build.
pub(crate) fn maybe_auto_install(cwd: &Path) {
    let claude_dir = cwd.join(".claude");
    if !claude_dir.is_dir() {
        return;
    }
    let target = claude_dir.join("skills/vow/SKILL.md");
    if target.exists() {
        return;
    }
    let _ = install_skill_tree_to(cwd);
}

// --- generated documentation (populated by scripts/generate_help.py) ---------

include!("skill_gen/help_text.rs");
include!("skill_gen/skill_text.rs");

#[cfg(test)]
mod tests_docs;
#[cfg(test)]
mod tests_install;
#[cfg(test)]
mod tests_scope;
