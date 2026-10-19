# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

bootstrap_stage_failure() {
    local stage="$1"
    local status="$2"
    local stderr_log="$3"
    local stderr_tail="<no stderr>"

    if [ -s "$stderr_log" ]; then
        stderr_tail=$(tail -20 "$stderr_log")
    fi
    fail "bootstrap/triple-test" "$stage failed with exit code $status; stderr (last 20 lines): $stderr_tail"
}

run_bootstrap_triple() {
    local rust="$1"
    local concat="$2"
    local status=0
    local stderr_log="$TMPDIR/bootstrap_concat.stderr"

    "$concat" clif > "$TMPDIR/compiler_clif.vow" 2>"$stderr_log" || status=$?
    if [ "$status" -ne 0 ]; then
        bootstrap_stage_failure "concat" "$status" "$stderr_log"
        return 0
    fi

    # Stage 0: Rust compiler → Binary A
    status=0
    stderr_log="$TMPDIR/bootstrap_stage0.stderr"
    "$rust" --no-verify "$TMPDIR/compiler_clif.vow" -o "$TMPDIR/compiler_a" >/dev/null 2>"$stderr_log" || status=$?
    if [ "$status" -ne 0 ]; then
        bootstrap_stage_failure "Stage 0" "$status" "$stderr_log"
        return 0
    fi
    if [ ! -x "$TMPDIR/compiler_a" ]; then
        fail "bootstrap/triple-test" "Stage 0 exited with code 0 but did not produce executable compiler_a"
        return 0
    fi

    # Stage 1: A → B
    status=0
    stderr_log="$TMPDIR/bootstrap_stage1.stderr"
    run_self_bin "$TMPDIR/compiler_a" build --no-verify --no-cache -o "$TMPDIR/compiler_b" "$TMPDIR/compiler_clif.vow" >/dev/null 2>"$stderr_log" || status=$?
    if [ "$status" -ne 0 ]; then
        bootstrap_stage_failure "Stage 1" "$status" "$stderr_log"
        return 0
    fi
    if [ ! -x "$TMPDIR/compiler_b" ]; then
        fail "bootstrap/triple-test" "Stage 1 exited with code 0 but did not produce executable compiler_b"
        return 0
    fi

    # Stage 2: B → C
    status=0
    stderr_log="$TMPDIR/bootstrap_stage2.stderr"
    run_self_bin "$TMPDIR/compiler_b" build --no-verify --no-cache -o "$TMPDIR/compiler_c" "$TMPDIR/compiler_clif.vow" >/dev/null 2>"$stderr_log" || status=$?
    if [ "$status" -ne 0 ]; then
        bootstrap_stage_failure "Stage 2" "$status" "$stderr_log"
        return 0
    fi
    if [ ! -x "$TMPDIR/compiler_c" ]; then
        fail "bootstrap/triple-test" "Stage 2 exited with code 0 but did not produce executable compiler_c"
        return 0
    fi

    local hash_b=""
    local hash_c=""
    status=0
    stderr_log="$TMPDIR/bootstrap_hash_b.stderr"
    hash_b=$({ sha256sum "$TMPDIR/compiler_b" | awk '{print $1}'; } 2>"$stderr_log") || status=$?
    if [ "$status" -ne 0 ]; then
        bootstrap_stage_failure "Hash B" "$status" "$stderr_log"
        return 0
    fi
    if [ -z "$hash_b" ]; then
        fail "bootstrap/triple-test" "Hash B exited with code 0 but produced no digest"
        return 0
    fi

    status=0
    stderr_log="$TMPDIR/bootstrap_hash_c.stderr"
    hash_c=$({ sha256sum "$TMPDIR/compiler_c" | awk '{print $1}'; } 2>"$stderr_log") || status=$?
    if [ "$status" -ne 0 ]; then
        bootstrap_stage_failure "Hash C" "$status" "$stderr_log"
        return 0
    fi
    if [ -z "$hash_c" ]; then
        fail "bootstrap/triple-test" "Hash C exited with code 0 but produced no digest"
        return 0
    fi

    if [ "$hash_b" = "$hash_c" ]; then
        pass "bootstrap/triple-test"
    else
        fail "bootstrap/triple-test" "sha256 mismatch: B=$hash_b C=$hash_c"
    fi
}

print_summary() {
    section_finalize
    echo ""

    echo -e "${BOLD}=== Summary ===${RESET}"
    local script_end
    local total
    script_end=$(date +%s)
    total=$((script_end - SCRIPT_START))
    echo -e "  ${GREEN}${PASS} passed${RESET}, ${RED}${FAIL} failed${RESET}, ${YELLOW}${SKIP} skipped${RESET} in ${total}s"

    if [ ${#FAILURES[@]} -gt 0 ]; then
        echo ""
        echo -e "${RED}Failures:${RESET}"
        local failure
        for failure in "${FAILURES[@]}"; do
            echo "  - $failure"
        done
    fi

    return $(( FAIL > 0 ? 1 : 0 ))
}
