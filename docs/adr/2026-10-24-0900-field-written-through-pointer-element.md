# ADR-2026-10-24-0900: a field written through a pointer element is a write to the object the slot holds

Status: accepted (issue #1663, epic #1398)

Addendum to [ADR-2026-10-08-1422](2026-10-08-1422-native-verification-semantics.md) ("reading a
pointer element back (address per object)", #1822). Native verifier only (no Rust twin,
[ADR-2026-10-08-1421](2026-10-08-1421-verifier-lives-only-in-compiler.md)).

## Context

`people[0].age = 5` is a `FieldSet` whose receiver is the result of a `Vec` `get`. The IR types that
result `i64`, a handle, so the write was a write through a handle and `Skipped` where ESBMC proves
it. After #1822 the slot of a `Vec` the function built holds the overlay address of the object that
was pushed, so a handle read from it names exactly one overlay entry.

## Decision

1. **The write is the write to each allocated struct entry under `pc` and `h == address(entry)`**
   (`vc_elem_write`, `compiler/vc_elemwrite.vow`), the reach of a write through a pointer Phi. One
   condition holds, so one object is written, and a direct read of the variable that was pushed,
   another slot holding the object and a later read through an element see it (`vc_agg_read_field`).
2. **The gate** (`compiler/vc_elemgate.vow`, `vc_ptr_store_detail`) admits the write only when the
   handle is the `get` of a `Vec` a pointer is stored into, the function is one the address model
   reads exactly (`vc_ptr_store_resolvable`), the value is a `Bool` or an integer of at most 64
   bits, and no parameter is stored into such a `Vec`. Every other write through a handle keeps its
   `Skipped` reason, with a detail of its own for the new refusals.
3. **A parameter stored into the `Vec` is refused.** `f(s, t)` that pushes `s`, writes
   `people[0].age` and reads `t.age` is `5` for `f(p, p)`. The alias analysis (`vc_agg_alias_marked`)
   and the overlay need (`vc_heap_overlay_needed`) key on a write whose receiver is a parameter, so
   the first draft proved `4` (found by review; `skip/vec_pointer_elem_field_param_stored_skipped`).
   Marking both would also model it; left to a follow-up.
4. **A function with such a write is never cut** (`vc_fh_writes_handle`, anywhere in the function).
   The loop cut gives a `Vec` of pointers arbitrary words, and a handle that names no entry would be a
   write the model drops. The bounded rounds are exact, so a symbolic loop is `unknown` (`unwinding
   assertion`), never proven (`unknown/vec_pointer_elem_field_write_in_loop_unknown`).

## Not covered, still `Skipped`

A field written through an element of a `Vec` parameter, a pointer or float written through an
element, an element of an element, a write through a Phi of handles.

## Guards

`pass/vec_pointer_elem_field_*` and `fail/vec_pointer_elem_field_*_wrong` (lost write, wrong
element, visible to the variable, two slots, stale copy, symbolic index, branch, replaced slot,
loop); each counterexample replays at run time.
