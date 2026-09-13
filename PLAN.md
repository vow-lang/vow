# PLAN: #1275 — Harden Operation Catalogue validation and documentation integration

## 0. Status of the blocking dependencies (read this before touching any code)

`#1275` is blocked by `#1271`, `#1272`, `#1273`, `#1274`, all of which are in turn blocked by
`#1270`. As of this planning session (2026-09-13), **none have merged to `origin/main`** — no
`docs/spec/operations.json`, no generator, no catalogue infrastructure exists on `main` at all.

This repo's worktrees share one `.git`, so local (unpushed, no-PR) branches from sibling
in-flight sessions are readable via `git show <branch>:<path>` without merging them:

- `sym/vow/1270-tracer-bullet-...` has **real implementation**: `docs/spec/operations.json`,
  `docs/spec/schemas/operation-catalogue.schema.json`, `scripts/generate_ops.py` (+
  `scripts/test_generate_ops.py`), three generated `op_catalogue.rs` files
  (`vow-ir/src/lower/`, `vow-codegen/src/cranelift_backend/`, `vow-clif-shim/src/`), a
  `// GENERATE:OP_CATALOGUE:START/END` splice in `compiler/lower.vow`, a freshness gate wired
  into `scripts/full_test.sh` and `.github/workflows/ci.yml`, and
  `compiler/tests/test_op_catalogue.vow`. This plan is written directly against that code.
- `sym/vow/1271-...`, `1272-...`, `1273-...`, `1274-...` each have **only a committed `PLAN.md`**
  (no implementation), each forked independently from the same `main` tip — they do not see
  #1270's or each other's work yet. Their plans describe *intended* schema extensions
  (a `verifier_category`-ish field, an arena-routing field) that do not exist in any code today.

**This plan's slices 2–4 reference fields that exist in no merged or even branch-local code
today — only in #1271/#1274's own planning prose.** Treat every such reference as a hypothesis to
confirm, not a fact.

### Preflight (implementation stage — do this before writing any test or code)

1. `git log origin/main --grep 1270`, `--grep 1271`, `--grep 1272`, `--grep 1273`, `--grep 1274`
   (squash merges break ancestry checks — grep the squash subject, per this repo's convention).
2. If any of #1270–#1274 has **not** landed on `origin/main`: stop. Do not scaffold catalogue
   infrastructure yourself or duplicate what a sibling issue owns. Post `gh issue comment 1275`
   explaining the block and exit cleanly, per the operating contract for this run.
3. If all have landed: rebase onto `origin/main`, then re-derive every name in the binding table
   below from the actual merged schema/generator — do not trust the guessed names.
4. Confirm whether `render_abi_rs` in the merged `scripts/generate_ops.py` already dedupes by
   `runtime_symbol` (see Risk Areas — this is a real correctness bug in #1270 as written, and
   #1271 is expected to trigger it first with `int_to_string`/`i64_to_string` both mapping to
   `__vow_string_from_i64`). If it's already fixed upstream, drop slice 1b; if not, slice 1b is
   this issue's to fix, since "malformed projection" is squarely catalogue-hardening scope.

### Name-binding table (fill in "Confirmed" at preflight; do not code against "Assumed")

| Role | Assumed name | Source | Confirmed (fill at implementation) |
|---|---|---|---|
| Verifier-model category field | `verifier_category` | #1271 PLAN.md L15,25,53; #1274 PLAN.md L15,25,53,88 (both hedge it "might be spelled differently") | — |
| Verifier-model category values | `Modeled` / `NotModeled` | #1271 PLAN.md §3 (used throughout) | — |
| Arena-routing category field | (unnamed in any sibling plan; #1271 PLAN.md L283 calls it "arena variant as an optional string") | #1271 PLAN.md L283, L297-299 | — |
| Arena-routing category values | none proposed yet — likely something like `RootArenaOnly` / `CandidateArenaRouted`, matching the real fork in `vow-codegen/src/cranelift_backend.rs::routed_vec_extern` (a symbol either has no `_in_arena` twin, or has one that takes a caller-supplied region) | derived by this plan from `vow-codegen/src/cranelift_backend.rs:769-870` on current `main`, not from any sibling plan | — |
| Return-shape tag for `Option`-wrapped returns | today `ir_return_shape` is a flat enum (`I8`..`LinearPtr`); #1271 plans to extend it to "an enum with an optional element-type payload for `Option`" | #1271 PLAN.md L290-291,298 | — |

## 1. Problem restated

Once #1270–#1274 land, the Operation Catalogue (`docs/spec/operations.json` +
`docs/spec/schemas/operation-catalogue.schema.json`) will carry runtime-symbol, ABI,
return-shape, arena-routing, verifier-category, and human-readable `description` facts for the
print, query/utility, filesystem/stdin/args/stderr, and process Builtin Operation families,
generated into Rust and self-hosted projections consumed by lowering, Cranelift codegen, and (for
pure operations) the verifier's known/modelable classifiers. What is demonstrably still missing,
based on #1270's actual code and #1271/#1274's stated intentions:

- `validate_catalogue()` today (per #1270) only checks JSON-Schema conformance plus duplicate
  `surface_name` — it does not reject an inconsistent duplicate `runtime_symbol` (two entries
  claiming the same runtime symbol with different ABI/return-shape facts), does not exist yet to
  reject a `verifier_category` recorded against an operation the signature table shows as
  effectful, and cannot yet validate an arena-routing category because that field does not exist.
- `render_abi_rs` renders one Rust match arm **per catalogue entry**, keyed by `runtime_symbol`,
  with no dedup step. Two entries sharing a `runtime_symbol` (which #1271 is expected to
  introduce for `int_to_string`/`i64_to_string` → `__vow_string_from_i64`) render two identical
  match arms, which `cargo clippy --all -- -D warnings` rejects as `unreachable_patterns`. This is
  a real "malformed generated projection" bug reachable the moment two surface names legitimately
  share a runtime symbol, not a hypothetical.
- `check_grammar_presence`/`check_env_rs_presence` are hardcoded to one heading ("Print / IO")
  for the print-only tracer bullet. As more families land they must generalize across every
  catalogue-covered `docs/spec/grammar.md` `####` section, and — separately — today only check
  that a surface name's row *exists*, never that the row's prose is consistent with the
  catalogue's `description` field, so a hand-edited row can still drift from the catalogue
  silently. This is exactly the "hand-authored text drifting from the catalogue" risk named in
  the issue and in #375's audit note.
- Freshness ("`--check`") coverage exists for the two splice points #1270 introduces
  (`vow-ir`/`vow-codegen`/`vow-clif-shim` Rust files + the `compiler/lower.vow` marker block) but
  must be reconfirmed to also cover whatever second self-hosted splice point (if any) #1274 adds
  in `compiler/c_emitter.vow` for verifier-category consumption.
- Diagnostics are mostly-consistent flat strings today; nothing enforces that new checks this
  issue adds follow the same actionable shape (which entry, which field, what's wrong) as the
  existing ones.

This issue closes those gaps by hardening `scripts/generate_ops.py`'s validation and
grammar/description cross-checks, and by adding regression tests that pin the hardened
behavior — without re-architecting the catalogue schema's core shape, without changing lowering,
codegen, or verifier *behavior*, and without migrating any additional Builtin Operation families.

## 2. Files to touch

All paths below are relative to repo root; "existing per #1270" means the file/function already
exists on the branch this plan is grounded in, not on `main` yet (see §0).

- `docs/spec/schemas/operation-catalogue.schema.json` (existing per #1270) — no *new* fields
  (those belong to #1271/#1274); confirm/tighten `enum` lists for whatever arena-routing and
  verifier-category fields land, so `schema_check.py`'s existing generic enum-rejection covers
  "unknown arena-routing category" / "unknown verifier-model category" automatically. If #1271
  landed either field as a bare unconstrained `string`, adding the `enum` *is* this issue's job.
- `scripts/generate_ops.py` (existing per #1270):
  - `validate_catalogue()` — add: (a) invalid-duplicate-`runtime_symbol` detection; (b)
    verifier-category-vs-effect-emptiness cross-check, including a hard error for an
    unrecognized effect token (see slice 2); (c) call into new arena-routing and
    description-consistency checks (below).
  - `render_abi_rs()` — dedupe by `runtime_symbol` before emitting match arms (slice 1b).
  - `check_grammar_presence()` — generalize from one hardcoded heading to searching the union of
    all catalogue-relevant `####` headings in `grammar.md`; extend to compare row prose against
    the catalogue `description` field.
  - New `check_arena_routing(entries, runtime_rs_text)` — cross-checks an entry's arena-routing
    category against whether `vow-runtime/src/lib.rs` actually exports a `<runtime_symbol>_in_arena`
    twin.
  - `main()` — add a `--catalogue PATH` override flag (defaults to the real
    `docs/spec/operations.json`) so tests can exercise the real CLI/`--check` path against a
    fixture file (slice 9) instead of only calling private functions.
  - Error-message formatting — align to `scripts/schema_check.py`'s existing path-style
    (`"<path> is <value>, expected ..."`) for anything this issue newly emits; do **not** modify
    `schema_check.py` itself (it is shared parity-gate infrastructure with its own docstring
    contract — out of scope, see §6).
- `scripts/test_generate_ops.py` (existing per #1270) — new fixtures/tests for every check added
  above (see §3's slices for the exact list). Follow the existing inline-`FIXTURE`-dict pattern;
  no new fixture files needed except where slice 9 requires a real on-disk JSON file for the CLI
  path.
- `scripts/full_test.sh` / `.github/workflows/ci.yml` — confirm `generate_ops.py --check` is
  already gated (per #1270); if #1274 adds a second self-hosted splice point, extend whatever
  freshness invocation covers it. Expected to be a small addition, not a new gate — the gate
  itself is #1270's responsibility, already delivered.
- `docs/spec/grammar.md` — no prose changes from this issue; confirm heading names/table shapes
  the generalized `check_grammar_presence` depends on are stable. Row content stays with
  #1270–#1273 (they own the operations being documented); this issue hardens the *check*.
- `compiler/tests/test_op_catalogue.vow` (existing per #1270) — extend only if self-hosted code
  independently interprets catalogue-derived facts in a way Python-side validation can't reach
  (expected: no, since validation runs at generation time against the JSON source, before any
  `.vow` file is touched — confirm during implementation and drop this file from scope if so).

No changes anticipated in `vow-ir/`, `vow-codegen/` (beyond the `render_abi_rs` dedup fix, which
is generator-side, not `vow-codegen`-side), `vow-clif-shim/`, `vow-verify/`, `compiler/lower.vow`,
or `compiler/c_emitter.vow` — this issue does not change lowering, codegen, or verifier
*behavior*, only the catalogue's own validation and the freshness/doc-consistency gates around
it.

## 3. TDD slices

### AC → slice coverage map

| Issue acceptance criterion | Slice(s) | Note |
|---|---|---|
| Rejects duplicate surface names | 0 (regression pin — already implemented by #1270) | |
| Rejects invalid duplicate Runtime Operation names | 1a | |
| Rejects malformed signatures | 0 (regression pin — schema `enum` on `abi_params`/`abi_return` already enforces this per #1270) | "malformed signature" has no catalogue-schema meaning beyond ABI-type well-formedness, since signatures/effects live in `env.rs`, not the catalogue (per #375's audit note) |
| Rejects unknown effects | 2b | An effect token in `env.rs` the parser doesn't recognize must hard-error, not be silently treated as "non-empty ⇒ impure" |
| Rejects unknown return shape tags | 0 (regression pin — schema `enum` on `ir_return_shape` already enforces this per #1270) | |
| Rejects unknown arena-routing categories | 3, 4 | |
| Rejects unknown verifier-model categories | 3, 2a | |
| Actionable diagnostics | 8 | |
| Schema carries doc metadata (description) | 0 (regression pin — `description` field already exists per #1270; the audit note's ask predates #1270's actual implementation) | |
| Generated help/spec tables consume or are checked against catalogue | 5, 6 | via the existing `grammar.md → generate_help.py → compiler/main.vow` chain, checked at the `grammar.md` end |
| Projection freshness in normal CI path | 7 | mostly pre-existing per #1270; confirm/extend |
| Invalid-fixture tests, no private internals | 9 | |
| Help/spec output freshness tests | 10 | |
| Rust + self-hosted checked together | 11 | |
| Existing behavior preserved | implicit in all slices (pure test/validation additions) | |

### Slice 0 — Regression pins for already-implemented checks (no production change expected)

Add explicit tests in `scripts/test_generate_ops.py` pinning behavior #1270 already delivers, so
this issue's own test suite documents full AC coverage rather than leaving three bullets
implicitly satisfied by someone else's commit:
- duplicate `surface_name` rejected (mirror of #1270's existing test, confirm still present)
- malformed `abi_params`/`abi_return`/`ir_return_shape` (out-of-enum value) rejected
- `description` field required and present

If any of these turns out *not* to hold once #1270 actually lands (e.g. `description` becomes
optional), this slice's tests fail loudly and the gap becomes this issue's to fix — do not skip
this slice as "already done" without running it.

### Slice 1a — Reject inconsistent duplicate `runtime_symbol`

- Test: `ValidateCatalogueTest.test_rejects_inconsistent_duplicate_runtime_symbol` — two fixture
  entries share `runtime_symbol` but differ in `abi_return`; assert an error naming both
  `surface_name`s and the shared symbol.
- Test: `test_allows_consistent_duplicate_runtime_symbol` — two entries share `runtime_symbol`
  with byte-identical `abi_params`/`abi_return`/`ir_return_shape`; assert no error (legitimate:
  multiple surface names may route to one runtime symbol).
- Production: extend `validate_catalogue()` to group entries by `runtime_symbol` and compare
  ABI/return-shape facts within each group.

### Slice 1b — Fix `render_abi_rs` to emit one match arm per unique `runtime_symbol`

- Test: `RenderTest.test_render_abi_rs_dedupes_shared_symbol` — two fixture entries share a
  `runtime_symbol` (consistent ABI); assert the rendered Rust text contains exactly one match arm
  for that symbol, not two.
- Production: dedupe-by-symbol in `render_abi_rs()` before building match arms. Only proceed with
  this slice if preflight step 4 confirms the bug still exists in the merged #1270/#1271 code.

### Slice 2a — Reject `verifier_category` on an effectful operation

- Test: fixture entry with `verifier_category="Modeled"` whose `surface_name` maps (in a fixture
  `env.rs`-shaped text snippet) to a non-empty effect list (e.g. `&[Effect::IO]`); assert an
  error naming the operation and the offending effect.
- Test (positive): an entry with `verifier_category` set and a genuinely empty effect list (`&[]`)
  passes.
- Production: new check (folded into `validate_catalogue()` or a sibling
  `check_verifier_category_purity(entries, env_rs_text)`) that parses each named builtin's effect
  list out of `env_rs_text` and rejects a non-`null`/absent verifier category when that list is
  non-empty.

### Slice 2b — Hard-error on an unrecognized effect token

- Test: fixture `env.rs` text containing `&[Effect::Frobnicate]` (a token not in the known set);
  assert the parser used by slice 2a raises/reports an error rather than silently treating the
  entry as "has effects, therefore impure, therefore fine to skip." Known set as of `main`:
  `IO`, `Read`, `Write` (confirm the complete list from `vow-types/src/env.rs` at implementation
  time — do not hardcode from this plan alone, there may be a `Panic` variant too).
- Production: give the effect-list parser (from slice 2a) an explicit whitelist and fail loudly
  on anything outside it, rather than a permissive "anything non-`[]` counts as non-empty" regex.

### Slice 3 — Unknown arena-routing / verifier-category enum values rejected by schema

- Test: fixture entry with an out-of-enum arena-routing category value (e.g. `"Bogus"`); assert
  `validate_catalogue()` rejects it. Same for an out-of-enum verifier-category value.
- Test (positive): a value legitimately in each enum passes.
- Production: if the merged schema (post #1271/#1274) already declares these as closed `enum`s,
  this slice is a regression pin only (`schema_check.py`'s existing generic enum-rejection
  already covers it — no new code). If either field landed as a bare `string`, add the `enum`
  here — this is squarely "harden validation," not a scope-widening schema redesign.

### Slice 4 — Arena-routing category vs. actual `_in_arena` runtime-symbol presence

- Test: fixture entry claiming a "not arena-routed" category, paired with a fixture
  `vow-runtime/src/lib.rs`-shaped text snippet that *does* export `<runtime_symbol>_in_arena`;
  assert an inconsistency error. And the reverse: claims "arena-routed," no `_in_arena` export
  in the fixture text → error.
- Production: new `check_arena_routing(entries, runtime_rs_text)`, text-scanning
  `vow-runtime/src/lib.rs` the same way `check_env_rs_presence` already text-scans `env.rs`,
  checking for a `pub unsafe extern "C" fn <symbol>_in_arena` (or equivalent) definition.
  Ground the two category values against the real fork already visible in
  `vow-codegen/src/cranelift_backend.rs::routed_vec_extern` (lines ~769–870 on current `main`):
  routed operations get a `(<symbol>_in_arena, Some(region))` pair; non-routed operations don't.

### Slice 5 — Generalize `check_grammar_presence` across all catalogue-covered headings

- Test: fixture catalogue with entries "belonging" to two different `grammar.md` sections (e.g.
  a print-family name and a filesystem-family name); assert presence is correctly resolved
  against whichever heading actually contains each name, and a name missing from *every* known
  builtin-operation heading still errors.
- Production: change `check_grammar_presence` to search the **union** of all catalogue-relevant
  `####` headings in `grammar.md` (fixed list: "Print / IO", "Debug", "Filesystem", "String
  Operations", "Conversion", "Collections", "Time", "System", "Encoding", "Input", "Process
  Management" — confirm this list against `grammar.md` at implementation time, it may have grown)
  rather than one hardcoded heading name. Do **not** add a `doc_table` field to the catalogue
  schema for this — the surface-name-uniqueness invariant (already enforced) means a union search
  is sufficient and avoids a second source of truth for something derivable from `grammar.md`'s
  own structure.

### Slice 6 — `description` vs. grammar.md row-prose consistency

- Test: fixture where the catalogue `description` text differs from the corresponding
  `grammar.md` row's prose cell; assert an error naming both texts (or a normalized diff).
- Production: extend `check_grammar_presence` (or split out
  `check_grammar_description(entries, grammar_text)`) to compare each row's prose column against
  the catalogue's `description` field for that `surface_name`, using exact match on trimmed text
  as the contract (both are meant to be kept identical, not paraphrases of each other).

### Slice 7 — Freshness-check coverage confirmation/extension

- Test: if preflight finds #1274 introduced a second self-hosted marker block (e.g. in
  `compiler/c_emitter.vow`), add a `DriftDetectionTest` case proving `compute_drift()` catches
  staleness there too, mirroring the existing `compiler/lower.vow` marker-block test.
- Production: extend `compute_drift()`'s list of checked targets accordingly. Skip this slice
  entirely (documented as "not applicable") if #1274 ends up reusing the existing
  `compiler/lower.vow` splice instead of adding a second one.

### Slice 8 — Diagnostic message uniformity

- Test: a fixture that trips one instance of every check kind added by this issue at once;
  assert every resulting message matches one consistent, documented shape (e.g.
  `"<locator>: <field> <problem>"`, matching `schema_check.py`'s existing style rather than
  inventing a new one — see §6, do not edit `schema_check.py` itself).
- Production: route all new check functions' error strings through one small formatting helper
  in `generate_ops.py` so maintainers and agents get one consistent format across every failure
  kind this issue adds (existing #1270 messages may be left as-is if they already conform;
  confirm rather than reformat for its own sake).

### Slice 9 — End-to-end CLI-level validation-rejection test (no private internals)

- Add `--catalogue PATH` to `generate_ops.py` (see §2) so the real CLI entry point can be pointed
  at a fixture file instead of the hardcoded `docs/spec/operations.json`.
- Test: a fixture JSON file (new, on disk — see §4 for placement) covering multiple violations at
  once — duplicate `surface_name`, an inconsistent duplicate `runtime_symbol`, an out-of-enum
  arena-routing category, an out-of-enum verifier-category — invoked via
  `python3 scripts/generate_ops.py --check --catalogue <fixture>` as a subprocess (or via
  `main()` with `sys.argv` patched, whichever this repo's existing test style prefers — check
  `test_generate_ops.py` and `test_check_help_coverage.py` for precedent), asserting non-zero
  exit and that stdout lists every distinct violation.
- **Fixture correctness requirement**: every entry in this fixture must use **real** existing
  `surface_name`s (e.g. `print_str`) with otherwise-corrupted facts — an invented name would fail
  `check_env_rs_presence`/`check_grammar_presence` before ever reaching the checks under test,
  producing a misleading pass/fail signal unrelated to what the test claims to cover.

### Slice 10 — Help/spec output freshness test for migrated Builtin Operations

- Test: an integration-style test (in `scripts/test_generate_ops.py` or alongside
  `scripts/test_check_help_coverage.py`) asserting `generate_help.py --check` and
  `generate_ops.py --check` both pass together against the **real** `docs/spec/operations.json`
  and `docs/spec/grammar.md` as of whatever operations have migrated by the time this issue
  lands — proving the two generators' outputs can't independently drift from each other via the
  shared `grammar.md` chokepoint.
- Production: none expected beyond wiring both `--check` invocations into `scripts/full_test.sh`
  if either is missing (expected: `generate_help.py --check` is already there; confirm
  `generate_ops.py --check` was actually added by #1270 as its plan claims).

### Slice 11 — Rust/self-hosted projection parity smoke test

- Test: mutate one fixture catalogue entry, render both the Rust and self-hosted projections from
  it, and assert both changed consistently from a single `load_catalogue()` call; separately,
  assert `compute_drift()` flags staleness when only one target file is updated to match the
  mutated entry and the other is left stale — proving the two projections cannot independently
  drift from the single catalogue source.
- Production: none expected — this slice should pass against #1270's existing
  `compute_drift()`/render functions once slices 1b and 3 land; if it doesn't, that's a genuine
  gap this issue must close.

## 4. Verification surface

This issue does not touch contracts, codegen, or the C model — it hardens a build-time Python
generator/validator and its test suite. No new ESBMC-provable properties are introduced, and no
`.vow` contract clauses should be added anywhere as a side effect of satisfying a Python-level
check (per the contract-authoring rules in `CLAUDE.md`, a build-time invariant is not a vow).

- No new fixtures are needed under `tests/run/` or `examples/` — #1270–#1274 already own
  end-to-end `.vow` coverage (e.g. `tests/run/print_catalogue_smoke.vow`) for the migrated
  operations' compiled/verified behavior; this issue's tests are catalogue-shaped fixtures in
  `scripts/test_generate_ops.py`.
- Slice 9's on-disk fixture file (if the CLI-subprocess approach is chosen over `sys.argv`
  patching) belongs under `scripts/testdata/` or co-located with `scripts/test_generate_ops.py`
  (check existing precedent among `scripts/test_*.py` for fixture-file conventions before
  inventing a new location) — **not** `tests/run/` or `examples/`, which are reserved for `.vow`
  programs, not generator test fixtures.
- `compiler/tests/test_op_catalogue.vow` is extended only if self-hosted code independently
  interprets catalogue facts in a way Python-side validation cannot reach (see §2 — expected: no).

## 5. Risk areas

- **Binary fixed point.** `load_catalogue()` already sorts entries by `surface_name` before
  handing them to any renderer (per #1270, explicitly for bootstrap-triple-test determinism).
  Any new renderer/formatter code this issue adds (slice 1b's dedup, slice 8's message
  formatter) must preserve deterministic ordering — never iterate a Python `dict`/`set` without
  sorting when building match arms or grouped error-message output. This is the same discipline
  #1274's own plan already flags for its verifier-category renderer; hold it here too, including
  for error-message ordering, so CI failures are stable/diffable across runs even though error
  strings aren't part of the generated compiler artifact.
- **`cargo clippy --all -- -D warnings`.** This issue is Python-only in its production code
  (`scripts/generate_ops.py`) plus test-only `.vow`/Rust additions if any. Slice 1b directly
  *prevents* a real clippy failure (`unreachable_patterns` from duplicate match arms) rather than
  risking one — no new Rust production code is added by this plan.
- **`parse → print → parse` idempotency.** Unaffected — no grammar, AST, or printer changes.
- **Sequencing.** Every slice referencing `verifier_category` or an arena-routing field name
  depends on #1270–#1274 having landed with the field names/shapes this plan assumed (see §0's
  binding table). If the implementation stage starts before they land, or they land with
  different names/shapes, slices 2–4 must be re-derived from the actual merged schema — this
  document's names are hypotheses, not commitments.
- **Scope creep.** It is tempting to also fix the `is_string_fresh_helper`-style
  pattern-predicate exclusions #1271 explicitly defers (`string_trim`/`string_to_upper`/
  `string_to_lower` keeping their hand-written verifier gate instead of the catalogue) while
  touching nearby validation code. That rewiring is #1274's territory, not this issue's — #1275
  hardens validation of whatever categories/fields exist post-#1271–#1274, it does not extend
  which operations consume them.

## 6. Out of scope

- Migrating any additional Builtin Operation family beyond what #1270–#1274 already migrate
  (constructors, methods, `pin_to_root`, `string_matches_literal_at`, the
  `is_string_fresh_helper`-gated string helpers) — excluded by the parent PRD for the entire
  first-slice arc, not just this issue.
- Rewriting `docs/spec/grammar.md` prose or `compiler/main.vow`'s help pipeline. Both already
  flow from `grammar.md` via the pre-existing `scripts/generate_help.py`, confirmed unrelated to
  the catalogue work by #1270's own plan. This issue only adds/generalizes a
  catalogue-vs-`grammar.md` *consistency check* — never a new generation path for those files.
- Adding a `doc_table`/section-membership field to the catalogue schema (see slice 5) — a union
  search across `grammar.md`'s known builtin-operation headings is sufficient and avoids a second
  source of truth for something already derivable from `grammar.md`'s structure.
- Changing verifier C emission bodies, `is_string_fresh_helper`/`string_model_*` special-case
  logic, or any modeled/skipped behavior for any operation — #1274's territory (explicitly
  behavior-preserving there) and out of scope for this purely validation-and-doc-integration
  issue.
- Introducing a shared `vow-ops` crate to de-duplicate the three byte-identical `op_catalogue.rs`
  copies (`vow-ir`, `vow-codegen`, `vow-clif-shim`) — #1270's plan already explicitly rejected
  this as disproportionate; this issue doesn't revisit that call.
- Modifying `scripts/schema_check.py`'s message format or validation keyword coverage. It is
  shared parity-gate infrastructure (used outside the Operation Catalogue) with its own
  documented contract ("the nine keywords those schemas actually use"); this issue aligns
  `generate_ops.py`'s *own* new messages to its existing style, not the other way around.
- Formatting/cleanup passes over `scripts/generate_ops.py` or `scripts/generate_help.py` unrelated
  to the specific functions this issue's slices touch.
