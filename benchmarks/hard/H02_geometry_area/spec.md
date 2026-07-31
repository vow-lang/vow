# H02: Geometry Area

## Problem

Implement geometric area and perimeter functions with exact overflow guards.

## Signatures

```vow
struct Point { x: i64, y: i64 }
fn point_new(x: i64, y: i64) -> Point
fn circle_area(r: i64) -> i64
fn rect_area(w: i64, h: i64) -> i64
fn rect_perimeter(w: i64, h: i64) -> i64
fn point_distance_sq(a: Point, b: Point) -> i64
```

## Contracts

- `circle_area`: `requires: r >= 0, r <= 1753413056`, `ensures: result >= 0`
- `rect_area`: `requires: w >= 0, h >= 0, h == 0 || w <= 9223372036854775807 / h`, `ensures: result >= 0`
- `rect_perimeter`: `requires: w >= 0, h >= 0, w <= 4611686018427387903 - h`, `ensures: result >= 0`

## Constraints

- Preconditions describe the complete non-negative domains where each result fits in `i64`
- Multiple interacting functions sharing the Point struct

## Hints

- `circle_area` approximates as `r * r * 3` (integer pi approximation)
- `rect_area` is `w * h`
- `rect_perimeter` is `2 * (w + h)`
