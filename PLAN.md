# Plan: model `Vec` as an SMT array plus symbolic length in the native verifier (#1423)

## Goal
Make the native verifier (`vowc verify --backend native`, `compiler/vc_*.vow`) model `Vec<T>` (`T` = Bool or integer up to 64 bits) as `(Array (_ BitVec 64) (_ BitVec 64))` plus a 64-bit symbolic length, with `new`, `push`, index-get, index-set, `len` and `pop`, no capacity cap anywhere, and an out-of-bounds index as a hard claim. This implements ADR-2026-10-08-1422 Rule 3 for `Vec` element access and growth. Epic #1398; ADR D5-D7 fixed the semantics.

## Scope facts established by reconnaissance
- Native verifier lives only in `compiler/` (CLAUDE.md "Scoped exception"): **no Rust twin, no `c_emitter` change, no parity-C impact**. `vow-verify/src/c_emitter.rs` and `compiler/c_emitter.vow` stay untouched, so `scripts/parity.py c` is unaffected. The one Rust file that may change is generated: `vow/src/skill.rs` via `scripts/generate_help.py` (Step 8).
- Today every `__vow_vec_*` call is `Skipped: unmodeled-builtin`: `catalogue_verifier_known` (`compiler/vc_ops.vow:175`) is `return false`, and `compiler/tests/test_vc_ops.vow:57` asserts `__vow_vec_len` is *not* known. `docs/spec/operations.json` has no `__vow_vec_*` entries and its entries are keyed by Vow builtin name with a mandatory `grammar.md` row plus doc lines in `main.vow`/`skill.rs` (`scripts/generate_operations.py:633-661`), so Vec methods do not belong there (ADR-1430 "Prerequisite for unmodeled-builtin" anticipated a backfill by runtime symbol).
- IR shape (`compiler/lower.vow:3829,3937,4141,4359,4648,4695`): `Vec::new()` -> `CALL extern __vow_vec_new` (Ptr); `v.push(x)` -> `__vow_vec_push_val(v, x)` (Unit); `v[i]` -> `__vow_vec_get_val(v, i)` typed `I64`, then an explicit narrowing (`lower_narrow_literal`); `v[i] = x` -> `__vow_vec_set_val(v, i, x)`; `v.len()` -> `__vow_vec_len(v)` typed `U64`; `pop` -> `__vow_vec_pop`. The ESBMC emitter also knows `_in_arena` variants (arena arg first). Wide elements use `__vow_vec_{get,set,push}_wide_ptr` + `WideSlot` (stay `Skipped`).
- The runtime stores every `_val` element in an 8-byte slot (`vow-runtime/src/lib.rs:1722-1725`); the Cranelift shim widens a narrower extern argument by the IR type's signedness (`vow-clif-shim/src/lib.rs:227-234`: signed -> `sextend`, unsigned and Bool -> `uextend`). The array model mirrors exactly that.
- Runtime growth path (`vow-runtime/src/lib.rs:1597-1600`): `push` computes `v.len.checked_add(1)` and on `None` calls `oom_trap("Vec::reserve")`; further `oom_trap`s fire when doubling `new_cap` reaches `VOW_CAP_VALUE_MASK` or the byte size overflows. So a push at `len == u64::MAX` is a trap (an abort, never a returning run) in the runtime itself.
- ESBMC's model for comparison (`vow-verify/src/c_emitter.rs:1561-1650`): `push` asserts `len < 128` ("vec capacity"), `get`/`set` assert "vec bounds", Vec param length is `nondet` pruned by `__ESBMC_assume(len <= CAP)`, `pop` is `if (len > 0) len--`.
- The executor (`compiler/vc_exec.vow`) walks the unrolled acyclic CFG in RPO; pointers are objects (`vc_agg`), never terms; `vc_agg`'s soundness argument "every write precedes every read" does **not** hold for Vec (push/set happen in many blocks), so Vec state needs its own flow-sensitive store.
- Differential harness facts (`scripts/verify_diff.py:75-88`, `scripts/verify_eval.py:124-180`): the comparison is **per file** (`verdict_of` reads the file-level `status`), so one `Skipped` function makes the whole fixture `Skipped`. `known-soundness-gap` is valid only under `tests/verify/` and means "corpus says correct, a backend false-accepts". There is no baseline file to re-record.
- Fixture census (grep of `Vec` in `tests/verify*`): the pure-Vec fixtures that flip to native-provable are `tests/verify/{vec_fill,bounds_correct}.vow` and `tests/verify-fail/{off_by_one_bounds,vec_overcount,replay_unattributed_bounds}.vow`, all `match` ESBMC. `tests/verify/model_capacity_bound_note.vow` and `raw_parts_copy_*` mix in `String`/`HashMap`/`from_raw_parts_copy` and stay file-level `Skipped` (`weaker` vs ESBMC, as today). No existing fixture is a cap-only artifact once `from_raw_parts_copy` is out of scope.

## Assumptions
- **Elements are one 64-bit sort.** Matches the runtime's 8-byte `_val` slots and ESBMC's `int64_t data[]`. One array sort. Bool element = `ite(b, 1, 0)` on push; reads come back as `I64` and the IR's own cast narrows. (best guess)
- **`Vec` parameters are distinct objects** (no aliasing between two parameters), as ESBMC and `vc_agg` param objects already assume. Recorded in the ADR addendum. (best guess)
- **`push` assumes `len != 0xFFFF_FFFF_FFFF_FFFF` on its path, and nothing else about length.** It is exactly the `checked_add` overflow branch of the runtime above, i.e. the path on which the runtime traps: the model treats "the runtime aborts" as an infeasible continuation. Without it a `Vec` parameter at `u64::MAX` wraps to 0 and yields a spurious `failed` (e.g. `fn f(v: Vec<i64>) vow { ensures: v.len() > 0 } { v.push(1) }`). Deliberately *not* modelled: the later `oom_trap`s (`VOW_CAP_VALUE_MASK`, byte-size overflow); encoding those would be a length cap. Alternatives rejected: a `len < 2^61` representation invariant (a cap-shaped constant); a soft claim (spurious `ArithOverflowReachable` on every push). Posted to the issue for reviewer override. (best guess)
- **`from_raw_parts_copy`, `pin_to_root`, `clear`, `truncate`, `reserve`, `vec_sort`, `Vec<i128>`/`Vec<u128>`, `Vec<Vec<_>>`, `Vec<String>`, Vec in struct fields, a Vec passed to a user call, and a Vec merged by a Phi all stay `Skipped`.** The issue lists push, indexing, set and length; `from_raw_parts_copy` is excluded because natively proving the `*_beyond_model_cap` fixtures would put them in `verify_diff` class `soundness` (native proves what the corpus labels failing only because of ESBMC's cap) and that needs a harness decision of its own. ESBMC does not model `clear`/`truncate` either. (best guess)
- **Vec ops are hand-listed in `vc_ops.vow`, not catalogued**, like `__vow_unwrap_panic` (`vc_ops.vow:97`). (best guess; ADR addendum)
- **Criterion 3 ("capacity-only verdicts explained") is met per function, in the ADR addendum table and in new `tests/verify-native/pass/` fixtures** (a property true for every length, with a comment that ESBMC proved it only up to its cap), because the file-level harness cannot show per-function movement in mixed files. (best guess)

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `compiler/vc_vec.vow` [new] | Deep module: Vec objects, flow-sensitive `(arr, len)` state, block-entry merges, op semantics, gate shape rule | - |
| `scripts/concat_vow.sh` | module list: insert `vc_vec` after `vc_agg`, before `vc_gate` (the gate calls it; `vc_vec` uses `vc_ops vc_term vc_smt vc_int vc_flow vc_agg` only, never `vc_gate`) | :23 |
| `compiler/vc_term.vow` | add `VC_OP_SELECT`, `VC_OP_STORE`, array sort code | `VC_SORT_BOOL` :8, ops :17-50 |
| `compiler/vc_smt.vow` | `select`/`store` symbols; array sort in `vc_sort_text` | `vc_op_symbol` :96, `vc_sort_text` :133 |
| `compiler/vc_ops.vow` | Vec symbol table `vc_vec_op_kind`; frame-in-`dv2` predicate for Vec get/set; `vc_frame_of` | `vc_is_unwrap_abort` :99, `vc_frame_of` :140, `vc_op_frame_marked` :159, `catalogue_verifier_known` :175 |
| `compiler/vc_inline.vow` | stamp the frame into `dv2` of Vec get/set calls when splicing | `vc_map_inst` :246-266 (the `vc_is_unwrap_abort` branch at :262) |
| `compiler/vc_gate.vow` | accept Vec calls with typed operands; call `vc_vec_shape_detail` | `vc_inst_supported` :201, `vc_inst_specific_reason` :271, `vc_skip_reason_with` :311 |
| `compiler/vc_exec.vow` | execute Vec calls, new claim kind/label, per-block state hook, logic selection | `VC_CLAIM_*` :50, `vc_walk_inst` :347, `vc_claims_mode` :394 |
| `compiler/vc_agg.vow` | object ids reused for Vec objects (`vc_agg_alloc`, `vc_agg_param`) | :75-89 |
| `compiler/vc_flow.vow` | edges (`in_head`/`e_src`/`e_cond`) drive the merges; `vc_flow_name` | :73-198 |
| `compiler/vc_cex.vow` | unattributed claim -> "Violated property" label path (no change expected) | :46-70 |
| `compiler/tests/test_vc_vec.vow` [new], `test_vc_smt.vow`, `test_vc_ops.vow`, `test_vc_gate.vow`, `test_vc_exec.vow`, `test_vc_inline.vow` | unit tests | `test_vc_ops.vow:57` flips |
| `tests/verify-native/{pass,fail,unknown,skip}/*.vow` [new fixtures] | end-to-end native fixtures | harness `scripts/full_test.sh:1421-1480` |
| `tests/verify-native/tests.sh` | fake-bitwuzla wiring test; add the structural "no capacity assumption" check | recorder :30-33 |
| `docs/spec/cli.md` | "Native backend" section: Subset (:62), Aggregates (:71), new Vec bullet | :58-76 |
| `compiler/main.vow`, `vow/src/skill.rs` | **generated** from the spec by `scripts/generate_help.py`; commit the regenerated diff | - |
| `docs/adr/2026-10-08-1422-native-verification-semantics.md` | `## Addendum (#1423)`; "Known limits" bullet on `Vec<i128>` | after Addendum #1416 |
| `docs/spec/contracts.md`, `docs/spec/errors.md` | **no change**: they describe the ESBMC pipeline, which is still the default (ADR "Contracts.md rewrite plan") | - |

## Design

### Representation (`compiler/vc_vec.vow`)
- A Vec is an existing `vc_agg` object id (`vc_agg_alloc` for creators, `vc_agg_param` for `GET_ARG` Ptr params), so `Return`/pointer plumbing is untouched. `vc_vec` keeps, per object, a state `(arr, len)` of two term ids, flow-sensitive per block.
- Creators: `__vow_vec_new*` -> `len = 0`, `arr` = fresh `declare-const` (like `vc_agg_junk`; contents beyond `len` are unconstrained and only readable behind the bounds claim). Parameter: `len`/`arr` are fresh `declare-const`s created lazily on first touch (`pK_vlen`, `pK_varr`), **unconstrained, no upper bound**.
- `push(v, x)`: path-guarded assumption `len != MAX` (`vc_assume`); `arr' = store(arr, len, ext64(x))`; `len' = len + 1`.
- `get(v, i)`: hard claim `bvult i len` (index operand must be 64-bit, signed or unsigned: a negative `i64` is a huge unsigned value, so one unsigned compare covers both ESBMC forms); value = `select(arr, i)` as `I64`.
- `set(v, i, x)`: same claim; `arr' = store(arr, i, ext64(x))`.
- `len(v)`: value = `len`, defined as `U64`.
- `pop(v)`: `len' = ite(len == 0, 0, len - 1)`, `arr` unchanged (runtime: `if v.len > 0 { v.len -= 1 }`).
- `ext64`: signed int -> `sign_extend`, unsigned int -> `zero_extend`, 64-bit -> as is, Bool -> `ite(b, #x..01, #x..00)`.

### Flow-sensitive state
- Writes (push/set/pop/creation) record `(block, arr, len)` for the object; the current block's state is a running pair.
- Block-entry state of object `o` = merge over the block's incoming edges (`fl.in_head`, `e_src`, `e_cond`) of the source block's exit state: all equal -> that term; otherwise the same first-edge-base + `ite(e_cond, v, acc)` chain as `vc_flow_phi_value`/`vc_agg_phi_read`, each merged term bound with `vc_flow_name` (a `define-fun` with the array sort) so merges of merges do not reprint shared subterms.
- Exit state of block `b` = its last write record for `o`, else `b`'s entry state. Entry states are memoised per `(o, block)`; a read in a block where `o` is not live cannot happen (defs dominate uses, validated by `ir_non_dominating_read`).
- **No unbounded recursion**: resolve unresolved ancestors iteratively (collect, then resolve in RPO order using `dom_rpo_numbers`), because inlined and unrolled functions can have tens of thousands of blocks. Sparse storage (linked write records per object, as `vc_agg` slots do), not an `n_objects x n_blocks` matrix.
- Unreached blocks (`vc_flow_enter` false) never contribute edges, so dead arms cost nothing.

### Gate (`vc_vec_shape_detail`, fail-closed, in `vc_vec.vow`, called from `vc_skip_reason_with` next to `vc_agg_shape_detail`)
Static use-scan over the loop-closed IR; no new skip code (the list is closed by ADR-1430). Each rejection is `unsupported-opcode: <detail>` or `unmodeled-builtin: <sym>`:
- a Ptr value used both as a Vec-op receiver and in `FieldGet`/`FieldSet`/user-call arg -> reject;
- a Vec-class Ptr through `Phi`/`Upsilon` -> `"Vec merged by a Phi"`, **unless every Upsilon feeding that Phi supplies the same value id** (a trivial same-object Phi, e.g. a loop-closing Phi, is allowed; Step 0 checks whether the core fixtures contain any);
- pushed/set value or `get` result of a type other than Bool/int<=64 (so `Vec<Vec>`, `Vec<String>`, `Vec<struct>`, wide) -> reject; index operand not 64-bit -> reject;
- any `__vow_vec_*` symbol not in the Vec table (`clear`, `truncate`, `reserve`, `sort`, `from_raw_parts_copy`, `pin_to_root`, `*_wide_ptr`) stays `unmodeled-builtin`.

### Executor
- `vc_walk_inst`: a `CALL extern` whose symbol is in the Vec table goes to `vc_vec_exec` before the `vc_define(... vc_exec_value ...)` fallthrough (which indexes `inst.args[1]` of a call and would crash on a 1-arg `len`). The `vc_is_unwrap_abort` branch precedes it today; keep the order explicit and tested.
- New `VC_CLAIM_VEC_BOUNDS()` (hard) with label `"vec bounds"` (the text ESBMC prints, so `build_ce_from_result` descriptions and the unattributed vow id `4294967293`/blame `none` of `tests/verify-fail/off_by_one_bounds.vow` match). Emitted through `vc_guard`, so later claims assume it.
- **Frames for inlined callees.** `vc_frame_of` (`vc_ops.vow:140`) reads the frame from `ds`, which for an extern call holds the symbol, so a bounds claim spliced from a callee would report frame -1 (the target's own) with an empty `call_sites` chain. The unwrap abort solves this by carrying `frame + 1` in `dv2` (`vc_inline.vow:262-263`, `vc_ops.vow:141`); the Vec get/set calls are emitted with `dv2 = 0` and so can use the same channel: generalise the predicate (`vc_frame_in_dv2(inst)` = unwrap abort or Vec bounds-checked call), use it in `vc_map_inst` and `vc_frame_of`. Confirm `vc_clone_inst` (`vc_loops.vow:419`, which already forwards `dv2`) and `vc_unroll` preserve `dv2` on calls (they do for the unwrap abort).
- `pre_only` functions drop hard claims already (`vc_add_claim`); no change.
- Logic: header command 0 is `QF_BV` today. Switch to `QF_ABV` **only** when the function created a Vec object (patch `names[0]` after the walk), so every non-Vec query stays byte-identical (no perf or golden churn).
- Counterexample values: still only integer params (`model_names`); a `Vec` parameter's length and contents are not reported (ESBMC does not report them either). Replay of such a counterexample is skipped by the existing replay gate.

## Steps (TDD slices; each = red test, minimal green, then refactor)
Every slice runs under `build/vowc test compiler/tests/<file>.vow` (needs `scripts/bootstrap.sh --skip-cargo` once; set `VOW_CACHE_DIR=$(mktemp -d)` when validating after compiler rebuilds). Never `sleep`-poll; run bootstrap and full_test in the foreground with an explicit timeout, or in the background with a done-marker file.

### 0. Confirm verifier-visible IR (no code)
`build/vowc build --no-verify --dump-ir` on `tests/verify/vec_fill.vow`, `bounds_correct.vow`, `tests/verify-fail/vec_overcount.vow`, `off_by_one_bounds.vow` and a small `v[i] = x` / `pop` / `Vec<u8>` program. Record: which extern names reach the verifier (base vs `_in_arena`, arg order); the cast after `get_val`; and **whether `v` goes through any header or loop-closing Phi/Upsilon** in the loop fixtures. If it does, the gate's "same value id" exception must cover it; if the lowerer routes `v` through a different-id Phi, this plan's gate rule has to change before Step 2. Correct the symbol table before writing it.

### 1. SMT array vocabulary
- Test: `compiler/tests/test_vc_smt.vow`: render `(declare-const a (Array (_ BitVec 64) (_ BitVec 64)))`, `(define-fun g0 () (Array ...) (store a #x..01 #x..02))`, `(select g0 i)`, and `ite` over arrays.
- Code: `vc_term.vow` (`VC_OP_SELECT`, `VC_OP_STORE`, `VC_SORT_ARRAY64() = -1`), `vc_smt.vow` (`vc_op_symbol`, `vc_sort_text`). Check `vc_flow_name`/`vc_script_define_fun` accept the new sort unchanged.

### 2. Vec op table and gate acceptance
- Test: `test_vc_ops.vow`: replace the `__vow_vec_len` "not known" assertion (line 57) with: catalogue still `false`; `vc_vec_op_kind("__vow_vec_len")` is LEN; `_in_arena` variants map with their arg offset; `__vow_vec_clear`, `__vow_vec_from_raw_parts_copy_val` are `VC_VEC_NONE`. `test_vc_gate.vow`: a hand-built `new; push; get; len; return` function has `vc_skip_reason == ""`; `get` with an `i32` index, a `Vec<String>` push (Ptr value), a Vec used as a struct and a Vec through a different-value Phi each return the specific detail; `clear` returns `unmodeled-builtin: __vow_vec_clear`.
- Code: `vc_ops.vow` (`VC_VEC_*` kinds, `vc_vec_op_kind`), `vc_gate.vow` (`vc_inst_supported` arm for CALL, `vc_inst_specific_reason` consults the table), `vc_vec.vow` (`vc_vec_shape_detail`, minimal).

### 3. Vec state store, straight-line
- Test: new `compiler/tests/test_vc_vec.vow` (+ `test_vc_exec.vow` golden): `new; push 1; push 2; len; ensures result == 2` yields queries whose negated goal is over the two-add length with no assumption besides the push guards; `get(v, 1)` claim text is `(bvult i len)` and is assumed afterwards.
- Code: `vc_vec.vow` state, creators, push/get/set/len/pop; `vc_exec.vow` dispatch + `VC_CLAIM_VEC_BOUNDS`; `QF_ABV` switch.

### 4. Branch and loop merges
- Test: `test_vc_vec.vow`: diamond where one arm pushes -> length after the join is one `ite` named by one `define-fun`; both arms pushing the same value collapse; push on one path does not leak onto the other; unrolled loop with 8 pushes (via `vc_unroll`) yields linear (not exponential) query size (mirror `check_query_size_is_linear_in_nesting`, `test_vc_exec.vow:422`); a 20000-block chain with an unwritten Vec does not blow the stack.
- Code: block-entry merge + iterative ancestor resolution.

### 5. Parameters, inlined frames, no-capacity property
- Test (`test_vc_vec.vow`, `test_vc_inline.vow`, `test_vc_exec.vow`):
  - Vec parameter `first_byte` shape: `ensures result <= 255` is provable; **structural no-capacity assertions** on every Vec query (not a literal grep): (a) a Vec-parameter function with no push has zero assumptions before the negated goal other than path conditions and user `requires`; (b) after one push, the push guard is the only added assumption and the only assumption that mentions a length term; (c) no `bvult`/`bvule` of a length term against a constant the program did not write, checked by walking the term arena rather than the rendered text.
  - Inlining: a contracted callee that indexes its own local Vec out of bounds yields a claim with `frame >= 0`, so `vc_chain_outcome` produces a non-empty `call_sites` chain; a caller-side Vec claim keeps frame -1.
- Code: param objects; the `dv2` frame channel generalisation in `vc_ops.vow`/`vc_inline.vow`.

### 6. End-to-end fixtures (`tests/verify-native/`)
- `pass/`: `vec_push_len.vow`, `vec_index_in_bounds.vow` (`requires i < v.len()` on a param), `vec_set_get_roundtrip.vow`, `vec_pop_len.vow`, `vec_fill_loop.vow` (copy of `tests/verify/vec_fill.vow`), `vec_bounds_correct.vow` (copy of `tests/verify/bounds_correct.vow`), `vec_param_len_unbounded.vow` (a property true for *all* lengths; comment: ESBMC proves it only for `len <= 128`), `vec_narrow_elem.vow` (`Vec<u8>`/`Vec<i32>` sign/zero extension), `inline_callee_builds_vec.vow` (callee pushes to a local Vec and returns a scalar).
- `fail/`: `vec_oob_index.vow` (`v[n]` on a param without `requires`; label "vec bounds", vow id `4294967293`), `vec_off_by_one.vow` (copy of `tests/verify-fail/off_by_one_bounds.vow`), `vec_overcount.vow` (copy; Callee blame, vow id 0), `vec_set_wrong_value.vow`, `inline_callee_local_vec_oob.vow` (bounds failure inside a spliced callee; `call_sites` lists the call).
- `unknown/`: `vec_loop_symbolic_len.vow` (push `n` times for symbolic `n`, ensures `len == n`): the unwinding claim stays `sat` -> `unknown` with the existing `unwinding assertion:` reason, never `proven`.
- `skip/`: keep `wide_vec_element_skipped.vow` (reword its comment to name the wide-slot follow-up), add `vec_of_vec_skipped.vow`, `vec_phi_merge_skipped.vow`, `vec_clear_skipped.vow`, `vec_from_raw_parts_skipped.vow`, each with `// TEST: skip-reason <code>` (`full_test.sh:1469`).
- `tests.sh` (fake bitwuzla): add one case running a Vec fixture and asserting each recorded `q.N.smt2` starts with `(set-logic QF_ABV)` and contains `select`/`store`; the capacity criterion itself is the structural unit test of Step 5, not text grepping.

### 7. Differential run and honesty table
- Run `python3 scripts/verify_diff.py --vowc build/vowc` over the corpus. Expected: the five pure-Vec fixtures in the census above `match`; no row becomes `weaker` or `soundness` relative to today; `model_capacity_bound_note`, `raw_parts_copy_*`, `replay_skip_string` stay as they are. If a row moves otherwise, stop and explain it in the ADR table (native result, ESBMC result, cause: cap assumption, new Skipped class, or unwinding); never edit a contract, never add a length bound. No baseline file exists to re-record.
- ADR addendum table lists, per function: the five corpus fixtures (match) and the `tests/verify-native/pass/` functions where native proves for all lengths what ESBMC proves only for `len <= 128` (the criterion-3 explanation).

### 8. Spec, ADR, help, follow-ups
- `docs/spec/cli.md` "Native backend": Subset bullet (:62) and Aggregates bullet (:71) drop "collections ... a loop over a `Vec`" and the "`Vec<i128>`... like every collection" wording; add a **Collections** bullet: `Vec<T>` of Bool/int<=64 is an array plus a 64-bit symbolic length with no capacity, index out of bounds is a hard claim (`vec bounds`, unattributed), a parameter's length is unconstrained, and the Skipped list (`String`, maps, `clear`/`truncate`/`from_raw_parts_copy`/`pin_to_root`, wide or nested elements, Vec across calls, Vec merged by a Phi). Then `uv run python scripts/generate_help.py` and commit the regenerated `compiler/main.vow` and `vow/src/skill.rs`; `scripts/check_help_coverage.py` must pass; `cargo build --release -p vow` rebuilds the Rust compiler with the regenerated skill (dual-compiler: both embed the same text).
- `docs/adr/2026-10-08-1422-native-verification-semantics.md` `## Addendum (#1423)`: representation, the `len != MAX` push decision with the runtime citation and rejected alternatives, distinct-parameter assumption, hand-listed op table vs catalogue, gate rules, frame channel, Skipped classes, the honesty table; update the "Known limits" bullet "`Vec<i128>` elements stay Skipped until the collection model (#1423)".
- File follow-up issues with `gh issue create`: `from_raw_parts_copy`/`pin_to_root` (and the harness decision for the `*_beyond_model_cap` fixtures flipping to class `soundness`), Vec across user-call boundaries, `clear`/`truncate`, `Vec<i128>`, Vec merged by a Phi, `String` (same array+length model, exact `string_eq`), maps.
- Post `gh issue comment 1423` listing the judgment calls (push guard, hand-listed table, distinct params, deferred ops, per-function criterion-3 mechanism) with the alternatives considered.

## Testing
- Unit: `build/vowc test compiler/ --filter vc_` (covers `test_vc_smt/ops/gate/vec/exec/inline`).
- Wiring: `VOWC_BIN=build/vowc bash tests/verify-native/tests.sh`.
- End to end (needs `bitwuzla` on PATH): `build/vowc verify --backend native --no-cache <fixture>` for each new fixture; statuses per `full_test.sh` Section 4g (`pass` Verified, `fail`/`unknown` VerifyFailed, `skip` Skipped with the declared code).
- Differential: `python3 scripts/verify_diff.py --vowc build/vowc`.
- Gates before pushing: `scripts/bootstrap.sh --skip-cargo --no-cache` re-run on the final head SHA (record SHA and the `scripts/seed.toml` pin in the PR checklist), then `VOW_FULL_TEST_SKIP_CARGO=1 scripts/full_test.sh` in the background with a done-marker (about 40 min), `python3 scripts/generate_operations.py --check` (must stay clean: catalogue untouched), `python3 scripts/check_help_coverage.py`, and `cargo fmt --all --check` plus `cargo clippy --all --all-targets -- -D warnings` only because `vow/src/skill.rs` is regenerated.

## Verification surface
- Properties the native solver must now discharge: bounds claim `i <u len` on every `get`/`set` (hard); user `ensures` over `len`/elements; the push `len != MAX` assumption is path-guarded and never a claim. Bitwuzla runs `QF_ABV` for Vec functions only; everything else stays `QF_BV`.
- ESBMC-comparison set: `tests/verify/{vec_fill,bounds_correct}.vow`, `tests/verify-fail/{vec_overcount,off_by_one_bounds,replay_unattributed_bounds}.vow`. No `tests/run/` or `examples/` growth. The ESBMC pipeline and its fixtures are untouched.

## Risks
- **Binary fixed point / determinism**: new vectors only, no `HashMap`; iteration orders are index-ordered. `vc_vec.vow` must be in `scripts/concat_vow.sh:23` after `vc_agg` and before `vc_gate` (load order); a missing entry fails Stage 0 with an unresolved name.
- **Stale state across merges** (soundness): a missed merge makes a pushed Vec look shorter on the join path and can prove false claims. Mitigate with the Slice 4 tests plus a fail fixture whose bug is visible only after a conditional push.
- **Alias unsoundness**: two model objects for one runtime Vec (struct field, call argument, Phi) would diverge. The gate rejects each route found; the shape rule needs one test per route.
- **Param-object classification**: IR types a Vec, String and struct param all as `Ptr`; a Ptr used both by Vec ops and field ops is rejected, a `String` param's `len()` stays `unmodeled-builtin`. Verify with `string_param_verify.vow`-style mixes.
- **Blame/frame misattribution**: covered by the `dv2` frame channel and the two inline fixtures; if `vc_unroll` or `vc_clone_inst` drops `dv2` on calls, the inline fail fixture catches it.
- **Query growth**: each write adds a named array `define-fun`; bit-blasting 64-bit index/element over 64 unroll iterations can be slow. Watch `scripts/verify_perf.py` (#1610 harness) on `vec_fill`; a timeout is `unknown`, never `proven`.
- **Spurious counterexamples**: push wrap (handled by `len != MAX`); signed narrow index (gated to 64-bit); `get` on narrow elements relies on the IR's own cast (Step 0 confirms).
- **Wire format**: no change to `VOWRES2` or the result schema; the new claim kind reuses the unattributed-label reporting.
- **Dual-compiler rule**: not violated (verifier exemption), but the PR description must say so and cite the CLAUDE.md exception; the only Rust-side diff is the generated `vow/src/skill.rs`.
- **Review size**: one PR on this branch. If the diff runs large, trim scope rather than split: drop `pop` and the narrow-element fixture first, keeping push/get/set/len and the gate; slices 1-2 are independently green (Vec still `Skipped`) so the implementer can commit them separately on the branch.

## Out of scope
- Any change to `vow-verify/`, `compiler/c_emitter.vow`, ESBMC flags, `ModelCapacityAssumed` (the ESBMC path keeps emitting it; native never does), or the ESBMC capacity constants.
- `from_raw_parts_copy`, `pin_to_root`, `String`, `HashMap`/`BTreeMap`, user-struct heap, `Vec<i128>`, nested collections, Vec across user calls, `clear`/`truncate`/`sort`, Vec in struct fields, Vec Phi merges (follow-ups above).
- k-induction / invariant-as-hypothesis (#1419/#1420): loops over a symbolic-length Vec stay `unknown` until those land.
- `docs/spec/contracts.md`, `errors.md`, `grammar.md` rewrites (ADR assigns them to the P3 spec child; no language or builtin-signature change here).
- Refactors or formatting of `vc_exec.vow`/`vc_agg.vow` beyond the minimal dispatch hook.
