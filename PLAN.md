# Plan: issue #1160 — reconcile grammar.md length-signedness rationale with ADR 0003

## 1. Problem restated

`docs/spec/grammar.md:142-144` justifies "no `isize`/`usize`" **and** "`Vec::len()`
returns `i64`" with a single binary-fixed-point argument, citing ADR 0001. ADR 0003
(accepted 2026-08-31, already merged via #1158) shows those are two independent claims:
the fixed-point argument applies only to *pointer-width* types (`isize`/`usize`), not to
the *signedness* of a fixed-width length. ADR 0003 Decision 2 reverses `Vec::len() -> i64`
to `u64` on exactly that ground, and ADR 0001's own `## Amendments` section (lines
141-149) already records the reversal. `grammar.md` still reproduces the pre-amendment
reasoning and cites only ADR 0001, without pointing at ADR 0003 or the pending reversal.
The factual claim ("`Vec::len()` returns `i64` today") is still true and must be kept —
only the rationale sentence and the citation need to change.

## 2. Files to touch

This is a documentation-only reconciliation. No language semantics, builtins, CLI flags,
or contracts change, so no `crates/` or `compiler/` edits are needed or in scope.

- `docs/spec/grammar.md` — lines 142-144 (the paragraph under the primitive-types table).
  This is the only edit.

Not touched, and confirmed out of scope by grep:
- `docs/spec/errors.md:49-50` cites ADR 0001 only for the "no `isize`/`usize`" claim
  itself (the `InvalidIntSuffix` diagnostic), which ADR 0003 explicitly keeps intact
  ("ADR 0001's exclusion is KEPT"). No conflation there — leave as is.
- `docs/spec/grammar.md:210,215` (integer-suffix section) makes the same "no
  `usize`/`isize` suffix" claim without the fixed-point rationale or an ADR citation —
  nothing to reconcile.
- `docs/adr/0001-numeric-tower-narrow-ints.md` and `docs/adr/0003-unsigned-size-types.md`
  already say the right thing (ADR 0001's amendment already records the reversal). Not
  touched.
- `scripts/check_help_coverage.py`, `scripts/generate_help.py`, `scripts/ci_docs_only.py`
  — grepped for `isize`/`usize`/`Vec::len`/`ADR`; none reference this prose, so the
  generated `--help`/skill text is unaffected and `generate_help.py` does not need to be
  re-run for this change.

## 3. Edit content (no TDD slices — this is prose, not code)

This issue has no behavior to drive with a red-green test loop: it is a rationale
correction in a spec paragraph, not a code change. The "TDD slice" for a docs fix is the
single edit plus the doc-consistency checks in section 4. One slice:

**Slice 1 — rewrite `docs/spec/grammar.md:142-144`.**

Replace:

```
There is no `isize`/`usize`. Vow targets 64-bit only; `Vec::len()` returns `i64`,
indices are `i64`. This is deliberate — it preserves binary fixed point
reproducibility across compilations. See [ADR 0001](../adr/0001-numeric-tower-narrow-ints.md).
```

With (exact wording to be finalized during implementation, but must satisfy the four
constraints below):

```
There is no `isize`/`usize`. Vow targets 64-bit only, because pointer-width types would
make the same source produce different binaries on different hosts, breaking the binary
fixed point that `scripts/bootstrap.sh` verifies — see
[ADR 0001](../adr/0001-numeric-tower-narrow-ints.md). This exclusion is orthogonal to
signedness: `Vec::len()` currently returns `i64` and indices are currently `i64`, but
[ADR 0003](../adr/0003-unsigned-size-types.md) reverses this to `u64` for lengths,
indices, and capacities as part of epic #1104's lengths migration. The binary-fixed-point
argument justifies excluding pointer-width types; it does not justify keeping a
fixed-width length signed.
```

Constraints the replacement text must satisfy (verify by re-reading the merged
paragraph, not by an automated test):
1. Keeps the still-true factual claims: no `isize`/`usize`; `Vec::len()` returns `i64`
   today; indices are `i64` today.
2. Attributes the binary-fixed-point/reproducibility argument *only* to the
   `isize`/`usize` (pointer-width) exclusion — never states or implies it justifies the
   `i64` signedness of `Vec::len()`.
3. Cites both ADR 0001 (for the pointer-width exclusion, still standing) and ADR 0003
   (for the pending signedness reversal), each attached to the claim it actually
   supports.
4. Does not overclaim the reversal as already-landed — `Vec::len()` returns `i64` until
   epic #1104's Phase-B lengths migration lands; phrase the ADR 0003 reference as
   forward-looking ("reverses this to `u64`", "will become"), matching how ADR 0001's own
   `## Amendments` section already phrases it.

## 4. Verification surface

No contracts, codegen, or C model are touched, so ESBMC has nothing new to prove and no
`tests/run/` or `examples/` fixtures are needed. Verification for this change is
documentation consistency, done manually (no existing script covers cross-references
between `grammar.md` prose and `docs/adr/*.md`):

- Re-read the edited paragraph against `docs/adr/0003-unsigned-size-types.md` Decisions
  1-3 and against ADR 0001's `## Amendments` entry for 2026-08-31, and confirm no
  remaining sentence attributes the fixed-point argument to signedness.
- Confirm both ADR links resolve (`../adr/0001-numeric-tower-narrow-ints.md` and
  `../adr/0003-unsigned-size-types.md` — both already used as link targets elsewhere in
  `grammar.md`/ADR 0001, so the relative path is already proven correct from this
  location).
- Run `scripts/check_help_coverage.py` (as `full_test.sh` does) to confirm the edit does
  not desync `grammar.md` from generated `--help` output — expected to pass untouched
  since this prose isn't part of the feature table the script cross-references, but worth
  the one confirming run given the "canonical source of truth" rule in `CLAUDE.md`.
- No need to run `scripts/generate_help.py` or rebuild `build/vowc` — this paragraph
  is not part of the generated skill/help text (confirmed by grep in section 2).

## 5. Risk areas

None of the usual Vow risk categories apply to this change:
- **Binary fixed point** (`compiler/` codegen ordering, `BTreeMap`/`HashMap`,
  `vow-clif-shim` stack slots) — untouched; no code changes at all.
- **`parse → print → parse` idempotency** — untouched; no grammar/parser changes.
- **`cargo clippy --all -- -D warnings`** — untouched; no Rust code changes.
- **`commitlint`** — the eventual PR title must still satisfy Conventional Commits
  (e.g. `docs(spec): reconcile grammar.md length rationale with ADR 0003`), lower-case
  subject, no trailing period, ≤100 chars including the `(#N)` suffix.

The only real risk is under- or over-correcting the prose: stating the reversal as
already-done (factually wrong until epic #1104 lands) or leaving residual wording that
still implies the fixed-point argument covers signedness (reproduces the exact bug this
issue exists to fix). Section 3's four constraints exist to guard against both.

## 6. Out of scope

- Editing `docs/spec/errors.md` — its ADR 0001 citation is for the still-correct
  `isize`/`usize` exclusion, not the signedness question ADR 0003 addresses. Not part of
  this conflation.
- Any part of epic #1104's actual `Vec::len() -> u64` migration (type checker, codegen,
  contract updates, corpus sweep). That is tracked separately and this issue is
  explicitly scoped to the *rationale text*, not the migration itself.
- Rewording or reformatting any other paragraph in `grammar.md` (e.g. the adjacent
  "128-bit implementation status" or "Struct field layout" notes) — untouched, no
  reported conflation there.
- Editing ADR 0001 or ADR 0003 themselves — both already say the correct thing; this
  issue is about `grammar.md` catching up to them, not about revising the ADRs.
- Any `scripts/generate_help.py` regeneration or `build/vowc` rebuild — confirmed
  unnecessary in section 2/4 since this prose isn't part of the generated `--help`/skill
  surface.
