# SAT Demo

`examples/sat/` is a self-contained DIMACS CNF SAT solver demo for Vow. The target is not a toy DPLL parser; it is a deterministic CDCL solver with watched literals, first-UIP learning, non-chronological backtracking, integer activity scoring, phase saving, Luby restarts, learned-clause cleanup, and periodic compaction.

## Scope

- Input: DIMACS CNF from a positional file path or from `stdin`
- Output:
  - `SAT` plus one or more `v ... 0` lines
  - `UNSAT`
  - exit code `10` for `SAT`, `20` for `UNSAT`, `1` for parse or internal errors
- Flags:
  - `--help`
  - `--stats`

The parser is strict and low-allocation:

- exactly one `p cnf <vars> <clauses>` header
- comment lines starting with `c`
- zero-terminated clauses
- out-of-range literals rejected
- clause-count mismatch rejected
- parser-time duplicate literal cleanup and tautology dropping

## Build And Run

Use the self-hosted compiler directly:

```sh
TMPDIR=/dev/shm build/vowc build --no-verify examples/sat/main.vow -o examples/sat/.local/sat
```

Run on a file:

```sh
examples/sat/.local/sat path/to/formula.cnf
```

Run from `stdin`:

```sh
cat path/to/formula.cnf | examples/sat/.local/sat
```

Optional stats go to `stderr`:

```sh
examples/sat/.local/sat --stats path/to/formula.cnf
```

If your environment has a tight `/tmp` quota, keep `TMPDIR=/dev/shm` for builds.

## Local Tests

The demo keeps its regression tests local:

```sh
examples/sat/run_tests.sh
```

The shell runner compiles the binary, exercises file and `stdin` modes, checks the CLI contract, and covers malformed-input cases.

## Benchmarks

The curated benchmark subset lives in [benchmarks.json](./benchmarks.json). It is split into:

- `smoke`
- `core`
- `stretch`

Download the selected subset:

```sh
python examples/sat/bench.py download --tier smoke --tier core
```

Run the local benchmark harness:

```sh
python examples/sat/bench.py run --tier smoke --tier core
```

The harness will:

- build the Vow SAT solver unless `--solver` is provided
- detect local baseline solvers such as `minisat`, `kissat`, or `cadical` on `PATH`
- skip missing baselines cleanly
- validate `SAT` assignments from the Vow solver
- write a JSON report to `examples/sat/.local/results/latest.json`

Downloaded and generated artifacts stay under `examples/sat/.local/`.

## Current Benchmark Snapshot

On the current local machine, no baseline solvers were available on `PATH`, so the local report currently measures only the Vow SAT binary itself.

Latest local benchmark runs on the current machine:

- `smoke`: 1 solved `SAT`, 2 timeouts
- `core`: 4 timeouts

The previous false-`UNSAT` regression on satisfiable `core` instances has been fixed with a watch-bucket retention fix in propagation, and that reduced case is now covered by a local regression fixture. The remaining problem on `core` is now performance rather than obvious wrong-answer behavior.

## Notes

- This demo assumes the runtime container-cap blocker from issue `#156` is fixed in the compiler/runtime used to build it.
- Integer activities are used intentionally in the first version. Float-based heuristic work remains part of the SAT-driven Vow audit, especially for EVSIDS-style decay and the current “floats in contracts” limitations.
- If branching-variable selection becomes a measured hotspot, issue `#165` tracks adding heap / priority queue support to Vow’s standard library.

## Vow Findings

- Issue `#165`: the standard library still lacks a heap / priority queue, so milestone 1 uses a linear scan for branching instead of a max-heap.
- Issue `#166`: local variable assignments immediately before `break` in a `loop { ... }` can be lost in compiled code. The SAT solver hit this in conflict analysis and now avoids that loop pattern.
- The local environment needed `TMPDIR=/dev/shm` for reliable linking because `/tmp` and the main filesystem were effectively quota-constrained during development. That is an environment issue rather than a SAT-specific requirement, but it affects practical iteration.

## Float Audit

Future SAT heuristics would benefit from better float ergonomics in Vow:

- EVSIDS-style multiplicative decay is more natural with `f64` than with integer bump-and-rescale.
- Clause activity and restart heuristics often use float-based scoring or smoothing.
- Benchmark-oriented tuning is easier when float behavior is boring and predictable across optimized builds.

Current reasons this demo stayed integer-only:

- Vow documentation still describes float support as limited.
- Floats are awkward in contracts and verification-heavy code paths.
- Float remainder and some verification/codegen expectations are not yet mature enough to make them the default choice for the first serious SAT demo.
