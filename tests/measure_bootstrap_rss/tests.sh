#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT
# Kill signals re-raise so the EXIT handler above still does the removal:
# EXIT alone does not fire on an untrapped SIGTERM, so a process-group kill
# would strand this scratch tree. See scripts/full_test.sh.
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

fail() {
    echo "FAIL: $1" >&2
    exit 1
}

make_fake_repo() {
    local repo="$1"

    mkdir -p "$repo/scripts" "$repo/build" "$repo/compiler"
    cp scripts/measure_bootstrap_rss.sh "$repo/scripts/measure_bootstrap_rss.sh"
    chmod +x "$repo/scripts/measure_bootstrap_rss.sh"

    cp tests/bootstrap/fake_compiler.sh "$repo/build/vowc"
    chmod +x "$repo/build/vowc"
}

test_verify_measurement_controls_json_field() {
    local repo="$TMPDIR/repo"
    local no_verify_json="$TMPDIR/no_verify.json"
    local with_verify_json="$TMPDIR/with_verify.json"
    local invocations="$TMPDIR/invocations.log"

    make_fake_repo "$repo"

    if ! env -u VOW_RSS_INCLUDE_VERIFY \
        VOW_RSS_SAMPLES=1 \
        VOW_RSS_OUT="$no_verify_json" \
        VOW_BOOTSTRAP_TEST_LOG="$invocations" \
        bash "$repo/scripts/measure_bootstrap_rss.sh" >/dev/null 2>&1; then
        fail "measurement without verify failed"
    fi

    if ! VOW_RSS_INCLUDE_VERIFY=1 \
        VOW_RSS_SAMPLES=1 \
        VOW_RSS_OUT="$with_verify_json" \
        VOW_BOOTSTRAP_TEST_LOG="$invocations" \
        bash "$repo/scripts/measure_bootstrap_rss.sh" >/dev/null 2>&1; then
        fail "measurement with verify failed"
    fi

    if ! python3 - "$no_verify_json" "$with_verify_json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as source:
    no_verify = json.load(source)
with open(sys.argv[2], encoding="utf-8") as source:
    with_verify = json.load(source)

assert list(no_verify) == ["stage2_no_verify_kb", "samples"], no_verify
assert list(with_verify) == [
    "stage2_no_verify_kb",
    "stage2_default_kb",
    "samples",
], with_verify
assert all(type(value) is int for value in no_verify.values()), no_verify
assert all(type(value) is int for value in with_verify.values()), with_verify
PY
    then
        fail "JSON fields did not match the measurements that ran"
    fi
}

test_verify_measurement_controls_json_field

echo "measure-bootstrap-rss tests passed"
