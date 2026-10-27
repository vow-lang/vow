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

1. **A `get` of a `Vec` parameter may be stored into a held `Vec`** (`push` or `set`;
   `vc_ptr_handle_status`, `compiler/vc_ptrhandle.vow`, checked in `vc_ptr_store_resolvable`). The word is
   stored as it is (`vc_walk_slot_word` already stores a non-pointer as its term), so a read of the slot is
   that term by array theory, and a read through it is the `ite` chain over the entries of the overlay at
   the state of the walk, ending in the heap array. The model must tell the two kinds of word apart, so the
   rule has two conditions, each with its own `Skipped`: every handle stored is the `get` of a `Vec`
   parameter, whose elements are the memory the function was given, and no value that may be an address
   (a `get` or a field read, closed over Phis) is stored into a `Vec` that is not held, so no parameter's
   elements can be a word the function wrote. Any other `i64` stored (a field read, a Phi, a `get` of a
   `Vec` the function built or of a field) keeps its `Skipped`.
2. **Such a `get` is noted as read from the given memory** (`vc_overlay_note_elem`, `VcOverlay.tagged`): its
   term is assumed different from the address of every object the function built (`vc_overlay_note_loaded`,
   the tags of #1805), and may still be a parameter's. Without it the solver chose a handle equal to the
   address of a built object, which no run has, and `n2 - n1 == 0` after a `push` to that object was a
   `VerifyFailed` for a correct function (found by review; `pass/..._not_a_built_object`). The #1822 addendum
   rejected noting the result of every `get`, which is unsound once a `get` can return a stored address;
   condition 1 is what excludes that, so the note is made only for the `Vec` parameters it names.
3. **The chain keeps its array arm.** `VcOverlay.exact` (an exact chain ends in its first entry, because
   every element is a stored address) is off for such a function. With it on, a symbolic `dst[i]` read the
   length of a stored object where the slot holds a given handle, and four `fail/` fixtures were proven.
4. **No element of such an element is a stored word.** `vc_elem_derived` returns all zero, the rule it
   already has for a stored `split` result: `dst[1][0]` is the element of a `Vec` of the given memory, so
   it stays a foreign `get` and, used as an object, `Skipped`.
5. **A field written through an element of such a `Vec` stays refused** (`vc_elem_write_detail`: `a field
   is written through an element of a Vec that holds a handle`). The write of `vc_elem_write` reaches the
   entries and, in an exact walk, nothing else; a handle of given memory would lose it. A `push` through the
   slot (`dst[1].push(9)`) is a write through a handle and was and stays `Skipped`. The consequences apply to
   the whole function, not to the one `Vec` that received the handle: a function that also writes through the
   elements of another held `Vec` is `Skipped`, as it was before this change.

## Not covered

- **Contracts and bodies that use a foreign element as an object** (`src[0].len()`) are `Skipped` as
  before, so a claim about the stored handle's own contents cannot be stated yet; the model supports the
  read, the gate does not.
- A handle copied into a `Vec` that holds no object (`out.push(src[0])`) in the same function, a handle
  read out of a `Vec` the function built or out of a field, a `Vec` parameter that also receives a pointer
  (a quantified tag fact), copies of a `Vec` that holds pointers (`vec_sort` and `pin_to_root` reject a
  `Vec` of pointers in the type checker, so the refusal has no source program), and the other slices of
  #1663.

## Guards

`pass/vec_pointer_elem_handle_stored_{mixed,set_slot,symbolic,not_a_built_object,struct_field,param_stored}`,
their `fail/..._wrong` twins (`foreign_slot`, `mixed`, `set_slot`, `symbolic`, `param_other`; each
counterexample was checked by running `main` in debug mode, replay does not rebuild a `Vec<Vec<i64>>`,
#1890), `unknown/..._loop_unknown` and `skip/..._{push_through,nested,field_write,copied,built}_skipped`.
`compiler/tests/test_vc_elemgate.vow` (`check_handles`). Mutations, each rebuilt: `exact` on makes
`foreign_slot_wrong`, `set_slot_wrong`, `symbolic_wrong`, `param_other_wrong` `Verified` and
`not_a_built_object` fail; no note makes `not_a_built_object` `VerifyFailed`; the derived rule off turns
`nested_skipped` into `VerifyFailed`. ESBMC skips every fixture (a `Vec` of pointers, `non-scalar element`),
so the differential report has no row.
