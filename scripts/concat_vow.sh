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
    FILES=(span diag perfetto token lexer ast parser types env checker ir module_io ir_printer contract_text lower region frontend mutants_oracle mutants_patch mutants_sites mutants_defaults mutants_main complexity complexity_graph complexity_main runner_plan cli_flags replay_main decl_text decl_main main)
elif [[ "$MODE" == "clif" ]]; then
    FILES=(span diag perfetto token lexer ast parser types env checker ir module_io ir_printer contract_text lower region frontend clif verifier_ids verify_report verifier_harness const_fold ir_dominance vc_strops vc_ops vc_bvfold vc_term vc_fp vc_qbind vc_smt vc_int vc_loops vc_cfg vc_flow vc_wide vc_agg_shape vc_agg vc_vec vc_map vc_mapops vc_strdeep vc_str vc_strsearch vc_hooks vc_numeric vc_strfold vc_narrow vc_heap vc_overlay vc_gate vc_linear vc_inline vc_query vc_exec vc_slice vc_unroll vc_induct c_emitter verifier vc_solver vc_cache vc_session vc_fmt vc_vecmodel vc_cex vc_clause vc_native vc_worker vc_queue vc_pool vc_probe vc_task vc_driver mutants_oracle mutants_patch mutants_sites mutants_defaults mutants_main complexity complexity_graph complexity_main runner_plan cli_flags replay_main decl_text decl_main main)
fi

for f in "${FILES[@]}"; do
    strip_header "$DIR/$f.vow"
done
