# M08: Map Insert Lookup

## Problem

Implement a function `store_entry` that creates a HashMap with a single key-value pair.

## Signature

```vow
fn store_entry(k: i64, v: i64) -> HashMap<i64, i64>
```

## Contracts

- `ensures: result.contains_key(k)` — the key is present
- `ensures: result.len() == 1` — exactly one entry

## Constraints

- Create with `HashMap::new()`, insert one entry
- The function is pure

## Hints

- `HashMap::new()` creates an empty map
- `m.insert(k, v)` adds a key-value pair
- After one insert, `len() == 1` and `contains_key(k)` hold
