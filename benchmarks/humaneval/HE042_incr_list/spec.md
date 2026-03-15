# HE042: Increment List

**Origin:** HumanEval-042 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

Implement a function to increment each element in a list of integers by 1.
Given a Vec of integers, return a new Vec where each element is the
corresponding element from the input plus one.

## Signature

```vow
fn incr_list(l: Vec<i64>) -> Vec<i64>
```

## Contracts

- `requires: l.len() >= 0` — valid input
- `requires: l.len() <= 8` — bounded for verification
- `ensures: result.len() == l.len()` — output length matches input

## Contract Fidelity

**PARTIAL** — the Dafny spec additionally ensures `forall i :: 0 <= i < |l| ==>
result[i] == l[i] + 1` (element-wise correctness). Vow cannot express universal
quantifiers, so only the length preservation is verified. ESBMC still verifies
loop invariants and bounds.

## Hints

- Create a new Vec and push each element + 1
- Use a while loop with index tracking
- Loop invariant should track the index bounds
