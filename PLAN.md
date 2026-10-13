# Plan: native verifier — String as array plus symbolic length, exact `string_eq` (#1426)

## Goal
Model `String` content operations in the native verifier (`compiler/vc_*.vow`, self-hosted only, no Rust twin per
ADR-1421) as a `(Array (_ BitVec 64) (_ BitVec 64))` plus a 64-bit symbolic length, the same representation #1423
gave `Vec`, with an exact `string_eq`. Every `__vow_string_*` runtime helper the lowerer can emit is either modelled
or has an explicit, tested skip reason in the op-model table. Closes the "Vec/String array+length model, exact
`string_eq`" P3 child of epic #1398 (ADR-1422 Rule 3; its #1423 addendum lists `String` as the named follow-up).

## Assumptions (decisions taken without an operator)
- **A1. Reuse the `vc_vec` machinery; do not build a second container model.** A `String` and a `Vec<u8>` share the
  runtime `VowVec` layout. String symbols are added to the `vc_vec_op_kind` family (new `VC_STR_*` kinds) so every
  existing `vc_inst_vec_kind(...) != VC_VEC_NONE()` consumer (gate `has_vec`, `vc_vec_shape_detail`, `QF_ABV` switch,
  `vc_induct_vec_written_in_loop`, `vc_unroll`, `vc_frame_in_dv2`) covers strings without a parallel code path.
  String-only transfer functions live in a new small module `compiler/vc_str.vow` (small-files rule).
- **A2. Byte representation.** A byte is stored zero-extended in the 64-bit slot and **masked with `0xFF` on every
  read and in every comparison**, so a parameter's unconstrained array can never yield a "byte" above 255
  (`tests/verify/string_offsets_u64.vow` needs `0 <= byte_at <= 255`). No new SMT array sort.
- **A3. Literal bytes.** The executor has no module string pool (`vc_claims_step(f, ...)`). `vc_inline(f, m, cap)` /
  `vc_module_skip_reason(f, m)` are module-aware, so the pre-executor pass copies pool bytes into the `ds` field of
  each `ConstStr` (unused for that opcode; `vc_op_frame_marked` does not include it) instead of threading `m` through
  every `vc_claims*` caller. The single call site of `vc_inline` is `compiler/vc_native.vow:485` (the gate in `vc_driver.vow:68` runs pre-inline and needs no bytes). Verify during slice 2 that every path to the executor (`vc_native`, `vc_induct`,
  `vc_unroll`, probes) goes through that pass; if one does not, route it through it rather than add a parameter.
- **A3b. `ConstStr` fails closed and cannot masquerade as a frame marker.** The gate accepts a `ConstStr` only when every use is
  `__vow_string_literal` arg 0 or `__vow_string_matches_literal_at` arg 2 (a stray use is `unsupported-opcode`).
  `vc_frame_of` (`vc_ops.vow`) parses any `ds` starting with `__vc_frame:`, so a user literal with that text must not be read as a
  frame: restrict the parse to `vc_op_frame_marked` opcodes (and test with a literal `"__vc_frame:3"`).
- **A4. Exactness for symbolic operands needs a bounded quantifier.** `string_eq(a,b) = (la == lb) ∧ ∀i <u la.
  a[i]&0xFF == b[i]&0xFF`; symbolic `push_str` and `substring` need the same range relation with offsets. Spike
  result: Bitwuzla 0.9.1 (the pinned solver) answers a quantified array query correctly with `(set-logic ALL)`. Plan:
  one new term node `VC_T_RANGE_EQ(a_arr, a_off, b_arr, b_off, n)` (5 operands, so the arena gains a fourth/fifth
  operand column or the node packs two operands into a child pair — implementer's choice, decided in slice 0) printed
  as `(forall ((i (_ BitVec 64))) (=> (bvult i n) (= (bvand (select a (bvadd a_off i)) #xFF…) (…))))`. Constant-length
  operands (literals, clones of literals, `matches_literal_at`) are expanded into plain `select`/`store` chains and
  stay quantifier-free. A function that never builds a `RANGE_EQ` keeps `QF_BV`/`QF_ABV` byte for byte.
  **Gate:** if slice 0 shows quantified queries are unusable (wrong answers, `unknown` on the string fixtures within
  `--timeout`), stop, comment on #1426 and file the follow-up for a quantifier-free encoding; do not ship an
  abstraction under the name `string_eq`.
- **A5. `byte_at` stays stricter than the runtime.** Per `docs/spec/grammar.md` (the "verifier is stricter" paragraph)
  an index not provably `< len` is a hard `index out of bounds` claim (vow id `4294967293`, blame None), same label
  and claim kind as the Vec bounds check; the `-1` sentinel is unreachable.
- **A6. Parse/format helpers are modelled opaquely where ESBMC proves fixtures; the rest are skipped with a reason.**
  The issue says they "stay opaque with documented limits"; ADR-1422 (Known limits, ~line 191) defines opacity as an
  *uninterpreted relation*, i.e. a model, not a skip. `tests/verify/parse_narrow_int_bounds.vow` and `parse_u8_clamped.vow`
  prove under ESBMC from the payload's type range alone, so skipping them would be `weaker` against acceptance criterion 1.
  Plan (slice 8): `parse_i8/i16/i32/u8/u16/u32` (and `parse_i64` if its fixture needs it) return a **fresh `Option` object**
  with an unconstrained tag and an unconstrained payload of the target width (built like a parameter object); `int_to_string` /
  `uint_to_string` return a fresh String object with unconstrained array and length (below the length mask). Documented
  limits: each call is independent (no congruence: `parse(s) == parse(s)` is not provable), no relation to the input bytes,
  `parse(format(n)) == n` is `unknown`. Pure catalogue entries get `verifier_model: known` in `docs/spec/operations.json`
  (regenerates `catalogue_verifier_known`). `parse_i128/u128`, `int128/uint128_to_string` stay skipped like ESBMC
  (`tests/verify-skip/parse_wide_skipped.vow`, `int128_format_call_skipped.vow`). If slice 8 outgrows the PR, split it out and
  list these as `unmodeled-builtin` with a reason (explicit deviation, follow-up issue named in the PR body).
  Everything else stays skipped with a documented one-line reason: `split`, `trim`, `to_upper/lower`, `replace`, `join`,
  `hex_*`, `hash_str`, `contains`, `starts_with`, `ends_with`, `from_raw_parts_copy`, `format_f64_bits`/`parse_f64_bits` (string
  side), IO, and every `_in_arena` variant. `contains`/`starts_with`/`ends_with` are cheap follow-ups on `RANGE_EQ`; not bundled.
- **A7. No capacity cap.** The only length fact is the existing runtime representation invariant (`len < 2^63 - 1`,
  `VOW_CAP_VALUE_MASK`), reused from `vc_vec_len_mask`/`vc_vec_push_guard`; a returning `push_str` is assumed to keep
  `ld + ls` below it (the runtime traps otherwise). No `len <= 256` anywhere (CLAUDE.md contract-authoring rules).
- **A8. ESBMC cap fixtures diverge by design.** `verify-fail/string_push_str_overflow.vow` (and the
  `raw_parts_copy_*_beyond_model_cap` family, already named a follow-up in the #1423 addendum) fail on ESBMC only
  through `STRING_MODEL_CAP = 256`; the native result for `over` is `proven` (the length equation is true). They are
  explained per fixture in the ADR addendum, not "fixed" by editing contracts or labels, following the #1423 precedent.

## Key Files
| File | Role | Lines of interest |
|------|------|-------------------|
| `compiler/vc_ops.vow` | op-model table: `vc_vec_op_kind` (symbol → kind), `VC_VEC_*` consts, `vc_frame_in_dv2`, `vc_op_unsupported` (lists `IOP_CONST_STR`), `catalogue_verifier_known` (generated, all `false`) | 129-165, 78, 195-212, 245-249 |
| `compiler/vc_vec.vow` | `(arr,len)` flow-sensitive state, `vc_vec_slot/create/push/set/pop/enter`, `vc_vec_shape_detail`, `vc_vec_len_mask`, `vc_vec_push_guard` | whole file (343) |
| `compiler/vc_exec.vow` | `vc_walk_vec` dispatch (404-440), `vc_walk_inst`, `vc_claims_step`, logic switch `QF_BV`→`QF_ABV` (556, 613) | 404-440, 556, 613 |
| `compiler/vc_gate.vow` | `vc_vec_call_ok` (213-228), `vc_inst_supported` call arm (263-267), `vc_inst_specific_reason` (306-315), `has_vec` (382) | as listed |
| `compiler/vc_term.vow`, `compiler/vc_smt.vow` | term arena, op codes, writer (`vc_term_text`, `vc_command_text`, `VC_CMD_SET_LOGIC`) — new `RANGE_EQ` node + printer | vc_term 12-70, vc_smt 125-245 |
| `compiler/vc_inline.vow` | module-aware pre-executor pass: fill `ConstStr.ds` | 381, 451 |
| `compiler/vc_induct.vow`, `compiler/vc_unroll.vow` | loop-body Vec-write detection must include string writers | induct 242-243, unroll 305-306 |
| `compiler/vc_slice.vow`, `compiler/vc_cache.vow`, `compiler/vc_bvfold.vow` | walk/hash/fold terms; must treat `RANGE_EQ` as an opaque predicate with 5 children | read in slice 0 |
| `compiler/vc_str.vow` [new] | string term builders: `vc_str_byte`, `vc_str_literal_state`, `vc_str_push_byte/push_str/clear/clone`, `vc_str_eq`, `vc_str_matches_literal`, `vc_str_substring` | — |
| `compiler/lower.vow` | read-only reference for emitted shapes: literal (`ConstStr`+`__vow_string_literal`, 2999-3010), `String::from` = `__vow_string_clone` (4435), `String::new` = `__vow_string_new(0,0)` (4465), `string_matches_literal_at` (3349-3363), `__vow_string_eq` typed Bool (3181) | — |
| `docs/spec/operations.json`, `scripts/generate_operations.py` | string-typed catalogue entries stay `verifier_model: unmodeled`; only touch if a reason field is chosen (see slice 8) | — |
| `docs/adr/2026-10-08-1422-native-verification-semantics.md` | add `Addendum (#1426)`: representation, ops, quantifier decision, skipped classes, divergence table | 385-480 |
| `docs/spec/contracts.md` | verification-semantics wording for `String` (check; spec rewrite is a separate P3 docs child) | — |
| `tests/verify-native/{pass,fail,skip}/` [new fixtures], `compiler/tests/test_vc_str.vow` [new], `test_vc_vec.vow`, `test_vc_gate.vow`, `test_vc_ops.vow` | tests | — |
| `scripts/verify_diff.py` (read-only) | differential classes; run with `--filter string` | — |

## Steps (TDD slices — each red → green → refactor, each its own commit; one PR)

### Slice 0. Spike: quantified array query + term-node shape (no production code merged)
- Write throwaway `.smt2` files under `$TMPDIR` for `string_eq` on two parameter strings, **`eq` used as a branch condition**, **`a.eq(b) ⇒ a.byte_at(k) == b.byte_at(k)` for symbolic `k`** (instantiation cases, where `unknown` shows up — not only assumption-position quantifiers), `push_str` with symbolic source,
  and substring; run Bitwuzla 0.9.1 for sat/unsat and time. Confirm get-value of an array/length in a `sat` model works
  (counterexample extraction, `vc_solver`/`vc_cex`) and that incremental sessions (`vc_session`) accept `ALL`.
- Read `vc_slice`, `vc_cache`, `vc_bvfold`, `vc_probe` term walkers; list every place that pattern-matches `ar.a/b/c`.
  Decide arena column vs packed node for the 5 operands. Record the decision in the ADR addendum.
- Exit criterion = A4 gate. Output: a few lines in the commit message / PR body, not a file.

### Slice 1. Table scaffolding and the writer predicate (no String op is accepted yet)
- Tests (`test_vc_ops`, `test_vc_gate`, `test_vc_induct`, `test_vc_unroll`): `VC_STR_*` consts exist and are distinct; a new
  `vc_vec_kind_writes(kind)` returns true for PUSH/SET/POP (and, as later slices land, each string writer); the induction cut and
  the unroller consult it (red: a loop whose body does `push_byte` is *not* cut for invariant induction).
- Code: `compiler/vc_ops.vow` adds `VC_STR_*` kinds and `vc_vec_kind_writes`; `vc_induct.vow:242` and `vc_unroll.vow:305` call it
  instead of the inline `== VC_VEC_PUSH() || ...` tests. **Soundness rule for every later slice: a symbol enters `vc_vec_op_kind`, the
  gate's accepted set, `vc_vec_kind_writes` (if it writes) and the executor transfer in the same commit** — never gate-accepted
  before it is modelled (a string-writing loop cut with stale entry state proves facts about the wrong length, ADR-1423).
- `vc_vec_shape_detail`: string constructors (`literal`, `clone`, `pin_to_root`, `new`, later `substring`) mark their result as a
  container like `VC_VEC_NEW`, so the call/Phi/struct rules see them; the second pointer operand of `push_str`/`eq` is marked too.

### Slice 2. Literals, clone, new, len (QF, no new term node) — accepts `literal`, `clone`, `pin_to_root`, `new`, `len`, `ConstStr`
- Tests (`compiler/tests/test_vc_str.vow`, builders as in `test_vc_vec.vow`): `String::from("hello").len() == 5` proves;
  `== 6` refuted; `String::new().len() == 0`; clone of a param has equal length; `tests/verify/string_literal_byte_at`, `string_clear_resets_len`,
  `string_param_verify` shapes, plus `.vow` fixtures `tests/verify-native/pass/string_literal_len.vow`, `string_clone_len.vow`.
- Gate: `IOP_CONST_STR` moves from `vc_op_unsupported` to `vc_op_modeled_core` with the use rule of A3b; `String::new` only with both args constant 0 (else `unsupported-opcode: String::new with data`); call arities/types in `vc_vec_call_ok` for each admitted symbol.
- Code: `vc_inline` fills `ConstStr.ds` with the pool bytes (A3); executor creates a literal object = fresh array +
  `store` chain of the literal bytes, constant length (`vc_str_literal_state`); `clone`/`pin_to_root` copy `(arr,len)` into a new
  alloc object (arrays are values, so no copy loop); `__vow_string_literal` is the literal constructor; `len` reuses `VC_VEC_LEN` path.
  New `vc_walk_vec` branch delegates `kind >= VC_STR_BASE` to a `vc_walk_str` helper (same file region, short).
- A String parameter already works through `vc_vec_slot`'s parameter branch (`pK_varr`/`pK_vlen`, length `< mask` assumed).

### Slice 3. `push_byte`, `byte_at`, `clear` (each joins the gate, `vc_vec_kind_writes` and the executor together)
- Tests: after `push_byte(65)` on a literal `""`, `byte_at(0) == 65` proves, `len == 1`; `byte_at` result in `0..=255` for `i < s.len()` (`string_offsets_u64`);
  unguarded `byte_at(i)` → refuted `index out of bounds` with vow id `4294967293`, blame None (`tests/verify-fail/string_byte_at_unguarded_u64.vow`
  expectations); `string_empty` (callee blame, vow id 0); `clear` then `len == 0`.
- Code: `vc_str_push_byte` = `store(arr, len, zext8(byte))`, `len+1`, assume `vc_vec_push_guard`; `byte_at` = hard claim `idx <u len` via `vc_guard(..., VC_CLAIM_VEC_BOUNDS(), VC_LABEL_VEC_BOUNDS(), ...)` then
  `zero_extend(extract[7:0](select(arr, idx)))` as i64; `clear` = write `(arr, 0)`.

### Slice 4. `matches_literal_at` (QF)
- Tests: `tests/verify/string_matches_literal_at.vow` shape proves; wrong literal refuted; `pos` near `u64::MAX` (no overflow wrap) is false.
- Gate: arg 2 must be a `ConstStr` and arg 3 a `ConstI64` (checkable with the existing literal table, `vc_literal_table`); anything else is
  `unsupported-opcode`. Code: `vc_str_matches_literal(s, pos, lit_bytes)` = `pos + n` no-overflow ∧ `pos + n <=u len` ∧ ⋀ byte(pos+k) == lit[k].
- `push_str` and `eq` are **not** admitted here: the gate runs on IR before execution and cannot tell a literal-clone source from a symbolic
  one without a provenance walk. They enter the gate in slice 5 and the executor takes the constant-length store-chain fast path there.

### Slice 5. `eq` and `push_str` (all sources), exact via `RANGE_EQ` — gate admits both here
- Tests (red first): `eq` reflexive (`s.eq(s)` proves `true`, by id-equality shortcut AND by the solver for two clones); `a.eq(b) ⇒ a.len()==b.len()`;
  `a.eq(b) ∧ a.len()>0 ⇒ a.byte_at(0)==b.byte_at(0)` proves; `a.len()==b.len()` alone does NOT imply eq (refuted, with differing bytes); literal `"ab"` vs `"ab"` true,
  `"ab"` vs `"ac"` false (QF expansion); `string_eq_idempotent` (`first == second` — exactness makes it trivially true, no shared nondet cache);
  `over(a,b)` with `push_str(a); push_str(b)` proves `result.len() == a.len()+b.len()`; after `push_str(src)`, `result.byte_at(ld) == src.byte_at(0)` proves; `byte_at` of the dest prefix unchanged.
- Code: add the `RANGE_EQ` node (term arena, hashing, `vc_smt` printer with fixed bound name, `ALL` logic via `vc_script_replace_logic` only when present, slice/cache/bvfold treat as opaque Bool app).
  `vc_str_eq(a,b)` = `la==lb ∧ RANGE_EQ(a,0,b,0,la)` (constant-length / identical-term operands short-circuit to QF / `true`);
  symbolic `push_str`: fresh result array `r`, assumed (under `pc`) `RANGE_EQ(r,0,d,0,ld) ∧ RANGE_EQ(r,ld,s,0,ls)` via `vc_assume`, length `ld+ls`, plus the `< mask` guard assumption.
  Result of `eq` is a plain Bool term (no abstraction, no per-pair nondet).
- Executor fast paths (no quantifier): operands whose length term is a bit-vector constant (literals and their clones carry `vc_str_literal_state`) are expanded into `store`/`select` chains; identical state terms give `eq == true`.
- Refactor: if `vc_exec.vow` grows past ~700 lines move `vc_walk_str` into `vc_str.vow`'s sibling `vc_str_walk.vow`.

### Slice 6. Loops and merges with strings
- Tests: `pass/string_fill_loop.vow` — a loop pushing a literal-bounded number of bytes with `ensures: result.len() == 8` is `proven` by bounded unwinding (the induction cut is refused for a string-writing loop, so unwinding decides alone, as for Vec); `fail/string_fill_loop_wrong_len.vow` — wrong constant is refuted; `pass/string_branch_push.vow` — push in one arm, merge, `len` is the `ite`.
- Code: by now the writer predicate already covers every string writer (slice-1 rule); this slice adds the loop/merge tests and confirms `vc_vec_enter` merges string state (it is kind-agnostic).

### Slice 7. `substr` / `substring` (closed-form length, ranged copy)
- Tests: `tests/verify/string_substring_length.vow` (all four functions prove, `result.len() <= s.len()`, exact `b - a`), `verify-fail/string_substring_length_wrong.vow`, `verify-fail/string_substring_bytes_observed.vow` (refutes `== 65`, callee blame vow id 1).
- Code: `clamped_start = min(start, slen)`, `substring`: `clamped_end = clamp(end, clamped_start, slen)`, `len = end - start`; `substr`: `len = min(len, slen - clamped_start)` — unsigned, exactly the runtime formulas (`vow-runtime/src/lib.rs:2353-2395`); content `RANGE_EQ(r,0,s,start,len)` assumed on a fresh array. Both symbols plus `_in_arena` variants decided: only the plain symbols (`_in_arena` is emitted by region inference — check `compiler/region.vow`/`ir.vow` fresh_arena_base_extern; if the verifier can see `_in_arena` string calls, list them as skipped with reason and file a follow-up).
- If this slice outgrows the PR, split it into a second PR and keep the symbols in the table as skipped with reason `string-substring-pending`.

### Slice 8. Opaque parse/format helpers (A6)
- Tests: `tests/verify/parse_narrow_int_bounds.vow` and `parse_u8_clamped.vow` prove natively (class `match`); new `fail/parse_narrow_payload_wrong_range.vow`
  (`ensures: result <= 100` over `parse_u8`) refutes; `skip/parse_wide_skipped.vow` stays skipped; `int_to_string(n).len()` has no relation to `n` (an
  `ensures: result.len() == 1` over it is refuted, not proven).
- Code: in the executor, a call whose symbol is a known pure opaque builtin defines a fresh object (`VC_OBJ_PARAM`-style, unique owner) with unconstrained
  tag/payload (Option) or `(arr,len)` (String); `operations.json` entries `parse_i8/i16/i32/u8/u16/u32` and `int_to_string`/`uint_to_string` get
  `verifier_model: known`, then `python3 scripts/generate_operations.py` and `--check`. `catalogue_verifier_known` alone does not encode calls (CLAUDE.md), so the
  executor handler is the real change. Check how `vc_agg` lays out `Option` (tag slot, payload slot, narrow widths) before writing it.

### Slice 9. Table completeness: every runtime helper has a model or a reason
- Test (`test_vc_ops.vow`): a hand list of all `__vow_string_*` symbols the lowerer or fresh-arena routing can emit (enumerate with
  `rg '"__vow_string_[a-z_0-9]*"' compiler/lower.vow compiler/ir.vow compiler/region.vow` and the catalogue's string-typed `runtime_symbol`s) —
  assert each satisfies `vc_vec_op_kind(sym) != VC_VEC_NONE()` XOR has a non-empty `vc_str_unmodeled_reason(sym)`; assert a new symbol in the list without either fails. Do not read `vow-runtime/src/lib.rs` from the test; instead add a short `scripts/` grep check (or extend `scripts/generate_operations.py --check` only if trivial) that every `extern "C" fn __vow_string_*` appears in the list. If neither is cheap, keep the hand list and say so in the PR body.
- Code: `vc_str_unmodeled_reason(sym)` in `vc_ops.vow` (wide parse/format, case/trim/replace/split/join transforms, search family, arena variants, raw-parts copy, IO). `vc_inst_specific_reason` appends it: `unmodeled-builtin: __vow_string_trim (case/whitespace transform not modelled)`.
  Unmodelled catalogue string builtins (`string_trim`, `parse_i128`, … ) keep `verifier_model: unmodeled`; the reason text lives in `vc_str_unmodeled_reason`, keyed by runtime symbol. No new catalogue field.
- Update `tests/verify-native/skip/` with one fixture per reason class (`string_trim_skipped`, `string_split_skipped`); `scripts/full_test.sh` skip-code coverage (`vc_skip_code_valid`) needs no new code (still `unmodeled-builtin`).

### Slice 10. Docs, ADR, differential
- `docs/adr/2026-10-08-1422-native-verification-semantics.md`: `Addendum (#1426)` — representation (A1/A2), literal bytes (A3), the quantified relation and its logic switch (A4), `byte_at` stricter semantics (A5), skipped table with reasons (A6), length invariant (A7), and the verdict-impact table from running `python3 scripts/verify_diff.py --filter string` (every String fixture row: class + explanation; `string_push_str_overflow` and `raw_parts_copy_*_beyond_model_cap` explained per A8).
- `docs/spec/contracts.md`: only the sentence(s) about String modelling if they describe caps/abstract `string_eq`; the full contracts.md rewrite is a separate epic child. `docs/spec/grammar.md` / `cli.md` / `errors.md`: **no change** (no language, CLI, or error-code surface change). No `generate_help.py` run needed; run `scripts/check_help_coverage.py` anyway.
- Add `tests/verify-native/{pass,fail}/` fixtures listed above (directive headers per neighbours, e.g. `vec_oob_index.vow`).

## Testing / verification commands
- Per slice: `build/vowc test compiler/tests/test_vc_str.vow` (and `--filter test_vc_`); needs `scripts/bootstrap.sh --skip-cargo` first if `build/vowc` is missing (not present in a fresh workspace). Bootstrap ≈5 min; run in foreground with explicit timeout or in chunks, never `pgrep -f` waits.
- Fixtures: `bash tests/verify-native/tests.sh` (wiring, fake bitwuzla) and the `tests/verify-native/{pass,fail,skip}` loop of `scripts/full_test.sh` Section 4g (real bitwuzla on PATH: 0.9.1 present).
- Differential: `python3 scripts/verify_diff.py --vowc build/vowc --filter string` (also `--command contracts`/`test` spot check); then full `python3 scripts/test_verify_diff.py`.
- Gates before PR: `scripts/bootstrap.sh --skip-cargo --no-cache` on the final head SHA (record SHA and the `scripts/seed.toml` pin if present), `python3 scripts/generate_operations.py --check`, `build/vowc test compiler/` (full), `cargo clippy --all --all-targets -- -D warnings` only if any Rust file is touched (none planned). Mind known pre-existing failures in memory notes (u64_marker_propagation, contracts_tmp_cleanup, concrete-block-region-parity).

## Verification surface
- Properties Bitwuzla must discharge: bounds claim `idx <u len` for `byte_at`; user `ensures` over `len`, bytes, `eq`; the `push_str`/`push_byte` no-overflow assumptions are *assumed*, not claimed (runtime trap path never returns).
- Quantified formulas appear only in functions with symbolic-length `eq`/`push_str`/`substring`; all others stay quantifier-free. Z3 cross-check (ADR native-z3-cross-check) must accept the new node — add a fixture to its corpus if it has a list.
- Fixtures to grow: `tests/verify-native/pass/string_{literal_len,clone_len,push_byte_byte_at,push_str_literal,eq_params,eq_literals,push_str_symbolic,fill_loop,branch_push,substring_len}.vow`; `fail/string_{byte_at_oob,eq_len_only,push_str_wrong_len,substring_wrong_len,clear_then_byte_at}.vow`; `skip/string_{trim,split}_skipped.vow`; opaque-parse fixtures in slice 8. No `tests/run/` change (runtime semantics unchanged).
- ESBMC fixtures are not edited. `tests/verify*/` C parity (`scripts/parity.py c`) unaffected: no change to `c_emitter.{rs,vow}`.

## Risks
- **Quantified queries (biggest).** `ALL` logic, solver time, `unknown`s, model extraction, incremental sessions, result cache keyed on canonical text, slicing, Z3 cross-check. Mitigation: slice 0 gate; QF fast paths for constants and identical terms; no quantifier unless symbolic operands; quantifier-free fallback is a follow-up issue, never an abstraction.
- **Trusted simplifier base** (ADR 2026-10-10-2041): `RANGE_EQ` must be inert in `vc_term`'s `vc_tm_app*` simplifications and `vc_bvfold`; add a test that `vc_tm_*` never rewrites through it.
- **Hash/equality of terms:** a 5-operand node must hash all operands (`vc_term_hash` mixes 5 fields incl. `aux`); wrong hashing = wrong hash-consing = unsound sharing. Unit-test two distinct `RANGE_EQ`s differing only in an offset.
- **Binary fixed point / bootstrap:** all `compiler/` changes must be deterministic (no HashMap iteration order; use the existing Vec/loop idioms). `build/vowc` verifies the compiler: the new `vc_str.vow` must itself pass `vowc verify` (no `vow_violation`s) and stay small.
- **Slot masking subtlety:** forgetting the `& 0xFF` on one read path (`byte_at`, `eq`, `matches_literal_at`, `RANGE_EQ`) silently makes parameter strings unsound for `byte_at <= 255`. Test each path with a parameter string.
- **Aliasing:** `push_str(s, s)` and two `String` params (distinct objects, as for Vec) — covered by tests; Phi-merged differing strings stay `unsupported-opcode`.
- **`ConstStr` handling in loops/inlining:** `vc_clone_inst` must preserve `ds` bytes when unrolling; literal bytes containing NUL or non-ASCII must survive (`ds` is a Vow `String`).
- **Verdict class drift:** `string_push_str_overflow` becomes native `proven` vs ESBMC refuted (class `soundness` in `verify_diff.py` because the corpus labels it failing). Harness exit code 1 for that row must be handled per #1423's precedent (ADR explanation); if `scripts/verify_diff.py` has no mechanism for explained rows, do **not** invent one in this PR — record it in the PR body and leave a follow-up issue.
- **Counterexamples:** String parameters are not reported in the counterexample (ESBMC doesn't either); replay of a string bounds failure is `skipped` (`// TEST: replay-skipped` directive, as Vec).
- **parse → print → parse idempotency and `vow-clif-shim` stack-slot layout / codegen ordering:** n/a — no parser, printer, lowerer, codegen or shim change; only `compiler/vc_*.vow` verifier modules, docs and fixtures.
- **Frame-marker confusion:** see A3b; a literal starting `__vc_frame:` must not be parsed as a frame.
- **Bound-variable capture:** the `RANGE_EQ` bound variable needs a name the declaration namer (`vc_agg_declare`, `vc_vec_fresh_name`, `pK_*`, `h<id>`) can never produce (e.g. `i!`), else it captures a constant; test it.
- **No Rust twin:** per ADR-1421 verifier exemption; do not touch `vow-*` crates. Anything outside the verifier (lexer/lowerer/runtime) is out of scope, so clippy gate is untouched.

## Out of scope
- `contains`, `starts_with`, `ends_with`, `split`, `trim`, `to_upper/lower`, `replace`, `join`, `hex_*`, `hash_str`, `parse_*`, `*_to_string`, `from_raw_parts_copy`, `_in_arena` variants (skipped with reasons; follow-ups).
- Quantifier-free encodings; Vec/String merged-by-Phi with differing values; `Vec<String>` element reads; maps.
- Counterexample reporting of string contents; replay of string failures.
- Changing `grammar.md` semantics, ESBMC emitter, `STRING_MODEL_CAP`, or editing fixture contracts/labels to hide divergence.
- Perf-gate work (P4/P5), the full `contracts.md` rewrite, refactors of `vc_vec.vow`/`vc_exec.vow` beyond what the slices need.
