# ADR-2026-11-09-1200: a `Vec` parameter that receives a pointer tells given elements from stored ones by the index

Status: accepted (issue #1905, slice B of #1663, epic #1398)

Addendum to [ADR-2026-10-30-0900](2026-10-30-0900-collection-payloads-and-stored-collection-params.md)
("Not covered") and [ADR-2026-10-27-0347](2026-10-27-0347-handle-stored-in-a-vec.md). Native verifier
only (no Rust twin, [ADR-2026-10-08-1421](2026-10-08-1421-verifier-lives-only-in-compiler.md)).

## Context

`outer.push(w); outer[i].len()` with `outer: Vec<Vec<i64>>` a parameter was `Skipped` (`a pointer element
is read back`). A `Vec` the function built holds only addresses it stored, and every `get` of a parameter
is memory the caller gave. A parameter that receives a pointer holds both kinds, and the gate
(`vc_ptr_store_resolvable`) refused it because the model could not tell them apart without a fact
quantified over the elements.

## Decision

1. **The index tells them apart, with no quantifier.** Each `get` of such a parameter with index `i`
   assumes `i < len0 -> tag(arr0[i]) == 0`, where `arr0` and `len0` are the array and length the parameter
   entered with (`base_arr` and `base_len` of its slot) and `tag` is the address tag array of #1805
   (`vc_overlay_note_given`). The fact is true of every run, and a later `push`, `set`, `pop` or `pop`
   followed by `push` cannot falsify it: it speaks of the array of the entry, which never changes, and
   the array theory decides whether the current read starts from it. A slot written since reads the
   stored word, which has no such fact.
2. **The parameter is marked 2 in `VcOverlay.tagged`** (`vc_ptr_param_held`, `compiler/vc_ptrparam.vow`),
   not 1. A 1 notes every `get` result as given memory, which prunes the objects the function allocated
   from the chain of a read through it; that is wrong for a slot the function stored. A held parameter is
   excluded from the parameters a stored handle may come from (`vc_ptr_handle_tagged`), so
   `outer.push(outer[0])` stays refused. `VcOverlay.exact` is off, the chain ends in the heap array.
3. **Gate.** `vc_ptr_store_resolvable` admits a held `GetArg` of a `Vec` parameter. It keeps `Skipped`
   for a parameter that a Phi merges (its state is then a Phi slot, not the array of the entry) and for a
   function that also stores a handle (`vc_ptr_handle_status` other than none).
4. **Counterexamples** list the identities and addresses as for any aliased parameter. An element the
   solver names as the address of a parameter of another declared type (the pointee type is not recorded,
   ADR-2026-10-23-0400) is reported `skipped` by `--replay-cex`, never as a false proof.

## Consequences

- `skip/vec_pointer_elem_param_vec_read_back` moves to `pass/`; the other six fixtures are new. The
  `fail/` twins replay `confirmed`.
- Mutations, each rebuilt: no tag fact makes `pass/..._given_slot_not_built` a `VerifyFailed`; marking the
  parameter 1 (given memory) turns `fail/..._stored_built_slot_wrong` into `Verified`, a false proof.
- Known imprecision, not a soundness gap: in a function that pushes into the parameter inside a loop, the
  solver may take a given element for the parameter itself (a type the pointee model does not
  separate), so a true claim about an element is `VerifyFailed` with a counterexample `--replay-cex`
  reports as `skipped`.

## Not covered

- Loops that havoc the parameter keep no link to the array of the entry, so a claim about an element
  after the loop may be refuted by a state no run has (see above).
- A field written through an element of such a parameter, and a handle stored into it, stay refused.
