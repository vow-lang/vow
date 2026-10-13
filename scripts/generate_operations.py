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
  - vow-verify/src/c_emitter.rs       (catalogue_c_model_known)
  - compiler/c_emitter.vow            (catalogue_c_model_known)
between `// GENERATE:OPERATIONS:START` / `// GENERATE:OPERATIONS:END` markers.

It also splices test-only fact tables between `// GENERATE:OPERATION_FACTS:START` /
`:END` markers (FACT_TARGET_FILES): every op's name, effects and ABI tokens plus the
catalogue's `excluded` section, which the type-checker tests read to tie
docs/spec/operations.json to both builtin signature tables.
  - vow-types/src/env.rs              (CATALOGUE_FACTS, EXCLUDED_FACTS; #[cfg(test)])
  - vow-ir/src/lower/mod.rs           (LOWERING_FACTS; #[cfg(test)])
  - compiler/tests/catalogue_facts.vow (op_facts_all, op_facts_excluded)

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
    "i128": "types::I128",
    "u128": "types::I128",
}

# Element types an `Option<T>` result may carry. A fresh Option cell is a heap
# aggregate whose payload type the lowerer must remember (see
# `result_shape`), so each element needs both spellings.
OPTION_ELEMS = {
    "i8": {"rust_ty": "Ty::I8", "ity_const": "ITY_I8()"},
    "u8": {"rust_ty": "Ty::U8", "ity_const": "ITY_U8()"},
    "i16": {"rust_ty": "Ty::I16", "ity_const": "ITY_I16()"},
    "u16": {"rust_ty": "Ty::U16", "ity_const": "ITY_U16()"},
    "i32": {"rust_ty": "Ty::I32", "ity_const": "ITY_I32()"},
    "u32": {"rust_ty": "Ty::U32", "ity_const": "ITY_U32()"},
    "i64": {"rust_ty": "Ty::I64", "ity_const": "ITY_I64()"},
    "i128": {"rust_ty": "Ty::I128", "ity_const": "ITY_I128()"},
    "u128": {"rust_ty": "Ty::U128", "ity_const": "ITY_U128()"},
}

# Effect keyword spellings, sourced from vow-syntax/src/token.rs's effect
# keyword lexing -- the same spellings the printer emits and grammar.md's
# "Builtin Function Signatures" table documents.
EFFECT_TOKENS = {"read", "write", "io", "panic", "unsafe"}

EFFECTS_RE = re.compile(r"^\[(?P<tokens>[a-z]+(?:, [a-z]+)*)?\]$")

DOC_SIGNATURE_RE = re.compile(r"^fn\((?P<params>.*)\) -> (?P<ret>.+)$")
VEC_DOC_RE = re.compile(r"^Vec<.*>$")
OPTION_DOC_RE = re.compile(r"^Option<(?P<elem>.+)>$")


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
    token. `String`, any `Vec<...>` and any `Option<...>` all denote a heap
    pointer, so all map to `ptr` -- this is a many-to-one mapping, not a
    bijection."""
    if (
        type_str == "String"
        or VEC_DOC_RE.match(type_str)
        or OPTION_DOC_RE.match(type_str)
    ):
        return "ptr"
    if type_str in ("i64", "u64", "i128", "u128", "bool"):
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
# "arena_routing: heap_fresh" means the op returns a fresh heap aggregate whose
# runtime symbol has an `_in_arena` variant (the `arena_routes` section lists
# it); "none" means it has no arena variant. An op whose result is a String, a
# Vec or an Option must declare one of the two; a scalar or unit op may omit
# the field. `check_arena_routing` enforces that `heap_fresh` holds exactly when
# the op's symbol is a route, and that a heap-returning `none` carries a
# reason. The shape tag the lowerer attaches to a result is a separate fact,
# derived from the doc_signature return type (`result_shape`).
#
# "verifier_model" is the native verifier's category (`known` means its
# symbolic executor models the call). "c_model" is the ESBMC C model's: `base`
# means the model knows the runtime symbol, `base_and_arena` also its
# `_in_arena` variant, `unmodeled` neither. The two differ on purpose: a
# `known` native entry changes the native skip code of every caller, while the
# C model is the stage 0 verifier's and goes away with it (epic #1398, P6).
OPTIONAL_FIELD_VALUES = {
    "verifier_model": {"known", "unmodeled"},
    "arena_routing": {"none", "heap_fresh"},
    "c_model": {"unmodeled", "base", "base_and_arena"},
}

# "arena_routing_reason" is free text, so it sits outside the closed
# vocabularies above: it says why an op that returns a heap value has no arena
# variant, and `check_arena_routing` requires it exactly on those ops.
ARENA_REASON_FIELD = "arena_routing_reason"

ALLOWED_FIELDS = (
    set(REQUIRED_FIELDS) | set(OPTIONAL_FIELD_VALUES) | {ARENA_REASON_FIELD}
)


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
                option = OPTION_DOC_RE.match(sig_match.group("ret").strip())
                if option and option.group("elem") not in OPTION_ELEMS:
                    errors.append(
                        f"operation '{name}' has Option element type "
                        f"'{option.group('elem')}' with no result tag "
                        f"(known: {sorted(OPTION_ELEMS)})"
                    )

    if errors:
        raise ValueError("\n".join(errors))

    return ops


def result_shape(op: dict) -> tuple[str, str | None] | None:
    """The shape tag the lowerer attaches to the op's result, read from the
    doc_signature return type: ("string", None), ("vec", None) or
    ("option", <element type>); None for a scalar or unit result. The tag is
    what `pin_to_root` and the arena inference use to know a direct builtin
    result is a fresh heap aggregate."""
    ret = DOC_SIGNATURE_RE.match(op["doc_signature"]).group("ret").strip()
    if ret == "String":
        return ("string", None)
    if VEC_DOC_RE.match(ret):
        return ("vec", None)
    option = OPTION_DOC_RE.match(ret)
    if option:
        return ("option", option.group("elem"))
    return None


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

    if errors:
        raise ValueError("\n".join(errors))
    return routes


def check_arena_routing(ops: list[dict], routes: list[str]) -> list[str]:
    """Drift check for the arena-routing category: a builtin with a String, Vec
    or Option result must say whether it has an arena route, and the category
    must agree with `arena_routes` in both directions (`heap_fresh` exactly when
    the runtime symbol is a route, `none` never with one). A heap-returning
    `none` must say why in `arena_routing_reason`, and only a `none` may. Routes
    no catalogued op uses, such as `map_get` or the `*_try` casts, stay legal."""
    problems: list[str] = []
    for op in ops:
        where = f"docs/spec/operations.json: '{op['name']}'"
        symbol = op["runtime_symbol"]
        routing = op.get("arena_routing")
        routed = symbol in routes
        heap_result = result_shape(op) is not None
        if routing is None:
            if heap_result:
                problems.append(
                    f"{where} returns a heap value but has no arena_routing (set "
                    "'heap_fresh' with an arena route, or an explicit 'none')"
                )
            elif routed:
                problems.append(
                    f"{where} has an arena route for '{symbol}' but is not heap_fresh"
                )
        elif routing == "heap_fresh":
            if not heap_result:
                problems.append(
                    f"{where} is heap_fresh but its return type is not a String, "
                    "Vec or Option"
                )
            if not routed:
                problems.append(
                    f"{where} is heap_fresh but its runtime_symbol '{symbol}' has "
                    "no arena route"
                )
        elif routed:
            problems.append(
                f"{where} is arena_routing 'none' but its runtime_symbol "
                f"'{symbol}' has an arena route"
            )
        problems += _check_arena_reason(op, where, routing, heap_result)
    return problems


def _check_arena_reason(
    op: dict, where: str, routing: str | None, heap_result: bool
) -> list[str]:
    reason = op.get(ARENA_REASON_FIELD)
    if reason is None:
        if routing == "none" and heap_result:
            return [
                f"{where} is arena_routing 'none' but has no {ARENA_REASON_FIELD} "
                "saying why a heap result has no arena variant"
            ]
        return []
    if not isinstance(reason, str) or not reason.strip():
        return [f"{where} has an empty or non-string {ARENA_REASON_FIELD}"]
    if routing != "none":
        return [f"{where} has an {ARENA_REASON_FIELD} but is not arena_routing 'none'"]
    return []


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


def _rust_result_tag(shape: tuple[str, str | None]) -> str:
    kind, elem = shape
    if kind == "string":
        return "BuiltinResultTag::StringHeap"
    if kind == "vec":
        return "BuiltinResultTag::VecHeap"
    return f"BuiltinResultTag::OptionOf({OPTION_ELEMS[elem]['rust_ty']})"


def _vow_result_tag(shape: tuple[str, str | None]) -> str:
    kind, elem = shape
    if kind == "string":
        return "BRT_STRING()"
    if kind == "vec":
        return "BRT_VEC()"
    return OPTION_ELEMS[elem]["ity_const"]


def gen_rust_ir_block(ops: list[dict], routes: list[str]) -> str:
    arms = "\n".join(
        f'        "{op["name"]}" => Some(("{op["runtime_symbol"]}", '
        f"{RETURN_TOKENS[op['return']]['rust_ty']})),"
        for op in ops
    )
    tag_arms = "\n".join(
        f'        "{op["name"]}" => Some({_rust_result_tag(shape)}),'
        for op in ops
        if (shape := result_shape(op)) is not None
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
    `known` is necessary but not sufficient to verify a call: the executor
    encodes a call by its runtime symbol (`vc_known_call_kind` in `vc_ops.vow`),
    and the gate skips a `known` symbol without a kind as `unsupported-opcode`."""
    known = [op["runtime_symbol"] for op in ops if op.get("verifier_model") == "known"]
    return _wrap_marker_block(_vow_symbol_set_fn("catalogue_verifier_known", known))


def c_model_symbols(ops: list[dict]) -> list[str]:
    """Every runtime symbol the ESBMC C model knows: the base symbol of a
    `base`/`base_and_arena` entry, and the `_in_arena` variant of a
    `base_and_arena` one."""
    symbols: list[str] = []
    for op in ops:
        model = op.get("c_model", "unmodeled")
        if model == "unmodeled":
            continue
        symbols.append(op["runtime_symbol"])
        if model == "base_and_arena":
            symbols.append(arena_variant(op["runtime_symbol"]))
    return symbols


def gen_rust_c_model_block(ops: list[dict], routes: list[str]) -> str:
    """`catalogue_c_model_known`, in the shape rustfmt gives a `matches!` over
    string literals, so the splice survives `cargo fmt --check`."""
    symbols = c_model_symbols(ops)
    if len(symbols) == 1:
        body = f'    matches!(name, "{symbols[0]}")\n'
    else:
        alternatives = "\n            | ".join(f'"{s}"' for s in symbols)
        body = f"    matches!(\n        name,\n        {alternatives}\n    )\n"
    return _wrap_marker_block(
        f"fn catalogue_c_model_known(name: &str) -> bool {{\n{body}}}\n"
    )


def gen_vow_c_model_block(ops: list[dict], routes: list[str]) -> str:
    return _wrap_marker_block(
        _vow_symbol_set_fn("catalogue_c_model_known", c_model_symbols(ops))
    )


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
        f'    if name == String::from("{op["name"]}") '
        f"{{ return {_vow_result_tag(shape)}; }}"
        for op in ops
        if (shape := result_shape(op)) is not None
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
    verifier models it, and an effectful one cannot be `known`. A builtin
    added without a `verifier_model` would otherwise be indistinguishable
    from one nobody has decided on. Verifier-model categories apply only to
    pure operations (PRD #375), so an effectful `known` entry is rejected
    even though the native gate admits callers whose only effect is `panic`;
    modelling a `[panic]` builtin would be a deliberate change to this rule."""
    problems: list[str] = []
    for op in ops:
        name = op["name"]
        model = op.get("verifier_model")
        if model is None:
            problems.append(
                f"docs/spec/operations.json: '{name}' has no verifier_model "
                "(set 'known' or an explicit 'unmodeled')"
            )
        elif model == "known" and op["effects"] != "[]":
            problems.append(
                f"docs/spec/operations.json: '{name}' has effects {op['effects']} "
                f"but verifier_model '{model}'; the native verifier only models "
                "pure builtins"
            )
    return problems


def check_c_models(ops: list[dict], routes: list[str]) -> list[str]:
    """Drift check for the ESBMC C-model category: every pure builtin must say
    whether the model knows it, an effectful one cannot be known (the model's
    gate takes only effect-free functions), and an `_in_arena` variant must be
    a real arena route."""
    problems: list[str] = []
    for op in ops:
        name = op["name"]
        model = op.get("c_model")
        if op["effects"] == "[]" and model is None:
            problems.append(
                f"docs/spec/operations.json: pure builtin '{name}' has no c_model "
                "(set 'base', 'base_and_arena' or an explicit 'unmodeled')"
            )
        if model in ("base", "base_and_arena") and op["effects"] != "[]":
            problems.append(
                f"docs/spec/operations.json: '{name}' has effects {op['effects']} "
                f"but c_model '{model}'; the C model only knows pure builtins"
            )
        if model == "base_and_arena" and op["runtime_symbol"] not in routes:
            problems.append(
                f"docs/spec/operations.json: '{name}' has c_model 'base_and_arena' "
                f"but '{op['runtime_symbol']}' has no arena route"
            )
    return problems


EXCLUDED_NAME_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
EXCLUDED_FRAGMENT_RE = re.compile(r"^[A-Za-z0-9_]+$")
EXCLUDED_MATCHERS = ("names", "prefix", "contains")
EXCLUDED_FIELDS = {"reason", "suffixes", *EXCLUDED_MATCHERS}


def excluded_label(index: int, entry: dict) -> str:
    """How a violation names an excluded entry: its position and its matcher."""
    for key in EXCLUDED_MATCHERS:
        if key in entry:
            return f"excluded entry #{index} ({key} {json.dumps(entry[key])})"
    return f"excluded entry #{index}"


def excluded_matches(entry: dict, name: str) -> bool:
    """Whether a validated `excluded` entry covers a builtin name. The matcher
    vocabulary is deliberately closed (explicit names, a prefix, or a substring
    plus a suffix set; no regex) so the Rust and Vow tests evaluate it
    identically."""
    if "names" in entry:
        return name in entry["names"]
    if "prefix" in entry:
        return name.startswith(entry["prefix"])
    return entry["contains"] in name and name.endswith(tuple(entry["suffixes"]))


def _check_excluded_matcher(label: str, entry: dict, errors: list[str]) -> None:
    present = [k for k in EXCLUDED_MATCHERS if k in entry]
    if len(present) != 1:
        errors.append(
            f"{label} must have exactly one matcher of {list(EXCLUDED_MATCHERS)}, "
            f"has {present}"
        )
        return
    kind = present[0]
    if kind != "contains" and "suffixes" in entry:
        errors.append(f"{label} has 'suffixes' but its matcher is '{kind}'")
    value = entry[kind]
    if kind == "names":
        if not isinstance(value, list) or not value:
            errors.append(f"{label} must list at least one name")
            return
        for name in value:
            if not isinstance(name, str) or not EXCLUDED_NAME_RE.match(name):
                errors.append(f"{label} has malformed name {name!r}")
        if len(set(map(str, value))) != len(value):
            errors.append(f"{label} lists a name more than once")
        return
    if not isinstance(value, str) or not EXCLUDED_FRAGMENT_RE.match(value):
        errors.append(f"{label} has malformed {kind} {value!r}")
    if kind == "contains":
        suffixes = entry.get("suffixes")
        if not isinstance(suffixes, list) or not suffixes:
            errors.append(f"{label} needs a non-empty 'suffixes' list with 'contains'")
        else:
            for suffix in suffixes:
                if not isinstance(suffix, str) or not EXCLUDED_FRAGMENT_RE.match(
                    suffix
                ):
                    errors.append(f"{label} has malformed suffix {suffix!r}")


def load_excluded(repo_root: Path, ops: list[dict]) -> list[dict]:
    """Read and validate the catalogue's `excluded` section: the builtin names
    the type-checker signature tables hold that the catalogue deliberately does
    not list, each with the reason. A catalogue without the section excludes
    nothing.

    Collects every violation before raising a ValueError, like load_catalogue:
    a missing reason, a malformed or ambiguous matcher, a name listed twice, and
    a name that is both catalogued and excluded.
    """
    catalogue_path = repo_root / "docs" / "spec" / "operations.json"
    entries = json.loads(catalogue_path.read_text()).get("excluded", [])
    if not isinstance(entries, list):
        raise ValueError(f"{catalogue_path}: 'excluded' key must be a list")

    errors: list[str] = []
    valid: list[tuple[int, dict]] = []
    seen_names: dict[str, int] = {}
    for index, entry in enumerate(entries):
        if not isinstance(entry, dict):
            errors.append(f"excluded entry #{index} must be an object")
            continue
        label = excluded_label(index, entry)
        before = len(errors)
        unknown = set(entry) - EXCLUDED_FIELDS
        if unknown:
            errors.append(
                f"{label} has unknown field(s) {sorted(unknown)} "
                f"(allowed: {sorted(EXCLUDED_FIELDS)})"
            )
        reason = entry.get("reason")
        if not isinstance(reason, str) or not reason.strip():
            errors.append(f"{label} has no reason")
        _check_excluded_matcher(label, entry, errors)
        if len(errors) != before:
            continue
        valid.append((index, entry))
        for name in entry.get("names", []):
            if name in seen_names:
                errors.append(
                    f"{label} lists '{name}', already excluded by "
                    f"entry #{seen_names[name]}"
                )
            seen_names[name] = index

    for index, entry in valid:
        for op in ops:
            if excluded_matches(entry, op["name"]):
                errors.append(
                    f"operation '{op['name']}' is both catalogued and excluded by "
                    f"{excluded_label(index, entry)}"
                )

    if errors:
        raise ValueError("\n".join(errors))
    return [entry for _, entry in valid]


FACTS_MARKER_START = "// GENERATE:OPERATION_FACTS:START"
FACTS_MARKER_END = "// GENERATE:OPERATION_FACTS:END"


def _doc_return(op: dict) -> str:
    return DOC_SIGNATURE_RE.match(op["doc_signature"]).group("ret").strip()


def _doc_params(op: dict) -> list[str]:
    """The parameter types as the docs spell them (`String`, `Vec<String>`)."""
    params = DOC_SIGNATURE_RE.match(op["doc_signature"]).group("params")
    return [p.partition(": ")[2].strip() for p in _split_top_level_commas(params) if p]


def result_tag_token(op: dict) -> str:
    """The op's result tag as one token for the fact tables: `none`, `string`,
    `vec` or `option:<elem>`, derived from `result_shape` so the tables and the
    generated lowering lookups cannot disagree."""
    shape = result_shape(op)
    if shape is None:
        return "none"
    kind, elem = shape
    return f"option:{elem}" if kind == "option" else kind


def _indent_facts_block(lines: list[str]) -> str:
    """Join a fact-table block, indenting every line but the marker line."""
    return "\n".join(
        line if i == 0 or not line else f"    {line}" for i, line in enumerate(lines)
    )


def _quoted_list(items: list[str]) -> str:
    return ", ".join(f'"{item}"' for item in items)


def gen_rust_facts_block(ops: list[dict], excluded: list[dict]) -> str:
    """`CATALOGUE_FACTS` and `EXCLUDED_FACTS` for vow-types' env.rs test module.
    Marked `#[rustfmt::skip]` so the one-row-per-op layout survives
    `cargo fmt`; every line after the first carries the module's indentation."""
    rows = [
        f'    CatalogueFact {{ name: "{op["name"]}", effects: "{op["effects"]}", '
        f'params: &[{_quoted_list(op["params"])}], ret: "{op["return"]}", '
        f"doc_params: &[{_quoted_list(_doc_params(op))}], "
        f'doc_ret: "{_doc_return(op)}" }},'
        for op in ops
    ]
    exclusions = []
    for entry in excluded:
        if "names" in entry:
            exclusions.append(
                f"    ExcludedFact::Names(&[{_quoted_list(entry['names'])}]),"
            )
        elif "prefix" in entry:
            exclusions.append(f'    ExcludedFact::Prefix("{entry["prefix"]}"),')
        else:
            exclusions.append(
                f'    ExcludedFact::Contains {{ needle: "{entry["contains"]}", '
                f"suffixes: &[{_quoted_list(entry['suffixes'])}] }},"
            )
    lines = (
        [
            FACTS_MARKER_START,
            "#[rustfmt::skip]",
            "const CATALOGUE_FACTS: &[CatalogueFact] = &[",
        ]
        + rows
        + ["];", "", "#[rustfmt::skip]", "const EXCLUDED_FACTS: &[ExcludedFact] = &["]
        + exclusions
        + ["];", FACTS_MARKER_END]
    )
    return _indent_facts_block(lines)


def gen_rust_lowering_facts_block(ops: list[dict], excluded: list[dict]) -> str:
    """`LOWERING_FACTS` for vow-ir's lowering test module: the runtime symbol,
    return token and result tag of every op, the columns the lowering lookups
    are checked against. Same layout rules as `gen_rust_facts_block`."""
    rows = [
        f'    LoweringFact {{ name: "{op["name"]}", symbol: "{op["runtime_symbol"]}", '
        f'ret: "{op["return"]}", tag: "{result_tag_token(op)}" }},'
        for op in ops
    ]
    lines = (
        [
            FACTS_MARKER_START,
            "#[rustfmt::skip]",
            "const LOWERING_FACTS: &[LoweringFact] = &[",
        ]
        + rows
        + ["];", FACTS_MARKER_END]
    )
    return _indent_facts_block(lines)


def gen_vow_facts_block(ops: list[dict], excluded: list[dict]) -> str:
    """`op_facts_all` and `op_facts_excluded` for compiler/tests/catalogue_facts.vow.
    Parameter tokens and exclusion lists are comma-joined strings (doc parameter
    types `|`-joined), since Vow has no slice literals; the readers split them
    back."""
    facts = "\n".join(
        f'    v.push(op_fact(String::from("{op["name"]}"), '
        f'String::from("{op["effects"]}"), '
        f'String::from("{",".join(op["params"])}"), '
        f'String::from("{op["return"]}"), '
        f'String::from("{"|".join(_doc_params(op))}"), '
        f'String::from("{_doc_return(op)}"), '
        f'String::from("{op["runtime_symbol"]}"), '
        f'String::from("{result_tag_token(op)}")));'
        for op in ops
    )
    exclusions = []
    for entry in excluded:
        if "names" in entry:
            args = (
                f'OPX_NAMES(), String::from("{",".join(entry["names"])}"), '
                'String::from("")'
            )
        elif "prefix" in entry:
            args = f'OPX_PREFIX(), String::from("{entry["prefix"]}"), String::from("")'
        else:
            args = (
                f'OPX_CONTAINS(), String::from("{entry["contains"]}"), '
                f'String::from("{",".join(entry["suffixes"])}")'
            )
        exclusions.append(f"    v.push(op_excluded({args}));")
    exclusion_rows = "\n".join(exclusions)
    return (
        f"{FACTS_MARKER_START}\n"
        "fn op_facts_all() -> Vec<OpFact> {\n"
        "    let v: Vec<OpFact> = Vec::new();\n"
        f"{facts}\n"
        "    v\n"
        "}\n"
        "\n"
        "fn op_facts_excluded() -> Vec<OpExcluded> {\n"
        "    let v: Vec<OpExcluded> = Vec::new();\n"
        f"{exclusion_rows}\n"
        "    v\n"
        "}\n"
        f"{FACTS_MARKER_END}"
    )


# Test-only fact tables, spliced between their own marker pair so the mutants
# skip-list (`// GENERATE:<NAME>:START`) covers them. They take (ops, excluded)
# where TARGET_FILES generators take (ops, routes).
FACT_TARGET_FILES = [
    (Path("vow-types/src/env.rs"), gen_rust_facts_block),
    (Path("vow-ir/src/lower/mod.rs"), gen_rust_lowering_facts_block),
    (Path("compiler/tests/catalogue_facts.vow"), gen_vow_facts_block),
]


def write_fact_tables(ops: list[dict], repo_root: Path, excluded: list[dict]) -> None:
    _write_blocks(
        FACT_TARGET_FILES,
        FACTS_MARKER_START,
        FACTS_MARKER_END,
        repo_root,
        ops,
        excluded,
    )


def check_fact_tables(
    ops: list[dict], repo_root: Path, excluded: list[dict]
) -> list[str]:
    """Regenerate each fact table in-memory and diff it against the checked-in
    file; a target missing its marker pair raises rather than reporting clean."""
    return _stale_blocks(
        FACT_TARGET_FILES,
        FACTS_MARKER_START,
        FACTS_MARKER_END,
        repo_root,
        ops,
        excluded,
        "spliced fact table is stale",
    )


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
    (Path("vow-verify/src/c_emitter.rs"), gen_rust_c_model_block),
    (Path("compiler/c_emitter.vow"), gen_vow_c_model_block),
]


def _write_blocks(
    targets: list, start: str, end: str, repo_root: Path, ops: list[dict], extra
) -> None:
    for rel_path, gen_fn in targets:
        path = repo_root / rel_path
        path.write_text(
            _replace_between_markers(path.read_text(), start, end, gen_fn(ops, extra))
        )


def _stale_blocks(
    targets: list,
    start: str,
    end: str,
    repo_root: Path,
    ops: list[dict],
    extra,
    message: str,
) -> list[str]:
    """Regenerate each target's block in-memory and name every file whose
    checked-in block differs. Raises if a target is missing its marker pair
    (never silently reports clean)."""
    mismatches: list[str] = []
    for rel_path, gen_fn in targets:
        content = (repo_root / rel_path).read_text()
        expected = _replace_between_markers(content, start, end, gen_fn(ops, extra))
        if expected != content:
            mismatches.append(f"{rel_path}: {message}")
    return mismatches


def write_projections(ops: list[dict], repo_root: Path, routes: list[str]) -> None:
    _write_blocks(TARGET_FILES, MARKER_START, MARKER_END, repo_root, ops, routes)


def check_projections(ops: list[dict], repo_root: Path, routes: list[str]) -> list[str]:
    return _stale_blocks(
        TARGET_FILES,
        MARKER_START,
        MARKER_END,
        repo_root,
        ops,
        routes,
        "spliced block is stale",
    )


def main() -> None:
    argv = sys.argv[1:]
    check_only = "--check" in argv
    repo_root = REPO
    if "--repo-root" in argv:
        repo_root = Path(argv[argv.index("--repo-root") + 1])

    try:
        ops = load_catalogue(repo_root)
        routes = load_arena_routes(repo_root, ops)
        excluded = load_excluded(repo_root, ops)
    except (ValueError, json.JSONDecodeError) as e:
        print(str(e), file=sys.stderr)
        sys.exit(1)

    undecided = (
        check_verifier_models(ops)
        + check_c_models(ops, routes)
        + check_arena_routing(ops, routes)
    )
    if undecided:
        for m in undecided:
            print(m, file=sys.stderr)
        sys.exit(1)

    if check_only:
        mismatches = (
            check_projections(ops, repo_root, routes)
            + check_fact_tables(ops, repo_root, excluded)
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
    write_fact_tables(ops, repo_root, excluded)
    for rel_path, _ in TARGET_FILES + FACT_TARGET_FILES:
        print(f"Updated {rel_path}")


if __name__ == "__main__":
    main()
