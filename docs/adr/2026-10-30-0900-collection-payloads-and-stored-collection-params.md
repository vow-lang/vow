# ADR-2026-10-30-0900: a `Vec` or `String` payload is a handle, and a stored `Vec` or `String` parameter has its group's address

Status: accepted (issues #1906 and #1905, slices E and B of #1663, epic #1398)

Addendum to [ADR-2026-10-23-0400](2026-10-23-0400-option-payloads-read-as-handles.md) (payloads
read as handles), [ADR-2026-10-19-1700](2026-10-19-1700-vec-string-parameter-aliasing.md)
(parameters of one type may be one object) and the "reading a pointer element back" addendum of
[ADR-2026-10-08-1422](2026-10-08-1422-native-verification-semantics.md). Native verifier only (no
Rust twin, [ADR-2026-10-08-1421](2026-10-08-1421-verifier-lives-only-in-compiler.md)).

## Context

Two shapes were `Skipped` (`unsupported-opcode`, fail closed) where ESBMC proves or cannot decide:

1. A payload that is a collection (`match o { Some(v) => v.len() ... }` of an `Option<Vec<i64>>`).
   The payload read was left a child object, and a `Vec` operation names its object by parameter,
   so the read was refused (`a pointer read out of a field is used as a collection`).
2. A `Vec` or `String` parameter stored into a `Vec` or a field and read back. The gate refused a
   stored collection parameter, because the model took two parameters of one type for distinct
   objects and a push through the other would not show in the stored one.

Both are the same question: which collection a pointer names. The model of a handle already answers
it (`ite(h == address(o), state of o, ...)` over the overlay entries, ADR-2026-10-08-1422), and a
parameter's state is one per alias group (ADR-2026-10-19-1700).

## Decision

1. **A collection payload is read as a handle.** `vc_payload` retypes the payload reads of a function
   the child model rejects, and now also accepts a read used as the object operand of a `Vec` or
   `String` operation (`vc_agg_collection_operand`; the child gate, `vc_agg_child_detail`, still
   refuses it). `len`, `get` and the other reads are then reads through the handle; the payload is
   the parameter whose address it is, an object the function stored, or memory the caller gave. A
   payload stored as an element, a write through a payload and a payload read from memory read
   through a handle keep their `Skipped`.
2. **A stored `Vec` or `String` parameter joins its type group.** `vc_vecalias_stored_classes`
   gives a parameter that a `push`, `set` or field write stores (through Phis) its declared class in
   the alias analysis, so a function that stores one and changes another of its declared type gets
   the identities of ADR-2026-10-19-1700 even when the stored parameter is never an operand. The two
   are one object exactly when their identities are equal, and a write through one is applied to the
   other under that test, so the element read back, a read of the state of the parameter whose
   address the slot holds, sees it. `vc_ptr_store_resolvable` no longer refuses a collection parameter
   (the `collection` table of `vc_objects` is gone).
3. **Counterexamples** list the identities and addresses as for any aliased parameter; `--replay-cex`
   rebuilds both parameters as one value and confirms them.

## Consequences

- `skip/option_vec_payload` moves to `pass/`; the three stored-parameter rows of #1905 move to `fail/`
  (the correct verdict: the push through `b` shows in the element when `a` and `b` are one value). The
  `native-gap` directive of `collection_param_in_field` goes with it; ESBMC cannot check that row
  (#1624).
- `skip/option_string_payload` is now `VerifyFailed` for the abort it has (`unwrap` of `None`), as an
  `Option<i64>` is (`fail/option_string_payload_unwrap_none`), with a `pass/` twin.
- **A false proof is gone.** `fn f(h: Holder, a: Vec<i64>, b: Vec<i64>)` that stores `a` in `h.v`,
  pushes to `b` and compares the length read through `h.v` was `PROVEN` on `origin/main` (the model took
  `a` for a parameter no other could be) and is refuted by a run with one value passed twice
  (`fail/ptr_field_vec_param_stored_wrong`).
- Cost: the identity machinery now also runs for a function that only stores a collection parameter
  next to another of its type; nothing else changes.

## Not covered

- A `Vec` parameter that is the receiver of a pointer store (`p.push(a)` with `p: Vec<Vec<_>>`): its
  given elements cannot be told from stored addresses without a quantified tag fact
  (`skip/vec_pointer_elem_param_vec_read_back_skipped`).
- A payload's pointee type is not recorded (ADR-2026-10-23-0400, item 4), so a `Vec<u8>` parameter may
  be the payload of an `Option<Vec<i64>>` in the model; a counterexample that needs it is reported
  `skipped` by `--replay-cex`, never a false proof.

## Guards

`pass/option_vec_payload{,_elem,_built_fresh}`, `pass/result_vec_payload`, `pass/option_string_payload`,
`fail/option_vec_payload_{elem,param_alias,built_param}_wrong`,
`pass/vec_pointer_elem_collection_param_{other_type,pushed_through,second_slot}` and
`fail/vec_pointer_elem_{collection,string}_param_*_wrong`. Each counterexample replays. The `only_stored`
rows are the ones the identity change alone catches: with the gate relaxed and no
`vc_vecalias_stored_classes`, they are `PROVEN` and a run refutes them.
