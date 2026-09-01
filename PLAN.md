# Plan: #1097 — self-hosted omits IntCast on narrow `Vec<T>` index reads used as binop operands

## 1. Problem restated

`__vow_vec_get_val` always returns `i64`; both compilers rely on the *consumer* of an
index-read value (not the read itself) to narrow it to the vector's element type. The
Rust lowerer (`vow-ir/src/lower/mod.rs`, `ExprKind::BinaryOp`, lines 1558–1601) computes
an `operand_ty` for a binary op and then unconditionally routes **both** `lhs_id` and
`rhs_id` through `lower_narrow_literal`, which inserts `IntCast[i64->operand_ty]` for
*any* integer-typed operand whose already-lowered type doesn't match `operand_ty` —
literal or not. The self-hosted lowerer's equivalent block (`compiler/lower.vow`,
`EXPR_BINOP`, lines 2312–2336) only re-lowers an operand when `expr_is_const_int(...)`
is true for that specific operand expression — i.e. only literals. A non-literal
integer-typed operand such as `v[j]` (an `EXPR_INDEX`, still carrying its raw `i64`
result from `__vow_vec_get_val`) is left untouched. When the *other* operand already has
a real narrow type (e.g. `s: u32` from a prior `let mut s: u32 = 0;`), `operand_ty`
defaults to that narrow type (`lower.vow:2273`, `let mut operand_ty: i64 = lhs_ty;`), the
binop opcode is selected at that narrow width, and Cranelift receives an
`iadd.i32`/`WrappingAdd` whose second SSA argument is still `i64` — an invalid-CLIF
verifier failure that the Rust compiler never produces because it always closes this gap.
The issue's "inside if/while" framing describes the reporter's confirmed repro, not an
apparent block-position check anywhere in the code: `lower_expr`/`lower_block` are the
same functions for a function's entry block and for `if`/`while` bodies in both
compilers, so `s = s + v[j];` with the `if` removed is *predicted* to fail the same way
— but this is a static-reading prediction, not something anyone has run yet (the issue's
only "no branch" row, `let s: u32 = 1u32 + v[0];`, is a different operand shape: a
literal LHS and a `let`-typed target, neither of which exercises the broken
literal-only gate). Slice 0 below runs the prediction before any code changes, precisely
so a wrong prediction is caught before the fix is scoped around it.

## 2. Files to touch

- **`compiler/lower.vow`** — the only production-code change. `EXPR_BINOP` handling
  inside `lower_expr`, specifically the narrow-operand-normalization block currently at
  lines 2312–2336. Replace the literal-only re-lowering logic with calls to the existing
  `lower_narrow_literal` helper (`lower.vow:2052–2075`), mirroring Rust's
  `mod.rs:1590–1598` exactly. `lower_narrow_literal` already:
  - bails out immediately if `ty` isn't one of the eight narrow integer types
    (lines 2056–2060),
  - defers to the wide-context marker when `ty` is I128/U128 and a wide control-flow
    context is already recorded (lines 2061–2063),
  - re-lowers via `lower_integer_marker_as` when the operand expression is a coercible
    integer marker — literal, negated literal, or a nested binop/block of markers
    (lines 2064–2067; strictly more general than the current hand-rolled
    `expr_is_const_int` + `IOP_CONST_U8`/`IOP_CONST_I32` inlining it replaces), and
  - **falls back to emitting `IntCast[source_ty -> ty]`** for any remaining
    integer-typed, non-marker operand whose lowered type doesn't match `ty`
    (lines 2068–2073) — this is the missing piece for `v[j]`.

  No other file in `compiler/` needs a change: `EXPR_INDEX` (lines 4052–4067),
  `lower_narrow_literal` itself, `EXPR_ASSIGN` (3290–3382), and the `let`-binding path
  (`lower_stmt`, 4759–4853) are all already correct and require no edits.

- **`crates/vow-ir`** — **no change**. Confirmed by reading `mod.rs:1537–1601`: Rust
  already performs the unconditional `lower_narrow_literal` call on both operands
  whenever `operand_ty` is narrow, regardless of literal-ness or block position. This is
  a self-hosted-only parity bug; per "modify both compilers in the same session" in
  `CLAUDE.md`, that rule applies to *language semantic* changes — this fix makes
  self-hosted match Rust's existing, already-correct behavior, not a semantic change to
  either compiler's target language.

- **`docs/spec/*.md`** — **no change**. This is an internal IR-lowering correctness fix.
  It does not add, remove, or alter any syntax, type, builtin, operator, effect, or CLI
  flag — narrow-typed arithmetic over a `Vec<T>` index read already has defined semantics
  (compute at the narrow element type); self-hosted was simply failing to implement them
  in one operand position. Nothing in `grammar.md`/`cli.md`/`contracts.md` describes
  lowering internals, so nothing there goes stale.

- **`tests/run/`** — new fixture(s), see TDD slices below.

## 3. TDD slices

Each slice is red today against `build/vowc` (either a hard `CompileFailed` from the
invalid-CLIF verifier error, or — for the u8 case — the same class of failure) and green
against `target/release/vow` already, so parity is the acceptance criterion baked into
`scripts/full_test.sh`'s `tests/run/` runner (Section 4), which builds+runs a fixture with
*both* compilers, diffs their stdout, and separately checks the `// TEST: stdout "..."`
directive against Rust's output. Existing convention: `tests/run/vec_pop.vow` shows the
header/module-name/print pattern to follow.

For every slice below, when confirming the red state, don't just read the `clif_shim:`
dump — check that `full_test.sh` actually reports `fail`, not `skip`. Section 4's runner
(`full_test.sh:595-598`) skips a fixture outright when either compiler's `build --no-verify`
JSON on stdout is empty; if self-hosted's verifier error goes to stderr with nothing on
stdout, the harness will silently skip rather than fail, and the slice was never really
red. Confirm with `build/vowc build --no-verify <fixture> -o /tmp/out; echo $?` directly
(non-zero exit, and inspect whether any JSON reached stdout) before trusting the harness
run.

0. **Slice 0 — falsify or confirm the block-independence prediction, no new fixture.**
   Before writing any fixture files, hand-run the issue's exact accumulation shape with
   the `if` removed: `let mut s: u32 = 0; let mut j: i64 = 0; s = s + v[j];` at top level
   (entry block, no branch), same `Vec<u32>` setup as the issue's repro. Run it through
   `build/vowc build --no-verify` directly (scratch file, not committed).
   - If it fails with the same CLIF verifier error: confirms §1's diagnosis — the defect
     is in `EXPR_BINOP`'s operand normalization generally, not block-position-dependent —
     proceed to slices 1–5 as planned.
   - If it *compiles*: the static reading missed a mechanism (something about entry-block
     lowering context differs after all). Stop, re-read `compiler/lower.vow`'s
     `EXPR_BINOP` path with fresh attention to anything that could depend on
     `ctx.current_block` or an entry-vs-branch flag, and revise this plan before touching
     production code.

1. **Slice 1 — minimal repro, `if` body, `Vec<u32>`.**
   Add `tests/run/vec_narrow_index_binop_if.vow`, adapted directly from the issue's
   minimal reproducer (`Vec<u32>`, `s: u32` accumulator, `s = s + v[j];` inside
   `if j < 1 { ... }`, `print_u64(s as u64)`), with `// TEST: stdout "7"`.
   - Red: `build/vowc build --no-verify tests/run/vec_narrow_index_binop_if.vow` fails
     with the CLIF verifier error from the issue; Rust succeeds and prints `7`.
   - Green: after the `EXPR_BINOP` fix in `compiler/lower.vow`, self-hosted builds and
     prints `7`, matching Rust.
   - This is the slice that directly closes the issue as filed.

2. **Slice 2 — `while` body, same shape.**
   Add `tests/run/vec_narrow_index_binop_while.vow`: same accumulation but inside a
   `while j < 1 { s = s + v[j]; j = j + 1; }` loop, `// TEST: stdout "7"`. Confirms the
   fix isn't accidentally `if`-specific (it shouldn't be — the fixed code path is shared
   by both — but the issue's own delta table lists this as a separately-confirmed-broken
   case, so it earns its own fixture rather than being inferred from slice 1).

3. **Slice 3 — narrower element/accumulator type, `Vec<u8>`/`u8`.**
   Add `tests/run/vec_narrow_index_binop_u8.vow`: `Vec<u8>`, `s: u8` accumulator, same
   `if`-body shape, expected `200` per the issue's delta table (push value chosen so the
   printed decimal is unambiguous, e.g. push `200u8`, one iteration, no wraparound).
   `// TEST: stdout "200"`. Exercises the `IOP_CONST_U8()`/narrow-cast-to-`u8` path
   specifically (distinct opcode selection from the `u32` case in the same block being
   fixed).
   - These three fixtures collectively cover every "CompileFailed" row in the issue's
     delta table. The "ok" rows (`let x: u32 = v[j]; ...`, no-arithmetic index write,
     no-branch literal add, `u64`/`Vec<u64>`) are deliberately **not** turned into new
     regression fixtures — they already pass today via unrelated, already-correct code
     paths (`let`-binding narrow dispatch, no narrow branch taken at all for `u64` since
     it's not in the eight-type narrow list), and duplicating already-green behavior into
     new fixtures adds maintenance surface without adding coverage.

4. **Slice 4 (refactor step, not a new behavior) — replace the narrow-operand block in
   `compiler/lower.vow:2312–2336`.**
   With slices 1–3 red, make the actual production change: delete the hand-rolled
   `narrow_op`/`narrow_data`/`expr_is_const_int`-gated re-lowering (lines 2318–2335) and
   replace it with:
   ```vow
   lhs_id = lower_narrow_literal(ctx, a, lhs_eid, lhs_id, operand_ty);
   if op != BINOP_SHL() && op != BINOP_SHR() {
       rhs_id = lower_narrow_literal(ctx, a, rhs_eid, rhs_id, operand_ty);
   }
   ```
   inside the existing `if operand_ty == ITY_I8() || ... || operand_ty == ITY_U128()`
   gate (lines 2315–2317), which can stay as-is (`lower_narrow_literal` itself no-ops for
   non-narrow `ty`, so the outer gate is redundant-but-harmless; keeping it minimizes the
   diff and keeps the code visually parallel to Rust's `mod.rs:1590–1598`, which the next
   maintainer will want to compare against). Confirm `narrow_op`/`narrow_data` become
   fully unused and delete them; do not leave dead locals.
   - Rebuild `build/vowc` (`scripts/bootstrap.sh --skip-cargo` — the Rust stage 0 binary
     is untouched, so `--skip-cargo` is correct) and rerun slices 1–3: all three must go
     green with output matching Rust byte-for-byte.

5. **Slice 5 — full suite regression check.**
   Run `scripts/full_test.sh` in full (not just Section 4) to confirm no other
   `tests/run/`, `tests/verify/`, or differential-parity fixture regresses from the wider
   `lower_narrow_literal` usage now reaching more call sites in `EXPR_BINOP` (in
   particular: any existing fixture exercising narrow-literal binops, shift ops with a
   narrow non-shiftee side, or I128/U128 binops, since those all flow through the same
   block and are now dispatched via `lower_narrow_literal` instead of the inlined logic).

## 4. Verification surface

This change touches codegen (self-hosted IR lowering), not contracts, so there is no
`requires`/`ensures` to write or weaken. The relevant correctness property is purely
**IR well-formedness**: every operand of an integer binop instruction must have the
instruction's declared operand width. That property is exactly what Cranelift's own
verifier already checks (`clif_shim: verifier errors ... arg 1 has type i64, expected
i32`), so there is no new ESBMC obligation — `vowc build` (which runs the CLIF verifier
as part of codegen) is the enforcement mechanism, and slices 1–3 above are the fixtures
that exercise it. No `examples/` changes are needed; `examples/` holds illustrative
programs, not regression coverage, and the issue's repro is adequately captured by the
new `tests/run/` fixtures.

No new `vow verify`/ESBMC-facing behavior is introduced or changed by this fix.

## 5. Risk areas

- **Binary fixed point (`compiler/` self-hosting).** `compiler/lower.vow` is part of the
  self-hosted compiler's own source, compiled by itself in the bootstrap triple test
  (`scripts/concat_vow.sh` → stage A → B → C, `sha256sum` compared). This fix changes
  what IR `EXPR_BINOP` emits for a strictly wider set of narrow-operand cases (adding
  `IntCast`s that were previously missing for non-marker operands). For literal/marker
  operands the new path is *not* a no-op rewrite of the old one: `lower_narrow_literal`
  has an early-return (`lower.vow:2061-2063`) that the inlined code being deleted did not
  have — when `ty` is I128/U128 and `lctx_wide_context_ty(ctx, eid) == ty` (a context
  `EXPR_BINOP` itself records at lines 2245–2254 before lowering operands), it returns
  `original` as-is instead of re-lowering via `lower_integer_marker_as`. The old code
  always called `lower_integer_marker_as` for an I128/U128 const-int operand regardless
  of recorded wide context. This should be correct — the operand was already lowered at
  the wide type by the recorded-context path, and re-lowering it would emit a redundant
  duplicate constant — but it is a genuine behavior difference, not "unchanged." If slice
  5's full-suite run regresses an I128/U128 fixture, look at `2061-2063` and the
  `lctx_record_wide_control_flow_context` calls at `2245-2254` first, rather than
  bisecting blind. Because the change is in the compiler's own lowering pass, re-run the
  full triple bootstrap
  (`./target/release/vow --no-verify /tmp/compiler_clif.vow -o /tmp/compiler_a` →
  `/tmp/compiler_a -o /tmp/compiler_b ...` → `/tmp/compiler_b -o /tmp/compiler_c ...` →
  `sha256sum` compare B and C) after the fix lands, not just the targeted fixtures. If
  `compiler/*.vow` itself contains any `narrow_var = narrow_var + vec_narrow[i]`-shaped
  expression that was previously silently miscompiled (unlikely, since bootstrap
  currently succeeds — meaning the pattern is either absent from the compiler's own
  source or was already going through a working path), the fixed lowering could change
  the self-hosted compiler binary's behavior, not just its bit pattern. Watch for this
  during bootstrap re-run rather than assuming it's a no-op.
- **`BTreeMap` / stack-slot layout in `vow-clif-shim`.** Not touched by this fix — the
  change only affects which IR instructions `lower.vow` emits (more `IntCast`s, a
  differently-sourced narrow-constant emission path for literals), not the shim's
  slot-allocation or codegen-ordering logic. No risk expected here, but the shim consumes
  whatever IR is handed to it, so a malformed-but-differently-malformed IR from a
  mis-scoped fix would surface as a *new* shim/Cranelift verifier error rather than
  silence — slices 1–3 are the guard against that.
- **`parse → print → parse` idempotency.** Not implicated — this fix is entirely in
  `lower.vow` (AST → IR), not the parser or printer. No risk.
- **`cargo clippy --all -- -D warnings`.** Not implicated — no Rust source changes in
  this plan (see §2). Nothing to lint.
- **Operand-order asymmetry (explicitly flagged, explicitly out of scope — see §6).**
  `operand_ty` defaults to `lhs_ty` in both compilers (`mod.rs:1588`, `lower.vow:2273`)
  when neither side is a literal. For `v[j] + s` (index read as the *left* operand, `s`
  narrow on the right), `operand_ty` would resolve to `lhs_ty = I64` in **both**
  compilers, so the narrow branch wouldn't fire at all and the op would run at `i64`
  width with one genuinely-narrow argument left un-widened — a *different*, likely
  `#1096`-shaped defect (silent wrong-width computation, not a verifier crash) that
  affects Rust too, and this plan's fix does not touch it. Do not attempt to fix this
  as part of #1097; it needs its own repro, its own decision about whether `operand_ty`
  should consider both operands symmetrically, and — because it would touch Rust's
  `mod.rs:1585–1588` too — its own cross-compiler session. Flag it as a candidate
  follow-up issue in the PR description.

## 6. Out of scope

- **#1096** (shared/both-compilers silent-i64-width bug when a narrow index read feeds
  an operator with no narrow-typed sibling operand present, e.g. `v[j] + v[j]` or
  `v[j] + v[k]`) — a distinct operand shape from this issue's repro, requires a decision
  in *both* compilers, and the issue text explicitly discusses it as related-but-separate.
  Do not bundle it into this PR.
- **#618** (self-hosted `EXPR_IF` mutation-variable Upsilons hardcoded to `ITY_UNIT()`
  instead of the real merge type, `lower.vow:2642` and `:2687`) — confirmed present in
  the same IR dump but not what trips the CLIF verifier for this issue; it's a distinct,
  already-tracked, pre-existing defect in the same function. Leave untouched.
  `EXPR_WHILE`'s Upsilons are already correctly typed (`lower.vow:2770–2823`) and are
  not part of this fix either way.
  If further exploration during the implementation stage cheaply reveals slice 5's full
  suite run doesn't regress here, don't proactively "fix while touching the area" — no
  #1097 fixture depends on Upsilon typing at all.
- **The "index-site" broader fix** (emitting `IntCast` directly inside `EXPR_INDEX`
  lowering for every narrow-element `Vec<T>` read, in both compilers, as the issue text
  itself speculates would resolve both #1096 and #1097 at once) — deliberately rejected
  for this PR. It would change Rust's IR shape for every narrow-`Vec` index read
  (currently `Ty::I64` at the read site, unconditionally, with narrowing deferred to
  consumers), requiring the Rust/self-hosted differential-equivalence corpus to be
  re-baselined and touching both compilers for what is, for #1097 alone, a self-hosted
  parity gap with a one-function fix. Worth considering as the eventual fix for #1096,
  in its own session, not folded into this bug-fix PR.
- **Operand-order symmetry** (`v[j] + s` case) — see §5, tracked as a candidate follow-up
  rather than fixed here.
- **No formatting/refactor cleanup** beyond deleting the two locals
  (`narrow_op`/`narrow_data`) that become dead as a direct consequence of the fix. No
  other reformatting of `EXPR_BINOP` or surrounding code.
