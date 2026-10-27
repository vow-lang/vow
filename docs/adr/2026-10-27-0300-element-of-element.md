# ADR-2026-10-27-0300: an element of an element is a stored word when every `Vec` on the way is one the function built

Status: accepted (issue #1663, epic #1398)

Addendum to [ADR-2026-10-08-1422](2026-10-08-1422-native-verification-semantics.md) ("reading a
pointer element back", #1822) and [ADR-2026-10-24-0900](2026-10-24-0900-field-written-through-pointer-element.md).
Native verifier only (no Rust twin, [ADR-2026-10-08-1421](2026-10-08-1421-verifier-lives-only-in-compiler.md)).

## Context

`outer[i][j].len()` was `Skipped` (`a pointer element is read back`). The first `get` is the element of a
`Vec` the function built; the second has that handle for receiver. `vc_ptr_store_resolvable` marked
every `get` of a `Vec` no pointer is stored into as foreign (it may name given memory, which the model
cannot tell from the address of an object the function stored) and refused a foreign result used as an
object. The handle of the first `get` is a stored word, so the second is too, but the gate only knew the
first level.

## Decision

1. **A derived value** (`vc_elem_derived`, `compiler/vc_elemgate.vow`) is a `get` whose receiver is a
   receiver of a pointer store (`held`) or derived, and a Phi all of whose edges are. A receiver of a
   pointer store is a `Vec::new` of the function (`vc_ptr_store_resolvable`), and every pointer stored
   into it is a known object with a class, so its elements are exactly the words the function stored.
   By induction so are the elements of a derived value, whatever the depth. A Phi with an edge that is
   none of these (an element of a `Vec` parameter, a pointer loaded from memory) is not derived, and a
   `get` of it is foreign as before.
2. **The model needs no change.** A read through a derived handle was already the `ite` chain of
   `vc_overlay_elem` over the `Vec` entries at the state of the walk, ending in the heap array, which
   is only reached for a handle that is no entry's address; the handle is, by array theory, one of the
   addresses the function stored. Nothing is noted as loaded for it, so no tag assumption can make a
   path infeasible. The gate was the only obstacle. The chain keeps the heap-array arm, so a derived
   read is not an "exact" handle (`vc_overlay_note_elem`): the cubic case split of an exact chain is
   not saved, which costs time and never soundness.
3. **A `split` result stored into a `Vec` switches the rule off** (`vc_elem_derived` returns all zero).
   Its pieces are free addresses the function did not store; the first level keeps its behaviour and
   the second stays `Skipped`.
4. **A field written through a derived handle** is the write of `vc_elem_write`: each struct entry
   under `h == address(entry)`. `vc_elem_write_detail` admits a handle whose `get` receiver is held
   or derived. The loop cut, the alias analysis and the overlay need are keyed on the `FieldSet`
   through a handle, so they apply unchanged.

## Not covered

- The direct source form `grid[i][j].age = v` and `grid[i][j].id`: both compilers lower the field access
  without the struct type of the element (#1898), drop the write and read slot 0. The verifier models
  the IR it is given, which is what the binary runs. The bound form `let row = grid[i]; row[j].age = v`
  is lowered correctly and is what the fixtures use.
- An element of an element of a `Vec` parameter or of a `Vec` read out of a field, a pointer pushed
  through an element (`outer[i].push(x)`), a pointer written through one, the other slices of #1663.

## Guards

`pass/vec_pointer_elem_nested_*` and `fail/vec_pointer_elem_nested_*_wrong` (a leaf length, a later push to
the leaf, the wrong leaf, a symbolic index, a field written through the nested element and seen by the
variable that was pushed, the wrong cell). Each counterexample replays at run time.
`compiler/tests/test_vc_elemgate.vow` (`check_nested`) pins the derived set: the built case, an element
of a `Vec` parameter and a stored `split` result.
