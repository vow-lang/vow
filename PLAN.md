# Plan: #1272 — Migrate filesystem, stdin, args, and stderr Builtin Operations to the Operation Catalogue

## 0. Branch staleness — rebase first

This worktree's branch was cut at `0cf9d948`, **55 commits behind `origin/main`**. Critically,
`origin/main` already contains PR #1279 (issue #1270's print tracer bullet): `docs/spec/operations.json`,
`scripts/generate_operations.py`, and `// GENERATE:OPERATIONS:START/END` marker pairs in
`vow-ir/src/lower/mod.rs`, `vow-codegen/src/cranelift_backend.rs`, `vow-clif-shim/src/lib.rs`, and
`compiler/lower.vow` — none of which exist in this worktree's current checkout. Everything below was
read via `git show origin/main:<path>`, per the standing note to diff against `origin/main`, never a
bare (worktree-local) `main`. **The implementation stage's first action must be to rebase this branch
onto current `origin/main`** before touching any file named below.

A sibling worktree for issue #1271 (query/utility builtins — memory/time/CPU queries, hex helpers,
parse/string-conversion) is independently extending the same generator on top of `origin/main`, not
yet merged (no PR opened as of this planning pass). Its unmerged commits add `"i64"`, `"u64"`, `"ptr"`
entries to `RETURN_TOKENS` in `scripts/generate_operations.py`, plus optional `verifier_model`/
`arena_routing` schema fields and a second generated marker pair in `vow-verify/src/c_emitter.rs` /
`compiler/c_emitter.vow`. This plan does not depend on that work landing — #1272's only declared
blocker is #1270 — but implementation must check whether #1271 has merged before starting the
generator-token slice below (§4 step 1): if it has, the `"i64"`/`"ptr"` tokens already exist and only
`"bool"` needs adding; if not, add all three needed tokens directly (§4 step 1 lists the exact literal
values either way, so this is a mechanical rebase-and-check, not a design decision).

## 1. Problem restated

Nineteen filesystem, stdin, args, and direct-stderr Builtin Operations — the 14 `fs_*` functions,
`args`, `stdin_read`, `stdin_read_line`, `stdin_ready`, and `eprintln_str` — have their runtime-symbol,
Cranelift ABI, and IR return-shape facts hand-duplicated across `vow-ir/src/lower/mod.rs`'s
`vow_static_builtin_to_runtime` match (plus a hand-mirrored test table asserting the same tuples),
two independently-maintained Cranelift extern-signature tables (`vow-codegen/src/cranelift_backend.rs`,
`vow-clif-shim/src/lib.rs`), and self-hosted `compiler/lower.vow`'s `builtin_to_extern`/`builtin_ret_ty`
if-chains — five sites total per operation. All 19 are effectful (`Effect::Read`/`Write`/`IO` in
`vow-types/src/env.rs::builtin_free_fn_signatures()`), so — like the print tracer bullet — this slice
never reaches the verifier's `is_known_builtin`/`is_modelable` gate, which short-circuits on any
non-empty effect set before consulting the known-builtin list. This slice extends the Operation
Catalogue (`docs/spec/operations.json` + `scripts/generate_operations.py`, established by #1270/PR
#1279) with these 19 operations, extends the generator's `RETURN_TOKENS` vocabulary to cover the
`i64`, `ptr`, and `bool` IR return shapes this family needs (the print tracer bullet only ever needed
`unit`), regenerates all four projection targets, and deletes the corresponding hand-written arms —
preserving runtime behavior, effects, and return shapes exactly.

## 2. Operations in scope

Cross-checked against `vow-types/src/env.rs::builtin_free_fn_signatures()` (read-only input, unchanged)
and the existing hand tables on `origin/main`:

| Surface name | Runtime symbol | Return | Effects |
|---|---|---|---|
| `fs_read` | `__vow_fs_read` | ptr | `[read]` |
| `fs_open` | `__vow_fs_open` | i64 | `[read]` |
| `fs_read_line` | `__vow_fs_read_line` | ptr | `[read]` |
| `fs_status` | `__vow_fs_status` | i64 | `[read]` |
| `fs_close` | `__vow_fs_close` | i64 | `[read]` |
| `fs_write` | `__vow_fs_write` | i64 | `[write]` |
| `fs_exists` | `__vow_fs_exists` | i64 | `[read]` |
| `fs_mkdir` | `__vow_fs_mkdir` | i64 | `[io]` |
| `fs_listdir` | `__vow_fs_listdir` | ptr | `[read]` |
| `fs_remove` | `__vow_fs_remove` | i64 | `[io]` |
| `fs_remove_dir` | `__vow_fs_remove_dir` | i64 | `[io]` |
| `fs_is_dir` | `__vow_fs_is_dir` | i64 | `[read]` |
| `fs_is_symlink` | `__vow_fs_is_symlink` | i64 | `[read]` |
| `fs_rename` | `__vow_fs_rename` | i64 | `[io]` |
| `args` | `__vow_args` | ptr | `[read]` |
| `stdin_read` | `__vow_stdin_read` | ptr | `[read]` |
| `stdin_read_line` | `__vow_stdin_read_line` | ptr | `[read]` |
| `stdin_ready` | `__vow_stdin_ready` | **bool** | `[read]` |
| `eprintln_str` | `__vow_eprintln_str` | unit | `[io]` |

**19 operations**, all effectful, none pure. `stdin_ready` is the one operation needing a `bool`
return token — confirmed by reading `vow-codegen/src/cranelift_backend.rs`'s hand arm directly:
`"__vow_stdin_ready" => { sig.returns.push(AbiParam::new(types::I64)); // bool as i64 }`. The Cranelift
ABI collapses bool to `types::I64` (there is no Cranelift bool return type in this ABI), but the
Rust-level `Ty` and self-hosted `ITY_*` stay distinct (`Ty::Bool`/`ITY_BOOL()`), matching how
`vow_static_builtin_to_runtime` already returns `Ty::Bool` for this symbol today. This is the one
place a naive "read the Vow surface type" pattern-match would go wrong (writing an `I8` or dedicated
bool Cranelift type where `types::I64` is actually required) — call it out explicitly in code review.

Dry-run validation performed during planning (not committed — throwaway script, run from
`$TMPDIR`): built all 19 catalogue entries in-memory using facts read from `env.rs`/`grammar.md`/
`cranelift_backend.rs`/`compiler/lower.vow`, ran them through `generate_operations.py`'s actual
`extract_builtin_signatures_table` + doc cross-check logic against the real `grammar.md`/`main.vow`/
`skill.rs` from `origin/main`. Result: **all 19 already have correct, matching 3-column
`grammar.md` rows and matching `main.vow`/`skill.rs` help lines today** — no doc gaps, unlike #1271's
`parse_f64_bits`/`format_f64_bits` gap. This slice requires zero `docs/spec/*.md` content changes.

### Explicitly excluded from this slice (judgment calls)

- **`gzip_write_file`** — has no row anywhere in `grammar.md` (confirmed by grep: zero matches), is
  not `fs_`-prefixed, and is not named in the issue's category list ("filesystem operations, stdin
  helpers, args, and stderr output"). Fixing its doc gap is unrelated to this migration and would
  bundle an unrelated fix into a fact-relocation PR. Left for a future slice (likely process/utility,
  since its effect is `[io]` like the process family, not `[read]`/`[write]` like true filesystem ops).
- **`process_get_stderr` / `process_stderr_for`** — these capture *subprocess* stderr output, not
  direct stderr writes. They belong to #1273's process family, not this issue's "stderr" (`eprintln_str`,
  the direct-write analogue of `print_str`).
- **`debug_str` / `debug_i64` / `debug_u64`** — routed through a separate `vow_debug_builtin_to_runtime`
  path (elided entirely in release builds), not the generic direct-call path this catalogue slice
  covers. Not named in the issue.
- **`BuiltinResultTag`/`tag_builtin_result` (Rust) and the inline `lctx_tag` calls in `compiler/lower.vow`
  (self-hosted)** — six of these 19 ops (`fs_read`, `fs_read_line`, `stdin_read`, `stdin_read_line`,
  `args`, `fs_listdir`) also carry a *separate* heap-result classification (`String`/`Vec` tag) that
  exists solely to support `pin_to_root`'s GC/lifetime bookkeeping. This is a distinct fact from the
  Operation Catalogue's "return shape" (the ABI/IR `Ty` token already in `RETURN_TOKENS`): confirmed by
  checking the #1271 sibling worktree's actual catalogue extensions, which added 22 new operations
  including several `String`/`Vec`-tagged ones (`hex_encode`→String, `hex_decode`→Vec) **without**
  touching `BuiltinResultTag` at all. PRD #375's Implementation Decisions explicitly exclude
  `pin_to_root` from the first several catalogue slices ("Complex operation bodies stay explicit...
  first vertical slice excludes `pin_to_root`"). Leave `tag_builtin_result` and its self-hosted
  mirror completely untouched.
- **`arena_routing` / `verifier_model` catalogue fields** — not added for any of these 19 entries.
  None of the 19 runtime symbols have an `_in_arena` sibling variant (confirmed by grep across
  `vow-ir/src/lower/mod.rs`, `vow-codegen/src/cranelift_backend.rs`, `vow-clif-shim/src/lib.rs`,
  `compiler/lower.vow`, and `vow-runtime/src/lib.rs` — zero hits), and all 19 are effectful so
  `verifier_model` never applies. This matches the #1271 sibling's own practice: its effectful
  entries (`time_unix`, `memory_peak_bytes`, etc.) omit both fields entirely rather than stamping
  `"none"`/absent placeholders. `grammar.md`'s note that `fs_read_line`/`stdin_read_line` return
  "runtime scratch storage valid until the next call" is a pointer-lifetime fact, unrelated to
  arena-routing (which is specifically about the `_in_arena` root-vs-candidate-arena redirection
  convention) — correctly out of scope here.

## 3. Files to touch

**Catalogue + generator (root):**
- `docs/spec/operations.json` — append the 19 operations from §2 (each: `name`, `runtime_symbol`,
  `params`, `return`, `doc_signature`, `effects` — the same six fields the 3 existing `print_*`
  entries use; no new fields for this slice per §2's exclusions).
- `scripts/generate_operations.py` — extend `RETURN_TOKENS` (currently only `"unit"`) with:
  ```python
  "i64":  {"rust_ty": "Ty::I64",  "ity_const": "ITY_I64()",  "clif_ret": "types::I64"},
  "ptr":  {"rust_ty": "Ty::Ptr",  "ity_const": "ITY_PTR()",  "clif_ret": "types::I64"},
  "bool": {"rust_ty": "Ty::Bool", "ity_const": "ITY_BOOL()", "clif_ret": "types::I64"},
  ```
  (If #1271 has merged by this point, `"i64"`/`"ptr"` already exist with these exact spellings —
  confirmed by reading its unmerged diff directly — so only `"bool"` needs adding.) No `PARAM_TOKENS`
  change needed: all 19 operations' parameters are `ptr` and `i64`, both already present.
- `scripts/test_generate_operations.py` — `test_real_catalogue_loads_and_matches_print_ops` asserts
  `load_catalogue(REPO_ROOT) == PRINT_OPS` **exactly**; this breaks the moment the 19 entries are
  added regardless of anything else. Rename/generalize it (e.g. to
  `test_real_catalogue_loads_and_matches_known_ops`) with an expected-ops fixture covering all 22
  (3 print + 19 new). Add `test_i64_ptr_bool_return_tokens_render` covering the three new
  `RETURN_TOKENS` entries through `gen_rust_ir_block`/`gen_cranelift_block`/`gen_vow_lower_block`.

**Rust compiler:**
- `vow-ir/src/lower/mod.rs` — remove the 19 arms from `vow_static_builtin_to_runtime`'s hand match
  (`"eprintln_str"` through `"fs_rename"`, and `"args"`/`"stdin_read"`/`"stdin_read_line"`/
  `"stdin_ready"`); they fall through to `catalogue_builtin_to_runtime` (the generated function) at
  the top of the same function, unchanged. **Do not touch** `builtin_result_tag`/`BuiltinResultTag`
  (§2) or `narrow_intrinsic_target` (unrelated family). The existing test
  `builtins_lower_to_runtime_symbols_and_return_types` already asserts all 19 `(name, symbol, Ty)`
  triples — including the `stdin_ready` → `Ty::Bool` case — and must keep passing **unchanged**
  throughout (it is the regression net, not a duplicated fact to delete: it pins
  `vow_builtin_to_runtime`'s public behavior regardless of whether the answer comes from a
  hand-written arm or the generated block).
- `vow-codegen/src/cranelift_backend.rs` — remove the 19 hand arms from `make_extern_sig`'s `match
  sym` (the `__vow_fs_*` block and the `__vow_eprintln_str`/`__vow_args`/`__vow_stdin_*` block); the
  generated `catalogue_extern_sig` (already consulted first, per the print tracer bullet's wiring)
  supersedes them. Add a sweep test alongside the existing
  `catalogue_print_externs_accept_a_single_i64_and_return_nothing` asserting, for each of the 19
  runtime symbols, the expected param count and return-slot presence/type via `catalogue_extern_sig`
  directly (not just via the end-to-end compile tests) — this is the regression net for a
  transcription typo in `docs/spec/operations.json` that unit tests elsewhere wouldn't catch.
- `vow-clif-shim/src/lib.rs` — same removal in its independent `match sym` table, plus the same
  sweep-test addition. This file's unknown-symbol fallback
  (`"clif_shim: unknown extern sig for '{sym}', using no-arg no-return"`) only logs to stderr and
  silently emits a wrong ABI — the sweep test is the only thing standing between a catalogue typo and
  a silent miscompile here (see §5 Risk areas).
- `vow-verify/src/c_emitter.rs` / `compiler/c_emitter.vow` — **no changes.** Confirmed by grepping
  both files in full for every one of the 19 runtime symbols (`__vow_fs_*`, `__vow_stdin_*`,
  `__vow_args`, `__vow_eprintln_str`): zero occurrences in either `is_known_builtin` or anywhere else
  in either file. Combined with all 19 having non-empty effects in `env.rs` (so `is_modelable`
  short-circuits before `is_known_builtin` is ever reached), this operation family never touches the
  verifier classifier — satisfying the issue's "otherwise no verifier-classifier changes are needed"
  acceptance criterion with direct evidence, not an assumption.
- `vow-types/src/env.rs` — **not modified** (signatures/effects are read, not owned, by the catalogue).

**Self-hosted compiler:**
- `compiler/lower.vow` — remove the 19 `if name == ...` lines from both `builtin_to_extern` and
  `builtin_ret_ty`'s hand chains; the existing generated `catalogue_builtin_to_extern`/
  `catalogue_builtin_ret_ty` calls at the top of each function (already wired by the print tracer
  bullet) supersede them once regenerated. **Do not touch** `builtin_result_tag`'s Vow-side
  equivalent or any `_in_arena`-adjacent logic (none exists for this family, confirmed in §2).
- Add one `compiler/tests/*.vow` test asserting `builtin_to_extern(name)` and `builtin_ret_ty(name)`
  for all 19 names match the expected runtime symbol / `ITY_*` constant, mirroring the Rust-side
  sweep test above — this is the self-hosted parity check the acceptance criteria require, and no
  existing self-hosted unit test currently pins these two functions directly by name for this family
  (only indirectly, through the `tests/run/*.vow` end-to-end fixtures below).

**Docs:** none required (§2 dry-run confirmed zero gaps). `scripts/check_help_coverage.py` (the
staleness detector) should still be run once as a regression check, not because this slice is
expected to change its output.

**Existing end-to-end regression fixtures** (no new ones needed; keep passing unchanged):
`tests/run/fs_read_line_basic.vow`, `tests/run/fs_read_line_pin_to_root.vow`,
`tests/run/fs_read_line_status.vow`, `tests/run/stdin_read_line_pin_to_root.vow`,
`tests/run/stdin_ready.vow`, `examples/stdin_read.vow`, `examples/streaming_file/fs_read_count.vow`.
If, during implementation, a gap surfaces (e.g. no fixture exercises `fs_listdir` or `args` through a
full compile+run), add a minimal one under `tests/run/` rather than leaving it uncovered — but do not
pre-emptively add fixtures the dry-run didn't show a need for.

## 4. TDD slices

Each slice is red (extend a test that fails against the current shape) → green (minimal production
change) → confirm the freshness gate (`python3 scripts/generate_operations.py --check`) passes.
Ordered so the generator extension is proven on its own before the 19-operation migration, keeping
each commit reviewable.

1. **Rebase and generator token extension.**
   Precondition: rebase this branch onto current `origin/main` (§0). Check whether #1271 has merged;
   if so, confirm `"i64"`/`"ptr"` already exist in `RETURN_TOKENS` with the exact spellings in §3 and
   only add `"bool"`. If not, add all three.
   Red: `scripts/test_generate_operations.py::test_i64_ptr_bool_return_tokens_render` — call
   `gen_rust_ir_block`/`gen_cranelift_block`/`gen_vow_lower_block` on synthetic ops with
   `"return": "i64"`/`"ptr"`/`"bool"` and assert the emitted Rust/Cranelift/Vow snippets use
   `Ty::I64`/`types::I64`/`ITY_I64()` etc. (and, for `"bool"`, `Ty::Bool`/`types::I64`/`ITY_BOOL()` —
   pin the Cranelift-collapses-bool-to-I64 fact directly in this test).
   Green: extend `RETURN_TOKENS` as in §3.

2. **Catalogue grows to 22 entries; generator tests updated.**
   Red: `test_real_catalogue_loads_and_matches_print_ops` fails once entries are added (expected —
   it asserts exact equality with a 3-entry fixture).
   Green: rename/generalize the test per §3; add the 19 entries to `docs/spec/operations.json`; run
   `python3 scripts/generate_operations.py` to splice all four target files; run
   `python3 scripts/generate_operations.py --check` and `python3 scripts/test_generate_operations.py`
   to confirm freshness and no doc-fact drift (§2's dry-run already showed this passes; this step
   re-confirms it against the actually-modified checked-in files).

3. **Rust: remove the 19 hand arms from `vow_static_builtin_to_runtime`.**
   Red: none needed — `builtins_lower_to_runtime_symbols_and_return_types` already exists and passes
   against the hand arms; it must **keep passing unchanged** once the arms are deleted and the
   catalogue-generated block supersedes them. Treat this test staying green through the deletion as
   the pass/fail signal.
   Green: delete the 19 arms.

4. **Rust: Cranelift ABI tables consume the catalogue.**
   Red: add the sweep test (§3) against `catalogue_extern_sig` for all 19 symbols in
   `vow-codegen/src/cranelift_backend.rs` — fails before the catalogue is extended (symbols not yet
   recognized by the generated function), passes once slice 2 regenerates the block.
   Green: delete the 19 hand arms from `make_extern_sig` in both `cranelift_backend.rs` and
   `vow-clif-shim/src/lib.rs`; add the identical sweep test to `vow-clif-shim/src/lib.rs`.

5. **Self-hosted: `builtin_to_extern`/`builtin_ret_ty` consume the catalogue.**
   Red: the new `compiler/tests/*.vow` test (§3) asserting all 19 names resolve correctly — fails
   before regeneration.
   Green: regenerate `compiler/lower.vow`'s marker block (already done in slice 2's
   `generate_operations.py` run); delete the 19 hand arms from `builtin_to_extern`/`builtin_ret_ty`.

6. **Bootstrap + full-test parity.**
   Red/green is not applicable here — this is the regression net for cross-compiler drift the unit
   tests can't see. Run `scripts/bootstrap.sh --skip-cargo` (self-hosted rebuild from updated
   `compiler/` sources) then `scripts/full_test.sh`, once before slice 3 (baseline) and once after
   slice 5 (confirm no divergence). Also confirm the existing end-to-end fixtures listed in §3 still
   pass unchanged.

7. **CI freshness gate.**
   Confirm `python3 scripts/generate_operations.py --check` and
   `python3 scripts/test_generate_operations.py` are still wired into `.github/workflows/ci.yml`
   (already true on `origin/main` today) and both pass with the grown catalogue. No new CI wiring
   needed for this slice — only confirm the existing gate catches a deliberately-stale block (e.g.
   temporarily hand-edit inside a marker pair, confirm `--check` fails, then revert) as a sanity check
   before opening the PR.

## 5. Verification surface

No contracts, codegen semantics, or C model behavior change. This is pure fact-relocation: the same
19 runtime symbols, the same Cranelift ABI signatures, the same IR return types as today, sourced from
a generated table instead of five hand-written ones.

- **Confirmed (not assumed):** `vow-verify/src/c_emitter.rs::is_modelable` short-circuits
  (`if !func.effects.is_empty() { return false; }`) before ever consulting `is_known_builtin`. All 19
  operations carry `Effect::Read`, `Effect::Write`, or `Effect::IO` in
  `vow-types/src/env.rs::builtin_free_fn_signatures()` — none are pure. Grepping both
  `vow-verify/src/c_emitter.rs` and `compiler/c_emitter.vow` in full for every one of the 19 runtime
  symbols returns zero hits in either file. **No verifier-model category is recorded for this slice**
  — the acceptance criterion "if any migrated operation turns out to be pure, record its category" is
  satisfied by direct evidence that none are.
- No new `tests/run/`/`examples/` fixtures are required (§3); the existing fixtures already exercise
  this family end-to-end through the real Cranelift/link pipeline, which is the actual regression net
  for an ABI-signature transcription error (a wrong param count or return type surfaces as a crash or
  link error at runtime, not a type error at compile time).
- Binary fixed point: `compiler/lower.vow`'s generated block is produced by
  `scripts/generate_operations.py` from a list sorted by whatever order `docs/spec/operations.json`
  lists operations in (append-only for this slice — the 3 `print_*` entries keep their existing
  position). Run the bootstrap triple test
  (`scripts/concat_vow.sh` + stage 0/1/2 SHA comparison) after regenerating, since `compiler/lower.vow`
  changes are exactly the kind of self-hosted edit that can silently break the fixed point if the
  generated block is hand-touched instead of produced by the generator.

## 6. Risk areas

- **Silent ABI corruption on a catalogue typo in `vow-clif-shim`.** Its unknown-extern-symbol fallback
  (`"clif_shim: unknown extern sig for '{sym}', using no-arg no-return"`) only logs to stderr and does
  not fail the build — a misspelled `runtime_symbol` in `docs/spec/operations.json` would silently
  produce a wrong-arity extern signature there. The sweep test in §3/§4 slice 4 is the concrete
  mitigation; without it, only an end-to-end crash at runtime would catch this.
- **`stdin_ready`'s bool-as-i64 Cranelift return.** The one place in this family where the Vow surface
  type (`bool`) and the Cranelift ABI type (`I64`) diverge — a naive implementation might introduce a
  dedicated Cranelift bool/I8 return type instead of reusing `types::I64`. Pin this with the dedicated
  test in slice 1.
- **Concurrent `RETURN_TOKENS` edit from #1271.** Both issues extend the same closed vocabulary in
  `scripts/generate_operations.py`; whichever PR merges second will hit a small, mechanical conflict
  in `RETURN_TOKENS`/`docs/spec/operations.json`'s tail. §0/§4 slice 1 describe the exact resolution
  (their `"i64"`/`"ptr"` spellings are already known and match what this plan independently derived,
  so resolution is "keep both sides' new dict keys and both sides' new JSON array entries," not a
  content disagreement).
- **`cargo clippy --all -- -D warnings`.** Shrinking `make_extern_sig`'s and
  `vow_static_builtin_to_runtime`'s hand matches by 19 arms each is unlikely to trip new lints (the
  match still has many remaining arms in both cases), but run clippy locally before opening the PR
  regardless. Test-only lints are out of scope per project convention (CI runs without
  `--all-targets`).
- **`parse → print → parse` idempotency:** unaffected — no syntax, AST, or printer changes.
- **rustfmt:** after `generate_operations.py` writes the Rust projection files, run `cargo fmt --all`
  before `cargo build` to avoid mixing spurious formatting churn into the review diff.

## 7. Out of scope

- `gzip_write_file`, `process_get_stderr`, `process_stderr_for`, `debug_str`/`debug_i64`/`debug_u64` —
  see §2's exclusion rationale.
- `BuiltinResultTag`/`tag_builtin_result` and its self-hosted mirror (`pin_to_root` heap-tag
  classification) — see §2.
- `arena_routing`/`verifier_model` catalogue fields — not applicable to any operation in this family
  (see §2); adding the schema vocabulary itself is #1271's concern, not duplicated here.
- Any `docs/spec/grammar.md` content change — the dry-run in §2 confirmed all 19 operations already
  have correct, matching doc rows; this slice only adds a machine-checkable cross-reference to facts
  that already exist.
- String/parse/hex/memory/time/CPU query builtins (#1271), process builtins (#1273), verifier
  classifier changes (#1274), and general catalogue hardening/validation work (#1275) — separate PRD
  sub-issues, not bundled here.
- Rewriting or reformatting any part of `compiler/lower.vow`, `vow-codegen/src/cranelift_backend.rs`,
  `vow-ir/src/lower/mod.rs`, or `vow-clif-shim/src/lib.rs` beyond the specific 19-operation arms this
  slice removes.

## 8. Judgment calls requiring a posted issue comment

Per the operating contract, a `gh issue comment` will be posted on #1272 before this run exits,
summarizing: (a) the branch-staleness/rebase-first requirement in §0; (b) the four exclusions in §2
(`gzip_write_file`, `process_get_stderr`/`process_stderr_for`, `debug_*`, `BuiltinResultTag`) with
their rationale; (c) the decision to omit `arena_routing`/`verifier_model` fields entirely rather than
stamping an explicit `"none"`, matching the #1271 sibling's own precedent for effectful operations;
and (d) the concurrent-edit note with #1271's in-progress `RETURN_TOKENS` extension.
