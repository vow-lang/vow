# H09: Tokenizer (Stretch)

## Problem

Implement a simple tokenizer that counts delimited segments in a Vec of integers, where 0 acts as a delimiter.

## Signatures

```vow
fn count_tokens(v: Vec<i64>) -> i64
```

## Contracts

- `ensures: result >= 0` — token count is non-negative
- `ensures: result <= v.len() as i64` — at most as many tokens as elements
- `count` and `i` are `u64` locals, so they need no `>= 0` invariants; convert with `count as i64`
- Loop `invariant: count <= i`
- Loop `invariant: i <= v.len()`

## Constraints

- Scan the Vec; count transitions from delimiter (0) to non-delimiter
- This is a Stretch problem — tracking transitions is complex for BMC

## Hints

- Track whether previous element was a delimiter with an `in_token` flag
- Increment count when transitioning from `in_token == 0` to non-zero element
- Verifier unwind and Vec-model limits are not source preconditions
