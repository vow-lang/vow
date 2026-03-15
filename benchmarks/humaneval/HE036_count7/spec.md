# HE036: Count7

**Origin:** HumanEval-036 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

function_signature: method count7(x: nat) returns (count: nat)
Count occurrences. Ensures: returns the correct value.

## Signature

```vow
fn count7(x: i64) -> i64
```

## Contracts

- `requires: x >= 0`
- `requires: x <= 100`
- `ensures: result >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method count7(x: nat) returns (count: nat) 

  ensures count == count7_r(x)
```

## Hints

- TODO: add implementation hints
