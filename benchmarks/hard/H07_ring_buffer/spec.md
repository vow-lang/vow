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

- `ring_new`: `requires: capacity > 0, requires: capacity <= 4`, `ensures: result.count == 0, ensures: result.capacity == capacity`
- `ring_write`: `requires: rb.count < rb.capacity, requires: rb.write_pos >= 0, requires: rb.write_pos < rb.capacity`, `ensures: result.count == rb.count + 1`
- `ring_count`: `ensures: result >= 0`

## Constraints

- Write position wraps around using modulo
- This is a Stretch problem — modular arithmetic invariants are hard for BMC

## Hints

- `ring_write` writes at `write_pos`, then `write_pos = (write_pos + 1) % capacity`
- Vec must be pre-filled to `capacity` size for indexed writes
