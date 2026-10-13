# Plan: model `Vec::clear` and `Vec::truncate` in the native verifier (first slice of #1613)

## Goal
#1613 is an umbrella of independent, fail-closed `Skipped` items. This PR lands exactly one: `clear` and `truncate`
(`__vow_vec_clear`, `__vow_vec_truncate`), which are pure `(array, len)` transitions and need no new design. The PR
body says `Refs #1613` (not `Closes`); every other item is recorded below as a follow-up and posted to #1613 as a
checklist comment.

## Assumptions
- First slice = `clear` + `truncate` only (best guess): smallest item with exact runtime semantics, no harness ruling, no
  new sort/axis, no dependency on the other items. ESBMC does not model them, so no soundness-class risk.
- `reserve` is dropped from the issue: it has no user-facing builtin (`__vow_vec_reserve` is a private, non-`extern "C"`
  runtime fn; grep of `compiler/lower.vow`/`vow-types/src/env.rs` finds no `reserve` method). Nothing can be `Skipped` on it.
  The only growth ops are `push` (already modelled, the push guard is its capacity trap). Recorded in the ADR addendum.
- `sort` is `vec_sort(Vec<i64>) -> Vec<i64>` (`vow-types/src/env.rs:227`): it returns a *new* Vec, so it needs a
  permutation + sortedness axiomatisation and a fresh object. Deferred (follow-up 6).
- `from_raw_parts_copy`/`pin_to_root` need a corpus-label ruling (`tests/verify-fail/raw_parts_copy_*_beyond_model_cap.vow`
  would become `verify_diff` class `soundness`). That is a decision for a human/ADR, not for this PR. Deferred (follow-up 1).
- Verifier-only change: the native verifier is exempt from the dual-compiler rule (CLAUDE.md "Scoped exception"). No Rust
  twin, no `c_emitter` change, so no verifier-C parity risk; `vow-verify` untouched.
- A `Vec` that is a rodata literal traps on `clear`/`truncate` (`region_literal_mutation_trap`,
  `vow-runtime/src/lib.rs:1775-1795`). Modelled like `pop`/`set`/`push` already are: the model describes returning paths
  only, so no abort claim is emitted.

## Semantics (from `vow-runtime/src/lib.rs:1775-1795`)
- `clear`: `len' = 0`; the array is unchanged (stale slots are unreadable: every read is claimed `idx <u len`).
- `truncate(n)`: `len' = ite(n >=u len, len, n)` (a no-op when `n >= len`); array unchanged. `n` is `u64`
  (`docs/spec/grammar.md:1120`), so a plain unsigned compare, no sign handling.
- Neither raises a bounds/abort claim, neither needs a length-mask assumption (both only lower `len`), so the "one length
  fact" of the #1423 addendum is untouched: `len' <= len < 2^63-1`.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `compiler/vc_ops.vow` | Vec op table: add `VC_VEC_CLEAR`/`VC_VEC_TRUNCATE`, dispatch in `vc_vec_op_kind`, new shared `vc_vec_kind_writes` | 113-135 (consts, dispatch by symbol length: `clear` = 15, `truncate` = 18, shares the 18-bucket with `push_val`), comment 107-112 |
| `compiler/vc_gate.vow` | gate: operand/result types of the new calls | `vc_vec_call_ok` 220-236 |
| `compiler/vc_vec.vow` | state transitions `vc_vec_clear`, `vc_vec_truncate` beside `vc_vec_pop` | `vc_vec_pop` ~215-222, `vc_vec_write` 143 |
| `compiler/vc_exec.vow` | executor dispatch in `vc_walk_vec` | 412-445 |
| `compiler/vc_induct.vow` | loop-write predicate: replace inline `PUSH\|\|SET\|\|POP` by `vc_vec_kind_writes` | `vc_induct_vec_written_in_loop` 235-250 (+ doc comment 230-234) |
| `compiler/vc_unroll.vow` | same predicate in `vc_step_applicable` | 305-308 |
| `compiler/tests/test_vc_ops.vow` | op-table test | 716 currently asserts `clear` is `VC_VEC_NONE` -> flip |
| `compiler/tests/test_vc_gate.vow` | gate accept/reject | `check_vec_ops_rejected` 614-619 (clear asserted `unmodeled-builtin` -> move to accepted) |
| `compiler/tests/test_vc_vec.vow` | state-machine unit tests | pattern: `param_push_get` 62-70 |
| `compiler/tests/test_vc_induct.vow`, `test_vc_unroll.vow` | loop-write regression | add clear/truncate-in-loop cases |
| `tests/verify-native/skip/vec_clear_skipped.vow` | currently the `unmodeled-builtin` skip-reason fixture; must be repointed | whole file |
| `tests/verify-native/pass/`, `fail/` | new fixtures (below) | pattern: `pass/vec_pop_len.vow` |
| `docs/adr/2026-10-08-1422-native-verification-semantics.md` | new addendum + edit #1423 follow-up list | 385-482 (esp. 425-427 skipped classes, 478-482 follow-ups) |
| `docs/spec/cli.md` | "Collections" bullet | line 73 (`**Skipped**: ... clear, truncate, reserve, sort`) |
| `docs/spec/contracts.md` | Native backend paragraph | line 56 |
| `docs/spec/errors.md` | `unmodeled-builtin` description | line 1410 (`Vec clear/sort/...`) |

## Steps (ordered TDD slices; each red -> green -> refactor)

### 1. `clear`: op table + gate + executor + state (vertical slice)
- **Red**: `compiler/tests/test_vc_ops.vow` — flip line 84 (`__vow_vec_clear` -> `VC_VEC_CLEAR()`); `compiler/tests/test_vc_gate.vow` —
  add `clear` (`n == 1`, `ITY_PTR` receiver, result `ITY_UNIT`) to the accepted checks and drop it from
  `check_vec_ops_rejected`; reject `clear` with result `ITY_I64` or 2 args. `compiler/tests/test_vc_vec.vow` — IR
  `fn f() -> u64 { v = Vec::new(); push; push; clear; len }` via `vc_exec` and assert the walked `len` term is the
  constant 0 and the query is `QF_ABV` (follow the existing `param_push_get` harness).
- **Green**: `vc_ops.vow` add `fn VC_VEC_CLEAR() -> i64 { 7 }`, bucket `n == 15` in `vc_vec_op_kind`; `vc_gate.vow`
  `if kind == VC_VEC_CLEAR() { return n == 1 && inst.ty == ITY_UNIT(); }`; `vc_vec.vow`
  `vc_vec_clear(vv, ar, hdr, s, bi)` = `vc_vec_write(vv, ar, hdr, s, bi, cur_arr, 0-constant)`; `vc_exec.vow` `vc_walk_vec`
  branch `else if kind == VC_VEC_CLEAR()` beside `POP`. No claim, no `vc_assume`.
- **Reuses**: `vc_vec_pop` (`vc_vec.vow`) as the template, `vc_vec_cur_arr`, `vc_tm_bv`.
- Fixture `tests/verify-native/pass/vec_clear_len.vow`: `push; push; clear; ensures result == 0`, plus clear-then-push-then-index
  (`ensures result == 7`). Repoint `skip/vec_clear_skipped.vow`: `git mv` to `skip/vec_sort_skipped.vow` and
  rewrite it to call `vec_sort`, keeping a Vec-specific `// TEST: skip-reason unmodeled-builtin` pin (code coverage itself is
  not at risk: other fixtures, e.g. `skip/unmodeled_builtin_skipped.vow`, already carry that code). Confirm the new body
  is `Skipped` with `build/vowc contracts --verify --backend native <fixture>`.

### 2. `truncate`
- **Red**: gate test: `truncate(ptr, u64)` accepted; `truncate(ptr, i32)` and `truncate(ptr, i64)` -> `unsupported-opcode`
  (the arg must be `vc_ty_is_int && vc_ty_bits == 64`, unsigned or not at the IR level is the lowerer's job — mirror
  `vc_vec_index_ok`); `test_vc_ops` maps `__vow_vec_truncate` -> `VC_VEC_TRUNCATE()`; `test_vc_vec`: after `push,push,truncate(1)`
  the solver is asked `len == 1` (unsat negation) and `truncate(5)` leaves `len == 2`; param case `truncate(n)` proves
  `len' <=u len` and `len' == n || len' == len`.
- **Green**: `VC_VEC_TRUNCATE() -> 8`, `n == 18` bucket compares `__vow_vec_truncate` alongside `push_val`; gate arm
  `n == 2 && inst.ty == ITY_UNIT() && vc_vec_index_ok(t1)`; `vc_vec_truncate(vv, ar, hdr, s, bi, n)` with
  `next = ite(VC_OP_BVUGE(n, len), len, n)` (`VC_OP_BVUGE` exists at `compiler/vc_term.vow:60`; unsigned, never the signed op);
  executor arm reads `w.var_of[inst.args[1]]`.
- Fixtures: `pass/vec_truncate_len.vow` (local Vec: `push x3; truncate(2)` gives `len == 2`; `truncate(10)` gives `len == 3`;
  param case `fn f(v: Vec<i64>, n: u64) -> u64 { v.truncate(n); v.len() }` with `ensures: result <= n`, which holds for
  every length: `len' = n` when `len > n`, else `len' = len <= n`);
  `fail/vec_truncate_wrong_post.vow` (`push x3; truncate(2); ensures result == 3` refuted);
  `fail/vec_get_after_clear_oob.vow` (`push; clear; v[0]` -> `VerifyFailed`, `index out of bounds`, vow id `4294967293`).

### 3. Loop and induction interaction
- **Red**: `test_vc_induct.vow` / `test_vc_unroll.vow`: a loop whose body calls `clear` or `truncate` makes
  `vc_induct_vec_written_in_loop` true and `vc_step_applicable` false.
- **Green**: add to `vc_ops.vow` `fn vc_vec_kind_writes(k: i64) -> bool` (`PUSH|SET|POP|CLEAR|TRUNCATE`) and call it from
  `vc_induct.vow:243` and `vc_unroll.vow:306`; update the "(`push`, `set`, `pop`)" doc comments. This is a *required*
  soundness edit, not a refactor: without it a loop that clears the Vec would be cut for invariant induction while the
  havoc copy keeps the entry `(array, len)` (the exact failure #1423 documents at ADR 436-441).
- Fixture `pass/vec_clear_in_loop.vow`: `while i < n { v.push(i); v.clear(); i += 1 }` with an `ensures: result == 0` on
  `v.len()`, proven by unwinding only (`TEST` expectation mirrors `pass/vec_fill_loop.vow`).

### 4. Inlined callee and arena-free check
- **Red**: `tests/verify-native/pass/inline_callee_clears_local_vec.vow`: callee creates a local Vec, pushes, clears; caller
  uses `len`. (A Vec as argument/result stays `Skipped`; the callee must own the Vec so the existing shape rule passes.)
- **Green**: no code expected (`clear`/`truncate` carry no frame, unlike `get`/`set` in `vc_frame_in_dv2`); if the splicer
  mis-maps `ds`, fix in `vc_inline.vow:246` `vc_map_inst`. Confirm `__vow_vec_clear` has no `_in_arena` variant in
  `docs/spec/operations.json` `arena_routes` (it should not: only fresh-aggregate builtins do).

### 5. Differential harness and verdict table
- Run `scripts/verify_diff.py` over `tests/verify*/` and `tests/verify-native/`. Expectation: every changed row moves
  `skipped -> proven|refuted`, class `weaker` (native decides what ESBMC skips), never `soundness`. Any `soundness` row
  is a bug in the model: stop and fix. Record the changed rows in the ADR addendum table (same format as ADR 455-466).
- `tests/verify/string_clear_resets_len.vow` is `String`, unaffected (confirm still `Skipped`).

### 6. Docs (same PR)
- ADR-2026-10-08-1422: append `## Addendum (#1613): Vec clear and truncate`: semantics above, no new length fact, why no
  claim, induction rule, the `reserve` finding, the follow-up list with the harness-ruling note. Edit the #1423 addendum
  follow-up paragraph (478-482) to point at it.
- `docs/spec/cli.md:73`, `contracts.md:56`, `errors.md:1410`: move `clear`/`truncate` from Skipped to the modelled list;
  keep `reserve`, `sort`, ... as Skipped text but drop `reserve` as nonexistent.
- No `grammar.md`/help change (no language or flag change), so no `generate_help.py`.

## Testing
- Unit: `build/vowc test compiler/ --filter test_vc_` (ops, gate, vec, exec, induct, unroll). Run per-file during slices:
  `build/vowc test compiler/tests/test_vc_vec.vow`.
- Native fixtures: `bash tests/verify-native/tests.sh`; skip-reason coverage: `python3 scripts/check_skip_codes.py`.
- Differential: `python3 scripts/verify_diff.py` (see its `--help`); its own tests `scripts/test_verify_diff.py`.
- Gate wall-clock is long (bootstrap ~5 min, `full_test.sh` ~40 min): run in the foreground with explicit timeouts, in chunks.
  Before claiming green: `scripts/bootstrap.sh --skip-cargo --no-cache` at the PR's final head SHA, record the SHA and the
  `scripts/seed.toml` pin in the checklist (CLAUDE.md "Green locally"). Then `VOW_FULL_TEST_SKIP_CARGO=1 scripts/full_test.sh`.
- `cargo test`/`clippy` are not exercised by this change (no Rust files), run `cargo fmt --all --check` only if anything Rust is touched (nothing planned).
- PR title (lower-case, <=92 chars): `feat(verify): model Vec clear and truncate in the native verifier`.

## Risks
- Missing an induction/unrolling write site: any other place that enumerates "Vec writes" would make a clearing loop unsound.
  Mitigation: step 3 centralises in `vc_vec_kind_writes`; grep `VC_VEC_POP\|VC_VEC_SET` across `compiler/` before finishing
  and assert no remaining inline lists (`vc_induct.vow:243`, `vc_unroll.vow:306` are the only two found).
- Symbol-length dispatch in `vc_vec_op_kind`: `truncate` (18) shares a bucket with `push_val`; a wrong length constant silently
  makes the op `unmodeled-builtin` again. The `test_vc_ops` assertions cover each symbol, including the negative `_in_arena` ones (718).
- `truncate` unsigned compare: using a signed op would treat `n >= 2^63` as negative and wrongly set `len' = n`. Add a unit
  test with `n = 2^64-1` on a length-3 Vec (expects len 3).
- Binary fixed point: new `.vow` code in `compiler/` changes `build/vowc`; the bootstrap fixed-point and the native
  verifier's own self-verification run under `scripts/bootstrap.sh`. Keep new functions contract-free helpers like their
  siblings; no `HashMap`/ordering changes.
- Native query-shape gate in `tests/verify-native/tests.sh` ~959-986 forbids capacity-like constants in vec queries; a
  `clear`/`truncate` query adds only constants 0 and operand terms, so it stays green; run it, do not assume.
- Dependent verify cache (`vc_cache.vow` stores `unsat`): a stale entry could hide a modelling bug; use `--no-cache` for every
  validation run.
- No Rust twin and no `c_emitter` change by design (verifier exemption); if the implementer finds a shared lowering edit is
  needed, stop: that would require both compilers and parity.

## Out of scope (follow-ups; each its own PR, post as a checklist comment on #1613)
1. `from_raw_parts_copy` / `pin_to_root` — needs a ruling on the `*_beyond_model_cap` corpus labels first.
2. `bool` parameters (`GetArg[Bool]`: `vc_gate.vow:256` admits only int/float/ptr; model needs a `Bool` param term in
   `vc_exec.vow:343`) and `Vec<bool>` element read (a Bool narrowing in the gate/model, e.g. admit `get` result
   `ITY_I64` feeding a `!= 0` cast — decide between a model-side narrowing and a lowerer cast in both compilers). Natural
   second PR; the lowerer option would trigger the dual-compiler rule.
3. Vec across a call boundary, Vec in a struct field, Vec merged by a differing Phi — `vc_vec_shape_detail`
   (`vc_vec.vow:272-343`) objects/aliasing design; largest item.
4. `Vec<i128>/Vec<u128>` (two-slot wide elements) and pointer elements.
5. Vec counterexample values (parameter length and contents) — `vc_cex.vow`.
6. `sort` (`vec_sort`, returns a new Vec).
7. `reserve` — nothing to model (no builtin); close the checklist item with the ADR finding.
- No refactors, formatting or unrelated cleanups; no change to ESBMC/`vow-verify`/`c_emitter.*`, to `docs/spec/operations.json`,
  or to the verifier's `unmodeled-builtin` reporting beyond the two symbols.
