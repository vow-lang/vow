# The claims of a native round share one incremental solver session

**Status:** accepted (2026-10-11)

Resolves #1428. Part of epic #1398. Builds on
[ADR-2026-10-11-1200](2026-10-11-1200-query-slicing-and-per-clause-verdicts.md)
("slice, then confirm"). Scoped exception of
[ADR-2026-10-08-1421](2026-10-08-1421-verifier-lives-only-in-compiler.md): the
native verifier has no Rust twin.

## Context

A round of the unwinding schedule asks one query per claim: a spawn of
`bitwuzla`, a temp file, and the whole prefix (header, definitions, assumptions)
re-rendered, re-parsed and re-bit-blasted every time. The prefix of claim `k` is
the prefix of claim `k + 1`: `VcClaims` keeps one append-only header and one
append-only assumption list and a claim is two lengths into them.

## Decision

1. **One interactive process per round** (`compiler/vc_session.vow`).
   `process_start_piped` runs `bitwuzla --bv-output-format 2` on stdin. Each
   claim sends only the header commands and assumptions it adds to what the
   process already has (asserted at base level, so they accumulate), then its
   negated goal in a `(push 1)` ... `(pop 1)` scope, followed by `(check-sat)`
   and `(echo "vow-<k>")`. No `get-value` is ever sent.
2. **Only decisive, model-free answers are taken from a session.** `unsat` is a
   proof of the claim, as in one-shot mode. `sat` on a soft claim (a checked
   operator's abort, an unwinding claim) is taken as is: the verdict never reads
   a model. A `sat` on a hard claim goes straight to the standalone **full**
   query, whose answer alone decides and supplies the counterexample; `unknown`,
   a timeout or a dead session goes through the standalone sliced-then-full path
   of ADR-2026-10-11-1200 unchanged. So counterexample models, and therefore
   every report, are byte-for-byte those of one-shot mode: an incremental
   solver's model for a query is not promised to equal a fresh solver's.
3. **A reply is a verdict only when it is exactly `<verdict>` then the claim's
   sentinel.** Anything else (an `(error ...)` line, a missing or foreign
   sentinel, extra lines, end of file, a write error, a timeout) kills the
   process and ends the session; the claim and every later claim of the round
   are answered by standalone queries. An open-time probe
   (`(push 1)(pop 1)(echo "vow-open")`) rejects a solver without scope support
   before any claim is asked. Bitwuzla 0.9.1 exits 1 with empty stdout on any
   malformed command, an undefined symbol and an unbalanced `pop`, so a failed
   command can never leave a stale negated goal asserted for the next claim.
4. **The session may wait for at most half of what is left of the function's
   budget** per claim, so a hard full prefix cannot starve the standalone
   fallback of an easy cone. The one place a verdict can differ from one-shot
   mode is a claim whose full query needs more than that wait and whose slice
   also fails in the other half; it is `timeout`/`unknown` in both modes up to
   scheduling, never a wrong proof.
5. **The session asks the full prefix, not the claim's slice.** Combining the two
   needs guard literals and `check-sat-assuming`, which bloats the base problem
   and, with a non-literal assumption, is outside SMT-LIB's standard. Slicing
   stays on the standalone path. A round with fewer than two claims that need a
   solver has nothing to share and starts no session.
6. **One session per round, not per function.** Rounds renumber instances
   (`vc_unroll` numbers them in DFS order with radix `bound + 1`), so no term or
   definition name is common to two bounds and each round builds its own arena.
   Sharing across rounds needs a prefix-stable unrolling (iteration-major order,
   bound-independent names, one arena); that is a separate change and is not made
   here.
7. **`VOW_VERIFY_NO_INCREMENTAL=1`** (test-only, like `VOW_VERIFY_NO_SLICE`)
   gives every claim its own process. `scripts/full_test.sh` runs every
   `tests/verify-native` fixture both ways and requires identical JSON and exit
   status; `tests/verify-native/tests.sh` covers the session against a fake
   solver (`unsat`/`sat`/`unknown` replies, a solver that dies mid-round, one
   that prints an error line or has no `push`, a hang, an out-of-memory
   message, a killed worker).

## Measured effect

Wall-clock median of 5 runs on this ADR's branch, `vowc verify --backend native
--no-cache`, bitwuzla 0.9.1, with and without `VOW_VERIFY_NO_INCREMENTAL=1`.
Peak resident set is the maximum over the 5 runs and the descendants the driver
reaped. Fixtures the simplifier decides alone start no solver and are unchanged
within noise (about 20 ms of process start).

| fixture | one-shot s | session s | speedup | RSS MiB (one-shot / session) |
|---|---|---|---|---|
| `pass/loop_symbolic_body_bound_16` | 0.39 | 0.29 | 1.3x | 14 / 14 |
| `fail/loop_symbolic_body_bound_16_wrong_post` | 0.38 | 0.05 | 7.9x | 14 / 14 |
| `pass/loop_unbounded_checked_add_warning` | 4.38 | 0.23 | 18.8x | 17 / 17 |
| `unknown/loop_bug_beyond_bound` | 1.61 | 0.13 | 12.9x | 14 / 14 |
| `pass/loop_repeated_division_guard` | 0.21 | 0.09 | 2.2x | 16 / 16 |
| `pass/loop_literal_bound` | 0.08 | 0.02 | 3.3x | 16 / 14 |
| `pass/loop_unbounded_nested` | 0.08 | 0.02 | 4.3x | 14 / 14 |
| `pass/inline_call_in_loop` | 0.05 | 0.01 | 3.5x | 14 / 14 |
| `tests/verify-stress/unwind_loop` | 0.16 | 0.09 | 1.8x | 14 / 14 |

With the real solver, `loop_symbolic_body_bound_16` spawns 4 processes (one per
round) instead of 36.

## Consequences

- A session keeps every clause of its round resident in the solver; peak memory
  is bounded by the one-shot peak of the largest claim of the same round plus
  the session's learnt state, and the corpus shows no regression. If a fixture
  regresses, `vc_session_wanted` (`compiler/vc_native.vow`) is the knob: raise
  the claim threshold or recycle the session after N claims.
- The one-shot path is not removed: it is the fallback for every failure of the
  session and the only source of models.
- Follow-up: prefix-stable unrolling so one session spans the bounds of a
  schedule (decision 6).
