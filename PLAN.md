# Plan: native verifier — inlined calls with blame and vow_id mapping (#1416)

## Goal
Make `vowc verify --backend native` verify functions that call other user functions by splicing the
callee IR into the caller before symbolic execution. A callee `requires` becomes a claim blamed on
the caller, a callee `ensures`/`invariant` becomes a claim blamed on the callee, both keep the
callee-local `vow_id` and source spans, and recursion is `Skipped` (`recursion-unsupported`).
Matches today's ESBMC semantics (callees co-emitted as C functions; labels `vow:pre:<fid>:<vid>` /
`vow:post:<fid>:<vid>`), which the report side (`build_ce_from_result`) already consumes.

## Assumptions
- Verifier-only change in `compiler/` (ADR-2026-10-08-1421 scoped exception): no Rust twin, no
  `vow-verify` change, no `c_emitter.*` change, so verifier C parity is untouched.
- Scalar-only inlined calls in this slice: arguments and result are integer, `Bool` or `Unit`. A
  pointer-typed argument/result at a call site stays `unsupported-opcode: Call[ptr]` (fail closed).
  Aggregates through calls are a follow-up child (aggregate values are #1417's scope on top of this).
  Callees may build and use aggregates internally. (best guess: keeps the slice surgical)
- Recursion code: any call-graph cycle reachable from the target ⇒ `recursion-unsupported`, detail
  names the cycle path (`a -> b -> a`). A reachable callee rejected by the gate ⇒
  `non-modelable-callee: <callee> (<code>)` per ADR-1430. (best guess; ADR rows are ambiguous for a
  target that merely calls a recursive function — inlining it would not terminate, so the cycle code wins)
- Size budget: a call tree above the instruction cap is `unknown` with a structured reason from the
  worker (same shape as `vc_unwinding_budget_reason`), never `Skipped` (closed code list) and never `proven`.
- A target with no `vow` block (the "caller-precondition" role, `caller_precondition_role == 1`) keeps
  ESBMC's demotion: only callee-`requires` claims (and the unwinding claim) are obligations; every
  other claim is assumed. Mirrors `caller_preconditions_only_source` (`vow-verify/src/c_emitter.rs:3060`).
- No change to `docs/spec/*` (the spec describes the ESBMC pipeline until P3's rewrite child; no
  flag/syntax/semantics change). Record the decisions as an ADR addendum, as #1418 did.
- `counterexample.schema.json` (`docs/spec/schemas/` and `skills/vow/schemas/` copy) describes `call_sites` as
  "caller-blame failures"; native will also fill it for callee-blame counterexamples. The wording is left
  stale until the P3 spec child (editing the schema needs the copy + help regeneration and the native
  backend is still opt-in). Recorded in the ADR addendum. (best guess)
- Counterexample `values` stay the target's parameters only (as today); deeper intermediate argument
  values are not recovered (`violating_args[].value` stays `""` beyond depth 1, which the schema allows).

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `compiler/vc_inline.vow` [new] | call-graph cycle check, module-level gate, IR splicer, frame table | whole file |
| `compiler/vc_ops.vow` | frame-marker helpers next to `VC_INTERNAL_UNWIND_SINK`; reason constants already exist (`VC_SKIP_RECURSION`, `VC_SKIP_NON_MODELABLE_CALLEE`) | 8-30, 120-135 |
| `compiler/vc_gate.vow` | `vc_inst_supported` CALL branch (only unwrap abort accepted today); `vc_skip_reason` | 185, 214 (CALL), 20-35 (`vc_reason_code`) |
| `compiler/vc_exec.vow` | claim kinds, `vc_add_claim`/`vc_guard`, `vc_walk_inst` VOW_REQ/ENS/INV, `VcClaims`, `vc_claims` | 42-52, 55-80, 100-125, 295-330 |
| `compiler/vc_cex.vow` | `vc_claim_result` sets `callee_precondition_*` / `callee_postcondition_*` | 55-90 |
| `compiler/vc_native.vow` | `VcOutcome`, `vc_run_round` (soft-site owner), `vc_verify_function` | 20-60, 80-140 |
| `compiler/vc_worker.vow` | wire format (`VOWRES1`), encode/decode of `VcOutcome` | 50-200 |
| `compiler/main.vow` | `native_plan` (gate), `run_verify_worker` (needs module), `native_report_result` (frames → `call_sites`) | 1728-1900 |
| `compiler/verifier.vow` | reuse `parse_vow_label`/`ParsedVowLabel` semantics; `is_verify_target`, `caller_precondition_role` | 790-860, 1250-1265 |
| `compiler/ir.vow` | `IrInst` (`ds` free on arith/vow insts), `IDATA_CALL_TARGET`, `IrFunction.id` | 225-330 |
| `compiler/vc_loops.vow` | reuse `vc_clone_inst`, `vc_replace_arg`, `vc_function_shell` | 419-470 |
| `compiler/ir_dominance.vow` | reuse `dom_block_id_table`, `dom_successors`, `dom_is_terminal` | — |
| `scripts/concat_vow.sh` | module list: add `vc_inline` after `vc_gate`, before `vc_native` | 23 |
| `compiler/tests/test_vc_inline.vow` [new] | unit tests for cycles, splice, gate | — |
| `compiler/tests/test_vc_exec.vow`, `test_vc_gate.vow`, `test_vc_cex.vow`, `test_vc_worker.vow` | extend | — |
| `tests/verify-native/{pass,fail,skip}/*.vow` [new fixtures] | end-to-end | — |
| `docs/adr/2026-10-08-1422-native-verification-semantics.md` | addendum (#1416) | end of file |
| `docs/adr/2026-10-08-1430-native-verifier-cli-and-status-surface.md` | note the recursion/non-modelable-callee detail formats | §4 table |

## Steps

### 1. Frame markers and claim kinds (`compiler/vc_ops.vow`, `compiler/vc_exec.vow`)
- **Change**: in `vc_ops.vow` add `VC_FRAME_PREFIX()` (`"__vc_frame:"`), `vc_frame_mark(k) -> String`,
  `vc_frame_of(inst) -> i64` (-1 when `inst.ds` has no prefix). In `vc_exec.vow` add
  `VC_CLAIM_CALLER_PRE() = 7`, `VC_CLAIM_CALLEE_POST() = 8` after `VC_CLAIM_UNWIND`. Add
  to `VcClaims` `owners: Vec<i64>` (function id owning the claim's clause or site, parallel to `kinds`;
  the target's own id for frame -1) and `caller_pre_only: bool`. The frame→function-id table is passed
  into `vc_claims_mode` and `owners[k]` is recorded when each claim is created, so `vc_exec`/`vc_cex` never
  depend on `vc_inline`.
- **Reuses**: marker convention of `vc_is_unwind_sink` (`vc_ops.vow`).
- **Why `ds`**: `vc_clone_inst` (`vc_loops.vow:419`) copies `ds`, so the marker survives
  `vc_close_loops` and `vc_unroll` with no side tables keyed by shifting ids/positions.

### 2. Executor: callee clauses become claims (`compiler/vc_exec.vow`)
- **Change** in `vc_walk_inst` (lines ~295-330):
  - `IOP_VOW_REQ`: frame < 0 ⇒ `vc_assume` as today; frame ≥ 0 ⇒ `vc_add_claim(CALLER_PRE, vow_id=inst.dv, …, frame)` then `vc_assume(pc, goal)` (claim-then-assume, like `vc_guard`).
  - `IOP_VOW_ENS`/`IOP_VOW_INV`: frame < 0 ⇒ `VC_CLAIM_ENSURES` as today; frame ≥ 0 ⇒ `CALLEE_POST` claim then assume.
  - `vc_guard`/`vc_exec_arith`/shift/unwrap claims take their frame from `vc_frame_of(inst)` so soft arithmetic sites keep the callee as owner.
  - `caller_pre_only`: `vc_add_claim` drops every kind except `CALLER_PRE` and `UNWIND` (assumptions kept). Expose via `vc_claims_mode(f, caller_pre_only)`; keep `vc_claims(f)` = mode false so existing tests do not churn.
- **Ordering**: a callee's `requires` precede its body in IR, so a caller violation is reported before any callee `ensures` that would only fail because of it (deterministic blame; ESBMC's disjunction left it solver-dependent).

### 3. Result mapping (`compiler/vc_cex.vow`)
- **Change** in `vc_claim_result` (sat branch): for `CALLER_PRE` set `vow_id` and `callee_precondition_func_id/vow_id` = (frame func id, callee-local id); for `CALLEE_POST` set `callee_postcondition_*`. `vow_id` stays the callee-local id (spec: "callee-local id a debug-mode VowViolation would report"). The func ids come from `VcClaims.owners` (step 1). `vc_claim_result` currently writes the
  "Violated property:" label whenever `kinds[k] != VC_CLAIM_ENSURES()`: exclude `CALLER_PRE` and
  `CALLEE_POST` too, otherwise they carry an empty label in `raw_output`, which the callee-post fallback
  in `build_ce_from_result` would read.
- **Reuses**: `ParsedVowLabel` field semantics in `verifier.vow:790-860`; `build_ce_from_result` (`main.vow:485-640`) already resolves both label kinds.

### 4. Module-level gate (`compiler/vc_gate.vow`, `compiler/vc_inline.vow` [new])
- `vc_gate.vow`: add `calls: bool` to `vc_inst_supported`; with `calls` the `IOP_CALL`/`IDATA_CALL_TARGET`
  branch accepts scalar-only arguments and a result of int/Bool/Unit. Add `vc_skip_reason_calls(src)`
  (per-function check with calls allowed); `vc_skip_reason(src)` stays `calls=false` (existing tests keep
  `unsupported-opcode: Call[i64]`).
- `vc_inline.vow`: `vc_module_skip_reason(f, m) -> String`, the single parent-side decision; the gate must
  vet the function the worker actually runs, i.e. the flattened one (splitting at a call creates join blocks
  of every callee `Return`, and `vc_agg`'s "field write only on a value built in the same block" rule and
  `vc_cfg_detail` see that shape, not the per-function one):
  1. iterative DFS (explicit stack, three colours) over `IDATA_CALL_TARGET` edges resolved with
     `IrFunction.id`; a back edge ⇒ `vc_reason(VC_SKIP_RECURSION(), "a -> b -> a")`.
  2. each distinct reachable callee in post-order, memoised by function index: call-site arity/type vs the
     callee signature; `vc_skip_reason_calls(callee)`; first failure ⇒ `non-modelable-callee: <callee>
     (<code>)` using `vc_reason_code` (this keeps the ADR-1430 code instead of a generic opcode reason).
  3. `vc_inline(f, m, cap)`; TOO_LARGE is not a skip (see step 6), the function proceeds to a worker that
     answers `unknown`.
  4. the existing full `vc_skip_reason(flat)` on the flattened result; a rejection here is reported as
     `unsupported-opcode: <detail>` (the aggregate-shape / CFG detail), fail closed, never a worker ERROR.
  The extra cost is one flatten per target, comparable to the `vc_close_loops` the gate already does.
- **Reuses**: `vc_reason`, `vc_reason_code` (`vc_gate.vow:20-35`), `VC_SKIP_*` (`vc_ops.vow`).

### 5. The splicer (`compiler/vc_inline.vow`)
- `vc_inline(f, m, cap) -> VcInlined { f, frames: VcFrames, status }`; `VcFrames { func_ids, call_starts, call_lens, parents }`.
- Algorithm per call instruction `c` at position p of block B (recursively for the callee's own calls, depth-first, so frame k+1 is the callee of frame k):
  - split B: B keeps `[0..p)` + `Jump callee_entry'`; new continuation block C gets a head `Phi(id = c.id, ty = c.ty)` (omitted for Unit) followed by `(p..]`, including B's original terminator. Reusing `c.id` for the Phi keeps every downstream read valid.
  - clone the callee blocks with fresh block ids and fresh value ids (counters start above the maximum of the working function); `GET_ARG k` is dropped and every read mapped to `c.args[k]` via a substitution table (`vc_replace_arg` style); each `Return v` becomes `Upsilon(v → phi)` + `Jump C`; for a `Unit`-returning callee the Phi **and** the
    Upsilon are both omitted (a `Return` may carry one `Unit` argument).
  - mark claim-bearing clones (VOW_REQ/ENS/INV, `IOP_CADD..CREM`, `WDIV`, `WREM`, `SHL`, `SHR`) with `ds = vc_frame_mark(k)`; leave all other `ds` (extern symbols, constants) intact.
  - record frame k = (callee.id, c.ostart, c.olen, parent frame).
- Cost: first compute the saturating call-tree instruction count by memoised post-order; `> VC_INLINE_INST_CAP()` (`200000`, same value as `VC_UNWIND_INST_CAP`) ⇒ status TOO_LARGE before materialising anything.
- Output must satisfy `ir_non_dominating_read` and `vc_cfg_detail` (Phi at block head, Upsilon in a predecessor): assert both in tests; any violation is `VERIFY_ERROR` "internal error: inlining produced an unsupported control-flow graph" (same pattern as the unroller).
- **Reuses**: `vc_clone_inst`, `vc_replace_arg`, `vc_function_shell` (`vc_loops.vow:419-470`), `dom_block_id_table`.

### 6. Pipeline wiring (`compiler/vc_native.vow`, `compiler/vc_worker.vow`, `compiler/main.vow`)
- `vc_verify_function(f, m, timeout_ms, scratch)`: inline first, then `vc_close_loops` → unroll rounds as today on the flattened function; `vc_run_round` passes `caller_pre_only = f.vows.len() == 0`, and the soft-site `owner` becomes the claim's frame func id (falls back to `f.id`). TOO_LARGE ⇒ `vc_plain_result(VERIFY_UNKNOWN, vc_inline_budget_reason(cap))`.
- `VcOutcome` gains the failing claim's frame chain (`frame_funcs`, `frame_starts`, `frame_lens`, outermost first). Wire tag `VOWRES1` → `VOWRES2`, encode/decode the three lists; decode stays fail-closed (incomplete ⇒ `None`).
- `main.vow`: `native_plan` calls `vc_module_skip_reason(f, ir_mod)`; `run_verify_worker` passes `fr.ir_mod`; `native_report_result` after `build_ce_from_result` replaces `call_site_*` with the exact chain (`caller_function` = function containing the call, offset/length of the call instruction, `file` as the existing code does) when the chain is non-empty; keeps the direct-call `violating_args` the existing scan finds. `unknown` reasons that start with the inlining-budget label are printed like the unwinding reason.
  Cross-target duplicate soft warnings already collapse in `report_arith_overflow` via
  `arith_already_reported` (`main.vow:1507`), so `checked_arith_callee_attribution` stays at one warning;
  assert it in the fixture run.

### 7. Docs
- ADR-1422 addendum (#1416): inlining model, claim kinds and order, caller-pre-only demotion, scalar-only boundary, budget ⇒ `unknown`, frame chain in `call_sites`.
- ADR-1430 §4: one line each for the `recursion-unsupported` detail (cycle path) and `non-modelable-callee` detail (`<callee> (<code>)`).
- `scripts/concat_vow.sh`: add `vc_inline`.

## Testing (vertical TDD slices, red → green, one commit each; every slice ends in an observable verdict)
Unit tests run with `build/vowc test compiler/tests/<file>`; end-to-end fixtures live in
`tests/verify-native/` (Section 4g of `scripts/full_test.sh`, needs `bitwuzla`; `skip/` runs without).
1. **Caller violates a callee `requires` (single-level scalar call).**
   Red: `fail/inline_caller_requires_violation.vow` (`counterexample-blame Caller`, vow-id = callee's,
   `counterexample-fn <caller>`) plus `test_vc_inline.vow` one-call splice test and `test_vc_exec.vow`
   `CALLER_PRE` claim test. Green touches: `vc_gate.vow::vc_inst_supported` (`calls`), `vc_inline.vow::vc_inline`
   (one level, `vc_module_skip_reason` steps 2-4), `vc_ops.vow::vc_frame_mark/vc_frame_of`,
   `vc_exec.vow::vc_walk_inst` (VOW_REQ in frame), `vc_cex.vow::vc_claim_result` (`callee_precondition_*`),
   `vc_native.vow::vc_verify_function`, `main.vow::native_plan/run_verify_worker`.
2. **Callee `ensures` is blamed on the callee.**
   Red: `fail/inline_callee_ensures_wrong.vow` (`Callee`, `counterexample-fn <callee>`), exec test for
   `CALLEE_POST`, cex test for `callee_postcondition_*` and no empty label. Green: `vc_exec.vow::vc_walk_inst`
   (ENS/INV in frame), `vc_cex.vow::vc_claim_result`. Also `pass/inline_callee_ensures_used.vow` (caller's
   `ensures` provable only through the callee's) and `pass/inline_requires_established.vow`.
3. **Recursion is `Skipped` with a reason.**
   Red: `skip/recursion_self.vow`, `skip/recursion_mutual.vow` (`// TEST: skip-reason recursion-unsupported`),
   cycle unit tests (self, mutual, indirect a→b→c→a, diamond DAG without false positive, unreachable cycle
   ignored); `skip/callee_float_rem.vow` where the callee is **uncontracted**, scalar-signature
   (float `%` computed internally, returns `i64`) so `non-modelable-callee: g (float-rem-unsupported)` is the
   only reported reason (`full_test.sh` requires every skipped message to carry the directive's code).
   Green: `vc_inline.vow::vc_module_skip_reason` step 1, `VC_SKIP_*` use.
4. **Nested calls, loops and aggregates live across calls; frame chain in the counterexample.**
   Red: `fail/inline_nested_requires_two_frames.vow` (two `call_sites`, outermost first),
   `pass/inline_nested_calls.vow`, `pass/inline_call_in_loop.vow`, `pass/inline_struct_live_across_call.vow`
   (a struct built before a scalar call and read after it: proves the gate vets the flattened function),
   splice tests (Phi reuses the call id, two returns ⇒ two Upsilons, `Unit` callee has no Phi/Upsilon,
   `ir_non_dominating_read` and `vc_cfg_detail` empty, `vc_close_loops` + `vc_unroll` still OK),
   `test_vc_worker.vow` `VOWRES2` round trip (truncated frame list ⇒ `None`). Green: `vc_inline.vow` recursive
   splice + frame table, `vc_worker.vow::vc_wire_encode/vc_wire_decode`, `vc_native.vow::VcOutcome`,
   `main.vow::native_report_result` (`call_site_*` from the chain).
5. **Targets without a `vow` block (caller-precondition role).**
   Red: `fail/inline_caller_precondition_role.vow` (unattributed helper division is not an obligation, the
   callee-`requires` violation is), exec test that `caller_pre_only` keeps only `CALLER_PRE`/`UNWIND` and
   still assumes callee `ensures`. Green: `vc_exec.vow::vc_claims_mode`/`vc_add_claim`,
   `vc_native.vow::vc_run_round` (`caller_pre_only = f.vows.len() == 0`).
6. **Owner attribution and budget.**
   Red: soft arithmetic inside a callee reports the callee as owner (`checked_arith_callee_attribution`
   stays one warning), and a tiny `cap` makes `vc_inline` return TOO_LARGE before materialising ⇒ the worker
   answers `unknown` (never `proven`; `tests/verify-native/tests.sh` with the fake `bitwuzla`: one
   self-contained query per claim including callee claims). Green: `vc_native.vow::vc_run_round` owner from
   `cs.owners[k]`, `vc_native.vow::vc_inline_budget_reason`, `main.vow` reason printing.
7. **Docs and registration** (no red test): ADR addenda, `scripts/concat_vow.sh` module list.
- Differential: `python3 scripts/verify_diff.py --vowc build/vowc --filter callee`, then all of `tests/verify*`
  (needs `bitwuzla` and ESBMC; otherwise record as not run). Key is `(fn, blame, vow_id)`
  (`verify_diff.py:81`); expect `match` on `callee_blame`, `caller_requires_*`,
  `callee_ensures_wrong_function`, `checked_arith_callee_attribution`.
- Gates: `bash tests/verify-native/tests.sh`; `build/vowc test compiler/` (or the touched files, small
  `--jobs`); `scripts/bootstrap.sh --skip-cargo --no-cache` on the final head SHA (record SHA +
  `scripts/seed.toml` pin); `VOW_FULL_TEST_SKIP_CARGO=1 scripts/full_test.sh` in the background with a
  done-marker file (~40 min); `python3 scripts/generate_operations.py --check` is unaffected. Use
  `VOW_CACHE_DIR=$(mktemp -d)` when validating codegen.

## Verification surface
- No source contracts change. New `compiler/*.vow` functions should stay contract-free or carry only true semantic contracts (e.g. `vc_frame_of` result `>= -1`); never cap collection sizes to suit ESBMC. Stage-1 bootstrap verifies `compiler/` with ESBMC, so keep new functions loop-simple and skip-able rather than distort anything.
- Properties the native verifier must establish on the new fixtures: caller `requires` at each inlined call; callee `ensures` per frame with the callee's own `requires` assumed after its claim; unwinding assertion unchanged for loops inside callees.
- No `tests/run/` or `examples/` growth; fixtures live in `tests/verify-native/`.

## Risks
- **Binary fixed point**: new module and `VOWRES2`; use `Vec` only (no `HashMap`), deterministic id assignment (depth-first, source order). Re-run bootstrap triple test.
- **Block/phi shape**: splitting a block that already feeds a Phi (Upsilons move to the continuation block) and a call as the last value before a Branch condition; covered by slice 3 plus `vc_cfg_detail` assertion.
- **Loop closing**: a call inside a loop makes callee values loop-defined; inline *before* `vc_close_loops` (done) and test slice 3 loop case.
- **Cost**: call-tree growth is inherent (ADR "Callee inlining"); the cap plus saturating pre-count bounds time/memory; a chain of N helpers is O(N·size) not O(N²) because only the final flat function is materialised.
- **Duplicate counterexample set**: callee is also its own verify target; same site reported twice must collapse (soft-site `vc_has_site` owner key is the callee id, keep).
- **`call_sites` change**: native now fills `call_sites` for callee-blame counterexamples and uses the exact frame chain; `verify_diff.py` compares only `(fn, blame, vow_id)`, so parity holds, but `counterexample.schema.json` description ("caller-blame failures") becomes narrower than behaviour — widen the wording in the P3 spec child, not here.
- **Multi-module `file`**: existing code reports `path` (root) for call sites; keep, flag in ADR addendum.
- **ESBMC demotion fidelity**: `caller_preconditions_only_source` (`c_emitter.rs:3061`) demotes every `__ESBMC_assert` except `vow:pre:` and the unsupported-op trap; the unwinding assertion is ESBMC-internal, not demoted. The native `caller_pre_only` mode keeps `CALLER_PRE` + `UNWIND`, which is the same set.
- **Environment**: ~8 vow-crate e2e tests SKIP-panic in the sandbox and `concrete-block-region-parity`/`u64_marker_propagation` pre-exist on clean main (see memory); verify against clean `origin/main` before blaming this change. `.github/workflows/bootstrap.yml` does not run on PRs, so the green-bootstrap claim must name the final head SHA.

## Out of scope
- Aggregate (struct/enum/`Option`/collection) arguments or results across calls; `Vec`/`String` models (#1423).
- Modular assume-guarantee verification, effects, recursion unrolling.
- Intermediate argument-value recovery for depth ≥ 2 `violating_args`.
- Rewriting `docs/spec/*`, `--help`/skill regeneration, any Rust crate change, emitter changes.
- Refactors or formatting of existing `vc_*` modules beyond the minimum signature additions.
