# Plan: issue #980 — if-expression branches with a bare literal + narrow-typed value merge at mismatched Cranelift widths

## 1. Problem restated

When an `if`/`else` expression is used as a value (not an assign-statement) and exactly one
branch is a bare integer literal (a "marker" expression, speculatively lowered at `i64`/32-bit
register width) while the other branch is a genuinely narrow-typed value (`u8`/`i32`/`u32`/etc,
e.g. a parameter or a prior binding), both compilers' IR lowering picks `merge_phi_ty` /
`choose_match_result_ty`'s simpler sibling — a plain "primary wins" rule — for the merge `Phi`'s
type, and never re-narrows the marker branch's `Upsilon` argument to that width. Depending on
branch order this produces one of two broken shapes: either the `Phi` is declared at the
marker's `i64` width while the real-typed branch's `Upsilon` writes a narrower value, or the
`Phi` is declared at the narrow width while the marker branch's `Upsilon` still writes the
speculative `i64`-width constant. Either way, Cranelift's SSA verifier rejects the mismatched
block-parameter write at `build` time (not `verify` time, since ESBMC works from the vow-level
model, not emitted Cranelift IR). The exact same class of bug was already fixed for `match`-arm
merges (`choose_match_result_ty` + a re-narrowing splice, landed for #976/#979); this issue is
about porting that fix to the two-branch `if`-expression-as-value merge site, which the earlier
work explicitly deferred.

## 2. Files to touch

Rust compiler (`vow-ir` crate):
- `vow-ir/src/lower/mod.rs`
  - `ExprKind::If` handling, the `(Some(t_up), Some(e_up))` arm of the final
    `match (then_upsilon_id, else_upsilon_id)` block (currently ~line 2082-2088, the arm that
    does `let phi_ty = merge_phi_ty(ctx.inst_ty(then_val), ctx.inst_ty(else_val));` then
    immediately backpatches both Upsilons with no re-narrowing). This is the only behavioral
    change site.
  - Add one small new helper near `merge_phi_ty` (~line 1503): `merge_if_result_ty(then_ty: Ty,
    then_is_marker: bool, else_ty: Ty, else_is_marker: bool) -> Ty`, mirroring
    `choose_match_result_ty`'s marker-aware rule reduced to exactly two arms. `merge_phi_ty`
    itself stays untouched — it is still correct and still used as-is for the mutated-variable
    Phi loop in the same `ExprKind::If` arm (mutated variables are never literal markers, so the
    marker-aware rule doesn't apply there).
  - Add one small new private helper, e.g. `renarrow_if_branch_result(ctx, block, up_id,
    result_expr, target_ty, span) -> Option<InstId>`, factored out of (but not shared with, see
    §6) the existing splice-by-id logic already proven out for `match` arms at
    ~line 3553-3579 (locate the Upsilon by id, not by block-tail position, because the
    mutated-variable Phi loop earlier in the same function may already have appended extra
    Upsilons after it; split off the tail, truncate, re-lower via the existing
    `lower_narrow_literal`, re-emit the Upsilon, restore the tail).
  - New unit test(s) in the existing `#[cfg(test)] mod tests` block (same file), see §3.

Self-hosted compiler (`compiler/`):
- `compiler/lower.vow`
  - `EXPR_IF()` handling in `lower_expr`, the final unconditional merge branch (currently
    ~line 3038-3044: `let result_ty: i64 = merge_phi_ty(lctx_inst_ty(ctx, then_result),
    lctx_inst_ty(ctx, else_result));` followed immediately by both `backpatch_upsilon` calls).
    This is the mirror of the Rust change site; same fix shape.
  - Add `fn merge_if_result_ty(then_ty: i64, then_is_marker: bool, else_ty: i64, else_is_marker:
    bool) -> i64` near `merge_phi_ty` (~line 2321), mirroring `choose_match_result_ty`'s rule.
  - Add `fn renarrow_if_branch_result(ctx: LowerCtx, a: AstArena, block_id: i64, up_id: i64,
    result_eid: i64, target_ty: i64, span: i64) -> i64` (self-hosted has no `Option`, so return
    the original `up_id` unchanged when the Upsilon can't be located, matching the existing
    `if up_pos != -1 { ... }` guard style used by the `match`-arm splice at ~line 4533-4588).
    `then_bid`'s own result expr is obtained via the already-existing `block_result_eid(a,
    then_bid)`; the else branch's result expr is `else_eid` directly (already in scope, `-1`
    when there is no `else`).
  - No new test *files* here beyond the shared `tests/run/*.vow` fixture (self-hosted and Rust
    share that fixture; see §3) — `tests/run_tests.sh`/`scripts/full_test.sh` build and run it
    with **both** compilers and diff their JSON + stdout, so one fixture exercises both sides of
    this change.

Docs:
- No `docs/spec/*.md` update. This is a pure internal IR-lowering correctness fix: it changes
  neither syntax, semantics, a builtin signature, an operator, an effect, nor a CLI flag — a
  `clamp_u8`-shaped program is already valid Vow per the spec, and today's bug is that valid
  program crashes the backend instead of compiling. Nothing in `grammar.md`/`cli.md`/
  `contracts.md`/`errors.md`/`examples.md` describes Cranelift-internal Phi/Upsilon widths.

## 3. TDD slices

1. **Red (Rust, IR-level, fast).** Add a unit test in `vow-ir/src/lower/mod.rs`'s test module,
   modeled directly on the existing `annotated_narrow_local_reduces_control_flow_result` test
   (constructs an `ExprKind::If` AST node and calls `lower_function` directly — no Cranelift
   backend needed, since the bug is already observable as inconsistent IR before codegen).
   Build a `fn clamp_u8(v: u8) -> u8 { if v > 200 { 200 } else { v } }`-shaped `FnDef` using
   `make_fn`/`make_param`/`named_ty`/`int_expr`/`ident_expr` exactly as the existing test does,
   lower it, then:
   - find the IR `Phi` instruction and record its `Ty`,
   - find both `Upsilon` instructions whose `InstData::PhiTarget` points at that `Phi`'s id,
   - for each Upsilon, resolve `args[0]` to its producing instruction and assert that
     instruction's `Ty` equals the Phi's `Ty`.
   Add a second case in the same test (or a sibling test) for the mirrored branch order,
   `if v > 200 { v } else { 200 }`, since the bug manifests differently depending on which side
   is the marker (see the "mirrored branch order" note below). Both cases fail today: in the
   `{200}else{v}` order the Phi is typed `I64` while the `else` Upsilon's argument is `U8`; in
   the `{v}else{200}` order the Phi is typed `U8` while the `else` Upsilon's argument is `I64`.
   Also add one case for `i32`/`u32` directly mirroring the issue title's named types, to avoid
   over-fitting the fix to `u8`.
2. **Green (Rust).** Implement `merge_if_result_ty` and `renarrow_if_branch_result` in
   `vow-ir/src/lower/mod.rs` and wire them into the `(Some(t_up), Some(e_up))` arm: compute
   `then_is_marker = block_result_is_coercible_int_marker(then_branch)` and `else_is_marker =
   else_branch.as_deref().is_some_and(expr_is_coercible_int_marker)` right where `then_val`/
   `else_val` are already in scope; replace the `merge_phi_ty(...)` call with
   `merge_if_result_ty(...)`; when `narrow_int_width(phi_ty).is_some()`, call
   `renarrow_if_branch_result` for whichever side is a marker with a mismatched arm type, using
   `integer_marker_from_block(then_branch)` to get the "then" side's result expr and `else_expr`
   directly for the "else" side. Re-run slice 1's tests; they must now pass. Run
   `cargo test -p vow-ir` in full to confirm no existing `match`/`if`/`let` narrow-literal test
   regresses (in particular `narrow_int_width_and_divergence_are_exhaustive_over_ty`,
   `narrow_seam_reproduces_the_legacy_inline_gates`, and the existing If-as-marker tests).
3. **Red/Green pair, both compilers, e2e (`tests/run/`).** Add
   `tests/run/if_expr_merge_width.vow`, modeled on `tests/run/match_arm_merge_width.vow` and
   `tests/run/narrow_branch_returns.vow`, covering the issue's exact repro plus the mirrored
   branch order and the `i32`/`u32` cases from the issue title:
   ```vow
   fn clamp_u8_hi(v: u8) -> u8 { if v > 200 { 200 } else { v } }   // literal-then, value-else
   fn clamp_u8_lo(v: u8) -> u8 { if v < 10 { v } else { 10 } }     // value-then, literal-else
   fn clamp_i32(v: i32) -> i32 { if v > 1000 { 1000 } else { v } }
   fn clamp_u32(v: u32) -> u32 { if v > 1000 { 1000 } else { v } }
   ```
   with a `main` that prints results for both in-range and clamped inputs (`// TEST: stdout
   "..."`). Before the fix this fixture fails `vow build --no-verify` on both compilers (Cranelift
   verifier panic) — confirm that red state by running it manually with `build/vowc build
   --no-verify` and `./target/release/vow build --no-verify` before implementing the self-hosted
   fix, so the regression test is proven to fail for the right reason, not a typo. After step 2
   (Rust fix) plus the mirrored self-hosted fix (next slice), both compilers build and run it, and
   `scripts/full_test.sh`'s `run_promoted_run_tests` / `tests/run_tests.sh`'s Phase 1 diff their
   JSON and stdout as usual.
4. **Green (self-hosted).** Implement `merge_if_result_ty` and `renarrow_if_branch_result` in
   `compiler/lower.vow`, wire them into the final `EXPR_IF()` merge branch the same way, using
   `block_result_is_coercible_int_marker(a, then_bid)` and `else_eid != -1 &&
   expr_is_coercible_int_marker(a, else_eid)` for the marker flags. Re-bootstrap
   (`scripts/bootstrap.sh --skip-cargo`, since only `compiler/*.vow` and already-rebuilt
   `./target/release/vow` are involved) and confirm slice 3's fixture now passes on
   `build/vowc` too.
5. **Full-suite confirmation.** Run `cargo test --all`, `scripts/full_test.sh`, and the
   self-hosted `tests/run_tests.sh` in full (not just the new fixture) to catch any
   previously-passing `tests/run/*.vow` fixture that happens to exercise if-expression-as-value
   merges incidentally (e.g. anything using `if`/`else` inside a `let` initializer, a `match` arm
   body, or a function's trailing expression with mixed literal/narrow branches) and confirm the
   binary fixed point still holds (`scripts/concat_vow.sh` triple-build + `sha256sum` compare, per
   CLAUDE.md's bootstrap-triple-test).

## 4. Verification surface

This change touches IR lowering/codegen only, not contracts or the C/ESBMC model — ESBMC never
sees Cranelift Phi/Upsilon widths, so there are no new properties for ESBMC to prove and no
`requires`/`ensures` text changes anywhere. The regression fixture
(`tests/run/if_expr_merge_width.vow`) deliberately goes in `tests/run/`, not `tests/verify/`,
specifically *because* `tests/verify/*.vow` fixtures are only ever `verify`d (never `build`t) by
either harness — the issue itself was filed because that gap let this exact bug class hide from
CI once already (see `[vow multi-property CE...]`-style harness-coverage lessons in this repo's
history). No existing `tests/verify/` fixture needs to grow; no `examples/` fixture needs to grow
(examples are curated for documentation, not regression coverage).

## 5. Risk areas

- **Binary fixed point.** `compiler/lower.vow`'s new functions must use the same `BTreeMap`-free,
  deterministic-iteration style already used by `choose_match_result_ty`'s splice (plain
  `while`-loop scans over `Vec<IrInst>` by id, no hashing) so stage A/B/C of the bootstrap triple
  stay byte-identical. Copy the existing splice's control flow exactly rather than inventing a
  new traversal order.
- **Upsilon re-narrowing ordering.** Both compilers' `ExprKind::If`/`EXPR_IF()` lowering appends
  the "mutated variable" Upsilons to `then_upsilon_block`/`else_upsilon_block` *before* the
  result-merge code runs (confirmed by reading both implementations), exactly like the `match`
  case the original splice was built for. `renarrow_if_branch_result` must locate the result
  Upsilon **by instruction id**, never by block-tail position, or it will corrupt a mutated
  variable's Upsilon instead when both are present in the same branch block.
- **`parse → print → parse` idempotency.** Unaffected — this change touches IR lowering only,
  after parsing/printing have already run; no AST or printer changes are planned.
- **`cargo clippy --all -- -D warnings`.** New helper functions must avoid needless clones /
  unused `mut` — `merge_if_result_ty` is a pure value function (cheap `Copy` `Ty` args), and
  `renarrow_if_branch_result` follows the existing splice's ownership pattern (`split_off`/
  `truncate`/`extend` on `ctx.func.blocks[idx].insts`) exactly, so no new lint surface is
  expected.
- **Behavior change for already-"working" cases.** Because `merge_if_result_ty` picks the
  *non-marker* side's type instead of always the `then`-side's type, a few existing
  fixtures/tests that happen to rely on `merge_phi_ty`'s current "then always wins" rule between
  two *non-marker* mismatched types (which should not type-check in the first place, since
  `vow-types` should have already unified both branches' types before lowering) need the full
  suite run in slice 5 to rule out any accidental dependency. This is expected to be a no-op in
  practice since `vow-types` enforces branch-type unification before lowering ever runs; flag it
  as the one risk worth a full-suite pass rather than just a targeted diff.

## 6. Out of scope

- **Do not** factor `renarrow_if_branch_result`/the Rust helper into a function shared with the
  existing `match`-arm splice (lines ~3553-3579 in `vow-ir/src/lower/mod.rs`, ~4533-4588 in
  `compiler/lower.vow`), even though the logic is nearly identical. That would touch
  already-shipped, already-tested code as part of a bug-fix PR — a deepening refactor, not a fix.
  Leave a short follow-up note (e.g. a new low-priority issue) suggesting the extraction once this
  fix has landed and baked.
- **Do not** extend this fix to `while`/`loop`-expression-as-value sites or any other merge point
  beyond `ExprKind::If`/`EXPR_IF()`. The issue is scoped to the if-expression-as-value shape only;
  `match` is already fixed (#976/#979) and `while`/`loop` do not currently produce a
  value-carrying merge in the same way.
- **Do not** add a general "every `tests/verify/*.vow` fixture should also be `build`-tested"
  harness change. That is a real, separate, larger gap the issue's "Why deferred" section
  surfaces, but fixing the harness is out of scope for this bug fix; file it separately if wanted.
- **Do not** reformat or refactor unrelated code in `vow-ir/src/lower/mod.rs` or
  `compiler/lower.vow` while touching these functions (both files are large; resist the urge to
  split them as a drive-by).
