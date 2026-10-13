# Plan: reject user-fn calls whose result is a collection in the ESBMC C model (#1631)

## Goal
Stop `vow-verify/src/c_emitter.rs` and `compiler/c_emitter.vow` from emitting uncompilable C (`__vow_string_t vN = int64_t-returning vow_user_fn_M(...)`). The functions that hit it become `Skipped` (fail-closed) via `modelability_issue`, in both compilers with byte-identical behaviour. Same shape as #505 (`vec_of_string_skipped`) and #1089 (`collection_arg_to_user_fn_skipped`).

## Root cause (verified by reading the emitter)
- A modelable user function is emitted as `int64_t vow_user_fn_N(...)` for any `Ptr`/`LinearPtr` return (`c_emitter.rs:2486-2490`, mirror at `:3402`), because a collection cannot cross a function boundary. `Return` of a structured value emits `return 0; /* modelled type return */` (`c_emitter.rs:1476-1489`).
- In the caller, a call result is classified `String`/`Vec`/map/`Option` purely by usage: `collect_typed_vars` marks it when it is a receiver or second-string operand of a `__vow_string_*`/`__vow_vec_*` builtin, e.g. `arena_str(..) == name` → `__vow_string_eq`, and the marking is propagated through Phi/Upsilon. The declaration is then `__vow_string_t vN;` (`:2584`), and the call is emitted as `vN = vow_user_fn_M(...)` (`:2111-2135`): `__vow_string_t = int64_t`, a C compile error.
- `modelability_issue`'s `CallTarget` arm (`:986-1008`) rejects structured *arguments* only (`passes_structured_arg`, `StructuredTarget`); it never looks at a structured *result*.

## Decision
Take the "reject" option from the issue: a `CallTarget` whose own result id is a structured value (`is_structured_value_id(inst.id.0, …)`) is non-modelable. The typing option (emit a `__vow_string_t` return) is declined: aggregates already cross user-fn boundaries as `0`/nondet (args are rejected for the same reason), so it would need a collection-by-value ABI for every callee and caller plus a new soundness argument. It is also a larger, riskier change to two emitters that must stay byte-identical until epic #1398 P6 deletes them. Rejecting is sound, small, and the established precedent.

## Assumptions
- Scope is call-site result classification only (best guess). A callee that returns a collection but whose caller never uses the result structurally stays modelable; that case compiles today (int64 `0`) and is not what #1631 reports. Noted under Out of scope.
- New diagnostic text: `Call target with a collection result` (best guess, parallel to `Call target with a collection argument`).

## Key Files
| File | Role | Lines |
|------|------|-------|
| `vow-verify/src/c_emitter.rs` | add `ModelIssue::StructuredResult`; check in `CallTarget` arm; name in `first_unsupported_opcode`; unit tests | enum `:889-898`, arm `:986-1008`, reason `:1108-1110`, tests `:3919-4052` |
| `compiler/c_emitter.vow` | add `MCI_STRUCTURED_RESULT()`; mirror check; name in `model_issue_name` | consts `:503-510`, arm `:548-557`, names `:660-662` |
| `compiler/tests/test_c_emitter_modelability.vow` | mirror unit test of the new issue code | helper `collection_issue` `:38-62` |
| `tests/verify-skip/collection_result_from_user_fn_skipped.vow` [new] | end-to-end fixture, both compilers, `Skipped` | model on `tests/verify-skip/vec_of_string_skipped.vow` |
| `docs/spec/errors.md` | list the new skip reason text | `:1415` |

## Steps (TDD slices)

### 1. Red: Rust unit test for the issue code
- **File**: `vow-verify/src/c_emitter.rs` test module near `modelability_issue_classifies_instruction_policy` (`:3919`).
- **Test**: build a module with a callee `FuncId(1)` (modelable: `ConstI64` + `Return`, `return_ty: Ty::Ptr`) and a caller: `c0 = ConstI64`; `s = Call CallTarget(1)` with `ty: Ty::Ptr`; `Call CallExtern("__vow_string_eq")` with args `[s, s]` (marks `s` a string var as receiver and second operand); `Return`. Assert `modelability_issue(&call_inst, …) == Some(ModelIssue::StructuredResult)`, and that the same call with a result used only as `i64` (e.g. `Add`) is `None` (guards against over-rejection).
- **Also**: extend `non_modelable_reason_names_each_issue` (`:3973`) with a case asserting `Call target with a collection result`.
- **Green**: step 2.

### 2. Green (Rust): implement the check
- **File**: `vow-verify/src/c_emitter.rs`.
- **Change**: add `StructuredResult` to `ModelIssue`. In the `CallTarget` arm, after the `passes_structured_arg` branch and before the const-fn/modelable check, add a branch: `is_structured_value_id(inst.id.0, &facts.vec_vars, &facts.string_vars, &facts.hashmap_vars, &facts.btreemap_vars, &facts.option_vars)` → `Some(ModelIssue::StructuredResult)`. Const-fn targets return scalars, so the order does not matter for them. Add the `first_unsupported_opcode` arm → `"Call target with a collection result".to_string()`.
- **Reuses**: `is_structured_value_id` (`:458`), `ModelFacts` (`:870-885`).
- **Refactor**: fold the two structured checks into one doc-commented helper only if it stays shorter than two copies; otherwise leave inline.

### 3. Red→Green (self-hosted): mirror unit test then code
- **File**: `compiler/tests/test_c_emitter_modelability.vow` — add a helper beside `collection_issue` that builds callee (id 1, `ITY_PTR` return, `ConstI64`+`Return`) + caller (const, `mk_inst_call_target`-style call, `__vow_string_eq` extern call over the call result), and assert `modelability_issue(...) == MCI_STRUCTURED_RESULT()` plus the scalar-use control `== MCI_OK()`. Builders exist in `compiler/tests/builders.vow` (`mk_inst_call_target` `:25`, `mk_inst_call_extern` `:21`, plus `mk_inst_arg`, `mk_function`, `mk_module`); no new builder needed.
- **File**: `compiler/c_emitter.vow` — add `fn MCI_STRUCTURED_RESULT() -> i64 { 8 }` after `MCI_BAD_CALL`; in the `IDATA_CALL_TARGET` arm after the `passes_structured_arg` branch add `if is_structured_value_id(inst.id, facts.vec_vars, facts.string_vars, facts.hashmap_vars, facts.btreemap_vars, facts.option_vars) { return MCI_STRUCTURED_RESULT(); }`; in `model_issue_name` add the matching `String::from("Call target with a collection result")`. Text must equal the Rust string exactly (parity of skip reasons in JSON).
- Run: `build/vowc test compiler/ --filter test_c_emitter_modelability`.

### 4. Fixture: end-to-end skip (both compilers)
- **File**: `tests/verify-skip/collection_result_from_user_fn_skipped.vow` [new], `// TEST: category unverifiable`, header comment citing #1631.
- **Content**: shape the repro like the issue: a callee taking a user struct (heap handle, `int64_t`) and returning the `String` read from its `Vec<String>` field (`struct Arena { strs: Vec<String> }`, `fn arena_str(a: Arena, sid: i64) -> String { a.strs[sid as u64] }`), and a contracted caller `fn find(a: Arena, name: String, sid: i64) -> i64 vow { ensures: result == sid || result == -1 } { if arena_str(a, sid) == name { sid } else { -1 } }`. Careful: the `name: String` param passed to `==` is fine (not passed to the user fn). Confirm with `VOW_VERIFY_DEBUG=1` on the Rust compiler that, pre-fix, the C has the `int64_t`/`__vow_string_t` mismatch, and that `find` is `Skipped` post-fix; adjust the shape if the caller turns out non-modelable for another reason pre-fix (the issue notes this masks the bug).
- Harness: `scripts/full_test.sh` Section 4d (`:1390-1415`) compares both compilers' JSON and asserts Rust `status == "Skipped"` (`:1413`), so the fixture is red pre-fix (ToolError/VerifyFailed) and green post-fix; `scripts/verify_eval.py` maps the `verify-skip/` dir to `Skipped` and needs no extra directive (only `// TEST: category unverifiable`, like all 15 existing fixtures). `IrInst.id` is the field the Vow mirror reads (`compiler/ir.vow:226`). Section 2c parity does not reach ESBMC for skipped fixtures (`:896-899`). Also run `tests/verify` fixtures through `python3 scripts/parity.py c …` to prove byte-identical C is unchanged for modelable programs.

### 5. Spec
- **File**: `docs/spec/errors.md:1415`: extend the ESBMC-backend list: "a call that passes a `Vec`, `String`, map or `Option` argument to a user function (`Call target with a collection argument`) or yields one as its result (`Call target with a collection result`)". `cli.md:76` already says "A call with a … collection argument or result stays `Skipped`" (native verifier) — no change. No grammar/CLI/help change, so no `generate_help.py` run.

## Testing / Verification surface
- `cargo test -p vow-verify` (new unit tests), `cargo clippy --all --all-targets -- -D warnings`, `cargo fmt --all`.
- `build/vowc test compiler/ --filter test_c_emitter` (needs `scripts/bootstrap.sh --skip-cargo` first; ~5 min, run in the foreground with an explicit timeout or poll; see memory notes on wall-clock).
- Fixtures: `tests/verify-skip/` new file via `scripts/full_test.sh` sections 2c/4d (full run ~40 min; run just the new fixture through both compilers' `verify --json` and diff, plus `python3 scripts/parity.py c RUST_BIN SELF_BIN tests/verify/*.vow tests/verify-skip/*.vow`).
- ESBMC properties: none new. The change removes functions from the modeled set rather than adding properties, so no proof obligation grows. Check no existing `tests/verify*/` fixture flips from verified/failed to `Skipped` (grep expected results before landing); if one does, it was previously emitting ill-typed C and its expectation was wrong.
- `scripts/check_help_coverage.py`/`generate_operations.py --check`: unaffected (no builtin/catalogue change).

## Risks
- **C parity drift**: the issue's only hard invariant. Mitigation: both edits in the same commit, identical order of checks (after structured-arg, before const-fn/target recursion), identical diagnostic text, parity script over all `tests/verify*/` fixtures.
- **Over-rejection**: a function that receives a modelable scalar result must stay modelable. Mitigation: `is_structured_value_id` is true only for ids classified by usage as a collection; unit-test control case; compare `non_modelable_reason` counts on `compiler/` before/after (`VOW_VERIFY_DEBUG`/`verify --json` skipped counts) and justify any increase.
- **Behaviour change in the self-hosted compiler's own verify run** (`scripts/bootstrap.sh` verifies `compiler/`): functions that today end `VerifyFailed` (unexpected esbmc output) become `Skipped`; bootstrap fixed point is codegen-only so is unaffected, but run `scripts/bootstrap.sh --skip-cargo --no-cache` on the final head SHA and record it, per CLAUDE.md.
- **Recursion cache order**: placing the new check before `is_modelable(callee)` avoids a needless recursive scan and cannot poison `cache` (it returns before touching it). In Vow keep it before the `fns` loop.
- No codegen, clif-shim, lexer/parser/printer, or `BTreeMap`/`HashMap` ordering touched, so binary fixed point and `parse → print → parse` are unaffected.

## Out of scope
- Typing collection returns correctly in the C model (the issue's other option) and removing the `return 0` aggregate model.
- Rejecting a modelable callee whose *return value* is a collection when no caller uses it structurally.
- Reverting #1580's `replay_item_ids` workaround (separate cleanup once this lands, ideally with `arena_str` callers re-checked).
- Native verifier (`vc_*.vow`), operations catalogue, formatting or unrelated refactors.
