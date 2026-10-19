"""Closed vocabularies, doc-type parsing and arena naming for catalogue entries."""

from __future__ import annotations

import re

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


ARENA_SYMBOL_RE = re.compile(r"^__vow_[a-z0-9_]+$")
ARENA_VARIANT_SUFFIX = "_in_arena"


def arena_variant(symbol: str) -> str:
    return f"{symbol}{ARENA_VARIANT_SUFFIX}"
