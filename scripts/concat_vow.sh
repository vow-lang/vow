#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-}"
DIR="$(cd "$(dirname "$0")/.." && pwd)/compiler"

if [[ "$MODE" != "ir" && "$MODE" != "cgen" ]]; then
    echo "Usage: $0 {ir|cgen}" >&2
    exit 1
fi

strip_header() {
    sed '/^module /d; /^use /d; /^$/d' "$1"
    echo
}

echo "module Compiler"
echo

if [[ "$MODE" == "ir" ]]; then
    FILES=(span token lexer ast parser types env checker ir ir_printer lower cgen main)
elif [[ "$MODE" == "cgen" ]]; then
    FILES=(span token lexer ast parser types env checker ir ir_printer lower cgen main)
fi

for f in "${FILES[@]}"; do
    strip_header "$DIR/$f.vow"
done
