# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

# ─── Section 2: Verify ─────────────────────────────────────────────

section_begin "Section 2: Verify"
for vow_file in examples/*.vow; do
    name=$(basename "$vow_file" .vow)
    if ! grep -q 'vow {' "$vow_file"; then
        continue
    fi

    rust_json="" self_json="" rust_exit=0 self_exit=0
    rust_json=$($RUST verify "$vow_file" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self verify "$vow_file" 2>/dev/null) || self_exit=$?

    if check_empty_output "${name}/verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit"; then
        continue
    fi

    compare_full_json "${name}/verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$vow_file"
done
echo ""

# ─── Section 2b: Verifier C Preamble ──────────────────────────────

section_begin "Section 2b: Verifier C Preamble"

fake_esbmc_dir="$TMPDIR/fake-esbmc"
mkdir -p "$fake_esbmc_dir"
cat > "$fake_esbmc_dir/esbmc" <<'SH'
#!/usr/bin/env bash
set -euo pipefail

capture_dir="${VOW_ESBMC_CAPTURE_DIR:?}"
mkdir -p "$capture_dir"

for arg in "$@"; do
    if [ -f "$arg" ] && [ "${arg##*.}" = "c" ]; then
        dest=$(mktemp "$capture_dir/esbmc.XXXXXX.c")
        cp "$arg" "$dest"
        break
    fi
done

echo "VERIFICATION SUCCESSFUL"
SH
chmod +x "$fake_esbmc_dir/esbmc"

# The vowed u64 parameter is modeled with __VERIFIER_nondet_unsigned_long(), exercising its preamble declaration.
u64_preamble_fixture="$TMPDIR/u64_nondet_preamble.vow"
cat > "$u64_preamble_fixture" <<'VOW'
module U64NondetPreamble

fn keep_u64(x: u64) -> u64 vow {
    ensures: result == x
} {
    x
}

fn main() -> i32 {
    0
}
VOW

check_unsigned_long_preamble_capture() {
    local label="$1" capture_dir="$2" json="$3" exit_code="$4"
    if grep -R -q 'extern unsigned long __VERIFIER_nondet_unsigned_long(void);' "$capture_dir" 2>/dev/null; then
        pass "$label"
    else
        local verify_status=""
        verify_status=$(python3 -c "import json,sys; print(json.loads(sys.argv[1]).get('status',''))" "$json" 2>/dev/null) || verify_status=""
        fail "$label" "missing unsigned-long nondet extern in captured C (verify_status=${verify_status:-unknown}, exit=$exit_code)"
    fi
}

rust_capture="$TMPDIR/rust-esbmc-capture"
self_capture="$TMPDIR/self-esbmc-capture"
mkdir -p "$rust_capture" "$self_capture"

rust_json="" self_json="" rust_exit=0 self_exit=0
rust_json=$(PATH="$fake_esbmc_dir:$PATH" VOW_ESBMC_CAPTURE_DIR="$rust_capture" \
    "$RUST" verify --no-cache --verify-jobs 1 "$u64_preamble_fixture" 2>/dev/null) || rust_exit=$?
self_json=$(PATH="$fake_esbmc_dir:$PATH" VOW_ESBMC_CAPTURE_DIR="$self_capture" \
    run_self verify --no-cache --verify-jobs 1 "$u64_preamble_fixture" 2>/dev/null) || self_exit=$?

if [ -z "$rust_json" ]; then
    fail "verifier-preamble/rust" "empty output (exit=$rust_exit)"
else
    check_unsigned_long_preamble_capture "verifier-preamble/rust" "$rust_capture" "$rust_json" "$rust_exit"
fi

if [ -z "$self_json" ]; then
    fail "verifier-preamble/self" "empty output (exit=$self_exit)"
else
    check_unsigned_long_preamble_capture "verifier-preamble/self" "$self_capture" "$self_json" "$self_exit"
fi

if VOWC_BIN="$SELF" bash tests/esbmc-path-cache/tests.sh >"$TMPDIR/esbmc-path-cache.log" 2>&1; then
    pass "verifier/esbmc-path-cache"
else
    fail "verifier/esbmc-path-cache" "$(tail -10 "$TMPDIR/esbmc-path-cache.log")"
fi

if VOWC_BIN="$SELF" bash tests/verify-bv-fallback/tests.sh >"$TMPDIR/verify-bv-fallback.log" 2>&1; then
    pass "verifier/bv-fallback"
else
    fail "verifier/bv-fallback" "$(tail -10 "$TMPDIR/verify-bv-fallback.log")"
fi

for stage_compiler in rust self; do
    if [ "$stage_compiler" = rust ]; then stage_bin="$RUST"; else stage_bin="$SELF"; fi
    if VOWC_BIN="$stage_bin" bash tests/build-staging/tests.sh >"$TMPDIR/build-staging-$stage_compiler.log" 2>&1; then
        pass "build-staging/$stage_compiler"
    else
        fail "build-staging/$stage_compiler" "$(tail -10 "$TMPDIR/build-staging-$stage_compiler.log")"
    fi
done

if command -v esbmc >/dev/null 2>&1; then
    if VOWC_BIN="$RUST" bash tests/verify-cache-stale-pass/tests.sh >"$TMPDIR/verify-cache-stale-pass.log" 2>&1; then
        pass "verifier/verify-cache-stale-pass"
    else
        fail "verifier/verify-cache-stale-pass" "$(tail -10 "$TMPDIR/verify-cache-stale-pass.log")"
    fi
else
    skip "verifier/verify-cache-stale-pass" "esbmc not on PATH"
fi
echo ""

# ─── Section 2c: Verifier C Parity ────────────────────────────────

# Both compilers must hand ESBMC byte-identical C for the same program: a fake
# esbmc on PATH records what each one writes, and parity.py c diffs the sets of
# distinct sources per function (scripts/parity_c.py).
section_begin "Section 2c: Verifier C Parity"
for vow_file in tests/verify/*.vow tests/verify-fail/*.vow tests/verify-skip/*.vow tests/verify-stress/*.vow tests/verify-fail-multi/*/main.vow; do
    [ -f "$vow_file" ] || continue
    c_parity_log="$TMPDIR/c_parity.log"
    c_parity_status=0
    python3 scripts/parity.py c "$RUST" "$SELF" "$vow_file" >"$c_parity_log" 2>&1 || c_parity_status=$?
    if [ "$c_parity_status" -ne 0 ]; then
        fail "verifier-c/$(basename "$(dirname "$vow_file")")/$(basename "$vow_file" .vow)" "$(head -40 "$c_parity_log")"
    elif grep -q '^SKIP' "$c_parity_log"; then
        # Only verify-skip fixtures may legitimately never reach ESBMC; anywhere
        # else an empty capture means both compilers broke before verifying.
        if [ "$(basename "$(dirname "$vow_file")")" = "verify-skip" ]; then
            skip "verifier-c/verify-skip/$(basename "$vow_file" .vow)" "neither compiler invoked ESBMC"
        else
            fail "verifier-c/$(basename "$(dirname "$vow_file")")/$(basename "$vow_file" .vow)" "neither compiler invoked ESBMC"
        fi
    else
        pass "verifier-c/$(basename "$(dirname "$vow_file")")/$(basename "$vow_file" .vow)"
    fi
done
echo ""
