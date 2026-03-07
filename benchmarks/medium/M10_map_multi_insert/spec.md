# M10: Map Multi Insert

## Problem

Implement a function `map_fill` that inserts `n` distinct key-value pairs into a HashMap.

## Signature

```vow
fn map_fill(n: i64) -> HashMap<i64, i64>
```

## Contracts

- `requires: n >= 0` — count is non-negative
- `requires: n <= 4` — bounded (HashMap capacity is limited)
- `ensures: result.len() == n` — map has exactly `n` entries
- Loop `invariant: i >= 0`
- Loop `invariant: i <= n`

## Constraints

- Use keys `0, 1, 2, ...` to ensure distinct keys
- Insert in a loop

## Hints

- `m.insert(i, i * 10)` with `i` as key guarantees distinct keys
- Distinct keys means each insert increases length by 1
