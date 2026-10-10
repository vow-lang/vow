# Plan: invariant-based induction for the native verifier (#1422)

## Goal
When bounded unwinding ends `unknown` because of the unwinding assertion, try to prove the function
inductively from the user's loop `invariant:` clauses (entry, preservation, exit), so a loop with an
unbounded trip count is `proven`. Induction only ever adds `proven`; `failed` and every existing
verdict, counterexample and warning stay the product of bounded unwinding. Covered by an ADR
addendum.

Scope: epic #1398 native verifier only (`compiler/vc_*.vow`). Per CLAUDE.md "Scoped exception: the
native verifier" there is **no Rust twin**, no `c_emitter` change and no C-parity impact. Blockers
#1418 (unwinding) and #1400 (ADR) are closed.

## Assumptions
(no operator available; decisions taken, to be overridable in review)
- **Order: induction runs only after bounded unwinding ends in an unwinding-unknown** (open at the
  largest bound, or over the size budget), not before the rounds. Reason: every existing `proven`
  / `failed` result, counterexample and `ArithOverflowReachable` set stays byte-identical; the
  change is strictly `unknown -> proven`. Alternative considered: induction first (cheaper for
  unbounded loops, but it replaces finite-loop warnings found in deeper iterations). Cost accepted:
  unbounded loops pay rounds 2..64 first; `--timeout` stays shared.
- **Induction never maps to `failed`.** Any non-`unsat` hard claim in the induction query means
  "not proved inductively"; the result stays the BMC `unknown`. Matches ADR Rule 1 ("a step-case
  failure is `unknown`, never `failed`"). A *wrong* invariant is `failed` because bounded
  unwinding checks the invariant as a claim on every header visit and finds a real run.
- **Eligibility:** every loop of the (loop-closed) function has >= 1 `invariant` in its *invariant
  region* (below), no header Phi is `ITY_PTR`, and no `VowInv` lies outside the invariant region
  of its innermost loop. Otherwise BMC only, as today. Keeps `unknown/loop_infinite.vow` and `unknown/loop_symbolic_bound.vow` `unknown`
  (invariant-free loops are never cut with an implicit `true`).
- **Soft (checked-arithmetic) claims: `sat` is ignored in the induction query** (the havoc state may
  be unreachable). The warning sites reported with an inductive proof are the ones collected in the
  last completed BMC round (reachable within that bound). Documented as "reachable within the
  unrolled prefix".
- **Partial correctness.** A proof means every *terminating* run satisfies the contract, so a
  non-terminating loop with an invariant (`while true vow { invariant: true }`) is `proven`
  (vacuous exit). ESBMC/BMC give `unknown` there. This is a deliberate verdict change, recorded in
  the ADR and pinned by a fixture; maintainers may veto in review.
- **`&&`/`||` in an invariant lowers to branches** (checked: `lower_expr`,
  `compiler/lower.vow:2927-3010`, emits `Branch` + rhs/short/merge blocks + Phi for
  `BINOP_AND/OR`; `lower_invariant_clauses` (`lower.vow:5792-5818`) calls `lower_expr` for the
  clause). So `invariant: i >= 0 && i <= n` puts its `VowInv` in a merge block after the header,
  and the cut must clone an *invariant region* (a small DAG of blocks), not a header-block prefix.
- Follow-up names: automatic k-induction (base/forward/step) is the next epic child (P3 "automatic
  k-induction"); #1419/#1420/#1423 are perf/harness/Vec issues, not this feature.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `compiler/vc_induct.vow` [new] | loop-cut pass: loop-closed IR -> acyclic IR with havoc'd headers, `eligible` predicate | n/a |
| `compiler/vc_ops.vow` | add `VC_INTERNAL_HAVOC` / `VC_INTERNAL_CUT_END` markers + predicates | 100-109 (beside `VC_INTERNAL_UNWIND_SINK`) |
| `compiler/vc_cfg.vow` | `vc_reachable_unreachable_detail` must accept the cut-end `Unreachable` | 32-43 |
| `compiler/vc_exec.vow` | `vc_walk_inst`: define a havoc marker as a fresh `declare-const`; header comment lines 34-39 ("assumed nowhere") | 308-347, 34-39 |
| `compiler/vc_native.vow` | `vc_verify_function`: induction attempt at both unwinding-unknown exits; header comment | 117-146, 14-29 |
| `compiler/vc_loops.vow` | reuse `vc_loop_analysis`, `vc_chain_*`, `vc_def_table`, `vc_clone_inst`, `vc_function_shell` | 152, 228-246, 255, 419, 461 |
| `compiler/ir_dominance.vow` | reuse `dom_successors`, `dom_block_id_table`, `dom_dominates`, `dom_max_inst_id`, `dom_fill`, `dom_is_terminal` | 14, 56, 274, 278 |
| `compiler/lower.vow` | read-only check: `lower_invariant_clauses` call sites at 3538, 3698, 3884 | confirm `VowInv` sits in the header after the Phis, before the condition |
| `scripts/concat_vow.sh` | add `vc_induct` to `FILES` after `vc_unroll`, before `c_emitter` | line 23 |
| `compiler/tests/test_vc_induct.vow` [new] | unit tests of the cut pass | model on `compiler/tests/test_vc_unroll.vow:19-217` |
| `compiler/tests/test_vc_exec.vow` | havoc marker executor test | append |
| `tests/verify-native/{pass,fail,unknown}/` | real-solver fixtures | see Testing |
| `docs/adr/2026-10-08-1422-native-verification-semantics.md` | addendum (#1422) | after "Addendum (#1418)" |
| `docs/spec/cli.md` | native "Loops" bullet | line 69 |
| `compiler/main.vow`, `vow/src/skill.rs` (+ whatever else `scripts/generate_help.py` emits) | regenerated embedded cli.md text | via script, not by hand |

## Steps

### 1. Add the two internal markers
- **File**: `compiler/vc_ops.vow` (after line 109)
- **Change**: `VC_INTERNAL_HAVOC() = "__vc_havoc"`, `vc_is_havoc(inst)` = `IOP_CALL` + `IDATA_CALL_EXTERN` + `ds`; `VC_INTERNAL_CUT_END() = "__vc_cut_end"`, `vc_is_cut_end(inst)` = `IOP_UNREACHABLE` + `ds`. Same shape as `vc_is_unwrap_abort` / `vc_is_unwind_sink` (lines 93-108).
- **File**: `compiler/vc_cfg.vow:32-43`: add `&& !vc_is_cut_end(last)` to the reachable-`Unreachable` rejection.

### 2. Loop-cut pass `vc_induct_cut(f) -> VcInduct { f, status }` (new module)
Input is the output of `vc_close_loops` (what `vc_verify_function` already holds as `closed`).
Module header `use ir; use ir_dominance; use vc_ops; use vc_loops`. Steps, per loop `j` with header
block `h = li.headers[j]` processed in `li.headers` order (determinism: no HashMap, no iteration
over unordered containers):
- **Invariant region of loop `j`.** The lowerer emits, in the header: Phis, then per clause the
  clause expression followed by its `VowInv`, then the loop condition. A clause with `&&`/`||`
  spreads over several blocks (rhs, short, merge). Let `blk_last` be the block holding the last
  `VowInv` whose innermost loop is `j` (the blocks holding `j`'s `VowInv`s must form a dominator
  chain ending in `blk_last`, else NA). The region `R_j` is `h` plus every block found by a
  backward walk from `blk_last` over predecessors that does not expand `h` and stays inside the
  loop. `blk_last` is cloned only up to and including that last `VowInv`; every other block of
  `R_j` is cloned whole. A block of `R_j` other than `blk_last` with a successor outside `R_j`
  makes the loop NA. For a plain comma-separated invariant `R_j = {h}` and this is a header prefix.
- **Applicable?** `vc_loop_analysis` not irreducible, `n_loops > 0`, entry not a header; every
  loop has an invariant region with >= 1 `IOP_VOW_INV`; the number of `VOW_INV` in all regions
  equals the number in the whole function (none stray); no header Phi of `ITY_PTR`; no block with
  both successors equal to a header. Else `status = VC_INDUCT_NA` and the caller keeps BMC only.
  Export `vc_induction_eligible(f)` doing the same checks without building the result.
- **New ids**: instruction ids from `dom_max_inst_id(f) + 1`, block ids from `max(block.id) + 1`
  (same scheme as `vc_close_values`, `vc_loops.vow:472-528`). New blocks are appended after the
  originals, `h_entry` then `h_back` per loop, so block 0 stays the entry.
- **`h_entry`** (entry check): a clone of `R_j` with fresh block ids and fresh instruction ids
  (including its merge Phis and the Upsilons feeding them, remapped consistently). The cloned
  header carries fresh Phis `p_i` (one per non-Unit header Phi) fed by the retargeted preheader
  Upsilons; header Phis are remapped to `p_i`; operands defined outside `R_j` are unchanged;
  `ostart/olen` preserved via `vc_clone_inst` so claims keep source spans. The cloned `blk_last`
  ends, right after the last `VowInv`, in `Jump h` (the original header = havoc copy).
  Invariants stay `VOW_INV` (claims).
- **`h_back`** (preservation check): the same clone with Phis `q_i` and fresh ids, whose cloned
  `blk_last` ends in an `Unreachable` with `ds = VC_INTERNAL_CUT_END()` (no claim, no
  assumption). Invariants stay claims. Cloned blocks are appended in `R_j`'s block-index order for
  determinism.
- **Havoc copy = the original `R_j` and the loop (original ids)**: each non-Unit header Phi becomes
  an `IOP_CALL` havoc marker with the Phi's id and type; each Unit Phi becomes `CONST_UNIT` with
  its id; every `VOW_INV` of `R_j` becomes `VOW_REQ` (the executor already assumes `VOW_REQ` via
  `vc_assume`). Condition, branch and body are untouched.
- **Edges**: any branch/jump target `== h` from a back-edge source (`dom_dominates(li.nums, nb, h,
  src)`) becomes `h_back`; from any other predecessor becomes `h_entry`. Compare block ids as
  `vc_clone_at` does (`vc_unroll.vow:236-242`).
- **Upsilons**: one targeting header Phi `i` of loop `j` is retargeted (`dv`) to `p_i` when its block
  is outside the loop, to `q_i` when dominated by `h`; Unit-typed ones are dropped.
- **Result is acyclic**; finish by running `vc_unrolled_detail(g, vc_ty_table(g))` in the caller and
  treat a non-empty detail as "induction not applicable", never as an error (BMC verdict stands).
- Size is linear: one copy of each loop plus two prefix clones; no `VC_UNWIND_INST_CAP` needed.
- **Reuses**: `vc_loop_analysis` (`vc_loops.vow:152`), `vc_function_shell` (`:461`),
  `vc_clone_inst` (`:419`), `vc_replace_arg` (`:435`), `ir_inst_new` (`ir.vow:322`),
  `ir_block_new` (`ir.vow:356`).
- Keep functions small (CLAUDE.md "small files, smaller functions"): split into
  `vc_induct_region` (compute `R_j`/`blk_last`), `vc_induct_eligibility`,
  `vc_induct_clone_region`, `vc_induct_rebuild_block`. If the module passes ~400 lines, move the
  region walk to its own `vc_induct_region.vow` (and register it in `concat_vow.sh`).

### 3. Executor: define a havoc marker
- **File**: `compiler/vc_exec.vow:308-347` (`vc_walk_inst`), new arm before the final `vc_define`
  arm: `else if vc_is_havoc(inst)` -> `vc_script_declare_const(cs.header, "h<id>", vc_ty_sort(inst.ty))`
  then `vc_define(w, inst, vc_tm_var(ar, "h<id>"))`. Not pushed to `cs.model_names` (it is not an
  input; induction never builds counterexamples). The cut-end `Unreachable` already falls through
  (`op != IOP_UNREACHABLE()`), so no arm is needed. Update the header comment lines 34-39: an
  `invariant` is assumed only in the havoc copy of an induction query.
- Confirm `vc_ty_sort(ITY_BOOL())` returns the Bool sort (`vc_int.vow:33`) so Bool flags can be havoc'd.

### 4. Verdict mapping in the native driver
- **File**: `compiler/vc_native.vow:117-146`.
- Keep `vc_run_round` unchanged: with a throwaway `sites` vec it already ignores soft `sat`, ends on
  any other non-`unsat`, and has no unwind claim in a cut function, so
  `!r.ended && !r.open` == "proved inductively".
- New helper `vc_induction_proves(f, closed, started, timeout_ms, scratch) -> bool [io, read, write]`:
  eligibility, `vc_induct_cut`, shape check, `vc_claims`, `vc_run_round`.
- Call it at the two unwinding-unknown exits (size budget `TOO_LARGE`, and `round + 1 ==
  bounds.len()` with `open`). On `true` return `VcOutcome { result: vc_proven_result(), sites:
  last_sites }` where `last_sites` are the sites of the last completed round (today they are
  discarded for open rounds, so carry them out of the loop). On `false` return the same
  `unwinding assertion:` unknown as today (the Section 4g assertions depend on that prefix).
- Update the file header comment (lines 14-29).

### 5. Register the module
- `scripts/concat_vow.sh:23`: insert `vc_induct` after `vc_unroll`. Module loading for `build/vowc`
  is DFS via `use`, so `vc_native.vow` gets `use vc_induct`.

### 6. Fixtures (real Bitwuzla; Section 4g of `scripts/full_test.sh`, lines 1421-1565)
See Testing.

### 7. Docs
- ADR: besides the new addendum, qualify the #1418 addendum bullet "An unwinding claim that is
  still `sat` at the largest bound is `unknown`, never `proven`" with "unless invariant induction
  (#1422) closes it", and the bullet on `invariant` "is a claim ... and is not assumed" with a
  pointer to the new addendum.
- ADR addendum "(#1422): invariant-based induction": eligibility; an invariant violated only
  beyond the largest bound stays `unknown`; the three obligations (entry =
  claim on pre-loop state, preservation = claim at back edge from havoc state with invariant and
  loop condition assumed, exit = rest of the function from havoc state with invariant assumed and
  condition negated); never `failed`; true-but-non-inductive invariant stays `unknown`; partial
  correctness (non-terminating loop with invariant is `proven`); soft-claim sites come from the last
  BMC round; ordering after BMC; supersedes the #1418 bullet "`invariant` ... is not assumed";
  the `loop_literal_100` flip (ADR Rule 2 already allows `k > 50` fixtures to newly prove).
- `docs/spec/cli.md:69` "Loops" - rewrite, not just drop the parenthetical. Three statements go
  false: "A function is `Verified` only when the unwinding assertion holds"; "a loop that can run
  longer (a bound that is a parameter, `while true`, a literal bound above 64) is
  `verify_status: "unknown"` ... never `Verified`"; "An `invariant` ... is not assumed to hold
  (inductive use is future work)". New text: when every loop has an `invariant` and bounded
  unwinding stays `unknown`, the function is attempted inductively (entry, preservation, exit) and
  is `Verified` when all hold; otherwise `unknown` with the same `unwinding assertion:` message;
  induction never yields `VerifyFailed`; an invariant violated only beyond the largest bound is
  `unknown`; warnings are those reachable within the unrolled prefix; proofs are partial
  correctness. Keep the `unwinding assertion:` prefix text.
- Do **not** touch `docs/spec/contracts.md` (ADR defers its rewrite to the P3 docs child) or
  `docs/spec/grammar.md` (no syntax/semantics change of the language).
- Run `uv run python scripts/generate_help.py`, commit everything it regenerates (the cli.md Loops
  paragraph is embedded twice in `compiler/main.vow` and in `vow/src/skill.rs`), then
  `python3 scripts/check_help_coverage.py`. Regenerated Rust text is not dual-compiler drift.
  Rebuild with `cargo build --release -p vow` and `scripts/bootstrap.sh --skip-cargo`.

## Testing (TDD slices, in order)
1. **Cut structure** - `compiler/tests/test_vc_induct.vow` [new]. Red first on `count_up()` (copy
   from `test_vc_unroll.vow:19-48`): status OK; `vc_unrolled_detail` empty; block count = orig + 2;
   header's first inst is a havoc marker with the original Phi id; header `VOW_INV` became
   `VOW_REQ`; `h_entry` holds a `VOW_INV` over a remapped Phi; `h_back` ends in the cut-end marker
   and its Phi is fed by the latch Upsilon; preheader Upsilon targets `p_i`. More cases: a
   multi-block invariant region (`i >= 0 && i <= n` lowered as in `lower.vow:2927-3010`: header,
   rhs, short, merge, `VowInv` in the merge block) -> region cloned, jump/cut-end placed right
   after the `VowInv`, condition code not cloned; two latches (`continue`), Bool Phi, Unit Phi,
   nested loops (both cut), loop with no invariant -> `NA`, Ptr Phi -> `NA`, stray `VOW_INV`
   outside any region -> `NA`; running the pass twice gives identical `ir_print` output.
   Production: `vc_induct.vow`.
2. **Executor** - `compiler/tests/test_vc_exec.vow`: `vc_claims(cut)` on `count_up` + an `ensures`:
   claim count/kinds as expected (entry, preservation, ensures), no `VC_CLAIM_UNWIND`, header script
   has a `declare-const h<id>`, `cs.model_names` holds only parameters. Production: step 3.
3. **Driver regression guard** - `tests/verify-native/tests.sh` wiring tier: add a
   loop-with-invariant source and assert the fake-solver behaviours that must not regress
   (`unsat` -> Verified; `sat` -> failed with the invariant's vow id from BMC). This guard does
   **not** count toward the acceptance criteria: the fake solver answers uniformly and cannot
   reach the induction path, which only the real-solver fixtures exercise. A skipped fixture tier
   is not a pass - run Section 4g with `bitwuzla` on `PATH`. Time the new `pass/` fixtures once
   (each runs rounds 2..64 before induction, ~65 header copies per round, one solver process per
   claim) and note the numbers in the PR so Section 4g wall-clock does not jump unnoticed.
4. **Real-solver fixtures** (`tests/verify-native/`, header `// TEST: category invariant`):
   - `pass/loop_unbounded_count.vow`: `count(n)`, `requires n >= 0`, `invariant: i >= 0, i <= n`,
     `ensures result == n` (acceptance 1).
   - `pass/loop_unbounded_and_invariant.vow`: `invariant: i >= 0 && i <= n` (branch-lowered
     `&&` region) with `ensures result == n`; guards against silently ineligible `&&` invariants.
   - `pass/loop_unbounded_break_flag.vow`: `while` with `break`/`continue` and a Bool carried flag.
   - `pass/loop_unbounded_nested.vow`: nested loops, each with an invariant.
   - `pass/loop_unbounded_checked_add_warning.vow`: `acc +! x` with unbounded `n`; `// TEST: warning
     ArithOverflowReachable` (pins soft-claim rule).
   - `pass/loop_literal_100_invariant.vow`: today's `unknown/loop_literal_100.vow` body with its
     invariant (`i <= 100 && !(i < 100) => i == 100`).
   - `pass/loop_nonterminating_partial_correctness.vow`: `while true vow { invariant: true }`
     (pins the partial-correctness decision).
   - `unknown/loop_literal_100.vow`: edit in place, drop the invariant clause and fix its comment,
     so it still guards "deeper than the largest bound and no invariant => unknown, never Verified".
   - `fail/loop_unbounded_wrong_invariant.vow`: unbounded `n`, `invariant: i <= 3`; BMC reaches it
     at bound 4: `// TEST: counterexample-vow-id`, `counterexample-blame Callee` (acceptance 2).
   - `unknown/loop_invariant_too_weak.vow`: true but non-inductive/too weak (e.g. `invariant:
     i >= 0` with `ensures result == n`): `unknown`, message starts `unwinding assertion:`.
   - Audit the 12 existing fixtures that carry `invariant:` (grep'd): `pass/*` and `fail/*` are
     decided by BMC before induction and must not change; only `unknown/loop_literal_100.vow` flips.
5. **Gates**: `build/vowc test compiler/` (all `test_vc_*`), `bash tests/verify-native/tests.sh`,
   `scripts/bootstrap.sh --skip-cargo --no-cache` re-run at the final head SHA (record the SHA in
   the PR checklist; bootstrap is not run on PRs by CI), `scripts/full_test.sh` Section 4g,
   `python3 scripts/generate_operations.py --check`, `python3 scripts/check_help_coverage.py`.

## Verification surface
- ESBMC is not involved in the new path; the new code is verified by ESBMC at bootstrap Stage 1
  like all of `compiler/`. New functions carry no contracts (like `vc_unroll`), so no obligation
  grows; keep every loop in the module simple so Stage 1 stays fast.
- SMT obligations the native solver must discharge for a proof: entry claim per invariant (real
  state), preservation claim per invariant at `h_back` (havoc state, invariant + loop condition
  assumed), every hard abort claim in the body and after the loop, and the function `ensures` on
  paths leaving the loop. All must be `unsat`.
- `tests/verify*` ESBMC fixtures and C parity (`scripts/parity.py c`) are unaffected: no emitter,
  lowering or model-helper change.

## Risks
- **Soundness of the cut** (highest risk). Checklist for review/adversarial pass:
  (a) every back edge reaches `h_back` and `h` is entered only from `h_entry` (the acyclicity check
  enforces it); (b) only Phis of the header are havoc'd, i.e. exactly the loop-modified names,
  everything else keeps its pre-loop value; (c) values defined in the body are read after the loop
  only through exit Phis (loop-closed form, already enforced by the gate); (d) `vc_flow` branch
  facts are keyed by immutable SSA ids so they stay valid across the havoc; (e) ignored soft
  claims never feed the verdict; (f) induction never returns `failed`.
- **Region cloning bugs**: wrong region membership would drop or duplicate invariant code. Unit
  tests assert the cloned claim count and arguments and the NA paths; the driver runs
  `vc_unrolled_detail` on every produced function, so an eligible-but-malformed cut fails closed
  (non-empty detail -> BMC only).
- **Partial correctness** (vacuous proof of non-terminating loops) is a user-visible semantics
  change; recorded in the ADR, flagged for maintainer confirmation.
- **Time budget**: induction runs after up to six BMC rounds; a function that exhausts `--timeout`
  in BMC gets no induction attempt (stays `unknown`). Acceptable; revisit with automatic
  k-induction work.
- **Warnings are incomplete under induction**: only sites reachable in the last BMC round. Called
  out in cli.md/ADR so it is not read as "all reachable aborts".
- **Binary fixed point / determinism**: new pass iterates Vecs in index order, no `HashMap`; new
  module added to `concat_vow.sh`; the self-hosted compiler is the only consumer (no Rust twin), so
  Stage 0 -> Stage 1 -> Stage 2 byte identity is checked with the usual bootstrap.
- **Self-hosted gotchas** (CLAUDE.md): chained field access on struct values needs annotated `let`
  bindings (`let li: VcLoopInfo = st.li;` as in `vc_unroll.vow`).
- **Verdict-parity gate** (epic P5): the `loop_literal_100` flip is an "extra proof with written
  explanation" (ADR addendum is that explanation).
- **Seed pin**: "green locally" claims must also record `scripts/seed.toml` once the seed lands.

## Out of scope
- Automatic k-induction (base/forward/step) for invariant-free or non-inductive loops - separate
  epic child.
- Loop-carried `ptr` values / heap writes in loops, `Vec` loops (#1423), nested loops with an
  un-annotated inner loop (stays BMC only).
- Hinting "invariant not inductive" in the `unknown` message; any change to the `unwinding
  assertion:` text, the schedule 2..64, or `--timeout`.
- Changes to `contracts.md`, `grammar.md`, `errors.md`, `--help` structure beyond regeneration,
  the Rust compiler, `c_emitter.{rs,vow}`, ESBMC plumbing, perf work (#1419/#1420).
- Refactors or formatting of `vc_unroll.vow`, `vc_loops.vow`, `vc_exec.vow` beyond the listed hooks.
- Suggested commit order inside one PR (squash-merged; title e.g.
  `feat(verify): prove loops inductively from user invariants`): (1) markers + `vc_induct` +
  unit tests, (2) executor + driver + fixtures, (3) ADR + cli.md + regenerated help.
