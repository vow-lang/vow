#!/usr/bin/env bash
set -euo pipefail

# Unknown-flag handling shared by both compilers (issue #580). The Rust driver
# gets it from clap; the self-hosted driver from compiler/cli_flags.vow. Both
# must exit 2 with `unexpected argument '<flag>' found` so a stale
# `--vec-max 10000` or a typo is rejected instead of silently ignored.
#
#   VOWC_BIN=build/vowc            bash tests/cli-flags/tests.sh
#   VOWC_BIN=target/release/vow VOWC_KIND=rust bash tests/cli-flags/tests.sh

VOWC_BIN="${VOWC_BIN:-build/vowc}"
VOWC_BIN=$(cd "$(dirname "$VOWC_BIN")" && pwd)/$(basename "$VOWC_BIN")
VOWC_KIND="${VOWC_KIND:-self}"
FIXTURE="examples/hello.vow"
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

unknown_flag() {
    local flag="$1"; shift
    local err rc=0
    err=$("$VOWC_BIN" "$@" 2>&1 >/dev/null) || rc=$?
    expect "exit for: $*" "$rc" "2"
    case "$err" in
        *"unexpected argument '$flag' found"*) ;;
        *) fail "stderr for '$*' lacks the unexpected-argument error: $err" ;;
    esac
}

for flag in --vec-max --string-max --hashmap-max --btreemap-max; do
    unknown_flag "$flag" build "$flag" 10000 "$FIXTURE"
    unknown_flag "$flag" verify "$flag" 10000 "$FIXTURE"
    unknown_flag "$flag" test "$flag" 10000 "$FIXTURE"
    unknown_flag "$flag" contracts "$flag" 10000 "$FIXTURE"
    unknown_flag "$flag" "$flag" 10000 "$FIXTURE"
done

unknown_flag --bogus build --bogus "$FIXTURE"
unknown_flag --bogus build "$FIXTURE" --bogus
unknown_flag --bogus complexity --bogus "$FIXTURE"
unknown_flag --bogus skill print --bogus
unknown_flag --verify complexity --verify "$FIXTURE"
unknown_flag --mode verify --mode debug "$FIXTURE"
unknown_flag -o verify -o "$TMP_ROOT/x" "$FIXTURE"
unknown_flag --no-verify test --no-verify "$FIXTURE"

# Rust (clap) accepts `--flag=value`; the self-hosted driver does not.
if [ "$VOWC_KIND" = "self" ]; then
    unknown_flag --mode=debug build --mode=debug "$FIXTURE"
    unknown_flag --backend=native verify --backend=native "$FIXTURE"
    err=$("$VOWC_BIN" build --mode=debug "$FIXTURE" 2>&1 >/dev/null) || true
    case "$err" in
        *"--mode <value>"*) ;;
        *) fail "--mode=debug lacks the '<flag> <value>' tip: $err" ;;
    esac

    rc=0
    err=$("$VOWC_BIN" build "$FIXTURE" --mode 2>&1 >/dev/null) || rc=$?
    expect "exit for a value flag with no value" "$rc" "2"
    case "$err" in
        *"a value is required for '--mode'"*) ;;
        *) fail "missing-value error lacks the flag name: $err" ;;
    esac

    # mutants owns its own flag set and is not validated by the driver.
    rc=0
    "$VOWC_BIN" mutants list --bogus-mutants-flag >/dev/null 2>&1 || rc=$?
    if [ "$rc" -eq 2 ]; then fail "mutants must not be rejected by the driver flag validator"; fi
fi

# Positive controls: documented flags still work, including `--output`.
for flag in -o --output; do
    out="$TMP_ROOT/hello$flag"
    rc=0
    "$VOWC_BIN" build --no-verify --no-cache "$flag" "$out" "$FIXTURE" >/dev/null 2>&1 || rc=$?
    expect "build $flag exit" "$rc" "0"
    if [ ! -x "$out" ]; then fail "build $flag did not produce $out"; fi
done

rc=0
"$VOWC_BIN" verify --no-cache "$FIXTURE" >/dev/null 2>&1 || rc=$?
if [ "$rc" -eq 2 ]; then fail "verify --no-cache was rejected as a usage error"; fi

# The bare `<file.vow>` form is `build` (issue #596): verify by default, fail
# closed, emit a binary only on success, and reject legacy-only flags.
unknown_flag --emit-c --emit-c "$FIXTURE"
unknown_flag --verify "$FIXTURE" --verify

if command -v esbmc >/dev/null 2>&1; then
    bare_status() {
        python3 -I -c 'import json,sys; print(json.loads(sys.stdin.read().strip().splitlines()[-1])["status"])'
    }
    BAD="tests/verify-fail/verify_jobs_ce_before_soft.vow"
    GOOD="tests/verify/clamp.vow"

    rc=0
        out=$("$VOWC_BIN" --no-cache "$BAD" -o "$TMP_ROOT/bare_bad" 2>/dev/null) || rc=$?
    if [ "$rc" -eq 0 ]; then fail "bare form exited 0 on a contract violation"; fi
    expect "bare form status on a contract violation" "$(printf '%s' "$out" | bare_status)" "VerifyFailed"

    rc=0
    rm -f "$TMP_ROOT/bare_good"
    out=$("$VOWC_BIN" --no-cache "$GOOD" -o "$TMP_ROOT/bare_good" 2>/dev/null) || rc=$?
    expect "bare form exit on a verified program" "$rc" "0"
    expect "bare form status on a verified program" "$(printf '%s' "$out" | bare_status)" "Verified"
    if [ ! -x "$TMP_ROOT/bare_good" ]; then fail "bare form did not emit a binary for a verified program"; fi
else
    echo "cli-flags: esbmc not found; skipping bare-form verification checks" >&2
fi

rc=0
rm -f "$TMP_ROOT/bare_nv"
out=$("$VOWC_BIN" --no-verify --no-cache "$FIXTURE" -o "$TMP_ROOT/bare_nv" 2>/dev/null) || rc=$?
expect "bare --no-verify exit" "$rc" "0"
if [ ! -x "$TMP_ROOT/bare_nv" ]; then fail "bare --no-verify did not emit a binary"; fi

if [ "$failures" -ne 0 ]; then
    echo "cli-flags: $failures failure(s)" >&2
    exit 1
fi
echo "cli-flags: ok"
