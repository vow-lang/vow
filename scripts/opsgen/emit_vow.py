"""Self-hosted (Vow) generated blocks: lowering, IR, verifier, C model."""

from __future__ import annotations

from opsgen.catalogue import c_model_symbols, result_shape
from opsgen.markers import _wrap_marker_block
from opsgen.tokens import OPTION_ELEMS, RETURN_TOKENS


def _vow_result_tag(shape: tuple[str, str | None]) -> str:
    kind, elem = shape
    if kind == "string":
        return "BRT_STRING()"
    if kind == "vec":
        return "BRT_VEC()"
    return OPTION_ELEMS[elem]["ity_const"]


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
