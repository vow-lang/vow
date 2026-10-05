# Plan: issue #1502 — `v[i] = rhs` operand order differs between lowerers

## 1. Problem restated

`vow-ir/src/lower/mod.rs` lowers every `ExprKind::Assign { lhs, rhs }` by computing
`new_val = lower_expr(ctx, rhs)` once, unconditionally, *before* matching on `lhs.kind`
(`vow-ir/src/lower/mod.rs:2243`). For an index assignment `v[i] = rhs` this means the Rust
lowerer emits the RHS instructions first, then `base` (`v`), then `index` (`i`)
(`vow-ir/src/lower/mod.rs:2313-2315`). `compiler/lower.vow`'s `EXPR_INDEX` arm
(`compiler/lower.vow:3873-3898`) instead lowers `base`, then `index`, then `rhs`
(`original = lower_expr(ctx, rhs_eid)` at line 3886, after `recv_id`/`idx_id`). The final
3-argument call to `__vow_vec_set_val` is `[vec_ptr, idx_id, val_id]` in both compilers —
the call's *argument order* already matches — but the *emission order* (and therefore
evaluation order) of the three sub-expressions differs. **This is predicted from reading the lowering code, not confirmed by execution**:
no `target/release/vow` or `build/vowc` binary exists yet in this fresh worktree (checked:
neither `target/` nor `build/vowc` is present), and building either from scratch is
implementation-stage work, not planning-stage work. The prediction: a fixture with an
effectful index (`print_i64(1)`) and an effectful RHS (`print_i64(2)`) evaluated inside
`v[idx_with_effect()] = rhs_with_effect();` would print the two markers in different
relative order under the two compilers today. `__vow_print_i64` itself is confirmed (by
reading `vow-runtime/src/lib.rs:138-141`) to print the bare value with **no trailing
newline** (`print!("{v}")`), unlike `print_str`, so any fixture must thread explicit `\n`
separators through `print_str` the way `tests/run/len_u64_unsigned_compare.vow` already
does — do not assume `print_i64` appends one.

More importantly, the divergence is **not limited to declared effects** (`io`/`read`/
`write`/`panic`/`unsafe`). A plain local-variable mutation inside the RHS is enough to
make evaluation order observable in the *final value*, with no effect annotation at all:
```vow
let mut i: u64 = 0;
let v: Vec<i64> = Vec::new();
v.push(0);
v.push(0);
v[i] = { i = i + 1; 7 };
```
If `index` is evaluated before `rhs` (canonical order), `i` is read as `0` before the
block mutates it, so this writes `v[0] = 7`. If `rhs` is evaluated first (today's Rust
order), the block runs first, `i` becomes `1`, and *then* `index` is read as `1`, writing
`v[1] = 7` instead — a different final `v`, provable by inspecting the lowered IR/C by
hand, independent of any print statement. This means the pre-fix divergence is a genuine
cross-compiler semantic disagreement about what the program computes (and, by extension,
what ESBMC proves about it), not merely an IR-text/cosmetic difference — the issue's "not
a miscompile today" framing is accurate only for the specific corpus it checked
(`compiler/main.vow`, which has no such expressions), not as a general claim.

**Chosen canonical order: left-to-right source order — `base`, then `index`, then `rhs`.**
Rationale: it matches how `v[i] = rhs` reads; it's what `compiler/lower.vow` already does
(so the self-hosted compiler needs no production-code change, only new tests); and it's
consistent with the type checker's own `lhs`-then-`rhs` traversal
(`vow-types/src/check.rs:3110-3111`). Only the Rust lowerer's `ExprKind::Assign` arm for the
`Index` case needs to change.

## 2. Files to touch

**Rust (production):**
- `vow-ir/src/lower/mod.rs` — restructure the `ExprKind::Assign` arm so the `Index` case
  lowers `rhs` *after* `base`/`index`, without changing the `Ident` or `FieldAccess` cases
  (see Slice 1 below for the exact restructuring shape).

**Rust (tests):**
- `vow-ir/src/lower/mod.rs` (test module, near `lower_source_to_module`/`insts_of` at
  line ~6609) — new unit test asserting instruction emission order for `v[i] = rhs`,
  reading the shared fixture below the same way `loops_carry_the_expected_variables`
  reads `tests/fixtures/loop_carried_scope.vow` via `include_str!`.

**Self-hosted (tests only — no production change; `compiler/lower.vow`'s `EXPR_INDEX` arm
is already canonical):**
- `compiler/tests/test_lower_index_assign_eval_order.vow` — new test file, modeled on
  `compiler/tests/test_lower_origin_spans.vow`'s `lower_source`/`function_named`/
  `first_call_with_symbol`/`inst_with_id` helpers, reading the same shared fixture via
  `fs_read` (mirroring `test_lower_loop_carried_scope.vow`'s
  `fs_read(String::from("tests/fixtures/..."))` call), asserting the same instruction-order
  invariant independently in Vow.

**Shared fixture (read by both unit tests above — nothing copied by hand, per the
`loop_carried_scope.vow` precedent from PR #1496):**
- `tests/fixtures/index_assign_eval_order.vow` — one small function using distinguishable
  literal markers (e.g. index `0`, RHS `4242`) so both unit tests can locate the
  `__vow_vec_set_val` call and assert that the instruction feeding `args[1]` (index)
  precedes the instruction feeding `args[2]` (RHS) in emission order.

**Cross-compiler behavioral fixture (the deliverable the issue asks for — pins the fix as
*executable, observable* behavior, not just an internal IR detail):**
- `tests/run/index_assign_eval_order.vow` — new `// TEST: stdout` fixture, run by both
  compilers through the existing `scripts/full_test.sh` Section 4
  (`run_promoted_run_tests`) and `tests/run_tests.sh` harnesses. Must cover **two**
  sub-cases (see Slice 4): an effectful index/RHS pair (observable via `print_i64` order)
  and a pure local-mutation case (observable via final `Vec` contents alone, no effects
  involved) — the second is required because evaluation order is observable without any
  declared effect, not only with one.

**Docs:**
- `docs/spec/grammar.md` — add one short paragraph to the `## Indexing` section (after the
  `v[i] = new_val;` example around line 1126) documenting the evaluation order: `v` and `i`
  are evaluated before `new_val`. This is a semantics clarification with no prior
  documentation (confirmed: `grammar.md` has no existing "evaluation order" language for
  assignment), so it must be added per the spec-is-the-source-of-truth rule.

**Not touched:** `vow-ir/src/region.rs`, `compiler/region.vow`, `vow-verify/src/c_emitter.rs`,
`compiler/c_emitter.vow`, `vow-codegen/src/cranelift_backend.rs`, `vow-clif-shim/src/lib.rs`.
All of these key off the *positional* call arguments (`args[0]`/`args[1]`/`args[2]` =
vec/idx/val), which are unchanged. Only the *order in which those three `InstId`s are
created* changes, and nothing downstream depends on that.

## 3. TDD slices

0. **Write the shared fixture first.**
   `tests/fixtures/index_assign_eval_order.vow`:
   ```vow
   module IndexAssignEvalOrder

   fn order_probe() -> i64 {
       let v: Vec<i64> = Vec::new();
       v.push(0);
       v[0] = 4242;
       v[0]
   }
   ```
   Note: `v` is declared `let`, not `let mut` — grammar.md:643-646 makes a binding that is
   only ever written via `.push`/`v[i] = e` an `UnusedMut` error (confirmed against the
   working example in `tests/run/len_u64_unsigned_compare.vow`, which uses plain `let v`
   for exactly this reason). Both new unit tests below read this one file; nothing is
   copied by hand, matching the `loop_carried_scope.vow` precedent.

1. **Red (Rust unit test): pin the wrong order, see it fail.**
   In `vow-ir/src/lower/mod.rs` test module, add
   `index_assign_evaluates_base_and_index_before_rhs`, reading the fixture via
   `include_str!("../../../tests/fixtures/index_assign_eval_order.vow")` (same relative
   path pattern as `loops_carry_the_expected_variables`'s fixture read):
   - Lower it, find the `CallExtern("__vow_vec_set_val")` instruction via `insts_of`.
   - Assert: the instruction at `call.args[1]` (the index, `ConstI64(0)`) appears *earlier*
     in `insts_of(&func)` than the instruction at `call.args[2]` (the RHS, `ConstI64(4242)`).
   - Run `cargo test -p vow-ir index_assign_evaluates_base_and_index_before_rhs` — this
     **fails** against current `main.rs`'s `ExprKind::Assign` arm, confirming the test
     exercises the real bug.

2. **Green (Rust production fix).**
   Restructure `vow-ir/src/lower/mod.rs:2224-2332`'s `ExprKind::Assign` arm:
   - Keep the context-recording calls (`known_assignment_ast_type`, `known_field_assignment_ty`,
     `known_index_assignment_ty`, `record_wide_control_flow_context`,
     `record_wide_expected_ast_context`) exactly where they are — they only read the AST
     (`rhs`/`lhs` references), they never lower anything, so their position relative to
     `lower_expr` calls doesn't affect evaluation order.
   - Remove the single unconditional `let mut new_val = lower_expr(ctx, rhs);` currently at
     line 2243.
   - In the `Ident` arm: lower `rhs` first, exactly as today (no behavior change — there is
     no base/index to evaluate first for a bare identifier).
   - In the `FieldAccess` arm: lower `rhs` first, exactly as today (**do not** reorder this
     arm — see "Out of scope" below).
   - In the `Index` arm: lower `base` (`vec_ptr`), then `index` (`idx_id`), **then** `rhs`
     (`new_val`), matching `compiler/lower.vow`'s existing order.
   - In the fallback `_ => {}` arm: still lower `rhs` (needed so the arm produces a valid
     `new_val` for the expression's result type), preserving current behavior for any
     lhs kind the type checker doesn't otherwise reject.
   - Add one line of comment at the `Index` arm noting the cross-compiler invariant (why:
     not obvious from local code that this ordering must match `compiler/lower.vow`).
   - Re-run the Slice 1 test — now green. Run the full `vow-ir` test suite
     (`cargo test -p vow-ir`) to confirm no other test asserted the old (buggy) order.

3. **Self-hosted parity test (no production change expected).**
   Add `compiler/tests/test_lower_index_assign_eval_order.vow`, modeled on
   `compiler/tests/test_lower_origin_spans.vow`:
   - `lower_source`, `function_named`, `first_call_with_symbol`, `inst_with_id` helpers
     (copy the pattern, not necessarily the exact code, from `test_lower_origin_spans.vow`).
   - Read the same shared fixture via
     `fs_read(String::from("tests/fixtures/index_assign_eval_order.vow"))`, exactly as
     `test_lower_loop_carried_scope.vow` does for its fixture.
   - Assert the same invariant: the instruction id at `call.args[1]` occurs at an earlier
     index within the function's flattened instruction list than `call.args[2]`.
   - Run `build/vowc test compiler/tests/test_lower_index_assign_eval_order.vow`. Expect
     this to pass **immediately** (self-hosted is already canonical) — this slice is a
     regression lock, not a bug fix. If it unexpectedly fails, STOP: that means
     `compiler/lower.vow` is not what the earlier investigation found, and the plan's
     "self-hosted needs no change" premise must be re-examined before proceeding.

4. **Cross-compiler behavioral fixture — the deliverable the issue asks for.**
   Add `tests/run/index_assign_eval_order.vow` covering **both** the effectful case and
   the pure-mutation case from Section 1, so the fixture pins real program behavior, not
   just print order:
   ```vow
   // TEST: stdout "1\n2\n42\npure=7\n"
   module IndexAssignEvalOrderRun

   fn idx_with_effect() -> u64 [io] {
       print_i64(1);
       print_str(String::from("\n"));
       0
   }

   fn rhs_with_effect() -> i64 [io] {
       print_i64(2);
       print_str(String::from("\n"));
       42
   }

   fn main() -> i32 [io] {
       let v: Vec<i64> = Vec::new();
       v.push(0);
       v[idx_with_effect()] = rhs_with_effect();
       print_i64(v[0]);
       print_str(String::from("\n"));

       let mut i: u64 = 0;
       let pv: Vec<i64> = Vec::new();
       pv.push(0);
       pv.push(0);
       pv[i] = { i = i + 1; 7 };
       print_str(String::from("pure="));
       print_i64(pv[0]);
       print_str(String::from("\n"));
       0
   }
   ```
   (The implementation stage must build this and run it by hand to pin the exact expected
   `// TEST: stdout` string — the sketch above is the *canonical*-order expectation,
   derived from the Section 1 reasoning, not yet execution-verified. `print_i64` adds no
   newline — confirmed from `vow-runtime/src/lib.rs:138-141` — so every line break above
   is an explicit `print_str("\n")`, matching the `len_u64_unsigned_compare.vow`
   convention.)
   - Before Slice 2's fix: predicted Rust output reverses the first two markers (`2\n1\n...`)
     and reports `pure=0` (today's Rust order mutates `i` before reading it as the index);
     self-hosted is predicted to already match the expected string. Build both binaries
     under `$TMPDIR` (not a hardcoded `/tmp/...` path) and run this fixture through each
     before touching `vow-ir/src/lower/mod.rs`, to empirically confirm the diagnosis and
     lock the exact expected string from a real run, not from this plan's prediction.
   - After Slice 2: both compilers must produce the same, now-canonical string. Run via
     `VOW_FULL_TEST_PROMOTED_ONLY=1 scripts/full_test.sh` (fast path, exercises
     `run_promoted_run_tests` over `tests/run/*.vow` for both compilers) to confirm it's
     wired into the existing harness with no extra plumbing.
   - Grepped during planning: `grep -rn known-divergence tests/` and
     `grep -rn known-cex-divergence tests/` found no existing fixture marked divergent for
     index-assign order — there is nothing to un-mark as part of this fix. Implementation
     should re-grep before landing in case something changed since planning.

5. **Confirm the issue's literal acceptance criterion: byte-diffable IR dumps.**
   After Slice 2 lands, reproduce `scripts/full_test.sh` Section 0b's command pair:
   ```bash
   "$RUST" build --no-verify --dump-ir compiler/main.vow > "$TMPDIR/rust.ir"
   build/vowc build --no-verify --dump-ir compiler/main.vow > "$TMPDIR/self.ir"
   diff "$TMPDIR/rust.ir" "$TMPDIR/self.ir"
   ```
   Confirm every remaining diff line is one of the two **other** pre-existing, out-of-scope
   divergences already on record (the `Eq[i64]` vs `Eq[Bool]` typing difference, and the
   `FieldAccess`/`FieldSet` evaluation-order difference this plan deliberately defers — see
   Section 6). Any diff line touching `__vow_vec_set_val`/`ConstI64` around an index
   assignment means the fix is incomplete and Slice 2 must be revisited. This is a stronger
   check than Section 0b's own block-placement-only comparison and directly verifies the
   issue's stated goal ("both compilers emit the same operand order").

6. **Docs.**
   Add the evaluation-order paragraph to `docs/spec/grammar.md`'s `## Indexing` section
   (after the `v[i] = new_val;` example, i.e. after line 1126). State that `v` and `i` are
   evaluated before `new_val`, left to right — and that this is observable whenever any of
   the three sub-expressions has a side effect, **including** a local-variable mutation
   that a sibling sub-expression later reads (as in the Section 1 pure-mutation example),
   not only a declared `[io]`/`[read]`/`[write]`/`[panic]`/`[unsafe]` effect. Do **not**
   write "matters only with a declared effect" — that claim is false (Section 1). Run
   `scripts/check_help_coverage.py` (via `full_test.sh`, already gates this) — no `--help`
   text changes are expected since this isn't a new flag/builtin, just confirm it still
   passes.

7. **Full gate.**
   - `cargo build --release --all`
   - `cargo test --all` (separately from clippy/fmt, per quality-gate discipline)
   - `cargo clippy --all --all-targets -- -D warnings`
   - `cargo fmt --all -- --check`
   - `scripts/bootstrap.sh --skip-cargo` (rebuild `build/vowc`; confirms the self-hosted
     compiler — unchanged in production code — still builds/verifies cleanly, and picks
     up the new `compiler/tests/` file)
   - `build/vowc test compiler/` (runs all self-hosted unit tests, including the new one)
   - `scripts/full_test.sh` (full gate; re-run Section 0b concrete-block-region-parity —
     expect it to still show the **pre-existing** `run_test%1216`/`%1241` mismatch per
     memory, unrelated to this change; do not try to fix that here)
   - `python3 scripts/generate_operations.py --check` (no catalogue changes expected, just
     confirm no drift was introduced)

## 4. Verification surface

This change touches lowering order, not contracts, codegen semantics, or the C model
directly:
- No `requires`/`ensures`/`invariant` clauses are added, removed, or weakened.
- `vow-verify/src/c_emitter.rs` / `compiler/c_emitter.vow` emit C from the *same* 3
  positional args (`args[0]`/`args[1]`/`args[2]`) regardless of emission order, so the C
  model for `__vow_vec_set_val` (bounds assert + `v.data[idx] = val`) is unaffected — no
  `scripts/parity.py c` fixture changes needed beyond what Slice 4's new `tests/run/`
  fixture already exercises (Section 2c of `full_test.sh` runs `parity.py c` over every
  `tests/verify*/` fixture; this new fixture lives under `tests/run/`, not
  `tests/verify*/`, so it is not itself a parity-C fixture — no action needed there unless
  review later decides a `tests/verify/index_assign_eval_order.vow` contract variant is
  also wanted, which is out of scope per Slice 4's scope).
- ESBMC does not need new proof obligations for the common, side-effect-free case
  (`v[0] = 4242`), where evaluation order is irrelevant to the verification conditions.
  But per Section 1, order **is** observable without any declared effect — any program
  whose index or RHS sub-expression mutates a variable a sibling sub-expression reads
  (e.g. the `v[i] = { i = i + 1; 7 }` pattern) has its IR, and therefore the C ESBMC
  consumes, differ in *value* between the two compilers today. This means a correct
  contract over such a program's final state could verify (`PROVEN`) under one compiler
  and fail under the other pre-fix — a real cross-compiler soundness-relevant divergence,
  not just a display/text artifact. The fix removes this divergence; no contract needs to
  change, and no new `tests/verify*/` fixture is required for #1502 itself, but the
  implementation stage should keep this reasoning in mind if it later considers whether a
  `tests/verify/` contract fixture over the pure-mutation pattern would add value (judged
  out of scope here — see Section 6).
- No new fixtures under `tests/verify*/` are required to close #1502.
  `tests/run/index_assign_eval_order.vow` (Slice 4) is the fixture surface for this issue.

## 5. Risk areas

- **Binary fixed point (`scripts/bootstrap.sh`):** zero risk — no `compiler/*.vow`
  production file changes, only a new `compiler/tests/*.vow` file (excluded from the
  bootstrap triple-test's `scripts/concat_vow.sh`, which only concatenates non-test
  modules used by the compiler itself). Confirm `compiler/tests/` is indeed excluded by
  checking `scripts/concat_vow.sh`'s file list before relying on this.
- **`parse → print → parse` idempotency:** not implicated — no syntax or printer changes.
- **`cargo clippy --all --all-targets -- -D warnings`:** the restructured `match` arm must
  still be exhaustive and must not introduce an unused-`mut`/unused-variable warning on
  `new_val` now that it's declared later and per-arm; watch for a clippy lint on
  "variable could be declared with more restricted scope" or similar if any instance keeps
  a stale `let mut new_val` declared too early.
- **Region/rodata analysis (`vow-ir/src/region.rs`, `compiler/region.vow`):** these index
  into `inst.args` positionally (`args[0]`, `args[2]`), not by relying on any relationship
  between `InstId` numeric values and emission order. Re-ordering emission only shifts
  which numeric `InstId` each sub-expression gets, never which *argument slot* it occupies
  in the call — so `for_each_extern_store_edge`, `stored_value_may_be_rodata`, and the
  self-hosted `region.vow` analogues keep working unchanged. Still worth re-running the
  targeted region/rodata test suites (`cargo test -p vow-ir region`) as a sanity check,
  since this reasoning, while verified by reading the code, has not been exercised by
  execution yet in this plan.
- **Existing snapshot/IR-text tests that hardcode the *old* (buggy) order:** grep
  `vow-ir/src/lower/mod.rs`'s test module and `compiler/tests/*.vow` for any other test
  asserting instruction order or exact `InstId` numbers around an `Index`-assignment
  lowering before writing Slice 1/3, in case one already pins the old order and needs
  updating rather than leaving untouched (the search done during planning found no such
  test, but the implementation stage should re-confirm with a fresh grep for
  `"__vow_vec_set_val"` across both test suites before adding new tests, since a stale
  memory of "no such test exists" is exactly the kind of claim that needs re-verifying
  against current code).
- **Codecov patch gate:** the restructured Rust `match` arm re-indents/moves existing
  lines; per project history (`codecov/patch` gate, 95% threshold), moved-but-unchanged
  lines can count as "new" and need coverage. Slice 1/2's new unit test should cover the
  `Index` arm's new position directly; the untouched `Ident`/`FieldAccess` arms are only
  touched by the minimal structural change (moving `new_val` declaration into each arm),
  so make sure existing tests already covering those arms still execute them post-refactor
  (they should, since behavior there is unchanged — just confirm via `cargo test -p vow-ir`
  that no existing `Ident`/`FieldAccess` assignment test regresses).

## 6. Out of scope

- **`s.f = rhs` (`FieldAccess` assignment) evaluation order.** Rust's current lowerer also
  evaluates `rhs` before `base` for `s.f = rhs` (same root cause: the single
  `new_val = lower_expr(ctx, rhs)` above the match), and `compiler/lower.vow`'s
  `EXPR_FIELD_SET` arm is already canonical (`base` before `rhs`) there too — identical
  shape to the `Index` bug. This plan deliberately does **not** fix it: issue #1502's
  title and body are scoped to `v[i] = rhs` / `__vow_vec_set_val` only. Fixing both in one
  PR would bundle two independent (if structurally similar) evaluation-order bugs into one
  change, against "many small changes beat one large change." **Recommendation:** file a
  follow-up issue for `s.f = rhs` evaluation order once #1502 lands, referencing this plan's
  Slice 2 restructuring as the template (the `FieldAccess` arm would get the same
  base-before-rhs treatment the `Index` arm gets here).
- **The pre-existing `compiler/concrete-block-region-parity` mismatch** (`run_test%1216` vs
  `%1241`, tracked in memory as pre-existing on clean `origin/main`) — unrelated to operand
  order, not touched.
- **No refactor of the shared `ExprKind::Assign` context-recording helpers**
  (`known_assignment_ast_type`, `record_wide_control_flow_context`, etc.) beyond what's
  strictly needed to relocate the `lower_expr(ctx, rhs)` call. No renaming, no
  consolidation of the `Ident`/`FieldAccess`/`Index` arms into a shared helper function,
  even though their post-fix shape (lower lhs parts, then rhs, then narrow, then emit) is
  now structurally similar across all three arms — that consolidation is a legitimate
  future deepening but is not required to close this issue and would make the diff harder
  to review and bisect.
- **No change to `vow-verify`/`c_emitter` bounds-check emission order** (`emit_bounds_assert`
  is called before the store in both emitters today, independent of lowering order) — not
  implicated, not touched.
- **No new CLI flag, builtin, or contract syntax** — this is a pure evaluation-order bug
  fix plus one documentation paragraph; no other `docs/spec/*.md` file needs updates
  beyond `grammar.md`.
