# Plan: fix(ir): float arithmetic opcodes are unreachable in both lowerers (#1164)

## 1. Problem restated

`binop_opcode` in `vow-ir/src/lower/mod.rs` and its counterpart in
`compiler/lower.vow` map every `BinOp::{Add,Sub,Mul,Div,Rem}` to the wrapping
*integer* opcode (`WrappingAdd`/`IOP_WADD` etc.) regardless of operand type.
The self-hosted version literally ignores its `operand_ty` parameter. Both
type checkers admit `f64 + f64` (via `check_same_numeric`), so a program like
`fn fadd(a: f64, b: f64) -> f64 { a + b }` type-checks, then emits an integer
add over Cranelift `F64` SSA values, which both backends reject at codegen
(`CodegenFailed`). The `AddF32/64`..`DivF32/64` opcodes and the `RemF32/64`
"unsupported" path already have full, working, and even *tested* backend
implementations (Cranelift backend, `vow-clif-shim`, and both C emitters) —
they are simply never produced by either lowerer. The fix is a dispatch-only
change in exactly two functions (`binop_opcode` in each compiler, plus
`binop_result_ty` and the emit call site in the self-hosted lowerer); no new
opcodes, no backend work, no C-model work.

Float `%`/`%!` is a separate, already-decided question: both backends
deliberately answer `RemF32`/`RemF64` with `CodegenUnsupported`
(`UnsupportedOpcode("float remainder not yet supported")` in
`cranelift_backend.rs`, `CLIF_ERR_UNSUPPORTED` in `vow-clif-shim`, both tested
by `float_remainder_is_refused_as_unsupported` in `vow-clif-shim/src/lib.rs`
from #1163). This plan does not change that decision — it makes the opcode
*reachable* so the existing refusal actually fires instead of the lowerer
silently mis-emitting an integer op.

## 2. Files to touch

**Rust compiler (`vow-ir`):**
- `vow-ir/src/lower/mod.rs` — `binop_opcode` (~line 4713): add a float
  dispatch branch for `BinOp::{Add,Sub,Mul,Div,Rem}` when `operand_ty` is
  `Ty::F32`/`Ty::F64`, returning `(Opcode::{Add,Sub,Mul,Div,Rem}F32/F64,
  operand_ty, InstData::None)`. Leave the existing integer match arm chain
  untouched as the fallback for everything else (comparisons, bitwise,
  checked-arith, and any float-typed non-arithmetic op — all pre-existing
  behavior, unchanged). The two call sites (`lower_expr`'s `EXPR_BINOP` path
  at line 1602, and `lower_integer_marker_as` at line 4665) need no changes —
  the second is gated by `expr_is_coercible_int_marker`, which is
  integer-literal-only and never reached with a float operand.

**Self-hosted compiler (`compiler/`):**
- `compiler/lower.vow`:
  - Add `ir_ty_is_float(ty: i64) -> bool` near `ir_ty_is_integer` (~line
    1680): `ty == ITY_F32() || ty == ITY_F64()`.
  - `binop_opcode` (~line 1283): add the same float-dispatch branch at the
    top, before the existing `if op == BINOP_ADD() { ... }` chain, using
    `IOP_ADD_F32()/IOP_ADD_F64()` etc. (already defined in `compiler/ir.vow`,
    opcodes 40–55). Contract (`requires: is_valid_binop(op)`, `ensures:
    result != -1`) is unaffected — every new branch returns a valid opcode
    constant.
  - `binop_result_ty` (~line 1325) and its postcondition helper
    `is_binop_result_ty` (~line 1320): currently `binop_result_ty` falls
    through to `ITY_I64()` for any non-integer `operand_ty`, which is wrong
    for floats today (dead code, since the caller never asked for a float
    result via this path) and would become live-wrong once `binop_opcode`
    emits `IOP_ADD_F64`. Add `if ir_ty_is_float(operand_ty) { return
    operand_ty; }` before the `ITY_I64()` fallback, and extend
    `is_binop_result_ty` to accept `ir_ty_is_float(ty)` as well as
    `ir_ty_is_integer(ty)`/`ITY_BOOL()`.
  - The `EXPR_BINOP` emit call site (~line 2346–2351): currently always
    passes `IDATA_INTEGER()` with `operand_ty` as the data value. For a float
    `operand_ty` this must become `IDATA_NONE()` (data value `0`), matching
    the Rust side's `InstData::None` and the existing backend test fixtures
    for `AddF64`/etc in `cranelift_backend.rs` and `c_emitter.rs`, which all
    use `InstData::None`. (The `lower_integer_marker_as` call site at
    line 2033–2035 is integer-only, same reasoning as the Rust side — no
    change.)

**Tests (both compilers, added in this PR):**
- `vow-ir/src/lower/mod.rs` — new `#[test]` fn(s) near the existing
  `Opcode::WrappingAdd`-searching tests (~line 5891, 6542, 6817) asserting
  float binops lower to `*F32`/`*F64` opcodes with `InstData::None`.
- `compiler/tests/test_lower_float_binop.vow` — new file, following the
  `compiler/tests/test_lower_inst_ty_index.vow` convention (`use lower`, call
  `binop_opcode`/`binop_result_ty` directly, return a nonzero code on
  mismatch).
- `tests/run/float_arithmetic.vow` — new build+run smoke fixture.
- `tests/verify/float_arithmetic.vow` — new ESBMC-provable fixture.
- `tests/error/float_remainder_unsupported.vow` — the fixture #1163's
  reviewers asked for.

**Docs:**
- `docs/spec/grammar.md` — the "Wrapping Arithmetic" section (~line 255–300)
  documents 128-bit operator gaps in prose but says nothing about float
  operator support. Add an analogous short paragraph: `+`/`-`/`*`/`/` on
  `f32`/`f64` lower to native Cranelift float ops; `%`/`%!` on floats type-check
  but have no backend lowering yet and fail closed with `CodegenUnsupported`
  at build time — a backend gap, not a language rule (mirrors the existing
  128-bit framing).
- After the grammar.md edit: regenerate `--help`/skill per CLAUDE.md
  (`uv run python scripts/generate_help.py`, then rebuild
  `cargo build --release -p vow` and `scripts/bootstrap.sh --skip-cargo`) so
  `scripts/check_help_coverage.py` doesn't flag drift.

**Not touched (verified already correct, no work needed):**
`vow-codegen/src/cranelift_backend.rs`, `vow-clif-shim/src/lib.rs`,
`vow-verify/src/c_emitter.rs`, `compiler/c_emitter.vow`, `compiler/ir.vow`,
`compiler/ir_printer.vow`, `vow-ir/src/printer.rs`, `vow-ir/src/serialize.rs`
— all already have full, matching float-arithmetic support (confirmed by
reading each). `docs/equivalence/ledger.json` — **no new entry**: once both
lowerers dispatch correctly, both compilers answer float remainder with the
same `CodegenUnsupported`/message, so there is no divergence to suppress (an
entry for a non-reproducing divergence is, per the issue itself, actively
wrong).

## 3. TDD slices

1. **Rust: float arithmetic dispatch.** Red: add a lowering unit test
   (module with `fn f(a: f64, b: f64) -> f64 { a + b }` style bodies for
   `+ - * /`, plus one `f32` variant) asserting the emitted instruction is
   `Opcode::AddF64`/`SubF64`/`MulF64`/`DivF64`/`AddF32` etc. with
   `InstData::None` — fails today because the opcode is `WrappingAdd`/etc.
   Green: implement the float branch in `binop_opcode`
   (`vow-ir/src/lower/mod.rs`).
2. **Rust: float remainder reaches `RemF32`/`RemF64`.** Red: assert `a % b`
   for `f64` lowers to `Opcode::RemF64` (not `WrappingRem`) — can be one
   assertion added to slice 1's test or its own small test. Green: covered by
   the same `binop_opcode` change (no separate production edit).
3. **Self-hosted: mirror slices 1–2.** Red:
   `compiler/tests/test_lower_float_binop.vow` calling `binop_opcode(BINOP_ADD(),
   ITY_F64())` (and Sub/Mul/Div/Rem, at both `ITY_F32()`/`ITY_F64()`)
   asserting `IOP_ADD_F32()`/`IOP_ADD_F64()`/etc., plus
   `binop_result_ty(BINOP_ADD(), ITY_F64()) == ITY_F64()` (fails today —
   returns `ITY_I64()`). Green: the `ir_ty_is_float`,
   `binop_opcode`, and `binop_result_ty` edits in `compiler/lower.vow`. Run
   via the self-hosted `vowc test` harness (see `compiler/main.vow`'s `test`
   subcommand).
4. **Self-hosted: `IDATA_NONE` on the float emit path.** Red: extend the
   slice-3 test (or add one) that lowers a full `EXPR_BINOP` AST node for
   `a + b` with `a`,`b`: f64 through the real `lower_expr` path (not just
   `binop_opcode` directly) and asserts the resulting inst's data kind is
   `IDATA_NONE()`, not `IDATA_INTEGER()`. Green: the emit-call-site edit at
   `compiler/lower.vow` ~line 2351.
5. **Cross-compiler build regression: `tests/run/float_arithmetic.vow`.**
   Red: add the fixture exercising `+ - * /` at both `f32` and `f64`
   (parameters only — float literals are always typed `f64` by both type
   checkers with no coercion marker, so an `f32` fixture must avoid mixing in
   a bare literal); build with `--no-verify` on both compilers, run, assert
   the process exits 0 (there is no float print/debug helper in
   `vow-runtime`, so this fixture proves "builds, links, and runs without a
   Cranelift verifier crash," not the numeric result — call that out in the
   fixture's header comment rather than implying more). Green: already
   satisfied by slices 1–4 landing; this fixture is a pure regression guard
   with no further production change.
6. **Verification regression: `tests/verify/float_arithmetic.vow`.** Red:
   add a fixture with tight identity postconditions (e.g. `fn fadd(a: f64, b:
   f64) -> f64 vow { ensures: result == a + b } { a + b }`, one per operator
   at f64) expected to reach `Verified` (Section 4b of `scripts/full_test.sh`
   already asserts the absolute `Verified` status, not just cross-compiler
   parity). This is a stronger correctness signal than the runtime smoke
   test: the C model's `Opcode::AddF64` arm emits real C `double` arithmetic,
   so ESBMC checks the identity under IEEE-754 semantics, not just "did it
   build." Green: already satisfied by slices 1–4; the C emitter needed no
   changes (already modelable for `AddF32`..`DivF32/64`).
7. **The #1163-requested fixture: `tests/error/float_remainder_unsupported.vow`.**
   Red: add the fixture (`fn frem(a: f64, b: f64) -> f64 { a % b }`),
   asserting `// TEST: error-code CodegenUnsupported` and `// TEST:
   error-count 1`. Before fixing the exact `// TEST: build-json '...' in
   x.get('message', '')` substring, run both `build/vowc build --no-verify`
   and the Rust `vow build --no-verify` on the fixture and read the actual
   `message` field from each JSON — do not assume the Rust
   `"float remainder not yet supported"` string and the self-hosted shim's
   surfaced message match verbatim; assert on a substring both share (or
   file a follow-up if they don't, per the "Risk areas" note below). Green:
   already satisfied by slices 1–4 (this is the fixture the issue exists to
   unblock; no new production code).
8. **Docs.** Red/green is not applicable to prose, but treat it as a slice
   with its own review: add the grammar.md paragraph, then regenerate
   `--help`/skill and confirm `scripts/check_help_coverage.py` (run inside
   `scripts/full_test.sh`) is clean.

Slices 1–2 and 3–4 are independent of each other and can be implemented in
either order, but both must land in the same PR (CLAUDE.md: language-semantics
changes land in both compilers in the same session). Slices 5–7 depend on
1–4 all being in place. Slice 8 can happen any time after the behavior is
settled.

## 4. Verification surface

- **No C-model changes required.** `vow-verify/src/c_emitter.rs::is_modelable`
  already lists `AddF32/64`..`DivF32/64` as modelable (returns `true`) and
  `RemF32/64` as **not** modelable (`false`, with a dedicated
  `first_unsupported_opcode` entry) — confirmed by reading both the Rust
  (`vow-verify/src/c_emitter.rs`) and self-hosted (`compiler/c_emitter.vow`)
  emitters, which already agree. This fix makes that existing, tested
  modeling code reachable for the first time.
- **New property proved:** slice 6's `tests/verify/float_arithmetic.vow`
  proves `result == a <op> b` for `+ - * /` at `f64` under ESBMC's IEEE-754
  float model — a genuine new verified property, not a verification-artifact
  bound. Do **not** add a `%` case there (it's `Skipped`/unmodelable by
  design, matching the `RemF*` line already documented in
  `docs/spec/errors.md`/`cli.md`).
- **`tests/run/` fixture (slice 5) is intentionally weak on correctness** —
  no float print/cast/compare path exists in `vow-runtime` to observe the
  computed value at runtime (checked: no `__vow_print_f*`/`__vow_debug_f*`,
  and there is no float→int cast opcode). It only proves the backend doesn't
  crash. The verify fixture (slice 6) is the actual correctness proof for
  this PR; say so in both fixtures' header comments so a future reader
  doesn't over-read the run fixture's guarantee.
- **`tests/error/` + `scripts/parity.py`/ledger:** confirmed
  `scripts/parity.py` only needs a `docs/equivalence/ledger.json` entry for a
  *divergence* between the two compilers' `error_code`s. Since both compilers
  will emit the same `CodegenUnsupported` for float remainder post-fix, no
  ledger entry is added — matches the issue body's own conclusion.

## 5. Risk areas

- **Self-hosted contract provability.** `binop_opcode`'s `ensures: result !=
  -1` and the new `is_binop_result_ty` disjunct are ESBMC-checked as part of
  bootstrapping the self-hosted compiler itself (`compiler/*.vow` carry `vow`
  contracts). The added branches are straightforward (bounded `op` values,
  simple equality dispatch), but confirm `scripts/bootstrap.sh` (full
  verification, not `--skip-cargo`) passes for `compiler/lower.vow` before
  considering the self-hosted side done — don't just run the `--skip-cargo`
  path.
- **Binary fixed point.** No opcode numbering, `BTreeMap`/`HashMap` usage, or
  `vow-clif-shim` stack-slot layout changes — this PR only changes dispatch
  logic in two existing functions plus one emit call site's data-kind
  argument. Still, rebuild in the standard order after the self-hosted change
  (`cargo build --release -p vow` → `scripts/bootstrap.sh --skip-cargo` →
  triple-test via `scripts/concat_vow.sh`) and reconverge the binary fixed
  point, per the ordinary "compiler binary changed" discipline — not a
  special risk unique to this change, but do not skip it.
- **Compile cache staleness.** Per project memory: the compile cache ignores
  compiler rebuilds. Use `VOW_CACHE_DIR=$(mktemp -d)` when manually
  validating the `tests/run`/`tests/verify` fixtures against a freshly
  rebuilt `build/vowc`, or the fixtures may silently execute stale cached
  objects from before the fix.
- **`clippy --all -- -D warnings`** (no `--all-targets`, per project memory —
  don't fix lints in `#[cfg(test)]` modules that CI doesn't gate). The new
  `binop_opcode` float branch and any small helper must not introduce
  unused-variable or unreachable-pattern warnings in non-test code.
- **`tests/error` message-text assumption.** Flagged in slice 7: the exact
  surfaced `message` string for the self-hosted compiler's
  `CLIF_ERR_UNSUPPORTED` path is not yet confirmed to match the Rust side's
  `"float remainder not yet supported"` verbatim (the shim only `eprintln!`s
  a similar-but-not-identical string; the JSON `message` field construction
  from `CLIF_ERR_UNSUPPORTED` needs to be traced, not assumed). If they
  diverge in wording (but agree on `error_code`), assert on `error-code`
  alone plus a substring both happen to share, rather than forcing message
  parity that isn't actually required by the ledger/parity infrastructure.
- **`f32` literal coercion.** Confirmed `Lit::Float(_) => Ty::F64`
  unconditionally in `vow-types/src/check.rs` (no coercible-float-marker
  analog to the integer literal system) — an `f32` fixture that mixes a bare
  float literal (`x_f32 + 1.0`) will fail to type-check on operand-type
  mismatch, independent of this fix. Keep all `f32` test fixtures to
  explicitly `f32`-typed operands only; do not treat a literal-mixing failure
  as a regression from this change.
- **parse → print → parse idempotency:** unaffected — no grammar, token, or
  AST changes; the printer already prints `AddF64`/etc. correctly (verified
  in `vow-ir/src/printer.rs`), this PR only changes which opcode gets
  produced for a given source expression, not how any opcode prints.

## 6. Out of scope

- **Float comparisons (`==`,`!=`,`<`,`<=`,`>`,`>=`) on `f32`/`f64`.** Same
  root-cause bug, same function (`binop_opcode` maps `BinOp::Eq`.. unconditionally
  to `Opcode::Eq`, never `EqF32`/`EqF64`), and all three backends
  (Cranelift, `vow-clif-shim`, both C emitters) already fully implement
  `EqF32/64`..`GeF32/64` — confirmed by reading each. This is issue #600's
  territory (open, broader-scoped, filed before #1164 narrowed the ask to
  arithmetic only); #1164's title and body are explicit about "arithmetic
  opcodes." Fixing it here would be scope creep beyond the assigned issue.
  Structure the float branch in both `binop_opcode` implementations as an
  isolated, additive match (see plan section 2) so a follow-up PR against
  #600 can extend it mechanically — same shape, same call sites, zero new
  backend work, since the backend arms already exist. A `gh issue comment`
  on #1164 records this scope decision for the operator.
- **Checked float arithmetic (`+!`,`-!`,`*!`,`/!`,`%!` on floats).** The type
  checker currently admits these (routes through `check_same_numeric` same as
  wrapping arithmetic), but there is no `CheckedAddF32`/etc. IR opcode and no
  backend support at all — this is a pre-existing, deeper gap (what would
  "checked" even mean for float overflow to infinity?) that #1164 does not
  ask about and this plan does not attempt to answer. Left exactly as
  currently broken (unreachable/mismatched), matching status quo.
  Genuinely a design question (not just wiring), so it needs its own issue
  if anyone wants it, rather than an implicit decision bundled into this fix.
- **Deciding float `%` semantics from scratch.** Already decided in #1163
  (`CodegenUnsupported`, tested by `float_remainder_is_refused_as_unsupported`).
  This plan only makes that decision reachable; it does not revisit whether
  to implement `fmod` via a runtime helper or reject at type-check time
  instead — either is a legitimate follow-up, neither is required to close
  #1164.
- **No refactor of `binop_opcode`'s overall shape** beyond adding the float
  branch (e.g. not restructuring the Rust match into a table, not touching
  the self-hosted `if`-chain style) — matches existing code style in both
  files; a bug fix doesn't need surrounding cleanup.
