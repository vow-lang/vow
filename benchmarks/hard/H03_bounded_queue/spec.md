# H03: Bounded Queue

## Problem

Implement a bounded queue with enqueue, size tracking, and full-check.

## Signatures

```vow
struct Queue { data: Vec<i64>, size: i64, capacity: i64 }
fn queue_new(capacity: i64) -> Queue
fn queue_enqueue(q: Queue, val: i64) -> Queue
fn queue_size(q: Queue) -> i64
fn queue_is_full(q: Queue) -> i64
```

## Contracts

- `queue_new`: `requires: capacity > 0, requires: capacity <= 6`
- `queue_enqueue`: `requires: q.size >= 0, requires: q.size < q.capacity`
- `queue_size`: `requires: q.size >= 0`, `ensures: result >= 0`
- `queue_is_full`: `ensures: result >= 0, ensures: result <= 1`

## Constraints

- Queue tracks size and capacity
- Enqueue requires space available
- Multiple functions maintain shared invariant

## Hints

- `queue_new` creates Queue with empty Vec, size 0, given capacity
- `queue_enqueue` pushes to data, increments size, preserves capacity
- `queue_size` returns `q.size`
- `queue_is_full` checks `q.size == q.capacity`
