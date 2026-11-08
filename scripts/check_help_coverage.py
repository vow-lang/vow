#!/usr/bin/env python3
"""Cross-reference grammar.md features against --help JSON output.

Usage:
    python3 scripts/check_help_coverage.py <grammar.md> <help-json-string>

Exits 0 if all features present, 1 with missing items listed.
"""

import json
import re
import sys
from pathlib import Path


def extract_table_rows(text: str, heading: str) -> list[list[str]]:
    """Extract cells from markdown tables that follow `heading`.
    Collects rows from all sub-tables in the section."""
    in_section = False
    in_table = False
    section_level = 0
    rows = []
    for line in text.splitlines():
        if re.match(r"^#{1,4}\s+" + re.escape(heading), line):
            in_section = True
            in_table = False
            section_level = len(line) - len(line.lstrip("#"))
            continue
        if in_section and line.startswith("#"):
            level = len(line) - len(line.lstrip("#"))
            if level <= section_level:
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
            cells = [c.strip() for c in line.split("|")]
            cells = [c for c in cells if c]
            rows.append(cells)
    return rows


def extract_backtick_value(cell: str) -> str | None:
    m = re.search(r"`([^`]+)`", cell)
    if m:
        return m.group(1)
    return None


def extract_table_column(text: str, heading: str, col: int = 0) -> list[str]:
    """Extract backtick-quoted values from column `col` of markdown tables."""
    items = []
    for row in extract_table_rows(text, heading):
        if col < len(row):
            value = extract_backtick_value(row[col])
            if value is not None:
                items.append(value)
    return items


def cli_command_names(cli: str) -> list[str]:
    """Names of the vow command headings (level 3, `vow <name>`) in cli.md."""
    names = []
    for line in cli.splitlines():
        m = re.match(r"^### `vow ([a-z][a-z0-9_-]*)`", line)
        if m:
            names.append(m.group(1))
    return names


def check_commands(help_data: dict, cli: str) -> list[str]:
    """Commands in cli.md, `commands` and `command_details` must agree."""
    commands = help_data.get("commands", {}).keys()
    details = help_data.get("command_details", {}).keys()
    documented = set(cli_command_names(cli))
    missing = [f"command:{n}" for n in documented - commands]
    missing += [f"cli.md heading for command:{n}" for n in commands - documented]
    missing += [f"command_details:{n}" for n in commands - details]
    missing += [f"command:{n}" for n in details - commands]
    return sorted(set(missing))


def main():
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <grammar.md> <help-json>", file=sys.stderr)
        sys.exit(2)

    grammar_path = sys.argv[1]
    help_json_str = sys.argv[2]

    with open(grammar_path) as f:
        grammar = f.read()

    try:
        help_data = json.loads(help_json_str)
    except json.JSONDecodeError as e:
        print(f"ERROR: --help output is not valid JSON: {e}", file=sys.stderr)
        sys.exit(1)

    lang = help_data.get("language", {})

    missing = []

    cli_path = Path(grammar_path).with_name("cli.md")
    if cli_path.exists():
        missing.extend(check_commands(help_data, cli_path.read_text()))
    else:
        missing.append(f"file:{cli_path} (cli.md must sit next to grammar.md)")

    # 1. Primitive types
    prims = extract_table_column(grammar, "Primitive Types", 0)
    assert len(prims) >= 6, f"Expected >=6 primitive types, got {len(prims)}: {prims}"
    types_list = lang.get("types", [])
    for t in prims:
        if t not in types_list:
            missing.append(f"type:{t}")

    # 2. Parameterized types
    params = extract_table_column(grammar, "Built-in Parameterized Types", 0)
    assert len(params) >= 4, f"Expected >=4 param types, got {len(params)}: {params}"
    for t in params:
        if t not in types_list:
            missing.append(f"type:{t}")

    # 3. Effects
    effects = extract_table_column(grammar, "Effect Types", 0)
    assert len(effects) >= 5, f"Expected >=5 effects, got {len(effects)}: {effects}"
    effects_list = lang.get("effects", [])
    for e in effects:
        if e not in effects_list:
            missing.append(f"effect:{e}")

    # 4. Builtin functions
    builtin_rows = extract_table_rows(grammar, "Builtin Function Signatures")
    builtins = []
    for row in builtin_rows:
        if len(row) >= 3:
            value = extract_backtick_value(row[0])
            if value is not None:
                builtins.append(value)
    assert len(builtins) >= 5, f"Expected >=5 builtins, got {len(builtins)}: {builtins}"
    builtins_dict = lang.get("builtins", {})
    for b in builtins:
        if b not in builtins_dict:
            missing.append(f"builtin:{b}")

    # 5. Structural keys
    required_keys = [
        "structs",
        "enums",
        "match_expression",
        "methods",
        "where_clauses",
        "modules",
        "control_flow",
        "type_aliases",
        "extern_blocks",
        "indexing",
    ]
    for key in required_keys:
        if key not in lang:
            missing.append(f"key:{key}")

    if missing:
        print(f"FAIL: {len(missing)} missing item(s) in --help output:")
        for m in missing:
            print(f"  - {m}")
        sys.exit(1)
    else:
        print("OK: all grammar.md features present in --help JSON")
        sys.exit(0)


if __name__ == "__main__":
    main()
