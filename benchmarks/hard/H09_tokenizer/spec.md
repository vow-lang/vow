# H09: Tokenizer (Stretch)

## Problem

Implement a simple tokenizer that counts delimited segments in a Vec of integers, where 0 acts as a delimiter.

## Signatures

```vow
fn count_tokens(v: Vec<i64>) -> i64
```

## Contracts

- `requires: v.len() >= 0`
- `requires: v.len() <= 8` — bounded for verification
- `ensures: result >= 0` — token count is non-negative
- `ensures: result <= v.len()` — at most as many tokens as elements
- Loop `invariant: count >= 0`
- Loop `invariant: count <= i`
- Loop `invariant: i >= 0`
- Loop `invariant: i <= v.len()`

## Constraints

- Scan the Vec; count transitions from delimiter (0) to non-delimiter
- This is a Stretch problem — tracking transitions is complex for BMC

## Hints

- Track whether previous element was a delimiter with an `in_token` flag
- Increment count when transitioning from `in_token == 0` to non-zero element
