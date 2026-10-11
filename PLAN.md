# Plan: native-verifier differential vs runtime and CI-only Z3 cross-check (#1431)

## Goal
Point the existing verifier-vs-runtime differential (`scripts/verifier_runtime.py`, #1084/#1092) at
`vowc verify --backend native`, and add a CI-only harness that re-runs every query the native
backend sends to Bitwuzla under Z3 and fails on a `sat`/`unsat` disagreement. Both run in one
scheduled/push-to-`main` workflow over the fixture corpus plus a seed-pinned generated corpus.
Nothing in `compiler/`, `vow-*` or `docs/spec/` changes: Z3 is never a product dependency.

## Assumptions
- **#1473 is still OPEN and no generated corpus with real soundness signal exists.** `gen_corpus.py`
  (#905) cannot supply it: its generated `main` only prints a literal and never calls `f0..f2`, and its
  one contracted generator (`requires: a > 0, ensures: result >= 0`, body `a`) is trivially true, so
  the runtime soundness direction can never fire on those programs. It is used here only as extra
  *query volume* for the Z3 cross-check (seed-pinned), never as "the generated corpus" for AC1.
  Both scripts take arbitrary corpus roots, so #1473's output drops in as one more root. The PR
  therefore uses `Refs #1431`, never `Closes`, while #1473 is open, and says AC1's generated-corpus
  half is pending #1473. `gen_corpus.py` is not grown in this PR (that is #1473's work). The
  implementer re-checks `gh issue view 1473` first; if it merged, add its generator step to the
  workflow. (advisor)
- **Query capture is a PATH shim, not a product knob, and it only copies then `exec`s.** Same
  technique as `scripts/parity_c.py`'s fake `esbmc`: a `bitwuzla` wrapper first on `PATH` copies the
  `.smt2` it is handed to the capture dir and then `exec`s the real solver with the same argv. It
  never waits on, wraps or forwards the solver, so the product's watchdog (`process_kill(handle)`)
  kills the real Bitwuzla, no orphan is left, and the product's own view of stdout/stderr/exit code is
  untouched. Verdicts for the comparison come from re-running both solvers offline on each captured
  file (`bitwuzla --bv-output-format 2 q.smt2`, `z3 -smt2 -T:N q.smt2`). No `compiler/` change, so no
  dual-compiler or fixed-point exposure. (advisor)
- **Standalone queries only**: runs set `VOW_VERIFY_NO_INCREMENTAL=1` (documented test-only switch,
  `compiler/vc_native.vow:101`, already shown verdict-neutral by `tests/verify-native/tests.sh`),
  `--no-cache` and a fresh `VOW_CACHE_DIR`. Incremental-session streams are a follow-up. A session
  invocation (no `.smt2` file argument) is logged by the shim (and still `exec`ed unchanged); the
  harness exits 2 if any session invocation appears while `VOW_VERIFY_NO_INCREMENTAL=1`. (advisor)
- **Disagreement policy**: Bitwuzla `unsat` + Z3 `sat` = SOUNDNESS (false proof); Bitwuzla `sat` + Z3
  `unsat` = PRECISION (spurious counterexample). Either fails. `unknown`/timeout on either side is
  `inconclusive`: reported by count, never a disagreement, bounded by `--min-agreed`. The Bitwuzla
  verdict is the offline re-run's, and a mismatch between that re-run and the verdict the product
  used cannot occur by construction because both are the same pinned binary on the same file. A Z3
  parse/dialect error is a harness failure (exit 2), never a silent skip. (best guess)
- **Z3 install** via a new checksum-pinned composite action modelled on `install-bitwuzla` (release
  zip + SHA-256), not `pip install`: avoids guessing whether the wheel exposes a `z3` CLI. The
  implementer looks up the pinned release asset names/hashes with `gh`/`curl` at implementation time.
  (best guess)
- **Workflow placement**: new `native-oracle.yml` on push to `main` + nightly + `workflow_dispatch`,
  like `bootstrap.yml`/`full-test.yml` (not per-PR: runtime is bootstrap-class). The job builds
  `build/vowc` with `scripts/bootstrap.sh --no-verify`, so it needs no ESBMC. (best guess)
- **A `skipped` replay is a finding** (AC3: counterexamples "all replay"). Native skips replay for
  struct/enum/`Option` parameters (`docs/spec/cli.md:516`). Like `known-soundness-gap`, a `skipped`
  replay fails the run unless its fixture carries a directive naming a tracking issue
  (`// TEST: known-replay-gap #N`, parsed like `DIRECTIVE_KNOWN_GAP`). No measured-baseline ceiling:
  that would be a ratchet that quietly accepts non-replays. (advisor)
- A genuine soundness disagreement found by the sweep is **not fixed in this PR**: file it
  `priority: critical`, annotate the fixture `// TEST: known-soundness-gap #N` (directive already
  honoured by `verifier_runtime.py`), and state "no untracked disagreement" in the PR. (best guess)

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `scripts/verifier_runtime.py` | existing differential; add `--backend`, native-strict precision rules, native default roots | `check_soundness` :104, `check_precision` :162, `main` :238-246 (`--verifier` default :243), `DIRECTIVE_*` :39-42 |
| `scripts/test_verifier_runtime.py` | unit tests for the above (patches `run_json`) | `CorpusTest` :46- |
| `scripts/pair_review.py` | imports `check_soundness(vow_file, verifier, outdir, timeout)` | :38, :806 — signature must stay compatible |
| `scripts/z3_crosscheck.py` [new] | capture shim, Z3 runner, verdict comparison, CLI | model on `scripts/parity_c.py` (shim :20-56, `capture` :57) and `verify_diff.py` exit codes |
| `scripts/test_z3_crosscheck.py` [new] | unit tests with fake `bitwuzla`/`z3` scripts | model on `test_parity_c.py` |
| `scripts/gen_corpus.py` | seed-pinned generator (`--seed`, `--count`, `--output-dir`); query volume only, not edited | `gen_contract_fn` :114, `main` :97 |
| `scripts/candidate_isolation.py` | `scrubbed_env`, `disposable_workdir` for running generated binaries | :25, :31 |
| `.github/actions/install-z3/action.yml` [new] | pinned, checksum-verified Z3 install | model on `.github/actions/install-bitwuzla/action.yml` |
| `scripts/test_install_z3_action.py` [new] | pin/shape guard | model on `scripts/test_install_bitwuzla_action.py` |
| `.github/workflows/native-oracle.yml` [new] | bootstrap `build/vowc`, run both oracles sharded | model on `.github/workflows/bootstrap.yml` (changes job :41-58, bootstrap job :60-) |
| `.github/workflows/ci.yml` | add the two new unit-test steps | steps :77-90 |
| `scripts/test_bootstrap_workflow.py` | structural guards for workflows | classes :93-:506 (`BitwuzlaPinTest` :261) |
| `docs/verifier-eval.md` | document both oracles next to the `verify_diff.py`/`verify_perf.py` sections | after :218 |
| `docs/adr/2026-10-11-<HHMM>-native-z3-cross-check.md` [new] | records shim capture, standalone-only, disagreement policy | format per `docs/adr/README.md` |

## Steps

### 1. Comparison core in `scripts/z3_crosscheck.py` [new] (pure, no processes)
- **Change**: `parse_verdict(stdout) -> {sat,unsat,unknown,timeout,error}` (first non-empty line, as
  `compiler/vc_solver.vow:vc_parse_check_sat`); `classify(bitwuzla, z3) -> agree | SOUNDNESS |
  PRECISION | inconclusive | harness`; `summarize(records, min_agreed)`.
- **Reuses**: verdict vocabulary of `compiler/vc_solver.vow:9-19`; severity/exit-code convention of
  `scripts/verify_diff.py` (0 clean, 1 disagreement, 2 harness cannot judge).

### 2. Capture shim
- **Change**: `SHIM` text + `install_shim(dir, real_bitwuzla)` in `z3_crosscheck.py`. Behaviour: if a
  trailing argument ends in `.smt2`, `cp` it to `$VOW_QUERY_CAPTURE_DIR/q.XXXXXX.smt2`; otherwise
  append a line to `$VOW_QUERY_CAPTURE_DIR/sessions.log`; in both cases `exec "$REAL_BITWUZLA" "$@"`
  (`--version` included). No sidecar, no output forwarding. The real binary is resolved to an
  absolute path *before* `PATH` is prefixed (the shim cannot recurse into itself).
- **Reuses**: `parity_c.install_shim` (:43) pattern.

### 3. Z3 runner and per-fixture orchestration
- **Change**: `run_solver(cmd, query, timeout_s)` used for both `bitwuzla --bv-output-format 2` and
  `z3 -smt2 -T:<s>` (first line decides; `(error` / non-zero exit with no verdict → `harness`);
  `capture_fixture(vowc, fixture, shim_dir, capture_dir)` runs `vowc verify --backend native --no-cache --verify-jobs 1 <fixture>` with
  `PATH`=shim, `VOW_VERIFY_NO_INCREMENTAL=1`, fresh `VOW_CACHE_DIR`, and `VOW_QUERY_CAPTURE_DIR`;
  `main()` with `roots…`, `--vowc` (default `build/vowc`), `--z3`, `--timeout`, `--z3-timeout`,
  `--shard K/N`, `--min-agreed N`, `--output-dir` → `results.json` (`schema_version`, per-fixture
  query counts, per-class counts, every disagreement with the query path copied into the output dir
  so it is a ready repro).
- **Reuses**: corpus enumeration + `--shard` idiom from `verifier_runtime.py:main` (:247-285);
  `DIRECTIVE_SKIP` handling.

### 4. `--backend` for the runtime differential (`scripts/verifier_runtime.py`)
- **Change**:
  - `check_soundness(vow_file, verifier, outdir, timeout, backend=None)` and
    `check_precision(..., backend=None)` put `["--backend", backend]` on the `verify` invocations
    **only**. `check_soundness`'s second call (`build --mode debug --no-verify`) must not receive
    it: `build` rejects `--backend native` with exit 1 (`docs/spec/cli.md:54`), which would turn
    every soundness row into `skipped`. Default `None` keeps `pair_review.py` and existing
    behaviour byte-identical.
  - `--backend {esbmc,native}`; with `native` and `--verifier` unset, default to `build/vowc`
    (Rust `target/release/vow` rejects `--backend`; exit 2 with a clear message if it is passed).
  - Native-strict precision rule, per `docs/spec/cli.md:516` ("a counterexample that does not
    replay is a verifier bug"): under `native`, replay `aborted` is PRECISION (not a skipped
    honest abort), and any `VerifierBug` entry in `diagnostics[]` is PRECISION. `skipped` is a finding (PRECISION-gap, exit 1)
    unless the fixture carries `// TEST: known-replay-gap #N`; skipped replays are also bucketed by
    `replay_reason` in the summary and `results.json` (`replay_skipped_by_reason`).
  - Default roots under `native` also include `tests/verify-native/{pass,fail,unknown}`.
- **Reuses**: existing `run_json`, `run_debug_binary`, ledger record shape (`records[]`).

### 5. Workflow, action, ADR, docs
- **Change**: `install-z3` action (version + per-platform SHA-256 inputs, `z3 --version` must contain
  the pin); `native-oracle.yml`: `changes` job (as `bootstrap.yml`), then a job with matrix
  `shard: [0/4,1/4,2/4,3/4]` that installs Bitwuzla + Z3, runs `scripts/bootstrap.sh --no-verify`,
  generates `gen_corpus.py --seed <pinned> --count 100` (seed pinned for push runs; date-derived and
  echoed on `schedule`), then runs `verifier_runtime.py --backend native` and `z3_crosscheck.py`
  over fixture roots + generated dir, `--min-checked` / `--min-agreed` floors set from the measured
  baseline (coverage floors only; replay has no ceiling), no `continue-on-error`/`|| true`. Cron offset away from 02:17, 03:53, 04:41, 06:29,
  07:20 (e.g. `37 5 * * *`). `ci.yml` gets `test_z3_crosscheck.py` and `test_install_z3_action.py`
  steps. ADR + `docs/verifier-eval.md` section (what each oracle proves, exit codes, how to repro a
  reported disagreement from the saved query, residual: sessions uncaptured).

## TDD slices (red → green, in order)
1. `test_z3_crosscheck.py::ClassifyTest` — unsat/sat→SOUNDNESS, sat/unsat→PRECISION, unknown/timeout→
   inconclusive, z3 error→harness; `summarize` fails below `--min-agreed`. → Step 1.
2. `ShimTest` — fake real `bitwuzla` script: `.smt2` argument copied then the real script's argv,
   stdout and exit code observed unchanged (it is `exec`ed, not wrapped), `--version` passthrough,
   stdin-session invocation logged to `sessions.log`, shim does not recurse. → Step 2.
3. `SolverRunnerTest` — fake `bitwuzla`/`z3` printing `sat`/`unsat`/`unknown`/`(error "…")`/sleeping past the
   timeout. → Step 3 `run_z3`.
4. `CrosscheckMainTest` — fake `vowc` that invokes the shim `bitwuzla` twice per fixture; fake z3
   disagreeing on one query ⇒ exit 1 and the query is saved; all-agree ⇒ 0; no queries captured, z3 absent
   or a session invocation logged ⇒ 2 (an empty or partial capture must never read as clean).
   → Step 3 `main`.
5. `test_verifier_runtime.py::BackendTest` — patched `run_json` sees `--backend native` on both
   `verify` calls and **not** on the `build --mode debug` call of `check_soundness`; absent by
   default; native `aborted`/`VerifierBug` → PRECISION while esbmc keeps `skipped`; native
   `skipped` replay is a finding unless `known-replay-gap #N` is present, and is bucketed by reason; Rust binary + `--backend native` exits 2; native default
   roots exist. → Step 4.
6. `test_install_z3_action.py` — 64-hex defaults for every supported platform, numbered release (no
   rolling tag), version checked against `z3 --version`. → Step 5 action.
7. `test_bootstrap_workflow.py::NativeOracleWorkflowTest` — triggers (push main + schedule +
   dispatch), Bitwuzla + Z3 installed via the local actions, both scripts invoked, bootstrap runs
   `--no-verify`, no `continue-on-error`, generated corpus seed present. → Step 5 workflow.
8. Local acceptance run (not a unit test): `scripts/bootstrap.sh --skip-cargo --no-cache --no-verify`
   then both scripts over fixtures + `gen_corpus.py --seed 1431 --count 50`; record measured
   `agreed`/`inconclusive`/`replay skipped` counts to set the CI floors.

## Verification surface
- No contract, codegen, IR or C-model change: nothing for ESBMC to prove, no `tests/run/` or
  `examples/` fixtures grow, `generate_help.py`/`check_help_coverage.py` not needed (no CLI/spec
  change).
- The oracles *consume* `tests/verify`, `tests/verify-fail`, `tests/verify-native/*`, `examples`,
  and the generated corpus. Any fixture in those that is not meant to be run by `main` is already
  handled by `// TEST: skip`.
- "Green locally" claim for the PR must record head SHA, `bitwuzla --version`, `z3 --version`, and
  the `gen_corpus.py` seed (CLAUDE.md pinning rule; the seed pin does not exist yet).
- Local tooling: `z3` is not on `PATH` here and `sudo` is unavailable — unpack the pinned release
  zip under `$TMPDIR` and put it on `PATH` for the local run.

## Risks
- **Solver dialect drift**: queries are written for Bitwuzla (`--bv-output-format 2`, `:produce-models`,
  `get-value`, QF_BV/QF_ABV today). Z3 rejecting one is exit 2, surfaced loudly. When the FP encoder
  lands (epic D7) unspecified functions (`fp.to_ubv/sbv` out of range, `fp.min/max` of ±0) can make
  two correct solvers disagree; triage as an encoder bug (guard the operation), do not allow-list.
- **Vacuous green**: no queries captured (cache hit, shim not first on `PATH`, solver not found) must
  be exit 2; `--min-agreed`/`--min-checked` floors and the fresh `VOW_CACHE_DIR` + `--no-cache` +
  empty-cache-dir assertion (as `verify_perf.py` does) guard it.
- **Incremental sessions uncaptured**: a session `unsat` is not re-decided by the product (see
  `compiler/vc_session.vow` header), so it is not Z3-checked here; verdict-equivalence relies on the
  existing slicing/incremental parity tests. Follow-up: tee the session stdin and replay under Z3.
- **`pair_review.py` coupling**: it calls `check_soundness` positionally; the new `backend`
  parameter must be keyword with a default. Run `scripts/test_pair_review.py`.
- **Soundness check strength**: the fixture sweep only executes each program's own `main`, so it
  exercises one input per program, and `gen_corpus.py` output exercises none (see Assumptions). The
  runtime soundness direction gets real depth only from #1473; until then AC1 is not claimed.
- **Workflow guards**: grep `scripts/test_bootstrap_workflow.py` and `.github/workflows/workflow-lint.yml`
  — the new workflow needs SHA-pinned actions and `permissions: contents: read`, and an existing
  guard may constrain `bootstrap.sh --no-verify` usage outside `bootstrap.yml`.
- **Runtime/RSS**: ~320 fixtures × (2 verifies + debug build + replay), sharded 4 ways; keep
  `--verify-jobs 1` per shard and do not reintroduce the removed 2 GB `ulimit` (#1348).
- **Candidate execution**: generated binaries run through `candidate_isolation` (scrubbed env,
  disposable cwd), as `run_debug_binary` already does.
- **Python lint gate**: pinned ruff in CI — run `uvx ruff@<pin>` (see repo's ruff pin), not the
  local one. `scripts/` Python stays stdlib-only like its neighbours.

## Out of scope
- Any change to `compiler/vc_*.vow`, the Rust compiler, ESBMC plumbing or `docs/spec/*`.
- Building #1473's generator axis itself, or the #1081 ledger/nightly plumbing beyond reusing
  `verifier_runtime.py`'s record shape.
- Z3-checking incremental-session streams; model/CEX validation inside Z3 (replay covers it).
- Making the oracle a required per-PR check; changing `verify_diff.py`/`verify_perf.py`.
- Fixing any verifier bug the sweep uncovers (file `priority: critical` issues instead).
