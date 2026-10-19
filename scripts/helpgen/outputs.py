"""Collect, write and check every generated file the help generator owns."""

from pathlib import Path

from helpgen.config import REPO, SKILL_RS
from helpgen.emit_rust import inject_rust, inject_skill_rust
from helpgen.emit_vow import vow_help_files, vow_skill_files

# Shard modules the generator owns; a shard it no longer emits is deleted.
SHARD_GLOBS = ("skill_bundle_*_gen.vow", "skill_docs_*_gen.vow")


def collect_files(
    json_str: str,
    human_str: str,
    entrypoint_md: str,
    bundle_md: str,
    support_files: dict[str, str],
) -> dict[Path, str]:
    """Map every generated file to its expected text."""
    vow = {
        **vow_help_files(json_str, human_str),
        **vow_skill_files(entrypoint_md, bundle_md, support_files),
    }
    files = {REPO / rel: text for rel, text in vow.items()}
    rust = inject_rust(SKILL_RS, json_str, human_str)
    files[SKILL_RS] = inject_skill_rust(rust, entrypoint_md, bundle_md, support_files)
    return files


def stale_files(files: dict[Path, str]) -> list[Path]:
    found: list[Path] = []
    for pattern in SHARD_GLOBS:
        found += [p for p in (REPO / "compiler").glob(pattern) if p not in files]
    return sorted(found)


def write_files(files: dict[Path, str]) -> list[Path]:
    """Write changed files, delete stale shards; return the paths touched."""
    touched = []
    for path, text in files.items():
        if not path.exists() or path.read_text() != text:
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
