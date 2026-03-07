# H02: Geometry Area

## Problem

Implement geometric area and perimeter functions with overflow-safe bounds.

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

- `circle_area`: `requires: r >= 0, r <= 1000000`, `ensures: result >= 0`
- `rect_area`: `requires: w >= 0, h >= 0, w <= 1000000, h <= 1000000`, `ensures: result >= 0`
- `rect_perimeter`: `requires: w >= 0, h >= 0, w <= 1000000, h <= 1000000`, `ensures: result >= 0`

## Constraints

- Bounds on inputs prevent overflow in multiplication
- Multiple interacting functions sharing the Point struct

## Hints

- `circle_area` approximates as `r * r * 3` (integer pi approximation)
- `rect_area` is `w * h`
- `rect_perimeter` is `2 * (w + h)`
