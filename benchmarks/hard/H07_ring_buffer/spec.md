# H07: Ring Buffer (Stretch)

## Problem

Implement a circular buffer with modular arithmetic for write position tracking.

## Signatures

```vow
struct RingBuf { data: Vec<i64>, write_pos: i64, count: i64, capacity: i64 }
fn ring_new(capacity: i64) -> RingBuf
fn ring_write(rb: RingBuf, val: i64) -> RingBuf
fn ring_count(rb: RingBuf) -> i64
```

## Contracts

- `ring_new`: `requires: capacity > 0`, `ensures: result.count == 0, ensures: result.capacity == capacity, ensures: result.data.len() == capacity`
- `ring_write`: requires a non-negative count, available capacity, a valid write position, and `rb.data.len() == rb.capacity`; ensures count increments and capacity/data length are preserved
- `ring_count`: `requires: rb.count >= 0`, `ensures: result >= 0`

## Constraints

- Write position wraps around using modulo
- This is a Stretch problem — modular arithmetic invariants are hard for BMC

## Hints

- `ring_write` writes at `write_pos`, then `write_pos = (write_pos + 1) % capacity`
- Vec must be pre-filled to `capacity` size for indexed writes
- Verifier unwind and Vec-model limits are not source preconditions
