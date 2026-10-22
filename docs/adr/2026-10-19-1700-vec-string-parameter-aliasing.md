# ADR-2026-10-19-1700: Vec, String and map parameters of one type may be one object

Status: accepted (issues #1823 and #1858, epic #1398)

Addendum to [ADR-2026-10-08-1422](2026-10-08-1422-native-verification-semantics.md) and to
the struct rule of #1810. Native verifier only (no Rust twin,
[ADR-2026-10-08-1421](2026-10-08-1421-verifier-lives-only-in-compiler.md)).

## Context

The native model took two `Vec` (or two `String`) parameters for two objects. The language does not
say so: `t(a, a)` type-checks and runs, and only a `linear struct` cannot be passed twice. A function
that pushed through one parameter and read the length of the other was `PROVEN` while a caller that
passed one value violated its `ensures` at run time (`--replay-cex` confirms it). ESBMC shares the
assumption (its C model declares each parameter as a local), so `verify_diff.py` cannot see the gap.

## Decision

1. **May-alias, like structs.** The parameters of one declared type (`Vec<i64>`, `Vec<String>` and
   `String` are three types) that a function changes, directly or through a Phi, and that have a
   second member in their type group, each get a free identity, listed as `<param>.alias` in a
   counterexample. Two are one object exactly when their identities are equal: the initial contents
   are then equal, and every write through one (`push`, `set`, `pop`, `clear`, `truncate`, the string
   operations, a loop cut's havoc, a write through a Phi) is applied to the other under the identity
   test. Every `ensures`, `invariant` and abort claim holds for every identity assignment.
   `compiler/vc_vecalias.vow`; the write hook is `vc_vec_spread` in `compiler/vc_vec.vow`.
2. **Untouched:** a group the function only reads, a group of one, parameters of different declared
   types, and block-entry merges (every slot already carries the writes).
3. **The type group is the declared type with aliases resolved, element included.**
   `IrFunction.param_keys` carries `Vec<i64>`, not `Vec` (`compiler/lower_keys.vow`). Found by
   review: keyed by the unresolved text, `Vec<Id>` and `Vec<i64>` under `type Id = i64` were two
   groups and the proof of their one-object case was lost. A parameter whose type has no known head
   puts every parameter in one group, as before.
4. **Maps follow the same rule (#1858).** `HashMap` and `BTreeMap` parameters are grouped by their
   declared type with aliases resolved, key and value included (`HashMap<i64,i64>`,
   `BTreeMap<i64,i64>` and `HashMap<u8,i64>` are three types; `param_keys` carries the heads of
   both arguments). A map parameter that the function changes (`insert`, `remove`, a loop cut's
   havoc) and that has a second member in its group gets an identity, equal identities equate the
   presence array, value array and length of the two, and every write through one is applied to the
   other under the identity test. A map is never merged by a Phi or passed to a call (the map gate),
   so its uses are all direct. `compiler/vc_mapalias.vow`; the write hook is `vc_map_spread` in
   `compiler/vc_map.vow`. The members of a group share one key type, so what any of them shows of
   its key width (the `unconstrained map state` rule of ADR-2026-10-08-1422) holds for the others.
   `--replay-cex` builds the one map with the listed keys of every member
   (`compiler/replay_mapkeys.vow`).

## Consequences

- A function that proves only for different arguments (`append(dst, src)` with
  `ensures: dst.len() == old_len + src.len()`) is now `VerifyFailed` with equal identities. A
  source contract cannot say "distinct" (there is no identity predicate), so the contract has to
  hold either way or the signature changes. The verifier adds no hidden precondition.
- A callee standing in as a trusted summary (ADR-2026-10-16-1800) is trusted only on a proof that
  already covers the one-object case.
- The identity machinery runs only for a function that changes a collection and has two parameters
  of one type; no other query changes. Over 606 native fixtures, 168 ESBMC-corpus files and the 11
  compiler roots the only verdict that moved is the function row of
  `trust/alias_two_vec_params_failed.vow` (`tail`: `PROVEN` to `FAILED`).
- Maps (#1858): no function of the 2156 `.vow` files of the repository (the fixtures, the ESBMC corpus,
  the benchmarks, the examples and the compiler) has two map parameters of one declared type besides
  the new fixtures, so no existing verdict moves; the model of a function without such a pair is
  untouched, and the existing map fixtures verify in the same time.
- **Not covered:** objects reached through a pointer loaded from a container or a field, which the
  heap model already fails or skips.
  ESBMC keeps its distinct-parameter model and is documented as unsound here until it is removed
  (epic #1398, P6).

## Alternatives rejected

- **Keep the parameters distinct and `Skip` a caller that passes one value twice.** It does not
  change the wrong verdict (the caller is usually `main`, which is not modelled), it rejects a
  correct proof (`pass/inline_vec_arg_aliased.vow`) and it hides a precondition no contract states.
