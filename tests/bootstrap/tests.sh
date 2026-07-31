#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

WARNING="warning: --no-verify supersedes --stage3-no-verify"

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

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

test_combined_flags_warn_and_preserve_no_verify_precedence
test_single_flags_do_not_warn

echo "bootstrap tests passed"
