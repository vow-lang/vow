# Verifier-Evaluation Suite

The **verifier-evaluation suite** is the Vow verifier's *acceptance harness*. It
answers a different question from the synthesis benchmarks under
[`benchmarks/`](../benchmarks/README.md):

| Suite | Question it answers |
| --- | --- |
| `benchmarks/` (synthesis) | Can an **agent** produce a verifying program from a spec? |
| **verifier-eval** (this) | Is the **verifier** accepting correct programs, rejecting incorrect ones, and attributing blame correctly? |

A synthesis suite can be at 100% while the verifier silently regresses — a
weaker check that accepts more programs would *raise* the synthesis score. This
suite exists to catch exactly that: it is a labelled corpus of small Vow
programs, each carrying a ground-truth outcome, run by
[`scripts/verify_eval.py`](../scripts/verify_eval.py).

It was built for issue #334 and is foundational for #335 (differential
replay of counterexamples against runtime semantics) and #337 (adaptive-retry
status discipline).

## What it measures

- **False accepts (soundness).** A program that is genuinely incorrect but the
  verifier reports `Verified` — including proofs that pass only *vacuously*
  (contradictory `requires`). These are the gravest failures and are surfaced
  under their own banner; in CI they turn the build red.
- **False rejects (precision).** A correct program the verifier rejects
  (`VerifyFailed`/`Skipped`). Often silent in practice because nobody re-tests a
  verifier that "just works"; here a regression is loud.
- **Blame correctness.** When a contract fails, is the violated `vow_id` and the
  `Caller`/`Callee` attribution exactly what we expect? Blame is mechanical from
  the contract kind (`requires` → Caller, `ensures`/`invariant` → Callee), so the
  ground-truth label is unambiguous.
- **Model drift.** Programs exercising constructs whose IR-to-C encoding could
  diverge from the executable semantics emitted by `vow-codegen`.

## Corpus layout

The corpus reuses the existing `tests/verify*` directories; the directory sets
the coarse expected status and the `// TEST:` directives carry the fine-grained
ground truth.

| Directory | Expected `vow verify` status | Gating |
| --- | --- | --- |
| `tests/verify/` | `Verified` | yes |
| `tests/verify-fail/` | `VerifyFailed` (+ expected counterexamples) | yes |
| `tests/verify-skip/` | `Skipped` (non-modelable, fail-closed, deterministic) | yes |
| `tests/verify-stress/` | `unverifiable-by-design` (timeout/`unknown`) | **no** — see below |
| `tests/debug/` | runtime `VowViolation` blame (debug mode) | local only — `tests/run_tests.sh` Phase 4 (**not** CI) |

`tests/verify-stress/` is **not** wired into CI or `full_test.sh`: its outcomes
depend on the ESBMC unwind budget and host, so asserting a fixed result would be
flaky. It documents that the verifier degrades *safely* (fails closed, never
falsely accepts) on intractable inputs. See
[`tests/verify-stress/README.md`](../tests/verify-stress/README.md).

`tests/debug/` runtime checks (including the caller-blame anchor
`caller_blame_debug.vow`) are exercised by `tests/run_tests.sh` Phase 4, a
local developer harness. Neither it nor `full_test.sh` runs in CI; only the
static `scripts/verify_eval.py` gate does. So runtime blame is regression-checked
locally, not in CI — keep that in mind when relying on the runtime half of
caller-blame coverage.

## Ground-truth directives

Directives live in `//` comments (stripped at lex time, zero compile impact),
extending the same `// TEST:` convention `tests/run_tests.sh` already uses.

| Directive | Meaning |
| --- | --- |
| `// TEST: category <name>` | One of `overflow`, `bounds`, `invariant`, `caller-blame`, `callee-blame`, `model-drift`, `unverifiable`. Required on every corpus program. |
| `// TEST: counterexample-fn "<fn>"` | Expected counterexample function (verify-fail). |
| `// TEST: counterexample-blame <Caller\|Callee\|None>` | Expected blame; `None` = a memory-safety/builtin failure with no contract attribution. |
| `// TEST: counterexample-vow-id <N>` | Expected violated `vow_id` (from the `vow verify` counterexample, which is a distinct id space from `vow contracts --verify`). |
| `// TEST: cex fn="<fn>" blame=<b> vow_id=<N>` | Repeatable form for programs with multiple expected counterexamples. |
| `// TEST: known-soundness-gap "<reason>" #<issue>` | Marks a documented false-accept in `tests/verify/` that the verifier does not yet catch. Reported under the KNOWN SOUNDNESS GAPS banner, non-fatal — until the verifier *starts* catching it, at which point the harness fails and demands promotion to a real verify-fail program. |
| `// TEST: status <Status>` / `// TEST: skip "<reason>"` | Override the directory's expected status / exclude a program. |

## How the harness classifies results

For each program `verify_eval.py` runs `vow verify` (status + counterexamples)
and, for should-pass programs, `vow contracts --verify` as a vacuity guard. Each
result is bucketed:

- **SOUNDNESS** — expected-fail program reported `Verified`, or a should-pass
  program proven vacuously. **Hard failure.**
- **PRECISION** — should-pass program reported `VerifyFailed`/`Skipped`.
- **BLAME / VOW_ID** — wrong blame, wrong violated `vow_id`, or surplus
  counterexamples.
- **STATUS** — any other expected/actual status mismatch, including a missing
  expected counterexample.
- **KNOWN SOUNDNESS GAPS** — tracked false-accepts (non-fatal).
- **KNOWN GAP APPEARS FIXED** — a known gap the verifier now catches (**hard
  failure**: promote the label).

Exit code is non-zero on any bucket except known-gaps. A machine-readable
`report.json` is written to the `--output-dir`.

## Category coverage

<!-- GENERATE:CORPUS_COUNTS:START -->
All 7 categories are represented (164 programs):

| Category | Count |
| --- | --- |
| callee-blame | 54 |
| model-drift | 32 |
| bounds | 29 |
| overflow | 19 |
| unverifiable | 16 |
| caller-blame | 10 |
| invariant | 4 |
<!-- GENERATE:CORPUS_COUNTS:END -->

## Known soundness gaps

None currently open.

- **Caller obligations are now checked statically (#764 — resolved by PR #736).**
  `vow verify` asserts a callee's `requires` at its in-module call sites (the G7
  call-boundary assert) instead of assuming it, so a caller passing a
  provably-out-of-contract argument is reported `VerifyFailed` with
  `blame=Caller`. The former `known-soundness-gap` xfail was promoted to
  `tests/verify-fail/caller_requires_unchecked.vow`; the runtime anchor
  `tests/debug/caller_blame_debug.vow` still covers the runtime half of
  caller-blame coverage.

## Running

```bash
# Local (uses target/release/vow by default):
cargo build --release -p vow
python3 scripts/verify_eval.py

# Local, against the fixed-point self-hosted compiler:
scripts/bootstrap.sh
python3 scripts/verify_eval.py --verifier build/vowc --output-dir /tmp/verify-eval-self

# The native backend (self-hosted vowc only): `--backend` reaches the verify, vacuity and
# replay runs. A program the corpus labels incorrect only because ESBMC refutes it through
# its capacity assertion (`esbmc-capacity-artifact`) is reported as an explained extra proof,
# not a soundness regression. The expected status mismatches for `Skipped` are expected.
python3 scripts/verify_eval.py --verifier build/vowc --backend native

# Authoring aid — print actual outcomes for every program:
python3 scripts/verify_eval.py --discover

# A single program:
python3 scripts/verify_eval.py --filter off_by_one_bounds

# Keep the category counts above in sync with the corpus:
python3 scripts/verify_eval.py --write-docs   # regenerate
python3 scripts/verify_eval.py --check-docs   # CI gate (no verifier needed)
```

It also runs as **Section 4e** of `scripts/full_test.sh`, as a dedicated Rust
verifier step in the `build-and-test` CI job, and against the fixed-point
self-hosted `build/vowc` in the Ubuntu `bootstrap` CI job. That means a
soundness, blame, or exact `vow_id` regression in either verifier blocks PRs.

## Differential harness: native verifier vs ESBMC

[`scripts/verify_diff.py`](../scripts/verify_diff.py) (epic #1398, issue #1414)
runs the same corpus (`tests/verify`, `tests/verify-fail`, `tests/verify-skip`,
with the ground truth `verify_eval.py` reads, and `tests/verify-native/{pass,fail,skip,unknown}`,
whose truth is the directory) under
`vowc verify --backend esbmc` and `vowc verify --backend native`, and classifies
every fixture:

| Class | Meaning |
| --- | --- |
| `match` | Same verdict; when both refute, the same `(function, blame, vow_id)` counterexamples. |
| `more_precise` | Native concludes where ESBMC does not, reports an extra counterexample, or refutes a program ESBMC proves and the corpus labels incorrect. |
| `weaker` | Native is `Skipped`/`unknown`/`timeout`/`tool_not_found` where ESBMC proves or refutes, drops or re-attributes a counterexample, or refutes a program the corpus labels correct. **Fails the script.** |
| `soundness` | Native proves a program ESBMC refutes, or one the corpus labels incorrect. **Fails the script.** |
| `harness` | A backend printed no parseable JSON, so the row says nothing about the verifiers. A backend that hangs (killed at its budget) or dies from a signal is not a `harness` row: it is inconclusive (`verify_status` `timeout` / `crashed`). |

Backends that agree on the verdict but not on which functions they left
unanalysed (the `VerificationSkipped` *warnings*, not the notes about uncontracted
callers) did not do the same work: native skipping a function ESBMC analyses is
`weaker`, native analysing a function ESBMC skips is `more_precise`, so such a row
is never `match` and never enters the performance ratios
(`verify-skip/wide_enum_payload_skipped`: ESBMC skips four functions, native proves
three of them). The last direction needs a native verdict that cannot have stopped
early (`proven` or `skipped`): native `verify` reports up to the first function that
is not proven, so after a refutation the absence of a skip warning does not show
that a later function was analysed.

Native `verify_status` of `panicked`, `error` or `crashed` is `weaker` even when
ESBMC is also inconclusive. File-level status is per module: native reports
`Skipped` for a file when any one function is outside its subset, so a partly
modelable file reads as `weaker` against an ESBMC `Verified` until the subset
grows.

### ESBMC model artefacts

A fixture the native verifier proves and ESBMC refutes is a `soundness` row
unless the fixture carries a written explanation, a `// TEST:` directive that
`verify_eval.artifact_directives` parses (fail closed: a malformed or misspelled
`esbmc-*` directive is an error). `verify_diff.py` and `verify_perf.py` classify
through one function (`verify_diff.classify_outcomes`), and `verify_eval.py
--backend native` reads the same directives, so the three harnesses agree.

| Directive | Meaning |
| --- | --- |
| `// TEST: esbmc-capacity-artifact "<why>"` | ESBMC refutes only through its own `string`/`vec` capacity assertion. Needs the evidence of the run: every ESBMC counterexample is a capacity assertion. |
| `// TEST: esbmc-model-artifact fn="<fn>" blame=<B> vow_id=<N> "<why>"` | One ESBMC counterexample comes from ESBMC's own model, not from the program (for example its nondeterministic `string_eq`). One directive per counterexample; the row is `more_precise` only when the corpus labels the fixture correct (`Verified`) and every ESBMC counterexample is declared, so any other ESBMC refutation of the file is still a `soundness` finding. |

Add a directive only after the runtime differential
(`scripts/verifier_runtime.py --backend native`) shows no false proof for the
fixture, and write the reason as the explanation gate item 1 asks for.

The report is one JSON document (`schema_version`, `summary`, `rows[]` with both
backends' verdict, `verify_status`, counterexamples and wall-clock seconds) on
stdout or in `--output FILE`; a human summary of non-`match` rows goes to
stderr. Exit codes: `0` clean, `1` any `weaker`/`soundness` row (takes precedence), `2`
the harness could not run or only found `harness` rows (missing `vowc`, `esbmc`
or `bitwuzla`, unparseable verifier output, no fixture selected, or an I/O error).

```bash
python3 scripts/verify_diff.py --vowc build/vowc --output /tmp/verify-diff.json
python3 scripts/verify_diff.py --filter max   # one fixture family
```

It is a developer and acceptance-gate tool (epic #1398, gate item 1), not a
`full_test.sh` section: the native backend covers a growing subset, so most
fixtures are `weaker` until it catches up.

`--command contracts` and `--command test` (issue #1427) run the same corpus
under `vowc contracts --verify` and `vowc test --verify` instead, with the same
four classes and exit codes. `contracts` compares every clause by
`(function, kind, vow_id)`: its status and its `trivially_satisfiable` flag. A
clause is `soundness` when native proves what ESBMC refutes, `more_precise` when
native proves what ESBMC leaves `unknown`/`skipped`, and `weaker` when native is
less conclusive, reports `vacuous` where ESBMC does not, or the flags differ.
ESBMC's `vacuous` for a function whose `requires` uses `&&`/`||` is
`more_precise`, because its vacuity label is only planted in the entry block
(see `cli.md`, "Native backend: `contracts` and `test`"). `test` compares the
file's test status: `passed`/`failed`/`timeout` read as verified,
`verify_failed` as refuted, `contract_skipped` as skipped; a fixture without
`fn main` is left out, since `test` has nothing to run. The ESBMC test path fails
a file for a reachable checked-arithmetic abort that `verify` only warns about,
so a native pass of a program the corpus labels correct is `more_precise`. A
fixture takes the most severe class of its clauses, and the report carries
`command`.

```bash
python3 scripts/verify_diff.py --command contracts --output /tmp/contracts-diff.json
python3 scripts/verify_diff.py --command test --jobs 4
```

The acceptance criterion for the native `contracts`/`test` backends is zero
`soundness` rows, and every `weaker` row also non-`match` under
`--command verify` (the native subset legitimately skips what ESBMC models) or
listed in `cli.md` under "Verdict divergence from ESBMC". A clause that ESBMC
calls `proven` or `failed` and native `unknown`, in a function the native
`verify` refutes without any clause of its own failing (an abort, or a callee's
precondition), is `more_precise`: the native report proves no clause of a
function that can abort, where ESBMC judges the clause on the executions that
continue past it. A fixture can also carry `// TEST: native-divergence "<why>"`
(the reason is mandatory): a clause ESBMC calls `proven` and native `unknown` in
that fixture is then `more_precise` when the corpus labels it `VerifyFailed`, and
nothing else in it is excused (a skipped, errored or refuted clause keeps its class). Three `verify-fail` fixtures carry it:
`callee_ensures_wrong_function` (`verify` stops at the callee, so the caller is never
refuted there), and `float_enum_payload_wrong` and `string_substring_bytes_observed`
(a `requires` clause is `proven` only when its whole function is, and the function's
`ensures` fails; see `cli.md`, "Clause status"). The one row left is
`verify-fail/verify_jobs_ce_before_soft` (an unbounded loop without an
`invariant`, which ESBMC's k-induction closes; `cli.md`, "Verdict divergence from
ESBMC", describes the symbolic-bound loop divergence).

The signed `MIN / -1` rows are not excused by a directive: native refutes them, the
runtime confirms it (`--replay-cex`), and the corpus programs that divided by a
value constrained only by `y != 0` were corrected to state the true domain
(`requires: x >= -9223372036854775807 || y != -1`), so both backends agree on every
divide program of the corpus (ruling for #1688). The fixtures that pin the native
refutation itself are `verify-native/fail/signed_div_min` and `i32_div_min`, labelled
`VerifyFailed`.

## Performance gate: native verifier vs ESBMC

[`scripts/verify_perf.py`](../scripts/verify_perf.py) (epic #1398, issue #1420)
encodes acceptance gate item 3. It times `vowc verify --backend esbmc` against
`--backend native` with the cache off (`--no-cache`, fresh `VOW_CACHE_DIR` per
run), one fixture at a time on one machine, and reports the median of five runs
per backend. The harness asserts that the cache directory is still empty after
each run, so a native result cache (#1429) can never answer a timed run.

**Corpus** (read-only, enumerated at start; a benchmark count other than 23 is
corpus drift and exits `2`):

- the 23 non-stretch `benchmarks/*/reference.vow` files from `benchmarks/manifest.toml`;
- `tests/verify`, `tests/verify-fail`, `tests/verify-skip` (honouring `// TEST: skip`);
- `tests/verify-fail-multi/*/main.vow`;
- `tests/verify-native/{pass,fail,skip,unknown}` (truth from the directory);
- the float fixtures, which live inside the sets above;
- `tests/verify-stress` only with `--include-stress`. Its rows are reported with
  `gating: false` and never enter the ratios or the timeout criterion (times out
  by design).

**Metrics.** Wall-clock is the process wall time. Peak RSS is the largest of
three readings (`scripts/peak_rss.py`): the sampled sum of resident sets over the
whole process tree (driver plus solver children; sampled every 1 ms for the first
250 ms, then every 20 ms), the largest kernel high-water mark (`VmHWM`) any
sampled process reported (so a peak between two samples is still seen, and a run
of a few milliseconds is caught by one late read), and the kernel's `ru_maxrss`
for the reaped child when it is above the harness's own peak. Linux seeds a
child's `ru_maxrss` with the parent's high-water mark, so a kernel value at or
below that peak is not a measurement. When no reading saw the run, `ru_maxrss` is
still an upper bound of the child's peak: it is used, labelled
`ru_maxrss_upper_bound` in the row's `rss_sources`, for native (it can only raise
the native/ESBMC ratio), and is a harness failure naming the fixture for ESBMC,
whose peak is the denominator. A peak RSS of 0 never reaches a ratio. One
discarded warm-up run per backend precedes the timed runs, and the timed runs
alternate between backends so host drift hits both.

**Gate.** A ratio is `native / esbmc` of the per-backend medians.

| Criterion | Threshold |
| --- | --- |
| Geometric mean of the ratios, wall-clock and RSS | `<= 1.0` on both |
| Worst single fixture, wall-clock and RSS | `<= 1.25` |
| Timeouts | native times out on no fixture where ESBMC finishes |
| Comparable rows | at least one |

Only rows classed `match` by `verify_diff.py` (same verdict and counterexamples,
and the same set of skipped functions, so the same work) enter the ratios.
`more_precise`, `weaker`, `soundness`,
`harness`, a verdict that changes between runs of one backend (`unstable`) and
rows where either backend timed out are listed with their reason and left out of
the ratios. Verdict parity is gate item 1 and stays with `verify_diff.py`. The
timeout criterion looks at every row, so a native timeout still counts when the
row is otherwise excluded. A fixture times out on a backend when more than half
of its runs did (`verify_status` `timeout` or the watchdog fired).

By default neither backend gets `--timeout`, so each runs at its shipped
budget (an explicit `--timeout` suppresses ESBMC's BV phase and its fallback).
`--timeout N` passes the same budget to both and is recorded in the report.
Both backends run with `--verify-jobs 1` unless overridden.

**Report.** One JSON document (`schema_version`, `vowc`, `esbmc_version`,
`bitwuzla_version`, `host`, `runs`, `budget`, `timeout_s`, `verify_jobs`,
`corpus`, `rows[]`, `gate`) on stdout or in `--output FILE`, plus a human
summary on stderr. Each row carries both backends' medians, raw samples,
verdicts, `timeout_runs`, the `verify_diff` class, the ratios and the reason a
row was excluded. `gate` carries the criteria, per-metric geometric mean and
worst fixture, and the timeout lists (`native_only`, `esbmc_only`, `both`). A
run with `--runs` below 5, or with `--filter`, sets `gate.provisional`; the exit
code does not distinguish a provisional pass, so read `gate.provisional` before
treating `0` as the gate.

Exit codes: `0` gate passed, `1` gate failed, `2` the harness could not run
(missing `vowc`, `esbmc` or `bitwuzla`; ESBMC that is not 8.5; non-Linux host;
corpus drift; no fixture selected; an I/O error).

```bash
python3 scripts/verify_perf.py --vowc build/vowc --output /tmp/verify-perf.json
python3 scripts/verify_perf.py --filter benchmarks/E01 --runs 1   # smoke run
```

Run it on an idle host with ESBMC 8.5 and the pinned Bitwuzla on `PATH`. The
full run measures hundreds of fixtures six times per backend and takes hours. It
is an acceptance-gate tool, not a `full_test.sh` section: it is host-dependent
and its thresholds only become meaningful once native covers the corpus.

## Whole-compiler native verification

[`scripts/verify_compiler.py`](../scripts/verify_compiler.py) (epic #1398, issue
#1432) runs the native verifier over every source of the self-hosted compiler,
the real-world workload, and holds it to three claims. The unit tests are
`scripts/test_verify_compiler.py`.

**Roots.** `compiler/main.vow` plus every top-level module of `compiler/` that
`main.vow` does not reach through `use` (today `module_io.vow`, `span.vow` and
the `test_*.vow` programs), so every compiler source, including the `vc_*.vow`
verifier modules, is under a `vowc verify` run. `compiler/tests/` is not a root:
those programs only re-load the modules above.

**Verdicts.** Per function, from the driver's progress lines
(`  <fn>: PROVEN|SKIPPED|UNKNOWN|TIMEOUT|FAILED|ERROR|PANICKED`) classified with
the `scripts/verify_diff.py` classes, with ground truth `Verified` (the compiler
is expected to satisfy its own contracts). A function native is `weaker` on, or
refutes, fails the run unless it is a skip the next paragraph explains.

**Skip list.** [`scripts/native_compiler_skips.toml`](../scripts/native_compiler_skips.toml)
lists every function native still skips, with the closed-list reason code and the
issue that tracks the gap. A native `Skipped` is accepted only for a listed
function with the listed code; a skipped function that is not listed fails, and
so does a listed function no root skips any more, so the list only shrinks.
Delete the row in the commit that closes its issue.

**Modes.**

```bash
# Native vs ESBMC: verdicts and the cost gate of scripts/verify_perf.py
python3 scripts/verify_compiler.py --vowc build/vowc --output /tmp/verify-compiler.json
# Native only, several binaries: identical outcomes required (bootstrap stages)
python3 scripts/verify_compiler.py --native-only --vowc build/vowc --vowc build/vowc2
```

The comparison times `--backend esbmc` against `--backend native` per root with
the cache off, median of `--runs` runs (default 5, one discarded warm-up), both at
`--verify-jobs 1` unless overridden, and applies the `verify_perf.py` thresholds:
geometric mean of native/ESBMC at most 1.0 on wall-clock and peak RSS, no root
above 1.25, native timing out on no root where ESBMC finishes. A root where both
backends finish within one second is reported but not gated (it measures process
start-up). `--no-perf-gate` keeps the cost in the report and out of the exit
code; `--allow-esbmc-version` accepts an ESBMC other than the pinned 8.5 and marks
the report provisional (as does `--runs` below 5).

`--native-only` verifies once with each `--vowc` and requires the status, the
per-function verdicts, the skipped set and the counterexamples to agree, so a
native verdict never depends on which bootstrap stage compiled the verifier.

**Bootstrap hook.** `scripts/bootstrap.sh --native-verify` runs the native mode
over the Stage 1 binary (`build/vowc`, built by the Rust compiler) and the Stage 2
binary (`build/vowc2`) after the fixed point and before the promotion, writing
`build/native-verify.json`. It needs `bitwuzla` and no ESBMC, and is independent
of `--no-verify`. `VOW_BOOTSTRAP_NATIVE_JOBS` sets its `--verify-jobs` (default 2).

**CI.** [`compiler-native-verify.yml`](../.github/workflows/compiler-native-verify.yml)
runs both on every push to `main` and nightly: `bootstrap.sh --no-verify
--native-verify`, then the comparison with `--runs 1 --no-perf-gate` (shared runners
are too noisy to gate a ratio on). The cost gate is enforced by running the command
above on an idle host.

Exit codes: `0` every criterion holds, `1` a criterion failed, `2` the harness
could not run (missing `vowc`, `esbmc` or `bitwuzla`, a malformed skip list).

**Status at `78f48c2a` (2026-10-14, ESBMC 8.5.0, single runs on a loaded host).**

- Verdicts: native proves every function ESBMC proves except the 37 listed in the
  skip list (tracked by #1676, #1677, #1678, #1679, all open); there is no
  counterexample and no `weaker` function outside the list. Native proves
  `note_vals_has_root` (ESBMC inconclusive) and `module_io.vow`'s `write_u8`
  (ESBMC refutes it only through its vec capacity assertion).
- Stages: Stage 1 and Stage 2 `vowc` give identical outcomes on every root
  (`bootstrap.sh --native-verify`, 697 s at `--verify-jobs 2`).
- Cost: peak RSS is 0.25x ESBMC (geometric mean; `main.vow` 0.11x), within the
  gate. Wall-clock is not: the geometric mean is 1.38x and the worst root 2.11x,
  because each `verify-worker` re-runs the whole frontend, roughly 8 s on
  `compiler/main.vow`, once per verified function (#1683). That is a wall-clock
  gap of the driver, not of the solver, and it is why the CI comparison does not
  enforce the cost gate.

## Native oracles: runtime differential and Z3 cross-check

Two CI-only checks hold `vowc verify --backend native` to a second opinion
(epic #1398, issue #1431). Both run from
[`native-oracle.yml`](../.github/workflows/native-oracle.yml) on every push to
`main`, nightly, and on pull requests that touch the verifier, replay harness,
oracle scripts or fixtures, sharded four ways, over `tests/verify`, `tests/verify-fail`,
`tests/verify-native/{pass,fail,unknown}`, `examples/` and two `gen_corpus.py`
corpora (see [Generated verifier corpus](#generated-verifier-corpus)). The seeds
are pinned on pushes (`1431` equivalence axis, `1473` verifier axis), the UTC
date on the nightly and an explicit input on a manual run; they are echoed in
the log.
The decision record is
[ADR-2026-10-11-0510](adr/2026-10-11-0510-native-z3-cross-check.md).

**Verifier versus runtime**,
[`scripts/verifier_runtime.py`](../scripts/verifier_runtime.py) `--backend native`:

| Class | Meaning | Exit |
|-------|---------|------|
| `SOUNDNESS` | `Verified`, but the `--mode debug` binary reports a `VowViolation` | 1 |
| `PRECISION` | a counterexample replays `diverged` or `aborted`, or a `VerifierBug` diagnostic is emitted | 1 |
| `REPLAY-GAP` | a replay was `skipped` for a reason other than "not a runtime abort", and the fixture carries no `// TEST: replay-skipped <why>` | 1 |
| `unverified-call-site` | a Caller violation of a `requires` whose function a `VerificationSkipped` note names as not verified (calls from `main`); counted, not a failure | 0 |

`--min-checked N` exits 2 if fewer than N fixtures reached an `ok` verdict.
`results.json` carries `replay_skipped_by_reason`. Add a
`// TEST: replay-skipped <why>` line (at the end of the file, so spans do not
move) only when the replay harness genuinely cannot model the fixture; the
reason is then reported rather than failed.

**Z3 cross-check**, [`scripts/z3_crosscheck.py`](../scripts/z3_crosscheck.py):
`vowc verify --backend native --no-cache` runs with a `bitwuzla` shim first on
`PATH` that copies each `.smt2` file it is handed and then `exec`s the real
solver, with `VOW_VERIFY_NO_INCREMENTAL=1` so every claim is a standalone file.
Each captured query is re-run under Bitwuzla (`--bv-output-format 2`) and Z3
(`-smt2 -T:N`).

| Class | Bitwuzla / Z3 | Exit |
|-------|---------------|------|
| `agree` | `sat`/`sat` or `unsat`/`unsat` | 0 |
| `SOUNDNESS` | `unsat` / `sat`: a false proof | 1 |
| `PRECISION` | `sat` / `unsat`: a spurious counterexample | 1 |
| `inconclusive` | `unknown` or a timeout on either side, or a Bitwuzla error | 0 |
| harness | a Z3 error, no query captured, a solver session bypassing capture, fewer than `--min-agreed` agreements, an unusable verifier | 2 |

A disagreement leaves the query under `queries/` in `--output-dir`, which is
the repro:

```bash
bitwuzla --bv-output-format 2 z3-crosscheck.out/queries/<fixture>.<n>.smt2
z3 -smt2 z3-crosscheck.out/queries/<fixture>.<n>.smt2
```

```bash
scripts/bootstrap.sh --skip-cargo --no-verify           # build/vowc
python3 scripts/verifier_runtime.py --backend native --shard 0/4
python3 scripts/z3_crosscheck.py --shard 0/4 --extra-root /path/to/generated
```

Not covered: incremental-session streams (a session `unsat` is not re-decided
by the product), and runtime soundness depth beyond what each program's own
`main` (or a generated verifier-axis `main`) exercises.

### Generated verifier corpus

`gen_corpus.py --axis verifier --seed N --count M` (module
[`scripts/gen_verifier_corpus.py`](../scripts/gen_verifier_corpus.py), issue
#1473) writes contracted programs over a `Vec` or a counted loop, and a `main`
that calls each generated function with concrete inputs that satisfy its
`requires`. The #905 equivalence axis never calls what it generates, so it only
adds query volume; this axis gives the runtime a say. Every program carries
`// TEST: gen-class <class> <shape> <regime>`:

| Class | Contract | Expectation |
|-------|----------|-------------|
| `exceeds-bound` | false, but only once the collection grows or the loop runs past a boundary (7, 8, 16, 17, 31..33, 63..65, 100, 127..129, 255..257): the ESBMC `Vec` capacity (128) and the native unwind bound (64) with their neighbours | `main` drives past the boundary, so the debug binary reports a `VowViolation`; `Verified` is a `SOUNDNESS` finding, `Violated`/`unknown` are fine |
| `control` | the same shape, true | `// TEST: expect-verified`: it must be `Verified` and run clean |

The regime (`within-unwind`, `beyond-unwind`) says which side of the native
verifier's internal 64-iteration unrolling bound the longest loop sits on: past
it a loop is decided only by an invariant. Shapes cover literal-count fills,
nested fills, straight-line push chains (no loop at all), pops, element
read-back through `result[i]`, in-place overwrite, a scalar accumulator, and a
`Vec` *parameter* longer than the bound the contract names.

`verifier_runtime.py` adds three verdicts for generated programs, all exit 1:

| Verdict | Meaning |
|---------|---------|
| `WITNESS-BROKEN` | the debug binary does not match the class: an `exceeds-bound` program that never violates, or whose violation blames the Caller (`main` broke a `requires`, which `Verified` may disclaim) instead of an `ensures`; a `control` that violates. Checked against the runtime, whatever the verifier said, so a green sweep cannot be a vacuous one |
| `CONTROL-LOST` | a `control` that is not `Verified` |
| `STALE-KNOWN-GAP` | a `known-control-gap` that now verifies; drop its entry |

The native verifier declares no collection capacity (a `Vec` is an array plus a symbolic
length, ADR-2026-10-08-1422), which `test_gen_verifier_corpus.py` pins by scanning
`compiler/vc_*.vow`; the collection boundaries are therefore the capacities the ESBMC
model had (`VEC_MODEL_CAP` = 128, also pinned) that a port of that model would bring back.

**Known disagreements are tracked by issue number, never skipped.**
`KNOWN_DISAGREEMENTS` in `gen_verifier_corpus.py` maps `(shape, class, regime)`
to an issue (`validate_known` rejects an entry without one); the generator
stamps matching programs with `// TEST: known-control-gap #N` or
`// TEST: known-soundness-gap #N`, which the runner reports and does not fail.
There are no entries today: the controls that fill a `Vec` in a loop past the
unrolling bound (#1645) verify through the loop cut. `REPLAY_SKIPS` is empty
too: `--replay-cex` rebuilds `Vec`, `String`, aggregate, narrow, 128-bit and `f64`
parameters (#1649), so no generated shape needs a `replay-skipped` line (an entry is
`(shape, class) -> "why (#N)"`, rendered at the end of the file). A hand-written
fixture keeps one for a linear struct parameter or result, which the replay does
not rebuild yet (#1649), and for an `f32` parameter, which no source text denotes. Only a `known-control-gap` goes stale (a control that now
verifies fails the run); a `known-soundness-gap` that stops reproducing
is not detected. The runner does not apply `ulimit -v`: the native verifier worker carries its
own address-space cap and the generated programs are small and bounded. A real
soundness disagreement is a native verifier bug: file an issue with the saved
reproducer, label it `verifier`, and track it here; do not weaken the program.

Reproduce a run from its echoed seed:

```bash
python3 scripts/gen_corpus.py --axis verifier --seed 1473 --count 300 --output-dir /tmp/gv
python3 scripts/verifier_runtime.py --backend native --extra-root /tmp/gv
python3 scripts/z3_crosscheck.py --extra-root /tmp/gv
```
