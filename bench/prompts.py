"""System and user prompt construction."""

from __future__ import annotations

from pathlib import Path

SKILL_FILES = [
    "index.md",
    "grammar.md",
    "contracts.md",
    "cli.md",
    "errors.md",
    "examples.md",
]


def build_system_prompt(root: Path) -> str:
    skill_dir = root / "docs" / "skill"
    parts = []
    for name in SKILL_FILES:
        path = skill_dir / name
        parts.append(f"# {name}\n\n{path.read_text()}")
    return "\n\n---\n\n".join(parts)


def build_initial_user_prompt(spec_md: str, skeleton_vow: str) -> str:
    return f"""Below is a specification for a Vow function and a skeleton .vow file with the contract already written but the body incomplete.

Fill in the function bodies so that all contracts verify. Return ONLY the complete .vow file, no explanation.

## Specification

{spec_md}

## Skeleton

```vow
{skeleton_vow}
```

Return the complete .vow file with function bodies filled in. Do not change the module name, function signatures, or contracts."""


def build_cegis_user_prompt(verify_output: str) -> str:
    return f"""Verification failed. Here is the full JSON output from `vow verify`:

```json
{verify_output}
```

Fix the implementation so all contracts verify. Return ONLY the complete updated .vow file, no explanation."""
