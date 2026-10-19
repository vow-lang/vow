# ADR-2026-10-19-1514: Reading a pointer element back costs one case per stored object

Status: accepted (issue #1825, epic #1398)

Addendum to [ADR-2026-10-08-1422](2026-10-08-1422-native-verification-semantics.md) ("reading a
pointer element back (address per object)", #1822) and to the address entries of #1756 and #1805.

## Context

#1822 reads `outer[i].len()` through the overlay: the slot holds the address of the object, and a read
through the handle `h` is `ite(h == address(o), state of o, ..., array[h])`. The addresses were free
64-bit symbols kept apart by one tag fact each, and the chain ended in the free heap array. Two shapes
were far too slow for what they decide:

- `n` constant rows pushed into a `Vec<Vec<i64>>` and read with a symbolic row: 8 rows 60 ms, 16 rows
  210 ms, 32 rows 1.9 s, 64 rows 8.4 s (cubic).
- rows built in a loop with a symbolic trip count and read back in a second loop: `Skipped` before #1822,
  `TIMEOUT` at the 300 s budget after it (a `Vec<Person>` the same, more than 120 s; 60 strings pushed in
  a loop and one read with a symbolic index, 3 s to 7 s).

Profiling the queries of the bounded rounds (`bitwuzla` behind a wrapper that keeps every `.smt2`) shows
where the time goes: the 64-row query of the second shape is 267 KB and takes 41 s; with the last arm of
every chain, `array[h]`, replaced by the first entry it takes 0.02 s. The solver was spending its time
proving that a handle read out of a slot is not an address no object has, once per read and per round.

## Decision

1. **The addresses of the allocated objects share one free symbol.** `vc_overlay_alloc_addr` gives an
   allocation `hbase + 16 * entry`, `hbase` one free 64-bit symbol (`hbase` in the query), where each
   address was a symbol of its own. The tag array of #1805 is unchanged. Two addresses now differ by a
   constant the solver folds, so `h == address(o)` on a known `h` is decided without a case split. The
   symbolic loop of #1825 ends in well under a second with this and the next point together; either
   alone leaves a part of the cost (see Numbers).
2. **A handle that is the result of a `get` of a stored-into `Vec` has an exact chain.** In a function
   admitted by `vc_ptr_store_resolvable` every `Vec` a pointer is stored into (a receiver of a pointer
   store, `held`) is a `Vec::new` of the function and holds nothing but pointers. A `get` is a claim
   (index in range) assumed not to fail afterwards, so the handle it returns is the address of an entry:
   the chain of its length, element or field read ends in its first entry instead of the heap array
   (`vc_overlay_arm`). `vc_overlay_note_elem` records the term of a `get` only when its receiver is a
   held `Vec`, and the chain is exact for a recorded term (`vc_overlay_is_elem`). A chain with no entry
   (every object born where the walk has no state for it) keeps the heap array.
3. **Not exact when the array of a held `Vec` is arbitrary.** A loop cut that havocs a held `Vec`
   (`__vc_havoc_vec` on a receiver of a pointer store) makes its elements arbitrary words: two reads
   would both be the one entry the loop body made, and `outer[0].len() == outer[1].len()` could be proven
   where the rows differ. `VcOverlay.exact` is false for such a function (`vc_overlay_vec_havoc`), which
   keeps the free array of #1822. A havoc of any other `Vec` does not matter. The k-induction step case
   writes no `Vec` in its loop, so its arrays hold what was stored before it and stay exact.
4. **The code lives in `vc_handle`.** The three chain builders and the handle reads moved out of
   `vc_overlay.vow` (over the 20 000-byte budget) into `compiler/vc_handle.vow`, and `vc_ptr_store_held`
   (the receivers of a pointer store) from `vc_ptrstore.vow` into `vc_overlay.vow`, which both need.

## Why this is sound

- `hbase + 16 * entry` is a model of the free addresses of #1756: any assignment of distinct addresses to
  the entries that keeps them off the parameters' and the loaded handles' is one the solver may pick
  by choosing `hbase`, and the tags forbid exactly those collisions. A scalar the function loads may be
  any value, 4096 included, and still be no address: `hbase` dodges it. **A literal address is not
  sound**: a loaded `i64` is assumed to have tag 0 (the IR types a pointer and an `i64` alike, so every
  loaded term is), which with the address 4096 would exclude the scalar 4096, and a function that
  stores an object and says `s.n != 4096` (or `requires: c.page == 4096` and `ensures: result == 7`)
  was `Verified`. A first head of this branch had literal addresses; review found it, and
  `fail/vec_pointer_scalar_*_wrong.vow` pin it.
- The exact chain assumes `h` is the address of some entry with state in the block of the read. `h` is
  the result of a `get` of a held `Vec` (a `get` of any other `Vec`, a parameter's or one of scalars, is
  not recorded, even if its handle is stored in a field and read back: `fail/vec_pointer_elem_foreign_
  handle_in_field_wrong.vow`). An element in range of a `Vec` the function built is a word a `push` or
  `set` stored, every store enters the stored object (`vc_overlay_addr_of`), and an object has state in
  every block its creation reaches, so the entry exists and has a state wherever the slot can be read.
  An element out of range is an abort the same walk reports; its claim is decided before the read and
  later claims assume it away.
- A handle loaded from the given memory never takes an exact chain: it names a parameter, not an
  entry (`vc_overlay_chain_start`).

## What stays as it was

- The set of functions the address model admits (`vc_ptr_store_resolvable`) and what stays `Skipped`.
- A symbolic loop whose claim no invariant can state stays `unknown` (`unwinding assertion`): the bounded
  rounds cannot close it, as for the same program over `Vec<i64>`. It is now `unknown` in well under a
  second instead of `TIMEOUT` after 300 s. The count of obligations does not change.

## Numbers

Query-level, from the dumps of the 64-round bounded schedule of the symbolic loop (`bitwuzla` 0.9.1):
free addresses and `array[h]` ending every chain 41 s for the largest query; the same with literal
addresses 29 s (no exact chain: the cost is the case split); free addresses with the exact chain 0.02 s
for the unsat queries and 1.5 s for the `sat` unwinding query; `hbase + 16 * entry` with the exact
chain 0.02 s and 0.02 s, 146 queries 1.7 s in all.

`tests/verify-native/perf/run.sh` pins the shapes with a verdict and a wall-time bound
(`// TEST: perf-expect`), and `compiler/tests/test_vc_handle.vow` pins the term structure, which cannot
flake: no address symbol, no `array[h]` arm, a query that grows linearly with the rows.
