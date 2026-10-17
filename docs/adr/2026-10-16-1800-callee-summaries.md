# ADR-2026-10-16-1800: A proven callee stands for its body where inlining is too large

Status: accepted (issue #1791, epic #1398)

Addendum to [ADR-2026-10-08-1421](2026-10-08-1421-verifier-lives-only-in-compiler.md)
(the native verifier has no Rust twin) and to the inlining of
`compiler/vc_inline.vow`.

## Context

The native verifier decides a function by inlining its whole call tree. A decoder
that is a long sequence of calls to small readers costs the sum of everything
below it, and a call tree above 16 384 instructions after flattening is skipped
as `inlining-too-large`: `read_function` (23 331) and `module_decode` of
`compiler/module_io.vow` are correct and fully modelled, but a worker does not
decide them within its 4 GB cap. Every claim of every inlined callee is also
decided again in every caller, although the callee was proven on its own.

## Decision

1. **A summary is a function.** For a callee `g`, `vc_stub_of` builds a function
   with the id, parameters, result type and `vow` entries of `g`: the code of `g`
   up to its last `requires` (so the clauses are claims of a spliced frame,
   blamed on the caller, with the call chain of an inlined call), then a
   `FieldSet` of an unconstrained value (`__vc_havoc`) for every field slot of a
   parameter object that `g` may write, then the `ensures` clauses of `g` as
   assumptions (point 6), then a `Return` of an unconstrained value.
   `vc_summary_module` returns a copy of the module in which a trusted callee is
   its summary, and the ordinary inliner, gate, loop cut and executor see plain
   IR. A callee's own claims, `ensures` and loop invariants are not in the
   summary: they were decided by its own proof.
2. **Only above the budget, only trusted.** A function whose call tree is within
   the budget (16 384 instructions; the test-only `VOW_VERIFY_INLINE_BUDGET`
   lowers it) is inlined as before, so no verdict, counterexample or blame of
   such a function changes. Above it, a callee is summarised when its function id
   is in the trusted set, its own call tree is above a quarter of the budget, and
   `vc_effects_of` accepts it. The trusted ids are the functions whose standalone
   proof was `PROVEN` in the same run; nothing else is ever trusted. A function
   whose size stays above the budget keeps today's `inlining-too-large` skip.
3. **What a summary must cover is decided on the callee's flattened body,
   fail-closed.** Every write must be a field of a parameter object (or of an
   object the callee built). Nothing the callee was given, nor anything it read
   out of given memory (a pointer or `i64` loaded from a field or a `Vec` slot,
   which the IR types alike), may be stored anywhere or returned as a pointer:
   the heap model assumes a value read out of memory is not one of the caller's
   own objects, which holds for an object the callee built and for no other.
   A callee that mutates a `Vec`, `String` or map it was given has no summary
   (a later change can havoc them like a loop cut does).
4. **The proofs a summary rests on are `PROOF` tasks.** A `PROOF` task decides
   every claim of a function as for a function with a contract (a function
   without a `vow` block is otherwise judged on its callees' preconditions
   only), and a `PROVEN` one is trusted. The parent runs them in waves
   (`compiler/vc_waves.vow`): callees whose own call tree needs no summary first,
   then those whose summaries the first wave proved, until nothing is left that
   can be tried. A callee that is no verify target but is itself above the budget (an uncontracted
   intermediate) is wanted too, together with the callees it needs. A callee that is not proven, or that the native subset does not
   take, is never trusted and the function that needed it is skipped as before.
   The parent plans from the proven ids and tells every worker the same ids
   (`--worker-summaries`), so planner and worker build the same module.
5. **A summary over-approximates, so it never produces a counterexample.** A
   function decided with a summary standing for a callee that `sat` on a hard
   claim is `unknown` (`callee summary: ...`), never `VerifyFailed`: the havoced
   state may be one the callee cannot leave. `PROVEN` is unaffected. A
   checked-arithmetic abort inside a summarised callee is reported by the
   callee's own verification, not again by its callers; an abort a caller finds
   under the summary may be spurious and is a warning, as for any soft claim.

6. **The `ensures` of the callee is assumed after the havoc.** The lowerer emits the
   code of every clause at every `Return`, between the returned value and the
   `Return`, and gives each instruction the source range of the expression it came
   from; a body never lies inside a clause. `vc_summary_ens` copies the code of
   each clause out of every return site and uses it only when all copies are the same code (a
   name a `let` of the body shadows reads another value at a later return): the data
   closure of the predicate, with a parameter read mapped to the summary's own and
   the returned value mapped to the summary's unconstrained result, and the
   control flow from the nearest block that dominates the clause to the block
   holding its `VowEnsures`, which becomes an assumption (`__vc_assume`, a
   `VowRequires` the splicer does not turn into a claim of the frame). A clause
   reads the state after the havoc, so a field the callee wrote has the value the
   clause constrains and a field it did not write keeps its value. The extraction
   is fail-closed: a clause that reads a value that is neither a parameter, the
   result nor code inside the clause (a local, a parameter the body reassigned),
   or whose region leaves the clause, is not copied, and a summary the gate would
   reject with its clauses is built without them.

## Consequences

- `read_function` and `module_decode` are decided instead of skipped; their
  rows leave `scripts/native_compiler_skips.toml`.
- `vowc contracts` and `vowc test --verify` (`vc_driver_run`) trust nothing and
  skip as before.
- A clause the extraction cannot copy (point 6) is not assumed, so a caller that
  needs it ends `unknown`; the summary is as sound without it.
- The summary budget is part of the run, not of the program; the language and
  the contracts are untouched, so no contract is bounded or weakened to fit it.
