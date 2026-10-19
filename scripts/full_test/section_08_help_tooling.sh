# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

# ─── Section 8: Help Output ────────────────────────────────────────

section_begin "Section 8: Help Output"

# --help → valid JSON with "tool" key
rust_help=$($RUST --help 2>/dev/null) || true
self_help=$(run_self --help 2>/dev/null) || true

help_ok=true
for name_src in "rust:$rust_help" "self:$self_help"; do
    src="${name_src%%:*}"
    json="${name_src#*:}"
    if ! python3 -c "
import json, sys
try:
    d = json.loads(sys.argv[1])
    assert 'tool' in d, 'missing tool key'
except Exception as e:
    print(str(e)); sys.exit(1)
" "$json" 2>/dev/null; then
        help_ok=false
    fi
done
if $help_ok; then
    pass "help/json"
else
    fail "help/json" "JSON help missing 'tool' key or invalid JSON"
fi

# --help --human → text containing USAGE
rust_human=$($RUST --help --human 2>/dev/null) || true
self_human=$(run_self --help --human 2>/dev/null) || true

if [[ "$rust_human" == *USAGE* ]] && [[ "$self_human" == *USAGE* ]]; then
    pass "help/human"
else
    fail "help/human" "human help output missing USAGE"
fi

# help/coverage-rust: cross-reference grammar.md → Rust --help
if uv run python scripts/check_help_coverage.py docs/spec/grammar.md "$rust_help" 2>/dev/null; then
    pass "help/coverage-rust"
else
    fail "help/coverage-rust" "Rust --help missing grammar.md features"
fi

# help/coverage-self: cross-reference grammar.md → self-hosted --help
if uv run python scripts/check_help_coverage.py docs/spec/grammar.md "$self_help" 2>/dev/null; then
    pass "help/coverage-self"
else
    fail "help/coverage-self" "self-hosted --help missing grammar.md features"
fi

# help/skills-dir-drift: confirm skills/vow/ matches what generate_help.py
# would produce (so `npx skills add vow-lang/vow` keeps installing the live skill).
if uv run python scripts/generate_help.py --check >/dev/null 2>&1; then
    pass "help/skills-dir-drift"
else
    fail "help/skills-dir-drift" "skills/vow/ drifted from generated content; run 'uv run python scripts/generate_help.py'"
fi

# ops/catalogue-drift: confirm the Operation Catalogue's splice targets and
# doc-fact cross-checks match docs/spec/operations.json.
ops_catalogue_drift_log="$TMPDIR/ops_catalogue_drift.log"
if uv run python scripts/generate_operations.py --check >"$ops_catalogue_drift_log" 2>&1; then
    pass "ops/catalogue-drift"
else
    fail "ops/catalogue-drift" "$(cat "$ops_catalogue_drift_log"); run 'uv run python scripts/generate_operations.py'"
fi

# ops/catalogue-generator-tests: the generator's own unit tests (loader
# validation, projection and fact-table determinism). ci.yml runs them too; the
# local harness must not let a generator regression pass here.
ops_catalogue_tests_log="$TMPDIR/ops_catalogue_generator_tests.log"
if uv run python scripts/test_generate_operations.py >"$ops_catalogue_tests_log" 2>&1; then
    pass "ops/catalogue-generator-tests"
else
    fail "ops/catalogue-generator-tests" "$(tail -40 "$ops_catalogue_tests_log")"
fi
echo ""

# ─── Section 8a: Memory Bounds ─────────────────────────────────────
#
# bench/memory/programs/*.vow loop 10^5-10^6 times over allocation shapes that
# must not grow resident memory. Built and run once per compiler, so a per-call
# leak in either region inference or either lowering fails here, not in a
# long-running service. Linux only (peak RSS is sampled from /proc).

section_begin "Section 8a: Memory Bounds"
if [ "$(uname -s)" = "Linux" ]; then
    for memory_label in rust self; do
        if [ "$memory_label" = "rust" ]; then memory_compiler="$RUST"; else memory_compiler="$SELF"; fi
        memory_bounds_log="$TMPDIR/memory_bounds_${memory_label}.log"
        if python3 scripts/check_memory_bounds.py --compiler "$memory_compiler" >"$memory_bounds_log" 2>&1; then
            pass "memory/bounds-${memory_label}"
        else
            fail "memory/bounds-${memory_label}" "$(grep -v '"status": "pass"' "$memory_bounds_log" | tail -8)"
        fi
    done
else
    skip "memory/bounds" "peak RSS is sampled from /proc (Linux only)"
fi
echo ""

# ─── Section 8b: Tooling Scripts ──────────────────────────────────

section_begin "Section 8b: Tooling Scripts"

install_toolchain_log="$TMPDIR/install_toolchain_tests.log"
if bash tests/install_toolchain/tests.sh >"$install_toolchain_log" 2>&1; then
    pass "install-toolchain/smoke"
else
    fail "install-toolchain/smoke" "$(tail -20 "$install_toolchain_log")"
fi

bootstrap_log="$TMPDIR/bootstrap_tests.log"
if bash tests/bootstrap/tests.sh >"$bootstrap_log" 2>&1; then
    pass "bootstrap/smoke"
else
    fail "bootstrap/smoke" "$(tail -20 "$bootstrap_log")"
fi

release_log="$TMPDIR/release_tests.log"
if python3 tests/test_release.py >"$release_log" 2>&1; then
    pass "release/smoke"
else
    fail "release/smoke" "$(tail -20 "$release_log")"
fi

measure_bootstrap_rss_probe="$TMPDIR/measure_bootstrap_rss_time_probe.log"
if /usr/bin/time -v -o "$measure_bootstrap_rss_probe" true >/dev/null 2>&1 \
    && grep -q "Maximum resident set size" "$measure_bootstrap_rss_probe"; then
    measure_bootstrap_rss_log="$TMPDIR/measure_bootstrap_rss_tests.log"
    if bash tests/measure_bootstrap_rss/tests.sh >"$measure_bootstrap_rss_log" 2>&1; then
        pass "measure-bootstrap-rss/smoke"
    else
        fail "measure-bootstrap-rss/smoke" "$(tail -20 "$measure_bootstrap_rss_log")"
    fi
else
    skip "measure-bootstrap-rss/smoke" "requires GNU-compatible /usr/bin/time -v"
fi
echo ""
