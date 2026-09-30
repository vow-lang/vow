# PLAN: #1275 — Harden Operation Catalogue validation and documentation integration

## 0. Ground truth as of this planning session (2026-09-30)

This supersedes the PLAN.md previously committed on this branch (b5356572, written 2026-09-13
against a hypothesis — a `docs/spec/schemas/operation-catalogue.schema.json`, a
`scripts/generate_ops.py`, a `render_abi_rs`/`validate_catalogue`/`compute_drift` API — that never
matched what actually got built). This plan is grounded in the real, currently-merged code, read
directly via `git show` in this session, not in sibling branches' planning prose.

**What is actually on `origin/main` today** (this worktree's own branch point, `0cf9d948`, predates
all of it — do not plan against the worktree's checked-out files):

- `docs/spec/operations.json` — the catalogue. A flat `{"operations": [...]}` list. 22 entries
  covering `print_*` (#1270, merged as PR #1279) and `fs_*`/stdin/args/stderr (#1272, merged as
  PR #1356 — the issue itself is still open on GitHub, but its code has landed).
- `scripts/generate_operations.py` (366 lines) — the one generator/validator. Splices generated
  match-arm functions between `// GENERATE:OPERATIONS:START/END` markers into four target files:
  `vow-ir/src/lower/mod.rs`, `vow-codegen/src/cranelift_backend.rs`, `vow-clif-shim/src/lib.rs`,
  `compiler/lower.vow`. Separately cross-checks (never generates) `docs/spec/grammar.md`'s
  "Builtin Function Signatures" table, `vow/src/skill.rs`, and `compiler/main.vow`'s help-JSON
  string against each entry's `doc_signature`/`effects` fields.
- `scripts/test_generate_operations.py` — direct-function-call tests against `generate_operations`
  imported as a module, plus tests against the real `docs/spec/operations.json`/`grammar.md`.
- CI wiring already exists and already fires on a catalogue-only change: `.github/workflows/ci.yml`
  runs `python3 scripts/generate_operations.py --check` and
  `python3 scripts/test_generate_operations.py` as explicit steps, gated by
  `scripts/ci_docs_only.py`'s `code` classifier. **Verified, not assumed**: `ci_docs_only.py`'s
  `is_prose()` only recognizes paths ending in `.md`; `docs/spec/operations.json` does not end in
  `.md`, so it is never classified as prose and always triggers the gated jobs. No fix needed here.
- `scripts/full_test.sh` does **not** call `generate_operations.py` or `generate_help.py` — neither
  does it call `check_help_coverage.py`. All three Python staleness/generator checks are CI-only
  steps in `ci.yml`, run as siblings of `full_test.sh`, not from inside it. This is the established
  pattern for every generator gate in this repo, not a gap specific to the Operation Catalogue —
  treat "full_test.sh should also run this" as an anti-pattern to avoid introducing, not a fix.
- `.pre-commit-config.yaml` has no project-specific script hooks (only generic ones: `check-yaml`,
  `ruff-check`/`ruff-format`, `typos`, `pydoclint`, `commitlint`). `check_help_coverage.py` isn't
  wired there either. Consistent — no gap.

**Current catalogue schema** (per `REQUIRED_FIELDS` in `load_catalogue`): every entry must have
`name`, `runtime_symbol`, `params` (list of `ptr`/`i64`/`u64` tokens), `return` (one of
`unit`/`i64`/`ptr`/`bool`), `doc_signature` (a human-readable `fn(...) -> ...` string), `effects`
(a human-readable bracket string, e.g. `"[io]"`, `"[]"`). Observed effects values across all 22
current entries: `{"[]", "[io]", "[read]", "[write]"}` — never combined, never multi-token yet.

**What today's `load_catalogue` already validates**: required-field presence, duplicate `name`,
duplicate `runtime_symbol` (unconditional — any duplicate is an error), `return` token against a
closed set, each `params` token against a closed set, `params` being a list. **What it does not
validate at all**: the shape of `effects` (free string, no vocabulary check), the shape of
`doc_signature` (free string, no arity/type check), unknown/extra keys on an entry (silently
ignored — a typo'd field name is never caught). **What is not actionable today**: `main()` calls
`load_catalogue(REPO)` with no `try`/`except` — a bad catalogue entry raises `ValueError` (or a bad
`operations.json` raises `json.JSONDecodeError`) straight through to an uncaught Python traceback,
not a clean diagnostic. `load_catalogue` also fails on the *first* bad entry it finds (plain
`raise`), unlike `check_doc_facts`, which already collects every mismatch before reporting — so a
catalogue with three unrelated problems takes three separate runs to fully diagnose today.

**What sibling in-progress branches (not yet merged) are adding**, read via
`git show <branch>:<path>` in this shared-`.git` worktree layout — **informational only, not a
dependency this plan blocks on** (see §0b):

- `sym/vow/1271-...` (query/utility slice) and `sym/vow/1273-...` (process slice) **each
  independently** add two new optional fields to the same 22-entry base: `verifier_model` (closed
  set `{"known", "unmodeled"}`) and `arena_routing` (closed set `{"none", "heap_fresh"}`), plus a
  `catalogue_verifier_known()` generated function spliced into `vow-verify/src/c_emitter.rs` and
  `compiler/c_emitter.vow`, and a `catalogue_builtin_result_tag()` function for arena-routed
  heap-returning ops. Both branches use **identical field and value names** (confirmed by diff) —
  they forked from a close-enough common point that the vocabulary already agrees.
- `sym/vow/1274-...` (verifier-classifier replacement) has **only its two planning commits** —
  no implementation yet.

### 0a. Files to touch

- `scripts/generate_operations.py` — all new validation logic (see §3). No change to the four
  existing splice targets' generated *content* is anticipated; this issue hardens the reader/
  checker, not the compiled projections.
- `scripts/test_generate_operations.py` — new tests for every check added, following the file's
  existing direct-import-and-call style (`import generate_operations as go`).
- `docs/spec/operations.json` — no content change anticipated. If slice-3 validation (§3) finds an
  existing entry that doesn't actually fit the tightened rules (e.g. a `doc_signature` whose arity
  doesn't match `params`), fixing that entry is in scope as a one-line data correction, not a
  schema change.
- `docs/spec/grammar.md`, `compiler/main.vow`, `vow/src/skill.rs` — no changes anticipated; §0's
  evidence shows the existing `doc_signature`+`effects` fields already fully derive every row/line
  `check_doc_facts` checks against (see §3, "doc-metadata" note). Touched only if slice-3
  discovers an actual mismatch in the real files.
- `CLAUDE.md`'s "Operation Catalogue" section — extend with one short paragraph once the new
  validation rules exist, mirroring how the section already documents `--check`'s purpose. Small,
  additive, no restructuring.
- No changes anticipated in `vow-ir/`, `vow-codegen/`, `vow-clif-shim/`, `vow-verify/`,
  `compiler/lower.vow`, `compiler/c_emitter.vow`, or any `.vow` test file — this issue hardens the
  Python-side catalogue reader, not lowering, codegen, or verifier behavior.

### 0b. Sequencing: this plan does not block on #1271/#1272/#1273/#1274

The issue lists all four as "Blocked by," and #1272 is the only one whose code has actually landed
on `origin/main` (#1270 also landed). #1271 and #1273 have real, unmerged implementation on sibling
branches; #1274 has none yet. Waiting for all four to merge before writing this plan would leave
the run with nothing to commit, and the operating contract for this run says to make the most
defensible call and document it rather than block.

**Decision**: plan this issue's slices against what is verifiably on `origin/main` today (22
entries, no `verifier_model`/`arena_routing` fields), written so every new check **also** covers
`verifier_model`/`arena_routing` using the exact field names and value sets both unmerged branches
already agree on (`verifier_model: known|unmodeled`, `arena_routing: none|heap_fresh`) — so the
mechanical work at implementation time is a rebase onto whichever of #1271/#1272/#1273/#1274 has
landed by then, re-deriving only whichever field names turn out to differ from what's written here.

**Implementation-stage preflight (do this before writing any test or code)**:
1. `git fetch origin main`; `git log origin/main --oneline -1` — confirm current tip.
2. Rebase this branch onto `origin/main`.
3. `git log origin/main --grep 1271`, `--grep 1273`, `--grep 1274` — if any has landed, its exact
   field names/values are ground truth over this plan's §0 snapshot. Re-read
   `scripts/generate_operations.py` fresh rather than trusting this document's field names.
4. If #1271 and #1273 have **both** landed independently (as of this session, neither is merged, so
   this is a real possibility by implementation time): expect a merge/rebase conflict in
   `load_catalogue` and the `TARGET_FILES`/marker-splice machinery, since both branches edit the
   same functions. Resolving it is normal integration work, not this issue's scope — but once
   resolved, there must be exactly **one** copy of the `verifier_model`/`arena_routing` vocabulary
   check, not two near-identical copies left over from a mechanical merge.
5. If #1274 has landed, its own validation additions (if any) become ground truth the same way.

A `gh issue comment 1275` records this decision (posted alongside this commit — see the run's
closing steps).

## 1. Problem restated

`docs/spec/operations.json` plus `scripts/generate_operations.py` is a working, CI-gated Operation
Catalogue: it already prevents four of the eight failure modes the issue's acceptance criteria
name (duplicate names, duplicate runtime symbols, bad return tokens, bad param tokens), and it
already keeps `grammar.md`/`skill.rs`/`main.vow`'s hand-written builtin-signature prose from
silently drifting from the catalogue. What it does not yet do: reject a catalogue entry whose
`effects` or `doc_signature` string is malformed or uses an unrecognized token, reject an entry
carrying an unexpected/misspelled field name (which today silently defeats any vocabulary check
tied to that field, including the `verifier_model`/`arena_routing` checks landing on sibling
branches), surface any of the above as a clean, actionable, non-traceback diagnostic, or report
more than one problem per run. This issue closes those specific gaps — it does not redesign the
catalogue's shape, generation mechanism, or splice targets, and it does not migrate any further
Builtin Operation family.

## 2. TDD slices

Each slice is additive to `scripts/generate_operations.py` / `scripts/test_generate_operations.py`.
Slices are ordered so each is independently mergeable and small; none depends on a later slice.

### AC → slice map

| Acceptance criterion (issue text) | Slice |
|---|---|
| Rejects duplicate surface names | already true — slice 1 adds the regression-pin test only |
| Rejects invalid duplicate Runtime Operation names | already true — slice 1 adds the regression-pin test + documents "invalid" = "any duplicate" |
| Rejects malformed signatures | slice 3 |
| Rejects unknown effects | slice 2 |
| Rejects unknown return shape tags | already true — slice 1 pins it |
| Rejects unknown arena-routing categories | slice 4 (extends/finalizes whatever lands from #1271/#1273) |
| Rejects unknown verifier-model categories | slice 4 |
| Actionable diagnostics | slice 5 (collect-all-errors) + slice 6 (no raw tracebacks) |
| Schema carries doc metadata for help/spec generation | already true — see note below, no slice needed |
| Generated help/spec tables consume or are checked against catalogue facts | already true (`check_doc_facts`) — slice 7 extends it to cover any new field/table the rebase introduces |
| Projection freshness in normal local/CI path | already true — see §0, no slice needed |
| Invalid-fixture tests without private generator internals | slice 8 |
| Help/spec output freshness tests for migrated ops | slice 7 |
| Rust and self-hosted projections checked together | already true (`check_projections` walks all four `TARGET_FILES` in one call) — slice 9 adds a regression-pin test, and covers whatever fifth/sixth target #1271/#1274 add |
| Existing behavior preserved | implicit — every slice is additive validation, no change to `gen_*_block` output for any of the 22 current entries |

**Doc-metadata note** (resolves the issue's "corrections from the original scope" point): the
issue asks the schema to "carry enough human-readable metadata to generate/check `main.vow`'s
help-JSON and `grammar.md`'s table rows... a bare name→symbol mapping is not sufficient." Verified
against the real, merged code: `docs/spec/operations.json` entries were never a bare name→symbol
mapping — every entry already carries `doc_signature` (a full human-readable `fn(...) -> ...`
string) and `effects` (a human-readable bracket string), and `check_doc_facts` already derives and
checks the exact three-column `grammar.md` row (`Function | Signature | Effects`) and the exact
`skill.rs`/`main.vow` help-JSON line (`"name": "sig effects"`) from those two fields for all 22
entries with zero hand-authored duplication. There is no fourth prose surface (no per-operation
description column exists in `grammar.md`, no separate description string in `main.vow`'s
help-JSON) that these two fields fail to cover. **No new `description` field is added by this
plan** — inventing one would be undocumented schema growth with no consumer. If a future migration
slice introduces an operation family whose `grammar.md` presentation needs prose beyond a
signature+effects row, that is that slice's schema-extension to propose, not a speculative
addition here.

### Slice 1 — Regression pins for checks that already work

- `test_duplicate_runtime_symbol_raises` — today's suite has `test_duplicate_name_raises` but
  nothing pinning the separate `seen_symbols` check in `load_catalogue`. Two fixture entries with
  distinct `name` but the same `runtime_symbol`; assert `ValueError` naming both.
- Confirm (already present, just verify) `test_unknown_return_token_raises` and
  `test_unknown_param_token_raises` still pass unmodified.
- No production change. This slice exists so the issue's own test suite documents full coverage of
  the "already true" AC rows above, rather than leaving them implicitly satisfied by someone else's
  commit with no test naming this issue's acceptance criterion.

### Slice 2 — Validate `effects` against a closed, ordered vocabulary

- Closed vocabulary, lowercase, sourced from `vow-types/src/effects.rs`'s `Effect` enum display
  mapping (`Read`, `Write`, `IO`, `Panic`, `Unsafe` → `read`, `write`, `io`, `panic`, `unsafe`).
  Confirm this list against `vow-types/src/effects.rs` fresh at implementation time (do not
  hardcode from this plan without checking — a variant could be added between now and then).
- Grammar: `effects` must match `^\[(token(, token)*)?\]$` where each `token` is in the closed set;
  tokens must be sorted and de-duplicated in the string as written (matches every observed value
  today: `[]`, `[io]`, `[read]`, `[write]` are all trivially sorted single-or-empty lists — this
  rule only becomes load-bearing once a multi-effect entry is added, which none are yet).
- Tests: `test_effects_unknown_token_raises` (e.g. `"[frobnicate]"`), `test_effects_malformed_bracket_raises`
  (e.g. `"io"` with no brackets, `"[io,]"` trailing comma), `test_effects_unsorted_raises` (e.g.
  `"[write, read]"`), plus a positive test that all four real observed values parse clean.
- Production: new `EFFECT_TOKENS` closed set + a small parser in `load_catalogue`, invoked per
  entry, contributing to the collected-errors list (slice 5).

### Slice 3 — Validate `doc_signature` arity and per-token type mapping

- Grammar: `doc_signature` must match `^fn\((.*)\) -> (.+)$`. The parenthesized parameter list,
  split on top-level commas (each parameter written `name: Type`), must have exactly
  `len(op["params"])` entries, in order. Each parameter's `Type` and the trailing return `Type`
  must map to the corresponding `params[i]`/`return` token via one fixed dict, derived empirically
  from the 22 real entries before writing the check (expected shape: `String → ptr`,
  `Vec<...> → ptr`, `i64 → i64`, `u64 → u64`, `() → unit`; confirm every existing entry fits this
  before assuming it's complete — if `bool` appears as a `return` token anywhere in the real
  catalogue with no corresponding `-> bool`-shaped `doc_signature`, that combination needs its own
  explicit rule, not a guess).
- Tests: `test_doc_signature_arity_mismatch_raises` (params has 2 tokens, doc_signature declares 1
  parameter), `test_doc_signature_type_mismatch_raises` (a `ptr` param token paired with a
  `doc_signature` parameter typed `i64`), `test_doc_signature_missing_arrow_raises` (no ` -> `),
  plus a positive test asserting all 22 real entries pass under the finished rule (this doubles as
  the "malformed signature" regression pin required by the AC table).
- Production: the arity/type-mapping check in `load_catalogue`, contributing to slice 5's
  collected-errors list.

### Slice 4 — Close `verifier_model`/`arena_routing`, reject unknown keys

- If the rebase (see §0b preflight) has landed either or both fields: confirm the closed sets in
  `generate_operations.py` match this plan's §0 values (`known`/`unmodeled`,
  `none`/`heap_fresh`); tighten if a sibling branch left either as a bare unvalidated string.
- If neither has landed yet: add the two optional fields' closed-set validation now, using the
  exact names/values both unmerged branches already agree on (§0), so the eventual rebase is a
  clean superset merge rather than a redesign. Do **not** wire either field into any consumer
  (`c_emitter.rs`/`c_emitter.vow`/lowering) — that is #1271/#1273/#1274's territory; this issue
  only validates the field's presence in the catalogue.
- New: reject any key on an entry not in `REQUIRED_FIELDS ∪ {"verifier_model", "arena_routing"}` —
  today a typo'd field name (`verifer_model`) is silently ignored, which would silently defeat this
  exact check. This is squarely "unknown verifier-model category" in spirit: a category so unknown
  the field name itself doesn't exist yet is a sharper failure than a recognized-field-bad-value
  case, and today neither is caught for a misspelled key.
- Tests: `test_unknown_verifier_model_raises`, `test_unknown_arena_routing_raises` (mirroring
  slice 1's pattern), `test_unexpected_field_name_raises` (an entry with a bogus extra key),
  `test_misspelled_optional_field_raises` (an entry with `verifer_model` instead of
  `verifier_model` — must fail as an unknown key, not silently pass with the real check skipped).
- Production: the two closed-set checks (new or confirmed) plus the new allowed-keys check, all
  contributing to slice 5's collected-errors list.

### Slice 5 — Collect all validation errors before reporting

- Today `load_catalogue` raises on the first bad entry (`raise ValueError(...)`), unlike
  `check_doc_facts`, which already returns a list of every mismatch found. A catalogue with three
  independent problems takes three separate `--check` runs to fully diagnose.
- Test: `test_multiple_violations_all_reported` — a fixture with three independent problems (e.g.
  a duplicate name, an unknown effect token, a bad doc_signature arity on a different entry);
  assert the raised error (or returned list, depending on the chosen shape — see production note)
  contains distinguishable text for all three, not just the first encountered.
- Production: change `load_catalogue`'s internal validation loop to collect `(entry, field,
  message)` tuples into a list instead of raising immediately; raise once at the end with all
  messages joined (one `ValueError` whose `str()` contains every message, newline-separated) if the
  list is non-empty. Keep the function's external contract (raises `ValueError` on any problem,
  returns the op list on success) unchanged so `main()`, `check_projections`, and `check_doc_facts`
  callers need no change beyond what slice 6 adds.

### Slice 6 — No raw tracebacks: `main()` catches and reports cleanly

- Today `main()` calls `ops = load_catalogue(REPO)` uncaught; both a malformed
  `docs/spec/operations.json` (bad JSON → `json.JSONDecodeError`) and a bad entry (→ `ValueError`,
  post-slice-5 carrying every collected message) surface as a raw Python traceback with a nonzero
  exit code that happens to be right but a message that isn't actionable for a human or an agent
  parsing CI output.
- Test: invoke `generate_operations.main()` (with `sys.argv` patched, matching this file's existing
  test style) against a fixture with a deliberately malformed `docs/spec/operations.json`-shaped
  file (via slice 8's `--repo-root`, see below) and assert: nonzero exit, no traceback on stdout/
  stderr (no `Traceback (most recent call last)` substring), and the collected message text is
  present.
- Production: wrap `ops = load_catalogue(...)` in `main()` in `try: ... except (ValueError,
  json.JSONDecodeError) as e: print(str(e), file=sys.stderr); sys.exit(1)`.

### Slice 7 — Extend `check_doc_facts` coverage as the rebase adds fields

- Only actionable once §0b's preflight determines what, if anything, #1271/#1273/#1274 add to
  `grammar.md`/help-JSON beyond the existing signature+effects row (per §2's doc-metadata note,
  expected: nothing — process_* and query/utility operations use the same three-column table
  shape, confirmed by `df0903b0` on the #1273 branch adding a `process_poll_wait` row in that same
  shape). If confirmed, this slice is a no-op beyond a regression-pin test
  (`test_check_doc_facts_covers_all_migrated_families`) asserting `check_doc_facts` returns zero
  mismatches against the real, post-rebase `docs/spec/operations.json`/`grammar.md`/`skill.rs`/
  `main.vow` — i.e. an end-to-end freshness test for every migrated Builtin Operation, not just the
  22 present today. This directly satisfies the "help/spec output freshness tests for migrated
  Builtin Operations" AC.

### Slice 8 — Black-box test seam: `--repo-root` override, no private internals

- The issue requires catalogue-fixture tests that don't rely on "implementation-private generator
  internals." Today's tests already call public module functions directly (`go.load_catalogue`,
  `go.check_doc_facts`, etc.) with a `repo_root` parameter each function already accepts — that
  part is fine. What's missing is a way to exercise the **CLI entry point** (`main()`,
  `--check` exit-code behavior) against a fixture tree instead of the hardcoded `REPO` constant.
- Add a `--repo-root PATH` flag to `main()` (default: `REPO`, i.e. no behavior change for the real
  CLI invocation used by CI/`--check`). Thread it through to `load_catalogue`/`check_projections`/
  `check_doc_facts`, all of which already take `repo_root` as a parameter.
- Test: build a minimal fixture directory tree under a `tempfile.TemporaryDirectory()` (a
  `docs/spec/operations.json` with a deliberate violation, plus the four/six splice-target files
  with valid marker pairs and a minimal `grammar.md`/`skill.rs`/`main.vow`), invoke
  `generate_operations.main()` with `sys.argv` patched to `["generate_operations.py", "--check",
  "--repo-root", str(fixture_dir)]`, assert nonzero exit and the expected message — proving the
  full CLI path end-to-end without reaching into `load_catalogue`/`check_projections` directly.
- This slice's fixture-tree helper is reused by slice 6's test.

### Slice 9 — Regression pin: freshness check covers all splice targets together

- `check_projections` already iterates `TARGET_FILES` (four entries today, more once #1271/#1274's
  `c_emitter.rs`/`c_emitter.vow` targets land) in one call and reports every stale target, not just
  the first. Add `test_check_projections_reports_every_stale_target` — hand-mutate two of the four
  (or however many exist post-rebase) target files' spliced blocks in a fixture tree (reusing
  slice 8's fixture helper), call `check_projections` once, assert both stale paths appear in the
  returned list. This is the concrete evidence for the "Rust and self-hosted Operation Projections
  remain generated from the same catalogue and are checked together" AC — it was already true
  structurally; this slice is the test that pins it.

## 3. Verification surface

This issue touches no `.vow` contracts, no C emission, no codegen, and no runtime behavior — it
hardens a build-time Python generator/validator and its own test suite. No ESBMC-provable property
is introduced or changed, and no `vow` contract clause should be added anywhere to satisfy a
Python-level check (a build-time catalogue invariant is not a vow, per `CLAUDE.md`'s
contract-authoring rules). No new fixtures under `tests/run/` or `examples/` are needed — the
migrated Builtin Operations' compiled/runtime behavior is already covered by each migration
slice's own end-to-end `.vow` tests (e.g. `tests/run/print_catalogue_smoke.vow`,
`compiler/tests/test_lower_catalogue_fs_stdin_args_stderr.vow`); this issue's new tests are
catalogue-fixture-shaped and belong in `scripts/test_generate_operations.py` plus (slice 8) a
temp-directory fixture tree, not under `tests/run/`.

## 4. Risk areas

- **Determinism / binary fixed point.** None of this issue's production code touches
  `gen_rust_ir_block`/`gen_cranelift_block`/`gen_vow_lower_block` or their sibling verifier-known
  renderers — it adds *rejection* checks in `load_catalogue`, which runs before any renderer sees
  the op list. The real `docs/spec/operations.json`'s 22 (or post-rebase, more) entries must
  continue to pass every new check with **zero** change to any of the four/six generated blocks —
  `python3 scripts/generate_operations.py --check` must report clean before and after this issue's
  changes, on the same input. If any new rule doesn't hold for a real existing entry, fix the data
  (one-line JSON edit), never loosen the rule to fit bad data silently.
- **`cargo clippy --all -- -D warnings`.** This issue's production code is Python-only
  (`scripts/generate_operations.py`); no new Rust or `.vow` production code is added. Slice 1's
  duplicate-`runtime_symbol` regression pin exists specifically because an *unrejected* duplicate
  would later render two identical Cranelift/IR match arms and fail `-D warnings` on
  `unreachable_patterns` — this issue prevents that class of bug from a migration slice, it doesn't
  introduce clippy risk itself.
- **`uvx ruff@<pin>` gate.** New Python code must be validated with the repo's pinned ruff, not
  whatever local ruff is on `$PATH` — see this session's own memory on this exact gotcha.
- **Sequencing / rebase conflicts.** §0b already covers this: expect `load_catalogue` and
  `TARGET_FILES` conflicts when rebasing onto a `main` that has absorbed #1271 and/or #1273
  independently; resolving to a single copy of any duplicated vocabulary-check code is expected
  integration work, not a design problem with this plan.
- **Scope creep.** It would be tempting to also wire `verifier_model`/`arena_routing` into an
  actual consumer (`c_emitter.rs`, lowering) while touching this file — that is #1271/#1273/#1274's
  job. This issue validates catalogue *data*, it does not change what consumes it.

## 5. Out of scope

- Migrating any further Builtin Operation family (constructors, methods, `pin_to_root`,
  `string_matches_literal_at`) — excluded by the parent PRD (#375) for this entire first-slice arc.
- Adding a `description` field or any new prose surface to the catalogue schema — §2's doc-metadata
  note shows the existing `doc_signature`+`effects` fields already fully derive every checked prose
  surface with zero hand-authored duplication; inventing a field with no consumer is undocumented
  schema growth this issue should not introduce.
- Wiring `verifier_model`/`arena_routing` into any actual consumer (`vow-verify/src/c_emitter.rs`,
  `compiler/c_emitter.vow`, lowering, arena-routing codegen) — #1271/#1273/#1274's territory.
- Cross-checking the catalogue's `effects` string against `vow-types/src/env.rs`'s authoritative
  `Effect` list per-operation (true semantic parity, not just vocabulary validity). The PRD is
  explicit that the catalogue reads signature/effects facts as inputs and does not re-own them;
  the catalogue's `effects` field is a doc-string mirror, and its accuracy relative to the real
  type-checker table is exercised by each migration slice's own type-checking tests, not by
  catalogue validation. Listed here as a legitimate follow-up if a future issue wants stronger
  parity than vocabulary-validity, not as this issue's job.
- Adding `generate_operations.py --check` to `scripts/full_test.sh` or `.pre-commit-config.yaml` —
  §0 confirms this would be inconsistent with how every analogous generator gate
  (`generate_help.py`, `check_help_coverage.py`) already works in this repo: CI-only, run as a
  sibling step to `full_test.sh`, not folded into it.
- Fixing `scripts/ci_docs_only.py`'s classifier — §0 confirms it already correctly treats
  `docs/spec/operations.json` as code (triggers the gated CI jobs), since its prose test is
  suffix-based (`.md` only) and the JSON file doesn't match it. No change needed.
- Reformatting or cleaning up unrelated parts of `scripts/generate_operations.py` (e.g. the
  existing `_split_table_row`/`extract_builtin_signatures_table` machinery) beyond what each slice
  above directly touches.
