#!/usr/bin/env bash
set -euo pipefail

printf '%s\n' "$*" >>"$VOW_BOOTSTRAP_TEST_LOG"

output=""
while [ "$#" -gt 0 ]; do
    if [ "$1" = -o ]; then
        shift
        output="$1"
        break
    fi
    shift
done

[ -n "$output" ] || {
    echo "fake compiler: missing -o output" >&2
    exit 1
}

cp "$0" "$output"
