#!/usr/bin/env python3
"""Generate --help JSON and human-readable output from skill docs.

Reads docs/spec/grammar.md and docs/spec/cli.md (the canonical specs),
builds the help JSON structure, and writes it into:
  - vow/src/skill.rs (skill_json raw string literal, skill_human string literal)
  - compiler/help_text_gen.vow, compiler/skill_text_gen.vow and the shard modules
    compiler/skill_bundle_NN_gen.vow, compiler/skill_docs_NN_gen.vow (chunked string
    literals; whole generated files)

The generator lives in the scripts/helpgen/ package; this file is the entry
point and re-exports the names other scripts and tests import.

Usage:
    python3 scripts/generate_help.py          # from repo root
    python3 scripts/generate_help.py --check  # validate without writing
"""

from __future__ import annotations

import json
import sys

import spec_assemble
from helpgen.config import CLI, CONTRACTS, GRAMMAR, REPO, SKILLS_DIR
from helpgen.emit_vow import (
    VOW_LITERAL_CHUNK_BYTES,
    _vow_payload_fn,
    vow_help_files,
    vow_skill_files,
)
from helpgen.emit_rust import rust_help_files, rust_skill_files
from helpgen.human_help import build_help_human
from helpgen.json_help import build_help_json
from helpgen.outputs import check_files, collect_files, write_files
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
    "generated_output_paths",
    "normalize_option",
    "rust_help_files",
    "rust_skill_files",
    "vow_help_files",
    "vow_skill_files",
]


def build_outputs():
    """Build the help JSON, the skill texts and every generated file's text."""
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

    files = collect_files(
        json_str,
        human_str,
        skill_entrypoint_md,
        skill_bundle_md,
        skill_support_files,
    )
    return data, skill_entrypoint_md, skill_bundle_md, skill_support_files, files


def generated_output_paths() -> set[str]:
    """Repo-relative paths of every file this generator writes, shards included."""
    files = build_outputs()[-1]
    return {path.relative_to(REPO).as_posix() for path in files}


def main() -> None:
    check_only = "--check" in sys.argv

    # The spec documents the generator reads are assembled from docs/spec/src/.
    if check_only:
        stale = spec_assemble.check()
        if stale:
            print("\n".join(stale), file=sys.stderr)
            sys.exit(1)
    else:
        for doc in spec_assemble.write_all():
            print(f"Updated docs/spec/{doc}.md")

    data, skill_entrypoint_md, skill_bundle_md, skill_support_files, files = (
        build_outputs()
    )

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
        drift = check_files(files) + check_skills_dir(
            skill_entrypoint_md, skill_support_files
        )
        if drift:
            print("\ngenerated file drift:")
            for line in drift:
                print(f"  {line}")
            print("\nRun scripts/generate_help.py to regenerate.")
            sys.exit(1)
        print(f"  generated files in sync ({len(files)} files)")
        print(f"  skills/vow/ in sync ({len(skill_support_files) + 1} files)")
        return

    for path in write_files(files):
        print(f"Updated {path.relative_to(REPO)}")

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
