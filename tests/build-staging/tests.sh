#!/usr/bin/env bash
# `build` finishes codegen and linking before any ESBMC process starts (#179).
# Runs against either compiler: VOWC_BIN=target/release/vow or build/vowc.
set -euo pipefail

VOWC_BIN="${VOWC_BIN:-build/vowc}"
TMP_ROOT=$(mktemp -d)
trap 'rm -rf "$TMP_ROOT"' EXIT
# Kill signals re-raise so the EXIT handler above still does the removal:
# EXIT alone does not fire on an untrapped SIGTERM. See scripts/full_test.sh.
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

FAKE_BIN="$TMP_ROOT/bin"
mkdir -p "$FAKE_BIN"

# ESBMC receives its own arguments, not `build`'s -o, so the executable path
# reaches the fake through VOW_TEST_EXE.
cat > "$FAKE_BIN/esbmc" <<'SH'
#!/bin/sh
if [ -e "$VOW_TEST_EXE" ]; then
    echo exe=present >> "$VOW_TEST_LOG"
else
    echo exe=absent >> "$VOW_TEST_LOG"
fi
echo 'VERIFICATION SUCCESSFUL'
SH
chmod +x "$FAKE_BIN/esbmc"

cat > "$TMP_ROOT/ok.vow" <<'VOW'
module StageOk

fn keep(x: i64) -> i64 vow {
    ensures: result == x
} {
    x
}

fn main() -> i32 {
    0
}
VOW

cat > "$TMP_ROOT/codegen_fail.vow" <<'VOW'
module StageCodegenFail

fn keep(x: i64) -> i64 vow {
    ensures: result == x
} {
    x
}

fn remainder(a: f64, b: f64) -> f64 {
    a % b
}

fn main() -> i32 {
    0
}
VOW

fail() {
    printf 'FAIL: %s\n' "$1" >&2
    exit 1
}

json_field() {
    python3 -I -c 'import json, sys; print(json.loads(sys.argv[1]).get(sys.argv[2], ""))' "$1" "$2"
}

# build <name> <fixture> [flags...]: sets BUILD_JSON, BUILD_EXIT, EXE, LOG.
build() {
    local name="$1" fixture="$2"
    shift 2
    EXE="$TMP_ROOT/$name.out"
    LOG="$TMP_ROOT/$name.log"
    rm -f "$EXE" "$LOG"
    BUILD_EXIT=0
    BUILD_JSON=$(PATH="$FAKE_BIN:$PATH" VOW_TEST_EXE="$EXE" VOW_TEST_LOG="$LOG" \
        "$VOWC_BIN" build "$@" "$fixture" -o "$EXE" 2>"$TMP_ROOT/$name.stderr") || BUILD_EXIT=$?
}

build verified "$TMP_ROOT/ok.vow" --no-cache --verify-jobs 1
[ "$BUILD_EXIT" -eq 0 ] || fail "verified build exited $BUILD_EXIT: $(cat "$TMP_ROOT/verified.stderr")"
[ "$(json_field "$BUILD_JSON" status)" = "Verified" ] || fail "expected Verified, got: $BUILD_JSON"
[ -e "$EXE" ] || fail "executable missing after a verified build"
[ -s "$LOG" ] || fail "ESBMC was never launched"
if grep -qv '^exe=present$' "$LOG"; then
    fail "ESBMC started before codegen+link finished: $(tr '\n' ' ' < "$LOG")"
fi

build codegen_fail "$TMP_ROOT/codegen_fail.vow" --no-cache --verify-jobs 1
[ "$BUILD_EXIT" -ne 0 ] || fail "codegen failure exited 0"
[ "$(json_field "$BUILD_JSON" status)" = "CompileFailed" ] || fail "expected CompileFailed, got: $BUILD_JSON"
case "$BUILD_JSON" in
    *CodegenUnsupported*) ;;
    *) fail "expected a CodegenUnsupported diagnostic, got: $BUILD_JSON" ;;
esac
[ ! -e "$LOG" ] || fail "verification started after a codegen failure: $(tr '\n' ' ' < "$LOG")"

build no_verify "$TMP_ROOT/ok.vow" --no-verify
[ "$BUILD_EXIT" -eq 0 ] || fail "--no-verify build exited $BUILD_EXIT"
[ "$(json_field "$BUILD_JSON" status)" = "Unverified" ] || fail "expected Unverified, got: $BUILD_JSON"
[ ! -e "$LOG" ] || fail "--no-verify launched ESBMC"

printf 'codegen and link complete before ESBMC starts\n'
