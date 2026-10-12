#!/usr/bin/env bash
set -euo pipefail

printf '%s %s\n' "$0" "$*" >>"$VOW_BOOTSTRAP_TEST_LOG"

output=""
is_verify=false
for arg in "$@"; do
    if [ "$arg" = verify ]; then
        is_verify=true
        break
    fi
    [ "$arg" = build ] && break
done

if [ "$is_verify" = true ]; then
    if [ "${VOW_BOOTSTRAP_TEST_VERIFY_FAIL:-0}" = 1 ]; then
        echo "fake compiler: verification failed" >&2
        exit 1
    fi
    exit 0
fi

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
if [ "${VOW_BOOTSTRAP_TEST_DIVERGE:-0}" = 1 ] && [[ "$output" == *vowc3 ]]; then
    printf '#diverge\n' >>"$output"
fi
