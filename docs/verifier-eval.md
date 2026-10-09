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
| `// TEST: counterexample-blame <caller\|callee\|none>` | Expected blame; `none` = a memory-safety/builtin failure with no contract attribution. |
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
All 7 categories are represented (150 programs):

| Category | Count |
| --- | --- |
| callee-blame | 48 |
| bounds | 28 |
| model-drift | 27 |
| overflow | 19 |
| unverifiable | 14 |
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
scripts/bootstrap.sh --stage3-no-verify
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
