# Native queries are sliced per claim and confirmed in full; every clause gets its own verdict

**Status:** accepted (2026-10-11)

Resolves #1424. Part of epic #1398. Builds on
[ADR-2026-10-08-1422](2026-10-08-1422-native-verification-semantics.md) (rule 2:
proof needs a closed unwinding claim).

## Context

Claim `k` of a function was asked as one query holding every declaration,
definition and assumption made before it, although most of them cannot
influence the claim's goal. The driver also stopped at the first hard claim that
was not `unsat`, so a function with several `ensures` produced one verdict, and
the clauses after a failing one were never decided. ESBMC's `--multi-property`
gives one verdict per clause, which `vowc contracts` reports (#1427 moves that
command to the native backend).

## Decision

1. **Slice, then confirm** (`compiler/vc_slice.vow`, `vc_check_claim`). The
   query of a claim is first asked on its cone of influence: the declarations,
   the logic and the model request, the definitions its negated goal reaches
   (through bound names, which stand for their definitions), and the
   assumptions made before it that mention a reached variable or a variable
   that depends on one through definitions. An `unsat` answer is final. Any
   other answer is re-asked on the full query, in what is left of the budget,
   and that answer alone decides.
2. **Hard claims always reach the full query on a non-`unsat` answer**, so a
   counterexample is the one the full query produces: reports are byte-for-byte
   what they were before slicing. The one exception is a soft claim (checked
   arithmetic, unwinding) from which only definitions were dropped: that query is
   equisatisfiable with the full one and its answer carries no model.
3. **All-claims mode** (`vc_verify_function_mode(.., all_claims)`, the worker
   flag `--worker-clauses`). Every claim of a round is decided; a hard claim
   that is not `unsat` is recorded and the round goes on. Only an answer no
   later claim can improve on (timeout, memory, missing or erroring solver)
   halts it. The overall result is still the first failing claim in claim order
   of the first round that has one, equal to the default mode's. A round with a
   failure is followed by larger bounds while its unwinding claim is open, so
   the clauses that did not fail can still be proven.
4. **Clause verdicts** (`compiler/vc_clause.vow`, pure): `failed` when any copy
   of the clause (one per unrolled iteration or path) is `sat`, in any round;
   `proven` when every copy is `unsat` in a round whose unwinding claim is
   `unsat`; otherwise no entry, and the caller maps the clause through
   `resolve_clause_status` onto the function's overall outcome, the vocabulary of
   `contracts-result.schema.json`. A `requires` is an assumption, never a claim,
   and gets no entry, exactly as under `--multi-property`.
5. **A clause with no claim is decided from the function as written.** The
   targets are the `ensures`/`invariant` ids of the pre-inline function. A target
   with no claim in a closed round had every claim point decided by the
   simplifier or on an unreachable path, so it is `proven`; in an open round it
   is undecided (its program point may need more iterations than the bound).
6. **Wire format v3.** `VOWRES3` appends the clause count and `(vow_id, proven)`
   pairs after the call chain; a stale `VOWRES2` or a proven flag other than 0/1
   decodes to nothing, never to a proof. The parent's `VcPool.clauses` adds the
   flag to every worker it starts.
7. **`VOW_VERIFY_NO_SLICE=1`** (test-only, like `VOW_VERIFY_WORKER_MEM_KB`) sends
   every full query. `scripts/full_test.sh` runs every `tests/verify-native`
   fixture both ways and requires identical JSON and exit status.

## Trust

The slicer is **not** in the trusted base for `sat`: a sliced `sat` is never
reported, it is re-asked. Its only trusted property is the proof direction: the
sliced query's assertions are a subset of the full query's (definitions are
macros and only unreferenced ones are removed), so `unsat(sliced)` implies
`unsat(full)`. That is checked structurally, not by a proof:

- `compiler/tests/test_vc_slice.vow` pins what is kept and dropped, that every
  kept definition precedes its use, and that nothing from after the claim leaks
  into its prefix;
- a sliced `sat` on a vacuous proof (a contradictory `requires` over an unrelated
  parameter) is covered by `tests/verify-native/tests.sh` and the
  `pass/slice_vacuous_unrelated_requires.vow` fixture: a trusting slicer would
  flip it to a counterexample;
- the both-ways corpus comparison above.

A bug in the slicer that drops a needed assumption can only cost a second query
for a claim that was `unsat` (the sliced query is then `sat`, the full one
`unsat`), never a wrong verdict. A bug that keeps a reference to a dropped
definition surfaces as a solver `error`, which is never a proof.

## Consequences

- A function whose claims depend on disjoint parameters sends smaller queries; a
  claim whose cone is the whole prefix sends exactly the query it used to.
- A non-`unsat` claim whose cone is a strict subset costs a second query (hard
  claims; soft claims that dropped an assumption). Counterexamples and
  `ArithOverflowReachable` sites are unchanged.
- **Result cache (#1429):** the cache key must be the query whose answer was
  used: the sliced query for an `unsat`, the full query for any other verdict.
  Never always the sliced one.
- **Pool early stop:** `vc_pool_finish` still cancels the tasks above the first
  function that did not prove, which is right for `verify` and wrong for a
  clause report across functions. #1427 owns the per-function independence it
  needs; this change only supplies the per-function clause list.
- Budget: a sliced attempt that times out leaves the confirming query no time.
  The shared function budget is unchanged.
