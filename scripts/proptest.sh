#!/usr/bin/env bash
# Property-based test runner for the Vow compiler.
#
# Usage:
#   ./scripts/proptest.sh              # Run all property tests with default case count
#   ./scripts/proptest.sh --cases 1000 # Run with more cases (for CI or deep testing)
#   ./scripts/proptest.sh --crate vow-syntax  # Run only vow-syntax property tests
#   ./scripts/proptest.sh --minimize   # Run with minimal cases for quick smoke test
#
# For agentic use:
#   The script exits 0 on success, non-zero on failure.
#   Failures include the shrunk minimal counterexample in stderr.
#   Pass PROPTEST_CASES=N as env var to override case count.

set -euo pipefail

CASES="${PROPTEST_CASES:-}"
CRATE=""
MINIMIZE=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --cases)
            CASES="$2"
            shift 2
            ;;
        --crate)
            CRATE="$2"
            shift 2
            ;;
        --minimize)
            MINIMIZE=true
            shift
            ;;
        --help|-h)
            head -13 "$0" | tail -11
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            exit 1
            ;;
    esac
done

# Build environment
export RUST_BACKTRACE=1

# Set proptest configuration via env vars
if [[ -n "$CASES" ]]; then
    export PROPTEST_CASES="$CASES"
fi

if [[ "$MINIMIZE" == "true" ]]; then
    export PROPTEST_CASES=10
fi

echo "=== Vow Property-Based Tests ==="
echo "Cases per test: ${PROPTEST_CASES:-default (see test config)}"
echo ""

FAILED=0

run_test_binary() {
    local crate="$1"
    local test_bin="$2"
    echo "--- $crate::$test_bin ---"
    if cargo test -p "$crate" --test "$test_bin" 2>&1; then
        echo "  PASS: $crate::$test_bin"
    else
        echo "  FAIL: $crate::$test_bin" >&2
        FAILED=1
    fi
    echo ""
}

if [[ -n "$CRATE" ]]; then
    case "$CRATE" in
        vow-syntax)
            run_test_binary "vow-syntax" "proptest_roundtrip"
            ;;
        vow-types)
            run_test_binary "vow-types" "proptest_typecheck"
            ;;
        vow-codegen)
            run_test_binary "vow-codegen" "proptest_pipeline"
            ;;
        *)
            echo "Unknown crate: $CRATE" >&2
            exit 1
            ;;
    esac
else
    run_test_binary "vow-syntax" "proptest_roundtrip"
    run_test_binary "vow-types" "proptest_typecheck"
    run_test_binary "vow-codegen" "proptest_pipeline"
fi

if [[ "$FAILED" -eq 0 ]]; then
    echo "=== All property tests passed ==="
else
    echo "=== Some property tests FAILED ===" >&2
    echo ""
    echo "To reproduce failures, re-run the failing test — proptest auto-replays"
    echo "from .proptest-regressions files saved in each crate."
    exit 1
fi
