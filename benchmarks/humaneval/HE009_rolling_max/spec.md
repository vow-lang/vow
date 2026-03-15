# HE009: Rolling Maximum

**Origin:** HumanEval-009 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

Implement a rolling maximum function that takes a list of integers and returns
a list where each element represents the maximum value encountered from the
beginning of the list up to and including the current position.

Example: `[1, 3, 2, 5, 1]` produces `[1, 3, 3, 5, 5]`.

## Signature

```vow
fn rolling_max(nums: Vec<i64>) -> Vec<i64>
```

## Contracts

- `requires: nums.len() >= 0` — valid input
- `requires: nums.len() <= 8` — bounded for verification
- `ensures: result.len() == nums.len()` — output length matches input

## Contract Fidelity

**PARTIAL** — the Dafny spec additionally ensures that each result element
equals `max_up_to(numbers, i)` (a recursive spec function), and that each
result element is >= all preceding input elements and equals some input element.
Vow can only express the length preservation property.

## Hints

- Track a running maximum in a mutable variable
- At each step, update the max and push it to the output Vec
- Initialize the running max with the first element
