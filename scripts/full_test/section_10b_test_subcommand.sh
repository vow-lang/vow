# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

# ─── Section 10b: Test Subcommand ───────────────────────────────────

section_begin "Section 10b: Test Subcommand"

# `vow test` builds in debug mode, so the loop invariants of the shipped
# stdlib/heap is_min_heap / is_max_heap are checked at runtime (empty heap
# included) in both compilers.
heap_order_test="tests/stdlib/heap_order_predicates.vow"
heap_rust=$($RUST test --module-root stdlib/heap "$heap_order_test" 2>/dev/null) || true
heap_self=$(run_self test --module-root stdlib/heap "$heap_order_test" 2>/dev/null) || true
heap_status() { printf '%s' "$1" | python3 -c "import json,sys; print(json.load(sys.stdin)['status'])" 2>/dev/null || true; }
heap_rust_status=$(heap_status "$heap_rust")
heap_self_status=$(heap_status "$heap_self")
if [ "$heap_rust_status" = "TestsPassed" ] && [ "$heap_self_status" = "TestsPassed" ]; then
    pass "test/heap-order-predicates"
else
    fail "test/heap-order-predicates" "rust=$heap_rust_status self=$heap_self_status"
fi

# Run vowc test with both compilers on compiler/ directory.
rust_test_exit=0 self_test_exit=0
rust_test_json=$($RUST test compiler/ 2>/dev/null) || rust_test_exit=$?
self_test_json=$(run_self test compiler/ 2>/dev/null) || self_test_exit=$?

if ! check_empty_output "test/subcommand" "$rust_test_json" "$self_test_json" "$rust_test_exit" "$self_test_exit"; then
    run_parity test "test/parity" "$rust_test_json" "$self_test_json" "$rust_test_exit" "$self_test_exit"

    # Check contract_density field exists
    rust_cd=$(echo "$rust_test_json" | uv run python -c "import json,sys; d=json.load(sys.stdin); print('ok' if 'contract_density' in d else 'missing')" 2>/dev/null) || rust_cd=""
    self_cd=$(echo "$self_test_json" | uv run python -c "import json,sys; d=json.load(sys.stdin); print('ok' if 'contract_density' in d else 'missing')" 2>/dev/null) || self_cd=""

    if [ "$rust_cd" = "ok" ] && [ "$self_cd" = "ok" ]; then
        pass "test/contract-density"
    else
        fail "test/contract-density" "rust=$rust_cd self=$self_cd"
    fi

    # Check --filter works
    rust_filter=$($RUST test compiler/ --filter arith 2>/dev/null) || true
    filter_total=$(echo "$rust_filter" | uv run python -c "import json,sys; print(json.load(sys.stdin)['total'])" 2>/dev/null) || filter_total=""

    if [ "$filter_total" = "1" ]; then
        pass "test/filter"
    else
        fail "test/filter" "expected 1 test with --filter arith, got $filter_total"
    fi

    # A single test file under compiler/tests/ resolves its `use` declarations
    # against the inferred module root (compiler/), in both compilers.
    rust_single=$($RUST test compiler/tests/test_region.vow 2>/dev/null) || true
    self_single=$(run_self test compiler/tests/test_region.vow 2>/dev/null) || true
    rust_single_status=$(echo "$rust_single" | uv run python -c "import json,sys; print(json.load(sys.stdin)['status'])" 2>/dev/null) || rust_single_status=""
    self_single_status=$(echo "$self_single" | uv run python -c "import json,sys; print(json.load(sys.stdin)['status'])" 2>/dev/null) || self_single_status=""
    if [ "$rust_single_status" = "TestsPassed" ] && [ "$self_single_status" = "TestsPassed" ]; then
        pass "test/single-file-module-root"
    else
        fail "test/single-file-module-root" "rust=$rust_single_status self=$self_single_status"
    fi

    # A nonexistent path and a hung test must both fail closed in both
    # compilers: TestsFailed, exit 1, exactly one failed/timeout entry (#415).
    hang_dir="$TMPDIR/test_hang"
    mkdir -p "$hang_dir"
    printf 'module M\nfn main() -> i32 [io] {\n    while true {\n    }\n    0\n}\n' >"$hang_dir/test_hang.vow"
    for who in rust self; do
        for case in missing-path timeout; do
            if [ "$case" = "missing-path" ]; then
                case_args=("$hang_dir/does_not_exist.vow"); want_status="failed"
            else
                case_args=("$hang_dir/test_hang.vow" --timeout 1000); want_status="timeout"
            fi
            case_exit=0
            if [ "$who" = "rust" ]; then
                case_json=$($RUST test "${case_args[@]}" 2>/dev/null) || case_exit=$?
            else
                case_json=$(run_self test "${case_args[@]}" 2>/dev/null) || case_exit=$?
            fi
            case_ok=$(printf '%s' "$case_json" | uv run python -c "
import json,sys
d=json.load(sys.stdin)
t=d['tests']
ok=(d['status']=='TestsFailed' and d['total']==1 and d['passed']==0 and d['failed']==1
    and len(t)==1 and t[0]['status']==sys.argv[1])
print('ok' if ok else 'bad')" "$want_status" 2>/dev/null) || case_ok=""
            if [ "$case_ok" = "ok" ] && [ "$case_exit" = "1" ]; then
                pass "test/$case-$who"
            else
                fail "test/$case-$who" "exit=$case_exit result=$case_ok json=$case_json"
            fi
        done
    done

    # --jobs 1 (strictly sequential) must report the same suite as the
    # concurrent default: same per-file statuses in the same order.
    rust_seq=$($RUST test compiler/ --filter test_lexer --jobs 1 2>/dev/null) || true
    rust_par=$($RUST test compiler/ --filter test_lexer --jobs 4 2>/dev/null) || true
    self_seq=$(run_self test compiler/ --filter test_lexer --jobs 1 2>/dev/null) || true
    self_par=$(run_self test compiler/ --filter test_lexer --jobs 4 2>/dev/null) || true
    for who in rust self; do
        printf '%s' "$([ $who = rust ] && echo "$rust_par" || echo "$self_par")" >"$TMPDIR/jobs_par_$who.json"
        printf '%s' "$([ $who = rust ] && echo "$rust_seq" || echo "$self_seq")" >"$TMPDIR/jobs_seq_$who.json"
        seq_same=$(uv run python -c "
import json,sys
a=json.load(open(sys.argv[1])); b=json.load(open(sys.argv[2]))
f=lambda d:[(t['file'],t['status']) for t in d['tests']]
print('ok' if f(a)==f(b) and f(a) and a['status']==b['status'] else 'differ')" "$TMPDIR/jobs_par_$who.json" "$TMPDIR/jobs_seq_$who.json" 2>/dev/null) || seq_same=""
        if [ "$seq_same" = "ok" ]; then
            pass "test/jobs-order-independent-$who"
        else
            fail "test/jobs-order-independent-$who" "concurrent and --jobs 1 runs disagree ($seq_same)"
        fi
    done

    # test_complexity_io must not depend on the caller's working directory.
    repo_root=$(pwd -P)
    temp_cwd="$TMPDIR/test_complexity_io_cwd"
    mkdir -p "$temp_cwd"
    rust_abs="$repo_root/target/release/vow"
    rust_cwd_json=$(cd "$temp_cwd" && "$rust_abs" test "$repo_root/compiler" --filter test_complexity_io 2>/dev/null) || true
    self_cwd_json=$(cd "$temp_cwd" && run_self test "$repo_root/compiler" --filter test_complexity_io 2>/dev/null) || true
    rust_cwd_status=$(echo "$rust_cwd_json" | uv run python -c "import json,sys; print(json.load(sys.stdin)['status'])" 2>/dev/null) || rust_cwd_status=""
    self_cwd_status=$(echo "$self_cwd_json" | uv run python -c "import json,sys; print(json.load(sys.stdin)['status'])" 2>/dev/null) || self_cwd_status=""

    if [ "$rust_cwd_status" = "TestsPassed" ] && [ "$self_cwd_status" = "TestsPassed" ]; then
        pass "test/complexity-io-path-independent"
    else
        fail "test/complexity-io-path-independent" "rust=$rust_cwd_status self=$self_cwd_status"
    fi
fi
echo ""
