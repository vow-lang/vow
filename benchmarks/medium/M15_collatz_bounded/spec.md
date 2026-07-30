# M15: Collatz Bounded

## Problem

Implement a function that counts Collatz steps for inputs from 1 through 27.

## Signature

```vow
fn collatz_steps(n: i64) -> i64
```

## Contracts

- `requires: n >= 1` — starting value is positive
- `requires: n <= 27` — intentional bounded benchmark domain
- `ensures: result >= 0` — step count is non-negative

## Constraints

- While `val != 1`: if even, `val = val / 2`; if odd, `val = 3 * val + 1`
- Count iterations
- Inputs in the specified domain take at most 111 steps and reach at most 9,232,
  so every intermediate value is representable in `i64`
- The domain is independent of the verifier unwind limit of 10; the benchmark
  remains Stretch

## Hints

- For example, `n = 3` follows `3 -> 10 -> 5 -> 16 -> 8 -> 4 -> 2 -> 1`
- Within the specified domain, `n = 27` has the longest trajectory: 111 steps
  with a peak value of 9,232
