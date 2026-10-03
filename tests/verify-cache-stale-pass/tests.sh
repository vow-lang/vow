#!/usr/bin/env bash
set -euo pipefail

# Issue #1307: a cache warmed by a passing version of a function must not mask
# a later, contract-breaking edit to that same function, and a genuine cache
# hit on a failing verdict must not be silently reported as a pass. This
# mirrors the #1291 incident shape exactly: version A is span_len's post-#1306
# (correct) body, version B is the pre-#1306 (buggy) body that produced the
# real `ensures result >= 0` counterexample in CI run 35252574778
# (pos=2^62, start=-2^62).
#
# Uses the real `vow` binary, real VerifyCache, and real ESBMC — no fake
# stub — because the scenario depends on the Rust counterexample parser
# accepting ESBMC's actual output. ESBMC itself is wrapped (not faked): the
# wrapper execs the real `esbmc` binary found on PATH and additionally counts
# invocations, so the test can prove a warm-cache run never re-invokes ESBMC
# at all — the only way to confirm the cache-hit branch (not just a second
# independent ESBMC run that happens to agree) is actually what served the
# verdict.

VOWC_BIN="${VOWC_BIN:-./target/release/vow}"
TMP_ROOT=$(mktemp -d)
trap 'rm -rf "$TMP_ROOT"' EXIT
# Kill signals re-raise so the EXIT handler above still does the removal:
# EXIT alone does not fire on an untrapped SIGTERM, so a process-group kill
# would strand this scratch tree. See scripts/full_test.sh.
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

REAL_ESBMC=$(command -v esbmc) || {
    echo "verify-cache-stale-pass: esbmc not found on PATH" >&2
    exit 1
}

WRAP_DIR="$TMP_ROOT/wrap"
mkdir -p "$WRAP_DIR"
CALL_COUNT="$TMP_ROOT/esbmc-call-count"
cat > "$WRAP_DIR/esbmc" <<EOF
#!/usr/bin/env bash
set -euo pipefail
count=0
if [ -f "\$ESBMC_CALL_COUNT" ]; then
    read -r count < "\$ESBMC_CALL_COUNT"
fi
printf '%s\n' "\$((count + 1))" > "\$ESBMC_CALL_COUNT"
exec "$REAL_ESBMC" "\$@"
EOF
chmod +x "$WRAP_DIR/esbmc"

FIXTURE="$TMP_ROOT/probe.vow"
CACHE_DIR="$TMP_ROOT/cache"

run_verify() {
    PATH="$WRAP_DIR:$PATH" ESBMC_CALL_COUNT="$CALL_COUNT" VOW_CACHE_DIR="$CACHE_DIR" \
        "$VOWC_BIN" verify --verify-jobs 1 "$FIXTURE"
}

status_of() {
    python3 -c 'import json, sys; print(json.loads(sys.argv[1]).get("status", ""))' "$1"
}

call_count() {
    if [ -f "$CALL_COUNT" ]; then
        local n
        read -r n < "$CALL_COUNT"
        printf '%s' "$n"
    else
        printf '0'
    fi
}

# Version A: post-#1306 body — requires/ensures tight, checked `-!`. Must verify.
cat > "$FIXTURE" <<'VOW'
module VerifyCacheStalePassProbe

fn span_len_probe(pos: u64, start: i64) -> i64 vow {
    requires: (pos as i64) >= start,
    ensures: result == (pos as i64) - start
} {
    (pos as i64) -! start
}

fn main() -> i32 {
    0
}
VOW

a_json=$(run_verify) || {
    printf 'version A: vow verify exited non-zero\n%s\n' "$a_json" >&2
    exit 1
}
a_status=$(status_of "$a_json")
if [ "$a_status" != "Verified" ]; then
    printf 'version A: expected status Verified, got %s\n%s\n' "$a_status" "$a_json" >&2
    exit 1
fi

# Version B: pre-#1306 body — same function name, same cache dir (now warm
# from version A). Plain wrapping `-` and a weak `ensures result >= 0`
# reproduce the exact #1291 bug. Must fail, not be masked by the warm cache.
# Version B's C source differs from version A's, so this is a fresh cache
# miss (a different content-hash key) and ESBMC runs again.
cat > "$FIXTURE" <<'VOW'
module VerifyCacheStalePassProbe

fn span_len_probe(pos: u64, start: i64) -> i64 vow {
    requires: (pos as i64) >= start,
    ensures: result >= 0
} {
    (pos as i64) - start
}

fn main() -> i32 {
    0
}
VOW

b_exit=0
b_json=$(run_verify) || b_exit=$?
b_status=$(status_of "$b_json")

if [ "$b_exit" -eq 0 ] || [ "$b_status" = "Verified" ]; then
    printf 'version B: cache masked a real contract violation (status=%s, exit=%s)\n%s\n' \
        "$b_status" "$b_exit" "$b_json" >&2
    exit 1
fi

if [ "$b_status" != "VerifyFailed" ]; then
    printf 'version B: expected status VerifyFailed, got %s\n%s\n' "$b_status" "$b_json" >&2
    exit 1
fi

violation=$(python3 -c '
import json, sys
d = json.loads(sys.argv[1])
ces = d.get("counterexamples", [])
print(ces[0].get("violation", "") if ces else "")
' "$b_json")

case "$violation" in
    *"result >= 0"*) ;;
    *)
        printf 'version B: expected violation to mention "result >= 0", got %s\n%s\n' "$violation" "$b_json" >&2
        exit 1
        ;;
esac

count_after_b=$(call_count)

# Version B replay: same buggy fixture, same warm cache, no source change.
# This must hit the cache (content-hash key is identical to the run above),
# which means ESBMC must NOT be invoked again — the only way to confirm a
# real cache hit served this verdict, rather than a second independent ESBMC
# run that happened to agree. This is the literal "a warm cache must not
# turn a real failure into a pass" guarantee the issue asked to confirm.
b2_exit=0
b2_json=$(run_verify) || b2_exit=$?
b2_status=$(status_of "$b2_json")
count_after_b2=$(call_count)

if [ "$b2_exit" -eq 0 ] || [ "$b2_status" = "Verified" ]; then
    printf 'version B replay: warm cache hit turned a real failure into a pass (status=%s, exit=%s)\n%s\n' \
        "$b2_status" "$b2_exit" "$b2_json" >&2
    exit 1
fi

if [ "$b2_status" != "VerifyFailed" ]; then
    printf 'version B replay: expected status VerifyFailed, got %s\n%s\n' "$b2_status" "$b2_json" >&2
    exit 1
fi

if [ "$count_after_b2" -ne "$count_after_b" ]; then
    printf 'version B replay: expected a cache hit (no new ESBMC invocation), but call count went from %s to %s\n' \
        "$count_after_b" "$count_after_b2" >&2
    exit 1
fi

printf 'a cache warmed by a passing version does not mask a later contract-breaking edit, and a warm cache hit on a failing verdict is not silently reported as a pass\n'
