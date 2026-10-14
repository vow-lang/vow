#!/usr/bin/env bash
set -euo pipefail

# Wiring tests for `vowc verify --backend native` (epic #1398, issue #1408).
# A fake `bitwuzla` first on PATH answers from $FAKE_BW_MODE and records every
# query it is given, so this tier needs no real solver: it checks that one
# self-contained .smt2 reaches the solver per claim, that each solver outcome
# maps to the right verdict, that temp files and child processes are cleaned up,
# and that the CLI rejects what the native backend does not support. The
# incremental sessions of #1428 are off for everything but the section that
# tests them (`run_incremental`), so the counts above stay per-claim.

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
export VOW_VERIFY_NO_INCREMENTAL=1

cat > "$FAKE_DIR/bitwuzla" <<'SHIM'
#!/usr/bin/env bash
set -eu
if [ "${1:-}" = "--version" ]; then
    [ -n "${FAKE_BW_VERSION:-}" ] || exit 2
    echo "$FAKE_BW_VERSION"
    exit 0
fi
file="${!#}"
if [ "${file%.smt2}" = "$file" ]; then
    # an interactive session: commands on stdin, one reply per `(check-sat)`.
    s=1
    until mkdir "$FAKE_BW_DIR/.sess.$s" 2>/dev/null; do s=$((s + 1)); done
    t="$FAKE_BW_DIR/s.$s.smt2"
    : > "$t"
    echo "$*" >> "$FAKE_BW_DIR/sargs"
    echo $$ >> "$FAKE_BW_DIR/spids"
    case "$FAKE_BW_MODE" in
        exit1) echo "[error] boom" >&2; exit 1 ;;
        oom_msg) echo "(error) Out of memory" >&2; exit 1 ;;
        hang) echo $$ > "$FAKE_BW_DIR/pid"; exec sleep 29 ;;
        kill_worker) kill -9 "$PPID"; sleep 5 ;;
        crash_worker) kill -6 "$PPID"; sleep 5 ;;
    esac
    die_after=-1
    case "$FAKE_BW_MODE" in session_die_after_*) die_after="${FAKE_BW_MODE#session_die_after_}" ;; esac
    checks=0
    while IFS= read -r line; do
        echo "$line" >> "$t"
        case "$line" in
            "(push 1)")
                if [ "$FAKE_BW_MODE" = session_noscope ]; then
                    echo "[error] push is not supported" >&2
                    exit 1
                fi
                ;;
            "(check-sat)")
                checks=$((checks + 1))
                if [ "$die_after" -ge 0 ] && [ "$checks" -gt "$die_after" ]; then
                    echo "[error] session died" >&2
                    exit 1
                fi
                case "$FAKE_BW_MODE" in
                    garbage) echo "what is this" ;;
                    session_error_line) echo '(error "boom")' ;;
                    sat) echo sat ;;
                    unknown|session_unknown) echo unknown ;;
                    *) echo unsat ;;
                esac
                ;;
            "(echo "*)
                msg="${line#\(echo }"
                echo "${msg%)}"
                ;;
        esac
    done
    exit 0
fi
# `sat` and a model: every scalar parameter symbol (`p<k>`) and aggregate
# field symbol (`p<k>f<slot>s<sort>`) the query declares gets `$1`, except a
# slot-0 field (an enum's discriminant), which gets `$2`. A Bool is true unless 0.
# A float is +0.0.
emit_model() {
    echo sat
    echo "("
    grep -oE '^\(declare-const p[0-9]+(f[0-9]+s[0-9]+)? (Bool|\(_ BitVec [0-9]+\))\)' "$file" \
        | sed -E 's/^\(declare-const (p[0-9]+(f[0-9]+s[0-9]+)?) (Bool|\(_ BitVec ([0-9]+)\))\)/\1 \4/' | while read -r name width; do
        v=$1
        case "$name" in p*f0s*) v=$2 ;; esac
        if [ -z "$width" ]; then
            if [ "$v" -ne 0 ]; then echo "  ($name true)"; else echo "  ($name false)"; fi
        elif [ "$width" -gt 64 ]; then
            printf '  (%s #x%0*x)\n' "$name" $((width / 4)) "$v"
        else
            echo "  ($name (_ bv$v $width))"
        fi
    done
    grep -oE '^\(declare-const p[0-9]+ \(_ FloatingPoint [0-9]+ [0-9]+\)\)' "$file" \
        | sed -E 's/^\(declare-const (p[0-9]+) \(_ FloatingPoint ([0-9]+) ([0-9]+)\)\)/\1 \2 \3/' | while read -r name eb sb; do
        printf '  (%s (fp #b0 #b%s #b%s))\n' "$name" "$(printf '%0*d' "$eb" 0)" "$(printf '%0*d' $((sb - 1)) 0)"
    done
    echo ")"
}
n=1
until mkdir "$FAKE_BW_DIR/.slot.$n" 2>/dev/null; do n=$((n + 1)); done
cp "$file" "$FAKE_BW_DIR/q.$n.smt2"
echo "$file" >> "$FAKE_BW_DIR/paths"
echo "$*" >> "$FAKE_BW_DIR/args"
case "$FAKE_BW_MODE" in
    unsat|session_unknown|session_die_after_*|session_error_line|session_noscope) echo unsat ;;
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
    record_unsat) echo "$PPID" >> "$FAKE_BW_DIR/worker_ppids"; echo unsat ;;
    kill_once_3)
        # the third query of the run kills its worker, once: a retry succeeds.
        echo "$PPID" >> "$FAKE_BW_DIR/worker_ppids"
        if [ "$n" -eq 3 ] && mkdir "$FAKE_BW_DIR/.killed" 2>/dev/null; then kill -9 "$PPID"; sleep 5; fi
        echo unsat
        ;;
    kill_from_3)
        # every query from the third on kills its worker.
        echo "$PPID" >> "$FAKE_BW_DIR/worker_ppids"
        if [ "$n" -ge 3 ]; then kill -9 "$PPID"; sleep 5; fi
        echo unsat
        ;;
    hang_once_3)
        # the third query of the run hangs, once, with a solver child of its own.
        echo "$PPID" >> "$FAKE_BW_DIR/worker_pids"
        if [ "$n" -eq 3 ] && mkdir "$FAKE_BW_DIR/.hung" 2>/dev/null; then
            echo $$ >> "$FAKE_BW_DIR/pids"
            sleep 29 &
            echo $! >> "$FAKE_BW_DIR/grandchild_pids"
            wait
        fi
        echo unsat
        ;;
    unknown_unwinding)
        # `unsat` for a claim whose goal is a negation (an ensures or invariant),
        # `unknown` for the unwinding claim, whose query ends in the bare path
        # condition.
        last=$(grep '^(assert' "$file" | tail -1)
        case "$last" in *"(not "*) echo unsat ;; *) echo unknown ;; esac
        ;;
    sat_if_contains|sat_unless_contains)
        hit=0
        grep -qF -- "$FAKE_BW_NEEDLE" "$file" && hit=1
        want=1
        [ "$FAKE_BW_MODE" = sat_unless_contains ] && want=0
        if [ "$hit" -ne "$want" ]; then echo unsat; exit 0; fi
        emit_model 7 7
        ;;
    sat|sat_zero)
        val=7
        [ "$FAKE_BW_MODE" = sat_zero ] && val=0
        emit_model "$val" "$val"
        ;;
    sat_tag_some) emit_model 0 1 ;;
    sat_tag_9) emit_model 0 9 ;;
    *) echo "unknown fake mode" >&2; exit 2 ;;
esac
SHIM
chmod +x "$FAKE_DIR/bitwuzla"

ONE_CLAIM="$TMP_ROOT/one.vow"
cat > "$ONE_CLAIM" <<'SRC'
module One

fn keep(x: i64) -> i64 vow {
  ensures: result - x == 0
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
  ensures: result - x == 0
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
  ensures: result - a / b == 0
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
  x % 2.0
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

LOOP_INV_CLAIMS="$TMP_ROOT/loop_inv.vow"
cat > "$LOOP_INV_CLAIMS" <<'SRC'
module LoopInv

fn count(n: i64) -> i64 vow {
  requires: n >= 0
  ensures: result == n
} {
  let mut i: i64 = 0;
  while i < n vow {
    invariant: i <= n
  } {
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
  ensures: result - y <= 0
} {
  keep(y)
}

fn keep(x: i64) -> i64 vow {
  requires: x > 0
  ensures: result - x == 0
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
run_cmd() {
    local mode="$1"; shift
    BW_DIR=$(mktemp -d "$TMP_ROOT/bw.XXXXXX")
    RUN_RC=0
    RUN_OUT=$(PATH="$FAKE_DIR:$PATH" TMPDIR="$SCRATCH" FAKE_BW_MODE="$mode" FAKE_BW_DIR="$BW_DIR" \
        "$VOWC_BIN" "$@" 2>/dev/null) || RUN_RC=$?
}

run_native() {
    local mode="$1"; shift
    run_cmd "$mode" verify --no-cache --backend native "$@"
}

# the same, with the claims of a round sharing one solver process.
run_incremental() {
    VOW_VERIFY_NO_INCREMENTAL=0 run_native "$@"
}

queries() { find "$BW_DIR" -name 'q.*.smt2' | wc -l | tr -d ' '; }
sessions() { find "$BW_DIR" -name 's.*.smt2' | wc -l | tr -d ' '; }
session_lines() { grep -c -- "$2" "$BW_DIR/s.$1.smt2" || true; }

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
# `caller` stops at its first claim (one query). The worker session may already
# have started the next target, `keep`, when the parent reads the failure and
# stops it, so one more query is allowed; no further claim of `caller` is.
call_sat_queries=$(queries)
if [ "$call_sat_queries" != "1" ] && [ "$call_sat_queries" != "2" ]; then
    fail "call stops at first claim: got '$call_sat_queries' queries, want 1 or 2"
fi
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
    out += [f"fn keep{i}(x: i64) -> i64 vow {{", "  ensures: result - x == 0", "} {", f"  x + {i} - {i}", "}", ""]
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

# A worker session lowers the program once and decides many functions (#1683):
# with one job every query of the six targets comes from the same worker, and
# no more workers than jobs are ever started while all targets are proven.
distinct_workers() { if [ -f "$1" ]; then sort -u "$1" | wc -l | tr -d ' '; else echo 0; fi; }
run_native record_unsat "$MANY" --verify-jobs 1
expect "session status" "$(field "$RUN_OUT" status)" "Verified"
if [ "$(queries)" -lt 6 ]; then fail "session: expected a query per target, got $(queries)"; fi
expect "session: one worker for all targets" "$(distinct_workers "$BW_DIR/worker_ppids")" "1"
no_leftovers "session one job"
run_native record_unsat "$MANY" --verify-jobs 3
expect "session jobs=3 status" "$(field "$RUN_OUT" status)" "Verified"
workers=$(distinct_workers "$BW_DIR/worker_ppids")
if [ "$workers" -lt 1 ] || [ "$workers" -gt 3 ]; then fail "session jobs=3: $workers workers"; fi
no_leftovers "session three jobs"

# A worker killed from outside after it has answered earlier targets may only
# have been squeezed by what they left behind: its target is retried once in a
# fresh worker, which is what a worker per function would have run.
run_native kill_once_3 "$MANY" --verify-jobs 1
expect "session retry status" "$(field "$RUN_OUT" status)" "Verified"
expect "session retry exit" "$RUN_RC" "0"
expect "session retry: a second worker" "$(distinct_workers "$BW_DIR/worker_ppids")" "2"
no_leftovers "session retry"
# ... but only once: a target that kills the fresh worker too is `unknown`.
run_native kill_from_3 "$MANY" --verify-jobs 1
expect "session retry once status" "$(field "$RUN_OUT" status)" "VerifyFailed"
expect "session retry once verify_status" "$(field "$RUN_OUT" verify_status)" "unknown"
case "$(field "$RUN_OUT" verify_message)" in
    *"memory limit exceeded"*) ;;
    *) fail "session retry once: reason is '$(field "$RUN_OUT" verify_message)'" ;;
esac
expect "session retry once: two workers" "$(distinct_workers "$BW_DIR/worker_ppids")" "2"
no_leftovers "session retry once"

# The deadline is per target: a hang on the third target times out that target
# alone, the whole process group of its worker is gone, and the earlier targets
# keep their proofs (nothing above the first non-proven one is reported).
run_native hang_once_3 "$MANY" --timeout 2 --verify-jobs 1
expect "session hang verify_status" "$(field "$RUN_OUT" verify_status)" "timeout"
all_dead "$BW_DIR/worker_pids" "session worker"
all_dead "$BW_DIR/pids" "session solver"
all_dead "$BW_DIR/grandchild_pids" "session solver child"
no_leftovers "session hang"

# Workers that never take a task: one that exits at once (here, as a worker
# does when the program is not the one the parent planned) fails the run closed
# after a second try, and one that cannot be started fails it too, never a hang.
FAKE_VOWC="$TMP_ROOT/fake_vowc"
cat > "$FAKE_VOWC" <<'SHIM'
#!/usr/bin/env bash
if [ "$1" = verify-worker ]; then echo "worker refused" >&2; exit 3; fi
exec -a "$0" "$REAL_VOWC" "$@"
SHIM
chmod +x "$FAKE_VOWC"
BW_DIR=$(mktemp -d "$TMP_ROOT/bw.XXXXXX")
RUN_RC=0
RUN_OUT=$(REAL_VOWC="$VOWC_BIN" PATH="$FAKE_DIR:$PATH" TMPDIR="$SCRATCH" FAKE_BW_MODE=unsat FAKE_BW_DIR="$BW_DIR" \
    "$FAKE_VOWC" verify --no-cache --backend native --verify-jobs 1 "$MANY" 2>/dev/null) || RUN_RC=$?
expect "refusing worker verify_status" "$(field "$RUN_OUT" verify_status)" "panicked"
expect "refusing worker exit" "$RUN_RC" "1"
case "$(field "$RUN_OUT" verify_message)" in
    *"exit status 3"*"worker refused"*) ;;
    *) fail "refusing worker: message is '$(field "$RUN_OUT" verify_message)'" ;;
esac
expect "refusing worker: no queries" "$(queries)" "0"
no_leftovers "refusing worker"
RUN_RC=0
RUN_OUT=$(REAL_VOWC="$VOWC_BIN" PATH="$FAKE_DIR:$PATH" TMPDIR="$SCRATCH" FAKE_BW_MODE=unsat FAKE_BW_DIR="$BW_DIR" \
    bash -c 'exec -a /nonexistent/vowc "$REAL_VOWC" verify --no-cache --backend native "$1"' _ "$MANY" 2>/dev/null) || RUN_RC=$?
expect "unstartable worker verify_status" "$(field "$RUN_OUT" verify_status)" "panicked"
expect "unstartable worker message" "$(field "$RUN_OUT" verify_message)" "cannot start the verification worker"
no_leftovers "unstartable worker"

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

# a loop with a literal bound folds to constants: every claim, the unwinding
# claim of the rounds that stop short and the invariants of the covering round
# included, is decided by the simplifier, so no bounded round spawns a query. The
# solver answers `unknown`, so neither inductive attempt proves the function:
# the 1-induction of the invariant (the loop has one) is the first query, and
# each of the two rounds that stop short (bounds 2 and 4) runs its k-induction
# step case, whose first claim is one more query. The covering round at bound 8
# then ends the check on its own, with no query. Sliced, the hypotheses of a
# step are outside the cone of its first claim, so that claim is asked sliced
# and then again in full: two queries per step case.
LITERAL_LOOP="$TMP_ROOT/literal_loop.vow"
cat > "$LITERAL_LOOP" <<'SRC'
module LiteralLoop

fn tally() -> u64 vow {
  ensures: result == 36
} {
  let mut sum: u64 = 0;
  let mut i: u64 = 0;
  while i < 8 vow {
    invariant: i <= 8
  } {
    i = i + 1;
    sum = sum + i;
  }
  sum
}

fn main() -> i32 [io] {
  print_u64(tally());
  0
}
SRC
run_native unknown "$LITERAL_LOOP"
expect "literal loop status" "$(field "$RUN_OUT" status)" "Verified"
expect "literal loop bounded rounds spawn nothing" "$(queries)" "5"
export VOW_VERIFY_NO_SLICE=1
run_native unknown "$LITERAL_LOOP"
unset VOW_VERIFY_NO_SLICE
expect "literal loop unsliced queries" "$(queries)" "3"
no_leftovers "literal loop"

# the same loop with a `requires` keeps its unwinding claim for the solver: a
# contradictory assumption would make it `unsat`, which only the solver can tell.
# With `unknown` the inductive attempt and the unwinding claim are one query each
# unsliced; sliced, the `requires` is outside both cones, so each undecided
# sliced query is asked again in full and alone decides.
GUARDED_LOOP="$TMP_ROOT/guarded_loop.vow"
sed -e 's/fn tally() -> u64 vow {/fn tally(n: u64) -> u64 vow {/' \
    -e 's/  ensures: result == 36/  requires: n > 0,\n  ensures: result == 36/' \
    -e 's/tally());/tally(1));/' \
    -e 's/module LiteralLoop/module GuardedLoop/' "$LITERAL_LOOP" > "$GUARDED_LOOP"
run_native unknown "$GUARDED_LOOP"
expect "guarded loop status" "$(field "$RUN_OUT" status)" "VerifyFailed"
expect "guarded loop verify_status" "$(field "$RUN_OUT" verify_status)" "unknown"
expect "guarded loop sliced queries" "$(queries)" "4"
export VOW_VERIFY_NO_SLICE=1
run_native unknown "$GUARDED_LOOP"
unset VOW_VERIFY_NO_SLICE
expect "guarded loop unwinding claim reaches the solver" "$(queries)" "2"
no_leftovers "guarded loop"

# a decided-false claim is not dropped: the counterexample needs the solver.
WRONG_LOOP="$TMP_ROOT/wrong_loop.vow"
sed -e 's/result == 36/result == 35/' -e 's/module LiteralLoop/module WrongLoop/' "$LITERAL_LOOP" > "$WRONG_LOOP"
run_native sat "$WRONG_LOOP"
expect "wrong literal loop status" "$(field "$RUN_OUT" status)" "VerifyFailed"
expect "wrong literal loop function" "$(field "$RUN_OUT" counterexamples.0.function)" "tally"
if [ "$(queries)" -lt 1 ]; then fail "wrong literal loop: no query reached the solver"; fi
no_leftovers "wrong literal loop"

# a loop whose every loop carries an invariant is tried inductively before any
# unrolling: with every query `unsat` the three claims of the cut function (the
# invariant on entry, the invariant at the back edge and the ensures) are the
# only queries, and the function is proven without a bounded round.
run_native unsat "$LOOP_INV_CLAIMS"
expect "invariant loop unsat status" "$(field "$RUN_OUT" status)" "Verified"
expect "invariant loop unsat queries" "$(queries)" "3"
no_leftovers "invariant loop unsat"

# an undecided inductive query proves nothing: bounded unwinding runs and ends
# on its own first undecided query.
run_native unknown "$LOOP_INV_CLAIMS"
expect "invariant loop unknown status" "$(field "$RUN_OUT" status)" "VerifyFailed"
expect "invariant loop unknown verify_status" "$(field "$RUN_OUT" verify_status)" "unknown"
expect "invariant loop unknown has no counterexample" "$(field "$RUN_OUT" counterexamples.0.function)" ""
no_leftovers "invariant loop unknown"

# a `sat` inductive query is never a counterexample (the havoc state may be
# unreachable): the verdict comes from bounded unwinding, which names the
# invariant's own vow id.
run_native sat "$LOOP_INV_CLAIMS"
expect "invariant loop sat status" "$(field "$RUN_OUT" status)" "VerifyFailed"
expect "invariant loop sat violation" "$(field "$RUN_OUT" counterexamples.0.violation)" "invariant i <= n"
no_leftovers "invariant loop sat"

# branches: the query grows with the number of changed names, not with the
# number of paths. N sequential (2^N paths) and N nested (N+1 paths) `if`s each
# update one name; the single ensures claim's query must have one `ite` per
# changed name and be about twice as large for 2N as for N.
stress_source() {
    local shape="$1" n="$2" i
    echo "module Stress"
    echo
    echo "fn bump(x: i64) -> i64 vow {"
    echo "  ensures: result - x + x == result"
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
  ensures: result - a / b == 0
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

# a skipped replay never ran, so it is not a verifier bug (an f32 parameter is not replayable: it has no source literal).
REPLAY_SKIP_SRC="$TMP_ROOT/replay_skip.vow"
cat > "$REPLAY_SKIP_SRC" <<'SRC'
module ReplaySkip

fn sum(a: f32, b: f32) -> f32 vow {
  ensures: result != a + b
} {
  a + b
}
SRC
run_native sat_zero "$REPLAY_SKIP_SRC" --replay-cex
expect "replay skipped" "$(field "$RUN_OUT" counterexamples.0.replay)" "skipped"
expect "skipped replay is not a verifier bug" "$(field "$RUN_OUT" diagnostics.1.error_code)" ""

run_native sat "$REPLAY_SRC"
expect "no replay without the flag" "$(field "$RUN_OUT" counterexamples.0.replay)" ""
expect "no bug report without the flag" "$(field "$RUN_OUT" diagnostics.0.error_code)" "VerifierAssertionUnattributed"

# aggregate parameters: a counterexample lists the fields of a struct, enum,
# `Option` or `Result` parameter as `<param>.<slot>` (an enum's discriminant is
# slot 0) and `--replay-cex` rebuilds the argument from them. The fake solver
# gives every slot a value and, in `sat_tag_*`, every discriminant another one.
ce_value() {
    python3 -c 'import json, sys
ce = json.loads(sys.argv[1])["counterexamples"][int(sys.argv[2])]
print(ce.get("values", {}).get(sys.argv[3], "<absent>"))' "$RUN_OUT" 0 "$1"
}

AGG_OPTION="$TMP_ROOT/agg_option.vow"
cat > "$AGG_OPTION" <<'SRC'
module AggOption

fn get(o: Option<i64>) -> i64 [panic] vow {
  ensures: result == result
} {
  o.unwrap()
}
SRC
run_native sat_zero "$AGG_OPTION" --replay-cex
expect "option param status" "$(field "$RUN_OUT" status)" "VerifyFailed"
expect "option param tag value" "$(ce_value o.0)" "0"
expect "option param payload value" "$(ce_value o.1)" "0"
expect "option param replay" "$(field "$RUN_OUT" counterexamples.0.replay)" "confirmed"
expect "option param confirmed is not a bug" "$(field "$RUN_OUT" diagnostics.1.error_code)" ""
no_leftovers "option param"

# a deliberately wrong model value: `Some(0)` does not abort on unwrap.
run_native sat_tag_some "$AGG_OPTION" --replay-cex
expect "wrong option tag value" "$(ce_value o.0)" "1"
expect "wrong option tag replay" "$(field "$RUN_OUT" counterexamples.0.replay)" "diverged"
expect "wrong option tag bug code" "$(field "$RUN_OUT" diagnostics.1.error_code)" "VerifierBug"
no_leftovers "wrong option tag"

AGG_STRUCT="$TMP_ROOT/agg_struct.vow"
cat > "$AGG_STRUCT" <<'SRC'
module AggStruct

struct Pair { lo: i64, hi: i64 }

fn span(p: Pair) -> i64 vow {
  ensures: result == p.lo
} {
  p.hi - p.lo
}
SRC
run_native sat "$AGG_STRUCT" --replay-cex
expect "struct param field lo" "$(ce_value p.0)" "7"
expect "struct param field hi" "$(ce_value p.1)" "7"
expect "struct param replay" "$(field "$RUN_OUT" counterexamples.0.replay)" "confirmed"
no_leftovers "struct param"

run_native sat_zero "$AGG_STRUCT" --replay-cex
expect "struct param all-zero replay" "$(field "$RUN_OUT" counterexamples.0.replay)" "diverged"
expect "struct param all-zero bug code" "$(field "$RUN_OUT" diagnostics.1.error_code)" "VerifierBug"
no_leftovers "struct param all-zero"

AGG_ENUM="$TMP_ROOT/agg_enum.vow"
cat > "$AGG_ENUM" <<'SRC'
module AggEnum

enum Shape { Circle(i64), Rect(i64, i64), Empty }

fn area(s: Shape) -> i64 vow {
  ensures: result == 1
} {
  match s {
    Shape::Circle(r) => r,
    Shape::Rect(w, h) => w + h,
    Shape::Empty => 0,
  }
}
SRC
run_native sat_tag_9 "$AGG_ENUM" --replay-cex
expect "enum param discriminant value" "$(ce_value s.0)" "9"
expect "enum param out-of-range replay" "$(field "$RUN_OUT" counterexamples.0.replay)" "skipped"
case "$(field "$RUN_OUT" counterexamples.0.replay_reason)" in
    *"discriminant 9 outside Shape (3 variants)"*) ;;
    *) fail "enum param skip reason: $(field "$RUN_OUT" counterexamples.0.replay_reason)" ;;
esac
expect "skipped enum replay is not a bug" "$(field "$RUN_OUT" diagnostics.1.error_code)" ""
no_leftovers "enum param"

run_native sat_zero "$AGG_ENUM" --replay-cex
expect "enum param in-range replay" "$(field "$RUN_OUT" counterexamples.0.replay)" "confirmed"
no_leftovers "enum param in range"

# a `Result` payload read at two sorts: the slot's symbols are told apart.
AGG_RESULT="$TMP_ROOT/agg_result.vow"
cat > "$AGG_RESULT" <<'SRC'
module AggResult

fn pick(r: Result<i64, bool>) -> i64 vow {
  ensures: result == 5
} {
  match r {
    Result::Ok(v) => v,
    Result::Err(e) => if e { 1 } else { 0 },
  }
}
SRC
run_native sat_zero "$AGG_RESULT"
expect "result param tag" "$(ce_value r.0)" "0"
expect "result param i64 view" "$(ce_value r.1:64)" "0"
expect "result param bool view" "$(ce_value r.1:bool)" "false"
expect "result param unsuffixed name is absent" "$(ce_value r.1)" "<absent>"
no_leftovers "result param"

# the cache keeps the field values: a warm run reports the same counterexample.
AGG_CACHE="$TMP_ROOT/agg_cache"
mkdir -p "$AGG_CACHE"
BW_DIR=$(mktemp -d "$TMP_ROOT/bw.XXXXXX")
RUN_OUT=$(PATH="$FAKE_DIR:$PATH" TMPDIR="$SCRATCH" FAKE_BW_MODE=sat FAKE_BW_DIR="$BW_DIR" VOW_CACHE_DIR="$AGG_CACHE" \
    "$VOWC_BIN" verify --backend native "$AGG_STRUCT" 2>/dev/null) || true
cold=$(ce_value p.1)
RUN_OUT=$(PATH="$FAKE_DIR:$PATH" TMPDIR="$SCRATCH" FAKE_BW_MODE=sat FAKE_BW_DIR="$BW_DIR" VOW_CACHE_DIR="$AGG_CACHE" \
    "$VOWC_BIN" verify --backend native "$AGG_STRUCT" 2>/dev/null) || true
expect "cold struct field value" "$cold" "7"
expect "warm struct field value" "$(ce_value p.1)" "7"
no_leftovers "aggregate cache"

# a Vec function is decided in the array logic: every query of it declares an
# array sort and uses select/store, and none carries a bound on a length (the
# model has no capacity), while a function without a Vec stays in QF_BV.
VEC_CLAIMS="$TMP_ROOT/vec.vow"
cat > "$VEC_CLAIMS" <<'SRC'
module Vec

fn grow(v: Vec<i64>, i: u64) -> i64 vow {
  requires: i < v.len()
  ensures: result == result
} {
  v.push(1);
  v[i]
}

fn main() -> i32 [io] {
  0
}
SRC
export VOW_VERIFY_NO_SLICE=1
run_native unsat "$VEC_CLAIMS"
unset VOW_VERIFY_NO_SLICE
expect "vec status" "$(field "$RUN_OUT" status)" "Verified"
if [ "$(queries)" -lt 1 ]; then fail "vec: no query reached the solver"; fi
for q in "$BW_DIR"/q.*.smt2; do
    grep -q '^(set-logic QF_ABV)$' "$q" || fail "vec query $q is not QF_ABV"
    grep -q '(Array (_ BitVec 64) (_ BitVec 64))' "$q" || fail "vec query $q declares no array"
    grep -q '(store ' "$q" || fail "vec query $q has no store"
    if grep -Eq '#x0*(80|7f|ff|100|10000)\)' "$q"; then fail "vec query $q carries a capacity-like constant"; fi
done
no_leftovers "vec"
run_native unsat "$ONE_CLAIM"
for q in "$BW_DIR"/q.*.smt2; do
    grep -q '^(set-logic QF_BV)$' "$q" || fail "scalar query $q left QF_BV"
done

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
usage_error "requires --verify" contracts --backend native "$ONE_CLAIM"
usage_error "requires --verify" test --backend native "$ONE_CLAIM"
usage_error "--solver" contracts --verify --backend native --solver z3 "$ONE_CLAIM"
usage_error "--solver" test --verify --backend native --solver z3 "$ONE_CLAIM"
usage_error "--max-k-step" test --verify --backend native --max-k-step 5 "$ONE_CLAIM"
usage_error "only supported by" "$ONE_CLAIM" --backend native
usage_error "--max-k-step" verify --backend native --max-k-step 5 "$ONE_CLAIM"
usage_error "--solver" verify --backend native --solver z3 "$ONE_CLAIM"
usage_error "--encoding" verify --backend native --encoding bv "$ONE_CLAIM"

# Query slicing (#1424). Each claim's query keeps only its cone of influence; a
# sliced query that is not `unsat` is re-asked in full, which alone decides, so
# slicing can change query sizes and counts but never a verdict or a model.
# VOW_VERIFY_NO_SLICE=1 (test-only) restores the unsliced queries.
SLICE_INDEP="tests/verify-native/pass/slice_independent_clauses.vow"
SLICE_VACUOUS="tests/verify-native/pass/slice_vacuous_unrelated_requires.vow"

query_bytes() {
    local total=0 q n
    for q in "$BW_DIR"/q.*.smt2; do
        n=$(wc -c < "$q" | tr -d ' ')
        total=$((total + n))
    done
    echo "$total"
}

run_native unsat "$SLICE_INDEP"
sliced_bytes=$(query_bytes)
expect "slice independent status" "$(field "$RUN_OUT" status)" "Verified"
expect "slice independent queries" "$(queries)" "2"
export VOW_VERIFY_NO_SLICE=1
run_native unsat "$SLICE_INDEP"
unset VOW_VERIFY_NO_SLICE
full_bytes=$(query_bytes)
expect "unsliced independent queries" "$(queries)" "2"
if [ "$sliced_bytes" -ge "$full_bytes" ]; then
    fail "slicing did not shrink the queries: $sliced_bytes bytes sliced, $full_bytes unsliced"
fi
no_leftovers "slice independent"

# A claim whose sliced query is `sat` is re-asked on the full query; the verdict
# and the counterexample are the unsliced run's. The needle is the negated goal
# of the second clause, so the fake solver agrees with both query shapes.
export FAKE_BW_NEEDLE="(assert (not v37))"
run_native sat_if_contains "$SLICE_INDEP"
sliced_out="$RUN_OUT"
expect "confirmed sat status" "$(field "$RUN_OUT" status)" "VerifyFailed"
expect "confirmed sat queries" "$(queries)" "3"
export VOW_VERIFY_NO_SLICE=1
run_native sat_if_contains "$SLICE_INDEP"
unset VOW_VERIFY_NO_SLICE
expect "unsliced sat queries" "$(queries)" "2"
if [ "$sliced_out" != "$RUN_OUT" ]; then
    fail "slicing changed the counterexample report: '$sliced_out' vs '$RUN_OUT'"
fi
no_leftovers "slice confirm sat"

# Dropping a contradictory `requires` on an unrelated parameter must not turn a
# vacuous proof into a failure: the sliced query is `sat`, the full one `unsat`.
export FAKE_BW_NEEDLE="#x0000000000000003"
run_native sat_unless_contains "$SLICE_VACUOUS"
expect "vacuous status" "$(field "$RUN_OUT" status)" "Verified"
expect "vacuous exit" "$RUN_RC" "0"
expect "vacuous queries" "$(queries)" "2"
unset FAKE_BW_NEEDLE
no_leftovers "slice vacuous"

# Per-clause verdicts (#1424). `verify-worker --worker-clauses` decides every
# claim of its function and reports, per ensures/invariant clause, 1 (proven) or
# 0 (failed) after the usual result; the overall result is the default mode's.
# Wire statuses: 0 proven, 1 failed, 2 timeout, 3 error, 6 unknown.
WIRE_DECODE="$TMP_ROOT/wire_decode.py"
cat > "$WIRE_DECODE" <<'PY'
import sys
t = sys.stdin.read()
i = t.index("VOWRES3\n") + 8
def line():
    global i
    j = t.index("\n", i)
    v = int(t[i:j])
    i = j + 1
    return v
def text():
    global i
    n = line()
    v = t[i:i + n]
    i += n + 1
    return v
status = line()
vow = line()
for _ in range(4):
    line()
for _ in range(line()):
    text(); text()
for _ in range(line()):
    line()
text()
for _ in range(line()):
    text(); line(); line(); line()
for _ in range(line()):
    line(); line(); line()
n = line()
print(status, vow, ",".join("%d:%d" % (line(), line()) for _ in range(n)))
PY

CLAUSES_SRC="$TMP_ROOT/clauses.vow"
cat > "$CLAUSES_SRC" <<'SRC'
module Clauses

fn two(a: i64, b: i64) -> i64 vow {
  requires: a > 0 && a < 1000000000
  requires: b > 0 && b < 1000000000
  ensures: a * 2 > a
  ensures: b * 7 > b
} {
  a + b
}

fn main() -> i32 [io] {
  print_i64(two(1, 1));
  0
}
SRC

# Prints "<status> <vow id> <id:verdict,...>" for the worker of `two`.
worker_clauses() {
    local mode="$1"; shift
    BW_DIR=$(mktemp -d "$TMP_ROOT/bw.XXXXXX")
    PATH="$FAKE_DIR:$PATH" TMPDIR="$SCRATCH" FAKE_BW_MODE="$mode" FAKE_BW_DIR="$BW_DIR" \
        "$VOWC_BIN" verify-worker "$CLAUSES_SRC" --worker-index 0 --worker-name two \
        --worker-budget-ms "${WORKER_BUDGET_MS:-60000}" "$@" 2>/dev/null | python3 "$WIRE_DECODE"
}

export FAKE_BW_NEEDLE="#x0000000000000007"
expect "clauses all proven" "$(worker_clauses unsat --worker-clauses)" "0 -1 2:1,3:1"
expect "clauses: only the second sat" "$(worker_clauses sat_if_contains --worker-clauses)" "1 3 2:1,3:0"
expect "default mode has no clauses" "$(worker_clauses sat_if_contains)" "1 3 "
expect "clauses: first failure is the overall result" \
    "$(worker_clauses sat --worker-clauses | cut -d' ' -f1,2)" "1 2"
unset FAKE_BW_NEEDLE
no_leftovers "worker clauses"

# A solver answer that no later claim can improve on stops the schedule, and the
# clauses it left undecided get no verdict (the caller maps them to the overall).
expect "clauses: unknown does not stop" "$(worker_clauses unknown --worker-clauses)" "6 -1 "
expect "clauses: error stops with no verdicts" "$(worker_clauses garbage --worker-clauses)" "3 -1 "
expect "clauses: error stops after the confirming query" "$(queries)" "2"
no_leftovers "worker clause halt"

# The same decisions against the real solver (skipped without one): a hard abort
# leaves the ensures verdicts intact, a clause with no claim is proven next to a
# failing sibling, and a loop's clauses are proven only by its closing round.
if command -v bitwuzla >/dev/null 2>&1; then
    CLAUSES_REAL="$TMP_ROOT/clauses_real.vow"
    cat > "$CLAUSES_REAL" <<'SRC'
module ClausesReal

fn quo(a: i64, b: i64) -> i64 vow {
  requires: a > 0 && a < 1000000000
  ensures: a * 2 > a
} {
  a / b
}

fn scale_wrong(a: i64, b: i64) -> i64 vow {
  requires: a > 0 && a < 1000000000
  requires: b > 0 && b < 1000000000
  ensures: a * 2 > a
  ensures: b * 3 > b + 2
  ensures: result == a * 2 + b * 3
} {
  a * 2 + b * 3
}
SRC
    real_clauses() {
        "$VOWC_BIN" verify-worker "$1" --worker-index "$2" --worker-name "$3" \
            --worker-budget-ms 60000 --worker-clauses 2>/dev/null | python3 "$WIRE_DECODE"
    }
    expect "real: abort claim leaves the ensures proven" "$(real_clauses "$CLAUSES_REAL" 0 quo)" "1 -1 1:1"
    expect "real: wrong clause and a clause without claims" "$(real_clauses "$CLAUSES_REAL" 1 scale_wrong)" "1 3 2:1,3:0,4:1"
    expect "real: closed loop proves every clause" \
        "$(real_clauses tests/verify-native/pass/loop_literal_bound.vow 0 fill_and_sum)" "0 -1 0:1,1:1,2:1"
    expect "real: open loop proves no clause" \
        "$(real_clauses tests/verify-native/unknown/loop_bug_beyond_bound.vow 0 count)" "6 -1 "
    expect "real: k-induction proves the clause of an invariant-free loop" \
        "$(real_clauses tests/verify-native/pass/loop_symbolic_bound.vow 0 count)" "0 -1 1:1"
    expect "real: failure in a late iteration" \
        "$(real_clauses tests/verify-native/fail/loop_bug_at_iteration_40.vow 0 trip)" "1 1 0:1,1:0,2:1"
fi

# An unwinding claim the solver cannot decide leaves the round open: nothing is
# proven although every clause claim was `unsat` (rule 2 of
# ADR-2026-10-08-1422), and the overall result is `unknown`.
# The function has two loops, so neither induction applies: the k-induction step
# case would otherwise prove the single loop with every non-unwinding claim
# `unsat`.
SYMBOLIC_LOOP="$TMP_ROOT/two_loops.vow"
cat > "$SYMBOLIC_LOOP" <<'SRC'
module TwoLoops

fn count(n: u64) -> u64 vow {
  ensures: result == n
} {
  let mut i: u64 = 0;
  while i < n {
    i = i + 1;
  }
  let mut j: u64 = 0;
  while j < n {
    j = j + 1;
  }
  i
}
SRC
BW_DIR=$(mktemp -d "$TMP_ROOT/bw.XXXXXX")
expect "undecided unwinding proves no clause" \
    "$(PATH="$FAKE_DIR:$PATH" TMPDIR="$SCRATCH" FAKE_BW_MODE=unknown_unwinding FAKE_BW_DIR="$BW_DIR" \
        "$VOWC_BIN" verify-worker "$SYMBOLIC_LOOP" --worker-index 0 --worker-name count \
        --worker-budget-ms 60000 --worker-clauses 2>/dev/null | python3 "$WIRE_DECODE")" "6 -1 "
no_leftovers "undecided unwinding"

# `contracts --verify` and `test --verify` on the native backend (#1427). The
# weak-contract probes are separate worker tasks, so a fake solver that answers
# `unsat` to everything reads as "every requires is contradictory" and
# `sat_empty` as "nothing is proven": the cases below pin the plumbing, and the
# real-solver fixtures in tests/verify-native/contracts/ pin the verdicts.
TWO_FUNCS="$TMP_ROOT/two_funcs.vow"
cat > "$TWO_FUNCS" <<'SRC'
module TwoFuncs

fn inc(a: i64) -> i64 vow {
  requires: a > 0 && a < 1000
  ensures: result - a == 1
} {
  a + 1
}

fn dec(a: i64) -> i64 vow {
  requires: a > 0 && a < 1000
  ensures: a - result == 1
} {
  a - 1
}

fn main() -> i32 [io] {
  print_i64(inc(1) + dec(2));
  0
}
SRC

# Nothing proven: every ensures is failed, every requires unknown, and the
# second function is reported although the first one failed.
run_cmd sat contracts --verify --backend native "$TWO_FUNCS"
expect "contracts sat exit" "$RUN_RC" "1"
expect "contracts sat total" "$(field "$RUN_OUT" summary.total)" "4"
expect "contracts sat failed" "$(field "$RUN_OUT" summary.failed)" "2"
expect "contracts sat unknown" "$(field "$RUN_OUT" summary.unknown)" "2"
expect "contracts sat vacuous" "$(field "$RUN_OUT" summary.vacuous)" "0"
expect "contracts sat trivial" "$(field "$RUN_OUT" summary.trivially_satisfiable)" "0"
expect "contracts later function status" "$(field "$RUN_OUT" contracts.3.status)" "failed"
expect "contracts later function name" "$(field "$RUN_OUT" contracts.3.function)" "dec"
if [ "$(queries)" -lt 2 ]; then fail "contracts native reached the solver for fewer than both functions"; fi
no_leftovers "contracts sat"

# Every query unsat: both probes are proven, so both contracts are vacuous and
# both ensures clauses trivially satisfiable.
run_cmd unsat contracts --verify --backend native "$TWO_FUNCS"
expect "contracts unsat exit" "$RUN_RC" "1"
expect "contracts unsat vacuous" "$(field "$RUN_OUT" summary.vacuous)" "4"
expect "contracts unsat trivial" "$(field "$RUN_OUT" summary.trivially_satisfiable)" "2"
expect "contracts unsat requires status" "$(field "$RUN_OUT" contracts.0.status)" "vacuous"
expect "contracts unsat ensures flag" "$(field "$RUN_OUT" contracts.1.trivially_satisfiable)" "True"
expect "contracts unsat requires flag" "$(field "$RUN_OUT" contracts.0.trivially_satisfiable)" "False"
no_leftovers "contracts unsat"

# Without a solver every clause that needed one is `error`, the JSON is still
# complete and the run fails closed.
RUN_RC=0
RUN_OUT=$(PATH="$EMPTY_DIR" TMPDIR="$SCRATCH" "$VOWC_BIN" contracts --verify --backend native "$TWO_FUNCS" 2>/dev/null) || RUN_RC=$?
expect "contracts no solver exit" "$RUN_RC" "1"
expect "contracts no solver error" "$(field "$RUN_OUT" summary.error)" "4"
expect "contracts no solver total" "$(field "$RUN_OUT" summary.total)" "4"
no_leftovers "contracts no solver"

# A function outside the native subset is `skipped`, needs no solver and fails
# the run closed; the clause rows keep the schema's shape.
RUN_RC=0
RUN_OUT=$(PATH="$EMPTY_DIR" TMPDIR="$SCRATCH" "$VOWC_BIN" contracts --verify --backend native tests/verify-native/skip/effects_skipped.vow 2>/dev/null) || RUN_RC=$?
expect "contracts skipped exit" "$RUN_RC" "1"
expect "contracts skipped status" "$(field "$RUN_OUT" contracts.0.status)" "skipped"
expect "contracts skipped count" "$(field "$RUN_OUT" summary.skipped)" "$(field "$RUN_OUT" summary.total)"

python3 - "$RUN_OUT" <<'PY' || fail "contracts native JSON does not match contracts-result.schema.json"
import json, sys
schema = json.load(open("docs/spec/schemas/contracts-result.schema.json"))
out = json.loads(sys.argv[1])
assert set(schema["required"]) <= set(out), (schema["required"], list(out))
row_schema = schema["properties"]["contracts"]["items"]
for row in out["contracts"]:
    assert set(row_schema["required"]) <= set(row), (row_schema["required"], list(row))
    assert row["status"] in row_schema["properties"]["status"]["enum"], row["status"]
summary_schema = schema["properties"]["summary"]
assert set(summary_schema["required"]) <= set(out["summary"]), list(out["summary"])
PY

# `--verify-jobs` and `--timeout` are honoured, not just accepted.
run_cmd unsat contracts --verify --backend native --verify-jobs 4 --timeout 30 "$TWO_FUNCS"
expect "contracts jobs exit" "$RUN_RC" "1"
expect "contracts jobs total" "$(field "$RUN_OUT" summary.total)" "4"

# `test --verify`: a proven module runs, an unproven one fails before running.
run_cmd unsat test --verify --backend native "$ONE_CLAIM"
expect "test proven status" "$(field "$RUN_OUT" status)" "TestsPassed"
expect "test proven entry" "$(field "$RUN_OUT" tests.0.status)" "passed"
expect "test proven exit" "$RUN_RC" "0"
no_leftovers "test proven"

run_cmd sat test --verify --backend native "$ONE_CLAIM"
expect "test failed status" "$(field "$RUN_OUT" status)" "TestsFailed"
expect "test failed entry" "$(field "$RUN_OUT" tests.0.status)" "verify_failed"
expect "test failed code" "$(field "$RUN_OUT" tests.0.diagnostics.0.error_code)" "VerifyFailed"
expect "test failed stdout" "$(field "$RUN_OUT" tests.0.stdout)" ""
expect "test failed exit" "$RUN_RC" "1"
no_leftovers "test failed"

# An undecided function fails closed too: the ESBMC path lets an `error` pass.
run_cmd unknown test --verify --backend native "$ONE_CLAIM"
expect "test unknown entry" "$(field "$RUN_OUT" tests.0.status)" "verify_failed"
run_cmd garbage test --verify --backend native "$ONE_CLAIM"
expect "test error entry" "$(field "$RUN_OUT" tests.0.status)" "verify_failed"

RUN_RC=0
RUN_OUT=$(PATH="$EMPTY_DIR" TMPDIR="$SCRATCH" "$VOWC_BIN" test --verify --backend native "$ONE_CLAIM" 2>/dev/null) || RUN_RC=$?
expect "test no solver entry" "$(field "$RUN_OUT" tests.0.status)" "verify_failed"

RUN_RC=0
RUN_OUT=$(PATH="$EMPTY_DIR" TMPDIR="$SCRATCH" "$VOWC_BIN" test --verify --backend native tests/verify-native/skip/effects_skipped.vow 2>/dev/null) || RUN_RC=$?
expect "test skipped entry" "$(field "$RUN_OUT" tests.0.status)" "contract_skipped"
expect "test skipped code" "$(field "$RUN_OUT" tests.0.diagnostics.0.error_code)" "VerificationSkipped"
expect "test skipped exit" "$RUN_RC" "1"

# Parallel test workers inherit the backend: both files reach the fake solver.
TEST_DIR="$TMP_ROOT/test_dir"
mkdir -p "$TEST_DIR"
cp "$ONE_CLAIM" "$TEST_DIR/test_one.vow"
sed 's/module One/module Two/' "$ONE_CLAIM" > "$TEST_DIR/test_two.vow"
run_cmd unsat test --verify --backend native --jobs 2 "$TEST_DIR"
expect "test parallel status" "$(field "$RUN_OUT" status)" "TestsPassed"
expect "test parallel total" "$(field "$RUN_OUT" total)" "2"
expect "test parallel queries" "$(queries)" "2"
no_leftovers "test parallel"

# A sibling module found through `--module-root` is lowered the same way by the
# worker as by the parent.
MOD_DIR="$TMP_ROOT/modroot"
mkdir -p "$MOD_DIR/lib" "$MOD_DIR/t"
cat > "$MOD_DIR/lib/helper.vow" <<'SRC'
module Helper

fn bump(x: i64) -> i64 vow {
  ensures: result - x == 1
} {
  x + 1
}
SRC
cat > "$MOD_DIR/t/test_helper.vow" <<'SRC'
module TestHelper

use lib.helper

fn main() -> i32 [io] {
  if bump(1) == 2 { 0 } else { 1 }
}
SRC
run_cmd unsat test --verify --backend native --module-root "$MOD_DIR" "$MOD_DIR/t/test_helper.vow"
expect "test module root entry" "$(field "$RUN_OUT" tests.0.status)" "passed"
expect "test module root queries" "$(queries)" "1"
no_leftovers "test module root"

# Real solver: the verdicts agree with what the program does.
if command -v bitwuzla >/dev/null 2>&1; then
    RUN_RC=0
    RUN_OUT=$("$VOWC_BIN" test --verify --backend native "$ONE_CLAIM" 2>/dev/null) || RUN_RC=$?
    expect "real: test proven entry" "$(field "$RUN_OUT" tests.0.status)" "passed"
    BROKEN="$TMP_ROOT/broken.vow"
    cat > "$BROKEN" <<'SRC'
module Broken

fn inc(a: i64) -> i64 vow {
  requires: a > 0 && a < 1000
  ensures: result == a + 2
} {
  a + 1
}

fn main() -> i32 [io] {
  0
}
SRC
    RUN_RC=0
    RUN_OUT=$("$VOWC_BIN" test --verify --backend native "$BROKEN" 2>/dev/null) || RUN_RC=$?
    expect "real: test failed entry" "$(field "$RUN_OUT" tests.0.status)" "verify_failed"
    expect "real: test failed exit" "$RUN_RC" "1"
fi

# A clause is only ever given 1/0 here; its status string comes from
# `resolve_clause_status`, whose vocabulary must stay inside the schema's.
python3 - <<'PY' || fail "clause statuses are not in contracts-result.schema.json"
import json
schema = json.load(open("docs/spec/schemas/contracts-result.schema.json"))
enum = schema["properties"]["contracts"]["items"]["properties"]["status"]["enum"]
missing = {"proven", "failed", "unknown", "timeout", "error"} - set(enum)
assert not missing, missing
PY

# Native result cache (#1429). The fake solver reports a version only when a
# test sets FAKE_BW_VERSION, so every run above is cache-off. A cold run asks
# the solver and stores its decisive answers; a warm run on the same cache
# directory and solver version replays them and reaches the same JSON.
CACHE_DIR=""
run_native_cached() {
    local mode="$1" version="$2"; shift 2
    BW_DIR=$(mktemp -d "$TMP_ROOT/bw.XXXXXX")
    RUN_RC=0
    RUN_OUT=$(PATH="$FAKE_DIR:$PATH" TMPDIR="$SCRATCH" FAKE_BW_MODE="$mode" FAKE_BW_DIR="$BW_DIR" \
        FAKE_BW_VERSION="$version" VOW_CACHE_DIR="$CACHE_DIR" \
        "$VOWC_BIN" verify --backend native "$@" 2>/dev/null) || RUN_RC=$?
}

cache_entries() { find "$CACHE_DIR/verify-native" -type f 2>/dev/null | wc -l | tr -d ' '; }

new_cache() { CACHE_DIR=$(mktemp -d "$TMP_ROOT/cache.XXXXXX"); }

for pair in "unsat:$THREE_CLAIMS" "sat:$THREE_CLAIMS" "sat:$ONE_CLAIM"; do
    mode="${pair%%:*}"
    src="${pair#*:}"
    new_cache
    run_native_cached "$mode" "fake 1.0" "$src"
    cold_out="$RUN_OUT"; cold_rc="$RUN_RC"; cold_q=$(queries)
    [ "$cold_q" -gt 0 ] || fail "cache cold $mode: no solver query"
    [ "$(cache_entries)" -gt 0 ] || fail "cache cold $mode: nothing stored"
    run_native_cached "$mode" "fake 1.0" "$src"
    expect "cache warm $mode queries" "$(queries)" "0"
    expect "cache warm $mode json" "$RUN_OUT" "$cold_out"
    expect "cache warm $mode exit" "$RUN_RC" "$cold_rc"
    run_native "$mode" "$src"
    expect "cache off $mode json" "$RUN_OUT" "$cold_out"
done
no_leftovers "cache warm"

# A different solver version is a different key.
new_cache
run_native_cached unsat "fake 1.0" "$THREE_CLAIMS"
run_native_cached unsat "fake 2.0" "$THREE_CLAIMS"
expect "new solver version asks again" "$(queries)" "3"
run_native_cached unsat "fake 2.0" "$THREE_CLAIMS"
expect "same solver version replays" "$(queries)" "0"

# --no-cache neither reads nor writes.
new_cache
run_native_cached unsat "fake 1.0" "$THREE_CLAIMS" --no-cache
run_native_cached unsat "fake 1.0" "$THREE_CLAIMS" --no-cache
expect "no-cache asks every run" "$(queries)" "3"
expect "no-cache stores nothing" "$(cache_entries)" "0"

# A solver that does not report a version cannot name the entries it answers:
# verdicts are right and nothing is stored.
new_cache
run_native_cached unsat "" "$THREE_CLAIMS"
expect "no version status" "$(field "$RUN_OUT" status)" "Verified"
run_native_cached unsat "" "$THREE_CLAIMS"
expect "no version asks every run" "$(queries)" "3"
expect "no version stores nothing" "$(cache_entries)" "0"

# Damaged entries are misses, never verdicts.
new_cache
run_native_cached unsat "fake 1.0" "$THREE_CLAIMS"
find "$CACHE_DIR/verify-native" -type f | while read -r entry; do
    printf 'vow-native-cache v1\nunsat\n0\n' > "$entry"
done
run_native_cached unsat "fake 1.0" "$THREE_CLAIMS"
expect "garbled entries status" "$(field "$RUN_OUT" status)" "Verified"
expect "garbled entries asked again" "$(queries)" "3"
find "$CACHE_DIR/verify-native" -type f | while read -r entry; do
    : > "$entry"
done
run_native_cached unsat "fake 1.0" "$THREE_CLAIMS"
expect "emptied entries asked again" "$(queries)" "3"
run_native_cached unsat "fake 1.0" "$THREE_CLAIMS"
expect "rewritten entries replay" "$(queries)" "0"

# A function whose sliced query is `sat` and whose full query is `unsat` is
# proven warm as it is cold: each query is cached under its own text.
export FAKE_BW_NEEDLE="#x0000000000000003"
new_cache
run_native_cached sat_unless_contains "fake 1.0" "$SLICE_VACUOUS"
cold_out="$RUN_OUT"
expect "cached vacuous cold status" "$(field "$RUN_OUT" status)" "Verified"
run_native_cached sat_unless_contains "fake 1.0" "$SLICE_VACUOUS"
expect "cached vacuous warm queries" "$(queries)" "0"
expect "cached vacuous warm json" "$RUN_OUT" "$cold_out"
unset FAKE_BW_NEEDLE

# An exhausted budget is a timeout before the cache is consulted.
new_cache
run_native_cached unsat "fake 1.0" "$ONE_CLAIM"
run_native_cached unsat "fake 1.0" "$ONE_CLAIM" --timeout 0
expect "cached timeout 0 verify_status" "$(field "$RUN_OUT" verify_status)" "timeout"
expect "cached timeout 0 spawns nothing" "$(queries)" "0"
no_leftovers "cache"
# Incremental solver sessions (#1428): the claims of a round share one solver
# process (header and assumptions asserted once, each negated goal in a
# push/pop scope). Only `unsat`, and `sat` on a soft claim, is taken from it; a
# `sat` on a hard claim is re-decided by a standalone full query, which alone
# supplies the model, and anything else falls back to standalone queries.
no_survivors() {
    local what="$1" p
    [ -f "$BW_DIR/spids" ] || return 0
    while read -r p; do
        case "$(ps -p "$p" -o stat= 2>/dev/null | tr -d ' ')" in
            ""|Z*) ;;
            *) fail "$what: solver session $p survived"; kill -9 "$p" 2>/dev/null || true ;;
        esac
    done < "$BW_DIR/spids"
}

run_incremental unsat "$THREE_CLAIMS"
expect "session unsat status" "$(field "$RUN_OUT" status)" "Verified"
expect "session unsat exit" "$RUN_RC" "0"
expect "session unsat: one solver process" "$(sessions)" "1"
expect "session unsat: no standalone query" "$(queries)" "0"
expect "session: header sent once" "$(session_lines 1 '^(set-logic')" "1"
expect "session: one check per claim" "$(session_lines 1 '^(check-sat)$')" "3"
expect "session: one scope per claim and the probe" "$(session_lines 1 '^(push 1)$')" "4"
# the last `(pop 1)` is written together with the last reply's question and the
# session is killed once that reply is read, so the solver may never log it.
if [ "$(session_lines 1 '^(pop 1)$')" -lt 3 ]; then fail "session: scopes are not closed"; fi
expect "session: declarations sent once" "$(session_lines 1 '^(declare-const p0 ')" "1"
expect "session: no model request" "$(session_lines 1 'get-value')" "0"
grep -q -- '--bv-output-format 2' "$BW_DIR/sargs" || fail "session not run with --bv-output-format 2"
no_survivors "session unsat"
no_leftovers "session unsat"

# a hard `sat` is re-decided by one standalone full query: same report, byte for byte.
run_incremental sat "$THREE_CLAIMS"
inc_out="$RUN_OUT"
expect "session sat: one solver process" "$(sessions)" "1"
expect "session sat: one standalone confirmation" "$(queries)" "1"
no_survivors "session sat"
no_leftovers "session sat"
run_native sat "$THREE_CLAIMS"
expect "session sat: report equals the standalone report" "$inc_out" "$RUN_OUT"
expect "session sat status" "$(field "$inc_out" status)" "VerifyFailed"
expect "session sat model" "$(field "$inc_out" counterexamples.0.values.a)" "7"

# a session that says `unknown` leaves the claim to the standalone queries.
run_incremental session_unknown "$THREE_CLAIMS"
expect "session unknown status" "$(field "$RUN_OUT" status)" "Verified"
expect "session unknown: every claim falls back" "$(queries)" "3"
no_survivors "session unknown"
no_leftovers "session unknown"

# a solver that dies mid-round: the claims it answered stand, the rest are
# standalone, and the verdict is the one-shot verdict.
run_incremental session_die_after_1 "$THREE_CLAIMS"
expect "session death status" "$(field "$RUN_OUT" status)" "Verified"
expect "session death exit" "$RUN_RC" "0"
expect "session death: one session" "$(sessions)" "1"
expect "session death: later claims standalone" "$(queries)" "2"
no_survivors "session death"
no_leftovers "session death"

# a solver that cannot run a session (no push/pop, an error line, junk) never
# decides a claim: every claim is standalone and the verdict is unchanged.
for mode in session_noscope session_error_line session_die_after_0; do
    run_incremental "$mode" "$THREE_CLAIMS"
    expect "$mode status" "$(field "$RUN_OUT" status)" "Verified"
    expect "$mode: every claim standalone" "$(queries)" "3"
    no_survivors "$mode"
    no_leftovers "$mode"
done

# the failure modes of the one-shot path hold with a session in front of it.
for mode in garbage exit1; do
    run_incremental "$mode" "$THREE_CLAIMS"
    expect "session $mode verify_status" "$(field "$RUN_OUT" verify_status)" "error"
    expect "session $mode exit" "$RUN_RC" "1"
    no_survivors "session $mode"
    no_leftovers "session $mode"
done
for mode in kill_worker oom_msg; do
    run_incremental "$mode" "$THREE_CLAIMS"
    expect "session $mode verify_status" "$(field "$RUN_OUT" verify_status)" "unknown"
    case "$(field "$RUN_OUT" verify_message)" in
        *"memory limit exceeded"*) ;;
        *) fail "session $mode lacks a memory reason: $(field "$RUN_OUT" verify_message)" ;;
    esac
    no_survivors "session $mode"
    no_leftovers "session $mode"
done
run_incremental crash_worker "$THREE_CLAIMS"
expect "session crash verify_status" "$(field "$RUN_OUT" verify_status)" "panicked"

# a hung session is killed at the budget, with the same status as without one.
start=$(date +%s)
run_incremental hang "$THREE_CLAIMS" --timeout 1
elapsed=$(( $(date +%s) - start ))
expect "session hang verify_status" "$(field "$RUN_OUT" verify_status)" "timeout"
if [ "$elapsed" -gt 10 ]; then fail "session hang took ${elapsed}s with --timeout 1"; fi
all_dead "$BW_DIR/spids" "hung session"
all_dead "$BW_DIR/pid" "hung solver"
no_leftovers "session hang"

# a round with a single claim for the solver has nothing to share.
run_incremental unsat "$ONE_CLAIM"
expect "single claim: verified" "$(field "$RUN_OUT" status)" "Verified"
expect "single claim: no session" "$(sessions)" "0"
expect "single claim: one standalone query" "$(queries)" "1"

# a function with a loop runs one session per unwinding round.
run_incremental unsat "$LOOP_CLAIMS"
expect "loop session status" "$(field "$RUN_OUT" status)" "Verified"
expect "loop session: no standalone query" "$(queries)" "0"
no_survivors "loop session"
no_leftovers "loop session"

# inlined calls: the same report with and without sessions.
run_incremental sat "$CALL_CLAIMS" --verify-jobs 1
inc_out="$RUN_OUT"
run_native sat "$CALL_CLAIMS" --verify-jobs 1
expect "calls: report equals the standalone report" "$inc_out" "$RUN_OUT"
no_survivors "calls"
no_leftovers "calls session"

# Per-clause mode with a session in front: every claim of the function is
# asked, a hard `sat` leaves the session usable for the claims after it, and
# the verdicts are the standalone ones.
CLAUSES_OUT="$TMP_ROOT/clauses_out"
VOW_VERIFY_NO_INCREMENTAL=0 worker_clauses unsat --worker-clauses > "$CLAUSES_OUT"
expect "session clauses all proven" "$(cat "$CLAUSES_OUT")" "0 -1 2:1,3:1"
expect "session clauses: one solver process" "$(sessions)" "1"
expect "session clauses: no standalone query" "$(queries)" "0"
no_survivors "session clauses"
no_leftovers "session clauses"
VOW_VERIFY_NO_INCREMENTAL=0 worker_clauses sat --worker-clauses > "$CLAUSES_OUT"
expect "session clauses sat: overall result" "$(cut -d' ' -f1,2 "$CLAUSES_OUT")" "1 2"
expect "session clauses sat: one solver process" "$(sessions)" "1"
expect "session clauses sat: each claim asked in the session" "$(session_lines 1 '^(check-sat)$')" "2"
expect "session clauses sat: each hard sat confirmed standalone" "$(queries)" "2"
no_survivors "session clauses sat"
no_leftovers "session clauses sat"

# The result cache in front of a session: an `unsat` the session decided is
# stored under the claim's standalone query text, so a warm run spawns neither a
# session nor a solver, and a standalone run on the same cache replays it too.
new_cache
VOW_VERIFY_NO_INCREMENTAL=0 run_native_cached unsat "fake 1.0" "$THREE_CLAIMS"
cold_out="$RUN_OUT"
expect "cached session cold status" "$(field "$RUN_OUT" status)" "Verified"
expect "cached session cold: one session" "$(sessions)" "1"
expect "cached session cold: no standalone query" "$(queries)" "0"
[ "$(cache_entries)" -gt 0 ] || fail "cached session cold: nothing stored"
no_survivors "cached session cold"
VOW_VERIFY_NO_INCREMENTAL=0 run_native_cached unsat "fake 1.0" "$THREE_CLAIMS"
expect "cached session warm: no session" "$(sessions)" "0"
expect "cached session warm: no standalone query" "$(queries)" "0"
expect "cached session warm json" "$RUN_OUT" "$cold_out"
run_native_cached unsat "fake 1.0" "$THREE_CLAIMS"
expect "cached session entries serve a standalone run" "$(queries)" "0"
expect "cached session entries standalone json" "$RUN_OUT" "$cold_out"
no_leftovers "cached session"

# A hard `sat` is re-decided by one standalone full query; its model is cached,
# so the warm run reports the same counterexample without asking anything.
new_cache
VOW_VERIFY_NO_INCREMENTAL=0 run_native_cached sat "fake 1.0" "$THREE_CLAIMS"
cold_out="$RUN_OUT"
expect "cached session sat cold: one confirmation" "$(queries)" "1"
expect "cached session sat cold status" "$(field "$RUN_OUT" status)" "VerifyFailed"
VOW_VERIFY_NO_INCREMENTAL=0 run_native_cached sat "fake 1.0" "$THREE_CLAIMS"
expect "cached session sat warm: no session" "$(sessions)" "0"
expect "cached session sat warm: no standalone query" "$(queries)" "0"
expect "cached session sat warm json" "$RUN_OUT" "$cold_out"
no_survivors "cached session sat"
no_leftovers "cached session sat"

if [ "$failures" -ne 0 ]; then
    echo "verify-native: $failures failure(s)" >&2
    exit 1
fi
echo "verify-native: ok"
