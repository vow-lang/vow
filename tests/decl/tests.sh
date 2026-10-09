#!/usr/bin/env bash
set -euo pipefail

# `decl` emits a `.vow.d` declaration stub (issue #595). Both compilers must
# write byte-identical text: the goldens (`*.vow.d.expected`) were generated
# once from the Rust compiler and reviewed by hand.
#
#   VOWC_BIN=build/vowc            bash tests/decl/tests.sh
#   VOWC_BIN=target/release/vow VOWC_KIND=rust bash tests/decl/tests.sh

VOWC_BIN="${VOWC_BIN:-build/vowc}"
VOWC_BIN=$(cd "$(dirname "$VOWC_BIN")" && pwd)/$(basename "$VOWC_BIN")
VOWC_KIND="${VOWC_KIND:-self}"
HERE=$(cd "$(dirname "$0")" && pwd)
TMP_ROOT=$(mktemp -d)
trap 'rm -rf "$TMP_ROOT"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

failures=0
fail() { echo "FAIL: $*" >&2; failures=$((failures + 1)); }

for case in fn_basic types multi/main; do
    out="$TMP_ROOT/$(basename "$case").vow.d"
    rc=0
    stdout=$("$VOWC_BIN" decl "$HERE/$case.vow" -o "$out" 2>"$TMP_ROOT/err") || rc=$?
    if [ "$rc" -ne 0 ]; then fail "$case: exit $rc: $(cat "$TMP_ROOT/err")"; continue; fi
    if [ -n "$stdout" ]; then fail "$case: stdout must be empty, got: $stdout"; fi
    grep -q "^wrote $out\$" "$TMP_ROOT/err" || fail "$case: stderr lacks 'wrote $out': $(cat "$TMP_ROOT/err")"
    if ! diff -u "$HERE/$case.vow.d.expected" "$out" >"$TMP_ROOT/diff"; then
        fail "$case: output differs from golden:
$(cat "$TMP_ROOT/diff")"
    fi
done

# Default output path: `<source>.d`.
cp "$HERE/fn_basic.vow" "$TMP_ROOT/dflt.vow"
"$VOWC_BIN" decl "$TMP_ROOT/dflt.vow" >/dev/null 2>&1 || fail "default output: decl failed"
[ -f "$TMP_ROOT/dflt.vow.d" ] || fail "default output: $TMP_ROOT/dflt.vow.d not written"

# Missing source file argument.
rc=0
err=$("$VOWC_BIN" decl 2>&1 >/dev/null) || rc=$?
[ "$rc" -eq 1 ] || fail "missing source: expected exit 1, got $rc"
case "$err" in
    *"vow decl: source file required"*) ;;
    *) fail "missing source: stderr lacks the diagnostic: $err" ;;
esac

# A type error writes no file and reports `vow decl: type error`.
cat >"$TMP_ROOT/bad.vow" <<'VOW'
module Bad

fn f() -> i64 {
    true
}
VOW
rc=0
err=$("$VOWC_BIN" decl "$TMP_ROOT/bad.vow" -o "$TMP_ROOT/bad.vow.d" 2>&1 >/dev/null) || rc=$?
[ "$rc" -eq 1 ] || fail "type error: expected exit 1, got $rc"
case "$err" in
    *"vow decl: type error"*) ;;
    *) fail "type error: stderr lacks 'vow decl: type error': $err" ;;
esac
[ ! -e "$TMP_ROOT/bad.vow.d" ] || fail "type error: a stub was written"

# A missing `use` target is a module load error.
cat >"$TMP_ROOT/nouse.vow" <<'VOW'
module NoUse

use does_not_exist

fn f() -> i64 {
    1
}
VOW
rc=0
err=$("$VOWC_BIN" decl "$TMP_ROOT/nouse.vow" -o "$TMP_ROOT/nouse.vow.d" 2>&1 >/dev/null) || rc=$?
[ "$rc" -eq 1 ] || fail "missing use: expected exit 1, got $rc"
case "$err" in
    *"vow decl: module load error"*) ;;
    *) fail "missing use: stderr lacks 'vow decl: module load error': $err" ;;
esac

# A generated stub is a valid `use` target that still verifies against its
# contract: the importer sees the stub, not the body.
mkdir "$TMP_ROOT/rt"
cp "$HERE/multi/base.vow" "$TMP_ROOT/rt/base.vow"
"$VOWC_BIN" decl "$TMP_ROOT/rt/base.vow" >/dev/null 2>&1 || fail "round trip: decl failed"
mv "$TMP_ROOT/rt/base.vow" "$TMP_ROOT/rt/base.vow.orig"
cat >"$TMP_ROOT/rt/main.vow" <<'VOW'
module Main

use base

fn main() -> i32 [io] {
    print_i64(base_val(7));
    0
}
VOW
"$VOWC_BIN" verify "$TMP_ROOT/rt/main.vow" >/dev/null 2>&1 \
    || fail "round trip: importer does not verify against the generated stub"

# The self-hosted AST drops struct-like enum variant field names, so decl
# refuses to print such an enum instead of emitting a different one.
if [ "$VOWC_KIND" = "self" ]; then
    rc=0
    err=$("$VOWC_BIN" decl "$HERE/struct_variant.vow" -o "$TMP_ROOT/sv.vow.d" 2>&1 >/dev/null) || rc=$?
    [ "$rc" -eq 1 ] || fail "struct variant: expected exit 1, got $rc"
    case "$err" in
        *"enum Msg has a struct-like variant"*) ;;
        *) fail "struct variant: stderr lacks the refusal: $err" ;;
    esac
    [ ! -e "$TMP_ROOT/sv.vow.d" ] || fail "struct variant: a stub was written"
fi

if [ "$failures" -ne 0 ]; then
    echo "$failures failure(s)" >&2
    exit 1
fi
echo "decl: all checks passed"
