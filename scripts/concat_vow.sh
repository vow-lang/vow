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
    FILES=(span diag token lexer ast parser types env checker ir module_io ir_printer lower region frontend mutants_oracle mutants_patch mutants_sites mutants_main main)
elif [[ "$MODE" == "clif" ]]; then
    FILES=(span diag token lexer ast parser types env checker ir module_io ir_printer lower region frontend clif c_emitter verifier mutants_oracle mutants_patch mutants_sites mutants_main main)
fi

for f in "${FILES[@]}"; do
    strip_header "$DIR/$f.vow"
done
