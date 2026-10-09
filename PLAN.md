# Plan: IR-type-keyed non-scalar guard for vec ops and map/btreemap keys+values (#572)

## Goal
Make the verifier's non-modelable guard fail closed for structured (`Ptr`-typed) values that `collect_typed_vars` never classifies — function params, untyped `get_val` results — and extend it to `__vow_map_*` / `__vow_btreemap_*` keys and values, so such functions are `Skipped` instead of mis-modelled or `VerifyFailed` (ESBMC PARSING ERROR). Rust (`vow-verify`) and self-hosted (`compiler/`) change in lockstep with byte-identical C.

## Assumptions
- **Over-broad by design (best guess, matches issue's "key off IR type")**: `Ty::Ptr` also covers user-struct pointers (modelled as `int64_t` heap slot indices) and `ConstStr` (`int64_t`). Storing those in a vec/map slot is well-typed C today, but the IR cannot distinguish them from an unclassified Vec/String param. Rule: any `Ptr`/`LinearPtr`-typed value or key operand of a vec store/load or map/btreemap op makes the function non-modelable. Consequence: functions doing `Vec<UserStruct>` push/get or `HashMap<_, UserStruct>`/`HashMap<String,_>` may flip Proven -> Skipped. Accepted: a false Proven is worse than a Skip. Mitigated by a measurement gate (Slice 7). `ConstStr`-keyed ops are likewise skipped.
- The reason text stays `non-scalar element` (JSON is diffed between compilers); map ops reuse `ModelIssue::NonScalarExtern`.
- `passes_structured_arg` (user-fn call args) is NOT switched to a Ptr check — it would skip every call passing a user struct. Follow-up only.
- Gap 3's read-only case (`HashMap<i64,String>` param, only `m.get(k)`, payload used as String) is probed by a repro slice first; fix only if the probe shows ill-typed/bogus C.

## Key Files
| File | Role | Lines |
|------|------|-------|
| `vow-verify/src/c_emitter.rs` | `is_structured_value_id` (414), `vec_op_carries_non_scalar` (455), `collect_typed_vars` (493), `collect_wide_vars` (612, template for new set), `ModelFacts` (794-810), `model_arg_index`/`ModelArgRole` (336-408), `modelability_issue` Call arm (900-935), map emit (1883-1990), unit tests (~7069-7270) | |
| `compiler/c_emitter.vow` | mirror: `model_arg_index` (264), `is_structured_value_id` (329), `vec_op_carries_non_scalar` (355), `ModelFacts`/`model_facts` (370-391), `modelability_issue` (404+), `collect_wide_vars` (1225) | |
| `compiler/tests/test_c_emitter_modelability.vow` | self-hosted unit tests for `modelability_issue` | |
| `compiler/tests/builders.vow` | `mk_inst_*`, `ir_inst_new`, `mk_function` helpers | |
| `tests/verify-skip/` | fixtures asserted `Skipped` with Rust/self JSON parity (full_test.sh 4d, l.1274); style: `vec_of_string_skipped.vow` | |
| `docs/spec/contracts.md` / `cli.md` | only if a skip-reason/behavior sentence exists for nested collections (grep "non-scalar", "nested"); update there | |

## Steps

### 1. Reproduce first (no production change)
Build Rust `vow` (`cargo build --release -p vow -j2`, `scripts/bootstrap.sh --skip-cargo` for `build/vowc`). Write the five fixtures from Slice 2-5 in `$TMPDIR`, run `vow verify` / `VOW_VERIFY_DEBUG=1` and record the actual status and C for each (confirms whether gap 1/2 yield bogus-but-well-typed C and gap 3 yields the PARSING ERROR). Paste findings into the PR body.

### 2. Add a `ptr_vars` fact set (both compilers)
- `vow-verify/src/c_emitter.rs`: add `fn collect_ptr_vars(func) -> HashSet<u32>` modelled on `collect_wide_vars` (612): ids of insts with `inst.ty` in `{Ptr, LinearPtr}`. Add `ptr_vars` to `ModelFacts` (794).
- `compiler/c_emitter.vow`: `collect_ptr_vars(f) -> Vec<i64>` modelled on `collect_wide_vars` (1225); add `ptr_vars` field to `ModelFacts` + `model_facts`. Compare via `vec_contains_i64`.
- No `inst_by_id` lookup in `modelability_issue`.

### 3. Rewrite `vec_op_carries_non_scalar` to use IR type
- Keep `is_structured_value_id` (still used by `passes_structured_arg`). New predicate `is_non_scalar_operand(id, facts)` = `is_structured_value_id(...) || ptr_vars.contains(id)` (classified-but-not-Ptr-typed ids stay covered, e.g. option vars).
- `__vow_vec_get_val`: check `inst.id`/`inst.ty` (Ptr) as well as classification. `push_val`/`push_val_in_arena`/`set_val`: check value operand via `ModelArgRole::VecValue`.
- Signature: pass `&ModelFacts` instead of the 5 sets (shrinks the call sites at c_emitter.rs:904 and .vow:436); update `passes_structured_arg` callers only if trivially shared — otherwise leave untouched.

### 4. Add map/btreemap key and value roles
- Rust `ModelArgRole`: add `MapKey`, `MapValue`; `model_arg_index` returns `offset+1` / `offset+2` for `__vow_map_insert[_in_arena]`, `offset+1` for `__vow_map_get[_in_arena]`/`contains`/`remove[_in_arena]` (key only). Mirror in `.vow` (`CARG_MAP_KEY()`, `CARG_MAP_VALUE()`). Update the `model_arg_index_covers_collection_roles_and_arena_shift` test (3362).
- `__vow_btreemap_*` is not covered by `model_arg_index` (prefix check excludes it) and has no arena variants: add a small `btreemap_operand_index(name, role)` helper in both compilers: insert -> key 1, val 2; get/contains -> key 1.
- Extend `vec_op_carries_non_scalar` (rename `collection_op_carries_non_scalar`; keep `NonScalarExtern` + reason text) to check these operands against `is_non_scalar_operand`.

### 5. Probe/handle map read path (gap 3, read-only)
If Step 1 shows `HashMap<i64,String>` param + `m.get(k).unwrap().len()` produces ill-typed/bogus C: additionally flag `__vow_map_get[_in_arena]`/`__vow_btreemap_get` whose option payload feeds a string/vec receiver (reuse the option-payload extraction site, c_emitter.rs ~2106). Otherwise record "not reproducible" in the PR and skip.

### 6. Spec
Grep `docs/spec/*.md` for the nested-collection skip sentence; widen it to "collections/handles stored in a Vec/HashMap/BTreeMap (keys or values)". No `--help`/skill regeneration needed unless the sentence lives in `grammar.md` embedded text; if so run `uv run python scripts/generate_help.py`.

### 7. Coverage-impact gate (before PR)
Count `Skipped` vs `Verified` over `tests/verify/*.vow tests/verify-stress/*.vow` and `uv run --project bench bench/run.py validate-references` (36 non-Stretch must stay passing) before and after; run bootstrap verification of `compiler/` (`scripts/bootstrap.sh --skip-cargo --no-cache`, record head SHA). Any flip in `tests/verify/` or a benchmark reference is a blocker: re-scope the Ptr rule (e.g. exempt ids defined by `RegionAlloc`/`ConstStr`, which are provably user-struct/literal handles) rather than editing the fixture.

## TDD slices (red -> green, each its own commit)
1. **Param value push** — Rust unit test `param_ptr_vec_push_marked_non_modelable` (pattern of `nested_vec_push_marked_non_modelable`, c_emitter.rs ~7069): `GetArg(Ptr)` x2, `vec_push_val(arg0, arg1)`, expects reason contains `non-scalar element`. Self-hosted twin in `compiler/tests/test_c_emitter_modelability.vow` (assert `MCI_NONSCALAR_EXTERN()`). Fixture `tests/verify-skip/vec_param_push_skipped.vow` (issue's `add_row`, with `ensures: result >= 0` on a length). Green: Steps 2-3.
2. **Untyped get_val** — unit tests (both) for a `Ptr`-typed `get_val` result that is `Return`ed (no typed receiver use) and one passed through `FieldGet`; fixture `vec_get_val_returned_skipped.vow` (`fn first_row(g: Vec<Vec<i64>>) -> Vec<i64>` with contract). Green: Step 3.
3. **Negative control** — unit test that `Vec<i64>` push/get_val (I64 operands) stays modelable; `tests/verify` Vec fixtures unchanged.
4. **Map value** — tests for `__vow_map_insert` and `_in_arena` with Ptr value, and `__vow_btreemap_insert`/`get`; fixtures `hashmap_string_value_skipped.vow` (`HashMap<i64,String>`, the exact #505 symptom), `btreemap_string_value_skipped.vow`. Update `model_arg_index` test. Green: Step 4.
5. **Map key** — `HashMap<String,i64>` insert/get/contains/remove with Ptr key; fixture `hashmap_string_key_skipped.vow`.
6. **Read-only map probe** (Step 5) — fixture only if it reproduces; else document.
7. **Parity** — run `python3 scripts/parity.py c RUST_BIN SELF_BIN tests/verify/*.vow tests/verify-skip/*.vow` (skipped functions emit no C, so parity is mostly "same decision"; the verify-skip JSON diff in full_test.sh 4d covers it).
Refactor commit last (if any): collapse the five-set argument lists into `&ModelFacts` for `vec_op_carries_non_scalar` only.

## Verification surface
- No contract changes; no new ESBMC properties. The change only widens the set of functions never sent to ESBMC (fail-closed). New `tests/verify-skip/*.vow` fixtures each have a real `ensures` so the function is a verification target and must report `Skipped`.
- `ty` parity: the gate now depends on `inst.ty`. Confirm `compiler/lower.vow` assigns `ITY_PTR` to the same values as `vow-ir/src/lower/` for: `GetArg` of collection params, `__vow_vec_get_val` results, map/btreemap insert/get operands, `ConstStr`. Add a self-hosted test if a mismatch is found (a mismatch = one compiler skips, the other emits C).
- Run: `cargo test -p vow-verify`, `cargo clippy --all --all-targets -- -D warnings`, `cargo fmt --all`, `build/vowc test compiler/tests/test_c_emitter_modelability.vow` (and `compiler/`), `scripts/bootstrap.sh --skip-cargo --no-cache` (record head SHA + seed pin), `scripts/full_test.sh` Sections 2c, 4d (background, ~40 min), each as separate commands.

## Risks
- **Coverage regression** (Vec<UserStruct>, map values/keys as structs/strings): handled by Step 7 gate; Ptr rule may need narrowing (exempt `RegionAlloc`/`ConstStr` defs) — decide from measurements, not preemptively.
- **C parity** (`c_emitter.rs` vs `.vow`): decision-only change (no C text changes) but `scripts/full_test.sh` 2c + 4d must pass; `non_modelable_reason` message text must be identical.
- **Binary fixed point**: new `.vow` code uses `Vec<i64>` + `vec_contains_i64` only (no HashMap) -> deterministic; no codegen-order or clif-shim change.
- **Self-hosted compiler verifies itself**: new helpers in `compiler/c_emitter.vow` must verify/skip cleanly in bootstrap; keep functions small, no weakened contracts.
- **`btreemap` outside `model_arg_index`**: mirrored helper needed in both compilers; easy to forget the `.vow` side.
- **clippy**: `too_many_arguments` on `vec_op_carries_non_scalar` is avoided by passing `&ModelFacts`.
- `bootstrap.yml` doesn't run on PRs: re-run bootstrap on the final head SHA before ticking the checklist.

## Out of scope
- `passes_structured_arg` / user-fn call arguments (possible follow-up).
- Modelling nested collections or struct elements precisely (making them verifiable rather than skipped).
- Changing C emitter text, `collect_typed_vars` propagation, or lowering/IR type tagging.
- Refactors, formatting, unrelated cleanups; no contract edits.
