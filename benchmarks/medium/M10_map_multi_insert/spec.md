# M10: Map Multi Insert

## Problem

Implement a function `map_fill` that inserts `n` distinct key-value pairs into a HashMap.

## Signature

```vow
fn map_fill(n: u64) -> HashMap<i64, i64>
```

## Contracts

- `n` is a `u64` count, so the type already says it is non-negative; no `requires: n >= 0` is needed
- `ensures: result.len() == n` — map has exactly `n` entries
- The loop counter `i` is a `u64` local, so it needs no `invariant: i >= 0`; convert with `i as i64` for the key
- Loop `invariant: i <= n`

## Constraints

- Use keys `0, 1, 2, ...` to ensure distinct keys
- Insert in a loop

## Hints

- `m.insert(k, k * 10)` with `let k: i64 = i as i64` as key guarantees distinct keys
- Distinct keys means each insert increases length by 1
- Verifier unwind and HashMap-model limits are not source preconditions
