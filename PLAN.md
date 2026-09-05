# Plan: ci: verify Linux arm64 release packages with ESBMC (#1213)

## 1. Problem restated

`release.yml`'s `build-package` matrix has a `linux/aarch64` leg (`ubuntu-24.04-arm`) that builds
and packages the toolchain but sets `verify: false`, because `.github/actions/install-esbmc`
has no checksum pin for that platform and therefore falls back to `--no-verify` bootstrapping.
This mirrors the state macOS/aarch64 was in before #1156/#1214 (landed as commit `81dd4625`,
"ci: verify macOS arm64 builds with ESBMC"), which added a `sha256-macos` pin and a
`RUNNER_OS/RUNNER_ARCH` selector to the action. The same pattern needs to be repeated for
`Linux/ARM64`: pin a checksum for ESBMC's `esbmc-linux-armv8.zip` release asset, teach the
action to select it for that platform, and flip only the `linux/aarch64` release matrix entry
to `verify: true`.

## 2. Independent verification of the upstream artifact (done during planning)

Downloaded directly from the upstream release (not copied from the issue body) and inspected
in the workspace sandbox:

- URL: `https://github.com/esbmc/esbmc/releases/download/v8.5/esbmc-linux-armv8.zip`
- SHA-256: `5470aac77f2057f60b232c95dfe0b9b71fef4e736246a53aeb19aaa549dd37f7`
- Archive layout matches the existing Linux/macOS 8.5 layout exactly:
  `release/bin/esbmc`, `release/license/{BOOST,Z3,BOOLECTOR}_LICENSE.txt`, `release/include/esbmc.h`,
  `release/README`, `release/release-notes.txt` — the existing generic
  `find "$HOME/esbmc" -type f -name esbmc -perm -u+x` PATH-discovery step needs no changes.
- Executable: `ELF 64-bit LSB executable, ARM aarch64, ..., statically linked, ... not stripped`,
  and `esbmc --version` reports `ESBMC version 8.5.0 64-bit aarch64 linux`.
- **Statically linked** — unlike the macOS archive (dynamically linked against Homebrew z3/mpfr/
  gmp/boost/llvm@21), this platform needs **no** runtime-dependency install step. The existing
  `Install ESBMC runtime dependencies (macOS)` step is already gated on `runner.os == 'macOS'` and
  needs no change.
- Boolector bundled and functional — this matters because `vow-verify`'s solver-selection logic
  (`vow-verify/src/solver_strategy.rs`) defaults `Solver::Auto` to `Solver::Boolector` for BV
  mode, i.e. Boolector is the solver most Vow verification runs actually exercise, not an optional
  extra. Confirmed two ways, not just a `strings` grep (which can false-positive on a message like
  "Boolector not available"): `BOOLECTOR_LICENSE.txt` ships in the archive, and running
  `./release/bin/esbmc --boolector tiny.c` (a trivial `__ESBMC_assert` program) against the
  downloaded binary in `$TMPDIR` produced `VERIFICATION SUCCESSFUL` — i.e. the bundled Boolector
  actually solved a query, not merely printed a string.

These facts de-risk the implementation slice below: no new install step, no new PATH-discovery
logic, no new licensing concern — only a new case arm in the platform selector plus one new
pinned checksum input.

## 3. Files to touch

All changes are CI/tooling — no compiler, IR, codegen, or contract semantics are touched, so
nothing under `crates/`, `compiler/`, or `docs/spec/*.md` needs updating for this issue.

- `.github/actions/install-esbmc/action.yml`
  - Add a `sha256-linux-arm64` input (default: the checksum above), alongside the existing
    `sha256` (Linux x86_64) and `sha256-macos` inputs.
  - Add a `Linux/ARM64)` arm to the `case "$RUNNER_OS/$RUNNER_ARCH"` selector in the
    "Select ESBMC archive" step, mapping to `esbmc_zip=esbmc-linux-armv8.zip` and
    `esbmc_sha256="$ESBMC_SHA256_LINUX_ARM64"`.
  - Update the header comment's bump instructions (the `curl -fsSLO .../esbmc-linux.zip` /
    `esbmc-macos.zip` list and the `sha256sum`/`shasum` lines) to also mention
    `esbmc-linux-armv8.zip`, matching how #1214 added the macOS lines.
  - No changes needed to the "Install ESBMC runtime dependencies (macOS)" step (already
    `runner.os == 'macOS'`-gated) or the "Add ESBMC to PATH" step (already archive-layout-agnostic).
- `.github/workflows/release.yml`
  - Flip the `linux`/`aarch64` matrix entry's `verify: false` → `verify: true`.
  - Remove/replace the now-stale comment `# Linux aarch64 has an upstream archive but no
    checksum pin yet.` (delete it, since the condition it describes no longer holds).
- `scripts/test_install_esbmc_action.py`
  - Extend `test_supported_platforms_get_checksum_verified_archives` to also assert
    `esbmc-linux-armv8.zip` appears in the action text and that a `sha256-linux-arm64` input
    block matches the same `default: [0-9a-f]{64}` shape already asserted for `sha256-macos`.
- `scripts/test_bootstrap_workflow.py`
  - Update `ReleaseWorkflowTest.test_release_matrix_verifies_supported_platforms`: assert
    `self.matrix[("linux", "aarch64")]["verify"] == "true"` (currently unasserted/implicitly
    `false`), keeping the existing assertions for `linux/x86_64`, `macos/aarch64`, and the
    `macos/x86_64` no-verify shape unchanged.

No changes are needed to `bootstrap.yml` — that workflow only has `bootstrap` (Linux x86_64) and
`bootstrap-macos` jobs; it has no `aarch64` Linux leg, so it is out of scope here. No changes
needed to `workflow-lint.yml` (`ubuntu-24.04-arm` is already a matrix runner label today under
`verify: false`; actionlint already accepts it, unlike the `macos-15-intel` label #1214 had to
special-case).

## 4. TDD slices

CI runs these two test files with exact, non-`unittest-discover` invocations
(`.github/workflows/ci.yml`): `python3 scripts/test_bootstrap_workflow.py` and `python3
scripts/test_install_esbmc_action.py`, both from the repo root. Use those exact commands locally
so a pass locally implies a pass in CI. Both edited files are also subject to the repo-wide
`ruff-pre-commit` hook (`.pre-commit-config.yaml`, pinned `v0.15.14`) run via `pre-commit`'s
`commit-msg`/`pre-commit` hooks on commit — check with the pinned version specifically
(`uvx ruff@0.15.14 check scripts/test_bootstrap_workflow.py
scripts/test_install_esbmc_action.py` and the equivalent `ruff@0.15.14 format --check`), since a
newer local `ruff` reports rules this repo's CI never runs.

1. **Red:** extend `scripts/test_install_esbmc_action.py` to assert
   `self.assertIn("esbmc-linux-armv8.zip", text)` and a `sha256-linux-arm64` regex block (mirror
   the existing `sha256-macos` regex). Run `python3 scripts/test_install_esbmc_action.py` — fails
   because the action has no such input/string yet.
   **Green:** add the `sha256-linux-arm64` input (default `5470aac7...dd37f7`) and the
   `Linux/ARM64)` case arm (`esbmc_zip=esbmc-linux-armv8.zip`) to
   `.github/actions/install-esbmc/action.yml`. Re-run the test — passes.

2. **Red:** extend `scripts/test_bootstrap_workflow.py`'s `ReleaseWorkflowTest` to assert
   `self.matrix[("linux", "aarch64")]["verify"] == "true"`. Run
   `python3 scripts/test_bootstrap_workflow.py` — fails against current `release.yml`
   (`verify: false`).
   **Green:** flip the `linux`/`aarch64` entry in `release.yml` to `verify: true` and delete the
   stale "no checksum pin yet" comment. Re-run — passes.

3. **Refactor/doc slice:** update the header comment in `install-esbmc/action.yml` (bump
   instructions) to list all three `curl`/checksum commands. Re-run both test files in full
   (`python3 scripts/test_bootstrap_workflow.py`, `python3 scripts/test_install_esbmc_action.py`)
   to confirm no other test class in `test_bootstrap_workflow.py` (e.g. `CiWorkflowTest`,
   `BootstrapWorkflowTest`) regressed from the comment edit.

4. **Integration confidence (not part of the automated gate, documented for the implementer):**
   this repo's CI cannot execute `ubuntu-24.04-arm` from a sandboxed planning session. The archive
   was independently downloaded to `$TMPDIR` (not a hardcoded `/tmp` path) and its binary
   executed successfully under this environment's transparent aarch64 emulation — both
   `esbmc --version` and `esbmc --boolector` against a trivial assertion program produced correct
   output. That is corroborating evidence the artifact is a genuine, runnable ESBMC+Boolector
   build, not proof the GitHub Actions arm64 runner will behave identically. There is no
   `workflow_dispatch` dry-run path that exercises this: `build-package` is gated on
   `needs.plan.outputs.published == 'true'`, which is only true when `semantic-release --dry-run`
   on `main` decides a release is due — a manual dispatch on a branch will not flip that output,
   so it cannot be used as a pre-release smoke test. The first real exercise of this pin is
   therefore the next scheduled Sunday `release.yml` run (or an on-demand `workflow_dispatch` of
   `release.yml` against `main`, which goes through the same `plan` gate for real). See the risk
   note below on pre-release exercise for why this matters more here than it did for macOS.

## 5. Verification surface

No contracts, codegen, or C model are touched. Nothing under `tests/run/` or `examples/` needs
new fixtures — this is a CI configuration + its own Python meta-tests. The only "verification" in
play is ESBMC verifying the self-hosted compiler's fixed point during `scripts/bootstrap.sh`
inside the `linux/aarch64` release-package job, which is exactly what flipping `verify: true`
turns on; no new properties need to be proved beyond what the existing `linux/x86_64` and
`macos/aarch64` verified legs already prove (the bootstrap script's ESBMC invocation is
platform-agnostic).

## 6. Risk areas

- **Blast radius: no pre-release exercise of this pin, unlike macOS.** #1214 (macOS) added the
  pin to `bootstrap.yml` (a `bootstrap-macos` job that runs on every push to `main` and nightly)
  specifically so the macOS ESBMC install path is exercised well before the weekly `release.yml`
  ever depends on it. This plan deliberately does *not* extend `bootstrap.yml` with a Linux/ARM64
  leg (§7, out of scope — keeping this PR surgical to the issue's literal ask), which means the
  Linux/ARM64 ESBMC pin's first real-world exercise is the next scheduled `release.yml` run itself.
  `publish` depends on `build-package` for *all* matrix legs (`fail-fast: false` only lets sibling
  legs finish before the job set fails; it does not let `publish` proceed without them), so a
  broken Linux/ARM64 ESBMC install (bad checksum, archive-layout change upstream, a solver crash
  specific to that architecture) would block the entire weekly release across every platform, not
  just fail one leg quietly. Filed as follow-up issue #1252 ("ci: exercise Linux/ARM64 ESBMC pin
  in bootstrap.yml before release") rather than folding it into this PR; the tradeoff is explained
  in a `gh issue comment` on #1213. A reviewer who'd rather bundle #1252's work into this PR is
  welcome to override.
- **Cache key collision:** none — the `Cache ESBMC` step's key already includes
  `${{ runner.os }}-${{ runner.arch }}`, so `Linux`/`ARM64` gets a distinct cache entry
  automatically; no cache-key changes needed.
- **`RUNNER_ARCH` value mismatch:** the plan assumes GitHub's `ubuntu-24.04-arm` runner reports
  `RUNNER_ARCH=ARM64` (matching the existing `macOS/ARM64` case-arm spelling). This is GitHub's
  documented value for arm64 Linux runners; the implementer should still confirm on the first
  real CI run of the flipped leg, since a wrong value would silently fall through to the `*)`
  no-pin error arm (fail-closed, not fail-silent — acceptable risk).
- **First real network flake:** the checksum is pinned against today's upstream artifact; if
  ESBMC re-cuts the `v8.5` tag's assets (rare but not impossible upstream), the pinned SHA-256
  will correctly fail-closed rather than silently accept a different binary — this is the
  intended behavior of the existing `sha256sum --check --strict` step, not a regression to guard
  against.
- **Binary fixed point / `parse → print → parse` / clippy gate:** none of these are touched;
  this issue does not modify `crates/` or `compiler/` source.
- **Workflow YAML drift with the regex-based meta-tests:** both `scripts/test_bootstrap_workflow.py`
  and `scripts/test_install_esbmc_action.py` parse YAML with hand-rolled regexes (deliberately,
  per their own docstrings/comments, to avoid a PyYAML dependency in a fast pre-merge check).
  The new matrix entry and input block must preserve exact indentation (10/12-space entry fields
  in `release.yml`, 2/4-space input blocks in `action.yml`) or the regexes will silently stop
  matching and the "red" step in slice 1/2 above will fail for the wrong reason (parse miss, not
  behavior miss) — worth double-checking diff whitespace by eye before trusting a red result.

## 7. Out of scope (deliberately not bundled)

- No refactor of the `case "$RUNNER_OS/$RUNNER_ARCH"` selector shape, no dedup of the three
  near-identical case arms into a table/loop — three arms is still readable; a fourth platform
  someday might justify it, this issue does not.
- No change to `bootstrap.yml` in this PR (no arm64 Linux leg exists there today, and the issue
  does not ask for one) — tracked instead as follow-up issue #1252 per the blast-radius risk note
  above, since it trades surgical scope for a period where the pin is exercised for real only on
  the weekly release cadence.
- No change to `workflow-lint.yml` (the `ubuntu-24.04-arm` label is already accepted by the pinned
  actionlint version; only `macos-15-intel` needed the version bump in #1214).
- No general "add a 4th pinned platform" abstraction or documentation rewrite beyond the header
  comment's bump-instructions list.
- No changes to `scripts/package-toolchain.sh` or the smoke-test step — packaging and smoke-testing
  already run identically regardless of `matrix.verify`.
