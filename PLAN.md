# Plan: skip functions that store a collection value into a user-struct field (#1624)

## Goal
`vow verify stdlib/heap/main.vow` halts on `min_heap_new` because the ESBMC C model emits
`__vow_heap[v3 + 0] = v2;` where `v2` is a `__vow_vec_t` model struct and the heap array is
`int64_t[]`. Make `FieldSet` of a collection value non-modelable in both compilers so the function is
reported `VerificationSkipped` (a static fact, per #1561) instead of a parse error. Same mechanism as
#505/#572/#1538 (`collection_op_carries_non_scalar`) and the 128-bit field gate (`field_access_is_wide`).

## Assumptions
- Skip, not model: modelling a Vec/String/map/Option inside the `int64_t` heap would be a new model
  axis (representation of aggregate handles in slots) and would need a Vec-in-struct memory model. The issue
  explicitly allows classification as non-modelable; CLAUDE.md says unprovable != distort. (best guess)
- Gate on the *classified* structured vars (`vec/string/hashmap/btreemap/option` vars via
  `is_structured_value_id`), NOT on `ptr_vars`: `ConstStr` is `Ty::Ptr` and emits `v = 0`, so a struct
  holding a string literal is modelled today and must not regress. Unclassified `Ptr` values (e.g. a Vec
  parameter only stored into a field) are declared `int64_t`, so they parse; they are out of scope.
- `FieldGet` needs no change: a collection-typed `FieldGet` already goes through
  `typed_collection_kind` -> nondet len (c_emitter.rs ~2185), and a function containing such a FieldGet
  only breaks if it also does a FieldSet/other op that is gated.
- Skip-reason text: new `ModelIssue::CollectionField` -> opcode name `FieldSet of a collection value`
  (so the message reads "contains unsupported opcode `FieldSet of a collection value`"), analogous to
  the existing `Call extern `x` with non-scalar element` / `Call target with a collection argument`.
- Other stdlib/heap functions (`min_heap_push`, ...) building `MinHeap { data: v, .. }` hit the same rule
  and become skipped too; that is intended. If, after the fix, `vow verify stdlib/heap/main.vow` still
  shows a different ESBMC parse error for another function, record it in a follow-up issue, do not widen
  this PR.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `vow-verify/src/c_emitter.rs` | Rust gate: `ModelIssue` enum, `modelability_issue`, `first_unsupported_opcode`, `field_access_is_wide`; emission of `FieldSet` | 457-470 `is_structured_value_id`; 676-688 `field_access_is_wide`; 864-1011 `ModelFacts`/`ModelIssue`/FieldGet+FieldSet arm; 1100-1130 issue names; 2166-2176 FieldSet emit; 3919-4090 existing modelability tests |
| `compiler/c_emitter.vow` | Self-hosted mirror: `MCI_*` codes, `modelability_issue`, `model_issue_name`, `field_access_is_wide` | 498-505 `MCI_*`; 573 FieldGet/FieldSet arm; 645-680 `model_issue_name`; 1379-1389 `field_access_is_wide` |
| `compiler/tests/test_c_emitter_modelability.vow` | self-hosted unit test of `modelability_issue` | whole file (`collection_issue` helper at ~45) |
| `tests/verify-skip/*.vow` | end-to-end skip fixtures compared byte-for-byte across both compilers by `full_test.sh` Section 4d | pattern: `vec_param_push_skipped.vow` |
| `docs/spec/errors.md`, `docs/spec/cli.md` | ESBMC-backend skip-reason prose (list of example reasons) | errors.md ~1415, cli.md ~76 |
| `skills/vow/reference/{errors,cli}.md`, `vow/src/skill.rs`, `compiler/main.vow` | generated from spec by `scripts/generate_help.py` | only if spec prose changes |

## TDD slices (each slice: red -> green, both compilers where noted)

### Slice 0 — reproduce before coding
- `cargo build --release -p vow -j2`; run `VOW_VERIFY_DEBUG=1 target/release/vow verify --no-cache --verify-jobs 1 stdlib/heap/main.vow`;
  open `/tmp/vow-verify-debug/min_heap_new.c` and confirm `v2` comes from `__vow_vec_new` (so it is in `vec_vars`).
- Record which functions of `min_heap.vow`/`max_heap.vow` error, skip, verify or fail today. Skipping `min_heap_new`
  un-halts the run, so later functions may newly surface counterexamples (e.g. `.len()` ensures through the nondet
  `FieldGet` length) or another parse error.
- **Acceptance bar**: `min_heap_new` (and any sibling that stores a collection into a field) reports
  `VerificationSkipped`, no ESBMC parse error for it. Any newly exposed failure elsewhere goes to a follow-up issue,
  not this PR.

### Slice 1 — Rust unit test, then Rust gate
- **Red** (`vow-verify/src/c_emitter.rs`, `mod tests`, next to `modelability_issue_classifies_instruction_policy`
  ~3919; use `one_block_func_module`/`inst` helpers): build `RegionAlloc(Ptr, size 16)`, `Call
  CallExtern("__vow_vec_new")` (Ty::Ptr), `FieldSet(base, vec, idx 0)`, and assert
  `modelability_issue(fieldset) == Some(ModelIssue::CollectionField)`. Extra cases: value is a string var
  (`__vow_string_new`), an option var (`__vow_string_parse_i64_opt`), a phi-merged vec; negative cases:
  scalar `ConstI64` value -> `None`, `ConstStr` value -> `None` (guards the regression noted above).
  Also assert `non_modelable_reason` contains `unsupported opcode `FieldSet of a collection value``, and
  that `emit_c`-level output is not produced for such a function (reuse the pattern at ~4067-4080 / 7350).
- **Green**: add `ModelIssue::CollectionField`; add helper `field_set_stores_collection(inst, facts)`
  next to `field_access_is_wide` (FieldSet && `args[1]` in `is_structured_value_id(..)` over the 5 var sets);
  in the `Opcode::FieldGet | Opcode::FieldSet` arm of `modelability_issue` (line ~1010) return
  `CollectionField` after the `Wide` check; add the name arm in `first_unsupported_opcode` (~1103):
  `"FieldSet of a collection value"`. Reuses `is_structured_value_id` (c_emitter.rs:457).

### Slice 2 — self-hosted mirror (same session, must be byte-identical in behaviour)
- **Red** (`compiler/tests/test_c_emitter_modelability.vow`): mirror the Slice 1 cases with
  `mk_inst_*` builders (assert `modelability_issue(...) == MCI_COLLECTION_FIELD()`; scalar and ConstStr
  -> `MCI_OK()`), plus `model_issue_name` text. Run: `build/vowc test compiler/tests/test_c_emitter_modelability.vow`.
- **Green** (`compiler/c_emitter.vow`): add `fn MCI_COLLECTION_FIELD() -> i64 { 8 }` after `MCI_BAD_CALL`
  (the `Call (dk=..)` fallthrough in `model_issue_name` stays last); add `field_set_stores_collection` mirror;
  in the FieldGet/FieldSet arm (line ~573) return it after the wide check; add the `model_issue_name`
  branch with the identical string. Keep the argument order of the five var sets identical to
  `is_structured_value_id` usage at c_emitter.vow:~440-470.

### Slice 3 — end-to-end fixture + doc
- **Red**: `tests/verify-skip/vec_field_construct_skipped.vow` (`// TEST: category unverifiable`): a
  `struct Holder { data: Vec<i64>, size: u64 }` and a `new_holder() -> Holder vow { ensures: result.size == 0 }`
  that does `let v: Vec<i64> = Vec::new(); Holder { data: v, size: 0 }` (mirror of `min_heap_new`),
  plus a string-field variant if cheap. Before the fix both compilers emit an ESBMC parse error
  (needs ESBMC); after, they emit identical `VerificationSkipped`. Section 4d checks empty output +
  identical JSON across compilers; Section 2c (`parity.py c`) auto-skips verify-skip fixtures that never
  reach ESBMC.
- **Green**: nothing beyond slices 1-2. Also manually confirm
  `target/release/vow verify --no-cache --verify-jobs 1 stdlib/heap/main.vow` no longer reports `min_heap_new`
  as an ESBMC error, and the `build/vowc` run agrees (this fixture is already in full_test.sh's #1561
  skip-set determinism loop, ~line 1139, which gives free cross-compiler coverage).
- **Docs**: in `docs/spec/errors.md` (~1415) and `docs/spec/cli.md` (~76) add one phrase to the ESBMC example
  reason list: "a `FieldSet` that stores a `Vec`, `String`, map or `Option` into a struct field"; then
  `uv run python scripts/generate_help.py` and `python3 scripts/check_help_coverage.py`; commit regenerated
  `skills/vow/reference/*.md`, `vow/src/skill.rs`, `compiler/main.vow`. If generation produces unrelated
  churn, drop the prose change rather than bundling it.

## Verification surface
- No `vow` contract changes and no new properties for ESBMC to prove; the C model's emitted text for all
  currently-modelable functions must be unchanged (gate only removes functions from the modelable set).
- `scripts/full_test.sh` Section 2c parity over `tests/verify*/` fixtures, Section 4d verify-skip, and the
  #1561 skip-set loop (`stdlib/heap/main.vow`) are the surfaces; no `tests/run/` or `examples/` growth needed.
- Expect `docs/verifier-eval.md` to mention skip counts (touched by #1538); update only if a count/line
  there becomes wrong (grep `FieldSet`/skipped counts before finishing).

## Pre-checked facts (planning)
- `scripts/full_test.sh` #1561 loop (~1139-1190): `stdlib/heap/main.vow` has no status/skip-count arm (only
  `skip_after_failure` and `clamp_wrong_op` do), so it needs no edit; it only compares both compilers and job counts.
  Side effect: heap stops exercising a halted run; the other two fixtures still cover it.
- `docs/verifier-eval.md` has no heap-specific counts. `MCI_*` constants are used only in `compiler/c_emitter.vow`.
  `ModelIssue::` is used only in `vow-verify/src/c_emitter.rs`. `compiler/c_emitter.vow::is_modelable` loops over
  `modelability_issue` (implementer: confirm the loop body, ~line 600-620, rather than a hand-written opcode list).

## Risks
- **C parity (hard rule)**: Rust and Vow gates must produce the same modelable set and the same reason
  string; both enumerate the five var sets in the same helper order. Mitigated by mirrored tests + Section 4d.
- **Over-skipping**: gating on `ptr_vars` would skip struct-with-string-literal programs that verify today
  (`ConstStr` is `Ptr`). The plan gates on classified vars only; slice-1 negative test pins it.
- **Under-skipping**: a Vec param/`FieldGet` result only stored into a field is unclassified and stays
  modelable (declared `int64_t`, parses). Pre-existing limitation, same as #572's note; not widened here.
- **Fixed point / stage 0 vs seed**: change is in `c_emitter.vow` (stays in stage 0 until P6); bootstrap
  stage-1 verification output for compiler functions may flip some functions from ESBMC error to Skipped:
  run `scripts/bootstrap.sh --skip-cargo --no-cache` and record the final head SHA.
- **Clippy**: new enum variant must be handled in every `match ModelIssue` (exhaustive); run
  `cargo clippy --all --all-targets -- -D warnings`, `cargo fmt --all`.
- **Environment**: ESBMC/linked-runtime tests may SKIP-panic in sandbox (see memory); compare against clean main.

## Out of scope
- Modelling Vec/String/map/Option stored in struct fields (a real heap-slot model for aggregates).
- Gating unclassified `Ptr` values or `FieldGet` of collections; arg-layout/native verifier (`vc_*.vow`).
- Fixing any other ESBMC parse error that surfaces in `stdlib/heap` after this change (file a follow-up).
- Refactors, formatting, unrelated cleanups.

## Commands
`cargo test -p vow-verify c_emitter`, `build/vowc test compiler/tests/test_c_emitter_modelability.vow`,
`cargo clippy --all --all-targets -- -D warnings`, `cargo fmt --all`,
`scripts/bootstrap.sh --skip-cargo --no-cache`, then background-run `scripts/full_test.sh` (~40 min).
Commit: `fix(verify): skip functions that store a collection value into a struct field`.
