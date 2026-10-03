# Plan: issue #1265 — `heap_producing_extern` omits BTreeMap and non-i64 `parse_*_opt` externs

## 1. Problem restated

`heap_producing_extern` (self-hosted `compiler/region.vow`, mirrored in Rust
`vow-ir/src/region.rs`) is the allow-list the region-escape pass uses to decide
whether a `Call`'s return value is a fresh heap allocation worth tracking for
`RegionRootEscape`. Its `map_creation_extern` sub-list only recognizes
`__vow_map_new`/`__vow_map_new_in_arena` (HashMap), omitting the three
`BTreeMap` externs (`__vow_btreemap_new`, `__vow_btreemap_insert`,
`__vow_btreemap_get`); its `option_creation_extern` sub-list only recognizes
`__vow_string_parse_i64_opt`/`_in_arena`, omitting `__vow_string_parse_u64_opt`
and the whole narrow-integer family (`parse_i8/u8/i16/u16/i32/u32_opt`). Per
`vow-runtime/src/lib.rs`, every omitted symbol allocates a fresh heap object
with the same shape as an already-tracked sibling (`__vow_btreemap_new` mirrors
`__vow_map_new`'s header-alloc; `__vow_btreemap_insert`/`_get` return a fresh
`Option<i64>` via `alloc_option_i64` — byte-identical to
`__vow_string_parse_i64_opt`'s own allocation; every narrow parser builds its
`Option<N>` via the same `__vow_vec_new(8, 8)` shape as the i64 variant). Because
`is_heap_producing_for_region`/`is_heap_producing` return `false` for all of
these, `RegionRootEscape` silently never fires when one of these values is
published through a parameter container — confirmed empirically in the issue
for `BTreeMap::new()`.

## 2. Files to touch

Production code (both compilers, same session, per `CLAUDE.md`):

- `vow-ir/src/region.rs` — `map_creation_extern` (~line 1967),
  `option_creation_extern` (~line 1960).
- `compiler/region.vow` — `map_creation_extern` (~line 4309),
  `option_creation_extern` (~line 4304).

Tests:

- `tests/run/region_btreemap_new_root_escape_span.vow` (new fixture).
- `tests/run/region_btreemap_insert_get_root_escape_span.vow` (new fixture).
- `tests/run/region_parse_opt_root_escape_span.vow` (new fixture; representative
  of the narrow-parse family — see §3, slice 3).
- `vow/tests/region_summary_equivalence.rs` — add one Rust-side test and one
  self-hosted test per new fixture, mirroring the existing
  `selfhosted_vec_new_root_escape_note_carries_nonzero_span_length` /
  `rust_param_field_mutation_emits_root_escape_note` pairs.

Docs: **none required.** `docs/spec/errors.md`'s `RegionRootEscape` section
documents the diagnostic's semantics generically (root-region placement via
store-effect or widen), not the specific extern allow-list — grepped and
confirmed no doc enumerates `heap_producing_extern`'s member symbols. This is
an internal classifier bug fix, not a change to syntax, semantics, builtins,
operators, effects, or CLI flags, so the "any change must update
`docs/spec/*.md`" rule in `CLAUDE.md` does not apply here. `CHANGELOG.md` is
semantic-release-generated — not hand-edited.

## 3. TDD slices

Each slice: write the fixture + both test functions first (confirm they fail
against current `HEAD`, i.e. zero `RegionRootEscape` notes), then make the
minimal production-code addition that turns both green. Build `build/vowc`
fresh with `scripts/bootstrap.sh --skip-cargo` (or a full bootstrap if cargo
artifacts are stale) between the Rust-side fix and the self-hosted-side fix,
since the self-hosted test exercises the compiled `build/vowc` binary, not the
Rust source directly.

**Slice 1 — `__vow_btreemap_new`.**
- Fixture `tests/run/region_btreemap_new_root_escape_span.vow`, mirroring
  `region_vec_new_root_escape_span.vow`'s shape exactly:
  ```vow
  // TEST: stdout ""
  module RegionBTreeMapNewRootEscapeSpan

  fn push_empty(maps: Vec<BTreeMap<i64, i64>>) {
      maps.push(BTreeMap::new());
  }

  fn main() -> i32 {
      let maps: Vec<BTreeMap<i64, i64>> = Vec::new();
      push_empty(maps);
      0
  }
  ```
- Test `rust_btreemap_new_root_escape_note` in
  `vow/tests/region_summary_equivalence.rs`: run `CARGO_BIN_EXE_vow build
  --no-verify` on the fixture, parse diagnostics, assert at least one
  `RegionRootEscape` note. Pattern after
  `rust_param_field_mutation_emits_root_escape_note`.
- Test `selfhosted_btreemap_new_root_escape_note`: run `build/vowc build
  --no-verify` on the same fixture (skip if `build/vowc` absent, same
  guard as existing self-hosted tests), same assertion. Pattern after
  `selfhosted_vec_new_root_escape_note_carries_nonzero_span_length` (drop the
  span-length-specific assertions — this slice only needs "note fires at all").
- Production fix: add `"__vow_btreemap_new"` to `map_creation_extern` in both
  `vow-ir/src/region.rs` and `compiler/region.vow`. `__vow_btreemap_new` has no
  `_in_arena` sibling (confirmed by grep over `vow-runtime/src/lib.rs`), so this
  is a single-arm addition in each file, not a pair.

**Slice 2 — `__vow_btreemap_insert` / `__vow_btreemap_get`.**
- Fixture `tests/run/region_btreemap_insert_get_root_escape_span.vow`:
  ```vow
  // TEST: stdout ""
  module RegionBTreeMapInsertGetRootEscapeSpan

  fn push_results(results: Vec<Option<i64>>, m: BTreeMap<i64, i64>) {
      results.push(m.insert(1, 2));
      results.push(m.get(1));
  }

  fn main() -> i32 {
      let results: Vec<Option<i64>> = Vec::new();
      let m: BTreeMap<i64, i64> = BTreeMap::new();
      push_results(results, m);
      0
  }
  ```
  This exercises both `insert` (which returns the *previous* value as
  `Option<i64>`, a fresh allocation regardless of hit/miss) and `get` in one
  fixture, since both return the identical `alloc_option_i64`-shaped value.
- Tests `rust_btreemap_insert_get_root_escape_note` and
  `selfhosted_btreemap_insert_get_root_escape_note`, same pattern as slice 1.
- Production fix: add `"__vow_btreemap_insert"` and `"__vow_btreemap_get"` to
  **`option_creation_extern`**, not `map_creation_extern` — a deliberate
  deviation from the issue body's literal grouping. Rationale: `heap_producing_extern`
  is a pure OR across its sub-lists, so classification is unaffected either
  way, but `__vow_btreemap_insert`/`_get`'s return value is byte-for-byte the
  same `Option<i64>` shape as `__vow_string_parse_i64_opt` (both go through
  `alloc_option_i64` / `__vow_vec_new(8, 8)` with a `[tag, payload]` layout) —
  grouping by allocation shape, not by source type, keeps the sub-list names
  meaningful for the next reader. `map_creation_extern` stays reserved for
  externs that allocate the *map itself* (header + backing buffers), which is
  what `__vow_btreemap_new` does and `insert`/`get` do not. Neither symbol has
  an `_in_arena` sibling.

**Slice 3 — narrow + u64 `parse_*_opt`.**
- Fixture `tests/run/region_parse_opt_root_escape_span.vow`, using
  `parse_u64` as the representative case (matches the issue title's primary
  callout; the other five — `parse_i8/u8/i16/u16/i32/u32` — share the exact
  same `define_narrow_parser!`/`alloc_option_*` → `__vow_vec_new(8, 8)` shape
  as `parse_u64` and the already-tracked `parse_i64`, so one executable
  regression test per *new allocation shape* matches this codebase's existing
  precedent — e.g. `vec_creation_extern`/`string_creation_extern` each cover
  many symbols behind a single `Vec::new()`/`String::from()` fixture, not one
  fixture per symbol):
  ```vow
  // TEST: stdout ""
  module RegionParseOptRootEscapeSpan

  fn push_parsed(results: Vec<Option<u64>>, s: String) {
      results.push(s.parse_u64());
  }

  fn main() -> i32 {
      let results: Vec<Option<u64>> = Vec::new();
      push_parsed(results, String::from("42"));
      0
  }
  ```
  (Confirm exact method-call syntax for `String.parse_u64()` against
  `docs/spec/grammar.md`'s String Methods table before finalizing — grep
  showed `.parse_u64()` listed as `() -> Option<u64>`.)
- Tests `rust_parse_u64_opt_root_escape_note` and
  `selfhosted_parse_u64_opt_root_escape_note`, same pattern as slices 1–2.
- Production fix: add all seven symbols — `"__vow_string_parse_u64_opt"`,
  `"__vow_string_parse_i8_opt"`, `"__vow_string_parse_u8_opt"`,
  `"__vow_string_parse_i16_opt"`, `"__vow_string_parse_u16_opt"`,
  `"__vow_string_parse_i32_opt"`, `"__vow_string_parse_u32_opt"` — to
  `option_creation_extern` in both files in the same edit (they are one
  data-table addition, not six independent behavioral changes; splitting them
  into six commits would be process overhead without a corresponding increase
  in review signal, since the executable test already proves the shared code
  path). None of the six has an `_in_arena` sibling (confirmed by grep); only
  `parse_i64_opt` does, and it is already handled.

After all three slices: run `cargo test -p vow --test region_summary_equivalence`
(Rust) and re-run the affected `tests/run/*.vow` fixtures through
`scripts/full_test.sh`'s promoted-fixtures path (or `tests/run_tests.sh` locally
for the stricter stderr check) to confirm no regression in existing
HashMap/Vec/String coverage.

## 4. Verification surface

This change touches region-escape **diagnostics**, not contracts, codegen, or
the C model — `heap_producing_extern` feeds `RegionRootEscape`/`RegionConflict`
note/error placement, which is a compile-time analysis, not an ESBMC-checked
property. No new ESBMC obligations, no new verifier-model entries in
`docs/spec/operations.json` (that catalogue currently covers only
`print_str`/`print_i64`/`print_u64`; BTreeMap/parse externs are out of its
scope per its own stated narrowing). No `examples/` fixtures need to grow —
`tests/run/*.vow` is the correct location and already has the harness
(`scripts/full_test.sh` Section 4) that checks `TEST: stdout`/`exit` for these
exact three new fixtures. Each new fixture needs `// TEST: stdout ""` and must
actually compile and link successfully under `--no-verify` for the harness to
reach the diagnostics-parsing step in the Rust test file (the `.vow` fixture
itself is executed by the shell harness only for exit/stdout; the JSON
diagnostics assertions live entirely in `region_summary_equivalence.rs`, run
via `cargo test`, as established by the existing pattern).

## 5. Risk areas

- **Binary fixed point.** The fix only adds string-literal match arms to two
  pure classifier functions; it does not change codegen, stack-slot layout, or
  IR shape. Low risk to `stage1 == stage2` sha256 parity, but slice-by-slice
  bootstrap-and-diff (per the TDD plan in §3) catches any surprise immediately
  rather than at PR time.
- **`BTreeMap` vs `HashMap` parity drift.** `map_creation_extern` is now
  asymmetric by design (`__vow_btreemap_new` only; `insert`/`get` live in
  `option_creation_extern`) where `HashMap`'s `__vow_map_insert` returns `()`
  (no fresh allocation, correctly absent from every sub-list) — this is a
  genuine semantic difference between the two map types' insert signatures,
  not an oversight, and should not be "fixed" by adding `__vow_map_insert` to
  any list.
- **`parse → print → parse` idempotency.** Unaffected — no AST/printer change.
- **`cargo clippy --all -- -D warnings`.** New `matches!` arms (Rust) and
  `||`-chained `sym == String::from(...)` arms (self-hosted) follow the
  existing style in each file exactly; no new clippy surface expected.
- **Dual-compiler drift.** Both files must be edited in the same commit/session
  per `CLAUDE.md`; the self-hosted test requires a fresh `build/vowc` (rebuilt
  via `scripts/bootstrap.sh --skip-cargo` after the self-hosted source edit) —
  forgetting the rebuild step will make the self-hosted test pass against a
  *stale* binary that doesn't contain the fix, silently defeating the test.
  Memory note on this exact failure mode: stale compile-cache/binary served
  after a compiler rebuild at the same source rev is a known trap in this repo
  (`VOW_CACHE_DIR=$(mktemp -d)` avoids the compile-cache variant; the
  `build/vowc` binary itself must simply be rebuilt, not cache-busted).
- **Note-count assertions.** Keep assertions at "at least one `RegionRootEscape`
  note" (as most existing tests do), not an exact count — exact-count tests
  (`rust_arena_push_fixture_pins_note_count`) are pinned to specific fixtures
  whose per-block dedup behavior is already characterized; a new fixture
  asserting an exact count risks a flaky/over-fitted test if dedup behavior
  shifts for unrelated reasons.

## 6. Out of scope

- Adding `__vow_map_insert`/`__vow_map_get` (HashMap) to any sub-list — they
  correctly return `()`/non-fresh values and are not part of this issue.
- Any fixture/test for `_in_arena` variants of the newly-added symbols — none
  exist (confirmed by grep); do not invent them.
- Extending `docs/spec/operations.json`'s Operation Catalogue to cover
  BTreeMap/parse externs — that catalogue is explicitly scoped to
  `print_str`/`print_i64`/`print_u64` today; widening it is tracked by
  issues #1271–#1275, not this one.
- Touching `region_root_escape_parity.vow` or
  `region_root_escape_note_count_parity_rust_vs_self_hosted` — that parity
  fixture is unrelated to this issue's symbol set; do not fold it in.
- Any refactor of `heap_producing_extern`'s four-function OR structure into a
  single combined list/set, even though the string-matching style could be
  unified — that is a structural refactor with no behavioral motivation here
  and would bundle unrelated cleanup into a bug fix, against the "surgical
  changes" principle in `CLAUDE.md`.
- Mutation-testing (`vowc mutants`) runs against the touched functions — local
  developer cadence, not CI-gated, not required for this PR.
