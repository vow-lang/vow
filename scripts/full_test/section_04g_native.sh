# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

# ─── Section 4g: Native Verifier (tests/verify-native/, #1408) ────
#
# The native verifier, the default backend of the self-hosted compiler, exists only
# there (epic #1398, ADR-2026-10-08-1421), so there is no Rust twin and no parity check. The
# wiring tier uses a fake Bitwuzla and always runs; the real-solver tier is
# skipped, not passed, when `bitwuzla` is not on PATH.
# tests/verify-native/ is deliberately outside Section 2c's explicit globs.

section_begin "Section 4g: Native Verifier (self-hosted only)"
native_wiring_log="$TMPDIR/verify-native-wiring.log"
if VOWC_BIN="$SELF" bash tests/verify-native/tests.sh >"$native_wiring_log" 2>&1; then
    pass "verify-native/wiring"
else
    fail "verify-native/wiring" "$(tail -20 "$native_wiring_log")"
fi

native_default_log="$TMPDIR/verify-native-default-backend.log"
if VOWC_BIN="$SELF" bash tests/verify-native/default-backend/tests.sh >"$native_default_log" 2>&1; then
    pass "verify-native/default-backend"
else
    fail "verify-native/default-backend" "$(tail -20 "$native_default_log")"
fi

native_have_solver=0
command -v bitwuzla >/dev/null 2>&1 && native_have_solver=1
# Every fixture's `// TEST:` directives (pass/, fail/, unknown/, skip/, contracts/) and the cache
# non-vacuity probe are checked by scripts/check_native_fixtures.sh, which pull requests run too
# (.github/workflows/native-fixtures.yml); its verdict lines become this suite's results.
run_shared_check "verify-native/fixtures" scripts/check_native_fixtures.sh "$SELF"
# Callee summaries (#1791): tests/verify-native/summary/run.sh decides each fixture with the
# inlining budget lowered so a proven callee is summarised, and with the default budget. A run
# that prints fewer lines than there are fixtures (a crash, a missing binary) fails.
if [ "$native_have_solver" -eq 0 ]; then
    skip "verify-native/summary fixtures" "bitwuzla not on PATH"
else
    summary_want=$(find tests/verify-native/summary -maxdepth 1 -name '*.vow' | wc -l)
    summary_got=0
    while IFS= read -r summary_line; do
        case "$summary_line" in
            PASS\ *) pass "verify-native/${summary_line#PASS }"; summary_got=$((summary_got + 1)) ;;
            FAIL\ *) fail "verify-native/${summary_line#FAIL }" "see tests/verify-native/summary/run.sh"; summary_got=$((summary_got + 1)) ;;
        esac
    done < <(bash tests/verify-native/summary/run.sh "$SELF" 2>/dev/null)
    if [ "$summary_got" -ne "$summary_want" ]; then
        fail "verify-native/summary" "run.sh reported $summary_got of $summary_want fixtures"
    fi
fi
# Proven callees whose claims are not decided again (#1803): tests/verify-native/trust/run.sh decides
# each fixture with the default size and with every caller waiting for its callees' proofs; both
# must give the verdict and counterexamples the fixture names.
if [ "$native_have_solver" -eq 0 ]; then
    skip "verify-native/trust fixtures" "bitwuzla not on PATH"
else
    trust_want=$(find tests/verify-native/trust -maxdepth 1 -name '*.vow' | wc -l)
    trust_got=0
    while IFS= read -r trust_line; do
        case "$trust_line" in
            PASS\ *) pass "verify-native/${trust_line#PASS }"; trust_got=$((trust_got + 1)) ;;
            FAIL\ *) fail "verify-native/${trust_line#FAIL }" "see tests/verify-native/trust/run.sh"; trust_got=$((trust_got + 1)) ;;
        esac
    done < <(bash tests/verify-native/trust/run.sh "$SELF" 2>/dev/null)
    if [ "$trust_got" -ne "$trust_want" ]; then
        fail "verify-native/trust" "run.sh reported $trust_got of $trust_want fixtures"
    fi
fi
# Cost of reading a pointer element back (#1825): tests/verify-native/perf/run.sh decides each
# fixture with the real solver; the verdict and the wall time must be the ones its
# `// TEST: perf-expect` directive names.
if [ "$native_have_solver" -eq 0 ]; then
    skip "verify-native/perf fixtures" "bitwuzla not on PATH"
else
    perf_want=$(find tests/verify-native/perf -maxdepth 1 -name '*.vow' | wc -l)
    perf_got=0
    while IFS= read -r perf_line; do
        case "$perf_line" in
            PASS\ *) pass "verify-native/${perf_line#PASS }"; perf_got=$((perf_got + 1)) ;;
            FAIL\ *) fail "verify-native/${perf_line#FAIL }" "see tests/verify-native/perf/run.sh"; perf_got=$((perf_got + 1)) ;;
        esac
    done < <(bash tests/verify-native/perf/run.sh "$SELF" 2>/dev/null)
    if [ "$perf_got" -ne "$perf_want" ]; then
        fail "verify-native/perf" "run.sh reported $perf_got of $perf_want fixtures"
    fi
fi
# Every code of the closed `Skipped` list is pinned by a skip/ fixture (or an
# explicit exemption) and the list matches ADR-1430 section 4.
skip_codes_log="$TMPDIR/verify-native-skip-codes.log"
if python3 scripts/check_skip_codes.py >"$skip_codes_log" 2>&1; then
    pass "verify-native/skip-code-coverage"
else
    fail "verify-native/skip-code-coverage" "$(cat "$skip_codes_log")"
fi
echo ""
