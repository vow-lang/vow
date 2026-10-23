# ADR-2026-10-23-2253: the declared key type of a map parameter bounds its length

Status: accepted (issue #1853, epic #1398)

Addendum to [ADR-2026-10-08-1422](2026-10-08-1422-native-verification-semantics.md), "Exactness
boundary" (#1678). Native verifier only (no Rust twin,
[ADR-2026-10-08-1421](2026-10-08-1421-verifier-lives-only-in-compiler.md)).

## Context

A `HashMap` key narrower than 64 bits bounds the length of the map by its domain: a `HashMap<u8, V>`
holds at most 256 entries. The native model keeps a presence array over 64-bit keys, so the
presence array and the length of a map the function did not build were unrelated to the domain,
and the driver reported a failure that reaches them only when it held for every state
(`unconstrained map state`). Four fixtures were `unknown` where ESBMC decides: a length bound a
`u8` key proves, a full `i8` table that holds `-1`, and two refutations over a map that may lack a
key.

The IR types a map as a bare pointer, and a key operand does not say the key type: an `i64` literal,
or a constant such as a negative literal's `0 - 1`, is `i64` whatever the key type is.

## Decision

1. **The key type is a fact for a parameter.** `IrFunction.param_keys` already records the declared
   head of every parameter with aliases resolved (`HashMap<u8,i64>`, `compiler/lower_keys.vow`,
   #1823). `compiler/vc_mapdom.vow` reads the key head of the map parameters (`bool`, or an integer up
   to 64 bits) and nothing else: no lowering or IR change, and no Rust twin. A 64-bit key type makes
   the parameter exact like a key operand of 64 bits did.
2. **What a domain of `D` values bounds.** For every arbitrary base of the slot (its first state and
   each state a loop cut forgets), over the keys the function names: a key that is not the widening
   of a value of the key type (`vc_cast` of the slot to the key type and back, the widening
   `vc_vec_elem64` uses) is absent; and, with `A` the number of distinct named keys that are inside
   the domain and absent, `len + A <= D`. Each fact holds of every real map, and, with the count rule
   `present <= len` of `vc_map_count_key`, any model of them extends to a real map of that key type
   (the room left is the unnamed keys of the domain), so such a base is exact and leaves `map_vars`.
   `D` is `2^bits`, 2 for `bool`.
3. **No contract is bounded.** The domain is a property of the declared type, not a clause the
   user writes. A `requires` no real narrow-key map can meet (`m.len() == 300` for `u8`) is
   unsatisfiable, as any contradictory `requires` is, where ESBMC fails it through its 64-entry
   capacity.
4. **Not covered, and still `unknown` where it matters.** A map the function built whose state a
   loop cut forgets (only an operand shows its key width), a parameter whose declared type the
   lowering did not record, and parameters that may be one object (`vc_mapalias`): their named keys
   are spread over several slots of one state, so the facts hold of it and do not make it exact.

## Evidence

`tests/verify-native/pass/map_narrow_param_len_bound.vow`, `map_narrow_key_folded_literal.vow`,
`map_narrow_full_table_has_every_key.vow` and `map_narrow_loop_havoc_len.vow` are proofs;
`fail/map_narrow_param_present_key_wrong.vow`, `map_literal_keys_present_wrong.vow` and the
`fail/map_narrow_*` fixtures are counterexamples `--replay-cex` confirms. Each of five mutations of
the model (no room for an absent key, no domain, a signed key taken for an unsigned one, a domain
one short, a repeated key counted twice) fails at least one of them. `tests/run/map_narrow_key_domain.vow`
pins the runtime facts the model rests on (a map holds all 256 `i8` keys, `-1` among them).
