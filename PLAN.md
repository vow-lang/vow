# Plan: isolate model-authored candidate programs before running them (#1188)

## 1. Problem restated

Tier 3 (`scripts/pair_review.py`, the adversarial pair-review harness, the only
tier that carries `ANTHROPIC_API_KEY`/`OPENAI_API_KEY` in its environment)
compiles and executes model-authored `.vow` candidates through four
subprocess call sites — `run_compiler`/`run_binary` in `scripts/equivalence.py`
and `run_json`/`run_debug_binary` in `scripts/verifier_runtime.py` — none of
which pass `env=`, and the two that run compiled *candidate binaries*
(`run_binary`, `run_debug_binary`) run them with `cwd=REPO_ROOT`. A candidate
that declares `[io]` and calls `process_run`/`fs_write`/`fs_remove` (all real,
implemented builtins — effects are a declaration, not a capability gate) can
therefore read the checkout's credential-bearing environment or touch
checkout files while under a credentialed Tier-3 run. This chore closes the
two cheapest gaps named in the issue — env scrubbing and a disposable cwd for
candidate binaries — and explicitly leaves full sandboxing (filesystem
allowlist, network denial, process-tree/CPU/fd limits) for separate,
individually reviewed follow-up work, per the issue's own "smallest credible
first" framing.

## 2. Files to touch

All changes are confined to `scripts/` (Python test-harness tooling). This is
**not** a language-semantics change — it does not touch `crates/`, `compiler/`,
or `docs/spec/*.md`; the "modify both compilers" rule in `CLAUDE.md` does not
apply here.

- **New:** `scripts/candidate_isolation.py` — shared isolation primitives:
  - `scrubbed_env(source=None) -> dict`: a copy of `source` (defaults to
    `os.environ`) with credential-shaped variables removed — names matching
    `(?:^|_)(API_KEY|TOKEN)$`, case-insensitive (catches `ANTHROPIC_API_KEY`,
    `OPENAI_API_KEY`, `GITHUB_TOKEN`, etc.). `PATH` and `VOW_CACHE_DIR` never
    match and need no special-casing.
  - `disposable_workdir() -> tempfile.TemporaryDirectory`: a fresh, empty
    directory a candidate-executed process can use as `cwd` instead of the
    checkout root. Returned as the stdlib context manager directly — no
    wrapper type — so callers write `with disposable_workdir() as d: ...`.
- **New:** `scripts/test_candidate_isolation.py` — unit tests for the above.
- `scripts/equivalence.py`:
  - `run_compiler` — add `env=candidate_isolation.scrubbed_env()`. `cwd`
    stays `REPO_ROOT` (the compiler resolves relative fixture/output paths
    and `VOW_CACHE_DIR` against it; this call runs a *compiler*, not
    model-authored code).
  - `run_binary` — add the same `env=`, plus a new `isolate_cwd: bool = False`
    parameter: when `True`, run inside `disposable_workdir()`; when `False`
    (default), keep `cwd=REPO_ROOT` unchanged.
  - `compare_runtime` — new `isolate_cwd: bool = False` parameter, forwarded
    to all three `run_binary` calls (the double reference-run plus the peer
    run).
  - `check_file` — pass `isolate_cwd=not honour_directives` at its one
    `compare_runtime` call site. This reuses the *existing* `honour_directives`
    flag (already `False` exactly when `--no-directives` is passed, which is
    already documented at `NO_DIRECTIVES` as "the input is a candidate, not a
    corpus fixture") rather than adding a new CLI flag or a second axis of
    trust. Tier 2's own corpus sweep (`honour_directives=True`, the default)
    is completely unaffected.
- `scripts/verifier_runtime.py`:
  - `run_json` — add `env=candidate_isolation.scrubbed_env()`. `cwd` stays
    `REPO_ROOT` (same reasoning as `run_compiler`).
  - `run_debug_binary` — add the same `env=`, and unconditionally run inside
    `disposable_workdir()` instead of `cwd=REPO_ROOT`. Unconditional (no
    `isolate_cwd` flag) is safe here: grepped `tests/verify/`,
    `tests/verify-fail/`, and `examples/` (its default corpus roots) for
    `fs_open`/`fs_read`/`fs_write`/`process_run` and found none that assume
    `cwd == REPO_ROOT` (see Risk Areas — re-verify this before implementing,
    since the corpus can have grown since this plan was written).
- `scripts/pair_review.py` — **no changes.** It already shells out to
  `equivalence.py` with `--no-directives` for `confirm`/`confirm_both_paths`,
  and calls `check_soundness` (which calls `run_json`/`run_debug_binary`
  directly) for `confirm_soundness`/`confirm_soundness_pair`. Both paths pick
  up the new isolation automatically because it lives in the shared functions,
  not in `pair_review.py` itself.
- `scripts/test_equivalence.py` — new tests for `run_compiler`/`run_binary`
  env scrubbing, `run_binary`/`compare_runtime` cwd isolation gating, and the
  `check_file` → `compare_runtime` wiring.
- `scripts/test_verifier_runtime.py` — new tests for `run_json`/
  `run_debug_binary` env scrubbing and `run_debug_binary` cwd isolation.
- `docs/equivalence/README.md` — one short paragraph documenting that Tier 3
  candidate execution now scrubs credential-shaped env vars and runs
  candidate binaries from a disposable directory, and that Tier 2's corpus
  sweep deliberately keeps `cwd=REPO_ROOT` because corpus fixtures
  (`tests/run/fs_read_line_basic.vow` and siblings, `tests/multi/
  vmod_region_roundtrip/main.vow`) reference fixture paths relative to the
  checkout root by convention. This is the one non-code file worth updating;
  it is the doc that already states each tier's guarantees.

No `docs/spec/*.md` changes: no new builtin, CLI flag, or language semantic
is introduced.

## 3. TDD slices

Each slice is red → green → (refactor if needed). Run with
`python3 scripts/test_<name>.py` per file, matching how CI invokes them
(`.github/workflows/ci.yml`).

1. **`scrubbed_env` strips credential-shaped variables, keeps the rest.**
   - Test (`scripts/test_candidate_isolation.py`): given a source dict with
     `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GITHUB_TOKEN`, `MY_APP_TOKEN`,
     `PATH`, `VOW_CACHE_DIR`, and an unrelated `HOME`, assert the four
     credential vars are absent from the result and the other three survive
     with their original values.
   - Production: `candidate_isolation.scrubbed_env`.

2. **`scrubbed_env` defaults to `os.environ`.**
   - Test: `mock.patch.dict(os.environ, {...}, clear=True)`, call
     `scrubbed_env()` with no argument, assert against the patched
     environment (not the real one).
   - Production: default `source=None` branch.

3. **`disposable_workdir` yields a fresh, existing, removed-on-exit directory.**
   - Test: `with candidate_isolation.disposable_workdir() as d: assert
     Path(d).is_dir()`; after the `with` block, assert `Path(d).exists()` is
     `False`.
   - Production: thin wrapper over `tempfile.TemporaryDirectory`.

4. **`run_compiler` passes a scrubbed environment.**
   - Test (`scripts/test_equivalence.py`): `mock.patch.object(subprocess,
     "run")` (or `mock.patch("equivalence.subprocess.run")`) plus
     `mock.patch.dict(os.environ, {"ANTHROPIC_API_KEY": "sk-x", "PATH": "/bin"},
     clear=True)`; call `equivalence.run_compiler(...)`; assert the captured
     `env=` kwarg lacks `ANTHROPIC_API_KEY` and keeps `PATH`. Assert `cwd`
     is still `equivalence.REPO_ROOT` (pins the "compiler keeps REPO_ROOT"
     half explicitly, since no test currently exercises the raw
     `subprocess.run` call at all).
   - Production: add `env=candidate_isolation.scrubbed_env()` to the
     `subprocess.run` call in `run_compiler`.

5. **`run_binary` passes a scrubbed environment.**
   - Test: same pattern as slice 4, against `equivalence.run_binary`.
   - Production: same `env=` addition in `run_binary`.

6. **`run_binary(..., isolate_cwd=False)` (default) keeps `cwd=REPO_ROOT`.**
   - Test: call `run_binary` with no `isolate_cwd` argument; assert the
     captured `cwd` kwarg equals `equivalence.REPO_ROOT`. This is a
     regression pin for Tier 2's corpus sweep before slice 7 changes anything.
   - Production: none yet (this documents current behavior against the new
     parameter's default).

7. **`run_binary(..., isolate_cwd=True)` runs inside a disposable directory,
   and the binary path is resolved to absolute before the cwd changes.**
   - `subprocess.run` resolves a relative `args[0]` against the *child's*
     `cwd`, not the parent's. `check_file` builds `rust_out`/`self_out` from
     `outdir = Path(args.output_dir)`, which is relative by default
     (`"equivalence.out"`); today that only works because `cwd=REPO_ROOT`
     matches where the compiler wrote the file. Switching `cwd` without also
     resolving the path would turn a relative `--output-dir` into a
     `FileNotFoundError` the moment `isolate_cwd=True` is used.
   - Test: call `run_binary("relative/path/to/bin", ..., isolate_cwd=True)`;
     assert the captured `args[0]` in the `subprocess.run` mock is the
     *absolute* form of that path (resolved before the temp dir is entered),
     and that `cwd` is not `equivalence.REPO_ROOT`.
   - Production: add the `isolate_cwd` parameter; resolve
     `path = Path(path).resolve()` unconditionally (cheap, harmless when
     `isolate_cwd=False` too) before building the `subprocess.run` call, and
     wrap the call in `with candidate_isolation.disposable_workdir() as d:`
     when `isolate_cwd=True`.

8. **`compare_runtime` forwards `isolate_cwd` to every `run_binary` call.**
   - Test: reuse the existing `CompareRuntimeTest.run_with` helper
     (`mock.patch.object(equivalence, "run_binary", side_effect=...)`),
     call `compare_runtime(..., isolate_cwd=True)`, and assert every
     recorded call's kwargs include `isolate_cwd=True` (via
     `mock.call_args_list`). Add a companion test that the default
     (`isolate_cwd` omitted) forwards `False`.
   - Production: add the parameter to `compare_runtime` and pass it through
     to each of the three `run_binary(...)` calls.

9. **`check_file` isolates the candidate's cwd exactly when directives are
   not honoured.**
   - Test: two cases driving `check_file` (mocking `run_compiler` to report
     two successful builds, per the existing `fake_run_compiler` pattern at
     `test_equivalence.py:591`/`785`) with `mock.patch.object(equivalence,
     "compare_runtime")` standing in for the runtime stage:
     - `honour_directives=True` (default) → `compare_runtime` called with
       `isolate_cwd=False`.
     - `honour_directives=False` → `compare_runtime` called with
       `isolate_cwd=True`.
   - Production: `isolate_cwd=not honour_directives` at the `compare_runtime`
     call site in `check_file`.

10. **`run_json` passes a scrubbed environment.**
    - Test (`scripts/test_verifier_runtime.py`): same pattern as slice 4,
      against `verifier_runtime.run_json`. Pin `cwd=verifier_runtime.REPO_ROOT`
      unchanged.
    - Production: add `env=` to `run_json`.

11. **`run_debug_binary` passes a scrubbed environment and always runs inside
    a disposable directory, with the same path-resolution fix as slice 7.**
    - Same relative-path hazard as slice 7: `verifier_runtime.py`'s own
      `main()` builds `exe = outdir / (stem + "_dbg")` from a relative
      `--output-dir` default (`"verifier-runtime.out"`); with `cwd` no longer
      `REPO_ROOT`, an unresolved relative `exe` would make the standalone
      sweep's own default invocation raise `FileNotFoundError` (uncaught —
      only `TimeoutExpired` is handled today).
    - Test: same pattern as slice 7 — assert `args[0]` is resolved to
      absolute and `cwd` is not `REPO_ROOT`.
    - Production: add `env=`, resolve `path = Path(path).resolve()` before
      the call, and wrap in the unconditional
      `with candidate_isolation.disposable_workdir() as d:`.

12. **Existing suites stay green.**
    - Run `python3 scripts/test_equivalence.py`, `python3
      scripts/test_verifier_runtime.py`, and `python3 scripts/test_pair_review.py`
      unmodified-assertions-wise (only new tests added) to confirm no
      behavioral change leaks into the mocked higher-level tests (`confirm`,
      `confirm_both_paths`, `confirm_soundness*`, `reconcile`, ledger
      proposals, etc.), since none of those tests patches below the
      `run_compiler`/`run_binary`/`run_json`/`run_debug_binary` seam.

13. **Docs.** Add the `docs/equivalence/README.md` paragraph described in
    §2. No test applies to prose; proofread against the actual tier
    descriptions already in that file (§"Tier 2", §"Tier 3").

## 4. Verification surface

None. This change touches Python test-harness code only — no contracts, no
codegen, no C model, no ESBMC-checked properties. No `tests/run/` or
`examples/` fixtures need to grow; the risk here runs the other way (see
below): existing fixtures must **not** regress under the corpus sweep.

## 5. Risk areas

- **Relative binary paths break once `cwd` moves.** `subprocess.run` resolves
  a relative `args[0]` against the *child's* `cwd`, not the parent's. Both
  `equivalence.py check_file` and `verifier_runtime.py main()` build their
  compiled-binary paths from a relative `--output-dir` by default
  (`"equivalence.out"`, `"verifier-runtime.out"`), which only works today
  because `cwd=REPO_ROOT` happens to match where the compiler wrote the
  file. Slices 7 and 11 must resolve the binary path to absolute *before*
  entering the disposable directory, not just change `cwd` — this is called
  out explicitly in both slices because it is the failure mode a literal
  reading of "add a disposable cwd" would ship, and it would surface as an
  uncaught `FileNotFoundError` (neither call site catches anything but
  `TimeoutExpired`), not a clean test failure.
- **The one real correctness risk: silently breaking Tier 2 corpus fixtures
  that assume `cwd=REPO_ROOT`.** Confirmed during planning that
  `tests/run/fs_read_line_basic.vow`, `tests/run/fs_read_line_status.vow`,
  `tests/run/fs_read_line_pin_to_root.vow` (all open
  `"tests/fixtures/fs_stream_lines.txt"`/`"...fs_stream_missing.txt"`), and
  `tests/multi/vmod_region_roundtrip/main.vow` (opens
  `"tests/fixtures/issue197_sample.vmod.hex"`) all reference paths relative to
  `REPO_ROOT` — exactly why `run_binary`'s cwd change is gated behind
  `isolate_cwd`/`honour_directives` rather than applied unconditionally. If
  slice 6 (the "stays at REPO_ROOT by default" regression pin) is skipped,
  this is the failure mode a later PR would hit as a nightly
  `equivalence.yml` regression, not a local test failure — the corpus sweep
  is nightly-only (`workflow_dispatch`/cron), not on the PR path.
- **Re-verify the `run_debug_binary` unconditional-isolation call before
  implementing.** The plan found no `fs_open`/`fs_read`/`fs_write`/
  `process_run` use under `tests/verify/`, `tests/verify-fail/`, or
  `examples/` that depends on `cwd=REPO_ROOT` (examples/sat/main.vow and
  examples/streaming_file/streaming_count.vow both key off `argv`/stdin, not
  a hardcoded relative path, and `run_debug_binary` never passes `argv`
  anyway). Re-run the grep at implementation time in case a fixture was added
  since this plan was written; if one now depends on `cwd=REPO_ROOT`, fall
  back to the same `isolate_cwd`-style gate used for `run_binary` instead of
  forcing it unconditionally.
- **`scrubbed_env`'s pattern is deliberately narrow.** It matches the two
  shapes the issue names (`*_API_KEY`, `*_TOKEN`) and nothing broader (no
  `*_SECRET`, no `*_PASSWORD`). This is a scope choice, not an oversight — see
  Out of Scope.
- **Not a binary-fixed-point or codegen risk.** Nothing here touches
  `compiler/`, `crates/vow-codegen`, `vow-clif-shim`, or IR — the change
  cannot affect `parse → print → parse` idempotency or the self-hosted
  compiler's binary fixed point.
- **`cargo clippy --all -- -D warnings`** does not apply (no Rust changes).
  **Ruff does apply** (`.pre-commit-config.yaml` pins `ruff-pre-commit
  v0.15.14` over the whole repo, including `scripts/*.py`); run `uvx
  ruff@0.15.14 check scripts/candidate_isolation.py scripts/equivalence.py
  scripts/verifier_runtime.py scripts/test_candidate_isolation.py` (matching
  the pinned-ruff verification note from prior sessions — a newer local ruff
  can report rules CI does not run) before considering the slice done.
- **Mock-level tests only; no live subprocess isolation test.** Slices 4–11
  mock `subprocess.run`, so they prove the *arguments* passed are correct,
  not that the OS actually isolates the process. That is intentional and
  matches the existing test style in both files (neither script has a single
  test today that lets a real subprocess run). A future integration-level
  check (e.g., actually attempting `fs_write` to a checkout file from a
  candidate program and asserting it lands in the disposable dir instead)
  would belong to the "full isolation" follow-up in Out of Scope, not this
  slice.
- **`scripts/test_scratch_cleanup.py` scope checked, no collision.** It lints
  shell scripts' `trap ... EXIT|INT|TERM|HUP` handling around `mktemp -d`
  scratch trees; it does not inspect Python `tempfile.TemporaryDirectory`
  usage (`pair_review.py` already uses one, unlisted). `disposable_workdir`
  needs no entry there.

## 6. Out of scope (deliberately deferred)

- **Full isolation** — restricted filesystem (allowlist/read-only bind
  mounts), no network, process-tree/CPU/file-size/descriptor limits. The
  issue itself calls this out as needing "its own review" because it changes
  corpus-fixture behavior more invasively than a cwd change (e.g., a fixture
  that legitimately needs to read multiple repo-relative fixture files would
  need an explicit allowlist, not just a relocated cwd). Tracked as
  follow-up work, not folded into this chore.
- **Broadening `scrubbed_env`'s credential pattern** beyond `*_API_KEY`/
  `*_TOKEN` (e.g. `*_SECRET`, `*_PASSWORD`, cloud-provider-specific variable
  names). Left for a future pass if a concrete credential shape shows up that
  this misses; adding speculative patterns now is exactly the kind of
  unrequested scope creep the issue's "smallest credible first" framing asks
  to avoid.
- **Passing a minimal/allowlisted environment** instead of a scrubbed one.
  The issue explicitly rejects this: `PATH` and `VOW_CACHE_DIR` are
  load-bearing for ESBMC/linker discovery and the shared compile-object
  cache, and enumerating everything else a compiler invocation might
  legitimately need is a much larger, riskier change than subtracting a
  narrow credential-shaped set.
- **Wiring `scripts/verifier_runtime.py`'s standalone corpus sweep into CI.**
  It currently has no workflow of its own (only its unit tests run in CI);
  that gap is unrelated to isolation and not part of this issue.
- **`check_precision`'s `verify --replay-cex` invocation.** This makes the
  *compiler* (via `run_json`) execute the candidate as its own child process
  during counterexample replay — a fifth execution site the issue didn't
  name. It inherits `run_json`'s scrubbed environment but not a disposable
  cwd (that child still runs under the compiler's `cwd=REPO_ROOT`). Not
  reachable from Tier 3 today — `pair_review.py` imports only
  `check_soundness`, never `check_precision` — so it is left alone here; note
  it for whoever later wires precision-direction checks into pair review.
- **Any change to `scripts/pair_review.py`.** As established in §2, it needs
  none — the isolation lands entirely inside the shared execution functions
  it already calls.
- **Formatting/refactoring passes** over `scripts/equivalence.py` or
  `scripts/verifier_runtime.py` beyond the specific lines this issue touches.
