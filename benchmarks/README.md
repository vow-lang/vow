# Vericoding Benchmark Suite

A suite of 40 specification-driven programming problems for evaluating AI agents' ability to write verified code in Vow.

## Overview

Each benchmark provides:
- **spec.md** — Natural language description of the problem, constraints, and hints
- **skeleton.vow** — Function signatures with contracts but placeholder bodies (`{ 0 }`)
- **reference.vow** — Verified reference implementation (ground truth)
- **meta.toml** — Machine-readable metadata (difficulty, tags, expected status)

An agent's task: read `spec.md` and `skeleton.vow`, fill in the function bodies so that `vow verify` reports `Verified`.

## Difficulty Tiers

| Tier | Count | Description |
|------|-------|-------------|
| **Easy** (E01–E15) | 15 | Single-function, base-type contracts (arithmetic, branching) |
| **Medium** (M01–M15) | 15 | Multi-function, collections, loop invariants |
| **Hard** (H01–H10) | 10 | Multi-module, stateful structs, interacting invariants |

## Running Benchmarks

```bash
# Verify a single reference implementation
ulimit -v 2000000; build/vowc verify benchmarks/easy/E01_absolute_value/reference.vow

# Verify an agent's solution (replace reference.vow with the agent's output)
ulimit -v 2000000; build/vowc verify solution.vow

# Compile a skeleton (should succeed — placeholder body compiles)
ulimit -v 2000000; build/vowc build --no-verify benchmarks/easy/E01_absolute_value/skeleton.vow
```

## Scoring

- **Verified**: `vow verify` returns `{"status":"Verified"}` — 1 point
- **Failed**: Verification fails or times out — 0 points
- **Score**: Verified count / Total applicable (excluding `expected_status = "Stretch"`)

Seventeen problems are currently Stretch. The original four Hard boundary
problems remain in that tier; thirteen additional arithmetic, loop, and
collection problems use honest contracts that exceed the current verifier's
unwind or model reach. They remain in the suite but are excluded from the
primary score denominator until the verifier can establish them without source
contract bounds.

## Verifier Model

ESBMC verification uses:
- a finite loop-unwind strategy
- Vec capacity ≤ 128 elements
- String length ≤ 256 bytes
- HashMap capacity ≤ 64 entries

These are internal verifier limits, not benchmark-domain constraints. Benchmark
contracts must remain functional and backend-independent: do not cap numeric
inputs, collection lengths or capacities, or struct fields merely to fit the
unwind limit or reduce solver state. A benchmark the current verifier cannot
establish with its honest contract belongs in the existing Stretch category.

## Contract Syntax Reference

```vow
fn example(x: i64 where x >= 0, y: i64) -> i64 vow {
  requires: y > 0,
  ensures: result >= 0
} {
  x / y
}

while cond vow {
  invariant: i >= 0,
  invariant: i <= n
} {
  // loop body
}
```

- `requires:` — precondition (caller's responsibility)
- `ensures:` — postcondition (callee's responsibility)
- `where` — inline parameter constraint (syntactic sugar for requires)
- `invariant:` — loop invariant (must hold at entry and after each iteration)
- `result` — refers to the function's return value in ensures clauses

## Manifest

`manifest.toml` lists all 40 benchmarks with paths and metadata for tooling integration.

## Comparison Targets

From the Vericoding paper (arxiv.org/abs/2509.22908):
- Dafny: 82% verification rate
- Verus/Rust: 44%
- Lean: 27%

Vow's hypothesis: blame-tracking contracts + structured counterexamples + CEGIS-ready pipeline yield higher verification rates.
