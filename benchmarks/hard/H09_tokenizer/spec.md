# H09: Tokenizer (Stretch)

## Problem

Implement a simple tokenizer that counts delimited segments in a Vec of integers, where 0 acts as a delimiter.

## Signatures

```vow
fn count_tokens(v: Vec<i64>) -> u64
```

## Contracts

- `ensures: result <= v.len()` — at most as many tokens as elements; the `u64` return type already makes the count non-negative
- `count` and `i` are `u64` locals, so they need no `>= 0` invariants
- Loop `invariant: count <= i`
- Loop `invariant: i <= v.len()`

## Constraints

- Scan the Vec; count transitions from delimiter (0) to non-delimiter
- This is a Stretch problem — tracking transitions is complex for BMC

## Hints

- Track whether previous element was a delimiter with an `in_token` flag
- Increment count when transitioning from `in_token == 0` to non-zero element
- Verifier unwind and Vec-model limits are not source preconditions
