# Plan: issue #1007 — adjacent-class false FAIL on a threshold-plateau grid

## 1. Problem restated

`vow_perf::analyze()`'s least-squares best-fit selection falsely reports `Fail` for a
semantically correct complexity declaration when the measurement grid's low end sits inside a
function's pre-threshold plateau (e.g. `input_size.saturating_sub(100)` is exactly `O(n)`, but on
the documented grid `[16, 32, 64, 128, 256, 512, 1024, 2048]` its `Linearithmic` fit narrowly
out-scores its `Linear` fit, so `analyze(Linear, …)` wrongly returns `Fail`). The issue's own
investigation shows this is broader than the original report — `(n-100)²` declared `O(n²)` and
`(n-100)³` declared `O(n³)` fail the same way — and that the reviewer's proposed fix (treat
near-tied adjacent R² as `Ambiguous`) is empirically unsound: measured ΔR² gaps for genuine
violations and false failures are fully interleaved, so no epsilon separates them. The real lever
is grid range, not R²-tie-breaking.

## 2. Empirical groundwork (done during planning, informs the design)

Using a Python re-implementation of `analyze()`'s exact formulas (`basis_value`, `r_squared`,
unweighted OLS, candidate fold), verified against `vow-perf/src/lib.rs` line-for-line:

- **Low-end-only exclusion (small-n "warmup") fixes only the degree-1 case.** Dropping the first
  1–3 points of the documented `[16..2048]` grid flips `n.saturating_sub(100)`/`Linear` from
  `Fail` to `Pass`, but `(n-100)²`/`Quadratic` and `(n-100)³`/`Cubic` **stay `Fail`** under every
  warmup count tried (1, 2, 3). This matches the issue's own claim that small-n exclusion alone
  does not resolve the degree-2/3 cases.
- **High-end extension (keeping the same low end, including the degenerate near-threshold points)
  fixes all three cases simultaneously, with no warmup needed.** `[16..4096]` (8 doublings) fixes
  only the `Linear` case; `[16..8192]` (9 doublings, 10 samples) fixes **all three** — `analyze()`
  returns `Pass` with `observed == declared` outright (not merely a narrow win), because the
  best-fit selection itself flips once enough large-n points dominate the unweighted residual sum.
- **No regression on genuine violations.** The three adjacent-pair true-violation analogues
  (`n·log n` declared `Linear`, `n²·log n` declared `Quadratic`, `n³·log n` declared `Cubic`)
  correctly return `Fail` on *both* the narrow documented grid and the extended `[16..8192]`/
  `[16..32768]` grids — extending the grid only makes genuine divergence more visible, never less.
- **This is a mitigation, not an elimination.** The required high end scales with the hidden
  threshold magnitude `K`, which `analyze()` cannot observe in advance (it only sees the samples
  it's given). A fixed default grid is validated against the issue's reported magnitude (`K≈100`
  at `min_input_size=16`) with margin; it is not a proof for arbitrary `K`. This matches the
  issue's own "neither mechanism alone is a complete fix" framing — restated as "grid range alone
  *is* a complete fix for the reported magnitude; it is bounded, like any finite-sample statistic,
  by how large a threshold the grid outgrows."
- **`vow-perf` has no self-hosted (`compiler/*.vow`) counterpart.** `grep -rl "ComplexityClass"
  compiler/` is empty, and the crate's own module doc says CLI/IR wiring is a separate, unbuilt
  concern. This is Rust-only internal tooling, not language syntax/semantics — the CLAUDE.md rule
  requiring same-session Rust + self-hosted changes does not apply to this issue.

## 3. Chosen design

**Do not change `analyze()`'s signature, algorithm, or verdict semantics.** It already classifies
correctly given an adequately-ranged grid; the bug is in grid *selection*, which is not something
`analyze()` can do (it only receives `&[Sample]`; there is no grid-generation or execution code
anywhere in `vow-perf` today — no harness exists yet to wire this into, per the issue's own
"Context" note pointing at #487).

Add a small, pure, parameter-light helper that gives the (future) harness a concrete,
empirically-justified default grid, and document the contract that `analyze()` assumes a
well-ranged sample set. This satisfies the issue's acceptance criterion 1 via the explicitly
offered alternative — "a harness-level … contract" — backed by actual code instead of prose only,
without touching the statistics core and without any risk of weakening soundness.

```rust
/// Number of geometrically-doubled sizes in `recommended_grid`'s output.
pub const RECOMMENDED_GRID_SAMPLE_COUNT: usize = 10;

/// A default measurement grid for `analyze()`'s input.
///
/// Ten sizes, doubling from `min_input_size` (so `min_input_size = 16` yields
/// `[16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192]`). Empirically
/// validated to resolve threshold-plateau false `Fail`s at the magnitude
/// reported in issue #1007 (a plateau ~100 below `min_input_size`'s first
/// few doublings) for every supported polynomial degree, with no change in
/// verdict for genuine complexity violations. Larger thresholds need a
/// wider grid than this default provides — this is a mitigation bounded by
/// how far the grid's high end sits above the hidden threshold, not a
/// guarantee for arbitrary workloads.
pub fn recommended_grid(min_input_size: u64) -> Result<Vec<u64>, DoublingRatioError> { … }
```

Reuse `DoublingRatioError::InputSizeTooSmall` for `min_input_size < 2` rather than inventing a new
error type — it already has the right shape and `analyze()` enforces the same floor.

Document the well-ranged-grid precondition directly on `analyze()`'s doc comment, and resolve the
design-doc contradiction (acceptance criterion 3) by editing `docs/design-performance-guarantees.md`:

- **Keep line ~296 (`FAIL` rule) exactly as-is.** `analyze()` does not and should not treat
  adjacent-class near-ties as `Ambiguous` — the issue's own ΔR² data proves no epsilon separates
  true violations from false failures.
- **Rewrite line ~396** ("Accept ambiguity: if both classes fit with R² ≥ 0.95 and the trend is
  inconclusive, the verifier reports `AMBIGUOUS`") — this describes a check `analyze()` does not
  implement for non-maximum adjacent pairs (only the existing `maximum_trend_is_ambiguous` guard,
  which structurally only ever fires for the maximum class, `CubicLogarithmic`, and only when
  `observed <= declared`). Replace it with: adjacent non-maximum classes are never resolved via
  R²-tie ambiguity; the practical mitigation is ensuring the measurement grid is well-ranged
  relative to the workload (see `recommended_grid` / new subsection below).
- **Clarify the "Small-n exclusion" bullet (~line 272)** to scope it to the doubling-ratio test
  (Step 3, "primary"), which `analyze()` does not implement at all (`expected_doubling_ratio` is a
  standalone pure method never called from `analyze()`) — it is not a mechanism available to, or
  needed by, the curve-fitting path this issue is about.
- **Add a new subsection** (near the "Distinguishing adjacent logarithmic factors" section)
  documenting the grid-range mitigation, `recommended_grid`, and the bounded-mitigation caveat.

## 4. Files to touch

- `vow-perf/src/lib.rs` — add `RECOMMENDED_GRID_SAMPLE_COUNT`, `recommended_grid()`, and doc
  comments on `analyze()` stating the well-ranged-grid precondition. No changes to `analyze()`'s
  body, `r_squared`, `basis_value`, or `maximum_trend_is_ambiguous`.
- `vow-perf/tests/threshold_plateau.rs` (new file) — regression tests (see TDD slices below).
- `docs/design-performance-guarantees.md` — edit the three spots identified above (~272, ~396,
  plus one new subsection). No other sections change.
- No `compiler/*.vow` changes (see §2, last bullet — no self-hosted counterpart exists).
- No `docs/spec/*.md` changes — this crate is not wired into the CLI/IR/verify pipeline, so no
  user-facing syntax, semantics, builtin, operator, effect, or CLI flag is affected.

## 5. TDD slices

Each slice is a small, independently reviewable commit.

1. **Red:** add `vow-perf/tests/threshold_plateau.rs` with a single test
   `recommended_grid_doubles_from_the_given_floor` asserting
   `recommended_grid(16).unwrap() == vec![16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192]`
   (fails to compile — `recommended_grid` doesn't exist).
   **Green:** implement `recommended_grid` and `RECOMMENDED_GRID_SAMPLE_COUNT` in
   `vow-perf/src/lib.rs`.

2. **Red:** add `recommended_grid_rejects_floors_below_the_measurement_domain` asserting
   `recommended_grid(1)` returns `Err(DoublingRatioError::InputSizeTooSmall { input_size: 1 })`
   (fails — no validation yet). **Green:** add the floor check, reusing
   `DoublingRatioError`.

3. **Red:** add `linear_threshold_plateau_is_no_longer_a_false_fail`: build samples from
   `recommended_grid(16)` paired with `input_size.saturating_sub(100)`, declare `Linear`, assert
   `analyze(...).verdict == Verdict::Pass` and `observed == Some(ComplexityClass::Linear)`. This
   is green immediately once slice 1 lands (per §2's probe data) — the test still earns its place
   as a pinned regression anchor for the issue's exact reported repro.

4. **Red/Green (same shape):** add `quadratic_threshold_plateau_is_no_longer_a_false_fail` for
   `input_size.saturating_sub(100).pow(2)` declared `Quadratic`.

5. **Red/Green (same shape):** add `cubic_threshold_plateau_is_no_longer_a_false_fail` for
   `input_size.saturating_sub(100).pow(3)` declared `Cubic`.

6. **Soundness lock (already green, write anyway):** add
   `linear_log_violation_still_fails_on_the_recommended_grid`,
   `quadratic_log_violation_still_fails_on_the_recommended_grid`,
   `cubic_log_violation_still_fails_on_the_recommended_grid` — same `recommended_grid(16)` sizes,
   workloads `n·log2(n)` / `n²·log2(n)` / `n³·log2(n)` declared one degree below, assert `Fail`
   with the expected `observed`. These prove slices 3–5 didn't buy `Pass` by weakening detection
   of genuine violations.

7. **Docs:** apply the three edits to `docs/design-performance-guarantees.md` from §3. No code
   changes in this slice — just doc text, reviewed on its own.

8. **Doc comments:** add the well-ranged-grid precondition to `analyze()`'s doc comment in
   `vow-perf/src/lib.rs` and the doc comment on `recommended_grid` shown in §3. Small, separable
   from the behavioral slices.

## 6. Verification surface

None. This is a pure-Rust statistics helper with no `.vow` surface: no contracts, no IR, no
codegen, no C model, no ESBMC properties. `vow-perf` is not wired into `vow build`/`vow verify`
yet (confirmed in §2), so no `tests/run/*.vow` or `examples/*.vow` fixtures are affected or need to
grow.

## 7. Risk areas

- **Patch-coverage gate.** `recommended_grid` is small but every line (including the error path)
  must be exercised — slice 2 exists specifically to cover the `min_input_size < 2` branch so the
  codecov/patch gate doesn't flag it as new-and-uncovered.
- **No binary-fixed-point, parse/print, or clippy-surface risk.** No `compiler/` files change, no
  `vow-syntax` grammar changes, and the new function follows the file's existing style (plain
  `Vec<u64>` construction, existing numeric-cast conventions) — low risk of new
  `cargo clippy --all -- -D warnings` findings, but run clippy anyway before committing.
- **Don't let `recommended_grid`'s default (`[16..8192]`, 10 samples) get read as a guaranteed
  fix.** The doc edits in slice 7 must state the bounded-mitigation caveat from §2/§3 explicitly —
  omitting it would misrepresent a statistical mitigation as a proof, which is exactly the kind of
  overclaim CLAUDE.md's contract-authoring rules warn against for this codebase's culture (even
  though this isn't a `requires`/`ensures` contract, the same "don't claim more certainty than the
  mechanism provides" norm applies).
- **Unrelated test noise.** `vow-perf/tests/separate_compilation.rs` links real runtime artifacts
  and may show environmental SKIP/FAILED independent of this change (see prior-session memory on
  vow-crate e2e tests in sandboxes) — verify any failures there reproduce on a clean `origin/main`
  checkout before attributing them to this PR.

## 8. Out of scope (deliberately not bundled)

- Any CLI/IR/harness wiring that would actually *generate and execute* a measurement grid against
  compiled Vow code — that's the unbuilt Phase 1/2 harness tracked under #487, not this crate.
- Changing `analyze()`'s signature, verdict semantics, `r_squared`, `basis_value`, or
  `maximum_trend_is_ambiguous` — the existing algorithm is correct given adequate input; no
  evidence supports touching it.
- Small-n/"warmup" exclusion machinery inside `analyze()` — empirically insufficient alone for the
  degree-2/3 cases (§2), and conceptually belongs to the unimplemented doubling-ratio test, not
  curve fitting.
- Making `recommended_grid` degree-aware or budget-aware (e.g. varying span by declared polynomial
  degree or an operation-count budget) — the flat 10-sample default already resolves all three
  reported repro cases; adding per-degree tuning now would be speculative generality without a
  validated need.
- Multi-variable grids (Phase 3, per the design doc's "Multiple Size Parameters" section).
- Any reformatting of `vow-perf/src/lib.rs` or the design doc beyond the specific edits listed in
  §3/§4.
