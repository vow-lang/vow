# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

# ─── Section 13: vow complexity Parity ──────────────────────────────

section_begin "Section 13: vow complexity Parity"
for vow_file in tests/fixtures/complexity/*.vow; do
    [ -f "$vow_file" ] || continue
    name=$(basename "$vow_file" .vow)
    rust_exit=0
    self_exit=0
    rust_json=$("$RUST" complexity "$vow_file" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self complexity "$vow_file" 2>/dev/null) || self_exit=$?
    if [ "$rust_exit" != "0" ] || [ "$self_exit" != "0" ]; then
        fail "complexity/${name}" "rust_exit=${rust_exit} self_exit=${self_exit} (expected both 0)"
        continue
    fi
    golden="tests/fixtures/complexity/${name}.expected.json"
    if [ "$rust_json" != "$self_json" ]; then
        fail "complexity/${name}" "JSON differs between compilers"
    elif [ -f "$golden" ] && [ "$rust_json" != "$(cat "$golden")" ]; then
        fail "complexity/${name}" "output differs from golden ${name}.expected.json"
    else
        pass "complexity/${name} (byte-identical + golden)"
    fi
    # AST<->IR self-check: the AST decision-count cyclomatic and the IR
    # branch-count cyclomatic_ir are independent computations that must agree
    # on these clean-control-flow fixtures (cross-validates both).
    # `break_value` is exempt: break-with-value requires an unconditional `loop`,
    # which the AST counts as a decision but the branch-count cyclomatic_ir does
    # not (the documented AST<->IR divergence) — agreement cannot hold here by
    # construction. Byte-identity + golden above still cover this fixture.
    if [ "$name" = "break_value" ]; then
        skip "complexity/${name}/ast-ir-cyclomatic-agree" "loop divergence (expected)"
    elif [ "$name" = "predicate_control_flow" ]; then
        skip "complexity/${name}/ast-ir-cyclomatic-agree" "contract predicate control-flow lowers to IR branches outside the body AST count"
    elif echo "$rust_json" | python3 -c "import sys,json; d=json.load(sys.stdin); sys.exit(0 if all(f['structural']['cyclomatic']==f['structural']['cyclomatic_ir'] for f in d['files'][0]['functions']) else 1)" 2>/dev/null; then
        pass "complexity/${name}/ast-ir-cyclomatic-agree"
    else
        fail "complexity/${name}/ast-ir-cyclomatic-agree" "cyclomatic != cyclomatic_ir"
    fi
done
# Exit-code gating must agree across compilers (deep has cyclomatic 4).
# These commands exit nonzero by design, so capture the code with `|| code=$?`
# (a bare `cmd; code=$?` would trip `set -e` and abort the script here).
r_gate=0; "$RUST" complexity tests/fixtures/complexity/nested.vow --max-cyclomatic 1 >/dev/null 2>&1 || r_gate=$?
s_gate=0; run_self complexity tests/fixtures/complexity/nested.vow --max-cyclomatic 1 >/dev/null 2>&1 || s_gate=$?
if [ "$r_gate" = "$s_gate" ] && [ "$r_gate" != "0" ]; then
    pass "complexity/exit-gating (--max-cyclomatic 1 -> $r_gate, both)"
else
    fail "complexity/exit-gating" "rust=$r_gate self=$s_gate (expected equal, nonzero)"
fi
# A malformed --max-* value must fail closed (nonzero) in BOTH compilers, never
# silently disable the opt-in gate. Exact codes differ (clap=2, self-hosted=1).
r_bad=0; "$RUST" complexity tests/fixtures/complexity/params_basic.vow --max-score notanint >/dev/null 2>&1 || r_bad=$?
s_bad=0; run_self complexity tests/fixtures/complexity/params_basic.vow --max-score notanint >/dev/null 2>&1 || s_bad=$?
if [ "$r_bad" != "0" ] && [ "$s_bad" != "0" ]; then
    pass "complexity/gate-fail-closed (--max-score notanint -> rust=$r_bad self=$s_bad, both nonzero)"
else
    fail "complexity/gate-fail-closed" "rust=$r_bad self=$s_bad (expected both nonzero)"
fi
# A non-ASCII (UTF-8) source path must stay byte-identical across compilers: the
# JSON escaper must emit raw UTF-8 bytes, not mojibake (Rust byte-as-char bug).
utf8dir=$(mktemp -d "$TMPDIR/utf8.XXXXXX")
cp tests/fixtures/complexity/params_basic.vow "$utf8dir/café.vow"
if diff -q <("$RUST" complexity "$utf8dir/café.vow" 2>/dev/null) <(run_self complexity "$utf8dir/café.vow" 2>/dev/null) >/dev/null; then
    pass "complexity/utf8-path-parity (café.vow byte-identical)"
else
    fail "complexity/utf8-path-parity" "non-ASCII path JSON diverges between compilers"
fi
rm -rf "$utf8dir"
# JSON named-control escapes in source paths must be byte-identical and must
# round-trip through a parser. Backspace/form-feed are control bytes that remain
# shell-log friendly but distinguish `\b`/`\f` from generic `?` escaping.
escape_dir=$(mktemp -d "$TMPDIR/escape.XXXXXX")
escape_name=$'quote"backslash\\backspace\bformfeed\f.vow'
escape_path="$escape_dir/$escape_name"
cp tests/fixtures/complexity/params_basic.vow "$escape_path"
rust_escape_json=$("$RUST" complexity "$escape_path" 2>/dev/null)
self_escape_json=$(run_self complexity "$escape_path" 2>/dev/null)
if [ "$rust_escape_json" = "$self_escape_json" ] && COMPLEXITY_EXPECT_PATH="$escape_path" python3 -c 'import json, os, sys; d=json.load(sys.stdin); sys.exit(0 if d["files"][0]["file"] == os.environ["COMPLEXITY_EXPECT_PATH"] else 1)' <<< "$rust_escape_json"; then
    pass "complexity/control-path-parity (quote/backslash/backspace/form-feed round-trip)"
else
    fail "complexity/control-path-parity" "escaped control-byte path did not stay byte-identical and round-trip"
fi
rm -rf "$escape_dir"
# Missing source file must produce a command-specific error in BOTH compilers and
# exit nonzero. Regression: the self-hosted compiler previously printed a generic
# `usage: vowc <command> ...` line for every subcommand (confusing `vow complexity` UX).
r_nosrc=0; "$RUST" complexity >/dev/null 2>"$TMPDIR/r_nosrc.err" || r_nosrc=$?
s_nosrc=0; run_self complexity >/dev/null 2>"$TMPDIR/s_nosrc.err" || s_nosrc=$?
if [ "$r_nosrc" != "0" ] && [ "$s_nosrc" != "0" ] \
   && grep -q "vow complexity: source file required" "$TMPDIR/r_nosrc.err" \
   && grep -q "vow complexity: source file required" "$TMPDIR/s_nosrc.err"; then
    pass "complexity/no-source-message (command-specific, both nonzero)"
else
    fail "complexity/no-source-message" "rust=$r_nosrc self=$s_nosrc (expected nonzero + 'vow complexity: source file required')"
fi
# The same per-command message must hold for the other three subcommands this PR
# fixed through the shared frontend_path_from_argv change, so a regression in any
# of them is caught — not just complexity.
for subcmd in build verify contracts; do
    r_sub=0; "$RUST" "$subcmd" >/dev/null 2>"$TMPDIR/r_${subcmd}_nosrc.err" || r_sub=$?
    s_sub=0; run_self "$subcmd" >/dev/null 2>"$TMPDIR/s_${subcmd}_nosrc.err" || s_sub=$?
    if [ "$r_sub" != "0" ] && [ "$s_sub" != "0" ] \
       && grep -q "vow ${subcmd}: source file required" "$TMPDIR/r_${subcmd}_nosrc.err" \
       && grep -q "vow ${subcmd}: source file required" "$TMPDIR/s_${subcmd}_nosrc.err"; then
        pass "${subcmd}/no-source-message (command-specific, both nonzero)"
    else
        fail "${subcmd}/no-source-message" "rust=$r_sub self=$s_sub (expected nonzero + 'vow ${subcmd}: source file required')"
    fi
done
# Legacy flag-only invocations also have no source path, but they bypass
# frontend_path_from_argv. Keep that fallback aligned with Rust instead of
# regressing to the old generic `usage: main ...` text.
r_legacy=0; "$RUST" --no-verify >/dev/null 2>"$TMPDIR/r_legacy_nosrc.err" || r_legacy=$?
s_legacy=0; run_self --no-verify >/dev/null 2>"$TMPDIR/s_legacy_nosrc.err" || s_legacy=$?
if [ "$r_legacy" != "0" ] && [ "$s_legacy" != "0" ] \
   && grep -q "vow: source file required (try --help or use a subcommand)" "$TMPDIR/r_legacy_nosrc.err" \
   && grep -q "vow: source file required (try --help or use a subcommand)" "$TMPDIR/s_legacy_nosrc.err" \
   && ! grep -q "usage: main" "$TMPDIR/r_legacy_nosrc.err" \
   && ! grep -q "usage: main" "$TMPDIR/s_legacy_nosrc.err"; then
    pass "legacy/no-source-message (fallback-specific, both nonzero)"
else
    fail "legacy/no-source-message" "rust=$r_legacy self=$s_legacy (expected nonzero + legacy source-required message, no 'usage: main')"
fi
# `--help --human` must document the complexity command in BOTH compilers. The JSON
# --help always listed it; the legacy human help previously omitted it.
if grep -q "vow complexity \[OPTIONS\]" <<< "$rust_human" \
   && grep -q "vow complexity \[OPTIONS\]" <<< "$self_human"; then
    pass "complexity/human-help-documented (both compilers)"
else
    fail "complexity/human-help-documented" "vow complexity missing from --help --human"
fi
echo ""
