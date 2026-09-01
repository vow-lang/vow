# Plan — issue #1083: adversarial AI pair review of Rust ↔ self-hosted module pairs

## 1. Problem restated

The tier-3 pair-review harness already exists (`scripts/pair_review.py`, landed in #1091) with the
non-negotiable confirmation gate the issue demands: a model claim is a *hypothesis* until
`scripts/equivalence.py` runs its discriminating `.vow` program through both compilers and observes a
divergence. What is *not* done is the rest of the definition of done. The harness **truncates** instead
of chunking — with `--max-bytes 180000`, `vow-ir/src/lower/mod.rs` (299.7 KB) loses 120 KB,
`compiler/lower.vow` (256.7 KB) loses 77 KB, `vow-verify/src/c_emitter.rs` (328.4 KB) loses 148 KB and
`vow-types/src/check.rs` (267.9 KB) loses 88 KB, and *those bytes are never reviewed by any run,
ever* — while the issue explicitly requires "Chunk by function". The harness also never **writes** the
ledger: it reads `docs/equivalence/ledger.json`, computes each pair's content hash, and then discards
it, so all five pairs are still stamped `"last_reviewed": "never"` and the incrementality the issue's
third DoD bullet asks for is dead code that can never fire. Finally the issue's separately-named
variant — pointing the same machinery at `c_emitter` and asking about **model-vs-language soundness**
(does an emitted `__ESBMC_assume` narrow ESBMC's model below what the language permits?) — has no mode,
and `scripts/test_pair_review.py` is not wired into CI, so the harness's own guarantees are unguarded.
This change closes those four gaps. It is Python tooling only: no Vow semantics, no compiler code.

## 2. Files to touch

Production:

- `scripts/pair_review.py` — function-boundary chunking replacing truncation; ledger writeback;
  `--mode soundness`; new/renamed CLI flags.
- `scripts/test_pair_review.py` — grows from 110 to cover chunking, coverage, ledger writeback,
  verdict mapping, and the soundness prompt.
- `.github/workflows/ci.yml` — add `python3 scripts/test_pair_review.py` and
  `python3 scripts/test_verifier_runtime.py` as **separate steps** in the existing `python-tests` job
  (both harnesses currently ship untested in CI; `test_verifier_runtime.py` from #1092 was missed the
  same way).

Docs:

- `docs/equivalence/README.md` — tier-3 paragraph: chunked full coverage replaces truncation; the
  ledger is written by the harness, not by hand; the soundness variant is named as a distinct question.
- `.claude/commands/equivalence-review.md` — Step 2 gains `--dry-run`, `--chunk-bytes`,
  `--max-chunks-per-pair`, `--update-ledger --date`; a new Step 2b for the soundness variant; the
  "findings the harness marks `truncated`" paragraph is rewritten around deferred chunks.

Read-only dependencies (imported, not modified): `scripts/equivalence.py` (equivalence gate),
`scripts/verifier_runtime.py::check_soundness` (soundness gate), `bench/llm.py` (provider abstraction),
`docs/equivalence/ledger.schema.json` (writeback validation).

**No `docs/spec/*.md` change.** Nothing here alters Vow syntax, semantics, types, builtins, operators,
effects, or any `vowc` flag — the change is confined to a repo-local review script and its CI wiring.
`docs/equivalence/README.md` is the spec for *this* programme and is updated instead.

**No `compiler/*.vow` and no `crates/` change.** The CLAUDE.md rule that language changes must land in
both compilers does not engage: this PR does not touch either compiler.

## 3. TDD slices

Each slice is red → green → refactor, with the test written first. Slices 1–3 are the chunking
vertical; 4 is the ledger; 5 the CI wiring; 6 the soundness variant; 7 the docs.

### Slice 1 — split a file into function units without losing a byte

- **Test** (`scripts/test_pair_review.py`, new `SplitUnitsTest`):
  - `test_units_reassemble_the_file_byte_for_byte` — for each of
    `compiler/lower.vow`, `compiler/c_emitter.vow`, `vow-ir/src/lower/mod.rs`,
    `vow-types/src/check.rs`, `preamble + "".join(u.text for u in units)` equals the file text exactly.
    This is the coverage guarantee and the reason a mis-placed boundary is harmless: it can shift code
    between chunks but can never drop it.
  - `test_vow_split_finds_every_top_level_fn` — unit count for `compiler/lower.vow` equals
    `grep -c '^fn '` (135); same shape for `compiler/lexer.vow` (14).
  - `test_rust_split_finds_methods_inside_impl_blocks` — `vow-ir/src/lower/mod.rs` yields 140 units and
    the list contains both a free function (`lower_expr`) and an `impl` method (`merge_inst_ty`).
  - `test_preamble_holds_the_leading_declarations` — the preamble of `compiler/lower.vow` is non-empty
    and contains no `\nfn ` at column 0.
- **Production**: `Unit` namedtuple `(name, text)`; `RUST_FN` / `VOW_FN` module-level `re.compile`
  patterns; `split_units(text, pattern) -> (preamble, [Unit])` slicing from each match start to the
  next match start (EOF for the last). Rust pattern tolerates leading indentation and the
  `pub(crate) / async / unsafe / const / extern "C"` prefixes; Vow pattern anchors at column 0 (the
  self-hosted compiler has only free functions).

### Slice 2 — pair units across the two languages and plan chunks

- **Test** (`ChunkPlanTest`):
  - `test_related_matches_the_receiver_prefix_convention` — `related("lctx_merge_inst_ty",
    "merge_inst_ty")` is true, `related("lower_expr", "lower_expr")` is true,
    `related("lower_expr", "lower_stmt")` is false. (The self-hosted side prefixes methods with the
    receiver, e.g. Rust `link_phi_input` ↔ Vow `lctx_link_phi_input`; the rule is `a == b` or one name
    ends with `"_" + other`, with no hard-coded prefix list.)
  - `test_every_unit_lands_in_exactly_one_chunk` — over the real `lower` pair, the union of chunk
    membership covers every self-hosted unit exactly once and every Rust unit at least once.
  - `test_unmatched_rust_units_get_their_own_chunk` — a Rust function with no self-hosted counterpart
    is still reviewed (that asymmetry is itself a finding class: a check the self-hosted side never
    performs). Assert on a synthetic pair, not a live file, so the test does not rot.
  - `test_chunks_respect_the_byte_budget` — with `chunk_bytes=40_000` no chunk's rendered size exceeds
    the budget unless a single unit alone does.
  - `test_oversize_unit_is_reported_not_silently_dropped` — a synthetic 200 KB single function with a
    50 KB budget yields one chunk flagged in `oversize_units`, and the unit's full text is still
    present.
  - `test_lower_pair_chunk_count_is_bounded` — the real `lower` pair plans ≤ 12 chunks at the default
    budget, so a monthly run's cost is predictable.
- **Production**: `related(a, b)`; `plan_chunks(rust_units, self_units, chunk_bytes) -> [Chunk]`
  walking self-hosted units in source order, pulling each unit's name-related Rust units in on first
  use, closing a chunk when the running size (preamble + accumulated) would exceed the budget, and
  emitting a trailing chunk (or chunks) for Rust units never matched. `render_chunk(chunk, preambles)`
  produces the `=== RUST: <path> ===` / `=== SELF-HOSTED: <path> ===` body, with a header naming
  chunk *i* of *n* so the model knows it is seeing a slice.

### Slice 3 — the harness reviews chunk by chunk and reports coverage honestly

- **Test** (`ReviewReportTest`, with `llm.chat` monkey-patched — no network, no key):
  - `test_dry_run_makes_no_model_call_and_emits_a_plan` — `--dry-run` over all five pairs writes
    `results.json` with a `plan` per pair (chunk count, per-chunk byte totals, coverage) and calls the
    stubbed `llm.chat` zero times. This is the credential-free way to verify the DoD's "runs over all
    five pairs, chunked".
  - `test_coverage_is_one_when_nothing_is_deferred` — every pair's `coverage` is `1.0` and
    `truncated` is `False` at the default budget, i.e. the 433 KB that today's `--max-bytes` silently
    drops is gone.
  - `test_deferred_chunks_are_reported_and_coverage_drops` — with `--max-chunks-per-pair 2`, the pair
    record lists `chunks_deferred` and `coverage < 1.0`, and the printed summary names them. A run that
    reviewed a fraction of the surface must never read as a clean bill of health.
  - `test_findings_carry_their_chunk_index` — a stubbed two-chunk review attributes each finding to the
    chunk that produced it, so a reviewer can reproduce the prompt.
  - `test_unparseable_model_reply_in_one_chunk_does_not_lose_the_others` — one chunk returning prose
    records an `error` for that chunk while the sibling chunk's findings survive.
- **Production**: `review_pair` loops over the plan, calls `llm.chat` per chunk, and accumulates
  findings, per-chunk token counts and per-chunk errors. `--max-bytes` is replaced by `--chunk-bytes`
  (default `120_000`); new `--dry-run` and `--max-chunks-per-pair` (default `0` = unlimited).
  `read_pair` is deleted; its three existing tests are rewritten against `render_chunk`. `--dry-run`
  also skips `main()`'s compiler-existence check: no gate runs, so requiring `target/release/vow` and
  `build/vowc` would make the plan unverifiable on a machine that has not bootstrapped. The existing
  confirmation gate (`confirm`) and the confirmed/hypothesis/refuted accounting are untouched —
  that is the part of the harness that already works.

### Slice 4 — the ledger is actually written, so the next run is incremental

- **Test** (`LedgerWritebackTest`, against a temp copy of the ledger):
  - `test_writeback_stamps_hash_date_and_outcome` — after a stubbed clean review, the pair entry has
    the new `content_hash`, `last_reviewed` equal to the supplied `--date`, and `outcome: "clean"`.
  - `test_outcome_reflects_the_strongest_verdict` — a runner-confirmed finding writes
    `outcome: "confirmed"`; unconfirmed claims only write `"hypotheses"`.
  - `test_partially_reviewed_pair_is_not_stamped` — a pair with deferred chunks or a chunk error keeps
    its **old** hash and `last_reviewed`, so the next run re-reviews it. This is what keeps a budget-
    truncated run from masquerading as complete coverage without needing a schema change.
  - `test_corpus_and_untouched_pairs_survive_verbatim` — the `corpus` block (which tier-1 parity
    suppressions in `scripts/parity.py` and the tier-2 sweep both read) is byte-identical after a
    single-pair writeback, and pairs not reviewed in this run are unchanged.
  - `test_written_entry_matches_the_schema_key_set` — read `docs/equivalence/ledger.schema.json` and
    assert the written entry's keys are a subset of `properties` and a superset of `required`. Use the
    same dependency-free technique `scripts/test_equivalence.py:951` already uses; **do not** import
    `jsonschema` — CI runs these files with bare `python3`, outside the `bench` uv project.
  - `test_writeback_is_off_by_default` — a run without `--update-ledger` leaves the file untouched.
- **Production**: `write_ledger(ledger, results, date, path)` doing a read-modify-write of only the
  `pairs` keys this run fully reviewed, then `Path.write_text` to a sibling temp file and `os.replace`
  for atomicity. New flags `--update-ledger` (default off) and `--date YYYY-MM-DD` (required when
  `--update-ledger` is passed). The date is supplied by the caller, never `date.today()`: the ledger
  schema states outright that `updated` is *"stamped by the caller, not derived in-process: workflow
  scripts here must stay deterministic."* Preserve `rust`, `self_hosted` and `confirmed_issues` from
  the prior entry — this script does not file issues, the `/equivalence-review` operator does.

### Slice 5 — CI runs the harness tests

- **Test**: the CI change is itself the test. Locally, `python3 scripts/test_pair_review.py` and
  `python3 scripts/test_verifier_runtime.py` must both pass before and after.
- **Production**: two new steps in `.github/workflows/ci.yml`'s `python-tests` job, placed beside the
  existing `test_equivalence.py` / `test_parity.py` steps. Separate `run:` steps, never `&&`-chained,
  so a failure in one cannot skip the other.

### Slice 6 — the model-vs-language soundness variant

- **Test** (`SoundnessModeTest`):
  - `test_soundness_prompt_asks_about_assume_narrowing` — the soundness system prompt names
    `__ESBMC_assume` and asks for a program the verifier proves but the language permits to violate.
  - `test_soundness_prompt_demands_a_module_header_and_permits_an_empty_answer` — the two prompt
    properties #1091 found by testing must hold in *both* prompts, not just the equivalence one.
  - `test_soundness_verdict_mapping` — `check_soundness` returning `SOUNDNESS` maps to `confirmed`,
    `ok` to `refuted`, and `not-applicable` / `skipped` to `inconclusive`. Stub `check_soundness`; do
    not shell out to ESBMC in a unit test.
  - `test_soundness_mode_defaults_to_the_c_emitter_pair` — the other four pairs do not emit a C model,
    so reviewing them under this question would burn budget on nothing.
- **Production**: `--mode {equivalence,soundness}` (default `equivalence`); `SYSTEM_SOUNDNESS`
  alongside `SYSTEM`; `confirm_soundness(program, verifier, timeout)` importing
  `verifier_runtime.check_soundness` (add `REPO_ROOT / "scripts"` to `sys.path` next to the existing
  `bench` insert) and mapping its verdict. The gate stays mechanical, which is the whole point: a
  soundness claim is a hypothesis until `vow verify` says `Verified` **and** the `--mode debug` binary
  reports a `VowViolation`. Soundness runs write to the same ledger pair entry only when
  `--update-ledger` is passed, and never overwrite an equivalence-mode outcome with a soundness one
  (distinct question, distinct evidence) — record the mode in `results.json` and skip ledger writeback
  in soundness mode.

### Slice 7 — docs, and the honest note about what this PR does not do

- `docs/equivalence/README.md`: replace the truncation caveat with the chunking guarantee and state the
  ledger is now machine-written; add one sentence naming the soundness variant as a separate question
  with a separate gate.
- `.claude/commands/equivalence-review.md`: Step 2 gains the new flags and the instruction to run
  `--dry-run` first and paste the chunk plan into the report; a new Step 2b for `--mode soundness`;
  Step 3.5's "record it in the ledger" now says *the harness writes the pair rows, you write the
  corpus rows and issue numbers*.
- The implementation stage must post a `gh issue comment 1083` recording the one judgement call it
  cannot avoid: **no model was called in this PR.** `ANTHROPIC_API_KEY` and `OPENAI_API_KEY` are both
  absent from the autonomous run environment (verified), so an actual review would fail at the first
  `llm.chat`. The `--dry-run` chunk plan over all five pairs goes in the PR body as the evidence that
  the harness runs over all five pairs, chunked; the credentialed review, the issue filing, and the
  fixture promotion remain the first `/equivalence-review` cycle's job, exactly as #1091's own comment
  set out. Do not fabricate a review run.

## 4. Verification surface

Nothing in this change reaches ESBMC. No contract is written, weakened or bounded; no C model is
emitted; no IR is lowered. There is no property for ESBMC to prove and no new obligation on any
existing proof.

No new fixture is needed under `tests/run/` or `examples/`. Fixtures are the *output* of a
credentialed tier-3 run (Step 3.4 of the `/equivalence-review` command promotes each confirmed
reproducer), not an input to this harness change, and inventing one here without a runner verdict would
violate the issue's central rule.

The synthetic pairs used by slices 2, 3 and 6 live inside `tempfile.TemporaryDirectory()` in the test
module — no new files under `tests/`.

## 5. Risk areas

- **Binary fixed point** — not at risk. No `compiler/*.vow` file, no `vow-clif-shim` stack-slot layout,
  no `BTreeMap`/`HashMap` choice and no codegen ordering is touched. `scripts/bootstrap.sh` is not
  invoked by this change.
- **`parse → print → parse` idempotency** — not at risk; no printer, parser or AST change.
- **`cargo clippy --all -- -D warnings` / `cargo fmt`** — not at risk; no Rust source changes. (Per the
  recorded gate shape, CI runs clippy without `--all-targets`; irrelevant here either way.)
- **Model spend is the real risk.** Chunking replaces one truncated call per pair with roughly 5–12
  calls, so a full five-pair `--all` review gets materially more expensive. Mitigations, all in the
  plan: `--dry-run` prints the plan and byte totals before anything is spent; `--max-chunks-per-pair`
  caps a run; the ledger's content hash means a quiet month costs nothing; and a budget-capped pair is
  *not* stamped in the ledger, so cost control can never be mistaken for coverage.
- **Ledger corruption.** `docs/equivalence/ledger.json` is shared state: `scripts/parity.py` and
  `scripts/equivalence.py` both read its `corpus` block for tier-1 and tier-2 suppressions. A bad
  writeback would silently disarm those. Mitigations: writeback is off by default, touches only
  `pairs`, is validated against the schema's key set before writing, is written atomically via
  `os.replace`, and has a test asserting `corpus` survives byte-identically.
- **Regex splitting is approximate.** `fn` inside a string literal or a comment would create a spurious
  unit boundary. The byte-exact reassembly test in slice 1 bounds the damage precisely: a wrong
  boundary can only move code between chunks, never drop it, and coverage stays 1.0. Do not chase
  a full parser here — that would be a much larger change for no coverage gain.
- **Chunking loses whole-file context.** A divergence whose two halves sit in different chunks may go
  unseen. This is a real and accepted trade against truncation, which sees the tail *never*. Each
  chunk's prompt states it is a slice, and the preamble (type and constant declarations) is repeated in
  every chunk so a chunk is self-contained for type reasoning.
- **The gate is not being changed.** `confirm()` and the confirmed/hypothesis/refuted accounting are
  the load-bearing part of #1091 and stay untouched; the tests around them stay green as a guard.

## 6. Out of scope

- **Running a real credentialed review, filing issues, promoting fixtures.** Owned by
  `/equivalence-review`; no API key exists in this environment. Documented in an issue comment, not
  faked.
- **Refactoring `scripts/equivalence.py` or `scripts/verifier_runtime.py`.** Both are imported as-is.
- **Extending `ledger.schema.json`** with chunk-coverage fields. Deliberately avoided: not stamping a
  partially-reviewed pair achieves the same correctness with no schema migration and no churn for the
  tier-1/tier-2 readers.
- **Adding a `jsonschema` dependency to `scripts/`.** `scripts/test_equivalence.py` already made this
  call explicitly; CI runs these tests with bare `python3` outside the `bench` uv project.
- **Wiring tier 3 into CI or a nightly.** It is credentialed and costs money; `docs/equivalence/README.md`
  is explicit that it is an agent command, and #1082 owns its scheduling.
- **Adding new module pairs** beyond the five the issue lists.
- **Any change to `compiler/`, `crates/`, `docs/spec/`, `build/`, or the `symphonika/` submodule.**
- **Reformatting `scripts/pair_review.py`** beyond the functions the slices name.

## Landing

Single PR, squash-merged, title (Conventional Commits, lower-case subject, 72 chars incl. the ` (#N)`
suffix headroom):

    feat(scripts): chunk pair review by function and write the tier-3 ledger

Quality gate before opening it, as separate commands: `python3 scripts/test_pair_review.py`;
`python3 scripts/test_equivalence.py`; `python3 scripts/test_verifier_runtime.py`;
`python3 scripts/pair_review.py --dry-run --rust target/release/vow --self build/vowc --all`
(no key required; `--dry-run` needs no compiler binaries). `PLAN.md` is `git rm`'d before the PR is opened.
