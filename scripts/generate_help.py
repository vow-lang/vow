#!/usr/bin/env python3
"""Generate --help JSON and human-readable output from skill docs.

Reads docs/skill/grammar.md and docs/skill/cli.md (the canonical specs),
builds the help JSON structure, and writes it into:
  - vow/src/main.rs  (skill_json raw string literal, skill_human string literal)
  - compiler/main.vow (skill_json push_str calls, skill_human push_str calls)

Usage:
    python3 scripts/generate_help.py          # from repo root
    python3 scripts/generate_help.py --check  # validate without writing
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
GRAMMAR = REPO / "docs" / "skill" / "grammar.md"
CLI = REPO / "docs" / "skill" / "cli.md"
CONTRACTS = REPO / "docs" / "skill" / "contracts.md"
MAIN_RS = REPO / "vow" / "src" / "main.rs"
MAIN_VOW = REPO / "compiler" / "main.vow"


# ---------------------------------------------------------------------------
# Markdown table extraction
# ---------------------------------------------------------------------------

def _split_table_row(line: str) -> list[str]:
    """Split a markdown table row on unescaped pipes, handling \\| escapes."""
    # Replace escaped pipes with a placeholder, split, then restore
    placeholder = "\x00PIPE\x00"
    line = line.replace("\\|", placeholder)
    cells = [c.strip().replace(placeholder, "|") for c in line.split("|")]
    return [c for c in cells if c != ""]


def extract_table(text: str, heading: str, *, heading_level: int = 3) -> list[list[str]]:
    """Extract rows from a markdown table under a heading.
    Returns list of rows, each row is a list of cell strings (backticks stripped).
    Collects rows from all tables in the section, including sub-sections."""
    prefix = "#" * heading_level + " "
    in_section = False
    in_table = False
    rows: list[list[str]] = []
    for line in text.splitlines():
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
            rows.append(cells)
    return rows


def extract_table_col(text: str, heading: str, col: int = 0, **kw) -> list[str]:
    """Extract a single column from a markdown table."""
    return [row[col] for row in extract_table(text, heading, **kw) if col < len(row)]


# ---------------------------------------------------------------------------
# Build JSON structure from grammar.md + cli.md
# ---------------------------------------------------------------------------

def build_help_json(grammar: str, cli: str, contracts: str) -> dict:
    # --- Types ---
    prim_types = extract_table_col(grammar, "Primitive Types", 0)
    param_types = extract_table_col(grammar, "Built-in Parameterized Types", 0)
    all_types = prim_types + param_types

    # --- Effects ---
    effects = extract_table_col(grammar, "Effect Types", 0)

    # --- Builtins (with full signatures) ---
    builtin_rows = extract_table(grammar, "Builtin Function Signatures")
    builtins = {}
    for row in builtin_rows:
        if len(row) >= 3:
            name, sig, eff = row[0], row[1], row[2]
            builtins[name] = f"{sig} {eff}"

    # --- Operators ---
    arith = extract_table_col(grammar, "Wrapping Arithmetic (default)", 0)
    checked = extract_table_col(grammar, "Checked Arithmetic", 0)
    comparison = extract_table_col(grammar, "Comparison Operators", 0)
    logical = extract_table_col(grammar, "Logical Operators", 0)
    unary = extract_table_col(grammar, "Unary Operators", 0)

    # --- Methods ---
    def method_names(heading: str) -> list[str]:
        return extract_table_col(grammar, heading, 0)

    vec_methods = method_names("Vec<T> Methods")
    string_methods = method_names("String Methods")
    hashmap_methods = method_names("HashMap<K, V> Methods")
    option_methods = method_names("Option<T> Methods")
    option_methods.append("? operator")

    # --- Pattern kinds ---
    pattern_rows = extract_table(grammar, "Pattern Kinds")
    patterns = []
    for row in pattern_rows:
        if len(row) >= 2:
            kind = row[0]
            example = row[1]
            patterns.append(f"{kind} ({example})")

    # --- CLI: build options ---
    build_opt_rows = extract_table(cli, "vow build", heading_level=3)
    build_options: dict[str, str] = {}
    for row in build_opt_rows:
        if len(row) >= 3:
            flag, default, desc = row[0], row[1], row[2]
            # Merge --mode debug / --mode release into a single entry
            if flag == "--mode debug":
                build_options["--mode <debug|release>"] = (
                    f"Build mode; debug inserts runtime vow checks (default: {default})"
                )
                continue
            if flag == "--mode release":
                continue
            # Add default to description if meaningful
            desc_with_default = desc
            if default not in ("", "(off)") and not desc.endswith(")"):
                desc_with_default = f"{desc} (default: {default})"
            # Normalize flag display: -o, --output → -o, --output <path>
            if flag == "-o, --output":
                flag = "-o, --output <path>"
                desc_with_default = f"{desc} (default: source without .vow extension)"
            build_options[flag] = desc_with_default

    # --- CLI: verify options ---
    verify_opt_rows = extract_table(cli, "vow verify", heading_level=3)
    verify_options = {}
    for row in verify_opt_rows:
        if len(row) >= 3:
            flag, _default, desc = row[0], row[1], row[2]
            verify_options[flag] = desc

    # --- CLI: test options ---
    test_opt_rows = extract_table(cli, "vow test", heading_level=3)
    test_options = {}
    for row in test_opt_rows:
        if len(row) >= 3:
            flag, default, desc = row[0], row[1], row[2]
            if flag == "--mode debug":
                test_options["--mode <debug|release>"] = (
                    f"Build mode; debug inserts runtime vow checks (default: {default})"
                )
                continue
            if flag == "--mode release":
                continue
            desc_with_default = desc
            if default not in ("", "(off)", "(none)") and not desc.endswith(")"):
                desc_with_default = f"{desc} (default: {default})"
            test_options[flag] = desc_with_default

    # --- CLI: contracts options ---
    contracts_opt_rows = extract_table(cli, "vow contracts", heading_level=3)
    contracts_options = {}
    for row in contracts_opt_rows:
        if len(row) >= 3:
            flag, _default, desc = row[0], row[1], row[2]
            contracts_options[flag] = desc

    # --- Verification limits from contracts.md ---
    collection_rows = extract_table(contracts, "Collection Models for Verification")
    verification_limits: dict[str, str | int] = {
        "loop_unwind": 10,
    }
    for row in collection_rows:
        if len(row) >= 2:
            type_name = row[0].strip()
            capacity = int(row[1].strip())
            verification_limits[type_name] = capacity

    return {
        "tool": "vow",
        "description": "Vow compiler: compiles Vow source to native executables with contract verification",
        "usage": "vow <command> [OPTIONS] <source.vow>",
        "commands": {
            "build": "Compile source to native executable (verifies by default; use --no-verify to skip)",
            "verify": "Verify contracts without producing an executable (use --no-cache to skip cache)",
            "test": "Run tests: discover, compile, execute test_*.vow files with JSON results",
            "decl": "Emit declaration file (.vow.d) with type signatures only",
            "contracts": "List all contracts with optional verification status",
        },
        "legacy_usage": "vow [OPTIONS] <source.vow> (equivalent to vow build)",
        "build_options": build_options,
        "verify_options": verify_options,
        "test_options": test_options,
        "contracts_options": contracts_options,
        "global_options": {
            "--help": "Print this JSON capability description",
            "--help --human": "Print human-readable capability description",
        },
        "output_json": {
            "status": "Verified | Unverified | CompileFailed | VerifyFailed",
            "executable": "path to compiled binary, or null",
            "diagnostics": "[array of {error_code, message, severity, span: {file, offset, length}}]",
            "message": "error detail (CompileFailed)",
            "function": "function name (VerifyFailed)",
            "counterexample": "ESBMC counterexample description (VerifyFailed)",
        },
        "exit_codes": {
            "0": "success (Verified or Unverified)",
            "1": "failure (CompileFailed or VerifyFailed)",
        },
        "language": {
            "module": "module <Name>",
            "use_declaration": "use foo.bar",
            "function": "fn <name>(<params>) -> <RetTy> [<effects>] { <body> }",
            "public_function": "pub fn <name>(<params>) -> <RetTy> [<effects>] { <body> }",
            "vow_function": "fn <name>(<params>) -> <RetTy> vow { requires: <expr>; ensures: <expr> } { <body> }",
            "while_with_invariant": "while <cond> vow { invariant: <expr> } { <body> }",
            "types": all_types,
            "effects": effects,
            "builtins": builtins,
            "operators": {
                "arithmetic": arith,
                "checked_arithmetic": checked,
                "comparison": comparison,
                "logical": logical,
                "unary": unary,
            },
            "vow_clauses": {
                "requires": "precondition \u2014 blame=Caller on violation",
                "ensures": "postcondition \u2014 blame=Callee on violation; use `result` for return value",
                "invariant": "loop invariant \u2014 checked at top of each iteration",
            },
            "where_clauses": "fn f(x: i64 where x >= 0) -> i64 \u2014 refinement types on parameters",
            "structs": {
                "definition": "struct Name { field: Type, ... }",
                "linear": "linear struct Name { field: Type, ... } \u2014 must be consumed exactly once",
                "literal": "Name { field: value, ... }",
                "field_access": "value.field",
            },
            "enums": {
                "definition": "enum Name { Variant1(T), Variant2, Variant3 { field: T } }",
                "construction": "Name::Variant(value)",
                "builtin_option": "Option<T> \u2014 variants: Some(T), None",
                "builtin_result": "Result<T, E> \u2014 variants: Ok(T), Err(E)",
            },
            "match_expression": {
                "syntax": "match value { Pattern => expr, ... }",
                "patterns": patterns,
            },
            "control_flow": {
                "if_else": "if cond { expr } else { expr } \u2014 expression, both branches same type",
                "while": "while cond { body }",
                "for_each": "for item in vec { body } \u2014 iterate Vec elements",
                "for_enumerate": "for i, item in vec { body } \u2014 iterate with index",
                "loop": "loop { ... break value; } \u2014 infinite loop, break to exit",
                "break": "break; or break value;",
                "return": "return; or return value;",
            },
            "modules": {
                "declaration": "module Name",
                "import": "use foo.bar \u2014 resolves to <rootdir>/foo/bar.vow",
                "visibility": "pub fn \u2014 public functions visible to importers",
            },
            "type_aliases": "type Name = Type",
            "extern_blocks": "extern \"C\" vow { requires: ... } { fn name(x: i64) -> i64 [unsafe] }",
            "methods": {
                "Vec<T>": vec_methods,
                "String": string_methods,
                "HashMap<K,V>": hashmap_methods,
                "Option<T>": option_methods,
            },
            "indexing": {
                "read": "v[i] \u2014 Vec index access",
                "write": "v[i] = val \u2014 Vec index assignment",
            },
        },
        "verification_limits": verification_limits,
    }


# ---------------------------------------------------------------------------
# Build human-readable help
# ---------------------------------------------------------------------------

def build_help_human(data: dict) -> str:
    lines: list[str] = []
    lines.append("vow \u2014 Vow compiler")
    lines.append("")
    lines.append("USAGE")
    lines.append("  vow build [OPTIONS] <source.vow>    Compile to native executable")
    lines.append("  vow verify [OPTIONS] <source.vow>    Verify contracts only (no executable)")
    lines.append("  vow test [OPTIONS] [<path>]          Run tests (test_*.vow / *_test.vow)")
    lines.append("  vow contracts [OPTIONS] <source.vow> List all contracts")
    lines.append("  vow decl [OPTIONS] <source.vow>    Emit declaration file (.vow.d)")
    lines.append("  vow [OPTIONS] <source.vow>          Legacy mode (same as vow build)")
    lines.append("")

    lines.append("BUILD OPTIONS")
    for flag, desc in data["build_options"].items():
        pad = max(24, len(flag) + 2)
        lines.append(f"  {flag:<{pad}s}{desc}")
    lines.append("")

    lines.append("VERIFY OPTIONS")
    for flag, desc in data["verify_options"].items():
        pad = max(24, len(flag) + 2)
        lines.append(f"  {flag:<{pad}s}{desc}")
    lines.append("")

    lines.append("TEST OPTIONS")
    for flag, desc in data["test_options"].items():
        pad = max(24, len(flag) + 2)
        lines.append(f"  {flag:<{pad}s}{desc}")
    lines.append("")

    lines.append("CONTRACTS OPTIONS")
    for flag, desc in data.get("contracts_options", {}).items():
        pad = max(24, len(flag) + 2)
        lines.append(f"  {flag:<{pad}s}{desc}")
    lines.append("")

    lines.append("GLOBAL OPTIONS")
    lines.append("  --help                Print JSON capability description (agent-friendly)")
    lines.append("  --help --human        Print this text")
    lines.append("")

    lines.append("OUTPUT (JSON on stdout)")
    lines.append("  status      : Verified | Unverified | CompileFailed | VerifyFailed")
    lines.append("  executable  : path to compiled binary, or null")
    lines.append("  diagnostics : array of {error_code, message, severity, span: {file, offset, length}}")
    lines.append("  message     : error detail (CompileFailed)")
    lines.append("  function    : function name (VerifyFailed)")
    lines.append("  counterexample: ESBMC counterexample (VerifyFailed)")
    lines.append("")

    lines.append("EXIT CODES")
    lines.append("  0  success (Verified or Unverified)")
    lines.append("  1  failure (CompileFailed or VerifyFailed)")
    lines.append("")

    lines.append("LANGUAGE SUMMARY")
    lines.append("  module Hello")
    lines.append("  use math.utils")
    lines.append("")
    lines.append("  struct Point { x: i64, y: i64 }")
    lines.append("")
    lines.append("  fn add(x: i64, y: i64) -> i64 {")
    lines.append("    x + y")
    lines.append("  }")
    lines.append("")
    lines.append("  fn divide(x: i64, y: i64) -> i64 vow {")
    lines.append("    requires: y != 0")
    lines.append("    ensures:  result * y == x")
    lines.append("  } {")
    lines.append("    x / y")
    lines.append("  }")
    lines.append("")
    lines.append("  fn main() -> i32 [io] {")
    lines.append("    let v: Vec<i64> = Vec::new();")
    lines.append("    v.push(divide(10, 2));")
    lines.append("    print_i64(v[0]);")
    lines.append("    0")
    lines.append("  }")
    lines.append("")

    lang = data["language"]
    types_str = "  ".join(lang["types"])
    lines.append(f"TYPES     : {types_str}")
    lines.append(f"EFFECTS   : {'  '.join(lang['effects'])}")

    builtins = lang["builtins"]
    bl = []
    for name, sig in builtins.items():
        bl.append(f"{name}: {sig}")
    bl_line = "   ".join(bl[:3])
    lines.append(f"BUILTINS  : {bl_line}")
    if len(bl) > 3:
        bl_line2 = "   ".join(bl[3:])
        lines.append(f"            {bl_line2}")

    methods = lang["methods"]
    def short_methods(names: list[str]) -> str:
        return "/".join(n.split("(")[0].lstrip(".") for n in names if not n.startswith("?"))
    vec_short = short_methods(methods["Vec<T>"])
    str_short = short_methods(methods["String"])
    hm_short = short_methods(methods["HashMap<K,V>"])
    opt_short = short_methods(methods["Option<T>"])
    lines.append(f"METHODS   : Vec: {vec_short}   String: {str_short}")
    lines.append(f"            HashMap: {hm_short}   Option: {opt_short}")

    ops = lang["operators"]
    arith = " ".join(ops["arithmetic"])
    checked = " ".join(ops["checked_arithmetic"])
    comp = " ".join(ops["comparison"])
    logical = " ".join(ops["logical"])
    unary = " ".join(ops["unary"])
    lines.append(f"OPERATORS : {arith}   {checked} (checked)   {comp}   {logical}   {unary}")
    lines.append("")

    vlimits = data.get("verification_limits", {})
    if vlimits:
        lines.append("VERIFICATION LIMITS")
        lines.append(f"  Loop unwind  : {vlimits.get('loop_unwind', 10)} iterations")
        for key, val in vlimits.items():
            if key != "loop_unwind":
                lines.append(f"  {key:<14s}: {val} max capacity")

    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Inject into Rust main.rs
# ---------------------------------------------------------------------------

def inject_rust(main_rs: Path, json_str: str, human_str: str) -> str:
    content = main_rs.read_text()

    # Replace skill_json body using marker-based find/replace
    json_fn_marker = 'fn skill_json() -> String {'
    json_start = content.index(json_fn_marker)
    json_end = content.index('.to_string()\n}', json_start) + len('.to_string()\n}')
    json_replacement = f'fn skill_json() -> String {{\n    r#"{json_str}"#\n    .to_string()\n}}'
    content = content[:json_start] + json_replacement + content[json_end:]

    # Replace skill_human body — use a raw string to avoid escape issues
    human_fn_marker = 'fn skill_human() -> String {'
    human_start = content.index(human_fn_marker)
    human_end = content.index('.to_string()\n}', human_start) + len('.to_string()\n}')
    human_replacement = f'fn skill_human() -> String {{\n    r#"{human_str}"#\n        .to_string()\n}}'
    content = content[:human_start] + human_replacement + content[human_end:]

    return content


# ---------------------------------------------------------------------------
# Inject into self-hosted main.vow
# ---------------------------------------------------------------------------

def _vow_pushstr_body(text: str) -> tuple[str, str]:
    """Convert a multiline string into Vow String::from first line + push_str rest.
    Returns (first_line_escaped, rest_pushstr_lines)."""
    vow_text = text.replace("\u2014", "--")
    lines = vow_text.split("\n")
    first_escaped = lines[0].replace("\\", "\\\\").replace('"', '\\"')
    rest = []
    for line in lines[1:]:
        escaped = line.replace("\\", "\\\\").replace('"', '\\"')
        rest.append(f'    r.push_str(String::from("{escaped}\\n"));')
    return first_escaped, "\n".join(rest)


def inject_vow(main_vow: Path, json_str: str, human_str: str) -> str:
    content = main_vow.read_text()

    # Replace skill_json function body using marker-based find/replace
    json_start_marker = "fn skill_json() -> String {\n    let r: String = String::from("
    json_start = content.index(json_start_marker)
    json_end_marker = "    r\n}\n\nfn skill_human"
    json_end = content.index(json_end_marker, json_start) + len("    r\n}")

    first_json, rest_json = _vow_pushstr_body(json_str)
    json_fn = f'fn skill_json() -> String {{\n    let r: String = String::from("{first_json}\\n");\n{rest_json}\n    r\n}}'
    content = content[:json_start] + json_fn + content[json_end:]

    # Replace skill_human function body
    human_start_marker = "fn skill_human() -> String {\n    let r: String = String::from("
    human_start = content.index(human_start_marker)
    # Find the closing "    r\n}\n" that ends this function
    # Search for the pattern "    r\n}\n\n" after skill_human start
    human_end_marker = "    r\n}\n\n"
    human_end = content.index(human_end_marker, human_start) + len("    r\n}")

    first_human, rest_human = _vow_pushstr_body(human_str)
    human_fn = f'fn skill_human() -> String {{\n    let r: String = String::from("{first_human}\\n");\n{rest_human}\n    r\n}}'
    content = content[:human_start] + human_fn + content[human_end:]

    return content


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

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

    if check_only:
        print("Generated JSON is valid.")
        print(f"  types: {len(data['language']['types'])}")
        print(f"  effects: {len(data['language']['effects'])}")
        print(f"  builtins: {len(data['language']['builtins'])}")
        print(f"  build_options: {len(data['build_options'])}")
        print(f"  verify_options: {len(data['verify_options'])}")
        return

    # Inject into Rust
    new_rs = inject_rust(MAIN_RS, json_str, human_str)
    MAIN_RS.write_text(new_rs)
    print(f"Updated {MAIN_RS}")

    # Inject into self-hosted
    new_vow = inject_vow(MAIN_VOW, json_str, human_str)
    MAIN_VOW.write_text(new_vow)
    print(f"Updated {MAIN_VOW}")

    print("\nDone. Run 'cargo build --release -p vow' and 'scripts/bootstrap.sh --no-verify --skip-cargo' to rebuild.")


if __name__ == "__main__":
    main()
