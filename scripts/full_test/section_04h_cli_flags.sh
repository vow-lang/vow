# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

# ─── Section 4h: Unknown CLI flags (tests/cli-flags/, #580) ────────
#
# Both compilers must reject a flag the subcommand does not implement with
# exit 2 and `unexpected argument '<flag>' found` (clap in Rust,
# compiler/cli_flags.vow in the self-hosted driver).

section_begin "Section 4h: Unknown CLI flags"
cli_flags_rust_log="$TMPDIR/cli-flags-rust.log"
if VOWC_BIN="$RUST" VOWC_KIND=rust bash tests/cli-flags/tests.sh >"$cli_flags_rust_log" 2>&1; then
    pass "cli-flags/rust"
else
    fail "cli-flags/rust" "$(tail -20 "$cli_flags_rust_log")"
fi
cli_flags_self_log="$TMPDIR/cli-flags-self.log"
if VOWC_BIN="$SELF" VOWC_KIND=self bash tests/cli-flags/tests.sh >"$cli_flags_self_log" 2>&1; then
    pass "cli-flags/self-hosted"
else
    fail "cli-flags/self-hosted" "$(tail -20 "$cli_flags_self_log")"
fi
echo ""

# ─── Section 4i: `decl` declaration stubs (tests/decl/, #595) ──────
#
# Both compilers must write byte-identical `.vow.d` stubs for the same
# source (goldens generated once from the Rust compiler).

section_begin "Section 4i: decl declaration stubs"
decl_rust_log="$TMPDIR/decl-rust.log"
if VOWC_BIN="$RUST" VOWC_KIND=rust bash tests/decl/tests.sh >"$decl_rust_log" 2>&1; then
    pass "decl/rust"
else
    fail "decl/rust" "$(tail -20 "$decl_rust_log")"
fi
decl_self_log="$TMPDIR/decl-self.log"
if VOWC_BIN="$SELF" VOWC_KIND=self bash tests/decl/tests.sh >"$decl_self_log" 2>&1; then
    pass "decl/self-hosted"
else
    fail "decl/self-hosted" "$(tail -20 "$decl_self_log")"
fi
echo ""

# ─── Section 4j: Frontend diagnostic I/O failures (tests/diag-io/, #957) ──
#
# A stderr that cannot be written (full disk, closed pipe) must leave the
# frontend result unchanged for BrokenPipe and become a structured
# CompileFailed otherwise, identically in both compilers.

section_begin "Section 4j: Frontend diagnostic I/O failures"
diag_io_rust_log="$TMPDIR/diag-io-rust.log"
if VOWC_BIN="$RUST" VOWC_KIND=rust bash tests/diag-io/tests.sh >"$diag_io_rust_log" 2>&1; then
    pass "diag-io/rust"
else
    fail "diag-io/rust" "$(tail -20 "$diag_io_rust_log")"
fi
diag_io_self_log="$TMPDIR/diag-io-self.log"
if VOWC_BIN="$SELF" VOWC_KIND=self bash tests/diag-io/tests.sh >"$diag_io_self_log" 2>&1; then
    pass "diag-io/self-hosted"
else
    fail "diag-io/self-hosted" "$(tail -20 "$diag_io_self_log")"
fi
echo ""

# ─── Section 4k: Skill installs (tests/skill-install/, #358 and #361) ─
#
# Auto-install must leave an existing SKILL.md (including the old monolithic
# layout) untouched; an explicit `skill install --local` migrates it to the
# split layout. Both compilers stage the skill tree next to its target and
# rename it into place, so a failed install never leaves a SKILL.md without its
# support files and auto-install stays silent. Both compilers implement this
# independently.

section_begin "Section 4k: Skill installs"
skill_install_rust_log="$TMPDIR/skill-install-rust.log"
if VOWC_BIN="$RUST" VOWC_KIND=rust bash tests/skill-install/tests.sh >"$skill_install_rust_log" 2>&1; then
    pass "skill-install/rust"
else
    fail "skill-install/rust" "$(tail -20 "$skill_install_rust_log")"
fi
skill_install_self_log="$TMPDIR/skill-install-self.log"
if VOWC_BIN="$SELF" VOWC_KIND=self bash tests/skill-install/tests.sh >"$skill_install_self_log" 2>&1; then
    pass "skill-install/self-hosted"
else
    fail "skill-install/self-hosted" "$(tail -20 "$skill_install_self_log")"
fi
echo ""
