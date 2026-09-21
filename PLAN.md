# Plan: issue #739 — `let (a, b) = ...` tuple destructuring

## 0. Scope decision (read first)

The issue's "Expected" section offers two options: implement destructuring, or retract the
grammar.md example. Investigation surfaced a fact neither option accounts for: tuple **values**
have zero IR-lowering support in either compiler today. `vow-ir/src/lower/mod.rs:3980` is a
catch-all `_ => todo!("IR lowering not implemented for {:?}", expr.kind)`, and `ExprKind::Tuple`
never appears in the preceding 2500-line match — so `let t = (1, 2);` already panics the Rust
compiler at IR-lowering time, independent of the `let`-pattern bug. The self-hosted compiler has
the identical gap: `EXPR_TUPLE` has no case in `lower.vow`'s `lower_expr` and silently falls
through to the default arm, which emits `IOP_CONST_UNIT()` — a silent miscompilation rather than
a panic, but the same underlying absence.

Full first-class tuple values (heap-allocated via `RegionAlloc`/`FieldSet`/`FieldGet`, mirroring
`StructLiteral` lowering at `vow-ir/src/lower/mod.rs:2904-2980`) would require a new IR
representation, codegen work, and — per `docs/adr` / CLAUDE.md's contract-authoring rules —
growth in `vow-verify/src/c_emitter.rs`'s C model on **both** compilers in the same session. That
is not a one-turn change and is explicitly out of scope here.

**Decision: implement `let (a, b) = ...` as a compile-time desugaring, not as first-class tuple
values.** When the `let` pattern is `PatKind::Tuple(pats)` (recursively, for nested tuples) *and*
the initializer is syntactically `ExprKind::Tuple(elems)` of matching arity at every nesting
level, lower each element independently and bind each leaf name — exactly as if the programmer
had written `let a = 1; let b = 2;`. No tuple ever materializes as a runtime value; no new IR
opcode, IR type, or C-model case is introduced. This satisfies CLAUDE.md's crisp rule ("add
surface sugar only when it desugars to today's core semantics with near-zero verifier impact")
and makes the exact `docs/spec/grammar.md:539` example (`let (a, b): (i64, i64) = (1, 2);`) true
as written.

`let (a, b) = f();` (initializer not syntactically a tuple literal — a call, a variable, a field)
is **rejected** with `UnsupportedPattern`, not silently accepted and miscompiled. That is the
seam for a future "first-class tuple values" issue, which should be filed separately (see
Out of scope). This plan will post the reasoning as a `gh issue comment 739` per the operating
contract, since it's a judgment call the issue text didn't anticipate.

## 1. Problem restated

`let (a, b): (i64, i64) = (1, 2)` is documented in `docs/spec/grammar.md` but does not work: the
parser's `parse_let_stmt` hardcodes `PatKind::Ident`/`PAT_IDENT` and never reaches the general
pattern parser that already produces `PatKind::Tuple`/`PAT_TUPLE` for `match` arms. Fixing only
the parser is insufficient and actively harmful (turns a clean parse error into an IR-lowering
panic), because every downstream stage — linear-consumption tracking, IR lowering — either
ignores non-`Ident` `let` patterns or has no representation for tuple values at all. The fix must
route `let` through the pattern parser, validate the pattern/initializer shape at check time, and
teach IR lowering to desugar matching tuple-literal `let`s element-wise, in both compilers.

Incidental finding, noted here so a reviewer doesn't wonder whether it's deliberate: `_` lexes as
a distinct `TokenKind::Underscore` (`vow-syntax/src/lexer.rs:276`), not as an identifier, so
`let _ = expr;` cannot parse today (`expect_ident` rejects it). Routing `let` through the general
pattern parser makes `let _ = expr;` parse as `PatKind::Wildcard` (discard-and-evaluate) as a side
effect. This is a strict improvement with no existing-program risk — no program could have relied
on `_` as a real `let`-bound identifier, since that path never compiled.

## 2. Files to touch

### Rust compiler

| File | What |
|---|---|
| `vow-syntax/src/parser/mod.rs:575-615` (`parse_let_stmt`) | Replace the hardcoded `is_mut`/`expect_ident`/`PatKind::Ident` construction (lines 579-590) with `let pattern = self.parse_pat_inner();`. `parse_single_pat` (`vow-syntax/src/parser/types.rs:146-330`) already handles a leading `mut` per binding (line 156-165) and recursive tuples (204-227), so `let mut x`, `let (mut a, b)`, and nested tuples fall out for free. |
| `vow-syntax/src/printer.rs` | No change needed — `print_stmt`'s `Stmt::Let` arm (234-259) already calls the generic `print_pat` (777-813), which already handles `PatKind::Tuple`. Confirmed by inspection; a round-trip test is still added (slice 2) to pin this down as a regression guard, since it was previously untested for `let`. |
| `vow-types/src/check.rs` | `check_stmt`'s `Stmt::Let` arm (1607-1656): add pattern-shape validation before `bind_pattern` (see slice 3). `bind_pattern` (1900-1917): harden the `PatKind::Tuple` arm's silent arity-mismatch no-op (1906-1914) and the catch-all `_ => {}` (1915) to emit `ErrorCode::UnsupportedPattern` instead of binding nothing. |
| `vow-types/src/linear.rs` | `register_pattern_linear` (165-180): currently `if let PatKind::Ident = &pat.kind` only — extend to recurse into `PatKind::Tuple` against a `Type::Tuple` annotation, mirroring `collect_pattern_binding_names` (487-507) which already recurses generically. |
| `vow-ir/src/lower/mod.rs` | `lower_stmt`'s `Stmt::Let` arm (4698-4802): the unconditional `lower_expr(ctx, init)` at line 4721 must not run when the pattern is `PatKind::Tuple` (it would hit the `todo!()` at 3980). Branch on `pattern.kind` first; add a new recursive desugaring path for `PatKind::Tuple` that lowers each element of `init` (`ExprKind::Tuple`'s `elems`) directly via `lower_expr`, binding `Ident` leaves via `ctx.define` (mirroring 4759-4796) and discarding `Wildcard` leaves after still lowering them (side effects / linear consumption). |
| `docs/spec/grammar.md:536-541` | Expand "### Pattern Destructuring" with the initializer restriction (tuple-literal RHS only, recursively), wildcard support, and a pointer to `UnsupportedPattern` for rejected shapes. |
| `docs/spec/errors.md:260-276` | Extend `UnsupportedPattern`'s "Meaning" text to cover the two new trigger conditions (refutable/unsupported pattern in `let`; non-tuple-literal initializer for a tuple `let` pattern), each with a short example, matching the existing entry's format. |

### Self-hosted compiler (`compiler/`)

| File | What |
|---|---|
| `compiler/parser.vow:632-658` (`parse_let_stmt`) | Replace the hardcoded `is_mut`/`expect_ident`/`arena_add_pat(..., PAT_IDENT(), ...)` (638-655) with a call to `parse_pattern(p)` (already defined at 1112-1191, currently only called from `parse_match_expr` at line 1091; already handles `PAT_TUPLE` at 1176-1187 and, per its `PAT_IDENT` arm, presumably a leading `mut` — confirm during implementation and extend `parse_pattern` if the self-hosted pattern parser doesn't yet consume `mut` the way `vow-syntax`'s does, since parity with the Rust parser's `let mut (a, b)`-per-element behavior is required). |
| `compiler/checker.vow` | `bind_pattern` (1607-1616): `PAT_IDENT`-only today — add a `PAT_TUPLE` branch that recurses per element against the tuple's `Ty` fields, mirroring Rust's `check.rs:1900-1917`. `check_block`'s `STMT_LET` handling (1551-1605, call to `bind_pattern` at 1581): add the same shape/refutability/tuple-literal-initializer validation as the Rust side (slice 3), emitting `"UnsupportedPattern"` (already defined at `compiler/diag.vow:280`). Do **not** touch `validate_arm_pattern` (1715-1769, which explicitly rejects `PAT_TUPLE` for `match` at 1759-1760) or `bind_arm_pattern`'s existing but unreachable `PAT_TUPLE` stub (2117-2127, binds to `CTY_UNIT()`) — match-arm tuple support is a separate, unopened concern. |
| `compiler/lower.vow` | `lower_stmt`'s `STMT_LET` handling (4835-4927, `PAT_IDENT`-only guard at ~4857): add the same desugaring recursion as Rust's `lower.rs` change — lower each `EXPR_TUPLE` element directly, do **not** use `IOP_REGION_ALLOC`/`IOP_FIELD_GET`/`IOP_FIELD_SET` (that's the template for first-class tuple materialization, which is explicitly out of scope). `EXPR_TUPLE` itself still has no construction case in `lower_expr` (falls through to the default `IOP_CONST_UNIT()` arm) — that stays unimplemented; the new code only ever looks *through* an `EXPR_TUPLE` node on the RHS of a matching `PAT_TUPLE`, never lowers it as a standalone value. |
| Self-hosted linear tracking | No separate `linear.vow` pass exists; linearity is checked inline in `checker.vow` via `is_linear_ty`/`is_linear_owner_ty` (27-214). Audit during implementation whether the `STMT_LET` env-definition path already derives per-binding linearity generically (i.e., whether the Rust-side `register_pattern_linear` gap has a self-hosted analogue at all) and extend it in lockstep if so. |
| `compiler/main.vow` (generated) | No hand-edits — the `UnsupportedPattern` / grammar doc strings embedded here (lines ~4224/4558/6720/8568/9402/9736/11906/13755) are generated from `docs/spec/*.md` by `scripts/generate_help.py`. Regenerate after the doc edits (see step below). |

### Regeneration step (both compilers, after doc edits)

```bash
uv run python scripts/generate_help.py
cargo build --release -p vow
scripts/bootstrap.sh --skip-cargo
python3 scripts/check_help_coverage.py   # staleness check, also run by full_test.sh
```

## 3. TDD slices

Each slice is red→green→refactor and independently reviewable. Slices 1-6 are Rust; 7-11 mirror
them in the self-hosted compiler; 12 is the cross-compiler differential fixture. Land Rust slices
before self-hosted ones so each self-hosted slice has a working reference behavior to match.

1. **Parser: tuple pattern in `let`.**
   Test (`vow-syntax/src/parser/mod.rs`, new `#[test]` in the existing `mod tests` at line 749):
   parse `let (a, b): (i64, i64) = (1, 2);` and assert the resulting `Stmt::Let.pattern.kind` is
   `PatKind::Tuple(vec![Ident("a"), Ident("b")])`. Also assert `let mut x = 1;` still parses to
   `PatKind::Ident{is_mut:true}` (regression) and a new case `let (mut a, b) = (1, 2);` parses
   `Tuple([Ident{a,mut:true}, Ident{b,mut:false}])`.
   Code: `parse_let_stmt` change described above.

2. **Printer round-trip regression guard.**
   Test (`vow-syntax/src/printer.rs`, new `#[test]` near 815+): `parse → print → parse` on
   `let (a, b): (i64, i64) = (1, 2);` and a nested case `let (a, (b, c)) = (1, (2, 3));` produces
   an identical AST on the second parse. Expected to pass with **no production change** (see file
   table) — this slice exists purely to pin the invariant now that `let` can carry non-`Ident`
   patterns, before any checker/lowering work lands.

3. **Checker: accept tuple-literal-shaped `let`, reject everything else.**
   Tests (`vow-types/src/check.rs`, new tests near the existing `Stmt::Let` test cluster at
   5666-5760):
   - `let (a, b): (i64, i64) = (1, 2);` → no diagnostics, `a: i64`, `b: i64` in scope.
   - `let (a, b) = (1, 2);` (no annotation) → same, via `default_literal_integer_types`.
   - `let (a, (b, c)) = (1, (2, 3));` → nested, all three bound.
   - `let (a, b) = (1, 2, 3);` (arity mismatch, **no annotation**) → exactly one diagnostic,
     `UnsupportedPattern`, from the new shape validator. This is the only path that can fire:
     `can_context_coerce` (check.rs:1618) only runs inside the `Some(ann)` branch (1613), so an
     unannotated mismatch never reaches it.
   - `let (a, b): (i64, i64) = (1, 2, 3);` (arity mismatch, **with annotation**) → two diagnostics:
     `TypeMismatch` from the existing annotation-vs-initializer check (1618-1629) *and*
     `UnsupportedPattern` from the new validator. Keep this as its own test case, separate from
     the unannotated one above — do not conflate the two arities-mismatch shapes into one
     assertion.
   - `let (a, b) = some_fn_call();` where `some_fn_call() -> (i64, i64)` → `UnsupportedPattern`
     ("tuple destructuring requires a tuple literal initializer").
   - `let Some(x) = opt;` (refutable `EnumVariant` pattern) → `UnsupportedPattern`.
   - `let (a, b) | c = ...;`-style `Or` pattern, and a bare literal pattern `let 5 = x;` →
     `UnsupportedPattern`.
   Code: validation logic in `check_stmt`'s `Stmt::Let` arm, plus the `bind_pattern` hardening
   (silent-no-op arms in 1906-1917 become `emit_error(UnsupportedPattern, ...)`).

4. **Linear checker: tuple-let registers every linear leaf.**
   Test (`vow-types/src/linear.rs`, extend the `Stmt::Let` test cluster at 844-920/1180-1230):
   a `let (a, b): (LinearS, i64) = (mk_linear(), 5);` followed by using `a` twice must produce a
   `LinearTypeViolation` on the second use (proves `a` is tracked at all); using `a` exactly once
   must be clean.
   Code: `register_pattern_linear` recursion into `PatKind::Tuple`.

5. **IR lowering: tuple-literal `let` desugars to independent element bindings.**
   Tests (`vow-ir/src/lower/mod.rs`, near the existing `Stmt::Let`-lowering test cluster at
   7300-7400/8495-8624): lower `let (a, b): (i64, i64) = (1, 2);` inside a function body and
   assert: (a) no `ExprKind::Tuple` ever reaches `lower_expr` (i.e., the `todo!()` path is never
   hit — a passing test here is itself the regression guard for the advisor-flagged risk), (b)
   two independent `ConstI64` instructions are emitted, (c) `ctx.lookup("a")`/`ctx.lookup("b")`
   resolve to the two distinct `InstId`s with the correct types. A second test covers the nested
   case `let (a, (b, c)) = (1, (2, 3));`. A third test covers `let (_, b) = (side_effecting(), 2);`
   — assert `side_effecting()`'s call instruction is still emitted (for effects/linear consumption)
   even though its result is never bound.
   Code: the new `Stmt::Let` branch in `lower_stmt` described in the file table.

6. **End-to-end runtime fixture.**
   New file `tests/run/let_tuple_destructure.vow` with `// TEST: exit 0` (or `stdout`, whichever
   fits — follow the convention of a neighboring `tests/run/*.vow` fixture) exercising
   `let (a, b): (i64, i64) = (1, 2);`, a nested-tuple case, and a `let (_, b) = (...)` wildcard
   case, asserting via `if`/`return` on mismatches, matching the style of
   `compiler/test_assign.vow`. This is the fixture that proves the feature works through the full
   pipeline (parse → check → lower → codegen → run), not just at the unit-test layer.

7. **Self-hosted parser: mirror slice 1.**
   Confirmed by direct inspection: `parse_pattern` (compiler/parser.vow:1112-1191) already
   consumes a leading `mut` per-leaf (`tok_kw_mut()` branch at 1172-1175, `arena_add_pat(...,
   PAT_IDENT(), sid, 1, ...)`), matching `vow-syntax`'s `parse_single_pat`. No extension to
   `parse_pattern` is needed — this slice is purely the `parse_let_stmt` swap. No dedicated
   self-hosted parser unit-test file exists (confirmed: no `compiler/test_parser.vow` analogue).
   Cover this slice's parser behavior indirectly through slice 11's runtime fixture, and directly
   by hand-inspecting `parse_pattern`'s output shape during implementation — do not invent a new
   unit-test harness file for this alone (would be its own unrelated infra change).
   Code: `parse_let_stmt` change in `compiler/parser.vow` per the file table.

8. **Self-hosted checker: mirror slice 3.**
   Add fixtures under `tests/error/` (CI-gated via `scripts/full_test.sh:313-353`
   `run_promoted_error_tests`, which runs each fixture through **both** compilers and diffs the
   structured JSON error via `compare_error`): `tests/error/let_tuple_arity_mismatch.vow`,
   `tests/error/let_tuple_non_literal_init.vow`, `tests/error/let_refutable_pattern.vow`, each
   with a `// TEST: stderr "..."` comment (checked by `tests/run_tests.sh` locally per
   CLAUDE.md's harness note; the CI-gating check is the Rust/self-hosted structural-JSON diff, not
   the stderr string). Each fixture must produce exactly **one** diagnostic in each compiler —
   `compare_error` diffs structured JSON, so a fixture that emits two diagnostics in one compiler
   and one in the other fails for reasons unrelated to this change. Use the unannotated form
   (`let (a, b) = (1, 2, 3);`, no type annotation) for `let_tuple_arity_mismatch.vow` specifically
   so only `UnsupportedPattern` fires (see slice 3's note on why the annotated form is excluded
   here). These fixtures are what force parity: `full_test.sh` fails if only one compiler emits
   `UnsupportedPattern` for a given fixture.
   Code: `bind_pattern` + `STMT_LET` validation in `compiler/checker.vow`.

9. **Self-hosted linear check: mirror slice 4, if applicable.**
   Contingent on the audit result from the file table ("Self-hosted linear tracking" row). If
   self-hosted has an equivalent gap, add a fixture under `tests/error/` (double-consume via
   tuple-destructured linear binding) exercised through both compilers the same way as slice 8.
   If the audit shows self-hosted's model doesn't need a parallel change (e.g., because linearity
   is enforced structurally elsewhere), record that finding as a one-line note in the PR
   description instead of adding a no-op test.

10. **Self-hosted IR lowering: mirror slice 5.**
    No dedicated self-hosted IR-lowering unit-test file exists either (confirmed: no
    `compiler/test_lower.vow`-style harness for `lower_stmt` in isolation). Covered by slice 11's
    runtime fixture at the self-hosted-binary level, same reasoning as slice 7.
    Code: `lower_stmt` change in `compiler/lower.vow`.

11. **Self-hosted end-to-end runtime fixture.**
    New file `compiler/test_tuple_let.vow`, following the existing naming/style convention
    (`compiler/test_assign.vow`, `compiler/test_struct.vow`, etc. — a `main() -> i32` that
    `return`s a distinct nonzero code per failed assertion, `0` on success), covering the same
    cases as slice 6. This is what proves parity at the binary level, distinct from the
    `tests/run/` fixture in slice 6 which primarily protects the Rust compiler's pipeline.

12. **Bootstrap triple test (manual verification, not a new automated test).**
    After slices 1-11 land, run `scripts/bootstrap.sh` end-to-end and the bootstrap triple test
    from CLAUDE.md (`scripts/concat_vow.sh` → stage 0/1/2 → `sha256sum` comparison) to confirm the
    self-hosted-compiler change doesn't perturb the binary fixed point. This is verification, not
    a slice with its own red/green cycle — it either reproduces or it doesn't.

## 4. Verification surface

The desugaring design was chosen specifically to keep this surface near-zero:

- **No new IR opcode, no new `Ty` variant.** Each bound leaf (`a`, `b`, …) keeps exactly the IR
  type it would have had as a standalone `let a = 1;` — `ConstI64`, a struct pointer, etc. There
  is nothing new for `vow-codegen` or `vow-verify/src/c_emitter.rs` to model.
- **No new ESBMC property class.** A `requires`/`ensures` clause referencing `a` or `b` after
  `let (a, b) = (1, 2);` sees ordinary already-verified scalar/pointer bindings — verification
  behaves exactly as if the two `let`s had been written separately. No fixture under
  `tests/verify/` or `tests/verify-fail/` should be needed for the destructuring mechanism itself;
  if the implementer finds a case where `vow verify` treats a desugared binding differently from
  an equivalent hand-written pair of `let`s, that's a bug in this change, not expected surface
  growth.
- **`AstType::Tuple` still has no dedicated IR-type mapping** (`lower_ty_with_linear`,
  `vow-ir/src/lower/mod.rs:554-583`, falls to the catch-all `_ => Ty::Ptr`). This plan does not
  touch that, because it only matters for tuple types used as function parameters/return types or
  struct fields — not for `let` with an initializer that's checked element-wise. Confirm during
  implementation that the `let`-with-annotation path (`check.rs:1613-1653`,
  `lower.rs:4703-4757`) never actually calls `lower_ty_with_linear` on a `Type::Tuple` in a way
  that would surface this; if it does, that's a new finding to fold into this plan or split out,
  not to route around silently.
- **`tests/error/*.vow` fixtures (slice 8) are the real cross-compiler verification gate** for
  this change, via `full_test.sh`'s `compare_error` — both compilers must emit the same
  `UnsupportedPattern` shape for the same rejected input.

## 5. Risk areas

- **The `todo!()` at `vow-ir/src/lower/mod.rs:3980` must never be reachable through this feature.**
  The `Stmt::Let` lowering change must branch on `pattern.kind` *before* the existing unconditional
  `lower_expr(ctx, init)` call (line 4721) — not lower the whole tuple expression and then also
  lower its elements. Slice 5's first assertion (no `ExprKind::Tuple` reaches `lower_expr`) exists
  specifically to catch a regression here.
- **`parse → print → parse` idempotency.** Believed already safe (`print_pat` already handles
  `PatKind::Tuple` generically — see file table), but slice 2 is a hard gate before any checker
  work proceeds, per CLAUDE.md's canonical-form invariant. If slice 2 fails, that's a blocking
  discovery, not a nice-to-have.
- **Silent-failure class in `bind_pattern`.** Prior to this change, `bind_pattern`'s
  `PatKind::Tuple` arity-mismatch case and its `_ => {}` catch-all silently bind nothing rather
  than erroring — currently unreachable dead code (the parser never produces non-`Ident`
  patterns), about to become reachable. Any shape this plan doesn't explicitly enumerate in slice
  3's test list must still hit the hardened error path, not the old silent no-op, or a name like
  `a` in `let (a, b) = (1, 2, 3);` would compile as "undefined identifier" at first *use* instead
  of a clear error at the `let` site.
- **Self-hosted parity is not optional.** Per CLAUDE.md ("Vow Compiler" section) and commit
  a4f01497 ("retract rust-only precedent"), landing only the Rust half is explicitly disallowed.
  Slices 7-11 are not a follow-up — they're required in the same PR.
- **Self-hosted linear-consumption model shape is unconfirmed** (see slice 9) — don't assume it
  mirrors `vow-types/src/linear.rs`'s separate-pass `LinearTracker` structure; audit first.
- **Binary fixed point.** The `lower.vow` change adds new codegen-affecting branches reached via
  `BTreeMap`-free scope lookups (`lctx_define`/`ctx.define` are simple name→value insertions, not
  the `BTreeMap` slot-map machinery in `vow-clif-shim`), so no ordering-nondeterminism risk is
  expected — but slice 12's bootstrap triple test is the actual check, not this assumption.

## 6. Out of scope

- **First-class tuple runtime values.** `ExprKind::Tuple`/`EXPR_TUPLE` construction lowering
  (`RegionAlloc`/`FieldSet`), `let (a, b) = f();` where `f()` returns a tuple, tuples as function
  parameters or return types, tuples as struct fields, and any C-model growth in
  `vow-verify/src/c_emitter.rs`. File as a separate follow-up issue once this PR lands; the
  `todo!()` finding from section 0 is the justification to link in that issue.
- **`match` on tuple patterns.** `compiler/checker.vow`'s `validate_arm_pattern` (1759-1760)
  explicitly rejects `PAT_TUPLE` for match arms today, and the Rust IR lowering for `match`
  (`vow-ir/src/lower/mod.rs:3241+`) assumes an enum-tagged scrutinee throughout — tuple scrutinees
  in `match` are a materially different, larger change than `let` destructuring. Not touched.
- **`let Point { x, y } = p;` struct-pattern destructuring.** Not requested by the issue; would
  actually be *easier* than tuples (structs already have a real `RegionAlloc`/`FieldGet`
  representation), but bundling it here would be exactly the kind of scope creep CLAUDE.md warns
  against ("many small changes beat one large change"). Leave `PatKind::Struct` in `let` position
  producing `UnsupportedPattern`, same as today's de facto behavior.
- **Refutable patterns in `let`** (`Lit`, `EnumVariant`, `Or`) — correctly rejected with
  `UnsupportedPattern`, not supported. Adding `let`-`else` or irrefutability-widening sugar is a
  separate language-design conversation.
- **Formatting/unrelated cleanup** in any touched file beyond the diff needed for this feature.
- **Regenerating `--help`/skill text** is mechanical (`scripts/generate_help.py`) and included in
  the plan only because the doc edit requires it — no other `--help`-affecting change is in scope.
