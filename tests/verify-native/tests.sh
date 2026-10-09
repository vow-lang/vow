#!/usr/bin/env bash
set -euo pipefail

# Wiring tests for `vowc verify --backend native` (epic #1398, issue #1408).
# A fake `bitwuzla` first on PATH answers from $FAKE_BW_MODE and records every
# query it is given, so this tier needs no real solver: it checks that one
# self-contained .smt2 reaches the solver per claim, that each solver outcome
# maps to the right verdict, that temp files and child processes are cleaned up,
# and that the CLI rejects what the native backend does not support.

VOWC_BIN="${VOWC_BIN:-build/vowc}"
VOWC_BIN=$(cd "$(dirname "$VOWC_BIN")" && pwd)/$(basename "$VOWC_BIN")
TMP_ROOT=$(mktemp -d)
trap 'rm -rf "$TMP_ROOT"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

FAKE_DIR="$TMP_ROOT/fake"
EMPTY_DIR="$TMP_ROOT/empty"
SCRATCH="$TMP_ROOT/tmp"
mkdir -p "$FAKE_DIR" "$EMPTY_DIR" "$SCRATCH"

cat > "$FAKE_DIR/bitwuzla" <<'SHIM'
#!/usr/bin/env bash
set -eu
file="${!#}"
n=$(find "$FAKE_BW_DIR" -name 'q.*.smt2' | wc -l)
cp "$file" "$FAKE_BW_DIR/q.$((n + 1)).smt2"
echo "$file" >> "$FAKE_BW_DIR/paths"
echo "$*" >> "$FAKE_BW_DIR/args"
case "$FAKE_BW_MODE" in
    unsat) echo unsat ;;
    unknown) echo unknown ;;
    garbage) echo "what is this" ;;
    exit1) echo "[error] boom" >&2; exit 1 ;;
    hang) echo $$ > "$FAKE_BW_DIR/pid"; exec sleep 29 ;;
    sat_empty) printf 'sat\n(\n)\n' ;;
    sat|sat_zero)
        val=7
        [ "$FAKE_BW_MODE" = sat_zero ] && val=0
        echo sat
        echo "("
        grep -o '^(declare-const p[0-9]*' "$file" | sed 's/(declare-const //' | while read -r name; do
            echo "  ($name (_ bv$val 64))"
        done
        echo ")"
        ;;
    *) echo "unknown fake mode" >&2; exit 2 ;;
esac
SHIM
chmod +x "$FAKE_DIR/bitwuzla"

ONE_CLAIM="$TMP_ROOT/one.vow"
cat > "$ONE_CLAIM" <<'SRC'
module One

fn keep(x: i64) -> i64 vow {
  ensures: result == x
} {
  x
}

fn main() -> i32 [io] {
  print_i64(keep(1));
  0
}
SRC

THREE_CLAIMS="$TMP_ROOT/three.vow"
cat > "$THREE_CLAIMS" <<'SRC'
module Three

fn quot(a: i64, b: i64) -> i64 vow {
  ensures: result == result
} {
  a / b
}

fn main() -> i32 [io] {
  print_i64(quot(4, 2));
  0
}
SRC

SKIPPED_ONLY="$TMP_ROOT/skipped.vow"
cat > "$SKIPPED_ONLY" <<'SRC'
module Skipped

fn pick(x: i64) -> i64 vow {
  ensures: result >= 0
} {
  if x > 0 { x } else { 0 }
}

fn main() -> i32 [io] {
  print_i64(pick(1));
  0
}
SRC

failures=0
fail() {
    echo "FAIL: $1" >&2
    failures=$((failures + 1))
}

field() {
    python3 -c 'import json, sys
v = json.loads(sys.argv[1])
for key in sys.argv[2].split("."):
    if isinstance(v, list):
        v = v[int(key)] if len(v) > int(key) else None
    elif isinstance(v, dict):
        v = v.get(key)
    else:
        v = None
print("" if v is None else v)' "$1" "$2"
}

RUN_OUT=""
RUN_RC=0
BW_DIR=""
run_native() {
    local mode="$1"; shift
    BW_DIR=$(mktemp -d "$TMP_ROOT/bw.XXXXXX")
    RUN_RC=0
    RUN_OUT=$(PATH="$FAKE_DIR:$PATH" TMPDIR="$SCRATCH" FAKE_BW_MODE="$mode" FAKE_BW_DIR="$BW_DIR" \
        "$VOWC_BIN" verify --no-cache --backend native "$@" 2>/dev/null) || RUN_RC=$?
}

queries() { find "$BW_DIR" -name 'q.*.smt2' | wc -l | tr -d ' '; }

expect() {
    local what="$1" got="$2" want="$3"
    if [ "$got" != "$want" ]; then
        fail "$what: got '$got', want '$want'"
    fi
}

no_leftovers() {
    if [ -n "$(ls -A "$SCRATCH")" ]; then
        fail "$1: temp files left behind: $(ls -A "$SCRATCH")"
        rm -rf "${SCRATCH:?}"/*
    fi
}

# all claims unsat -> Verified, one self-contained query per claim.
run_native unsat "$THREE_CLAIMS"
expect "unsat status" "$(field "$RUN_OUT" status)" "Verified"
expect "unsat exit" "$RUN_RC" "0"
expect "one query per claim" "$(queries)" "3"
for q in "$BW_DIR"/q.*.smt2; do
    grep -q '^(check-sat)$' "$q" || fail "$q has no check-sat"
    grep -q '^(set-option :produce-models true)$' "$q" || fail "$q lacks produce-models"
done
grep -q -- '--bv-output-format 10' "$BW_DIR/args" || fail "solver not run with --bv-output-format 10"
while read -r p; do
    if [ -e "$p" ]; then fail "query file $p still exists"; fi
done < "$BW_DIR/paths"
no_leftovers "unsat"

# the source path may precede the flag.
run_native unsat "$ONE_CLAIM" --timeout 30
expect "path before flag" "$(field "$RUN_OUT" status)" "Verified"
no_leftovers "path before flag"

# sat + model -> VerifyFailed with the first failing claim; the solver stops there.
run_native sat "$THREE_CLAIMS"
expect "sat status" "$(field "$RUN_OUT" status)" "VerifyFailed"
expect "sat exit" "$RUN_RC" "1"
expect "stops at first failing claim" "$(queries)" "1"
expect "sat function" "$(field "$RUN_OUT" counterexamples.0.function)" "quot"
expect "abort claim vow id" "$(field "$RUN_OUT" counterexamples.0.vow_id)" "4294967293"
expect "abort claim blame" "$(field "$RUN_OUT" counterexamples.0.blame)" "none"
expect "abort claim text" "$(field "$RUN_OUT" counterexamples.0.violation)" "division or remainder by zero"
expect "model value a" "$(field "$RUN_OUT" counterexamples.0.values.a)" "7"
expect "model value b" "$(field "$RUN_OUT" counterexamples.0.values.b)" "7"
no_leftovers "sat"

run_native sat "$ONE_CLAIM"
expect "ensures vow id" "$(field "$RUN_OUT" counterexamples.0.vow_id)" "0"
expect "ensures blame" "$(field "$RUN_OUT" counterexamples.0.blame)" "callee"

# anything that is not a clean verdict is never a proof.
run_native unknown "$ONE_CLAIM"
expect "unknown status" "$(field "$RUN_OUT" status)" "VerifyFailed"
expect "unknown verify_status" "$(field "$RUN_OUT" verify_status)" "unknown"
expect "unknown exit" "$RUN_RC" "1"
for mode in garbage exit1 sat_empty; do
    run_native "$mode" "$ONE_CLAIM"
    expect "$mode verify_status" "$(field "$RUN_OUT" verify_status)" "error"
    expect "$mode exit" "$RUN_RC" "1"
    no_leftovers "$mode"
done

# a hung solver is killed at the budget and reaped.
start=$(date +%s)
run_native hang "$ONE_CLAIM" --timeout 1
elapsed=$(( $(date +%s) - start ))
expect "hang verify_status" "$(field "$RUN_OUT" verify_status)" "timeout"
if [ "$elapsed" -gt 10 ]; then fail "hang took ${elapsed}s with --timeout 1"; fi
hung_pid=$(cat "$BW_DIR/pid" 2>/dev/null || true)
if [ -n "$hung_pid" ]; then
    hung_state=$(ps -p "$hung_pid" -o stat= 2>/dev/null | tr -d ' ' || true)
    case "$hung_state" in
        ""|Z*) ;;
        *)
            fail "hung solver child $hung_pid was not killed"
            kill "$hung_pid" 2>/dev/null || true
            ;;
    esac
else
    fail "fake solver did not record its pid"
fi
no_leftovers "hang"

# --timeout 0 never reaches the solver.
run_native unsat "$ONE_CLAIM" --timeout 0
expect "timeout 0 verify_status" "$(field "$RUN_OUT" verify_status)" "timeout"
expect "timeout 0 spawns nothing" "$(queries)" "0"

# no Bitwuzla on PATH: its own status, no counterexample, ESBMC irrelevant.
RUN_RC=0
RUN_OUT=$(PATH="$EMPTY_DIR" TMPDIR="$SCRATCH" "$VOWC_BIN" verify --no-cache --backend native "$ONE_CLAIM" 2>/dev/null) || RUN_RC=$?
expect "tool_not_found verify_status" "$(field "$RUN_OUT" verify_status)" "tool_not_found"
expect "tool_not_found status" "$(field "$RUN_OUT" status)" "VerifyFailed"
expect "tool_not_found exit" "$RUN_RC" "1"
expect "tool_not_found has no counterexample" "$(field "$RUN_OUT" counterexamples.0.function)" ""

# a module with nothing in the subset needs no solver at all.
RUN_RC=0
RUN_OUT=$(PATH="$EMPTY_DIR" TMPDIR="$SCRATCH" "$VOWC_BIN" verify --no-cache --backend native "$SKIPPED_ONLY" 2>/dev/null) || RUN_RC=$?
expect "all-skipped status" "$(field "$RUN_OUT" status)" "Skipped"
expect "all-skipped needs no solver" "$(field "$RUN_OUT" verify_status)" ""

# a function with no obligation is proven without a solver, so it needs no Bitwuzla.
NO_CLAIMS="$TMP_ROOT/noclaims.vow"
cat > "$NO_CLAIMS" <<'SRC'
module NoClaims

fn positive(x: i64) -> i64 vow {
  requires: x > 0
} {
  x
}
SRC
RUN_RC=0
RUN_OUT=$(PATH="$EMPTY_DIR" TMPDIR="$SCRATCH" "$VOWC_BIN" verify --no-cache --backend native "$NO_CLAIMS" 2>/dev/null) || RUN_RC=$?
expect "no-claims status" "$(field "$RUN_OUT" status)" "Verified"
expect "no-claims exit" "$RUN_RC" "0"
expect "no-claims needs no solver" "$(field "$RUN_OUT" verify_status)" ""

# --replay-cex: a counterexample the runtime reproduces is `confirmed`; one it
# does not is a verifier bug. The fake solver answers b = 0 (a real divide-by-zero)
# or b = 7 (a model the runtime disagrees with). The source defines no `main`:
# the self-hosted replay harness cannot splice one in.
REPLAY_SRC="$TMP_ROOT/replay.vow"
cat > "$REPLAY_SRC" <<'SRC'
module Replay

fn quot(a: i64, b: i64) -> i64 vow {
  ensures: result == result
} {
  a / b
}
SRC
run_native sat_zero "$REPLAY_SRC" --replay-cex
expect "replay confirmed status" "$(field "$RUN_OUT" status)" "VerifyFailed"
expect "replay confirmed exit" "$RUN_RC" "1"
expect "replay confirmed" "$(field "$RUN_OUT" counterexamples.0.replay)" "confirmed"
expect "replay confirmed has no bug report" "$(field "$RUN_OUT" diagnostics.1.error_code)" ""
no_leftovers "replay confirmed"

run_native sat "$REPLAY_SRC" --replay-cex
expect "replay diverged status" "$(field "$RUN_OUT" status)" "VerifyFailed"
expect "replay diverged exit" "$RUN_RC" "1"
expect "replay diverged" "$(field "$RUN_OUT" counterexamples.0.replay)" "diverged"
expect "replay diverged keeps the violation diagnostic first" "$(field "$RUN_OUT" diagnostics.0.error_code)" "VerifierAssertionUnattributed"
expect "replay diverged bug code" "$(field "$RUN_OUT" diagnostics.1.error_code)" "VerifierBug"
no_leftovers "replay diverged"

# a skipped replay never ran, so it is not a verifier bug (THREE_CLAIMS defines main).
run_native sat_zero "$THREE_CLAIMS" --replay-cex
expect "replay skipped" "$(field "$RUN_OUT" counterexamples.0.replay)" "skipped"
expect "skipped replay is not a verifier bug" "$(field "$RUN_OUT" diagnostics.1.error_code)" ""

run_native sat "$REPLAY_SRC"
expect "no replay without the flag" "$(field "$RUN_OUT" counterexamples.0.replay)" ""
expect "no bug report without the flag" "$(field "$RUN_OUT" diagnostics.0.error_code)" "VerifierAssertionUnattributed"

# flag handling.
usage_error() {
    local want="$1"; shift
    local err rc=0
    err=$(PATH="$FAKE_DIR:$PATH" "$VOWC_BIN" "$@" 2>&1 >/dev/null) || rc=$?
    expect "usage exit for: $*" "$rc" "1"
    case "$err" in
        *"$want"*) ;;
        *) fail "usage error for '$*' lacks '$want': $err" ;;
    esac
}
usage_error "--backend must be" verify --backend bogus "$ONE_CLAIM"
usage_error "--backend must be" verify "$ONE_CLAIM" --backend
usage_error "only supported by" build --backend native "$ONE_CLAIM"
usage_error "only supported by" contracts --backend native "$ONE_CLAIM"
usage_error "only supported by" test --backend native "$ONE_CLAIM"
usage_error "only supported by" "$ONE_CLAIM" --backend native
usage_error "--max-k-step" verify --backend native --max-k-step 5 "$ONE_CLAIM"
usage_error "--solver" verify --backend native --solver z3 "$ONE_CLAIM"
usage_error "--encoding" verify --backend native --encoding bv "$ONE_CLAIM"

if [ "$failures" -ne 0 ]; then
    echo "verify-native: $failures failure(s)" >&2
    exit 1
fi
echo "verify-native: ok"
