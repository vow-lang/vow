# PLAN: Migrate process Builtin Operations to the Operation Catalogue (#1273)

**Revision note (2026-09-30):** this plan supersedes the version committed 2026-09-13. That
version was written against a local preview of #1270's branch and assumed the blocker was still
unmerged. #1270 merged as PR #1279 (`refactor(codegen): source print_* facts from an Operation
Catalogue`, merged 2026-09-13T20:56:59Z). This revision was re-verified directly against
`origin/main` (HEAD `f59d2f88`, 55 commits ahead of this branch's base `0cf9d948`) via a
detached worktree, not against the stale preview. Every file/function/field name below is a
direct read of the merged code, not a guess. A prior implementation attempt on this branch
correctly detected the old blocker and exited without touching code — no production code exists
on this branch yet, so this revision is a pure re-plan.

## §0. Preflight — rebase required, blocker cleared

`origin/main` now has the real Operation Catalogue infrastructure. This branch is still based on
`0cf9d948`, pre-dating it entirely. **Implementation-stage step 1 (hard precondition):**

1. `git fetch origin main` and `git rebase origin/main` (or merge) this branch. Expect no
   conflicts — nothing has been added to this branch yet besides `PLAN.md`.
2. Re-run the verification greps in §2/§3 below against the post-rebase tree before writing any
   code — `main` moves fast (55 commits since this branch's base, including #1351 which
   restructured the heap-tag classifier this plan touches in §3). If any cited function name has
   moved again, treat this plan's *facts* (schema shape, scope decision in §7) as authoritative
   and re-locate the function by grepping for it, rather than assuming the plan is wrong.
3. Confirm no sibling issue landed first: `gh pr list --search 1271 --state all` and
   `gh pr list --search 1272 --state all`. As of this writing, neither has a merged PR — both
   exist only as unmerged local branches (`sym/vow/1271-...`, `sym/vow/1272-...`) that also
   haven't extended `docs/spec/operations.json` yet. If either merges before this issue is
   implemented, re-read `docs/spec/operations.json` and `scripts/generate_operations.py` fresh,
   since a merged #1271 in particular would add `arena_routing`/`verifier_model` fields this plan
   currently has to introduce itself (see §7).

## §1. Problem restated

Eleven `process_*` Builtin Operations (`process_exit`, `process_run`, `process_get_stdout`,
`process_get_stderr`, `process_start`, `process_wait`, `process_wait_timeout`,
`process_poll_wait`, `process_kill`, `process_stdout_for`, `process_stderr_for`) have their
runtime-symbol, Cranelift ABI signature, and IR return-type facts hand-duplicated across four
call sites: `vow-ir/src/lower/mod.rs::vow_static_builtin_to_runtime`,
`vow-codegen/src/cranelift_backend.rs::make_extern_sig`, `vow-clif-shim/src/lib.rs::make_extern_sig`,
and `compiler/lower.vow`'s `builtin_to_extern`/`builtin_ret_ty`. Four of them
(`process_get_stdout`, `process_get_stderr`, `process_stdout_for`, `process_stderr_for`) also have
a *second* hand-duplicated fact — their heap-allocated-`String`-result tag, consumed by
`pin_to_root`/arena tracking — in `vow-ir/src/lower/mod.rs::builtin_result_tag` and its
self-hosted mirror in `compiler/lower.vow`. All eleven are `[Effect::IO]`-tagged, so this is a
pure ABI/runtime-symbol/return-shape/arena-routing/doc migration; it does not touch the
verifier's `is_known_builtin`/`is_modelable` gate (confirmed empty of any `process_*` symbol on
both `vow-verify/src/c_emitter.rs` and its self-hosted mirror `compiler/c_emitter.vow`).

The Operation Catalogue (`docs/spec/operations.json` + `scripts/generate_operations.py`) already
does this for three `print_*` operations (PR #1279), but **only for `Unit`-returning, non-heap
operations** — its `RETURN_TOKENS` vocabulary currently defines only `"unit"`, and its schema has
no arena/heap-routing field at all. Six of the eleven `process_*` operations return `I64`, four
return a heap `Ptr`, and issue #1273's acceptance criterion #1 explicitly requires the catalogue
to record "arena-routing categories where applicable." **This slice therefore has to extend the
generator's schema/codegen (not just append data)** before it can add a single `process_*` entry
— this is the main way this plan differs from the print-only precedent, and from the version of
this plan written before #1270 merged.

## §2. Verified current state of the Operation Catalogue (read from `origin/main` directly)

**Catalogue data** — `docs/spec/operations.json`, 3 entries today, fields
`name`/`runtime_symbol`/`params`/`return`/`doc_signature`/`effects`:
```json
{"name": "print_str", "runtime_symbol": "__vow_string_print", "params": ["ptr"], "return": "unit", "doc_signature": "fn(s: String) -> ()", "effects": "[io]"}
```

**Generator** — `scripts/generate_operations.py` (NOT `generate_ops.py` — renamed at some point
before merge). Key symbols:
- `RETURN_TOKENS = {"unit": {"rust_ty": "Ty::Unit", "ity_const": "ITY_UNIT()", "clif_ret": None}}`
  — **only one entry exists**. Adding `process_*` requires adding `"i64"` and `"ptr"` entries.
- `PARAM_TOKENS = {"ptr": "types::I64", "i64": "types::I64", "u64": "types::I64"}` — already
  covers every ABI param shape `process_*` needs (all params are `I64`-width at the Cranelift
  level; no change needed here).
- `load_catalogue`, `check_doc_facts` (cross-checks `doc_signature`/`effects` against
  `docs/spec/grammar.md`, `vow/src/skill.rs`, `compiler/main.vow`), `gen_rust_ir_block`,
  `gen_cranelift_block`, `gen_vow_lower_block`, `write_projections`/`check_projections` (the
  `--check` flag), `extract_builtin_signatures_table`.
- **`extract_builtin_signatures_table` already sweeps the whole level-3 `### Builtin Function
  Signatures` section across every `####` subsection** (confirmed by reading its docstring and
  loop logic directly) — it is **not** scoped to `#### Print / IO` the way an earlier plan draft
  assumed. **No change is needed to broaden this function or its heading target** — that entire
  slice from the pre-merge plan draft is now moot.
- A test already exists, `GenCraneliftBlockTest.test_non_unit_return_token_pushes_return_slot` in
  `scripts/test_generate_operations.py`, that proves the `clif_ret` plumbing works for a
  temporary non-`unit` token (`"fake_i64"`) by monkeypatching `RETURN_TOKENS` mid-test. This
  de-risks adding real `"i64"`/`"ptr"` tokens — the plumbing is already exercised, just not with a
  permanent token.

**Generated splice targets** (all four wrapped in `// GENERATE:OPERATIONS:START/END`, not
`OP_CATALOGUE`):
- `vow-ir/src/lower/mod.rs` → `fn catalogue_builtin_to_runtime(name: &str) -> Option<(&'static str, Ty)>`, consulted first by `vow_static_builtin_to_runtime` before its hand-written match.
- `vow-codegen/src/cranelift_backend.rs` → `fn catalogue_extern_sig(sym: &str, sig: &mut Signature) -> bool`, consulted first by `make_extern_sig`.
- `vow-clif-shim/src/lib.rs` → same `catalogue_extern_sig`, consulted first by its own `make_extern_sig` (kept byte-identical to the codegen crate's copy per existing convention).
- `compiler/lower.vow` → `fn catalogue_builtin_to_extern(name: String) -> String` and `fn catalogue_builtin_ret_ty(name: String) -> i64`, consulted first by `builtin_to_extern`/`builtin_ret_ty`.

**Heap-tag classifier** (separate from the four splice targets above — PR #1351 restructured
this into a pure classifier, but it is **not** part of the Operation Catalogue's generated code
today):
- Rust: `enum BuiltinResultTag { StringHeap, VecHeap, OptionOf(Ty) }` +
  `fn builtin_result_tag(name: &str) -> Option<BuiltinResultTag>` in `vow-ir/src/lower/mod.rs`,
  still hand-listing `"process_get_stdout" | "process_get_stderr" | "process_stdout_for" |
  "process_stderr_for" | "proc_sample"` (among others) in its `StringHeap` arm. Dispatched by
  `fn tag_builtin_result(ctx: &mut LowerCtx, name: &str, result: InstId)`.
- Self-hosted mirror in `compiler/lower.vow`: `fn builtin_result_tag(name: String) -> i64`
  (sentinels `BRT_NONE()`/`BRT_STRING()`/`BRT_VEC()` from `compiler/ir.vow`) + the same
  `tag_builtin_result` dispatcher, with the identical five names hand-listed as `BRT_STRING()`.
- Pinned by tests today: Rust `builtin_result_tag_classifies_names` (in
  `vow-ir/src/lower/mod.rs`'s test module) and self-hosted
  `compiler/tests/test_lower_builtin_result_tag.vow` (`expect_tag(..., BRT_STRING(), N)` calls for
  all five names). **Both must keep passing unchanged after migration** — they test behavior, not
  implementation path.

**Confirmed still hand-written, not yet catalogued** (all four call sites, unchanged names):
`vow_static_builtin_to_runtime`, `make_extern_sig` (×2), `builtin_to_extern`/`builtin_ret_ty` all
still have explicit `process_*` arms outside their respective marker blocks.

**Verifier exclusion confirmed clean**: no `__vow_process_*` symbol appears in
`vow-verify/src/c_emitter.rs::is_known_builtin` or its self-hosted mirror
`compiler/c_emitter.vow::is_known_builtin`. No change needed (closes AC #6 — see §5).

**Doc gap confirmed real**: `docs/spec/grammar.md`'s `#### Process Management` table (under the
level-3 `### Builtin Function Signatures` heading, line ~1063/1224) lists 10 of the 11 operations
— `process_poll_wait` is missing. `check_doc_facts` will reject any catalogue entry whose name
has no `grammar.md` row, so this row must be added before `process_poll_wait` can be catalogued.

**No JSON schema file exists** (`docs/spec/schemas/` has no `operation-catalogue.schema.json` —
validation is done entirely by `load_catalogue`'s Python-side checks). No schema doc to update.

**No `compiler/tests/test_op_catalogue.vow` exists.** There is no dedicated self-hosted test file
for catalogue consumption today; `print_*` catalogue behavior is verified indirectly through
whatever existing lowering/extern-resolution tests exercise `builtin_to_extern`/`builtin_ret_ty`
end-to-end. This plan adds `process_*` coverage to that same indirect style rather than inventing
a new file, unless a `test_op_catalogue.vow`-equivalent surfaces during the post-rebase re-check.

## §3. Operation inventory (verified return types from `vow-ir/src/lower/mod.rs` directly)

| name | runtime_symbol | params | return | rust `Ty` | arena_routing |
|---|---|---|---|---|---|
| `process_exit` | `__vow_process_exit` | `[i64]` | `unit` | `Ty::Unit` | `none` |
| `process_run` | `__vow_process_run` | `[ptr, ptr]` | `i64` | `Ty::I64` | `none` |
| `process_get_stdout` | `__vow_process_get_stdout` | `[]` | `ptr` | `Ty::Ptr` | `heap_fresh` |
| `process_get_stderr` | `__vow_process_get_stderr` | `[]` | `ptr` | `Ty::Ptr` | `heap_fresh` |
| `process_start` | `__vow_process_start` | `[ptr, ptr]` | `i64` | `Ty::I64` | `none` |
| `process_wait` | `__vow_process_wait` | `[i64]` | `i64` | `Ty::I64` | `none` |
| `process_wait_timeout` | `__vow_process_wait_timeout` | `[i64, i64]` | `i64` | `Ty::I64` | `none` |
| `process_poll_wait` | `__vow_process_poll_wait` | `[i64, i64]` | `i64` | `Ty::I64` | `none` |
| `process_kill` | `__vow_process_kill` | `[i64]` | `i64` | `Ty::I64` | `none` |
| `process_stdout_for` | `__vow_process_stdout_for` | `[i64]` | `ptr` | `Ty::Ptr` | `heap_fresh` |
| `process_stderr_for` | `__vow_process_stderr_for` | `[i64]` | `ptr` | `Ty::Ptr` | `heap_fresh` |

`params`/`return` tokens confirmed against the current hand-written `make_extern_sig` arms in
`vow-codegen/src/cranelift_backend.rs` (e.g. `process_run`'s two `*VowVec` params, `process_wait`'s
single `i64` param) and against `vow_static_builtin_to_runtime`'s current `Ty` values (line-cited
in the pre-rebase read; re-confirm post-rebase since #1351 landed between then and now, though it
touched `builtin_result_tag`, not `vow_static_builtin_to_runtime`).

**Pinned fact, do not "fix" during this migration:** `process_exit`'s `vow-types` signature
returns `Ty::Never` conceptually, but `vow-ir::Ty` has no `Never` variant, and the lowering site
has always recorded `Ty::Unit` for it. The catalogue entry uses `return: "unit"` to preserve this
pre-existing behavior; this is not a bug this slice fixes.

`doc_signature`/`effects` per operation: read verbatim from `docs/spec/grammar.md`'s `Process
Management` table for the 10 existing rows (do not retype by hand — copy the exact strings so
`check_doc_facts` matches on the first try); author a new row for `process_poll_wait` matching its
current Rust/self-hosted signature `(pid: i64, timeout_ms: i64) -> i64 [IO]` (confirm against
`vow-runtime/src/lib.rs::__vow_process_poll_wait`'s actual parameter meaning before writing the
row — do not guess parameter names from the ABI param count alone).

## §4. Files to touch

**Rust, generator infrastructure:**
- `scripts/generate_operations.py` — add `"i64"` and `"ptr"` entries to `RETURN_TOKENS`
  (`rust_ty: "Ty::I64"`/`"Ty::Ptr"`, `ity_const: "ITY_I64()"`/`"ITY_PTR()"`, `clif_ret:
  "types::I64"` for both — Cranelift has no distinct pointer type, everything is `I64`-width).
  Add an `arena_routing` field: extend `REQUIRED_FIELDS` (or make it optional with a validated
  enum `{"none", "heap_fresh"}` if any existing/parallel-branch precedent uses optional — re-check
  #1271's branch shape post-rebase per §0 step 3 before deciding required-vs-optional, to avoid
  inventing a third incompatible schema shape). Add a fourth generated function,
  `catalogue_builtin_result_tag`, to `gen_rust_ir_block` and `gen_vow_lower_block` (mapping
  `arena_routing: "heap_fresh"` → `Some(BuiltinResultTag::StringHeap)` / `BRT_STRING()`, `"none"` →
  `None`/`BRT_NONE()`), spliced into `vow-ir/src/lower/mod.rs` and `compiler/lower.vow` alongside
  the existing three splice functions.
- `scripts/test_generate_operations.py` — update `test_real_catalogue_loads_and_matches_print_ops`
  (currently an exact-equality assert `ops == PRINT_OPS`; will need to become `ops[:3] ==
  PRINT_OPS` or equivalent once real entries are appended — do not weaken it to a subset-only
  check without reason). Add fixture coverage for a non-`unit` return (`process_run`, `return:
  "i64"`), a `ptr` return (`process_get_stdout`), and an `arena_routing: "heap_fresh"` entry
  exercising the new `catalogue_builtin_result_tag` generation, mirroring the existing
  `GenRustIrBlockTest`/`GenVowLowerBlockTest` structure.
- `docs/spec/operations.json` — append the 11 entries from §3.
- `docs/spec/grammar.md` — add the missing `process_poll_wait` row to `#### Process Management`.
  Run `uv run python scripts/generate_help.py` afterward and confirm the diff is exactly that one
  new line propagating into `vow/src/skill.rs`, `skills/vow/reference/grammar.md`, and
  `compiler/main.vow`'s help JSON.

**Rust, consumption:**
- `vow-ir/src/lower/mod.rs` — remove the 11 `process_*` arms from `vow_static_builtin_to_runtime`;
  remove `"process_get_stdout" | "process_get_stderr" | "process_stdout_for" |
  "process_stderr_for"` from `builtin_result_tag`'s `StringHeap` arm (leave `proc_sample` — it is
  not `process_*`-prefixed and is explicitly owned by #1271/#1277, see §7). Update
  `builtin_result_tag_classifies_names` and `builtins_lower_to_runtime_symbols_and_return_types`
  (the lockstep table) — both should keep passing with the exact same assertions, now satisfied via
  the catalogue path; add one new test,
  `process_builtins_resolve_via_operation_catalogue`, asserting
  `catalogue_builtin_to_runtime("process_run")` etc. directly.
- `vow-codegen/src/cranelift_backend.rs` — remove the 11 `"__vow_process_*"` arms from
  `make_extern_sig`; add `process_extern_sigs_come_from_the_operation_catalogue` asserting
  `catalogue_extern_sig` produces the right params/returns for a couple of representative symbols.
- `vow-clif-shim/src/lib.rs` — identical removal and test, kept in sync with the codegen crate.

**Self-hosted:**
- `compiler/lower.vow` — after regenerating the `GENERATE:OPERATIONS` block (now 14 entries per
  helper, including the new `catalogue_builtin_result_tag`), manually remove the 11 redundant
  `process_*` lines from `builtin_to_extern`/`builtin_ret_ty`'s hand-written chains, and remove
  the four `process_*` names from the self-hosted `builtin_result_tag`'s `BRT_STRING()` arm
  (leave `proc_sample`).
- `compiler/tests/test_lower_builtin_result_tag.vow` — no assertion changes needed (behavior
  preserved), but re-run explicitly after the removal to confirm the fallback-free catalogue path
  produces identical `expect_tag` results.
- Add `process_*` coverage wherever `print_*` catalogue consumption is currently exercised
  end-to-end for `builtin_to_extern`/`builtin_ret_ty` (locate this during implementation per §2's
  note that no dedicated `test_op_catalogue.vow` exists yet — either extend that file if one is
  found post-rebase, or add a small new `compiler/tests/test_op_catalogue.vow` if none exists,
  matching whatever pattern `print_*` uses).

**Tests:**
- `tests/run/process_catalogue_smoke.vow` — new e2e fixture (§5 slice 12).

**No changes needed to:** `vow-runtime/src/lib.rs` (runtime symbol implementations, linkage, and
existing unit tests like `process_poll_wait_captures_stdout` are unchanged — only the tables that
*name* these symbols move), `vow-types/src/env.rs` (signatures/effects are read, not modified),
`vow-verify/src/c_emitter.rs` / `compiler/c_emitter.vow` (confirmed no `process_*` reference
exists in either — §2), `compiler/main.vow`'s hand-written help-JSON (fully regenerated by
`scripts/generate_help.py` from `grammar.md`), `extract_builtin_signatures_table`/
`check_doc_facts` (already heading-agnostic — §2), any JSON schema file (none exists).

## §5. TDD slices

1. **Preflight** (§0) — rebase onto `origin/main`, re-verify facts. Gates every slice below.
2. **Doc gap: add the missing `process_poll_wait` grammar row.** Verify the true parameter
   semantics against `vow-runtime/src/lib.rs::__vow_process_poll_wait` first. Add the row to
   `docs/spec/grammar.md`; run `scripts/generate_help.py`; confirm only the expected propagation
   diff.
3. **Extend `RETURN_TOKENS` with `"i64"` and `"ptr"`.** Red: add a permanent-token variant of the
   existing `test_non_unit_return_token_pushes_return_slot` pattern in
   `scripts/test_generate_operations.py` asserting `gen_cranelift_block` emits a return-slot push
   for an op using `return: "i64"` — fails against current `RETURN_TOKENS` (`KeyError`). Green: add
   the two entries. Also assert `gen_rust_ir_block` emits `Ty::I64`/`Ty::Ptr` and
   `gen_vow_lower_block` emits `ITY_I64()`/`ITY_PTR()` for the respective tokens.
4. **Add the `arena_routing` field and `catalogue_builtin_result_tag` generation.** Red: a test
   asserting a catalogue op with `arena_routing: "heap_fresh"` produces a `Some(StringHeap)`/
   `BRT_STRING()` arm in the generated block, and one with `"none"` produces nothing (or an
   explicit `None`/`BRT_NONE()`, matching whatever style `catalogue_builtin_to_runtime` already
   uses for its "not found" case) — fails since the field/function don't exist yet. Green:
   implement the field validation, the new `gen_*_block` additions, and the `--check` wiring.
5. **Catalogue data: append the 11 `process_*` entries.** Red: `uv run python
   scripts/generate_operations.py --check` fails (data present, generated files stale). Update
   `test_real_catalogue_loads_and_matches_print_ops` and add the `process_run`/`process_get_stdout`
   fixture coverage from §4. Green: run the generator to regenerate all four splice targets;
   `--check` passes.
6. **Rust runtime-symbol/return-shape/arena-tag consumption.** Remove the 11
   `vow_static_builtin_to_runtime` arms and the 4 `builtin_result_tag` `StringHeap` arms; add
   `process_builtins_resolve_via_operation_catalogue`; rerun
   `builtins_lower_to_runtime_symbols_and_return_types` and `builtin_result_tag_classifies_names`
   and confirm both still pass unchanged, proving the catalogue path is a transparent substitution.
7. **Rust ABI consumption.** Remove the 11 hand-written arms from `make_extern_sig` in both
   `vow-codegen/src/cranelift_backend.rs` and `vow-clif-shim/src/lib.rs`; add
   `process_extern_sigs_come_from_the_operation_catalogue` to each.
8. **Self-hosted consumption.** After slice 5's regeneration, `compiler/lower.vow`'s
   `GENERATE:OPERATIONS` block already covers `process_*`. Remove the 11 redundant lines from
   `builtin_to_extern`/`builtin_ret_ty` and the 4 `process_*` names from the self-hosted
   `builtin_result_tag`; rerun `compiler/tests/test_lower_builtin_result_tag.vow` and whatever
   end-to-end `builtin_to_extern`/`builtin_ret_ty` coverage exists (§4) — both must still pass.
9. **Verifier purity confirmation (closes AC #6).** Add a small test asserting
   `!is_known_builtin("__vow_process_exit")` (and 1-2 more) in both `vow-verify/src/c_emitter.rs`
   and `compiler/c_emitter.vow`, freezing the already-true invariant with evidence.
10. **End-to-end smoke test.** `tests/run/process_catalogue_smoke.vow`, using the portable
    `process_start`/`process_run` pattern already established elsewhere in the codebase (grep
    `compiler/main.vow` or existing `tests/run/*.vow` fixtures for the current idiom post-rebase,
    since exact line numbers will have moved). Cover both call families:
    `process_run("sh", ["-c", "echo hello"])` + `process_get_stdout()`/`process_get_stderr()`
    (synchronous path), and `process_start("sh", ["-c", "echo world"])` + `process_wait(pid)` +
    `process_stdout_for(pid)` (async path). Assert via `TEST: stdout` only — `scripts/full_test.sh`
    ignores `TEST: stderr` (redirected to `/dev/null`), so don't rely on it for CI coverage.
    `process_kill`/`process_wait_timeout`/`process_poll_wait` already have dedicated Rust unit
    coverage in `vow-runtime/src/lib.rs` and don't need duplicate `.vow` fixtures; `process_exit`
    already has `tests/run/issue850_never_call_statement.vow`.
11. **Full rebuild and quality gates**, run as separate commands (not `&&`-chained): `cargo fmt
    --all`; `cargo build --release -p vow` (cap `-j` to the attempt's memory share);
    `scripts/bootstrap.sh --skip-cargo`; `cargo test --all` (or targeted `-p vow-ir -p vow-codegen
    -p vow-clif-shim -p vow-verify -p vow` first); `cargo clippy --all -- -D warnings`; `uv run
    python scripts/generate_operations.py --check`; `uv run python scripts/generate_help.py
    --check`; `uvx ruff@<CI-pinned version>` over touched Python files; `scripts/full_test.sh` and,
    locally, `tests/run_tests.sh`, covering the new smoke fixture and updated self-hosted tests.

## §6. Verification surface

None of the 11 `process_*` operations carry Vow contracts (`requires`/`ensures`) — they are
runtime-implemented builtins, not user-authored Vow functions. This slice introduces no new
`requires`/`ensures` clauses and no new ESBMC proof obligations. All 11 are `[Effect::IO]`, so
`vow-verify/src/c_emitter.rs::is_modelable`'s effect-emptiness gate already excludes every one of
them from the C model, before and after this migration (slice 9 adds a regression test pinning
this fact). No `tests/verify/` or `tests/verify-fail/` fixtures need to grow, and no
`verifier_model` catalogue field is populated for these operations, consistent with the issue
body's statement that this slice is "not expected to exercise the verifier's
`is_known_builtin`/`is_modelable` classifier."

## §7. Scope decision: `arena_routing` is added, `verifier_model` is not

Issue #1273's AC #1 explicitly asks for "arena-routing categories where applicable." Since no
`arena_routing`-equivalent field exists in `docs/spec/operations.json`'s schema on `main` today,
this slice must introduce it (§4 slice 4) rather than defer it — unlike the pre-merge draft of
this plan, which incorrectly reasoned the AC didn't require it. The four `Ptr`-returning
operations get `arena_routing: "heap_fresh"`; the other seven get `"none"`.

`verifier_model` (a hypothetical field for AC #6's purity classification) is **not** added,
because AC #6 itself is conditional: "if [a process_* operation is pure], record its
verifier-model category, otherwise no verifier-classifier changes are needed." §2/§6 confirm all
11 are IO-effectful, so the "otherwise" branch applies — no field, no category, just the
regression test in slice 9.

**Naming risk:** an unmerged sibling branch for #1271 independently prototypes both
`arena_routing: {"none", "heap_fresh"}` and `verifier_model: {"known", "unmodeled"}` fields with
those exact names/values (discovered by inspecting the local branch during planning — it has not
merged and could still change shape). This plan reuses the `arena_routing` name/values on the
theory that landing first with an incompatible field name would force whichever of #1271/#1273
lands second to do a painful rename; if #1271 merges its version first, re-read its actual landed
schema before slice 4 and conform to it exactly rather than to this plan's guess.

**Explicitly out of scope for the same reason as before:** `proc_sample` (not `process_*`-
prefixed; its Rust/self-hosted `tag_builtin_result` parity bug is `#1277`, owned by #1271's plan)
and `__vow_string_parse_u64_opt`'s self-hosted verifier gap (`#1276`, unrelated).

## §8. Risk areas

- **Cross-issue collision on shared files.** #1271 and #1272 are both blocked only by #1270
  (now cleared) and touch the same generator/splice files. Re-check both before starting (§0 step
  3). `docs/spec/operations.json` additions are a simple array append (low conflict surface); the
  generator schema extension (`RETURN_TOKENS`, `arena_routing`) is the riskier shared surface — if
  #1271 lands its own version of either first, adapt to the landed shape rather than re-deriving.
- **`RETURN_TOKENS`/`arena_routing` extension touches the generator itself, not just data** —
  this is new surface area beyond the print-only precedent. Keep the two additions (return tokens,
  arena field) as separate commits/slices from the data-only append (slice 5), so a review can
  isolate a generator bug from a data-entry typo.
- **Self-hosted determinism.** The generator sorts/renders deterministically (confirmed by
  `GeneratorDeterminismTest` in `scripts/test_generate_operations.py`), so regenerating all four
  splice targets does not put the bootstrap triple test's binary fixed point at risk. No
  `BTreeMap`/`HashMap` choice or `vow-clif-shim` stack-slot layout is touched by this slice.
- **`parse → print → parse` idempotency** is unaffected — no parser, AST, or printer changes.
- **`cargo clippy --all -- -D warnings`** — removing 11+4 match arms across four hand-written
  matches risks leaving unreachable `_ =>` arms or unused imports; run clippy per-crate after each
  removal slice, not just once at the end.
- **PR #1351 already restructured `builtin_result_tag` once** (splitting it out of an inline
  `tag_builtin_result` body). Re-confirm both the Rust and self-hosted function shapes post-rebase
  before editing — a further refactor could have landed between this plan's research and
  implementation.

## §9. Out of scope

- Migrating any non-`process_*` Builtin Operations (query/utility: #1271; filesystem/stdin/args/
  stderr: #1272).
- `verifier_model` / any verifier-classifier schema field or `is_known_builtin`/`is_modelable`
  change — confirmed unnecessary (§6/§7).
- `proc_sample`'s Rust/self-hosted `tag_builtin_result` parity bug (`#1276`... `#1277`, see §7) —
  pre-existing, unrelated, owned by other issues.
- Any change to `vow-runtime/src/lib.rs`'s `process_*` implementations, linkage, or the process
  handle-table mechanism — only the lowering/codegen tables that *name* these symbols move.
- Rewriting `compiler/main.vow`'s help-JSON by hand — fully derived by `scripts/generate_help.py`.
- Broader hardening of `scripts/generate_operations.py` beyond the two additions this slice
  actually needs (`RETURN_TOKENS` i64/ptr, `arena_routing` field) — general schema robustness is
  left to whichever issue in the PRD family is chartered for it (e.g. #1275).
- Any formatting-only or unrelated cleanup in files this slice must touch anyway.
