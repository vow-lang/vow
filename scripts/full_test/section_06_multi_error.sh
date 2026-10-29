# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

# ─── Section 6: Multi-Module ───────────────────────────────────────

section_begin "Section 6: Multi-Module"

for multi in stack geometry bignum gc math heap; do
    main_file="stdlib/${multi}/main.vow"
    printf "${BOLD}%s${RESET}\n" "$multi"

    # build --no-verify
    rust_json="" self_json="" rust_exit=0 self_exit=0
    rust_json=$($RUST build --no-verify "$main_file" -o "$TMPDIR/rust_${multi}_main" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self build --no-verify "$main_file" -o "$TMPDIR/self_${multi}_main" 2>/dev/null) || self_exit=$?

    if ! check_empty_output "${multi}/build-no-verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit"; then
        compare_json "${multi}/build-no-verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$main_file"
    fi

    # verify (isolated cache: a shared ~/.cache/vow must not influence the result)
    rust_json="" self_json="" rust_exit=0 self_exit=0
    multi_cache_dir=$(mktemp -d "$TMPDIR/multi-cache.XXXXXX")
    rust_json=$(VOW_CACHE_DIR="$multi_cache_dir" $RUST verify "$main_file" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self verify --backend esbmc "$main_file" 2>/dev/null) || self_exit=$?
    rm -rf "$multi_cache_dir"

    if ! check_empty_output "${multi}/verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit"; then
        compare_full_json "${multi}/verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$main_file"
    fi

    # runtime execution
    compare_runtime "${multi}/runtime" "$TMPDIR/rust_${multi}_main" "$TMPDIR/self_${multi}_main"
done
echo ""

# ─── Section 6b: Multi-Module Fixtures (tests/multi/) ──────────────
# Discovers every tests/multi/<dir>/main.vow, builds it with both
# compilers (use-based module loading resolves siblings), checks
# rust/self parity, and validates its // TEST: directives. Covers the
# vmod_* serialization reject-path fixtures, which were otherwise built
# only by the concat bootstrap and never executed.

section_begin "Section 6b: Multi-Module Fixtures"

for dir in tests/multi/*/; do
    name=$(basename "$dir")
    main_file="${dir}main.vow"
    [ -f "$main_file" ] || continue
    printf "${BOLD}%s${RESET}\n" "$name"

    rust_json="" self_json="" rust_exit=0 self_exit=0
    rust_json=$($RUST build --no-verify "$main_file" -o "$TMPDIR/rust_multi_${name}" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self build --no-verify "$main_file" -o "$TMPDIR/self_multi_${name}" 2>/dev/null) || self_exit=$?

    if check_empty_output "${name}/build" "$rust_json" "$self_json" "$rust_exit" "$self_exit"; then
        continue
    fi
    compare_json "${name}/build" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$main_file"

    # rust/self runtime parity (exit code + stdout)
    compare_runtime "${name}/runtime" "$TMPDIR/rust_multi_${name}" "$TMPDIR/self_multi_${name}"

    # Validate // TEST: stdout directive (single-line) against the rust exe;
    # compare_runtime above guarantees the self exe matches.
    expected=$(sed -n 's|^// TEST: stdout "\(.*\)"$|\1|p' "$main_file" | head -1)
    if [ -n "$expected" ]; then
        actual=$(run_stdout_with_optional_stdin "$TMPDIR/rust_multi_${name}" "") || true
        expected_decoded=$(printf '%b' "$expected")
        if [ "$actual" = "$expected_decoded" ]; then
            pass "${name}/expected"
        else
            fail "${name}/expected" "expected '$expected' got '$(echo "$actual" | head -c 80)'"
        fi
    fi

    # Validate // TEST: exit directive (the vmod decode-reject fixtures exit 1
    # after printing the structured decode error; the encode-side invariant
    # trap in vmod_rejects_invalid_instdata_encode still exits 134, the
    # reserved runtime-abort code — see #877).
    expected_exit=$(sed -n 's|^// TEST: exit \([0-9]*\)$|\1|p' "$main_file" | head -1)
    if [ -n "$expected_exit" ]; then
        actual_exit=0
        run_discard_with_optional_stdin "$TMPDIR/rust_multi_${name}" "" || actual_exit=$?
        if [ "$actual_exit" = "$expected_exit" ]; then
            pass "${name}/exit"
        else
            fail "${name}/exit" "expected exit $expected_exit got $actual_exit"
        fi
    fi
done
echo ""

# ─── Section 7: Error Handling ─────────────────────────────────────

run_promoted_error_tests
echo ""
