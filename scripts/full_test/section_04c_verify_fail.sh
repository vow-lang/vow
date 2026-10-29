# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

# ─── Section 4c: Verify-Fail Tests (tests/verify-fail/) ───────────

section_begin "Section 4c: Verify-Fail Tests"
for vow_file in tests/verify-fail/*.vow; do
    name=$(basename "$vow_file" .vow)

    rust_json="" self_json="" rust_exit=0 self_exit=0
    rust_json=$($RUST verify "$vow_file" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self verify --backend esbmc "$vow_file" 2>/dev/null) || self_exit=$?

    if check_empty_output "${name}/verify-fail-test" "$rust_json" "$self_json" "$rust_exit" "$self_exit"; then
        continue
    fi

    compare_full_json "${name}/verify-fail-test" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$vow_file"
    actual_status=$(python3 -c "import json,sys; print(json.loads(sys.argv[1]).get('status',''))" "$rust_json" 2>/dev/null) || actual_status=""
    if [ -n "$actual_status" ] && [ "$actual_status" != "VerifyFailed" ]; then
        fail "${name}/verify-expected-fail" "expected VerifyFailed, got $actual_status"
    fi

    # `// TEST: counterexample-violation "<text>"` pins the first counterexample's
    # `violation` text on both compilers. Parity alone would pass a regression
    # that makes both printers render the contract text the same wrong way.
    expected_violation=$(sed -n 's|^// TEST: counterexample-violation "\(.*\)"$|\1|p' "$vow_file" | head -1)
    if [ -n "$expected_violation" ]; then
        violation_errors=()
        for violation_side in rust self; do
            if [ "$violation_side" = "rust" ]; then violation_json="$rust_json"; else violation_json="$self_json"; fi
            actual_violation=$(python3 -c "
import json, sys
cx = json.loads(sys.stdin.read()).get('counterexamples') or []
print(cx[0].get('violation', '') if cx else '')
" <<< "$violation_json" 2>/dev/null) || actual_violation="<unparseable>"
            if [ "$actual_violation" != "$expected_violation" ]; then
                violation_errors+=("$violation_side violation='$actual_violation'")
            fi
        done
        if [ ${#violation_errors[@]} -eq 0 ]; then
            pass "${name}/verify-fail-violation"
        else
            fail "${name}/verify-fail-violation" "expected '$expected_violation'; $(IFS='; '; echo "${violation_errors[*]}")"
        fi
    fi

    # `// TEST: replay <status>` pins the `replay` status of every counterexample
    # under `verify --replay-cex` on both compilers. Only the status is compared:
    # `replay_reason` wording differs between the compilers by design.
    expected_replay=$(sed -n 's|^// TEST: replay \([a-z,]*\)$|\1|p' "$vow_file" | head -1)
    if [ -n "$expected_replay" ]; then
        replay_errors=()
        for replay_side in rust self; do
            if [ "$replay_side" = "rust" ]; then
                replay_json=$($RUST verify --replay-cex --no-cache "$vow_file" 2>/dev/null) || true
            else
                replay_json=$(run_self verify --backend esbmc --replay-cex --no-cache "$vow_file" 2>/dev/null) || true
            fi
            actual_replays=$(python3 -c "
import json, sys
cx = json.loads(sys.stdin.read()).get('counterexamples') or []
print(','.join(sorted(set(c.get('replay', '') for c in cx))) if cx else '<none>')
" <<< "$replay_json" 2>/dev/null) || actual_replays="<unparseable>"
            if [ "$actual_replays" != "$expected_replay" ]; then
                replay_errors+=("$replay_side replay='$actual_replays'")
            fi
        done
        if [ ${#replay_errors[@]} -eq 0 ]; then
            pass "${name}/verify-fail-replay"
        else
            fail "${name}/verify-fail-replay" "expected '$expected_replay'; $(IFS='; '; echo "${replay_errors[*]}")"
        fi
    fi
done
echo ""

name="verify_jobs_counterexample_suppresses_later_soft_meta"
fixture="tests/verify-fail/verify_jobs_ce_before_soft.vow"
errors=()
for mode in verify build legacy; do
    self_json="" self_exit=0
    case "$mode" in
        verify)
            self_json=$(run_self verify --backend esbmc --verify-jobs 2 "$fixture" 2>/dev/null) || self_exit=$?
            ;;
        build)
            self_json=$(run_self build --backend esbmc --verify-jobs 2 "$fixture" -o "$TMPDIR/ce_before_soft" 2>/dev/null) || self_exit=$?
            ;;
        legacy)
            self_json=$(run_self --backend esbmc --verify-jobs 2 "$fixture" -o "$TMPDIR/ce_before_soft_bare" 2>/dev/null) || self_exit=$?
            ;;
    esac
    actual_status=$(python3 -c "import json,sys; print(json.loads(sys.argv[1]).get('status',''))" "$self_json" 2>/dev/null) || actual_status=""
    verify_status=$(python3 -c "import json,sys; print(json.loads(sys.argv[1]).get('verify_status',''))" "$self_json" 2>/dev/null) || verify_status=""
    ce_function=$(python3 -c "import json,sys; d=json.loads(sys.argv[1]); c=d.get('counterexamples') or []; print(c[0].get('function','') if c else '')" "$self_json" 2>/dev/null) || ce_function=""
    if [ "$self_exit" -eq 0 ]; then
        errors+=("$mode exited 0")
    elif [ "$actual_status" != "VerifyFailed" ]; then
        errors+=("$mode status=$actual_status")
    elif [ "$ce_function" != "early_bad" ]; then
        errors+=("$mode counterexample function=$ce_function")
    elif [ -n "$verify_status" ]; then
        errors+=("$mode verify_status=$verify_status")
    fi
done
if [ ${#errors[@]} -eq 0 ]; then
    pass "$name"
else
    fail "$name" "$(IFS='; '; echo "${errors[*]}")"
fi
echo ""

name="verify_jobs_reports_first_hard_failure"
fixture="tests/verify-fail/verify_jobs_multi_hard_failure.vow"
errors=()
for mode in verify build; do
    rust_json="" self_json="" rust_exit=0 self_exit=0
    case "$mode" in
        verify)
            rust_json=$("$RUST" verify --no-cache --verify-jobs 3 "$fixture" 2>/dev/null) || rust_exit=$?
            self_json=$(run_self verify --backend esbmc --no-cache --verify-jobs 3 "$fixture" 2>/dev/null) || self_exit=$?
            ;;
        build)
            rust_json=$("$RUST" build --no-cache --verify-jobs 3 "$fixture" -o "$TMPDIR/multi_hard_rust" 2>/dev/null) || rust_exit=$?
            self_json=$(run_self build --backend esbmc --no-cache --verify-jobs 3 "$fixture" -o "$TMPDIR/multi_hard_self" 2>/dev/null) || self_exit=$?
            ;;
    esac

    compare_full_json "$name/$mode" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$fixture"
    for compiler in rust self; do
        if [ "$compiler" = "rust" ]; then
            result_json="$rust_json"
            result_exit="$rust_exit"
        else
            result_json="$self_json"
            result_exit="$self_exit"
        fi
        actual_status=$(python3 -c "import json,sys; print(json.loads(sys.argv[1]).get('status',''))" "$result_json" 2>/dev/null) || actual_status=""
        ce_count=$(python3 -c "import json,sys; print(len(json.loads(sys.argv[1]).get('counterexamples') or []))" "$result_json" 2>/dev/null) || ce_count=""
        ce_function=$(python3 -c "import json,sys; c=json.loads(sys.argv[1]).get('counterexamples') or []; print(c[0].get('function','') if c else '')" "$result_json" 2>/dev/null) || ce_function=""
        if [ "$result_exit" -eq 0 ]; then
            errors+=("$mode/$compiler exited 0")
        elif [ "$actual_status" != "VerifyFailed" ]; then
            errors+=("$mode/$compiler status=$actual_status")
        elif [ "$ce_count" != "1" ]; then
            errors+=("$mode/$compiler counterexamples=$ce_count")
        elif [ "$ce_function" != "first_bad" ]; then
            errors+=("$mode/$compiler counterexample function=$ce_function")
        fi
    done
done
if [ ${#errors[@]} -eq 0 ]; then
    pass "$name/policy"
else
    fail "$name/policy" "$(IFS='; '; echo "${errors[*]}")"
fi
echo ""
