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
All 7 categories are represented (157 programs):

| Category | Count |
| --- | --- |
| callee-blame | 52 |
| model-drift | 29 |
| bounds | 28 |
| overflow | 19 |
| unverifiable | 15 |
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
with the ground truth `verify_eval.py` reads) under
`vowc verify --backend esbmc` and `vowc verify --backend native`, and classifies
every fixture:

| Class | Meaning |
| --- | --- |
| `match` | Same verdict; when both refute, the same `(function, blame, vow_id)` counterexamples. |
| `more_precise` | Native concludes where ESBMC does not, reports an extra counterexample, or refutes a program ESBMC proves and the corpus labels incorrect. |
| `weaker` | Native is `Skipped`/`unknown`/`timeout`/`tool_not_found` where ESBMC proves or refutes, drops or re-attributes a counterexample, or refutes a program the corpus labels correct. **Fails the script.** |
| `soundness` | Native proves a program ESBMC refutes, or one the corpus labels incorrect. **Fails the script.** |
| `harness` | A backend printed no parseable JSON, so the row says nothing about the verifiers. A backend that hangs (killed at its budget) or dies from a signal is not a `harness` row: it is inconclusive (`verify_status` `timeout` / `crashed`). |

Native `verify_status` of `panicked`, `error` or `crashed` is `weaker` even when
ESBMC is also inconclusive. File-level status is per module: native reports
`Skipped` for a file when any one function is outside its subset, so a partly
modelable file reads as `weaker` against an ESBMC `Verified` until the subset
grows.

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

## Performance gate: native verifier vs ESBMC

[`scripts/verify_perf.py`](../scripts/verify_perf.py) (epic #1398, issue #1420)
encodes acceptance gate item 3. It times `vowc verify --backend esbmc` against
`--backend native` with the cache off (`--no-cache`, fresh `VOW_CACHE_DIR` per
run), one fixture at a time on one machine, and reports the median of five runs
per backend.

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

**Metrics.** Wall-clock is the process wall time. Peak RSS is the larger of the
sampled sum over the whole process tree (driver plus solver children, 20 ms
interval) and the kernel's `ru_maxrss` for the run. Linux seeds a child's
`ru_maxrss` with the parent's high-water mark, so a kernel value at or below the
harness's own peak is ignored. One discarded warm-up run per backend precedes
the timed runs, and the timed runs alternate between backends so host drift hits
both.

**Gate.** A ratio is `native / esbmc` of the per-backend medians.

| Criterion | Threshold |
| --- | --- |
| Geometric mean of the ratios, wall-clock and RSS | `<= 1.0` on both |
| Worst single fixture, wall-clock and RSS | `<= 1.25` |
| Timeouts | native times out on no fixture where ESBMC finishes |
| Comparable rows | at least one |

Only rows classed `match` by `verify_diff.py` (same verdict and counterexamples,
so the same work) enter the ratios. `more_precise`, `weaker`, `soundness`,
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
