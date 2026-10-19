# Explained differential rows

`scripts/verify_diff.py` (see [verifier-eval.md](verifier-eval.md), "Differential harness") compares the
native verifier with ESBMC on every corpus fixture. A row that would read as `soundness` or `weaker` can
carry a written explanation, a `// TEST:` directive next to the fixture. `verify_diff.py` and
`verify_perf.py` classify through one function (`verify_diff.classify_outcomes`), and
`verify_eval.py --backend native` reads the same artefact directives, so the harnesses agree.

## ESBMC model artefacts

A fixture the native verifier proves and ESBMC refutes is a `soundness` row unless the fixture
carries a written explanation, a `// TEST:` directive that `verify_eval.artifact_directives` parses
(fail closed: a malformed or misspelled `esbmc-*` directive is an error).

| Directive | Meaning |
| --- | --- |
| `// TEST: esbmc-capacity-artifact "<why>"` | ESBMC refutes only through its own `string`/`vec` capacity assertion. Needs the evidence of the run: every ESBMC counterexample is a capacity assertion. |
| `// TEST: esbmc-model-artifact fn="<fn>" blame=<B> vow_id=<N> "<why>"` | One ESBMC counterexample comes from ESBMC's own model, not from the program (for example its nondeterministic `string_eq`, or a Vec field it reads afresh at each use). One directive per counterexample. |

The model-artefact directive explains two kinds of row, and in both every ESBMC counterexample that
native does not report must be declared:

- native proves, ESBMC refutes, and the corpus labels the fixture correct (`Verified`): the row is
  `more_precise`; any other ESBMC refutation of the file is still a `soundness` finding.
- both refute, and the corpus labels the fixture incorrect (`VerifyFailed`): ESBMC reports its own
  artefact (an `index out of bounds` or a capacity assertion with `blame=None vow_id=4294967293`)
  where native reports the failure the program really has. The row is `more_precise`; a declared
  counterexample of a `Verified` fixture, or an undeclared one native lacks, stays `weaker`.

Add a directive only after the runtime differential
(`scripts/verifier_runtime.py --backend native`) shows no false proof for the
fixture, and write the reason as the explanation gate item 1 asks for. The
fixture's own `main` exercises one input, so for a function with a wider domain
also run the fixture's functions over many inputs that satisfy their `requires`
in `--mode debug`, with a deliberately broken contract as a control that must
abort. For a refuted fixture the control is the failure itself: the replay must abort only on the
violation the native counterexample names, never on an out-of-bounds read or a capacity assertion.
`scripts/z3_crosscheck.py` over the fixture checks the native queries against a second solver.

ESBMC's `verify` report names only the first function it refutes, although it
verifies every function: its stderr lists each `FAILED` one. When a fixture
has several, declare every one (peel the first off a copy of the file and
re-run to read the next one's `fn`, `blame` and `vow_id`); the classifier needs
only the one the report shows, but the others are the evidence that the whole
file is explained and not just its first refutation. A refutation by ESBMC's own
`string`/`vec` capacity assertion in a fixture that also has a model artefact is
declared with `esbmc-model-artifact` and `blame=None vow_id=4294967293` (the
unattributed property), because `esbmc-capacity-artifact` covers only fixtures
whose every counterexample is a capacity assertion.

## Tracked native gaps

Where native is `Skipped` or `unknown` and ESBMC decides, the row is `weaker` and the fixture is lost
coverage once native is the default. A fixture that documents such a gap says so, with an open issue
that closes it:

```
// TEST: native-gap "<why native declines>" #<issue>
```

The reason and the issue number are mandatory, there is at most one per fixture, and a malformed line
is an error (`verify_diff_explain.native_gap`; a unit test reads every directive of the corpus). The row
becomes `tracked`: it stays in every report and in the summary count, it does not fail the exit status,
and it applies only to `vowc verify` rows where native is `Skipped` or `unknown`. A native refutation of
a program the corpus labels correct, a crash, a panic, an error, a timeout and a row that is not `weaker`
are never excused by it. When native starts to decide the fixture the row is `match` again and the
directive is to be removed with the change that closes the issue.
