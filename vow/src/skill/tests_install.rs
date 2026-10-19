use super::*;
use tempfile::TempDir;

fn skills_entries(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root.join(".claude/skills"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn seed_old_tree(root: &Path) -> PathBuf {
    let target = root.join(".claude/skills/vow");
    std::fs::create_dir_all(target.join("reference")).unwrap();
    std::fs::write(target.join("SKILL.md"), "old").unwrap();
    std::fs::write(target.join("reference/stale.md"), "stale").unwrap();
    target
}

fn assert_old_tree_intact(target: &Path) {
    assert_eq!(
        std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
        "old"
    );
    assert_eq!(
        std::fs::read_to_string(target.join("reference/stale.md")).unwrap(),
        "stale"
    );
}

const GOOD_FILES: &[(&str, &str)] = &[("reference/a.md", "a"), ("schemas/b.json", "{}")];
const BAD_FILES: &[(&str, &str)] = &[("a", "x"), ("a/b.md", "y")];

#[test]
fn atomic_install_failure_leaves_no_partial_tree() {
    let dir = TempDir::new().unwrap();
    let err = install_tree_atomic(dir.path(), "entry", BAD_FILES).unwrap_err();
    assert!(err.to_string().contains("cannot create"), "{err}");
    assert!(!dir.path().join(".claude/skills/vow").exists());
    assert!(skills_entries(dir.path()).is_empty());
}

#[test]
fn atomic_install_success_leaves_only_the_skill_directory() {
    let dir = TempDir::new().unwrap();
    install_skill_tree_to(dir.path()).unwrap();
    assert_eq!(skills_entries(dir.path()), vec!["vow".to_string()]);
    let skill_dir = dir.path().join(".claude/skills/vow");
    for (rel, expected) in skill_support_files() {
        assert_eq!(
            &std::fs::read_to_string(skill_dir.join(rel)).unwrap(),
            expected
        );
    }
}

#[test]
fn write_skill_files_writes_entrypoint_last() {
    let dir = TempDir::new().unwrap();
    let err = write_skill_files(dir.path(), "entry", BAD_FILES).unwrap_err();
    assert!(err.to_string().contains("cannot create"), "{err}");
    assert!(!dir.path().join("SKILL.md").exists());
}

#[test]
fn atomic_install_rewrites_owned_files_and_carries_over_foreign_ones() {
    let dir = TempDir::new().unwrap();
    let target = seed_old_tree(dir.path());
    std::fs::write(target.join("reference/a.md"), "edited").unwrap();
    std::fs::create_dir_all(target.join("mine/deep")).unwrap();
    std::fs::write(target.join("mine/deep/x.txt"), "deep").unwrap();
    std::fs::write(target.join("notes.md"), "notes").unwrap();
    let installed = install_tree_atomic(dir.path(), "new", GOOD_FILES).unwrap();
    assert_eq!(installed, target.join("SKILL.md"));
    assert_eq!(std::fs::read_to_string(&installed).unwrap(), "new");
    let read = |rel: &str| std::fs::read_to_string(target.join(rel)).unwrap();
    assert_eq!(read("reference/a.md"), "a");
    assert_eq!(read("reference/stale.md"), "stale");
    assert_eq!(read("mine/deep/x.txt"), "deep");
    assert_eq!(read("notes.md"), "notes");
    assert_eq!(skills_entries(dir.path()), vec!["vow".to_string()]);
}

#[cfg(unix)]
#[test]
fn atomic_install_carries_over_symlinks_as_links() {
    let dir = TempDir::new().unwrap();
    let target = seed_old_tree(dir.path());
    std::os::unix::fs::symlink("missing-target", target.join("link")).unwrap();
    install_tree_atomic(dir.path(), "new", GOOD_FILES).unwrap();
    assert_eq!(
        std::fs::read_link(target.join("link")).unwrap(),
        PathBuf::from("missing-target")
    );
}

#[test]
fn atomic_install_failure_keeps_existing_tree() {
    let dir = TempDir::new().unwrap();
    let target = seed_old_tree(dir.path());
    install_tree_atomic(dir.path(), "new", BAD_FILES).unwrap_err();
    assert_old_tree_intact(&target);
    assert_eq!(skills_entries(dir.path()), vec!["vow".to_string()]);
}

#[test]
fn atomic_install_repairs_orphan_entrypoint_blocked_by_a_file() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join(".claude/skills/vow");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("reference"), "not a directory").unwrap();
    install_tree_atomic(dir.path(), "new", GOOD_FILES).unwrap();
    assert!(target.join("reference/a.md").is_file());
    assert_eq!(
        std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
        "new"
    );
}

/// Fails the renames whose 1-based call number is in `failing`.
fn install_with_failing_renames(root: &Path, failing: &'static [u32]) -> std::io::Result<PathBuf> {
    let calls = std::cell::Cell::new(0u32);
    install_tree_with(root, "new", GOOD_FILES, &unique_token, &|from, to| {
        calls.set(calls.get() + 1);
        if failing.contains(&calls.get()) {
            Err(std::io::Error::other("injected rename failure"))
        } else {
            std::fs::rename(from, to)
        }
    })
}

#[test]
fn commit_failure_on_fresh_install_cleans_the_stage() {
    let dir = TempDir::new().unwrap();
    let err = install_with_failing_renames(dir.path(), &[1]).unwrap_err();
    assert!(err.to_string().contains("cannot install"), "{err}");
    assert!(!dir.path().join(".claude/skills/vow").exists());
    assert!(skills_entries(dir.path()).is_empty());
}

#[test]
fn commit_failure_restores_the_previous_tree() {
    let dir = TempDir::new().unwrap();
    let target = seed_old_tree(dir.path());
    let err = install_with_failing_renames(dir.path(), &[2]).unwrap_err();
    assert!(err.to_string().contains("cannot install"), "{err}");
    assert_old_tree_intact(&target);
    assert_eq!(skills_entries(dir.path()), vec!["vow".to_string()]);
}

#[test]
fn failed_backup_rename_leaves_the_previous_tree_and_no_backup() {
    let dir = TempDir::new().unwrap();
    let target = seed_old_tree(dir.path());
    install_with_failing_renames(dir.path(), &[1]).unwrap_err();
    assert_old_tree_intact(&target);
    assert_eq!(skills_entries(dir.path()), vec!["vow".to_string()]);
}

#[test]
fn failed_restore_reports_where_the_previous_tree_is_kept() {
    let dir = TempDir::new().unwrap();
    seed_old_tree(dir.path());
    let err = install_with_failing_renames(dir.path(), &[2, 3]).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("previous tree kept at"), "{message}");
    let backup = message.rsplit("kept at ").next().unwrap();
    assert_eq!(
        std::fs::read_to_string(Path::new(backup).join("SKILL.md")).unwrap(),
        "old"
    );
}

#[test]
fn reserve_unique_dir_retries_taken_names() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir(dir.path().join(".s-a")).unwrap();
    let token = |attempt: u32| if attempt == 0 { "a" } else { "b" }.to_string();
    let reserved = reserve_unique_dir(dir.path(), "s", &token).unwrap();
    assert_eq!(reserved, dir.path().join(".s-b"));
}

#[test]
fn reserve_unique_dir_gives_up_when_every_name_is_taken() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir(dir.path().join(".s-a")).unwrap();
    let err = reserve_unique_dir(dir.path(), "s", &|_| "a".to_string()).unwrap_err();
    assert!(err.to_string().contains("no unused name"), "{err}");
}

#[test]
fn reserve_unique_dir_reports_other_failures() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("missing");
    let err = reserve_unique_dir(&missing, "s", &|_| "a".to_string()).unwrap_err();
    assert!(err.to_string().contains("cannot create"), "{err}");
}

#[test]
fn install_fails_cleanly_when_skills_path_is_a_file() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    std::fs::write(dir.path().join(".claude/skills"), "file").unwrap();
    let err = install_skill_tree_to(dir.path()).unwrap_err();
    assert!(err.to_string().contains("cannot create"), "{err}");
    maybe_auto_install(dir.path());
    assert!(dir.path().join(".claude/skills").is_file());
}

#[cfg(unix)]
#[test]
fn atomic_install_writes_through_a_symlinked_skill_directory() {
    let dir = TempDir::new().unwrap();
    let real = dir.path().join("dotfiles/vow");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::create_dir_all(dir.path().join(".claude/skills")).unwrap();
    let link = dir.path().join(".claude/skills/vow");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    install_tree_atomic(dir.path(), "new", GOOD_FILES).unwrap();
    assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(
        std::fs::read_to_string(real.join("SKILL.md")).unwrap(),
        "new"
    );
    assert!(real.join("schemas/b.json").is_file());
    assert_eq!(skills_entries(dir.path()), vec!["vow".to_string()]);
}

#[cfg(unix)]
#[test]
fn symlinked_install_failure_leaves_no_entrypoint() {
    let dir = TempDir::new().unwrap();
    let real = dir.path().join("dotfiles/vow");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::create_dir_all(dir.path().join(".claude/skills")).unwrap();
    std::os::unix::fs::symlink(&real, dir.path().join(".claude/skills/vow")).unwrap();

    install_tree_atomic(dir.path(), "new", BAD_FILES).unwrap_err();
    assert!(!real.join("SKILL.md").exists());
}

#[cfg(unix)]
#[test]
fn unwritable_skills_directory_keeps_the_previous_tree() {
    use std::os::unix::fs::PermissionsExt;

    struct RestoreMode(PathBuf);
    impl Drop for RestoreMode {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
    }

    let dir = TempDir::new().unwrap();
    let target = seed_old_tree(dir.path());
    let skills = dir.path().join(".claude/skills");
    let _restore = RestoreMode(skills.clone());
    std::fs::set_permissions(&skills, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::File::create(skills.join("probe")).is_ok() {
        return;
    }

    let err = install_tree_atomic(dir.path(), "new", GOOD_FILES).unwrap_err();
    assert!(err.to_string().contains("cannot create"), "{err}");
    assert_old_tree_intact(&target);
}

#[test]
fn auto_install_skill_skips_when_no_claude_dir() {
    let dir = TempDir::new().unwrap();
    maybe_auto_install(dir.path());
    let claude_dir = dir.path().join(".claude");
    assert!(
        !claude_dir.exists(),
        "auto-install must not create .claude/ when absent"
    );
}

#[test]
fn auto_install_skill_installs_when_claude_dir_present() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    maybe_auto_install(dir.path());
    let installed = dir.path().join(".claude/skills/vow/SKILL.md");
    assert!(
        installed.exists(),
        "auto-install must populate the skill when .claude/ is present"
    );
    let contents = std::fs::read_to_string(installed).unwrap();
    assert!(!contents.contains("\nallowed-tools:"));
}

#[test]
fn auto_install_skill_leaves_existing_file_untouched() {
    let dir = TempDir::new().unwrap();
    let target_dir = dir.path().join(".claude/skills/vow");
    std::fs::create_dir_all(&target_dir).unwrap();
    let target = target_dir.join("SKILL.md");
    std::fs::write(&target, "user-managed content").unwrap();
    maybe_auto_install(dir.path());
    let contents = std::fs::read_to_string(&target).unwrap();
    assert_eq!(contents, "user-managed content");
    for support_dir in ["reference", "examples", "schemas"] {
        assert!(
            !target_dir.join(support_dir).exists(),
            "auto-install must not add {support_dir}/ next to an existing SKILL.md"
        );
    }
}
