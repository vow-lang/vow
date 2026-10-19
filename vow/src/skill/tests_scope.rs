use super::*;
use crate::{Args, Command, SkillAction};
use clap::Parser;
use tempfile::TempDir;

fn local_project_with_monolithic_skill() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let skill_dir = dir.path().join(".claude/skills/vow");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(dir.path().join(".git"), "gitdir: ../real-git-dir\n").unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), "user-managed content").unwrap();
    (dir, skill_dir)
}

fn install_local(cwd: &Path) -> PathBuf {
    let mut stdin = std::io::Cursor::new(Vec::<u8>::new());
    let mut stderr = Vec::new();
    run_skill_install_scoped(cwd, None, true, false, &mut stdin, &mut stderr).unwrap()
}

#[test]
fn explicit_install_migrates_monolithic_skill_to_split_layout() {
    let (dir, skill_dir) = local_project_with_monolithic_skill();

    let installed = install_local(dir.path());

    assert_eq!(installed, skill_dir.join("SKILL.md"));
    assert_eq!(
        std::fs::read_to_string(&installed).unwrap(),
        entrypoint_markdown()
    );
    for (relative_path, contents) in skill_support_files() {
        assert_eq!(
            std::fs::read_to_string(skill_dir.join(relative_path)).unwrap(),
            *contents,
            "explicit install must write {relative_path}"
        );
    }
}

#[test]
fn explicit_install_repairs_missing_support_files_and_keeps_foreign_files() {
    let (dir, skill_dir) = local_project_with_monolithic_skill();
    install_local(dir.path());
    std::fs::remove_dir_all(skill_dir.join("reference")).unwrap();
    std::fs::write(skill_dir.join("notes.md"), "mine").unwrap();

    install_local(dir.path());

    assert!(skill_dir.join("reference/cli.md").exists());
    assert_eq!(
        std::fs::read_to_string(skill_dir.join("notes.md")).unwrap(),
        "mine"
    );
}

#[test]
fn skill_install_local_writes_into_current_git_project() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    std::fs::write(dir.path().join(".git"), "gitdir: ../real-git-dir\n").unwrap();

    let installed = install_local(dir.path());

    assert_eq!(installed, dir.path().join(".claude/skills/vow/SKILL.md"));
    assert!(installed.exists());
    assert!(
        dir.path()
            .join(".claude/skills/vow/reference/cli.md")
            .exists()
    );
}

#[test]
fn skill_install_accepts_explicit_scope_flags() {
    let parsed = Args::try_parse_from(["vow", "skill", "install", "--local"]).unwrap();
    let Some(Command::Skill(skill)) = parsed.command else {
        panic!("expected skill command");
    };
    let Some(SkillAction::Install { local, global }) = skill.action else {
        panic!("expected install action");
    };
    assert!(local);
    assert!(!global);

    let parsed = Args::try_parse_from(["vow", "skill", "install", "--global"]).unwrap();
    let Some(Command::Skill(skill)) = parsed.command else {
        panic!("expected skill command");
    };
    let Some(SkillAction::Install { local, global }) = skill.action else {
        panic!("expected install action");
    };
    assert!(!local);
    assert!(global);
}

#[test]
fn skill_install_global_writes_under_home() {
    let cwd = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let mut stdin = std::io::Cursor::new(Vec::<u8>::new());
    let mut stderr = Vec::new();

    let installed = run_skill_install_scoped(
        cwd.path(),
        Some(home.path()),
        false,
        true,
        &mut stdin,
        &mut stderr,
    )
    .unwrap();

    assert_eq!(installed, home.path().join(".claude/skills/vow/SKILL.md"));
    assert!(installed.exists());
    assert!(
        home.path()
            .join(".claude/skills/vow/schemas/diagnostic.schema.json")
            .exists()
    );
}

#[test]
fn skill_install_local_requires_git_and_claude_project() {
    let dir = TempDir::new().unwrap();
    let mut stdin = std::io::Cursor::new(Vec::<u8>::new());
    let mut stderr = Vec::new();

    let err = run_skill_install_scoped(dir.path(), None, true, false, &mut stdin, &mut stderr)
        .unwrap_err();
    assert!(err.contains(".claude"));

    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    let err = run_skill_install_scoped(dir.path(), None, true, false, &mut stdin, &mut stderr)
        .unwrap_err();
    assert!(err.contains("git"));
}

#[test]
fn skill_install_prompts_when_scope_is_omitted() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    std::fs::write(dir.path().join(".git"), "gitdir: ../real-git-dir\n").unwrap();
    let mut stdin = std::io::Cursor::new(b"local\n".to_vec());
    let mut stderr = Vec::new();

    let installed =
        run_skill_install_scoped(dir.path(), None, false, false, &mut stdin, &mut stderr).unwrap();
    assert_eq!(installed, dir.path().join(".claude/skills/vow/SKILL.md"));
    assert!(String::from_utf8(stderr).unwrap().contains("[l/g]"));

    let mut stdin = std::io::Cursor::new(Vec::<u8>::new());
    let mut stderr = Vec::new();
    let err = run_skill_install_scoped(dir.path(), None, false, false, &mut stdin, &mut stderr)
        .unwrap_err();
    assert!(err.contains("--local or --global"));
}
