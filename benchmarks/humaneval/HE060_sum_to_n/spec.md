# HE060: Sum from 1 to N

**Origin:** HumanEval-060 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

Compute the sum of all integers from 1 to n inclusive. The expected result is
`n * (n + 1) / 2`.

## Signature

```vow
fn sum_to_n(n: i64) -> i64
```

## Contracts

- `requires: n >= 1` — positive input
- `requires: n <= 100` — bounded for verification
- `ensures: result == n * (n + 1) / 2` — exact formula

## Contract Fidelity

**EXACT** — the Vow contracts fully capture the Dafny specification.

## Hints

- Can be implemented directly with the formula, or iteratively with a loop
- If using a loop, track a running sum with an invariant
