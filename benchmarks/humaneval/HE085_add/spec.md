# HE085: Add

**Origin:** HumanEval-085 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

function_signature: def solve(n: list[int]) -> int
Given a non-empty list of integers lst, add the even elements that are at odd indices.

## Signature

```vow
fn add(v: Vec<i64>) -> i64
```

## Contracts

- `requires: v.len() >= 0`
- `requires: v.len() <= 8`
- `ensures: result >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method add(v: seq<int>) returns (r : int)

    ensures r == sumc(v, add_conditon(v))
```

## Hints

- TODO: add implementation hints
