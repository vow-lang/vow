#!/usr/bin/env bash
# Regression tests for the chess example's public UCI self-test commands.
# Override the compiler with VOWC_BIN=/path/to/vowc (defaults to ./build/vowc).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

VOWC="${VOWC_BIN:-$ROOT/build/vowc}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() {
    echo "FAIL: $1" >&2
    exit 1
}

if [ ! -x "$VOWC" ]; then
    fail "vowc binary not found or not executable: $VOWC"
fi

if ! (ulimit -v 2000000; "$VOWC" build --no-verify examples/chess/main.vow -o "$TMP/chess") \
    >"$TMP/build.out" 2>"$TMP/build.err"; then
    tail -20 "$TMP/build.out" >&2
    tail -20 "$TMP/build.err" >&2
    fail "could not build examples/chess/main.vow"
fi

if [ ! -x "$TMP/chess" ]; then
    fail "chess build did not produce an executable"
fi

printf '%s\n' \
    'position startpos' \
    'perft 3' \
    'position fen r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1' \
    'perft 3' \
    'position fen 8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1' \
    'perft 3' \
    'position fen rnbq1k1r/pp1Pbppp/2p2n2/8/2B5/8/PPP1NPPP/RNBQK2R b KQ - 1 8' \
    'perft 3' \
    'position startpos' \
    'captest 3' \
    'quit' >"$TMP/commands"

if ! (ulimit -v 2000000; "$TMP/chess" <"$TMP/commands") \
    >"$TMP/chess.out" 2>"$TMP/chess.err"; then
    tail -20 "$TMP/chess.err" >&2
    fail "chess self-test process exited non-zero"
fi

actual=$(grep -E '^(Nodes searched|captest mismatches): ' "$TMP/chess.out" || true)
expected=$'Nodes searched: 8902\nNodes searched: 97862\nNodes searched: 2812\nNodes searched: 39764\ncaptest mismatches: 0'

if [ "$actual" != "$expected" ]; then
    printf 'expected summaries:\n%s\nactual summaries:\n%s\n' "$expected" "$actual" >&2
    fail "chess perft/captest regression"
fi

echo "chess self-tests passed"
