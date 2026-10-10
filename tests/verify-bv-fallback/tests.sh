#!/usr/bin/env bash
# Pins the BV -> IR fallback outcomes of the self-hosted verify driver with a
# scripted fake `esbmc`: which stderr line, which soft-fail record and how many
# ESBMC invocations each (BV verdict, IR verdict) pair produces.
set -euo pipefail

VOWC_BIN="${VOWC_BIN:-build/vowc}"
TMP_ROOT=$(mktemp -d)
trap 'rm -rf "$TMP_ROOT"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

FAKE_BIN="$TMP_ROOT/bin"
FIXTURE="$TMP_ROOT/single-ensures.vow"
mkdir -p "$FAKE_BIN"

# Picks its answer from VOW_FAKE_BV (no --ir in argv) or VOW_FAKE_IR (--ir in
# argv); records one "bv"/"ir" line per invocation that was given a .c file.
cat > "$FAKE_BIN/esbmc" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
has_c=0
has_ir=0
for arg in "$@"; do
    case "$arg" in
        *.c) has_c=1 ;;
        --ir) has_ir=1 ;;
    esac
done
if [ "$has_c" -eq 0 ]; then
    exit 0
fi
if [ "$has_ir" -eq 1 ]; then
    echo ir >> "${VOW_FAKE_LOG:?}"
    verdict="${VOW_FAKE_IR:?}"
else
    echo bv >> "${VOW_FAKE_LOG:?}"
    verdict="${VOW_FAKE_BV:?}"
fi
case "$verdict" in
    timeout) printf 'Timed out\n' ;;
    memlimit) printf 'Out of memory: memory limit exceeded\n' ;;
    unknown) printf 'VERIFICATION UNKNOWN\n' ;;
    success) printf 'VERIFICATION SUCCESSFUL\n' ;;
    failed) printf 'VERIFICATION FAILED\n' ;;
    *) printf 'fake esbmc: unknown verdict %s\n' "$verdict" >&2; exit 2 ;;
esac
SH
chmod +x "$FAKE_BIN/esbmc"

cat > "$FIXTURE" <<'VOW'
module SingleEnsures

fn keep(x: i64) -> i64 vow {
    ensures: result == x
} {
    x
}

fn main() -> i32 {
    0
}
VOW

failures=0

# run_case NAME BV IR EXPECTED_STDERR_LINE EXPECTED_STATUS EXPECTED_MESSAGE
#          EXPECTED_INVOCATIONS [extra verify flags...]
run_case() {
    local name="$1" bv="$2" ir="$3" want_line="$4" want_status="$5" want_msg="$6" want_calls="$7"
    shift 7
    local log="$TMP_ROOT/$name.log" err="$TMP_ROOT/$name.stderr" out="$TMP_ROOT/$name.json"
    : > "$log"
    PATH="$FAKE_BIN:$PATH" VOW_FAKE_LOG="$log" VOW_FAKE_BV="$bv" VOW_FAKE_IR="$ir" \
        "$VOWC_BIN" verify --no-cache --verify-jobs 1 "$@" "$FIXTURE" >"$out" 2>"$err" || true

    local calls
    calls=$(wc -l < "$log" | tr -d ' ')
    local ok=1
    if ! grep -qxF -- "  $want_line" "$err"; then
        printf '%s: stderr lacks %q\n' "$name" "$want_line" >&2
        ok=0
    fi
    if [ "$calls" != "$want_calls" ]; then
        printf '%s: expected %s esbmc invocations, got %s (%s)\n' "$name" "$want_calls" "$calls" "$(tr '\n' ' ' < "$log")" >&2
        ok=0
    fi
    if [ -n "$want_status" ]; then
        local got
        got=$(python3 -c '
import json, sys
d = json.load(open(sys.argv[1]))
print(d.get("verify_status", ""), "|", d.get("verify_message", ""), "|", d.get("function", ""))
' "$out")
        local want="$want_status | $want_msg | keep"
        if [ "$got" != "$want" ]; then
            printf '%s: soft-fail record %q, expected %q\n' "$name" "$got" "$want" >&2
            ok=0
        fi
    fi
    if [ "$ok" -eq 0 ]; then
        printf -- '--- %s stderr ---\n' "$name" >&2
        cat "$err" >&2
        printf -- '--- %s stdout ---\n' "$name" >&2
        cat "$out" >&2
        failures=$((failures + 1))
    fi
}

run_case timeout-ir-proves   timeout  success "keep: PROVEN (IR)"          ""        ""                      2
run_case timeout-ir-failed   timeout  failed  "keep: TIMEOUT"              timeout   ""                      2
run_case timeout-ir-timeout  timeout  timeout "keep: TIMEOUT (BV and IR)"  timeout   ""                      2
run_case timeout-ir-unknown  timeout  unknown "keep: UNKNOWN (BV and IR)"  unknown   "ESBMC returned VERIFICATION UNKNOWN" 2
run_case memlimit-ir-proves  memlimit success "keep: PROVEN (IR)"          ""        ""                      2
run_case memlimit-ir-failed  memlimit failed  "keep: UNKNOWN"              unknown   "memory limit exceeded"  2
run_case unknown-no-retry    unknown  success "keep: UNKNOWN"              unknown   "ESBMC returned VERIFICATION UNKNOWN" 1
run_case ir-encoding-timeout timeout  timeout "keep: TIMEOUT"              timeout   ""                      1 --encoding ir
run_case bitwuzla-timeout    timeout  success "keep: TIMEOUT"              timeout   ""                      1 --solver bitwuzla

if [ "$failures" -ne 0 ]; then
    printf '%s bv-fallback case(s) failed\n' "$failures" >&2
    exit 1
fi

printf 'bv to ir fallback outcomes pinned\n'
