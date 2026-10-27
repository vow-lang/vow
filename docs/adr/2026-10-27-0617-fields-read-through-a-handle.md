# ADR-2026-10-27-0617: the fields an object is read through a handle for are listed in the counterexample

Status: accepted (issue #1890, epic #1398)

Addendum to [ADR-2026-10-08-1422](2026-10-08-1422-native-verification-semantics.md) (memory read through a
handle, `vc_heap`) and [ADR-2026-10-23-0400](2026-10-23-0400-option-payloads-read-as-handles.md). Native
verifier and replay harness only (no Rust twin, [ADR-2026-10-08-1421](2026-10-08-1421-verifier-lives-only-in-compiler.md)).

## Context

A struct, an `Option` payload or an element of a `Vec<S>` that the function reads through a handle is the
address `o.0` in a counterexample: the heap is an unconstrained array per slot (`hf<k>[h]`), and the
model's value of an array entry was never asked for. `--replay-cex` therefore could not rebuild the object
and reported `skipped`: building it from zeros would run the function on a value the solver never chose and
could turn a sound counterexample into a `VerifierBug`.

## Decision

1. **Each read through a handle defines its slot** (`vc_heaplist_note`, `compiler/vc_heaplist.vow`): a
   `define-fun` of `hf<k>[h]`, taken at the array the function was given (a write through an element
   changes the array the walk reads, never this one), typed as the slot is read, and a model row
   `<handle>.<k>` for each field symbol `h` is made of (a merge of fields reads either). The element of a
   `Vec` parameter is a handle too: its rows are `<param>[<i>].<k>`, defined with the elements in the
   enrichment of the model (`compiler/vc_fieldrows.vow`). A collection such an object holds is a held
   collection (`vc_held_note`, `o.0.0.len`, `o.0.0[<i>]`).
2. **The listing is all or nothing.** A read whose handle is not made of field symbols and `Vec` elements
   (the value of another read, a scalar, a stored address) sets `VcHeapListed.bad`, and the function lists no
   row. An address with rows is described completely, an address without them is not described: replay needs
   that to tell a slot the function never read (zero) from one the listing lost.
3. **Replay rebuilds an object whose address has rows** (`replay_through_memory`,
   `compiler/replay_listed.vow`). Objects with one address are one object, which the harness can build only by
   sharing a binding: it does for the elements of one `Vec` and declines otherwise. An object whose address
   is a parameter's `.addr` is that parameter, as before.

## Consequences

The counterexample format only grows: new `values` keys, no key changes. Eight `replay-skipped` fixtures
now replay `confirmed`. Still skipped: an element or field that is another object's alias without a read of
its own (`vec_pointer_elem_field_foreign_heap_wrong`), a handle that is the value of a read
(`..._handle_value_wrong`) and a `Vec<Vec<_>>` parameter (`..._handle_in_field_wrong`).
