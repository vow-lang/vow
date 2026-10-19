"""Rust-side generated blocks: lowering, Cranelift, shim, region, C model."""

from __future__ import annotations

from opsgen.catalogue import c_model_symbols, result_shape
from opsgen.markers import _wrap_marker_block
from opsgen.tokens import OPTION_ELEMS, PARAM_TOKENS, RETURN_TOKENS, arena_variant


def _rust_result_tag(shape: tuple[str, str | None]) -> str:
    kind, elem = shape
    if kind == "string":
        return "BuiltinResultTag::StringHeap"
    if kind == "vec":
        return "BuiltinResultTag::VecHeap"
    return f"BuiltinResultTag::OptionOf({OPTION_ELEMS[elem]['rust_ty']})"


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
