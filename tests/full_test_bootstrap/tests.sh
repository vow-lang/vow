#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

TEST_TMPDIR=$(mktemp -d)
trap 'rm -rf "$TEST_TMPDIR"' EXIT
# Kill signals re-raise so the EXIT handler above still does the removal:
# EXIT alone does not fire on an untrapped SIGTERM, so a process-group kill
# would strand this scratch tree. See scripts/full_test.sh.
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

fail_test() {
    echo "FAIL: $1" >&2
    exit 1
}

assert_contains() {
    local haystack="$1"
    local needle="$2"
    local label="$3"

    if [[ "$haystack" != *"$needle"* ]]; then
        fail_test "$label: expected output to contain: $needle"
    fi
}

assert_trace() {
    local expected="$1"

    if [ "$RUN_TRACE" != "$expected" ]; then
        fail_test "unexpected command trace: expected '$expected', got '$RUN_TRACE'"
    fi
}

make_fixture() {
    local fixture="$1"

    mkdir -p "$fixture/bin"

    cat > "$fixture/fake-concat" <<'EOF'
#!/usr/bin/env bash
printf 'concat\n' >> "$VOW_FULL_TEST_TRACE"
if [ "$VOW_FULL_TEST_SCENARIO" = "concat_failure" ]; then
    printf 'concat failure sentinel\n' >&2
    exit 41
fi
printf 'fake compiler source\n'
EOF
    chmod +x "$fixture/fake-concat"

    cat > "$fixture/fake-rust" <<'EOF'
#!/usr/bin/env bash
printf 'Stage 0\n' >> "$VOW_FULL_TEST_TRACE"
if [ "$VOW_FULL_TEST_SCENARIO" = "stage0_failure" ]; then
    printf 'stage 0 failure sentinel\n' >&2
    exit 42
fi

output=""
while [ "$#" -gt 0 ]; do
    if [ "$1" = "-o" ]; then
        output="$2"
        break
    fi
    shift
done
cp "$VOW_FULL_TEST_COMPILER_TEMPLATE" "$output"
chmod +x "$output"
EOF
    chmod +x "$fixture/fake-rust"

    cat > "$fixture/generated-compiler" <<'EOF'
#!/usr/bin/env bash
output=""
while [ "$#" -gt 0 ]; do
    if [ "$1" = "-o" ]; then
        output="$2"
        break
    fi
    shift
done

case "$(basename "$0")" in
    compiler_a)
        printf 'Stage 1\n' >> "$VOW_FULL_TEST_TRACE"
        if [ "$VOW_FULL_TEST_SCENARIO" = "stage1_failure" ]; then
            printf 'stage 1 failure sentinel\n' >&2
            exit 43
        fi
        ;;
    compiler_b)
        printf 'Stage 2\n' >> "$VOW_FULL_TEST_TRACE"
        if [ "$VOW_FULL_TEST_SCENARIO" = "stage2_failure" ]; then
            printf 'stage 2 failure sentinel\n' >&2
            exit 44
        fi
        ;;
    *) exit 97 ;;
esac

cp "$0" "$output"
chmod +x "$output"
if [ "$VOW_FULL_TEST_SCENARIO" = "mismatch" ] && [ "$(basename "$0")" = "compiler_b" ]; then
    printf '# mismatch\n' >> "$output"
fi
EOF
    chmod +x "$fixture/generated-compiler"

    cat > "$fixture/bin/sha256sum" <<'EOF'
#!/usr/bin/env bash
printf 'hash:%s\n' "$(basename "$1")" >> "$VOW_FULL_TEST_TRACE"
if [ "$VOW_FULL_TEST_SCENARIO" = "hash_failure" ]; then
    printf 'hash failure sentinel\n' >&2
    exit 45
fi
exec "$VOW_FULL_TEST_REAL_SHA256SUM" "$@"
EOF
    chmod +x "$fixture/bin/sha256sum"
}

run_case() {
    local name="$1"
    local scenario="$2"
    local fixture="$TEST_TMPDIR/$name"
    local output_file="$fixture/output"
    local trace_file="$fixture/trace"
    local real_sha256sum

    real_sha256sum=$(command -v sha256sum)
    make_fixture "$fixture"

    RUN_STATUS=0
    VOW_FULL_TEST_BOOTSTRAP_ONLY=1 \
        VOW_FULL_TEST_CONCAT="$fixture/fake-concat" \
        VOW_FULL_TEST_RUST="$fixture/fake-rust" \
        VOW_FULL_TEST_COMPILER_TEMPLATE="$fixture/generated-compiler" \
        VOW_FULL_TEST_REAL_SHA256SUM="$real_sha256sum" \
        VOW_FULL_TEST_SCENARIO="$scenario" \
        VOW_FULL_TEST_TRACE="$trace_file" \
        PATH="$fixture/bin:$PATH" \
        bash scripts/full_test.sh >"$output_file" 2>&1 || RUN_STATUS=$?
    RUN_OUTPUT=$(cat "$output_file")
    RUN_TRACE=""
    if [ -f "$trace_file" ]; then
        RUN_TRACE=$(cat "$trace_file")
    fi
}

assert_summary_failure() {
    local label="$1"

    [ "$RUN_STATUS" -eq 1 ] || fail_test "$label: expected status 1, got $RUN_STATUS"
    assert_contains "$RUN_OUTPUT" "bootstrap/triple-test" "$label failure label"
    assert_contains "$RUN_OUTPUT" "=== Summary ===" "$label summary"
    assert_contains "$RUN_OUTPUT" "0 passed" "$label pass count"
    assert_contains "$RUN_OUTPUT" "1 failed" "$label failure count"
}

assert_stage_failure() {
    local scenario="$1"
    local stage="$2"
    local status="$3"
    local sentinel="$4"
    local trace="$5"

    run_case "$scenario" "$scenario"
    assert_summary_failure "$stage"
    assert_contains "$RUN_OUTPUT" "$stage failed with exit code $status" "$stage exit status"
    assert_contains "$RUN_OUTPUT" "$sentinel" "$stage stderr"
    assert_trace "$trace"
}

test_concat_failure_reaches_summary() {
    assert_stage_failure "concat_failure" "concat" 41 "concat failure sentinel" "concat"
}

test_stage0_failure_reaches_summary() {
    assert_stage_failure "stage0_failure" "Stage 0" 42 "stage 0 failure sentinel" $'concat\nStage 0'
}

test_stage1_failure_reaches_summary() {
    assert_stage_failure "stage1_failure" "Stage 1" 43 "stage 1 failure sentinel" $'concat\nStage 0\nStage 1'
}

test_stage2_failure_reaches_summary() {
    assert_stage_failure "stage2_failure" "Stage 2" 44 "stage 2 failure sentinel" $'concat\nStage 0\nStage 1\nStage 2'
}

test_success_preserves_fixed_point_gate() {
    run_case "success" "success"

    [ "$RUN_STATUS" -eq 0 ] || fail_test "fixed-point success: expected status 0, got $RUN_STATUS"
    assert_contains "$RUN_OUTPUT" "PASS" "fixed-point success result"
    assert_contains "$RUN_OUTPUT" "bootstrap/triple-test" "fixed-point success label"
    assert_contains "$RUN_OUTPUT" "=== Summary ===" "fixed-point success summary"
    assert_contains "$RUN_OUTPUT" "1 passed" "fixed-point success pass count"
    assert_contains "$RUN_OUTPUT" "0 failed" "fixed-point success failure count"
    assert_trace $'concat\nStage 0\nStage 1\nStage 2\nhash:compiler_b\nhash:compiler_c'
}

test_mismatch_preserves_fixed_point_failure() {
    run_case "mismatch" "mismatch"

    assert_summary_failure "fixed-point mismatch"
    assert_contains "$RUN_OUTPUT" "sha256 mismatch: B=" "fixed-point mismatch hashes"
    assert_contains "$RUN_OUTPUT" " C=" "fixed-point mismatch C hash"
    assert_trace $'concat\nStage 0\nStage 1\nStage 2\nhash:compiler_b\nhash:compiler_c'
}

test_hash_failure_reaches_summary() {
    assert_stage_failure "hash_failure" "Hash B" 45 "hash failure sentinel" $'concat\nStage 0\nStage 1\nStage 2\nhash:compiler_b'
}

test_concat_failure_reaches_summary
test_stage0_failure_reaches_summary
test_stage1_failure_reaches_summary
test_stage2_failure_reaches_summary
test_success_preserves_fixed_point_gate
test_mismatch_preserves_fixed_point_failure
test_hash_failure_reaches_summary

echo "full-test bootstrap tests passed"
