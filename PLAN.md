# Plan: issue #1328 — Tier 1.5 for `vowc mutants`

## 1. Problem restated

`vowc mutants run`'s oracle is two-tiered: Tier 1 (`scripts/bootstrap.sh --skip-cargo`, ~cheap,
build-to-fixed-point only) and Tier 2 (`VOW_FULL_TEST_SKIP_CARGO=1 scripts/full_test.sh`, ~27-46
min, the only thing that actually exercises program *behavior*). Every mutant that compiles but
isn't a build-level regression falls straight through to the full, hours-scale Tier 2 run even
though most real kills (8 of 10 in the #1118 sample) happen inside the first ~590s of
`full_test.sh` (Sections 0 through 8c), before the two genuinely expensive, largely redundant-for-
mutation-purposes sections (`9: Bootstrap Triple Test`, which re-does what Tier 1 already
verified, and `10b: Test Subcommand`, which alone is 564s). Inserting a cheap "Tier 1.5" — the
same fast prefix, stopped before Section 9 — lets most Tier-1 survivors get a verdict in ~10
minutes instead of ~30-45, without weakening Tier 2 as the full-coverage fallback.

## 2. Files to touch

This feature is self-hosted-only: `vowc mutants` has no Rust-compiler counterpart (confirmed —
`Cargo.toml` workspace members list has no `vow-mutants` crate; `vow/src/skill.rs` only carries the
*documentation* string, generated, not an implementation). The cross-cutting "touch both
compilers" rule in `CLAUDE.md` does not apply here: there is only one compiler that implements this
command. `scripts/full_test.sh` is plain bash, not Vow source, so it's a third, independent file
with no self-hosted/Rust duality at all.

**Hand-edited:**

- `scripts/full_test.sh` — add a `VOW_FULL_TEST_TIER15_ONLY` env-var early-exit gate, inserted right
  after Section 8c's closing `echo ""` (currently line 1651) and before the
  `# ─── Section 9: Bootstrap Triple Test ───` comment (currently line 1653). This is a pure
  insertion — no existing section needs to move, merge, or be refactored into a function.
  **The gate must copy `VOW_FULL_TEST_PROMOTED_ONLY`'s shape (lines 512-520), not
  `VOW_FULL_TEST_SETUP_ONLY`'s (lines 508-510):**
  ```bash
  if [ "${VOW_FULL_TEST_TIER15_ONLY:-0}" = "1" ]; then
      summary_status=0
      print_summary || summary_status=$?
      exit "$summary_status"
  fi
  ```
  `SETUP_ONLY` exits with a bare `exit 0` because Section 0 never calls `pass`/`fail`. By Section 8c
  there is real `FAIL` state accumulated, and only `print_summary`'s `return $(( FAIL > 0 ? 1 : 0 ))`
  turns that into a nonzero exit. Copying `SETUP_ONLY`'s bare `exit 0` here would make Tier 1.5
  exit 0 unconditionally, so every mutant would "pass" Tier 1.5 regardless of outcome and fall
  through to Tier 2 every time — Tier 1.5 would add 10 minutes to every run and catch nothing. This
  is the single most consequential implementation detail in this plan.
  Confirmed the prefix this gate exposes is exactly what's wanted: `run_promoted_run_tests`
  (Section 4) and `run_promoted_error_tests` (Section 7) are *called* at lines 795 and 1367
  respectively (not just defined at 228/328) — both before line 1653, so they're included. The
  file also has a second, unrelated section labeled "Section 6: Perfetto Trace" at line ~1918 (a
  pre-existing numbering duplicate with "Section 6: Multi-Module" at line 1275, not introduced by
  this change) — that one sits near the very end of the script, after Sections 9-13, and is
  correctly *excluded* from the Tier-1.5 prefix. Two untimed checks not listed in the issue's
  per-section table — "Checked-arithmetic abort model (#585)" (line ~935) and "span_pack
  verifier-bound regression (#1308)" (line ~1041), both between Sections 4c and 4d — also fall
  inside the included prefix, so the issue's ~590s sum is a floor, not the real total; the
  `--tier15-timeout-secs` default below already has headroom for this.
- `compiler/mutants_defaults.vow` — add `default_tier15_cmd()`, sibling to the existing
  `default_tier2_cmd()`, returning
  `"VOW_FULL_TEST_SKIP_CARGO=1 VOW_FULL_TEST_TIER15_ONLY=1 scripts/full_test.sh"`. No new module file
  is introduced (this function joins the existing `mutants_defaults.vow`), so `scripts/concat_vow.sh`
  — whose `FILES=(...)` arrays list self-hosted modules in dependency order for the bootstrap
  triple-test concat, and which *did* need an entry added when `mutants_defaults.vow` itself was
  created (see commit `00e18ac0`, `#1456`) — needs no change this time; `mutants_defaults` is
  already present in both `FILES` arrays.
- `compiler/mutants_main.vow` — in `run_mutants_run`:
  - parse `--tier15-cmd` (default `default_tier15_cmd()`) and `--tier15-timeout-secs` (default
    `1200`, i.e. 20 min — roughly 2x the ~590s measured sample cost, with headroom both for slower
    machines and for the untimed checks noted above), mirroring the existing
    `--tier1-cmd`/`--tier1-timeout-secs` parsing at lines 478-487.
  - insert the Tier-1.5 run between the existing Tier-1 success branch (`rc1 == 0`, currently line
    591) and the Tier-2 budget check (currently lines 592-611): run `tier15_cmd` via `run_shell`
    with `cd_with_log(..., "tier15", append: true)` (Tier 1's log write uses `append: false` to
    start the file; Tier 1.5 and Tier 2 both append to the same per-mutant log). Reuse
    `classify_oracle_rc(rc15)` for every outcome except `rc15 == 0`, which means "survived Tier 1.5,
    proceed to the existing Tier-2 budget/run logic unchanged."
  - **Tier encoding: keep Tier 1 = `1` and Tier 2 = `2` exactly as today; add Tier 1.5 as a
    separate, nominal value rather than renumbering.** Renumbering (`1`/`2`/`3`) would silently
    change what `tier == 2` means in every existing and future `outcomes.json`, break the one
    existing assertion that checks for it (`t11`'s `caught_t2` case), and gain nothing semantic.
    Instead, introduce an internal sentinel (e.g. a `TIER_15() -> i64 { 15 }` constant, analogous to
    the existing `ST_*()` constants in `mutants_oracle.vow`) and special-case *only its JSON
    serialization* in `outcome_record_json` to emit the literal number `1.5` instead of `15`:
    `1`, `1.5`, `2` all round-trip as ordinary, correctly-ordered JSON numbers, and every existing
    `"tier":2` meaning (and test) is untouched. The `unrun` path (Tier-2 budget exhausted after
    surviving Tier 1.5) reports `tier = 1.5`, since that mutant got further than Tier 1 before being
    deferred — matching the field's documented "highest tier reached" meaning.
  - accumulate `oracle_ms` as `t1_dur + t15_dur` (+ `t2_dur` if Tier 2 ran), not just the
    currently-summed `t1_dur` (+ `t2_dur`).
- `compiler/tests/test_mutants_main_tier2_default.vow` stays as-is (still true); add a sibling
  `compiler/tests/test_mutants_main_tier15_default.vow` for `default_tier15_cmd()`, same shape.
- `tests/mutants/tests.sh` — extend the black-box suite (see TDD slices below).
- `docs/mutants.md` — hand-edit: flag table (`--tier15-cmd`, `--tier15-timeout-secs`); the `tier`
  field's schema description (enum gains `1.5`, documented as a genuine three-way ordinal, not a
  renumbering); every place that currently says "Tier-1 survivors" in the context of the `unrun`
  status and the `--tier2-budget-secs` row — these now mean "Tier-1.5 survivors," since `unrun` can
  only happen after Tier 1.5 passed — including the `Summary.unrun` doc, the `unrun.txt` bullet, and
  the `logs/<id>.log` bullet; a short new paragraph on what Tier 1.5 is and why (point at this
  issue's rationale); a one-line mention of `VOW_FULL_TEST_TIER15_ONLY` so the default
  `--tier15-cmd` is self-explanatory from this doc alone; and a note that `--tier15-cmd 'true'` is
  how to opt back out to the old two-tier behavior — call out explicitly that this matters most for
  non-default `--root` runs, since the default Tier-1.5 command shells out to this repo's own
  `scripts/full_test.sh`, which is meaningless as an oracle for a `--root` pointing at a different
  Vow source tree.
- `docs/spec/cli.md` — hand-edit the `vow mutants run` flag synopsis and table (same additions as
  `docs/mutants.md`'s flag table, since `CLAUDE.md` requires CLI-flag changes to land here).
- `docs/spec/schemas/mutants-result.schema.json` — hand-edit: `Outcome.tier` enum `[1, 2]` →
  `[1, 1.5, 2]`, with the description explaining `1.5` as the new Tier-1.5 checkpoint between the
  two existing tiers.
- `CLAUDE.md` — the "Mutation Testing" section's one-sentence oracle description ("runs a tiered
  oracle (`scripts/bootstrap.sh --skip-cargo` then `VOW_FULL_TEST_SKIP_CARGO=1 scripts/full_test.sh`)")
  needs the Tier-1.5 step named, matching the precedent set by `00e18ac0` (`#1456`), which updated
  this exact sentence for its own tier-command change.

**Generated — regenerate, do not hand-edit:**

- `compiler/main.vow` (`GENERATE:SKILL_FULL:START/END` block, which embeds `docs/spec/cli.md` and
  `docs/mutants.md` verbatim — two separate copies of the mutants flag table live in this one file,
  both inside the generated span, confirmed by grep).
- `vow/src/skill.rs`, `skills/vow/reference/cli.md`, `skills/vow/schemas/mutants-result.schema.json`.
- Run, after all hand-edits above:
  ```bash
  uv run python scripts/generate_help.py
  cargo build --release -p vow
  scripts/bootstrap.sh --skip-cargo
  ```
  per `CLAUDE.md`'s documented spec-update workflow. `scripts/check_help_coverage.py` does not key
  on `mutants` content (grep confirmed), so it won't gate this change either way, but the three
  regen commands are still required for the generated files to stop drifting from the hand-edited
  spec sources.

## 3. TDD slices

1. **`default_tier15_cmd()` unit test (red → green).**
   Add `compiler/tests/test_mutants_main_tier15_default.vow` asserting
   `default_tier15_cmd() == "VOW_FULL_TEST_SKIP_CARGO=1 VOW_FULL_TEST_TIER15_ONLY=1 scripts/full_test.sh"`.
   Write the test first (it fails to compile — function doesn't exist yet), then add the function
   to `compiler/mutants_defaults.vow`.

2. **`full_test.sh` gate, structural regression test.**
   Running the real Section-0-through-8c span takes ~10 minutes, so a full black-box exercise of
   this gate is an integration check (slice 6 below), not a fast unit test. Add a cheap *structural*
   pin instead, next to the existing `tests/full_test_bootstrap/tests.sh` bootstrap-only tests (that
   file already tests `VOW_FULL_TEST_SETUP_ONLY` the same way): assert, via `grep -n` on
   `scripts/full_test.sh`'s own source, that (a) the line matching `VOW_FULL_TEST_TIER15_ONLY`
   appears at a line number strictly between `section_begin "Section 8c: Contract Quality"` and
   `section_begin "Section 9: Bootstrap Triple Test"`, and (b) the gate block itself contains
   `print_summary` (not a bare `exit 0`) — this second check is what would have caught the
   `SETUP_ONLY`-shape mistake described above, since a bare `exit 0` has no `print_summary` call in
   its vicinity. Red: write both assertions against current `full_test.sh` (fail, gate doesn't exist
   yet). Green: insert the gate block at the planned location in `PROMOTED_ONLY`'s shape.

3. **Tier-1.5 "caught" classification (`tests/mutants/tests.sh`).**
   New test `t15_run_classifies_caught_at_tier15`, modeled directly on the existing
   `t11_run_classifies_caught_and_missed`'s `caught_t1`/`caught_t2` sub-cases: run
   `do_run caught_t15 --root tests/fixtures/mutants --tier1-cmd 'true' --tier15-cmd 'false' --tier2-cmd 'true'`
   and assert `outcomes.json` contains `"status":"caught","tier":1.5,` for at least one mutant —
   anchor the grep with the trailing comma (`"tier":1.5,`, not a bare `"tier":1.5`) since `"tier":1`
   is a textual prefix of `"tier":1.5` and an unanchored pattern would false-positive-match either
   direction. Red: `--tier15-cmd` flag doesn't exist, `do_run` fails or the flag is silently ignored.
   Green: wire the flag through `run_mutants_run` and the classification branch described above.

4. **Tier-1.5 "pass-through to Tier 2" path.**
   New test `t15_tier15_pass_reaches_tier2`: `--tier1-cmd 'true' --tier15-cmd 'true' --tier2-cmd 'false'`,
   assert `"status":"caught","tier":2,` appears (anchored the same way, so it can't match a stray
   `"tier":2` substring from elsewhere) — this proves Tier 1.5 passing doesn't short-circuit Tier 2,
   and because Tier 2's numeric meaning is unchanged (see the encoding decision above), the
   *existing* `t11_run_classifies_caught_and_missed` test needs no edit to its assertions. It does,
   however, get the same `--tier15-cmd 'true'` override as every other pre-existing case — see slice
   5.

5. **Existing tests must not silently start invoking the real default Tier 1.5.**
   Every existing `tests/mutants/tests.sh` case that omits `--tier15-cmd` (all of them, today) would
   otherwise fall through to `default_tier15_cmd()`, which shells out to the real
   `scripts/full_test.sh` — turning fast, hermetic unit tests into 10-minute integration tests, and
   breaking them entirely under the fixtures used (`tests/fixtures/mutants`, not a real repo
   checkout). Audit every `do_run`/`run_vowm run` call site in `tests/mutants/tests.sh` (there are
   ~10, per the earlier grep) and add `--tier15-cmd 'true'` to each one that doesn't already
   specifically test Tier-1.5 behavior, so they keep exercising exactly Tier 1 and Tier 2 as before.
   This is a mechanical, red → green per call site: each test fails first (hangs or errors trying to
   run real `full_test.sh` against the tiny fixture tree) then passes once the no-op override is
   added.

6. **Manual integration check (not CI-automated — documented as a one-time verification step).**
   Run `VOW_FULL_TEST_SKIP_CARGO=1 VOW_FULL_TEST_TIER15_ONLY=1 scripts/full_test.sh` for real twice
   during implementation:
   - **Positive case**, clean tree: confirm (a) it exits 0, (b) wall-clock lands near the ~590s
     estimate plus the two untimed checks noted above (not near the full ~1630s+ suite time), (c)
     `Section 9` and later section headers never print.
   - **Negative case**, deliberately broken tree (e.g. temporarily revert one line of a
     `tests/run/*.vow` fixture's expected output, or point `VOW_FULL_TEST_RUST` at a stale binary so
     a parity check fails): confirm the run exits **nonzero**. The positive case alone only proves
     the gate exits early; it says nothing about whether `FAIL` state actually propagates through
     `print_summary`'s exit code, which is the entire point of mirroring `PROMOTED_ONLY` instead of
     `SETUP_ONLY` above.
   These acceptance criteria are what the issue is actually about, but deliberately are *not* turned
   into an automated test that runs on every `full_test.sh` invocation or in CI — doing so would just
   re-add the wall-clock cost this issue exists to avoid, for a check that slices 2-5 already pin
   structurally and behaviorally. Record the observed wall-clock and both exit codes in the PR
   description.

## 4. Verification surface

No contracts, codegen, or C-model changes. The only `vow { requires/ensures }` surface touched is
none — `run_mutants_run`, `default_tier15_cmd`, and the new test files are plain control flow and
string construction, same shape as the existing Tier-1/Tier-2 code they extend. No new `tests/run/`
or `examples/` fixtures are needed; `tests/fixtures/mutants` (already used by every
`tests/mutants/tests.sh` case) is sufficient for the new Tier-1.5 classification tests. ESBMC is not
in this change's path at all — `vowc mutants` itself is a CLI tool, not a verified-contract function,
and `scripts/full_test.sh` is bash.

## 5. Risk areas

- **Binary fixed point**: `compiler/mutants_defaults.vow` and `compiler/mutants_main.vow` are normal
  self-hosted source, compiled through the standard pipeline — no `clif.vow`, `lower.vow`, or
  stack-slot code is touched, so there is no new stack-slot-layout or `BTreeMap`/`HashMap`-ordering
  risk. The standard bootstrap triple-test (Section 9 of `full_test.sh`, run via
  `scripts/bootstrap.sh`'s own pipeline and via CI's `full-test.yml`) still covers this file the same
  way it covers every other `compiler/*.vow` change; nothing here is exempt from it. Explicitly:
  **`VOW_FULL_TEST_TIER15_ONLY` must never be set in `.github/workflows/full-test.yml` or any other
  CI-gating job** — CI must keep running the complete suite including Sections 9/10b/11/12/13. The
  new env var is for `vowc mutants`'s internal oracle only.
- **`parse → print → parse` idempotency**: unaffected — no grammar, syntax, or printer changes.
- **`cargo clippy --all -- -D warnings`**: unaffected — the only Rust-adjacent file touched
  (`vow/src/skill.rs`) is fully regenerated text content inside a `String::from(...)` literal, not
  hand-written logic; `cargo build --release -p vow` after regeneration (per the spec-update
  workflow) is the correct check, not clippy specifically, though clippy will still run clean since
  nothing structural changed.
- **Grep-anchoring pitfall from the `1.5` tier value.** Because the encoding decision above keeps
  `1` and `2` meaning exactly what they always meant, the *existing* `t11_run_classifies_caught_and_missed`
  assertions (`grep -cE '"status":"caught","tier":1'` / `...,"tier":2'`) remain correct as-is for
  this PR (Tier 1.5 is pinned to `'true'` in that test per slice 5, so no `1.5` record can appear in
  its output). But the patterns are unanchored, so they would silently start over- or under-counting
  the moment any `1.5` record appears in the same `outcomes.json` (`"tier":1` prefix-matches
  `"tier":1.5`). Harden them to `'"tier":1,'` / `'"tier":2,'` (trailing comma) in the same PR as a
  defensive fix, even though nothing in this change strictly requires it yet.
- **Tier-1.5 cost is not bounded by `--tier2-budget-secs`.** Tier 1.5 always runs for every Tier-1
  survivor, bounded only per-mutant by `--tier15-timeout-secs`; it does not count against the
  existing shard-level Tier-2 budget. For a shard with many Tier-1 survivors this adds a predictable
  `N × ~590s` floor that isn't currently capped in aggregate. This is an accepted trade-off for this
  minimal slice (Tier 1.5's whole purpose is to be cheap enough not to need its own budget); if it
  proves to need one, `--tier15-budget-secs` is a natural, separately-scoped follow-up — not bundled
  here.
- **Log file ordering**: Tier 1.5's `cd_with_log` call must use `append: true` (matching Tier 2's
  existing call), not `append: false` (which Tier 1 uses to start the file) — getting this backwards
  would silently truncate the Tier-1 portion of `logs/<id>.log` whenever Tier 1.5 runs. Slice 3's
  test only checks `outcomes.json` status/tier, not log contents, so add an explicit assertion
  (modeled on the existing `t14_...` difflog test, which already greps `logs/*.log` for `TIER1
  PROBE`/`TIER2 PROBE` markers) that a Tier-1.5-reaching run's log contains markers from all tiers it
  passed through, in order.

## 6. Out of scope

- The issue's own suggested mechanism — a generic `full_test.sh --sections 0,0b,1,2,...` flag — is
  **not** implemented. Most sections after Section 4 (`0b`, `1`, `2`, `2b`, `3`, `5`, `5b`, `5c`,
  `6`, `6b`, `8`, `8b`, `8c`) are inline top-level script code today, not wrapped in callable
  functions; building a generic section-selector would mean refactoring ~15 inline sections into
  functions first — a large, high-blast-radius change to a CI-gating script, for a selector this
  issue doesn't actually need (it only ever wants one fixed prefix, not arbitrary subsets). The
  single-purpose `VOW_FULL_TEST_TIER15_ONLY` env-var gate delivers the same outcome the issue asks
  for with a one-line, additive change, consistent with the script's existing
  `VOW_FULL_TEST_SETUP_ONLY`/`VOW_FULL_TEST_PROMOTED_ONLY` precedent. A generic section selector
  remains a legitimate idea for a separate, dedicated issue if a future use case needs arbitrary
  subsets rather than one fixed fast prefix.
- `--tier15-budget-secs` (a shard-level cap on aggregate Tier-1.5 time) — see Risk Areas; deferred
  until real shard runs show it's needed.
- Any change to Tier 1 or Tier 2's existing default commands, timeouts, or budget semantics beyond
  the `tier` field re-numbering required to make room for Tier 1.5.
- Reformatting or otherwise touching unrelated parts of `scripts/full_test.sh`, `docs/mutants.md`, or
  `docs/spec/cli.md` beyond the specific additions above.
- `#1296` (shared-`target/` corruption after Tier 2) — the issue notes Tier 1.5 reduces how often
  Tier 2 fires and therefore how often #1296 can trigger, but does not fix #1296 itself. No change
  here touches worktree/`target/` handling.
