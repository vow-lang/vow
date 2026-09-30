# Plan: #1120 — Unsigned sizes seam 5/7, Phase A corpus (tests/, stdlib/, examples/, benchmarks/)

## 1. Problem restated

`Vec::len()`/`String::len()`/`HashMap::len()` still return `i64`, but ADR 0003 has already
reversed that to `u64` and the two compiler-side prerequisites this seam was blocked on —
the canonical descending-loop idiom (`docs/spec/grammar.md#descending-loops`, verified present)
and the `TautologicalComparison` diagnostic (verified present in `vow-types/src/check.rs`,
`vow-diag/src/lib.rs`, `compiler/diag.vow`, `compiler/main.vow`) — are both landed. This seam
migrates the *internal* index/length/capacity locals of `tests/`, `stdlib/`, `examples/`, and
`benchmarks/` to `u64` under the Phase-A cast-bridge rule (retype internals, bridge at the
`v.len()` call site with `as u64`, never change an exported signature), and deletes exactly the
`>= 0` contract clauses that the retype turns into a compiler-rejected `TautologicalComparison`.
Every clause that still compiles — because its subject is a parameter that stays `i64` under the
internals-only rule, or is discharged by a sibling clause — is left alone. Two descending-loop
invariants (`stdlib/math/vec_math.vow`, `examples/sat/solver.vow`) carry the corpus's only
negative literals inside contract clauses and must be rewritten to the canonical guard-on-`>0`,
decrement-first idiom rather than mechanically retyped, because a `-1` literal against a `u64`
subject is a hard `LiteralOutOfRange` error. The work is a pure source-level retype-and-delete
refactor with existing tests as the safety net — it touches no compiler, no IR, no codegen, and
no runtime code in either the Rust or self-hosted compiler.

## 2. Files to touch

No `vow-*` crate or `compiler/*.vow` module changes — this seam is corpus-only by definition
(blocked-by/blocks relationship in the issue confirms `compiler/` is a separate, independent seam).
Files touched are exclusively under:

- `stdlib/bignum/bignum.vow` (own PR, alone)
- `stdlib/gc/gc.vow`, `stdlib/math/vec_math.vow`, `stdlib/heap/min_heap.vow`,
  `stdlib/heap/max_heap.vow`, `stdlib/stack/stack.vow`, `stdlib/geometry/shape.vow`
  — plus their `tests/multi/` mirrors (`tests/multi/stack/stack.vow`,
  `tests/multi/geometry/shape.vow`) in the *same* commit, since they are byte-identical copies
- `examples/*.vow` excluding `sat/*` and `search.vow` — `examples/cmdloop.vow`,
  `examples/streaming_file/*.vow`, `examples/vec_*.vow`, `examples/map_*.vow`, and the remaining
  ~17 files with `.len()`/index hits (exact list to be re-derived fresh at PR time — see §5 on
  census drift)
- `examples/sat/*.vow` (`solver.vow`, `types.vow`, plus the other files in that directory) — own PR
- `benchmarks/easy/*/{reference,skeleton}.vow`, `benchmarks/medium/*/{reference,skeleton}.vow`
  (one PR), then `benchmarks/hard/*/{reference,skeleton}.vow` (second PR)
- `tests/run/*.vow` (own PR)
- `tests/verify/*.vow`, `tests/verify-fail/*.vow`, `tests/verify-skip/*.vow` (own PR)
- `tests/multi/**` residual — every `tests/multi/<dir>/*.vow` **except** the 8
  `vmod_*/module_io.vow` copies (still byte-identical to the unmigrated `compiler/module_io.vow`
  as of HEAD — confirmed via `md5sum`, so they move with the `compiler/` seam, not here),
  `tests/multi/bignum_legacy/bignum.vow` (frozen per its own header, issue #841), and the two
  stdlib mirrors already folded into the "stdlib rest" PR above (own PR)

`docs/spec/*.md` changes: **none expected.** The only documented contract whose subject is a
migrated local is `vec_reverse`'s entry in `docs/spec/stdlib.md:174`
(`` ensures result.len() == v.len() ``) — that wording is unchanged by an internals-only retype
(the visible contract is over the unmigrated parameter `v`, not the internal loop index `i`), so
no regeneration is needed. If any batch turns out to touch a documented signature or contract
wording after all, the closing step of that PR is
`uv run python scripts/generate_help.py` → `cargo build --release -p vow` →
`scripts/bootstrap.sh --skip-cargo`, per the issue's acceptance criteria — treat this as a
contingency, not a default step.

## 3. TDD slices

This is a refactor-under-characterization-tests task, not new-feature development: the tests
already exist (452+ fixtures across the four directories) and are the safety net. Each slice
below is "retype, let the compiler enumerate the now-tautological clauses, delete exactly those,
re-run the full corpus" rather than a classic red-green-refactor cycle, because there is no new
behavior to assert — the retype must be behavior-preserving by construction. Slices are ordered
by risk (lowest blast-radius / most mechanical first) so early PRs validate the workflow before
the two semantically-sensitive ones (`vec_math.vow`'s invariant rewrite, `sat/solver.vow`'s
non-mechanical loop restructure).

1. **`stdlib/bignum/bignum.vow`** (own PR — isolated because of the sign/`Ordering` `-1` values).
   - Before touching anything: `build/vowc verify stdlib/bignum/bignum.vow` and whatever
     `tests/` fixtures exercise it, captured as the baseline JSON/exit code.
   - Retype internal `len`/index/capacity locals to `u64`; bridge every `.len()` call site with
     `as u64`. Do **not** touch `sign: i64` (bounded `-1..=1`, documented invariant at
     `docs/spec/stdlib.md:301`) or the `Ordering`-shaped `-1`/`0`/`1` return values (`:215, 236,
     244, 1034, 1048`) — these are semantic values, not length-shaped, and retyping them changes
     program meaning with no compile error.
   - Rewrite the five descending loops (`:148, 239, 519, 556, 825` per the issue's line census —
     re-locate by content match, not line number, since the file has likely drifted since the
     issue's `2feeb7d6` census) to the canonical `while i > 0 { i = i - 1; ... }` idiom.
   - Audit `bignum.vow:146-147`'s `chunks[clen - 1]` / `let mut i: i64 = clen - 2;` for
     underflow once `clen` is `u64` — confirm both are guarded by a `clen > 0` (or stronger) check
     before subtracting, or restructure to decrement-first if not.
   - Compile, let `TautologicalComparison` enumerate now-dead `>= 0` clauses, delete exactly
     those. Re-run the file's `tests/` fixtures; JSON/exit code must match the captured baseline.

2. **`stdlib` rest** — `gc/gc.vow`, `math/vec_math.vow`, `heap/min_heap.vow`,
   `heap/max_heap.vow`, `stack/stack.vow`, `geometry/shape.vow` + `tests/multi/stack/stack.vow`
   and `tests/multi/geometry/shape.vow` in the same commit (keep the mirrors byte-identical to
   their `stdlib/` source after the edit — re-`md5sum` to confirm).
   - `gc.vow:37,162` (`fl.len() - 1`, `wl.len() - 1`) are already guarded by `if fl.len() > 0` /
     `while wl.len() > 0` — confirmed by reading the file; the retype is mechanical here, no
     restructure needed.
   - `vec_math.vow`'s `vec_reverse` is the first of the two canonical-idiom rewrites: replace
     `let n: i64 = v.len(); let mut i: i64 = n - 1; while i >= 0 { invariant: i >= -1, invariant:
     i < n } { ...; i = i - 1; }` with `let n: u64 = v.len() as u64; let mut i: u64 = n; while i >
     0 vow { invariant: i <= n } { i = i - 1; out.push(v[i]); }` (decrement-first so the guard
     doubles as the bounds check, matching `grammar.md:625-632` verbatim). The documented contract
     `ensures result.len() == v.len()` is untouched (subject is the unmigrated parameter), so no
     `docs/spec/stdlib.md` edit.
   - `heap/{min,max}_heap.vow`: retype heap-index locals; verify sift-up/sift-down index
     arithmetic (`(i - 1) / 2`, `2*i + 1`, `2*i + 2`) has no underflow path once `i` is `u64` —
     these are all ascending from `0` or guarded by a `> 0`/`<` bound already, but confirm per
     function rather than assume.
   - Full `tests/` pass (including the `tests/multi/stack`, `tests/multi/geometry` main.vow
     builds under Section 6b of `full_test.sh`) must stay green.

3. **`examples`, excluding `sat/` and `search.vow`** — `cmdloop.vow`, `streaming_file/*.vow`,
   `vec_*.vow`, `map_*.vow`, and the rest of the ~17-file hit list (re-enumerate at PR time with
   the issue's grep recipe: `find examples -name '*.vow' | xargs sed -E 's://.*::; s:"[^"]*":"":g'`
   piped through the `.len()`/index/`>= 0` counts, excluding `sat/` and `search.vow`).
   - `search.vow` is explicitly excluded: it is mirrored verbatim in `docs/spec/examples.md:196-215`
     and `docs/spec/grammar.md:561-567`, and its `-1` at `:10` (`break -1;`) is a true sentinel,
     out of scope (sentinel→`Option` conversion is epic track C2).
   - Purely mechanical retype-and-delete per file; no descending-loop rewrites expected outside
     `sat/`.

4. **`examples/sat`** (own PR — densest, most semantics-sensitive file set: `solver.vow` alone is
   ~47 `.len()` sites, ~95 index sites, ~48 clauses).
   - `types.vow:10`'s `const VAL_FALSE: i64 = -1;` (three-valued logic, alongside
     `VAL_UNASSIGNED = 0`, `VAL_TRUE = 1`) stays `i64` — not length-shaped.
   - `solver.vow:612-650`'s `analyze` function is the **non-mechanical** case: `idx` seeds at
     `trail.len() - 1` and the loop at `:641-650` reads `trail[idx]` *before* decrementing
     (confirmed by reading the file — `:645 let cand: i64 = trail[idx]; :646 idx = idx - 1;`).
     Moving to decrement-first shifts which element is read on each iteration, which is a
     semantic change, not a re-encoding. Two options, decided per the issue's own guidance:
     a. Work out the conflict-analysis semantics precisely enough to restructure to
        decrement-first with the index shifted by one (e.g., seed `idx` at `trail.len()` directly
        as `u64` and read `trail[idx - 1]` then `idx = idx - 1`, or equivalently restructure to
        read-after-decrement consistently) and prove by construction (trace through a small
        example trail) that the visited sequence is unchanged; or
        b. Leave `idx` at `i64` for this function only, document why in the PR body, and migrate
        every other local in `solver.vow` to `u64`.
     Preference: attempt (a) first since it closes the seam's stated goal
     (no `>= 0`/negative-literal clause survives over a `u64` subject); fall back to (b) and say
     so explicitly if (a) cannot be verified correct by hand-tracing — per the issue, this is not
     a mechanical rewrite and getting it wrong is worse than deferring it.
   - `solver.vow:702, 706` (`.len() - k` sites) — audit for the same underflow risk as `:612`
     before retyping; these were flagged unguarded in the issue and need the same treatment.
   - Everything else in the 4-file set (`solver.vow` plus its siblings) is mechanical
     retype-and-delete once the two loops above are settled.

5. **`benchmarks` easy+medium** (one PR), then **`benchmarks` hard** (second PR).
   - Before the first edit: `uv run --project bench bench/run.py validate-references` — record
     the printed pass count out of 23 non-stretch as the recorded baseline in the PR body
     (issue's own text warns the historically-cited "80/103" baseline is stale; trust the live run,
     not the number in the issue).
   - Per the issue's explicit warning, **do not retype abstract scalar parameters** —
     `benchmarks/medium/M01_binary_search/reference.vow`'s `bisect(lo: i64, hi: i64)` is
     bisection over an abstract range, not a `Vec` index, and is the known `Verified → unknown`
     regression trap. The internals-only rule already excludes parameters; this is a reminder to
     apply it literally even when a parameter *looks* length-shaped.
   - Keep `reference.vow` and `skeleton.vow` in lockstep (identical contracts) per file.
   - Re-run `validate-references` after each batch; run `validate-references --compare` (rust vs
     self-hosted) at least once per PR. Kill criterion (epic-level, reported on #1104 not worked
     around here): a net loss of ≥2 non-stretch references to a `u64`-induced `unknown` stops the
     epic.

6. **`tests/run`** (own PR).
   - Re-derive the current hit list fresh (issue's census is at `2feeb7d6`; `tests/` has grown
     452→560 `.vow` files since, per `git diff --stat 2feeb7d6..HEAD -- tests/` — 135 files
     changed, mostly additions from unrelated seams). Mechanical retype-and-delete; the issue's
     `-1`-value exclusion table (`fs_read_line_status.vow`, `match_option_payload.vow`,
     `bitwise_lit_expr.vow`, the two `i128_*_min_by_neg_one.vow` files) still applies — confirm
     each still exists at the stated purpose before excluding it (don't trust the line numbers,
     trust the content).

7. **`tests/verify*`** (`tests/verify/`, `tests/verify-fail/`, `tests/verify-skip/`) (own PR).
   - The eight tautology-only fixtures need re-authoring, not deletion, because
     `vow/src/verification.rs:222`'s `.filter(|f| !f.vows.is_empty())` means a clauseless function
     is silently dropped from verification:
     - `tests/verify/string_param_verify.vow` — confirmed via direct read: three functions
       (`str_len_nonneg`, `vec_len_nonneg`, `map_len_nonneg`), each with only
       `ensures: result >= 0` over a `.len()` result. Re-author each with a real, still-true,
       non-tautological postcondition (candidates: relate `result` to a known input property,
       e.g. `ensures: result == <some derivable invariant>`, or convert to an equality against a
       second call — needs a concrete design decision at implementation time, not deferred to
       "figure it out then"). Preserve or deliberately drop the `// TEST: category model-drift`
       directive at `:1` — document the choice in the PR.
     - `tests/verify-fail/string_push_str_overflow.vow` — confirmed via direct read: the comment
       block (`:5-10`) explains the `ensures: result.len() >= 0` clause exists *purely* to make
       `over` a verify target so the raw C-level capacity assert (blame `none`) can be exercised.
       Re-author with a non-tautological but true `ensures` (e.g.
       `ensures: result.len() == a.len() + b.len()` under the pre-overflow model, or whatever the
       push_str contract should honestly state) that still makes `over` a target; keep the
       `// TEST: counterexample-*` directives intact since Section 4c
       (`full_test.sh` around the verify-fail loop, `~line 788`) asserts `VerifyFailed`.
     - `tests/verify-skip/vec_of_string_skipped.vow`, `vec_of_vec_skipped.vow` — confirmed via
       direct read: both comment blocks say the contract exists only to make the function a
       target for the non-modelable/Skipped path (issue #505 regression). Re-author with a
       non-tautological `ensures` that preserves the "String/Vec `get_val` result is a model
       struct" trigger condition — the contract's truth doesn't matter for these (the function
       is never actually verified, only classified as Skipped), but it must still compile and
       must still not be a bare `>= 0` tautology once the subject is `u64`.
   - `tests/verify/postcondition_correct.vow`, `tests/verify-fail/missing_precondition.vow`,
     `tests/verify-fail/cegis_broken.vow`, `tests/verify-fail/caller_requires_unchecked.vow` —
     confirmed via direct read: all four have plain `i64` scalar subjects (`x`, `y`, `n`), not
     length-shaped locals. **Untouched** under the internals-only rule. Included in the PR's file
     list only to state explicitly "read, confirmed out of scope."
   - `tests/verify/string_matches_literal_at.vow:6-7` — `requires: s.len() >= 2` stays (subject
     becomes `u64`-typed via `.len()`, but the clause `s.len() >= 2` is not a `>= 0` tautology, it
     is a real precondition); `requires: pos <= s.len() - 2` stays too (its own subject `pos`
     stays `i64` under internals-only, since `pos` is a parameter) — but it is now *load-bearing*
     for `pos >= 0`'s underflow safety in a way it wasn't before, since `s.len() - 2` no longer
     obviously fails closed the same way. Do not delete either clause as "redundant."

8. **`tests/multi` residual** (own PR) — every `tests/multi/<dir>/*.vow` except: the 8
   `vmod_*/module_io.vow` copies (excluded, moves with the `compiler/` seam), `bignum_legacy/bignum.vow`
   (frozen, issue #841), and `stack/stack.vow` + `geometry/shape.vow` (already handled in slice 2).
   After every edit in this slice, confirm `scripts/full_test.sh` Section 6b (multi-file builds,
   both compilers, JSON+runtime comparison) stays green, and specifically that the vmod reject
   fixtures still exit `134` (index-out-of-bounds trap) — these fixtures are untouched by this
   slice but share the harness, so a residual-directory regression would show up there too.

Each slice ends with the full acceptance-criteria checklist from the issue (bootstrap fixed
point, `full_test.sh`, `cargo test --all`, `cargo clippy --all -- -D warnings` as separate
commands) before moving to the next slice — these are independently revertable, always-green PRs,
not a single large branch.

## 4. Verification surface

This seam changes no IR, codegen, or the C model — it is a source-level retype exercising the
*existing* cast-bridge path (`as u64` at `.len()` call sites), which is already proven by the
epic's earlier seams. No new ESBMC property classes are introduced. What each PR's ESBMC run
must confirm, per file:

- **No `Verified → unknown`/`VerifyFailed` regression.** Every function that was `Verified` before
  the retype must still report `Verified` after, with the same or a logically equivalent (not
  weaker) contract. `scripts/full_test.sh` Section 4e (`verify_eval.py`) is the SOUNDNESS/PRECISION
  regression gate for this — no new banner.
- **The two rewritten descending-loop invariants must still verify.** `vec_reverse`'s new
  `invariant: i <= n` and (if slice 4's option (a) succeeds) `analyze`'s restructured loop
  invariant are the two places where the *proof strategy* changes, not just the syntax — these
  need explicit before/after JSON comparison, not just "it compiled."
  Per `grammar.md:645-650`, the unsigned idiom does not make an otherwise-`unknown` loop newly
  provable (ESBMC's unwind bound sits below the modelled capacity either way) — so `unknown`
  before and after for these two specifically is an acceptable, expected outcome, not a
  regression, as long as it was already `unknown` before the retype. Confirm the baseline first.
- **`tests/verify-fail/*` fixtures must still produce a `VerifyFailed` with the same
  `counterexample-vow-id`/`counterexample-fn`/`counterexample-blame` directives.** Re-authoring
  the three tautology-only fixtures (slice 7) must not change which vow-id fails or who's blamed
  — the new contract is additive context, not a replacement mechanism.
- **Both compilers must agree.** Every `full_test.sh` `compare_json` call (Sections 4b/4c/4d, 6b)
  is a Rust-vs-self-hosted differential — since neither compiler's source changes in this seam,
  any new divergence would indicate the retype exposed a latent Rust/self-hosted gap in how
  `u64` locals or `as u64` casts are handled (unlikely given prior seams already exercise this
  path elsewhere in the corpus, but not per se impossible for a code shape not previously tested,
  e.g. `u64` loop counters interacting with `HashMap`/`String` methods in ways `stdlib/` hasn't
  exercised before).
- **`benchmarks/`: `validate-references --compare`** (rust vs self-hosted) is the equivalent gate
  for the benchmark suite; run at least once per benchmarks PR.
- No fixture under `tests/run/` or `examples/` needs to *grow* — this seam is retyping existing
  correct programs, not adding new behavior, so no new positive/negative test cases are required
  beyond what already exists. The one addition is the re-authored contracts in slice 7 (still the
  same fixtures, new clause bodies, same file count).

## 5. Risk areas

- **Census drift.** The issue's file/line counts are pinned to commit `2feeb7d6`; current HEAD is
  `8b12f0ad`, 36 commits and 135 changed files ahead in `tests/` alone (`git diff --stat
  2feeb7d6..HEAD -- tests/`), pushing the raw `tests/*.vow` count from 452 to 560. All specific
  claims spot-checked in this planning session (bignum sign/Ordering values, `vec_math.vow`'s
  `vec_reverse` shape, `solver.vow`'s read-before-decrement loop at `:641-650`, the three
  tautology-only `tests/verify*` fixtures, the four plain-`i64`-scalar fixtures, the `module_io.vow`
  / `geometry/shape.vow` / `stack/stack.vow` copy relationships, `gc.vow`'s existing guards, the
  benchmarks manifest `total=40`) still hold verbatim — but line numbers in the issue should be
  treated as "approximately here, confirm by content" rather than exact, and each slice's
  implementer should re-run the issue's own `find | sed | grep -c` census recipe fresh rather than
  trust the issue's numbers, since new files added by unrelated seams may add new `.len()`/`>= 0`
  sites not in the original count.
- **`examples/sat/solver.vow`'s `analyze` loop (slice 4) is the one real semantic-restructure
  risk in this seam.** Everything else is mechanical. If hand-tracing the read-before-decrement
  restructure is not obviously correct, defer `idx` to `i64` and say so — a wrong "fix" here
  silently changes which trail literal is treated as a conflict pivot, which is a correctness bug
  in a SAT solver that a passing test suite may not catch if the existing `examples/sat` fixtures
  don't exercise the specific backtrack path affected.
- **The eight tautology-only `tests/verify*` fixtures (slice 7) require a genuine design
  decision per fixture**, not a mechanical rule — "replace `>= 0` with some other true clause" is
  underspecified until the implementer picks the actual replacement predicate. Get this wrong
  (e.g. pick a replacement that's *also* somehow tautological, or one that changes which vow-id
  is the verify target) and the fixture silently stops testing what it was meant to test, with no
  compiler error to catch it — CI would stay green while the regression test for issue #505 (or
  whichever issue the fixture protects) goes dark. Cross-check the replacement contract against
  the file's own explanatory comment before finalizing.
- **`bignum.vow`'s sign/Ordering `-1` values and `examples/sat/types.vow`'s `VAL_FALSE = -1`** are
  easy to mis-migrate if an implementer greps for `-1` and retypes mechanically rather than
  reading the exclusion table first. No compiler error catches this either (a `-1` sign assigned
  into a now-`u64` field *would* be a `TypeMismatch` and caught — but the risk is scope creep,
  e.g. accidentally retyping `sign: i64` itself because it sits next to migrated fields in the
  same struct).
- **No binary-fixed-point risk.** This seam touches no `compiler/*.vow` codegen path, no
  `vow-clif-shim` stack-slot layout, and no `BTreeMap`/`HashMap` ordering-sensitive code — the
  `scripts/concat_vow.sh` triple test is unaffected in mechanism, only re-run as a standard gate
  because `scripts/bootstrap.sh --skip-cargo` is required per-PR regardless of what changed.
- **`parse → print → parse` idempotency** is a live risk only in the trivial sense that every
  edited `.vow` file must still round-trip through the canonical printer — not expected to be an
  issue for ordinary retypes, but worth running `vowc build --no-verify` (which exercises
  parse+print) on every touched file as a cheap early check before the full gate.
- **`cargo clippy --all -- -D warnings`** is unaffected — no Rust source changes in this seam. Run
  it anyway per the issue's acceptance criteria (it's a fast, cheap confirmation, not a real risk).
- **Cross-PR ordering.** Slice 2 (stdlib rest) touches `tests/multi/stack/` and
  `tests/multi/geometry/` in the same commit as the `stdlib/` source — if slice 8 (tests/multi
  residual) is started before slice 2 lands, it must exclude those two directories explicitly to
  avoid a merge conflict or duplicate edit; sequence slice 2 before slice 8 if run within the same
  session, or re-check `stdlib/{stack,geometry}` status before starting slice 8 if resumed later.

## 6. Out of scope

- Sentinel→`Option<u64>` conversion (`examples/sat/solver.vow:4`'s `REASON_NONE = -1`,
  `examples/search.vow:10`'s `break -1;`) — epic track C2, not this seam.
- `String` byte-offset semantics — separate epic track.
- The `.len()` return-type flip itself (`i64` → `u64` on the builtin signature) — Phase B, not
  Phase A. This seam only adds `as u64` bridges at call sites; it does not change what `.len()`
  returns.
- Deleting the `as u64` cast-bridge once `.len()` itself returns `u64` — Phase C.
- `stdlib/geometry/shape.vow:37`'s `requires: w <= 4611686018427387903 - h` — a
  CLAUDE.md-banned verifier bound (exists to keep ESBMC's arithmetic model happy, not a real
  domain constraint), belongs to the checked-arithmetic seam, not here. Do not touch it, do not
  "fix" it opportunistically while editing the rest of the file.
- `examples/search.vow` — excluded from slice 3 entirely because it's mirrored verbatim in two
  `docs/spec/*.md` files; editing it drags a spec-regeneration dependency this seam doesn't need.
- `tests/multi/bignum_legacy/bignum.vow` — frozen per its own header (issue #841); must keep
  compiling and passing, nothing more.
- The 8 `vmod_*/module_io.vow` copies — move with `compiler/module_io.vow` in the `compiler/`
  seam (epic step A12), not here.
- Any refactor, formatting pass, or comment cleanup incidental to a touched file. Each slice's
  diff should be exactly: retyped locals, `as u64` bridges, deleted/rewritten contract clauses,
  and (for the two descending loops) the canonical-idiom restructure. Nothing else in a touched
  file should change, even if something adjacent looks improvable while reading it.
- Benchmark contract strengthening/weakening of any kind, including adding a bound to make a
  previously-`unknown` benchmark verify — contracts stay exactly as semantically correct as they
  were before this seam; a benchmark's verify status may only change as a side effect of a type
  becoming more precise (`i64`→`u64` eliminating a `>= 0` clause), never as a deliberate goal.
