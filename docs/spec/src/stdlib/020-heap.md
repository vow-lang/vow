
## heap

`stdlib/heap/min_heap.vow` and `max_heap.vow` are structural mirrors (a min-heap and
a max-heap over `i64`), with the comparator flipped. Both are value types: every
mutator takes a heap by value and returns a new one.

The defining contract pattern is the **size-shadow invariant** `size == data.len()`
(`size` and `.len()` are both `u64`), threaded through every mutator. This is what
lets ESBMC reason about in-bounds `data[i]` access without a universal quantifier:
```vow
pub fn min_heap_push(h: MinHeap, val: i64) -> MinHeap vow {
    requires: h.size == h.data.len(),
    requires: h.size < 18446744073709551615,
    ensures: result.size == h.size + 1,
    ensures: result.size == result.data.len()
}
```

| Function (min; `max_*` mirrors) | Signature | Key contracts |
|---------------------------------|-----------|---------------|
| `min_heap_new` | `() -> MinHeap` | `ensures result.size == 0, result.data.len() == 0` |
| `min_heap_len` | `(h) -> u64` | `ensures result == h.size` |
| `min_heap_is_empty` | `(h) -> bool` | `ensures result == (h.size == 0)` |
| `min_heap_push` | `(h, val: i64) -> MinHeap` | size-shadow in/out; `ensures result.size == h.size + 1` |
| `min_heap_peek` | `(h) -> i64` | `requires h.size > 0, size-shadow`; `ensures result == h.data[0]` |
| `min_heap_pop` | `(h) -> MinHeap` | `requires h.size > 0, size-shadow`; `ensures result.size == h.size - 1` |
| `min_heap_clear` | `(h) -> MinHeap` | size-shadow in; `ensures result.size == 0` |
| `is_min_heap` | `(h) -> bool` | `requires size-shadow` — runtime check of the heap-order property |

**Heap-order is a runtime predicate, by design.** Vow has no universal quantifier, so
the property `∀i. data[parent(i)] <= data[i]` cannot be written as an `ensures`.
`is_min_heap` / `is_max_heap` check it at runtime instead; the static contracts cover
index safety and the size-shadow invariant only.

---

## stack

`stdlib/stack/stack.vow` — a `Vec<i64>`-backed LIFO stack (value type). `node.vow` in
the same directory is a vestigial `Node` struct kept for the demo; the stack does not
use it.

| Function | Signature | Key contracts |
|----------|-----------|---------------|
| `stack_new` | `() -> Stack` | — |
| `stack_push` | `(s, val: i64) -> Stack` | `ensures result.size == s.size + 1` |
| `stack_peek` | `(s) -> i64` | `requires s.size > 0` |
| `stack_size` | `(s) -> i64` | — |
| `stack_is_empty` | `(s) -> bool` | — |

**Known gaps (move-verbatim; tracked follow-up):** no `stack_pop`; no size-shadow
invariant (`size == data.len() as i64`) like `heap` has; `stack_peek` has no `ensures`
relating the result to `data[size-1]`; functions are not marked `pub`; `node.vow` is
unused.

---

## geometry

`stdlib/geometry/point.vow` (a `Point` struct) and `shape.vow` (a `Shape` enum with
circle/rectangle area and perimeter). **The only module whose `vow verify` passes
today** — its shape functions use exact derived overflow bounds. Note this means the
*vowed* checks verify (`vow verify stdlib/geometry/main.vow` → `Verified`); it is not a
proof of the whole API, since `point_distance_sq` carries no contract (see Known gaps).

| Function | Signature | Key contracts |
|----------|-----------|---------------|
| `point_new` / `point_x` / `point_y` | `Point` accessors | — |
| `point_distance_sq` | `(a, b: Point) -> i64` | — (no overflow guard — gap for large coordinates) |
| `circle_area` | `(r: i64) -> i64` | `requires 0 <= r <= 1753413056`; `ensures result >= 0` |
| `rect_area` | `(w, h: i64) -> i64` | `requires w >= 0, h >= 0, h == 0 \|\| w <= I64_MAX / h`; `ensures result >= 0` |
| `circle_perimeter` | `(r: i64) -> i64` | `requires 0 <= r <= 1537228672809129301`; `ensures result >= 0` |
| `rect_perimeter` | `(w, h: i64) -> i64` | `requires w >= 0, h >= 0`; `ensures result >= 0` (uses `+!`/`*!`, so overflow aborts rather than needing an operand bound) |

Each magic bound is the exact threshold below which the arithmetic cannot overflow —
e.g. `circle_area` caps `r` at `floor(sqrt(I64_MAX/3))` because it computes `r*r*3`:
```vow
fn circle_area(r: i64) -> i64 vow {
    requires: r >= 0,
    requires: r <= 1753413056,
    ensures: result >= 0
}
```

**Known gaps:** the `Shape` enum is declared but the area/perimeter functions are
free functions that don't dispatch on it; `point_distance_sq` lacks an overflow
guard; `shape_at` is a demo artifact, not a real API.

---

## bignum

`stdlib/bignum/bignum.vow` — arbitrary-precision **signed** integers with a
small-int fast path (`enum BigNum { Small(i64), Big(BigMag) }`). `Small(v)` holds
any value fitting in `i64` with **no heap allocation**; `Big(m)` holds a `BigMag`
magnitude (base 2³² limbs, `Vec<u64>`, sign-magnitude) for `|value| > i64::MAX`.
Pure core language; no builtins beyond `Vec`/`String`/`u64`/`i64`. A non-negative
`BigNum` is the natural number (`Nat`) an arbitrary-precision `Nat` consumer needs;
the binary limb base makes the bitwise operations trivial limb-wise ops, which is
why this module can back a proof kernel's `Nat` / `BitVec` reductions past the 2⁶⁴
ceiling (issue #838). The fast path measured ~3–5× faster and ~40× less peak
memory on small-op-heavy loops vs. the always-allocating representation.

**Public API (selected):**
- Construct: `bignum_zero`, `bignum_from_i64`, `bignum_from_u64`, `bignum_from_string`
- Convert: `bignum_to_string`, `bignum_to_u64` (`Option<u64>`; `None` if negative or > u64)
- Predicates: `bignum_is_zero`, `bignum_is_negative`, `bignum_is_positive`
- Compare: `bignum_cmp`, `bignum_cmp_abs`, `bignum_eq`, `bignum_lt`, `bignum_gt`, `bignum_le`, `bignum_ge`
- Arithmetic: `bignum_negate`, `bignum_abs`, `bignum_add`, `bignum_sub`, `bignum_monus`, `bignum_mul`, `bignum_div`, `bignum_mod`, `bignum_divmod`
- Bitwise (on magnitude): `bignum_and`, `bignum_or`, `bignum_xor`, `bignum_shl`, `bignum_shr`
- Higher-level: `bignum_pow(base, exp: u64)`, `bignum_gcd`, `bignum_factorial(n: u64)`

**Contracts present:** `bignum_div`/`bignum_mod`/`bignum_divmod` require
`!bignum_is_zero(b)`; the `bignum_pow` exponent, the `bignum_shl`/`bignum_shr` shift
count and the `bignum_factorial` argument are `u64`, so non-negativity is carried by
the type (internal `bigmag_sub_abs` requires `bigmag_cmp_abs(a, b) >= 0`).

**Semantics to know:**
- **Canonicalization invariant:** a value fits `i64` ⟺ it is `Small`. Every
  result-producing op returns through `bignum_normalize`, which demotes a `Big`
  magnitude back to `Small` when it fits — so `cmp`/`eq`/`to_string` never see two
  encodings of one value. The `BigMag` magnitude keeps the usual limb invariant
  (non-empty, no leading-zero limbs except canonical zero `[0]`, `sign ∈ {-1, 1}`,
  each limb `< 2³²`); none of this is stated as a struct invariant or `ensures`.
- The `Small`/`Small` fast path uses conservative magnitude bounds (`2⁶²−1` for
  `add`/`sub`/`monus`, `2³¹` for `mul`) so it never overflows `i64`; values outside
  the bound fall to the limb path and re-normalize. Result correctness is identical
  to the all-`Big` representation.
- Division truncates toward zero; the remainder's sign matches the dividend.
- `bignum_monus` is truncated (Nat) subtraction — `max(a − b, 0)`, saturating at 0.
- `bignum_to_u64` returns `Option::None` when the value is negative or exceeds `u64`.
- Bitwise `and`/`or`/`xor` act on the **magnitude** (Nat semantics) and return a
  non-negative result; `shl`/`shr` shift the magnitude and preserve the sign
  (= multiply / floor-divide by 2ⁿ; a logical bit shift for non-negative operands).
- `bignum_pow`/`bignum_factorial` take a native `u64` exponent/argument, not a BigNum.
- `bignum_gcd` operates on absolute values; the result is non-negative.
- Multiplication is O(n·m) schoolbook (no Karatsuba).
- The limb algorithms live in internal `bigmag_*` functions over the `BigMag`
  magnitude (`bigmag_add`, `bigmag_mul`, `bigmag_divmod`, `bigmag_strip_zeros`,
  `bigmag_to_string`, …); the public `bignum_*` API wraps them with the fast path
  and `bignum_normalize`. `bignum_to_bigmag`/`bignum_normalize`/`u64_to_decimal*`
  and the `bigmag_*` set are internal, not part of the public API.

**Verification:** `Skipped` — limb arithmetic allocates `Vec`s per call (`RegionAlloc`),
which the verifier cannot model. Contracts are runtime-enforced in `--mode debug`.

---

## gc

`stdlib/gc/gc.vow` — a mark-and-sweep garbage collector over a heap of `i64` values
with explicit roots and reference edges (`struct GcHeap`). Slots are opaque integer
handles returned by `gc_alloc`; never fabricate them.

| Function | Signature | Key contracts |
|----------|-----------|---------------|
| `gc_new` | `() -> GcHeap` | — |
| `gc_alloc` | `(h, val: i64) -> i64` | — (returns a slot; reuses freed slots) |
| `gc_add_root` | `(h, slot: i64)` | `requires slot >= 0, slot < values.len() as i64, alive[slot] == 1` |
| `gc_remove_root` | `(h, slot: i64)` | `requires slot >= 0, slot < values.len() as i64` (does **not** require alive — you may unroot a freed slot) |
| `gc_add_ref` | `(h, from, to: i64)` | `requires` both in range and alive |
| `gc_read` | `(h, slot: i64) -> i64` | `requires slot >= 0, slot < values.len() as i64, alive[slot] == 1` |
| `gc_write` | `(h, slot, val: i64)` | `requires slot >= 0, slot < values.len() as i64, alive[slot] == 1` |
| `gc_is_alive` | `(h, slot: i64) -> bool` | `requires slot >= 0, slot < values.len() as i64` |
| `gc_count` | `(h) -> i64` | — |
| `gc_collect` | `(h) -> i64` | — (returns count of newly-freed objects) |

**Semantics to know:**
- `gc_collect` invalidates every slot not reachable from a root; calling
  `gc_read`/`gc_write` on a freed slot violates its precondition.
- Roots and references are not deduplicated — adding a root twice needs two
  `gc_remove_root` calls.
- The heap stores only `i64`; represent richer object graphs as indices/tagged ints.
- Mark/sweep handles cycles naturally via the mark bit; no separate cycle detection.

**Verification:** `VerifyFailed` — ESBMC produces a `gc_add_root` precondition
counterexample tied to how in-module caller-`requires` are checked (cf. issue #764).
Contracts are runtime-enforced in `--mode debug`.

---

## Known gaps and roadmap

These are tracked follow-ups, intentionally **not** addressed by the reorg that
created `stdlib/` (which moved code verbatim):

- **Static verifiability.** Make `Vec`/region-allocating functions modelable so
  `stack`, `bignum`, and most of `heap` can be statically verified; resolve the
  `gc_add_root` caller-`requires` counterexample (#764).
- **Contract hardening.** Add struct/representation invariants and `ensures` clauses
  to `bignum` and `gc`; add a size-shadow invariant and `stack_pop` to `stack`; add
  an overflow guard to `point_distance_sq` and to `pow` (so its `ensures result >= 0`
  holds under `i64`); wire the `Shape` enum into `geometry`'s area/perimeter functions.
- **Consistency.** Mark all intended-public functions `pub` (currently only `math`
  and `heap` do); remove or rebuild the vestigial `stack/node.vow`.
- **Distribution.** A module search path so stdlib modules can be imported without
  copying source into the consuming project.
