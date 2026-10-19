"""Splice generated blocks between `// GENERATE:NAME:START/END` markers."""

from __future__ import annotations

from pathlib import Path

MARKER_START = "// GENERATE:OPERATIONS:START"
MARKER_END = "// GENERATE:OPERATIONS:END"


def _wrap_marker_block(body: str) -> str:
    """Wrap a generated function body (ending in "}\\n") in the marker pair
    every splice target shares."""
    return f"{MARKER_START}\n{body}{MARKER_END}"


def _replace_between_markers(
    content: str, start_marker: str, end_marker: str, replacement: str
) -> str:
    """Replace content between start/end marker lines (inclusive).

    Raises ValueError if either marker is missing, or if a second
    `start_marker` appears after the first block's end -- a lone orphaned
    block (e.g. left behind by a bad merge) would otherwise be silently
    invisible to both write and --check, since only the first pair is ever
    inspected. Every failure mode here must fail loudly, never silently no-op.
    """
    start_idx = content.find(start_marker)
    if start_idx == -1:
        raise ValueError(f"marker '{start_marker}' not found")
    end_idx = content.find(end_marker, start_idx)
    if end_idx == -1:
        raise ValueError(f"marker '{end_marker}' not found")
    end_idx += len(end_marker)
    if start_marker in content[end_idx:]:
        raise ValueError(
            f"multiple '{start_marker}' blocks found -- the splice mechanism "
            "only supports one marker pair per file"
        )
    return content[:start_idx] + replacement + content[end_idx:]


def _write_blocks(
    targets: list, start: str, end: str, repo_root: Path, ops: list[dict], extra
) -> None:
    for rel_path, gen_fn in targets:
        path = repo_root / rel_path
        path.write_text(
            _replace_between_markers(path.read_text(), start, end, gen_fn(ops, extra))
        )


def _stale_blocks(
    targets: list,
    start: str,
    end: str,
    repo_root: Path,
    ops: list[dict],
    extra,
    message: str,
) -> list[str]:
    """Regenerate each target's block in-memory and name every file whose
    checked-in block differs. Raises if a target is missing its marker pair
    (never silently reports clean)."""
    mismatches: list[str] = []
    for rel_path, gen_fn in targets:
        content = (repo_root / rel_path).read_text()
        expected = _replace_between_markers(content, start, end, gen_fn(ops, extra))
        if expected != content:
            mismatches.append(f"{rel_path}: {message}")
    return mismatches
