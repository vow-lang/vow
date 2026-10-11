# The native verifier caches solver answers by the exact query it asked

**Status:** accepted (2026-10-11)

Resolves #1429. Part of epic #1398. Builds on
[ADR-2026-10-11-1200](2026-10-11-1200-query-slicing-and-per-clause-verdicts.md)
(slice, then confirm in full) and
[ADR-2026-10-08-1430](2026-10-08-1430-native-verifier-cli-and-status-surface.md)
(`--no-cache` row).

## Context

Every claim of every function is a Bitwuzla run, and a rebuild of an unchanged
program asks the same questions again. The epic's "Cache" decision calls for an
on-disk result cache keyed on the query and the pinned solver. The Rust driver's
`VerifyCache` (`vow/src/cache.rs`) never stores a proof, so it cannot make a
proven function fast; this cache has to.

## Decision

1. **The seam is the solver call, keyed by its query text**
   (`compiler/vc_cache.vow`, `vc_solver_check_cached`). Every standalone query
   of `vc_check_claim`, sliced or full, goes through the wrapper. An answer is
   filed under the text it answered, so a sliced `sat` is a fact about the
   sliced text and the caller still re-asks the full one, exactly as in
   ADR-1200. No entry is ever keyed by a claim index or by a text other than
   the one sent. A shared solver session (ADR-1500 incremental sessions) never
   sees a standalone text, so `vc_check_claim` also looks the claim's full text
   up *before* asking the session (`vc_cache_peek`) and files a session `unsat`
   under that same text (`vc_cache_put`): a session's claim is by construction
   the standalone full query's claim, so a decision made either way serves
   both. A session `sat` on a soft claim carries no model and is not stored.
2. **Key** = two FNV-1a passes (`hash_str`, 128 bits in 32 hex characters) over
   a salt and the query. The salt is a schema tag (`vow-native-cache v1`), the
   first non-empty line of `bitwuzla --version` and the fixed solver argument
   list. A solver upgrade or any change to the query is a different key; a
   stale entry cannot exist.
3. **What is stored.** Only `unsat` and `sat` with its model values. `unknown`,
   timeout, memory, error and not-found depend on the budget or environment and
   are never stored. A `sat` hit replays the stored model, so a counterexample
   is byte-identical to the cold run's. A non-positive budget is a `timeout`
   before any lookup.
4. **Proofs are cached. This reverses the Rust cache's stance.**
   `VerifyCache` and `docs/spec/cli.md` keep proofs and the compile-object
   cache out of the loop so that a forged artifact cannot make a build wrongly
   *green*. Storing `unsat` is what makes a warm run fast, so this cache does
   accept that risk and bounds it instead:
   - an entry holds the whole query and a lookup compares it byte for byte, so
     a hash collision or a stale or foreign file is a miss, not a verdict;
   - every decode is fail-closed: a wrong tag, an unknown verdict word, a model
     of the wrong size, a truncated or over-long text is a miss;
   - writes are a unique temporary name plus `rename`, so a concurrent reader
     sees a whole entry or none, and a torn file is a miss.
   The **residual risk** is deliberate: anyone who can write the cache
   directory can forge an `unsat`. There is no MAC (a key stored beside the
   cache would not stop them). `--no-cache` is the opt-out and stays mandatory
   for a "green locally" claim. **Rejected:** caching only `sat` (no benefit
   for the common case, a proven function).
5. **Solver identity is `bitwuzla --version`**, probed once by the parent. A
   failed or empty probe disables the cache for the run (fail closed). The
   parent and its workers locate `bitwuzla` through `$PATH` separately; a swap
   between the probe and a worker mis-labels entries (the same TOCTOU window
   as `vc_solver_locate`). Accepted.
6. **Location and bound.** `$VOW_CACHE_DIR/verify-native/`, else
   `$HOME/.cache/vow/verify-native/`, sharded by the first two hex digits. A
   shard holds at most 64 entries: a store into a full shard first removes half
   of them, chosen by name order (`fs_listdir` order is not age order and there is no mtime
   builtin, so this is not LRU). Total at most 256 x 64 entries. A directory
   that cannot be created disables the cache; a cache failure never fails
   verification.
7. **Plumbing.** The parent resolves the cache once, only when a task will run,
   and passes `--worker-cache-dir` and `--worker-solver-id` to every worker
   (only when both are known). A worker without them asks the solver every
   time. `--no-cache` skips resolution.
8. **The performance gate measures with the cache off, by assertion.**
   `scripts/verify_perf.py` already passes `--no-cache` with a fresh
   `VOW_CACHE_DIR`; it now also fails a sample (harness exit 2) when that
   directory is non-empty after the run.

## Consequences

- A warm run's verdicts equal the cold run's only when no solver call timed out
  cold: a hit costs nothing, so a claim that timed out cold may be decided warm.
  The tests use no near-limit budgets.
- `contracts --verify` and `test --verify` on the native backend (#1427)
  build their own pool (`vc_driver_run`) and do not use the cache; only
  `verify` resolves it. Threading it through is a follow-up.
- Every oracle that must exercise the solver passes `--no-cache`
  (`scripts/full_test.sh`, `scripts/verify_diff.py`, `scripts/verify_perf.py`,
  `tests/verify-native/tests.sh`); the cache cases in those files use a private
  `VOW_CACHE_DIR` and assert, cold against warm, that the JSON is identical.
