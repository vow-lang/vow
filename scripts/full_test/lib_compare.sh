# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

compare_runtime() {
    local label="$1" rust_bin="$2" self_bin="$3" stdin_file="${4:-}" vow_file="${5:-}"

    if [ ! -x "$rust_bin" ] || [ ! -x "$self_bin" ]; then
        skip "$label" "binary not found"
        return
    fi

    local known_div=""
    if [ -n "$vow_file" ]; then
        known_div=$(sed -n 's|^// TEST: known-divergence \([0-9]*\) "\(.*\)"$|#\1: \2|p' "$vow_file" | head -1)
    fi

    local rust_out="" self_out="" rust_exit=0 self_exit=0
    if [ -n "$stdin_file" ]; then
        rust_out=$("$rust_bin" < "$stdin_file" 2>/dev/null) || rust_exit=$?
        self_out=$(run_self_bin "$self_bin" < "$stdin_file" 2>/dev/null) || self_exit=$?
    else
        rust_out=$("$rust_bin" </dev/null 2>/dev/null) || rust_exit=$?
        self_out=$(run_self_bin "$self_bin" </dev/null 2>/dev/null) || self_exit=$?
    fi

    local errors=()
    if [ "$rust_exit" != "$self_exit" ]; then
        errors+=("exit: $rust_exit vs $self_exit")
    fi
    if [ "$rust_out" != "$self_out" ]; then
        errors+=("stdout differs")
    fi

    # The directive documents a stdout miscompile and nothing else. An exit-code
    # change — a crash, or a signal death such as 139 — is never what it covers,
    # so it must still FAIL on a fixture carrying the directive. Suppressing the
    # whole errors array would let a documented stdout gap hide a new crash.
    if [ -n "$known_div" ]; then
        local uncovered="" stdout_diverged=0 e
        for e in ${errors[@]+"${errors[@]}"}; do
            if [ "$e" = "stdout differs" ]; then
                stdout_diverged=1
            else
                uncovered="${uncovered:+$uncovered; }$e"
            fi
        done
        if [ -n "$uncovered" ]; then
            fail "$label" "known-divergence ($known_div) does not cover: $uncovered"
        elif [ "$stdout_diverged" -eq 0 ]; then
            fail "$label" "known-divergence ($known_div) no longer reproduces — remove the directive and update docs/equivalence/ledger.json"
        else
            skip "$label" "known divergence ($known_div)"
        fi
        return
    fi

    if [ ${#errors[@]} -eq 0 ]; then
        pass "$label"
    else
        fail "$label" "$(IFS='; '; echo "${errors[*]}")"
    fi
}

run_stdout_with_optional_stdin() {
    local bin="$1" stdin_file="${2:-}"
    if [ -n "$stdin_file" ]; then
        "$bin" < "$stdin_file" 2>/dev/null
    else
        "$bin" </dev/null 2>/dev/null
    fi
}

run_discard_with_optional_stdin() {
    local bin="$1" stdin_file="${2:-}"
    if [ -n "$stdin_file" ]; then
        "$bin" < "$stdin_file" >/dev/null 2>/dev/null
    else
        "$bin" </dev/null >/dev/null 2>/dev/null
    fi
}

capture_stderr_with_optional_stdin() {
    local err_file="$1" stdin_file="${2:-}"; shift 2
    "$@" < "${stdin_file:-/dev/null}" >/dev/null 2>"$err_file"
}

compare_error() {
    run_parity error "$@"
}

# Verdict for a both-compilers call site where at least one side's JSON is
# empty. Both empty means neither compiler produced anything to compare, a
# benign SKIP; exactly one empty means that compiler produced nothing while
# its counterpart succeeded, which is a real divergence (most often a crash)
# and must FAIL rather than be silently swallowed as a SKIP.
# Returns 0 (caller should skip the comparison) after logging SKIP or FAIL;
# returns 1 (caller should proceed to compare_json/compare_error) if neither
# side is empty.
check_empty_output() {
    local label="$1" rust_json="$2" self_json="$3" rust_exit="$4" self_exit="$5"
    local rust_empty=0 self_empty=0
    [ -z "$rust_json" ] && rust_empty=1
    [ -z "$self_json" ] && self_empty=1
    if [ "$rust_empty" = 0 ] && [ "$self_empty" = 0 ]; then
        return 1
    fi
    local verdict
    verdict=$(python3 scripts/parity.py empty-output "$rust_empty" "$self_empty")
    if [ "$verdict" = "SKIP" ]; then
        skip "$label" "empty output (rust=$rust_exit, self=$self_exit)"
    else
        fail "$label" "one-sided empty output (rust=$rust_exit, self=$self_exit)"
    fi
    return 0
}

# Every `// TEST: stderr "<text>"` line of a fixture must occur as a substring of
# the captured stderr, with the same decoding tests/run_tests.sh applies (`\"` is
# a quote and backslash escapes such as `\n` are expanded). All lines are
# checked, not just the last. Prints one line per missing pattern.
missing_stderr_directives() {
    local vow_file="$1" stderr_file="$2" line pattern actual
    actual=$(cat "$stderr_file")
    while IFS= read -r line; do
        pattern="${line#// TEST: stderr \"}"
        pattern="${pattern%\"}"
        [ -z "$pattern" ] && continue
        pattern="${pattern//\\\"/\"}"
        pattern=$(printf '%b' "$pattern")
        if [[ "$actual" != *"$pattern"* ]]; then
            printf '%s\n' "$pattern"
        fi
    done < <(grep '^// TEST: stderr "' "$vow_file")
}

# Check a fixture's stderr directives against both compilers' captured stderr
# and report one pass/fail. Does nothing when the fixture has no directive.
check_stderr_directives() {
    local label="$1" vow_file="$2" rust_err_file="$3" self_err_file="$4"
    if ! grep -q '^// TEST: stderr "' "$vow_file"; then
        return
    fi
    local errors=() missing
    missing=$(missing_stderr_directives "$vow_file" "$rust_err_file")
    if [ -n "$missing" ]; then errors+=("rust stderr missing: ${missing//$'\n'/ | }"); fi
    missing=$(missing_stderr_directives "$vow_file" "$self_err_file")
    if [ -n "$missing" ]; then errors+=("self stderr missing: ${missing//$'\n'/ | }"); fi
    if [ ${#errors[@]} -eq 0 ]; then
        pass "$label"
    else
        fail "$label" "$(IFS='; '; echo "${errors[*]}")"
    fi
}
