# HE026: Count

**Origin:** HumanEval-026 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

function_signature: method count(a: seq<int>, x: int) returns (cnt: int)
Count occurrences. Ensures: returns the correct count; returns the correct count.

## Signature

```vow
fn count(a: Vec<i64>, x: i64) -> i64
```

## Contracts

- `requires: a.len() >= 0`
- `requires: a.len() <= 8`
- `requires: x >= 0`
- `requires: x <= 100`
- `ensures: result >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method count(a: seq<int>, x: int) returns (cnt: int)

  ensures cnt == |set i | 0 <= i < |a| && a[i] == x|
  ensures cnt == count_rec(a, x)
```

## Hints

- TODO: add implementation hints
