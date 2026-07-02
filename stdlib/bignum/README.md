# stdlib/bignum

Arbitrary-precision **signed** integers with a small-int fast path
(`enum BigNum { Small(i64), Big(BigMag) }`). `Small(v)` holds any value fitting in
`i64` with **no heap allocation**; `Big(m)` holds a `BigMag` magnitude (base 2³²
limbs, `Vec<u64>`, sign-magnitude) for `|value| > i64::MAX`. Pure core language — no
builtins beyond `Vec`/`String`/`u64`/`i64`. Single file: copy `bignum.vow`.

A non-negative `BigNum` is the natural number (`Nat`) an arbitrary-precision `Nat`
consumer needs. The binary limb base makes `and`/`or`/`xor`/`shl`/`shr` trivial
limb-wise operations — the reason this module can back a proof kernel's `Nat` /
`BitVec` reductions past the 2⁶⁴ ceiling (see issue #838). The fast path measured
~3–5× faster and ~40× less peak memory on small-op-heavy loops.

**Invariant:** a value fits `i64` ⟺ it is `Small`. Every op returns through
`bignum_normalize` (demotes `Big`→`Small` when it fits), so `cmp`/`eq`/`to_string`
never see two encodings of one value.

Public API (selected):
- Construct: `bignum_zero`, `bignum_from_i64`, `bignum_from_u64`, `bignum_from_string`
- Convert: `bignum_to_string`, `bignum_to_u64` (`Option<u64>`; `None` if negative or > u64)
- Predicates: `bignum_is_zero`, `bignum_is_negative`, `bignum_is_positive`
- Compare: `bignum_cmp`, `bignum_cmp_abs`, `bignum_eq`, `bignum_lt`, `bignum_gt`, `bignum_le`, `bignum_ge`
- Arithmetic: `bignum_negate`, `bignum_abs`, `bignum_add`, `bignum_sub`, `bignum_monus`, `bignum_mul`, `bignum_div`, `bignum_mod`, `bignum_divmod`
- Bitwise (on magnitude): `bignum_and`, `bignum_or`, `bignum_xor`, `bignum_shl`, `bignum_shr`
- Higher-level: `bignum_pow(base, exp: i64)`, `bignum_gcd`, `bignum_factorial(n: i64)`

Full details: [docs/spec/stdlib.md#bignum](../../docs/spec/stdlib.md#bignum).

## Usage

```
ulimit -v 2000000; build/vowc build --no-verify stdlib/bignum/main.vow -o /tmp/bignum_demo && /tmp/bignum_demo
```

## Gotchas

- The `BigMag` magnitude invariant (non-empty `limbs`, no leading-zero limbs except
  the canonical zero `[0]` with `sign == 1`, `sign ∈ {-1, 1}`, every limb `< 2³²`) is
  maintained internally but **not** stated as a struct invariant or `ensures`.
- The `Small`/`Small` fast path uses conservative bounds (`2⁶²−1` for add/sub/monus,
  `2³¹` for mul) so it never overflows `i64`; out-of-bound operands fall to the limb
  path. Results are identical to the all-`Big` representation.
- Division truncates toward zero; the remainder's sign matches the dividend.
- `bignum_monus` is truncated (Nat) subtraction — `max(a − b, 0)`, saturating at 0.
- Bitwise `and`/`or`/`xor` operate on the **magnitude** (Nat semantics) and return a
  non-negative result; the sign is ignored.
- `bignum_shl` / `bignum_shr` shift the magnitude (`requires: n >= 0`) and preserve the
  sign — i.e. multiply / floor-divide by 2ⁿ. For non-negative operands this is a
  logical bit shift.
- `bignum_pow` / `bignum_factorial` take a native `i64` exponent/argument, not a BigNum.
- `bignum_gcd` works on absolute values; the result is non-negative.
- Multiplication is O(n·m) schoolbook (no Karatsuba).
- The limb algorithms are internal `bigmag_*` functions over the `BigMag` magnitude
  (`bigmag_add`, `bigmag_mul`, `bigmag_divmod`, `bigmag_strip_zeros`, …); the public
  `bignum_*` API wraps them with the fast path + `bignum_normalize`. Those, plus
  `bignum_to_bigmag` / `bignum_normalize` / `u64_to_decimal*`, are not public API.

## Verification

`vow verify stdlib/bignum/main.vow` reports `Skipped`: limb arithmetic allocates
`Vec`s per call (`RegionAlloc`), which the verifier cannot model. The present
contracts (`requires` on division-by-zero, `exp >= 0`, `n >= 0`, shift `n >= 0`, etc.)
are enforced at runtime in `--mode debug`. See
[docs/spec/stdlib.md#verification-status](../../docs/spec/stdlib.md#verification-status).
