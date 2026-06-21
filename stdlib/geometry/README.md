# stdlib/geometry

2D integer geometry. `point.vow` defines a `Point` struct; `shape.vow` defines a
`Shape` enum with area/perimeter for circles and rectangles. `shape.vow` internally
does `use point`, so **copy both files together** to consume this module.

This is the **only stdlib module whose `vow verify` passes today** — its shape
functions use exact derived overflow bounds. That proves the *vowed* checks reachable
from the demo, not the whole API: `point_distance_sq` carries no contract (see Gotchas
below), so a `Verified` result does not prove all of this module's arithmetic.

Public API:
- `point_new`, `point_x`, `point_y`, `point_distance_sq`
- `circle_area`, `rect_area`, `circle_perimeter`, `rect_perimeter`

Full signatures and contracts:
[docs/spec/stdlib.md#geometry](../../docs/spec/stdlib.md#geometry).

## Usage

```
ulimit -v 2000000; build/vowc build stdlib/geometry/main.vow -o /tmp/geo_demo && /tmp/geo_demo
```

## Key idea: exact overflow bounds

Each shape function caps its inputs at the precise threshold below which the
arithmetic cannot overflow — e.g. `circle_area` requires `r <= 1753413056`
(= `floor(sqrt(i64::MAX / 3))`) because it computes `r * r * 3`. This is the pattern
to copy when you need a statically-provable numeric routine.

## Gotchas / known gaps

- The `Shape` enum is declared but the area/perimeter functions are free functions
  that don't dispatch on it.
- `point_distance_sq` has no overflow guard (a gap for large coordinates).
- `shape_at` is a demo artifact, not a real API.

## Verification

`vow verify stdlib/geometry/main.vow` → `Verified`. See
[docs/spec/stdlib.md#verification-status](../../docs/spec/stdlib.md#verification-status).
