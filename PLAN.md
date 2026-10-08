# Plan: pin Bitwuzla in a checksummed install action (#1402)

## Goal
Add `.github/actions/install-bitwuzla` (version + per-platform SHA-256 pinned in that one file), call it from every workflow that will run native verification, and document that `bitwuzla` must be on PATH. Part of epic #1398 (ADR 2026-10-08-1430: `tool_not_found` = Bitwuzla missing from PATH or off-pin).

## Assumptions
- Separate action, not folded into `install-esbmc`: ESBMC is deleted in P6 (#1398); a standalone action deletes cleanly and has its own cache key. Consumers add a second `uses:` step. (best guess)
- Pin Bitwuzla **0.9.1** (latest tagged release; `latest` is a rolling tag and must not be used). Hashes below were computed locally with `sha256sum` on the downloaded archives and match GitHub's asset digests:
  - Linux x86_64 `Bitwuzla-Linux-x86_64-static.zip`: `057f1546ae2068df57beb178f3eeab1678f0e5f0c378787a05b7bb294617c9c6`
  - Linux arm64 `Bitwuzla-Linux-arm64-static.zip`: `f2e9f77b5f5c5d6a7bbb2c0fbea096952f61f9fd5d387f0a06fd235f2ec0d3a1`
  - macOS arm64 `Bitwuzla-macOS-arm64-static.zip`: `86a6fb1af2b7cdaf3f7807662ab679088113bbf3e55d243597f98d826bcb7511`
  - Download URL: `https://github.com/bitwuzla/bitwuzla/releases/download/0.9.1/<archive>`. Implementer must re-download and re-hash before committing (do not trust this plan blindly).
- Archive layout (verified for 0.9.1): `Bitwuzla-<plat>-static/bin/bitwuzla` (+ lib/, include/); `zipinfo` confirms the exec bit is preserved (`-rwxr-xr-x`). Locate the binary with `find "$HOME/bitwuzla" -type f -path '*/bin/bitwuzla'` (no perm filter), then `chmod +x`, rather than a hardcoded path.
- Runtime deps (verified by `ldd` / Mach-O load commands): Linux binary needs libgmp.so.10, libmpfr.so.6, libstdc++ (present on GitHub ubuntu-latest and ubuntu-24.04-arm images via gcc deps). **macOS binary hard-links `/opt/homebrew/opt/gmp/lib/libgmp.10.dylib` and `/opt/homebrew/opt/mpfr/lib/libmpfr.6.dylib`** -> action must `brew install gmp mpfr` on macOS (same pattern as install-esbmc's macOS step; do not rely on install-esbmc having run first).
- Pin location: epic #1398 D2 says only "pinned with SHA-256, required on `PATH`" and names no file (unlike the seed's `scripts/seed.toml`); no sibling issue specifies one. So `action.yml` is the single pin for CI. The compiler-side runtime check (binary hash, cache key) is a later P-phase issue and may derive from or move this pin; flagged under Risks.
- Which workflows get it: bootstrap.yml (3 jobs), ci.yml (job at line 71), full-test.yml, promoted-fixtures.yml, equivalence.yml, cargo-mutants.yml, release.yml (gated `if: matrix.verify`). Not arena-verify.yml: it proves a standalone C harness with ESBMC only. (best guess; a surplus install is cached and ~5 MB, a missing one breaks native verify later)
- macOS x86 is not supported (no upstream artifact); release.yml already sets `verify: false` there.
- docs/spec/*.md are NOT changed: no CLI/language behaviour changes today (spec still describes ESBMC-based `--solver`); mentioning a Bitwuzla requirement there now would contradict the current binary. Spec updates belong to the issues that land the native verifier.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `.github/actions/install-esbmc/action.yml` | template to mirror (select -> validate -> cache -> install -> macOS deps -> PATH -> verify) | all; pin header 4-24, `inputs:` 25, `runs:` 43, case arms 54-71, cache action 92 |
| `.github/actions/install-bitwuzla/action.yml` [new] | the one pin + installer | n/a |
| `scripts/test_install_esbmc_action.py` | template for the new action test | all; assertRegex block 19-24 |
| `scripts/test_install_bitwuzla_action.py` [new] | guards pin shape/platform contract | n/a |
| `scripts/test_bootstrap_workflow.py` | workflow-shape tests; `CASE_ARM` regex line 51, `test_verified_platforms_have_a_pinned_case_arm` 236-253, per-workflow `install-esbmc` asserts 105-113, 293, 359 | add Bitwuzla analogues |
| `.github/workflows/ci.yml` | add step after line 72; add unit-test step after line 93 | 71-93 |
| `.github/workflows/bootstrap.yml` | add step after each ESBMC step (75, 119, 140); update comments 105-109,127-130 only if they mislead | 74-140 |
| `.github/workflows/full-test.yml`, `promoted-fixtures.yml`, `equivalence.yml`, `cargo-mutants.yml` | add step after each ESBMC install (58-59, 43-44, 44-45, 34-35) | |
| `.github/workflows/release.yml` | add step with `if: matrix.verify` after line 96 | 92-96 |
| `README.md` | Quick Start (lines 12-28): add a "Verification prerequisites" note | 12-28 |
| `scripts/ci_docs_only.py` | no change expected: `.github/**` is not prose, so the new action already counts as code; confirm by running its tests | 76-100 |

## Steps

### 1. Create the Bitwuzla install action [new]
- **File**: `.github/actions/install-bitwuzla/action.yml`
- **Change**: composite action modelled on install-esbmc: header comment stating "single source of truth" + bump recipe (`curl` the three archives, `sha256sum`/`shasum -a 256`); inputs `version` (default `0.9.1`), `sha256` (Linux x86), `sha256-linux-arm64`, `sha256-macos` with the hashes above (use identical input names to install-esbmc: `sha256`, `sha256-macos`, `sha256-linux-arm64`). Steps: select archive by `$RUNNER_OS/$RUNNER_ARCH` (case arms indented exactly 10 spaces like install-esbmc, because `CASE_ARM` at `test_bootstrap_workflow.py:51` matches `^ {10}OS/ARCH\)$`; arms `Linux/X64`, `Linux/ARM64`, `macOS/ARM64`, wildcard errors) -> validate non-empty -> `actions/cache` (reuse the exact pinned SHA of install-esbmc line 85, path `~/bitwuzla`, key `bitwuzla-<version>-<sha256>-<os>-<arch>`) -> download with `curl --fail --proto '=https' --tlsv1.2`, verify with `sha256sum --check --strict` (fallback `shasum -a 256`), `unzip -q` into `~/bitwuzla` -> macOS: `brew install gmp mpfr` runs unconditionally (not gated on cache miss; a cached binary still needs the dylibs, as in install-esbmc) -> locate binary via `find`, `chmod +x`, append dirname to `$GITHUB_PATH` -> final step `bitwuzla --version` (fails install-time, per acceptance criterion 1). Optionally also assert the printed version equals `inputs.version` so a mislabelled archive fails loudly.
- **Reuses**: structure and cache action pin from `.github/actions/install-esbmc/action.yml:92`; temp download path must follow the existing `/tmp` pattern in the template (runner-local, not the dev workspace).

### 2. Wire the action into workflows
- **Files**: `ci.yml`, `bootstrap.yml` (x3), `full-test.yml`, `promoted-fixtures.yml`, `equivalence.yml`, `cargo-mutants.yml`, `release.yml`.
- **Change**: insert `- name: Install Bitwuzla\n  uses: ./.github/actions/install-bitwuzla` immediately after every `Install ESBMC` step; in release.yml carry `if: matrix.verify`. No version/hash inputs at call sites (pin stays in one place).

### 3. Document the requirement
- **File**: `README.md`
- **Change**: under Quick Start add a short "Verification prerequisites" paragraph: verification requires `bitwuzla` (pinned release, version and checksums in `.github/actions/install-bitwuzla/action.yml`) on `PATH` alongside ESBMC; list platforms with prebuilt archives (Linux x86_64/arm64, macOS arm64), runtime libs (gmp, mpfr; `brew install gmp mpfr` on macOS), and check with `bitwuzla --version`. Do not hardcode the version number in README -- point at the action file so the pin stays single-sourced. Write: "pinned in `.github/actions/install-bitwuzla/action.yml`", no number.

## TDD slices
1. **Red**: `scripts/test_install_bitwuzla_action.py` [new] asserts action file exists and contains the three archive names, three `[0-9a-f]{64}` defaults (regex pattern from `test_install_esbmc_action.py:19-24`), `sha256sum` + `shasum -a 256`, `bitwuzla --version`, a non-`latest` version default, and a `brew install gmp mpfr` macOS step. **Green**: step 1.
2. **Red**: in `scripts/test_bootstrap_workflow.py` add `INSTALL_BITWUZLA_ACTION` constant; (a) every job that has `install-esbmc` (explicitly excluding `arena-verify.yml`, which is ESBMC-only and not in the checked set) in bootstrap/ci/full-test/promoted-fixtures/equivalence/cargo-mutants/release also has `install-bitwuzla` (iterate `job_blocks`; release: both gated on `matrix.verify`); (b) `CASE_ARM` set of the Bitwuzla action equals `RUNNER_PLATFORM` of verified release legs, and equals the ESBMC action's arm set (so the two pins cannot diverge in platform coverage); (c) single-pin check: no `uses: ./.github/actions/install-bitwuzla` step has a `with:` block, and no file under `.github/workflows/` contains the pinned version string or any of the three hashes (read them from the action file in the test, don't hardcode). **Green**: step 2.
3. Wire `python3 scripts/test_install_bitwuzla_action.py` into `ci.yml` next to the ESBMC action test (after line 93). **Green** with step 2.
4. README step 3; add a trivial test only if a docs-mention test pattern exists (none found) -- otherwise manual check via `grep -n -i bitwuzla README.md`.
5. Refactor: if the two action tests duplicate >15 lines, leave as is (surgical; no shared helper).

## Testing / verification commands (run separately, not `&&`-chained)
- `python3 scripts/test_install_bitwuzla_action.py`
- `python3 scripts/test_install_esbmc_action.py`
- `python3 scripts/test_bootstrap_workflow.py`
- `python3 scripts/test_ci_docs_only.py`
- `python3 scripts/ci_docs_only.py` sanity unaffected; workflow lint: `actionlint` if installed (workflow-lint.yml runs it in CI); `python3 -c "import yaml,sys; yaml.safe_load(open('.github/actions/install-bitwuzla/action.yml'))"` for YAML validity.
- Local dry-run of the install logic on Linux x86: download 0.9.1 archive into a fresh `mktemp -d`, `sha256sum --check`, unzip, `bin/bitwuzla --version` prints `0.9.1` (already confirmed during planning).
- Real acceptance (3 platforms) is only provable in CI: the three bootstrap jobs (ubuntu-latest, macos-15, ubuntu-24.04-arm) execute the action's final `bitwuzla --version` step. Note in the PR body that macOS arm64 was not run locally; link the CI run.
- Commit/PR title (Conventional Commits, lower-case): `chore(ci): pin Bitwuzla in the install action and docs`.

## Verification surface
No contract, codegen, C-model, or `compiler/`/Rust-crate change; ESBMC proof obligations unchanged; no `tests/run/` or `examples/` fixtures grow. Dual-compiler and C-parity rules are not triggered. Bootstrap fixed point unaffected (CI-only + docs files).

## Risks
- **macOS dylib paths**: binary links `/opt/homebrew/opt/{gmp,mpfr}`; macos-15 arm64 runners have Homebrew at `/opt/homebrew`, so `brew install gmp mpfr` suffices. If the install-time `bitwuzla --version` fails there, the failure names itself (that is the point of the final step).
- **Linux libgmp/libmpfr**: assumed present on both ubuntu images. If `ldd` shows missing libs, add `sudo apt-get install -y libgmp10 libmpfr6` guarded to `runner.os == 'Linux'` (sudo on GH runners is fine; never in local runs).
- **Pin ≠ future binary-hash check**: ADR 1430 says the native verifier checks the *binary's* SHA-256 at runtime. This action pins *archive* hashes. Per-binary hashes (extracted `bin/bitwuzla`) belong to the verifier-driver issue (P4); recommend that issue read the pin from this action's file or a derived `scripts/` file rather than re-pinning. Not done here to avoid inventing format. Record as a follow-up comment on #1398.
- **Cache key**: includes version + sha256 + OS + arch, so a rotation re-downloads; `latest` tag deliberately not used because its asset hashes change.
- **Mutable workflow churn**: 9 edited workflow files; mitigated by the workflow-shape tests in slice 2. `arena-verify.yml` intentionally untouched (its path gate in `scripts/ci_docs_only.py` is ESBMC-specific).
- **clippy gate**: no Rust touched.
- **Third-party download supply chain**: HTTPS-only, TLS>=1.2, SHA-256 `--strict`; hashes were taken from a trusted local download and cross-checked with GitHub asset digests.

## Out of scope
- Teaching `vowc` to find/validate Bitwuzla, binary-hash pinning in the compiler, `tool_not_found` plumbing (later #1398 issues).
- Removing/reworking `install-esbmc` or the `--solver` flag; any `docs/spec/*` or `--help` regeneration.
- Intel macOS support, Windows, building Bitwuzla from source, dependabot/auto-bump automation.
- Refactoring the two install actions into a shared helper.
