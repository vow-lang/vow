# HE005: Insert Delimiter

**Origin:** HumanEval-005 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

Implement a function that inserts a delimiter value between every two
consecutive elements in a Vec of integers. For a Vec with 0 or 1 elements,
return a copy. For longer Vecs, the result should have alternating original
elements and delimiters.

Example: `[1, 2, 3]` with delimiter `0` produces `[1, 0, 2, 0, 3]`.

## Signature

```vow
fn insert_delimiter(nums: Vec<i64>, delim: i64) -> Vec<i64>
```

## Contracts

- `requires: nums.len() >= 0` — valid input
- `requires: nums.len() <= 8` — bounded for verification
- `ensures: result.len() >= nums.len()` — result is at least as long

## Contract Fidelity

**PARTIAL** — the Dafny spec ensures exact length (`2 * |numbers| - 1` for
len > 1), element positions (`result[2*i] == numbers[i]`), and delimiter
positions (`result[2*i+1] == delimiter`). Vow can express the length bound
but not the element-wise quantified properties.

## Hints

- If the input has 0 or 1 elements, return a copy
- Otherwise, push each element followed by the delimiter, except the last element
- Track the loop counter with invariants
