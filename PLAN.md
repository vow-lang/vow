# Plan: #1296 — Tier-2 `cargo build` corrupts shared `target/` under the symlink fast path

## 1. Problem restated

`vowc mutants run`'s default Tier-2 oracle is `scripts/full_test.sh`, whose `setup_compilers()`
(`scripts/full_test.sh:76-82`) unconditionally runs `cargo build --all --release` before every
escalated mutant. `vowc mutants` only ever mutates `*.vow` source under `--root` (op-flip,
const-flip, body-replace, contract-weaken all operate on Vow tokens — see `docs/mutants.md`), so the
Rust bootstrap compiler's source is invariant across every mutant in a run; the Tier-2 cargo rebuild
is therefore always redundant work, and when the documented local speedup is in play (`docs/mutants.md`
"Worktree mode", option (b): symlink `--workdir`'s `target/` from the real repo so Tier-1 oracle
`scripts/bootstrap.sh --skip-cargo` doesn't need to rebuild the Rust compiler per mutant), that
redundant rebuild is actively destructive — `cargo build --all --release`, invoked from inside the
throwaway worktree, writes through the symlink into the real repo's shared `target/`, and
`vow-linker::cargo_target_dir()` (`vow-linker/src/lib.rs:157-159`) bakes the worktree's
`CARGO_MANIFEST_DIR`-derived absolute path into the rebuilt artifacts as a runtime fallback. Once
`vowc mutants run` tears the worktree down, that fallback path is dangling, and the shared `target/`
is left in a state that can misbehave on the next unrelated `vow build`/`bootstrap.sh` invocation —
silently, since the corrupting mutant run itself still reports a normal verdict. The fix is to stop
Tier 2 from ever re-running `cargo build --all --release` when invoked by `vowc mutants`, since
nothing about a Vow-source mutation run ever requires it.

## 2. Files to touch

**Hand-edited:**

- `scripts/full_test.sh` — confirmed via `grep -n 'cargo\|bootstrap\.sh' scripts/full_test.sh` that
  the Section 0 setup step is the *only* place in this script, or in any of the sub-harnesses it
  shells out to (`tests/bootstrap/tests.sh`, `tests/esbmc-path-cache/tests.sh`,
  `tests/full_test_bootstrap/tests.sh`, `tests/install_toolchain/tests.sh`,
  `tests/measure_bootstrap_rss/tests.sh`, `tests/mutants/tests.sh`), that invokes `cargo` at all —
  `tests/bootstrap/tests.sh` already calls `scripts/bootstrap.sh --skip-cargo`, and
  `tests/mutants/tests.sh`'s one "cargo" hit is the unrelated string `"cargo-mutants format"` in a
  test label. So gating this one call site closes the issue completely; there is no second rebuild
  trigger hiding elsewhere on the Tier-2 path. The setup step (currently lines 76-82) gains a
  `VOW_FULL_TEST_SKIP_CARGO` env-var gate around the `cargo build --all --release` call, mirroring
  `scripts/bootstrap.sh`'s existing `--skip-cargo` flag semantics but as an env var, consistent with
  this script's existing `VOW_FULL_TEST_BOOTSTRAP_ONLY` / `VOW_FULL_TEST_PROMOTED_ONLY` toggles (no
  `getopts`/arg-parsing loop exists in this script today — env vars are the established convention
  here). When `VOW_FULL_TEST_SKIP_CARGO=1` and `$RUST` is not executable, fail loudly
  (`echo ... >&2; exit 1`) instead of letting the step fall through to a confusing downstream error —
  without this, a missing `./target/release/vow` under the skip flag dies with exit 127 and gets
  silently misclassified as a Tier-2 "caught" verdict for the wrong reason. The top-level
  `RUST="./target/release/vow"` (line 15) becomes `RUST="${VOW_FULL_TEST_RUST_BIN:-./target/release/vow}"`
  so tests can redirect it to a fixture binary — a **new, distinct** env var name, deliberately not
  reusing the existing `VOW_FULL_TEST_RUST` (which already has an unrelated meaning as a required
  argument inside the `VOW_FULL_TEST_BOOTSTRAP_ONLY` path); conflating the two would let a leftover
  exported `VOW_FULL_TEST_RUST` from one test session silently redirect an unrelated real run. A new
  `VOW_FULL_TEST_SETUP_ONLY=1` early-exit check is added **immediately after the existing
  `setup_compilers` call** (currently line 498), before the `VOW_FULL_TEST_PROMOTED_ONLY` gate — not
  as a new parallel block mirroring `VOW_FULL_TEST_BOOTSTRAP_ONLY` further up the file, because that
  guard runs *before* Section 0 and would need its own call to the setup step, which would push the
  literal-substring count in `scripts/test_bootstrap_workflow.py::test_compiler_setup_is_a_reusable_step`
  (`self.assertEqual(2, self.script.count("setup_compilers"))`) from 2 to 3 and fail that test. Placing
  the new check right after the existing call adds no new occurrence of that identifier and leaves
  `test_promoted_only_route_stops_before_the_complete_suite`'s forward-searching index assertions
  intact (it only asserts relative ordering, which is preserved). For the same reason, no new comment
  near this check should mention the setup-step function's name literally.
- `compiler/mutants_main.vow` — extract the inlined Tier-2 default (currently
  `let tier2_cmd: String = if tier2_in.len() == 0 { String::from("scripts/full_test.sh") } else { tier2_in };`
  at line 479-480) into a named function `default_tier2_cmd() -> String` returning
  `"VOW_FULL_TEST_SKIP_CARGO=1 scripts/full_test.sh"`. The Tier-1 default
  (`scripts/bootstrap.sh --skip-cargo`, line 478) is already skip-cargo-safe and does not change.
- `docs/mutants.md` — the `--tier2-cmd` row in the flags table (line 24) and the "Worktree mode"
  Caveats bullet (lines 36-38) get updated to state the new default and explain why: Tier 2 never
  needs to rebuild the Rust compiler for a Vow-source mutation run, and a custom `--tier2-cmd`
  override must preserve this (or avoid `cargo build --all --release` some other way) if it's going
  to be used with the symlinked-`target/` fast path.
- `docs/spec/cli.md` — the `--tier2-cmd` row in the `vow mutants` flags table (line 199) gets the
  same default-value update, per this repo's rule that a CLI-surface change must be reflected in
  `docs/spec/*.md`.
- `CLAUDE.md` — the "Mutation Testing" section's one-line description of the tiered oracle (line 354,
  `runs a tiered oracle (`scripts/bootstrap.sh --skip-cargo` then `scripts/full_test.sh`)`) gets the
  same default-value update so project guidance doesn't go stale relative to the actual default.

**Regenerated, not hand-edited** (per `docs/spec/cli.md`'s own rule — run
`uv run python scripts/generate_help.py` after the `docs/spec/cli.md` edit above, then rebuild both
compilers):

- `compiler/main.vow` (the `// GENERATE:SKILL_*` block containing the `vow mutants` help text)
- `vow/src/skill.rs` (same generated block, Rust side)
- `skills/vow/reference/cli.md`

**Not touched:** `vow/src/main.rs` (the Rust compiler explicitly refuses `mutants` and points to
`build/vowc` — confirmed at `vow/src/main.rs:1010-1011` — so this is a self-hosted-only change; there
is no Rust-side mutants implementation to keep in parity with per the "modify both compilers" rule,
because that rule is about language-semantics drift between the two compilers, and `mutants` is a
deliberate, pre-existing, documented exception to having a Rust counterpart at all). `vow-linker`'s
`cargo_target_dir()` is also not touched — see "Out of scope" below.

## 3. TDD slices

1. **`scripts/full_test.sh`: `VOW_FULL_TEST_SKIP_CARGO` gate.**
   - Test: extend the existing `tests/full_test_bootstrap/tests.sh` (not a new file) with a second
     fixture helper and two new test functions, reusing its already-registered scratch tree
     (`$TEST_TMPDIR`, already listed in `scripts/test_scratch_cleanup.py`'s `SCRATCH_SCRIPTS`) and its
     trap/signal preamble, so no new top-level-scratch-script registration or signal-trap lint
     exposure is introduced. (A brand-new sibling script would need its own `SCRATCH_SCRIPTS` entry,
     its own EXIT/INT/TERM/HUP traps to satisfy `scripts/test_scratch_cleanup.py`'s lint, and its own
     invocation wired into `scripts/full_test.sh`'s Section 9 — all of which this file already has.)
     The new fixture adds a fake `cargo` shim placed first on `PATH` that appends a marker line to a
     trace file and exits 0 without doing real work, and a fake Rust-compiler stand-in script
     (pointed to via `VOW_FULL_TEST_RUST_BIN`) that accepts any args and exits 0. Two new test
     functions, both invoking `bash scripts/full_test.sh` with `VOW_FULL_TEST_SETUP_ONLY=1` set
     (and wrapped in `timeout` as a backstop — see below):
     - Default (`VOW_FULL_TEST_SKIP_CARGO` unset): trace file contains the fake-cargo marker.
     - `VOW_FULL_TEST_SKIP_CARGO=1`: trace file does **not** contain the fake-cargo marker, script
       output contains an explicit "skipped" message (so a silently-vanished cargo build can't pass
       by accident), and the self-hosted-build step (fake `VOW_FULL_TEST_RUST_BIN` invocation) still
       runs — proving only the cargo stage is gated, not the whole setup step.
     This is RED against current `scripts/full_test.sh`: neither `VOW_FULL_TEST_SKIP_CARGO` nor
     `VOW_FULL_TEST_SETUP_ONLY` exist yet, so the harness would ignore both and run the full ~40-minute
     suite instead of exiting right after setup. Run the RED step itself under `timeout` (e.g.
     `timeout 60 bash ...`) so a mistaken "not recognized" case can't actually block the implementer's
     terminal for 40 minutes before showing red.
   - Production code: add the `VOW_FULL_TEST_SKIP_CARGO` conditional (plus the loud failure when
     `$RUST` is missing under that flag) inside the Section 0 setup step, the
     `VOW_FULL_TEST_RUST_BIN`-overridable `RUST=` assignment, and the `VOW_FULL_TEST_SETUP_ONLY`
     early-exit check placed immediately after the existing setup call, as described in section 2.
   - Also re-run `scripts/test_bootstrap_workflow.py::FullTestPromotedGateTest::test_compiler_setup_is_a_reusable_step`
     and `test_promoted_only_route_stops_before_the_complete_suite` to confirm the placement choice in
     section 2 keeps both green (they should, per the reasoning given there, but both assert on raw
     text/ordering so this is cheap insurance before moving on).

2. **`compiler/mutants_main.vow`: `default_tier2_cmd()`.**
   - Test: new `compiler/tests/test_mutants_main_tier2_default.vow` (module `use mutants_main;`,
     `fn main() -> i32` returning 0/1 per the house `test_*.vow` convention — see
     `compiler/tests/test_json_escape.vow` for the pattern) asserting
     `default_tier2_cmd() == String::from("VOW_FULL_TEST_SKIP_CARGO=1 scripts/full_test.sh")`. RED
     because `default_tier2_cmd` does not exist yet on current `main`.
   - Production code: extract the function as described in section 2, and change the inlined
     `tier2_cmd` assignment in `run_mutants_run` to call it instead of constructing the literal
     inline.
   - No change needed to the existing `tests/mutants/tests.sh` integration suite: every existing case
     there already passes explicit `--tier1-cmd`/`--tier2-cmd` overrides (confirmed by grep — none
     rely on the default), so none of them exercise `default_tier2_cmd()` and none should change
     behavior. Deliberately **not** adding an end-to-end integration test that lets the real default
     string execute `scripts/full_test.sh` for real inside a `vowc mutants run` worktree — that would
     either take the full ~40-minute suite's wall-clock per test run, or require faking out
     `scripts/full_test.sh` inside the ephemeral `git worktree`, which isn't possible without first
     committing the fake (the worktree is checked out from a git ref, not from the working tree's
     uncommitted state). The unit test on `default_tier2_cmd()` plus slice 1's direct test of the
     setup step's env-var handling together cover the fix compositionally: (a) the tool's default
     literal contains the env-var prefix, (b) the script honors that env var by skipping
     `cargo build --all --release`, and (c) that call site is the only one on the whole Tier-2 path
     (confirmed in section 2). Confirm `vow test compiler/` (run by `.github/workflows/bootstrap.yml`
     lines 89 and 91, for both the Rust-built and self-hosted compiler) actually discovers
     `compiler/tests/*.vow` recursively before relying on it — confirmed already, so the new test
     genuinely gates CI rather than sitting dead.

3. **Docs.** No tests (prose-only): update `docs/mutants.md`, `docs/spec/cli.md`, and `CLAUDE.md` as
   described in section 2, then regenerate `compiler/main.vow` / `vow/src/skill.rs` /
   `skills/vow/reference/cli.md` via `scripts/generate_help.py` and rebuild both compilers
   (`cargo build --release -p vow`, then `scripts/bootstrap.sh --skip-cargo`) so the generated copies
   actually reflect the new default before this slice is considered done.

## 4. Verification surface

Not applicable in the ESBMC/contracts/codegen/C-model sense: this change touches no `vow { requires
/ ensures / invariant }` blocks, no codegen path, and no C emitter behavior. `default_tier2_cmd()` is
a pure, unvowed `String`-literal-returning function; `scripts/full_test.sh` is a bash harness, not
compiled Vow. No growth needed under `tests/run/` or `examples/` for the same reason. The only
"verification" in the repo-specific sense that applies here is the self-hosted compiler's own
`vow test` run picking up the new `compiler/tests/test_mutants_main_tier2_default.vow` file, and
`scripts/bootstrap.sh` successfully rebuilding `build/vowc` with the new function before
`tests/mutants/tests.sh` (which exercises the compiled `vowc mutants` binary, not the source
directly) can be trusted to reflect the change.

## 5. Risk areas

- **Binary fixed point / `compiler/*.vow` changes.** `default_tier2_cmd()` is a trivial new top-level
  function with no control-flow surprises (single `String::from(...)` return); it carries the same
  fixed-point risk as any other self-hosted source edit — none beyond "rebuild via
  `scripts/bootstrap.sh` and confirm the SHA-256 triple-test still matches," which is already part of
  normal bootstrap. No interaction with `BTreeMap`/`HashMap` ordering, stack-slot layout, or codegen
  ordering, since nothing here touches IR, lowering, or `vow-clif-shim`.
- **`parse → print → parse` idempotency.** The new function is syntactically unremarkable (a
  zero-argument function returning a string literal); no new syntax, no new printer cases needed.
- **`cargo clippy --all -- -D warnings`.** The only Rust-side change is the regenerated literal text
  in `vow/src/skill.rs` (produced by `scripts/generate_help.py`, not hand-written) — a string content
  change only, no new code paths, so no new lint surface expected.
- **Codecov patch gate (95%).** Per prior experience in this repo (see memory: re-indenting existing
  lines can make them count as new+uncovered under codecov's diff-coverage accounting), regenerating
  `vow/src/skill.rs`'s help text replaces existing string-literal lines with new ones carrying the
  updated default. These are inert documentation strings, likely not exercised by any Rust test
  (nothing asserts on the literal help text content beyond the structural `test_bootstrap_workflow.py`
  checks, which only look at the *workflow* YAML files, not `skill.rs`), so the patch-coverage gate
  may flag them as uncovered new lines. This is the same known false-positive class as before, not a
  real coverage regression — flag it in the PR description if it trips the gate rather than
  contorting the change to dodge it.
- **Residual gap, not a regression:** slice 2 proves the *default string* is correct; slice 1 proves
  `full_test.sh` *honors* `VOW_FULL_TEST_SKIP_CARGO` when set. Nothing in this plan runs a real,
  full `vowc mutants run` end-to-end against a symlinked `target/` to literally reproduce and then
  disprove the original corruption (the issue's own repro steps). That would be valuable
  defense-in-depth but is expensive (ties up a real `target/` for the duration) and risky to automate
  safely (a bug in the test could corrupt the *real* `target/` it's trying to protect). Treat it as an
  optional manual smoke test before closing the issue, not a required automated slice: after
  implementing, a developer can follow the issue's own repro recipe once by hand and confirm
  `strings target/release/vow | grep <workdir>` no longer finds the dead path, without making that
  step part of the committed test suite.

## 6. Out of scope

- **`vow-linker::cargo_target_dir()`'s `CARGO_MANIFEST_DIR` reliance** (candidate fix #3 in the issue).
  Switching it to resolve relative to the invoking binary's own location (or `$PWD`) instead of a
  compile-time macro is a real, separable hardening that would also help cases outside `vowc mutants`
  entirely (e.g. anyone else who happens to build from a path that later gets deleted or renamed).
  It touches `vow-linker/src/lib.rs`'s existing, already-tested `find_library_in_cargo_target`
  fallback path and its unit tests, which is a self-contained enough change to deserve its own PR
  rather than being bundled into this bug fix — especially since eliminating the Tier-2 cargo rebuild
  (this plan) already removes the only known trigger for the corruption, making the `cargo_target_dir()`
  fix pure defense-in-depth rather than required to close #1296. File as a follow-up issue if the
  maintainer agrees it's worth doing proactively.
- **`vowc mutants run` refusing or auto-repairing the symlinked fast path** (candidate fix #2). Moot
  once Tier 2 never invokes `cargo build --all --release` for a mutants run — there is nothing left
  to refuse or repair. Not implementing any detection/warning machinery in `mutants_main.vow` for a
  hazard this plan already removes at the source.
- **Any CLI-flag form of `--skip-cargo` for `scripts/full_test.sh`.** Chose an env var
  (`VOW_FULL_TEST_SKIP_CARGO`) to match this script's existing convention
  (`VOW_FULL_TEST_BOOTSTRAP_ONLY`, `VOW_FULL_TEST_PROMOTED_ONLY`) instead of introducing the first
  `getopts`/positional-arg parsing loop the script has ever had. Not revisiting that convention here.
- **`.github/workflows/cargo-mutants.yml`.** This is a separate, pre-existing, unrelated nightly job
  running the third-party `cargo-mutants` tool against the *Rust* workspace crates — a different tool
  from the self-hosted `vowc mutants` this issue is about (confirmed: it mutates `--workspace
  --all-targets` Rust code, not `compiler/*.vow`). No changes needed there.
- **Formatting/refactoring of `scripts/full_test.sh` or `compiler/mutants_main.vow` beyond the
  surgical edits above.** Not touching `run_promoted_run_tests`, `compare_runtime`, or any other
  unrelated section of either file.
