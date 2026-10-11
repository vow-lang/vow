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
    FILES=(span diag perfetto token lexer ast parser types env checker ir module_io ir_printer contract_text lower region frontend clif verifier_ids verify_report verifier_harness const_fold ir_dominance vc_ops vc_bvfold vc_term vc_smt vc_int vc_loops vc_cfg vc_flow vc_agg vc_gate vc_inline vc_exec vc_slice vc_unroll vc_induct c_emitter verifier vc_solver vc_fmt vc_cex vc_clause vc_native vc_worker mutants_oracle mutants_patch mutants_sites mutants_defaults mutants_main complexity complexity_graph complexity_main runner_plan cli_flags replay_main decl_text decl_main main)
fi

for f in "${FILES[@]}"; do
    strip_header "$DIR/$f.vow"
done
