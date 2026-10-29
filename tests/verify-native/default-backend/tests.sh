#!/usr/bin/env bash
set -euo pipefail

# The default verification backend of the self-hosted `vowc` is native (epic
# #1398, issue #1433); `--backend esbmc` opts out. A fake `bitwuzla` and a fake
# `esbmc` first on PATH each log what they are asked, so this tier needs no real
# solver: it checks which of the two every verifying command reaches, and that
# the ESBMC-only flags are usage errors unless `--backend esbmc` is given.
#
#   VOWC_BIN=build/vowc bash tests/verify-native/default-backend/tests.sh

VOWC_BIN="${VOWC_BIN:-build/vowc}"
VOWC_BIN=$(cd "$(dirname "$VOWC_BIN")" && pwd)/$(basename "$VOWC_BIN")
HERE=$(cd "$(dirname "$0")/.." && pwd)
TMP_ROOT=$(mktemp -d)
trap 'rm -rf "$TMP_ROOT"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

FAKE_DIR="$TMP_ROOT/fake"
mkdir -p "$FAKE_DIR"
cp "$HERE/fake_bitwuzla.sh" "$FAKE_DIR/bitwuzla"
printf '#!/bin/sh\nexec %s -p "$@"\n' "$(command -v mkdir)" > "$FAKE_DIR/mkdir"
cat > "$FAKE_DIR/esbmc" <<'SH'
#!/bin/sh
echo call >> "$FAKE_ESBMC_LOG"
echo 'VERIFICATION SUCCESSFUL'
SH
chmod +x "$FAKE_DIR/mkdir" "$FAKE_DIR/bitwuzla" "$FAKE_DIR/esbmc"

CLAIM="$TMP_ROOT/one.vow"
cat > "$CLAIM" <<'SRC'
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

failures=0
fail() { echo "FAIL: $*" >&2; failures=$((failures + 1)); }

# run NAME ARGS...: runs vowc with both fakes on PATH; sets BW_CALLS, ESBMC_CALLS.
run() {
    local name="$1"; shift
    local bw_dir="$TMP_ROOT/bw.$name"
    mkdir -p "$bw_dir"
    export FAKE_ESBMC_LOG="$TMP_ROOT/esbmc.$name.log"
    : > "$FAKE_ESBMC_LOG"
    PATH="$FAKE_DIR:$PATH" TMPDIR="$TMP_ROOT" FAKE_BW_MODE=unsat FAKE_BW_DIR="$bw_dir" \
        VOW_VERIFY_NO_INCREMENTAL=1 "$VOWC_BIN" "$@" >/dev/null 2>&1 || true
    BW_CALLS=$(find "$bw_dir" -name 'q.*.smt2' | wc -l | tr -d ' ')
    ESBMC_CALLS=$(wc -l < "$FAKE_ESBMC_LOG" | tr -d ' ')
}

check() {
    local cmd="$1"; shift
    local cache=(--no-cache)
    # `test` takes no --no-cache.
    if [ "$cmd" = test ]; then cache=(); fi
    run "$cmd-default" "$@" ${cache[@]+"${cache[@]}"} "$CLAIM"
    if [ "$BW_CALLS" -lt 1 ] || [ "$ESBMC_CALLS" -ne 0 ]; then
        fail "$cmd without --backend: want native (bitwuzla=$BW_CALLS esbmc=$ESBMC_CALLS)"
    fi
    run "$cmd-native" "$@" --backend native ${cache[@]+"${cache[@]}"} "$CLAIM"
    if [ "$BW_CALLS" -lt 1 ] || [ "$ESBMC_CALLS" -ne 0 ]; then
        fail "$cmd --backend native: want native (bitwuzla=$BW_CALLS esbmc=$ESBMC_CALLS)"
    fi
    run "$cmd-esbmc" "$@" --backend esbmc ${cache[@]+"${cache[@]}"} "$CLAIM"
    if [ "$BW_CALLS" -ne 0 ] || [ "$ESBMC_CALLS" -lt 1 ]; then
        fail "$cmd --backend esbmc: want ESBMC (bitwuzla=$BW_CALLS esbmc=$ESBMC_CALLS)"
    fi
}

check verify verify
check build build -o "$TMP_ROOT/built"
check bare -o "$TMP_ROOT/bare"
check contracts contracts --verify
check test test --verify

usage_error() {
    local want="$1"; shift
    local err rc=0
    err=$(PATH="$FAKE_DIR:$PATH" "$VOWC_BIN" "$@" 2>&1 >/dev/null) || rc=$?
    if [ "$rc" -ne 1 ]; then fail "usage exit for '$*': $rc"; fi
    case "$err" in
        *"$want"*) ;;
        *) fail "usage error for '$*' lacks '$want': $err" ;;
    esac
}
for flag in "--solver z3" "--max-k-step 5" "--encoding bv"; do
    for cmd in verify build "contracts --verify" "test --verify"; do
        # shellcheck disable=SC2086
        usage_error "ESBMC-only" $cmd $flag "$CLAIM"
    done
done
usage_error "conflicts with --no-verify" build --no-verify --backend native "$CLAIM"
usage_error "requires --verify" contracts --backend native "$CLAIM"

# Without verification the ESBMC-only flags stay inert, as before.
rc=0
PATH="$FAKE_DIR:$PATH" "$VOWC_BIN" build --no-verify --solver z3 --no-cache "$CLAIM" -o "$TMP_ROOT/noverify" >/dev/null 2>&1 || rc=$?
if [ "$rc" -ne 0 ]; then fail "build --no-verify --solver z3 exited $rc"; fi

# A refuted contract reaches the build result with its counterexample (needs the
# real solver: the fake never produces a model).
if command -v bitwuzla >/dev/null 2>&1; then
    WRONG="$TMP_ROOT/wrong.vow"
    sed 's/result - x == 0/result - x == 1/' "$CLAIM" > "$WRONG"
    rc=0
    out=$("$VOWC_BIN" build --no-cache "$WRONG" -o "$TMP_ROOT/wrong" 2>/dev/null) || rc=$?
    got=$(python3 -c 'import json, sys
d = json.loads(sys.argv[1])
print(d["status"], len(d["counterexamples"]))' "$out" 2>/dev/null) || got=""
    if [ "$rc" -ne 1 ] || [ "$got" != "VerifyFailed 1" ]; then
        fail "build of a false contract: exit $rc, '$got'"
    fi
fi

if [ "$failures" -ne 0 ]; then
    echo "$failures failure(s)" >&2
    exit 1
fi
echo "default backend: native, --backend esbmc opts out"
