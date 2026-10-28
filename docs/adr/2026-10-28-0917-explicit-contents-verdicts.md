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
no marker and list nothing yet, and replay does not rebuild the contents behind an element (stage 3 of #1890). The replay harness is a differential test of the markers: a wrong
`unconstrained` shows as `diverged`.

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
Replay is unchanged: it still skips a `Vec` of collections whose elements carry contents (stage 3 rebuilds
them).
