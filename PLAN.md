# Plan: automatic k-induction in the native verifier (#1425)

## Goal
When bounded unwinding of a loop is `open` (base case holds, unwinding claim `sat`), also run a
k-induction **step case** with `k` = the round's bound (2, 4, 8, 16, 32, 64). A function whose step
case is all `unsat` is `proven` for every trip count; a failing step case is never `failed`,
only "not proved". Works for loops with no usable invariant and, uniformly, for user invariants that
are k-inductive but not 1-inductive. The step case also exploits the loop conditions of the earlier
copies (they all took the back edge), so a plain counting loop with `ensures: result == n` closes
at k=2 with no invariant at all. Native verifier only (ADR-2026-10-08-1421): no Rust twin, no C
emitter change, no C-parity impact.

## Assumptions
- **Scope = functions whose loop-closed form has exactly one loop** (`n_loops == 1`, which
  also means no nesting, and no pointer header Phi). Multi-loop/nested functions keep today's verdict
  (bounded, plus #1422 invariant induction). Reason: with several loops the step case needs a
  summary of the loops before it; without invariants there is none. Listed as follow-up. (best guess)
- **"Growing k until the timeout"** is the existing internal schedule 2..64 sharing `--timeout`
  (ADR #1418 fixes the schedule; "not a flag"). A round that runs out of budget ends `timeout`/
  `unknown`, the schedule end ends `unknown` (`unwinding assertion: ...`). No new bounds.
  (best guess; widening the schedule is a perf decision for the #1610 harness, not this PR)
- **Verdict flips (stronger, allowed by ADR Rule 1/2).** Hand-checked against `S_2` (below):
  `unknown/loop_symbolic_bound`, `loop_literal_100`, `loop_invariant_too_weak` and `loop_infinite`
  become `Verified` and move to `pass/`. `loop_infinite` is partial correctness, exactly as pinned for
  invariants by `pass/loop_nonterminating_partial_correctness.vow` / ADR #1422 addendum (a loop that
  never exits has no exit claim); ESBMC k-induction also reports success there. `loop_bug_beyond_bound`
  (a real bug past the bound) must stay `unknown`: a sound step case cannot prove a property that a
  reachable state violates. (best guess; the maintainer can still veto `loop_infinite` with one
  predicate in `vc_step_attempt`, see Risks)
- **Property P** (what is inducted on) = every claim of the loop region: the implicit hard claims
  (div/rem zero and signed overflow, shift count, `unwrap` abort, callee `requires`/`ensures`
  frames), the target's loop `invariant`s, plus soft checked-arith claims (judged as in #1422:
  a `sat` soft claim in the step case is ignored). Post-loop `ensures` is a *goal* at exit from
  copy k, never a hypothesis. No invariant is synthesised.
- Soft (checked-arith) functions keep the #1422 policy: the step proof is recorded and the bounded
  rounds continue so warnings are the aborts of the last completed round; the proof is used where
  the schedule ends `unknown`.

## Key Files
| File | Role | Lines |
|------|------|-------|
| `compiler/vc_unroll.vow` | add step mode to the unroller; new `vc_unroll_step` | state 34-48, `vc_discover` 108-136, `vc_upsilon_phi` 184-211, `vc_clone_at` 215-244, `vc_unroll_capped` 253-335 |
| `compiler/vc_loops.vow` | receive `vc_havoc_of` / `vc_retag` moved from `vc_induct` (next to `vc_clone_inst`, avoids the `vc_inline -> vc_unroll -> vc_induct -> vc_inline` import cycle) | 419-433 |
| `compiler/vc_induct.vow` | call the moved helpers instead of its own | 391-406, 457 |
| `compiler/vc_exec.vow` | hypothesis blocks: claims suppressed, still assumed; target `invariant` claim-then-assume in step mode | `VcClaims` 75-94, `vc_add_claim` 113-137, `vc_walk_clause` 360-376, `vc_claims_mode` 429-505 |
| `compiler/vc_native.vow` | `vc_step_attempt`; per-round call; mutable `ind` | `vc_run_round` 155-193, `VcInduction` 200-215, `vc_verify_function` 251-296, header comment 18-43 |
| `compiler/vc_cfg.vow` | nothing expected (`vc_is_cut_end` already exempt at :41) — verify only | 41 |
| `compiler/tests/test_vc_unroll.vow`, `test_vc_exec.vow`, `test_vc_induct.vow` | unit tests | see slices |
| `tests/verify-native/{pass,unknown,fail}/*.vow` | new/moved fixtures | |
| `tests/verify-native/tests.sh` | wiring: query-count expectations change | 537-583 |
| `docs/adr/2026-10-08-1422-native-verification-semantics.md` | addendum (#1425); fix stale sentence in addendum #1418 ("automatic k-induction ... is still open") | end of file; "Addendum (#1418)" bullet 4 |
| `docs/spec/cli.md` | rewrite the **Loops** bullet (line 69) | 69 |
| `compiler/main.vow`, `vow/src/skill.rs`, `skills/vow/reference/cli.md` | **generated** from cli.md via `uv run python scripts/generate_help.py` (as #1612 did) | |

No change: `scripts/concat_vow.sh` (no new module), `vc_gate.vow`, `vc_cfg.vow`, `vc_inline.vow`,
`c_emitter.*`, anything in `vow-*` Rust crates except the generated `skill.rs`.

## Design (why it is sound, in four lines)
Step function `S_k` = the loop-closed function unrolled to bound `k` **from an arbitrary header
state**, with:
1. header instance with counter 0 (loop entry): each header Phi is a havoc constant
   (`VC_INTERNAL_HAVOC`, executor already models it); the entry Upsilons into it are dropped;
2. every instance of the loop with counter `< k` is a *hypothesis block*: claims are not asked
   but their goals are assumed (path-guarded, as `vc_guard` already assumes after a claim);
3. every loop-exit edge from a counter `< k` instance is cut (those runs are not "k consecutive
   iterations then look at the next state"; their post-loop claims are P-hypotheses, not goals);
4. counter-`k` instance: claims are real, exits are real, its back edge ends in `VC_INTERNAL_CUT_END`
   (no unwinding claim, nothing assumed).

Base + forward come from the round that is already run at the same bound (hard claims of copies
0..b `unsat` = base for P(s_0..s_{k-1}) since b = k; unwinding `unsat` = forward condition).
Step `unsat` for all claims ⇒ P holds at every state of every run ⇒ `proven`. Pre-loop values stay
real (only loop-carried state is havoc'd), which is strictly stronger than full havoc and still sound.

### Hand-checks of `S_2` for the existing `unknown/` fixtures
Earlier copies took the back edge, so `S_k` knows `cond(s_0..s_{k-1})`; exits at copies < k are cut.
- `loop_symbolic_bound` (`n >= 0`, `i < n`, `i = i + 1`, `ensures result == n`): `i_0 < n`, `i_1 = i_0+1 < n`,
  `i_2 = i_1+1` cannot wrap (`i_1 < n <= MAX`), exit `i_2 >= n` ⇒ `i_2 == n`: `unsat` ⇒ proven at k=2.
- `loop_literal_100`: same with 100 for `n` (u64) ⇒ proven at k=2.
- `loop_invariant_too_weak` (`invariant: i >= 0`): hypotheses `i_0 >= 0`, `i_1 >= 0`; claim `i_2 >= 0` holds
  (`i_1 < n` rules out wrap); exit gives `i_2 == n`. #1422's 1-induction cannot (exit from an
  arbitrary header state only gives `i >= n`) ⇒ **the acceptance-3 fixture**: 1-induction fails,
  k-induction closes it.
- `loop_infinite`: exit edge infeasible, `S_2` has no claim ⇒ proven (partial correctness).
- `loop_bug_beyond_bound` (`invariant: i <= n`, `i == 100` adds 2): `S_k` has a model `i_0 = 100-(k-1)`,
  `n = 101` violating the invariant at copy k ⇒ `sat` for every k ⇒ stays `unknown`.

## Steps

### 1. Move havoc helpers to `vc_loops.vow`
- **File**: `compiler/vc_loops.vow` after `vc_clone_inst` (419-433); `compiler/vc_induct.vow` 391-406, 457.
- **Change**: move `vc_induct_retag` and `vc_induct_havoc` verbatim, rename `vc_retag`, `vc_havoc_of`;
  `vc_induct_rebuild_inst` calls them. Behaviour-preserving; `test_vc_induct.vow` is the guard.
- **Reuses**: `VC_INTERNAL_HAVOC` (`vc_ops.vow:115`), `IOP_CONST_UNIT` Unit case.

### 2. Step mode in the unroller
- **File**: `compiler/vc_unroll.vow`.
- **Change**:
  - `VcUnrollState` gains `step: bool`; `VcUnrolled` gains `hyp: Vec<i64>` (one 0/1 per block of
    the result, sink last = 0; `Vec::new()` for plain unrolling; `vc_unroll_status` sets it).
  - `VC_UNROLL_NOT_APPLICABLE() -> 3`.
  - Refactor `vc_unroll_capped` body into `vc_unroll_build(f, bound, cap, step)`; `vc_unroll_capped`
    calls it with `false`; new `vc_unroll_step(f, bound)` (cap `VC_UNWIND_INST_CAP()`) calls it with
    `true`. In step mode return `NOT_APPLICABLE` unless `li.n_loops == 1` and
    no header Phi has `ty == ITY_PTR()` (same test as `vc_induct_ptr_phi`, which is in `vc_induct`;
    re-express locally over `f.blocks[li.headers[0]]`).
  - Single flat loop ⇒ an instance's counter is `key` itself (`key < r`; non-loop blocks key 0), so
    helpers `vc_step_in_loop(st, b)` = `vc_chain_len(li,b) == 1`, hypothesis(n) = in loop && `ikey[n] < bound`.
  - `vc_discover`: in step mode, an edge from an in-loop instance with `ikey < bound` to a block
    with `vc_chain_len == 0` is cut (`isucc = -2`), like a past-bound back edge.
  - `vc_clone_at`: step mode and header block and `ikey == 0` and `orig.op == IOP_PHI()` ⇒
    `vc_havoc_of(vc_clone_inst(orig, new_id, args, dv, dv2))`.
  - `vc_upsilon_phi`: step mode and the successor instance `t` is a header instance with
    `ikey[t] == 0` ⇒ `-2` (dropped; otherwise `vc_upsilon_detail` would reject "Upsilon to an
    unknown Phi").
  - Sink marker: `VC_INTERNAL_CUT_END()` in step mode, `VC_INTERNAL_UNWIND_SINK()` otherwise
    (line 325). Fill `hyp` while emitting blocks (line 309-322).
- **Reuses**: `vc_unrolled_cost`, `vc_edge_key`, `vc_chain_len`, `vc_loop_analysis`.

### 3. Executor: hypothesis blocks
- **File**: `compiler/vc_exec.vow`.
- **Change**: `VcClaims` gains `step: bool` (the function is a step function) and `hyp_now: bool`
  (the block being walked is a hypothesis block). `vc_claims_mode(f, fr, pre_only, hyp)`; `vc_claims`
  passes `Vec::new()`. In the block loop (485-502) set `cs.hyp_now = bi < hyp.len() && hyp[bi] == 1`.
  `vc_add_claim`: first line `if cs.hyp_now { return; }`. `vc_walk_clause`, frame < 0, non-REQ
  branch: after `vc_add_claim`, `if cs.step { vc_assume(cs, pc, goal); }` (invariant claim-then-
  assume: lets the exit continuation use it, and makes hypothesis invariants assumptions).
  Callee-frame clauses already assume after claiming; `vc_guard` already assumes — both are
  therefore correct in hypothesis blocks with no further change. Update the module header comment
  (37-45) with a "step function" paragraph.
- **Reuses**: `vc_assume`, `vc_guard`, `vc_is_havoc` branch (409).

### 4. Driver: step attempt per open round
- **File**: `compiler/vc_native.vow`.
- **Change**:
  - `vc_run_round(f, g, fr, hyp, sites, ...)` threads `hyp` to `vc_claims_mode`; both existing call
    sites pass `Vec::new()`.
  - New `vc_step_attempt(f, closed, fr, bound, started, timeout_ms, scratch) -> VcInduction
    [io, read, write]`: `vc_unroll_step`; status != OK ⇒ none; `vc_unrolled_detail(g, vc_ty_table(g))`
    non-empty ⇒ none (not an `ERROR`: a step shape that cannot be walked just proves nothing);
    `r = vc_run_round(..., u.hyp, Vec::new(), ...)`; `proves = !r.ended && !r.open`, `soft = r.soft`.
  - In `vc_verify_function`: `let mut ind`. After `if !r.open { return proven }` and
    `last_sites = sites`, `if !ind.proves { let st = vc_step_attempt(..., bound, ...); if st.proves {
    ind = VcInduction { proves: true, soft: st.soft || r.soft }; if !ind.soft { return
    vc_proven_with(sites); } } }`. Everything after reuses the #1422 handling unchanged
    (`vc_open_outcome`, `vc_undecided` fallback, `vc_merge_sites`).
  - Update the module header comment (18-43): "A function whose loops all carry..." paragraph gains
    the step case; mention it never yields `failed`.
- **Reuses**: `vc_proven_with`, `vc_open_outcome`, `VcInduction`.

### 4b. (only if the timeout measurement in Testing slice 5 shows it is needed) prefix as hypothesis
- **File**: `compiler/vc_unroll.vow` (`hyp` fill), same `hyp` channel.
- **Change**: in step mode also flag blocks outside the loop that dominate the header
  (`dom_dominates(li.nums, nb, b, header)`): their claims were already decided `unsat` by the base
  round of the same bound over identical code, so the step set neither re-asks nor loses them.
  Sound for the same reason the base round is a precondition of the step.

### 5. Fixtures, wiring tests, docs (see Testing for order)

## Testing — TDD slices (each: red test first, then the production edit)
All unit tests are `compiler/tests/*.vow` int-code mains run by `build/vowc test compiler/tests/<f>.vow`.
1. **Helper move** — `test_vc_induct.vow` unchanged and green (guard for Step 1).
2. **Unroller step shape** — `test_vc_unroll.vow`: `check_step_structure` on `count_up()` (copy the
   builder from `test_vc_induct.vow`): `vc_unroll_step(f, 2)` → `status OK`; 9 blocks; no unwind
   sink, exactly one `vc_is_cut_end` block; header instance 0 holds a `vc_is_havoc` call and no Phi;
   `upsilon_count == 3` (body0→h1, body1→h2, exit from h2); `hyp` has four 1s (h0,b0,h1,b1);
   `vc_unrolled_detail(g)` and `ir_non_dominating_read(g)` empty; deterministic print twice.
   `check_step_not_applicable`: `nested()` (existing builder), a loop-free function, two sequential
   loops, a loop with a ptr header Phi ⇒ `VC_UNROLL_NOT_APPLICABLE`. `check_step_too_large`:
   `vc_unroll_build(f, 4, 10, true)` ⇒ `TOO_LARGE`. Production: Step 2.
3. **Executor hypotheses** — `test_vc_exec.vow`: build `S_2` of a body with one `/` by a loop-carried
   `d` (IR builders as in existing tests): `vc_claims_mode(S, vc_frames_new(), false, u.hyp)` has
   exactly 1 `VC_CLAIM_DIV_ZERO` while the plain bound-2 unroll has 3 (dedupe-aware: assert
   `<= 1` vs `>= 2`), and `cs.assumed.len()` counts the hypothesis goals. Same with a header
   `invariant`: 1 `ENSURES` claim at copy k, and a post-loop `ensures` query contains the
   invariant as an assumption (`assumed_lens`). Production: Step 3.
4. **Native fixtures with real Bitwuzla** (`tests/verify-native/`, run by `full_test.sh` §4g; `which
   bitwuzla` is present locally). Expected status by directory: `pass`→Verified, `unknown`→
   VerifyFailed + `unwinding assertion` message + no counterexample, `fail`→counterexample.
   - `pass/loop_kinduction_alternating_divisor.vow` — no invariant: `acc = acc + 100 / d; d = 3 - d;
     i = i + 1` with `d` starting at 1, `requires: n >= 0`, a vow block so `pre_only` is off. Div-zero
     claim is 2-inductive (not 1-inductive: `d0 = 3` breaks k=1). Provable **only** by automatic
     induction (vc_induct is NA without invariants; unwinding is open for parameter `n`). Add
     `ensures: result == n` on the counter (now itself step-provable) so the vow block exists and
     the div claim is an obligation; keep the quotient live (e.g. fold `q - q` into the returned
     value) in case the checker rejects an unused local.
   - `pass/loop_kinduction_callee_requires.vow` — same shape but the divisor goes through an inlined
     callee `requires: d != 0` (callee-frame claim in hypothesis blocks).
   - `pass/loop_kinduction_invariant_not_one_inductive.vow` — **interaction with user invariants**:
     user invariant `a <= 1` for `a = b; b = 1` (b carried, starts 0), `ensures: result <= 1`:
     1-induction (#1422) fails, step k=2 proves (invariant used as hypothesis and goal).
   - `pass/loop_kinduction_inductive_invariant_unchanged.vow` — existing inductive invariant still
     proven by #1422 first (0 step rounds); covered by the existing `loop_unbounded_count.vow`, so
     only add the wiring assertion (below).
   - Moves `unknown/` → `pass/` (git mv, rewrite the header comment of each to name the step case):
     `loop_symbolic_bound.vow`, `loop_literal_100.vow`, `loop_invariant_too_weak.vow` (header: "1-induction
     of #1422 fails, k-induction closes it" — the acceptance-3 interaction fixture),
     `loop_infinite.vow` → `loop_infinite_partial_correctness.vow`. `unknown/loop_literal_100.vow`'s
     replacement note in ADR #1422 ("the `unknown/` fixture keeps the invariant-free loop") is stale.
   - `unknown/loop_kinduction_not_inductive.vow` — `d = d + 1` (wrapping), `i = i + 1`, `while i < n`,
     `100 / d`, `d` starting at 1, vow block `requires: n >= 0`, `ensures: result == n`. The only thing
     bounding `d` is its unstated relation to `i`; real runs hit `d == 0` after 2^64 steps (base never
     sees it) and `S_k` has `d_0 = -k` for every k (hand-check `S_64`: `d_j = -64 + j != 0` for j < 64,
     copy 64 divides by 0, with `i_0 = 0`, `n = 100`). Ends `unknown` with `unwinding assertion`,
     never `Verified`, never a counterexample. Together with the unchanged
     `unknown/loop_bug_beyond_bound.vow` this carries acceptance 2.
   - `fail/loop_kinduction_bug_at_iteration_40.vow` or reuse `fail/loop_bug_at_iteration_40.vow`:
     must still be a counterexample from the base round (order matters: base before step in a round).
   - A wrong invariant stays `fail/loop_unbounded_wrong_invariant.vow` (existing, unchanged).
   - `pass/loop_kinduction_break.vow`: the alternating-divisor loop with a `break` arm, so the
     exit-edge cut of copies < k is exercised on a body exit, not only on the header exit.
5. **Wiring (`tests/verify-native/tests.sh`, fake solver)**: re-measure and update the two
   expectations that change (`literal loop bounded rounds spawn nothing` 537-569, the `1` becomes
   one claim query + one per open round at bounds 2 and 4; update the comment). `guarded loop` (2),
   `loop unsat` (2), `invariant loop *` (3/…) must stay green: rounds that end on a first
   non-`unsat` claim never reach the step. Add: with `sat` and `unknown` modes a step failure
   yields the same verdict as before (no `failed` from the step). Optionally a fake mode that
   answers by discriminating the unwinding claim — only if a reliable discriminator exists in
   `vc_smt_render` output; otherwise rely on 4.
6. **Timeout measurement (before docs)**: run every remaining and new `unknown/` fixture with real
   Bitwuzla at the default `--timeout`, record wall-clock per fixture in the PR body. §4g requires
   `unwinding assertion` in `verify_message` for `unknown/`, so a fixture that now runs out of
   budget during the extra step rounds fails with a timeout message. Blocks only if it fires; the
   remedy is Step 4b, then (if still needed) limiting the step to the last two bounds.
7. **Perf harness**: `scripts/test_verify_perf.py:209` names `verify-native/unknown/loop_infinite`
   (truth `None` for `unknown/`); point it at `unknown/loop_bug_beyond_bound`. Re-run
   `scripts/verify_perf.py` (#1610) for before/after numbers.
8. **Docs** (after green): ADR addendum #1425 (+ fix stale text: addendum #1418 bullet "automatic
   k-induction for invariant-free loops is still open"; addendum #1422 "Order"/"Verdict impact"
   paragraphs and its `unknown/loop_literal_100` sentence; rule 1 stays as is), `docs/spec/cli.md`
   Loops bullet (line 69: drop "A function whose loops lack an invariant ... is decided by
   unwinding alone", add the step case, single-loop scope, never-`VerifyFailed`, partial
   correctness for any loop), the comment at `tests/verify-native/tests.sh:537-545` and
   `pass/loop_literal_100_invariant.vow` header; then `generate_help.py` and
   `uv run python scripts/check_help_coverage.py`.

Gate before claiming done (CLAUDE.md "green locally" rule): `scripts/bootstrap.sh --skip-cargo
--no-cache` on the final head SHA, record SHA (and the `scripts/seed.toml` pin once that file exists; it does not yet); `build/vowc test
compiler/`; `VOW_FULL_TEST_SKIP_CARGO=1 scripts/full_test.sh` (~40 min, background + poll);
`bash tests/verify-native/tests.sh`; `cargo test -p vow` (skill.rs regenerated);
`cargo clippy --all --all-targets -- -D warnings`; PR title `feat(verify): automatic k-induction ...`
lower-case subject ≤ 92 chars; `git rm PLAN.md` before opening the PR.

## Verification surface
- No contracts or C model change. The verifier is verified *by* ESBMC at bootstrap only for compiler
  functions that carry `vow` blocks; the new code adds none (matching `vc_unroll`/`vc_exec` style),
  so bootstrap verification cost is unchanged.
- Properties the native verifier must prove on the new fixtures: div-zero claim at copy k from
  hypotheses at copies 0..k-1 (QF_BV, 64-bit); invariant `a <= 1` at copy k, then `ensures`.
- `examples/` and `tests/run/` untouched (verifier-only change; no codegen).

## Risks
- **Soundness of dropping exits from copies < k.** Required: those paths are not "k iterations then
  next state" and their post-loop claims are P-hypotheses. Mitigation: unit test asserts the exit
  Upsilons/edges from copies 0..k-1 are gone and only copy k reaches the exit; a fixture with a
  `break` in the body (`loop_break_*` shapes) in `pass/` and one `unknown/` where the break path
  violates the `ensures` only for arbitrary states.
- **Base coverage.** Step with `k = b` uses base states s_0..s_{b-1}; the round at bound `b`
  covers s_0..s_b. Do not call the step before its base round, and never when the base round
  `ended`. Test: `fail/loop_bug_at_iteration_40` stays a counterexample (step cannot hide a real
  violation: the real prefix state satisfies the hypotheses).
- **Four `unknown/` fixtures flip to `Verified`** (see hand-checks). All are stronger verdicts
  ADR Rule 1 allows; `loop_infinite` is partial correctness (documented). If the maintainer
  objects to that one, the fix is a single predicate in `vc_step_attempt` (require a feasible
  exit). The flips are the main acceptance-1 evidence, plus the new `pass/` fixtures.
- **`unknown/` message check vs. timeout** (see Testing 6): extra step rounds before bound 64.
- **Cost.** An open round now also runs a step round (≤ 2x unroll + claims per round; literal-bound
  loops needing b=64 pay five failed step rounds first). Mitigation: step runs only on open rounds,
  shares the `--timeout`; re-measure with `scripts/verify_perf.py` (#1610) before the PR and note
  numbers in the PR body. If a regression shows, restrict the step to the last two bounds.
- **Soft-claim functions keep running to bound 64 after a step proof** (same as #1422), so their
  wall-clock is unchanged vs today's `unknown` path, not shorter.
- **Fixed point / bootstrap.** Only `compiler/vc_*.vow` change; no codegen path (`clif.vow`, ordering,
  `BTreeMap`) is touched. New code must use only Vow features Rust stage 0 compiles (no
  generics/closures). Run bootstrap twice and compare `sha256sum` of stage 1/2 (`scripts/bootstrap.sh`).
- **Import cycle** if the helper move is skipped: `vc_inline` already uses `vc_unroll`, and
  `vc_induct` uses `vc_inline`; hence Step 1.
- **Generated docs drift**: `compiler/main.vow` embeds `cli.md`; `check_help_coverage.py` and
  `scripts/full_test.sh` help-coverage will fail if `generate_help.py` is not re-run.
- **Verdict parity** (epic gate): verdicts only move `unknown` → `Verified`; a `fail`/`Verified`
  verdict a bounded round reaches is never changed (base runs first; an undecided or errored
  round cannot be overturned by a step proof except through the existing `vc_undecided` path).

## Out of scope
- Multi-loop and nested-loop k-induction (function with 2+ loops keeps bounded + #1422 verdicts); strengthening / invariant synthesis; widening the unwinding
  schedule past 64; changing `--timeout` semantics; collections (#1423); `contracts.md` rewrite
  (spec still describes the ESBMC default; ADR "rewrite plan" owns it); Rust compiler or ESBMC
  pipeline changes; the soft-claim "run all rounds" policy; refactors of `vc_native.vow` beyond the
  call-site threading.
