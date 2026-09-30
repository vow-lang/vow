# Plan: #1255 — reject overflowing f64 literals instead of emitting uncompilable C

## 0. Status (re-planning pass, 2026-09-30)

This branch already carries a full implementation of slices 1-4 below, committed
but **never pushed and with no PR opened**. The planning stage was re-invoked on
this workspace after that work landed; rather than re-plan from scratch, this
section records what exists so the implementation stage verifies and ships
instead of redoing it.

**Already done, verified against `git show --stat`:**

- Slice 1 (Rust lexer): `86b6c1fe fix(syntax): reject f64 literals that overflow
  to infinity` — `vow-syntax/src/lexer.rs`, `is_infinite()` check + inline test.
- Slice 2 (self-hosted lexer): `2baffa72 fix(compiler): reject f64 literals that
  overflow to infinity` — `compiler/token.vow`, `compiler/lexer.vow`,
  `compiler/parser.vow`, plus `compiler/tests/test_lexer_float_overflow.vow`.
- Slice 3 (parity fixture): `4d67d089 test(error): add cross-compiler parity
  fixture for f64 literal overflow` — `tests/error/f64_literal_overflow.vow`.
- Slice 4 (docs): `1ed829d6 docs(spec): document f64 literal overflow as an
  InvalidCharacter error` — `docs/spec/errors.md`, `docs/spec/grammar.md`, and
  the regenerated `--help`/skill output (`vow/src/skill.rs`, `compiler/main.vow`,
  `skills/vow/reference/{errors,grammar}.md`) via `generate_help.py`, matching
  the plan's slice-4 instructions exactly.

**Do not re-implement any of the above.** The remaining work is verification
and shipping, not new code:

1. Run the quality gate as separate commands (per user CLAUDE.md — never
   `&&`-chained), each with a generous timeout, polled rather than assumed:
   - `cargo test -p vow-syntax`
   - `cargo clippy --all -- -D warnings`
   - `scripts/bootstrap.sh --skip-cargo` (rebuilds `build/vowc`; ~5 min)
   - the self-hosted `compiler/tests/test_lexer_float_overflow.vow` test, via
     whichever runner the sibling `test_lexer_dot_and_suffix.vow` uses
   - `scripts/full_test.sh` in the background, polled (~40 min) — this is what
     actually exercises the new `tests/error/f64_literal_overflow.vow` fixture
     through `parity.py` against both compilers
   - `uv run python scripts/check_help_coverage.py` — confirm the slice-4
     regen left no staleness
2. If the bootstrap triple (`scripts/concat_vow.sh` + three-stage build +
   `sha256sum` comparison, see Risk Areas below) hasn't been run against these
   exact commits, run it now — the self-hosted compiler's own source changed
   (`token.vow`/`lexer.vow`/`parser.vow`), so this is required, not optional,
   before merging.
3. File the fast-follow issue from the "Out of scope" section (defense-in-depth
   `is_finite()` guards in both C emitters' `ConstF64` render arms). Confirmed
   via `gh issue list --search` that no such issue exists yet.
4. If everything above is green: `git rm PLAN.md`, commit, push the branch, and
   open the PR with `gh pr create` (explicit `--base`/`--head`/`--title`/`--body`
   flags, no `--web`). If anything is red, fix it as a small follow-up commit
   on this same branch (still within scope of slices 1-4 — e.g. a clippy nit),
   not a new plan.

The rest of this document (sections 1-6) is the original plan, unchanged, and
is kept for the record of what was decided and why. Slices 1-4 in section 3
describe work that is now already committed, not work still to do.

## 1. Problem restated

A source float literal is lexed as `digits '.' digits` with no exponent notation
in the grammar, so the only way an `f64` literal reaches a non-finite value is
decimal-magnitude overflow (e.g. 400+ digits before the dot). Today both
lexers happily convert such a literal to `f64::INFINITY` and let it flow all
the way to the C emitters used by the ESBMC verification pipeline
(`vow-verify/src/c_emitter.rs` and `compiler/c_emitter.vow`), which render the
constant via `f64::to_string()`/`__vow_format_f64_bits` as the text `inf` —
not a valid C double-literal token — so the generated verification model
fails to compile with no diagnostic pointing back at the offending literal.
NaN is not reachable through this path today (the grammar has no textual
spelling for it and neither compiler constant-folds float arithmetic), so the
fix is scoped to rejecting magnitude overflow at lex time, in both compilers,
the same way the integer-literal path already rejects magnitudes that don't
fit `u128`.

## 2. Files to touch

**Rust compiler (stage 0):**
- `vow-syntax/src/lexer.rs` — `lex_number`'s float branch (~line 292-307):
  after `text.parse::<f64>()` succeeds, reject `value.is_infinite()` with a
  `LexError` before constructing `TokenKind::LitFloat`. Reuses
  `ErrorCode::InvalidCharacter` (the same code the adjacent u128-digit-overflow
  check on the integer path already uses, ~line 310-314) — no new
  `ErrorCode` variant. Message: `"float literal out of range"` (omit the raw
  digits — the span already covers them, and a 300+ digit literal in a
  diagnostic string is noise). Use this exact string on both compilers (see
  below) — not because any harness enforces byte-identical messages
  (`scripts/parity.py`'s `compare_error` only diffs the `error_code` field
  between compilers, never `message`; confirmed by reading it), but because
  there's no reason for them to drift when nothing forces the choice either
  way.
- `vow-syntax/src/lexer.rs` inline `#[cfg(test)] mod tests` — new unit test
  mirroring `lex_integer_rejects_magnitude_above_u128` (~line 512-518):
  `lex_float_rejects_magnitude_above_f64_max`.

**Self-hosted compiler:**
- `compiler/token.vow` — add `fn tok_invalid_float() -> i64 { 83 }` (next free
  tag after `tok_lit_float() = 82`).
- `compiler/lexer.vow` — the float branch (~line 281-289): after
  `parse_f64_bits(text)`, compare the returned bits against the f64
  +Infinity bit pattern (`0x7FF0000000000000` — the literal grammar has no
  unary minus, so only the positive-infinity pattern is reachable) and, on
  match, push a `tok_invalid_float()` token carrying `EC_INVALID_CHARACTER()`
  and the message `"float literal out of range"` (same string as the Rust
  side), exactly mirroring the `wide_overflow` branch's `tok_invalid_int()` /
  `"integer literal out of range"` construction immediately below it.
- `compiler/parser.vow` — add an `else if tag == tok_invalid_float()` arm next
  to the existing `tok_invalid_int()` arm (~line 875): `push_error_code(p,
  t.int_val, t.str_val)` then advance and return a dummy zero-bits
  `EXPR_LIT_FLOAT`, mirroring the int arm exactly.
- `compiler/tests/test_lexer_float_literal.vow` (or a new adjacent
  `test_lexer_float_overflow.vow`, matching the existing
  `test_lexer_dot_and_suffix.vow` convention) — asserts the overflowing
  literal produces `tok_invalid_float()` with `EC_INVALID_CHARACTER()`.

**Docs (required by CLAUDE.md's "any change to syntax/semantics" rule, since
this changes a previously-silently-accepted literal into a compile error):**
- `docs/spec/errors.md` — extend the existing `### InvalidCharacter` entry
  with a second trigger example for an overflowing float literal, alongside
  the existing invalid-character example.
- `docs/spec/grammar.md` — extend `### Float Literals` (~line 228-234) with an
  "Out-of-range literals" paragraph analogous to the integer section's
  (~line 199-203), showing that a literal whose magnitude exceeds
  `f64::MAX` is an `InvalidCharacter` error at lex time.

**Test fixture (cross-compiler parity, exercised automatically):**
- `tests/error/f64_literal_overflow.vow` — new fixture with `// TEST:
  error-code InvalidCharacter` and `// TEST: stderr "..."` header comments,
  following the exact convention of every other file in `tests/error/`
  (e.g. `i128_literal_out_of_range.vow`). `scripts/full_test.sh`'s error-fixture
  loop (~line 336-353, `for fixture_path in ... tests/error/*.vow`) picks up
  any file dropped in that directory automatically and runs it through both
  `$RUST build --no-verify` and `run_self build --no-verify`, comparing
  their JSON diagnostics via `scripts/parity.py` (`compare_error` /
  `run_parity error`) — this is what proves both lexers agree, not a new
  harness. The literal itself: a decimal magnitude of ~310+ digits before the
  `.0`, comfortably past `f64::MAX` (~1.7976931348623157e308).

No changes needed to `vow-codegen`/`vow-clif-shim` (Cranelift's `f64const`
takes the bit pattern directly and has no C-literal-token problem), and no
changes to `vow-ir`/`lower.vow` (both `ConstF64` construction sites just
carry through whatever bits the lexer already validated).

## 3. TDD slices

1. **Rust lexer rejects overflowing float literals.**
   Red: add `lex_float_rejects_magnitude_above_f64_max` to
   `vow-syntax/src/lexer.rs`'s test module, asserting
   `Lexer::new("<310 nines>.0").tokenize()` returns `Err` with
   `code == ErrorCode::InvalidCharacter` and a message containing
   `"out of range"`. Fails today (produces `Ok(LitFloat(f64::INFINITY))`).
   Green: add the `is_infinite()` check in `lex_number`'s float branch.
   `cargo test -p vow-syntax`.

2. **Self-hosted lexer rejects overflowing float literals.**
   Red: add a test to `compiler/tests/test_lexer_float_literal.vow` (or a new
   file) that lexes the same overflowing literal and asserts the resulting
   token's tag is `tok_invalid_float()` and `int_val == EC_INVALID_CHARACTER()`.
   Fails today (produces a `tok_lit_float()` token with Infinity bits).
   Green: add `tok_invalid_float()` to `token.vow`, the bit-pattern check in
   `lexer.vow`, and the parser arm in `parser.vow`. Run via
   `build/vowc build --no-verify compiler/main.vow -o "$TMPDIR/vow_main"` then
   the test binary, or through whatever harness
   `compiler/tests/test_lexer_dot_and_suffix.vow`'s sibling int-overflow test
   already uses (check `scripts/full_test.sh` / a `run_compiler_tests`
   section for how `compiler/tests/*.vow` are invoked, and reuse it — do not
   invent a new runner).

3. **Cross-compiler parity fixture closes the issue.**
   Red: add `tests/error/f64_literal_overflow.vow` with the `// TEST:
   error-code InvalidCharacter` / `// TEST: stderr "out of range"` header.
   Run `scripts/full_test.sh`'s error section (or invoke `parity.py` directly
   against both binaries) — fails today because both compilers currently
   accept the file and go on to try to build/verify it.
   Green: slices 1 and 2 together make this fixture pass; this slice is the
   integration proof, not new production code.

4. **Docs.**
   Update `docs/spec/errors.md` and `docs/spec/grammar.md` as described above.
   Then, per `CLAUDE.md`'s "any change to syntax/semantics must update the
   spec" rule, run:
   ```
   uv run python scripts/generate_help.py   # regenerate --help / embedded skill
   cargo build --release -p vow             # rebuild Rust compiler
   scripts/bootstrap.sh --skip-cargo        # rebuild build/vowc
   uv run python scripts/check_help_coverage.py   # staleness check
   ```
   This change adds no new keyword/builtin/CLI surface, so the regen should
   produce no diff and the coverage check should be a no-op — but both are
   cheap and CLAUDE.md requires running them, not assuming the outcome. If
   `generate_help.py` does produce a diff, that diff is part of the PR.

## 4. Verification surface

This change is entirely in the two lexers; it does not touch contracts,
`requires`/`ensures`, or anything ESBMC proves about *user* Vow programs, and
it does not modify either C emitter. No new benchmark or `tests/verify/`
fixture is needed — a rejected literal never reaches the verifier.

`tests/run/` needs no new fixture either: there is no valid *runtime*
behavior to exercise here (the literal is rejected before codegen), so this
lives entirely in `tests/error/`.

## 5. Risk areas

- **Binary fixed point.** `compiler/token.vow`'s new tag value (83) and the
  new `tok_invalid_float()` branch in `lexer.vow`/`parser.vow` change the
  self-hosted compiler's own source, so it must be rebuilt through the full
  bootstrap triple (`scripts/concat_vow.sh` + three-stage build +
  `sha256sum` comparison) before merging, per `CLAUDE.md`'s bootstrap-triple
  test. A bad tag collision or an off-by-one in the parser arm would surface
  there as a stage mismatch, not as a test failure in isolation.
- **Error-code parity, not just presence.** The existing integer-overflow
  path already has a latent mismatch between the two compilers (Rust's
  u128-digit-overflow check uses `ErrorCode::InvalidCharacter`; the
  self-hosted `wide_overflow` check uses `EC_UNEXPECTED_TOKEN()`, i.e.
  `ErrorCode::UnexpectedToken`) — that mismatch is pre-existing and
  out of scope for this issue (do not touch it). This plan's new float path
  must not repeat that mistake: use `EC_INVALID_CHARACTER()` on the
  self-hosted side specifically so it matches `ErrorCode::InvalidCharacter`
  on the Rust side bit-for-bit, since `scripts/parity.py`'s error-code
  comparison is exact-match, not fuzzy.
- **`cargo clippy --all -- -D warnings`.** The new Rust `is_infinite()` check
  is a simple `if`, low risk, but double-check the new inline test doesn't
  trip `clippy::approx_constant` or similar lints the way the neighboring
  `3.14` test already has to suppress.
- **`parse → print → parse` idempotency.** Not affected — a rejected literal
  never produces an AST node the canonical printer would round-trip.
- **`BTreeMap` vs `HashMap` / stack-slot layout in `vow-clif-shim`.** Not
  touched; this change never reaches codegen.
- **Fixture literal choice.** Pick a magnitude comfortably past
  `f64::MAX` (~1.7976931348623157e308) with enough digits that no plausible
  future widening of `f64` parsing changes the fixture's outcome — 310+
  digits before the `.0` is a safe margin, not a knife's-edge value.

## 6. Out of scope

- **Hex-float / exponent-form C emission** (the issue's alternative fix
  path) — a materially bigger change to both C emitters that would let
  finite-but-precision-sensitive floats round-trip more faithfully through
  C. Rejecting overflow at lex time fully closes this issue's reported bug
  (uncompilable C from a literal) without that scope.
- **Defense-in-depth `is_finite()`/bit-pattern guards in
  `vow-verify/src/c_emitter.rs` and `compiler/c_emitter.vow`'s `ConstF64`/
  `ConstF32` render arms.** Worth doing, but deliberately not bundled here:
  the path is confirmed unreachable today (no constant folding of float
  arithmetic exists in either compiler, and the lexer fix in this PR is the
  only source of `ConstF64` values), so it guards a hypothetical future
  regression rather than closing #1255 itself. It also isn't a small
  addition — the self-hosted half needs whatever `compiler/c_emitter.vow`
  uses for an "unreachable" abort (unverified whether one already exists
  there), and a unit test that hand-builds an IR module bypassing the normal
  lowering pipeline is real test-infrastructure work, not a one-line check.
  File a fast-follow issue instead of stretching this PR to cover a case
  nothing can currently trigger.
- **Fixing the pre-existing integer-literal error-code mismatch**
  (`InvalidCharacter` vs `UnexpectedToken` for >u128 digit sequences,
  described in Risk Areas) — real, but unrelated to floats and not
  introduced or worsened by this change.
- **`f32` literal overflow** — there is no `f32` literal syntax in the
  grammar (`f32` values only arise from casts/computation); `ConstF32` is
  never constructed by either lowering path from a literal today, confirmed
  by grepping both `lower.vow` and the Rust `vow-ir` lowering for
  `ConstF32`/`IOP_CONST_F32()` construction sites. No literal-rejection logic
  is needed for it.
- **NaN rejection at lex time** — not reachable from the literal grammar
  (no textual spelling, no constant folding of float arithmetic in either
  compiler today), so there is nothing to reject.
- **Constant folding of float expressions** — out of scope; not part of this
  issue and would be its own feature with its own verification-impact
  review per `CLAUDE.md`'s language-design criteria.
- **Refactoring `lex_number` or `wide_literal_push_digit`** beyond the
  minimal insertion needed for the new check — no unrelated cleanup bundled
  into this fix.
