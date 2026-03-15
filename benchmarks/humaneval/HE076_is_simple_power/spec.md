# HE076: Is Simple Power

**Origin:** HumanEval-076 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

function_signature: def is_simple_power(x: int, n: int) -> bool
Your task is to write a function that returns true if a number x is a simple power of n and false in other cases. x is a simple power of n if n**int=x

## Signature

```vow
fn is_simple_power(x: i64, n: i64) -> i64
```

## Contracts

- `requires: x >= 0`
- `requires: x <= 100`
- `requires: n >= 0`
- `requires: n <= 100`
- `ensures: result >= 0`
- `ensures: result <= 1`

## Contract Fidelity

**PARTIAL** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method is_simple_power(x: nat, n: int) returns (ans : bool)

    requires x > 0

    ensures ans <==> exists y :: n == power(x, y)
```

## Hints

- TODO: add implementation hints
