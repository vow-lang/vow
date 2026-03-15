# HE159: Eat

**Origin:** HumanEval-159 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This verification task involves implementing a method that calculates carrot consumption for a rabbit. Given the number of carrots already eaten, the number of additional carrots needed, and the number of carrots remaining in stock, the method should return the total carrots that will be eaten and how many carrots will be left. The rabbit will eat as many carrots as possible from the remaining stock, up to the number needed.

## Signature

```vow
fn eat(number: i64, need: i64, remaining: i64) -> Vec<i64>
```

## Contracts

- `requires: number >= 0`
- `requires: number <= 100`
- `requires: need >= 0`
- `requires: need <= 100`
- `requires: remaining >= 0`
- `requires: remaining <= 100`
- `ensures: result.len() >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method eat(number: int, need: int, remaining: int) returns (result: seq<int>)
    requires ValidInput(number, need, remaining)
    ensures ValidResult(result, number, need, remaining)
```

## Hints

- TODO: add implementation hints
