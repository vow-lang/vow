#!/usr/bin/env python3
"""Generate --help JSON and human-readable output from skill docs.

Reads docs/spec/grammar.md and docs/spec/cli.md (the canonical specs),
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
SPEC_DIR = REPO / "docs" / "spec"
GRAMMAR = SPEC_DIR / "grammar.md"
CLI = SPEC_DIR / "cli.md"
CONTRACTS = SPEC_DIR / "contracts.md"
INDEX = SPEC_DIR / "index.md"
ERRORS = SPEC_DIR / "errors.md"
EXAMPLES = SPEC_DIR / "examples.md"
SCHEMAS_DIR = SPEC_DIR / "schemas"
MAIN_RS = REPO / "vow" / "src" / "main.rs"
MAIN_VOW = REPO / "compiler" / "main.vow"

SCHEMA_VERSION = "2"
DEFAULT_MAX_K_STEP = 50
DEFAULT_SOLVER = "auto"
DEFAULT_ENCODING = "auto"


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
    Returns list of rows, each row is a list of cell strings (backticks stripped)."""
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


def normalize_option(
    flag: str,
    default: str,
    desc: str,
    *,
    output_default: str | None = None,
    merge_mode: bool = False,
) -> tuple[str, str, dict]:
    """Normalize an option row for both legacy dict output and structured help."""
    default = default.strip("`")
    description = desc
    normalized_flag = flag

    if merge_mode:
        normalized_flag = "--mode <debug|release>"
        description = (
            f"Build mode; debug inserts runtime vow checks (default: {default})"
        )
    elif flag == "-o, --output":
        normalized_flag = "-o, --output <path>"
        if output_default is not None:
            description = f"{desc} (default: {output_default})"
    elif default not in ("", "(off)", "(default)") and not desc.endswith(")"):
        description = f"{desc} (default: {default})"

    option: dict[str, object] = {
        "form": normalized_flag,
        "description": description,
    }

    if normalized_flag.startswith("-o, --output"):
        option["short"] = "-o"
        option["long"] = "--output"
        option["value_name"] = "path"
        option["value_kind"] = "path"
    else:
        head, _, tail = normalized_flag.partition(" ")
        option["long"] = head
        if tail:
            value_name = tail.strip()[1:-1]
            option["value_name"] = value_name
            if value_name == "N":
                option["value_kind"] = "integer"
            elif "|" in value_name:
                option["value_kind"] = "enum"
                option["values"] = value_name.split("|")
            else:
                option["value_kind"] = "string"
        else:
            option["value_kind"] = "flag"

    if normalized_flag == "--mode <debug|release>":
        option["value_name"] = "mode"
        option["value_kind"] = "enum"
        option["values"] = ["debug", "release"]
        option["default"] = default
    elif normalized_flag == "--debug-trace <off|calls|full>":
        option["value_name"] = "trace"
        option["value_kind"] = "enum"
        option["values"] = ["off", "calls", "full"]
        option["default"] = default
    elif normalized_flag in ("--max-k-step <N>", "--vec-max <N>", "--string-max <N>", "--hashmap-max <N>", "--btreemap-max <N>"):
        option["default"] = int(default) if default.isdigit() else default
    elif output_default is not None:
        option["default"] = output_default
    elif default not in ("", "(off)", "(default)"):
        option["default"] = default

    return normalized_flag, description, option


# ---------------------------------------------------------------------------
# Build JSON structure from grammar.md + cli.md
# ---------------------------------------------------------------------------

def build_help_json(grammar: str, cli: str, _contracts: str) -> dict:
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
    bitwise = extract_table_col(grammar, "Bitwise Operators", 0)

    # --- Methods ---
    def method_names(heading: str) -> list[str]:
        return extract_table_col(grammar, heading, 0)

    vec_methods = method_names("Vec<T> Methods")
    string_methods = method_names("String Methods")
    hashmap_methods = method_names("HashMap<K, V> Methods")
    btreemap_methods = method_names("BTreeMap<K, V> Methods")
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

    # --- CLI: command options ---
    build_opt_rows = extract_table(cli, "vow build", heading_level=3)
    build_options: dict[str, str] = {}
    build_option_entries: list[dict] = []
    for row in build_opt_rows:
        if len(row) < 3:
            continue
        flag, default, desc = row[0], row[1], row[2]
        if flag == "--mode debug":
            key, value, option = normalize_option(
                flag,
                default,
                desc,
                merge_mode=True,
            )
            build_options[key] = value
            build_option_entries.append(option)
            continue
        if flag == "--mode release":
            continue
        key, value, option = normalize_option(
            flag,
            default,
            desc,
            output_default="source without .vow extension" if flag == "-o, --output" else None,
        )
        build_options[key] = value
        build_option_entries.append(option)

    verify_opt_rows = extract_table(cli, "vow verify", heading_level=3)
    verify_options: dict[str, str] = {}
    verify_option_entries: list[dict] = []
    for row in verify_opt_rows:
        if len(row) < 3:
            continue
        key, value, option = normalize_option(row[0], row[1], row[2])
        verify_options[key] = value
        verify_option_entries.append(option)

    decl_opt_rows = extract_table(cli, "vow decl", heading_level=3)
    decl_options: dict[str, str] = {}
    decl_option_entries: list[dict] = []
    for row in decl_opt_rows:
        if len(row) < 3:
            continue
        key, value, option = normalize_option(
            row[0],
            row[1],
            row[2],
            output_default="<source>.vow.d" if row[0] == "-o, --output" else None,
        )
        decl_options[key] = value
        decl_option_entries.append(option)

    test_opt_rows = extract_table(cli, "vow test", heading_level=3)
    test_options: dict[str, str] = {}
    test_option_entries: list[dict] = []
    for row in test_opt_rows:
        if len(row) < 3:
            continue
        flag, default, desc = row[0], row[1], row[2]
        if flag.startswith("<"):
            continue
        if flag == "--mode debug":
            key, value, option = normalize_option(
                flag,
                default,
                desc,
                merge_mode=True,
            )
            test_options[key] = value
            test_option_entries.append(option)
            continue
        if flag == "--mode release":
            continue
        key, value, option = normalize_option(flag, default, desc)
        test_options[key] = value
        test_option_entries.append(option)

    contracts_opt_rows = extract_table(cli, "vow contracts", heading_level=3)
    contracts_options: dict[str, str] = {}
    contracts_option_entries: list[dict] = []
    for row in contracts_opt_rows:
        if len(row) < 3:
            continue
        key, value, option = normalize_option(row[0], row[1], row[2])
        contracts_options[key] = value
        contracts_option_entries.append(option)

    # --- Verification defaults (configurable via CLI flags) ---
    verification_defaults: dict[str, str | int] = {
        "strategy": "k-induction-parallel",
        "max_k_step": DEFAULT_MAX_K_STEP,
        "vec_max": 128,
        "string_max": 256,
        "hashmap_max": 64,
        "btreemap_max": 64,
    }

    return {
        "schema_version": SCHEMA_VERSION,
        "kind": "tool_help",
        "tool": "vow",
        "audience": "agent",
        "default_format": "json",
        "description": "Vow compiler: compiles Vow source to native executables with contract verification",
        "usage": "vow <command> [OPTIONS] <source.vow>",
        "legacy_usage": "vow [OPTIONS] <source.vow> (equivalent to vow build)",
        "references": {
            "grammar": "reference/grammar.md",
            "cli": "reference/cli.md",
            "contracts": "reference/contracts.md",
            "errors": "reference/errors.md",
            "examples": "examples/examples.md",
            "schemas": {
                "build_result": "schemas/build-result.schema.json",
                "contracts_result": "schemas/contracts-result.schema.json",
                "diagnostic": "schemas/diagnostic.schema.json",
                "counterexample": "schemas/counterexample.schema.json",
                "mutants_result": "schemas/mutants-result.schema.json",
                "test_result": "schemas/test-result.schema.json",
                "vow_violation": "schemas/vow-violation.schema.json",
            },
        },
        "invocation": {
            "canonical": "vow <command> [OPTIONS] <source.vow>",
            "default_command": "build",
            "legacy_equivalent": "vow [OPTIONS] <source.vow>",
            "source_argument": {
                "name": "source",
                "kind": "path",
                "required": True,
                "suffix": ".vow",
            },
        },
        "commands": {
            "build": "Compile source to native executable (verifies by default; use --no-verify to skip)",
            "verify": "Verify contracts without producing an executable (use --no-cache to skip cache)",
            "test": "Run tests: discover, compile, execute test_*.vow files with JSON results",
            "decl": "Emit declaration file (.vow.d) with type signatures only",
            "contracts": "List all contracts with optional verification status",
            "skill": "Generate or install the Claude Code skill document for this compiler version",
        },
        "command_details": {
            "build": {
                "status": "implemented",
                "usage": "vow build [OPTIONS] <source.vow>",
                "default_when_command_omitted": True,
                "arguments": [
                    {"name": "source", "kind": "path", "required": True, "suffix": ".vow"}
                ],
                "options": build_option_entries,
                "stdout": {
                    "format": "json",
                    "schema_ref": "schemas/build-result.schema.json",
                    "suppressed_by": ["--dump-ir"],
                },
                "stderr": {
                    "channels": ["diagnostic stream", "debug trace"],
                    "debug_trace_flag": "--debug-trace <off|calls|full>",
                },
                "notes": [
                    "verification is enabled by default",
                    "debug mode inserts runtime vow checks",
                ],
            },
            "verify": {
                "status": "implemented",
                "usage": "vow verify [OPTIONS] <source.vow>",
                "arguments": [
                    {"name": "source", "kind": "path", "required": True, "suffix": ".vow"}
                ],
                "options": verify_option_entries,
                "stdout": {
                    "format": "json",
                    "schema_ref": "schemas/build-result.schema.json",
                    "fixed_fields": {"executable": None},
                },
                "notes": [
                    "runs verification only and never emits a binary",
                ],
            },
            "test": {
                "status": "implemented",
                "usage": "vow test [OPTIONS] [<path>]",
                "arguments": [
                    {
                        "name": "path",
                        "kind": "path",
                        "required": False,
                        "default": ".",
                        "description": "Directory to scan or single .vow file",
                    }
                ],
                "options": test_option_entries,
                "stdout": {
                    "format": "json",
                },
                "notes": [
                    "discovers test_*.vow and *_test.vow files",
                    "each test must contain main() -> i32 returning 0 on success",
                    "default mode is debug (runtime vow checks enabled)",
                ],
            },
            "decl": {
                "status": "implemented",
                "usage": "vow decl [OPTIONS] <source.vow>",
                "arguments": [
                    {"name": "source", "kind": "path", "required": True, "suffix": ".vow"}
                ],
                "options": decl_option_entries,
                "stdout": {"format": "none"},
                "side_effects": [
                    {
                        "kind": "write_file",
                        "default_path": "<source>.vow.d",
                    }
                ],
            },
            "contracts": {
                "status": "implemented",
                "usage": "vow contracts [OPTIONS] <source.vow>",
                "arguments": [
                    {"name": "source", "kind": "path", "required": True, "suffix": ".vow"}
                ],
                "options": contracts_option_entries,
                "stdout": {
                    "format": "json",
                    "schema_ref": "schemas/contracts-result.schema.json",
                },
                "notes": [
                    "runs frontend only by default",
                    "use --verify for per-contract ESBMC status",
                ],
            },
        },
        "build_options": build_options,
        "verify_options": verify_options,
        "test_options": test_options,
        "decl_options": decl_options,
        "contracts_options": contracts_options,
        "global_options": {
            "--help": "Emit versioned JSON tool-help data",
            "--help --human": "Emit legacy human-readable help (compatibility mode)",
        },
        "outputs": {
            "build_result": {
                "schema_ref": "schemas/build-result.schema.json",
                "emitted_by": ["build", "verify"],
                "status_values": ["Verified", "Unverified", "CompileFailed", "VerifyFailed"],
                "legacy_fields": ["counterexample"],
            },
            "contracts_result": {
                "schema_ref": "schemas/contracts-result.schema.json",
                "emitted_by": ["contracts"],
            },
            "diagnostic": {
                "schema_ref": "schemas/diagnostic.schema.json",
                "embedded_in": "build_result.diagnostics",
            },
            "runtime_vow_violation": {
                "schema_ref": "schemas/vow-violation.schema.json",
                "emitted_on": "stderr",
                "requires_mode": "debug",
            },
            "runtime_trace": {
                "emitted_on": "stderr",
                "enabled_by": "--debug-trace <off|calls|full>",
                "format": "jsonl",
            },
        },
        "output_json": {
            "status": "Verified | Unverified | CompileFailed | VerifyFailed",
            "executable": "path to compiled binary, or null",
            "diagnostics": "[array of {error_code, message, severity, span: {file, offset, length}}]",
            "message": "error detail (CompileFailed)",
            "function": "function name (VerifyFailed)",
            "counterexample": "ESBMC counterexample description (VerifyFailed)",
        },
        "diagnostics": {
            "schema_ref": "schemas/diagnostic.schema.json",
            "fields": [
                "error_code",
                "message",
                "severity",
                "span.file",
                "span.offset",
                "span.length",
            ],
        },
        "exit_codes": {
            "0": "success (Verified or Unverified)",
            "1": "failure (CompileFailed or VerifyFailed)",
        },
        "language": {
            "module": "module <Name>",
            "use_declaration": "use foo.bar",
            "const_declaration": "const NAME: i64 = 1024",
            "comments": "// line comments only; block comments unsupported",
            "let_binding": "let name: Type = expr; or let mut name: Type = expr;",
            "function": "fn <name>(<params>) -> <RetTy> [<effects>] { <body> }",
            "public_function": "pub fn <name>(<params>) -> <RetTy> [<effects>] { <body> }",
            "vow_function": "fn <name>(<params>) -> <RetTy> vow { requires: <expr>; ensures: <expr> } { <body> }",
            "while_with_invariant": "while <cond> vow { invariant: <expr> } { <body> }",
            "literals": {
                "integer": "42 | -1 | 42u64 (unsuffixed integers default to i64)",
                "float": "3.14 | -0.5",
                "bool": "true | false",
                "string": "\"text\" with escapes \\n \\t \\r \\\\ \\\" \\0",
            },
            "casts": "x as u64 or y as i64",
            "types": all_types,
            "effects": effects,
            "builtins": builtins,
            "operators": {
                "arithmetic": arith,
                "checked_arithmetic": checked,
                "comparison": comparison,
                "logical": logical,
                "bitwise": bitwise,
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
                "linear": "linear struct Name { field: Type, ... } \u2014 linear obligation must be consumed or returned before region close",
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
                "BTreeMap<K,V>": btreemap_methods,
                "Option<T>": option_methods,
            },
            "error_propagation": "? on Option<T> or Result<T, E> propagates None/Err to the caller",
            "indexing": {
                "read": "v[i] \u2014 Vec index access",
                "write": "v[i] = val \u2014 Vec index assignment",
            },
            "feature_status": {
                "implemented": {
                    "function_vow_blocks": "requires / ensures / invariant",
                    "where_clauses": "parameter-level refinement sugar",
                    "loop_invariants": "simple invariant predicates",
                },
                "partial": {
                    "refinement_type_predicates": "parsed but semantically erased; use where clauses or function vows for verification",
                    "effect_tracking": "user-defined effect propagation is enforced; some builtin panic/unsafe effects are not yet modeled",
                },
                "target": {
                    "module_level_vow_blocks": "specified in docs but not parsed or represented in the AST",
                    "quantifiers": "forall / exists are not yet in the lexer or parser",
                },
                "unsupported": [
                    "user-defined generics",
                    "traits",
                    "closures",
                    "operator overloading",
                    "macros",
                    "assert / assume statements",
                ],
            },
        },
        "verification_defaults": verification_defaults,
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
    lines.append("  vow test [OPTIONS] [<path>]          Run tests with JSON results")
    lines.append("  vow contracts [OPTIONS] <source.vow> List all contracts")
    lines.append("  vow decl [OPTIONS] <source.vow>    Emit declaration file (.vow.d)")
    lines.append("  vow skill [print [--bundle]|install [--local|--global]]")
    lines.append("                                        Generate or install Claude Code skill")
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

    if data.get("test_options"):
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

    if data.get("decl_options"):
        lines.append("DECL OPTIONS")
        for flag, desc in data["decl_options"].items():
            pad = max(24, len(flag) + 2)
            lines.append(f"  {flag:<{pad}s}{desc}")
        lines.append("")

    lines.append("GLOBAL OPTIONS")
    lines.append("  --help                Emit versioned JSON tool-help data")
    lines.append("  --help --human        Emit legacy text help")
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
    bm_short = short_methods(methods["BTreeMap<K,V>"])
    opt_short = short_methods(methods["Option<T>"])
    lines.append(f"METHODS   : Vec: {vec_short}   String: {str_short}")
    lines.append(f"            HashMap: {hm_short}   BTreeMap: {bm_short}   Option: {opt_short}")

    ops = lang["operators"]
    arith = " ".join(ops["arithmetic"])
    checked = " ".join(ops["checked_arithmetic"])
    comp = " ".join(ops["comparison"])
    logical = " ".join(ops["logical"])
    bitwise_str = " ".join(ops["bitwise"])
    unary = " ".join(ops["unary"])
    lines.append(f"OPERATORS : {arith}   {checked} (checked)   {comp}   {logical}   {bitwise_str} (bitwise, integer-only)   unary {unary}")
    lines.append("")

    vdefaults = data.get("verification_defaults", {})
    if vdefaults:
        lines.append("VERIFICATION DEFAULTS (configurable via --max-k-step, --vec-max, --string-max, --hashmap-max, --btreemap-max)")
        lines.append(f"  Strategy        : {vdefaults.get('strategy', 'k-induction-parallel')} (incremental BMC + k-induction)")
        lines.append(f"  Incremental BMC : {vdefaults.get('max_k_step', DEFAULT_MAX_K_STEP)} max iterations (--max-k-step)")
        lines.append(f"  Vec<T>          : {vdefaults.get('vec_max', 128)} max capacity")
        lines.append(f"  String          : {vdefaults.get('string_max', 256)} max capacity")
        lines.append(f"  HashMap<K, V>   : {vdefaults.get('hashmap_max', 64)} max capacity")
        lines.append(f"  BTreeMap<K, V>  : {vdefaults.get('btreemap_max', 64)} max capacity")

    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Inject into Rust main.rs
# ---------------------------------------------------------------------------

def _replace_between_markers(content: str, start_marker: str, end_marker: str, replacement: str) -> str:
    """Replace content between start/end marker lines (inclusive)."""
    start_idx = content.find(start_marker)
    if start_idx == -1:
        raise ValueError(f"Cannot find marker: {start_marker}")
    end_idx = content.find(end_marker, start_idx)
    if end_idx == -1:
        raise ValueError(f"Cannot find marker: {end_marker}")
    end_idx += len(end_marker)
    return content[:start_idx] + replacement + content[end_idx:]


def inject_rust(main_rs: Path, json_str: str, human_str: str) -> str:
    content = main_rs.read_text()

    content = _replace_between_markers(
        content,
        "// GENERATE:SKILL_JSON:START",
        "// GENERATE:SKILL_JSON:END",
        f'// GENERATE:SKILL_JSON:START\nfn skill_json() -> String {{\n    r##"{json_str}"##\n    .to_string()\n}}\n// GENERATE:SKILL_JSON:END',
    )

    content = _replace_between_markers(
        content,
        "// GENERATE:SKILL_HUMAN:START",
        "// GENERATE:SKILL_HUMAN:END",
        f'// GENERATE:SKILL_HUMAN:START\nfn skill_human() -> String {{\n    r##"{human_str}"##\n        .to_string()\n}}\n// GENERATE:SKILL_HUMAN:END',
    )

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

    first_json, rest_json = _vow_pushstr_body(json_str)
    json_fn = f'// GENERATE:SKILL_JSON:START\nfn skill_json() -> String {{\n    let r: String = String::from("{first_json}\\n");\n{rest_json}\n    r\n}}\n// GENERATE:SKILL_JSON:END'
    content = _replace_between_markers(
        content, "// GENERATE:SKILL_JSON:START", "// GENERATE:SKILL_JSON:END", json_fn,
    )

    first_human, rest_human = _vow_pushstr_body(human_str)
    human_fn = f'// GENERATE:SKILL_HUMAN:START\nfn skill_human() -> String {{\n    let r: String = String::from("{first_human}\\n");\n{rest_human}\n    r\n}}\n// GENERATE:SKILL_HUMAN:END'
    content = _replace_between_markers(
        content, "// GENERATE:SKILL_HUMAN:START", "// GENERATE:SKILL_HUMAN:END", human_fn,
    )

    return content


# ---------------------------------------------------------------------------
# Build full skill markdown from spec files
# ---------------------------------------------------------------------------

SKILL_FRONTMATTER = """\
---
name: vow-toolchain
description: >-
  Write, compile, debug, and verify Vow programs (.vow files) with contracts,
  CEGIS, ESBMC counterexamples, diagnostics, and vow build / vow verify.
when_to_use: >-
  Use when the user edits or creates .vow files, says "write a Vow program",
  "fix this counterexample", "add contracts", "why did verification fail",
  "ESBMC", "vow build", or "vow verify".
argument-hint: "[file.vow]"
allowed-tools: "Bash(build/vowc *) Bash(vow *) Bash(vowc *) Bash(ulimit *)"
---"""


def _index_for_skill() -> str:
    index_text = INDEX.read_text()
    omit_start = "<!-- OMIT-FROM-SKILL-START -->"
    omit_end = "<!-- OMIT-FROM-SKILL-END -->"
    start_idx = index_text.find(omit_start)
    if start_idx != -1:
        end_idx = index_text.find(omit_end, start_idx)
        if end_idx != -1:
            index_text = (
                index_text[:start_idx].rstrip()
                + index_text[end_idx + len(omit_end):]
            )
    return index_text.rstrip()


def build_skill_entrypoint() -> str:
    """Build the concise installed SKILL.md entrypoint."""
    return "\n".join(
        [
            SKILL_FRONTMATTER,
            "",
            "# Vow Toolchain",
            "",
            "Use this skill when writing, compiling, debugging, or verifying Vow programs.",
            "Keep the workflow tight: run the compiler, read the structured JSON, fix the",
            "program or contract, and repeat until the result is `Verified`.",
            "",
            "## Installed toolchain (live)",
            "",
            "!`(command -v vow >/dev/null 2>&1 && vow --help 2>/dev/null | head -200) || (command -v build/vowc >/dev/null 2>&1 && build/vowc --help 2>/dev/null | head -200)`",
            "",
            "## Core workflow",
            "",
            "1. Write a `.vow` file with explicit contracts.",
            "2. Run `ulimit -v 2000000; build/vowc build <file.vow>`.",
            "3. Parse stdout JSON and inspect `status`, `diagnostics`, and `counterexamples`.",
            "4. Fix compile errors, verification failures, or weak contracts, then rerun.",
            "",
            "## Minimal program",
            "",
            "```vow",
            "module Hello",
            "",
            "fn main() -> i32 [io] {",
            "    print_str(\"Hello, world!\");",
            "    0",
            "}",
            "```",
            "",
            "## Reference files",
            "",
            "- Grammar, types, effects, builtins: [reference/grammar.md](reference/grammar.md)",
            "- CLI commands, flags, JSON output: [reference/cli.md](reference/cli.md)",
            "- Contracts and CEGIS guidance: [reference/contracts.md](reference/contracts.md)",
            "- Diagnostics and fixes: [reference/errors.md](reference/errors.md)",
            "- Worked examples: [examples/examples.md](examples/examples.md)",
            "- JSON schemas: [schemas/](schemas/)",
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
    for spec_file in [GRAMMAR, CLI, CONTRACTS, ERRORS, EXAMPLES]:
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
        "reference/errors.md": ERRORS.read_text().rstrip() + "\n",
        "examples/examples.md": EXAMPLES.read_text().rstrip() + "\n",
    }
    for sf in sorted(SCHEMAS_DIR.glob("*.json")):
        support[f"schemas/{sf.name}"] = sf.read_text().rstrip() + "\n"
    return support


def _rust_raw_string(text: str) -> str:
    for n in range(1, 16):
        hashes = "#" * n
        if f'"{hashes}' not in text:
            return f'r{hashes}"{text}"{hashes}'
    raise ValueError("could not find Rust raw string delimiter")


def inject_skill_rust(
    content: str, entrypoint_md: str, bundle_md: str, support_files: dict[str, str]
) -> str:
    """Inject generated skill helpers into Rust main.rs."""
    entries = ",\n        ".join(
        f"({_rust_raw_string(path)}, {_rust_raw_string(body)})"
        for path, body in support_files.items()
    )
    replacement = f'''// GENERATE:SKILL_FULL:START
fn skill_entrypoint_markdown() -> String {{
    {_rust_raw_string(entrypoint_md)}
    .to_string()
}}

fn skill_bundle_markdown() -> String {{
    {_rust_raw_string(bundle_md)}
    .to_string()
}}

fn skill_support_files() -> &'static [(&'static str, &'static str)] {{
    &[
        {entries}
    ]
}}
// GENERATE:SKILL_FULL:END'''
    return _replace_between_markers(
        content, "// GENERATE:SKILL_FULL:START", "// GENERATE:SKILL_FULL:END", replacement,
    )


def inject_skill_vow(
    content: str, entrypoint_md: str, bundle_md: str, support_files: dict[str, str]
) -> str:
    """Inject generated skill helpers into self-hosted main.vow."""
    first_entrypoint, rest_entrypoint = _vow_pushstr_body(entrypoint_md)
    first_bundle, rest_bundle = _vow_pushstr_body(bundle_md)

    sections = [
        "// GENERATE:SKILL_FULL:START",
        "fn skill_entrypoint() -> String {",
        f'    let r: String = String::from("{first_entrypoint}\\n");',
        rest_entrypoint,
        "    r",
        "}",
        "",
        "fn skill_bundle() -> String {",
        f'    let r: String = String::from("{first_bundle}\\n");',
        rest_bundle,
        "    r",
        "}",
        "fn skill_support_paths() -> Vec<String> {",
        "    let v: Vec<String> = Vec::new();",
    ]
    for path in support_files:
        escaped = path.replace("\\", "\\\\").replace('"', '\\"')
        sections.append(f'    v.push(String::from("{escaped}"));')
    sections.extend(["    v", "}", ""])

    content_fn_names = []
    for idx, body in enumerate(support_files.values()):
        fn_name = f"skill_support_content_{idx}"
        content_fn_names.append(fn_name)
        first, rest = _vow_pushstr_body(body)
        sections.extend(
            [
                f"fn {fn_name}() -> String {{",
                f'    let r: String = String::from("{first}\\n");',
                rest,
                "    r",
                "}",
                "",
            ]
        )

    sections.extend(
        [
            "fn skill_support_contents() -> Vec<String> {",
            "    let v: Vec<String> = Vec::new();",
        ]
    )
    for fn_name in content_fn_names:
        sections.append(f"    v.push({fn_name}());")
    sections.extend(["    v", "}", "// GENERATE:SKILL_FULL:END"])

    fn_body = "\n".join(line for line in sections if line is not None)
    return _replace_between_markers(
        content, "// GENERATE:SKILL_FULL:START", "// GENERATE:SKILL_FULL:END", fn_body,
    )


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
        return

    # Inject into Rust
    new_rs = inject_rust(MAIN_RS, json_str, human_str)
    new_rs = inject_skill_rust(new_rs, skill_entrypoint_md, skill_bundle_md, skill_support_files)
    MAIN_RS.write_text(new_rs)
    print(f"Updated {MAIN_RS}")

    # Inject into self-hosted
    new_vow = inject_vow(MAIN_VOW, json_str, human_str)
    new_vow = inject_skill_vow(new_vow, skill_entrypoint_md, skill_bundle_md, skill_support_files)
    MAIN_VOW.write_text(new_vow)
    print(f"Updated {MAIN_VOW}")

    print("\nDone. Run 'cargo build --release -p vow' and 'scripts/bootstrap.sh --no-verify --skip-cargo' to rebuild.")


if __name__ == "__main__":
    main()
