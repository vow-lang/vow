# Architecture deepening backlog

Persisted candidate memory for the `pm-deepen` routine. Statuses: proposed | in-flight |
landed | dropped | rejected. Never delete rows — `landed`/`dropped`/`rejected` are the memory
that stops the next firing re-deriving them. See `.architecture/reviews/` for the scored reports.

## narrow-int-width-self-hosted

- **Status**: in-flight
- **Score**: 22/25 (leverage 4, locality 4, blast radius 1, heat 5)
- **Files**: ~2 estimated (`compiler/lower.vow` + new `compiler/tests/test_lower_narrow_int_width.vow`)
- **Modules**: `compiler/lower.vow` — `lower_narrow_literal` self-gate `:2112-2116` (9-set), binop
  operand `:2375-2377`, call argument `:2623-2627`, assign to ident `:3442-3444`, match-result Phi
  `:4473-4475` (8-set each), `let` annotation `:4909-4932` (8 `if`s over type names), fn trailing
  return `:5533-5535` (9-set). Rust twin already deep: `vow-ir/src/lower/mod.rs:4139-4155`.
- **Summary**: mirror the landed Rust `narrow_int_width` / `diverges_from_speculative_int` seam
  (#1327) into the self-hosted lowerer, derived from an integer-width accessor over `ITY_*` codes
  rather than re-spelled membership lists; behaviour-preserving row for row.
- **First seen**: 2026-09-22
- **Report**: `.architecture/reviews/2026-09-22-narrow-int-width-self-hosted.md`
- **PR**: #1334
- **Reason**: **picked this firing** (2026-09-22), the first under the #1330 parity rule. Three-way
  tie at 22 with `builtin-result-tag-self-hosted` and `builtin-method-spec-self-hosted` — all in
  `lower.vow`, so the rubric's three tie-break keys tie; broken by two recorded extensions (smaller
  file-count estimate — tie at 2; more inline sites collapsed — 7 vs 1). Parity reading: a
  self-hosted-only mirror of an already-landed Rust seam completes, not halves, a change; recorded
  explicitly in the report. Branch adopted (`sym/vow/routine/refactor-audit/01M33370Q9`), not
  renamed.
- **Adjudicated design B** (the `ity_int_width_bits` domain fact in `ir.vow`, with the two predicates
  in `lower.vow`) over **A** (minimal mirror, the runner-up design, which re-spelled the width set
  inside its own table) and **C** (context-keyed entry point, which would put a second vocabulary for
  the policy in one compiler only). **Opened as #1334.** Verification: differential self-hosted IR
  over 675 inputs including the concatenated compiler — 675/675 byte-identical; bootstrap fixed point
  `c5bd7496…` with verification on; `cargo test --all` 1699/0; `scripts/full_test.sh` 1088 passed /
  0 failed / 19 skipped. Diff: 4 files, +151/−43, inside the step-4 estimate of 3 files plus the
  optional Rust comment.

## builtin-result-tag-self-hosted

- **Status**: proposed
- **Score**: 22/25 (leverage 4, locality 4, blast radius 1, heat 5)
- **Files**: ~2 estimated
- **Modules**: `compiler/lower.vow:2541-2603` (inline in the `ext != ""` builtin-call branch of
  `lower_expr`); Rust twin `vow-ir/src/lower/mod.rs:207-241` (#1290).
- **Summary**: mirror `builtin_result_tag` into the self-hosted lowerer as a pure classification
  (`kind` + option element `ITY`), leaving `lctx_tag`/`lctx_tag_option_elem` at the call site.
- **First seen**: 2026-09-22
- **Report**: `.architecture/reviews/2026-09-22-narrow-int-width-self-hosted.md`
- **Reason**: **runner-up candidate** 2026-09-22 (tied at 22, lost tie-break key 5 — 1 site vs 7).
  Row-for-row identical to Rust, no drift; strictly behaviour-preserving. The natural next firing.

## builtin-method-spec-self-hosted

- **Status**: proposed
- **Score**: 22/25 (leverage 4, locality 4, blast radius 1, heat 5)
- **Files**: ~2 estimated (~−200/+110 in `lower.vow`, ~100-line test)
- **Modules**: `compiler/lower.vow:3747-4114` (`EXPR_METHOD`, 20 uniform rows + 6 inline arms);
  Rust twin `builtin_method_spec`, `vow-ir/src/lower/mod.rs:305` (#1299).
- **Summary**: mirror `builtin_method_spec` into the self-hosted lowerer.
- **First seen**: 2026-09-22
- **Report**: `.architecture/reviews/2026-09-22-narrow-int-width-self-hosted.md`
- **Reason**: tied at 22; lost tie-break key 5. **Argument-mode drift vs Rust**: `push_str`/`eq`/
  `byte_at`/`push_byte`/`contains` use `lower_expr` (`:3768/3784/3800/3816/3882`), `truncate`
  `lower_expr` + `ConstI64(0)`, HashMap `get`/`contains_key`/`remove` use `lower_consumed_expr` with
  no missing-arg guard; dead empty `if` at `:3896`. A behaviour-preserving mirror needs 5 arg modes,
  not Rust's 4 — decide before implementing.

## narrow-literal-context-admission

- **Status**: landed
- **Score**: 22/25 (leverage 4, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-ir/src/lower/mod.rs` — admission predicate at `:1698-1701` (binop operand),
  `:1796-1806` (call argument), `:2125-2135` (assign to ident), `:3538-3541` (match-result Phi),
  `:4598-4602` (`lower_narrow_literal` self-gate), `:4723-4739` (`let` annotation, 8-branch `if/else`
  over type-name strings), `:4961-4964` (fn trailing return). 13 call sites of `lower_narrow_literal`
  (`:4597`).
- **Summary**: one context-keyed pure seam answering "must a value of type `T` be re-lowered at its
  native narrow width in this context?", replacing seven inline spellings of the membership; the
  `let` chain composes the existing `scalar_ty_for_field_type_name` (`:691-724`) rather than
  hand-rolling a second `&str -> Ty` map.
- **First seen**: 2026-09-21
- **Report**: `.architecture/reviews/2026-09-21-narrow-literal-context-admission.md`
- **PR**: #1327 (merged 2026-09-21; reconciled 2026-09-22 via `gh pr view 1327` → MERGED). The
  self-hosted half is carded separately as `narrow-int-width-self-hosted` so this `landed` row does
  not hard-filter it.
- **Reason**: **picked this firing** (2026-09-21). `U64` is handled three incompatible ways in one
  file: rejected before the call at `:1700`/`:1805`/`:2134`/`:3540` (8-type lists), admitted then
  conditionally escaped at `:4600` via the `wide_literal_contexts` guard (`:4604-4611`), and admitted
  outright at `:4961-4964` plus the nine unfiltered call sites. The escape shows the *intended* policy
  is "U64 is admitted but yields to an explicit wide context"; the four 8-type pre-filters predate it.
  The change is behaviour-preserving — the table reproduces today's per-context membership row for
  row. **Tie at 22 with `loop-scope-break-policy`**, broken by the rubric's deterministic rule (equal
  blast radius 1, equal heat 5 → most recently touched file: `lower/mod.rs` 2026-09-18 vs `check.rs`
  2026-09-14). Heat 5 is load-bearing: at heat 4 this scores 21 and the runner-up wins outright;
  `lower/mod.rs` is 21 commits/90d, last touched three days ago, the same grade the 2026-09-18 firing
  gave the same file. **Parity gap, not precedent:** landed Rust-only; `compiler/lower.vow` mirrors
  the chain at `:4867-4888` and was left untouched. `CLAUDE.md` requires both compilers to change
  together, with no exemption for behaviour-preserving changes — outstanding debt, like
  `builtin-result-tag` and `builtin-method-spec`.
  **Branch adopted, not created**: the run was started on `sym/vow/routine/refactor-audit/01M30GVTZN`
  (non-default, 0 commits ahead of `origin/main`, no upstream, unpublished), so per the autonomy
  contract it was adopted and **not** renamed to `pm-deepen/<slug>` — the caller's harness identifies
  the run by that branch. The slug is recorded here instead.
  **Corrected during the design pass**: the `let` site is a *fifth* U64-excluding context, not one of
  the admitting ones — `:4741-4759` emits a bare `IntCast` to `U64` rather than re-lowering, skipping
  both the `wide_literal_contexts` escape and `lower_integer_marker_as`. So the split is 5 exclude /
  2 admit. Two further rows were found and deliberately left out of scope: `{I128, U128}` at six
  sites, and `{U64, I128, U128}` at `:1497`/`:4334`/`:4604`.
  **Adjudicated design C** (domain fact: `narrow_int_width` + `diverges_from_speculative_int`,
  derived from `IntegerType { width, signedness }`) over **B** (context parameter on
  `lower_narrow_literal`, the runner-up design, which supplied the binding 13-site audit) and **A**
  (minimal surface, 2-variant enum). C won on depth — it is the only design that *derives* the
  membership instead of re-spelling the 8-set and 9-set inside the new seam — and on seam placement
  (two real adapters matching two mechanisms the file already distinguishes) and test surface (an
  exhaustive `match` with no `_` arm).
  **Opened as #1327.** Verification: differential IR over `tests/run/` against the parent commit with
  a fresh `VOW_CACHE_DIR` — **213/213 byte-identical, 0 different** (212 produced IR; the 213th,
  `u64_marker_propagation.vow`, fails type-checking identically on both sides, a pre-existing
  Rust-stage-0 divergence also recorded by the 2026-09-18 firing). Gate: build, clippy `-D warnings`,
  1682 tests / 0 failures, fmt — all green. Diff: 1 file, 169 added / 57 deleted, of which ~135
  added lines are the three new tests — inside the ~1-file estimate. One clippy fix during
  implementation (`needless_borrow` on the reused `scalar_ty_for_field_type_name` call).

## loop-scope-break-policy

- **Status**: proposed
- **Score**: 19/25 (leverage 3, locality 4, blast radius 2, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs:2671-2686` (`While`), `:2687-2718` (`ForEach`), `:2719-2755`
  (`Loop`); consumers `:2756-2784` (`Break`), `:2785-2794` (`Continue`); state `:980` (`in_loop`),
  `:983` (`break_types_stack`).
- **Summary**: `loop_kind_spec(kind) -> LoopSpec { vow_context, break_slot }` over
  `{Push(None), Push(Some(vec![])), NoPush}` plus a private `with_loop_scope` applier owning
  `in_loop` and the stack symmetrically; each arm keeps its own type computation.
- **First seen**: 2026-09-21
- **Report**: `.architecture/reviews/2026-09-21-narrow-literal-context-admission.md`
- **Reason**: **runner-up candidate** this firing at 22, one tie-break step behind the pick, and the
  natural next firing. `ForEach` increments `in_loop` but **never pushes onto `break_types_stack`**
  (`:2713-2715`), where `While` pushes `None` (`:2680-2684`) and `Loop` pushes `Some(Vec::new())`
  (`:2723-2727`). Three wrong behaviours follow: `loop { for i in v { break 42; } break 7; }` merges
  `42` into the outer `loop`'s type while `vow_syntax::ast::loop_break_values`
  (`vow-syntax/src/ast.rs:402`, `:431`) never walks a for-each body, so type-check and lowering
  (`vow-ir/src/lower/mod.rs:4371`, `:4417`) disagree; `while c { for i in v { break 42; } }` blames
  the `while`; a top-level `for i in v { break 42; }` is silently accepted. `ExprKind::ForEach` has
  **zero** test coverage in `check.rs`. The deepening is behaviour-preserving via the `NoPush` row;
  the anomaly itself is a correctness fix worth a separate issue.
- **Parity re-check 2026-09-22** (rescored 22→19: leverage 4→3, blast radius 1→2): **the premise
  does not hold.** Self-hosted `EXPR_FOR` *does* push loop kind 0 (`compiler/checker.vow:2998-2999`)
  and `break` with a value is rejected under kind 0 (`:3101`), matching `grammar.md:674/723`. The
  `NoPush` row would import the Rust bug into the primary compiler. Remaining work is a Rust
  correctness fix (ForEach pushes `None`) plus a cosmetic Vow seam — file as a fix, not a deepening.
  Rust anchors drifted +44 (While `:2715`, ForEach `:2731`, Loop `:2763`). Report: `.architecture/reviews/2026-09-22-narrow-int-width-self-hosted.md`.

## builtin-arg-layout-spec

- **Status**: proposed
- **Score**: 21/25 (leverage 4, locality 4, blast radius 1, heat 4)
- **Files**: ~1 estimated
- **Modules**: `vow-verify/src/c_emitter.rs` — tables `:218-227`, `:233-240`, `:296-318`, `:320-326`,
  `:332-339`; inline destructurings `:1346-1351`, `:1413-1418`, `:1452-1457`, `:1474-1479`,
  `:1489-1494`, `:1588-1594`, `:1613-1619`; arena-offset closure `:1704-1714`.
- **Summary**: `builtin_arg_layout(name) -> Option<ArgLayout { arena_offset, receiver, value, extra }>`,
  collapsing thirteen restatements of the "plain ⇒ receiver `args[0]`; `_in_arena` ⇒ arena `args[0]`,
  receiver `args[1]`" convention.
- **First seen**: 2026-09-21
- **Report**: `.architecture/reviews/2026-09-21-narrow-literal-context-admission.md`
- **Reason**: `__vow_vec_pin_to_root_val` is absent from `vec_model_receiver_arg` (`:218-226`) while
  its twin `__vow_string_pin_to_root` is present at `:307`. It *is* in `is_vec_model_creator`
  (`:178`), so its result becomes a `__vow_vec_t`, but `collect_typed_vars` (`:366-370`) never marks
  its source, so `emit_inst` (`:1341-1344`) can emit `__vow_vec_t v{id} = int64_t v{source};` — the
  `int64_t = __vow_vec_t` class this file's own comment at `:260-262` cites issue #505 for. Latent in
  the common flow. Held at 21 on heat 4 (`c_emitter.rs`, 16 commits/90d, last 2026-09-02).
- **Parity re-check 2026-09-22** (score unchanged): `compiler/c_emitter.vow:716-767` carries the same
  tables and ~14 inline restatements, and the **same** missing `__vow_vec_pin_to_root_val` receiver
  row (`:727-737`, present in `is_vec_model_constructor` `:687`). Now ~3 files (`c_emitter.rs`,
  `c_emitter.vow`, `compiler/tests/test_c_emitter.vow`). Report: `.architecture/reviews/2026-09-22-narrow-int-width-self-hosted.md`.

## parse-opt-payload-spec

- **Status**: proposed
- **Score**: 19/25 (leverage 3, locality 4, blast radius 1, heat 4)
- **Files**: ~1 estimated
- **Modules**: `vow-verify/src/c_emitter.rs:1636-1686` (7 arms); same 9-name set re-spelled at
  `:414-422` (`collect_option_vars`) and `:535-543` (`is_known_builtin`).
- **Summary**: `parse_opt_model(name) -> Option<OptionParseModel { payload_nondet, payload_range }>`;
  the two list sites become `.is_some()`.
- **First seen**: 2026-09-21
- **Report**: `.architecture/reviews/2026-09-21-narrow-literal-context-admission.md`
- **Reason**: `:1677` emits `__VERIFIER_nondet_ulong()`, which the preamble never declares —
  `emit_c_preamble` (`:2966`) declares `__VERIFIER_nondet_unsigned_long`, the spelling
  `c_nondet_suffix` (`:2094-2112`) calls canonical, and every other unsigned sibling (`u8` `:1649`,
  `u16` `:1670`) uses plain `__VERIFIER_nondet_long()`. Two tests pin the two spellings independently
  (`:3690`, `:4475`) and neither compiles the emitted C, which is why it survived.
- **Parity re-check 2026-09-22** (rescored 21→19: leverage 4→3): the Vow side is already factored via
  `emit_narrow_parse_option` (`compiler/c_emitter.vow:1953-1964`) and does **not** have the
  `nondet_ulong` bug; only the i32 arm (`:2270-2281`) is still inline. What remains is a Rust fix.
  Overlaps `nondet-spec`. Report: `.architecture/reviews/2026-09-22-narrow-int-width-self-hosted.md`.

## extern-heap-origin-kind

- **Status**: proposed
- **Score**: 20/25 (leverage 4, locality 4, blast radius 1, heat 3)
- **Files**: ~1 estimated
- **Modules**: `vow-ir/src/region.rs:1917-1977` (five predicates + the `||` chain
  `heap_producing_extern`); consumers `:1694`, `:2029-2048`, `:3049`, `:3195`.
- **Summary**: `extern_heap_origin(sym) -> Option<HeapOriginKind>`, one row per family with a single
  `_in_arena` suffix rule; `heap_producing_extern` collapses to `.is_some()`.
- **First seen**: 2026-09-21
- **Report**: `.architecture/reviews/2026-09-21-narrow-literal-context-admission.md`
- **Reason**: rows are missing because the kind tag is discarded at the `||` join.
  `__vow_btreemap_new` is absent from `map_creation_extern` (`:1967-1969`) though lowering emits it at
  `vow-ir/src/lower/mod.rs:3100` exactly parallel to `__vow_map_new` at `:3088`, so every
  `BTreeMap::new()` is classified non-heap. The eight
  `__vow_string_parse_{i8,i16,i32,u8,u16,u32,u64}_opt` siblings are absent from
  `option_creation_extern` (`:1960-1965`). `vec_creation_extern` (`:1925`) omits the `_in_arena`
  variants that `string_creation_extern` spells out for all 14 of its families. Held at 20 on heat 3
  — `region.rs` is 11 commits/90d, last touched 2026-08-17.
- **Parity re-check 2026-09-22** (score unchanged): `compiler/region.vow:4263-4318` matches row for
  row, with the same missing rows; ~3–4 files for both compilers. Report: `.architecture/reviews/2026-09-22-narrow-int-width-self-hosted.md`.

## extern-container-op-spec

- **Status**: proposed
- **Score**: 20/25 (leverage 4, locality 4, blast radius 1, heat 3)
- **Files**: ~1 estimated
- **Modules**: `vow-ir/src/region.rs:1979-1994` (`for_each_extern_store_edge`), `:1996-2013`
  (`extern_growth_target`), `:2015-2027` (`extern_mutation_operation`), `:2406-2414`
  (`vec_clear_targets`), `:2416-2435` (`vec_element_write_source`).
- **Summary**: `extern_container_op(sym) -> Option<ContainerOp { receiver_arg, stored_value_args, label, grows }>`
  with the five functions becoming thin appliers.
- **First seen**: 2026-09-21
- **Report**: `.architecture/reviews/2026-09-21-narrow-literal-context-admission.md`
- **Reason**: `__vow_vec_push_val`, `__vow_vec_push_val_in_arena` and `__vow_vec_set_val`
  (`:1981-1983`) are known store edges and element writes but appear in **neither**
  `extern_growth_target` nor `extern_mutation_operation`, while their non-`_val` twins do (`:1998`,
  `:2017`) — so a push into a rodata-backed `Vec` via `__vow_vec_push_val` escapes
  `check_literal_mutations_post_inference` (`:3358-3414`). Arity guards drift on the identical symbol
  pair: `args.len() >= 3` at `:1984` vs `!args.is_empty()` at `:2007`/`:2010`. Overlaps
  `extern-heap-origin-kind` in the same file — whichever lands first re-anchors the other's lines.
- **Parity re-check 2026-09-22** (score unchanged): `compiler/region.vow:4320-4384` + `:1519-1545`
  match row for row incl. the missing `push_val` rows; no arity-guard drift on the Vow side (uniform
  `pos >= 0 && pos < len`). Report: `.architecture/reviews/2026-09-22-narrow-int-width-self-hosted.md`.

## extern-abi-spec-table

- **Status**: dropped
- **Score**: 23/25 before the filter (leverage 4, locality 5, blast radius 1, heat 5)
- **Files**: ~1 estimated (132 arms / 625 lines)
- **Modules**: `vow-codegen/src/cranelift_backend.rs:2451-3075` (the `match sym` inside
  `make_extern_sig`, `:2412-3077`); 9 call sites; an independently-maintained identical copy at
  `vow-clif-shim/src/lib.rs:3330+`.
- **Summary**: `extern_abi_spec(sym) -> Option<ExternAbi { params, ret }>`, with `make_extern_sig`
  keeping only the call conv, the three computed families and an applier.
- **First seen**: 2026-09-21
- **Report**: `.architecture/reviews/2026-09-21-narrow-literal-context-admission.md`
- **Reason**: **dropped — competes with a recorded architectural direction** (a judgement drop, not
  the ADR hard filter, which `docs/adr/0001-0003` do not trigger; `dropped` is reversible). `CLAUDE.md`
  designates `docs/spec/operations.json` + `scripts/generate_operations.py` as the single checked
  source of runtime-symbol/ABI facts and splices `catalogue_extern_sig` into *this very function*
  between `GENERATE:OPERATIONS` markers (`:2392-2410`); it states that follow-ups #1271–1275 "should
  add entries and target files to the existing generator rather than inventing a new mechanism". A
  hand-rolled parallel Rust table is that invention. **Reopen when #1271–1275 extend the catalogue to
  these operation groups** — the two divergences found are worth carrying into that work:
  `__vow_fs_write` is declared `-> i32` in `vow-runtime/src/lib.rs:3516` while
  `cranelift_backend.rs:2654-2658` and `vow-ir/src/lower/mod.rs:53` both say `i64`, and every sibling
  `__vow_fs_*` status return is `i64` (`:3429`, `:3495`, `:3507`, `:3556`, `:3604`, `:3622`, `:3688`);
  and `__vow_string_eq` (`:1896`) / `__vow_string_contains` (`:1910`) return `i64` where IR types them
  `Ty::Bool` and Cranelift declares `types::I8` (`:2577`, `:2582`), unlike `__vow_map_contains`
  (`:4227`) and `__vow_btreemap_contains` (`:4407`) which are `-> bool`. The vow-codegen and
  vow-clif-shim tables were diffed symbol-by-symbol and are currently in sync — no drift between them.

## model-capacity-bound-spec

- **Status**: dropped
- **Score**: 19/25 (leverage 3, locality 4, blast radius 1, heat 4)
- **Files**: ~1 estimated (12 sites)
- **Modules**: `vow-verify/src/c_emitter.rs:1338`, `:1355`, `:1363`, `:1421`, `:1446`, `:1483`,
  `:1498`, `:1735`, `:1814`, `:1936-1962`, `:2047-2052`, `:2260-2286`.
- **Summary**: `model_capacity_spec(kind, limits) -> CapacitySpec { c_ty, max, bound_is_inclusive, extra_invariant }`,
  making the `<` vs `<=` choice one reviewable column instead of twelve string literals.
- **First seen**: 2026-09-21
- **Report**: `.architecture/reviews/2026-09-21-narrow-literal-context-admission.md`
- **Reason**: **dropped — leverage 3.** The `<`-at-create / `<=`-at-havoc split correlates cleanly
  with "freshly created value" vs "opaque incoming value": coherent but undocumented policy, not a
  defect. The one self-inconsistency is within strings — `__vow_string_new` (`:1421`) assumes
  `len < string_max` while `__vow_string_push_str` (`:1483`) asserts `<= string_max`, so `push_str`
  may model a string `string_new` never can; `push_byte` (`:1498`) uses `<`. Deciding which is correct
  is a behaviour fork the autonomy contract reserves for a human. Tests pin both spellings separately
  (`<=` at `:5618`, `:5661`, `:5697`, `:6448`, `:6496`, `:6544`; `<` at `:5993`, `:6622`, `:6827`),
  which is why nothing flags the split.

## fs-path-arg-prologue

- **Status**: dropped
- **Score**: 19/25 (leverage 3, locality 4, blast radius 1, heat 4)
- **Files**: ~1 estimated
- **Modules**: `vow-runtime/src/lib.rs:3411`, `:3429`, `:3516`, `:3537`, `:3556`, `:3574`, `:3604`,
  `:3622`, `:3640`, `:3664`, `:3688`.
- **Summary**: `vow_path_arg(ptr) -> Option<&str>` collapsing the identical null-check →
  `sanitize_on_read` → `VowVec` deref → `from_utf8` prologue repeated across nine `__vow_fs_*` entry
  points.
- **First seen**: 2026-09-21
- **Report**: `.architecture/reviews/2026-09-21-narrow-literal-context-admission.md`
- **Reason**: **dropped — leverage 3.** Real repeated prologue, but the seam is an argument-decoding
  helper rather than a policy table, and each caller's error sentinel differs, so the epilogue stays
  at the call site either way. Worth doing; not the highest-leverage deepening available.

## narrowing-conversion-matrix

- **Status**: dropped
- **Score**: n/a — excluded by the leverage-1 hard filter
- **Files**: n/a
- **Modules**: `vow-types/src/env.rs:352-461` (three near-identical generator loops) and
  `vow-ir/src/lower/mod.rs:161-184` + `:78-116`.
- **Summary**: unify the 27 source→target × 3 mode narrowing-conversion name matrix spelled on both
  sides of the type-checker/lowering seam.
- **First seen**: 2026-09-21
- **Report**: `.architecture/reviews/2026-09-21-narrow-literal-context-admission.md`
- **Reason**: **dropped — leverage 1, fails the deletion test.** Both sides were enumerated in full:
  81 names each, **no live drift**. The only asymmetries are intentional-looking (u8 accepts
  `i128`/`u128` sources; no other target does). Complexity would move to a shared constant, not
  concentrate. Recorded so a future firing does not re-derive and re-drop it.

## main-help-prologue

- **Status**: dropped
- **Score**: ~16/25 (leverage 2, locality 3, blast radius 1, heat 4)
- **Files**: ~1 estimated
- **Modules**: `vow/src/main.rs:826-1084`.
- **Summary**: collapse the eight byte-identical
  `if x.help { if x.human { skill::human() } else { skill::json() } return }` prologues and seven
  `source file required` epilogues.
- **First seen**: 2026-09-21
- **Report**: `.architecture/reviews/2026-09-21-narrow-literal-context-admission.md`
- **Reason**: **dropped — leverage 2.** The interface would shrink but callers do the same work; the
  seam is a formatting/exit helper, not a policy table. Overlaps the existing `driver-command-epilogue`
  entry. Worth noting for whoever takes that one: `Some(Command::Mutants(_))` (`:1008-1014`) is the
  lone arm carrying no `--help` handling at all.

## builtin-method-spec

- **Status**: landed
- **Score**: 22/25 (leverage 4, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (**actual: 1**). Diff estimate was ~250 removed / ~120 added plus ~60 test
  lines; **actual 430 added / 384 deleted in 1 file, net +46**. About 262 of that churn is the five
  kept arms re-indented one level into the new `else` block, counted on both sides; excluding it,
  real churn is ~552 against ~430 estimated — a 28% overshoot, inside the 2x bail-out threshold, and
  driven by the table growing from an estimated 18 rows to 20.
- **Modules**: `vow-ir/src/lower/mod.rs:3519-3902` — the `match (recv_struct, method)` in the
  `ExprKind::MethodCall` arm of `lower_expr` (`:1405`). Seam sited beside the landed `builtin_result_tag`
  (`:199-259`) and the `vow_static_builtin_to_runtime` name table (`:60-159`).
- **Summary**: table the 18 uniform builtin-method lowering arms into a pure
  `builtin_method_spec(recv, method) -> Option<MethodSpec>` carrying `{symbol, ret_ty, arity,
  consume_arg, missing_arg, result_struct_tag}`, with a generic applier keeping `ctx.emit` and the
  `inst_struct_type` insert at the call site. The 7 non-uniform arms (`String::substring`,
  `String::parse_i64`, `String::parse_u64`, `HashMap::insert`, `BTreeMap::insert`, `Vec::push`, the
  `unwrap` fallback) stay inline — each does real extra work. **Parity gap, not precedent:** landed
  Rust-only; `compiler/lower.vow` (which mirrors the same arms at `:3810/3826/3949/3967`) was left
  untouched. `CLAUDE.md` requires both compilers to change in the same session, with no exemption for
  behaviour-preserving changes, so this is an outstanding debt and must not be cited to justify
  skipping the self-hosted side.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **PR**: #1299 (merged 2026-09-18; reconciled 2026-09-21 via `gh pr view 1299` → MERGED)
- **Reason**: **picked this firing** (2026-09-18). Fresh candidate; the arm had never been carded
  despite heavy `lower/mod.rs` coverage (prior firings carded `unwrap-payload-ty` and the landed
  `builtin-result-tag`, never the method-dispatch table). Leverage 4 by precedent with the landed
  `builtin_constructor_spec` and `builtin_result_tag` — same shape: one dispatch site, a name-keyed
  table, a pure spec output. Heat 5: `lower/mod.rs` was touched two days prior by PR #1290, which
  extracted the sibling `builtin_result_tag` seam from this same file. **The pick was close — three
  runners-up at 21**: `vec-reserve-next-capacity-seam`, `arena-variant-rule` and
  `esbmc-auto-timeout-policy`. Won on being the only candidate at heat 5, on being purely
  behaviour-preserving (the other two 21s change behaviour on at least one path), and on sitting in
  `vow-ir`, where IR-shape `#[cfg(test)]` assertions plus the 213-program `tests/run/` corpus check
  the change — unlike `vow-codegen`, which produces stage-0 `vowc1` with no IR-level diff to lean on.
  Three drifts the table makes visible: `String::contains` (`:3557`) uses `lower_expr` where all 17
  siblings use `lower_consumed_expr`; `Vec::truncate` (`:3868`) falls back to `ConstI64(0)` where the
  others use `ConstUnit`; `BTreeMap::get` (`:3710`) tags its result `"Option"` while `HashMap::get`
  (`:3772`) returns a bare `Ty::I64` and tags nothing. **No overlap with `builtin_result_tag`**: that
  seam is keyed on free-function builtin names and dispatches only at `:1815`; `BTreeMap::get`'s bare
  `"Option"` tag (no element type) is not expressible as `BuiltinResultTag::OptionOf(Ty)`.
  Adjudicated **design A** (minimal surface: one enum + a 4-tuple) over **C** (shape-variant enum,
  the runner-up design), **B** (uniform spec, 24 arms) and **D** (single adapter, spec private). A
  and C tied on depth, seam placement and test surface; A won on blast radius. A correction during
  adjudication changed the winner: the first pass ranked C above A on depth, believing only C could
  table `parse_i64`/`parse_u64` — A's result tag is the fourth tuple slot, independent of
  `MethodArg`, so A tables them too, reaching 20 of 25 arms.
  **Landed:** IR identity verified differentially against `HEAD~1` over `tests/run/` — 212 identical,
  0 different, 1 skipped for a type error pre-existing on both compilers. Gate: build, clippy
  `-D warnings`, 1676 tests / 0 failures, fmt — all green. `clippy::type_complexity` does **not**
  fire on the 4-tuple, settling design A's one open question empirically.

## builtin-result-tag

- **Status**: landed
- **Score**: 22/25 (leverage 4, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (actual: 1; +79/-31 in `vow-ir/src/lower/mod.rs`)
- **Modules**: `vow-ir/src/lower/mod.rs` (`tag_builtin_result` L201-251, single caller L1808;
  seam sited beside the pure `narrow_intrinsic_target` L161-183 and the `vow_static_builtin_to_runtime`
  name table L60-159)
- **Summary**: extract the ~40-name builtin-result classification (String heap / Vec heap /
  `Option<elem_ty>`) into a pure `builtin_result_tag(name) -> Option<BuiltinResultTag>`, mirroring the
  landed `builtin_constructor_spec` seam; `tag_builtin_result` keeps only the `ctx.inst_struct_type`
  / `inst_option_elem_ty` inserts. Preserves the `_try`+`narrow_intrinsic_target` early-path ordering
  (the `i16_to_u8_try` / `i64_to_i32_try` fall-through traps). **Parity gap, not precedent:** landed
  Rust-only; `compiler/lower.vow` was left untouched, which `CLAUDE.md` does not permit for
  behaviour-preserving changes. Outstanding debt, not a pattern to repeat.
- **First seen**: 2026-09-16
- **Report**: `.architecture/reviews/2026-09-16-builtin-result-tag.md`
- **PR**: #1290 (merged 2026-09-16; reconciled 2026-09-18 via `gh pr view 1290` → MERGED)
- **Reason**: **picked the 2026-09-16 firing**. Fresh candidate — `tag_builtin_result` was hand-
  edited by #1288 two days prior (adding `proc_sample`), evidence the untested list already drifts.
  Scored leverage 4 by precedent with the landed `builtin_constructor_spec` (same shape: one dispatch
  site, name-keyed table, pure spec output). Tied `call-argument-coercion-action` at 22; won the
  deterministic tie-break on most-recently-touched file (`lower/mod.rs` #1288 > `cranelift_backend.rs`
  #1279). Runner-up *candidate* by score `coerce-context-argument-epilogue` (21, within 1 pt).

## question-operator-verdict

- **Status**: landed
- **Score**: 22/25 (leverage 4, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (actual: 1)
- **Modules**: `vow-types/src/check.rs` (`ExprKind::Question` arm of `check_expr_inner`, ~L2776-2830;
  seam sited beside `cast_verdict` L668 / `same_operand_ty` L808)
- **Summary**: extract the inline 5-way `?`-operator payload policy (Option-ok → unwrap arg0;
  Option-without-Option-return → reject; Result → reject "not lowered"; `Never` → propagate; anything
  else → reject "needs Option/Result") into a pure
  `question_verdict(inner_ty, return_ty) -> Result<Ty, QuestionReject>`, mirroring the landed
  `same_operand_ty` Result-shaped seam; diagnostic emission and the `pattern_aggregates` bookkeeping
  stay at the call site.
- **First seen**: 2026-09-14
- **Report**: `.architecture/reviews/2026-09-14-question-operator-verdict.md`
- **PR**: #1281 (merged 2026-09-14; reconciled 2026-09-16 via `gh pr view 1281` → MERGED)
- **Reason**: **picked this firing** (2026-09-14). Fresh candidate surfaced by the step-1 scan; not
  previously carded despite heavy `check.rs` coverage (prior firings carded the cast/operator/method
  arms, never the `?` arm). Scored leverage 4, matching the landed single-arm pure-verdict seams
  `cast_verdict`/`builtin-constructor-spec`; above the leverage-3 error-reason-only extractions
  (`arm-pattern-support-classifier`, `builtin-receiver-kind`) because the success path returns a
  computed payload `Ty` used downstream, not just a rejection reason. Deterministic top at 22; pick was
  close — runner-up candidates `coerce-context-argument-epilogue` and `call-argument-coercion-action`
  at 21 (within 1 pt). Adjudicated
  design C (Result-shaped, mirrors `same_operand_ty`) over A (classification enum à la `cast_verdict`)
  and B (verdict owns the diagnostic strings). Adjudicated without an advisor (rate-limited this run).

## same-operand-type-verdict

- **Status**: landed
- **Score**: 23/25 (leverage 4, locality 5, blast radius 1, heat 5)
- **Files**: ~1 estimated (actual: 1)
- **Modules**: `vow-types/src/check.rs` (`check_same_numeric`, `check_same_integer`;
  seam `same_operand_ty` at L784, `OperandError` L763, `OperandClass`)
- **Summary**: collapse the two line-for-line-twin operand-type checks into one pure
  `same_operand_ty(lhs, rhs, class) -> Result<Ty, OperandError>` (Design C, adjudicated over the
  bespoke-enum Design A), mirroring the landed `cast_verdict` seam; each method keeps only its distinct
  `ErrorCode::TypeMismatch` message/hint at the call site.
- **First seen**: 2026-09-03
- **Report**: `.architecture/reviews/2026-09-03-same-operand-type-verdict.md`
- **PR**: #1234 (merged 2026-09-03)

## integer-literal-range-fit

- **Status**: landed
- **Score**: 22/25 (leverage 4, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (actual: 1)
- **Modules**: `vow-types/src/check.rs` (`literal_out_of_range` seam beside `integer_type_range`;
  adapter `check_integer_value_range`; call sites L1522, L2003)
- **Summary**: extract the pure literal-fits-target decision + range-text into
  `literal_out_of_range(value, target) -> Option<String>` (Design A, adjudicated over the
  neutral-bounds newtype Design C on depth), leaving `emit_error_with_hints` at the call site; pins
  the `negative_max`/`i64::MIN` asymmetry.
- **First seen**: 2026-08-31
- **Report**: `.architecture/reviews/2026-09-04-integer-literal-range-fit.md`
- **PR**: #1241 (merged 2026-09-04)

## call-argument-coercion-action

- **Status**: proposed
- **Score**: 19/25 (leverage 3, locality 3, blast radius 1, heat 4)
- **Files**: ~1 estimated
- **Modules**: `vow-codegen/src/cranelift_backend.rs:425-459` (`coerce_call_argument`; re-anchored
  2026-09-18 from ~L398-432). 2 production call sites (L1523, L1630) + 6 test; shim twin at
  `vow-clif-shim/src/lib.rs` has 2 production + 4 test.
- **Summary**: extract the pure coercion decision into
  `call_argument_coercion(actual_bits, expected_bits, is_i128, signed) -> CoercionAction`, leaving
  `builder.ins()` emission at the call site. Codegen must stay a byte-identical bootstrap fixed point.
- **First seen**: 2026-08-31
- **Reason**: **rescored 21→19 on 2026-09-18** (leverage 4→3, locality 4→3). New evidence: there is no
  shared crate to hold a `CoercionAction` type — `vow-clif-shim` depends only on `vow-linker` +
  cranelift, `vow-codegen` on `vow-ir`. Extraction therefore yields *two* independently-testable
  copies that still must be hand-synced, buying none of the cross-crate agreement that is the only
  thing worth paying for here. Both sides are already tested
  (`abi_and_field_widening_preserve_logical_signedness`,
  `call_argument_coercion_refuses_only_wide_narrowing`, each with a shim twin). Also corrected: the
  earlier "byte-identical-bootstrap risk" framing was wrong — the fixed point is
  `sha256(vowc2) == sha256(vowc3)`, both produced by the self-hosted pipeline
  (`scripts/bootstrap.sh:224-225`); `vow-codegen` produces only stage-0 `vowc1`. The real risk is that
  a silent stage-0 miscompile has no IR-level diff to catch it.
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## arm-pattern-support-classifier

- **Status**: proposed
- **Score**: 20/25 (leverage 3, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs:3167-3225` (`validate_arm_pattern`; classifying match
  3168-3212, emit tail 3214-3224). Re-anchored 2026-09-18 from ~L3045.
- **Summary**: extract the pure unsupported-match-arm reason into
  `unsupported_arm_pattern(pat, is_last) -> Option<(&str, &str)>`, leaving the emit/return-bool
  wrapper at the call site.
- **First seen**: 2026-08-31
- **Reason**: re-heated 19→20 as `check.rs` moved to heat 5. Re-verified 2026-09-18: the classifying
  match is already a value-returning expression with zero `&self` — the cleanest pure shape in the
  file and the lowest-risk extraction — but no defect sits behind it, which is what holds leverage
  at 3.
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## ce-trace-reconstruction

- **Status**: proposed
- **Score**: 18/25 (leverage 3, locality 4, blast radius 1, heat 3)
- **Files**: ~1 estimated (26 + 25 lines)
- **Modules**: `vow/src/counterexample.rs:331-356` (execution path) and `:357-381` (branch decisions),
  inside `build_structured_counterexample_with_module` (`:188-399`, 212 lines). Re-anchored
  2026-09-18 (drift +4).
- **Summary**: extract the two pure block-visit → source-trace loops into
  `reconstruct_execution_path` / `reconstruct_branch_decisions`, leaving blame/name/call-site work in
  the builder.
- **First seen**: 2026-08-31
- **Reason**: **rescored 20→18 on 2026-09-18** (leverage 4→3): both loops are total pure functions of
  `(&[BasicBlock], &HashSet<u32>)`, but each has exactly one call site, so nothing "stops reaching
  past a seam". The payoff is test surface, and it is real: the only test covering both loops
  (`:1743-1840`, `execution_path_and_branch_decisions_from_block_visits`) spends **98 lines of
  scaffolding** — a full `Function`, 3 `BasicBlock`s, 4 `Inst`s, a `VowEntry`, a `Counterexample`, an
  empty `call_site_index` — to assert 8 facts. Heat 3 (`counterexample.rs` 7 commits/90d).
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## narrow-shift-findings

- **Status**: proposed
- **Score**: 20/25 (leverage 3, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs:2084-2115` (narrow `Shl`/`Shr` arm; re-anchored 2026-09-18
  from ~L1875-1906)
- **Summary**: extract the two independent shift-operand checks (count must be `u32`; const count in
  range) into a pure seam returning a **struct of two findings** (not a single-variant enum — both can
  fire on one expression via `Cast`-folded const counts).
- **First seen**: 2026-09-03
- **Reason**: re-verified 2026-09-18; best-pinned candidate in `check.rs` (8 existing tests at
  L4427/4436/4452/4492/4501/4517/4821/4383). The `.expect("narrow shift types have an integer width")`
  at L2092 re-derives a width the arm's own guard already decided — an `Option<u16>` predicate
  removes it.
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## builtin-constructor-spec

- **Status**: landed
- **Score**: 22/25 (leverage 4, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (actual: 1; +260/-131 in `vow-types/src/check.rs`)
- **PR**: #1268 (merged 2026-09-11)
- **Modules**: `vow-types/src/check.rs` (`EnumConstruct` builtin dispatch, ~L2867-3016)
- **Summary**: extract the inline `match (enum, variant)` builtin-constructor policy (arity, per-arg
  `ArgExpect`, result shape) into a pure `builtin_constructor_spec(enum, variant) -> Option<CtorSpec>`
  table, mirroring the landed `method_result_type`/`method_argument_expectations` seams; the generic
  argument-checking loop and payload-wrapping stay at the call site.
- **First seen**: 2026-09-04
- **Report**: `.architecture/reviews/2026-09-11-builtin-constructor-spec.md`
- **Reason**: picked 2026-09-11; **merged 2026-09-11** (reconciled 2026-09-14 via
  `gh pr view 1268` → MERGED). Adjudicated design C (mirror the landed-seam pattern) over A (minimal,
  deepens only 8/11) and B (uniform spec, widest interface).

## comparison-operand-verdict

- **Status**: proposed
- **Score**: 18/25 (leverage 2, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (~52 lines)
- **Modules**: `vow-types/src/check.rs` (`Eq|Ne|Lt|Le|Gt|Ge` sub-arm, now ~L2028-2080)
- **Summary**: fold the comparison type-mismatch predicate + zero-comparison-tautology (with the
  `widened_to` split) into a pure `comparison_verdict(lhs, rhs, never_negative) -> ComparisonVerdict`;
  caller pre-computes `never_negative` from the existing `zero_comparison_verdict` +
  `never_negative_operand` helpers, message/hint strings stay at the call site.
- **First seen**: 2026-09-11
- **Reason**: **rescored 20→18** on 2026-09-16 (leverage 3→2): the step-1 explore pass found the
  tautology half is already extracted (`zero_comparison_verdict` L565, `never_negative_operand`
  L1927) and the composite verdict cannot be fully pure — `never_negative_operand` reads
  `self.nonneg_casts` — so the only un-extracted pure piece is a one-line mismatch predicate.
  Partially resolved by prior seams. Eligible but low; not vetoed. Re-verified 2026-09-18 at
  `check.rs:2028-2080` (line numbers unchanged): `self.nonneg_casts` is genuinely necessary — a
  chained `v as u32 as i64` carries its root source forward — so any seam must take the *lookup
  result* as an argument rather than `&self`. Unusually dense existing coverage (16 tests,
  `:4591`-`:4815`).
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## builtin-function-spec

- **Status**: proposed
- **Score**: 18/25 (leverage 2, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (~70 lines)
- **Modules**: `vow-types/src/check.rs:2212-2240` (`pin_to_root`) and `:2241-2281`
  (`string_matches_literal_at`), ahead of the ordinary `env.lookup_fn` path at `:2282`.
  **Re-anchored 2026-09-18**: the carded ~L2082-2151 is now the bitwise/shift/logical arms.
- **Summary**: table the two free-function builtins into `builtin_function_spec(name) -> Option<..>`,
  mirroring `builtin-constructor-spec`, generic arg loop + static-literal emit stay at the call site.
- **First seen**: 2026-09-11
- **Reason**: **rescored 20→18** on 2026-09-16 (leverage 3→2): the step-1 explore pass confirmed the
  extraction "mostly moves" — only two bespoke builtins, each with its own per-arg checking that
  cannot leave the `&mut self` call site, so the pure spec mostly relabels. Thin. Re-verified
  2026-09-18: `env.rs:140-460 builtin_free_fn_signatures()` is already a 320-line table of builtin
  free functions, and these two are the *exceptions* that did not fit it — so any seam here should be
  shaped as "what extra constraint does this intrinsic impose beyond its signature", not as a second
  signature table. Neither has a named test today, so the red step is larger.
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## oversized-chunk-path-predicate

- **Status**: proposed
- **Score**: 17/25 (leverage 2, locality 3, blast radius 1, heat 4)
- **Files**: ~1 estimated (2 sites)
- **Modules**: `vow-runtime/src/lib.rs:1011` (inside `__vow_arena_alloc`, `:983-1047`) and `:1237`
  (inside `arena_grow_backing`, `:1209-1243`). Re-anchored 2026-09-18 (drift −9).
- **Summary**: single-source the must-agree `bytes > OVERSIZED_THRESHOLD || bytes + (align-1) >
  CHUNK_PAYLOAD` predicate into `const fn takes_oversized_path(bytes, align) -> bool`, called from
  both sites; the L1246 comment documents that its correctness depends on the L1020 decision.
- **First seen**: 2026-09-11
- **Reason**: leverage 2 — one-line predicate; value is a drift-kill, not interface depth. Distinct
  from `vec-reserve-next-capacity-seam`. Re-verified 2026-09-18 with new evidence: the two sites have
  *different* overflow protection. `:1011` is preceded by an explicit guard at `:1016-1019`; `:1237`
  has none, and its safety rests on a **non-local argument** — `__vow_arena_alloc(arena, new_size,
  align)` at `:1221` runs first and guards `new_size + align`, and `old_size < new_size`. Nothing in
  `arena_grow_backing` states this. Both sites also underflow `align - 1` when `align == 0`, so the
  seam should use `saturating_add`/`saturating_sub` and be total.
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## coerce-context-argument-epilogue

- **Status**: proposed
- **Score**: 18/25 (leverage 2, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs` — **site inventory corrected 2026-09-18**: 5 loop-shaped sites
  (`:2254-2267`, `:2340-2353`, `:2919-2943`, `:3056-3072`, `:3116-3133`), 3 single-value sites
  (`:1525-1542`, `:1613-1629`, `:2797-2814`), 2 near-misses on a *different* predicate
  (`:2362-2382` via `ArgExpect::accepts`, `:2862-2874` via the strictly-weaker
  `can_assignment_coerce`), and 2 range-check-only sites with no coercion test (`:1651`, `:2538`).
  The earlier "~10 sites" list was stale.
- **Summary**: collapse the repeated `check_expr + check_contextual_integer_literal_ranges +
  can_context_coerce + emit-mismatch` epilogue into one `&mut self` helper taking a per-site message
  closure. `can_context_coerce` (L263) is already pure; this is sibling-epilogue dedup, not a pure
  seam. Not compiler drift — `compiler/checker.vow` inlines the same glue.
- **First seen**: 2026-09-04
- **Reason**: **rescored 21→18 on 2026-09-18** (leverage 3→2, locality 5→4). The policy is *already*
  pure (`can_context_coerce`, `:263-286`); what repeats is sequencing, and the diagnostic message
  differs at every site, so the honest extraction is a `&mut self` helper taking a message closure —
  a shallow 4-param wrapper of 3 statements, which fails the deletion test on the helper itself.
  Score it as a shallow-helper cleanup, not a deepening. The genuinely interesting finding nearby is
  an *inconsistency*, not the duplication: `:2538` range-checks an index against `I64` while
  `ArgExpect::AnyInteger` (`:318`) range-checks against `I64` but accepts any width, and `:2862` uses
  a strictly weaker predicate than the other nine. A `CoercionSite` enum naming which predicate
  applies where would be the honest deepening, and it is a larger job.
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## recover-unknown-name-prologue

- **Status**: proposed
- **Score**: 18/25 (leverage 2, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs` — **re-anchored 2026-09-18, 5 sites not 4**: `:1991-2005`
  (undefined variable, recovery `Ty::Unit`), `:2285-2304` (undefined function, recovery `Ty::Never`),
  `:2455-2470` (unknown method, + "available methods:" fallback), `:2503-2521` (unknown field, +
  "available fields:" fallback), `:2900-2916` (unknown struct, recovery `Ty::Unit`). All four carded
  line numbers were stale.
- **Summary**: collapse the five unresolved-name recovery epilogues (`suggest_similar` + emit + drain
  child exprs + fallback `Ty`) into one `&mut self` recovery helper. `suggest_similar` (L203) already
  pure; low leverage, divergent messages. The pure part worth extracting is
  `did_you_mean_hints(name, candidates, list_fallback) -> Vec<String>`; candidate *sourcing* and the
  recovery-type choice legitimately differ per site and stay put.
- **First seen**: 2026-09-04
- **Reason**: re-verified 2026-09-18; three hint variants exist (suggestion-only for var/fn/struct,
  suggestion-else-list for method/field) and the recovery type differs (`Unit` vs `Never`) for
  reasons documented only at `:2301-2303`. Pairs naturally with `hint-candidate-capping`.
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## division-abort-spec

- **Status**: proposed
- **Score**: 16/25 (leverage 3, locality 4, blast radius 4, heat 4)
- **Files**: ~3 estimated (3 crates)
- **Modules**: `vow-codegen/src/cranelift_backend.rs` (`divisor_abort_condition`, ~L325);
  `vow-verify/src/c_emitter.rs` (`emit_checked_arith`, ~L2882); `vow-runtime/src/lib.rs`
  (`define_wide_div_rem!`, ~L2987)
- **Summary**: extract the triplicated "zero divisor aborts all div/rem; signed div also aborts on
  `MIN/-1`" policy into a shared pure `division_abort_spec(opcode, ty) -> {check_zero,
  check_min_neg_one}`; each backend keeps its own emission. `vow-runtime` L2987 documents the manual
  sync.
- **First seen**: 2026-09-04
- **Reason**: blast radius 4 (crosses codegen/verify/runtime) plus byte-identical bootstrap risk — a
  human-scheduled pick, not unattended. NB an unverified soundness asymmetry noticed nearby (codegen
  traps wrapping-div-by-zero, verifier may not model it) is recorded in the 2026-09-04 report for a
  human to triage; deliberately not filed. Re-checked 2026-09-18: still blast radius 4, still
  triplicated across `vow-codegen`/`vow-verify`/`vow-runtime`.

## vec-reserve-next-capacity-seam

- **Status**: proposed
- **Score**: 21/25 (leverage 4, locality 4, blast radius 1, heat 4)
- **Files**: ~1 estimated (26 + 7 + 8 lines across 3 sites)
- **Modules**: `vow-runtime/src/lib.rs:1497-1522` (`vec_reserve_in_arena_no_null_check`),
  **`:4193-4199`** (`__vow_map_insert_in_arena`), **`:4362-4369`** (`__vow_btreemap_insert`).
  Two sites added 2026-09-18; `vec_reserve` re-anchored from ~L1473-1519.
- **Summary**: extract the capacity-doubling/overflow policy into a pure
  `next_capacity(old_cap, required, initial) -> Option<usize>` (`None` = caller must `oom_trap`),
  called from all three growth sites; `oom_trap` and the `arena_grow_backing` call stay at each site.
- **First seen**: 2026-08-31
- **Reason**: **rescored 18→21 on 2026-09-18** (leverage 3→4, locality 3→4). New evidence: there are
  *three* growth sites with divergent overflow policy, not one. `Vec::reserve` uses
  `checked_mul(2)` + a `< VOW_CAP_VALUE_MASK` guard + `checked_mul(elem_size)`, and its comment
  (`:1491-1495`) explains exactly why unchecked doubling is a bug — "doubling `new_cap` past
  usize::MAX wraps it to 0 so the `< required` loop never terminates". The two map paths then do
  precisely that: `m.cap * 2` and `new_cap * MAP_ENTRY_BYTES` are unchecked at `:4195-4196`, as are
  `m.keys_cap * 2` and `new_cap * BTREEMAP_ENTRY_BYTES` at `:4364-4365`. NB both maps are constructed
  with an initial cap of 8 (`:4160`, `:4306`), so the zero-cap doubling hazard is unreachable through
  today's constructors and the overflow needs a capacity above `usize::MAX / 2` — this is a latent
  divergence to close, not a live bug, and a PR landing it must say so. Runner-up **candidate** this
  firing (21, one point behind the pick).
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Parity re-check 2026-09-22** (score unchanged): `vow-runtime` is linked by both compilers
  (`vow-linker/src/lib.rs:4`, `vow-clif-shim/src/lib.rs:2918`); no `.vow` twin, so the parity rule is
  satisfied trivially. Report: `.architecture/reviews/2026-09-22-narrow-int-width-self-hosted.md`.

## negation-verdict

- **Status**: proposed
- **Score**: 20/25 (leverage 3, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (38 lines across 2 sites)
- **Modules**: `vow-types/src/check.rs:2161-2183` (`UnaryOp::Neg` arm of `check_expr_inner`) **and
  `:1696-1710`** (the `UnaryOp{Neg}` branch of `check_integer_literal_range`) — second site added
  2026-09-18
- **Summary**: extract the 3-way `{Unsigned, NonNumeric, Ok}` negation decision into a pure verdict,
  leaving the two emits and the literal-folding (L2138-2158) at the call site.
- **First seen**: 2026-09-03
- **Reason**: **rescored 18→20** on 2026-09-16 (leverage 2→3): the arm is a *total pure function of
  `operand_ty`*, testable with zero `&mut self`. **Justification strengthened 2026-09-18** (score
  unchanged): the unsigned-negation rule is encoded *twice* with byte-identical message text
  (`"unary negation is not allowed on unsigned type ..."` at `:1702-1706` and `:2165-2167`), and the
  two sites already disagree on recovery (`:1707` recurses into the operand, `:2170` returns
  `Ty::Unit`). The `Unsigned` branch of the `:2161` arm has no test — `unary_neg_numeric_ok` (:4259)
  and `unary_neg_non_numeric_error` (:4271) cover only the other two.
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## builtin-receiver-kind

- **Status**: proposed
- **Score**: 17/25 (leverage 2, locality 3, blast radius 1, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs:2383-2395` (5 `matches!` booleans) + `:2438-2454` (the
  display-name ladder). **Re-anchored 2026-09-18** from ~L2174-2245.
- **Summary**: extract the pure receiver-kind classification into
  `builtin_receiver(ty: &Ty) -> BuiltinReceiver` with a `display_name` method; marginal on its own,
  because its primary output is a display string the landed-seam pattern keeps at the call site.
- **First seen**: 2026-09-03
- **Reason**: re-verified 2026-09-18 and now demonstrably redundant — `:2446`/`:2449` re-derive
  Option/Result with fresh `matches!` instead of reusing `is_option_or_result` computed at `:2393`,
  and the same classification is spelled a third and fourth time inside `method_result_type`
  (`:392-420`) and `builtin_method_names` (`:441-451`). A shared discriminator would make adding a
  builtin container a one-place edit instead of four. Held at leverage 2: no caller simplifies.
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## unwrap-payload-ty

- **Status**: proposed
- **Score**: 20/25 (leverage 3, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (9 + 21 + 9 lines across 3 sites)
- **Modules**: `vow-ir/src/lower/mod.rs:4206-4214` (`lower_unwrap`, itself now `:4145-4226`),
  **`:3264-3284`** (`ExprKind::Match` arm), **`:4022-4030`** (`ExprKind::Question`, the `?` desugar).
  Two sites added 2026-09-18; `lower_unwrap` re-anchored from ~L4186-4194.
- **Summary**: extract the payload-slot type selection into a total pure
  `payload_slot_ty(aggregate, declared_wide, recorded, slot_index) -> Ty`, with each site
  pre-computing its own `ctx` lookups; `ctx.emit` stays at the call site.
- **First seen**: 2026-09-03
- **Reason**: **rescored 17→20 on 2026-09-18** (leverage 2→3, heat 4→5). As carded — one 9-line
  selection with one call site — it was too thin. There are three sites sharing only the two-rung
  aggregate prefix (`is_linear → LinearPtr`, `is_some → Ptr`) and diverging after it: `:3264-3284`
  adds a `declared_wide_payload_ty` rung and an `if i == 0` slot rule, and **`:4022-4030` omits the
  `variant_payload_ty` consult the other two make**. The extraction must *preserve* that asymmetry,
  not silently fix it — the seam's job is to make it visible. Separately: the comment at `:4202-4205`
  says the omitted lookup is what stops a wide `Result` payload loading as a truncated limb, so the
  `?`-desugar site is either deliberately exempt or latently wrong. **Confirm before unifying; do not
  fold that question into the extraction.**
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## cast-legality-verdict

- **Status**: landed
- **Score**: 22/25 (leverage 4, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (actual: 1)
- **Modules**: `vow-types/src/check.rs` (`ExprKind::Cast` arm of `check_expr_inner`)
- **Summary**: extract the inline 4-way `as`-cast legality policy into a pure
  `cast_verdict(src, tgt) -> CastVerdict`, mirroring the `method_result_type` seam, leaving
  diagnostics and the `nonneg_casts` mutation at the call site.
- **First seen**: 2026-08-31
- **Report**: `.architecture/reviews/2026-08-31-cast-legality-verdict.md`
- **PR**: #1161 (merged 2026-09-01)

## builtin-method-result-type-seam

- **Status**: landed
- **Score**: 22/25 (leverage 4, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (actual: 1)
- **Modules**: `vow-types/src/check.rs` (`ExprKind::MethodCall` arm of `check_expr`, ~L1941-2122)
- **Summary**: extract the inline builtin-method result-type and known-methods resolution into pure
  free functions mirroring the existing `method_argument_expectations` seam, leaving diagnostics in
  `check_expr`.
- **First seen**: 2026-08-31
- **PR**: #1153 (merged 2026-08-31)
- **Report**: `.architecture/reviews/2026-08-31-builtin-method-result-type-seam.md`

## arena-variant-rule

- **Status**: proposed
- **Score**: 20/25 (leverage 4, locality 4, blast radius 2, heat 4)
- **Files**: ~1 estimated (136 lines, 22 arms)
- **Modules**: `vow-codegen/src/cranelift_backend.rs:769-904`; 2 production call sites (`:1582`,
  `:3156`). Target shape already exists in `vow-clif-shim/src/lib.rs:625-819`
  (`routed_vec_extern(sym, inst_rgn, receiver_route) -> (&str, Option<i64>)`); third copy in
  `compiler/clif.vow:244-380`.
- **Summary**: table the arena-routed extern-symbol rule into
  `arena_variant_rule(sym) -> Option<ArenaVariantRule>` over
  `{InstRegion, Receiver, ReceiverWithCandidate}`, with a ~15-line applier keeping `inst.region` and
  `first_arg_route` at the call site. The 22 arms collapse to two shapes: 17 `match inst.region`
  (`Root` → passthrough, else `"<sym>_in_arena"`) and 5 `first_arg_route`
  (`Direct(Block|Caller)` → `_in_arena`; `ProjectionCandidate` → `_in_candidate_arena`, only for
  `string_push_str`/`string_push_byte`).
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: runner-up **candidate** at 21 (one point behind the pick). Signature-mirroring of a
  landed shape rather than invention. Held below `builtin-method-spec` on risk, not score:
  `vow-codegen` produces stage-0 `vowc1` directly and, unlike a `vow-ir` change, has no IR-level diff
  to catch a regression — only the bootstrap triple. Coverage is the 213-program `tests/run/` corpus
  (`string_substring.vow`, `dealloc_string.vow`, `region_string_trim_root_escape_span.vow`,
  `cmdloop.vow`).
- **Parity re-check 2026-09-22** (rescored 21→20: blast radius 1→2): the `compiler/clif.vow:244-380`
  copy agrees with backend + shim but is now an edit target — three implementations plus tests.
  Report: `.architecture/reviews/2026-09-22-narrow-int-width-self-hosted.md`.

## esbmc-auto-timeout-policy

- **Status**: proposed
- **Score**: 20/25 (leverage 4, locality 4, blast radius 2, heat 4)
- **Files**: ~2 estimated (43 Rust lines)
- **Modules**: `vow-verify/src/esbmc.rs:942-964` (`effective_multi_property_config`) and
  `vow-verify/src/solver_strategy.rs:227-246` (`bv_config_for`). Self-hosted mirror at
  `compiler/verifier.vow:504-517` (`effective_timeout_for`, 5 call sites) is documentation, not an
  edit target.
- **Summary**: extract `auto_timeout_secs(encoding, solver, user_timeout) -> Option<u32>` into
  `solver_strategy.rs`, reusing `SolverConfig::resolve()` for the BV-solver rule instead of the two
  local partial copies.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: runner-up **candidate** at 21. The shared rule — *user `--timeout` wins verbatim; else
  Bitwuzla is uncapped; else `DEFAULT_AUTO_TIMEOUT_SECS`* — is spelled three times and the doc comment
  at `esbmc.rs:936-938` admits it. **Not picked because the two Rust spellings already differ**, so
  unifying them is a behaviour change on at least one path, not a pure extraction:
  `effective_multi_property_config` gates on `config.encoding == Encoding::Auto` and `bv_config_for`
  does not, and both re-derive `resolve()`'s `Auto → Boolector` rule locally in a way that disagrees
  with `resolve()` (`solver_strategy.rs:80-84`) for `(Encoding::Ir, Solver::Auto)`, where it yields
  `Z3`. Deciding which spelling is correct is a fork the autonomy contract reserves for a human. The
  duplication has reached the tests too (`solver_strategy.rs:1091/1102/1115` vs `esbmc.rs:2001`
  assert the same four facts).
- **Parity re-check 2026-09-22** (rescored 21→20: blast radius 1→2): under the #1330 rule
  `compiler/verifier.vow:504-517` is an edit target, not documentation. Still a behaviour fork.
  Report: `.architecture/reviews/2026-09-22-narrow-int-width-self-hosted.md`.

## builtin-generic-arity-spec

- **Status**: proposed
- **Score**: 20/25 (leverage 4, locality 4, blast radius 1, heat 3)
- **Files**: ~1 estimated (68 lines)
- **Modules**: `vow-types/src/env.rs:656-723` (`TypeEnv::resolve`, `AstType::Generic` arm); 14
  `resolve` call sites in `check.rs`.
- **Summary**: replace the five hand-written builtin-generic arms with
  `builtin_generic_spec(name) -> Option<BuiltinGeneric>` carrying `{base: Ty, arity: usize}`; one
  arity check, one message shape, then `resolve` mapped over the arguments.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: the five builtin generics use **two different arity policies**. `Option` (`:660`),
  `Result` (`:671`) and `BTreeMap` (`:704`) reject `args.len() != N`; `Vec` (`:682`) and `HashMap`
  (`:692-697`) use `args.first()`/`args.get(1)` with `ok_or_else`, so too *few* arguments error and
  too *many* are silently dropped — `Vec<i64, bool>` resolves to `Ty::Applied(Vec, [I64])`, and the
  surplus argument is never `resolve`d, so an unknown type name in that position is also never
  reported. `vow-syntax/src/parser/types.rs` does not constrain generic arity at parse time.
  Test-shaped gap: `env.rs:1213`/`:1272` have `resolve_{option,result}_generic_rejects_wrong_arity`
  and there is deliberately no `Vec`/`HashMap` equivalent. **Not picked on heat 3** (`env.rs` is 9
  commits/90d, last touched 2026-09-05) — and note it *changes behaviour*: `Vec<i64, bool>` would
  start erroring. That is a correctness fix worth making, but it should land as a fix, not as a
  deepening.

## hidden-region-store-targets

- **Status**: proposed
- **Score**: 19/25 (leverage 3, locality 4, blast radius 1, heat 4)
- **Files**: ~1 estimated (46 lines)
- **Modules**: `vow-codegen/src/cranelift_backend.rs:214-259` — `hidden_region_store_targets`
  (`:214-226`), `hidden_region_param_count` (`:228-237`), `hidden_region_idx_for_store_target`
  (`:239-259`). 3 call sites.
- **Summary**: retype `hidden_region_store_targets` from `&IrFunction` to `&RegionSummary` so
  `hidden_region_idx_for_store_target` can share it instead of re-inlining the identical
  collect/sort/dedup at `:247-252`; `hidden_region_param_count` then becomes a total pure function of
  `(&RegionSummary, is_main)`.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: the tightest precedent match in the audit — `hidden_region_param_count` is the exact
  twin of the shim's landed `hidden_region_count` (`vow-clif-shim/src/lib.rs:355-365`), same
  `is_main → 0`, `FreshInCaller → +1`, `+ store_targets.len()` shape, and the code already carries a
  "Keep this slot order in sync with compiler/clif.vow's clif_hidden_store_targets" comment at
  `:215-216`. Held at leverage 3: only ~10 lines of real duplication. Coverage:
  `vow/tests/region_summary_equivalence.rs`.

## contracts-clause-status-precedence

- **Status**: proposed
- **Score**: 19/25 (leverage 4, locality 4, blast radius 1, heat 2)
- **Files**: ~1 estimated (89 lines)
- **Modules**: `vow/src/contracts.rs:98-186` (`update_contract_statuses`); 1 call site
  (`run_contracts_command`, `:234`), 5 near-identical mutation loops at `:110-114`, `:122-126`,
  `:138-143`, `:158-162`, `:172-176`.
- **Summary**: have the probe block produce a plain
  `FunctionProbe { modelable, esbmc_found, overall, verdicts, vacuous, trivially_satisfiable }` and
  extract `clause_status(entry, func_id, probe) -> (&'static str, bool)`, leaving `find_esbmc`,
  `emit_*_c_source` and `run_esbmc_*` at the call site.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: the *policy* is the precedence order — non-modelable ⇒ `skipped` and skip all probes;
  missing ESBMC ⇒ `error`; else per-clause `resolve_clause_status`; then a `Vacuous` reach verdict
  **overwrites every status including `proven`**; then body-replace sets an orthogonal boolean. Only
  the innermost step is tested (`resolve_clause_status`, `:466`/`:488`); **the vacuous-override
  precedence has zero unit coverage** — `:339` only tallies pre-set statuses. Today "a `proven`
  clause is demoted to `vacuous`" cannot be tested without installing ESBMC. Held down by heat 2
  (`contracts.rs` is 3 commits/90d, last touched 2026-08-24).

## ce-diagnostic-shaping

- **Status**: proposed
- **Score**: 18/25 (leverage 3, locality 4, blast radius 1, heat 3)
- **Files**: ~1 estimated (69 lines)
- **Modules**: `vow/src/verify_outcome.rs:221-289`, inside `to_output_with_warnings` (`:205-367`,
  163 lines); 1 call site (`:220`).
- **Summary**: extract `counterexample_diagnostic(&StructuredCounterexample) -> Diagnostic`, total and
  pure — it already calls only the extracted `blame_to_error_code` (`:177-183`) and
  `blame_to_diag_blame` (`:185-191`). The `match sce.blame.as_str()` hint selection (`:243-263`) and
  the `UNATTRIBUTED_VOW_ID` code/message split (`:264-280`) move with it.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: low-risk and mechanical; the win is altitude (the 163-line function drops to ~95) rather
  than new testability — coverage already exists and carries over unchanged (`:571`, `:588`, `:641`,
  `:659`, `:682`). Ranked accordingly.

## c-source-variant-spec

- **Status**: proposed
- **Score**: 18/25 (leverage 3, locality 4, blast radius 1, heat 4)
- **Files**: ~1 estimated (89 lines across 3 siblings)
- **Modules**: `vow-verify/src/esbmc.rs:573-595` (`emit_verify_c_source`), `:602-625`
  (`emit_reach_c_source`), `:635-661` (`emit_bodyreplace_c_source`); 7 call sites across
  `vow/src/contracts.rs:130,153,171`, `vow/src/verification.rs:116,279`, `esbmc.rs:558`,
  `solver_strategy.rs:760`.
- **Summary**: a `CSourceVariant { Verify, Reach, BodyReplace }` enum plus a pure
  `variant_applies(variant, &Function) -> bool` composing the already-extracted
  `function_has_requires` (`:663`), `function_has_ensures` (`:671`), `returns_scalar` (`:680`) and
  `body_replaceable_result` (`:697`).
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: the three siblings share an identical 9-line body — `collect_modelable_callees` →
  `HashSet` → `emit_c_module_with_callees(.., bool, bool)` → `push_str(&emit_harness(func))` —
  differing only in a guard predicate and **two positional booleans** taking `(false,false)`,
  `(true,false)`, `(false,true)`. The fourth combination `(true,true)` is unrepresentable in intent
  but constructible in code, which is the shallowness worth removing.

## artifact-path-naming

- **Status**: proposed
- **Score**: 16/25 (leverage 2, locality 3, blast radius 1, heat 3)
- **Files**: ~2 estimated (32 lines across 3 policies)
- **Modules**: `vow/src/main.rs:501-505` (build output/object), `:771-783` (decl output, `.vow` →
  `.vow.d`), `vow/src/replay.rs:504-517` (replay harness/bin/object,
  `__vow_replay_{stem}_{vow_id}_{pid}`).
- **Summary**: three total pure functions of `(source, output)` — the replay one additionally taking
  `vow_id` and an **injected** pid rather than calling `std::process::id()` inline, which is its only
  impurity.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: no unit test exercises any of the three. The replay policy is the interesting one: the
  `cleanup` closure at `replay.rs:514-518` must enumerate exactly the paths the naming produced, and a
  divergence leaks temp files into the user's source directory. Held at leverage 2 — three unrelated
  one-call-site policies, no shared rule between them.

## hint-candidate-capping

- **Status**: proposed
- **Score**: 17/25 (leverage 2, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs:2503-2509` (unknown-field hint) vs `vow-types/src/env.rs:66-96`
  (`sorted_capped_keys`); 4 capping sites (`env.rs:632`, `:636`, `:589`, `check.rs:2503`).
- **Summary**: lift `sorted_capped_keys` to take an iterator or `&[String]` instead of a
  `&HashMap<String, V>` and reuse it from all four hint sites, including `all_var_names`' per-scope
  heap. Total and pure: `(&[String], usize, usize) -> Vec<String>`.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: two disagreeing capping policies feed the same "did you mean" surface.
  `sorted_capped_keys` picks the lex-smallest `MAX_HINT_CANDIDATES` names via a bounded max-heap;
  `check.rs:2503-2509` filters by length then `.take(256)` in *declaration* order. For a struct with
  more than 256 fields the field hint is non-deterministic relative to the other four sites, and the
  "available fields:" list is unsorted while "available methods:" (`:2463`) follows the hard-coded
  `builtin_method_names` order. Pin the ordering with a >256-field struct test before unifying.
  Pairs naturally with `recover-unknown-name-prologue`.

## enum-payload-slot-index

- **Status**: proposed
- **Score**: 17/25 (leverage 2, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs:1790-1794`, `:3327-3338`, `:3420-3437`, and implicitly `:416`
  (`method_result_type`'s `unwrap` → `args.first()`).
- **Summary**: `builtin_variant_payload_slots(enum_name, variant) -> Option<&'static [usize]>` —
  `Some([0])` for `Option::Some`/`Result::Ok`, `Some([1])` for `Result::Err`, `Some([])` for
  `Option::None`, `None` for a non-builtin.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: the smallest "one fact, four copies" item in the audit — the `Some/Ok → slot 0,
  Err → slot 1` mapping. `:416` is correct only *because* `Ok` is slot 0, and nothing states that.
  Sites 2 and 3 additionally read `self.env`, so only the mapping itself extracts.

## const-item-literal-spec

- **Status**: proposed
- **Score**: 17/25 (leverage 2, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (77 lines)
- **Modules**: `vow-types/src/check.rs:1268-1344` (Pass 1b2 of `check_module`); 1 call site with three
  duplicated sub-branches.
- **Summary**: `const_item_spec(ty, value) -> ConstSpec` classifying int / bool / negated-int /
  not-a-literal, leaving `env.resolve`, emission, the range check and the
  `const_values`/`const_types` inserts at the call site.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: `ty != Ty::I64 && ty != Ty::I32` appears twice (`:1284`, `:1313`) with an identical
  message, and the "must be a literal" rejection appears twice (`:1328`, `:1336`). The arm interleaves
  four concerns, and the accepted-type set is *narrower than the language* — `const X: u64 = 1;` is
  rejected, a policy statement buried in a `&mut self` pass. **No existing tests**, so the red step is
  larger than for the other `check.rs` candidates; that is what holds it at 17.

## cast-plan

- **Status**: proposed
- **Score**: 17/25 (leverage 2, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (62 lines)
- **Modules**: `vow-ir/src/lower/mod.rs:4044-4105` (`ExprKind::Cast`); 1 call site.
- **Summary**: `cast_plan(literal, src, tgt) -> CastPlan` over
  `{ConstU8, ConstU64, ConstI128, ConstU128, IntCast{from,to}, Passthrough}`. Total and pure.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: clean but thin — one call site and no cross-backend twin, which is what separates it
  from `builtin-method-spec` in the same file. Coverage:
  `integer_literals_lower_at_their_native_ir_width` (`:5847`),
  `explicit_wide_suffixes_lower_through_the_parser_to_native_constants` (`:5922`),
  `vow/tests/wide_literal_ranges.rs`.

## driver-command-epilogue

- **Status**: proposed
- **Score**: 16/25 (leverage 2, locality 3, blast radius 1, heat 3)
- **Files**: ~1 estimated (~40 lines)
- **Modules**: `vow/src/main.rs:726-735` (`run_build_command`) and `:813-823` (`run_verify_command`);
  separately `:539-570` vs `:600-637` inside `run_pipeline_from_frontend`.
- **Summary**: sibling-epilogue dedup — `finish_command(result, session, replay, emit) -> !` over the
  repeated `run_replay_cex` → `session.finish()` → `emit_json()` → `build_exit_code` →
  `process::exit` sequence.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: not a pure-verdict seam — the verdict half (`build_exit_code`, `:61-68`) is already
  extracted and tested (`:1121`). The only divergence between the two epilogues is
  `run_build_command`'s `if !dump_ir` guard on both the replay call and `emit_json`. Lowest-value item
  carded this firing; recorded so the next run does not re-derive it.

## literal-marker-propagation

- **Status**: proposed
- **Score**: 15/25 (leverage 3, locality 4, blast radius 3, heat 5)
- **Files**: ~1 estimated, but a restructure of two mutually-recursive `&mut self` walkers — the
  blast-radius band reflects the restructure, not the file count.
- **Modules**: `vow-types/src/check.rs:1674-1737` (`check_integer_literal_range`) and `:1739-1801`
  (`check_contextual_integer_literal_ranges`).
- **Summary**: a pure `literal_range_targets(expr, target) -> Vec<(&Expr, Ty)>` that *enumerates* the
  (sub-expression, effective-target) pairs, leaving one flat `&mut self` loop to run
  `check_integer_value_range` over them.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: the two near-sibling recursive walkers **silently diverge**. The first descends blocks
  via `integer_marker_from_block` (`:1721`) and `if` via `integer_marker_from_block(then_branch)`
  (`:1730`); the second uses raw `block.trailing_expr` (`:1746`, `:1756`). The first handles
  `UnaryOp{Neg}` and `BinaryOp` (with the `Shl/Shr → U32` retarget at `:1713-1717`); the second
  handles `EnumConstruct` payloads (`:1784-1800`) and `Match` arms. So a literal inside
  `Some(if c { 1 } else { 2 })` takes a different path than one inside `if c { 1 } else { 2 }`.
  Well-pinned (`:5728`, `:5738`, `:5774`, `:5804`, `:5823`, `:6515`, `:6267`, `:6280`), but the
  refactor restructures two mutually-recursive `&mut self` walkers — high cost, and it needs a
  decision about which traversal is correct, which is a human's call.

## builtin-method-arity-check

- **Status**: proposed
- **Score**: n/a — **not a deepening candidate.** Recorded here so the finding survives; a future
  firing should not try to "deepen" it, and a human should decide whether to file it.
- **Files**: n/a (diagnosis only)
- **Modules**: `vow-types/src/check.rs` (`ExprKind::MethodCall` arm, builtin-method branch)
- **Summary**: the type checker does **not** enforce arity for builtin methods. `hay.contains()` and
  `v.truncate()` type-check with no argument and reach IR lowering with an empty argument list,
  where the lowerer synthesises a placeholder constant (`ConstUnit`, or `ConstI64(0)` for
  `truncate`). Reproduced directly against `./target/debug/vow build --dump-ir --no-verify` on
  2026-09-18:
  - `fn f(hay: String) -> bool { hay.contains() }` → `ConstUnit()` then
    `Call[extern:__vow_string_contains](%0, %1)`
  - `fn f(a: String, b: String) -> bool { a.eq() }` → same shape for `__vow_string_eq`
  - `v.truncate()` → `ConstI64[0]()` then `Call[extern:__vow_vec_truncate](%2, %5)`
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: surfaced while closing a coverage gap on PR #1299, which needed to know whether the
  lowerer's missing-argument fallbacks were reachable. They are. The behaviour is pre-existing and
  was preserved exactly by #1299 — pinned there by
  `tabled_builtin_methods_synthesise_their_missing_arguments` — so the tests now *document* it. For
  `v.truncate()` the zero fallback is plausibly intended ("truncate to 0"); for `contains()` and
  `eq()` passing unit into a call that reads a `String` looks like a genuine checker gap. Deciding
  which is which is a human's call, which is why this is recorded rather than filed.

## clif-shim-region-parity

- **Status**: dropped
- **Score**: n/a (not scored — excluded before ranking)
- **Files**: ~20+ estimated
- **Modules**: `vow-clif-shim/src/lib.rs`, `vow-codegen/src/cranelift_backend.rs`
- **Summary**: give the Rust `vow-codegen` backend parity with the shim's pure, tested
  `hidden_region_count` / `hidden_region_for_store_target` seams.
- **First seen**: 2026-08-31
- **Reason**: Too large to automate — blast radius 4, crosses a crate/tier seam and touches codegen
  output. A human should schedule it. Re-checked 2026-09-03: still large. Re-checked 2026-09-11: still
  large. Re-checked 2026-09-16: still large. Re-checked 2026-09-18: still large — but the sibling
  `hidden-region-store-targets` (19/25) is the tractable slice of the same idea and is now carded
  separately, so a human scheduling this can start there. Re-checked 2026-09-21: still large. Re-checked 2026-09-22:
  still large; under the #1330 rule `compiler/clif.vow` joins the edit set.

## esbmc-ce-description-heuristic

- **Status**: proposed
- **Score**: 16/25 (leverage 2, locality 3, blast radius 1, heat 4)
- **Files**: ~1 estimated (6 lines)
- **Modules**: `vow-verify/src/esbmc.rs:170-175` (`parse_esbmc_output` description branch)
- **Summary**: extract the counterexample-description heuristic into a pure
  `counterexample_description(output: &str) -> String`, total over one `&str`, leaving the
  `Counterexample` construction at the call site.
- **First seen**: 2026-08-31
- **Reason**: **Un-dropped 2026-09-18 — the drop reason was false.** It was excluded on 2026-08-31 as
  "inert: the only caller destructures `Failed(_)` and discards the description", and that reason was
  re-affirmed without re-verification on 2026-09-03, 09-11 and 09-16. The description is live on three
  surfaces: `esbmc.rs:907` wraps it into `VerificationResult::Failed`; `vow/src/verification.rs:190`
  clones it into `VerifyOutcome::Failed`, which `vow/src/report.rs` serializes as the public
  `counterexample` JSON field (asserted at `vow/src/main.rs:2893`); and `vow/src/cache.rs:237`
  persists it into the on-disk verify-failure record (round-trip assertion at `cache.rs:476`), making
  it part of a cache file format. Zero direct test coverage: all ten tests calling
  `parse_esbmc_output` assert `vow_id`/`values`/`block_visits`/`arith_overflow`, never `description`.
  Eligible but low-scoring — 6 lines, one production call site.
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## nondet-spec

- **Status**: proposed
- **Score**: 21/25 (leverage 4, locality 5, blast radius 2, heat 4)
- **Files**: ~5–6 estimated
- **Modules**: harness `vow-verify/src/esbmc.rs:109-127` / `compiler/verifier_harness.vow:14-37`;
  body suffix `vow-verify/src/c_emitter.rs:2094-2112` / `compiler/c_emitter.vow:664-679`; preamble
  `c_emitter.rs:2950-2971` (17 externs) / `c_emitter.vow:3440-3459` (13).
- **Summary**: one nondet table per compiler generating harness calls, body suffixes and preamble
  declarations.
- **First seen**: 2026-09-22
- **Report**: `.architecture/reviews/2026-09-22-narrow-int-width-self-hosted.md`
- **Reason**: drift — Rust harness `uchar`/`ushort`/`uint` vs long names everywhere else; Vow body
  falls back to `int` for i128/u128; Rust `parse_u32_opt` calls undeclared `__VERIFIER_nondet_ulong`
  (`c_emitter.rs:1677`); test suites pin opposite spellings (`esbmc.rs:1173-1195` vs
  `test_verifier.vow:8-26`). Behaviour-changing on Rust; **open question**: does ESBMC treat the
  SV-COMP short names specially? Test both spellings against the installed ESBMC before picking.

## bool-context-verdict

- **Status**: proposed
- **Score**: 21/25 (leverage 4, locality 4, blast radius 2, heat 5)
- **Files**: ~4–5 estimated
- **Modules**: `vow-types/src/check.rs:1474`, `:1513`, `:1550`, `:2162`, `:2170`, `:2229`, `:2685`;
  `compiler/checker.vow:806` (only check), `:2350-2352`, `:2493-2495`, `:2865` (no check).
- **Summary**: one `require_bool(context)` seam per compiler.
- **First seen**: 2026-09-22
- **Report**: `.architecture/reviews/2026-09-22-narrow-int-width-self-hosted.md`
- **Reason**: the self-hosted checker accepts `if 5 {}`, `1 && true`, `!3`, which Rust rejects. A
  correctness fix that makes the primary compiler reject more programs — `examples/` and
  `benchmarks/` (self-hosted only) were not swept. File as a fix. Shared gap: neither checks a
  `while` condition.

## cex-const-fold-self-hosted

- **Status**: proposed
- **Score**: 20/25 (leverage 3, locality 4, blast radius 1, heat 5)
- **Files**: ~3 estimated
- **Modules**: `compiler/main.vow:240-278`, `:288-334`; Rust seam `vow/src/cex_eval.rs:33-47`.
- **Summary**: a self-hosted `fold_binary_i64` shared by both `main.vow` folders.
- **First seen**: 2026-09-22
- **Report**: `.architecture/reviews/2026-09-22-narrow-int-width-self-hosted.md`
- **Reason**: both Vow copies fold checked `+!`/`-!`/`*!` as wrapping (#585 fixed Rust only); the
  first lacks the `IOP_CONST_U64` case. Behaviour-changing on the self-hosted side.

## violated-property-lines

- **Status**: proposed
- **Score**: 19/25 (leverage 3, locality 4, blast radius 1, heat 4)
- **Files**: ~3 estimated
- **Modules**: `vow-verify/src/esbmc.rs:196-212`, `:217-241`, `:297-313`, `:381-400`;
  `compiler/verifier.vow:634-653`, `:686-705`, `:750-772`, `:1109-1148`, `:1150-1180`.
- **Summary**: one section-scanner per compiler (`violated_property_lines`, `counterexample_lines`).
- **First seen**: 2026-09-22
- **Report**: `.architecture/reviews/2026-09-22-narrow-int-width-self-hosted.md`
- **Reason**: drift on malformed labels and multi-CE output; Vow `parse_block_visits` is write-only
  dead code that never strips ` (binary)`.

## ty-carries-region

- **Status**: proposed
- **Score**: 18/25 (leverage 3, locality 4, blast radius 1, heat 3)
- **Files**: ~3–4 estimated
- **Modules**: `vow-ir/src/region.rs:3650-3656` (8 names, 5 callers) + `:2689`;
  `compiler/region.vow:3285-3291` (12 names, 7 callers).
- **Summary**: `ty_carries_region` / `ity_carries_region` as a derived fact (`Ptr`/`LinearPtr`).
- **First seen**: 2026-09-22
- **Report**: `.architecture/reviews/2026-09-22-narrow-int-width-self-hosted.md`
- **Reason**: #995 (`f5d2a16e`) added i8/i16/u16/u32 to the Vow list only. The principled seam changes
  i128/u128 on both sides — a behaviour fork for a human.

## vec-elem-untagged-scalars

- **Status**: proposed
- **Score**: 20/25 (leverage 3, locality 4, blast radius 1, heat 5)
- **Files**: ~2–3 estimated
- **Modules**: `vow-ir/src/lower/mod.rs:841-855`, `:4817-4820`, `:4950-4953`, `:5191-5194`,
  `:3733-3738`; `compiler/lower.vow:1276-1278`, `:4950-4953`, `:5469-5473`, `:5753`, `:4052`.
- **Summary**: one "Vec element types not recorded" predicate per compiler.
- **First seen**: 2026-09-22
- **Report**: `.architecture/reviews/2026-09-22-narrow-int-width-self-hosted.md`
- **Reason**: four different list contents across eight sites; Rust push lowering filters to
  I128/U128 where Vow does not (PLAUSIBLE: `v.push(5)` on `Vec<u64>` lowers differently).
  Behaviour-changing; adjacent to `narrow-int-width-self-hosted` — land that first.

## branch-result-merge

- **Status**: proposed
- **Score**: 20/25 (leverage 3, locality 4, blast radius 1, heat 5)
- **Files**: ~2 estimated
- **Modules**: Rust `merge_result_ty` `vow-types/src/check.rs:480-494` (used `:2665`, `:2697`,
  `:2782`); Vow `merge_result_tid` `compiler/checker.vow:2283-2288` (loop/match only) and inline `if`
  merge `:2871-2890`.
- **Summary**: give the self-hosted checker Rust's `Never`/incompatible handling via one merge seam.
- **First seen**: 2026-09-22
- **Report**: `.architecture/reviews/2026-09-22-narrow-int-width-self-hosted.md`
- **Reason**: parity gap in the Vow checker only; behaviour-changing on the self-hosted side.

## test-status-failure-set

- **Status**: dropped
- **Score**: n/a — not a deepening
- **Files**: n/a
- **Modules**: `vow/src/test_runner.rs:388-393` vs `compiler/main.vow:2319-2328`.
- **Summary**: which `vowc test` statuses count as failures.
- **First seen**: 2026-09-22
- **Report**: `.architecture/reviews/2026-09-22-narrow-int-width-self-hosted.md`
- **Reason**: **bug, not a deepening.** Rust omits `"timeout"` (produced at `:38-44`), so a run whose
  only failure is a timeout exits 0; Vow counts it. Recorded for a human to file.

## esbmc-status-classify

- **Status**: dropped
- **Score**: ~15/25 (leverage 2, locality 3, blast radius 1, heat 4)
- **Files**: ~2 estimated
- **Modules**: `vow-verify/src/esbmc.rs:916`; `compiler/verifier.vow:535-543` + 5 `exit_code == -2`
  sites (`:924`, `:979`, `:1101`, `:1380`, `:1409`).
- **Summary**: timeout classification of ESBMC output.
- **First seen**: 2026-09-22
- **Report**: `.architecture/reviews/2026-09-22-narrow-int-width-self-hosted.md`
- **Reason**: **dropped — leverage 2.** Small drift fix (Rust misses "Timed out", Vow misses
  "TIMEOUT"), not a deepening.

## solver-classify-function

- **Status**: dropped
- **Score**: n/a
- **Files**: n/a
- **Modules**: `vow-verify/src/solver_strategy.rs` (`classify_function`)
- **Summary**: solver-strategy classification for a function.
- **First seen**: 2026-08-31
- **Reason**: Already a pure, unit-tested seam (`test_classify_*`). No shallowness to remove.
  Re-checked 2026-09-03. Re-checked 2026-09-11. Re-checked 2026-09-16. Re-checked 2026-09-18.
  Re-checked 2026-09-21. Re-checked 2026-09-22.
