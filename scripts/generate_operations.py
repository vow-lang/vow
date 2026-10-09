#!/usr/bin/env python3
"""Generate Operation Catalogue projections for both compilers.

Reads docs/spec/operations.json (the Operation Catalogue: the checked,
hand-edited source of runtime-symbol/ABI/return-shape/doc facts for a set of
Builtin Operations) and splices generated lookup functions into:
  - vow-ir/src/lower/mod.rs           (catalogue_builtin_to_runtime)
  - vow-codegen/src/cranelift_backend.rs (catalogue_extern_sig)
  - vow-clif-shim/src/lib.rs             (catalogue_extern_sig, FRESH_ARENA_VARIANTS)
  - compiler/lower.vow                (catalogue_builtin_to_extern, catalogue_builtin_ret_ty)
  - vow-ir/src/region.rs              (FRESH_ARENA_VARIANTS)
  - compiler/ir.vow                   (fresh_arena_base_extern)
  - compiler/vc_ops.vow               (catalogue_verifier_known)
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
    "i64": {"rust_ty": "Ty::I64", "ity_const": "ITY_I64()", "clif_ret": "types::I64"},
    "ptr": {"rust_ty": "Ty::Ptr", "ity_const": "ITY_PTR()", "clif_ret": "types::I64"},
    "u64": {"rust_ty": "Ty::U64", "ity_const": "ITY_U64()", "clif_ret": "types::I64"},
    "bool": {
        "rust_ty": "Ty::Bool",
        "ity_const": "ITY_BOOL()",
        "clif_ret": "types::I64",
    },
}
PARAM_TOKENS = {
    "ptr": "types::I64",
    "i64": "types::I64",
    "u64": "types::I64",
}

# Effect keyword spellings, sourced from vow-syntax/src/token.rs's effect
# keyword lexing -- the same spellings the printer emits and grammar.md's
# "Builtin Function Signatures" table documents.
EFFECT_TOKENS = {"read", "write", "io", "panic", "unsafe"}

EFFECTS_RE = re.compile(r"^\[(?P<tokens>[a-z]+(?:, [a-z]+)*)?\]$")

DOC_SIGNATURE_RE = re.compile(r"^fn\((?P<params>.*)\) -> (?P<ret>.+)$")


def _split_top_level_commas(s: str) -> list[str]:
    """Split on commas at bracket depth 0, so a generic type's internal
    comma (e.g. a future `Pair<A, B>`) never splits a single parameter."""
    if not s:
        return []
    parts = []
    depth = 0
    start = 0
    for i, ch in enumerate(s):
        if ch in "<(":
            depth += 1
        elif ch in ">)":
            depth -= 1
        elif ch == "," and depth == 0:
            parts.append(s[start:i])
            start = i + 1
    parts.append(s[start:])
    return [p.strip() for p in parts]


def _doc_type_to_token(type_str: str) -> str | None:
    """Map a doc_signature parameter/return Type spelling to its catalogue
    token. `String` and any `Vec<...>` both denote a heap pointer, so both
    map to `ptr` -- this is a many-to-one mapping, not a bijection."""
    if type_str == "String" or re.match(r"^Vec<.*>$", type_str):
        return "ptr"
    if type_str in ("i64", "u64", "bool"):
        return type_str
    if type_str in ("()", "!"):
        # `!` (never-returns, e.g. process_exit) produces no value either --
        # same ABI shape as unit, just with an unreachable fallthrough.
        return "unit"
    return None


REQUIRED_FIELDS = [
    "name",
    "runtime_symbol",
    "params",
    "return",
    "doc_signature",
    "effects",
]

# Optional fields a catalogue entry may carry, each with its own closed
# vocabulary. Field names/values agreed with sibling in-progress catalogue
# slices (#1271/#1273) so a future rebase is a clean superset merge.
#
# "arena_routing: heap_fresh" means the op returns a freshly heap-allocated
# String that pin_to_root/arena tracking must tag; "none" (or omitting the
# field) means no tagging is needed.
OPTIONAL_FIELD_VALUES = {
    "verifier_model": {"known", "unmodeled"},
    "arena_routing": {"none", "heap_fresh"},
}

ALLOWED_FIELDS = set(REQUIRED_FIELDS) | set(OPTIONAL_FIELD_VALUES)


def load_catalogue(repo_root: Path) -> list[dict]:
    """Read and hand-validate docs/spec/operations.json under repo_root.

    Collects every violation across every entry before raising, so a
    catalogue with several unrelated problems is fully diagnosed in one run.
    Raises ValueError with all violations (newline-separated) if any exist.
    """
    catalogue_path = repo_root / "docs" / "spec" / "operations.json"
    data = json.loads(catalogue_path.read_text())
    if not isinstance(data.get("operations"), list):
        raise ValueError(f"{catalogue_path}: top-level 'operations' key must be a list")
    ops = data["operations"]

    errors: list[str] = []
    seen_names: set[str] = set()
    seen_symbols: set[str] = set()

    for op in ops:
        name = op.get("name", "<unnamed>")

        missing_fields = [f for f in REQUIRED_FIELDS if f not in op]
        if missing_fields:
            for field in missing_fields:
                errors.append(f"operation '{name}' is missing required field '{field}'")
            # Required fields are a precondition for every check below --
            # a missing field must not reach e.g. the doc_signature arity
            # check and raise a raw TypeError one layer deeper.
            continue

        unknown_fields = set(op) - ALLOWED_FIELDS
        if unknown_fields:
            errors.append(
                f"operation '{name}' has unknown field(s) {sorted(unknown_fields)} "
                f"(allowed: {sorted(ALLOWED_FIELDS)})"
            )

        for field, allowed_values in OPTIONAL_FIELD_VALUES.items():
            if field in op and op[field] not in allowed_values:
                errors.append(
                    f"operation '{name}' has unknown {field} '{op[field]}' "
                    f"(known: {sorted(allowed_values)})"
                )

        if name in seen_names:
            errors.append(f"duplicate operation name '{name}'")
        seen_names.add(name)

        symbol = op["runtime_symbol"]
        if symbol in seen_symbols:
            errors.append(f"duplicate runtime_symbol '{symbol}' (operation '{name}')")
        seen_symbols.add(symbol)

        ret = op["return"]
        ret_valid = ret in RETURN_TOKENS
        if not ret_valid:
            errors.append(
                f"operation '{name}' has unknown return token '{ret}' "
                f"(known: {sorted(RETURN_TOKENS)})"
            )

        params = op["params"]
        params_valid = isinstance(params, list)
        if not params_valid:
            errors.append(
                f"operation '{name}' has 'params' of type {type(params).__name__}, "
                "expected a list"
            )
        else:
            for param in params:
                if param not in PARAM_TOKENS:
                    errors.append(
                        f"operation '{name}' has unknown params token '{param}' "
                        f"(known: {sorted(PARAM_TOKENS)})"
                    )

        effects = op["effects"]
        if not isinstance(effects, str):
            errors.append(
                f"operation '{name}' has 'effects' of type "
                f"{type(effects).__name__}, expected a string"
            )
        else:
            match = EFFECTS_RE.match(effects)
            if match is None:
                errors.append(
                    f"operation '{name}' has malformed effects string {effects!r} "
                    f"(expected e.g. '[]', '[io]', '[read, write]')"
                )
            else:
                tokens = match.group("tokens")
                eff_tokens = tokens.split(", ") if tokens else []
                unknown = [t for t in eff_tokens if t not in EFFECT_TOKENS]
                if unknown:
                    errors.append(
                        f"operation '{name}' has unknown effects token(s) {unknown} "
                        f"in {effects!r} (known: {sorted(EFFECT_TOKENS)})"
                    )
                elif eff_tokens != sorted(eff_tokens):
                    errors.append(
                        f"operation '{name}' has unsorted effects {effects!r} "
                        f"(expected {sorted(eff_tokens)!r} order)"
                    )

        doc_signature = op["doc_signature"]
        if not isinstance(doc_signature, str):
            errors.append(
                f"operation '{name}' has 'doc_signature' of type "
                f"{type(doc_signature).__name__}, expected a string"
            )
        else:
            sig_match = DOC_SIGNATURE_RE.match(doc_signature)
            if sig_match is None:
                errors.append(
                    f"operation '{name}' has malformed doc_signature "
                    f"{doc_signature!r} (expected 'fn(...) -> Type')"
                )
            else:
                if params_valid:
                    doc_params = [
                        p
                        for p in _split_top_level_commas(sig_match.group("params"))
                        if p
                    ]
                    if len(doc_params) != len(params):
                        errors.append(
                            f"operation '{name}' doc_signature declares "
                            f"{len(doc_params)} parameter(s) but 'params' has "
                            f"{len(params)}: {doc_signature!r}"
                        )
                    else:
                        for i, doc_param in enumerate(doc_params):
                            _, _, type_str = doc_param.partition(": ")
                            token = _doc_type_to_token(type_str.strip())
                            if token != params[i]:
                                errors.append(
                                    f"operation '{name}' doc_signature parameter "
                                    f"{i} ({doc_param!r}) does not match params "
                                    f"token '{params[i]}': {doc_signature!r}"
                                )
                if ret_valid:
                    ret_token = _doc_type_to_token(sig_match.group("ret").strip())
                    if ret_token != ret:
                        errors.append(
                            f"operation '{name}' doc_signature return type "
                            f"({sig_match.group('ret')!r}) does not match return "
                            f"token '{ret}': {doc_signature!r}"
                        )

    if errors:
        raise ValueError("\n".join(errors))

    return ops


ARENA_SYMBOL_RE = re.compile(r"^__vow_[a-z0-9_]+$")
ARENA_VARIANT_SUFFIX = "_in_arena"


def arena_variant(symbol: str) -> str:
    return f"{symbol}{ARENA_VARIANT_SUFFIX}"


def load_arena_routes(repo_root: Path, ops: list[dict]) -> list[str]:
    """Read and validate the catalogue's `arena_routes` section: the runtime
    symbols of every builtin whose result is a fresh heap aggregate (an Option
    cell, a String or a Vec). Each has a `<symbol>_in_arena` variant that takes
    the target arena as its first argument. A catalogue without the section
    has none.

    Collects every violation before raising a ValueError, like load_catalogue.
    """
    catalogue_path = repo_root / "docs" / "spec" / "operations.json"
    routes = json.loads(catalogue_path.read_text()).get("arena_routes", [])
    if not isinstance(routes, list):
        raise ValueError(f"{catalogue_path}: 'arena_routes' key must be a list")

    errors: list[str] = []
    seen: set[str] = set()
    for symbol in routes:
        if not isinstance(symbol, str) or not ARENA_SYMBOL_RE.match(symbol):
            errors.append(
                f"arena route {symbol!r} is not a symbol "
                f"(expected {ARENA_SYMBOL_RE.pattern})"
            )
            continue
        for sym in (symbol, arena_variant(symbol)):
            if sym in seen:
                errors.append(f"arena route symbol '{sym}' appears more than once")
            seen.add(sym)

    for op in ops:
        if (
            op.get("arena_routing") == "heap_fresh"
            and op["runtime_symbol"] not in routes
        ):
            errors.append(
                f"operation '{op['name']}' is heap_fresh but its runtime_symbol "
                f"'{op['runtime_symbol']}' has no arena route"
            )

    if errors:
        raise ValueError("\n".join(errors))
    return routes


def check_runtime_exports(routes: list[str], repo_root: Path) -> list[str]:
    """Every arena route's base and variant symbol must appear as an
    identifier in vow-runtime's sources (some are generated by macros there), or a
    backend would call a symbol that does not link."""
    runtime = "\n".join(
        path.read_text()
        for path in sorted((repo_root / "vow-runtime" / "src").glob("*.rs"))
    )
    mismatches: list[str] = []
    for route in routes:
        for sym in (route, arena_variant(route)):
            if not re.search(rf"\b{re.escape(sym)}\b", runtime):
                mismatches.append(f"vow-runtime/src: does not define '{sym}'")
    return mismatches


MARKER_START = "// GENERATE:OPERATIONS:START"
MARKER_END = "// GENERATE:OPERATIONS:END"


def _wrap_marker_block(body: str) -> str:
    """Wrap a generated function body (ending in "}\\n") in the marker pair
    every splice target shares."""
    return f"{MARKER_START}\n{body}{MARKER_END}"


def gen_rust_ir_block(ops: list[dict], routes: list[str]) -> str:
    arms = "\n".join(
        f'        "{op["name"]}" => Some(("{op["runtime_symbol"]}", '
        f"{RETURN_TOKENS[op['return']]['rust_ty']})),"
        for op in ops
    )
    tag_arms = "\n".join(
        f'        "{op["name"]}" => Some(BuiltinResultTag::StringHeap),'
        for op in ops
        if op.get("arena_routing") == "heap_fresh"
    )
    tag_arms_block = f"{tag_arms}\n" if tag_arms else ""
    return _wrap_marker_block(
        "fn catalogue_builtin_to_runtime(name: &str) -> Option<(&'static str, Ty)> {\n"
        "    match name {\n"
        f"{arms}\n"
        "        _ => None,\n"
        "    }\n"
        "}\n"
        "\n"
        "fn catalogue_builtin_result_tag(name: &str) -> Option<BuiltinResultTag> {\n"
        "    match name {\n"
        f"{tag_arms_block}"
        "        _ => None,\n"
        "    }\n"
        "}\n"
    )


def _cranelift_sig_fn(ops: list[dict]) -> str:
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
    return (
        "fn catalogue_extern_sig(sym: &str, sig: &mut Signature) -> bool {\n"
        "    match sym {\n"
        f"{arms}\n"
        "        _ => false,\n"
        "    }\n"
        "}\n"
    )


def gen_cranelift_block(ops: list[dict], routes: list[str]) -> str:
    return _wrap_marker_block(_cranelift_sig_fn(ops))


# rustfmt keeps a tuple on one line only while its elements fit `fn_call_width` (60).
RUSTFMT_TUPLE_WIDTH = 60


def _rust_fresh_table(routes: list[str], pub_items: bool) -> str:
    rows = []
    for base in routes:
        inner = f'"{base}", "{arena_variant(base)}"'
        if len(inner) <= RUSTFMT_TUPLE_WIDTH:
            rows.append(f"    ({inner}),")
        else:
            rows.append(
                f'    (\n        "{base}",\n        "{arena_variant(base)}",\n    ),'
            )
    visibility = "pub " if pub_items else ""
    table_rows = "\n".join(rows)
    return (
        "/// Builtins returning a fresh heap aggregate, paired with their\n"
        "/// `<name>_in_arena` variant (target arena first, then the base parameters).\n"
        f"{visibility}const FRESH_ARENA_VARIANTS: &[(&str, &str)] = &[\n"
        f"{table_rows}\n"
        "];\n"
    )


def gen_region_block(ops: list[dict], routes: list[str]) -> str:
    return _wrap_marker_block(_rust_fresh_table(routes, True))


def gen_shim_block(ops: list[dict], routes: list[str]) -> str:
    return _wrap_marker_block(
        _cranelift_sig_fn(ops) + "\n" + _rust_fresh_table(routes, False)
    )


def _vow_symbol_set_fn(name: str, symbols: list[str]) -> str:
    """A `fn name(sym: String) -> bool` that tests membership with a decision
    tree: dispatch on length, then on the byte at the most discriminating
    position, and confirm with a single full comparison at each leaf. A lookup
    costs a few byte reads and at most one String allocation, not one
    comparison (and allocation) per symbol."""
    lines = [f"fn {name}(sym: String) -> bool {{"]
    if symbols:
        lines.append("    let n: u64 = sym.len();")

    def emit(group: list[str], depth: int, indent: str) -> None:
        if len(group) == 1:
            lines.append(f'{indent}return sym == String::from("{group[0]}");')
            return
        width = len(group[0])
        position = max(
            (i for i in range(width) if len({g[i] for g in group}) > 1),
            key=lambda i: len({g[i] for g in group}),
        )
        var = f"c{depth}"
        lines.append(f"{indent}let {var}: i64 = sym.byte_at({position});")
        for ch in sorted({g[position] for g in group}):
            subset = [g for g in group if g[position] == ch]
            lines.append(f"{indent}if {var} == {ord(ch)} {{")
            emit(subset, depth + 1, indent + "    ")
            lines.append(f"{indent}}}")

    for length in sorted({len(s) for s in symbols}):
        group = [s for s in symbols if len(s) == length]
        lines.append(f"    if n == {length} {{")
        emit(group, 0, "        ")
        lines.append("    }")
    lines.append("    return false;")
    lines.append("}")
    return "\n".join(lines) + "\n"


def gen_vow_ir_block(ops: list[dict], routes: list[str]) -> str:
    return _wrap_marker_block(_vow_symbol_set_fn("fresh_arena_base_extern", routes))


def gen_vow_vc_ops_block(ops: list[dict], routes: list[str]) -> str:
    """The native verifier's builtin model, keyed by runtime symbol (the gate
    sees extern calls, not Vow builtin names). Only `known` needs a lookup: an
    `unmodeled` entry and an absent symbol both skip the function, and
    `check_verifier_models` already forces every entry to say which it is.
    `known` is necessary but not sufficient to verify a call: until the
    symbolic executor encodes calls, the gate still skips a `known` builtin as
    `unsupported-opcode`."""
    known = [op["runtime_symbol"] for op in ops if op.get("verifier_model") == "known"]
    return _wrap_marker_block(_vow_symbol_set_fn("catalogue_verifier_known", known))


def gen_vow_lower_block(ops: list[dict], routes: list[str]) -> str:
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
    tag_arms = "\n".join(
        f'    if name == String::from("{op["name"]}") {{ return BRT_STRING(); }}'
        for op in ops
        if op.get("arena_routing") == "heap_fresh"
    )
    tag_arms_block = f"{tag_arms}\n" if tag_arms else ""
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
        "\n"
        "fn catalogue_builtin_result_tag(name: String) -> i64 {\n"
        f"{tag_arms_block}"
        "    return BRT_NONE();\n"
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


def check_verifier_models(ops: list[dict]) -> list[str]:
    """Drift check: every catalogued builtin must say whether the native
    verifier models it. A builtin added without a `verifier_model` would
    otherwise be indistinguishable from one nobody has decided on."""
    return [
        f"docs/spec/operations.json: '{op['name']}' has no verifier_model "
        "(set 'known' or an explicit 'unmodeled')"
        for op in ops
        if "verifier_model" not in op
    ]


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


# Every generator takes (ops, routes): the catalogue's operations and the
# runtime symbols of its arena routes.
TARGET_FILES = [
    (Path("vow-ir/src/lower/mod.rs"), gen_rust_ir_block),
    (Path("vow-codegen/src/cranelift_backend.rs"), gen_cranelift_block),
    (Path("vow-clif-shim/src/lib.rs"), gen_shim_block),
    (Path("compiler/lower.vow"), gen_vow_lower_block),
    (Path("vow-ir/src/region.rs"), gen_region_block),
    (Path("compiler/ir.vow"), gen_vow_ir_block),
    (Path("compiler/vc_ops.vow"), gen_vow_vc_ops_block),
]


def write_projections(ops: list[dict], repo_root: Path, routes: list[str]) -> None:
    for rel_path, gen_fn in TARGET_FILES:
        path = repo_root / rel_path
        content = path.read_text()
        new_content = _replace_between_markers(
            content, MARKER_START, MARKER_END, gen_fn(ops, routes)
        )
        path.write_text(new_content)


def check_projections(ops: list[dict], repo_root: Path, routes: list[str]) -> list[str]:
    """Regenerate each target's block in-memory and diff it against what is
    currently spliced into the checked-in file. Raises if a target is
    missing its marker pair (never silently reports clean)."""
    mismatches: list[str] = []
    for rel_path, gen_fn in TARGET_FILES:
        path = repo_root / rel_path
        content = path.read_text()
        expected = _replace_between_markers(
            content, MARKER_START, MARKER_END, gen_fn(ops, routes)
        )
        if expected != content:
            mismatches.append(f"{rel_path}: spliced block is stale")
    return mismatches


def main() -> None:
    argv = sys.argv[1:]
    check_only = "--check" in argv
    repo_root = REPO
    if "--repo-root" in argv:
        repo_root = Path(argv[argv.index("--repo-root") + 1])

    try:
        ops = load_catalogue(repo_root)
        routes = load_arena_routes(repo_root, ops)
    except (ValueError, json.JSONDecodeError) as e:
        print(str(e), file=sys.stderr)
        sys.exit(1)

    undecided = check_verifier_models(ops)
    if undecided:
        for m in undecided:
            print(m, file=sys.stderr)
        sys.exit(1)

    if check_only:
        mismatches = (
            check_projections(ops, repo_root, routes)
            + check_doc_facts(ops, repo_root)
            + check_runtime_exports(routes, repo_root)
        )
        if mismatches:
            for m in mismatches:
                print(m)
            print("\nRun scripts/generate_operations.py to regenerate.")
            sys.exit(1)
        print("OK: operations catalogue up to date")
        return

    write_projections(ops, repo_root, routes)
    for rel_path, _ in TARGET_FILES:
        print(f"Updated {rel_path}")


if __name__ == "__main__":
    main()
