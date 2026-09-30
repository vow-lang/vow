# Plan: #1357 — align self-hosted contract `source.offset` to the clause keyword

## 1. Problem restated

For `requires`/`ensures`/`invariant` clauses inside a `vow { ... }` block, the Rust
compiler reports `VowEntry.offset` (and therefore `vow contracts`'s `source.offset`,
and every downstream `VowViolation`/counterexample offset) as the byte offset of the
clause **keyword** token (`vow-syntax/src/parser/mod.rs:452` captures
`clause_start` before matching on `KwRequires`/`KwEnsures`/`KwInvariant`, then
`vow-ir/src/lower/vow.rs:208/256/276` passes `span.start` — the keyword's start —
into `alloc_vow`). The self-hosted compiler instead derives the same offset from
the **predicate expression's own span** (`compiler/lower.vow:5288/5315/5341`,
`span_unpack_start(expr_span(a, clause_eid))`), which starts 9–10 bytes later
(the length of `"requires: "`/`"ensures: "`/`"invariant: "`). This is pure
self-hosted lowering bug, not a parser gap: the self-hosted parser already
captures the correct keyword-anchored span per clause
(`compiler/parser.vow:507`, `span_pack(ct.span_start, ct.span_len)` where `ct`
is the clause keyword token) and stores it in the AST arena
(`compiler/ast.vow:441-447`, third slot of `a.vow_data`) — lowering just never
reads it back, reaching instead for the predicate's span. The fix is to anchor
on the clause keyword (matching Rust, the already-correct reference behavior),
document that as the spec's intended anchor, read the AST's existing clause
span in `lower.vow` instead of the predicate's span, and widen the
`contract-quality/parity` case in `scripts/full_test.sh` to assert the two
compilers now agree on `source.offset`, not just on `(function, kind, quality)`.

## 2. Files to touch

**Docs (spec is source of truth — update first):**
- `docs/spec/cli.md` — `vow contracts` "Contract Fields" table (`source` row,
  ~line 457): add one sentence pinning the anchor rule, scoped explicitly to
  clauses inside a `vow { ... }` block (`requires`/`ensures`/`invariant`):
  `offset` is the byte offset of the clause keyword, not the predicate. Do
  **not** word it as covering every `requires`-kind entry — a parameter's
  inline `where` refinement also reports `kind: "requires"` but anchors on the
  **parameter name**, not the keyword (see Out of scope). State that anchor
  too, and note that self-hosted does not yet match it there.
- `docs/spec/contracts.md` — "Interpreting Counterexamples" field table
  (`source` row, ~line 372): same clarification, since the same `offset` value
  feeds `VowViolation`/counterexample JSON, not just `vow contracts`.
- **Regenerate immediately after editing, in the same slice — not later.**
  Both table rows above are copied verbatim into generated help/skill text:
  `docs/spec/cli.md`'s `source` row is byte-for-byte duplicated at
  `compiler/main.vow:5274` and `vow/src/skill.rs:3123`/`8359`;
  `docs/spec/contracts.md`'s is duplicated at `compiler/main.vow:5815` and
  `vow/src/skill.rs:3664`/`8901`. Editing the spec alone leaves these stale
  and `scripts/check_help_coverage.py` (run by `full_test.sh`) will fail on
  the drift. Run `uv run python scripts/generate_help.py` right after the doc
  edit — this rewrites `compiler/main.vow` and `vow/src/skill.rs` in place —
  then `cargo build --release -p vow` and `scripts/bootstrap.sh --skip-cargo`
  to rebuild both compilers against the regenerated text.

**Self-hosted compiler (`compiler/`):**
- `compiler/ast.vow` — add a `vow_clause_span(a: AstArena, cid: i64) -> i64`
  accessor next to the existing `vow_clause_kind`/`vow_clause_expr`
  (~line 449-450), reading `a.vow_data[cid * 3 + 2]` (the span already stored
  by `arena_add_vow_clause`, currently write-only).
- `compiler/lower.vow` — in `lower_requires_clauses` (~line 5288),
  `lower_ensures_clauses` (~line 5315), `lower_invariant_clauses` (~line 5341):
  replace `span_unpack_start(expr_span(a, clause_eid))` with
  `span_unpack_start(vow_clause_span(a, cid))`. Three call sites, same
  one-line change each. `cid` (the clause's own AST id) is already in scope in
  all three loops.

**Rust compiler — test only, no production change:**
- Rust's `alloc_vow` call sites already anchor on the clause keyword
  (`span.start` where `span` is `clause_start.merge(expr.span)` and
  `clause_start` is captured before the keyword token in
  `vow-syntax/src/parser/mod.rs:452`). No production code changes in `vow-ir`
  or `vow-syntax`.
- `vow/tests/contracts.rs` — add an integration-level pin of the anchor
  decision through the public `vow contracts` JSON, mirroring the existing
  `contracts_json_has_all_entry_fields`/`contracts_quality_classifies_clause_shapes`
  style (`run_contracts` helper, ~line 301): write a small temp fixture with a
  `requires:` clause at a known column, run `vow contracts` against it, and
  assert the exact `source.offset` integer — today that test only asserts
  `is_some()`. This is the only new test; skip a `vow-ir`-internal unit test
  (the existing `#[cfg(test)]` fixtures in `vow-ir/src/lower/vow.rs` hand-build
  ASTs with every span collapsed to `sp() == (0,1)` via a 10-argument
  `lower_function` call, so they can't distinguish keyword- from
  predicate-anchored offsets without real work to route real spans through —
  not worth it when the integration test already exercises the real parser).

**Test harness:**
- `scripts/full_test.sh` — Section 8c `contract-quality/parity` (~line
  1470-1522): widen the compared tuple from `(function, kind, quality)` to
  include `source.offset` (or add it as a second, explicit comparison), and
  delete the now-stale bullet about the `source.offset` divergence in the
  comment block (~line 1479-1481) — keep the `description` (`as <type>`,
  #1113) bullet, since that divergence is untouched by this issue.

## 3. TDD slices

1. **Spec decision, scoped precisely (docs + regeneration).** Update
   `docs/spec/cli.md` and `docs/spec/contracts.md`'s `source` rows to state:
   for a clause inside a `vow { ... }` block (`requires`/`ensures`/
   `invariant`), `offset` is the byte offset of the clause keyword, not the
   predicate; for a parameter's inline `where` refinement (also
   `kind: "requires"`), Rust anchors on the parameter name and self-hosted
   does not yet match (tracked by the follow-up issue filed in slice 6). Then run
   `uv run python scripts/generate_help.py` (rewrites the duplicated table
   rows inside `compiler/main.vow` and `vow/src/skill.rs`),
   `cargo build --release -p vow`, and `scripts/bootstrap.sh --skip-cargo`.
   Confirm `python3 scripts/check_help_coverage.py` (or a full
   `scripts/full_test.sh` run later in slice 5) sees no drift. No production
   *behavior* changes in this slice — it's the "decide the anchor" step the
   issue asks for, landing before the code slices so the tests below encode a
   documented decision rather than an implicit one.
2. **Rust characterization test (pins already-correct behavior).** In
   `vow/tests/contracts.rs`, add a test (alongside
   `contracts_quality_classifies_clause_shapes`'s temp-fixture style) that
   writes a small `.vow` file with a `requires:` clause at a byte offset
   computable from the literal source string (e.g. via `.find("requires")`),
   runs `vow contracts` on it, and asserts `source.offset` equals that exact
   integer — not the predicate's offset. Expected result: **passes
   immediately**, since Rust already anchors correctly; this is a regression
   lock on the decision from slice 1, not a red step.
3. **Self-hosted RED: widen the parity case, and make it assert content, not
   just cross-compiler agreement.** Edit `scripts/full_test.sh`'s
   `contract-quality/parity` Python block (~line 1490-1516) to, for every
   contract in both the Rust and self-hosted
   `tests/fixtures/contracts/quality_shapes.vow` JSON: (a) compare
   `source.offset` between the two compilers (extend `triples` to a 4-tuple,
   or add a parallel offset-diff check), and (b) read
   `quality_fixture`'s own source text at `c['source']['offset']` and assert
   it starts with `c['kind']` (i.e. `"requires"`, `"ensures"`, or
   `"invariant"`) — this encodes the spec decision directly, the way the
   existing block already pins `expected = {...}` for `summary.quality`
   (~line 1512) rather than only checking Rust==self, and it won't pass if
   both compilers regress to the same wrong anchor. Drop the stale
   `source.offset` bullet from the comment above the block (~line 1479-1481);
   keep the `description` (#1113) bullet. Rebuilding is not required to
   observe RED: run the existing `$RUST contracts` / `run_self contracts`
   invocations already in that block against the **current** `build/vowc` and
   confirm both new checks fail (self-hosted offsets land 9-10 bytes into the
   predicate, so the keyword-prefix check fails too) before touching
   `lower.vow`.
4. **Self-hosted GREEN: fix the anchor.** Add `vow_clause_span` to
   `compiler/ast.vow` and switch the three `lower.vow` call sites listed above
   to read it instead of `expr_span(a, clause_eid)`. Rebuild with an isolated
   cache (`VOW_CACHE_DIR=$(mktemp -d) scripts/bootstrap.sh --skip-cargo`) —
   per prior-session notes, the compile cache does not key on compiler
   changes, so reusing the default cache dir here would silently serve stale
   objects/JSON and make the fix look like it didn't work. Re-run the
   Section 8c parity check in isolation — both new checks should now pass.
   Before moving on, grep `compiler/*.vow` for every other read of `.offset`
   on an `IrVowEntry`/`ve` value (not just the one site already found at
   `compiler/main.vow:14481`) to confirm nothing else — e.g.
   `compiler/module_io.vow`'s `.vmod` serialization, or any ESBMC-model /
   counterexample-mapping code — reads the pre-fix offset in a way this slice
   misses.
5. **Full regression sweep.** `cargo test --all` (only new tests added, no
   Rust production change, so no regressions expected but must still pass
   green), `cargo clippy --all -- -D warnings` (new Rust test code must be
   clippy-clean), `scripts/bootstrap.sh`, `scripts/full_test.sh` in full (not
   just Section 8c — confirm nothing else depended on the old, wrong
   self-hosted offset, and that `check_help_coverage.py` sees the regenerated
   help/skill text as fresh), and `build/vowc test` over `compiler/test_*.vow`
   (sanity: these don't assert JSON offsets but must still compile/run/verify
   cleanly after the `lower.vow` edit touches every function with a vow
   block).
6. **File the follow-up and record the decision.** `gh issue create` for the
   parameter `where`-refinement offset anchor (see Out of scope) referencing
   #1357, then `gh issue comment 1357` summarizing the anchor decision (clause
   keyword for vow-block clauses, parameter name for `where` refinements —
   still unimplemented on the self-hosted side) and linking the new issue.

## 4. Verification surface

This is a pure diagnostics/metadata change — `offset` is carried on
`IrVowEntry`/`VowEntry` and baked into codegen as a constant consumed by
`__vow_violation` and by the `vow contracts`/`verify` JSON emitters
(`compiler/main.vow:14481` is the only self-hosted reader of `ve.offset`). It
does not change:
- which properties ESBMC proves (the predicate IR lowered for verification is
  unchanged; only the diagnostic metadata attached to the `VowId` moves),
- codegen shape, instruction ordering, or `BTreeMap`/stack-slot layout in
  `vow-clif-shim` (the offset is plain data threaded through `IrVowEntry`, not
  an instruction or slot),
- `parse → print → parse` idempotency (no AST or printer change; the fix only
  changes which already-stored span field lowering reads).

No new ESBMC-facing properties and no new `tests/run/` or `examples/`
fixtures are needed. The existing `tests/fixtures/contracts/quality_shapes.vow`
fixture already has non-trivial `requires`/`ensures` clauses at multiple byte
offsets, which is exactly what the widened parity case needs — no new
verification fixture required.

## 5. Risk areas

- **Downstream offset consumers.** `VowViolation` (runtime, debug mode) and
  CEGIS counterexample JSON both read the same `offset` value this PR moves
  for self-hosted-compiled binaries. Confirmed via grep that
  `compiler/main.vow:14481` is the sole self-hosted read site, so the change
  is contained to one write site (3 call sites in `lower.vow`) and one read
  site. No other fixture in `tests/` pins a self-hosted-produced absolute
  `source.offset`/violation `offset` value (checked `tests/run_tests.sh`'s
  `TEST_CX_*` directives — the only offset-like assertion there is
  `call_sites[].offset`, which comes from `inst.ostart`/call-site instruction
  origin, a fully separate code path untouched by this change).
- **Generated help/skill text will go stale if the spec edit and
  regeneration aren't in the same commit.** `docs/spec/cli.md` and
  `docs/spec/contracts.md`'s `source` table rows are copied verbatim into
  `compiler/main.vow` (~lines 5274, 5815) and `vow/src/skill.rs` (~lines
  3123/8359, 3664/8901) by `scripts/generate_help.py`. Editing only the spec
  files would leave those four generated sites stale and fail
  `scripts/check_help_coverage.py`'s drift check in `full_test.sh`. The
  numeric example values embedded alongside them (`"offset": 42`,
  `"offset": 76`, etc.) are separate, hand-written illustration strings, not
  computed from actually compiling the referenced example files — those are
  genuinely unaffected and don't need touching.
- **Do not weaken the `contract-quality/parity` case's existing scope.** The
  comment at `scripts/full_test.sh:1475-1481` documents two *separate* known
  divergences (`description`'s `as <type>` rendering, #1113; and this
  `source.offset` one). Only remove the bullet this issue closes — leave the
  `description` divergence and its narrower tuple comparison in place so a
  real #1113 regression isn't silently masked.
- **Fixed point / clippy.** No Cranelift, stack-slot, or `BTreeMap` code is
  touched; no self-hosted binary fixed-point risk beyond the normal one-line
  `lower.vow` semantic change (verified by the existing triple-bootstrap
  test in CI, unaffected by this plan). The only new Rust code is test-only,
  so `cargo clippy --all -- -D warnings` risk is limited to keeping the new
  test idiomatic (no `unwrap()` in non-test-appropriate places, etc. — tests
  are exempt from the CI clippy gate per
  `vow-clippy-gate-excludes-test-targets`, but keep it clean anyway since
  local `cargo clippy --all-targets` runs may still flag it for reviewers).

## 6. Out of scope

- **Parameter `where`-refinement offset anchor.** Rust anchors a parameter's
  inline refinement `requires` (`lower_param_refinements`,
  `vow-ir/src/lower/vow.rs:235`) on `param.span.start` — the byte offset of
  the **parameter name**, not the `where` keyword or the refinement
  predicate. The self-hosted parser (`compiler/parser.vow:468-495`,
  `parse_params`) does not currently track a per-parameter span at all — only
  `(name_sid, tid, refinement_eid)` triples — so `compiler/lower.vow:5480`
  falls back to the refinement expression's own span
  (`expr_span(a, ref_eid)`), which is a *third*, different anchor. Fixing this
  requires adding a fourth field (span) to every `params_lid` triple and
  updating every stride-3 consumer (`compiler/checker.vow:688/909`,
  `compiler/complexity.vow:946`, `compiler/lower.vow:5401-5468/5668`) to
  stride 4 — a materially larger, more invasive change than the three
  one-line `lower.vow` fixes above, and not exercised by the issue's own
  repro (`tests/fixtures/contracts/quality_shapes.vow` has zero `where`
  clauses). Bundling it here would violate "many small changes beat one large
  change." TDD slice 6 files a follow-up issue via `gh issue create` (not a
  vague "once this PR lands" deferral) scoped specifically to
  parameter-refinement offset parity, and records the decision on #1357 via
  `gh issue comment`.
- **`description` field parity (#1113).** The self-hosted printer renders a
  cast as `" as <type>"` vs. Rust's `" as i64"`; unrelated to offsets, already
  tracked separately, left untouched.
- **Any refactor of `lctx_alloc_vow`'s signature, `IrVowEntry`'s shape, or the
  `vow_data` flat-array encoding.** The existing 3-slot-per-clause layout
  already has room for what's needed (the span slot exists, just wasn't
  read); no schema change required or planned.
- **Widening `contract-quality/parity` to a full `compare_json`.** The
  existing comment already flags this as a followup once both known
  divergences (`description`, `source.offset`) are fixed; only
  `source.offset` is fixed here, so `description` stays excluded and the full
  `compare_json` widening stays a separate future step.
