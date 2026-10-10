# Plan: per-claim slicing and per-clause verdicts in the native verifier (#1424)

## Goal
Make each native claim query carry only its cone of influence (CoI), and let the
native verifier decide *every* claim of a function so each `requires`/`ensures`/
`invariant` clause gets its own status, in the vocabulary `contracts-result.schema.json`
already defines. Part of epic #1398 (P3). Native-verifier-only: `compiler/` (no Rust twin,
ADR-2026-10-08-1421). Blocker #1418 (unwinding) is closed and merged.

## Assumptions
- Scope is the verifier core, not the CLI: `vowc contracts --verify` on native is #1427
  (open) and the result cache is #1429. This PR delivers the per-clause *API*, its worker
  wire encoding, and the slicer; #1427 consumes them. (best guess; the issue lists no CLI
  surface and the epic splits it out.)
- #1427 (`contracts --verify` on native) is blocked by this issue and owns the command and
  its JSON, so step 6's worker flag and wire v3 are the plumbing it will consume; they are
  kept here because "report each clause with its own status" is meaningless unless the clause
  list can leave the worker. (best guess; if #1427's owner prefers, steps 5-6 can be the
  last commits and dropped without affecting steps 1-4.)
- Slicing is *proof-direction sound by construction*: a sliced query is a relaxation
  (fewer conjuncts), so `unsat(sliced) => unsat(full)`. Any non-`unsat` answer on a
  sliced query is re-asked on the full query, which alone decides. Hence slicing can never
  change a verdict and counterexamples stay byte-identical to today's. The alternative
  (trusting `sat` on the sliced query) was rejected: a dropped assumption set may be
  unsatisfiable (`requires: x > 0 && x < 0` on a variable unrelated to the claim), which
  today yields `proven` (vacuous) and a naive slicer would flip it to `failed`.
- A clause is `proven` only when its claims are `unsat` in a round whose unwinding claim is
  `unsat` too (ADR-2026-10-08-1422 rule 2). Otherwise it falls back to the function's overall
  outcome through the existing `resolve_clause_status` (`compiler/verifier.vow:1182`),
  which is also what makes `requires` clauses (assumptions, never claims) behave exactly as
  they do under ESBMC `--multi-property`.
- The overall result of the all-claims mode equals the first-failure mode's result (first
  non-proven hard claim in claim order); only the clause list is extra. Rounds continue
  past a failure while the unwinding claim is open, so other clauses can still be proven.
- `VOW_VERIFY_NO_SLICE=1` is a new *test-only* env (same status as
  `VOW_VERIFY_WORKER_MEM_KB`, ADR-2026-10-08-1430 table); it exists so the corpus gate
  "slicing never changes a verdict" can run both ways. Not a user flag, no `--help` change.

## Files to touch
| File | Role | Lines of Interest |
|------|------|-------------------|
| `compiler/vc_exec.vow` | `VcClaims`, `vc_add_claim`, `vc_claim_script` (full prefix query), `vc_walk_clause` | 72-91, 110-134, 154-167, 357-373 |
| `compiler/vc_term.vow` | arena; `named[v]` = definition term of a bound var, `named[t]` = var of a composite | 80-100, 200-210, 246 |
| `compiler/vc_smt.vow` | `SmtScript`, `vc_script_prefix`, writer; header = declare-const / define-fun | 14-62, 245-282 |
| `compiler/vc_flow.vow`, `vc_agg.vow` | the only other `define-fun` / `declare-const` emitters | vc_flow.vow:62-73, vc_agg.vow:130-140 |
| `compiler/vc_native.vow` | `VcOutcome`, `vc_check_claim`, `vc_run_round`, `vc_verify_function` | 36-44, 85-89, 142-176, 178-216 |
| `compiler/vc_cex.vow` | `vc_claim_result`, `vc_proven_result`; clause kinds | 33-93 |
| `compiler/verifier.vow` | `MultiPropertyResult`, `resolve_clause_status` (reuse, do not fork) | 67-82, 1182-1215 |
| `compiler/vc_worker.vow` | wire format `VOWRES2`, `vc_worker_argv`, `VcPool` | 56-170, 260-310 |
| `compiler/main.vow` | `run_verify_worker` (flag parsing, `vc_verify_function` call) | 1913-1934 |
| `scripts/concat_vow.sh` | module order list used by the bootstrap triple test | 23 |
| `tests/verify-native/tests.sh` | fake-`bitwuzla` wiring tier (records every query) | 18-60, 249-300 |
| `scripts/full_test.sh` | Section 4g native tiers | 1421-1565 |
| `docs/adr/2026-10-10-2041-term-simplifier-trusted-base.md` | template for the new ADR (trusted-base wording) | whole |

## Steps (TDD slices, in order; each is one commit and keeps `build/vowc test compiler/` green)

### 1. Slicer core: `vc_slice` (pure, solver-free)
- **Test first**: `compiler/tests/test_vc_slice.vow` [new], helpers copied from
  `test_vc_exec.vow:1-60` (`one_block`, `mk_vow`, `query`, `count_of`). Cases:
  (a) two independent value chains (`a`-only and `b`-only `ensures`): sliced query for the
  `a` clause contains no define-fun/assert mentioning `b`'s chain, strictly fewer commands
  and bytes than `vc_claim_script`; (b) a clause over both params keeps everything
  (`dropped == false`, text equals `vc_claim_script`); (c) transitive reach through a
  `define-fun` chain and through a shared path-condition name (`g0`); (d) an assumption over
  unrelated params is dropped; (e) every `declare-const` and the `get-value` line survive
  (counterexample model stays complete); (f) kept commands preserve original order
  (definitions precede uses); (g) prefix respect: assumptions/definitions created *after*
  claim `k` never appear in its slice.
- **Change**: `compiler/vc_slice.vow` [new], `use vc_exec, vc_term, vc_smt`. API:
  `struct VcSlice { cmds: Vec<i64>, assumed: Vec<i64>, dropped: bool }`,
  `vc_slice_claim(cs: VcClaims, k: u64) -> VcSlice`,
  `vc_claim_script_sliced(cs, k) -> SmtScript` (same shape as `vc_claim_script`).
  Algorithm: build once per `VcClaims` an inverted index var-term-id -> assumption
  indices (linked parallel `Vec<i64>`, deterministic, no `HashMap` iteration) and, from
  `named[terms[i]]` of each header `define-fun`, var -> command index. Per claim: iterative
  worklist (explicit stack, no recursion) over leaves of `negated_goals[k]`; a bound leaf
  (`named[v] >= 0`) keeps its `define-fun` and enqueues the leaves of its definition; an
  assumption with index `< assumed_lens[k]` that mentions a reached var is kept and its
  leaves enqueued; term-level visited marks use an epoch counter so no per-claim clearing
  (O(cone), not O(arena)). Always keep `set-logic`, `set-option`, every `declare-const`.
- **Reuses**: `vc_script_prefix`, `vc_script_assert`, `vc_script_get_value`
  (`vc_smt.vow`), `named` bind convention (`vc_term.vow:200`), `dom_fill` (`ir_dominance`).
- **Register**: add `vc_slice` after `vc_exec` in `scripts/concat_vow.sh:23`.

### 2. Confirm-on-non-unsat in the claim driver
- **Test first**: extend `tests/verify-native/tests.sh` (fake solver). New fake mode
  `sat_if_contains` (env `FAKE_BW_NEEDLE`): `sat` + model when the query file contains the
  needle, else `unsat`. Assertions: (i) all-`unsat` fixture with independent clauses:
  exactly one query per claim, each strictly smaller than the unsliced bytes recorded with
  `VOW_VERIFY_NO_SLICE=1`; (ii) a claim whose sliced query is `sat` is re-asked once on the
  full query and the verdict/counterexample equal the `NO_SLICE` run byte for byte;
  (iii) fixture with a contradictory `requires` on an unrelated param stays `Verified`
  (the vacuous-proof case that a trusting slicer would flip).
- **Change**: `compiler/vc_native.vow` `vc_check_claim` (line 85): when
  `vc_slice_claim(...).dropped` and slicing is enabled, query the sliced script; return the
  answer only on `VC_ANS_UNSAT`; otherwise query `vc_claim_script` with the budget minus
  elapsed (`time_micros`) and return that answer. Slicing switch is read once per function
  in `vc_verify_function` (`getenv("VOW_VERIFY_NO_SLICE")`) and threaded through
  `vc_run_round`.
- **Fixtures** [new]: `tests/verify-native/pass/slice_independent_clauses.vow`
  (`fn f(a: i64, b: i64) -> i64`, three `ensures`, two disjoint chains, `requires` on each
  param), `tests/verify-native/pass/slice_vacuous_unrelated_requires.vow`,
  `tests/verify-native/fail/slice_one_clause_wrong.vow`. Header directives follow the
  existing `// TEST:` convention in that directory.

### 3. Clause accumulator (pure seam)
- **Test first**: `compiler/tests/test_vc_clause.vow` [new]: unsat/sat/unknown folds per
  `vow_id`; a clause repeated per unrolled iteration (same id, many claims) is `failed` if
  any copy is sat, `proven` only if all copies unsat **and** the round is closed; callee-frame claims (`frame >= 0`) never contribute to a
  target clause; output feeds `resolve_clause_status` and yields only
  `proven|failed|unknown|timeout|error`. Vocabulary drift guard lives in `tests.sh`
  (step 5): it reads the `status` enum out of
  `docs/spec/schemas/contracts-result.schema.json` with `python3 -c json` and asserts every
  clause status the worker emits is a member. The vocabulary itself is guaranteed by reusing
  `resolve_clause_status`; end-to-end JSON schema validation of `contracts --verify` output
  belongs to #1427 (blocked by this issue, owns the command).
- **Change**: `compiler/vc_clause.vow` [new]: `VcClauseAcc { ids, proven }`,
  `vc_clause_note(acc, vow_id, verdict)`, `vc_clause_finish(acc, target_ids, round_closed)
  -> (ids, proven)`; no solver, no I/O.
- **Register**: `vc_clause` before `vc_native` in `scripts/concat_vow.sh:23`.

### 4. Clauses with no claim are decided from the pre-inline IR, not from claims
- **Test first**: in `compiler/tests/test_vc_clause.vow` / `test_vc_exec.vow`: (a) an
  `ensures` folded away by `vc_add_claim` (goal implied by an assumption), (b) an `ensures`
  in a block pruned by `vc_flow_enter` or by a decided branch, and (c) an invariant elided in
  one unrolled copy each end up in the target-clause id list with **zero claims**, so with a
  failing sibling they read `proven` in a closed round, not `unknown`. Claim counts asserted
  by the #1611 tests are unchanged.
- **Change**: no `VcClaims` change. `vc_clause` takes the target's clause ids scanned from the
  *pre-inline* function `f` (`IOP_VOW_ENS` / `IOP_VOW_INV` insts whose id is a target vow id,
  `frame < 0`; `IrVowEntry.blame` Caller ids are `requires` and are excluded). Rule: a
  target ensures/invariant with zero claims in a closed round is `proven` (every one of its
  claim points was decided statically or is unreachable); `requires` clauses are never
  given a verdict and fall back to the overall outcome exactly as under ESBMC
  `--multi-property`. This subsumes recording elided ids and avoids touching `vc_add_claim`.

### 5. All-claims mode and `VcOutcome` clause fields
- **Test first**: `tests/verify-native/tests.sh`, fake solver, driving the worker directly
  (`vowc verify-worker ... --worker-clauses`, see step 6 for the flag): (a) two ensures, only the
  second is `sat`: clause list = first proven, second failed, overall = first-failure result
  and identical to default mode; (b) a failing hard abort claim (`vow_id -1`, e.g. `/` by an
  unguarded param) leaves ensures verdicts intact and overall `failed`; (c) solver
  `timeout`/memory stops the schedule and undecided clauses are absent (so they resolve to
  `timeout`/`unknown` through `resolve_clause_status`); (d) loop fixture: clause is
  `proven` only after the closing round, `unknown` when the last bound is still open, and
  `failed` when any iteration copy is sat; (e) a dead-branch `ensures` next to a failing
  sibling is `proven`; (f) every status string emitted is in the schema's enum (read from
  `docs/spec/schemas/contracts-result.schema.json`).
- **Change**: `compiler/vc_native.vow`: `VcOutcome` gains `clause_ids`, `clause_proven`
  (empty in default mode; `vc_bare_outcome` sets empties); `vc_run_round` takes
  `all_claims: bool`: a hard non-`unsat` claim is recorded and the loop continues instead of
  returning, except halting answers (`timeout`, `memory`, `not_found`, `error`) which end
  the round; `vc_verify_function_mode(f, m, timeout_ms, scratch, all_claims)` is the
  engine, `vc_verify_function` stays a thin wrapper with `all_claims = false` so
  `main.vow:1933` and every existing test keep compiling. `VcRound` carries the first
  failing `VerifyResult` so overall == first-failure behaviour.
- **Reuses**: `vc_claim_result` (`vc_cex.vow:54`), `resolve_clause_status`
  (`verifier.vow:1182`) for the final per-clause string; no new status vocabulary.

### 6. Carry clauses across the worker boundary
- **Test first**: `compiler/tests/test_vc_worker.vow`: wire round-trip with clauses;
  truncated clause section decodes to `None` (never a partial proof); `vc_worker_argv`
  adds `--worker-clauses` only when the pool is in clause mode.
- **Change**: `compiler/vc_worker.vow`: tag `VOWRES2` -> `VOWRES3`, append `n` then
  `(vow_id, proven)` pairs after the chain section in `vc_wire_encode`/`vc_wire_decode`;
  `VcPool` gains `clauses: bool` (default false in `vc_pool_new`); `vc_worker_argv` pushes
  `--worker-clauses`. `compiler/main.vow:1913` `run_verify_worker` passes
  `has_flag(argv, "--worker-clauses")`; confirm the flag is valueless so
  `get_source_path_sub` (`main.vow:86`) skips any `-`-prefixed token and only value-taking
  flags need `cli_flag_takes_value` (ADR-1430 section 2), so no list edit is needed.
- **Wiring test**: `tests/verify-native/tests.sh` runs `vowc verify-worker <fixture>
  --worker-index N --worker-name F --worker-budget-ms 60000 --worker-clauses` under the
  fake solver and greps the clause section.

### 7. Corpus parity gate and ADR
- **Change**: `scripts/full_test.sh` Section 4g: when `bitwuzla` is present, for every
  `tests/verify-native/{pass,fail,unknown}/*.vow` run `verify --backend native` twice (default and
  `VOW_VERIFY_NO_SLICE=1`) and require identical stdout JSON and exit code; reuse the loop
  at lines 1448-1565 rather than a new walker.
- **Recorded local run** (not CI, wall-clock heavy): the epic's corpus is broader than
  `verify-native`. Run the same both-ways comparison over the native-eligible files of
  `tests/verify`, `tests/verify-fail`, `tests/verify-skip` (take the file list from
  `scripts/verify_diff.py`'s selection) and the benchmark references, and record the
  fixture count and "0 differences" in the PR body.
- **Docs**: `docs/adr/2026-10-11-1200-query-slicing-and-per-clause-verdicts.md` [new]:
  decision (slice-then-confirm, why not trust sliced `sat`, clause fold rules, elided
  clauses, worker wire v3), *Trust* section stating the slicer is **not** in the trusted base
  for `sat` and that its only trusted property is "kept commands are a subset of the full
  prefix" (the proof direction), and the cache note for #1429: the key must be the
  query whose answer was used (the sliced query for an `unsat`, the full query for any other
  verdict), never always the sliced one. Add the `VOW_VERIFY_NO_SLICE` row to the env table in
  `docs/adr/2026-10-08-1430-native-verifier-cli-and-status-surface.md` (additive addendum).
- No `docs/spec/*.md` change: no syntax, flag, schema or user-visible status changes
  (contracts.md/cli.md are rewritten by #1430 once `contracts --verify` is on native).
  `generate_help.py` need not run.

## Verification surface
- Properties ESBMC must prove: none; no `.vow` contract changes. New Vow code in
  `compiler/` is checked by the bootstrap's verify step like the rest; keep new functions
  loop-bounded and non-recursive so they stay inside the verifiable subset (epic criterion 4).
- New fixtures: the three `tests/verify-native/*` files above (pass/fail directories are
  walked by `full_test.sh` 4g with real Bitwuzla; `tests.sh` covers fake-solver wiring).
- Acceptance mapping: "statuses match schema" -> step 3 enum test + step 5/6 wiring;
  "never changes a verdict" -> steps 2 and 7 (by construction + corpus run both ways);
  "query size drops" -> step 1 unit assertion and step 2 recorded-query byte comparison.
  Record the measured byte/command reduction on the multi-clause fixture in the PR body.

## Testing commands
- `build/vowc test compiler/tests/test_vc_slice.vow` (then `test_vc_clause`, `test_vc_exec`,
  `test_vc_worker`), then `build/vowc test compiler/`.
- `VOWC_BIN=build/vowc bash tests/verify-native/tests.sh`.
- `scripts/bootstrap.sh --skip-cargo --no-cache` on the final head SHA (record SHA and
  `scripts/seed.toml` pin per CLAUDE.md) and `scripts/full_test.sh` Section 4g. `python3
  scripts/verify_perf.py --filter <fixture>` for a before/after timing note (cache off).
- Background long runs and poll their done-marker (bootstrap ~5 min, full_test ~40 min).

## Risk areas
- Term-level semantics: the slicer must treat `named[v]` correctly (bound var vs composite
  back-pointer). Mitigated by tests (c)/(f) and by the confirm step: a wrongly dropped
  assumption can only cost a second query, never a wrong verdict. A *malformed* sliced
  query (use of a dropped definition) would surface as solver `error`, so test (f) asserts
  closure and a debug self-check in tests renders the sliced script and re-parses names.
- Double query on every non-`unsat` answer with something dropped (soft `sat`, e.g.
  `checked_add_reachable`): measured against the `verify_perf.py` 1.25x per-fixture gate;
  if it binds, add a cheap pre-test "dropped part is ground/trivially satisfiable" rather
  than trusting `sat`.
- Binary fixed point: new modules must appear in `scripts/concat_vow.sh` in dependency
  order; no `HashMap` iteration or address-dependent ordering in the slicer; vectors only.
- `VcClaims` field additions touch every struct literal (`vc_claims_mode` is the only
  constructor; `test_vc_*` build through it).
- Wire v3 breaks any stale `VOWRES2` worker/parent pair: both are the same binary, and
  `vc_wire_decode` returns `None` (-> `panicked`, never `proven`) on a tag mismatch.
- Pool early-stop (`vc_pool_finish` cancels tasks above a non-proven one) is correct for
  `verify`; clause mode (#1427) will need per-function independence. Left unchanged here;
  noted in the ADR.
- Budget: a sliced query that times out leaves the confirming full query no time, so a
  borderline `unsat` can become `timeout`. Accepted: the full query is never harder than the
  sliced one in practice (it is a superset only in conjuncts the solver can ignore), and the
  shared function budget is unchanged. If the perf harness shows otherwise, cap the sliced
  attempt at half the remaining budget.
- Untouched gates: no printer/AST change, so `parse -> print -> parse` idempotency is
  unaffected; no Rust change, so `cargo clippy --all --all-targets -- -D warnings` and the
  verifier C-parity rule (`c_emitter.{rs,vow}`) are unaffected.

## Out of scope
- `contracts --verify` / `test --verify` on native (#1427), vacuity and weakness probes.
- Result cache (#1429), incremental solving, k-induction, Z3 cross-check, float theory.
- Interval domains or any change to the simplifier/term arena.
- Any change to `--help`, skill text, `docs/spec/*`, JSON schemas, or the Rust compiler.
- Refactors of `vc_native.vow`/`vc_exec.vow` unrelated to the above.
