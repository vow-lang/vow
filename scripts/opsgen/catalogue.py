"""Load and validate docs/spec/operations.json entries."""

from __future__ import annotations

import json
from pathlib import Path

from opsgen.tokens import (
    DOC_SIGNATURE_RE,
    EFFECT_TOKENS,
    EFFECTS_RE,
    OPTION_DOC_RE,
    OPTION_ELEMS,
    PARAM_TOKENS,
    RETURN_TOKENS,
    VEC_DOC_RE,
    _doc_type_to_token,
    _split_top_level_commas,
    arena_variant,
)

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

# "verifier_skip_reason" is free text too: it says which semantics keep a pure
# op out of the native verifier's model, so a caller's `Skipped` reason is
# explained in the table and not only in code. `check_verifier_models` requires
# it exactly on a pure `unmodeled` op (an effectful one is skipped by its
# effects, a `known` one has a model).
VERIFIER_REASON_FIELD = "verifier_skip_reason"

ALLOWED_FIELDS = (
    set(REQUIRED_FIELDS)
    | set(OPTIONAL_FIELD_VALUES)
    | {ARENA_REASON_FIELD, VERIFIER_REASON_FIELD}
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
