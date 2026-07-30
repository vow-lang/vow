# M15: Collatz Bounded

## Problem

Implement a function that counts Collatz steps for positive inputs.

## Signature

```vow
fn collatz_steps(n: i64) -> i64
```

## Contracts

- `requires: n >= 1` — starting value is positive
- `ensures: result >= 0` — step count is non-negative

## Constraints

- While `val != 1`: if even, `val = val / 2`; if odd, `val = 3 * val + 1`
- Count iterations
- The source contract is independent of verifier unwind limits

## Hints

- For example, `n = 3` follows `3 -> 10 -> 5 -> 16 -> 8 -> 4 -> 2 -> 1`
