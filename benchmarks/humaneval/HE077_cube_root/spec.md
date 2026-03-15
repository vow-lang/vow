# HE077: Cube Root

**Origin:** HumanEval-077 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

function_signature: method cube_root(N: nat) returns (r: nat)
Find the integer cube root. Ensures: the result r is the largest integer such that r³ ≤ N < (r+1)³; the result is at most N.

## Signature

```vow
fn cube_root(n: i64) -> i64
```

## Contracts

- `requires: n >= 0`
- `requires: n <= 100`
- `ensures: result >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method cube_root(N: nat) returns (r: nat)

  ensures cube(r) <= N < cube(r + 1)
  ensures r <= N
```

## Hints

- TODO: add implementation hints
