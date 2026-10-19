use super::*;
use tempfile::TempDir;

#[test]
fn interface_exposes_help_and_installable_docs() {
    // The module's seam: machine help, human help, the concise entrypoint,
    // and the full bundle. main.rs and tests cross exactly this interface.
    let parsed: serde_json::Value = serde_json::from_str(&json()).unwrap();
    assert_eq!(parsed["kind"], "tool_help");
    assert_eq!(parsed["tool"], "vow");
    assert!(human().contains("USAGE"));
    assert!(entrypoint_markdown().starts_with("---\nname: vow\n"));
    assert!(bundle_markdown().contains("# Vow Language Reference"));
}

#[test]
fn skill_install_writes_concise_entrypoint_and_support_files() {
    let dir = TempDir::new().unwrap();
    install_skill_tree_to(dir.path()).unwrap();

    let skill_dir = dir.path().join(".claude/skills/vow");
    let contents = std::fs::read_to_string(skill_dir.join("SKILL.md")).unwrap();
    assert!(contents.starts_with("---\nname: vow\n"));
    assert!(contents.contains("when_to_use: >-"));
    assert!(!contents.contains("\npaths:"));
    assert!(!contents.contains("\nallowed-tools:"));
    assert!(contents.lines().count() < 500);
    assert!(
        !contents.contains("```json"),
        "SKILL.md entrypoint should link to schemas, not inline them"
    );
    let do_nots = contents.find("## Do nots").expect("Do nots section");
    let toolchain = contents.find("## Installed toolchain").unwrap();
    assert!(do_nots < toolchain);

    let grammar = std::fs::read_to_string(skill_dir.join("reference/grammar.md")).unwrap();
    assert!(grammar.contains("# Vow Grammar Reference"));
    let schema =
        std::fs::read_to_string(skill_dir.join("schemas/build-result.schema.json")).unwrap();
    assert!(schema.contains("\"title\": \"BuildResult\""));
}

#[test]
fn skill_entrypoint_points_raw_harnesses_at_the_bundle() {
    let entry = skill_entrypoint_markdown();
    assert!(
        entry.contains("`build/vowc skill print --bundle`"),
        "entrypoint must name the self-contained bundle command"
    );
}

#[test]
fn skill_bundle_markdown_contains_full_skill_document() {
    let out = skill_bundle_markdown();
    assert!(out.starts_with("---\nname: vow\n"));
    assert!(!out.contains("\npaths:"));
    assert!(!out.contains("\nallowed-tools:"));
    assert!(out.contains("# Vow Language Reference"));
    assert!(out.contains("### `vow skill`"));
    assert!(out.contains("schemas/build-result.schema.json"));
}

/// `npx skills add vow-lang/vow` clones the repo and scans `skills/*/SKILL.md`.
/// The checked-in `skills/vow/` mirror must therefore match what the compiler
/// installs, or the two install paths drift.
#[test]
fn checked_in_skills_vow_matches_install_output() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("CARGO_MANIFEST_DIR has a parent");
    let skill_md = repo_root.join("skills/vow/SKILL.md");
    let checked_in = std::fs::read_to_string(&skill_md).expect("skills/vow/SKILL.md must exist");
    assert!(
        checked_in.starts_with("---\nname: vow\n"),
        "checked-in skills/vow/SKILL.md must declare `name: vow` for npx skills"
    );
    assert_eq!(
        checked_in,
        skill_entrypoint_markdown(),
        "skills/vow/SKILL.md drifted from compiler-embedded skill; \
         run `uv run python scripts/generate_help.py`"
    );
    for (rel, expected) in skill_support_files() {
        let path = repo_root.join("skills/vow").join(rel);
        let actual = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("missing skills/vow/{rel}: {e}"));
        assert_eq!(
            actual, *expected,
            "skills/vow/{rel} drifted from compiler-embedded copy"
        );
    }
}

fn generated_skill_support_count(source: &str) -> usize {
    let body = generated_vow_function_lines(source, "fn skill_support_count() -> u64 {");
    let mut literal_lines = body
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty());
    let literal = literal_lines
        .next()
        .expect("skill_support_count() must return an integer literal");
    assert!(
        literal_lines.next().is_none(),
        "skill_support_count() should contain only the generated count literal"
    );
    literal
        .parse::<usize>()
        .expect("skill_support_count() must return a non-negative integer literal")
}

fn generated_skill_support_branch_count(source: &str, signature: &str) -> usize {
    generated_vow_function_lines(source, signature)
        .into_iter()
        .filter(|line| line.trim_start().starts_with("if index =="))
        .count()
}

fn generated_vow_function_lines<'a>(source: &'a str, signature: &str) -> Vec<&'a str> {
    let mut lines = source.lines().skip_while(|line| line.trim() != signature);
    lines
        .next()
        .unwrap_or_else(|| panic!("generated Vow function `{signature}` must exist"));

    let mut body = Vec::new();
    for line in lines {
        if line.trim() == "}" {
            return body;
        }
        body.push(line);
    }
    panic!("generated Vow function `{signature}` must close with a standalone `}}`");
}

#[test]
fn generated_vow_skill_support_uses_indexed_lookup() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("CARGO_MANIFEST_DIR has a parent");
    let source = generated_vow_skill_source(repo_root);

    assert!(source.contains("fn skill_support_count() -> u64"));
    assert!(source.contains("fn skill_support_path(index: u64) -> String vow {"));
    assert!(source.contains("fn skill_support_content_index_guard(index: u64) vow {"));
    assert!(source.contains("fn skill_support_content(index: u64) -> String {"));
    assert!(source.contains("    skill_support_content_index_guard(index);"));
    assert_eq!(
        source
            .matches("requires: index < skill_support_count()")
            .count(),
        2,
        "indexed support lookup contracts should guard path lookup and content access"
    );
    let support_count = generated_skill_support_count(&source);
    let path_branch_count = generated_skill_support_branch_count(
        &source,
        "fn skill_support_path(index: u64) -> String vow {",
    );
    let content_branch_count = generated_skill_support_branch_count(
        &source,
        "fn skill_support_content(index: u64) -> String {",
    );
    assert_eq!(
        path_branch_count, support_count,
        "skill_support_count() returned {support_count}, \
         but skill_support_path has {path_branch_count} generated index branches"
    );
    assert_eq!(
        content_branch_count, support_count,
        "skill_support_count() returned {support_count}, \
         but skill_support_content has {content_branch_count} generated index branches"
    );
    assert!(
        !source.contains("fn skill_support_paths() -> Vec<String>"),
        "generated Vow should expose indexed path lookup, not a path vector"
    );
    assert!(
        !source.contains("fn skill_support_content_0()"),
        "generated Vow support helpers should not be numbered by bundle position"
    );
    assert!(
        source.contains("fn skill_support_content_reference_grammar_md_92418de9() -> String {"),
        "generated Vow content helpers should use stable path-derived names with an 8-char hash suffix"
    );
    assert!(
        !source.contains("fn skill_support_contents() -> Vec<String>"),
        "generated Vow should stream support contents by index during install"
    );
}

/// The generated skill modules of the self-hosted compiler, concatenated.
fn generated_vow_skill_source(repo_root: &Path) -> String {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(repo_root.join("compiler"))
        .expect("compiler/ must exist")
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("skill_") && name.ends_with("_gen.vow"))
        })
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "compiler/skill_*_gen.vow must exist");
    paths
        .iter()
        .map(|path| std::fs::read_to_string(path).unwrap())
        .collect()
}
