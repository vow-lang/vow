# HE150: X Or Y

**Origin:** HumanEval-150 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

function_signature: def x_or_y(int n, int x, int y) -> int
A simple program which should return the value of x if n is a prime number and should return the value of y otherwise.

## Signature

```vow
fn x_or_y(n: i64, x: i64, y: i64) -> i64
```

## Contracts

- `requires: n >= 0`
- `requires: n <= 100`
- `requires: x >= 0`
- `requires: x <= 100`
- `requires: y >= 0`
- `requires: y <= 100`
- `ensures: result >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method x_or_y(n: nat, x: int, y: int) returns (result: int)

  ensures IsPrime(n) ==> result == x
  ensures !IsPrime(n) ==> result == y
```

## Hints

- TODO: add implementation hints
