# HE084: Solve

**Origin:** HumanEval-084 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

function_signature: def solve(n: int) -> str
Given a positive integer N, return the total sum of its digits in binary.

## Signature

```vow
fn solve(n: i64) -> i64
```

## Contracts

- `requires: n >= 0`
- `requires: n <= 100`
- `ensures: result >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method solve(n: nat) returns (r: nat)

  ensures r == popcount(n)
```

## Hints

- TODO: add implementation hints
