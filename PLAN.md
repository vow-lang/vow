# Plan: native verifier models 128-bit struct fields and enum payloads (#1421)

## Goal
Make `vowc verify --backend native` prove and refute contracts over `i128`/`u128` struct fields and
enum/`Option`/`Result` payloads, and delete the transitional `wide-aggregate-field` skip code (the
issue's "`i128-aggregate` skip reason"). Native-verifier-only work in `compiler/vc_*.vow`
(CLAUDE.md "Scoped exception: the native verifier": no Rust twin; `c_emitter.{rs,vow}` untouched).

## Assumptions
- **Compiler support is confirmed, no blocking feature issue.** #1573 (`b2873949`) lowers a 128-bit
  field/payload as `FieldGet`/`FieldSet` with `dk == IDATA_WIDE_SLOT`, `dv` = first of two 8-byte
  slots (`lower_field_dk`, `lctx_struct_field_slot`, `lower_payload_field_dk` in `compiler/lower.vow:1133-1170`),
  later fields shifted up by one. Runtime/debug fixtures exist (`tests/run/wide_struct_field*.vow`,
  `wide_enum_payload.vow`, `wide_option_routes.vow`, `tests/debug/wide_*`). Step 0 re-confirms on this
  tree and records it in a `gh issue comment` (the AC "compiler support is confirmed").
- **"`i128-aggregate`" = the shipped code `wide-aggregate-field`** (`compiler/vc_ops.vow:21`, ADR-1430
  table row). No string `i128-aggregate` exists in the tree.
- **`Vec<i128>` elements stay `Skipped`, as `unmodeled-builtin`, and are not part of this PR.** The
  native verifier has no `Vec` model at all (epic #1398 P3 "Vec/String array+length model", separate
  ticket; `catalogue_verifier_known` returns false for everything, `vc_inst_supported` accepts only
  the unwrap-abort `Call`). A wide element is read through `__vow_vec_get_wide_ptr` (a `Call` returning
  `ptr`) before the `WIDE_SLOT` `FieldGet`, so it is already stopped by the call, never by the field
  op. Removing the field-width code therefore cannot let a Vec element through. Modelling it belongs to
  **#1423** (`feat(verify): Vec as array plus symbolic length`, open, not blocked by #1421, silent on
  128-bit elements). Step 0 posts a `gh issue comment` on #1423 handing it that scope: wide element
  access arrives as `__vow_vec_{get,set,push}_wide_ptr` + `WIDE_SLOT` `FieldGet`/`FieldSet` (dv 0) on the
  call's pointer, and the gate this PR relaxes accepts `WIDE_SLOT` ops on any `ptr`, so #1423 must model
  the call and give the returned pointer an object. ADR-0001 line ~185 ("skips these accesses until
  #1421 models them") is corrected in step 5.
- **Vow-block bindings never read wide fields.** `binding_is_primitive_field`
  (`compiler/lower.vow:5664`) lists only `i32/i64/f32/f64/bool`, and `emit_binding_field_get`
  (`:5670`) emits `FieldGet[i64]` with `IDATA_FIELD`; a wide field is dropped from the binding list, so
  contracts mentioning `result.value` read it through the ordinary `WIDE_SLOT` path, and a narrow field
  after a wide one is already covered by `tests/verify-native/pass`'s sibling `wide_struct_narrow_field_slot`.
- Slot model keyed `(idx, sort)` with `sort` = bit width already yields sort 128 (`vc_ty_sort`,
  `compiler/vc_int.vow:33`); a 128-bit value is **one** BV128 term occupying slots `idx` and `idx+1`.
  No `hi`/`lo` split in the verifier.

## Key Files
| File | Role | Lines |
|------|------|-------|
| `compiler/vc_gate.vow` | gate: `vc_is_field_ty`, FIELD_GET/SET arms of `vc_inst_supported`, `vc_field_ty`, specific-reason branch | 130-134, 200-206, 241-267 |
| `compiler/vc_agg.vow` | slot store; `vc_agg_set` must invalidate overlapping slots | 90-126 |
| `compiler/vc_exec.vow` | `vc_walk_agg` FieldSet/FieldGet (works unchanged; sort comes from `vc_ty_sort`) | 266-297 |
| `compiler/vc_ops.vow` | `VC_SKIP_WIDE_AGGREGATE_FIELD`, `vc_skip_code_valid`, `VC_OPM_FIELD`, `vc_op_modeled_core`, `vc_op_gate_candidate` | 21, 24-35, 43-44, 66-75, 88-113 |
| `compiler/tests/test_vc_gate.vow` | wide cases 83/85 expect the old code | 444-456 |
| `compiler/tests/test_vc_agg.vow` | case 31 expects the old code; add exec/slot tests | 119-125 |
| `compiler/tests/test_vc_ops.vow` | classes-disjoint count, `VC_OPM_FIELD` check 603, code-valid 808 | 27, 43, 71 |
| `compiler/tests/test_vc_exec.vow` | add wide field query tests (helper style at 495-650) | |
| `compiler/tests/builders.vow` | `mk_inst_field_get/set` hard-code `IDATA_FIELD` (lines 120-131); add `*_wide` variants | |
| `tests/verify-native/{pass,fail,skip}/` | end-to-end fixtures; harness `scripts/full_test.sh:1378-1510` | |
| `tests/verify-native/skip/wide_field_skipped.vow` | delete (its reason disappears) | |
| `docs/spec/cli.md` | lines 62, 70 (native subset/aggregates), 322 (ESBMC row stays) | |
| `docs/spec/grammar.md` | 396-410 "Every 128-bit aggregate access is `Skipped`" | |
| `docs/adr/2026-10-08-1430-native-verifier-cli-and-status-surface.md` | row 161, paragraphs 225-240 | |
| `docs/adr/2026-10-08-1422-native-verification-semantics.md` | Rule 4 sentence (~line 147), bullet 204 | |
| `docs/adr/0001-numeric-tower-narrow-ints.md` | line ~185 "until #1421 models them" | |
| generated: `compiler/main.vow`, `vow/src/skill.rs`, `skills/vow/reference/{cli,grammar}.md` | regenerate with `scripts/generate_help.py`; never hand-edit | |

## Steps (TDD slices, each a separate commit; conventional-commit subjects lower-case)

### 0. Confirm compiler support (no code)
- Build `build/vowc` (`scripts/bootstrap.sh --skip-cargo` if stale). For a scratch struct
  `{a: i64, b: i128, c: u64}`, an `enum E { V(i128), W(i64, i128), N }` and `Option<i128>`/`Result<i128,i64>`:
  `build/vowc build --no-verify --dump-ir` and `build/vowc build --mode debug` + run. Check: `FieldGet`/`FieldSet`
  of type `i128`/`u128` carry `IDATA_WIDE_SLOT`; **a struct field initialised with a literal
  (`Wide { value: 5 }`) is stored as `ConstI128`, not an `i64` const** (if not, the verifier would read a
  sort-64 write at sort 128 and report a false counterexample; that would be a lowerer gap -> file a
  `feat`/`fix` issue per the epic's stop rule and mark #1421 blocked by it).
- Also dump the IR of a *contracted* function (`ensures: result.value == x`, `ensures: result.c == c` on a
  `{a: i64, b: i128, c: u64}` struct) and inspect every `FieldGet`: wide reads must be `WIDE_SLOT`, the
  narrow `c` read `FIELD` at the shifted slot. If a wide field is read with `IDATA_FIELD` or as `i64`
  anywhere, that is a lowering gap (outside the verifier exemption: Rust twin, C parity in play): stop,
  file a `fix`/`feat` issue, mark #1421 blocked by it.
- `gh issue comment 1421` with the result and the Vec-element decision; `gh issue comment 1423` with the
  hand-off described in Assumptions.

### 1. Gate accepts width-consistent wide field ops and the old code is retired (RED -> GREEN)
- **Tests** `compiler/tests/test_vc_gate.vow` (`check_unsupported_aggregate_shapes`, replace 83/85 and
  `test_vc_agg.vow` 31), using new builders `mk_inst_field_get_wide` / `mk_inst_field_set_wide`
  (`IDATA_WIDE_SLOT`) in `compiler/tests/builders.vow`:
  1. `FieldGet[i128|u128]` + `WIDE_SLOT` on a ptr param -> `""`.
  2. `FieldSet` of an `i128` value + `WIDE_SLOT` -> `""`.
  3. `FieldGet[i128]` + `IDATA_FIELD` -> `unsupported-opcode: FieldGet[i128]` (codegen already treats this as
     an internal error, `CLIF_ERR_WIDE_AGGREGATE_FIELD`; IR with it is never well-formed).
  4. `FieldGet[i64]` + `WIDE_SLOT`, `FieldSet` of an `i64` value + `WIDE_SLOT` -> generic `unsupported-opcode`
     (width and slot kind must agree; this is what keeps a mis-widthed store from being modelled).
  5. Existing: `FieldGet[ptr]` nested and `FieldGet[i64]` on a non-ptr stay rejected.
- **Code** (one commit, because the old code's specific-reason branch would otherwise shadow test 3):
  `compiler/vc_gate.vow`: replace `vc_is_field_ty(ty)` by `vc_field_access_ok(dk, ty)`:
  `dk == IDATA_FIELD()` -> Bool or int <= 64 bits; `dk == IDATA_WIDE_SLOT()` -> `ITY_I128()`/`ITY_U128()`.
  Use it in the `IOP_FIELD_GET` (on `inst.ty`) and `IOP_FIELD_SET` (on `t1`) arms; keep the
  `n`, `t0 == ITY_PTR()`, `dv >= 0`, `inst.ty == ITY_UNIT()` checks. Update the comment at line 130.
- **Retire the code in the same commit**: `compiler/vc_ops.vow`: delete `VC_SKIP_WIDE_AGGREGATE_FIELD`
  and its `vc_skip_code_valid` clause; move `IOP_FIELD_GET`/`IOP_FIELD_SET` into `vc_op_modeled_core`;
  delete `VC_OPM_FIELD`, the `FIELD` arm in `vc_op_model`, and its term in `vc_op_gate_candidate` (leave
  `VC_OPM_FLOAT_REM() == 4` numbering alone). `compiler/vc_gate.vow`: delete `vc_field_ty`, the
  `VC_OPM_FIELD` branch of `vc_inst_specific_reason`, and fix the comments at 247 and 300.
  `compiler/tests/test_vc_ops.vow`: drop the line-27 term, make 603 expect `VC_OPM_MODELED`, make 808
  assert `!vc_skip_code_valid(String::from("wide-aggregate-field"))`.
  Guard: `rg -n "wide-aggregate-field|WIDE_AGGREGATE_FIELD|VC_OPM_FIELD" compiler/vc_*.vow compiler/tests`
  is empty (the `CLIF_*WIDE_AGGREGATE_FIELD` codegen error is unrelated and stays).

### 2. Executor + slot store model a wide slot exactly (RED -> GREEN)
- **Tests** `compiler/tests/test_vc_exec.vow` / `test_vc_agg.vow`:
  1. alloc; `FieldSet` WIDE slot 1 with `ConstU128`; `FieldGet` slot 1; `ensures result == const` ->
     query declares/uses `(_ BitVec 128)`, no `j` junk const (the read returns the stored term).
  2. parameter ptr; `FieldGet[i128]` slot 1 twice -> one declared `p0f1s128` (cached), and a `FieldGet[i64]`
     at slot 3 declares `p0f3s64` (shifted narrow slot independent).
  3. Overlap: after `FieldSet(slot 2, i64)` then `FieldSet(slot 1, i128)` a `FieldGet[i64]` of slot 2 is a fresh
     unconstrained term (not the stale value); symmetric: write at slot 2 invalidates cached `(1, 128)`.
  4. `Phi` over two constructed objects with a wide payload (the `Some(x)`/`None` shape) -> `ite` over the BV128
     slot terms, the other arm's slot unconstrained.
- **Code** `compiler/vc_agg.vow` `vc_agg_set`: a slot of sort 128 occupies `[idx, idx+1]`, any other sort
  `[idx]`. On write, reset (`sl_term = -1`) every entry in the object whose range intersects the written
  range, other than the exact `(idx, sort)` entry (which `vc_agg_store` then overwrites). Add a small
  `vc_agg_slot_width(sort)` helper. `vc_exec.vow` needs no change (`vc_ty_sort` already maps 128-bit types
  to sort 128); verify rather than assume via the tests above.
- Update the header comment of `vc_agg.vow` (slots may span two indices).

### 3. End-to-end native fixtures (RED -> GREEN, no new production code expected)
Add under `tests/verify-native/` (directives per `full_test.sh` Section 4g; fail fixtures need
`counterexample-fn/vow-id/blame`, and `// TEST: replay-skipped 128-bit parameter` when a counterexample
parameter is 128-bit; struct/enum params have no listed values):
- `pass/wide_struct_roundtrip.vow` (the old skip fixture: `Wide{value:x}.value == x`, `i128` and `u128`).
- `pass/wide_struct_mixed_slots.vow`: `{a: i64, b: i128, c: u64}`; reading `c` and `a` from a parameter and from
  a constructed value proves slots are not conflated (`result.c == c`, `result.b == y`).
- `pass/wide_param_field.vow`: struct param `{lo: i64, v: u128}`; `requires: p.v > 0`, `ensures: result > 0`
  with a field read (parameter fields are unconstrained symbols).
- `pass/wide_enum_payload.vow`: `enum Wide { Value(i128), Empty }` and `Pair { Both(i64, i128), Empty }`
  `match`, and a `Result<i128,i64>` parameter `match` (migrate `low_limb`, `second_limb`, `ok_limb` of
  `tests/verify-skip/wide_enum_payload_skipped.vow`). Do **not** migrate `call_ret_limb`/`passthrough`:
  a user call is `Skipped` until inlining (#1416), so it would turn the whole file `Skipped`. Add
  `Option<u128>` `Some(x).unwrap()` mirroring `pass/unwrap_some.vow` (`[panic]` effect on the function
  and `main`, native-only comment).
- `fail/wide_struct_field_overflow.vow`: `build(x: i128) -> Wide { Wide{value: x + 1} }`,
  `ensures: result.value > x`; counterexample value `x 170141183460469231731687303715884105727`
  (cf. `fail/i128_overflow_post.vow`).
- `fail/wide_struct_swapped_fields.vow`: `{a: i128, b: i128}` built `{a: y, b: x}`, `ensures: result.a == x`;
  two `i128` params so `// TEST: replay-skipped 128-bit parameter`.
- `fail/wide_enum_payload_width.vow`: build the enum inside the function from an `i128` scalar parameter
  (like `fail/enum_wrong_arm.vow`: `if k == 0 { Wide::Value(v) } else { Wide::Empty }`), then
  `match { Value(x) => if x > 9223372036854775807 {1} else {0} }` with `ensures: result == 0` must be
  `VerifyFailed`; it would wrongly prove if the payload were modelled at 64 bits (width-erasure
  regression guard, see the old comment in `wide_enum_payload_skipped.vow`). `replay-skipped 128-bit
  parameter` applies.
- `skip/wide_vec_element_skipped.vow` with `// TEST: skip-reason unmodeled-builtin` (confirm the exact
  code by running it; documents the Vec boundary). `git rm tests/verify-native/skip/wide_field_skipped.vow`.
- Keep 128-bit `*` out of fixtures (BV128 multiply is slow); use `+`, `-`, comparisons.
- Leave `tests/verify-skip/wide_*` (ESBMC, `category unverifiable`) as they are: ESBMC still skips them.

### 4. Docs/spec, ADR, generated help
- `docs/adr/0001-numeric-tower-narrow-ints.md` line ~185: replace "The verifier still skips these accesses
  until #1421 models them" with the true state (struct fields and enum payloads modelled by #1421 under
  `--backend native`; `Vec` elements wait for the Vec model, #1423). Add to Key Files.
- `docs/spec/cli.md`: native "Subset" (line 62) drop `wide-aggregate-field` from the code list; "Aggregates"
  (line 70): "each field is its own `bool` or integer term (a 128-bit field is one 128-bit term)", remove
  "`wide-aggregate-field` for 128-bit" from the Skipped list, state that a `Vec<i128>` element is
  `Skipped` as `unmodeled-builtin` like any collection; "Extra proofs" (line 69): add 128-bit aggregates.
  Line 322 (ESBMC) stays: ESBMC behaviour is unchanged.
- `docs/spec/grammar.md` 396-410: say the ESBMC backend skips 128-bit aggregate access and
  `--backend native` models struct fields and enum payloads; Vec elements stay skipped.
- ADR-1430: remove row 161 and the code from the two lists at 225-240, with a dated addendum
  "(issue #1421): code removed, 128-bit aggregates modelled". ADR-1422: addendum to Rule 4 and the
  "stay gated" bullet (ADRs are append-only records, do not rewrite their original text).
- `uv run python scripts/generate_help.py`, then `uv run python scripts/check_help_coverage.py`.
  Regenerated files must be committed with the doc change (they carry the cli.md text).

## Testing / Verification surface
- Unit: `build/vowc test compiler/tests/test_vc_gate.vow`, `test_vc_agg.vow`, `test_vc_exec.vow`, `test_vc_ops.vow`
  (or `build/vowc test compiler/ --filter vc_`).
- End to end: `build/vowc verify --backend native --no-cache <fixture>` per fixture, then
  `VOW_FULL_TEST_SKIP_CARGO=1 scripts/full_test.sh` Section 4g (needs `bitwuzla` on PATH; `skip` fixtures
  run without it). Fail fixtures also run `--replay-cex` in that harness.
- ESBMC obligations: none new. Query shape: QF_BV, `(_ BitVec 128)` for wide slots; obligations are the
  `ensures` clauses and abort sites already emitted (`.unwrap()`, checked arithmetic). Contracts in
  fixtures are the true semantic ones (no capacity/unwind bounds).
- Gates: `scripts/bootstrap.sh --skip-cargo --no-cache` (fixed point; vc_* functions are compiled and
  verified by stage 0, so keep new helpers simple integer code with no contracts) and
  `cargo build --release -p vow` (regenerated `vow/src/skill.rs`), `cargo clippy --all --all-targets -- -D warnings`.
  Record the head SHA (and, once it exists, the `scripts/seed.toml` pin) next to any "green locally" claim.
  Gate wall-clocks: bootstrap ~5 min, `full_test.sh` ~40 min: run foreground with explicit bounds.

## Risks
- **Soundness of slot aliasing**: a wide write covers two slots; stale narrow terms at `idx+1` (or a wide term
  at `idx-1`) must not survive. Mitigated by step 2 overlap invalidation + tests; the lowerer writes each slot
  once, so this is defence in depth, not a reachable bug today.
- **Width/kind mismatch silently modelled**: gate requires `WIDE_SLOT` iff 128-bit; a wide enum payload
  mis-lowered as `i64`/`IDATA_FIELD` is undetectable from IR, so `fail/wide_enum_payload_width.vow` pins the
  lowering (it already guards `compiler/lower.vow:1133`).
- **False counterexample if a literal reaches a wide field as a narrow const**: caught by gate rule (skip,
  not wrong verdict); step 0 checks whether the lowerer emits `ConstI128`.
- **Closed reason list**: removing a code must be reflected in ADR-1430 (it says additions require an
  amendment; removal is the documented "Transitional" lifetime). Docs-vs-help drift is caught by
  `check_help_coverage.py` and `generate_operations.py --check`.
- **Parity / fixed point**: no change to `c_emitter.{rs,vow}`, lowering, or codegen, so byte-identical C and
  the binary fixed point are unaffected; Section 2c must stay green unchanged. Generated help text edits
  change `compiler/main.vow` and `vow/src/skill.rs` consistently (same generator).
- **Counterexample display**: struct/enum parameters list no `values` yet (cli.md:70); wide fields inherit that.
  A fail fixture's reported values come from scalar parameters only.
- **Cost**: BV128 `*`/`/` can be slow in Bitwuzla; fixtures avoid them. No timeout tuning.

## Out of scope
- Any Vec/String array+length model, `__vow_vec_*_wide_ptr` modelling, and `parse_i128`/`parse_u128`
  (calls stay unmodeled builtins).
- ESBMC / `c_emitter` changes (stays `Skipped` at 128-bit width until P6 deletes it); Rust compiler logic.
- Counterexample values for aggregate parameters; nested aggregates; field writes outside the allocating block.
- Refactors of `vc_gate.vow`/`vc_agg.vow` beyond the lines above; formatting; `docs/verifier-eval.md` rewording.
