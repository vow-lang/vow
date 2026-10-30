#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-}"
DIR="$(cd "$(dirname "$0")/.." && pwd)/compiler"

if [[ "$MODE" != "ir" && "$MODE" != "clif" ]]; then
    echo "Usage: $0 {ir|clif}" >&2
    exit 1
fi

strip_header() {
    sed '/^module /d; /^use /d; /^$/d' "$1"
    echo
}

echo "module Compiler"
echo

if [[ "$MODE" == "ir" ]]; then
    FILES=(span diag perfetto token lexer ast parser types env checker ir module_io ir_printer contract_text lower_keys lower_loops lower_branches lower region vc_typekeys frontend mutants_oracle mutants_patch mutants_sites mutants_defaults mutants_main complexity complexity_graph complexity_main runner_plan cli_flags replay_mapkeys replay_ints replay_alias replay_listed replay_free replay_entry replay_main replay_nested replay_elems decl_text decl_main main)
elif [[ "$MODE" == "clif" ]]; then
    FILES=(span diag perfetto token lexer ast parser types env checker ir module_io ir_printer contract_text lower_keys lower_loops lower_branches lower region vc_typekeys frontend clif verifier_ids verify_report verifier_harness const_fold ir_dominance vc_strops vc_ops vc_bvfold vc_term vc_fp vc_qbind vc_smt vc_int vc_loops vc_cfg vc_flow vc_wide vc_agg_shape vc_agg vc_vec vc_map vc_mapdom vc_mapkeys vc_mapops vc_mapgate vc_strdeep vc_str vc_strsearch vc_hooks vc_numeric vc_strfold vc_narrow vc_heap vc_reads vc_elemgate vc_held vc_heaplist vc_objects vc_ptrhandle vc_overlay vc_elemwrite vc_handle vc_wideref vc_ptrstore vc_vecgate vc_mapalias vc_vecalias vc_gate vc_linear vc_sources vc_ptrphi vc_ptrstores vc_ptrfield vc_handle_phis vc_payload vc_inline vc_query vc_model vc_exec vc_slice vc_unroll vc_induct_agg vc_summary_ens vc_summary vc_induct c_emitter verifier vc_solver vc_cache vc_session vc_fmt vc_fieldrows vc_readrows vc_vecmodel vc_typed vc_nested vc_cex vc_clause vc_recheck vc_claim_solver vc_round vc_native vc_worker vc_queue vc_irwire vc_pool vc_probe vc_task vc_driver vc_waves mutants_oracle mutants_patch mutants_sites mutants_defaults mutants_main complexity complexity_graph complexity_main runner_plan cli_flags replay_mapkeys replay_ints replay_alias replay_listed replay_free replay_entry replay_main replay_nested replay_elems vc_cex_elem vc_cex_path vc_cex_reads vc_cex_join vc_cex_verdict decl_text decl_main main)
fi

# Generated modules and the main_* driver modules are appended by glob, so a new
# shard or driver module needs no edit here. main stays last.
EXTRA=()
for f in "$DIR"/*_gen.vow "$DIR"/main_*.vow; do
    [[ -e "$f" ]] || continue
    EXTRA+=("$(basename "$f" .vow)")
done
FILES=("${FILES[@]:0:${#FILES[@]}-1}" "${EXTRA[@]}" main)

for f in "${FILES[@]}"; do
    strip_header "$DIR/$f.vow"
done
