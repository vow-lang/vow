#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

WARNING="warning: --no-verify supersedes --stage3-no-verify"

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT
# Kill signals re-raise so the EXIT handler above still does the removal:
# EXIT alone does not fire on an untrapped SIGTERM, so a process-group kill
# would strand this scratch tree. See scripts/full_test.sh.
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

fail() {
    echo "FAIL: $1" >&2
    exit 1
}

assert_contains() {
    local haystack="$1"
    local needle="$2"
    local label="$3"

    if [[ "$haystack" != *"$needle"* ]]; then
        fail "$label: expected output to contain: $needle"
    fi
}

assert_not_contains() {
    local haystack="$1"
    local needle="$2"
    local label="$3"

    if [[ "$haystack" == *"$needle"* ]]; then
        fail "$label: expected output not to contain: $needle"
    fi
}

invocation_count() {
    wc -l <"$1" | tr -d ' '
}

make_fake_repo() {
    local repo="$1"

    mkdir -p "$repo/scripts" "$repo/compiler" "$repo/target/release"
    cp scripts/bootstrap.sh "$repo/scripts/bootstrap.sh"
    chmod +x "$repo/scripts/bootstrap.sh"

    cp tests/bootstrap/fake_compiler.sh "$repo/target/release/vow"
    chmod +x "$repo/target/release/vow"
}

run_bootstrap() {
    local name="$1"
    shift

    local repo="$TMPDIR/$name"
    local stdout="$TMPDIR/$name.stdout"
    local stderr="$TMPDIR/$name.stderr"
    local invocations="$TMPDIR/$name.invocations"

    make_fake_repo "$repo"
    if ! VOW_BOOTSTRAP_TEST_LOG="$invocations" bash "$repo/scripts/bootstrap.sh" --skip-cargo "$@" >"$stdout" 2>"$stderr"; then
        fail "$name: bootstrap failed: $(tail -20 "$stderr")"
    fi

    printf '%s\n' "$stdout" "$stderr" "$invocations"
}

test_combined_flags_warn_and_preserve_no_verify_precedence() {
    local name

    for name in no_verify_first stage3_no_verify_first; do
        local -a flags
        if [ "$name" = no_verify_first ]; then
            flags=(--no-verify --stage3-no-verify)
        else
            flags=(--stage3-no-verify --no-verify)
        fi

        local paths
        local stdout
        local stderr
        local invocations
        paths=$(run_bootstrap "$name" "${flags[@]}")
        stdout=$(sed -n '1p' <<<"$paths")
        stderr=$(sed -n '2p' <<<"$paths")
        invocations=$(sed -n '3p' <<<"$paths")

        local warning_count
        warning_count=$(grep -Fxc -- "$WARNING" "$stderr" || true)
        [ "$warning_count" -eq 1 ] || fail "$name: expected exactly one warning on stderr"
        assert_not_contains "$(cat "$stdout")" "$WARNING" "$name stdout"

        local invocation_count
        invocation_count=$(wc -l <"$invocations" | tr -d ' ')
        [ "$invocation_count" -eq 3 ] || fail "$name: expected three compiler invocations"
        while IFS= read -r invocation; do
            assert_contains "$invocation" "--no-verify" "$name compiler invocation"
            assert_not_contains "$invocation" "--verify-jobs 1" "$name compiler invocation"
        done <"$invocations"
    done
}

test_single_flags_do_not_warn() {
    local name
    local flag

    for name in no_verify_only stage3_no_verify_only; do
        if [ "$name" = no_verify_only ]; then
            flag=--no-verify
        else
            flag=--stage3-no-verify
        fi

        local paths
        local stderr
        paths=$(run_bootstrap "$name" "$flag")
        stderr=$(sed -n '2p' <<<"$paths")
        assert_not_contains "$(cat "$stderr")" "$WARNING" "$name stderr"
    done
}

test_no_cache_flag_is_forwarded_to_all_stages() {
    local paths
    local invocations
    paths=$(run_bootstrap no_cache --no-cache)
    invocations=$(sed -n '3p' <<<"$paths")

    local invocation_count
    invocation_count=$(wc -l <"$invocations" | tr -d ' ')
    [ "$invocation_count" -eq 4 ] || fail "no_cache: expected four compiler invocations"
    while IFS= read -r invocation; do
        assert_contains "$invocation" "--no-cache" "no_cache compiler invocation"
    done <"$invocations"
}

test_no_cache_combined_with_no_verify() {
    local paths
    local invocations
    paths=$(run_bootstrap no_cache_no_verify --no-cache --no-verify)
    invocations=$(sed -n '3p' <<<"$paths")

    local invocation_count
    invocation_count=$(wc -l <"$invocations" | tr -d ' ')
    [ "$invocation_count" -eq 3 ] || fail "no_cache_no_verify: expected three compiler invocations"
    while IFS= read -r invocation; do
        assert_contains "$invocation" "--no-cache" "no_cache_no_verify compiler invocation"
        assert_contains "$invocation" "--no-verify" "no_cache_no_verify compiler invocation"
    done <"$invocations"
}

run_bootstrap_expect_failure() {
    local name="$1"
    shift

    local repo="$TMPDIR/$name"
    local stdout="$TMPDIR/$name.stdout"
    local stderr="$TMPDIR/$name.stderr"
    local invocations="$TMPDIR/$name.invocations"

    make_fake_repo "$repo"
    if VOW_BOOTSTRAP_TEST_LOG="$invocations" bash "$repo/scripts/bootstrap.sh" --skip-cargo "$@" >"$stdout" 2>"$stderr"; then
        fail "$name: bootstrap unexpectedly succeeded"
    fi

    printf '%s\n' "$stdout" "$stderr" "$invocations" "$repo"
}

test_default_sequence() {
    local paths
    local invocations
    paths=$(run_bootstrap default_sequence)
    invocations=$(sed -n '3p' <<<"$paths")

    [ "$(invocation_count "$invocations")" -eq 4 ] || fail "default_sequence: expected four compiler invocations"

    local stage1 stage2 stage3 verify
    stage1=$(sed -n '1p' "$invocations")
    stage2=$(sed -n '2p' "$invocations")
    stage3=$(sed -n '3p' "$invocations")
    verify=$(sed -n '4p' "$invocations")

    assert_contains "$stage1" "target/release/vow build" "default_sequence stage 1"
    assert_contains "$stage1" "--verify-jobs 1" "default_sequence stage 1"
    assert_not_contains "$stage1" "--no-verify" "default_sequence stage 1"

    assert_contains "$stage2" "build/vowc build" "default_sequence stage 2"
    assert_contains "$stage2" "--no-verify" "default_sequence stage 2"
    assert_not_contains "$stage2" "--verify-jobs" "default_sequence stage 2"

    assert_contains "$stage3" "build/vowc2 build" "default_sequence stage 3"
    assert_contains "$stage3" "--no-verify" "default_sequence stage 3"
    assert_not_contains "$stage3" "--verify-jobs" "default_sequence stage 3"

    assert_contains "$verify" "build/vowc2 verify" "default_sequence verify pass"
    assert_contains "$verify" "--verify-jobs 1" "default_sequence verify pass"
    assert_contains "$verify" "compiler/main.vow" "default_sequence verify pass"
    assert_not_contains "$verify" "--no-verify" "default_sequence verify pass"
    assert_not_contains "$verify" " -o " "default_sequence verify pass"
}

test_no_verify_runs_no_verify_pass() {
    local paths
    local invocations
    paths=$(run_bootstrap no_verify_pass --no-verify)
    invocations=$(sed -n '3p' <<<"$paths")

    [ "$(invocation_count "$invocations")" -eq 3 ] || fail "no_verify_pass: expected three compiler invocations"
    while IFS= read -r invocation; do
        assert_contains "$invocation" "--no-verify" "no_verify_pass compiler invocation"
        assert_not_contains "$invocation" " verify " "no_verify_pass compiler invocation"
    done <"$invocations"
}

test_stage3_no_verify_is_a_noop() {
    local paths
    local default_invocations
    local flag_invocations
    paths=$(run_bootstrap noop_default)
    default_invocations=$(sed -n '3p' <<<"$paths")
    paths=$(run_bootstrap noop_flag --stage3-no-verify)
    flag_invocations=$(sed -n '3p' <<<"$paths")

    [ "$(invocation_count "$flag_invocations")" -eq 4 ] || fail "stage3_no_verify: expected four compiler invocations"
    # Absolute repo paths differ per run; compare the relative command tails.
    diff <(sed -E 's#^[^ ]*/(build|target)#\1#' "$default_invocations") \
         <(sed -E 's#^[^ ]*/(build|target)#\1#' "$flag_invocations") >/dev/null ||
        fail "stage3_no_verify: invocation sequence differs from the default"
}

test_no_cache_forwarded_to_verify_pass() {
    local paths
    local invocations
    paths=$(run_bootstrap no_cache_verify --no-cache)
    invocations=$(sed -n '3p' <<<"$paths")

    assert_contains "$(sed -n '4p' "$invocations")" "verify" "no_cache_verify pass"
    assert_contains "$(sed -n '4p' "$invocations")" "--no-cache" "no_cache_verify pass"
}

test_verify_failure_fails_bootstrap_without_promotion() {
    local paths
    paths=$(VOW_BOOTSTRAP_TEST_VERIFY_FAIL=1 run_bootstrap_expect_failure verify_fails)
    local stdout repo
    stdout=$(sed -n '1p' <<<"$paths")
    repo=$(sed -n '4p' <<<"$paths")

    assert_not_contains "$(cat "$stdout")" "Bootstrap successful" "verify_fails stdout"
    [ -e "$repo/build/vowc2" ] || fail "verify_fails: build/vowc2 must not be promoted"
    [ ! -e "$repo/build/vowc" ] || [ -e "$repo/build/vowc2" ] ||
        fail "verify_fails: build/vowc must not be replaced"
}

test_fixed_point_mismatch_skips_verify_pass() {
    local paths
    paths=$(VOW_BOOTSTRAP_TEST_DIVERGE=1 run_bootstrap_expect_failure diverges)
    local invocations
    invocations=$(sed -n '3p' <<<"$paths")

    [ "$(invocation_count "$invocations")" -eq 3 ] || fail "diverges: expected three compiler invocations"
    assert_not_contains "$(cat "$invocations")" " verify " "diverges invocations"
}

test_combined_flags_warn_and_preserve_no_verify_precedence
test_single_flags_do_not_warn
test_default_sequence
test_no_verify_runs_no_verify_pass
test_stage3_no_verify_is_a_noop
test_no_cache_forwarded_to_verify_pass
test_verify_failure_fails_bootstrap_without_promotion
test_fixed_point_mismatch_skips_verify_pass
test_no_cache_flag_is_forwarded_to_all_stages
test_no_cache_combined_with_no_verify

echo "bootstrap tests passed"
