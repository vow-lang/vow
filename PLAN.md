# Plan: #1366 — self-hosted parameter where-refinement offset anchor parity

## 1. Problem restated

A parameter's inline `where` refinement (e.g. `fn f(x: i64 where x >= 0) -> i64`) is reported
in `vow contracts` JSON as `kind: "requires"`. The Rust compiler anchors this entry's
`source.offset` on the **parameter name**'s byte offset (`vow-ir/src/lower/vow.rs:221-246`,
`lower_param_refinements`, via `param.span.start` — `param.span` starts at the name token per
`vow-syntax/src/parser/mod.rs:383-404`). The self-hosted compiler instead anchors on the
refinement **predicate expression**'s own span (`compiler/lower.vow:5530`,
`span_unpack_start(expr_span(a, ref_eid))`) — a third anchor, matching neither the parameter-name
rule nor the keyword-anchor rule #1357/#1367 already gave self-hosted `vow { ... }` clauses. This
divergence is currently *documented as intended future work* in `docs/spec/cli.md:462-468` and
`docs/spec/contracts.md:375-381`. The fix makes self-hosted match Rust's parameter-name anchor,
which requires threading a parameter-name span through the self-hosted parser's per-parameter
list and every one of its consumers.

The self-hosted parser's `parse_params` (`compiler/parser.vow:472-499`) currently packs each
parameter as a 3-element stride in a flat `Vec<i64>`: `(name_sid, tid, refinement_eid)`. Only
`parse_params` produces this list (confirmed: `parse_params` has exactly two call sites,
`compiler/parser.vow:298` and `:346`, both ultimately feeding `arena_add_fn`; there is no separate
method- or extern-param-parsing path). Seven call sites across three files consume it at stride 3;
all seven must move to stride 4 together, or a half-converted tree misreads any function with 3+
parameters — which includes most of `compiler/*.vow` itself, so a partial conversion breaks
`scripts/bootstrap.sh`, not just this feature.

## 2. Files to touch

**Self-hosted compiler (`compiler/`) — the only compiler whose production code changes:**

- `compiler/parser.vow:472-499` (`parse_params`) — capture the parameter name token's span
  *before* `expect_ident` consumes it, push it as a 4th element per parameter.
- `compiler/checker.vow:678-696` (`register_fn`) — stride 3→4 (reads indices 0,1 only; no new
  field needed here).
- `compiler/checker.vow:881-917` (`check_fn`) — stride 3→4 (reads indices 0,1 only).
- `compiler/complexity.vow:930-969` (`cx_analyze_contract`) — stride 3→4 (reads index 2,
  `refinement_eid`, unchanged offset).
- `compiler/complexity.vow:1393-1403` (`cx_analyze_fn`) — stride 3→4 (count only); update the
  stale "Each parameter occupies 3 list slots" comment to say 4.
- `compiler/lower.vow:5402-5514` (`lower_function_vow`, param-declaration loop) — stride 3→4
  (reads indices 0,1 only).
- `compiler/lower.vow:5516-5538` (`lower_function_vow`, refinement loop) — stride 3→4, read the
  new index-3 (name span) field, and change line 5530's
  `span_unpack_start(expr_span(a, ref_eid))` to `span_unpack_start(name_span)`. **This one line is
  the actual bug fix; everything else is plumbing to make the data available here.**
- `compiler/lower.vow:5574-5724` (`lower_module_vow`, function-signature collection loop) —
  stride 3→4 (reads index 1 only).

**Rust compiler (root-level crates) — no production-code change, pin-test only:**

- `vow/tests/contracts.rs` — add one integration test mirroring the existing
  `contracts_requires_offset_anchors_on_keyword_not_predicate` (added by #1367, same file) but for
  a `where` refinement, asserting the Rust binary's offset lands on the parameter name. Rust
  already implements the target behavior (confirmed by reading `lower_param_refinements` and
  `parse_param`), so this is a **characterization test**, not a red one — it documents and pins
  existing-correct behavior so a future Rust regression is caught, matching the precedent set by
  #1367's own addition to this file. This satisfies the "touch both compilers" rule without
  requiring a Rust behavior change: the language semantics are not changing, only self-hosted's
  internal bug is being fixed to match Rust's already-correct output.

**Docs (spec is the source of truth — must change in the same session as the behavior):**

- `docs/spec/cli.md:462-468` — rewrite the paragraph: both compilers now anchor the parameter
  `where`-refinement's `requires` entry on the parameter name's byte offset.
- `docs/spec/contracts.md:375-381` — same rewrite (duplicate paragraph, same wording).
- Regenerate derived artifacts after editing the two files above:
  ```bash
  uv run python scripts/generate_help.py          # rewrites vow/src/skill.rs, compiler/main.vow's
                                                    # 4 skill_bundle()/skill_support_content_*()
                                                    # sites, skills/vow/reference/{cli,contracts}.md
  uv run python scripts/generate_help.py --check   # must report clean after the above
  ```
  (This is exactly the set of generated files #1367 touched for the analogous keyword-anchor doc
  change — confirmed via `git show d363e9ff --stat`.)
- `scripts/full_test.sh:1512-1517` — the `quality_fixture` block's comment explicitly documents
  *why* `where` refinements are excluded from that fixture ("the self-hosted compiler does
  not yet track a parameter-name span... keep such fixtures out of `quality_fixture`"). That
  factual premise becomes false once this lands; reword it to state the new parity coverage lives
  in a separate fixture/block (added below) rather than implying the gap is still open. Do **not**
  add a `where` refinement into `quality_fixture` itself — its keyword-prefix check
  (`needle = (kind + ':').encode()`) is specifically wrong for parameter refinements, by design,
  and its `expected = {'weak': 6, 'tautological': 2, 'substantive': 7}` counts are pinned to its
  current contents.

**New test fixture:**

- `tests/fixtures/contracts/where_refinement_offsets.vow` — new, minimal, no vow blocks (so its
  `kind: "requires"` entries are *only* the parameter refinements, keeping the parity check
  simple). See slice 2 below for exact contents.

**Explicitly checked and found not to need changes:**
- No `known-divergence` fixture anywhere references this gap (grepped `tests/`, `scripts/`; the
  only hits are the harness's own directive-handling code in `full_test.sh`, not a fixture using
  it for this issue).
- No `tests/run/*.vow` or `tests/debug/*.vow` fixture exercises `where`-refinement offsets (the
  few files matching `where` are incidental prose in comments, not refinement clauses).
- `scripts/parity.py`'s `where`/`offset` handling is about **diagnostic** spans (parse/type
  errors), an unrelated code path; not touched.

## 3. TDD slices

1. **Rust characterization test (no production change).** Add
   `contracts_where_refinement_offset_anchors_on_param_name` to `vow/tests/contracts.rs`, built
   against `vow_bin()` (the Rust binary). Source: `fn f(n: i64 where 0 <= n) -> i64 { n }`.
   Assert `c["kind"] == "requires"` and the offset equals `source_text.find("n: i64")` (the
   parameter-name position). This passes immediately — it is a pin on already-correct Rust
   behavior, not a red step.

2. **Add the new fixture and its `full_test.sh` parity block — RED.**
   `tests/fixtures/contracts/where_refinement_offsets.vow`:
   ```vow
   module WhereRefinementOffsets

   // 2 params, refinement on the 2nd (the common case). Predicate "0 <= n"
   // deliberately does not start with the param name, so a check that just
   // matched "starts with param name" couldn't pass by coincidence with the
   // old (predicate-anchored) offset.
   fn clamp_nonneg(bound: i64, n: i64 where 0 <= n) -> i64 {
     if n <= bound { n } else { bound }
   }

   // 3 params, refinement on the 3rd (index 2). Pins stride-4 indexing past
   // the first two parameters, where a stray /3 or *3 left in one consumer
   // would misread an adjacent slot instead of the name span.
   fn clamp3(lo: i64, hi: i64, n: i64 where 0 <= n) -> i64 {
     if n < lo { lo } else if n > hi { hi } else { n }
   }

   fn main() -> i32 { 0 }
   ```
   New block in `scripts/full_test.sh`, immediately after the existing `contract-quality/parity`
   block (before "Section 9: Bootstrap Triple Test"), named
   `"where-refinement/offset-parity"`. It runs `$RUST contracts` and `run_self contracts` on the
   new fixture and a Python check that:
   - Filters `d['contracts']` to entries whose `description` contains the substring
     `"(where on parameter"` (the marker both compilers already emit — confirmed in
     `vow-ir/src/lower/vow.rs`'s `lower_param_refinements` and `compiler/lower.vow:5522-5526`) —
     this isolates parameter-refinement `requires` entries from any other kind.
   - Asserts the filtered count is exactly 2 for both compilers (so an empty/filtered-to-nothing
     result can't silently pass).
   - For each such entry, extracts the parameter name from the description's
     `"(where on parameter <name>)"` suffix and asserts
     `fixture_bytes[offset:].startswith((name + ':').encode())` — mirroring the existing
     `(kind + ':')` keyword-prefix check's style, and robust to a predicate whose first token
     happens to equal the param name, because only the true name occurrence in `name: type` is
     immediately followed by `:`.
   - Asserts Rust's and self-hosted's `(function, offset)` tuples for these entries are equal
     (cross-compiler parity, not just "both individually look plausible").

   Run this block against the current (unfixed) `build/vowc`: it must **fail**, since self-hosted
   still anchors on the predicate. This confirms the test actually exercises the bug before any
   production code changes.

3. **Layout-only migration — behavior-neutral, green on bootstrap.** In one slice, change
   `compiler/parser.vow`'s `parse_params` to push a 4th element (name span) and update all seven
   stride-3 consumers (`checker.vow` ×2, `complexity.vow` ×2, `lower.vow` ×3) to stride 4,
   *without* changing which index `lower.vow:5530` reads (still `expr_span(a, ref_eid)`, just at
   the new stride). Capture the span the same way `parse_vow_block` already does for clause
   tokens (`compiler/parser.vow:511`): `let name_tok: Token = peek(p); let name_span: i64 =
   span_pack(name_tok.span_start, name_tok.span_len);` — called *before* `expect_ident(p)`
   advances past the token. Green bar for this slice: `scripts/bootstrap.sh` completes to a
   binary fixed point (stage A/B/C triple test, `sha256sum` match) and `scripts/full_test.sh`
   passes *except* the new slice-2 block, which must still fail identically (proving this slice
   changed nothing observable). This isolates "did I wire the plumbing correctly" from "did I fix
   the bug" — a half-converted tree would show up here as a bootstrap crash on compiling
   `compiler/*.vow` itself (which has many 3+-parameter functions), not as a subtle offset
   mismatch later.

4. **The actual fix — GREEN.** Change `compiler/lower.vow:5530` (now at the stride-4 index) from
   `span_unpack_start(expr_span(a, ref_eid))` to `span_unpack_start(name_span)`, where `name_span`
   is the newly available 4th list element. Re-run slice 2's `full_test.sh` block: it must now
   pass. Re-run the full `scripts/full_test.sh` and the bootstrap triple test to confirm nothing
   else regressed.

5. **Docs and generated artifacts.** Edit `docs/spec/cli.md:462-468` and
   `docs/spec/contracts.md:375-381` to state both compilers anchor parameter `where`-refinement
   `requires` entries on the parameter name's byte offset (no remaining divergence). Edit the
   stale comment in `scripts/full_test.sh:1512-1517` and the "3 list slots" comment in
   `complexity.vow:1401`. Run `uv run python scripts/generate_help.py` then `--check`; rebuild
   (`cargo build --release -p vow`, `scripts/bootstrap.sh --skip-cargo`) to confirm the
   regenerated skill/help text is consistent and nothing else broke.

## 4. Verification surface

This change touches only: parser output shape (an internal `Vec<i64>` layout, not surface
syntax), and one offset-computation line in codegen-adjacent lowering. It does **not** touch:
- Contract semantics (`requires`/`ensures`/`invariant` predicates, blame rules) — unaffected.
- The IR, Cranelift codegen, or the C/verification model — `source.offset` is diagnostic/JSON
  metadata consumed by `vow contracts` and `VowViolation` reporting, not by ESBMC or codegen.
- No new ESBMC properties are needed. No new `tests/run/*.vow` or `examples/*.vow` fixtures are
  needed for the verification pipeline itself — the only new fixture
  (`tests/fixtures/contracts/where_refinement_offsets.vow`) exists purely to drive `vow contracts`
  JSON comparison in `full_test.sh`, mirroring the existing `quality_shapes.vow` fixture's role.

## 5. Risk areas

- **Binary fixed point / bootstrap.** The highest-risk step is slice 3: if any one of the seven
  stride-3 consumers is missed, `build/vowc` compiling `compiler/*.vow` (which itself contains
  many 3+-parameter functions) will misread parameter data for any affected function, surfacing
  as a bootstrap failure or — worse — a *silent* miscompile that only shows up as a later behavior
  diff. Mitigation: do slice 3 as a single atomic change covering all seven sites, and treat a
  clean `scripts/bootstrap.sh` triple-test (`sha256sum` match across stage A/B/C) as the gate
  before moving to slice 4, not just "it compiles."
- **`BTreeMap`/`HashMap` ordering, stack-slot layout (`vow-clif-shim`).** Not implicated — this
  change doesn't touch IR lowering of instructions, Cranelift codegen, or FFI shim stack slots,
  only AST-level parameter metadata and a diagnostic offset.
- **`parse → print → parse` idempotency.** Not implicated — the canonical printer doesn't emit
  `source.offset`; the new 4th list element is derived from the existing name token's span, not
  from new surface syntax, so the printer's round-trip output is unaffected. Worth a quick sanity
  check (`build/vowc` printer idempotency test suite) but not expected to need any change.
- **`cargo clippy --all -- -D warnings`.** No Rust production code changes (only a new test in
  `vow/tests/contracts.rs`); clippy on test code is still worth a quick local check since CI
  clippy excludes `--all-targets` (test-module lints aren't normally gated, but a new test should
  still be clean).
- **Off-by-stride bugs are easy to introduce quietly.** Because the fix is "change 3 to 4 in
  N places," a transposition (e.g. updating the divisor but not a multiplier, or vice versa) is
  the most likely mistake. Slice 2's fixture deliberately includes a 3-parameter function with the
  refinement on the *last* param specifically so an off-by-stride bug has a chance to manifest
  within the targeted parity test, not just in the (slower, coarser) bootstrap signal.
- **`full_test.sh` wall-clock.** Per project memory, a full run is ~40 minutes and
  `scripts/bootstrap.sh` is ~5 minutes; the implementation stage should run these in the
  foreground with explicit bounds / poll loops, never backgrounded with an idle wait (no
  asynchronous wakeup is available), and should use `VOW_CACHE_DIR=$(mktemp -d)` to avoid stale
  compile-cache artifacts masking the fix (observed previously: the compile-object cache does not
  key on compiler-binary changes at a fixed source revision).

## 6. Out of scope

- Any change to whether/how parameter `where`-refinement expressions are **type-checked** for
  bool-ness or single-parameter-reference — `compiler/checker.vow:902-917`'s param loop doesn't
  currently read `refinement_eid` at all (only indices 0 and 1); this plan doesn't change that,
  only the stride it uses to reach those same two indices. If that's a real gap, it's a separate
  issue.
- Reformatting or refactoring any of the seven touched consumer sites beyond the mechanical
  stride-3→4 change (e.g. extracting a shared `param_name_span(a, params_lid, i)` accessor
  function). The existing code already inlines `list_get(a, params_lid, i*3+k)` everywhere rather
  than using per-field accessor helpers (unlike `vow_clause_*`, which does have them); this plan
  follows the existing convention rather than introducing a new abstraction layer as a drive-by.
- Widening `tests/fixtures/contracts/quality_shapes.vow` itself to include a `where` refinement —
  its keyword-anchor check is categorically wrong for parameter refinements; a new, separate
  fixture is used instead (see Files to touch).
- Any Rust-side behavior change — Rust already implements the target anchor rule; only a
  characterization test is added there.
- The pre-existing `description` divergence tracked by #1113 (cast rendering: `as <type>` vs
  `as i64`) — unrelated, already explicitly scoped out of the adjacent parity block's `description`
  comparison, and this plan's new block avoids comparing `description` verbatim for the same
  reason (it only extracts the parameter name from it via substring, not equality).

## Handoff

This file is a stage-handoff artifact. The implementation stage will `git rm PLAN.md` before
opening the PR. Suggested PR title (fits the ≤100-char header budget with ` (#N)` appended, ~91
chars): `fix(lower): anchor self-hosted where-refinement source.offset on param name`.
