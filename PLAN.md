# PLAN.md — issue #1119: Unsigned sizes seam 4/7 (Phase A hard)

## 0. Scoping decision (read first)

Issue #1119's own body defines **four non-overlapping PRs** (PR1 `str_rfind_byte` extraction,
PR2 `complexity_main.vow` + `main.vow`, PR3 `lower.vow`, PR4 `region.vow` alone) and says so
explicitly: *"Ships at i64 throughout, so it is reviewable as a pure refactor and revertable
independently"* (PR1), and *"`region.vow`, alone, nothing else in flight"* (PR4). This
workflow, however, squash-merges **one PR per branch/issue run**. Those two facts conflict:
one PR cannot simultaneously be "PR1, revertable independently" and "PR4, with nothing else
in flight" and also be a single squash commit containing all four.

**Decision: this run implements PR1 only.** PR1 is the minimal slice that:
- is genuinely self-contained (ships at `i64` throughout, touches only
  `compiler/frontend.vow` + `compiler/main.vow` driver internals, no signature changes),
  matching "many small changes beat one large change" and "surgical changes" in CLAUDE.md;
  and
- **defuses a real, currently-live bug** (Landmine 1: `compiler/main.vow`'s `default_output`
  relies on unguarded `-1 + 1 == 0` wraparound) independently of whether seams 4b/4c/4d
  (PR2-4) ever land, so it has standalone value and a clean revert story.

PR2, PR3, and PR4 (the actual `u64` retyping of `main.vow`, `lower.vow`, `region.vow`) are
**out of scope for this run** and are left as follow-up work. Issue #1119 already carries
their full checklists in its body — **do not** file new issues for them; the next
implementation run against #1119 (or a manually re-triggered one) picks up where this one
left off using the refreshed line map in §5 below, not the issue's original (now stale)
citations.

**Consequence for the PR body:** the implementation stage's PR description must say
`Part of #1119` (or `Refs #1119`), **never** `Closes #1119` — a squash-merge with `Closes`
would auto-close the issue and orphan PR2-4's checklist, which is the entire reason this
issue stayed a single tracking issue instead of being split into seams 4a-4d up front like
the epic's other seams were. The implementation stage should also post one
`gh issue comment 1119` recording this scoping decision (per the operating contract's
best-effort-decision rule), pointing at the merged PR1 and noting PR2-4 remain open.

## 1. Problem restated

`compiler/frontend.vow::frontend_dir_path` and its two near-duplicate siblings in
`compiler/main.vow` (`default_output`, `skill_parent_dir`) each hand-roll a "find the last
`/` in a path" scan using an `i64 last_slash = -1` sentinel. Two of the three guard the
sentinel correctly before using it; one — `default_output`, the function that computes the
output binary name for every `vowc build <path>` invocation that omits `-o` — does not: it
computes `start = last_slash + 1` and relies on wrapping `-1 + 1 == 0` to produce the
correct "no slash" fallback. This is silently correct today under `i64`/wrapping `+`, but
is exactly the kind of unguarded sentinel arithmetic that (a) has no test coverage for the
"no slash" path today, (b) is a landmine for the future `u64` retyping of this module (where
the same wraparound becomes `u64::MAX`, and `+!` would trap), and (c) the general Phase-A
rule says should be centralized once, not fixed three times inconsistently. This run extracts
one shared, guarded `str_rfind_byte` helper, migrates the three call sites to it, adds the
missing test coverage, and leaves a documented, accurate map of the remaining PR2-4 work for
the next run — without changing any exported signature and without retyping anything to
`u64` (that is PR2-4's job, gated on this landmine being defused first, per the issue).

## 2. Files to touch

All changes are confined to the self-hosted compiler (`compiler/`); this is driver-internal
plumbing with no corresponding Rust-crate implementation to mirror (the Rust compiler
(`vow/src/*.rs`) has its own independent path-handling code, not a line-for-line twin of
these `.vow` helpers). Precedent: `ef8a050c` ("migrate size locals to u64", #1314) touched
`compiler/frontend.vow` alone with no Rust-side change, for an analogous internals-only
refactor of the same module. **No `docs/spec/*.md` edit is needed** — no syntax, semantics,
builtin signature, operator, effect, or CLI flag changes.

- `compiler/frontend.vow` — add `fn str_rfind_byte`; rewrite `frontend_dir_path` (currently
  `:58-75`) to use it.
- `compiler/main.vow` — rewrite `default_output` (currently `:89-106`) and `skill_parent_dir`
  (currently `:14040-14058`) to use it; guard the `argv.len() - 1` underflow in
  `get_flag_arg` (currently `:31-40`, underflow at `:33`).
- New test coverage (exact mechanism decided in Slice 3 below): either a `compiler/test_*.vow`
  unit-test module, or a CLI-level fixture — see §3.

No other file changes. In particular:
- **Do not** touch `compiler/complexity_main.vow` in this run — it is part of PR2, not PR1,
  even though it is currently byte-identical to the issue's baseline and low-risk. Bundling
  it here would blur PR1's "ships at i64 throughout, revertable independently" boundary for
  no benefit (it does not call `str_rfind_byte` or any function this PR touches).
- **Do not** touch `compiler/lower.vow` or `compiler/region.vow` — out of scope (PR3/PR4).
- **Do not** touch `compiler/verifier.vow` — `str_find_byte`/`str_find_str` there are a
  naming/shape precedent to follow, not code to modify.

## 3. TDD slices

Vow's test surface here is unusual: there is no unit-test framework import story visible in
`compiler/test_*.vow` (`test_arith.vow`, `test_assign.vow`, `test_string.vow`,
`test_struct.vow`, `test_vec.vow`, `test_vow.vow`, `test_while.vow` — one file per language
feature, run via `vowc test` or similar, not per-function). Confirm the exact harness
mechanism as the first step of Slice 1 (`grep -rn "test_vow\|compiler/test_" scripts/
tests/`) before writing anything, since "red" must be a real failing run, not a written but
unexecuted assertion.

1. **Slice 1 — red: `str_rfind_byte` has no test today, and the no-guard bug is
   unexercised.**
   - Locate (or create) the right home for a self-hosted unit test of a pure `String, i64 ->
     Option<u64>` helper — likely a new `compiler/test_str_rfind_byte.vow` alongside the
     existing `test_string.vow` pattern, or an addition to `test_string.vow` itself if that
     file already covers `String` helpers generally (check its contents first).
   - Write test cases: empty string (`None`), no match (`None`), match at byte 0 (`Some(0)`),
     match at the last byte (`Some(n-1)`), multiple matches (returns the *last*, i.e. the
     highest index — confirm this is the desired "rfind" semantics, matching the three
     current call sites' "last slash" intent).
   - Confirm this fails to compile/run (function doesn't exist yet) — that is the "red".

2. **Slice 2 — green: implement `str_rfind_byte`.**
   - In `compiler/frontend.vow`, add:
     ```vow
     fn str_rfind_byte(s: String, b: i64) -> Option<u64> {
         let n: u64 = s.len() as u64;
         let mut i: u64 = n;
         while i > 0 {
             i = i - 1;
             if s.byte_at(i as i64) == b {
                 return Some(i);
             }
         }
         None
     }
     ```
     This is the canonical descending-loop idiom from `docs/spec/grammar.md:620-637`
     (guard-then-decrement-first, `while i > 0 { i = i - 1; ... }`), adapted to return on
     match instead of accumulating. No `vow` contract block — match `str_find_byte`
     (`compiler/verifier.vow:177-187`)'s precedent of being an unannotated, unverified
     helper; a length-and-byte-scan loop needs no `requires`/`ensures` to be meaningful, and
     inventing one here would be exactly the kind of verifier-driven busywork CLAUDE.md's
     contract-authoring rules warn against (no genuine semantic domain constraint exists
     beyond what the types already say).
   - Run the Slice 1 tests; they should now pass ("green").
   - Bootstrap-verify this one file's compile (`build/vowc verify --no-verify` is not a real
     flag — use `build/vowc build --no-verify compiler/frontend.vow` or the project's actual
     per-module smoke path) to confirm it type-checks before wiring callers.

3. **Slice 3 — red then green: `frontend_dir_path` migrates, behavior preserved.**
   - `compiler/frontend.vow:58-75` currently:
     ```vow
     fn frontend_dir_path(path: String) -> String {
         let mut last_slash: i64 = -1;
         let mut i: u64 = 0u64;
         let n: u64 = path.len() as u64;
         while i < n {
             if path.byte_at(i as i64) == 47 { last_slash = i as i64; }
             i = i + 1;
         }
         if last_slash == -1 { String::from(".") }
         else { /* copy bytes [0, last_slash) */ }
     }
     ```
   - This already has `#1314`'s `u64` loop counter; only the scan itself needs to move to the
     helper. Rewrite as:
     ```vow
     fn frontend_dir_path(path: String) -> String {
         match str_rfind_byte(path, 47) {
             None => String::from("."),
             Some(k) => {
                 let r: String = String::from("");
                 let mut j: u64 = 0u64;
                 while j < k {
                     r.push_byte(path.byte_at(j as i64));
                     j = j + 1;
                 }
                 r
             }
         }
     }
     ```
   - This is a pure refactor (same observable behavior); the existing behavior is exercised
     indirectly by every multi-module compile in `tests/run/` and `tests/multi/` (module
     resolution depends on `frontend_dir_path`). Confirm at least one `tests/multi/*` fixture
     exists and currently passes (red/green here is "does the existing multi-module test
     suite still pass after the rewrite" — there's no behavior change to newly assert).

4. **Slice 4 — red: `default_output`'s "no slash" path is unexercised today.**
   - `compiler/main.vow:89-106` currently (`:92` `last_slash = -1`, `:100`
     `start = last_slash + 1`, relying on wraparound):
     ```vow
     fn default_output(path: String) -> String {
         ...
         let mut last_slash: i64 = -1;
         ... // scan, no guard on last_slash before use
         let start: i64 = last_slash + 1;
         ...
     }
     ```
   - Confirm there is no existing test that calls `vowc build <bare-filename-with-no-slash>`
     and checks the output binary name. Add one: a CLI-level fixture invoking
     `build/vowc build --no-verify <name>.vow` from a `cwd` where `<name>.vow` has no `/` in
     its path, asserting the produced binary is named exactly `<name>` (no leading garbage
     character, no truncation). The right harness is whichever one already drives
     `build/vowc` end-to-end with real subprocess invocation and filesystem assertions —
     confirm by reading `scripts/cli_compat_test.sh` in full and `tests/run_tests.sh`'s
     Phase structure; this may be better added to `tests/run_tests.sh` as a new phase/case
     than shoehorned into `cli_compat_test.sh`, which is a Rust/self-hosted *parity* harness,
     not a default-output-naming harness. Pick whichever is the closer semantic fit and note
     the choice in the PR description.
   - Confirm this new test fails against the *current* (pre-fix) code only if the current
     wraparound actually produces a wrong answer — it doesn't (by design, `-1 + 1 == 0` under
     wrapping `+` happens to be correct). So this slice's "red" is: **write the test, watch it
     pass against unmodified code** (proving no regression exists yet), because the actual
     bug is latent (only surfaces once PR2 changes `last_slash` or the `+` to something that
     doesn't wrap the same way). Record explicitly in the PR description that this test is
     *regression insurance for PR2*, not a currently-failing-bug fix — do not claim a false
     "this was broken and now isn't" in the PR body.

5. **Slice 5 — green: migrate `default_output` to `str_rfind_byte`.**
   - Rewrite:
     ```vow
     fn default_output(path: String) -> String {
         let start: i64 = match str_rfind_byte(path, 47) {
             None => 0,
             Some(k) => (k + 1) as i64,
         };
         ... // rest of the function, unchanged from this point
     }
     ```
     The `+ 1` now lives inside the `Some` arm over a `u64`, where it cannot underflow, and
     the `None => 0` arm replaces the old wraparound-dependent fallback with an explicit one.
   - Run Slice 4's new test; it should still pass (now for the right reason — an explicit
     `None => 0` arm, not wraparound).
   - Run the full existing test suite for anything exercising default output naming
     (`tests/run/`, any test that omits `-o`) to confirm no behavior change.

6. **Slice 6 — green: migrate `skill_parent_dir`.**
   - `compiler/main.vow:14040-14058` currently guards correctly (`if last_slash < 0`), so this
     is a pure like-for-like swap to `str_rfind_byte`, no behavior change:
     ```vow
     fn skill_parent_dir(path: String) -> String {
         match str_rfind_byte(path, 47) {
             None => String::from("."),
             Some(k) => { /* copy bytes [0, k) */ }
         }
     }
     ```
   - This function sits at `:14040`, **after** `// GENERATE:SKILL_FULL:END` at `:14031` — it
     is hand-written code, not inside a generated block, so it is directly editable (confirm
     this placement still holds at edit time; do not assume the offset survives untouched —
     re-`grep` immediately before editing).
   - Existing behavior is covered indirectly by any test that runs `vowc skill install`;
     confirm at least one such test exists and passes unchanged.

7. **Slice 7 — red then green: guard `get_flag_arg`'s `argv.len() - 1` underflow.**
   - `compiler/main.vow:31-40`, underflow at `:33`:
     ```vow
     fn get_flag_arg(argv: Vec<String>, flag: String) -> String {
         let mut i: i64 = 0;
         while i < argv.len() - 1 {
             ...
         }
     }
     ```
   - `argv.len()` is `i64` today (Phase A hasn't retyped `.len()` itself yet — that's a later,
     separate seam, #1121, not this one), so `argv.len() - 1` on an empty `argv` is `-1`, and
     `i < -1` with `i` starting at `0` is simply false — **this does not underflow or misbehave
     under today's `i64` `.len()`**. Re-verify this claim before treating it as a live bug:
     the issue's framing ("a distinct underflow... on an empty argv") anticipates the *future*
     `u64` `.len()`, where `argv.len() - 1` on empty argv wraps to `u64::MAX` and the loop
     would iterate forever / index out of bounds. Since `.len()` itself stays `i64` until seam
     6 (#1121), this is **not yet exploitable** and arguably belongs in seam 6's PR, not here.
   - **Decision for this slice:** guard it anyway, now, as defensive hardening consistent with
     "no shortcuts" / "scalability is a requirement" in CLAUDE.md, and because it costs nothing
     to fix correctly today:
     ```vow
     fn get_flag_arg(argv: Vec<String>, flag: String) -> String {
         let n: i64 = argv.len();
         let mut i: i64 = 0;
         while n > 1 && i < n - 1 {
             ...
         }
     }
     ```
   - Add a test: `get_flag_arg(Vec::new(), "-o")` (empty argv) returns the function's existing
     not-found fallback value without looping. Confirm this passes both before and after (it's
     not currently broken at `i64`) — this slice is pure hardening + test-gap closure, not a
     bug fix; say so in the PR body for the same honesty reason as Slice 4.

## 4. Verification surface

None of these six functions carry `requires`/`ensures`/`invariant` clauses today, and this
PR does not add any (see Slice 2's rationale — no genuine semantic domain constraint beyond
what `Option<u64>` and `u64` already encode). Verification impact is therefore limited to:

- `build/vowc build` (default: verify on) must still succeed on `compiler/main.vow` and
  `compiler/frontend.vow` after the rewrite — the ESBMC pass runs over these files as part of
  every `scripts/bootstrap.sh` invocation regardless of whether these specific functions have
  contracts, because bootstrap verifies the whole self-hosted compiler, not per-function.
- No new `unknown`/`unwind` verdicts should appear for `frontend.vow`/`main.vow` — if one does,
  it's most likely the new `while i > 0 { i = i - 1; ... }` descending loop in
  `str_rfind_byte` needing the same treatment `docs/spec/grammar.md:620-637` prescribes
  (its documented form already verifies cleanly elsewhere in the corpus, e.g.
  `str_find_byte`'s ascending twin at `verifier.vow:177`).
- No `tests/run/` or `tests/verify*` fixture needs a **new verification property** — this PR
  doesn't touch contracts, codegen, or the C model. The new tests added in Slices 1, 4, and 7
  are behavioral/regression tests, not verification fixtures.

## 5. Risk areas

- **Stale line numbers.** This plan's line citations for `compiler/main.vow` and
  `compiler/frontend.vow` were re-verified against current `HEAD` (`8b12f0ad`) during
  planning, **not** against the issue's own citations, which were written against
  `2feeb7d6` and have drifted: `main.vow` grew from 13,640 to 14,769 lines (+1,129, mostly
  from Operation Catalogue migrations landing in the interim) and `frontend.vow` was already
  partially touched by `#1314`. `region.vow` (out of scope here) is byte-identical to
  `2feeb7d6` and its citations in the issue body remain exactly accurate for whenever PR4
  is picked up. Re-`grep` every line number immediately before editing in the implementation
  stage regardless of what this plan says — more commits may land on `main` between planning
  and implementation.
- **Binary fixed point.** This PR changes control flow (new function, new call sites) in
  code that participates in the self-hosted compiler's own compilation (`main.vow`,
  `frontend.vow` are compiled by `build/vowc` to produce `build/vowc` itself). The
  `scripts/concat_vow.sh` triple-stage SHA-256 fixed point must be re-established — any
  nondeterminism (e.g., if `str_rfind_byte`'s loop were accidentally order-dependent, which
  it isn't) would show up there first.
- **`Option<u64>` boundary.** This is the first `Option<u64>`-returning helper introduced in
  this module. Confirm `Option<u64>`'s codegen/lowering path is already exercised elsewhere
  in the self-hosted compiler (it is — `Option<T>` is a built-in generic, not new syntax) so
  this isn't inadvertently exploring new IR-lowering territory; if `vow-ir`/`compiler/lower.vow`
  have any `Option<u64>`-specific gaps, they'd surface as a compile failure on `frontend.vow`
  itself, which is caught by bootstrap.
- **Test harness choice (Slice 4).** Picking the wrong harness for the "no slash in path" CLI
  test (parity harness vs. a dedicated driver test) risks either not running in CI
  (`tests/run_tests.sh`'s phases are developer-local per this repo's own `CLAUDE.md` note on
  `full_test.sh` vs `run_tests.sh`) or asserting the wrong thing (a parity harness checks
  Rust-vs-self-hosted agreement, not correctness against a golden filename). Read both
  harnesses in full before choosing, and if the test lands in `run_tests.sh`-only territory
  (not CI-gating), say so explicitly in the PR body rather than implying CI coverage that
  doesn't exist.
- **Both-compilers rule.** CLAUDE.md requires touching both the Rust compiler and self-hosted
  compiler "when implementing changes across Vow compilers." This PR is self-hosted-only by
  design (driver-internal helper in code that has no Rust-crate twin), following `#1314`'s
  precedent exactly. Flag this reasoning explicitly in the PR body so a reviewer doesn't
  bounce it for a missing Rust-side change.
- **`clippy`/`cargo test`.** Since no Rust file changes, `cargo build --all`, `cargo test
  --all`, and `cargo clippy --all -- -D warnings` should be unaffected; run them anyway as
  the Definition of Done requires, as separate commands, to catch anything unexpected (e.g.
  a generated-artifact staleness check that happens to touch Rust).
- **Not a contract-weakening risk.** No contracts are touched, added with a narrower-than-true
  bound, or removed in this PR — the contract-authoring risk class from CLAUDE.md doesn't
  apply here.

## 6. Out of scope (deliberately not bundled)

- **PR2** (`compiler/complexity_main.vow` + `compiler/main.vow` `u64` retyping, the GENERATE-
  block generator edits for `skill_support_path`/`skill_support_content_index_guard`
  contracts, and the `.len() - X` audit at the (re-verified, see below) current locations
  `main.vow:3136`-equivalent sites). Re-verification during planning found:
  - The descending `while i >= 0` loop the issue cites at old `:761` is now at `:762` — still
    present, still needs the canonical-idiom conversion in PR2.
  - The issue's cited `.len() - X` underflow sites at old `:1656`/`:1736`/`:1884` currently
    correspond to `file_stem_from_path` (`parts[parts.len() - 1]`, guarded by a prior
    `if parts.len() == 0` early return), a `while stack.len() > 0 { ... stack[stack.len() -
    1] ... }` loop (guarded by the loop condition itself), and a
    `if name_parts.len() > 0 { name_parts[name_parts.len() - 1] } else { ... }` (explicitly
    guarded) — **all three already have guards** at current `HEAD`, unlike what the issue's
    phrasing implies. PR2 should confirm these are genuinely safe under `u64` (they appear to
    be) rather than assuming unguarded-underflow work is needed.
  - A new site not in the issue's original text: `main.vow:14264` `while pos >= 0` inside
    `cq_without_casts`, where `pos` comes from `str_find_str` (an exported, `i64`-returning
    function in `compiler/verifier.vow`). This is a "-1-sentinel loop guard" pattern, not a
    descending-counter underflow — it should stay `i64` (changing `str_find_str`'s signature
    is out of the internals-only rule's scope), but PR2 should classify it explicitly per the
    issue's own "classify every integer local... before editing it" discipline (that
    instruction is stated for PR4/region.vow but is equally good practice here).
  - The issue's claim that `-!` and `-` are "currently indistinguishable to ESBMC" is now
    stale — `#1150` ("model checked-arithmetic abort so `+!` is not identical to `+`") has
    landed since the issue was filed. The recommendation to use guarded plain `-` for the
    descending loop likely still stands, but PR2 should re-derive the reasoning rather than
    citing the issue's now-outdated justification.
- **PR3** (`compiler/lower.vow`). Re-verification found the file has churned more than pure
  growth (844 insertions / 462 deletions since the issue's baseline, net +382 lines) — beyond
  line-number drift, some of the cited code may have been restructured. The six `.len() - 1`
  sites the issue cites appear to correspond (by a consistent offset pattern) to current
  `:3136`, `:3271`, `:3274`, `:3480`, `:4625`, `:4636`, and the for-each desugar hardcoded
  `ITY_I64()` payloads the issue flags as highest-risk (old `:3046`) now appear to be at
  `:3353`-`:3453` (the `__vow_vec_len` call, index init, `IOP_LT()` guard, and
  `__vow_vec_get_val` call) — **but this was confirmed only by pattern-matching, not a full
  re-read of the surrounding function**, since `lower.vow` is out of scope for this run.
  Re-verify from scratch, do not trust these offsets.
- **PR4** (`compiler/region.vow`, alone). Confirmed **byte-identical** to the issue's baseline
  commit `2feeb7d6` — every line citation in the issue body for this file (Landmine 2's
  tri-state `block_parent` at `:3311-3313`/`:3515`, the four descending loops, the twelve
  `.len() - X` sites, `marker_caller_store` at `:3207-3212`) was spot-checked during planning
  and found exact or within one line. This file can be picked up directly from the issue's
  own text without a fresh line-number audit, **except** that it must still wait for PR2 and
  PR3 to land first (per the issue's stated ordering) and must ship with nothing else from
  this epic in flight.
- **Any refactor, formatting, or cleanup unrelated to the sentinel-scan landmine** — e.g. not
  touching `file_stem_from_path`'s unrelated logic, not reformatting surrounding code, not
  renaming anything beyond what's needed to introduce `str_rfind_byte`.
- **Mutation testing.** Per the epic's 2026-09-21 addendum, seams 4/5/7 are pure
  internals-only retypes where `vowc mutants` is advisory, not required. PR1 isn't even a
  retype (it ships at `i64`), so this applies even more clearly — skip it unless time allows
  opportunistically.
