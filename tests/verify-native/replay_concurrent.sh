#!/usr/bin/env bash
set -uo pipefail

# `--replay-cex` writes its harness next to the source file (#1820). Runs whose sources share a
# directory, a function name and a vow id must not share a harness path: each of the concurrent
# runs below has to be `confirmed`, and none may leave a file behind. Two of the runs replay the
# very same source, which only a per-run token keeps apart.

VOWC_BIN="${VOWC_BIN:-build/vowc}"
VOWC_BIN=$(cd "$(dirname "$VOWC_BIN")" && pwd)/$(basename "$VOWC_BIN")
TMP_ROOT=$(mktemp -d)
trap 'rm -rf "$TMP_ROOT"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

export VOW_VERIFY_NO_INCREMENTAL=1
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FAKE_DIR="$TMP_ROOT/fake"
SRC_DIR="$TMP_ROOT/src"
SCRATCH="$TMP_ROOT/tmp"
mkdir -p "$FAKE_DIR" "$SRC_DIR" "$SCRATCH"
cp "$HERE/fake_bitwuzla.sh" "$FAKE_DIR/bitwuzla"
chmod +x "$FAKE_DIR/bitwuzla"

for name in Alpha Beta; do
    lower=$(printf '%s' "$name" | tr 'A-Z' 'a-z')
    cat > "$SRC_DIR/$lower.vow" <<SRC
module $name

fn quot(a: i64, b: i64) -> i64 vow {
  ensures: result - a / b == 0
} {
  a / b
}
SRC
done

failures=0
fail() {
    echo "FAIL: $1" >&2
    failures=$((failures + 1))
}

replay_status() {
    python3 -c 'import json, sys
try:
    v = json.load(open(sys.argv[1]))
    print(v["counterexamples"][0]["replay"])
except Exception:
    print("none")' "$1"
}

rounds=3
round=0
runs=(alpha beta alpha beta)
while [ "$round" -lt "$rounds" ]; do
    round=$((round + 1))
    idx=0
    pids=()
    for src in "${runs[@]}"; do
        idx=$((idx + 1))
        bw="$TMP_ROOT/bw.$round.$idx"
        mkdir -p "$bw"
        PATH="$FAKE_DIR:$PATH" TMPDIR="$SCRATCH" FAKE_BW_MODE=sat_zero FAKE_BW_DIR="$bw" \
            "$VOWC_BIN" verify --no-cache --backend native --replay-cex "$SRC_DIR/$src.vow" \
            >"$TMP_ROOT/out.$round.$idx" 2>"$TMP_ROOT/err.$round.$idx" &
        pids+=("$!")
    done
    for pid in "${pids[@]}"; do wait "$pid" || true; done
    idx=0
    for src in "${runs[@]}"; do
        idx=$((idx + 1))
        got=$(replay_status "$TMP_ROOT/out.$round.$idx")
        if [ "$got" != "confirmed" ]; then
            fail "round $round run $idx ($src): replay is '$got', want 'confirmed': $(head -c 400 "$TMP_ROOT/err.$round.$idx")"
        fi
    done
done

left=$(ls -A "$SRC_DIR")
if [ "$left" != "$(printf 'alpha.vow\nbeta.vow')" ]; then
    fail "files left in the source directory: $left"
fi
if [ -n "$(ls -A "$SCRATCH")" ]; then
    fail "temp files left behind: $(ls -A "$SCRATCH")"
fi

if [ "$failures" -ne 0 ]; then
    echo "$failures replay concurrency check(s) failed" >&2
    exit 1
fi
echo "replay concurrency: ok"
