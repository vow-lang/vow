# Plan: fix checked float arithmetic opcodes crash the Cranelift verifier (#1218)

## 1. Problem restated

`+!`, `-!`, `*!`, `/!`, `%!` on `f32`/`f64` operands are accepted by the type checker in both
compilers but both `binop_opcode` lowerers (`vow-ir/src/lower/mod.rs:4713` and
`compiler/lower.vow:1283`) unconditionally map the five checked `BinOp`/`BINOP_*_CHK` variants to
the *integer* checked-arithmetic opcodes (`Opcode::CheckedAdd`/`IOP_CADD`, etc.), which the
Cranelift backend lowers to `sadd_overflow`/`ssub_overflow`/`smul_overflow` — integer-only
Cranelift instructions. Feeding them `f32`/`f64` SSA values crashes the Cranelift verifier and
surfaces as an opaque `CodegenFailed` ("Verifier errors") instead of a clean, diagnosable
rejection. This is a regression class already fixed for the *unchecked* float operators in #1164;
checked operators were out of that issue's scope and still crash.

## 2. Decision: reject checked float arithmetic at type-check time

Per the issue's suggested options, this plan picks **option 1**: checked arithmetic operators are
rejected at type-check time with a clear `UnsupportedFeature` diagnostic when either operand is
`f32`/`f64`. Rationale, weighed against CLAUDE.md's three-criteria bar and the "reject anything
that introduces a new type-system axis" rule:

- **Floats have no integer-overflow condition to check.** "Checked" arithmetic's entire meaning is
  "abort instead of wrapping on integer overflow." There is no wrapping semantics for float
  arithmetic to compare against, so a "checked float add" has no principled meaning to define —
  unlike unchecked `%`, which *does* have a well-defined meaning (IEEE remainder) and is only
  blocked by a missing backend lowering.
- **Option 2 (NaN/Inf-as-overflow semantics) invents new semantics, not a backend gap.** It would
  require new float IR opcodes, dedicated Cranelift lowering in *both* compilers, C-emitter
  support, and an ESBMC proof obligation for a condition (`ArithOverflowReachable`-style NaN/Inf
  reachability) that isn't what "checked" means anywhere else in the language. That fails
  CLAUDE.md's "does not make verification harder" and "reject anything that introduces a new
  type-system axis" criteria for a feature nobody has asked for.
  For the record: IEEE-754 arithmetic on `f32`/`f64` cannot overflow into a trap in the way integer
  overflow does — the operations are already fully closed over their domain (results saturate to
  `±Inf` or `NaN`), so there is no missing case to make total, only a new, invented "abort on
  Inf/NaN" convention this plan declines to add.
- **Matches and improves on the #1164 precedent.** Unchecked `%` fails closed at codegen
  (`CodegenUnsupported`) because a real lowering is merely unimplemented. Checked float arithmetic
  fails closed at type-check (`UnsupportedFeature`) because there is nothing to implement — this is
  earlier and cleaner, exactly as the issue suggests ("ideally surfaced earlier, at type-check,
  since there's no legitimate lowering to fall back to").
- **No contract/ESBMC surface.** Nothing here touches `requires`/`ensures`, contract-authoring
  rules do not apply, and the fix reduces verification surface area rather than growing it (a
  malformed program is rejected before IR/codegen/verify ever run).

## 3. Files to touch

### Rust compiler (`vow-types/`)
- `vow-types/src/check.rs` — the checked-arithmetic arm of the `BinaryOp` match in `check_expr`
  (currently ~line 1893-1897: `BinOp::AddChecked | BinOp::SubChecked | BinOp::MulChecked |
  BinOp::DivChecked | BinOp::RemChecked => self.check_same_numeric(lhs_ty, rhs_ty, expr.span)`,
  already a separate arm from the unchecked operators at ~line 1892). Change what this arm calls:
  add a small private helper next to `check_same_numeric` (~line 3095), e.g.
  `check_checked_numeric(&mut self, lhs: Ty, rhs: Ty, op_span: Span) -> Ty`, that calls
  `check_same_numeric` and then, if the result `.is_float()` (`vow-types/src/types.rs:66`), emits
  `ErrorCode::UnsupportedFeature` **and returns the resolved float type unchanged** (not
  `Ty::Unit`). The expression is well-typed (`f64 +! f64 : f64`); only the *operator* is
  unsupported, so the type should not collapse to `Unit` — doing so would make the checked-op
  expression's type disagree with, say, a declared `-> f64` return type and fire a second,
  spurious `TypeMismatch` from the body-vs-return-type check, breaking the `error-count 1`
  fixtures in §3. This mirrors the codebase's existing anti-cascade idiom (e.g. the
  undefined-function path returns `Ty::Never` specifically "so a failed call does not cascade into
  its consumers" — `vow-types/src/check.rs:~2172`). This keeps `check_same_numeric` itself
  unchanged (still correct and shared with the unchecked path) and gives the self-hosted mirror an
  equally narrow seam to copy. Only emit the new error when `check_same_numeric` succeeded on a
  float type; a prior `TypeMismatch` (mismatched operand types, where `check_same_numeric` already
  returns `Ty::Unit`) must not double-report — `Ty::Unit.is_float() == false`, so this falls out
  for free as long as the float check runs strictly after `check_same_numeric`'s result.

### Self-hosted compiler (`compiler/`)
- `compiler/checker.vow` — the arithmetic fallthrough in `check_expr_inner` (~line 2388-2408:
  the block starting `if ty_is_numeric_or_lit_int(e.ts, lhs_tid) && ty_is_numeric_or_lit_int(...)
  ...` down through `return CTY_I64();` that currently handles *all* five unchecked and five
  checked arithmetic ops identically). Mirror the Rust split: after the existing "operands have
  different types" and lit-int-coercion logic resolves a concrete numeric type, if `op` is one of
  `BINOP_ADD_CHK()/BINOP_SUB_CHK()/BINOP_MUL_CHK()/BINOP_DIV_CHK()/BINOP_REM_CHK()`
  (`compiler/ast.vow:40-44`) and the resolved type's tag is `CTY_F32()`/`CTY_F64()`
  (`compiler/types.vow:13-14`), call `env_emit_error_code(e, EC_UNSUPPORTED_FEATURE(), <msg>,
  expr_span(a, eid))` (precedent: `compiler/checker.vow:2517`, `compiler/diag.vow:23`) and
  **return the resolved float type id unchanged** (`lhs_tid`, the same value the existing success
  path returns two lines later) — mirroring the Rust side's decision above: the expression is
  well-typed, only the operator is unsupported, and returning anything else (e.g. `CTY_I64()`,
  which this function's *other* fallback paths use) risks a second, spurious diagnostic from a
  declared-return-type check downstream and would break the `error-count 1` fixtures. Both
  compilers must return the *same kind* of "keep going with this type" value here, not just "some
  error-shaped value", since a divergence would only surface as unrelated parity noise in a
  different, unrelated multi-error fixture. Keep the inline five-way `op ==` check consistent with
  the existing style in this file (`checker.vow:854-859`, `lower.vow:1748-1749`) rather than
  introducing a new named helper used only once.
- No change needed to `compiler/lower.vow::binop_opcode` or `vow-ir/src/lower/mod.rs::binop_opcode`
  themselves — with the type checker rejecting the program first, the lowerer's existing (buggy)
  mapping becomes dead code for float operands and is never reached. Leave it as-is; do not
  "clean up" the unreachable branch as part of this fix (see §6 Out of scope).

### Tests
- `tests/error/checked_add_float_unsupported.vow` — `a +! b` on `f64`, mirroring
  `tests/error/float_remainder_unsupported.vow`'s shape and
  `tests/error/extern_call_unsupported.vow`'s `error-code UnsupportedFeature` directive.
- `tests/error/checked_sub_float_unsupported.vow`,
  `tests/error/checked_mul_float_unsupported.vow`,
  `tests/error/checked_div_float_unsupported.vow`,
  `tests/error/checked_rem_float_unsupported.vow` — same shape, one file per operator (matches
  this directory's one-scenario-per-file convention; `error-count 1` per file). Use `f32` for one
  of the five and `f64` for the rest, so both float widths are covered across the set without
  bloating to 10 files.

### Docs
- `docs/spec/grammar.md` — replace the placeholder paragraph at the end of the "Wrapping
  Arithmetic" section (lines ~279-286: "Checked float operators do not yet have dedicated float IR
  or backend lowering... crash the Cranelift verifier... ([#1218](...))") with a description of the
  shipped behavior: checked operators (`+!`, `-!`, `*!`, `/!`, `%!`) require integer operands;
  using them on `f32`/`f64` is rejected at type-check time with `UnsupportedFeature`, because
  "checked" only has meaning relative to integer overflow. Point readers to the unchecked
  operators for float arithmetic instead. Also add one sentence to the "Checked Arithmetic" section
  itself (~line 298, right after "Checked operators abort with `ArithmeticOverflow` on overflow.")
  noting the operands must be integer types, so the constraint is visible from both the wrapping
  and checked sections.
- Regenerate `--help` / embedded skill after the grammar.md edit:
  `uv run python scripts/generate_help.py`, then `cargo build --release -p vow` and
  `scripts/bootstrap.sh --skip-cargo` to rebuild `build/vowc` so the skill/help text embedded in
  the binary matches. Run `scripts/check_help_coverage.py` (or let `full_test.sh` catch it) to
  confirm no drift remains.
- `docs/spec/errors.md`'s existing `UnsupportedFeature` entry (line 335) is generic enough
  ("A language feature that is not supported in Vow was used") that it does not strictly need a
  new example, but if the implementer wants one, a short second `vow` snippet showing `a +! b` on
  `f64` fits the existing pattern used for `trait`/`impl` blocks. Not required to close the issue.

## 4. TDD slices

Each slice is a small, independently reviewable red-green step. Rust and self-hosted changes for
the same behavior are grouped into one slice (per CLAUDE.md's dual-compiler-parity rule: both must
land in the same session), but the test fixture is shared — `tests/error/*.vow` fixtures run
against *both* compilers via `scripts/full_test.sh::run_promoted_error_tests`, which asserts (via
`scripts/parity.py::compare_error`) that both compilers reach `status: CompileFailed` with at least
one diagnostic and an identical multiset of `error_code`s. That comparator does **not** read the
`// TEST: error-code X` / `// TEST: error-count N` comments programmatically (confirmed: no script
in this repo parses those directives; they are human-readable documentation only, matching the
existing convention in `tests/error/*.vow`) — so the pass/fail gate is cross-compiler parity, and
the comments must still be written accurately since they're what a reviewer reads.

1. **Rust: unit-test the new type-check seam in isolation.**
   - Test location: `vow-types/src/check.rs`, in `mod tests` (~line 3493), next to
     `same_operand_checks_pin_diagnostics_and_result_types` (~line 7062) which already exercises
     `check_same_numeric` directly via the `numeric_operand_result(lhs, rhs)` helper
     (~line 7045).
   - Behavior under test: a new `checked_numeric_operand_result(lhs, rhs)` helper (same shape,
     calling the new `check_checked_numeric` method) — assert that `(F64, F64)` emits exactly one
     `ErrorCode::UnsupportedFeature` diagnostic with a message naming the float type **and returns
     `Ty::F64`** (not `Ty::Unit` — see §3's anti-cascade rationale), that `(I64, I64)` still emits
     nothing and returns `Ty::I64` (no regression on the integer path), and that `(F64, I64)`
     (mismatched classes) emits the existing `TypeMismatch` from `check_same_numeric` and *not* an
     additional `UnsupportedFeature` (no double-report).
   - Production code: add `check_checked_numeric` next to `check_same_numeric`
     (`vow-types/src/check.rs:3095`) and wire the `BinOp::*Checked` match arm
     (`vow-types/src/check.rs:~1893`) to call it instead of `check_same_numeric`.
   - Red: helper doesn't exist / arm still calls `check_same_numeric` → test doesn't compile or the
     float case emits nothing. Green: helper added and wired.

2. **Self-hosted: mirror the checker change.**
   - Test location: no isolated self-hosted unit test for this seam (no existing precedent for
     checker-level unit tests of individual `BinOp` arms in `compiler/tests/test_checker.vow` — the
     existing file only tests the `item_files`-length invariant). Rely on slice 3's shared fixture
     for both compilers instead of inventing a new self-hosted-only unit test harness for one
     `if`-branch; this matches how `float_remainder_unsupported.vow` and
     `extern_call_unsupported.vow` were verified for the self-hosted side.
   - Behavior under test: same as slice 1, expressed as "does `compiler/checker.vow`'s arithmetic
     fallthrough reject checked+float".
   - Production code: `compiler/checker.vow`'s arithmetic fallthrough (~line 2388-2408), per §3.
   - Red: building `build/vowc` (`build/vowc build --no-verify compiler/main.vow -o
     "$TMPDIR/vow_main"` after `scripts/concat_vow.sh`, or the existing bootstrap path) against a
     scratch `a +! b : f64` fixture still crashes with `CodegenFailed`. Green: it now reports
     `UnsupportedFeature` before codegen runs. Use `VOW_CACHE_DIR=$(mktemp -d)` for these ad hoc
     rebuild-and-check invocations — the compile cache keys on source, not compiler version, so a
     patched `compiler/` can otherwise serve stale cached objects from before the fix (see prior
     session note on this repo's compile cache). Keep all scratch paths under `$TMPDIR`, never a
     bare `/tmp/...` path.

3. **Fixtures: land the five `tests/error/checked_*_float_unsupported.vow` files.**
   - Test location: `tests/error/` (5 new files, per §3).
   - Behavior under test: `scripts/full_test.sh::run_promoted_error_tests` picks up
     `tests/error/*.vow` automatically (glob, `full_test.sh:333`) — no registration step needed.
     Confirms both compilers reject with `status: CompileFailed` and matching `UnsupportedFeature`
     in `error_code`.
   - Production code: none new — this slice is purely the fixtures exercising slices 1+2's
     production code end-to-end (frontend → diagnostic, not just the isolated checker call).
   - Red (before slices 1-2 land): Rust reports `UnsupportedFeature`-shaped only if slice 1 is
     done; self-hosted still crashes with `CodegenFailed` until slice 2 lands, so
     `run_promoted_error_tests` fails parity (`self status=CodegenFailed, expected CompileFailed`)
     or, worse, one side segfaults. Green once both slices land: `run_promoted_error_tests` passes
     for all five fixtures.
   - Manually verify before committing: run
     `./target/release/vow build --no-verify tests/error/checked_add_float_unsupported.vow` and the
     self-hosted equivalent (`build/vowc build --no-verify ...` after rebuilding it from the
     patched `compiler/`) and confirm both print `UnsupportedFeature` JSON, not a crash/panic.

4. **Docs + regen.**
   - Test location: none (docs-only), but `scripts/check_help_coverage.py` (run inside
     `full_test.sh`) is the regression guard that catches drift between `grammar.md` and `--help`.
   - Behavior under test: `--help`/embedded skill text reflects the new grammar.md wording.
   - Production code: `docs/spec/grammar.md` edits (§3), then
     `uv run python scripts/generate_help.py`, `cargo build --release -p vow`,
     `scripts/bootstrap.sh --skip-cargo`.
   - Red: `scripts/check_help_coverage.py` (or eyeballing `vowc --help`) shows the old placeholder
     text mentioning issue #1218 by number, which reads oddly once the issue is closed. Green:
     `--help` matches the updated spec section, coverage check passes.

## 5. Verification surface

- **No new ESBMC proof obligations.** The fix is a type-checker rejection before IR/codegen/verify
  ever run; ESBMC never sees a program using checked float arithmetic (the build fails before
  verification is invoked). Nothing in `vow-verify` changes.
- **No new `tests/run/` or `examples/` fixtures needed.** There is no valid runtime behavior to
  demonstrate (the feature is rejected, not executed) — coverage lives entirely in `tests/error/`.
- **Existing checked-arithmetic verification tests are unaffected.** The "In verification" wording
  in `docs/spec/grammar.md`'s Checked Arithmetic section (i8/u8 through i64/u64 modelled, 128-bit
  `Skipped`) already implicitly assumes integer operands; the doc addition in §3 makes that
  assumption explicit rather than changing any verified behavior. No existing `tests/verify/` or
  `tests/verify-fail/` fixture uses checked float arithmetic (confirmed by search — no `.vow` file
  under `tests/`, `examples/`, or `benchmarks/` combines `f32`/`f64` with `+!`/`-!`/`*!`/`/!`/`%!`),
  so nothing needs updating there.

## 6. Risk areas

- **Binary fixed point.** The self-hosted change is a straight-line diagnostic-emission addition
  inside `check_expr_inner`, not a data-structure or iteration-order change — no `BTreeMap`/
  `HashMap` interaction, no new stack slots in `vow-clif-shim`, no codegen ordering change. Low
  risk to the bootstrap triple test, but the plan still runs it (`scripts/concat_vow.sh` +
  three-stage bootstrap + `sha256sum` compare) as part of the implementation stage's validation,
  per the repo's standing discipline, since any `compiler/*.vow` edit is in scope for that check.
- **`parse → print → parse` idempotency.** Untouched — this change is checker-only, not
  parser/printer. No grammar or AST shape changes.
- **`cargo clippy --all -- -D warnings`.** The new Rust arm is a straightforward match-arm split
  plus one new private method; watch for an unused-import or unreachable-pattern lint if
  `check_same_numeric`'s call sites are refactored carelessly. Run clippy before considering the
  Rust slice done.
- **Diagnostic double-reporting.** The main correctness risk is emitting *both* `TypeMismatch` (from
  `check_same_numeric` on a bad pair) and `UnsupportedFeature` (from the new float check) for the
  same expression, which would break the `error-count 1` expectation documented in the new
  fixtures and could cascade into unrelated `tests/error/*.vow` parity noise. Slice 1's unit test
  explicitly pins the `(F64, I64)` mismatched-class case to confirm only `TypeMismatch` fires.
  Same care needed on the self-hosted side: the float check must be gated on the *resolved* numeric
  type, not run independently of the existing "operands have different types" check.
  Note: `check_same_numeric` itself already returns `Ty::Unit` on error today, so `Ty::Unit.is_float()
  == false` naturally prevents the double-report if the new check is placed strictly after
  (dependent on) `check_same_numeric`'s result — no extra guard needed, just correct ordering.
- **Error-path return type must be the resolved float type, not a generic error sentinel.**
  Resolved in §3: both compilers return the operand's actual float type (`Ty::F64`/`lhs_tid`) from
  the new branch, not `Ty::Unit`/`CTY_I64()`. Getting this wrong in either compiler — or getting it
  wrong *differently* in the two compilers — risks a second, spurious `TypeMismatch` from a
  declared-return-type check on the enclosing function, which would both break the new fixtures'
  `error-count 1` expectation and show up as unexplained parity noise in unrelated multi-error
  fixtures. Slice 1/2's tests must assert the returned type explicitly, not just "some diagnostic
  fired."

## 7. Out of scope

- **Do not implement checked-float NaN/Inf semantics** (option 2 from the issue) — rejected by
  design per §2; would require new IR opcodes, dual-compiler backend lowering, and new ESBMC proof
  obligations for a feature with no requester and no principled "checked" meaning for floats.
- **Do not touch `binop_opcode` in either lowerer.** The buggy integer-opcode mapping for checked
  float ops becomes unreachable once the type checker rejects the program; deleting or "fixing" it
  is unrelated cleanup that would bundle a second change into a bug-fix PR (CLAUDE.md: "many small
  changes beat one large change"). Leave it as dead code for now; a follow-up could assert
  unreachability if desired, but that is not part of closing #1218.
- **Do not implement unchecked `%` float codegen** (`RemF32`/`RemF64` in `cranelift_backend.rs`).
  That is #1164's already-shipped, deliberate `CodegenUnsupported` fail-closed behavior and is
  unrelated to checked operators.
- **No refactor of the shared arithmetic-checking fallthrough** in either checker beyond the minimal
  split needed to special-case checked+float. Do not generalize into a broader "operator class"
  abstraction as part of this fix.
- **No changes to `ArithOverflowReachable` / ESBMC checked-arithmetic verification machinery** —
  out of scope; this issue is purely about the type-check-time rejection.
