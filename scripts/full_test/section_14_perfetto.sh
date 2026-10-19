# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

# ─── Section 6: Perfetto Trace (--perfetto, #784) ───────────────────
section_begin "Section 6: Perfetto Trace"
ptrace_dir=$(mktemp -d "$TMPDIR/ptrace.XXXXXX")
# (a) build --no-verify --perfetto: valid gz trace with frontend + codegen spans + counters.
if run_self build --no-verify --perfetto "$ptrace_dir/build.json.gz" examples/hello.vow -o "$ptrace_dir/hello" >/dev/null 2>&1 \
   && python3 scripts/validate_trace_gz.py "$ptrace_dir/build.json.gz" --require parse,codegen >/dev/null 2>&1; then
    pass "perfetto/build-trace (gz+json, parse+codegen spans)"
else
    fail "perfetto/build-trace" "missing or malformed trace from build --no-verify --perfetto"
fi
# (b) build --no-verify stdout JSON must be byte-identical with/without --perfetto (same -o).
boff=$(run_self build --no-verify examples/hello.vow -o "$ptrace_dir/h" 2>/dev/null)
bon=$(run_self build --no-verify --perfetto "$ptrace_dir/b.json.gz" examples/hello.vow -o "$ptrace_dir/h" 2>/dev/null)
if [ "$boff" = "$bon" ]; then
    pass "perfetto/build-stdout-parity (build JSON identical with/without --perfetto)"
else
    fail "perfetto/build-stdout-parity" "stdout build JSON changed with --perfetto"
fi
# (c) verify --perfetto: esbmc proof span present + stdout parity (needs ESBMC).
if command -v esbmc >/dev/null 2>&1; then
    if run_self verify --perfetto "$ptrace_dir/verify.json.gz" examples/divide.vow >/dev/null 2>&1 \
       && python3 scripts/validate_trace_gz.py "$ptrace_dir/verify.json.gz" --require parse,esbmc >/dev/null 2>&1; then
        pass "perfetto/verify-trace (esbmc proof span + flow)"
    else
        fail "perfetto/verify-trace" "missing or malformed trace from verify --perfetto"
    fi
    voff=$(run_self verify examples/divide.vow 2>/dev/null)
    von=$(run_self verify --perfetto "$ptrace_dir/parity.json.gz" examples/divide.vow 2>/dev/null)
    if [ "$voff" = "$von" ]; then
        pass "perfetto/verify-stdout-parity (verify JSON identical with/without --perfetto)"
    else
        fail "perfetto/verify-stdout-parity" "stdout verify JSON changed with --perfetto"
    fi
else
    skip "perfetto/verify-trace" "ESBMC not installed"
    skip "perfetto/verify-stdout-parity" "ESBMC not installed"
fi
rm -rf "$ptrace_dir"
echo ""
