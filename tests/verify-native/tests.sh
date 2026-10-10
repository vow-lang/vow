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
n=1
until mkdir "$FAKE_BW_DIR/.slot.$n" 2>/dev/null; do n=$((n + 1)); done
cp "$file" "$FAKE_BW_DIR/q.$n.smt2"
echo "$file" >> "$FAKE_BW_DIR/paths"
echo "$*" >> "$FAKE_BW_DIR/args"
case "$FAKE_BW_MODE" in
    unsat) echo unsat ;;
    unknown) echo unknown ;;
    garbage) echo "what is this" ;;
    exit1) echo "[error] boom" >&2; exit 1 ;;
    hang) echo $$ > "$FAKE_BW_DIR/pid"; exec sleep 29 ;;
    sat_empty) printf 'sat\n(\n)\n' ;;
    oom_msg) echo "(error) Out of memory" >&2; exit 1 ;;
    kill_worker) echo $$ >> "$FAKE_BW_DIR/pids"; kill -9 "$PPID"; sleep 5 ;;
    crash_worker) kill -6 "$PPID"; sleep 5 ;;
    mixed)
        echo $$ >> "$FAKE_BW_DIR/pids"
        h=$(cksum < "$file" | cut -d' ' -f1)
        sleep "0.$((h % 4))"
        if [ $((h % 5)) -eq 0 ]; then echo unknown; else echo unsat; fi
        ;;
    hang_tree)
        echo "$PPID" >> "$FAKE_BW_DIR/worker_pids"
        echo $$ >> "$FAKE_BW_DIR/pids"
        sleep 29 &
        echo $! >> "$FAKE_BW_DIR/grandchild_pids"
        wait
        ;;
    slow_unsat) echo $$ >> "$FAKE_BW_DIR/pids"; sleep 0.3; echo unsat ;;
    sat|sat_zero)
        val=7
        [ "$FAKE_BW_MODE" = sat_zero ] && val=0
        echo sat
        echo "("
        grep -o '^(declare-const p[0-9]* (_ BitVec [0-9]*)' "$file" \
            | sed -E 's/\(declare-const (p[0-9]+) \(_ BitVec ([0-9]+)\)/\1 \2/' | while read -r name width; do
            if [ "$width" -gt 64 ]; then
                printf '  (%s #x%0*x)\n' "$name" $((width / 4)) "$val"
            else
                echo "  ($name (_ bv$val $width))"
            fi
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

WIDE_CLAIM="$TMP_ROOT/wide.vow"
cat > "$WIDE_CLAIM" <<'SRC'
module Wide

fn keep(x: u128, y: i8) -> u128 vow {
  ensures: result == x
} {
  x
}

fn main() -> i32 [io] {
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

fn half(x: f64) -> f64 vow {
  ensures: result == result
} {
  x / 2.0
}

fn main() -> i32 [io] {
  0
}
SRC

LOOP_CLAIMS="$TMP_ROOT/loop.vow"
cat > "$LOOP_CLAIMS" <<'SRC'
module Loop

fn count(n: i64) -> i64 vow {
  requires: n >= 0
  ensures: result == n
} {
  let mut i: i64 = 0;
  while i < n {
    i = i + 1;
  }
  i
}

fn main() -> i32 [io] {
  print_i64(count(1));
  0
}
SRC

CALL_CLAIMS="$TMP_ROOT/calls.vow"
cat > "$CALL_CLAIMS" <<'SRC'
module Calls

fn caller(y: i64) -> i64 vow {
  ensures: result == y
} {
  keep(y)
}

fn keep(x: i64) -> i64 vow {
  requires: x > 0
  ensures: result == x
} {
  x
}

fn main() -> i32 [io] {
  print_i64(caller(1));
  0
}
SRC

RECURSIVE="$TMP_ROOT/recursive.vow"
cat > "$RECURSIVE" <<'SRC'
module Recursive

fn spin(n: i64) -> i64 vow {
  requires: n >= 0
  ensures: result == 0
} {
  if n == 0 {
    return 0;
  }
  spin(n - 1)
}

fn main() -> i32 [io] {
  print_i64(spin(1));
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
grep -q -- '--bv-output-format 2' "$BW_DIR/args" || fail "solver not run with --bv-output-format 2"
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
expect "abort claim blame" "$(field "$RUN_OUT" counterexamples.0.blame)" "None"
expect "abort claim text" "$(field "$RUN_OUT" counterexamples.0.violation)" "division or remainder by zero"
expect "model value a" "$(field "$RUN_OUT" counterexamples.0.values.a)" "7"
expect "model value b" "$(field "$RUN_OUT" counterexamples.0.values.b)" "7"
no_leftovers "sat"

run_native sat "$ONE_CLAIM"
expect "ensures vow id" "$(field "$RUN_OUT" counterexamples.0.vow_id)" "0"
expect "ensures blame" "$(field "$RUN_OUT" counterexamples.0.blame)" "Callee"

# models are read per declared width: a 128-bit and an 8-bit parameter.
run_native sat "$WIDE_CLAIM"
expect "wide model status" "$(field "$RUN_OUT" status)" "VerifyFailed"
expect "128-bit model value" "$(field "$RUN_OUT" counterexamples.0.values.x)" "7"
expect "8-bit model value" "$(field "$RUN_OUT" counterexamples.0.values.y)" "7"

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

# an inlined call adds the callee's clauses to the caller's claims: the callee's
# `requires` and `ensures` and the caller's `ensures`, then the callee on its own.
run_native unsat "$CALL_CLAIMS"
expect "call status" "$(field "$RUN_OUT" status)" "Verified"
expect "call queries" "$(queries)" "4"
no_leftovers "calls"

# the caller's precondition claim comes first and blames the caller; the
# counterexample names the callee's clause and the call site.
run_native sat "$CALL_CLAIMS" --verify-jobs 1
expect "call sat status" "$(field "$RUN_OUT" status)" "VerifyFailed"
expect "call sat function" "$(field "$RUN_OUT" counterexamples.0.function)" "caller"
expect "call sat blame" "$(field "$RUN_OUT" counterexamples.0.blame)" "Caller"
expect "call sat vow id" "$(field "$RUN_OUT" counterexamples.0.vow_id)" "0"
expect "call site function" "$(field "$RUN_OUT" counterexamples.0.call_sites.0.caller_function)" "caller"
expect "call stops at first claim" "$(queries)" "1"
no_leftovers "calls sat"

# recursion is skipped, never sent to the solver.
run_native unsat "$RECURSIVE"
expect "recursion status" "$(field "$RUN_OUT" status)" "Skipped"
expect "recursion queries" "$(queries)" "0"
no_leftovers "recursion"

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

# a worker killed from outside (the kernel OOM killer) or a solver out of memory
# is `unknown` with a memory reason, never a verdict and never a compiler crash.
for mode in kill_worker oom_msg; do
    run_native "$mode" "$ONE_CLAIM"
    expect "$mode verify_status" "$(field "$RUN_OUT" verify_status)" "unknown"
    expect "$mode status" "$(field "$RUN_OUT" status)" "VerifyFailed"
    expect "$mode exit" "$RUN_RC" "1"
    case "$(field "$RUN_OUT" verify_message)" in
        *"memory limit exceeded"*) ;;
        *) fail "$mode lacks a memory reason: $(field "$RUN_OUT" verify_message)" ;;
    esac
    no_leftovers "$mode"
done

# any other abnormal worker death is `panicked`.
run_native crash_worker "$ONE_CLAIM"
expect "crash verify_status" "$(field "$RUN_OUT" verify_status)" "panicked"
expect "crash exit" "$RUN_RC" "1"

# timeout kills the worker's whole process group: the worker, every solver and
# every process a solver started (which only a group kill reaches) are gone.
MANY="$TMP_ROOT/many.vow"
python3 - "$MANY" <<'PY'
import sys
out = ["module Many", ""]
for i in range(6):
    out += [f"fn keep{i}(x: i64) -> i64 vow {{", "  ensures: result == x", "} {", f"  x + {i} - {i}", "}", ""]
out += ["fn main() -> i32 [io] {", "  print_i64(keep0(1));", "  0", "}", ""]
open(sys.argv[1], "w").write("\n".join(out))
PY
all_dead() {
    local file="$1" what="$2" p
    [ -f "$file" ] || { fail "$what: no pids recorded"; return; }
    while read -r p; do
        for _ in 1 2 3 4 5 6 7 8 9 10; do
            case "$(ps -p "$p" -o stat= 2>/dev/null | tr -d ' ')" in
                ""|Z*) break ;;
            esac
            sleep 0.1
        done
        case "$(ps -p "$p" -o stat= 2>/dev/null | tr -d ' ')" in
            ""|Z*) ;;
            *) fail "$what $p survived the timeout"; kill -9 "$p" 2>/dev/null || true ;;
        esac
    done < "$file"
}
for jobs in 1 4; do
    run_native hang_tree "$MANY" --timeout 1 --verify-jobs "$jobs"
    expect "hang_tree jobs=$jobs verify_status" "$(field "$RUN_OUT" verify_status)" "timeout"
    all_dead "$BW_DIR/worker_pids" "worker (jobs=$jobs)"
    all_dead "$BW_DIR/pids" "solver (jobs=$jobs)"
    all_dead "$BW_DIR/grandchild_pids" "solver child (jobs=$jobs)"
    no_leftovers "hang_tree jobs=$jobs"
done

# --verify-jobs N gives the same bytes as --verify-jobs 1, however the workers
# interleave, and never leaves a solver behind.
combined() {
    PATH="$FAKE_DIR:$PATH" TMPDIR="$SCRATCH" FAKE_BW_MODE="$1" FAKE_BW_DIR="$BW_DIR" \
        "$VOWC_BIN" verify --no-cache --backend native --verify-jobs "$2" "$MANY" 2>&1 || true
}
BW_DIR=$(mktemp -d "$TMP_ROOT/bw.XXXXXX")
ref=$(combined mixed 1)
for round in 1 2 3; do
    got=$(combined mixed 8)
    if [ "$got" != "$ref" ]; then
        fail "--verify-jobs 8 output differs from --verify-jobs 1 (round $round)"
        diff <(echo "$ref") <(echo "$got") >&2 || true
    fi
done
ref_ok=$(combined slow_unsat 1)
got_ok=$(combined slow_unsat 8)
if [ "$ref_ok" != "$got_ok" ]; then fail "all-proven output differs between --verify-jobs 1 and 8"; fi
no_leftovers "determinism"
if [ -f "$BW_DIR/pids" ]; then all_dead "$BW_DIR/pids" "solver (determinism)"; fi

# real Bitwuzla, real memory pressure: a straight-line function whose query is
# too large for the worker's address-space cap is `unknown` with a memory reason
# and the compiler survives. The cap is lowered with the test-only
# VOW_VERIFY_WORKER_MEM_KB (there is no user-facing memory flag); without it the
# same function is decided normally.
if command -v bitwuzla >/dev/null 2>&1; then
    BIG="$TMP_ROOT/big.vow"
    python3 - "$BIG" <<'PY'
import sys
n = 400
params = ", ".join(f"x{i}: i64" for i in range(2 * n))
body = " + ".join(f"x{2 * i} * x{2 * i + 1}" for i in range(n))
open(sys.argv[1], "w").write(
    f"module Big\n\nfn big({params}) -> i64 vow {{\n  ensures: result != 12345\n}} {{\n  {body}\n}}\n\n"
    "fn main() -> i32 [io] {\n  print_i64(0);\n  0\n}\n")
PY
    RUN_RC=0
    RUN_OUT=$(TMPDIR="$SCRATCH" VOW_VERIFY_WORKER_MEM_KB=100000 "$VOWC_BIN" verify --no-cache --backend native "$BIG" 2>/dev/null) || RUN_RC=$?
    expect "explosive verify_status" "$(field "$RUN_OUT" verify_status)" "unknown"
    expect "explosive status" "$(field "$RUN_OUT" status)" "VerifyFailed"
    expect "explosive exit" "$RUN_RC" "1"
    expect "explosive reason" "$(field "$RUN_OUT" verify_message)" "memory limit exceeded"
    RUN_RC=0
    RUN_OUT=$(TMPDIR="$SCRATCH" "$VOWC_BIN" verify --no-cache --backend native "$BIG" 2>/dev/null) || RUN_RC=$?
    expect "explosive uncapped status" "$(field "$RUN_OUT" status)" "VerifyFailed"
    expect "explosive uncapped is a counterexample" "$(field "$RUN_OUT" verify_status)" ""
    no_leftovers "explosive"
fi

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

# a loop is unrolled before it is checked: with every query `unsat` the
# unwinding assertion holds at the first bound, so one query per claim of that
# single round (the ensures and the unwinding claim) is enough for a proof.
run_native unsat "$LOOP_CLAIMS"
expect "loop unsat status" "$(field "$RUN_OUT" status)" "Verified"
expect "loop unsat queries" "$(queries)" "2"
no_leftovers "loop unsat"

# an undecided query is inconclusive, never a proof, and stops the schedule.
run_native unknown "$LOOP_CLAIMS"
expect "loop unknown status" "$(field "$RUN_OUT" status)" "VerifyFailed"
expect "loop unknown verify_status" "$(field "$RUN_OUT" verify_status)" "unknown"
expect "loop unknown has no counterexample" "$(field "$RUN_OUT" counterexamples.0.function)" ""
no_leftovers "loop unknown"

# a `sat` unwinding assertion alone never fails the function: every round asks
# its claims again and the last one ends `unknown`. The fake solver answers
# `sat` to the ensures claim too, so the first hard claim fails the function
# before any further round, with the clause's own vow id.
run_native sat "$LOOP_CLAIMS"
expect "loop sat status" "$(field "$RUN_OUT" status)" "VerifyFailed"
expect "loop sat vow id" "$(field "$RUN_OUT" counterexamples.0.vow_id)" "1"
no_leftovers "loop sat"

# branches: the query grows with the number of changed names, not with the
# number of paths. N sequential (2^N paths) and N nested (N+1 paths) `if`s each
# update one name; the single ensures claim's query must have one `ite` per
# changed name and be about twice as large for 2N as for N.
stress_source() {
    local shape="$1" n="$2" i
    echo "module Stress"
    echo
    echo "fn bump(x: i64) -> i64 vow {"
    echo "  ensures: result == result"
    echo "} {"
    echo "  let mut acc: i64 = x;"
    if [ "$shape" = sequential ]; then
        for ((i = 1; i <= n; i++)); do
            echo "  if x > $i { acc = acc + 1; }"
        done
    else
        for ((i = 1; i <= n; i++)); do
            echo "  if x > $i { acc = acc + 1;"
        done
        for ((i = 1; i <= n; i++)); do
            echo "  }"
        done
    fi
    echo "  acc"
    echo "}"
    echo
    echo "fn main() -> i32 [io] {"
    echo "  print_i64(bump(1));"
    echo "  0"
    echo "}"
}
query_size_for() {
    local shape="$1" n="$2" src="$TMP_ROOT/stress_${1}_${2}.vow"
    stress_source "$shape" "$n" > "$src"
    run_native unsat "$src"
    expect "$shape $n status" "$(field "$RUN_OUT" status)" "Verified"
    expect "$shape $n queries" "$(queries)" "1"
    local ites
    ites=$(grep -o '(ite ' "$BW_DIR/q.1.smt2" | wc -l | tr -d ' ')
    expect "$shape $n ite count" "$ites" "$n"
    wc -c < "$BW_DIR/q.1.smt2" | tr -d ' '
}
for shape in sequential nested; do
    small=$(query_size_for "$shape" 10)
    large=$(query_size_for "$shape" 20)
    if [ -z "$small" ] || [ -z "$large" ] || [ "$large" -gt $((small * 3)) ]; then
        fail "$shape query size is not linear: ${small:-?} bytes at 10, ${large:-?} at 20"
    fi
done
no_leftovers "stress"

# --replay-cex: a counterexample the runtime reproduces is `confirmed`; one it
# does not is a verifier bug. The fake solver answers b = 0 (a real divide-by-zero)
# or b = 7 (a model the runtime disagrees with).
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

# an entry file that defines `main` replays like any other (THREE_CLAIMS defines it).
run_native sat_zero "$THREE_CLAIMS" --replay-cex
expect "replay with main confirmed" "$(field "$RUN_OUT" counterexamples.0.replay)" "confirmed"
expect "replay with main has no bug report" "$(field "$RUN_OUT" diagnostics.1.error_code)" ""
no_leftovers "replay with main"

# a skipped replay never ran, so it is not a verifier bug (an i32 parameter is not replayable).
REPLAY_SKIP_SRC="$TMP_ROOT/replay_skip.vow"
cat > "$REPLAY_SKIP_SRC" <<'SRC'
module ReplaySkip

fn quot(a: i32, b: i32) -> i32 vow {
  ensures: result == result
} {
  a / b
}
SRC
run_native sat_zero "$REPLAY_SKIP_SRC" --replay-cex
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
