# Plan: fix checked float arithmetic opcodes crash the Cranelift verifier (#1218)

## 0. Branch state finding (read this first)

This branch already carries a complete, working implementation from a prior run (commits dated
2026-09-04, authored before this planning pass started):

- `5567f795` — `docs(plan): add implementation plan for issue #1218` (the superseded prior `PLAN.md`)
- `52886603` — `fix(types): reject checked arithmetic on float operands` (Rust checker)
- `717cd486` — `fix(checker): mirror checked-float rejection in self-hosted compiler`
- `3656255e` — `docs(grammar): document checked-float rejection, regenerate skill/help`

No PR exists for this branch (`gh pr list --head <branch>` is empty) and the issue is still open —
the prior run evidently stopped before the hand-off step (`git rm PLAN.md` + push + `gh pr create`),
not before the fix was written. `git log origin/main..HEAD` shows exactly these four commits; nothing
else is on the branch. Verified against the Rust checker on `origin/main`
(`vow-types/src/check.rs`): there is no `check_checked_numeric` there and no checked/float handling
in `compiler/checker.vow` either, so the bug is **not** independently fixed upstream — this work is
still needed, just already done here.

**Decision:** this plan does not redo the implementation. It supersedes the prior `PLAN.md` and
directs the implementation stage to **validate, rebase, and ship** the three existing commits, only
writing new code if validation surfaces a real gap. Re-implementing from scratch would duplicate
correct work and risks introducing a second, divergent fix.

The one real complication: `origin/main` has moved **87 commits** past this branch's fork point
(`940e2f91`, ~2026-09-03) in the month since. The branch has not been rebased. This must happen
before the PR can be opened.

## 1. Problem restated

`+!`, `-!`, `*!`, `/!`, `%!` on `f32`/`f64` operands are accepted by the type checker in both
compilers, but both `binop_opcode` lowerers (`vow-ir/src/lower/mod.rs` and `compiler/lower.vow`)
unconditionally map the five checked `BinOp`/`BINOP_*_CHK` variants to the *integer*
checked-arithmetic opcodes (`Opcode::CheckedAdd`/`IOP_CADD`, etc.), which the Cranelift backend
lowers to `sadd_overflow`/`ssub_overflow`/`smul_overflow` — integer-only Cranelift instructions.
Feeding them `f32`/`f64` SSA values crashes the Cranelift verifier and surfaces as an opaque
`CodegenFailed` ("Verifier errors") instead of a clean, diagnosable rejection. This is the same
regression class already fixed for the *unchecked* float operators in #1164; checked operators were
out of that issue's scope and still crash on `origin/main` today.

## 2. Decision already encoded in the existing commits (for the record)

The existing fix picks **option 1** from the issue: reject checked arithmetic on float operands at
type-check time with `UnsupportedFeature`, in both compilers, rather than inventing a checked-float
semantics (NaN/Inf-as-overflow). Rationale (unchanged from the superseded plan, still sound):

- "Checked" means "abort instead of wrapping on integer overflow." Floats have no wrapping semantics
  to check against — IEEE-754 arithmetic is already total over its domain (results saturate to `±Inf`
  or `NaN`), so there is no missing case to fill, only a new, invented convention this plan declines
  to add.
- A NaN/Inf-as-overflow semantics would need new float IR opcodes, dedicated Cranelift lowering in
  both compilers, C-emitter support, and a new ESBMC proof obligation for a condition that isn't what
  "checked" means anywhere else in the language — it fails CLAUDE.md's "does not make verification
  harder" and "reject anything that introduces a new type-system axis" bars for a feature nobody
  asked for.
- This improves on the #1164 precedent: unchecked `%` fails closed at codegen (`CodegenUnsupported`)
  because a real lowering is merely unimplemented; checked float arithmetic fails closed at
  type-check (`UnsupportedFeature`) because there is nothing to implement — earlier and cleaner,
  exactly as the issue suggests.
- No contract/ESBMC surface: nothing here touches `requires`/`ensures`; the fix *reduces*
  verification surface area (a malformed program never reaches IR/codegen/verify).

## 3. What was actually implemented (files already touched)

- `vow-types/src/check.rs` — the checked-arithmetic arm of `check_expr`'s `BinaryOp` match now calls
  a new private `check_checked_numeric(&mut self, lhs: Ty, rhs: Ty, op_span: Span) -> Ty`, which
  delegates to `check_same_numeric` and then, if the resolved type `.is_float()`, emits
  `ErrorCode::UnsupportedFeature` while still returning the resolved float type (not `Ty::Unit`), so
  the expression doesn't cascade into a spurious second `TypeMismatch` against an enclosing float
  return type. Unit tests added: float/float → one `UnsupportedFeature`, `f64`; int/int →
  unaffected; mismatched classes → existing single `TypeMismatch`, no double-report.
- `compiler/checker.vow` — the self-hosted `check_expr_inner`'s checked-arithmetic fallthrough
  mirrors the same logic using `ty_tag`/`CTY_F32`/`CTY_F64`/`EC_UNSUPPORTED_FEATURE`.
- `tests/error/checked_{add,sub,mul,div,rem}_float_unsupported.vow` — five new fixtures, one per
  operator, each asserting `// TEST: error-code UnsupportedFeature` / `// TEST: error-count 1`,
  mirroring the format of the pre-existing `tests/error/float_remainder_unsupported.vow`. These are
  picked up automatically by `scripts/full_test.sh`'s `run_promoted_error_tests` (globs
  `tests/error/*.vow` and diffs Rust-vs-self-hosted error JSON) — no registration needed anywhere
  else.
- `docs/spec/grammar.md` — the Checked Arithmetic section's #1218 placeholder replaced with the
  shipped behavior; `compiler/main.vow` and `vow/src/skill.rs` regenerated from it via
  `scripts/generate_help.py` (both are generated artifacts, not hand-edited).

Both lowerers' `binop_opcode` functions still contain the original integer-opcode mapping for the
checked-float case — that code is now unreachable (type-check rejects first) and is deliberately
left alone; see "Out of scope" below.

## 4. Validation slices (replaces TDD slices — the fix is already test-first; this is a rebase +
   verification checklist for the implementation stage)

1. **Rebase onto current `origin/main`.**
   `git fetch origin main`, then rebase the four commits (`5567f795` can simply be dropped/replaced
   since this new `PLAN.md` supersedes it — the implementation stage will `git rm PLAN.md` before
   opening the PR anyway, so don't fight to preserve that commit) onto `origin/main`. Expect the real
   conflict risk in two places:
   - `vow-types/src/check.rs`: `check_same_numeric` still exists on `origin/main` (confirmed, just at
     a different line number), so the new `check_checked_numeric` function and its call site should
     apply with at most line-offset noise, not semantic conflicts. Re-read the surrounding function
     after rebase to confirm nothing about `BinOp::*Checked` handling changed upstream in the
     interim.
   - `compiler/checker.vow`: confirm the `EXPR_BINOP` arithmetic fallthrough this patch touches still
     has the same shape; `grep` for `ty_is_numeric_or_lit_int`, `CTY_F32`, `CTY_F64` on the rebased
     tree before assuming the hunk applies cleanly.
   - `compiler/main.vow` and `vow/src/skill.rs` are **generated**. If either produces a rebase
     conflict, do not hand-merge: take upstream's version, then regenerate via
     `uv run python scripts/generate_help.py` and let the generator reproduce the #1218 wording from
     the (possibly also-conflicting, hand-resolved) `docs/spec/grammar.md`.
   - After resolving, run `git status` to confirm nothing is left unstaged (per this repo's git
     safety rules) before proceeding.

2. **Targeted Rust test.**
   `cargo test -p vow-types checked_numeric` — confirms
   `checked_numeric_rejects_float_operands_without_double_reporting` and the existing
   `checked_add_returns_integer_type` (int/int path, unaffected) both still pass post-rebase.

3. **Fixture parity check.**
   Run the five new fixtures plus `float_remainder_unsupported.vow` through both compilers directly
   (`cargo run -p vow -- build --no-verify tests/error/checked_add_float_unsupported.vow` and the
   self-hosted equivalent via `build/vowc`) to sanity-check the diagnostic shape before relying on
   the full `scripts/full_test.sh` sweep to confirm it. This is a fast, cheap check — don't skip it
   in favor of only running the expensive full gate.

4. **Regenerate and diff-check help/skill.**
   `uv run python scripts/generate_help.py`, then `git diff --stat` — expect zero diff if the rebase
   already left `compiler/main.vow`/`vow/src/skill.rs` in the regenerated state; a non-empty diff
   here means the rebase left stale generated output and must be committed as part of finishing this
   branch. Also run `scripts/check_help_coverage.py` to confirm no `grammar.md`/`--help` drift.

5. **Full quality gate, run as separate commands (per this repo's gate discipline), each with an
   explicit time bound, polled rather than backgrounded-and-forgotten:**
   - `cargo build --all` (capped parallelism per the run's memory budget)
   - `cargo test --all`
   - `cargo clippy --all -- -D warnings`
   - `cargo fmt --all -- --check`
   - `scripts/bootstrap.sh --skip-cargo` (~5 min; rebuilds `build/vowc` from the rebased
     `compiler/*.vow`)
   - `scripts/full_test.sh` (~40 min; this is what actually exercises
     `run_promoted_error_tests` against the five new fixtures end-to-end)
   Use `VOW_CACHE_DIR=$(mktemp -d)` for any manual `vowc build` invocations in steps 3/5 — the vow
   compile cache ignores compiler rebuilds at the same source revision and will otherwise serve stale
   objects.

6. **Hand off.**
   `git rm PLAN.md`, commit, push, `gh pr create --base main --head <branch> --title "fix(types): reject checked arithmetic on float operands" --body ...` (title under the ~92-char budget, lower-case
   subject, no trailing period — the `52886603` commit subject already satisfies this and is a
   reasonable PR title as-is). Squash-merge is enforced repo-side; do not plan for a merge commit.

## 5. Verification surface

None. This is a pure type-checker change: it rejects a program before IR lowering, codegen, or
ESBMC ever see it. No new `requires`/`ensures`, no C-emitter change, no new proof obligation. The
five `tests/error/*.vow` fixtures are the complete verification surface for this fix, and they
already exist. No new `tests/run/` or `examples/` fixtures are needed — there is no valid program for
checked-float arithmetic to execute.

## 6. Risk areas

- **Rebase conflicts**, concentrated in `vow-types/src/check.rs` and `compiler/checker.vow` as
  described in slice 1 — both files have almost certainly changed elsewhere in the 87 intervening
  upstream commits, even if the specific functions this fix touches are stable.
- **Generated-file conflicts** in `compiler/main.vow` / `vow/src/skill.rs` — resolve by regenerating,
  never by hand-merging, per slice 1.
- **codecov/patch gate (95% threshold)** — the new `check_checked_numeric` function and its Rust unit
  test are the only non-`.vow` lines this change adds; the unit test already exercises all three
  branches (float/float, int/int, mismatched), so patch coverage should clear the gate without
  further test-writing. Recheck after rebase in case line attribution shifted.
- **`cargo clippy --all -- -D warnings`** — matches CI's own invocation (no `--all-targets`); don't
  chase lints in test modules that CI doesn't check.
- **Binary fixed point / `parse → print → parse` idempotency** — unaffected. This change adds no new
  grammar, no new AST node, and no new printer case; `BinOp::*Checked` parsing and printing are
  unchanged.

## 7. Out of scope (deliberately not bundled into this PR)

- Removing the now-unreachable checked-float branch from `binop_opcode` in
  `vow-ir/src/lower/mod.rs`/`compiler/lower.vow`. It is dead code after the type-check rejection, but
  deleting it is a separate, non-bug-fix cleanup; leaving a defensive integer-opcode mapping in place
  (never reached in practice, since the type checker now rejects first) is consistent with this
  repo's own prior precedent for unchecked `%`'s `CodegenUnsupported` fallback. Do not combine this
  cleanup with the bug fix commit.
- Implementing an actual checked-float semantics (NaN/Inf-as-overflow or otherwise). Rejected for the
  reasons in §2; not a follow-up item, a closed question.
- `RemF*` codegen for unchecked `%` (tracked by #1164's own follow-up, not this issue).
- Any refactor of `check_same_numeric`, `check_expr`'s `BinaryOp` arm, or the self-hosted
  `check_expr_inner` beyond the minimal addition described in §3.
- Rebasing eagerly-and-often throughout the implementation stage — rebase once, at the start (slice
  1), not repeatedly chasing a moving `origin/main` mid-task.
