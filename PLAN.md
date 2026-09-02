# Plan: #1156 — ci: verify on macOS now that ESBMC ships a macOS artifact

## 1. Problem restated

Two CI paths bootstrap the self-hosted compiler on macOS/arm64 without ever running ESBMC —
`bootstrap-macos` in `.github/workflows/bootstrap.yml` (`scripts/bootstrap.sh --no-verify`) and the
`macos/aarch64` leg of `release.yml`'s `build-package` matrix (`verify: false`) — because until
ESBMC 8.4 there was no macOS release artifact for `install-esbmc` to fetch. That stopped being true
at 8.5: `esbmc-macos.zip` exists, unpacks to the same `release/bin/esbmc` layout as the Linux
archive, is a real arm64 Mach-O binary, and bundles Boolector (the solver the arena proof requires).
`install-esbmc` is centralized (#1134) and always downloads `esbmc-linux.zip` regardless of
`runner.os`, so today it silently fetches the wrong archive if pointed at a macOS runner. This plan
makes `install-esbmc` OS-aware and turns both macOS legs into real ESBMC verification, closing the
gap the issue describes: macOS proves only a byte-identical fixed point today, not that the
self-hosted compiler's contracts actually hold there.

## 2. Files to touch

No `crates/` or `compiler/` changes — this is CI-only infrastructure, not a language, contract, or
CLI change, so **no `docs/spec/*.md` update is required** (nothing here is user-facing Vow syntax,
semantics, builtins, or CLI surface).

| File | Change |
|---|---|
| `.github/actions/install-esbmc/action.yml` | Select archive + checksum by `$RUNNER_OS`/`$RUNNER_ARCH`; new `sha256-macos` input; portable checksum tool; OS/arch-scoped cache key |
| `.github/workflows/bootstrap.yml` | `bootstrap-macos`: install ESBMC, run `scripts/bootstrap.sh --stage3-no-verify` (not a bare drop of `--no-verify`); refresh the job comment |
| `.github/workflows/release.yml` | `build-package` matrix: `macos/aarch64` → `verify: true`; fix the retired `macos-13` runner label → `macos-15-intel`; refresh matrix/step comments |
| `scripts/bootstrap.sh` | One-line usage-text fix: drop the now-stale "(e.g. macOS)" example on `--no-verify` |
| `scripts/test_bootstrap_workflow.py` | Extend `BootstrapWorkflowTest` with a macOS-verifies assertion; add a `ReleaseWorkflowTest` class; update module docstring |
| `scripts/test_install_esbmc_action.py` (new) | Small stdlib-only structural test for the action's OS-selection logic |
| `.github/workflows/ci.yml` | New step invoking `scripts/test_install_esbmc_action.py`, alongside the existing workflow-shape test steps |

## 3. TDD slices

Each slice is red (test fails against current file) → green (minimal edit) → refactor if needed.
These are YAML/shell infrastructure, not Rust/Vow, so "tests" are this repo's existing convention:
stdlib-only `re`-based structural assertions over the workflow/action text (see
`scripts/test_bootstrap_workflow.py`'s own docstring for why — no PyYAML dependency).

1. **install-esbmc selects the right archive per platform.**
   - Test (new `scripts/test_install_esbmc_action.py`): assert `action.yml` contains both
     `esbmc-linux.zip` and `esbmc-macos.zip`; assert a `sha256-macos` input exists with a 64-char
     lowercase-hex default; assert the literal `ubuntu-24.04` no longer appears in the cache key;
     assert both `sha256sum` and `shasum -a 256` appear (portable checksum fallback).
   - Production: add `sha256-macos` input (default `a5a9b4443775c346ed14c9a20c36e459c460f61ebeaa33f89daa14f9ae4d786d`,
     per the issue body's verified checksum). Add a "Select ESBMC archive" step before `Cache ESBMC`
     that branches on `case "$RUNNER_OS/$RUNNER_ARCH"` (both are runner-provided env vars — prefer
     this over a two-way if/else so the linux-aarch64 follow-up is a one-case addition later) and
     writes `zip`/`sha256` to `$GITHUB_OUTPUT` (not `$GITHUB_ENV`, which would leak into the caller
     job). Fail loudly for `Linux/ARM64` ("no pin for this platform yet") rather than silently
     falling through to the x86_64 archive. Update `Cache ESBMC`'s key to
     `esbmc-${{ inputs.version }}-${{ steps.select.outputs.sha256 }}-${{ runner.os }}-${{ runner.arch }}`.
     Update `Install ESBMC` to read `steps.select.outputs.zip`/`.sha256` instead of the hardcoded
     `esbmc-linux.zip` / `inputs.sha256`. Make the checksum step portable:
     `command -v sha256sum` with a `shasum -a 256 --check --strict` fallback (macOS ships `shasum`
     natively; GNU coreutils' presence/PATH-prefixing on GitHub's macOS image is not something to
     assume). Move "Validate inputs" to validate the *resolved* sha (post-selection) is non-empty,
     not just the raw `inputs.sha256`. Update the top-of-file bump-procedure comment to mention the
     macOS archive alongside the Linux one.
   - `Add ESBMC to PATH` and `Verify ESBMC runs` steps: unchanged (issue confirms the `find`/`-perm`
     PATH resolution needs no change — BSD `find` on macOS supports the same `-perm -u+x` syntax).

2. **`bootstrap-macos` actually verifies.**
   - Test (`scripts/test_bootstrap_workflow.py`): add `test_macos_bootstrap_verifies_with_esbmc`,
     mirroring the existing `test_linux_bootstrap_verifies_with_esbmc` — assert `install-esbmc`
     appears in the `bootstrap-macos` job block, assert `--stage3-no-verify` appears, assert
     `bootstrap.sh --no-verify` does not. Update `test_covers_both_platforms` / the module docstring
     ("the Linux leg still verifies" → both legs verify).
   - Production: in `bootstrap.yml`'s `bootstrap-macos` job, add an `Install ESBMC` step
     (`uses: ./.github/actions/install-esbmc`) before the bootstrap step, and change
     `scripts/bootstrap.sh --no-verify` to `scripts/bootstrap.sh --stage3-no-verify` — matching the
     Linux `bootstrap` job's shape exactly (Stages 1–2 verify, Stage 3 skips to roughly halve wall
     time; the SHA-256 fixed-point check stays meaningful either way since verification doesn't
     touch codegen). Rewrite the job's explanatory comment: it currently says `--no-verify` "is no
     longer forced on us... Wiring it up is #1156" — replace with a comment describing what's
     actually verified now.

3. **`release.yml`'s macOS/aarch64 leg verifies; the dead x86_64 runner label is fixed.**
   - Test: add `ReleaseWorkflowTest` to `scripts/test_bootstrap_workflow.py` (that file already spans
     four workflow files via shared `header()`/`job_blocks()`/`crons()` helpers — release.yml is a
     fifth). Parse each matrix entry with one regex per `- os: X / arch: Y / runner: Z / verify: W`
     block (instruct any future editor to keep comments *between* entries, not inside one, so the
     regex stays robust) and assert: `macos`/`aarch64` → `verify: true`; `macos`/`x86_64` → runner
     `macos-15-intel` and `verify: false`; `linux`/`x86_64` → `verify: true` (unchanged, guards
     against accidental regression).
   - Production: flip `verify: false` → `verify: true` for the `macos`/`aarch64` entry only. Fix the
     `macos`/`x86_64` entry's `runner: macos-13` → `runner: macos-15-intel` — **`macos-13` was fully
     retired by GitHub on 2025-12-04** (confirmed via GitHub's changelog), so this leg has been
     failing outright since then, and because `publish` needs the whole `build-package` matrix (no
     `continue-on-error` on any leg), **every scheduled release has likely been blocked since
     December 2025**, independent of this issue. Fixing it is a one-line, evidence-backed drive-by
     directly adjacent to the line already being edited, not scope creep — but flag it prominently in
     the PR body as a separate, named fix. Update the stale matrix comment ("The pinned ESBMC zip is
     Ubuntu x86_64-only...") and the `Install ESBMC` step's `if: matrix.verify` comment, both of which
     describe the pre-change state.
   - Implementation-stage sanity checks before committing this slice: `grep -rn macos-13` across the
     repo (confirm no other stale references); optionally `gh run list --workflow=release.yml --limit 5`
     to capture concrete evidence of the blocked-release history for the PR body.

4. **Wire the new test into CI.**
   - Test: none (this is the wiring itself) — but confirm `python3 scripts/test_install_esbmc_action.py`
     exits 0 locally before adding the step.
   - Production: add a step to `ci.yml`'s `build-and-test` job, alongside the other workflow-shape
     unit-test steps: `- name: Install-ESBMC action unit tests` /
     `run: python3 scripts/test_install_esbmc_action.py`.

## 4. Verification surface

This issue does not add, change, or weaken any Vow contract, and touches no codegen or C-model
path — `compiler/*.vow`'s contracts are unchanged. What changes is *where* ESBMC runs, not *what* it
proves: the same per-function verification that already runs on Linux (`bootstrap`, `ci.yml`
`build-and-test`, `full-test.yml`) now also runs for real on macOS/arm64 via `bootstrap-macos` and
`release.yml`'s `macos/aarch64` leg. No new properties, no new `tests/run/` or `examples/` fixtures
are required.

The actual verification risk is platform/solver-specific, not contract-specific: arm64 Boolector
could in principle diverge from x86_64 Boolector on some formula (different floating-point behavior,
different default tactics, etc.). That's exactly the gap the issue wants closed, and the only way to
find such a divergence is to let `bootstrap-macos` (blocking, no `continue-on-error`, unchanged from
today) actually run ESBMC on push to `main` and nightly.

`vow-runtime/verify/arena.c` (the standalone Boolector-pinned proof with the documented 2 GB
`ulimit -v` headroom: 508 MiB @8.1, 290 MiB @8.3, 585 MiB @8.5, all Linux-measured) is **not** on
any path this plan changes — `arena-verify.yml` and `full-test.yml` stay Ubuntu-only, and
`scripts/bootstrap.sh` never calls `scripts/verify_arena.sh`. Extending that proof to macOS is
deliberately out of scope here (see §6).

## 5. Risk areas

- **Binary fixed point (`compiler/` codegen ordering, `BTreeMap`/stack-slot layout):** unaffected.
  ESBMC verification runs after codegen and does not feed back into it — this is the same invariant
  the existing `--stage3-no-verify` / `--no-verify` comments already assert for Linux.
- **`parse → print → parse` idempotency:** unaffected; no parser/printer changes.
- **`cargo clippy --all -- -D warnings` / `cargo fmt --all -- --check`:** unaffected; no Rust source
  changes.
- **`actionlint`/`zizmor` (`.github/workflows/workflow-lint.yml`):** the new `case` block in
  `install-esbmc/action.yml`'s `run:` step must pass actionlint's embedded shellcheck pass — keep
  `${{ }}` expressions out of `run:` bodies and pass them through `env:`, matching every existing
  step in that file. Run `./actionlint -color` (or open the PR and let `workflow-lint.yml` gate it)
  before considering the slice done.
- **macOS bootstrap wall-clock:** `--verify-jobs 1` (already unconditional in `bootstrap.sh` once
  `--no-verify` is off) serializes ESBMC calls; macOS verification was previously free (skipped
  entirely), so `bootstrap-macos`'s wall-clock will grow substantially — likely toward, but the
  90-minute job timeout should hold since it already matches the Linux `bootstrap` job's ceiling
  under the same `--verify-jobs 1` constraint. Watch the first few real runs; if it blows the
  timeout for legitimate (non-hanging) reasons, that's a follow-up to raise the ceiling, not to
  revert verification.
- **`release.yml`'s `publish` gate:** `publish` `needs: [plan, build-package]` with no
  `continue-on-error` per matrix leg, so a red `macos/aarch64` verify run now blocks the *entire*
  release, not just that platform's package. This is the correct fail-closed behavior per the
  issue's intent (an arm64-only solver regression should stop a release), but it's a real behavior
  change from today (where that leg was `--no-verify` and effectively couldn't fail on verification
  grounds). Rollback if this proves too disruptive: re-add `verify: false` to the `macos/aarch64`
  entry alone, filing a follow-up issue with the specific failure — do not touch the Linux legs.
- **Darwin `ulimit -v` semantics are unmeasured** — noted here only because it's the reason the arena
  macOS leg is deferred rather than bundled in (§6), not because this plan's changes exercise it.
  None of the three production edits above add or rely on a `ulimit -v` call on macOS.
- **Checksum tool portability:** the `shasum -a 256` fallback path is new and untested against a real
  macOS runner from this sandbox. Low risk (both tools are POSIX-adjacent and widely available), but
  the implementation stage should treat the first real `bootstrap-macos`/`release.yml` run as the
  actual confirmation, not just the local `python3 scripts/test_install_esbmc_action.py` pass.

## 6. Out of scope (this PR) — explicit follow-ups

- **Item 4 from the issue: extending `vow-runtime/verify/arena.c` verification to macOS.** Not
  wired into any path this PR touches (see §4). Bundling it would mean guessing at macOS's
  `ulimit -v`/`RLIMIT_AS` behavior — Darwin's baseline per-process virtual-address-space footprint
  (dyld shared cache, guard pages) is structurally different from Linux's, and the existing 2 GB cap
  in `scripts/verify_arena.sh` is Linux-tuned. That needs its own PR where a red run is attributable
  to this specific, isolated change rather than mixed in with the bootstrap-macos wiring. File a
  follow-up issue for it.
- **The "Bonus" from the issue: `linux/aarch64` (`ubuntu-24.04-arm`) verification in `release.yml`.**
  `esbmc-linux-armv8.zip` exists per the issue body, but no checksum for it was provided or
  independently verified (unlike the macOS archive, whose sha256 the issue body states was already
  confirmed by download). Pinning an unverified checksum would violate the same integrity bar the
  macOS pin meets. File a follow-up issue; the `case "$RUNNER_OS/$RUNNER_ARCH"` structure this PR
  introduces in `install-esbmc` makes that a one-case addition once a checksum is in hand.
- **Any `docs/spec/*.md` update.** Confirmed not required — nothing here changes Vow syntax,
  semantics, builtins, effects, or CLI flags.
- **Raising `bootstrap-macos`'s 90-minute timeout** preemptively. Only do this reactively if real
  runs show it's needed (see Risk areas).
- **Parity steps Linux's `bootstrap` job has that macOS's does not** (the tier-1 compiler test-suite
  comparison, `verify_eval.py --verifier build/vowc`) — both already exist as Linux-only per prior
  design decisions unrelated to this issue; not extending them to macOS here.

## Exit actions for the implementation stage (not part of this planning commit)

- Post `gh issue comment 1156` explaining the item-4 and linux-aarch64-bonus deferral rationale
  above, so the reasoning isn't lost when the orchestrator closes #1156 on merge.
- File the two follow-up issues (arena-on-macOS; linux/aarch64 verification) via `gh issue create`,
  and link them from the PR body.
- Name the `macos-13` → `macos-15-intel` fix explicitly in the PR body as a separate, evidence-backed
  fix (retired 2025-12-04), not silently folded into the verification-flip description.
