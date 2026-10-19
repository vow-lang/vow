# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

run_promoted_run_tests() {
    section_begin "Section 4: Run Tests"
    for vow_file in tests/run/*.vow; do
        name=$(basename "$vow_file" .vow)

        skip_reason=$(sed -n 's|^// TEST: skip "\(.*\)"$|\1|p' "$vow_file" | head -1)
        if [ -n "$skip_reason" ]; then
            skip "${name}/test-build" "$skip_reason"
            continue
        fi

        if grep -q '^// TEST: verify-only$' "$vow_file"; then
            rust_json="" self_json="" rust_exit=0 self_exit=0
            rust_json=$($RUST verify "$vow_file" 2>/dev/null) || rust_exit=$?
            self_json=$(run_self verify "$vow_file" 2>/dev/null) || self_exit=$?

            if ! check_empty_output "${name}/test-verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit"; then
                compare_full_json "${name}/test-verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$vow_file"
                # Parity alone would pass a regression that makes BOTH compilers
                # reject the fixture, so pin the absolute expectation too (as
                # Section 4b does for tests/verify/).
                actual_status=$(python3 -c "import json,sys; print(json.loads(sys.argv[1]).get('status',''))" "$rust_json" 2>/dev/null) || actual_status=""
                if [ "$actual_status" != "Verified" ]; then
                    fail "${name}/test-verify-expected-pass" "expected Verified, got ${actual_status:-<none>}"
                fi
            fi
            continue
        fi

        # Build with both compilers
        rust_json="" self_json="" rust_exit=0 self_exit=0
        rust_json=$($RUST build --no-verify "$vow_file" -o "$TMPDIR/test_rust_${name}" 2>/dev/null) || rust_exit=$?
        self_json=$(run_self build --no-verify "$vow_file" -o "$TMPDIR/test_self_${name}" 2>/dev/null) || self_exit=$?

        if check_empty_output "${name}/test-build" "$rust_json" "$self_json" "$rust_exit" "$self_exit"; then
            continue
        fi

        compare_json "${name}/test-build" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$vow_file"

        # Extract executables
        rust_exe=$(python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('executable') or '')" <<< "$rust_json" 2>/dev/null) || rust_exe=""
        self_exe=$(python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('executable') or '')" <<< "$self_json" 2>/dev/null) || self_exe=""

        if [ -z "$rust_exe" ] && [ -z "$self_exe" ]; then
            skip "${name}/test-run" "no executable"
            continue
        fi
        if [ -z "$rust_exe" ] || [ -z "$self_exe" ]; then
            fail "${name}/test-run" "executable mismatch: rust='${rust_exe:-null}' self='${self_exe:-null}'"
            continue
        fi

        test_stdin=$(sed -n 's|^// TEST: stdin "\(.*\)"$|\1|p' "$vow_file" | head -1)
        test_stdin_file=$(sed -n 's|^// TEST: stdin-file \(.*\)$|\1|p' "$vow_file" | head -1)
        stdin_path=""
        if [ -n "$test_stdin_file" ]; then
            stdin_path="$(dirname "$vow_file")/$test_stdin_file"
            if [ ! -f "$stdin_path" ]; then
                fail "${name}/test-run" "stdin fixture not found: $stdin_path"
                continue
            fi
        elif [ -n "$test_stdin" ]; then
            stdin_path="$TMPDIR/stdin_${name}.txt"
            printf '%b' "$test_stdin" > "$stdin_path"
        fi

        # Compare runtime output between compilers
        compare_runtime "${name}/test-run" "$TMPDIR/test_rust_${name}" "$TMPDIR/test_self_${name}" "$stdin_path" "$vow_file"

        # Validate against // TEST: stdout directive if present
        expected=$(sed -n 's|^// TEST: stdout "\(.*\)"$|\1|p' "$vow_file" | head -1)
        if [ -n "$expected" ]; then
            actual=$(run_stdout_with_optional_stdin "$TMPDIR/test_rust_${name}" "$stdin_path") || true
            # Interpret \n escapes in expected string
            expected_decoded=$(printf '%b' "$expected")
            if [ "$actual" = "$expected_decoded" ]; then
                pass "${name}/test-expected"
            else
                fail "${name}/test-expected" "expected '$expected' got '$(echo "$actual" | head -c 80)'"
            fi
        fi

        # Validate against // TEST: exit directive if present
        expected_exit=$(sed -n 's|^// TEST: exit \([0-9]*\)$|\1|p' "$vow_file" | head -1)
        if [ -n "$expected_exit" ]; then
            actual_exit=0
            run_discard_with_optional_stdin "$TMPDIR/test_rust_${name}" "$stdin_path" || actual_exit=$?
            if [ "$actual_exit" = "$expected_exit" ]; then
                pass "${name}/test-exit"
            else
                fail "${name}/test-exit" "expected exit $expected_exit got $actual_exit"
            fi
        fi

        # Validate against // TEST: stderr directives (substring, both compilers)
        if grep -q '^// TEST: stderr "' "$vow_file"; then
            rust_err_file="$TMPDIR/test_rust_${name}.stderr"
            self_err_file="$TMPDIR/test_self_${name}.stderr"
            capture_stderr_with_optional_stdin "$rust_err_file" "$stdin_path" "$TMPDIR/test_rust_${name}" || true
            capture_stderr_with_optional_stdin "$self_err_file" "$stdin_path" run_self_bin "$TMPDIR/test_self_${name}" || true
            check_stderr_directives "${name}/test-stderr" "$vow_file" "$rust_err_file" "$self_err_file"
        fi
    done
}

run_promoted_error_tests() {
    section_begin "Section 7: Error Handling"
    cat > "$TMPDIR/parse_error.vow" <<'EOF'
module M 123
EOF

    cat > "$TMPDIR/type_error.vow" <<'EOF'
module Bad
fn f() -> i32 { true }
EOF

    cat > "$TMPDIR/missing_module.vow" <<'EOF'
module Main
use nonexistent
fn main() -> i32 { 0 }
EOF

    cat > "$TMPDIR/const_type_mismatch.vow" <<'EOF'
module Bad
const BAD: bool = 42;
fn main() -> i32 { 0 }
EOF

    for fixture_path in \
        "$TMPDIR/parse_error.vow" \
        "$TMPDIR/type_error.vow" \
        "$TMPDIR/missing_module.vow" \
        "$TMPDIR/const_type_mismatch.vow" \
        tests/error/*.vow; do
        fixture=$(basename "$fixture_path" .vow)
        rust_json="" self_json="" rust_exit=0 self_exit=0
        rust_json=$($RUST build --no-verify "$fixture_path" -o "$TMPDIR/rust_${fixture}" 2>/dev/null) || rust_exit=$?
        self_json=$(run_self build --no-verify "$fixture_path" -o "$TMPDIR/self_${fixture}" 2>/dev/null) || self_exit=$?

        if check_empty_output "${fixture}/error" "$rust_json" "$self_json" "$rust_exit" "$self_exit"; then
            continue
        fi

        compare_error "${fixture}/error" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$fixture_path"
    done
}
