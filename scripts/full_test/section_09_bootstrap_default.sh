# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

# ─── Section 9: Bootstrap Triple Test ──────────────────────────────

section_begin "Section 9: Bootstrap Triple Test"

bootstrap_harness_log="$TMPDIR/full_test_bootstrap_tests.log"
if bash tests/full_test_bootstrap/tests.sh >"$bootstrap_harness_log" 2>&1; then
    pass "bootstrap/harness-tests"
else
    fail "bootstrap/harness-tests" "$(tail -20 "$bootstrap_harness_log")"
fi

run_bootstrap_triple "$RUST" "scripts/concat_vow.sh"
echo ""

# ─── Section 10: Build + Verify Default Mode ───────────────────────

section_begin "Section 10: Build + Verify Default Mode"

for name in clamp max callee_blame cegis_broken; do
    vow_file="examples/${name}.vow"

    rust_json="" self_json="" rust_exit=0 self_exit=0
    rust_json=$($RUST build "$vow_file" -o "$TMPDIR/rust_bv_${name}" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self build "$vow_file" -o "$TMPDIR/self_bv_${name}" 2>/dev/null) || self_exit=$?

    if check_empty_output "${name}/build-verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit"; then
        continue
    fi

    compare_json "${name}/build-verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$vow_file"
done
echo ""
