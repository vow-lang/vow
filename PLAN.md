# Plan: report and replay aggregate parameters in native counterexamples (#1580)

## Goal
A failing native verification of a function with a struct, enum, `Option` or `Result` parameter lists that
parameter's field values in `values`, and `--replay-cex` rebuilds the argument from them so the
counterexample replays `confirmed`/`diverged` instead of `skipped`. Self-hosted only (native-verifier
exemption, ADR-2026-10-08-1421); no Rust twin.

## Assumptions
- Names are slot-based, `<param>.<slot>` (`o.0` tag, `o.1` payload; struct `p.0`, `p.1`), because the IR does
  not name a parameter's type. If one slot is read at two sorts in the function (i32 payload in one variant,
  bool in another) every symbol of that slot gets `:<sort>` (`o.1:bool`, `o.1:64`; the suffix is `bool` or the bit width, signedness is not part of it). Replay maps slots back to
  source fields from the AST. Source field names in `values` are out of scope (needs a type table in the IR;
  that is dual-compiler drift).
- Discriminant: replay rejects an out-of-range tag (`skipped` + reason). The model is NOT range-constrained:
  it needs the parameter's type, which the IR does not carry. Follow-up issue: IR param-type table, then
  constrain the tag in `vc_agg`. A spurious out-of-range counterexample is a pre-existing verifier
  imprecision (#1574), not a regression, and is never reported as `VerifierBug` (skipped never ran).
- Aggregate rebuilding is gated on the native flag (`abort_claims`/`native` in `replay_all`) so the ESBMC
  replay path and its Rust/self-hosted parity (`// TEST: replay skipped` fixtures) are untouched.
- 128-bit fields and any non-scalar field stay `skipped` with a reason (v1). Unread slots default to
  `0`/`false`/variant 0: a slot the model never read before the claim cannot influence the path to it.
- Display signedness of a field symbol comes from the IR type of the first `FieldGet` that declared it.
  A value whose text sign disagrees with the AST field type (same slot+width read as i64 and u64) is `skipped`.

## Key Files
| File | Role | Lines |
|------|------|-------|
| `compiler/vc_agg.vow` | records each lazily declared param field symbol (param, slot, sort, ty, header index) | `vc_agg_param_symbol` 148-157, `vc_agg_get` 162-185, `vc_agg_phi_read` 190-206, struct `VcAgg` 33-44 |
| `compiler/vc_exec.vow` | `VcClaims` model lists; harvest field symbols after the walk; per-claim name count | struct 85-106, `vc_claim_script_finish` 173-178, `vc_claims_step` 501-585, FIELD_GET 363-367 |
| `compiler/vc_cex.vow` | `vc_claim_result` lists aggregate fields, Bool formatting | 54-93 |
| `compiler/vc_native.vow` | passes names/widths/count of claim `k` to the solver and cache | 130-175 |
| `compiler/vc_solver.vow` | Bool literal in model parse | `vc_parse_value_at` 133-149 |
| `compiler/vc_fmt.vow` | `true`/`false` text for Bool symbols | `vc_format_int` 29-45 |
| `compiler/replay_main.vow` | already registered everywhere (`use`d by main, in both `concat_vow.sh` lists); gains the pure AST-layout lookup + argument-literal rendering, and receives `replay_is_int_literal`/`replay_render_scalar` moved from `main.vow:727,745` (a module cannot `use main`) | 1-64 |
| `compiler/main.vow` | thread `merged: Module` into replay; use `replay_main` | `replay_one_ce` 800-984, `replay_all` 1115-1150, `verify_finish` ~2036-2045, callers 1917, 2030, 2336 |
| `compiler/tests/test_vc_agg.vow`, `test_vc_exec.vow`, `test_vc_cex.vow`, `test_vc_solver.vow`, `test_vc_fmt.vow`, `test_replay_main.vow` | unit tests (`parse_src` helper at `test_replay_main.vow:10`) | - |
| `tests/verify-native/fail/*.vow` [new x4], `tests/verify-native/tests.sh` | fixtures + fake-solver wiring | fake solver `sat`/`sat_zero` 100-150 |
| `docs/spec/cli.md` | Aggregates bullet (line 72), "Counterexample replay > Native backend" (line ~519) | - |

## Steps (ordered TDD slices; each slice = failing test, then code)

### 1. Bool literals in the model parser
- **Test**: `compiler/tests/test_vc_solver.vow`: `vc_parse_model` with width `VC_SORT_BOOL()` (0) reads `((p0f1s0 true))`
  and `false` as value 1/0; a `#b1` or `(_ bv1 1)` at Bool width is rejected (`ok == 0`); a BV name still parses.
- **Code**: `vc_parse_value_at` (`compiler/vc_solver.vow:133`): when `width == 0`, match literal `true`/`false`
  (end check `vc_value_end_ok`), value 1/0.
- Note: `vc_agg_param_symbol` embeds the sort integer, so a Bool symbol is `p<k>f<idx>s0`; the fake solver regex must match `s[0-9]+`.

### 2. Format Bool values
- **Test**: `compiler/tests/test_vc_fmt.vow`: `vc_format_int(ITY_BOOL(), 1, 0) == "true"`, `0 -> "false"`.
- **Code**: `vc_format_int` (`vc_fmt.vow:29`): early return for `ty == ITY_BOOL()`. `vc_ty_bits(bool)` is 0 today.

### 3. Record param field symbols in `VcAgg`
- **Test**: `compiler/tests/test_vc_agg.vow`: after `vc_agg_get` on a param object at (idx 0, sort 64) and
  (idx 1, sort 0), `ag.ps_*` holds two entries with param index, slot, sort, ty, header index = position of the
  `declare-const` in the header; a second read of the same slot, and a Phi object over the parameter that reads the same slot, add no
  second entry (de-duplicated by (param, slot, sort)).
- **Code**: new vectors `ps_param`, `ps_slot`, `ps_sort`, `ps_ty`, `ps_hdr` on `VcAgg`; `vc_agg_param_symbol`
  appends (header index = `hdr.kinds.len()` before the declare, `ps_ty` = -1). Keep `vc_agg_get`'s signature:
  at `vc_exec.vow:364` (the only caller outside `vc_agg.vow`) record `ag.ps_param.len()` before the call and stamp
  `inst.ty` onto entries appended during it (avoids editing the 472-line `test_vc_agg.vow`).

### 4. Harvest into `VcClaims`, per-claim name count
- **Test**: `test_vc_exec.vow`: `vc_claims(f)` for a `Pair`-parameter function whose `ensures` reads `p.hi`:
  `cs.model_names` = `["p0f1s64"]` (declaration order, scalars first), `model_slots == [1]`, `model_hdr` ordered;
  claim 0's `vc_claim_model_count` covers it; an earlier claim (made before the first field read) does not.
  Two claims in different blocks that read different fields get different counts. Update the struct literal
  in `test_vc_cex.vow`/`claims_with` for the new fields.
- **Code**: add to `VcClaims`: `model_slots` (-1 for a scalar param), `model_hdr` (header index of the
  declaration), `model_ambig` (computed once). At the end of `vc_claims_step` (after the walk, before
  `replace_logic`) append `ag.ps_*` entries to `model_names/params/tys/widths` in declaration order.
  Add `vc_claim_model_count(cs, k)` = number of entries with `model_hdr[j] < header_lens[k]` (monotone: a scan).
  Reason: claim `k`'s query is only the first `header_lens[k]` header commands, so a `get-value` of a symbol
  declared later is a solver error.
- **Code**: `vc_claim_script_finish` (`vc_exec.vow:173`) takes the first `count` names for `get-value`.
  `vc_native.vow:132,149,175` pass `cs.model_names`/`cs.model_widths` truncated to the count
  (`vc_slice` queries keep every `declare-const` of the prefix, so names stay valid) and the cache peek uses
  the same count. Add a `vc_claim_model_names(cs, k)`/`..._widths` helper in `vc_exec.vow` for these.

### 5. Report field values
- **Test**: `test_vc_cex.vow`: `vc_claim_result` with a model of `[scalar p0, field p1 slot 0, field p1 slot 1 bool]`
  yields names `["x", "o.0", "o.1"]` and values `["7", "1", "true"]`; with two sorts at slot 1 the names are
  `o.1:64` / `o.1:bool` (suffix = `bool` or the decimal bit width, helper `vc_agg_sort_label`), and `ans.values`
  shorter than the model (claim earlier than later symbols) is handled.
- **Code**: `vc_claim_result` (`vc_cex.vow:54`): loop over `ans.values.len()`; name = `vc_source_param_name(f, k)`
  for a scalar, else `<param>.<slot>` (+ `:<label>` when `model_ambig[j]`); value via `vc_format_int(model_tys[j],..)`.
  `VerifyCE.var_names/var_values` flow unchanged into `values` JSON and the worker wire format (strings).

### 6. Replay argument rendering (`compiler/replay_main.vow`)
- **Test**: `compiler/tests/test_replay_main.vow`, building a small module with the `parse_src` helper
  pattern of `test_replay_main.vow:10`: 
  `struct Pair {lo: i64, hi: i64}` + `[p.0=3, p.1=-4]` -> `Pair { lo: 3, hi: -4 }`; `i128` field -> reason;
  `enum Shape {Circle(i64), Rect(i64, i64), Empty}` with `[s.0=1, s.1=5, s.3=6]` -> `Shape::Rect(5, 6)`;
  `s.0=2` -> `Shape::Empty`; `s.0=7` -> reason "discriminant 7 outside Shape (3 variants)"; absent tag ->
  variant 0 with defaults; `Option<i64>` `o.0=0` -> `Option::None`, `o.0=1,o.1=9` -> `Option::Some(9)`;
  `Result<i64, bool>` tag 0/1 -> `Result::Ok(..)`/`Result::Err(false)` (confirm Ok=0/Err=1 against the
  lowerer's built-in variant order, `lower.vow:4370-4460`, `checker.vow:1883`); `Option<Pair>` / `Vec<i64>`
  / `&Pair` / alias / unknown name -> reason; unsigned field with text `-1` -> reason;
  ambiguous slot picks `o.1:bool` for a bool payload and `o.1:64` for an i64 payload (suffix = `bool` / bit width).
- **Code**: `replay_agg_arg(m: Module, ty_tid: i64, pname: String, names, vals) -> ReplayArg {lit, reason, ty_text}`;
  helpers `replay_find_struct/enum(m, name)` over `m.items` (`item_kind`/`item_id`, `sdef_*`, `edef_*`;
  list layouts: fields 2 entries each, variants 2 entries each — see `decl_text.vow:115-160`); slot arithmetic
  `1 + sum(width)` for enums (tag slot 0), `sum(width)` for structs; scalar field types limited to
  `bool,i8..i64,u8..u64`. Type text via `print_type_text` (`contract_text.vow:22`). Uses `replay_render_scalar`
  semantics (move it or re-export if it lives in main.vow; keep one definition).
- Move `replay_is_int_literal` and `replay_render_scalar` from `main.vow` into `replay_main.vow` (main already
  `use`s it); no module registration change is needed.

### 7. Wire into the harness
- **Test**: `test_replay_main.vow`/`tests.sh` wiring (slice 9) cover it end to end; keep the existing scalar
  skip test unchanged for the non-native path.
- **Code**: `compiler/main.vow`: `replay_one_ce` gains `m: Module`; in the parameter loop, for a `ITY_PTR`
  param when `abort_claims` (native) call `replay_agg_arg` on the AST parameter (find the `fn` item by
  `ce.function`; param type tid = `list_get(a, params_lid, i*4+1)`) and emit
  `let __replay_aN: <type text> = <lit>;`. On a reason, `replay_skip(reason)`. Scalars and the non-native path are
  unchanged. Plumb `merged` via `replay_all` and `verify_finish` (callers 1917, 2030 pass `fr.merged`; the
  ESBMC build-path call at 2336 passes `fr.merged` too, native=false). Update the stale header comment
  (`main.vow:670-673`) and the `replay_one_ce` unsupported-type message stays for non-native.

### 8. Fixtures (real solver, `tests/verify-native/fail/`)
**Fixture rules.** (a) Every function needs a `vow` block, otherwise it is `pre_only` and the unwrap/arith claims
are never asked (mirror `fail/unwrap_none.vow`: `[panic] vow { ensures: result == result }`). (b) The model leaves
an enum tag unconstrained, so the solver may return an out-of-range tag, which replay skips and the `fail/`
harness rejects. Put each bug in an arm entered only by an exact in-range tag compare, never in a fall-through
arm; before writing the enum/`Result` fixtures read how `match` lowers (`lower.vow`, match lowering: compare chain
vs explicit compares + `unreachable`) and which `Result` tag is compared (`Ok` = 0). Run each fixture 3x against
real bitwuzla; if the out-of-range tag can still violate, restrict the fixture to a shape where it cannot or
mark `replay-skipped` with a reason and keep the fake-solver coverage of slice 9.
Each carries `// TEST: counterexample-fn/vow-id/blame/violation` and `// TEST: counterexample-value <name> <v>`;
the harness already requires `--replay-cex` => every counterexample `confirmed`.
- `struct_param_wrong_post.vow`: `fn span(p: Pair) -> i64 ensures result == p.lo` with body `p.hi - p.lo`;
  values `p.0`/`p.1` listed.
- `option_param_unwrap_none.vow`: `fn get(o: Option<i64>) -> i64 [panic]` `o.unwrap()`; abort claim;
  `counterexample-value o.0 0`; replay builds `Option::None` and confirms `UnwrapOnNone`.
- `enum_param_wrong_arm.vow`: `Shape` param, `ensures` false for one arm; value `s.0` is the tag.
- `result_param_wrong_post.vow` (Ok/Err with bool payload to exercise Bool parse and the ambiguous-slot name).
- Existing `// TEST: replay-skipped` fixtures (15 files, grep `tests/verify-native/fail`) have only scalar, `Vec`,
  or 128-bit parameters (structs/enums appear only as return types or with `i128` fields), so none flips to
  `confirmed`; re-run the native fail loop to confirm. `wide_struct_field_overflow.vow` and
  `wide_enum_payload_width.vow` stay `skipped` (128-bit field).

### 9. Fake-solver wiring (`tests/verify-native/tests.sh`)
- Extend the fake `sat`/`sat_zero` regexes to answer `p<k>f<idx>s<sort>` (BV and Bool `true`/`false`,
  sort 0) so `get-value` of field symbols parses; add `sat_tag_some` (answers tag fields with `1`), and
  `sat_tag_9` (tag `9`).
- Cases: (a) `Option<i64>` param + unwrap, `sat_zero`: `replay == confirmed`, no `VerifierBug`;
  (b) same source under `sat_tag_some` ("deliberately wrong model value"): `replay == diverged`,
  `diagnostics.1.error_code == VerifierBug`; and a `Pair` struct param whose `ensures` fails under `sat_zero`
  (`hi - lo == 0 == lo`) diverges into `VerifierBug`; (c) enum param under `sat_tag_9`: `replay == skipped`,
  reason mentions the discriminant, no `VerifierBug`; (d) `values` of (a) contains `o.0`;
  (e) cache: a second run on the same cache dir still lists the field values (`no_leftovers` checks).

### 10. Docs
- `docs/spec/cli.md:72` Aggregates bullet: replace "A struct or enum parameter has no counterexample values
  yet" with the `<param>.<slot>` naming (tag at `.0` for enums/`Option`/`Result`, payload from `.1`, struct fields
  from `.0`, 128-bit takes two slots, `:<sort>` suffix on an ambiguous slot).
- "Counterexample replay > Native backend" (~519): struct/enum/`Option`/`Result` parameters of scalar fields are
  rebuilt; `skipped` reasons (nested aggregate, 128-bit field, reference/alias, out-of-range discriminant).
  Fix the "v1 input scope" sentence so the native/ESBMC difference is stated.
- `docs/spec/schemas/counterexample.schema.json` `values` description: add the `<param>.<slot>` naming (schema is
  `additionalProperties: string`, no pattern, so no structural change).
- Then `uv run python scripts/generate_help.py` (embedded `--help`/skill in both compilers), commit the
  regenerated files; `python3 scripts/check_help_coverage.py`.

## Testing / verification commands
- `build/vowc test compiler/tests/test_vc_agg.vow` (and `test_vc_exec`, `test_vc_cex`, `test_vc_solver`,
  `test_vc_fmt`, `test_replay_main`, `test_replay_main`); then `build/vowc test compiler/`.
- `VOWC_BIN=build/vowc tests/verify-native/tests.sh` (fake solver; needs no bitwuzla).
- With bitwuzla on PATH: `build/vowc verify --backend native --no-cache --replay-cex tests/verify-native/fail/<new>.vow`.
- Run `scripts/bootstrap.sh --skip-cargo --no-cache` on the final head SHA, record SHA + seed pin
  (CLAUDE.md "green locally"); `scripts/full_test.sh` Sections 2c (C parity untouched), 4c, native fail loop
  (`full_test.sh:1498-1545`).
- Run bootstrap in the background with a done-marker file and poll; never `pgrep -f`.

## Verification surface
No contract, codegen, or C-emitter change; ESBMC proves nothing new. New `.vow` code in `compiler/` is
verified by bootstrap's Stage 1 verification (all new functions need honest `vow` blocks only where they
already have them elsewhere; pure helpers without contracts are fine). No `tests/run/` or `examples/` growth.

## Risks
- **Header-prefix mismatch** (highest): `get-value` of a symbol declared after claim `k`'s prefix makes the
  solver return an error => `VC_ANS_ERROR`, a wrongly "unknown" function. Mitigated by slice 4's per-claim count;
  test with two claims in different blocks. Also covers the incremental session and sliced paths (declares
  are never sliced away; only `define-fun`s are).
- **Cache**: `vc_cache_decode` validates `n_names`; use the per-claim count in `vc_cache_peek`/store. The
  `get-value` line is part of the query text, so entries written before this change cannot be confused.
- **Fake solver** in `tests.sh` returns values only for `p[0-9]*`; any existing fake-`sat` test over an
  aggregate parameter would turn `error`. Grep `tests/verify-native` and the `contracts/` fixtures before
  extending, and keep the fake's `p<k>` pattern for scalars.
- **Bool parse**: Bitwuzla prints Bool as `true`/`false`; `--bv-output-format 2` does not affect it.
- **Determinism / fixed point**: harvest order = declaration order (no HashMap); no codegen change beyond
  new `.vow` functions, so the bootstrap fixed point only needs a normal rebuild.
- **Rust parity**: native verifier is exempt, but `replay_one_ce` is shared with the ESBMC path; keep the
  aggregate branch behind the native flag so the `// TEST: replay skipped` fixtures and Rust parity hold.
- **Struct literal sites**: every `VcClaims { .. }` / `VcAgg { .. }` literal (src and tests) gains fields.
- **Field-name vs. variable-name collision** in `values` (a param named `o` plus field names `o.0`): impossible,
  identifiers contain no `.`.
- **Replay type text**: a type from a `use`d module or an alias is not rebuildable by name => harness
  compile failure is already `skipped`; resolve eagerly and skip with a reason where the name is not an item
  of the merged module.

## Gate checklist (all N/A, stated for the record)
- `parse -> print -> parse` idempotency: no syntax change; the harness is generated source.
- Verifier C parity (`c_emitter.rs`/`.vow`): untouched; `full_test.sh` Section 2c stays green.
- `cargo clippy --all --all-targets -- -D warnings`: no Rust change.
- `vow-clif-shim` stack-slot layout / `BTreeMap` ordering: untouched; new Vow code uses `Vec`s only.

## Out of scope
- Constraining the discriminant range in the model (needs IR param types; follow-up issue to file).
- Source field names in `values`; 128-bit fields, nested aggregates, `Vec`/`String` parameters in replay.
- Rust compiler/ESBMC replay changes; any change to `c_emitter.*` or the proof itself.
- Refactors of `main.vow` replay code beyond threading `merged`.
