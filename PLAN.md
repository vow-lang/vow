# Plan: native-vs-ESBMC verifier performance harness (#1420)

## Goal
Commit `scripts/verify_perf.py`: times `vowc verify --backend esbmc` against `--backend native` over the gate corpus (23 non-stretch benchmark references + `tests/verify*` + float fixtures), median of 5 runs, cache off, and encodes epic #1398 acceptance gate item 3 as pass/fail with a machine-readable JSON report. Blocked-by #1414 is merged (`scripts/verify_diff.py`, #1549); this script builds on its helpers.

## Assumptions
- Python-only change; no compiler, spec, `--help`, ADR or C-emitter change, so the dual-compiler rule, byte-identical C parity and bootstrap fixed point are untouched (best guess; the issue asks only for a script in `scripts/`).
- Sibling script, not an extension of `verify_diff.py`: that script is a verdict gate with a stable report schema (`SCHEMA_VERSION = 1`, :47) and a CI unit-test file; the perf gate has a different corpus, runs each fixture 5x per backend, and a different exit contract. It imports `verify_diff`/`verify_eval` helpers and leaves them untouched.
- Peak RSS metric = `max(sampled /proc sum over the process tree, wait4 ru_maxrss)` per run. `ru_maxrss` alone misses summed concurrency (driver + solver children, see `scripts/measure_build_tree_rss.py:3-9`); sampling alone can miss a short spike. Taking the max keeps an exact lower bound. Both raw numbers are in the report.
- Both backends run with `--verify-jobs 1` by default (override `--verify-jobs N`), so wall-clock is not distorted by scheduler contention between concurrent workers; recorded in the report.
- Budget: no `--timeout` is passed by default, so each backend runs at its shipped default (`compiler/verifier.vow:652-663`: an explicit `--timeout` suppresses ESBMC's 30s BV budget that triggers the IR/z3 fallback, which would inflate ESBMC times and flatter native). `--timeout N` is available, is passed to both, is recorded in the report, and sets `"budget": "explicit"`. Watchdog = `(DEFAULT_BUDGET_S = 330) * function_count + slack` by default (300s per-function default + 30s BV phase), computed by the caller and passed to `run_once` in seconds.
- Timeout semantics: a run "timed out" when the JSON has `verify_status == "timeout"` or the watchdog killed it. A fixture times out on a backend when a strict majority of its 5 runs timed out (consistent with "median of 5"); minority timeouts are reported as `timeout_runs` but not gating. Timed-out runs contribute the watchdog budget as their wall time.
- Comparability: only rows `verify_diff.classify(...)` classes `match` enter the geometric mean and worst-ratio (same verdict and counterexamples, so same work). Every other class (`more_precise`, `weaker`, `soundness`, `harness`, or a verdict that varies between runs) is excluded from the ratios and listed in the report with its class and detail. Verdict parity is epic gate item 1, owned by `verify_diff.py`; `comparable` is therefore report-only here and never fails the gate. The timeout criterion runs over ALL rows (a native timeout classifies as `weaker` and must still count). The gate fails, not vacuously passes, when fewer than `MIN_COMPARABLE = 1` comparable rows remain, since a geomean over nothing is undefined.
- Truth for `classify`: `verify_eval.collect` rows use `Expect.expected_status` / known-gap rule as in `verify_diff.diff_fixture` (:233-240); `tests/verify-native/pass|fail|skip|unknown` -> Verified / VerifyFailed / Skipped / None, `verify-fail-multi/*/main.vow` -> VerifyFailed (dir convention, `full_test.sh:1374-1395`); each file is parsed with `verify_eval.parse_directives` so `// TEST: skip` is honoured everywhere. `unknown`-truth rows are only ever compared on verdict equality.
- Corpus: `tests/verify`, `tests/verify-fail`, `tests/verify-skip` (via `verify_eval.collect`), `tests/verify-fail-multi/*/main.vow` (invoked as `vowc verify <dir>/main.vow`, no extra flags, `full_test.sh:1374-1382`), `tests/verify-native/{pass,fail,skip,unknown}/*.vow`, all float fixtures are already inside these (`tests/verify/float_*.vow`, `tests/verify-fail/float_enum_payload_wrong.vow`, `tests/verify-native/skip/float_*.vow`; asserted by name in a test). `tests/verify-stress` is excluded by default (non-gating by design, timeouts by construction; five 60s+ timeouts per run) and opt-in with `--include-stress`. `tests/verify-bv-fallback` and `tests/verify-cache-stale-pass` hold only `tests.sh` and contribute no `.vow`.
- Benchmarks: the 23 entries of `benchmarks/manifest.toml` whose `expected_status != "Stretch"`, file `<path>/reference.vow`. The count must equal 23 (the gate's definition); otherwise exit 2 with a corpus-drift message rather than silently gating a different set.
- Strict thresholds, no noise floor: geomean(native/esbmc) <= 1.0 on wall and RSS; max ratio <= 1.25; no native timeout where ESBMC does not time out. Tiny-fixture jitter is mitigated only by the median of 5; a `--floor` knob is a follow-up if the first real run shows noise failures.
- ESBMC pin: preflight requires `esbmc --version` to report `8.5` (matches `.github/actions/install-esbmc/action.yml:29`). The local sandbox has ESBMC 8.3.0 and no `build/vowc`, so the real gate cannot be run here; implementation is verified through injected-runner unit tests plus a smoke run with `--esbmc-version-pin` left default failing preflight cleanly.

## Files to touch
- `scripts/verify_perf.py` [new]
- `scripts/test_verify_perf.py` [new]
- `docs/verifier-eval.md` (new section)
- `.github/workflows/ci.yml` (one unit-test step)

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `scripts/verify_perf.py` [new] | the harness | - |
| `scripts/test_verify_perf.py` [new] | unittest suite, run from CI | model on `scripts/test_verify_diff.py:1-273` (`mock.patch.object` on `collect`, runner, `preflight`) |
| `scripts/verify_diff.py` | reuse `verdict_of` (:75), `cex_keys` (:91), `classify` (:113), `function_count` (:179), `synthetic_failure` (:171), `WATCHDOG_SLACK` (:70), `preflight` pattern (:291) | read-only |
| `scripts/verify_eval.py` | `collect` (:460), `Expect` (:90), `actual_cex` (:282), `REPO_ROOT` (:47) | read-only |
| `scripts/measure_build_tree_rss.py` | `snapshot(root)` (:42) tree-sum sampler; import rather than copy (has `__main__` guard at :97, safe to import) | read-only |
| `scripts/candidate_isolation.py` | `scrubbed_env()` (:25) for the child env | read-only |
| `benchmarks/manifest.toml` | `[[benchmarks]]` with `path`, `expected_status` | read-only |
| `docs/verifier-eval.md` | add "Performance gate" section after the differential-harness section (ends ~:190) | edit |
| `.github/workflows/ci.yml` | add unit-test step after `test_verify_diff.py` (:78-79) | edit |

## Steps (TDD slices; each = red test in `scripts/test_verify_perf.py`, then code in `scripts/verify_perf.py`)

### 1. Statistics core (pure)
- Test: `median` of 5 / even n; `geomean` of ratios (known values, e.g. [2, 0.5] -> 1.0); ratio computation `native/esbmc`; zero or missing denominators raise `ValueError` rather than yield inf.
- Code: `median(xs)`, `geomean(xs)`, `ratio(native, esbmc)`. stdlib only (`statistics`, `math`).

### 2. Gate evaluation (pure)
- Test: table-driven `evaluate_gate(rows)` -> `{"passed": bool, "criteria": {geomean_wall, geomean_rss, worst_wall, worst_rss, no_new_timeouts, enough_comparable}}`: geomean exactly 1.0 passes; 1.001 fails; worst 1.25 passes, 1.2501 fails; native timeout with ESBMC finishing fails; both timing out passes; a `weaker`/`more_precise`/unstable row is excluded from ratios and listed but does not fail the gate; a native timeout in a `weaker` row still fails `no_new_timeouts`; zero comparable rows fails the gate; wall and RSS evaluated independently.
- Code: constants `GEOMEAN_MAX = 1.0`, `WORST_MAX = 1.25`, `RUNS = 5`; `evaluate_gate`. Report carries `worst_wall: {fixture, ratio}`, `worst_rss`, `timeouts: {native_only, esbmc_only, both}`.

### 3. Corpus enumeration
- Test: manifest with 3 stretch + 1 verified entry yields only the verified one as `benchmarks/<path>/reference.vow`; real repo corpus has exactly 23 benchmark rows (guards manifest drift) and includes the float fixtures by name; `verify-stress` absent unless `include_stress`; multi dirs yield `main.vow`; skipped `Expect.skip_reason` fixtures dropped as `verify_diff.main` does (:362).
- Code: `collect_corpus(include_stress)` returning `[(group, name, path, truth)]`, using `tomllib` (3.11+; CI is 3.13) and `verify_eval.collect`; unknown-truth benchmark rows use `expected_status` from the manifest.

### 4. Single measured run
- Test (uses a tiny stub executable written to a temp dir that sleeps, allocates, prints JSON): returns `{wall_s, peak_rss_kb, rss_self_kb, rss_tree_kb, result_json, timed_out, exit_code}`; stub that sleeps past a 0.5s `watchdog_s` is killed (test stays sub-second) and reported `timed_out` (process group killed, mirror `verify_diff.run_backend` :185-231); stub exiting by signal -> `crashed`; non-JSON stdout -> `result None`; `VOW_CACHE_DIR` in the child env points at a fresh empty dir and args contain `--no-cache`, `--backend X`, `--verify-jobs N`, and `--timeout T` only when explicitly given.
- Code: `run_once(vowc, backend, path, jobs, timeout_or_none, watchdog_s)`: `Popen` with stdout/stderr to temp files, `start_new_session=True`, `env=scrubbed_env()` + fresh `VOW_CACHE_DIR`; loop `os.wait4(pid, os.WNOHANG)` sampling `measure_build_tree_rss.snapshot(pid)` every 10 ms, final `ru_maxrss` (KiB on Linux) from the rusage; `run_once(..., watchdog_s)` takes the watchdog from the caller (computed in step 5 as above), then `os.killpg` on expiry; tests pass ~0.5s. Linux-only: other platforms -> exit 2 with message.

### 5. Per-fixture measurement and classification
- Test: with an injected runner, each backend is run 5x interleaved (esbmc, native, esbmc, native, ...) so drift hits both; one warm-up run per backend per fixture is discarded (not in the issue; page-cache/dynamic-loader warmth; adds 2 of 12 runs per fixture, folded into the runtime estimate); row holds medians, the 5 raw samples, verdicts from `verify_diff.verdict_of`, `class` from `verify_diff.classify`; verdict that differs between runs of one backend sets `unstable: true` and makes the row `incomparable`; timeout-majority rule from Assumptions.
- Code: `measure_fixture(...)` -> row dict; fixtures run sequentially (parallelism would perturb the very thing measured; no `--jobs`).

### 6. Report, CLI and exit codes
- Test: `main([...])` with patched `collect_corpus`/`run_once`/`preflight` as in `test_verify_diff.py:200-273`: JSON on stdout or `--output`, `schema_version == 1`, keys `{schema_version, vowc, esbmc_version, bitwuzla_version, host, runs, budget, timeout_s, verify_jobs, corpus, rows, gate}`; exit 0 when gate passes, 1 when not, 2 for missing `vowc`/`esbmc`/`bitwuzla`, `esbmc --version` != 8.5, corpus drift, empty selection, non-Linux; stderr summary prints geomean wall/RSS, worst fixture per metric, timeout list; `--filter`, `--runs` (default 5; values < 5 recorded and the gate marked `"provisional": true` so a smoke run cannot be mistaken for the gate), `--timeout` (default none = shipped defaults), `--output`.
- Code: `build_report`, `exit_code`, `preflight` (reuse `shutil.which` pattern from `verify_diff.preflight` :291-298 plus the version check), `print_summary`, `main`.

### 7. Docs and CI wiring
- `docs/verifier-eval.md`: new "Performance gate: native verifier vs ESBMC" section: corpus, metrics definition (wall, `max(tree-sum sample, ru_maxrss)`), thresholds table, comparability rules, exit codes, report schema, run command (`python3 scripts/verify_perf.py --vowc build/vowc --output /tmp/verify-perf.json`), "same machine, idle host, ESBMC 8.5 and the pinned Bitwuzla on PATH", note that it is an acceptance-gate tool and not a `full_test.sh` section (hours-scale wall-clock, host-dependent).
- `.github/workflows/ci.yml`: step `Native-vs-ESBMC performance harness unit tests` -> `python3 scripts/test_verify_perf.py`, directly after the `test_verify_diff.py` step.

## Verification surface
- No contracts, codegen, IR or C-model change: nothing for ESBMC to prove, no new `tests/run/` or `examples/` fixtures.
- The harness's own correctness is covered by unit tests; no `.vow` fixtures are added.
- Local checks before PR: `python3 scripts/test_verify_perf.py`, `python3 scripts/test_verify_diff.py` (shared helpers unchanged), `uvx ruff@0.15.14 check scripts/verify_perf.py scripts/test_verify_perf.py` and `uvx ruff@0.15.14 format --check` on the same files (pin from `.pre-commit-config.yaml:24`), `python3 scripts/verify_perf.py --vowc /nonexistent` returns 2 cleanly.
- Not runnable here: a full gate run needs ESBMC 8.5 + Bitwuzla + a bootstrapped `build/vowc`; state this plainly in the PR body instead of claiming a green gate.

## Risks
- Tool-version drift: local ESBMC is 8.3.0, so the preflight must hard-fail rather than warn, otherwise the gate silently compares against the wrong ESBMC.
- Native backend still incomplete (`weaker` rows are expected until P3/P5): those rows drop out of the ratios and are listed; an unfinished native will mostly show as few comparable rows / `no_new_timeouts` failures, which is the intended signal.
- No Rust or `.vow` source is touched: binary fixed point, `parse -> print -> parse` idempotency, byte-identical verifier C (`c_emitter.{rs,vow}`) and `cargo clippy -D warnings` are unaffected; `bootstrap.sh` need not be re-run.
- Reparented descendants: `snapshot` walks PPid; a double-forking solver would be missed. Since the child leads its own session (`start_new_session=True`), summing RSS by session id from `/proc/<pid>/stat` is the fallback if the first real run shows undercounting; `ru_maxrss` already floors it.
- RSS sampling overhead and 10 ms resolution: sampling reads `/proc` of the whole machine (`snapshot` lists all pids); acceptable at 100 ms+ run times, and `ru_maxrss` floors the result. Long-term fix (cgroup memory.peak) is a follow-up.
- Wall-clock duration: ~70+ fixtures x 2 backends x 6 runs; document expected multi-hour runtime and `--filter`/`--runs` for smoke runs.
- Importing `verify_diff` couples to its helper names; mitigated by testing against the real module (CI runs both test files) and not modifying it.
- CI python-lint pin and `typos`: keep identifiers/comment text plain ASCII; comments sparse per user guidelines.
- `codecov/patch` applies to Rust only; no effect.

## Out of scope
- Any change to `verify_diff.py`, `verify_eval.py`, the compilers, `docs/spec/*`, `--help` generation, ADRs or `CLAUDE.md`.
- A noise-floor option, cgroup-based memory accounting, a `full_test.sh` section, scheduled CI runs, and the #1398 P5 criterion 4 (native verifying `compiler/*.vow`).
- Making native pass the gate (performance work is P4 tickets).
