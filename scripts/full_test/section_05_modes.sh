# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

# ─── Section 5: Debug Mode ─────────────────────────────────────────

section_begin "Section 5: Debug Mode"

# divide.vow: VowViolation at runtime — aborts with the reserved runtime-abort
# exit code 134, not a plain 1 (#877)
$RUST build --mode debug --no-verify examples/divide.vow -o "$TMPDIR/rust_divide_debug" >/dev/null 2>/dev/null
run_self build --mode debug --no-verify examples/divide.vow -o "$TMPDIR/self_divide_debug" >/dev/null 2>/dev/null

rust_exit=0 self_exit=0
"$TMPDIR/rust_divide_debug" </dev/null >"$TMPDIR/rust_dbg_out" 2>"$TMPDIR/rust_dbg_err" || rust_exit=$?
run_self_bin "$TMPDIR/self_divide_debug" </dev/null >"$TMPDIR/self_dbg_out" 2>"$TMPDIR/self_dbg_err" || self_exit=$?
rust_err=$(cat "$TMPDIR/rust_dbg_err")
self_err=$(cat "$TMPDIR/self_dbg_err")

errors=()
if [ "$rust_exit" -ne 134 ]; then errors+=("rust exit=$rust_exit, expected 134"); fi
if [ "$self_exit" -ne 134 ]; then errors+=("self exit=$self_exit, expected 134"); fi
for pattern in VowViolation Caller "y != 0"; do
    if ! echo "$rust_err" | grep -q "$pattern"; then errors+=("rust stderr missing '$pattern'"); fi
    if ! echo "$self_err" | grep -q "$pattern"; then errors+=("self stderr missing '$pattern'"); fi
done
if [ ${#errors[@]} -eq 0 ]; then
    pass "divide/debug-violation"
else
    fail "divide/debug-violation" "$(IFS='; '; echo "${errors[*]}")"
fi
compare_runtime_json "divide/debug-violation-json" "$TMPDIR/rust_dbg_err" "$TMPDIR/self_dbg_err" "$rust_exit" "$self_exit"

# u8_requires_violation.vow: a captured u8 free variable must report its real
# value in the VowViolation payload, not 0 (numeric-tower u8 first-class fix).
$RUST build --mode debug --no-verify tests/debug/u8_requires_violation.vow -o "$TMPDIR/rust_u8_violation_debug" >/dev/null 2>/dev/null
run_self build --mode debug --no-verify tests/debug/u8_requires_violation.vow -o "$TMPDIR/self_u8_violation_debug" >/dev/null 2>/dev/null

rust_exit=0 self_exit=0
"$TMPDIR/rust_u8_violation_debug" </dev/null >"$TMPDIR/rust_u8_dbg_out" 2>"$TMPDIR/rust_u8_dbg_err" || rust_exit=$?
run_self_bin "$TMPDIR/self_u8_violation_debug" </dev/null >"$TMPDIR/self_u8_dbg_out" 2>"$TMPDIR/self_u8_dbg_err" || self_exit=$?
rust_err=$(cat "$TMPDIR/rust_u8_dbg_err")
self_err=$(cat "$TMPDIR/self_u8_dbg_err")

errors=()
if [ "$rust_exit" -ne 134 ]; then errors+=("rust exit=$rust_exit, expected 134"); fi
if [ "$self_exit" -ne 134 ]; then errors+=("self exit=$self_exit, expected 134"); fi
for pattern in VowViolation Caller '"x":5'; do
    if ! echo "$rust_err" | grep -qF "$pattern"; then errors+=("rust stderr missing '$pattern'"); fi
    if ! echo "$self_err" | grep -qF "$pattern"; then errors+=("self stderr missing '$pattern'"); fi
done
if [ ${#errors[@]} -eq 0 ]; then
    pass "u8_requires_violation/debug-violation"
else
    fail "u8_requires_violation/debug-violation" "$(IFS='; '; echo "${errors[*]}")"
fi

# i128_requires_violation.vow: both compilers must preserve both limbs of a
# captured i128 value in the runtime VowViolation payload (#1077).
$RUST build --mode debug --no-verify tests/debug/i128_requires_violation.vow -o "$TMPDIR/rust_i128_violation_debug" >/dev/null 2>/dev/null
run_self build --mode debug --no-verify tests/debug/i128_requires_violation.vow -o "$TMPDIR/self_i128_violation_debug" >/dev/null 2>/dev/null

rust_exit=0 self_exit=0
"$TMPDIR/rust_i128_violation_debug" </dev/null >"$TMPDIR/rust_i128_dbg_out" 2>"$TMPDIR/rust_i128_dbg_err" || rust_exit=$?
run_self_bin "$TMPDIR/self_i128_violation_debug" </dev/null >"$TMPDIR/self_i128_dbg_out" 2>"$TMPDIR/self_i128_dbg_err" || self_exit=$?
rust_err=$(cat "$TMPDIR/rust_i128_dbg_err")
self_err=$(cat "$TMPDIR/self_i128_dbg_err")

errors=()
if [ "$rust_exit" -ne 134 ]; then errors+=("rust exit=$rust_exit, expected 134"); fi
if [ "$self_exit" -ne 134 ]; then errors+=("self exit=$self_exit, expected 134"); fi
for pattern in VowViolation Caller '"x":3154393236604333326336'; do
    if ! echo "$rust_err" | grep -qF "$pattern"; then errors+=("rust stderr missing '$pattern'"); fi
    if ! echo "$self_err" | grep -qF "$pattern"; then errors+=("self stderr missing '$pattern'"); fi
done
if [ ${#errors[@]} -eq 0 ]; then
    pass "i128_requires_violation/debug-violation"
else
    fail "i128_requires_violation/debug-violation" "$(IFS='; '; echo "${errors[*]}")"
fi

# Every tests/debug fixture that carries `// TEST: stderr` directives: build in
# debug mode with both compilers, run, and require the expected exit code
# (`// TEST: exit N`, default 134 = runtime abort) plus every stderr substring.
# This is what keeps the VowViolation text (description, rendered values) from
# regressing in CI; tests/run_tests.sh Phase 4 is a local-only second check.
for vow_file in tests/debug/*.vow; do
    name=$(basename "$vow_file" .vow)
    if ! grep -q '^// TEST: stderr "' "$vow_file"; then continue; fi
    if grep -q '^// TEST: mode ' "$vow_file"; then continue; fi
    expected_exit=$(sed -n 's|^// TEST: exit \([0-9]*\)$|\1|p' "$vow_file" | head -1)
    expected_exit="${expected_exit:-134}"

    $RUST build --mode debug --no-verify "$vow_file" -o "$TMPDIR/rust_dbgstderr_${name}" >/dev/null 2>/dev/null
    run_self build --mode debug --no-verify "$vow_file" -o "$TMPDIR/self_dbgstderr_${name}" >/dev/null 2>/dev/null

    rust_exit=0 self_exit=0
    capture_stderr_with_optional_stdin "$TMPDIR/rust_dbgstderr_${name}.err" "" "$TMPDIR/rust_dbgstderr_${name}" || rust_exit=$?
    capture_stderr_with_optional_stdin "$TMPDIR/self_dbgstderr_${name}.err" "" run_self_bin "$TMPDIR/self_dbgstderr_${name}" || self_exit=$?

    if [ "$rust_exit" -ne "$expected_exit" ] || [ "$self_exit" -ne "$expected_exit" ]; then
        fail "${name}/debug-stderr" "expected exit $expected_exit, rust=$rust_exit self=$self_exit"
        continue
    fi
    check_stderr_directives "${name}/debug-stderr" "$vow_file" "$TMPDIR/rust_dbgstderr_${name}.err" "$TMPDIR/self_dbgstderr_${name}.err"
    # A runtime abort's JSON (`VowViolation` with its `file` and `offset`, or
    # the structured abort of another check) is a machine contract: it must be
    # identical, not merely contain the same substrings.
    if [ "$expected_exit" -eq 134 ]; then
        compare_runtime_json "${name}/debug-runtime-json" "$TMPDIR/rust_dbgstderr_${name}.err" "$TMPDIR/self_dbgstderr_${name}.err" "$rust_exit" "$self_exit"
    fi
done

# callee_blame, clamp, hello: contracts pass (or none), compare runtime
for name in callee_blame clamp hello; do
    $RUST build --mode debug --no-verify "examples/${name}.vow" -o "$TMPDIR/rust_${name}_debug" >/dev/null 2>/dev/null
    run_self build --mode debug --no-verify "examples/${name}.vow" -o "$TMPDIR/self_${name}_debug" >/dev/null 2>/dev/null
    compare_runtime "${name}/debug" "$TMPDIR/rust_${name}_debug" "$TMPDIR/self_${name}_debug"
done
echo ""

# ─── Section 5b: Profile Mode ─────────────────────────────────────

section_begin "Section 5b: Profile Mode"

# Build profile_mode.vow with both compilers
$RUST build --mode profile --no-verify tests/run/profile_mode.vow -o "$TMPDIR/rust_profile_mode" >/dev/null 2>/dev/null
run_self build --mode profile --no-verify tests/run/profile_mode.vow -o "$TMPDIR/self_profile_mode" >/dev/null 2>/dev/null

# Run and capture stderr (profile report) and stdout (program output)
rust_prof_out=$("$TMPDIR/rust_profile_mode" </dev/null 2>"$TMPDIR/rust_prof_err") || true
self_prof_out=$(run_self_bin "$TMPDIR/self_profile_mode" </dev/null 2>"$TMPDIR/self_prof_err") || true

errors=()
# Verify stdout matches expected output
if [ "$rust_prof_out" != "5" ]; then errors+=("rust stdout='$rust_prof_out', expected '5'"); fi
if [ "$self_prof_out" != "5" ]; then errors+=("self stdout='$self_prof_out', expected '5'"); fi
# Verify profile report structure in stderr
for compiler in rust self; do
    errfile="$TMPDIR/${compiler}_prof_err"
    if ! grep -q "vow profile report" "$errfile"; then errors+=("${compiler} stderr missing 'vow profile report'"); fi
    if ! grep -q "total calls: 5" "$errfile"; then errors+=("${compiler} stderr missing 'total calls: 5'"); fi
    if ! grep -q "unique functions: 2" "$errfile"; then errors+=("${compiler} stderr missing 'unique functions: 2'"); fi
    # helper called 4 times (4/5 = 80.0%)
    if ! grep -qE "helper\s+4\s" "$errfile"; then errors+=("${compiler} stderr: helper not called 4 times"); fi
    # main called 1 time
    if ! grep -qE "main\s+1\s" "$errfile"; then errors+=("${compiler} stderr: main not called 1 time"); fi
    # helper should appear before main (sorted by count descending)
    helper_line=$(grep -n "helper" "$errfile" | head -1 | cut -d: -f1)
    main_line=$(grep -n "main" "$errfile" | grep -v "vow_main" | tail -1 | cut -d: -f1)
    if [ -n "$helper_line" ] && [ -n "$main_line" ] && [ "$helper_line" -gt "$main_line" ]; then
        errors+=("${compiler} stderr: helper should appear before main (sorted by count)")
    fi
done
if [ ${#errors[@]} -eq 0 ]; then
    pass "profile_mode/profile"
else
    fail "profile_mode/profile" "$(IFS='; '; echo "${errors[*]}")"
fi
echo ""

# ─── Section 5c: Sanitize Mode ────────────────────────────────────

section_begin "Section 5c: Sanitize Mode"

# sanitize_vec.vow: Vec operations with sanitize instrumentation
$RUST build --mode sanitize --no-verify tests/debug/sanitize_vec.vow -o "$TMPDIR/rust_sanitize_vec" >/dev/null 2>/dev/null
run_self build --mode sanitize --no-verify tests/debug/sanitize_vec.vow -o "$TMPDIR/self_sanitize_vec" >/dev/null 2>/dev/null
compare_runtime "sanitize_vec/sanitize" "$TMPDIR/rust_sanitize_vec" "$TMPDIR/self_sanitize_vec"

# tests/debug_multi/<dir>/main.vow: a VowViolation whose clause lives in a
# module other than the entry file. Both compilers must report the defining
# module as `file` and the same `offset`.
for multi_dir in tests/debug_multi/*/; do
    name=$(basename "$multi_dir")
    main_file="${multi_dir}main.vow"
    $RUST build --mode debug --no-verify "$main_file" -o "$TMPDIR/rust_dbgmulti_${name}" >/dev/null 2>/dev/null
    run_self build --mode debug --no-verify "$main_file" -o "$TMPDIR/self_dbgmulti_${name}" >/dev/null 2>/dev/null
    rust_exit=0 self_exit=0
    capture_stderr_with_optional_stdin "$TMPDIR/rust_dbgmulti_${name}.err" "" "$TMPDIR/rust_dbgmulti_${name}" || rust_exit=$?
    capture_stderr_with_optional_stdin "$TMPDIR/self_dbgmulti_${name}.err" "" run_self_bin "$TMPDIR/self_dbgmulti_${name}" || self_exit=$?
    compare_runtime_json "${name}/debug-multi-runtime-json" "$TMPDIR/rust_dbgmulti_${name}.err" "$TMPDIR/self_dbgmulti_${name}.err" "$rust_exit" "$self_exit"
    defining_file=$(sed -n 's|^// TEST: violation-file \(.*\)$|\1|p' "$main_file" | head -1)
    if [ -n "$defining_file" ]; then
        for side in rust self; do
            reported=$(python3 -c "
import json, sys
docs = [json.loads(l) for l in sys.stdin if l.startswith('{')]
print(next((d.get('file', '') for d in docs if d.get('error') == 'VowViolation'), ''))
" < "$TMPDIR/${side}_dbgmulti_${name}.err" 2>/dev/null) || reported="<unparseable>"
            case "$reported" in
                *"$defining_file") pass "${name}/debug-multi-file-${side}" ;;
                *) fail "${name}/debug-multi-file-${side}" "expected a file ending in '$defining_file', got '$reported'" ;;
            esac
        done
    fi
done

# Nested-container growth and escaping stores route allocations through owner
# arenas, and loop-local Vecs die with their region every iteration; sanitize
# mode must run them cleanly (no UseAfterFree) and print what the plain runs print.
for sanitize_fixture in container_growth_owner_arena region_container_effect_outlives sanitize_region_close; do
    sanitize_src="tests/run/${sanitize_fixture}.vow"
    $RUST build --mode sanitize --no-verify "$sanitize_src" -o "$TMPDIR/rust_sanitize_${sanitize_fixture}" >/dev/null 2>/dev/null
    run_self build --mode sanitize --no-verify "$sanitize_src" -o "$TMPDIR/self_sanitize_${sanitize_fixture}" >/dev/null 2>/dev/null
    compare_runtime "${sanitize_fixture}/sanitize" "$TMPDIR/rust_sanitize_${sanitize_fixture}" "$TMPDIR/self_sanitize_${sanitize_fixture}"
    sanitize_want=$(sed -n 's|^// TEST: stdout "\(.*\)"$|\1|p' "$sanitize_src" | head -1)
    sanitize_got=$("$TMPDIR/rust_sanitize_${sanitize_fixture}" </dev/null 2>/dev/null | awk 'BEGIN{ORS="\\n"} {print}')
    if [ "$sanitize_got" = "$sanitize_want" ]; then
        pass "${sanitize_fixture}/sanitize-stdout"
    else
        fail "${sanitize_fixture}/sanitize-stdout" "sanitize stdout differs from the fixture's TEST: stdout"
    fi
done

echo ""
