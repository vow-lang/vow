# Plan: native verifier result cache (#1429)

## Goal
Reuse solver answers across `vowc verify --backend native` runs. Each answer is keyed on a hash of
the exact query text sent to Bitwuzla plus the pinned solver identity, stored on disk, and
replayed on a later run so that warm and cold verdicts are identical. `--no-cache` keeps
disabling it and the perf gate (`scripts/verify_perf.py`) is guaranteed to measure with it off.
Self-hosted compiler only (scoped exception, ADR-2026-10-08-1421): no Rust twin, no C-emitter change.

## Assumptions
- **Seam = the solver call, keyed by query text.** Both `vc_solver_check` calls in
  `vc_check_claim` (sliced query, then full query) go through one cached wrapper that is keyed by
  the exact text it is given. That satisfies ADR-2026-10-11-1200 ("key must be the query whose
  answer was used; never always the sliced one") by construction: a sliced `sat` entry is a fact
  about the sliced text, and the caller still re-asks the full text on it. (best guess)
- **What is cached.** Only `unsat` and `sat`-with-model. `unknown`, timeout, memory, error and
  not-found depend on budget/environment and are never stored. A `sat` hit replays the stored
  model values, so the counterexample JSON is byte-identical to the cold run. (best guess)
- **Proofs are cached (trust decision) — this REVERSES an existing security stance.** The Rust
  `VerifyCache` (`vow/src/cache.rs`) and the `compiler/verifier.vow:1679` note deliberately never
  trust a cached PROVEN, and `docs/spec/cli.md:34` disables the compile-object cache during
  verification for the same reason (an attacker-supplied artifact must not bypass the checker).
  Storing `unsat` is what makes warm runs fast and AC1 meaningful, and the epic's "Cache" decision
  calls for it, so the plan caches it but does not pretend it is equivalent: a forged `unsat`
  entry makes a build wrongly GREEN, not just wrongly red. Mitigations: content-addressed (a stale
  entry is impossible; only forgery can mislead), the full query is stored and compared
  byte-for-byte on every hit (a hash collision cannot produce a verdict), every malformed, short
  or mismatching entry is a miss, writes are temp+rename. Residual risk: anyone with write access
  to the cache directory can forge `unsat`. `--no-cache` is the opt-out, and stays mandatory for
  "green locally" claims (CLAUDE.md). Alternative rejected: cache only `sat` (zero benefit for
  proven functions, which is the common case). The implementation stage posts this decision and
  the rejected alternative as a `gh issue comment 1429` and records it in the ADR.
- **Solver identity = `bitwuzla --version` output**, probed once by the parent when there is a
  task to run. Probe failure/empty output => cache disabled for the run (fail closed). The cache
  salt also includes a schema tag and the fixed solver argv (`--bv-output-format 2`).
- **Location** `$VOW_CACHE_DIR/verify-native/` else `$HOME/.cache/vow/verify-native/` (same root
  as the compile-object cache; documented in cli.md). Dir creation failure => cache off, silently
  (a cache problem must never fail verification).
- **Bounded disk.** Sharded by the first two hex digits of the key; a shard holds at most
  `VC_CACHE_SHARD_CAP` (64) entries; a store into a full shard first evicts an arbitrary half
  (`fs_listdir` order is not age order and no mtime builtin exists; eviction is arbitrary, not LRU). Total ≤ 256 × 64 entries.
- **`--timeout 0` / non-positive remaining budget stays an immediate `timeout`, checked before
  the lookup**, so cold and warm results are identical there too.
- **Observability for tests** is the fake solver's invocation log (already in
  `tests/verify-native/tests.sh`), not new CLI output; no stderr/JSON field is added.
- **Perf gate.** `verify_perf.py` already passes `--no-cache` and a fresh `VOW_CACHE_DIR`. AC3 is
  closed by making that an enforced invariant (post-run assertion + test), not by new product code.
- `contracts --verify` / `test --verify` on native belong to #1427; the cache config lives on
  `VcPool` so that work inherits it.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `compiler/vc_cache.vow` [new] | key, entry codec, disk lookup/store, eviction, dir + solver-id resolution | — |
| `compiler/vc_solver.vow` | `VcSolverAnswer`, `VC_ANS_*`, `vc_solver_args`, `vc_solver_check` (the call being wrapped) | 181-263 |
| `compiler/vc_native.vow` | `vc_check_claim` (two solver calls), `vc_run_round`, `vc_induction_attempt`, `vc_step_attempt`, `vc_verify_function_mode` | 127 (`vc_check_claim`), 228, 307, 325, 417 |
| `compiler/vc_worker.vow` | `VcPool` (add `cache_dir`, `solver_id`), `vc_pool_new`, `vc_worker_argv` | 305, 322, 359 |
| `compiler/cli_flags.vow` | `cf_is_worker_value` (new worker value flags) | 42-44 |
| `compiler/main.vow` | `run_verify_native` (resolve cache, honour `--no-cache`), `run_verify_worker` (read flags); generated help blocks | 1865 (`run_verify_native`), 1917 (`run_verify_worker`) |
| `docs/spec/cli.md` | line 77 "`--no-cache` ... has no effect (the native driver is uncached)"; cache location | 25, 48, 77 |
| `docs/adr/2026-10-11-1500-native-result-cache.md` [new] | decision record (schema above) | — |
| `docs/adr/2026-10-08-1430-native-verifier-cli-and-status-surface.md` | `--no-cache` row already promises the key; add "landed in #1429" pointer | 49 |
| `scripts/verify_perf.py`, `scripts/test_verify_perf.py` | AC3 enforcement | 285-325 / 249-270 |
| `scripts/concat_vow.sh` | `clif` module list: insert `vc_cache` after `vc_solver` | 23 |
| `tests/verify-native/tests.sh` | fake `bitwuzla` shim (add `--version`), new cache section | 24-96 |
| `scripts/full_test.sh` | Section 4g real-solver cold/warm/no-cache comparison | 1421-1480 |
| `compiler/tests/test_vc_cache.vow` [new], `compiler/tests/test_vc_worker.vow` | unit tests | — |

## Steps

### 1. Add the cache module `compiler/vc_cache.vow` [new]
Module `VcCache`; `use vc_solver`. Prefix every symbol `vc_cache_`. No `vow` blocks (matches the
rest of `vc_*`; effectful IO functions are `Skipped` by the gate anyway).
- `struct VcCache { dir: String, solver_id: String }`; `vc_cache_off()` = empty dir; `vc_cache_on(c)` = `dir.len() > 0 && solver_id.len() > 0`.
- Pure: `vc_cache_salt(solver_id, args) -> String` (schema tag `vow-native-cache v1`, solver id,
  `vc_solver_args("")` shape); `vc_cache_key(salt, query) -> String` = 32 hex chars from two
  `hash_str` passes with different prefixes; `vc_cache_path(dir, key)` = `<dir>/<key[0..2]>/<key>`.
- Pure codec: `vc_cache_encode(verdict, values, highs, query) -> String` and
  `vc_cache_decode(text, query, n_names) -> Option<VcSolverAnswer>`. Layout: tag line, verdict
  (`sat`|`unsat`), value count + `lo hi` lines (count must equal `n_names` for sat, 0 for unsat),
  `<qlen>\n<query>\nEND\n`. Decode returns `None` unless the whole text parses, the stored query
  equals `query` byte-for-byte and `END` is the last line. Reuse the length-prefixed reader style
  of `vc_wire_read_int` / `vc_wire_read_str` (`compiler/vc_worker.vow:117-150`).
- IO: `vc_cache_lookup(c, query, names, widths) -> Option<VcSolverAnswer> [read]`;
  `vc_cache_store(c, query, ans) [io, read, write]` (only `VC_ANS_SAT`/`VC_ANS_UNSAT`; `fs_mkdir`
  shard, write `<key>.<time_micros>.tmp`, `fs_rename`, `fs_remove` tmp on failure; evict via
  `fs_listdir` when the shard has ≥ `VC_CACHE_SHARD_CAP`).
- `vc_solver_check_cached(c, bin, query, names, widths, budget_ms, scratch)`: budget ≤ 0 =>
  delegate (timeout without lookup); hit => answer (raw `""`); miss => `vc_solver_check`, store
  decisive answers, return it.
- Resolution (called by the parent): `vc_cache_dir_from(vow_cache_dir, home) -> String` (pure,
  precedence `VOW_CACHE_DIR` > `$HOME/.cache/vow`, `""` if neither; appends `/verify-native`),
  `vc_cache_solver_id(out: String) -> String` (pure: first non-empty trimmed line),
  `vc_cache_resolve(bin) -> VcCache [io, read, write]` (empty `bin` => off; reads env;
  `process_start(bin, ["--version"])` with a 10 s `process_wait_timeout`; reads stdout/stderr,
  then `process_kill(handle)` to free the process-table entry as `vc_solver_check` does
  (`vc_solver.vow:249-253`); nonzero/empty/timeout => off; creates the dir).

### 2. Thread the cache through the engine (`compiler/vc_native.vow`)
- `vc_check_claim`: replace both `vc_solver_check(...)` with `vc_solver_check_cached(cache, ...)`;
  add a `cache: VcCache` parameter. The slicing logic and its "unsat is final / re-ask full" rule
  stay byte-for-byte as they are.
- Add `cache: VcCache` to `vc_run_round`, `vc_induction_attempt`, `vc_step_attempt` and
  `vc_verify_function_mode` (after `scratch`). Update the call sites in this file and the one
  caller in `compiler/main.vow:1933`. Grep `vc_verify_function_mode|vc_run_round` in
  `compiler/tests/` for other callers.
- Update the header comment of `vc_check_claim` to say answers go through the cache.

### 3. Plumb parent -> worker (`vc_worker.vow`, `cli_flags.vow`, `main.vow`)
- `VcPool`: add `cache_dir: String`, `solver_id: String` (empty in `vc_pool_new`);
  `vc_worker_argv` appends `--worker-cache-dir <dir> --worker-solver-id <id>` only when both are
  non-empty (mirrors `--worker-scratch`).
- `cli_flags.vow`: add both flags to `cf_is_worker_value`.
- `run_verify_native`: when `!has_flag(argv, "--no-cache")` and at least one `en.task >= 0`,
  `vc_cache_resolve(vc_solver_locate())` once, store into the pool.
- `run_verify_worker`: build `VcCache { dir: get_flag_arg(..), solver_id: ... }`, pass to
  `vc_verify_function_mode`. Update the doc comment on `run_verify_worker`.

### 4. Make AC3 an enforced invariant (`scripts/verify_perf.py`)
- In `run_once`, after the child exits and before the temp dir is removed, assert
  `os.listdir(cache) == []`; on violation print `verify_perf: cache was used while measuring` and
  mark the sample invalid (exit 2 from the harness, same class as corpus drift).
- Keep `--no-cache` first in `args` (already `scripts/verify_perf.py:285`).

### 5. Harness sweep (done in planning; the implementation must keep it true)
- Every `--backend native` call site passes `--no-cache`: `scripts/full_test.sh:1461,1520,1565`,
  `scripts/verify_diff.py:196`, `scripts/verify_perf.py:285`, and the `run_native`/`combined`
  helpers in `tests/verify-native/tests.sh:271-274,457`. The only fake `bitwuzla` shim is
  `tests/verify-native/tests.sh:24-96`; `.github/actions/install-bitwuzla` is the real pin. No
  oracle can pass vacuously because of a warm cache. Do not remove those flags.
- Shim: handle `--version` as the first statement, before the slot counter, `q.N.smt2` copy,
  `paths`/`args` logs and any mode (`hang` writes `$FAKE_BW_DIR/pid`): print
  `${FAKE_BW_VERSION:-}` and exit 0 if set, else exit 2. A `--version` probe must never appear in
  the query log or block.
- Add `run_native_cached` (same as `run_native` minus `--no-cache`, plus `VOW_CACHE_DIR`); only
  the new cache section uses it.
- `scripts/concat_vow.sh:23`: add `vc_cache` to the `clif` file list after `vc_solver` and before
  `vc_native` (the concatenated bootstrap triple test breaks on a missing module).

### 6. Docs and generated help
- `docs/spec/cli.md`: native bullet (line 77) says `--no-cache` disables the native result cache;
  add a "Native result cache" paragraph (location, key, what is cached, solver version
  invalidation, `--no-cache`, perf gate runs with it off); keep the `--no-cache` table rows.
- New ADR `docs/adr/2026-10-11-1500-native-result-cache.md` carrying the Assumptions above
  (including the proof-trust decision and the residual risk). Reference ADR-1200's consequence.
- `docs/verifier-eval.md` (~line 198): one sentence that the harness asserts the cache dir stays
  empty.
- `CLAUDE.md` "Bootstrap" subsection: one sentence that the native cache does store proofs, so
  `--no-cache` stays mandatory for "green locally" claims. (The existing sentence about the
  Rust `VerifyCache` stays true and is not edited.)
- Regenerate: `uv run python scripts/generate_help.py` (rewrites the embedded help in
  `compiler/main.vow` and `vow/src/skill.rs`, plus `skills/vow/reference/cli.md` if listed);
  then `python3 scripts/check_help_coverage.py`.

## TDD slices (red -> green, each a small commit)
1. **Codec + key** — `compiler/tests/test_vc_cache.vow`: round-trip sat/unsat; decode rejects
   bad tag, verdict other than sat/unsat, value-count != names, truncated body, extra bytes
   after `END`, stored-query mismatch (same key, different text); key golden value; key differs
   when solver id, args salt or one query byte differs. Green: step 1 pure functions.
2. **Disk store/lookup** — same file, `mktemp_dir`: miss; store then hit; corrupt file => miss;
   `unknown`/timeout/error/memory/not-found are not stored; 65 stores into one shard stay ≤ cap
   and leave no `.tmp`. Green: IO functions + eviction.
3. **Resolution** — same file: `vc_cache_dir_from` precedence/empty; `vc_cache_solver_id`
   (blank lines, trailing newline, empty => `""` => off).
4. **Wrapper budget rule** — same file: store an entry for query Q in a temp cache dir, then
   call `vc_solver_check_cached` with `budget_ms = 0` and the same cache; expect
   `VC_ANS_TIMEOUT`, not the stored answer (budget is checked before the lookup). A hit with
   positive budget returns the stored answer without spawning (`bin = ""` proves no spawn).
5. **Plumbing** — `compiler/tests/test_vc_worker.vow`: `vc_worker_argv` contains the two flags
   only when set and never when `--no-cache` is modelled by empty fields. Green: step 3.
6. **End-to-end with fake solver** — `tests/verify-native/tests.sh` (extend the shim: handle
   `--version` first, before it records the query, printing `$FAKE_BW_VERSION`; with it unset the
   shim must exit non-zero so existing tests are unaffected and run cache-off). Add, with
   `VOW_CACHE_DIR="$TMP_ROOT/cache"`: (a) cold run records N>0 queries, warm run records 0 new and
   JSON identical (`unsat` function and a `sat` counterexample function); (b) change
   `FAKE_BW_VERSION` => queries are asked again; (c) `--no-cache` => solver asked every run and
   cache dir empty; (d) shim without `--version` support => verdicts still correct, cache dir
   empty; (e) truncate/garble every entry => verdict unchanged, solver asked again; (f) a
   function whose sliced query is `sat` but full is `unsat` (existing
   `pass/slice_vacuous_unrelated_requires.vow`) gives the same verdict warm and cold;
   (g) `--timeout 0` warm still `timeout`. Set `VOW_CACHE_DIR` for every existing invocation in
   this file that lacks `--no-cache` so they cannot touch `$HOME/.cache/vow`.
7. **Real solver** — `scripts/full_test.sh` Section 4g: after the existing `--no-cache` loop,
   run every pass/fail/unknown fixture twice with one fresh shared `VOW_CACHE_DIR` and require
   JSON + exit status equal to the `--no-cache` baseline in both passes. Non-vacuity: after the
   cold pass assert `$VOW_CACHE_DIR/verify-native` contains at least one entry (a failed
   `--version` probe silently disables the cache and would otherwise make cold == warm trivially);
   fail the section if the real `bitwuzla --version` probe yields nothing.
8. **Perf guard** — `scripts/test_verify_perf.py`: a stub that writes into `$VOW_CACHE_DIR`
   makes `run_once` flag the sample; the existing `test_cache_off_flags_env_and_result_json`
   still passes. Green: step 4.

## Verification surface
- No contracts or C-model change. The native verifier does not verify `compiler/` yet (no
  `scripts/seed.toml`): bootstrap verifies it through Rust stage 0 + ESBMC. So `vc_cache.vow`
  must type-check under the Rust compiler and its pure helpers must pass ESBMC's bounds checks on
  `byte_at`/`substring`. Shape the decoder like `vc_wire_read_int`/`vc_wire_read_str`
  (`vc_worker.vow:117-150`), which already verify: index only after a `< len` guard, explicit
  failed-cursor state, no nested index arithmetic. Mark nothing unverifiable unless ESBMC rejects
  a correct function; never weaken a contract. IO functions carry effects and no `vow` blocks.
- Fixtures: none under `tests/run/` or `examples/`. New coverage lives in
  `compiler/tests/test_vc_cache.vow`, `tests/verify-native/tests.sh` and the Section 4g loop.
- Run: `build/vowc test compiler/tests/test_vc_cache.vow`, `... test_vc_worker.vow`,
  `bash tests/verify-native/tests.sh` (`VOWC_BIN=build/vowc`),
  `python3 -m unittest scripts/test_verify_perf.py`, `scripts/bootstrap.sh --skip-cargo --no-cache`,
  `VOW_FULL_TEST_SKIP_CARGO=1 scripts/full_test.sh` (background + poll; ~40 min).

## Risks
- **Forged `unsat` (trust).** Different from the Rust cache by design; see Assumptions. Mitigated
  by exact-query equality and fail-closed decode, documented in the ADR, `--no-cache` retained.
- **Wrong cache key (sliced vs full).** Solved by construction (key = text actually sent). Slice
  6(f) pins it; do not key on claim index or on the sliced text while storing the full answer.
- **Test contamination.** A shared `~/.cache/vow` plus the fake solver (same query, different
  fake answers) would replay another test's answer. The shim only supports `--version` when a
  test opts in, and every other invocation sets `VOW_CACHE_DIR` to a temp dir.
- **Concurrency.** Workers share the dir: unique temp name + `fs_rename` (atomic), torn/short
  reads decode to a miss, eviction racing a read is a miss.
- **Timeouts are not reproducible.** A hit costs ~0 ms, so a claim that timed out cold can be
  decided warm. "Cold and warm verdicts identical" therefore holds only for runs in which no
  solver call times out; the ADR and cli.md say so. The test slices use no near-limit budgets.
- **Warm run changing output.** Only `unsat`/`sat`+values are stored; reports for
  unknown/timeout/error are never replayed. `raw` is not needed for those two verdicts
  (`vc_cex.vow:60-92` reads `ans.raw` only for non-decisive answers).
- **Parent/worker solver mismatch.** The parent probes `bitwuzla --version` once and the worker
  re-locates `bitwuzla` via `$PATH`; a swap in between mis-labels entries. Accepted (same TOCTOU
  window as the existing `vc_solver_locate`); noted in the ADR.
- **Signature churn.** `vc_verify_function_mode` gains a parameter; a missed caller is a compile
  error in `compiler/tests`, so run the full `build/vowc test compiler/`.
- **Bootstrap fixed point / parity.** No codegen, lowering, or emitter change; the Rust compiler
  gets only regenerated help text (`cargo build --release -p vow` must still pass,
  `scripts/check_help_coverage.py` clean). C parity (Section 2c) untouched.
- **Commit hygiene.** Conventional Commits, lower-case subject, e.g.
  `feat(verify): cache native solver answers by query and solver version`.

## Out of scope
- Caching for the ESBMC backend or any change to Rust `vow/src/cache.rs` / `VerifyCache`.
- A cryptographic hash or MAC over entries; signed/keyed cache; sharing across machines.
- LRU/mtime-aware eviction, `vowc cache clean` subcommand, size-based global quota.
- Per-function (whole-result) cache, caching incomplete answers (timeout/unknown/error).
- `contracts --verify` / `test --verify` on native (#1427); pool early-stop independence.
- New CLI output fields (hit/miss statistics); perfetto spans for cache hits.
- Refactoring the `scratch`/`slicing` parameters into a context struct.
