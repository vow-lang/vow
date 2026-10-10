# Plan: native verifier — bounded loop unwinding with a mandatory unwinding assertion (#1418)

## Goal
`vowc verify --backend native` accepts reducible loops: it unrolls each natural loop to an internal, incrementally growing bound, checks every claim inside the unrolled prefix, and adds a mandatory unwinding assertion ("no execution takes more back edges than the bound"). A loop deeper than the bound is `unknown`, never `proven`; a violation inside the bound is `failed` with the existing vow-id/blame/span mapping. Part of epic #1398 (P3); k-induction (#1419/#1420) is out of scope.

## Assumptions
Decisions made without an operator (headless run); override in review.
- **Unroll at IR level, not in the executor.** A new pass clones loop bodies into an acyclic `IrFunction` with fresh ids; the existing acyclic executor (`vc_exec`/`vc_flow`) and the SMT writer stay unchanged except for two small hooks. Reason: the executor keeps single-assignment state keyed by instruction id and block index (`vc_exec.vow` `VcWalk`, `VcFlow`); re-walking blocks would overwrite bindings of interleaved iterations. Cloning gives every iteration unique ids for free.
- **Bound = number of back edges allowed** (a `while` that runs N body iterations takes N back edges). Copies `c = 0..k` of every loop block exist; the back edge out of copy `k` goes to a per-loop sink block (`Unreachable` carrying the marker `ds = "__vc_unwind_sink"`). The sink's path condition is the unwinding claim: "this path is infeasible".
- **Schedule** `2, 4, 8, 16, 32, 64` (`vc_unwind_bounds()`), max 64 ≥ ESBMC's default `--max-k-step 50` so everything ESBMC proves within 50 still proves. Constants are internal; no CLI flag (`--timeout` stays the only knob, ADR-1430 §2). Implementer may retune the list after measuring the loop fixtures, but the max must stay ≥ 50.
- **Per-round size cap** `VC_UNWIND_INST_CAP = 200000` unrolled instructions (checked *before* allocating, saturating arithmetic). Exceeding it ends the schedule with `unknown` ("unwinding budget"), never `Skipped` and never `proven`.
- **`invariant:` (IOP_VOW_INV) is a claim only, no assumption**, exactly like `ensures`: `lower_invariant_clauses` (`lower.vow:5792`) emits it at every header visit before the condition, and both C emitters treat it as `__ESBMC_assert` (`c_emitter.vow:1864-1877`, `c_emitter.rs:1410-1429`). Blame Callee, vow id from the instruction. Assuming it is the invariant-based induction of #1419.
- **The unwinding claim assumes nothing afterwards** (use `vc_add_claim`, not `vc_guard`): failing it is not a program failure.
- **Hard claim `sat` beats unwinding `sat`**: within one round every claim is decided; a real violation → `failed`; only if all hard claims are `unsat` and the unwinding claim is `sat` does the round advance (or end `unknown`).
- **No new `Skipped` code** (ADR-1430 §4: list is closed). Loop shapes the verifier cannot unroll stay `unsupported-opcode` with a new `detail` text (see Step 2).
- **Loop-carried values reach post-loop code only through exit Phis.** The lowerer already builds loop-closed SSA (`lower.vow:3545-3575`: exit-block Phis fed by header-exit and `break` Upsilons). The gate verifies this and skips anything else (`unsupported-opcode: loop value used outside its loop`) instead of inserting LCSSA Phis.
- **Collection loops stay `Skipped` (deliberate deviation from "existing loop fixtures give the same verdicts as ESBMC").** `bounds_correct`, `vec_fill` and `off_by_one_bounds` use `Vec` operations the native op-model does not cover (`unmodeled-builtin`; owned by the "Vec/String array+length" P3 child), so they remain `Skipped` natively and show as `weaker` in `scripts/verify_diff.py`, which exits 1 on any `weaker` row. This PR reports the loop rows only and does not claim the script passes. Post a `gh issue comment 1418` recording this and the schedule choice.
- **Source spans for loop counterexamples** come from the existing vow-id path (`ce_source_for_vow`, `verify_report.vow:21-44`, reads the original IR's VowInv/VowEns span). Unattributed aborts (div-by-zero etc.) inside a loop keep today's empty `source` exactly like outside a loop (ESBMC parity); not widened here. Cloned instructions keep the original `ostart/olen`/vow id, so the mapping is automatic.
- **No Rust twin, no C-model change.** D1 exception (ADR-2026-10-08-1421): the verifier lives only in `compiler/`. `c_emitter.{rs,vow}` untouched, so the byte-identical C parity rule is not engaged.

## Slice 0 (do first): confirm the lowerer's loop-closed SSA on the real fixtures
The design relies on post-loop code reading only exit-block Phis (`lower.vow:3545-3575`). Before writing `vc_unroll`, dump the IR (`build/vowc build --no-verify --dump-ir`) of `tests/verify/strong_invariant.vow`, `void_loop_invariant.vow`, `wide_literal_arithmetic.vow`, `tests/verify-fail/weak_invariant.vow`, `examples/sum_range.vow`, `examples/countdown.vow` and run the Step 1 analysis on them (a throwaway test); every one must be accepted. If any is rejected as "loop value used outside its loop", replace the gate rejection with LCSSA Phi insertion inside `vc_unroll` (a Phi in the single dedicated exit block per escaping value, one Upsilon per exit-predecessor instance, uses outside rewritten to it) and update this plan before continuing.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `compiler/vc_cfg.vow` | gate CFG shape; `vc_back_edge_detail` rejects every loop today | 5-8 (header comment), 74-90, 198-212 |
| `compiler/vc_gate.vow` | `vc_skip_reason`, `vc_inst_supported`; "loop-free" header comment | 8-16, 175-183 (VOW_REQ/ENS), 287-331 |
| `compiler/vc_ops.vow` | op-model table; `IOP_VOW_INV` is in `vc_op_unsupported` | 80-100 |
| `compiler/vc_exec.vow` | claims, claim kinds `VC_CLAIM_*`, `vc_walk_inst` (no VOW_INV case) | 34-44, 299-336, 339-413 |
| `compiler/vc_native.vow` | per-claim solver loop; becomes the bound loop | 22-60 |
| `compiler/vc_cex.vow` | `vc_claim_result`, `vc_proven_result` | 91-141 |
| `compiler/verifier.vow` | `memory_limit_reason`/`output_has_memory_limit` (model for the unwinding reason helpers) | 720-733 |
| `compiler/main.vow` | `native_report_result` unknown branch drops `raw_output`; embedded help/skill text | ~1736-1742, 2908, 3227, 3656, 5527-5547 |
| `compiler/ir_dominance.vow` | reuse `dom_successors`, `dom_rpo_numbers`, `dom_pred_offsets/list`, `dom_idoms`, `dom_tree_numbers`, `dom_dominates`, `dom_block_id_table`, `dom_fill` | 32-276 |
| `compiler/ir.vow` | `IrInst` 225-237, `IrBlock` 239-242, `IrFunction` 259-289, `ir_inst_new` 322, `ir_function_new` 363 | |
| `compiler/lower.vow` | loop lowering shape (read-only reference): while 3489, loop 3651, for 3817, break 5138, continue 5193, invariant 5792 | |
| `compiler/region.vow` | precedent for minting fresh inst/block ids and rebuilding blocks (`build_edge_split_block` 4708, max-id scan 5598) | |
| `scripts/concat_vow.sh` | bootstrap module list (line 23) — must list new modules in dependency order | 23 |
| `tests/verify-native/tests.sh` | fake-bitwuzla wiring tier; `SKIPPED_ONLY` fixture is a `while` loop | 124-140, 386-389 |
| `scripts/full_test.sh` | Section 4g native fixture runner (pass/fail/skip) | 1378-1508 |
| `docs/spec/cli.md` | "Native backend" section lists loops/`invariant` as `Skipped` | 58-80 |
| `docs/adr/2026-10-08-1422-native-verification-semantics.md`, `...-1430-native-verifier-cli-and-status-surface.md` | addenda | Rule 2; §4 |

New files: `compiler/vc_loops.vow`, `compiler/vc_unroll.vow`, `compiler/tests/test_vc_loops.vow`, `compiler/tests/test_vc_unroll.vow`, fixtures under `tests/verify-native/`.

## Steps

### 1. Add `compiler/vc_loops.vow` [new]: natural-loop analysis (pure, no solver)
- **Change**: `module VcLoops`, `use ir`, `use ir_dominance`. From `succ`/`rpo` (existing `dom_*`) compute:
  - retreating edges `b -> s` where `rpo[s] <= rpo[b]`; each must satisfy `dom_dominates(nums, nb, s, b)` (via `dom_idoms` + `dom_tree_numbers`), else irreducible;
  - natural loop of each header (backward walk from the back-edge sources over `dom_pred_*` until the header), merging back edges that share a header;
  - per-block loop chain outer→inner (loops with different headers are nested or disjoint in a reducible CFG; order by member count descending), flat CSR `chain_off/chain_loop`, plus `loop_of_header[b]`, `def_blk[id]`, `def_pos[id]`.
  - `vc_loop_shape_detail(f, tys, succ, rpo) -> String` ("" when unrollable). Details (all under code `unsupported-opcode`): `irreducible loop (back edge to a block that does not dominate its source)`, `loop value used outside its loop` (a use whose def block is in loop L and use block is not in L; Upsilon uses inside L are fine), `entry block is a loop header`, `loop nest deeper than 8`.
- **Reuses**: `dom_successors` (`ir_dominance.vow:63`), `dom_rpo_numbers` (:90), `dom_pred_offsets` (:145), `dom_pred_list` (:169), `dom_idoms` (:206), `dom_tree_numbers` (:240), `dom_dominates` (:274), `dom_fill` (:14).
- Keep functions short and contract-free like the other `vc_*` modules (bootstrap verifies them with ESBMC).

### 2. Replace the blanket back-edge rejection in the gate
- **File**: `compiler/vc_cfg.vow` (74-90, 198-212): drop `vc_back_edge_detail`; call `vc_loop_shape_detail` from `vc_cfg_detail`. Update the header comment ("the reachable graph is acyclic" → "reducible"). Teach `vc_reachable_unreachable_detail` (:34-45) to accept an `Unreachable` carrying the unwind-sink marker so `vc_cfg_detail` can validate unrolled output too.
- **File**: `compiler/vc_ops.vow`: move `IOP_VOW_INV` from `vc_op_unsupported` (:80-100) to `vc_op_modeled_core`; add `fn VC_INTERNAL_UNWIND_SINK() -> String { "__vc_unwind_sink" }` and `vc_is_unwind_sink(inst)` next to `vc_is_unwrap_abort` (:111).
- **File**: `compiler/vc_gate.vow`: `vc_inst_supported` treats `IOP_VOW_INV` like `VOW_REQ/ENS` (`n == 1 && t0 == Bool && dk == IDATA_VOW_ID`, :175-183); update the "loop-free" header comment (:8-16). The Phi-head / one-Upsilon-per-predecessor rules in `vc_upsilon_detail` already hold on cyclic graphs.

### 3. Add `compiler/vc_unroll.vow` [new]: `vc_unroll(f, bound) -> VcUnrolled`
- **Change**: returns `{ fn: IrFunction, too_large: bool }`. The result function is acyclic, copies `id/name/params/param_names/return_ty/effects/vows/source_file` from `f` via `ir_function_new` + field copies (do not alias `f.blocks`).
  - **Instance** = (orig block, counter per loop in its chain). Dense table per block of size `(bound+1)^depth(b)`; total instruction count computed first with saturating multiply and compared to `VC_UNWIND_INST_CAP`.
  - **Edge rule** from instance `(b, ctrs)` to orig block `s`: if `s` is the header of a loop `L` containing `b` → counter of `L` + 1, and `> bound` → the per-loop sink block (created lazily, one `Unreachable` with `ds = VC_INTERNAL_UNWIND_SINK()`, `ostart/olen` = the cut back edge's terminator span); otherwise counters for `chain(s)` are taken from `ctrs` where the loop also contains `b`, `0` for loops being entered. Exits drop counters by construction.
  - **Ids**: every cloned instruction gets a fresh id `base[instance] + position` (holes are allowed; tables are sized `max_id + 1`). A use of orig id `v` resolves to the def instance = `(def_blk[v], ctrs restricted to chain(def_blk[v]))` — always a prefix, because Step 1 rejects out-of-loop uses. Fresh block ids start at `max_block_id + 1`.
  - **Terminators/Upsilons**: successor instances are resolved first, then instructions are cloned. A cloned `Branch`/`Jump` targets the new block ids. A cloned `Upsilon` targets the Phi of the successor instance containing its orig Phi (`phi_blk` as in `vc_cfg.vow:104-120`); an Upsilon whose successor edge was cut to the sink is dropped.
  - Discovery order is a fixed DFS over `succ[2b]`, `succ[2b+1]`, so output is deterministic (no `HashMap`).
  - **The clone of the original entry (block 0, counters all 0) must be `g.blocks[0]`.** `vc_claims` gives index 0 the `true` path condition, `dom_rpo_numbers` starts its DFS at index 0 and `vc_phi_head_detail` rejects a Phi there; any other first block would walk from the wrong entry and could make claims vacuously unreachable (wrongly `proven`). Assert it in the slice-3 test.
- **Reuses**: id-minting/rebuild pattern from `region.vow:5598-5610` and `build_edge_split_block` (`region.vow:4708`); `ir_inst_new` (`ir.vow:322`), `ir_block_new` (:356).

### 4. Executor hooks
- **File**: `compiler/vc_exec.vow`:
  - add `VC_CLAIM_UNWIND() -> 6` and `VC_LABEL_UNWIND()` ("unwinding assertion");
  - in `vc_walk_inst` (:299-336): `op == IOP_VOW_INV()` shares the `VOW_ENS` branch (claim kind `VC_CLAIM_ENSURES`, vow id `inst.dv`); a reachable `Unreachable` with `vc_is_unwind_sink` adds `vc_add_claim(cs, VC_CLAIM_UNWIND(), -1, label, pc, false, inst.ostart, inst.olen)` **without** `vc_assume`;
  - `vc_claims` stays the entry point, now called with the unrolled function.
- The sink case must sit before the final `else if op != IOP_RETURN() && op != IOP_UNREACHABLE()` (:333) so it is never `vc_define`d; an ordinary `Unreachable` stays a no-op (the cfg gate already vets reachable ones).
- Claim order: sinks are walked in RPO position, so the unwind claim may precede later hard claims; Step 5 therefore never returns on it (it only sets `open`).

### 5. Bound loop in the native driver
- **File**: `compiler/vc_native.vow`: `vc_verify_function` runs the schedule `vc_unwind_bounds()`. Per round: `u = vc_unroll(f, k)`; `too_large` → return `unknown` with the budget reason; `cs = vc_claims(u.fn)`; decide claims in order with the existing budget arithmetic (`started` outside the round loop, budget shared by all rounds). Per claim:
  - `UNSAT` → continue; soft `ARITH` `SAT` → record site (dedupe by `(cause, owner, ostart, olen)` because every iteration copy repeats the source site; sites are meaningful only for the final `proven` round);
  - `UNWIND` + `SAT` → `open = true`, keep deciding the remaining claims;
  - any other non-`UNSAT` answer (hard `SAT`, `unknown`, `timeout`, `error`, memory) → return `vc_claim_result(f, cs, k, ans)` with the **original** `f` (vow table, names) as today;
  - after the claims: `!open` → `proven` (forward condition holds); `open` and not last round → next bound; `open` at the last bound → `unknown`.
  - A function with no claims in a round (`cs.kinds.len() == 0`) is `proven` without a solver, as today.
- Extract a pure helper `vc_round_decision(open, round, rounds) -> i64` (PROVEN / NEXT / UNKNOWN) so it is unit-testable without a solver.
- **File**: `compiler/vc_cex.vow` (it already builds the unknown results; **not** `verifier.vow`, which is the ESBMC driver with a Rust twin and outside the D1 exemption): `vc_unwinding_reason(bound) -> String` ("unwinding assertion: a loop may run more than <bound> iterations (internal bound reached)"), `vc_unwinding_budget_reason()`, `vc_output_has_unwinding(raw)`, modelled on `memory_limit_reason`/`output_has_memory_limit` (`verifier.vow:720-733`).
- **File**: `compiler/main.vow` `native_report_result` unknown branch (~1736-1742): surface the unwinding reason (instead of the generic "the solver answered unknown") so `verify_message` carries it; reason text travels in `raw_output`, so `vc_wire_encode/decode` (`vc_worker.vow:52-190`) need no format change.

### 6. Docs and embedded help
- `docs/spec/cli.md` "Native backend" (:60-80): subset sentence (acyclic → reducible loops, `invariant` modelled), new **Loops** bullet: unwinding assertion, internal bound (not a flag), `unknown` with `verify_message` text, hard violations inside the bound are `failed`, `proven` covers all iteration counts only when the unwinding assertion is discharged, deeper-than-bound / symbolic-bound loops without a closing argument are `unknown` (so are ESBMC's), still `Skipped`: irreducible loops, out-of-loop value use, collections inside loops (`unmodeled-builtin`).
- ADR addenda (do not edit the accepted text): ADR-1422 "Addendum (#1418)" under Rule 2 — bound semantics (back edges), schedule max 64, claim without assumption, `failed` beats `unknown`, invariant is a claim only until #1419; ADR-1430 "Addendum (#1418)" — loops leave the `Skipped` set, new `unsupported-opcode` details, `unknown` reason text, and that `proven-ir`/`ModelCapacityAssumed` are untouched.
- Regenerate the embedded copies: `uv run python scripts/generate_help.py` (touches `compiler/main.vow`, `vow/src/skill.rs`, `skills/vow/reference/cli.md` — all generated from the spec), then `python3 scripts/check_help_coverage.py`.
- `scripts/concat_vow.sh:23`: insert `vc_loops` after `vc_cfg`'s dependencies are available (before `vc_cfg`, which calls it) and `vc_unroll` before `vc_native`.
- `docs/spec/contracts.md` is the ESBMC contract document; its native rewrite is the separate P3 docs child — **not** touched here.

## TDD slices
Each slice is red → green → refactor; run `build/vowc test compiler/tests/<file>.vow` (never `tsx`; Vow tests only).
1. **Loop analysis** — `compiler/tests/test_vc_loops.vow` [new] (style: `check_*() -> i64` returning 0 or a unique code, `use tests.builders`): hand-built IR for (a) `while` (header with Phi, body, exit with exit-Phi) detects one loop with the right members; (b) nested loops give chains `[outer]`, `[outer, inner]`; (c) two back edges (`continue`) share one header; (d) irreducible CFG rejected; (e) in-loop value read after the loop rejected, exit-Phi read accepted; (f) entry-as-header rejected. Code: Step 1.
2. **Gate** — `compiler/tests/test_vc_gate.vow`: `check_loop_rejected` (:114-128) becomes `check_loop_accepted`; the VowInvariant pin (:206-208) becomes accepted; add rejection cases from slice 1. `compiler/tests/test_vc_ops.vow` must still pass (table covers every opcode). Code: Step 2.
3. **Unroll structure** — `compiler/tests/test_vc_unroll.vow` [new]: unroll the `count_up` dump shape (header Phi, `VowInvariant`, body, exit) at bound 2: assert `g.blocks[0]` is the clone of the entry, `vc_cfg_detail(g) == ""` (acyclic, marker accepted), `ir_non_dominating_read(g) == ""`, block/instruction counts, exactly one sink block, the cut Upsilon dropped, exit Phi gets one Upsilon per exit edge instance, golden `print_function(g)`. Add: bound 0..k monotone counts, nested loops (`(k+1)^2` copies), `break` + `continue` + early `return` shapes, `too_large` at a tiny injected cap, determinism (two runs print identically). Code: Step 3.
4. **Executor** — `compiler/tests/test_vc_exec.vow` (extend; helpers `mk_vow`, `one_block` exist): claims of an unrolled loop contain one `VC_CLAIM_ENSURES` per invariant per header copy with the original vow id and span, plus one `VC_CLAIM_UNWIND` whose query asserts the sink path condition and *no* assumption follows it; function with no cut edge has no unwind claim. Code: Step 4.
5. **Driver decision + reason text** — `compiler/tests/test_vc_cex.vow` / a new case in `test_vc_worker.vow`: `vc_round_decision` truth table, schedule is strictly increasing with max ≥ 50, `unwinding_reason` round-trips through `vc_wire_encode/decode` as `raw_output` with status `VERIFY_UNKNOWN`. Code: Step 5 (pure parts).
6. **End to end (real solver, `tests/verify-native/`)** — fixtures + runner. `pass/`: `loop_literal_bound.vow` (u64 sum, invariant, `ensures result == 36`, like `tests/verify/strong_invariant.vow`), `loop_u128_count.vow`, `loop_nested_literal.vow`, `loop_break_continue.vow`, `loop_early_return_ensures.vow`, `loop_checked_add_warning.vow` (`// TEST: warning ArithOverflowReachable`, one warning per source site, not per iteration). `fail/`: `loop_invariant_violated.vow` (copy of `tests/verify-fail/weak_invariant.vow`: `counterexample-vow-id 2`, `-fn`, `-blame Callee`, replay confirmed), `loop_ensures_wrong.vow`, `loop_bug_at_iteration_40.vow` (violation only reachable after 40 iterations; found at bound 64 → proves incrementality). New `unknown/` dir: `git mv skip/loop_skipped.vow unknown/loop_symbolic_bound.vow`, `loop_literal_100.vow` (concrete 100 iterations → deeper than max bound → `unknown`, never `Verified` — acceptance #1), `loop_infinite.vow`. Runner (`scripts/full_test.sh` Section 4g: add `tests/verify-native/unknown/*.vow` to the `for` glob at :1397 **and** a new `unknown)` branch in the `case` at :1407, otherwise the `*)` default treats them as `Skipped`): expect `status == VerifyFailed`, exit 1, `verify_status == unknown`, empty `counterexamples`, `verify_message` containing `unwinding`; skipped without `bitwuzla` like pass/fail.
7. **Wiring tier** — `tests/verify-native/tests.sh`: replace the `SKIPPED_ONLY` loop fixture (:124-140, assertion ~386-389) with another still-unsupported construct (e.g. a float function) so "all skipped needs no solver" keeps its meaning; add a loop fixture under fake `unsat` (→ Verified, one query per claim per round recorded) and fake `unknown` (→ `verify_status unknown`).
8. **ESBMC parity (acceptance #2)** — run `python3 scripts/verify_diff.py` over `tests/verify`, `tests/verify-fail`, `tests/verify-skip` and attach the loop rows to the PR: expected `match` for `strong_invariant`, `void_loop_invariant`, `wide_literal_arithmetic`, `weak_invariant`; `bounds_correct`, `vec_fill`, `off_by_one_bounds` stay `Skipped` natively (see Assumptions: they are `weaker` rows, not regressions of this issue, and the script's exit status is not claimed); `verify_jobs_ce_before_soft` loop function is `unknown` in both. Any `soundness` row blocks the PR.

## Verification surface
- ESBMC is not involved (native only). Per function the obligations are: one `ensures`/`invariant` claim per clause per cloned instance, the existing abort claims per cloned instruction, plus one unwinding claim per loop with a cut back edge. `proven` needs every claim `unsat`, including the unwinding claim of the same round.
- Fixtures: `tests/verify-native/{pass,fail,unknown}`; no change to `tests/run/` or `examples/`. `examples/sum_range.vow`/`examples/countdown.vow` (symbolic bound) become `unknown` natively, same class as ESBMC.
- The new Vow code itself is verified by ESBMC during `scripts/bootstrap.sh` (stage 0): new functions carry no contracts, use the same `while`/index patterns as `vc_cfg.vow`.

## Risks
- **Query size / solver spawns.** No BV simplification exists (`vc_term.vow` header), so literal-bound loops are decided by the solver, one process per claim, re-solved every round (~2× the final round's work). Mitigation: geometric schedule, instruction cap, shared per-function `--timeout`; incremental reuse is P4 and out of scope. Measure `strong_invariant`-class fixtures and nested loops before finalising the schedule.
- **Instance-count blow-up for nested loops** `(k+1)^depth`: the cap makes it `unknown`, with smaller rounds still able to prove; gate rejects depth > 8.
- **Id/offset tables sized by max id** (`vc_ty_table`, `dom_block_id_table`): unrolled ids are fresh and dense per instance; check memory under the 4 GiB worker cap (`VC_WORKER_MEM_KB`).
- **Soundness traps**: (a) Upsilon to a cut edge must be dropped, not retargeted; (b) never assume the unwinding claim; (c) soft arithmetic sites from a non-final round must not leak; (d) a `sat` unwinding claim must not become `failed` (`vc_claim_result` always maps `SAT` to `VERIFY_FAILED`, `vc_cex.vow:113`) — handle in `vc_native` before calling it.
- **Binary fixed point**: new modules must be deterministic (no `HashMap` iteration; fixed DFS order) and added to `scripts/concat_vow.sh`; run `scripts/bootstrap.sh --skip-cargo --no-cache` on the final head SHA and record the SHA and the `scripts/seed.toml` pin (CLAUDE.md "green locally").
- **Replay**: `--replay-cex` on loop counterexamples relies on debug-mode `VowViolation` for invariants (supported); counterexample `values` list only integer parameters (existing limit), loop variables are not shown.
- **Doc drift**: `docs/spec/cli.md` change must be mirrored by `generate_help.py`; `check_help_coverage.py` runs in `full_test.sh`.
- Commit/PR title must be lower-case conventional: `feat(verify): bound loop unwinding with a mandatory unwinding assertion`.

## Out of scope
- k-induction and invariant-based induction (#1419, #1420); assuming invariants; havoc of loop-modified variables.
- Vec/String/collection modelling (loops over `Vec` stay `Skipped: unmodeled-builtin`), per-claim slicing, incremental solving across rounds, result cache, perfetto spans for rounds.
- Inlined calls, recursion, floats, `contracts --verify`/`test --verify` on native.
- BV constant folding/simplifier, source spans for unattributed aborts, widening counterexample `values` to loop variables.
- `docs/spec/contracts.md` rewrite, `errors.md`, removal of ESBMC flags; any Rust-compiler change.
- Refactors of `vc_exec.vow`/`vc_gate.vow` beyond the hooks above.
