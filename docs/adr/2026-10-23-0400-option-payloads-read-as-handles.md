# ADR-2026-10-23-0400: an Option payload the child model cannot serve is read as a handle

Status: accepted (issue #1851, epic #1398)

Addendum to [ADR-2026-10-08-1422](2026-10-08-1422-native-verification-semantics.md) (the child
objects of `vc_agg`, `vc_ptrfield`, memory read through a handle). Native verifier only (no Rust
twin, [ADR-2026-10-08-1421](2026-10-08-1421-verifier-lives-only-in-compiler.md)).

## Context

The payload of an `Option`, `Result` or enum that is an object (`FieldGet[ptr]`) is a child of its
parameter: one object per parent and slot, whose own fields are symbols. That is exact while the
function only reads parameters, and it has no answer for three shapes, which were `Skipped`
(`unsupported-opcode`, fail closed) where ESBMC proves the claim:

1. the `Option` is built in the function, so its payload is a field of an allocation;
2. the payload is read from a Phi of two parameters, so there is no single object to take it from;
3. a parameter is written beside the read, and the payload may be the very object it names.

The IR types a pointer loaded out of a struct field `i64`: a handle, an address whose object is a
function of it (`vc_heap`), with the parameters and the objects the function stored laid over it
(`vc_overlay`). That model already decides all three shapes for a struct that holds a struct (a
`Hold { h: Handle }` in place of `Option<Handle>` was `PROVEN`, and refuted by an aliased write
with `o.1` equal to `h.addr` before this change); the payload was typed `ptr` only by the lowering.

## Decision

1. **Read the payload as the handle it is.** For a function the child model rejects, for its base
   or for the aliasing of its parameters, `vc_retype_payload_reads` (`compiler/vc_payload.vow`)
   retypes every `FieldGet[ptr]` to an `i64` read of the same slot, marked in `IrInst.ds`, and the
   Phis that merge only handles become handles (`vc_retype_handle_phis`, moved to
   `compiler/vc_handle_phis.vow`). It runs in `vc_prepare_for_gate`, so the pre-inline check, the
   flattened function, the executor and the callee decisions see one rewrite. A function the child
   model accepts is not touched: its model, counterexamples and cost are as before.
2. **Applies only when it is exact.** Every pointer read has a pointer base (a parameter, an
   allocation, a merge, a call result or another payload), never a handle, and every use of a read
   is one the aggregate model understands (`vc_agg_ptr_reads_used_ok`). A payload that is a value, a
   write through a payload and a payload read from memory read through a handle stay `Skipped`. A
   `Vec` or `String` payload (shape 4, `option_vec_payload`) is read as a handle too
   (ADR-2026-10-30-0900).
3. **No field-address ranking in such a function.** The cycle exclusion of #1779 ranks the value a
   by-value struct-typed field holds below the object holding it, because no program can build a
   value of a self-containing struct. A payload can hold the object that holds it
   (`p.next = Option::Some(p)` needs only a `p` whose `next` is `None`), so a function that reads a
   payload as a handle ranks no edge (`VcHeap.ranked`).
4. **Types are not known.** A payload's pointee type is not recorded, so a parameter of another
   type may be the payload in the model, as for a struct field read through a handle: a
   counterexample can name an alias `--replay-cex` reports as `skipped` (different types).

## Consequences

`tests/verify-native/skip/option_payload_{built_in_function,of_merged_param,param_write}.vow` move
to `pass/`; `option_vec_payload.vow` moved to `pass/` with #1906. The guards fail on a model
that reads the value at the store, takes one arm of a merge, swaps the edges or keeps the payload
apart from the parameters (`fail/option_payload_*_wrong.vow`).
