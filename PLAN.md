# Plan: issue #1009 — vow-perf minor follow-ups from PR #989 review

## 1. Problem restated

Issue #1009 bundles four non-blocking review nits from PR #989 against
`vow-perf/src/lib.rs`'s complexity-classification core. Checking each against
the code as it stands today (not the PR #989 diff the issue quotes) changes
the actual scope: **item 2 (precision loss) is already fixed** by the
translation-invariant delta centering in `r_squared`/`normalized_interval_slope`
and already pinned by an existing test
(`large_operation_baselines_preserve_small_growth_deltas`); **item 1
(tolerance sensitivity)** still has no real fix available because it requires
real noisy instrumentation data that doesn't exist yet (tracked generally
under #487) — it gets documentation only; **item 3 (readability)** is real:
the exact `observed == ComplexityClass::CubicLogarithmic && observed <=
declared` pattern the issue describes still exists verbatim at
`vow-perf/src/lib.rs:277-278`; **item 4 (CANDIDATES duplication)** is real and
gets a drift guard. The actual work in this PR is therefore: a behavior-
preserving readability refactor of the maximum-class guard (item 3), a
compile/test-time drift guard tying `CANDIDATES` to `ComplexityClass`'s
variant order (item 4), a short doc comment capturing the known tolerance
limitation (item 1), and citing the existing fix + test for item 2 in the PR
body rather than touching code for it.

## 2. Files to touch

- `vow-perf/src/lib.rs` — production file. Changes:
  - Add `const MAXIMUM_CANDIDATE: ComplexityClass = CANDIDATES[CANDIDATES.len() - 1];` right after the `CANDIDATES` array (currently lines 226-235).
  - Rewrite the maximum-class guard in `analyze` (currently lines 277-279) from
    `observed == ComplexityClass::CubicLogarithmic && observed <= declared` to
    `observed == MAXIMUM_CANDIDATE && declared == MAXIMUM_CANDIDATE`.
  - Rewrite `let maximum = ComplexityClass::CubicLogarithmic;` in
    `maximum_trend_is_ambiguous` (currently line 299) to `let maximum =
    MAXIMUM_CANDIDATE;`.
  - Add a short doc comment above `NORMALIZED_TREND_TOLERANCE` (currently
    line 223) documenting that the value is tuned against exact synthetic
    fixtures and will need revisiting once real (noisy) instrumentation
    exists. No issue/PR numbers in the comment (per repo comment style).
  - Add a `#[cfg(test)] mod tests` block at the bottom of the file with one
    unit test enforcing that `CANDIDATES`'s order matches
    `ComplexityClass`'s declaration order (item 4's drift guard). This must
    live inside `lib.rs`, not the external integration test file, because
    `CANDIDATES` is a private const and only in-crate unit tests can see it.
- `vow-perf/tests/log_polynomial.rs` — two new characterization tests (see
  TDD slices below) that pin the maximum-class guard's behavior at its two
  boundary conditions before the refactor lands.
- No changes to `compiler/`, `docs/spec/*.md`, or any `.vow` fixture.
  `vow-perf` has no self-hosted counterpart — confirmed by grepping
  `compiler/` for `ComplexityClass`/`CANDIDATES`/`NORMALIZED_TREND_TOLERANCE`
  (no real hits; the one case-insensitive grep hit in `clif.vow` was an
  unrelated use of the word "candidate"). `vow-perf` is Rust-only tooling for
  performance-contract analysis, not part of the Vow language or the
  self-hosted compiler pipeline, so the "modify both compilers" rule in
  CLAUDE.md does not apply here. None of the changes touch syntax, semantics,
  builtins, operators, effects, or CLI flags, so no `docs/spec/*.md` update
  is required. `docs/design-performance-guarantees.md` already documents the
  maximum-class guard and the relative-tolerance idea at the design level
  without pinning the exact `1.0e-9` constant or the `<=`-vs-`==` phrasing,
  so it needs no edit either.

## 3. TDD slices

1. **Characterization test: a tight non-maximum Pass must not trigger the
   maximum-class guard at the minimum sample count.**
   Add `tight_linear_declaration_passes_with_minimum_sample_count` to
   `vow-perf/tests/log_polynomial.rs`: three samples
   `[(16,16),(32,32),(64,64)]` (exact `O(n)` growth), `declared =
   ComplexityClass::Linear`. Assert `Verdict::Pass` and `observed ==
   Some(ComplexityClass::Linear))`. By hand-verification, `Linear` is the
   unique candidate reaching `r_squared == 1.0` on this 3-point grid (every
   other basis — log, n², n³, n·log n, etc. — is not affine in `n` for
   `{16,32,64}`, so none can also hit exactly 1.0), so this is deterministic,
   not a near-tie. This test passes against today's code already; it exists
   to catch the specific wrong rewrite the issue's own wording invites —
   dropping the `observed == <max class>` anchor and keeping only `observed
   == declared` — which would wrongly route this case through
   `maximum_trend_is_ambiguous`, and since `samples.len() == 3 <
   REQUIRED_MAXIMUM_TREND_SAMPLES (4)`, that function returns `true`
   immediately, flipping the verdict to `Ambiguous`. During implementation,
   confirm red/green by hand: temporarily apply the wrong rewrite, confirm
   this test fails, then apply the correct rewrite from slice 3 and confirm
   it passes again. Do not commit the wrong rewrite at any point.

2. **Characterization test: the maximum-fit class with a non-maximum
   declaration must still Fail, not Ambiguous.**
   Add `maximum_class_fit_fails_a_strictly_lower_declaration` to the same
   file: reuse the quartic-growth samples from the existing
   `maximum_supported_declaration_does_not_pass_quartic_work` test (`input_size.pow(4)`
   over `[16,32,64,128,256,512]`, which best-fits `CubicLogarithmic`), but
   declare `ComplexityClass::Cubic` (one step below the maximum) instead of
   `CubicLogarithmic`. Assert `Verdict::Fail` and `observed ==
   Some(ComplexityClass::CubicLogarithmic)`. This guards the other half of
   the corrected `&&`: it catches an accidental `||` in the rewrite (which
   would wrongly invoke the trend guard here since `observed` alone already
   equals the maximum) as well as a rewrite that forgets the `declared ==
   MAXIMUM_CANDIDATE` conjunct entirely.

3. **Refactor: introduce `MAXIMUM_CANDIDATE` and rewrite the guard.**
   Make the three production edits in `vow-perf/src/lib.rs` described in
   Section 2 (the new const, the `analyze` guard rewrite, the
   `maximum_trend_is_ambiguous` rewrite). Run the full `vow-perf` test suite
   (existing tests plus slices 1 and 2) and confirm all green with no
   behavior change. This closes issue item 3 and removes two of the three
   hardcoded `ComplexityClass::CubicLogarithmic` literals the issue's item 4
   flags as duplication-prone, tying "the maximum candidate" to `CANDIDATES`
   itself instead of a separately-typed literal.

4. **Drift guard: tie `CANDIDATES`'s order to `ComplexityClass`'s declared
   variant order.**
   Add a `#[cfg(test)] mod tests` block to `vow-perf/src/lib.rs` with a
   private `const fn expected_position(class: ComplexityClass) -> usize`
   containing an exhaustive match from each variant to its declared index
   (0-7), and a test `candidates_match_declared_variant_order` that loops
   `CANDIDATES.iter().enumerate()` and asserts `expected_position(*class) ==
   index` for every entry. Two independent guarantees fall out of this:
   (a) exhaustiveness checking is a static property of the `match` itself,
   so adding a ninth `ComplexityClass` variant without adding a match arm is
   a **compile error**, not just a test failure, regardless of where the
   match lives; (b) reordering, omitting, or duplicating an entry in
   `CANDIDATES` relative to the enum's declaration order fails this test the
   next time `cargo test -p vow-perf` runs. This must be a plain (non-`const`)
   test function, not a `const _: () = { ... };` compile-time assertion:
   derived `PartialOrd`/`PartialEq` comparisons are unavailable in `const`
   context on stable Rust without extra ceremony, and more importantly a
   compile-time-only assertion executes during `rustc`'s own build, not
   during the instrumented test run — codecov/patch (a real blocking gate
   per project history) would then see its lines as new-and-uncovered. A
   `#[test]` function executes at test runtime and is covered normally.
   During implementation, confirm red/green by hand: temporarily swap two
   entries in `CANDIDATES`, confirm this test fails, then restore the
   correct order and confirm it passes. Do not commit the swapped order.

5. **Documentation-only: capture the tolerance-sensitivity limitation.**
   Add a short doc comment directly above `NORMALIZED_TREND_TOLERANCE` in
   `vow-perf/src/lib.rs` explaining that the value is tuned against today's
   exact synthetic fixtures (where "any non-decreasing step counts as
   rising"), and will need revisiting once real instrumented operation
   counts (noisy, integer) feed `analyze`, since measurement noise alone
   could otherwise tip a genuinely maximum-class function into a spurious
   `Ambiguous` verdict. No code or behavior change, no issue/PR number in the
   comment text. This closes issue item 1 as "documented, not fixed" — the
   issue's own text defers the actual fix to #487's later phases since there
   is no real noise data to tune against yet. Do not guess a new tolerance
   value.
   - Optional stretch (attempt only if it comes together cleanly; skip
     without blocking the PR if not): a test demonstrating that a minimal
     ±1-operation integer nudge at `CubicLogarithmic`-scale operation counts
     (~10⁹, where a relative nudge of `NORMALIZED_TREND_TOLERANCE` is close
     to one integer unit) is already enough to flip the trend guard's
     "rising" classification, concretely illustrating the sensitivity the
     comment describes. If the exact numbers don't fall out cleanly within a
     short effort, drop this and keep only the comment — the comment alone
     satisfies the issue's ask.

Item 2 (precision loss) gets no new slice: cite the existing fix (the
`operation_origin`/`operation_delta` centering in `r_squared`, and the
`i128` delta in `normalized_interval_slope`) and the existing test
`large_operation_baselines_preserve_small_growth_deltas` in the PR body as
evidence it is already resolved. Verified during planning: `cargo test -p
vow-perf large_operation_baselines` passes today, and `git log -S
operation_origin --oneline -- vow-perf/src/lib.rs` shows the fix landed in
`07500b9a` (`feat(vow-perf): classify log-polynomial complexity (#989)`) —
i.e. PR #989 itself already shipped with the centering fix, and `lib.rs` has
had no further changes since (`git log --oneline -- vow-perf/src/lib.rs`
shows only `07500b9a` and the unrelated `2464f9b2` instrumentation-isolation
commit). Cite `07500b9a` directly in the PR body; no need to re-derive it in
the implementation stage.

## 4. Verification surface

Not applicable. `vow-perf` is pure Rust statistical/tooling code consumed
internally by the (not-yet-wired) performance-contract pipeline; it has no
`.vow` source, no contracts, no codegen, and is never passed through ESBMC or
the C model. No `tests/run/*.vow` or `examples/*.vow` fixtures need to grow.

## 5. Risk areas

- **codecov/patch (blocking gate).** All new/changed lines in this PR must be
  exercised by the test suite. The drift guard (slice 4) is deliberately a
  runtime `#[test]`, not a compile-time-only `const` assertion, specifically
  to avoid lines that execute only during `rustc`'s build and therefore show
  as new-and-uncovered under runtime-instrumented coverage. The guard
  rewrite (slice 3) is exercised by the full existing suite plus slices 1-2;
  no net-new untested branches are introduced.
- **`cargo clippy --all -- -D warnings`.** The new exhaustive `match` in
  `expected_position` and the `MAXIMUM_CANDIDATE` const should be clippy-
  clean (plain match, plain const indexing); nothing here should trip
  `clippy::match_like_matches_macro` or similar. Run clippy after each slice,
  not just at the end.
- **Binary fixed point / self-hosted compiler / `BTreeMap` vs `HashMap` /
  `vow-clif-shim` stack-slot layout.** Not applicable — no `compiler/` or
  codegen files are touched.
- **`parse → print → parse` idempotency.** Not applicable — no `vow-syntax`
  or grammar changes.
- **Behavior preservation for the slice-3 refactor.** The only logic change
  in this PR is the guard rewrite. Its correctness rests entirely on slices
  1 and 2 passing both before and after the rewrite, plus the full existing
  `vow-perf` suite staying green. Do not land slice 3 without both
  characterization tests in place first.

## 6. Out of scope

- Changing the value of `NORMALIZED_TREND_TOLERANCE`. There is no real noisy
  instrumentation data yet to tune it against; guessing a new value would be
  exactly the kind of unjustified parameter change this project's contract-
  authoring discipline warns against. Deferred to #487.
- Any real noise/measurement model for instrumented operation counts.
  Deferred to #487's later phases.
- A `strum`-style (or hand-rolled) enum-reflection crate/dependency for
  `ComplexityClass`. The exhaustive-match test in slice 4 gets the same
  compile-time safety net with zero new dependencies.
- Widening `ComplexityClass`, `CANDIDATES`, or `Sample` visibility, or adding
  any new public API surface. The issue's own framing treats the 8-class set
  as fixed today.
- Touching `vow-perf/src/instrumentation.rs` or the instrumentation tests —
  the issue is scoped entirely to the classification core in `lib.rs`.
- Any `compiler/` or `docs/spec/*.md` change — confirmed not needed (Section 2).
- Bundling unrelated formatting, refactors, or cleanups into this PR.
- Closing #487 or any other issue. The PR body should itemize the
  disposition of all four #1009 points individually (3 and 4 fixed/guarded
  in code, 1 documented only, 2 already fixed and now cited with its
  existing commit/test) rather than silently dropping item 1 behind a bare
  "Closes #1009".
