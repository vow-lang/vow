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
| `tests/debug/` | runtime `VowViolation` blame (debug mode) | yes (via `full_test.sh`) |

`tests/verify-stress/` is **not** wired into CI or `full_test.sh`: its outcomes
depend on the ESBMC unwind budget and host, so asserting a fixed result would be
flaky. It documents that the verifier degrades *safely* (fails closed, never
falsely accepts) on intractable inputs. See
[`tests/verify-stress/README.md`](../tests/verify-stress/README.md).

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
| `// TEST: known-soundness-gap "<reason>" #<issue>` | Marks a documented false-accept the verifier does not yet catch. Reported under the KNOWN SOUNDNESS GAPS banner, non-fatal — until the verifier *starts* catching it, at which point the harness fails and demands promotion to a real verify-fail program. |
| `// TEST: status <Status>` / `// TEST: skip "<reason>"` | Override the directory's expected status / exclude a program. |

## How the harness classifies results

For each program `verify_eval.py` runs `vow verify` (status + counterexamples)
and, for should-pass programs, `vow contracts --verify` as a vacuity guard. Each
result is bucketed:

- **SOUNDNESS** — expected-fail program reported `Verified`, or a should-pass
  program proven vacuously. **Hard failure.**
- **PRECISION** — should-pass program reported `VerifyFailed`/`Skipped`.
- **BLAME / VOW_ID** — wrong blame or wrong violated `vow_id`.
- **STATUS** — any other expected/actual status mismatch.
- **KNOWN SOUNDNESS GAPS** — tracked false-accepts (non-fatal).
- **KNOWN GAP APPEARS FIXED** — a known gap the verifier now catches (**hard
  failure**: promote the label).

Exit code is non-zero on any bucket except known-gaps. A machine-readable
`report.json` is written to the `--output-dir`.

## Category coverage

All seven categories are represented (41 programs):

| Category | Count |
| --- | --- |
| overflow | 7 |
| callee-blame | 13 |
| bounds | 5 |
| model-drift | 6 |
| invariant | 4 |
| caller-blame | 4 |
| unverifiable | 2 |

## Known soundness gaps

- **Caller obligations are not checked statically (#764).** `vow verify` treats
  a callee's `requires` purely as an assumption and never asserts it at
  in-module call sites, so a caller passing a provably-out-of-contract argument
  is silently `Verified`. Static `blame=Caller` is therefore not producible for
  ordinary in-module code; caller blame is observable only at runtime
  (`tests/debug/caller_blame_debug.vow`). The static side is carried as a
  `known-soundness-gap` entry (`tests/verify/caller_requires_unchecked.vow`).
  When #764 is fixed, that entry must become a `tests/verify-fail/` program
  asserting `blame=Caller`.

## Running

```bash
# Local (uses target/release/vow by default):
cargo build --release -p vow
python3 scripts/verify_eval.py

# Authoring aid — print actual outcomes for every program:
python3 scripts/verify_eval.py --discover

# A single program:
python3 scripts/verify_eval.py --filter off_by_one_bounds
```

It also runs as **Section 4e** of `scripts/full_test.sh` and as a dedicated step
in the `build-and-test` CI job, so a soundness or blame regression blocks PRs.
