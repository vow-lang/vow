# M10: Map Multi Insert

## Problem

Implement a function `map_fill` that inserts `n` distinct key-value pairs into a HashMap.

## Signature

```vow
fn map_fill(n: i64) -> HashMap<i64, i64>
```

## Contracts

- `requires: n >= 0` — count is non-negative
- `ensures: result.len() as i64 == n` — map has exactly `n` entries
- The loop counter `i` is a `u64` local, so it needs no `invariant: i >= 0`; bridge with `n as u64` and `i as i64`
- Loop `invariant: i <= n as u64`

## Constraints

- Use keys `0, 1, 2, ...` to ensure distinct keys
- Insert in a loop

## Hints

- `m.insert(k, k * 10)` with `let k: i64 = i as i64` as key guarantees distinct keys
- Distinct keys means each insert increases length by 1
- Verifier unwind and HashMap-model limits are not source preconditions
