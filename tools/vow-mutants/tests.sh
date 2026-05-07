#!/usr/bin/env bash
# Smoke tests for vow-mutants. Built and invoked by scripts/full_test.sh Section 12.
# Standalone usage: bash tools/vow-mutants/tests.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

VOWC="$ROOT/build/vowc"
VOWM=""
TMP=""

GREEN="\033[32m"
RED="\033[31m"
RESET="\033[0m"
PASS=0
FAIL=0
FAILURES=()

setup() {
    TMP="$(mktemp -d)"
    trap 'rm -rf "$TMP"' EXIT
    if [ -n "${VOW_MUTANTS_BIN:-}" ]; then
        VOWM="$VOW_MUTANTS_BIN"
    else
        VOWM="$TMP/vow-mutants"
        (ulimit -v 2000000; "$VOWC" build --no-verify tools/vow-mutants/main.vow -o "$VOWM") >/dev/null
    fi
}

run_vowm() {
    (ulimit -v 2000000; "$VOWM" "$@")
}

assert_eq() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        printf "  ${GREEN}PASS${RESET} %s\n" "$label"
        PASS=$((PASS + 1))
    else
        printf "  ${RED}FAIL${RESET} %s\n    expected: %q\n    actual:   %q\n" "$label" "$expected" "$actual"
        FAIL=$((FAIL + 1))
        FAILURES+=("$label")
    fi
}

assert_grep() {
    local label="$1" pattern="$2" haystack="$3"
    if echo "$haystack" | grep -qE "$pattern"; then
        printf "  ${GREEN}PASS${RESET} %s\n" "$label"
        PASS=$((PASS + 1))
    else
        printf "  ${RED}FAIL${RESET} %s\n    pattern: %s\n    in:      %s\n" "$label" "$pattern" "$haystack"
        FAIL=$((FAIL + 1))
        FAILURES+=("$label")
    fi
}

# --- Tracer 1: version ---

t1_version() {
    local out rc
    out=$(run_vowm version)
    rc=$?
    assert_eq "T1: version exit code" "0" "$rc"
    assert_grep "T1: version output contains 'vow-mutants 0.1.0'" "vow-mutants 0\\.1\\.0" "$out"
}

t1_no_args_exits_1() {
    set +e
    run_vowm >/dev/null 2>&1
    local rc=$?
    set -e
    assert_eq "T1: no-args exit code" "1" "$rc"
}

t1_unknown_subcommand_exits_1() {
    set +e
    run_vowm nonsense >/dev/null 2>&1
    local rc=$?
    set -e
    assert_eq "T1: unknown-subcommand exit code" "1" "$rc"
}

# --- Tracer 2: list with empty root ---

t10_run_round_trip_leaves_files_unchanged() {
    local fixture="$TMP/run_root"
    mkdir -p "$fixture"
    cp tests/fixtures/mutants/sample_op.vow "$fixture/sample_op.vow"
    local before_sum after_sum
    before_sum=$(sha256sum "$fixture/sample_op.vow" | awk '{print $1}')
    set +e
    out=$(run_vowm run --root "$fixture" --tier1-cmd 'true' --tier2-cmd 'true' 2>&1)
    rc=$?
    set -e
    after_sum=$(sha256sum "$fixture/sample_op.vow" | awk '{print $1}')
    assert_eq "T10: run exit code" "0" "$rc"
    assert_eq "T10: file content unchanged after run" "$before_sum" "$after_sum"
}

t11_run_classifies_caught_and_missed() {
    local fixture="$TMP/run_classify"
    mkdir -p "$fixture"
    cp tests/fixtures/mutants/sample_op.vow "$fixture/sample_op.vow"
    # All-true oracle: every mutant survives Tier 1 and Tier 2 → "missed".
    local out_missed
    set +e
    out_missed=$(run_vowm run --root "$fixture" --tier1-cmd 'true' --tier2-cmd 'true' 2>&1)
    set -e
    local missed_count
    missed_count=$(echo "$out_missed" | grep -c '"status":"missed"' || true)
    if [ "$missed_count" -ge 1 ]; then
        printf "  ${GREEN}PASS${RESET} T11: all-true oracle yields missed records (got %d)\n" "$missed_count"
        PASS=$((PASS + 1))
    else
        printf "  ${RED}FAIL${RESET} T11: expected >=1 missed, got %d\n    output:\n%s\n" "$missed_count" "$out_missed"
        FAIL=$((FAIL + 1))
        FAILURES+=("T11-missed")
    fi
    # All-false Tier-1 oracle: every mutant caught at Tier 1.
    local out_caught caught_t1
    set +e
    out_caught=$(run_vowm run --root "$fixture" --tier1-cmd 'false' --tier2-cmd 'true' 2>&1)
    set -e
    caught_t1=$(echo "$out_caught" | grep -c '"status":"caught","tier":1' || true)
    if [ "$caught_t1" -ge 1 ]; then
        printf "  ${GREEN}PASS${RESET} T11: false-tier1 oracle yields tier-1 caught records (got %d)\n" "$caught_t1"
        PASS=$((PASS + 1))
    else
        printf "  ${RED}FAIL${RESET} T11: expected >=1 caught at tier 1, got %d\n" "$caught_t1"
        FAIL=$((FAIL + 1))
        FAILURES+=("T11-caught-t1")
    fi
    # Tier-1 pass + Tier-2 fail → caught at tier 2.
    local out_caught_t2 caught_t2
    set +e
    out_caught_t2=$(run_vowm run --root "$fixture" --tier1-cmd 'true' --tier2-cmd 'false' 2>&1)
    set -e
    caught_t2=$(echo "$out_caught_t2" | grep -c '"status":"caught","tier":2' || true)
    if [ "$caught_t2" -ge 1 ]; then
        printf "  ${GREEN}PASS${RESET} T11: false-tier2 oracle yields tier-2 caught records (got %d)\n" "$caught_t2"
        PASS=$((PASS + 1))
    else
        printf "  ${RED}FAIL${RESET} T11: expected >=1 caught at tier 2, got %d\n" "$caught_t2"
        FAIL=$((FAIL + 1))
        FAILURES+=("T11-caught-t2")
    fi
}

t12_tier2_budget_zero_marks_remaining_unrun() {
    local fixture="$TMP/run_budget"
    mkdir -p "$fixture"
    cp tests/fixtures/mutants/sample_op.vow "$fixture/sample_op.vow"
    # Tier-1 passes ('true') on every mutant; Tier-2 budget is 0 seconds, so
    # every Tier-1 survivor must be reported as "unrun".
    local out unrun_count missed_count
    set +e
    out=$(run_vowm run --root "$fixture" --tier1-cmd 'true' --tier2-cmd 'true' --tier2-budget-secs 0 2>&1)
    set -e
    unrun_count=$(echo "$out" | grep -c '"status":"unrun"' || true)
    missed_count=$(echo "$out" | grep -c '"status":"missed"' || true)
    if [ "$unrun_count" -ge 1 ]; then
        printf "  ${GREEN}PASS${RESET} T12: zero-budget yields unrun records (got %d)\n" "$unrun_count"
        PASS=$((PASS + 1))
    else
        printf "  ${RED}FAIL${RESET} T12: expected >=1 unrun, got %d\n    output:\n%s\n" "$unrun_count" "$out"
        FAIL=$((FAIL + 1))
        FAILURES+=("T12-unrun")
    fi
    assert_eq "T12: zero-budget produces zero missed records" "0" "$missed_count"
}

t9_sharding_is_deterministic_and_partitions_total() {
    local out_full out_a out_b
    set +e
    out_full=$(run_vowm list --root tests/fixtures/mutants 2>&1)
    out_a=$(run_vowm list --root tests/fixtures/mutants --shard 0/3 2>&1)
    out_b=$(run_vowm list --root tests/fixtures/mutants --shard 0/3 2>&1)
    set -e
    # Determinism: two identical invocations produce byte-identical output.
    if [ "$out_a" = "$out_b" ]; then
        printf "  ${GREEN}PASS${RESET} T9: identical shard runs produce byte-identical output\n"
        PASS=$((PASS + 1))
    else
        printf "  ${RED}FAIL${RESET} T9: two runs of shard 0/3 differ\n"
        FAIL=$((FAIL + 1))
        FAILURES+=("T9-determinism")
    fi
    # Partition: union of shards 0/3 + 1/3 + 2/3 covers all sites in shard 0/1.
    local total_full total_a sum
    total_full=$(echo "$out_full" | tail -1 | grep -oE '"total":[0-9]+' | head -1 | grep -oE '[0-9]+')
    local s0 s1 s2
    s0=$(run_vowm list --root tests/fixtures/mutants --shard 0/3 2>&1 | tail -1 | grep -oE '"total":[0-9]+' | head -1 | grep -oE '[0-9]+')
    s1=$(run_vowm list --root tests/fixtures/mutants --shard 1/3 2>&1 | tail -1 | grep -oE '"total":[0-9]+' | head -1 | grep -oE '[0-9]+')
    s2=$(run_vowm list --root tests/fixtures/mutants --shard 2/3 2>&1 | tail -1 | grep -oE '"total":[0-9]+' | head -1 | grep -oE '[0-9]+')
    sum=$((s0 + s1 + s2))
    if [ "$sum" -eq "$total_full" ]; then
        printf "  ${GREEN}PASS${RESET} T9: shard 0/3 + 1/3 + 2/3 totals equal full run (%d == %d)\n" "$sum" "$total_full"
        PASS=$((PASS + 1))
    else
        printf "  ${RED}FAIL${RESET} T9: shard sum (%d) != full total (%d)\n" "$sum" "$total_full"
        FAIL=$((FAIL + 1))
        FAILURES+=("T9-partition")
    fi
}

t8_finds_body_replace_sites() {
    local out rc
    set +e
    out=$(run_vowm list --root tests/fixtures/mutants 2>&1)
    rc=$?
    set -e
    assert_eq "T8: list exit code" "0" "$rc"
    local body_count
    body_count=$(echo "$out" | grep -c '"kind":"body-replace"' || true)
    if [ "$body_count" -ge 3 ]; then
        printf "  ${GREEN}PASS${RESET} T8: body-replace sites found (got %d, expected >=3)\n" "$body_count"
        PASS=$((PASS + 1))
    else
        printf "  ${RED}FAIL${RESET} T8: expected >=3 body-replace sites, got %d\n    output:\n%s\n" "$body_count" "$out"
        FAIL=$((FAIL + 1))
        FAILURES+=("T8-count")
    fi
    # Specific replacements: 0 for i64, false for bool
    local i64_replace bool_replace
    i64_replace=$(echo "$out" | grep '"file":"tests/fixtures/mutants/sample_body.vow"' | grep -c '"kind":"body-replace","from":"[^"]*","to":" 0 "' || true)
    bool_replace=$(echo "$out" | grep '"file":"tests/fixtures/mutants/sample_body.vow"' | grep -c '"kind":"body-replace","from":"[^"]*","to":" false "' || true)
    if [ "$i64_replace" -ge 1 ]; then
        printf "  ${GREEN}PASS${RESET} T8: i64-returning fn yields body-replace with 0\n"
        PASS=$((PASS + 1))
    else
        printf "  ${RED}FAIL${RESET} T8: expected i64 body-replace with \" 0 \", got %d\n" "$i64_replace"
        FAIL=$((FAIL + 1))
        FAILURES+=("T8-i64")
    fi
    if [ "$bool_replace" -ge 1 ]; then
        printf "  ${GREEN}PASS${RESET} T8: bool-returning fn yields body-replace with false\n"
        PASS=$((PASS + 1))
    else
        printf "  ${RED}FAIL${RESET} T8: expected bool body-replace with \" false \", got %d\n" "$bool_replace"
        FAIL=$((FAIL + 1))
        FAILURES+=("T8-bool")
    fi
}

t7_finds_contract_weaken_sites() {
    local out rc
    set +e
    out=$(run_vowm list --root tests/fixtures/mutants 2>&1)
    rc=$?
    set -e
    assert_eq "T7: list exit code" "0" "$rc"
    local cw_count
    cw_count=$(echo "$out" | grep -c '"kind":"contract-weaken"' || true)
    if [ "$cw_count" -ge 4 ]; then
        printf "  ${GREEN}PASS${RESET} T7: contract-weaken sites found (got %d, expected >=4)\n" "$cw_count"
        PASS=$((PASS + 1))
    else
        printf "  ${RED}FAIL${RESET} T7: expected >=4 contract-weaken sites, got %d\n    output:\n%s\n" "$cw_count" "$out"
        FAIL=$((FAIL + 1))
        FAILURES+=("T7-count")
    fi
    # All replacements should be "true"
    local non_true_replacements
    non_true_replacements=$(echo "$out" | grep '"kind":"contract-weaken"' | grep -cv '"to":"true"' || true)
    assert_eq "T7: every contract-weaken site replaces with \"true\"" "0" "$non_true_replacements"
    # clause_index distinguishes sibling clauses on pos_max (3 clauses → indices 0,1,2)
    local pos_max_indices
    pos_max_indices=$(echo "$out" | grep '"file":"tests/fixtures/mutants/sample_contract.vow"' | grep '"kind":"contract-weaken"' | grep -oE '"clause_index":[0-9]+' | sort -u | wc -l)
    if [ "$pos_max_indices" -ge 2 ]; then
        printf "  ${GREEN}PASS${RESET} T7: distinct clause_index values for sibling clauses (got %d unique)\n" "$pos_max_indices"
        PASS=$((PASS + 1))
    else
        printf "  ${RED}FAIL${RESET} T7: expected >=2 distinct clause_index values, got %d\n" "$pos_max_indices"
        FAIL=$((FAIL + 1))
        FAILURES+=("T7-clause_index")
    fi
}

t6_skips_extern_c_block() {
    local out rc
    set +e
    out=$(run_vowm list --root tests/fixtures/mutants 2>&1)
    rc=$?
    set -e
    assert_eq "T6: list exit code" "0" "$rc"
    # extern "C" block in sample_extern.vow ends roughly at byte 130;
    # live_outside_extern starts after that, contains a `+` and `1`.
    local in_block_count outside_count
    in_block_count=$(echo "$out" | grep '"file":"tests/fixtures/mutants/sample_extern.vow"' | awk -F'"off":' '{print $2}' | awk -F',' '{print $1}' | awk '$1 < 130' | wc -l)
    outside_count=$(echo "$out" | grep '"file":"tests/fixtures/mutants/sample_extern.vow"' | awk -F'"off":' '{print $2}' | awk -F',' '{print $1}' | awk '$1 >= 130' | wc -l)
    assert_eq "T6: zero sites inside extern \"C\" block" "0" "$in_block_count"
    if [ "$outside_count" -ge 1 ]; then
        printf "  ${GREEN}PASS${RESET} T6: live_outside_extern yields >=1 site (got %d)\n" "$outside_count"
        PASS=$((PASS + 1))
    else
        printf "  ${RED}FAIL${RESET} T6: expected >=1 site outside extern, got %d\n    output:\n%s\n" "$outside_count" "$out"
        FAIL=$((FAIL + 1))
        FAILURES+=("T6-outside")
    fi
}

t5_skips_generate_block() {
    local out rc
    set +e
    out=$(run_vowm list --root tests/fixtures/mutants 2>&1)
    rc=$?
    set -e
    assert_eq "T5: list exit code" "0" "$rc"
    # No site whose `file` is sample_generate.vow may have an `off` < the
    # END marker offset. Easy proxy: count sites in sample_generate.vow that
    # appear before live_after_block. live_after_block starts well past byte 200,
    # while skipped_in_block sites would be before byte 200.
    local in_block_count outside_count
    # GENERATE END marker terminates at byte 160; live_after_block opens at 161+.
    in_block_count=$(echo "$out" | grep '"file":"tests/fixtures/mutants/sample_generate.vow"' | awk -F'"off":' '{print $2}' | awk -F',' '{print $1}' | awk '$1 < 161' | wc -l)
    outside_count=$(echo "$out" | grep '"file":"tests/fixtures/mutants/sample_generate.vow"' | awk -F'"off":' '{print $2}' | awk -F',' '{print $1}' | awk '$1 >= 161' | wc -l)
    assert_eq "T5: zero sites inside GENERATE block" "0" "$in_block_count"
    if [ "$outside_count" -ge 1 ]; then
        printf "  ${GREEN}PASS${RESET} T5: live_after_block yields >=1 site (got %d)\n" "$outside_count"
        PASS=$((PASS + 1))
    else
        printf "  ${RED}FAIL${RESET} T5: expected >=1 site outside GENERATE block, got %d\n" "$outside_count"
        FAIL=$((FAIL + 1))
        FAILURES+=("T5-outside")
    fi
}

t4_list_finds_const_flip_sites() {
    local out rc
    set +e
    out=$(run_vowm list --root tests/fixtures/mutants 2>&1)
    rc=$?
    set -e
    assert_eq "T4: list exit code" "0" "$rc"
    local int_count bool_count
    int_count=$(echo "$out" | grep -c '"kind":"const-flip","from":"0"\|"kind":"const-flip","from":"1"' || true)
    bool_count=$(echo "$out" | grep -c '"kind":"const-flip","from":"true"\|"kind":"const-flip","from":"false"' || true)
    if [ "$int_count" -ge 2 ]; then
        printf "  ${GREEN}PASS${RESET} T4: integer 0/1 const-flips found (got %d)\n" "$int_count"
        PASS=$((PASS + 1))
    else
        printf "  ${RED}FAIL${RESET} T4: expected >=2 int const-flips, got %d\n    output:\n%s\n" "$int_count" "$out"
        FAIL=$((FAIL + 1))
        FAILURES+=("T4-int")
    fi
    if [ "$bool_count" -ge 2 ]; then
        printf "  ${GREEN}PASS${RESET} T4: bool true/false const-flips found (got %d)\n" "$bool_count"
        PASS=$((PASS + 1))
    else
        printf "  ${RED}FAIL${RESET} T4: expected >=2 bool const-flips, got %d\n    output:\n%s\n" "$bool_count" "$out"
        FAIL=$((FAIL + 1))
        FAILURES+=("T4-bool")
    fi
}

t3_list_finds_op_flip_site() {
    local out rc
    set +e
    out=$(run_vowm list --root tests/fixtures/mutants 2>&1)
    rc=$?
    set -e
    assert_eq "T3: list exit code" "0" "$rc"
    # JSONL records precede the summary; assert at least one op-flip record.
    local op_count
    op_count=$(echo "$out" | grep -c '"kind":"op-flip"' || true)
    if [ "$op_count" -ge 1 ]; then
        printf "  ${GREEN}PASS${RESET} T3: at least one op-flip site found (got %d)\n" "$op_count"
        PASS=$((PASS + 1))
    else
        printf "  ${RED}FAIL${RESET} T3: expected >=1 op-flip site, got %d\n    output: %s\n" "$op_count" "$out"
        FAIL=$((FAIL + 1))
        FAILURES+=("T3")
    fi
    # Summary total >= 1
    local summary
    summary=$(echo "$out" | tail -1)
    if echo "$summary" | grep -qE '"total":[1-9]'; then
        printf "  ${GREEN}PASS${RESET} T3: summary total >= 1\n"
        PASS=$((PASS + 1))
    else
        printf "  ${RED}FAIL${RESET} T3: summary total not >= 1\n    summary: %s\n" "$summary"
        FAIL=$((FAIL + 1))
        FAILURES+=("T3-summary")
    fi
}

t2_list_empty_dir_prints_total_zero() {
    local empty="$TMP/empty"
    mkdir -p "$empty"
    local out rc
    set +e
    out=$(run_vowm list --root "$empty" 2>&1)
    rc=$?
    set -e
    assert_eq "T2: list --root <empty> exit code" "0" "$rc"
    local summary
    summary=$(echo "$out" | tail -1)
    assert_grep "T2: summary line contains \"total\":0" '"total":0' "$summary"
}

# --- main ---

setup
t1_version
t1_no_args_exits_1
t1_unknown_subcommand_exits_1
t2_list_empty_dir_prints_total_zero
t3_list_finds_op_flip_site
t4_list_finds_const_flip_sites
t5_skips_generate_block
t6_skips_extern_c_block
t7_finds_contract_weaken_sites
t8_finds_body_replace_sites
t9_sharding_is_deterministic_and_partitions_total
t10_run_round_trip_leaves_files_unchanged
t11_run_classifies_caught_and_missed
t12_tier2_budget_zero_marks_remaining_unrun

echo ""
if [ "$FAIL" -eq 0 ]; then
    printf "${GREEN}All %d tests passed.${RESET}\n" "$PASS"
    exit 0
else
    printf "${RED}%d/%d failed.${RESET}\n" "$FAIL" "$((PASS + FAIL))"
    for f in "${FAILURES[@]}"; do echo "  - $f"; done
    exit 1
fi
