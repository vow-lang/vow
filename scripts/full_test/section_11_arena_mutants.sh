# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

# ─── Section 11: Arena Primitive ESBMC Verification ────────────────

section_begin "Section 11: Arena Primitive Verification"
default_solver_command=$(env -u SOLVER_FLAGS make -s -C vow-runtime/verify -n verify)
override_solver_command=$(make -s -C vow-runtime/verify -n \
    SOLVER_FLAGS="--z3 --incremental-bmc" verify)
if [[ "$default_solver_command" == *"--64 --boolector"* \
    && "$override_solver_command" == *"--64 --z3 --incremental-bmc"* \
    && "$override_solver_command" != *"--boolector"* \
    && "$override_solver_command" != *'"--z3 --incremental-bmc"'* ]]; then
    pass "arena/solver-flags"
else
    fail "arena/solver-flags" \
        "default=$default_solver_command; override=$override_solver_command"
fi

# The shared runner applies the same VOW_ULIMIT_KB cap as run_self, so under a
# cap this also guards against a regression in the verify invocation. With the
# single-shot --unwind 5 --boolector command (#516) the harness stays below
# 2 GB, but --incremental-bmc / Bitwuzla blew past it (#546).
if command -v esbmc >/dev/null 2>&1; then
    if scripts/verify_arena.sh >"$TMPDIR/arena_verify.log" 2>&1; then
        pass "arena/esbmc"
    else
        fail "arena/esbmc" "$(tail -5 "$TMPDIR/arena_verify.log")"
    fi
else
    skip "arena/esbmc" "esbmc not on PATH"
fi
echo ""

# ─── Section 12: vowc mutants Smoke Test ────────────────────────────

section_begin "Section 12: vowc mutants Smoke Test"
if [ -f tests/mutants/tests.sh ]; then
    if (apply_vmem_limit; VOWC_BIN="$SELF" bash tests/mutants/tests.sh) >"$TMPDIR/vowc-mutants-tests.log" 2>&1; then
        pass "vowc-mutants/tests"
    else
        fail "vowc-mutants/tests" "$(tail -10 "$TMPDIR/vowc-mutants-tests.log")"
    fi
else
    skip "vowc-mutants" "tests/mutants/tests.sh not present"
fi
echo ""
