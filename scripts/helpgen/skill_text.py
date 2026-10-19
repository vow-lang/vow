"""Build the skill entrypoint, bundle and support files from docs/spec/."""

from helpgen.config import (
    CLI,
    CONTRACTS,
    ERRORS,
    EXAMPLES,
    GRAMMAR,
    INDEX,
    METHODOLOGY,
    SCHEMAS_DIR,
    STDLIB,
)

SKILL_FRONTMATTER = """\
---
name: vow
description: >-
  Write, compile, debug, and verify Vow programs (.vow files) with contracts,
  CEGIS, verifier counterexamples, diagnostics, and vow build / vow verify.
when_to_use: >-
  Use when the user edits or creates .vow files, says "write a Vow program",
  "fix this counterexample", "add contracts", "why did verification fail",
  "ESBMC", "vow build", or "vow verify".
argument-hint: "[file.vow]"
---"""

SKILL_DO_NOTS = [
    "- Do not weaken, cap, or distort a contract to make verification pass; fix the code or report that the proof is out of reach.",
    "- Do not add verifier-motivated bounds (`n <= 10`, length or capacity caps) to `requires`; keep only real domain constraints and use checked operators (`+!`) for overflow.",
    "- Do not use `--no-verify` to get past a real verification failure; it only skips the proof (status `Unverified`).",
    "- Do not report a program as verified unless `status` is `Verified`; `Unverified` means the proof never ran and `Skipped` means a function could not be modelled.",
    "- Do not invent CLI flags, JSON fields, syntax, or builtins; check `--help` and the reference files first.",
    "- Do not scrape terminal prose; parse the JSON result that `build` and `verify` print to stdout (`status`, `diagnostics`, `counterexamples`), not the human-readable text on stderr.",
]


def _index_for_skill() -> str:
    index_text = INDEX.read_text()
    omit_start = "<!-- OMIT-FROM-SKILL-START -->"
    omit_end = "<!-- OMIT-FROM-SKILL-END -->"
    start_idx = index_text.find(omit_start)
    if start_idx != -1:
        end_idx = index_text.find(omit_end, start_idx)
        if end_idx != -1:
            index_text = (
                index_text[:start_idx].rstrip() + index_text[end_idx + len(omit_end) :]
            )
    return index_text.rstrip()


def build_skill_entrypoint() -> str:
    """Build the concise installed SKILL.md entrypoint. Ends with a trailing
    newline so the on-disk SKILL.md is POSIX-conformant and matches the
    `.rstrip() + "\\n"` convention used by support files."""
    return "\n".join(
        [
            SKILL_FRONTMATTER,
            "",
            "# Vow",
            "",
            "Use this skill when writing, compiling, debugging, or verifying Vow programs.",
            "Keep the workflow tight: run the compiler, read the structured JSON, fix the",
            "program or contract, and repeat until the result is `Verified`.",
            "",
            "## Do nots",
            "",
            *SKILL_DO_NOTS,
            "",
            "## Installed toolchain (live)",
            "",
            "!`(command -v vow >/dev/null 2>&1 && vow --help 2>/dev/null | head -200) || (command -v build/vowc >/dev/null 2>&1 && build/vowc --help 2>/dev/null | head -200) || echo '(vow toolchain not found on PATH; run scripts/bootstrap.sh to build build/vowc)'`",
            "",
            "## Core workflow",
            "",
            "1. Write a `.vow` file with explicit contracts.",
            "2. Run `build/vowc build <file.vow>`.",
            "3. Parse stdout JSON and inspect `status`, `diagnostics`, and `counterexamples`.",
            "4. Fix compile errors, verification failures, or weak contracts, then rerun.",
            "",
            "## Minimal program",
            "",
            "```vow",
            "module Hello",
            "",
            "fn main() -> i32 [io] {",
            '    print_str("Hello, world!");',
            "    0",
            "}",
            "```",
            "",
            "## Reference files",
            "",
            "- Grammar, types, effects, builtins: [reference/grammar.md](reference/grammar.md)",
            "- CLI commands, flags, JSON output: [reference/cli.md](reference/cli.md)",
            "- Contracts and CEGIS guidance: [reference/contracts.md](reference/contracts.md)",
            "- Which contracts to write (taxonomy & strength): [reference/contracts-methodology.md](reference/contracts-methodology.md)",
            "- Diagnostics and fixes: [reference/errors.md](reference/errors.md)",
            "- Standard library (math, heap, stack, geometry, bignum, gc): [reference/stdlib.md](reference/stdlib.md)",
            "- Worked examples: [examples/examples.md](examples/examples.md)",
            "- JSON schemas: [schemas/](schemas/)",
            "",
            "Outside Claude Code (raw API or custom agent harness that cannot load these files): run `build/vowc skill print --bundle` for one self-contained document with all of the above inlined.",
            "",
        ]
    )


def build_skill_bundle() -> str:
    """Assemble the full self-contained skill document from spec files.

    Produces a single self-contained markdown file with YAML frontmatter,
    suitable for raw API/system-prompt use.
    """
    parts: list[str] = [SKILL_FRONTMATTER, ""]
    parts.append(_index_for_skill())
    parts.append("")

    # Append each spec file
    for spec_file in [GRAMMAR, CLI, CONTRACTS, METHODOLOGY, ERRORS, STDLIB, EXAMPLES]:
        parts.append("---")
        parts.append("")
        parts.append(spec_file.read_text().rstrip())
        parts.append("")

    # Inline JSON schemas
    schema_files = sorted(SCHEMAS_DIR.glob("*.json"))
    if schema_files:
        parts.append("---")
        parts.append("")
        parts.append("# JSON Schemas")
        parts.append("")
        for sf in schema_files:
            stem = sf.stem.replace(".schema", "")
            parts.append(f"## {stem}")
            parts.append("")
            parts.append("```json")
            parts.append(sf.read_text().rstrip())
            parts.append("```")
            parts.append("")

    return "\n".join(parts)


def build_skill_support_files() -> dict[str, str]:
    support: dict[str, str] = {
        "reference/grammar.md": GRAMMAR.read_text().rstrip() + "\n",
        "reference/cli.md": CLI.read_text().rstrip() + "\n",
        "reference/contracts.md": CONTRACTS.read_text().rstrip() + "\n",
        "reference/contracts-methodology.md": METHODOLOGY.read_text().rstrip() + "\n",
        "reference/errors.md": ERRORS.read_text().rstrip() + "\n",
        "reference/stdlib.md": STDLIB.read_text().rstrip() + "\n",
        "examples/examples.md": EXAMPLES.read_text().rstrip() + "\n",
    }
    for sf in sorted(SCHEMAS_DIR.glob("*.json")):
        support[f"schemas/{sf.name}"] = sf.read_text().rstrip() + "\n"
    return support
