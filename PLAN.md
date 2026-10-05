# Plan: issue #1504 — trailing assignment without `;` in an if-branch can fail Cranelift verification

## 0. Repro — confirmed empirically, not just inferred from code

Built `target/release/vow` (`cargo build --release -p vow -j4`) and ran it (with
`VOW_CACHE_DIR` pointed at a scratch dir, per the compile-cache staleness note) against:

```vow
module Repro1504

fn bump(x: i64) -> i64 {
    let mut y: i64 = x;
    if x > 0 {
        y = y + 1
    } else {
        y = y - 1;
    }
    y
}

fn main() -> i32 [io] {
    print_i64(bump(5));
    print_str(String::from("\n"));
    print_i64(bump(-5));
    print_str(String::from("\n"));
    0
}
```

Result: `error[CodegenFailed]: function define error: in function 'bump': Compilation error:
Verifier errors`, dumping this Cranelift IR for `bump`:

```
block1:
    v6 = iconst.i64 1
    v7 = iadd.i64 v2, v6
    jump block3(v7, v7)

block2:
    v8 = iconst.i64 1
    v9 = isub.i64 v2, v8
    v10 = iconst.i32 0
    jump block3(v9, v10)

block3(v0: i64, v1: i64):
    return v0
```

`block3`'s second parameter (`v1`) is declared `i64` — inferred from the `then`-branch's
`y = y + 1` tail, which wrongly reports `i64` (see §1) — but `block2` (the `else` path, whose tail
is genuinely `()`) jumps in with `v10 = iconst.i32 0`, the dummy value `ConstUnit` always
materializes. An `i32` argument into a declared-`i64` block parameter is exactly what Cranelift's
own verifier rejects. This is the literal, confirmed mechanism, not a hypothesis — the issue's
"not isolated or reproduced" caveat is resolved: `bump`/`main` above is the repro, and it is also
slice 5's integration fixture (§3) almost verbatim.

Self-hosted (`build/vowc`) was **not** empirically exercised this session — there is no prior
`build/` in this workspace, and bootstrapping it (~5 min) was not spent on a side-path this
planning stage won't implement. Based on the architecture (`vow-clif-shim` uses fixed-width stack
slots, not Cranelift SSA block params — see `CLAUDE.md`'s "vow-clif-shim architecture"), the
self-hosted backend most likely does **not** hit a hard verifier crash on the same mistyped IR —
it probably stores/loads the mismatched-width value silently instead. This means slice 3's direct
IR-shape inspection test (not an end-to-end crash reproduction) is the only reliable self-hosted
regression lock for this bug; the implementation stage must not substitute an end-to-end run for
it on the self-hosted side and conclude "no bug" if nothing crashes.

Corpus check (this session, both to sanity-check the premise and to bound the fix's blast radius):
scanned every `.vow` file under `compiler/`, `stdlib/`, `examples/`, `euler/`, `tests/`,
`benchmarks/` for a bare (non-`;`) assignment directly preceding a block's closing `}` — **zero
hits**. No existing fixture, including the self-hosted compiler's own source, exercises this shape
today. That also means the self-hosted compiler compiling itself has never hit this bug, so a
clean `scripts/bootstrap.sh` run today is not (and never was) evidence against the bug — it simply
never reaches this code shape.

## 1. Problem restated

`Block { stmts, trailing_expr }` (`vow-syntax/src/ast.rs:388`) promotes any semicolon-less final
expression to `trailing_expr` with no check on its kind (`parse_block`,
`vow-syntax/src/parser/mod.rs:520-573`), so `y = y + 1` with no `;` as the last line of a block
becomes that block's value — `ExprKind::Assign` is a real expression variant
(`vow-syntax/src/ast.rs:316`), not a statement-only form. Both type checkers already treat an
`Assign` expression as `Ty::Unit`/`CTY_UNIT()` regardless of the RHS's type
(`vow-types/src/check.rs:3109-3136`, `compiler/checker.vow:3789-3816`), and `if`/`else` branch
unification (`check.rs:2881-2916`, `grammar.md:681`) is already correct on that basis — a program
where one branch's tail is a bare assignment and the other is independently `()` type-checks
cleanly as `Ty::Unit` in both compilers today (confirmed: `vow build` got past type-checking in §0
and failed only at Cranelift codegen). **The bug is a lowering-only divergence from that
already-correct contract.** Both IR lowerers return the *RHS's own instruction* (e.g. an `i64`
`Add`/`Sub` result) as the value of the `Assign` expression, instead of a `Ty::Unit` value, at the
single generic `ExprKind::Assign`/`EXPR_ASSIGN` lowering site
(`vow-ir/src/lower/mod.rs:2224-2332`, shared return at line 2331 regardless of `lhs` kind;
`compiler/lower.vow:3900-3925`, `return rhs_id;` at line 3924 for the identifier-`lhs` case only —
its field/index-`lhs` arms already return their `FieldSet`/`Call` instruction, which is already
`Ty::Unit`, at lines 3869/3898). When that wrong value leaks out through a "value position" —
a block's `trailing_expr`, a bare `else`-operand, a `match` arm body — into the corresponding merge
logic (`merge_if_result_ty`, `mod.rs:1552-1561`; `choose_match_result_ty`, `mod.rs:1510-1537`; and
their self-hosted mirrors at `compiler/lower.vow:2403-2415`), the merge picks a non-`Unit` type for
what the type checker already decided was `Unit`. §0's Cranelift dump shows the exact resulting
defect: a declared-`i64` block parameter fed an `i32` argument (the `ConstUnit` dummy from the
*other*, genuinely-unit branch) — Cranelift's verifier rejects it. The self-hosted backend consumes
the same wrongly-typed IR but, per §0, most likely corrupts data silently instead of crashing.

## 2. Design decision: fix at the value-consuming call sites, not inside `Assign` lowering

Two designs were considered; the plan below is the second, chosen after the first was found to
regress both cross-compiler parity and IR-shape stability:

- **Rejected: edit the shared `new_val`/`rhs_id` return inside the generic
  `ExprKind::Assign`/`EXPR_ASSIGN` arm** to always emit a fresh `Ty::Unit` value. Two problems:
  (a) In Rust, this one return point is shared by all three `lhs` kinds (identifier, field, index),
  so it would add a spurious extra `ConstUnit` instruction after `s.f = e`/`v[i] = e` too — but
  self-hosted's field/index arms *already* return their own `Ty::Unit`-typed `FieldSet`/`Call`
  instruction directly, with no extra instruction. That divergence (Rust emits one more instruction
  than self-hosted for the exact same field/index-assignment program) is exactly the shape of
  defect `#1496` fixed (`concrete-block-region-parity`: instruction-id drift between the two
  compilers for logically identical programs) — unacceptable. (b) `lower_expr`'s `Assign` arm is
  reached from **every** assignment, including the overwhelmingly common statement-position case
  (`x = e;`, via `lower_stmt`'s `Stmt::Expr { expr, .. } => { lower_expr(ctx, expr); }`,
  `mod.rs:5103-5105`, which discards the return value entirely). Changing the generic arm would add
  one extra instruction to *every* assignment statement in every program, in both compilers — a
  large, unnecessary blast radius (shifted instruction IDs corpus-wide, bench/memory bound risk,
  risk to any test asserting exact instruction IDs) for a bug that, per §0's corpus scan, has zero
  existing exposure.

- **Chosen: leave `ExprKind::Assign`/`EXPR_ASSIGN` lowering completely unchanged** (so every
  statement-position assignment in both compilers is byte-for-byte unaffected — zero blast radius,
  zero parity risk), and instead introduce a small wrapper used **only** at the handful of call
  sites where an expression's lowered value is actually read as a merge/value candidate:
  a block's `trailing_expr`, a bare `else`-operand, and a `match` arm's body. The wrapper lowers the
  expression exactly as today (preserving every side effect: the variable-binding update via
  `ctx.assign`/`lctx_assign`, the `FieldSet`/`__vow_vec_set_val` emission) and then, **only if the
  expression's own AST kind is `Assign`**, discards the returned id and substitutes one fresh
  `Ty::Unit` `ConstUnit`. Because the check is on AST shape, not `lhs` kind, this uniformly covers
  identifier/field/index assignment alike, with identical behavior in both compilers — no per-`lhs`
  asymmetry, no parity risk. Because the wrapper runs only at the 3-4 named call sites (not inside
  `lower_stmt`), ordinary assignment statements are provably untouched.

Rust helper (new, placed next to `lower_block_inner`, `vow-ir/src/lower/mod.rs:5109`):

```rust
// `Assign` is always `Ty::Unit` per the type checker (check.rs:3109-3136), independent of its
// RHS's type. `lower_expr` itself must keep returning the RHS id (ordinary statement-position
// callers via `lower_stmt` need it discarded, not retyped), so this wrapper exists only for the
// handful of positions where an expression's lowered value is read as a block/arm's result.
fn lower_value_expr(ctx: &mut LowerCtx, expr: &Expr) -> InstId {
    let val = lower_expr(ctx, expr);
    if matches!(expr.kind, ExprKind::Assign { .. }) {
        ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, expr.span)
    } else {
        val
    }
}
```

Call sites to switch from `lower_expr(ctx, expr)` to `lower_value_expr(ctx, expr)`:
- `lower_block_inner`'s `trailing_expr` branch — `vow-ir/src/lower/mod.rs:5128`.
- If-lowering's bare `else`-operand — `mod.rs:2071` (`else_val = lower_expr(ctx, else_expr)`).
  Likely unreachable in practice today since the grammar's `else` is always followed by `{ }` (so
  `else_branch.kind` is always `ExprKind::Block`, itself routed through the now-fixed
  `lower_block_inner`) — included anyway as free, correct-by-construction insurance rather than
  relying on that grammar fact never changing.
- Match-arm body lowering — `mod.rs:3466` and `mod.rs:3522` (two call sites; confirm both cover all
  arm-lowering paths, e.g. linear vs. non-linear arms, during implementation).

Self-hosted mirror (`compiler/lower.vow`, new `fn lower_value_expr(ctx: LowerCtx, eid: i64) -> i64`
placed next to `lower_block_inner`, `compiler/lower.vow:5183`):

```
fn lower_value_expr(ctx: LowerCtx, eid: i64) -> i64 {
    let a: AstArena = ctx.arena;
    let val: i64 = lower_expr(ctx, eid);
    if expr_tag(a, eid) == EXPR_ASSIGN() {
        let u_args: Vec<i64> = Vec::new();
        return lctx_emit(ctx, IOP_CONST_UNIT(), ITY_UNIT(), u_args, IDATA_NONE(), 0, 0, String::from(""), expr_span(a, eid));
    }
    val
}
```

Call sites to switch:
- `lower_block_inner`'s two trailing-expr return points — `compiler/lower.vow:5214` and `:5217`
  (self-hosted represents a block's tail two ways: as the last `STMT_EXPR` with `has_semi == 0`, or
  via a dedicated `blk_trail` field — both must be covered).
- If-lowering's `else_result` — `compiler/lower.vow:3164`.
- Match-arm body lowering — `compiler/lower.vow:4549` and `:4607`.

With this design, `merge_if_result_ty`/`choose_match_result_ty` (and their self-hosted mirrors)
need **no changes** — they already compute the right merged type once their inputs are correctly
typed.

## 3. Files to touch

**Rust (`vow-ir`):**
- `vow-ir/src/lower/mod.rs` — add `lower_value_expr` (§2); switch the 4 call sites listed in §2.
- `vow-ir/src/lower/mod.rs` test module (near `lower_assignment_updates_identifier_binding`,
  line 8678, and `lower_if_else`, line 8735) — new unit tests (slice 1 below).

**Self-hosted (`compiler/`):**
- `compiler/lower.vow` — add `lower_value_expr` (§2); switch the 4 call sites listed in §2.
- `compiler/tests/test_lower_trailing_assign_unit.vow` (new) — self-hosted IR-shape regression
  test, following the `compiler/tests/test_lower_loop_carried_scope.vow` template (parses a
  fixture, lowers it via `use lower`, inspects `IrInst.ty`/`.op` directly — `IrInst` has a `ty: i64`
  field per `compiler/ir.vow:224-236`, so asserting `inst.ty == ITY_UNIT()` on the merge `Phi` is
  direct). Covers both an `if`-shaped and a `match`-shaped function (see slice 3), matching Rust's
  three-test coverage rather than only the `if` case.
- `tests/fixtures/trailing_assign_unit.vow` (new) — the fixture the above test loads via
  `fs_read`, mirroring `tests/fixtures/loop_carried_scope.vow`'s convention.

**Shared integration fixture:**
- `tests/run/if_trailing_assign_unit.vow` (new) — essentially §0's confirmed repro (`bump`/`main`),
  run through both compilers end-to-end (slice 5 below). This is the fixture the issue body asks
  for, and it is already proven to trigger the bug today.

**Docs (optional, recommended — not mandated, see rationale below):**
- `docs/spec/grammar.md`, immediately after the "last expression of a block (its value)" paragraph
  (lines 534-541) — one or two sentences stating that an assignment's type as an expression is
  `()`, independent of its RHS's type, so a block/if-branch/match-arm ending in a bare assignment is
  `()`-typed. This documents already-true checker behavior; it is not a semantics change (both
  checkers already enforce it), so it is not mandated by the "any change to semantics must update
  docs/spec" rule, but it directly names the fact this bug violated in the implementation, so it's
  worth adding in the same PR. If this addition is made, run `uv run python
  scripts/generate_help.py` and `python3 scripts/check_help_coverage.py` afterward regardless of
  whether the new prose looks unrelated to the `--help`/skill JSON — don't assume a no-op, confirm
  it (per the project's standing rule: any `docs/spec/*.md` edit goes through the regen + coverage
  check, not just edits that look like they touch a tracked builtin/operator/flag).

**No change needed (confirmed during planning, not assumed):**
- `vow-types/src/check.rs`, `compiler/checker.vow` — already correct (`Ty::Unit`/`CTY_UNIT()`).
- `merge_if_result_ty`, `choose_match_result_ty` (`mod.rs:1552-1561`, `:1510-1537`) and self-hosted
  mirrors (`compiler/lower.vow:2403-2415`+) — correct once their inputs are correctly typed; the
  marker-handling logic (`expr_is_coercible_int_marker`, `mod.rs:1451-1489`) already excludes
  `ExprKind::Assign` (no match arm for it, falls through to `_ => false`).
- `vow-verify/src/c_emitter.rs`, `compiler/c_emitter.vow` — neither matches on `ExprKind`/`EXPR_*`
  at all; both consume the already-lowered IR, so they inherit the fix automatically. Per §0's
  corpus scan, no existing fixture under `tests/verify*/` exercises this shape today, so
  `scripts/parity.py c`'s existing corpus is not perturbed.
- `vow-syntax` (parser/AST) — the parser's blanket "any expression may be a tail" behavior is
  correct per the grammar and is not being changed; this is a lowering bug, not a parsing bug.
- `ExprKind::Assign`/`EXPR_ASSIGN` lowering itself — deliberately left untouched; see §2.

## 4. TDD slices

1. **Red — vow-ir unit tests (Rust).** Add, next to `lower_if_else` (`mod.rs:8735`):
   - `lower_block_tail_assign_is_unit_typed`: a function body whose only content is a `trailing_expr`
     of `ExprKind::Assign { lhs: ident_expr("x"), rhs: x + 1 }` (mirror
     `lower_assignment_updates_identifier_binding`'s shape, line 8678, but as the tail, not a
     semicolon statement). Assert the instruction feeding `Return` is `Opcode::ConstUnit` with
     `ty == Ty::Unit` — today it's the `Add` instruction with `Ty::I64`.
   - `lower_if_trailing_assign_branch_merges_as_unit`: reproduce §0's exact shape — **the bare
     `Assign` tail must be in `then_branch` specifically**, with `else_branch` a block with no
     trailing expr (so it's `ConstUnit`/`Ty::Unit` by construction). This orientation is not
     arbitrary: `merge_if_result_ty` (`mod.rs:1552-1561`) always returns `then_ty` unconditionally
     in the non-marker case, completely ignoring `else_ty` — so putting the bare assign in the
     `else_branch` instead would *not* make the merge `Phi`'s type wrong (the wrong-typed else-path
     Upsilon value would just be silently unused, since a `Ty::Unit` `Phi` is never even registered
     as a Cranelift block param — see `ir_ty_to_cranelift`, `cranelift_backend.rs:172-188`, and the
     `is_some()` filter at `cranelift_backend.rs:65`). Assert the merge `Phi`'s `ty == Ty::Unit` —
     today it's wrongly `Ty::I64`.
   - `lower_match_arm_trailing_assign_merges_as_unit`: a `match` with the bare-`Assign` arm body
     **first** (same unconditional-first-arm-wins bias in `choose_match_result_ty`,
     `mod.rs:1510-1537` — it folds from `first_ty` and only ever overwrites `result_ty` inside the
     marker-override branch, so a later arm being correctly `Unit` would not surface the bug if the
     bad arm weren't first) and a later arm body independently `Ty::Unit`. Assert the merge `Phi`'s
     `ty == Ty::Unit`.
   Run `cargo test -p vow-ir lower_block_tail_assign_is_unit_typed
   lower_if_trailing_assign_branch_merges_as_unit lower_match_arm_trailing_assign_merges_as_unit` —
   all three fail (red).

2. **Green — Rust production fix.** Implement `lower_value_expr` and switch the 4 call sites per
   §2. Re-run the three new tests (green). Re-run `cargo test -p vow-ir --no-fail-fast` for the full
   crate (regression check: confirmed in planning that the three existing `ExprKind::Assign`
   unit-test fixtures already in this file — lines 8694, 9913, 10042 — all use
   `has_semicolon: true`, i.e. assignment-as-statement, so none of them touch a switched call site
   and none should need updating). Then rebuild `target/release/vow` and re-run §0's `bump`/`main`
   repro directly **with a fresh `VOW_CACHE_DIR=$(mktemp -d)`** (not the cache dir used in §0's
   pre-fix run — `VerifyCache` only ever persists `FAILED` verdicts, never `PROVEN`, so reusing that
   dir could replay the stale `FAILED` verdict and mask the fix) — it must now build, verify
   (ESBMC, default on), link, and run, printing `6` and `-6`.

3. **Red — self-hosted IR-shape test.** Bootstrap `build/vowc` first if not already done this
   session (`scripts/bootstrap.sh --skip-cargo` if `target/release/vow` from slice 2 is already
   built, else full `scripts/bootstrap.sh`) — slice 3 cannot run without it. Add
   `tests/fixtures/trailing_assign_unit.vow` (two functions: one `if`-shaped, one `match`-shaped,
   both with the bad branch/arm first — same ordering rationale as slice 1) and
   `compiler/tests/test_lower_trailing_assign_unit.vow` (template: §3), checking both functions'
   merge `Phi.ty == ITY_UNIT()`. Run with `build/vowc test
   compiler/tests/test_lower_trailing_assign_unit.vow` — fails (red): `inst.ty` is `ITY_I64()`
   today for both merge `Phi`s, same root cause as slice 1.

4. **Green — self-hosted production fix.** Implement the self-hosted `lower_value_expr` and switch
   its 4 call sites per §2 (include the match-arm call sites, so the self-hosted suite gets the same
   `if`+`match` coverage Rust gets in slice 1 — self-hosted should not ship with narrower regression
   coverage than Rust for the identical fix). Re-run slice 3's test (green). Re-run `build/vowc test
   compiler/` for the full self-hosted suite, then re-bootstrap (`scripts/bootstrap.sh
   --skip-cargo`) to confirm the self-hosted compiler still compiles and verifies itself cleanly
   with the `lower.vow` change, and that the binary fixed point still holds
   (`scripts/concat_vow.sh` triple-build, `compiler_b == compiler_c` via `sha256sum`).

5. **Integration fixture — both compilers (closes the issue).** Commit §0's confirmed repro as
   `tests/run/if_trailing_assign_unit.vow`, with a `// TEST: stdout "6\n-6\n"` directive and a short
   comment naming the regression it guards against (following `tests/run/if_expr_merge_width.vow`'s
   convention). Confirm it now passes the default (verification-on) `vowc build` path on both
   `target/release/vow` and `build/vowc` — this is a plain arithmetic function with no
   `requires`/`ensures` complexity for ESBMC to choke on, so the stronger "ESBMC/Cranelift both
   accept the fixed IR" check applies, not just `--no-verify` codegen.

6. **Full regression sweep.** Run as separate, non-chained commands (per project convention):
   `cargo test --all`, `cargo clippy --all --all-targets -- -D warnings`, `cargo fmt --all --
   --check`, `scripts/bootstrap.sh --skip-cargo --no-cache` (run at the PR's actual final head SHA,
   immediately before any "bootstrap is green" checklist claim, per CLAUDE.md), `build/vowc test
   compiler/`, `scripts/full_test.sh` (covers Section 2c's `scripts/parity.py c` sweep over
   `tests/verify*/`, Sections 4/4c/5, and the differential/region-parity checks),
   `scripts/generate_help.py --check`, `scripts/check_memory_bounds.py --compiler vow` and
   `--compiler vowc` (confirm no `bench/memory` bound is pushed over its threshold — expected
   impact is at most one extra instruction per triggering function, but verify rather than assume).

## 5. Verification surface

- No contracts (`requires`/`ensures`/`invariant`) are touched — this is pure IR-lowering
  correctness, not a change to verifiable semantics. No new ESBMC-facing properties are introduced.
- The new `tests/run/if_trailing_assign_unit.vow` fixture goes through the **default** `vowc build`
  path (verification on), per slice 5 — this directly exercises "does ESBMC/Cranelift accept the
  fixed IR," the stronger version of the regression check the issue asks for.
- `vow-verify`/`compiler/c_emitter.vow` need no direct code change (§3), but slice 6's
  `scripts/parity.py c` run is the concrete check that the two compilers' C emission stays
  byte-identical, not just an assumption from "they don't match on `ExprKind`."
- No new `ErrorCode` is introduced and `docs/spec/errors.md` needs no update — this fix does not
  change which programs are accepted or rejected by either type checker; it only fixes what valid
  IR gets generated for programs already accepted.

## 6. Risk areas

- **Binary fixed point.** `compiler/lower.vow` is part of the self-hosted compiler's own source.
  The edit is a deterministic, unconditional, AST-shape-keyed branch (no `HashMap`/iteration-order
  dependency), so no fixed-point risk is expected — slice 4 treats the triple-build as a hard gate,
  not an assumption. Per §0, the self-hosted compiler's own source never exercises the changed
  shape (zero corpus hits), so compiling `compiler/*.vow` with the fix is expected to produce
  byte-identical output to before the fix, except where the fixture/tests introduced in slices 3/5
  themselves exercise it.
- **IR golden/snapshot drift.** Confirmed in planning that none of the three existing
  `ExprKind::Assign` unit-test fixtures in `vow-ir/src/lower/mod.rs` (lines 8694, 9913, 10042) are
  assignment-as-tail (all `has_semicolon: true`), and the §0 corpus scan found zero existing uses of
  a bare assignment as a block/if/match tail anywhere under `compiler/`, `stdlib/`, `examples/`,
  `euler/`, `tests/`, `benchmarks/`. Because the fix (§2) only changes behavior at the 3-4 named
  call sites and only when the AST node being lowered there is literally `Assign`, no currently
  passing fixture or snapshot should need updating — slice 6's full `cargo test --all` and
  `scripts/full_test.sh` runs are the actual confirmation, not this scan.
- **`cargo clippy --all --all-targets -- -D warnings`.** Test-target lints gate CI, so the new
  vow-ir unit tests must be clippy-clean, not just compiling.
- **Self-hosted codegen (`vow-clif-shim`) stays silently-wrong until the self-hosted `lower.vow` fix
  lands, and may never have produced a crash to notice.** Per §0, slice 3's direct IR-shape
  inspection test is the only reliable self-hosted regression lock for this bug — do not substitute
  an end-to-end run and conclude "no bug" if nothing crashes on the self-hosted path.
- **Match-arm coverage is new scope relative to the issue's literal title** ("if-statement"), but
  is included because it costs nothing extra once the wrapper exists and because leaving it unfixed
  would mean `choose_match_result_ty` keeps the identical latent defect for `match` the issue
  describes for `if`. If this is judged out of scope by review, slice 1/3's third test
  (`lower_match_arm_trailing_assign_merges_as_unit`) and the two match-arm call-site switches are
  the only parts to drop; the if-only fix stands on its own.

## 7. Out of scope

- Any refactor of `merge_if_result_ty`/`merge_phi_ty`/`choose_match_result_ty` beyond what's needed
  — they need no changes at all (§2).
- `while`/`loop` bodies ending in a bare trailing assignment: their body value is always discarded
  (`Ty::Unit` unconditionally), so there's no merge to get wrong the way `if`/`match` have.
- Dedicated field/index-lhs integration fixtures: the wrapper fix (§2) uniformly covers
  identifier/field/index assignment alike (it keys on AST shape, not `lhs` kind), so field/index-as-
  tail is fixed as a side effect, but slice 1/3's unit-test coverage is the identifier case only,
  matching the issue's literal repro. A dedicated field/index fixture is a small, optional
  follow-up, not a blocker.
- No change to `docs/spec/errors.md`, `docs/spec/cli.md`, or `docs/spec/contracts.md` — none of
  this fix's surface touches error codes, CLI flags, or contract semantics.
- No change to `vow-perf`, `vow-linker`, `bench/`, or the mutation-testing harness — unrelated to
  this bug, beyond slice 6's confirmatory `check_memory_bounds.py` run.
