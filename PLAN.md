# Plan: stop the `as u128`/`as i128` cast target leaking wide context into non-literal operands (#1581)

## Goal
`(i * 10 + j) as u128` with `i, j: u64` must lower `10` as a u64-compatible constant, not `ConstU128`, so Cranelift no longer rejects a u128 literal mixed with u64 operands. Fix both lowerers (Rust `vow-ir`, self-hosted `compiler/lower.vow`) identically; pure-literal cast operands keep their wide context.

## Root cause (verified)
- `vow-ir/src/lower/mod.rs:4313-4317` (`ExprKind::Cast` arm) and `compiler/lower.vow:5232-5237` (`EXPR_CAST`) call `record_wide_marker_context(operand, tgt)` for every `as i128/u128`, unconditionally.
- `record_wide_marker_context` (`mod.rs:4815`, `lower.vow:2359`) recurses through unary-neg / binop / block / if / match / loop and stores the 128-bit type for every sub-node. `lower_expr`'s `Lit::Int` arm (`mod.rs:1756-1763`) then emits `ConstU128`/`ConstI128` for any literal with a recorded context, so `10` in `i * 10` becomes a 128-bit constant next to a u64 `i`.
- `Vec<u128>::push((i*10+j) as u128)` reaches the same arm: `mod.rs:4122-4135` / `lower.vow:4627` call `record_wide_control_flow_context` (which ignores a `Cast` node) then `lower_consumed_expr`, which lowers the Cast.
- The checker only treats a cast operand as a bare literal when its type is `Ty::LitInt` (`vow-types/src/check.rs:711-714`, `CastVerdict::LiteralRange`; `check_integer_literal_range` at `:1848`). Any operand with a typed leaf (`i`, `j`) has a concrete type; its literals are reconciled by binop lowering (`mod.rs:1944-1990`, `contextual_narrow_literal_ty`). No cast context is needed there.

## Fix
Record the cast-target context only when the operand is literal-only (the lowering-side mirror of "operand types as `LitInt`"). New predicate, no new type-system axis, no checker change:

`int_expr_is_literal_only(expr)`: true for `Lit::Int`; `Neg` of literal-only; arithmetic/bitwise/shift `BinaryOp` (same op set as `expr_is_coercible_int_marker`, `mod.rs:1618`) with both sides literal-only; `Block` whose result is literal-only; `If` with else whose branch results are both literal-only; `Match` whose every arm body is literal-only; `Loop` whose break values (`loop_break_values`) are non-empty and all literal-only. Everything else (identifiers, calls, field/index, casts, params) is false.

Cast arm becomes: `if matches!(tgt, I128|U128) && int_expr_is_literal_only(expr) { record_wide_marker_context(ctx, expr, tgt) }`.

Why not reuse `expr_is_coercible_int_marker`: it excludes `Match`/`Loop` (the checker's twin `is_coercible_integer_marker` includes them), and `(match x { 0 => 1, _ => 1 << 100 }) as u128` relies on the cast context today; gating on the narrower predicate would regress it.

## Key Files
| File | Role | Lines |
|------|------|-------|
| `vow-ir/src/lower/mod.rs` | add `int_expr_is_literal_only`; gate Cast arm; add unit tests | predicate near `wide_context_contains_control_flow` ~4798; Cast arm 4313-4317; tests module 5900+ (helpers `lower_source_to_module` 6988, `insts_of` 7001) |
| `compiler/lower.vow` | twin predicate `int_expr_is_literal_only(a, eid)`; gate `EXPR_CAST` | near `wide_context_contains_control_flow` 2342; cast arm 5232-5237; reuse `block_result_eid`, `list_len/list_get`, `loop_break_value_eids` |
| `compiler/tests/test_lower_cast_wide_context.vow` [new] | Vow twin of the Rust unit test; model on `compiler/tests/test_lower_wide_match_marker.vow` | whole file |
| `tests/run/u128_cast_non_literal_operand.vow` [new] | end-to-end fixture run by both compilers | whole file |

No `docs/spec/*.md`, `--help`, skill or ADR change: bug fix, no syntax/semantics/CLI change (spec already says a u64 expression casts to u128 by zero-extension).

## Steps (TDD slices, in order)

### 1. Characterize literal-only casts (green before and after)
- `tests/run/u128_cast_non_literal_operand.vow` [new], first only the literal-only part, to pin behaviour the fix must not change: `((1 << 100) as u128) >> 99` -> 2 (u128), `(5 + 7) as i128`, a literal-arm `match`/`if` operand cast to u128 with a value above `u64::MAX` (e.g. `match n { 0 => 1 << 100, _ => 3 }`), negative `(-5) as i128`. Print with `print_i64(... as i64)`-style narrowing via `u128_to_u8_wrap` / `i128_to_i64_*` intrinsics already used in `tests/run/i128_codegen.vow`. Directive: `// TEST: stdout "..."` (see `tests/run/i32_widening_cast.vow`).
- Run on the unmodified tree with both compilers; adjust expected output only if it disagrees with Rust-vs-self-hosted parity (record as a pre-existing bug and drop that line rather than bundling a fix).

### 2. RED: Rust unit test
- `vow-ir/src/lower/mod.rs` tests (use `lower_source_to_module` with an empty `PayloadScalarMap`; the leak is in the Cast arm regardless of `checked_wide_vec_elem`, so `push` is reproducible with the plain helper too):
  - `cast_to_u128_keeps_typed_operand_at_its_own_width`: source `fn f(i: u64, j: u64) -> u128 { (i * 10 + j) as u128 }` (no trailing `== 34` comparison: a u128-vs-literal compare legitimately emits `ConstU128` and would defeat a zero-wide-const assertion). Assert: no `ConstU128`/`ConstI128` inst; the `Mul` and `Add` insts and both their operands are typed `U64`; there is an `IntCast` U64 -> U128 whose operand is the `Add`.
  - Baseline oracle (the actual invariant: a non-literal cast operand lowers exactly as without the cast): also lower `fn g(i: u64, j: u64) -> u64 { i * 10 + j }`; assert the cast form's operand instruction opcodes/types and constant widths equal the uncast baseline's, modulo the trailing `IntCast`.
  - `cast_to_u128_in_vec_push_keeps_typed_operand_at_its_own_width`: `v.push((i * 10 + j) as u128)` with `v: Vec<u128>`; same assertions on the cast operand (end-to-end fixture covers the real `Vec<u128>` store).
  - `cast_to_u128_of_literal_only_operand_stays_wide`: `(1 << 100) as u128` still yields `ConstU128` consts (guards the gate from over-narrowing).
- Fails today with `ConstU128(10)` feeding the `Mul`.

### 3. GREEN: Rust fix
- Add `int_expr_is_literal_only`, gate the Cast arm. Reuse `integer_marker_from_block` (`mod.rs:4565`) and `loop_break_values`. Run `cargo test -p vow-ir`.

### 4. RED -> GREEN: Vow twin
- `compiler/tests/test_lower_cast_wide_context.vow` [new]: the same four assertions (typed-operand widths, baseline-equivalence, push, literal-only stays wide) through `lower_module_vow` (copy the `lower_*`/`all_insts` scaffolding of `test_lower_wide_match_marker.vow`; ids via `IOP_CONST_U128()` / `ITY_U64()` from `ir.vow`). Run `build/vowc test compiler/tests/test_lower_cast_wide_context.vow` against the current `build/vowc` -> red.
- Add `int_expr_is_literal_only(a, eid)` to `compiler/lower.vow` (same recursion, arena accessors as in `wide_context_contains_control_flow`), gate the `EXPR_CAST` arm. Use `while` loops with `u64`/`i64` indices as in `lctx_record_wide_marker_context`; no closures.

### 5. Extend the end-to-end fixture
- Add to `tests/run/u128_cast_non_literal_operand.vow`: the issue reproducer (exit/print 34), `Vec<u128>` push of `(i * 10 + j) as u128` then read back, `i128` variant, narrower sources (`u8`, `i32` typed vars with a literal: `(a * 10 + 1) as u128`), mixed branches `(if c { i } else { 2 }) as u128` and `(match k { 0 => i, _ => 2 }) as u128` with `i: u64`, and `(i * 10 + j) as u128` where the product exceeds `i64::MAX` but not `u64::MAX` (zero-extension, not sign-extension).
- Before adding the mixed-branch lines, run their uncast equivalents with an unannotated let (`let y = if c { i } else { 2 };`) on the unmodified tree. If the uncast form also fails it is a separate merge bug: file a follow-up issue and drop the mixed-branch lines from the fixture; do not add new context recording here. Only if the uncast form works and the cast form fails after the fix is it in scope.

### 6. Gates
- `cargo fmt --all`, `cargo clippy --all --all-targets -- -D warnings`, `cargo test --all`.
- `scripts/bootstrap.sh --skip-cargo --no-cache` (record the final head SHA), then `build/vowc test compiler/`, then `VOW_FULL_TEST_SKIP_CARGO=1 scripts/full_test.sh` (background + poll; ~40 min).
- Validate codegen with `VOW_CACHE_DIR=$(mktemp -d)` (compile cache ignores compiler changes).

## Verification surface
- No contracts or ESBMC properties change. The change alters lowered IR for typed-operand casts to 128-bit types (literal consts become `ConstI64`/`ConstU64` + IntCast instead of `ConstU128`), which feeds `c_emitter.{rs,vow}`; because both lowerers change symmetrically, `scripts/parity.py c` (full_test.sh Section 2c over `tests/verify*/`) must stay byte-identical. No new verify fixture required unless one of the existing `tests/verify*/` fixtures casts a typed expression to u128 (grep `tests/verify*/` when implementing; `compiler/*.vow` contains no such casts).
- Fixtures: one new `tests/run/` file and one new `compiler/tests/` file, nothing under `examples/`.

## Risks
- **Mixed literal/typed merge** (`if c { i } else { 2 }` under a cast): probably already broken today (a u64 Upsilon meets a `ConstU128` branch). Handled by the Step 5 uncast-baseline check; treat a failing baseline as a separate follow-up issue, not part of this PR.
- **Predicate drift**: the new predicate must match in both compilers (the Vow twin cannot use closures/`matches!`); keep op sets identical to `expr_is_coercible_int_marker` and assert via the paired tests.
- **Fixed point**: no `as u128`/`as i128` occurs anywhere in `compiler/*.vow` (grepped), so the fix cannot change the self-hosted compiler's own codegen; low risk. Still re-run the bootstrap triple. `BTreeMap`/slot ordering untouched.
- **Clippy `-D warnings`** (includes test targets): avoid needless `matches!` nesting and unused helper warnings.
- **Assumption**: `Match`/`Loop` scrutinee/loop condition expressions are irrelevant to literal-only-ness (only result positions matter), mirroring `record_wide_marker_context`'s descent.

## Out of scope
- Any checker/type-table change (no per-expression type map exposed to lowering).
- `record_wide_control_flow_context` / `record_wide_expected_ast_context` behaviour (they already ignore `Cast` nodes).
- Cast context for non-wide targets, shift-count typing, Vec<u128> layout, region/verifier changes, refactoring or deduplicating the two predicate families (`expr_is_coercible_int_marker` vs checker's).
- Docs/spec edits.
