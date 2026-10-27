# ADR-2026-10-27-0347: a handle stored into a `Vec` that also holds objects keeps the chain open

Status: accepted (issue #1663, epic #1398)

Addendum to [ADR-2026-10-08-1422](2026-10-08-1422-native-verification-semantics.md) ("reading a
pointer element back", #1822) and [ADR-2026-10-27-0300](2026-10-27-0300-element-of-element.md).
Native verifier only (no Rust twin, [ADR-2026-10-08-1421](2026-10-08-1421-verifier-lives-only-in-compiler.md)).

## Context

`dst.push(w); dst.push(src[0]); dst[1].len()` was `Skipped` (`a pointer element is read back`). The
IR types the result of a `get` `i64`, a handle, so pushing `src[0]` is no pointer store: a `Vec` that
receives only handles is an ordinary `Vec` of words and already verified. A `Vec` that also receives an
object (`held`, `vc_ptr_store_held`) holds two kinds of word, the overlay address of an object the
function stored and a word of the memory it was given, and `vc_ptr_store_resolvable` refused any stored
value that was not a pointer with a class, because the model assumed the first kind only.

## Decision

1. **A direct `get` result may be stored into a held `Vec`** (`push` or `set`; `vc_ptr_handle_gets`,
   `compiler/vc_ptrhandle.vow`, checked in `vc_ptr_store_resolvable`). Any other `i64` stored (a field
   read, a Phi, a call result) keeps its `Skipped`. The word is stored as it is (`vc_walk_slot_word`
   already stores a non-pointer as its term), so a read of the slot is that term by array theory, and a
   read through it is the `ite` chain over the entries of the overlay at the state of the walk, ending in
   the heap array. Both kinds of word are read exactly: an address hits its entry, a handle of given
   memory hits none and reads the array.
2. **The chain keeps its array arm.** `VcOverlay.exact` (an exact chain ends in its first entry, because
   every element is a stored address) is off when `vc_ptr_store_handles` holds. With it on,
   `dst[1].len()` above read the length of `w` and `ensures result == 0` was proven for a given `Vec`
   that has an element; the first draft of the fixtures caught it (mutation, below).
3. **No element of such an element is a stored word.** `vc_elem_derived` returns all zero, the rule it
   already has for a stored `split` result: `dst[1][0]` is the element of a `Vec` of the given memory,
   so it stays a foreign `get` and, used as an object, `Skipped`.
4. **A field written through an element of such a `Vec` stays refused** (`vc_elem_write_detail`:
   `a field is written through an element of a Vec that holds a handle`). The write of `vc_elem_write`
   reaches the entries and, in an exact walk, nothing else; a handle of given memory would lose it. The
   heap-array store the non-exact walk adds is scoped to a 64-bit slot of a `Vec` parameter
   (ADR-2026-10-24-0900, item 5), not to this case. A `push` through the slot (`dst[1].push(9)`) is a
   write through a handle and was and stays `Skipped`.

## Not covered, and the cost

- **Tags.** The stored handle is not noted as a term of the given memory, so the model allows a state in
  which it equals the address of an object the function built. Every real run has the handle unequal to
  every such address, so this only adds counterexamples, never proofs: a claim that depends on one built
  object being mutated while the handle is read may report a counterexample no run has. Noting it
  would be unsound whenever the `get` can return an address the function stored (a `Vec` read back out of
  a field it was stored into), which the #1822 addendum rejects; noting only the `get`s of a `Vec`
  parameter needs the function to store no handle into one, a separate decision.
- **Contracts and bodies that use a foreign element as an object** (`src[0].len()`) are `Skipped`
  as before, so a claim about the stored handle's own contents cannot be stated yet.
- A `Vec` parameter that also receives a pointer (a quantified tag fact), copies of a `Vec` that holds
  pointers (`vec_sort` and `pin_to_root` reject a `Vec` of pointers in the type checker, so the refusal
  has no source program), and the other slices of #1663.

## Guards

`pass/vec_pointer_elem_handle_stored_{mixed,set_slot,symbolic}` and their `fail/..._wrong` twins plus
`fail/..._foreign_slot_wrong` (each counterexample was checked by running `main` in debug mode; replay does
not rebuild a `Vec<Vec<i64>>`, #1890). `skip/vec_pointer_elem_handle_stored_{push_through,nested,field_write}_skipped`.
`compiler/tests/test_vc_elemgate.vow` (`check_handles`). Mutations, each rebuilt: `exact` on makes
`foreign_slot_wrong`, `set_slot_wrong` and `symbolic_wrong` `Verified`; the derived rule off turns
`nested_skipped` into `VerifyFailed`.
ESBMC skips every fixture (a `Vec` of pointers, `non-scalar element`), so the differential report has no row.
