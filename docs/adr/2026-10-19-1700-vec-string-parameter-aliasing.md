# ADR-2026-10-19-1700: Vec and String parameters of one type may be one object

Status: accepted (issue #1823, epic #1398)

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
- **Not covered:** `HashMap` and `BTreeMap` parameters (the same wrong verdict), and objects reached
  through a pointer loaded from a container or a field, which the heap model already fails or skips.
  ESBMC keeps its distinct-parameter model and is documented as unsound here until it is removed
  (epic #1398, P6).

## Alternatives rejected

- **Keep the parameters distinct and `Skip` a caller that passes one value twice.** It does not
  change the wrong verdict (the caller is usually `main`, which is not modelled), it rejects a
  correct proof (`pass/inline_vec_arg_aliased.vow`) and it hides a precondition no contract states.
