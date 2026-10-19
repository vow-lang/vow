#!/usr/bin/env python3
"""Generate --help JSON and human-readable output from skill docs.

Reads docs/spec/grammar.md and docs/spec/cli.md (the canonical specs),
builds the help JSON structure, and writes it into:
  - vow/src/skill.rs (skill_json raw string literal, skill_human string literal)
  - compiler/main.vow (skill_json and skill_human as chunked string literals)

The generator lives in the scripts/helpgen/ package; this file is the entry
point and re-exports the names other scripts and tests import.

Usage:
    python3 scripts/generate_help.py          # from repo root
    python3 scripts/generate_help.py --check  # validate without writing
"""

from __future__ import annotations

import json
import sys

from helpgen.config import (
    CLI,
    CONTRACTS,
    GRAMMAR,
    MAIN_VOW,
    REPO,
    SKILL_RS,
    SKILLS_DIR,
)
from helpgen.emit_rust import inject_rust, inject_skill_rust
from helpgen.emit_vow import (
    VOW_LITERAL_CHUNK_BYTES,
    _vow_payload_fn,
    inject_skill_vow,
    inject_vow,
)
from helpgen.human_help import build_help_human
from helpgen.json_help import build_help_json
from helpgen.skill_text import (
    SKILL_FRONTMATTER,
    build_skill_bundle,
    build_skill_entrypoint,
    build_skill_support_files,
)
from helpgen.skills_dir import check_skills_dir, write_skills_dir
from helpgen.tables import extract_table, extract_table_col, normalize_option

__all__ = [
    "SKILL_FRONTMATTER",
    "VOW_LITERAL_CHUNK_BYTES",
    "_vow_payload_fn",
    "build_help_human",
    "build_help_json",
    "build_skill_bundle",
    "build_skill_entrypoint",
    "build_skill_support_files",
    "extract_table",
    "extract_table_col",
    "inject_skill_vow",
    "inject_vow",
    "normalize_option",
]


def main() -> None:
    check_only = "--check" in sys.argv

    grammar = GRAMMAR.read_text()
    cli = CLI.read_text()
    contracts = CONTRACTS.read_text()

    data = build_help_json(grammar, cli, contracts)
    json_str = json.dumps(data, indent=2)
    human_str = build_help_human(data)

    # Validate the generated JSON is parseable
    json.loads(json_str)

    skill_entrypoint_md = build_skill_entrypoint()
    skill_bundle_md = build_skill_bundle()
    skill_support_files = build_skill_support_files()

    if check_only:
        print("Generated JSON is valid.")
        print(f"  types: {len(data['language']['types'])}")
        print(f"  effects: {len(data['language']['effects'])}")
        print(f"  builtins: {len(data['language']['builtins'])}")
        print(f"  build_options: {len(data['build_options'])}")
        print(f"  test_options: {len(data['test_options'])}")
        print(f"  verify_options: {len(data['verify_options'])}")
        print(f"  skill_entrypoint: {len(skill_entrypoint_md)} bytes")
        print(f"  skill_bundle: {len(skill_bundle_md)} bytes")
        print(f"  skill_support_files: {len(skill_support_files)}")
        drift = check_skills_dir(skill_entrypoint_md, skill_support_files)
        if drift:
            print("\nskills/vow/ drift:")
            for line in drift:
                print(f"  {line}")
            print("\nRun scripts/generate_help.py to regenerate.")
            sys.exit(1)
        print(f"  skills/vow/ in sync ({len(skill_support_files) + 1} files)")
        return

    # Inject into Rust
    new_rs = inject_rust(SKILL_RS, json_str, human_str)
    new_rs = inject_skill_rust(
        new_rs, skill_entrypoint_md, skill_bundle_md, skill_support_files
    )
    SKILL_RS.write_text(new_rs)
    print(f"Updated {SKILL_RS}")

    # Inject into self-hosted
    new_vow = inject_vow(MAIN_VOW, json_str, human_str)
    new_vow = inject_skill_vow(
        new_vow, skill_entrypoint_md, skill_bundle_md, skill_support_files
    )
    MAIN_VOW.write_text(new_vow)
    print(f"Updated {MAIN_VOW}")

    # Write the checked-in skill mirror that `npx skills add vow-lang/vow` reads.
    write_skills_dir(skill_entrypoint_md, skill_support_files)
    print(
        f"Updated {SKILLS_DIR.relative_to(REPO)}/ ({len(skill_support_files) + 1} files)"
    )

    print(
        "\nDone. Run 'cargo build --release -p vow' and 'scripts/bootstrap.sh --no-verify --skip-cargo' to rebuild."
    )


if __name__ == "__main__":
    main()
