#!/usr/bin/env bash
set -euo pipefail

# Frontend diagnostic emission under stderr failures (#944 for the Rust driver,
# #957 for the self-hosted one). A stderr that cannot be written must not abort
# the build with a signal or silently succeed: BrokenPipe leaves the frontend
# result unchanged, any other failure becomes a structured CompileFailed whose
# `message` starts with `failed to emit frontend diagnostics`.
#
#   VOWC_BIN=build/vowc            bash tests/diag-io/tests.sh
#   VOWC_BIN=target/release/vow VOWC_KIND=rust bash tests/diag-io/tests.sh

VOWC_BIN="${VOWC_BIN:-build/vowc}"
VOWC_BIN=$(cd "$(dirname "$VOWC_BIN")" && pwd)/$(basename "$VOWC_BIN")
VOWC_KIND="${VOWC_KIND:-self}"
BAD="tests/error/assign_mismatch.vow"
GOOD="tests/diag-io/note_only.vow"
TMP_ROOT=$(mktemp -d)
trap 'rm -rf "$TMP_ROOT"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

failures=0
fail() { echo "FAIL: $*" >&2; failures=$((failures + 1)); }
expect() {
    if [ "$2" != "$3" ]; then fail "$1: expected '$3', got '$2'"; fi
}

# Prints `<status>|<n diagnostics>|<message>` for the JSON on stdin, or `invalid`.
summarize() {
    python3 -I -c '
import json, sys
try:
    d = json.load(sys.stdin)
except ValueError:
    print("invalid"); sys.exit(0)
print("%s|%d|%s" % (d.get("status"), len(d.get("diagnostics", [])), d.get("message", "")))
'
}

PREFIX="failed to emit frontend diagnostics"

# `2>/dev/full` fails every stderr write with ENOSPC.
full_disk_case() {
    local label="$1" fixture="$2" want_diags="$3"; shift 3
    local out rc=0 sum
    out=$("$VOWC_BIN" "$@" "$fixture" 2>/dev/full) || rc=$?
    expect "$label: exit" "$rc" "1"
    sum=$(printf '%s' "$out" | summarize)
    case "$sum" in
        "CompileFailed|"*"|$PREFIX"*) ;;
        *) fail "$label: expected CompileFailed with '$PREFIX' message, got '$sum'" ;;
    esac
    if [ "$want_diags" = "some" ]; then
        case "$sum" in
            "CompileFailed|0|"*) fail "$label: original diagnostics were dropped: $sum" ;;
        esac
    fi
}

for fixture_kind in bad good; do
    if [ "$fixture_kind" = bad ]; then fixture="$BAD"; diags=some; else fixture="$GOOD"; diags=any; fi
    full_disk_case "build/$fixture_kind" "$fixture" "$diags" build --no-verify -o "$TMP_ROOT/out"
    full_disk_case "verify/$fixture_kind" "$fixture" "$diags" verify
    full_disk_case "contracts/$fixture_kind" "$fixture" "$diags" contracts
done

# decl and complexity report through stderr only, so a dead stderr is
# observable through the exit code and the absent output. decl's frontend
# emits no note on a clean source, so only its failing path reaches a write.
rc=0
"$VOWC_BIN" decl -o "$TMP_ROOT/bad.vow.d" "$BAD" 2>/dev/full >/dev/null || rc=$?
expect "decl: exit under a full stderr" "$rc" "1"
if [ -e "$TMP_ROOT/bad.vow.d" ]; then fail "decl wrote a stub for a source with errors"; fi

rc=0
out=$("$VOWC_BIN" complexity "$GOOD" 2>/dev/full) || rc=$?
expect "complexity: exit under a full stderr" "$rc" "1"
expect "complexity: stdout under a full stderr" "$out" ""

# A closed stderr (EBADF) is not a write failure: the run behaves normally.
rc=0
out=$("$VOWC_BIN" build --no-verify -o "$TMP_ROOT/closed" "$GOOD" 2>&-) || rc=$?
expect "closed stderr: exit" "$rc" "0"
sum=$(printf '%s' "$out" | summarize)
case "$sum" in
    "Unverified|"*) ;;
    *) fail "closed stderr: expected Unverified, got '$sum'" ;;
esac

# EPIPE: a stderr pipe whose reader is gone. The child must neither die from
# SIGPIPE nor change its frontend result.
cat > "$TMP_ROOT/epipe.py" <<'PY'
import json, os, subprocess, sys

bin_path, fixture, out_path = sys.argv[1:4]
r, w = os.pipe()
os.close(r)
res = subprocess.run(
    [bin_path, "build", "--no-verify", "-o", out_path, fixture],
    stdout=subprocess.PIPE, stderr=w, text=True,
)
os.close(w)
print(res.returncode)
try:
    d = json.loads(res.stdout)
    print("%s|%d" % (d.get("status"), len(d.get("diagnostics", []))))
except ValueError:
    print("invalid")
PY
epipe=$(python3 -I "$TMP_ROOT/epipe.py" "$VOWC_BIN" "$BAD" "$TMP_ROOT/epipe-out")
expect "epipe (bad source): exit" "$(printf '%s' "$epipe" | sed -n 1p)" "1"
case "$(printf '%s' "$epipe" | sed -n 2p)" in
    "CompileFailed|0") fail "epipe (bad source): original diagnostics were dropped" ;;
    "CompileFailed|"*) ;;
    *) fail "epipe (bad source): expected CompileFailed JSON, got '$(printf '%s' "$epipe" | sed -n 2p)'" ;;
esac
epipe=$(python3 -I "$TMP_ROOT/epipe.py" "$VOWC_BIN" "$GOOD" "$TMP_ROOT/epipe-good")
expect "epipe (good source): exit" "$(printf '%s' "$epipe" | sed -n 1p)" "0"
expect "epipe (good source): status" "$(printf '%s' "$epipe" | sed -n 2p)" "Unverified|1"

if [ "$failures" -ne 0 ]; then
    echo "diag-io ($VOWC_KIND): $failures failure(s)" >&2
    exit 1
fi
echo "diag-io ($VOWC_KIND): ok"
