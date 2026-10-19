#!/usr/bin/env python3
"""Generate Operation Catalogue projections for both compilers.

The generator lives in the scripts/opsgen/ package; this file is the entry
point and re-exports the names the tests import.

Reads docs/spec/operations.json (the Operation Catalogue: the checked,
hand-edited source of runtime-symbol/ABI/return-shape/doc facts for a set of
Builtin Operations) and splices generated lookup functions into:
  - vow-ir/src/lower/mod.rs           (catalogue_builtin_to_runtime)
  - vow-codegen/src/cranelift_backend.rs (catalogue_extern_sig)
  - vow-clif-shim/src/lib.rs             (catalogue_extern_sig, FRESH_ARENA_VARIANTS)
  - compiler/lower.vow                (catalogue_builtin_to_extern, catalogue_builtin_ret_ty)
  - vow-ir/src/region.rs              (FRESH_ARENA_VARIANTS)
  - vow-verify/src/c_emitter.rs       (catalogue_c_model_known)
  - compiler/c_emitter.vow            (catalogue_c_model_known)
between `// GENERATE:OPERATIONS:START` / `// GENERATE:OPERATIONS:END` markers, and writes
whole generated modules (complete files, exempt from the authored-file budget):
  - compiler/catalogue_ir_gen.vow     (fresh_arena_base_extern)
  - compiler/catalogue_vc_ops_gen.vow (catalogue_verifier_known)

It also splices test-only fact tables between `// GENERATE:OPERATION_FACTS:START` /
`:END` markers (FACT_TARGET_FILES): every op's name, effects and ABI tokens plus the
catalogue's `excluded` section, which the type-checker tests read to tie
docs/spec/operations.json to both builtin signature tables.
  - vow-types/src/env.rs              (CATALOGUE_FACTS, EXCLUDED_FACTS; #[cfg(test)])
  - vow-ir/src/lower/mod.rs           (LOWERING_FACTS; #[cfg(test)])
  - compiler/tests/catalogue_facts_gen.vow (op_facts_all, op_facts_excluded; a whole file)

Usage:
    python3 scripts/generate_operations.py          # from repo root
    python3 scripts/generate_operations.py --check  # validate without writing
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import spec_assemble
from opsgen.config import REPO
from opsgen.arena import check_arena_routing, check_runtime_exports, load_arena_routes
from opsgen.catalogue import c_model_symbols, load_catalogue, result_shape
from opsgen.checks import (
    _split_table_row,
    check_c_models,
    check_doc_facts,
    check_verifier_models,
    extract_builtin_signatures_table,
)
from opsgen.emit_rust import (
    gen_cranelift_block,
    gen_region_block,
    gen_rust_c_model_block,
    gen_rust_ir_block,
    gen_shim_block,
)
from opsgen.emit_vow import (
    _vow_symbol_set_fn,
    gen_vow_c_model_block,
    gen_vow_lower_block,
    gen_vow_vc_ops_block,
)
from opsgen.excluded import excluded_matches, load_excluded
from opsgen.facts import (
    FACTS_MARKER_END,
    FACTS_MARKER_START,
    FACT_TARGET_FILES,
    FACT_WHOLE_TARGETS,
    _doc_params,
    check_fact_tables,
    gen_rust_facts_block,
    gen_rust_lowering_facts_block,
    gen_vow_facts_block,
    result_tag_token,
    write_fact_tables,
)
from opsgen.markers import MARKER_END, MARKER_START, _replace_between_markers
from opsgen.projections import (
    TARGET_FILES,
    WHOLE_FILE_TARGETS,
    check_projections,
    write_projections,
)
from opsgen.tokens import (
    OPTION_ELEMS,
    PARAM_TOKENS,
    RETURN_TOKENS,
    _doc_type_to_token,
    arena_variant,
)

__all__ = [
    "FACTS_MARKER_END",
    "FACTS_MARKER_START",
    "FACT_TARGET_FILES",
    "FACT_WHOLE_TARGETS",
    "MARKER_END",
    "MARKER_START",
    "OPTION_ELEMS",
    "PARAM_TOKENS",
    "RETURN_TOKENS",
    "TARGET_FILES",
    "WHOLE_FILE_TARGETS",
    "_doc_params",
    "_doc_type_to_token",
    "_replace_between_markers",
    "_split_table_row",
    "_vow_symbol_set_fn",
    "arena_variant",
    "c_model_symbols",
    "check_arena_routing",
    "check_c_models",
    "check_doc_facts",
    "check_fact_tables",
    "check_projections",
    "check_runtime_exports",
    "check_verifier_models",
    "excluded_matches",
    "extract_builtin_signatures_table",
    "gen_cranelift_block",
    "gen_region_block",
    "gen_rust_c_model_block",
    "gen_rust_facts_block",
    "gen_rust_ir_block",
    "gen_rust_lowering_facts_block",
    "gen_shim_block",
    "gen_vow_c_model_block",
    "gen_vow_facts_block",
    "gen_vow_lower_block",
    "gen_vow_vc_ops_block",
    "generated_output_paths",
    "load_arena_routes",
    "load_catalogue",
    "load_excluded",
    "result_shape",
    "result_tag_token",
    "write_fact_tables",
    "write_projections",
]


def generated_output_paths() -> set[str]:
    """Repo-relative paths of the whole files this generator writes."""
    targets = WHOLE_FILE_TARGETS + FACT_WHOLE_TARGETS
    return {path.as_posix() for path, _, _ in targets}


def main() -> None:
    argv = sys.argv[1:]
    check_only = "--check" in argv
    repo_root = REPO
    if "--repo-root" in argv:
        repo_root = Path(argv[argv.index("--repo-root") + 1])

    # operations.json is assembled from docs/spec/src/operations.json/*.part.
    spec_dir = repo_root / "docs" / "spec"
    if check_only:
        stale = spec_assemble.check(spec_dir)
        if stale:
            print("\n".join(stale), file=sys.stderr)
            sys.exit(1)
    else:
        spec_assemble.write_all(spec_dir)

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
    for rel_path, _, _ in WHOLE_FILE_TARGETS + FACT_WHOLE_TARGETS:
        print(f"Updated {rel_path}")


if __name__ == "__main__":
    main()
