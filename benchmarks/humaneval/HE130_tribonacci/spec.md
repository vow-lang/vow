# HE130: Tribonacci

**Origin:** HumanEval-130 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

function_signature: def tri(n: int) -> List[int]
Everyone knows Fibonacci sequence, it was studied deeply by mathematicians in the last couple centuries. However, what people don't know is Tribonacci sequence. Tribonacci sequence is defined by the recurrence: tri(1) = 3 tri(n) = 1 + n / 2, if n is even. tri(n) =  tri(n - 1) + tri(n - 2) + tri(n + 1), if n is odd. For example: tri(2) = 1 + (2 / 2) = 2 tri(4) = 3 tri(3) = tri(2) + tri(1) + tri(4) = 2 + 3 + 3 = 8 You are given a non-negative integer number n, you have to a return a list of the first n + 1 numbers of the Tribonacci sequence.

## Signature

```vow
fn tribonacci(n: i64) -> Vec<i64>
```

## Contracts

- `requires: n >= 0`
- `requires: n <= 10`
- `ensures: result.len() >= 0`

## Contract Fidelity

**PARTIAL** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method Tribonacci(n: nat) returns (result: seq<nat>)

  ensures |result| == n + 1
  ensures forall i :: 0 <= i <= n ==> result[i] == tri(i)
```

## Hints

- TODO: add implementation hints
