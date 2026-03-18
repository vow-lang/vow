#!/usr/bin/env python3
"""Cross-reference grammar.md features against --help JSON output.

Usage:
    python3 scripts/check_help_coverage.py <grammar.md> <help-json-string>

Exits 0 if all features present, 1 with missing items listed.
"""

import json
import re
import sys


def extract_table_column(text: str, heading: str, col: int = 0) -> list[str]:
    """Extract backtick-quoted values from column `col` of a markdown table
    that follows a heading matching `heading`."""
    in_section = False
    in_table = False
    items = []
    for line in text.splitlines():
        if re.match(r"^#{1,4}\s+" + re.escape(heading), line):
            in_section = True
            in_table = False
            continue
        if in_section and line.startswith("|") and "---" in line:
            in_table = True
            continue
        if in_section and in_table:
            if not line.startswith("|"):
                in_section = False
                in_table = False
                continue
            cells = [c.strip() for c in line.split("|")]
            cells = [c for c in cells if c]
            if col < len(cells):
                m = re.search(r"`([^`]+)`", cells[col])
                if m:
                    items.append(m.group(1))
    return items


def flatten_json(obj) -> str:
    """Flatten a JSON object into a single string for substring searching."""
    if isinstance(obj, dict):
        parts: list[str] = []
        for k, v in obj.items():
            parts.append(k)
            parts.append(flatten_json(v))
        return " ".join(parts)
    elif isinstance(obj, list):
        return " ".join(str(flatten_json(v)) for v in obj)
    else:
        return str(obj)


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
    flat = flatten_json(lang)

    missing = []

    # 1. Primitive types
    prims = extract_table_column(grammar, "Primitive Types", 0)
    assert len(prims) >= 6, f"Expected >=6 primitive types, got {len(prims)}: {prims}"
    types_list = flat
    for t in prims:
        if t not in types_list:
            missing.append(f"type:{t}")

    # 2. Parameterized types
    params = extract_table_column(grammar, "Built-in Parameterized Types", 0)
    assert len(params) >= 4, f"Expected >=4 param types, got {len(params)}: {params}"
    for t in params:
        base = t.split("<")[0]
        if base not in types_list:
            missing.append(f"type:{t}")

    # 3. Effects
    effects = extract_table_column(grammar, "Effect Types", 0)
    assert len(effects) >= 5, f"Expected >=5 effects, got {len(effects)}: {effects}"
    for e in effects:
        if e not in flat:
            missing.append(f"effect:{e}")

    # 4. Builtin functions
    builtins = extract_table_column(grammar, "Builtin Function Signatures", 0)
    assert len(builtins) >= 5, f"Expected >=5 builtins, got {len(builtins)}: {builtins}"
    builtins_flat = flatten_json(lang.get("builtins", {}))
    for b in builtins:
        if b not in builtins_flat:
            missing.append(f"builtin:{b}")

    # 5. Structural keys
    required_keys = [
        "structs", "enums", "match_expression", "methods",
        "where_clauses", "modules", "control_flow",
        "type_aliases", "extern_blocks", "indexing",
    ]
    for key in required_keys:
        if key not in lang:
            missing.append(f"key:{key}")

    if missing:
        print(f"FAIL: {len(missing)} missing item(s) in --help language section:")
        for m in missing:
            print(f"  - {m}")
        sys.exit(1)
    else:
        print("OK: all grammar.md features present in --help JSON")
        sys.exit(0)


if __name__ == "__main__":
    main()
