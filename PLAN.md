# Plan: native backend for `vowc contracts --verify` and `vowc test --verify` (#1427)

## Goal
Make `vowc contracts --verify --backend native` and `vowc test --verify --backend native` run on the
native verifier (epic #1398), including the two weak-contract probes (`vacuous`,
`trivially_satisfiable`), with the status vocabulary and exit codes unchanged and per-clause agreement
with ESBMC on the `tests/verify*` corpus. Self-hosted `vowc` only (scoped exception in CLAUDE.md,
ADR-2026-10-08-1421): no Rust twin, no C-emitter change, default backend stays `esbmc`.

Classification: **Large** (new CLI surface, pool semantics, two new IR probe transforms, two commands,
spec/ADR, differential harness). Explored inline (no Explore subagents were available to this run).

## Assumptions
- `--backend native` on `contracts`/`test` **without `--verify`** is a usage error (exit 1, message
  names `--verify`), not a silent no-op: otherwise an agent reads `not_verified` as "native ran". Other
  prover flags are silently accepted without `--verify` today, but those are tuning knobs, this selects
  the verifier. Alternative (accept and ignore) rejected for fail-closed reasons. (best guess)
- `test --verify` keeps ESBMC's function selection: only functions with `vows.len() > 0`
  (not `is_verify_target`, which also covers callers of contracted functions); `verify` keeps its own.
  Agreement with ESBMC on `test` is the acceptance criterion. (best guess)
- `test --verify` verification budget under native is the fixed 300 s per function (`watchdog_ms_for("")`).
  `test --timeout` is the per-test execution timeout in ms and is unrelated (ADR-1430 flag table). (ADR)
- Weak-contract probes are **separate pool tasks** (own worker process, own `--timeout` budget), not
  extra work inside the main worker: a slow probe must never turn a finished main verdict into
  `timeout` because the parent kills the whole worker group at the budget. Costs one extra frontend
  re-lowering per probe; probes only run for functions that pass the cheap IR applicability test. (best guess)
- Under native, every non-`proven` per-function result fails `test --verify` (`verify_failed`), including
  `error`/`panicked`; the ESBMC path in `run_test` treats `VERIFY_ERROR` as pass (fail-open). Native
  must be fail-closed, so this is an intentional, documented divergence. (best guess)
- Native `contracts --verify` with Bitwuzla missing marks only clauses of functions that need the
  solver `error` (functions skipped or without claims still resolve without one), unlike ESBMC which
  marks everything `error`. Same shape as `verify --backend native` today. Exit code is 1 either way
  once any clause is `error`/`skipped`. (best guess)
- `ArithOverflowReachable` warnings, counterexamples and `--replay-cex` are not part of the
  `contracts`/`test` surfaces today and stay out. (scope)

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `compiler/main.vow` | `backend_flag_error` (3996), `run_contracts` (3692, ESBMC branch from 3786), `run_test` verify block (2477-2640), `test_worker_common_flags` (1334), `run_verify_worker` (1917), `native_skip_function` (1820) | see left |
| `compiler/cli_flags.vow` | `cf_is_worker_value` (l.42) must learn the two new worker flags | 42-44 |
| `compiler/vc_worker.vow` | `VcPool` (305), `vc_pool_new` (322), `vc_worker_argv` (359), `vc_pool_finish` early stop (397-415) | |
| `compiler/vc_native.vow` | `vc_verify_function_mode` (381) engine, reused unchanged | |
| `compiler/vc_clause.vow` | `vc_clause_targets` (104): clause ids == `IrVowEntry.id` | |
| `compiler/verifier.vow` | reuse `resolve_clause_status` (1182), `function_has_requires` (1332), `function_has_ensures` (1388), `returns_scalar` (1411), `body_replaceable_result` (1419), `watchdog_ms_for` (518) | |
| `compiler/vc_exec.vow` | `vc_walk_clause` (410), `vc_add_claim` (117): probe claims come out of existing machinery | |
| `compiler/vc_gate.vow` | `vc_const_ty_ok` (168) = const opcode per type, inverse needed for the default constant (`compiler/vc_gate.vow`) | |
| `compiler/c_emitter.vow` | READ ONLY reference for probe semantics: `vow_reach` after last `requires` of block 0 (3140-3165); body-replace of the single `Return` value (3070-3170) | |
| `compiler/vc_probe.vow` [new] | pure IR transforms for the two probes | |
| `compiler/vc_driver.vow` [new] | run one module's per-function native checks (verify + probes) through the pool; shared by `contracts` and `test` | |
| `docs/spec/cli.md` | `--backend` row (54), "Native backend" (58-77), `contracts` options/exit code (84-100), `test` section | |
| `docs/adr/2026-10-08-1430-native-verifier-cli-and-status-surface.md` | flag table; add an addendum line | |
| `scripts/concat_vow.sh` | `FILES=(...)` list (line 23): add `vc_probe vc_driver` after `vc_worker`, before `mutants_oracle` | |
| `scripts/verify_diff.py`, `scripts/test_verify_diff.py` | gain `--command verify\|contracts\|test` | |
| `tests/verify-native/tests.sh` | wiring tier (fake Bitwuzla); `usage_error` lines 799-801 flip | |
| `scripts/full_test.sh` | Section 4g (1421-) real-solver fixtures | |

## Design

### Weak-contract probes as IR rewrites (no executor change)
Both probes are `IrFunction -> IrFunction` rewrites fed to the *existing* engine
(`vc_verify_function_mode`), so slicing, inlining, loops, induction and the solver pipeline are reused
and the executor/term simplifier (trusted base, ADR-2026-10-10-2041) is untouched.

1. **Vacuity probe** (`vc_probe_vacuity(f, m) -> IrFunction`). Semantics: "the point after the
   function's last `requires` is unreachable". Applicable iff the function has an `IOP_VOW_REQ`.
   `lower_requires_clauses` (`compiler/lower.vow:5731`) lowers each predicate with `lower_expr`, and
   `&&`/`||` lower to branches, so for `requires: a > 0 && b > 0` the `VOW_REQ` lands in a join block,
   not block 0. Rewrite: find the block B* holding the last `IOP_VOW_REQ` in reverse-postorder, truncate
   B* right after it, append `CONST_BOOL false` (fresh value id) and `IOP_VOW_ENS` on it with a fresh vow
   id (max id over the module's vows + 1) whose `IrVowEntry` is appended to `vows` (keeps
   `f.vows.len() > 0`, so `pre_only` stays false, and gives `vc_cex` a descriptor), then
   `IOP_UNREACHABLE`; drop every block no longer reachable from the entry (the body). Phis/Upsilons
   of kept blocks are untouched.
   **ESBMC divergence to pin in slice 3 (esbmc is on this host):** `c_emitter.vow:3140-3165` plants
   `vow_reach` only in `first_block_id`. When the last `requires` is in a later block ESBMC emits no
   label; run `--error-label vow_reach` on such a fixture (`requires: a > 0 && b > 0` with and without a
   contradiction) and record what ESBMC does (never-vacuous, or a spurious `SUCCESSFUL` ⇒ falsely
   `vacuous`). Native stays on the semantic definition above; the observed ESBMC behaviour is documented
   under "Verdict divergence from ESBMC" and the fixture is labelled in the diff allow-list.
   Verdict: whole-function `VERIFY_PROVEN` ⇔ `requires` jointly unsatisfiable ⇔ vacuous. Anything else
   (failed/unknown/timeout/error/not found) ⇒ not vacuous (never claim vacuity on an undecided probe,
   same one-sidedness as `verify_function_vacuity`). Slicing is safe: the claim's cone is empty, a sliced
   `sat` is re-asked on the full query (ADR-2026-10-11-1200 rule 1).
2. **Trivial probe** (`vc_probe_trivial(f) -> IrFunction`). Applicable iff `function_has_ensures(f)`
   && `returns_scalar(f)` && `body_replaceable_result(f)` (single `Return`, value not a `GetArg`).
   `R` may be a **Phi** (the usual single-exit shape, `if c { x } else { 0 }`): Upsilons name their Phi
   target by `dv`, so `R`'s definition and id are never moved or renamed. Rewrite: insert a fresh
   default constant `C` of `R`'s type immediately after `R`'s definition (after the leading Phi group of
   its block when `R` is a Phi, so Phis stay first), then replace `R` by `C` in the `args` of every
   instruction (the `Return` operand, the `ensures` predicate operands, Upsilon sources); Upsilon `dv`
   targets are left alone. `R`'s own computation stays in the function, so its abort claims survive
   (the C model keeps `emit_inst` and overwrites afterwards: `c_emitter.vow:3165`, the `Phi` emits
   nothing and the `v<R> = 0` lands at its position, so ESBMC zeroes Phi results too).
   Const opcode by type is the
   inverse of `vc_const_ty_ok` (`I64→CONST_I64`, `U64→CONST_U64`, `I32/U32/I16/U16→CONST_I32`,
   `I8/U8→CONST_U8`, `I128→CONST_I128`, `U128→CONST_U128`, `Bool→CONST_BOOL`), `dv = dv2 = 0`.
   Verdict: whole-function `VERIFY_PROVEN` ⇒ `trivially_satisfiable: true` on every `ensures` clause of
   the function (matches `verify_function_trivial`: `PROVEN`/`PROVEN_IR`). Other ⇒ false.
3. Precedence in `contracts` (identical to the ESBMC path): vacuity overrides all of the function's
   clause statuses with `vacuous`; the trivial flag is set independently.

### Pool: per-function independence and task kinds
`vc_pool_finish` cancels every task above the first non-proven one — right for `verify`, wrong for a
clause report across functions (ADR-2026-10-11-1200, "Pool early stop", which assigns this to #1427).
Add to `VcPool`: `halt: bool` (default `true`, consulted in `vc_pool_finish`), `kinds: Vec<i64>`
(`VC_TASK_KIND_VERIFY`=0, `_VACUITY`=1, `_TRIVIAL`=2, default all 0) and `module_root: String`
(default ""). `vc_worker_argv` appends `--worker-probe vacuity|trivial` and
`--worker-module-root <dir>`; `--worker-clauses` is emitted only for kind 0 (probe workers run
`all_claims=false`) (the module root because `run_test` lowers with an explicit module root while
`run_verify_worker` lowers with ""). `run_verify_worker` applies the probe rewrite to the planned
function and runs `vc_verify_function_mode(g, fr.ir_mod, budget, scratch, all_claims=false)`; the wire
format (`VOWRES3`) is unchanged: parent reads `result.status` only.

### Driver (`compiler/vc_driver.vow`, deep module)
`vc_driver_run(exe, path, module_root, ir_mod, only_vows, probes, halt, jobs, budget_ms) -> VcDriverRun`
returns, per function index: `skip_reason` (from `vc_module_skip_reason`), `ran`, `outcome: VcOutcome`
(`clauses` mode on for contracts), `vacuous`, `trivial`. It builds the task list (verify task per
non-skipped function; probe tasks only when `probes` and the applicability tests pass), drives
`vc_pool_step` until every launched task is done, aborts the pool, and returns. `contracts` and `test`
only interpret the result; neither touches the pool.

### `contracts` (self-hosted `run_contracts`)
A separate native block is added; the existing ESBMC block stays diff-identical (its guard becomes
`if do_verify && !native {`, the old `esbmc_bin` lookup is skipped for native). No re-indentation of
existing lines. (`codecov.yml` ignores `compiler/` and `scripts/`, but the diff stays reviewable.) Same entry arrays; only the status/trivial
fill differs: skipped → `skipped`; else `resolve_clause_status(vow_id, oc.clause_ids, oc.clause_proven,
oc.result.status, false)` (the multi-property vocabulary, `tainted=false` because there is no ESBMC taint
class natively); vacuity ⇒ `vacuous`; trivial ⇒ `e_trivial`. JSON building, summary and exit-code code
stay as is (the `proven-ir` mention there is left for P6). No stderr chatter (ESBMC path prints none).

### `test` (self-hosted `run_test`)
The existing block's guard `if do_verify {` becomes `if do_verify && !native {` (no re-indent of the
~160 existing lines) and a sibling `if do_verify && native { ... }` block follows it. Per file: one
`vc_driver_run(only_vows=true, probes=false, halt=true)` with the file's `module_root`;
any ran function with status != `VERIFY_PROVEN` ⇒ `verify_failed` entry (same JSON as today); else any
skipped function ⇒ `contract_skipped` entry (same JSON, each skip also emitted via the existing
`native_skip_function` warning); else proceed to compile/run. Add `--backend` to
`test_worker_common_flags` or parallel test workers silently fall back to ESBMC.

## Steps (TDD slices, each a separate commit; all red-first)

### 1. CLI surface
- **Test**: `compiler/tests/test_cli_flags.vow` — `--worker-probe`/`--worker-module-root` are
  value flags of `verify-worker`; `--backend` stays a value flag for contracts/test. `tests/verify-native/tests.sh`
  `usage_error`: flip lines 799-800 (`contracts`/`test --backend native "$ONE_CLAIM"` no longer "only supported by");
  add `contracts --backend native` and `test --backend native` without `--verify` → `requires --verify`;
  `contracts --verify --backend native --solver z3` → `--solver`; `test --verify --backend native --max-k-step 5`
  → `--max-k-step`; `build --backend native` and bare form keep "only supported by" (message now names
  `verify`, `contracts --verify` and `test --verify`).
- **Code**: `compiler/main.vow::backend_flag_error` (accept `CMD_CONTRACTS`/`CMD_TEST`, require
  `--verify`); `compiler/cli_flags.vow::cf_is_worker_value`; `test_worker_common_flags` forwards `--backend`.
- **Reuses**: `has_flag`, `get_flag_arg`, `push_test_flag_value`.

### 2. Pool: independence and task kinds
- **Test**: `compiler/tests/test_vc_worker.vow` — extend `check_clause_mode_flag` pattern: argv carries
  `--worker-probe vacuity` only for kind 1, `--worker-module-root` only when set; with `halt=false`,
  `vc_pool_finish` of a failed task leaves `state[VC_ST_LIMIT()]` and later `phase`s untouched.
- **Code**: `compiler/vc_worker.vow` (`VcPool` fields, `vc_pool_new` defaults, `vc_worker_argv`,
  `vc_pool_finish`).

### 3. Probe transforms (pure)
- **Test**: `compiler/tests/test_vc_probe.vow` [new], builders from `compiler/tests/builders`:
  both probes: `vc_skip_reason(probe) == ""` (the gate and CFG code accept a single block ending in
  `IOP_UNREACHABLE` with no `Return`, and a Phi-fed return after the rewrite). vacuity: cut right after
  the last `requires` including one in a join block (`&&` predicate), unreachable body blocks dropped,
  fresh vow id not in module, `vows` grew by one, inapplicable without a `requires`; trivial: Phi result
  (`if c { x } else { 0 }`) rewritten with the Upsilon targets intact, returned value replaced by the
  typed zero constant for each int width and bool, original computation retained under a fresh id
  (a division by zero guard claim survives), inapplicable for multi-return / `GetArg` result / unit /
  no `ensures`. Fixed-width coverage includes `i128`/`u128`.
- **Code**: `compiler/vc_probe.vow` [new]; reuse `function_has_requires`/`function_has_ensures`/
  `returns_scalar`/`body_replaceable_result` from `compiler/verifier.vow` (vc_native already `use`s
  verifier). `scripts/concat_vow.sh` FILES gains `vc_probe`.

### 4. Worker probe entry
- **Test**: `tests/verify-native/tests.sh` wiring tier with the fake Bitwuzla: a function with a
  contradictory `requires` and probe `unsat` ⇒ worker result `proven` (wire decode helper already
  exists, `WIRE_DECODE`); `sat_empty` ⇒ `failed`; probe on a function without `requires` is refused
  (exit 3). The fake's plain `unsat` mode proves *every* probe, so wiring tests for the driver need a
  new `FAKE_BW_MODE` that dispatches on query content like `unknown_unwinding` does (`unsat` only for
  the probe's query: its negated goal is the bare path condition; `sat_empty` for the rest, or the
  reverse), otherwise every function with a `requires` reads as `vacuous`. Real-solver tier gated on
  `command -v bitwuzla`.
- **Code**: `compiler/main.vow::run_verify_worker` (`--worker-probe`, `--worker-module-root`).

### 5. Driver + `contracts --verify --backend native`
- **Test**: wiring tier — JSON shape/summary identical to schema (`contracts-result.schema.json`
  validate via the existing python snippet); a failing function does not suppress later functions'
  statuses (`halt=false`); skipped function ⇒ `skipped`, exit 1; Bitwuzla absent ⇒ `error`, exit 1,
  full JSON on stdout. Real-solver fixtures `tests/verify-native/contracts/*.vow` [new directory] with
  directives `// TEST: contract-status <fn> <kind> <status>` and `// TEST: contract-trivial <fn> <bool>`:
  `vacuous_requires.vow`, `weak_ensures_trivial.vow` (`ensures: result >= 0`, scalar, single exit),
  `strong_ensures.vow`, `mixed_clause_statuses.vow` (one `ensures` fails, another proven, `requires` ⇒
  `unknown`), `loop_invariant_clauses.vow`, `caller_callee_inline.vow`, `skipped_effects.vow`.
  Section 4g of `scripts/full_test.sh` gets a loop asserting them (skipped without `bitwuzla`).
  Assert `vow_id` in the JSON equals `IrVowEntry.id` (stability claim of ADR-1430).
- **Code**: `compiler/vc_driver.vow` [new]; `compiler/main.vow::run_contracts` native branch;
  `scripts/concat_vow.sh` gains `vc_driver`; `compiler/main.vow` `use vc_driver`, `use vc_probe`.

### 6. `test --verify --backend native`
- **Test**: wiring tier — `test --verify --backend native f.vow` with fake `unsat` ⇒ entry `passed`
  (program run); `sat_empty` ⇒ `verify_failed` and the program is not executed; non-subset vowed
  function ⇒ `contract_skipped` with `VerificationSkipped`; failure wins over skip; parallel test workers
  (`--jobs 2` over a directory with `--verify`) inherit `--backend native` (worker query files appear).
  Module-root case: fixture that `use`s a sibling module under `--module-root`.
- **Code**: `compiler/main.vow::run_test` native branch (only vowed functions, fail-closed on every
  non-proven).

### 7. Spec, ADR, generated help
- `docs/spec/cli.md`: `--backend` row ("Accepted by `verify`, `contracts --verify` and `test --verify`;
  `build` and the bare form reject it"), `contracts` options (`--backend`, `--timeout`, `--verify-jobs`
  now honoured under native), a "Native backend: `contracts` and `test`" subsection (probe semantics,
  Bitwuzla-missing behaviour, the one fail-closed divergence in `test`, 300 s budget).
- `docs/spec/contracts-methodology.md`/`contracts.md`: one sentence each that `vacuous` and
  `trivially_satisfiable` are produced natively by the probes above.
- New ADR `docs/adr/2026-10-11-1500-native-contract-probes-and-pool-independence.md` [new, UTC timestamp name per
  `docs/adr/README.md`; there is no index to update] (decisions: probes as IR rewrites, separate tasks,
  `halt` flag, fail-closed `test`); addendum line in ADR-1430's flag table row for `--backend`.
- `docs/verifier-eval.md`: document the new harness.
- Regenerate: `uv run python scripts/generate_help.py` (updates the embedded skill/cli text in
  `compiler/main.vow`, `vow/src/skill.rs`, `skills/vow/reference/cli.md`); `scripts/check_help_coverage.py`.

### 8. Differential harness ("agree with ESBMC on the corpus")
Read `docs/verifier-eval.md:158-250` first (where `verify_diff` stands). Extend
`scripts/verify_diff.py` with `--command verify|contracts|test` (default `verify`, output unchanged)
instead of a new script: it already owns `run_backend`, `candidate_isolation`, the report and exit codes.
- **Test first**: `scripts/test_verify_diff.py` gains cases (fake `vowc` shim already there): for
  `contracts` the per-fixture verdict is the ordered tuple of `(vow_id, status, trivially_satisfiable)`;
  for `test` the file's `tests[].status`. Classification reuses `match`/`more_precise`/`weaker`/
  `soundness`: native `proven` where ESBMC says `failed`/`vacuous`, native not-`vacuous` where ESBMC says
  `vacuous` on a genuinely contradictory `requires`, or native `trivially_satisfiable: false` where ESBMC
  says true are NOT soundness (weaker); native `proven`/`trivial` where ESBMC `failed`/non-trivial and
  the ground truth agrees with ESBMC IS `soundness`.
- **Gate (stated relative to the verify gate, because the native subset legitimately skips String,
  maps and floats)**: zero `soundness` rows, and every `weaker` row of `contracts`/`test` is also
  non-`match` for the same fixture under `--command verify` or is on a written allow-list with the reason
  in `docs/spec/cli.md` "Verdict divergence from ESBMC" (known: `MIN / -1`, 128-bit, unwrap, the
  non-block-0 `requires` case above).
- **Wire**: `.github/workflows/ci.yml` already runs `test_verify_diff.py` (line 79). The real corpus
  run needs ESBMC and Bitwuzla (both are at `~/.local/bin` on this host, so the implementation stage can
  produce the evidence; CI has no corpus run, as for verify): run `scripts/verify_diff.py --command
  contracts` and `--command test` once and paste both summaries into the PR.

## Verification surface
- The probes are ordinary Vow-level IR; no new SMT feature, no new executor claim kind. What Bitwuzla
  must prove: (a) vacuity: `⋀ requires` unsat on a 1-block function; (b) trivial: the program's real
  `ensures` and aborts on the body-replaced function.
- No fixture under `tests/run/` or `examples/` grows; new fixtures live in
  `tests/verify-native/contracts/` and are native-only (outside Section 2c's C-parity globs, like the
  rest of `tests/verify-native/`).
- ESBMC path of `run_contracts` / `run_test` must stay byte-identical: the new code is under
  `if native`, the `else` is the old block untouched. Section 2c C-parity and `contract-quality/parity`
  must stay green.

## Testing / gates
- `build/vowc test compiler/tests/test_vc_probe.vow`, `.../test_vc_worker.vow`, `.../test_cli_flags.vow`
  (≈3 min per file; run in background with a done-marker file, not `pgrep -f`).
- `bash tests/verify-native/tests.sh` (fake solver; no Bitwuzla needed).
- `scripts/bootstrap.sh --skip-cargo --no-cache` on the final head SHA; record the SHA and the
  `scripts/seed.toml` pin in the PR checklist.
- `VOW_FULL_TEST_SKIP_CARGO=1 scripts/full_test.sh` (~40 min), `python3 scripts/generate_operations.py --check`
  (untouched, should stay green), `cargo build --release -p vow` after regenerating `vow/src/skill.rs`,
  `cargo clippy --all --all-targets -- -D warnings` (only the generated skill text changed in Rust).

## Risks
- **Fixed point / determinism**: new modules add code but no codegen-order change; no `HashMap`.
  Pool iteration order and driver output must not depend on worker completion order (results are read by
  task index). Confirm binary fixed point via bootstrap.
- **Contracts statuses depend on `resolve_clause_status` fallbacks**: a `requires` clause of a failing
  function becomes `unknown`, of a proven one `proven` — same as ESBMC multi-property; pinned by the
  `mixed_clause_statuses` fixture.
- **Trivial-probe agreement**: ESBMC's body-replace overwrites after `emit_inst`, native rewrite keeps the
  computation under a fresh id; a function whose result inst can abort (checked op, division) must keep
  that claim or native would say `trivial` where ESBMC does not. Covered by a slice-3 unit test and a
  fixture.
- **Probe vs slicing**: the vacuity claim has an empty cone; correctness relies on the "sliced
  non-`unsat` is re-asked in full" rule. Fixture `slice_vacuous_unrelated_requires.vow` already pins the
  reverse; add the vacuity case under `VOW_VERIFY_NO_SLICE=1` both ways.
- **Cost**: each task re-lowers the whole program; a module with many contracted functions pays up to
  3 lowerings per function. Acceptable for now (same architecture as `verify`); note a follow-up to let one
  worker run several tasks if the #1420 perf gate flags it.
- **`test` concurrency**: `--jobs N` test workers × `--verify-jobs` verify workers multiply; default
  stays `--jobs 1` under `--verify`. Each verify worker has the fixed 4 GiB address-space cap.
- **Module root**: forgetting `--worker-module-root` makes the worker reject the program with exit 3,
  surfacing as `panicked` → `verify_failed`; unit-tested in slice 2/6.
- **Vow-id stability**: contracts uses `IrVowEntry.id`, native clause ids are `inst.dv` of
  `VOW_ENS`/`VOW_INV`; asserted equal in the real-solver fixtures.
- **Help drift**: `check_help_coverage.py` and `scripts/test_bootstrap_workflow.py` fail if
  `docs/spec/cli.md`, `skills/vow/reference/cli.md`, `vow/src/skill.rs` and `compiler/main.vow` diverge.
- **Blocked-by #1424/#1416**: #1424 (slicing + per-clause verdicts, merged as #1615) is in the tree; the
  `--worker-clauses` plumbing is present but unused by any driver — this plan is its first user.

## Out of scope
- Making native the default backend (P5), deleting ESBMC/`proven-ir`/`ModelCapacityAssumed` (P6).
- A Rust-driver `--backend` (delegation to the pinned seed).
- Result cache for native (#1429), counterexample output or `--replay-cex` for `contracts`/`test`.
- Changing `verify`'s early-stop behaviour, the executor, simplifier, gate subset, or probe semantics
  beyond ESBMC parity (e.g. multi-return trivial probes stay skipped, "sound but incomplete").
- Moving `function_has_*`/`returns_scalar`/`body_replaceable_result` out of `verifier.vow` (P6 chore).
- Any contract edits: probes and verdicts never weaken or bound a contract for the verifier.
