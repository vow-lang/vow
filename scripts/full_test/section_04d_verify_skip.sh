# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

# ─── Section 4d: Verify-Skip Tests (tests/verify-skip/) ───────────
#
# Functions that exercise a non-modelable construct (e.g. nested-collection
# vec ops, issue #505) must be Skipped (fail-closed), not Verified. Such a
# file cannot live under tests/verify/ because Section 4b requires "Verified";
# here each file gives the offending function a contract so it becomes a real
# verification target, then we assert status == "Skipped" with Rust/self
# parity.

section_begin "Section 4d: Verify-Skip Tests"
for vow_file in tests/verify-skip/*.vow; do
    name=$(basename "$vow_file" .vow)

    rust_json="" self_json="" rust_exit=0 self_exit=0
    rust_json=$($RUST verify "$vow_file" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self verify --backend esbmc "$vow_file" 2>/dev/null) || self_exit=$?

    if check_empty_output "${name}/verify-skip-test" "$rust_json" "$self_json" "$rust_exit" "$self_exit"; then
        continue
    fi

    compare_full_json "${name}/verify-skip-test" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$vow_file"
    actual_status=$(python3 -c "import json,sys; print(json.loads(sys.argv[1]).get('status',''))" "$rust_json" 2>/dev/null) || actual_status=""
    if [ -n "$actual_status" ] && [ "$actual_status" != "Skipped" ]; then
        fail "${name}/verify-expected-skip" "expected Skipped, got $actual_status"
    fi
done
echo ""

# ─── Section 4e: Verifier-Evaluation Suite (issue #334) ───────────
#
# The verifier's *acceptance* harness, distinct from the Rust/self parity
# checks above: it asserts each labelled program in tests/verify* against its
# ground-truth `// TEST:` directives (status + Caller/Callee blame + violated
# vow_id), runs a vacuity guard over the should-pass set, and surfaces
# false-accepts (SOUNDNESS) and false-rejects (PRECISION) under dedicated loud
# banners. Runs against the Rust verifier; Rust/self parity is covered by 4b-4d.

section_begin "Section 4e: Verifier-Evaluation Suite (#334)"
ve_out="$TMPDIR/verify_eval.out"
if python3 scripts/verify_eval.py --verifier "$RUST" --output-dir "$TMPDIR/verify-eval" >"$ve_out" 2>&1; then
    pass "verifier-eval/ground-truth"
else
    fail "verifier-eval/ground-truth" "ground-truth mismatch — see banners below"
fi
sed 's/^/    /' "$ve_out"
echo ""

# ─── Section 4f: Verify-Fail Multi-Module Fixtures (#1472) ────────
#
# tests/verify-fail/*.vow is a flat glob of single-file targets (Section 4c);
# it cannot host a fixture that needs a sibling `dep.vow`/`dep.vow.d` pair.
# Mirrors tests/multi/'s subdirectory-per-fixture convention (Section 6b) but
# runs `verify` instead of `build --no-verify`, so each fixture's
# `// TEST: counterexample-blame <value>` directive can be checked alongside
# the existing VerifyFailed + full-JSON-parity checks Section 4c already uses.

section_begin "Section 4f: Verify-Fail Multi-Module Fixtures"
for dir in tests/verify-fail-multi/*/; do
    name=$(basename "$dir")
    main_file="${dir}main.vow"
    [ -f "$main_file" ] || continue

    rust_json="" self_json="" rust_exit=0 self_exit=0
    rust_json=$($RUST verify "$main_file" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self verify --backend esbmc "$main_file" 2>/dev/null) || self_exit=$?

    if check_empty_output "${name}/verify-fail-multi-test" "$rust_json" "$self_json" "$rust_exit" "$self_exit"; then
        continue
    fi

    compare_full_json "${name}/verify-fail-multi-test" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$main_file"
    actual_status=$(python3 -c "import json,sys; print(json.loads(sys.argv[1]).get('status',''))" "$rust_json" 2>/dev/null) || actual_status=""
    if [ -n "$actual_status" ] && [ "$actual_status" != "VerifyFailed" ]; then
        fail "${name}/verify-expected-fail" "expected VerifyFailed, got $actual_status"
    fi

    # `// TEST: counterexample-blame <value>` pins the first counterexample's
    # blame on both compilers — the acceptance criterion for #1472 is Caller
    # blame specifically, not merely "not Verified" (Skipped would also pass
    # a bare non-Verified check).
    expected_blame=$(sed -n 's|^// TEST: counterexample-blame \(.*\)$|\1|p' "$main_file" | head -1)
    if [ -n "$expected_blame" ]; then
        blame_errors=()
        for blame_side in rust self; do
            if [ "$blame_side" = "rust" ]; then blame_json="$rust_json"; else blame_json="$self_json"; fi
            actual_blame=$(python3 -c "
import json, sys
cx = json.loads(sys.stdin.read()).get('counterexamples') or []
print(cx[0].get('blame', '') if cx else '')
" <<< "$blame_json" 2>/dev/null) || actual_blame="<unparseable>"
            if [ "$actual_blame" != "$expected_blame" ]; then
                blame_errors+=("$blame_side blame='$actual_blame'")
            fi
        done
        if [ ${#blame_errors[@]} -eq 0 ]; then
            pass "${name}/verify-fail-multi-blame"
        else
            fail "${name}/verify-fail-multi-blame" "expected '$expected_blame'; $(IFS='; '; echo "${blame_errors[*]}")"
        fi
    fi
done
echo ""
