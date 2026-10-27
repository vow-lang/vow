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
3. **A parameter stored into the `Vec` is the caller's object, and the write reaches every parameter
   that may be it.** `f(s, t)` that pushes `s`, writes `people[0].age` and reads `t.age` is `5` for
   `f(p, p)`. The first draft proved `4` (found by review): the alias analysis
   (`vc_agg_alias_marked`) and the overlay need (`vc_heap_overlay_needed`) keyed on a write whose
   receiver is a parameter. Now a `FieldSet` through an element of a `Vec` counts as a write of a
   parameter in both, `vc_elem_write` writes the parameter entries of the overlay too, and
   `vc_agg_write` reaches the other parameters of the same type group under the identity test
   (`pass/vec_pointer_elem_field_param_stored_*`, `fail/..._param_stored_wrong`, `..._not_aliased_wrong`,
   `..._phi_wrong`, `..._heap_wrong`). A `Vec` or `String` parameter stored into a `Vec` stays refused
   (`vc_ptr_store_resolvable`, #1823). Mutations: ignoring the alias marking is caught by
   `fail/..._param_stored_wrong`, writing no parameter entry by 6 fixtures, writing every entry
   whatever the handle by 9. Marking `vc_heap_overlay_needed` is **not** observable for a stored
   parameter: its entry is added when it is stored, before any write the walk reaches, and an
   unstored parameter that is the same object is the stored one, so a handle read through it names
   the stored entry. The marking is kept because it is what a write through an element the function
   did not store needs (a parameter may be that element).
4. **A function with such a write is never cut** (`vc_fh_writes_handle`, anywhere in the function).
   The loop cut gives a `Vec` of pointers arbitrary words, and a handle that names no entry drops the
   write. That only widens the states, since every real handle names an entry, and a cut counterexample
   is `unknown`; the function is still left to the bounded rounds, which are exact, so a symbolic
   loop is `unknown` (`unwinding assertion`), never proven
   (`unknown/vec_pointer_elem_field_write_in_loop_unknown`, `..._write_after_pushing_loop_unknown`).
   Defence in depth: the mutation that removes the refusal changes no fixture, because the cut
   already refuses a write whose receiver is no parameter or allocation (`vc_fh_roots`) and the
   spurious branch above is `unknown`.

5. **An element of a `Vec` parameter is a write to the heap** (`vc_elem_foreign_ok`,
   `vc_heap_store`). The handle names memory the function was given. The write is the write to
   every struct entry of the overlay under `h == address(entry)` (a parameter that is that object),
   **and** a store to the heap array of its slot: `hf<k> := ite(pc, store(hf<k>, h, v), hf<k>)`,
   the one place the arrays are written, so a read through any handle that may equal `h` sees it
   and a read made before it does not. A read consults the entries first and the array only for a
   handle that is no entry's address, so the two cannot disagree. The overlay is needed for a
   handle write whether or not the function reads through a handle (`p.age` is read directly, and
   `p` may be the element), so `vc_heap_overlay_needed` answers true before it asks for a read.
   Gate: the `Vec` is a `Vec` parameter (not one read out of a field), the value is a 64-bit
   integer (the slot the array holds; a narrower or `Bool` write would have to merge into the slot),
   the function stores no pointer anywhere (an object it built has an address of its own),
   reads no payload object (`FieldGet[ptr]`: a child of a parameter is a symbol that ignores the
   write) and the write is not in a loop. A loop is refused for cost: the unrolled copies store to
   the array one after another and neither the memory nor the time bound holds (a bounded run
   timed out after 5 minutes); the loop cut cannot help (it would give the array arbitrary
   contents), so such a function stays `Skipped` as before rather than becoming a timeout.
   A pointer read out of given memory is typed `i64` and is admitted as a value: it is a 64-bit word
   in the model and at run time, stored as one and read back as the same word (`pass/..._foreign_handle_value`,
   `fail/..._foreign_handle_value_wrong`). A pointer-typed object value is not (`pointer_value`).
   The counterexample of a function with such a write is replayed when every element of the `Vec`
   is a struct parameter (`..._param_wrong`, `..._callee_wrong`, `..._two_params_wrong`); an element
   that names no parameter has no listed fields and is not rebuilt (#1890). Each other
   `fail/vec_pointer_elem_field_foreign_*` was checked by running its `main` in debug mode, which
   aborts on the violation the model names.

   Adversarial cases pinned by fixtures: two elements one object (`..._two_elements_wrong`), a
   parameter that is the element (`..._param_wrong`, `..._two_params_wrong`), an object reached
   through another parameter's field (`..._heap_wrong`), a callee's write seen by the caller
   (`..._callee_wrong`), a read before the write (`..._read_before_wrong`), a conditional write,
   an untouched sibling field, and the payload of an `Option` parameter that is a stored parameter
   (`..._param_stored_payload_wrong`; `vc_payload` reads it as a handle because the write counts
   as a write of a parameter). Mutations of the model, each rebuilt and run over
   `vec_pointer_elem_field`: no array store (5 fixtures fail), no overlay for a write without a
   read (3 proofs of a false claim), an unconditional store (2). The refusals are pinned by
   `compiler/tests/test_vc_elemgate.vow` (the payload, stored-pointer, `Vec`-parameter, 64-bit
   and loop refusals, and the overlay rule; each mutation fails it), not by `skip/` fixtures: ESBMC
   decides those programs, so each would be a `weaker` row of the differential report and need a
   `native-gap` line. A function that stores a pointer is refused twice over: a `Vec` the function
   pushes into is also refused by `vc_ptr_store_resolvable`, and a pointer stored into a field
   makes the refusal observable only through a spurious counterexample.

6. **The heap arrays are keyed by slot, not by type** (found by review). Every handle that is not an
   entry's address reads `hf<k>[h]` whatever object it names, so a write to slot 0 through an element
   of a `Vec<Person>` is seen by a read of slot 0 of an unrelated `Config` or `Pair`: a counterexample
   that equates two handles of different types is not a run. Two refusals and one rule keep it out of
   the common shapes. A struct parameter declared as another type than the element's is not written
   (`vc_heap_elem_head`, `param_keys`: different heads are never one object,
   `pass/..._foreign_other_type`; before, `cfg.limit` was refuted by `people[0] == cfg`). A function
   with two `Vec` parameters of different element types is refused (`vc_elem_mixed_vecs`, unit-tested).
   What remains is a handle loaded out of a field, whose type the IR does not record: if it is read at
   a slot written through an element of another type, the model can equate them and report a
   counterexample that no run has. That is a false `fail`, never a false `proven` (the extra states
   only add counterexamples), and it needs two struct types reached by handle in one function.

## Not covered, still `Skipped`

A field written through an element of a `Vec` read out of a field, or of a `Vec` parameter in a
function that stores a pointer, reads a payload object or loops; a pointer, float, `Bool` or
narrower integer written through the element of a `Vec` parameter; an element of an element of a `Vec`
the function did not build (a built one is ADR-2026-10-27-0300); a
write through a Phi of handles; a pointer written through any element
(`skip/..._pointer_value_skipped`, tracked in #1663: the C emitter cannot store a `Vec` in a field
either, ESBMC rejects the emitted C with a parse error, as it does for
`skip/..._collection_param_in_field_skipped`).

## Guards

`pass/vec_pointer_elem_field_*` and `fail/vec_pointer_elem_field_*_wrong` (lost write, wrong
element, visible to the variable, two slots, stale copy, symbolic index, branch, replaced slot,
loop); each counterexample replays at run time.
