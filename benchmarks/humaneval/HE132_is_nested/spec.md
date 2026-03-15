# HE132: Is Nested

**Origin:** HumanEval-132 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

function_signature: def is_nested(string: str) -> Bool
Create a function that takes a string as input which contains only parentheses. The function should return True if and only if there is a valid subsequence of parentheses where at least one parenthesis in the subsequence is nested.

## Signature

```vow
fn is_nested(s: Vec<i64>) -> i64
```

## Contracts

- `requires: s.len() >= 0`
- `requires: s.len() <= 8`
- `ensures: result >= 0`
- `ensures: result <= 1`

## Contract Fidelity

**PARTIAL** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method is_nested(s: seq<int>) returns (res: bool) 

    ensures res == exists x, y, z, w :: 0 <= x < y < z < w < |s| && s[x] == 0 && s[y] == 0 && s[z] == 1 && s[w] == 1
```

## Hints

- TODO: add implementation hints
