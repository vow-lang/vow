# stdlib/bignum

Arbitrary-precision **signed** integers in base 2³² sign-magnitude form
(`struct BigNum { limbs: Vec<u64>, sign: i64 }`). Pure core language — no builtins
beyond `Vec`/`String`/`u64`/`i64`. Single file: copy `bignum.vow`.

A non-negative `BigNum` is the natural number (`Nat`) an arbitrary-precision `Nat`
consumer needs. The binary limb base makes `and`/`or`/`xor`/`shl`/`shr` trivial
limb-wise operations — the reason this module can back a proof kernel's `Nat` /
`BitVec` reductions past the 2⁶⁴ ceiling (see issue #838).

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

- The representation invariant (non-empty `limbs`, no leading-zero limbs except the
  canonical zero `[0]` with `sign == 1`, `sign ∈ {-1, 1}`, every limb `< 2³²`) is
  maintained internally but **not** stated as a struct invariant or `ensures`.
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
- Functions prefixed for internal use (`bignum_strip_zeros`, `bignum_shift_limbs`,
  `bignum_mul_single`, `bignum_add_small`, `bignum_divmod_small`, `bignum_pow2`,
  `u64_to_decimal*`, `bignum_divmod_long`, …) are not part of the public API.

## Verification

`vow verify stdlib/bignum/main.vow` reports `Skipped`: limb arithmetic allocates
`Vec`s per call (`RegionAlloc`), which the verifier cannot model. The present
contracts (`requires` on division-by-zero, `exp >= 0`, `n >= 0`, shift `n >= 0`, etc.)
are enforced at runtime in `--mode debug`. See
[docs/spec/stdlib.md#verification-status](../../docs/spec/stdlib.md#verification-status).
