# M09: Map Update

## Problem

Implement a function `map_update` that inserts a key-value pair and then overwrites it with a new value.

## Signature

```vow
fn map_update(k: i64, v1: i64, v2: i64) -> HashMap<i64, i64>
```

## Contracts

- `ensures: result.contains_key(k)` — the key is present after update
- `ensures: result.len() == 1` — still exactly one entry (update, not insert)

## Constraints

- Insert then overwrite the same key
- The function is pure

## Hints

- First `m.insert(k, v1)`, then `m.insert(k, v2)` overwrites
- After overwrite, `len()` is still 1
