#!/usr/bin/env python3
"""Generate Operation Catalogue projections for both compilers.

Reads docs/spec/operations.json (the Operation Catalogue: the checked,
hand-edited source of runtime-symbol/ABI/return-shape/doc facts for a set of
Builtin Operations) and splices generated lookup functions into:
  - vow-ir/src/lower/mod.rs           (catalogue_builtin_to_runtime)
  - vow-codegen/src/cranelift_backend.rs (catalogue_extern_sig)
  - vow-clif-shim/src/lib.rs             (catalogue_extern_sig)
  - compiler/lower.vow                (catalogue_builtin_to_extern, catalogue_builtin_ret_ty)
between `// GENERATE:OPERATIONS:START` / `// GENERATE:OPERATIONS:END` markers.

Usage:
    python3 scripts/generate_operations.py          # from repo root
    python3 scripts/generate_operations.py --check  # validate without writing
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# Closed vocabulary: catalogue tokens -> target-specific spellings. Adding a
# new token here is the one place a follow-up catalogue slice needs to touch
# to introduce a new ABI shape. `clif_ret` is the Cranelift `types::*` spelling
# pushed onto the extern signature's return slot (None for a void return) --
# gen_cranelift_block() reads this so a non-unit token can never be silently
# dropped from the Cranelift ABI.
RETURN_TOKENS = {
    "unit": {"rust_ty": "Ty::Unit", "ity_const": "ITY_UNIT()", "clif_ret": None},
}
PARAM_TOKENS = {
    "ptr": "types::I64",
    "i64": "types::I64",
    "u64": "types::I64",
}

REQUIRED_FIELDS = [
    "name",
    "runtime_symbol",
    "params",
    "return",
    "doc_signature",
    "effects",
]


def load_catalogue(repo_root: Path) -> list[dict]:
    """Read and hand-validate docs/spec/operations.json under repo_root.

    Raises ValueError naming the offending operation/field on any violation.
    """
    catalogue_path = repo_root / "docs" / "spec" / "operations.json"
    data = json.loads(catalogue_path.read_text())
    if not isinstance(data.get("operations"), list):
        raise ValueError(f"{catalogue_path}: top-level 'operations' key must be a list")
    ops = data["operations"]

    seen_names: set[str] = set()
    seen_symbols: set[str] = set()
    for op in ops:
        for field in REQUIRED_FIELDS:
            if field not in op:
                name = op.get("name", "<unnamed>")
                raise ValueError(
                    f"operation '{name}' is missing required field '{field}'"
                )

        name = op["name"]
        if name in seen_names:
            raise ValueError(f"duplicate operation name '{name}'")
        seen_names.add(name)

        symbol = op["runtime_symbol"]
        if symbol in seen_symbols:
            raise ValueError(
                f"duplicate runtime_symbol '{symbol}' (operation '{name}')"
            )
        seen_symbols.add(symbol)

        ret = op["return"]
        if ret not in RETURN_TOKENS:
            raise ValueError(
                f"operation '{name}' has unknown return token '{ret}' "
                f"(known: {sorted(RETURN_TOKENS)})"
            )

        params = op["params"]
        if not isinstance(params, list):
            raise ValueError(
                f"operation '{name}' has 'params' of type {type(params).__name__}, "
                "expected a list"
            )
        for param in params:
            if param not in PARAM_TOKENS:
                raise ValueError(
                    f"operation '{name}' has unknown params token '{param}' "
                    f"(known: {sorted(PARAM_TOKENS)})"
                )

    return ops


MARKER_START = "// GENERATE:OPERATIONS:START"
MARKER_END = "// GENERATE:OPERATIONS:END"


def _wrap_marker_block(body: str) -> str:
    """Wrap a generated function body (ending in "}\\n") in the marker pair
    every splice target shares."""
    return f"{MARKER_START}\n{body}{MARKER_END}"


def gen_rust_ir_block(ops: list[dict]) -> str:
    arms = "\n".join(
        f'        "{op["name"]}" => Some(("{op["runtime_symbol"]}", '
        f"{RETURN_TOKENS[op['return']]['rust_ty']})),"
        for op in ops
    )
    return _wrap_marker_block(
        "fn catalogue_builtin_to_runtime(name: &str) -> Option<(&'static str, Ty)> {\n"
        "    match name {\n"
        f"{arms}\n"
        "        _ => None,\n"
        "    }\n"
        "}\n"
    )


def gen_cranelift_block(ops: list[dict]) -> str:
    arm_blocks = []
    for op in ops:
        lines = [
            f"            sig.params.push(AbiParam::new({PARAM_TOKENS[p]}));"
            for p in op["params"]
        ]
        clif_ret = RETURN_TOKENS[op["return"]]["clif_ret"]
        if clif_ret is not None:
            lines.append(f"            sig.returns.push(AbiParam::new({clif_ret}));")
        lines.append("            true")
        body = "\n".join(lines)
        arm_blocks.append(f'        "{op["runtime_symbol"]}" => {{\n{body}\n        }}')
    arms = "\n".join(arm_blocks)
    return _wrap_marker_block(
        "fn catalogue_extern_sig(sym: &str, sig: &mut Signature) -> bool {\n"
        "    match sym {\n"
        f"{arms}\n"
        "        _ => false,\n"
        "    }\n"
        "}\n"
    )


def gen_vow_lower_block(ops: list[dict]) -> str:
    extern_arms = "\n".join(
        f'    if name == String::from("{op["name"]}") '
        f'{{ return String::from("{op["runtime_symbol"]}"); }}'
        for op in ops
    )
    ret_ty_arms = "\n".join(
        f'    if name == String::from("{op["name"]}") '
        f"{{ return {RETURN_TOKENS[op['return']]['ity_const']}; }}"
        for op in ops
    )
    return _wrap_marker_block(
        "fn catalogue_builtin_to_extern(name: String) -> String {\n"
        f"{extern_arms}\n"
        '    return String::from("");\n'
        "}\n"
        "\n"
        "fn catalogue_builtin_ret_ty(name: String) -> i64 {\n"
        f"{ret_ty_arms}\n"
        "    return -1;\n"
        "}\n"
    )


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


def _split_table_row(line: str) -> list[str]:
    """Split a `| a | b |`-style row into cells.

    Only the empty strings produced by the row's own leading/trailing pipe
    are dropped -- a genuinely empty interior cell (`| a |  | c |`) is kept,
    so columns never silently shift.
    """
    placeholder = "\x00PIPE\x00"
    line = line.replace("\\|", placeholder)
    cells = [c.strip().replace(placeholder, "|") for c in line.split("|")]
    if cells and cells[0] == "":
        cells = cells[1:]
    if cells and cells[-1] == "":
        cells = cells[:-1]
    return cells


def extract_builtin_signatures_table(grammar_text: str) -> dict[str, tuple[str, str]]:
    """Map builtin name -> (signature, effects) from the whole level-3
    `### Builtin Function Signatures` section of grammar.md, sweeping every
    `####` subsection (Print / IO, Debug, Filesystem, ...) so any catalogued
    op can be looked up by name regardless of which subsection it lives in.

    Raises ValueError on a duplicate row for the same builtin name -- e.g. a
    stale row left behind in another subsection by a bad merge -- rather than
    silently keeping whichever row happens to be read last.
    """
    heading = "Builtin Function Signatures"
    heading_level = 3
    prefix = "#" * heading_level + " "
    in_section = False
    in_table = False
    table: dict[str, tuple[str, str]] = {}
    for line in grammar_text.splitlines():
        if line.startswith(prefix) and heading in line:
            in_section = True
            in_table = False
            continue
        if in_section and line.startswith("#"):
            level = len(line) - len(line.lstrip("#"))
            if level <= heading_level:
                break
            in_table = False
            continue
        if in_section and line.startswith("|") and "---" in line:
            in_table = True
            continue
        if in_section and in_table:
            if not line.startswith("|"):
                in_table = False
                continue
            cells = _split_table_row(line)
            cells = [re.sub(r"`([^`]*)`", r"\1", c) for c in cells]
            if len(cells) >= 3:
                if cells[0] in table:
                    raise ValueError(
                        f"duplicate Builtin Function Signatures row for '{cells[0]}'"
                    )
                table[cells[0]] = (cells[1], cells[2])
    return table


def check_doc_facts(ops: list[dict], repo_root: Path) -> list[str]:
    """Cross-check each op's doc facts against grammar.md, main.vow and
    skill.rs. Collects every mismatch instead of failing on the first."""
    mismatches: list[str] = []

    grammar_text = (repo_root / "docs" / "spec" / "grammar.md").read_text()
    table = extract_builtin_signatures_table(grammar_text)
    main_vow_text = (repo_root / "compiler" / "main.vow").read_text()
    skill_rs_text = (repo_root / "vow" / "src" / "skill.rs").read_text()

    for op in ops:
        name = op["name"]
        sig = op["doc_signature"]
        eff = op["effects"]

        row = table.get(name)
        if row is None:
            mismatches.append(f"grammar.md: no row found for '{name}'")
        elif row != (sig, eff):
            mismatches.append(
                f"grammar.md: '{name}' row is {row}, catalogue says ({sig!r}, {eff!r})"
            )

        skill_rs_line = f'"{name}": "{sig} {eff}"'
        if skill_rs_line not in skill_rs_text:
            mismatches.append(f"vow/src/skill.rs: missing doc line for '{name}'")

        escaped_sig = sig.replace("\\", "\\\\").replace('"', '\\"')
        main_vow_line = f'\\"{name}\\": \\"{escaped_sig} {eff}\\"'
        if main_vow_line not in main_vow_text:
            mismatches.append(f"compiler/main.vow: missing doc line for '{name}'")

    return mismatches


TARGET_FILES = [
    (Path("vow-ir/src/lower/mod.rs"), gen_rust_ir_block),
    (Path("vow-codegen/src/cranelift_backend.rs"), gen_cranelift_block),
    (Path("vow-clif-shim/src/lib.rs"), gen_cranelift_block),
    (Path("compiler/lower.vow"), gen_vow_lower_block),
]


def write_projections(ops: list[dict], repo_root: Path) -> None:
    for rel_path, gen_fn in TARGET_FILES:
        path = repo_root / rel_path
        content = path.read_text()
        new_content = _replace_between_markers(
            content, MARKER_START, MARKER_END, gen_fn(ops)
        )
        path.write_text(new_content)


def check_projections(ops: list[dict], repo_root: Path) -> list[str]:
    """Regenerate each target's block in-memory and diff it against what is
    currently spliced into the checked-in file. Raises if a target is
    missing its marker pair (never silently reports clean)."""
    mismatches: list[str] = []
    for rel_path, gen_fn in TARGET_FILES:
        path = repo_root / rel_path
        content = path.read_text()
        expected = _replace_between_markers(
            content, MARKER_START, MARKER_END, gen_fn(ops)
        )
        if expected != content:
            mismatches.append(f"{rel_path}: spliced block is stale")
    return mismatches


def main() -> None:
    check_only = "--check" in sys.argv
    ops = load_catalogue(REPO)

    if check_only:
        mismatches = check_projections(ops, REPO) + check_doc_facts(ops, REPO)
        if mismatches:
            for m in mismatches:
                print(m)
            print("\nRun scripts/generate_operations.py to regenerate.")
            sys.exit(1)
        print("OK: operations catalogue up to date")
        return

    write_projections(ops, REPO)
    for rel_path, _ in TARGET_FILES:
        print(f"Updated {rel_path}")


if __name__ == "__main__":
    main()
