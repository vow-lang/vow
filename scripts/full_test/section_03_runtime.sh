# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

# ─── Section 3: Runtime Execution ──────────────────────────────────

section_begin "Section 3: Runtime Execution"
for vow_file in examples/*.vow; do
    name=$(basename "$vow_file" .vow)
    if [ "$name" = "divide" ]; then
        skip "${name}/runtime" "division by zero UB in release mode"
        continue
    fi

    # Check if build produced executables (from Section 1 JSON)
    rust_exe=$(python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('executable') or '')" < "$TMPDIR/rust_${name}.json" 2>/dev/null) || rust_exe=""
    self_exe=$(python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('executable') or '')" < "$TMPDIR/self_${name}.json" 2>/dev/null) || self_exe=""

    if [ -z "$rust_exe" ] && [ -z "$self_exe" ]; then
        skip "${name}/runtime" "no executable (library module)"
        continue
    fi
    if [ -z "$rust_exe" ] || [ -z "$self_exe" ]; then
        fail "${name}/runtime" "executable mismatch: rust='${rust_exe:-null}' self='${self_exe:-null}'"
        continue
    fi

    compare_runtime "${name}/runtime" "$TMPDIR/rust_${name}" "$TMPDIR/self_${name}"
done
echo ""

# ─── Section 4: Run Tests (tests/run/) ────────────────────────────

run_promoted_run_tests
echo ""

# ─── Section 4b: Verify Tests (tests/verify/) ─────────────────────

section_begin "Section 4b: Verify Tests"
for vow_file in tests/verify/*.vow; do
    name=$(basename "$vow_file" .vow)

    rust_json="" self_json="" rust_exit=0 self_exit=0
    rust_json=$($RUST verify "$vow_file" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self verify --backend esbmc "$vow_file" 2>/dev/null) || self_exit=$?

    if check_empty_output "${name}/verify-test" "$rust_json" "$self_json" "$rust_exit" "$self_exit"; then
        continue
    fi

    compare_full_json "${name}/verify-test" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$vow_file"
    actual_status=$(python3 -c "import json,sys; print(json.loads(sys.argv[1]).get('status',''))" "$rust_json" 2>/dev/null) || actual_status=""
    if [ -n "$actual_status" ] && [ "$actual_status" != "Verified" ]; then
        fail "${name}/verify-expected-pass" "expected Verified, got $actual_status"
    fi
done
echo ""
