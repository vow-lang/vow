# HE123: Get Odd Collatz

**Origin:** HumanEval-123 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

function_signature: method get_odd_collatz(n: nat) returns (sorted: seq<int>)
Retrieve elements. Requires: requires n > 1. Ensures: the result is sorted according to the ordering relation; the result is sorted according to the ordering relation.

## Signature

```vow
fn get_odd_collatz(n: i64) -> Vec<i64>
```

## Contracts

- `requires: n >= 0`
- `requires: n <= 100`
- `ensures: result.len() >= 0`

## Contract Fidelity

**PARTIAL** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method get_odd_collatz(n: nat) returns (sorted: seq<int>)
  decreases *
  requires n > 1
  ensures forall i, j :: 0 <= i < j < |sorted| ==> sorted[i] <= sorted[j]
  ensures forall i :: 0 <= i < |sorted| ==> sorted[i] % 2 == 1
```

## Hints

- TODO: add implementation hints
