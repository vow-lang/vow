# Plan: Anchor source spans for generic user-defined enum-constructor allocations (#1026)

## 1. Problem restated

In the self-hosted lowerer (`compiler/lower.vow`), `lctx_emit` deliberately does not take a
span — callers that need an instruction anchored for diagnostics must follow up with a separate
`lctx_set_inst_span(ctx, inst_id, expr_span(a, eid))` call (see the doc comment at
`compiler/lower.vow:282-287`, and the pattern fixed for seven named builtin collection constructors
by #991/#331 — `String::from`, `from_raw_parts_copy`, `String::new`, `HashMap::new`, `BTreeMap::new`,
`Vec::new`, `Vec::from_raw_parts_copy`, matching the seven `lctx_set_inst_span` calls added by commit
`1c90bf38`). The generic `EXPR_ECTOR` fallback — the `RegionAlloc` + `FIELD_SET` path taken for
every **user-defined** enum-variant construction that is not one of those seven named builtins — emits
its `ptr_id` allocation (`compiler/lower.vow:4042`) and never patches its span. Any `RegionRootEscape`
note anchored to that allocation surfaces with `span.length == 0`, unanchored to source, for every
user-defined enum constructor in every Vow program — a much wider blast radius than #991's narrowly
scoped fix (confirmed by static analysis in §3 slice 2 below, not just the issue's assertion — the
zero-length prediction is correct for this opcode specifically, as opposed to the different
"anchored to the whole enclosing function" failure mode #1262 found on a `Call`-based path). The
Rust compiler (`vow-ir/src/lower/mod.rs`) does not have this bug class at all: its
`ctx.emit(...)` takes `span` as a direct, non-optional parameter (see `vow-ir/src/lower/mod.rs:3282-3288`,
the Rust twin of this exact code path), so the span is always attached at emission time. This is a
self-hosted-only fix, mirroring #991's precedent (commit `1c90bf38`), which also touched only
`compiler/lower.vow` + tests, not `vow-ir`.

## 2. Files to touch

- `compiler/lower.vow` — one-line fix: add `lctx_set_inst_span(ctx, ptr_id, expr_span(a, eid));`
  immediately after the `ptr_id` allocation in the generic `EXPR_ECTOR` fallback
  (currently `compiler/lower.vow:4042`, just before the existing `lctx_tag(ctx, ptr_id, enum_name);`
  at line 4043 — matching the order used by the `EXPR_SLIT` struct-literal path at lines 3852-3854:
  emit → `lctx_set_inst_span` → `lctx_tag`).
- `tests/run/region_enum_ctor_root_escape_span.vow` — new regression fixture (see slice 1 below).
- `vow/tests/region_summary_equivalence.rs` — new self-hosted-only test asserting the fixture's
  `RegionRootEscape` note carries the exact pinned `span.length == 12` (see slice 2 below).
- **No changes to `vow-ir/src/lower/mod.rs`, any other root-level Rust crate, or `docs/spec/*.md`.**
  This is a self-hosted-only bug (see §1) with no language, CLI, or builtin-signature surface
  change, so none of the spec-sync triggers in `CLAUDE.md`'s "Canonical Source of Truth" section
  apply. Precedent: commit `1c90bf38` (#991) made the identical class of fix and touched only
  `compiler/lower.vow` + tests.

## 3. TDD slices

1. **Red: add the reproduction fixture.**
   Create `tests/run/region_enum_ctor_root_escape_span.vow`, structurally identical to
   `tests/run/region_vec_new_root_escape_span.vow` / `region_string_from_root_escape_span.vow`
   but exercising a **user-defined** enum variant instead of a named builtin collection
   constructor:

   ```vow
   // TEST: stdout ""
   // Issue #1026 regression: an inline user-defined enum constructor is published
   // through a parameter container, so its allocation must surface as a
   // RegionRootEscape note anchored to the constructor expression (not
   // span.length == 0).
   module RegionEnumCtorRootEscapeSpan

   enum Box {
       Item(i64),
   }

   fn push_item(boxes: Vec<Box>) {
       boxes.push(Box::Item(5));
   }

   fn main() -> i32 {
       let boxes: Vec<Box> = Vec::new();
       push_item(boxes);
       0
   }
   ```

   This is a `// TEST: stdout ""` fixture (consistent with the sibling fixtures), so
   `scripts/full_test.sh`/`tests/run_tests.sh` only assert it runs and exits cleanly — it does
   not by itself assert anything about spans. Its purpose at this stage is just to confirm the
   program is accepted (compiles, pushes a fresh enum-variant allocation into a parameter
   `Vec<Box>`, triggering the root-escape routing) before the diagnostic-content test is added.

2. **Red: add the diagnostic-content assertion (fails on current code).**
   In `vow/tests/region_summary_equivalence.rs`, add
   `selfhosted_enum_ctor_root_escape_note_span_is_call_site_not_zero_length`, modeled on
   `selfhosted_vec_new_root_escape_note_carries_nonzero_span_length` (lines 1034-1094) **and**
   `selfhosted_string_trim_root_escape_note_span_is_call_site_not_whole_function` (lines
   1178-1238) — pin the exact length, not just "nonzero". A bare `length > 0` check is not by
   itself sufficient evidence of correctness: `selfhosted_string_trim_...` documents a sibling
   bug (#1262) where an unfixed call site anchored its note to the *whole enclosing function*,
   which also has nonzero length, so a `> 0` check alone would not have caught it. Pin
   `span.length == 12` (`Box::Item(5)` is exactly 12 bytes) for every `RegionRootEscape` note:
   - Skip cleanly if `build/vowc` is absent (same `vowc.exists()` guard as every sibling test).
   - Run `build/vowc build --no-verify` on the new fixture.
   - Parse stdout as JSON; tolerate the documented `libvow_runtime.a` link-only failure via
     `self_hosted_runtime_link_failure`.
   - Assert at least one `RegionRootEscape` diagnostic fires (confirms the fixture actually
     routes to root — if it didn't, the span assertion below would vacuously pass).
   - Assert **every** such note has `span.length == 12`.

   **Why the zero-length prediction (not the whole-function one) is the right one to pin here:**
   confirmed by reading `root_escape_span_start`/`root_escape_span_len`
   (`compiler/region.vow:1365-1389`) rather than just trusting the issue body. Both helpers return
   `inst.ostart`/`inst.olen` directly unless `inst.olen > 0`, with exactly one fallback: when
   `inst.op == IOP_CALL()`, they borrow the span from the call's first argument. The six/seven
   named builtins (where #1262's whole-function bug lives) all emit via `IOP_CALL()` to an extern
   symbol, so that fallback applies to them. The generic `EXPR_ECTOR` fallback's `ptr_id` is an
   `IOP_REGION_ALLOC()`, not a `Call` — the fallback branch does not apply to it — so with no
   patched span it falls straight through to the raw `ir_inst_new` default of `ostart: 0, olen: 0`
   (`compiler/ir.vow:315-329`, always zero regardless of any span argument, since `ir_inst_new`
   takes no span parameter at all). So `span.length == 0` is the predicted failure mode for this
   specific opcode, not the whole-function one — the fixture and assertion are built around that
   confirmed mechanism, not a guess.
   **Stop-condition:** if this test unexpectedly passes *before* the production fix lands (step 3),
   do not proceed to "green" — it means the fixture isn't reaching the generic `EXPR_ECTOR`
   fallback (e.g. `Box::Item` accidentally resolved to a different lowering path), and the fixture
   needs to be revised until it demonstrably exercises `compiler/lower.vow:4042` before continuing.
   Confirm the fail by rebuilding `build/vowc` via `scripts/bootstrap.sh --skip-cargo` and running
   this test against the *current*, unfixed `compiler/lower.vow`.

3. **Green: the one-line production fix.**
   Add `lctx_set_inst_span(ctx, ptr_id, expr_span(a, eid));` at `compiler/lower.vow` in the
   generic `EXPR_ECTOR` fallback, per §2. Rebuild (`scripts/bootstrap.sh --skip-cargo`) and
   re-run the slice-2 test; it must now pass.

4. **Refactor: none planned.** This is a single-line, surgical fix with no structural follow-up
   inside `lower.vow` — there is no refactor step beyond the fix itself. (Do not use this as an
   opportunity to touch the five other `lctx_set_inst_span` call sites or restructure
   `EXPR_ECTOR` lowering — out of scope, see §6.)

## 4. Verification surface

- **No contract, codegen, or C-model changes.** This fix touches only diagnostic span metadata
  (`IrInst.ostart`/`olen`, patched post-hoc by `lctx_set_inst_span`) — it does not change the
  emitted IR opcodes, operand order, allocation size/alignment, or any ESBMC-visible behavior.
  ESBMC never sees `ostart`/`olen`; they are consumed only by `vow-diag`'s JSON/human emitters
  when rendering a `RegionRootEscape` note's `span` field. No new ESBMC properties need proving.
- **Fixture growth:** one new fixture under `tests/run/` (slice 1) exercising the previously-
  unanchored path, picked up automatically by both `scripts/full_test.sh` and
  `tests/run_tests.sh`'s existing `tests/run/*.vow` walk (no harness changes needed — it is a
  plain `// TEST: stdout ""` fixture like its siblings).
- **No `examples/` changes** — this is a regression-fixture-only concern, not a user-facing
  example worth promoting.
- Run `cargo test -p vow --test region_summary_equivalence` after the fix to confirm the new test
  passes and no sibling test in that file regresses (the file is shared across many `RegionRootEscape`
  span/parity regression tests — see the file's existing issue #316/#320/#326/#331/#1262 tests).

## 5. Risk areas

- **Binary fixed point:** `lctx_set_inst_span` mutates `ostart`/`olen` on an already-emitted
  `IrInst` in place (scanning `ctx.func.blocks`, with a fast path for the common case where the
  target is the last instruction in the current block — see `compiler/lower.vow:308-348`). It
  does not change instruction count, ordering, opcodes, or operand encoding, so it cannot affect
  Cranelift codegen, stack-slot layout in `vow-clif-shim`, or the three-stage bootstrap fixed
  point (`compiler_a`/`compiler_b`/`compiler_c` must still hash-match). Confirm this explicitly by
  re-running the bootstrap triple test (`scripts/concat_vow.sh` + the three-stage build +
  `sha256sum`) after the fix, since this fix is inside `compiler/lower.vow`, which is part of the
  self-hosted compiler's own source and therefore part of what bootstraps itself.
- **`parse → print → parse` idempotency:** unaffected — this fix is entirely inside the lowerer
  (AST → IR), not the lexer/parser/printer. No printer-visible syntax changes.
- **`cargo clippy --all -- -D warnings`:** unaffected — no Rust files change in this PR (see §2).
  Still worth a sanity `cargo clippy --all -- -D warnings` run since `vow/tests/region_summary_equivalence.rs`
  is Rust source being added to, even though its content is a new `#[test]` fn following the
  exact structure of its neighbors (low risk of a new lint).
- **Ordering sensitivity in `lctx_set_inst_span`'s fast path:** the fast path assumes the target
  instruction is the *last* instruction pushed to the *current* block. In the `EXPR_ECTOR`
  fallback, `ptr_id`'s `REGION_ALLOC` is immediately followed only by `lctx_tag` (a bookkeeping
  call with no `lctx_emit`, confirmed by reading lines 4042-4078) until the tag-value `CONST_I64`
  emit at line ~4063 — so inserting `lctx_set_inst_span` right after the `ptr_id` emit (before any
  other `lctx_emit` call) keeps `ptr_id` as the last instruction in the block when the patch runs,
  hitting the fast path. Do not move the new call below the `tag_id`/`FIELD_SET` emits, which
  would push it onto the slower full-scan path (still correct, just slower — but also diverges
  from the established call-site convention of patching immediately after emission).
- **Test-file churn:** `vow/tests/region_summary_equivalence.rs` is a single large file with many
  independent `#[test]` fns; append the new test at the end (or near its two nearest siblings,
  `selfhosted_vec_new_root_escape_note_carries_nonzero_span_length` /
  `selfhosted_string_from_root_escape_note_carries_nonzero_span_length`) rather than reordering
  existing tests, to keep the diff reviewable.

## 6. Out of scope

- Auditing or fixing any *other* `lctx_emit` call site for a missing `lctx_set_inst_span` — issue
  #1262 already did a pass for free-function builtin dispatch, and #991 did one for the seven named
  constructors. This issue is scoped to exactly the one fallback branch named in the issue body.
  If a future audit finds more gaps, they get their own issue (same deferral pattern that produced
  #1026 itself).
- Restructuring `EXPR_ECTOR` lowering (e.g. deduplicating the seven named-builtin special cases
  against the generic fallback) — a legitimate deepening opportunity, but a refactor bundled into
  a bug fix, which `CLAUDE.md`'s "Surgical changes" principle rules out here.
- Adding a Rust-side (`vow-ir`) test or fix — there is nothing to fix on that side (see §1); adding
  a same-shaped Rust test would be testing code that was never buggy, which is not what TDD red/green
  is for.
- Enforcing "every `lctx_emit` call site must pair with `lctx_set_inst_span`" mechanically (e.g. a
  linter or wrapper that forces span to be passed at emission time, matching Rust's `ctx.emit(...,
  span)` shape). That is a real structural fix for this whole bug *class*, but it is a design
  change to `lctx_emit`'s signature affecting every call site in `lower.vow`, not a surgical fix —
  explicitly out of scope for a single deferred-follow-up issue.
- Any change to `docs/spec/*.md`, `docs/design/arena_memory.md`, or the `RegionRootEscape`
  error-code documentation in `docs/spec/errors.md` — the documented behavior (notes are anchored
  to source) is already correct as written; only the self-hosted implementation was out of sync
  with it.
