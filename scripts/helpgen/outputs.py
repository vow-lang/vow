"""Collect, write and check every generated file the help generator owns."""

from pathlib import Path

from helpgen.config import REPO
from helpgen.emit_rust import rust_help_files, rust_skill_files
from helpgen.emit_vow import vow_help_files, vow_skill_files

# Shard files the generator owns; a shard it no longer emits is deleted.
SHARD_GLOBS = (
    ("compiler", "skill_bundle_*_gen.vow"),
    ("compiler", "skill_docs_*_gen.vow"),
    ("vow/src/skill_gen", "bundle_*.rs"),
    ("vow/src/skill_gen", "docs_*.rs"),
)


def collect_files(
    json_str: str,
    human_str: str,
    entrypoint_md: str,
    bundle_md: str,
    support_files: dict[str, str],
) -> dict[Path, str]:
    """Map every generated file to its expected text."""
    generated = {
        **vow_help_files(json_str, human_str),
        **vow_skill_files(entrypoint_md, bundle_md, support_files),
        **rust_help_files(json_str, human_str),
        **rust_skill_files(entrypoint_md, bundle_md, support_files),
    }
    return {REPO / rel: text for rel, text in generated.items()}


def stale_files(files: dict[Path, str]) -> list[Path]:
    found: list[Path] = []
    for directory, pattern in SHARD_GLOBS:
        found += [p for p in (REPO / directory).glob(pattern) if p not in files]
    return sorted(found)


def write_files(files: dict[Path, str]) -> list[Path]:
    """Write changed files, delete stale shards; return the paths touched."""
    touched = []
    for path, text in files.items():
        if not path.exists() or path.read_text() != text:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
            touched.append(path)
    for path in stale_files(files):
        path.unlink()
        touched.append(path)
    return touched


def check_files(files: dict[Path, str]) -> list[str]:
    errors = []
    for path, text in files.items():
        if not path.exists():
            errors.append(f"missing: {path.relative_to(REPO)}")
        elif path.read_text() != text:
            errors.append(f"drift: {path.relative_to(REPO)}")
    errors += [f"orphan: {p.relative_to(REPO)}" for p in stale_files(files)]
    return errors
