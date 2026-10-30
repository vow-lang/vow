# ADR-2026-10-28-0917: a listed address says whether its contents matter

Status: proposed (issue #1890, epic #1398); the direction is the user ruling of 2026-10-28 (list container contents, say so when they do not matter), the format and scope await maintainer acceptance

Addendum to [ADR-2026-10-27-0617](2026-10-27-0617-fields-read-through-a-handle.md) (objects the function
reads through memory are listed as addresses with their fields) and
[ADR-2026-10-08-1422](2026-10-08-1422-native-verification-semantics.md). The native verifier and its replay
harness only (no Rust twin, [ADR-2026-10-08-1421](2026-10-08-1421-verifier-lives-only-in-compiler.md)).

## Context

A counterexample lists a `Vec<String>` held in a field, an element of a `Vec<Vec<i64>>` or a returned `Vec`
the function never measures as an address. An address with no contents is useless to an agent: it cannot
tell an object the failure does not depend on from one it does, and replay cannot rebuild the second kind,
so it either skipped or guessed. The ruling is that the listing must give the contents of every container
it holds, and where the contents are not needed it must say so explicitly, never leave them silently empty.
Replay is not weakened: it rebuilds what is listed, treats an explicit "free" marker as free to choose, and
still skips (never diverges, never guesses) anything unlisted.

## Decision

Every position that holds the address of a `Vec`, a `String` or a struct and lists no contents for it gets
`<position>.contents` with one of two tokens:

- `unconstrained`: the failing claim performs no length, element, byte or field read through that address,
  so any contents reproduce the failure.
- `unlisted`: it does, or the verifier cannot tell, and the contents are not listed (also a listed length
  above 512, or an unanswered second query).

The verifier records, in the walk, the class and term of every handle it reads memory through (the four
`vc_handle_*` functions are the only callers of the heap readers). The registry allocates no term and writes
nothing to the header, so a claim's query is unchanged; the terms are defined only in the model projection
that follows a failed claim, behind the elements, so the handles come from the same model as the addresses.
The driver (`vc_cex_verdict`) joins listed positions to reads on class and address and strips the handoff
entries (`$read.n`, `$read.<i>`). No `$read.n` means the reads are unknown, and nothing is called
`unconstrained`. A read set that over-approximates only turns `unconstrained` into `unlisted`.

Replay builds the simplest value for an `unconstrained` position (empty `Vec`/`String`, zero struct, a
`Vec` of such) and skips an `unlisted` one or one with no marker.

## Consequences

Only new keys appear; `values` stays a string map. Positions of type `Option`, enum or `linear struct` carry
no marker and list nothing yet. The replay harness is a differential test of the markers: a wrong
`unconstrained` shows as `diverged`, which surfaces as the `VerifierBug` diagnostic.

## Stage 2: the contents behind a read element

A position the function reads through and that no earlier listing covers (an element of a `Vec<String>` or
`Vec<Vec<_>>`, a struct element of a held `Vec`) gets its contents instead of `unlisted`. After the first
enrichment the verifier asks the claim once more with every value of that model fixed (regular entries, the
elements and the handles), so the contents belong to the same counterexample, and projects `hlen[h]`,
`helem[h ++ i]` or `hbyte[h ++ i]`, or `hf<k>[h]` for each slot the function read at `h`, for one handle per
distinct (kind, address), at most 32. The lengths are bounded by 2, 8, 64 and 512 and shrunk like the first
enrichment, in a query that is outside the layout of the first answer (the closing count of
`vc_nest_extra`). A model with a float, an unanswered query or a length above 512 lists nothing, and the driver
keeps `unlisted`. The handoff entries `$read.<i>.len`, `$read.<i>[<j>]` and `$read.<i>.<slot>` carry raw
slots; the driver (`vc_cex_join`) copies them under every position of that kind, address and declared type,
typed from the declaration, and settles the positions the copies add in the next round (4096 entries at
most). A position at the address of a parameter or of an object of another declared type is not copied to.
Replay is unchanged by this stage: it still skips a `Vec` of collections whose elements carry contents
(stage 3 rebuilds them).

## Stage 3: replay rebuilds what is listed

Replay (`replay_nested`) builds the object behind every address a position holds, from the rows listed behind
it: a `Vec` from `<position>.len` and its elements (scalars from `<position>[<j>]`, objects recursively), a
`String` from its bytes, a struct from `<position>.<slot>`. A position marked `unconstrained` is built as the
simplest value; `unlisted`, no marker and no contents, a marker next to contents, and a length above 512 are
`skipped` with a reason that names the position, never `diverged` and never a guess.

The harness declares one object per (type, address), as a `let` of its own, and every position at that address
names the binding, because Vec, String and struct addresses share one number space in the model and a model
with `q.p` and `people[0]` at one address has one object. Before it builds anything, a position takes the
parameter of its own type whose `.addr` is that address. A parameter of the same class and another type at
the address is a handle the model typed differently than the program: the position is `skipped`, unless it is
`unconstrained`, when nothing reads through it. A parameter of another class at the number is a coincidence
and is ignored. A second position at an address that lists contents must list the same ones as the first,
or the replay is `skipped`.

Not part of this stage: a counterexample whose model needs a type-different alias (`q.p` is the `Node`
parameter `q` where a `Pair` is declared) stays `skipped`; stage 4 asks the solver for a type-consistent model.

## Stage 4: a model that keeps objects of two declared types apart

The heap arrays are keyed by slot, not by type (ADR-2026-10-24-0900, item 6), so a model of a failed claim
may give a `Pair` and a `Node` one address (`q.p` is the address of `q`): a counterexample no run has, which
replay declines (#1909). The first enriched model is checked (`vc_typed`): the declared key of each entry that
holds an object's address (a parameter's `.addr`, a listed slot, a `Vec` element, a slot read through an
element) comes from the program, not the IR instructions: `IrFunction.param_keys` and the per-slot keys of each
struct (`IrModule.field_keys`, filled from the declarations by `vc_annotate_field_keys` after lowering, empty
in a module nothing annotated, never read by codegen). When two entries of one kind and two keys hold an
address the function reads through, the enrichment is asked again with those entries required to differ
(`vc_assert_apart`, by symbol name, so the pairs survive a change of length bound), at most 3 times, and the
last model with no such pair, or the last that the solver found, is listed. The extra assertions only choose
among models of the claim's own query and run only after the claim failed, so no verdict, proof or cost of a
proof changes. When no model keeps the types apart the first stands and replay skips it as before. Not keyed, so
not kept apart: the handles read through (`$read.<i>`) and anything else without a struct, `Vec` or `String`
key, such as an `Option` payload, an enum or a `HashMap`.
