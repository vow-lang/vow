# 0003. Size types are unsigned

**Status:** accepted (2026-08-31)

## Context

`Vec::len()` returns `i64`. So do `String::len()`, `HashMap::len()`, and
`BTreeMap::len()`, and indices and capacities are `i64` throughout the surface
language. A length is a cardinality: it has no negative values. The type does
not say so, and the corpus pays for the gap in three ways.

**Contracts carry the type's missing information.** Several hundred
`requires`/`ensures`/`invariant` clauses across `compiler/`, `tests/`,
`stdlib/`, `examples/`, and `benchmarks/` exist only to restate that a
length-shaped value is non-negative. Those clauses are exactly the
tautological-contract population that #81 was filed over. Strengthening the
type deletes them; policing them one at a time does not.

**The runtime already disagrees with the surface type.** The Vec side of the
runtime is `usize` (`vow-runtime/src/lib.rs`), so the `i64` surface type is
already fiction at the boundary. A negative index is representable in the type
system and unrepresentable in the implementation.

**Descending loops cannot be written honestly.** The two corpus descending
loops that carry an invariant — `stdlib/math/vec_math.vow` and
`examples/sat/solver.vow` — both express the bottom of the range with
`>= -1`. They are the only `invariant` clauses containing a negative literal
anywhere in the corpus, and under `u64` a negative literal is not merely
unprovable but a `LiteralOutOfRange` compile error. The replacement idiom
(guard on `> 0`, decrement first) was already the majority convention in the
corpus but was undocumented until the `Descending Loops` section of
[`grammar.md`](../spec/grammar.md#descending-loops) landed alongside this ADR.

ADR 0001 Decision 1 forbids this change in writing: *"No `isize`/`usize`.
`Vec::len() -> i64` stays."* That sentence conflates two separable claims, and
this ADR separates them rather than silently overriding the record.

## Decisions

1. **No `isize`/`usize` — ADR 0001's exclusion is KEPT.** Pointer-width types
   would make the same source produce different binaries on different hosts,
   which breaks the SHA-256 binary fixed point that `scripts/bootstrap.sh`
   verifies (`sha256(vowc2) == sha256(vowc3)`). Vow remains 64-bit-only. This
   half of ADR 0001 Decision 1 stands unchanged.

2. **`Vec::len() -> u64` — ADR 0001's second claim is REVERSED.** `u64` is a
   fixed-width type. It has the same representation on every host, so the
   binary fixed point is fully preserved. The determinism argument that
   justifies Decision 1's first half has no bearing on its second half.
   `String::len()`, `HashMap::len()`, `BTreeMap::len()`, indices, capacities,
   and length-shaped parameters move with it.

3. **The two halves are independent.** Nothing about excluding pointer-width
   types implies anything about the signedness of a fixed-width length. Future
   readers should treat Decision 1 of ADR 0001 as two decisions that were
   written as one.

4. **`LiteralOutOfRange` is a partial completeness oracle.** `i64 + u64` and
   `u64 < i64` are already `TypeMismatch`, and a `-1` comparison against a
   migrated value is already a hard `LiteralOutOfRange` error, so every site
   *type-connected* to a migrated signature is found by the type checker rather
   than by inspection. The oracle stops there, and the boundary matters: an
   index expression accepts any width and either signedness, so an index-shaped
   parameter that never touches a migrated signature stays silently signed and
   emits no diagnostic. `examples/vec_bounds.vow`'s `get_element(i: i64)` —
   guarded only by literal comparisons and used as `v[i]` — is the shape that
   survives the whole migration untouched. Indices, capacities, and other
   semantically length-shaped parameters therefore still need an explicit
   inventory. The checker narrows that audit; it does not replace it.

5. **Sentinel returns are not swept into this decision.** `String.byte_at`
   returns a byte *value*, not a position, and stays `i64` under any lengths
   migration. Only genuine lookup functions that return a position-or-absent
   result are candidates for `Option<u64>`, and that is separate work.

## Consequences

The honest cost is a real one and is recorded here rather than in the epic.

**`len - k` underflow becomes silent.** Under `i64` an underflowed length is
negative — a self-announcing value that a `>= 0` check or an index bounds
check catches. Under `u64` it is `18446744073709551615`, which is a valid
value of the type. `-1 as u64` yields the same bit pattern with no check at
all; same-width signedness changes are bit reinterpretations and are
explicitly legal per `grammar.md`. The `.len() - 1` sites in the corpus are
where this bites, which is why the documented descending-loop idiom starts the
index at `n` and decrements under a `> 0` guard instead of starting at
`n - 1`.

**Checked arithmetic does not yet compensate.** `-!` traps at runtime under
`--mode debug`, but both C emitters collapse the checked and wrapping
arithmetic opcodes to the same C operator, so the verifier does not model the
distinction and `n -! 1` and `n - 1` produce byte-identical counterexamples.
Closing that gap (#585) is a prerequisite for the migration's later phases,
not something this ADR assumes is already done.

**There is no prover win.** This decision buys type-system correctness, not
solver performance. Zero `__ESBMC_assume` calls become deletable: every length
assume is two-sided, and while the `>= 0` half becomes redundant, the
`< CAP` half is the array bound and stays. For *scalar* length-shaped
parameters the admitted domain actually grows — `i64 n` with
`requires: n >= 0` admits 2⁶³, a bare `u64 n` admits 2⁶⁴. The cost is small
and bounded, but it is a cost, not a saving.

## Considered options (and why rejected)

- **Keep `i64` and police the tautologies.** Requires a diagnostic that
  rejects `requires: len >= 0` while the type still admits negative lengths,
  which is a lint against a legal program rather than a fix. It also leaves
  the descending-loop idiom unwritable, since `>= -1` remains the only way to
  express the bottom of the range.
- **Add `usize` and use it for lengths.** The natural choice in a language
  that is not reproducibility-constrained, and rejected here for exactly the
  reason ADR 0001 gave: pointer width defeats the binary fixed point.
- **Introduce a distinct `Len` or `Index` newtype.** Expresses the intent
  precisely, but adds a new type-system axis, which the language design rule
  in `CLAUDE.md` rejects outright. `u64` reuses machinery that already exists.
- **Migrate everything in one change.** 4,793 corpus sites are in scope.
  Rejected in favour of the dependency-ordered seams tracked by epic #1104,
  each independently correct and independently landable.

## Open follow-ups

- The six other spec sites that teach `invariant: i >= 0` / `requires: i >= 0`
  on ascending `i64` loops remain correct until the lengths migration reaches
  them, and are tracked separately from this ADR.
- Whether `Vec::len()`'s `as u64` bridge can be deleted once the return type
  itself changes is a later phase of epic #1104; the documented idiom carries
  the bridge until then.

## Amendments

None yet.
