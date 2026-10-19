"""Write and check the checked-in skills/vow/ mirror."""

from helpgen.config import REPO, SKILLS_DIR


def _expected_skills_files(
    entrypoint_md: str, support_files: dict[str, str]
) -> dict[str, str]:
    files = {"SKILL.md": entrypoint_md}
    files.update(support_files)
    return files


def write_skills_dir(entrypoint_md: str, support_files: dict[str, str]) -> None:
    """Write SKILL.md + support files into skills/vow/, removing any stale files."""
    expected = _expected_skills_files(entrypoint_md, support_files)
    SKILLS_DIR.mkdir(parents=True, exist_ok=True)
    for rel, contents in expected.items():
        target = SKILLS_DIR / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(contents)
    expected_paths = {(SKILLS_DIR / rel).resolve() for rel in expected}
    for path in SKILLS_DIR.rglob("*"):
        if path.is_file() and path.resolve() not in expected_paths:
            path.unlink()
    for path in sorted((p for p in SKILLS_DIR.rglob("*") if p.is_dir()), reverse=True):
        try:
            path.rmdir()
        except OSError:
            pass


def check_skills_dir(entrypoint_md: str, support_files: dict[str, str]) -> list[str]:
    """Return list of drift errors; empty if skills/vow/ matches generated content."""
    expected = _expected_skills_files(entrypoint_md, support_files)
    errors: list[str] = []
    for rel, contents in expected.items():
        target = SKILLS_DIR / rel
        if not target.exists():
            errors.append(f"missing: {target.relative_to(REPO)}")
        elif target.read_text() != contents:
            errors.append(f"drift: {target.relative_to(REPO)}")
    expected_paths = {(SKILLS_DIR / rel).resolve() for rel in expected}
    if SKILLS_DIR.exists():
        for path in SKILLS_DIR.rglob("*"):
            if path.is_file() and path.resolve() not in expected_paths:
                errors.append(f"orphan: {path.relative_to(REPO)}")
    return errors
