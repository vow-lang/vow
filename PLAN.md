# Plan: model `parse_f64_bits` / `format_f64_bits` in the native verifier (#1646)

## Goal
Make both hooks `verifier_model: known` and encoded by the symbolic executor: exact 64-bit vector on the `u64`
side, a *function of the string* on the `String` side (so `parse(s) == parse(s)` is proven, never refuted), opaque
otherwise. Native verifier only (`compiler/vc_*.vow`): no Rust twin, no C-emitter change (`c_model` stays
`unmodeled`, so the C-parity rule is untouched; CLAUDE.md "Scoped exception: the native verifier").

## Assumptions
- Blocker #1426 has landed (072c8e8e: `String` = `(array, length)` slot, exact `vc_str_eq`); the plan builds on it. (read from git log)
- Encoding = fresh term per call + **pairwise consistency axioms** (Ackermann) built on `vc_str_eq`, not an SMT
  uninterpreted function. A string's array has junk high bits and junk past `len`, so array-level congruence would
  not make equal strings agree. ADR-1422 line ~124 ("no uninterpreted function") is about `string_eq` exactness
  only; the Known-limits entry already calls the String side an "uninterpreted relation". (advisor)
- The model is an over-approximation of the runtime's text conversion, so a `sat` whose sliced query contains a
  hook term may not be a real failure. ADR Known limits promise such properties are `unknown`. Policy: such a `sat`
  is re-checked (independence re-check, step 6) and reported `failed` only if the claim fails for *every* hook
  value; otherwise `unknown` with reason `opaque builtin result`. (advisor; best guess on the reason text)
- No round-trip axiom (`parse(format(b)) == b` is false for NaN payloads) and no length/content constraint on
  `format`'s result beyond the `Vec` representation invariant. Real `parse_f64_bits` returns the bits of `0.0` for
  non-float text; deliberately not modelled (opaque).
- `__vow_*_in_arena` variants stay unmodelled (existing `vc_str_unmodeled_reason` rule; the verifier sees the
  pre-region IR).

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `docs/spec/operations.json` | flip both entries to `known`, drop `verifier_skip_reason` | 659-683 |
| `compiler/vc_ops.vow` | generated `catalogue_verifier_known` block (regenerate, do not hand-edit); `vc_vec_op_kind`, `vc_frame_in_dv2` | 237-261, 108-124 |
| `compiler/vc_strops.vow` | add kinds 21/22, symbol lookup (symbols lack the `__vow_string_` prefix), creates / no-object-operand / reads-bytes / is-string-op predicates | 14-117 |
| `compiler/vc_str.vow` | new pure encoders (or a new module `vc_hooks.vow`, see step 3) | 36-120 |
| `compiler/vc_exec.vow` | `vc_walk_vec` (format: before the `args[0]` slot lookup), `vc_walk_str` (parse) | 432-535 |
| `compiler/vc_gate.vow` | `vc_vec_call_ok` (format has a `u64` operand), `vc_str_call_ok` | 220-270, 305-310 |
| `compiler/vc_vec.vow` | `VcVec` hook-call registry fields; `vc_vec_shape_detail` aliasing | 36-80, 280-373 |
| `compiler/vc_native.vow` | independence re-check after a hard `sat` | 130-200, 300-340 |
| `compiler/vc_solver.vow`, `compiler/vc_cex.vow` | model values of scalar params for pinning | cex param tables |
| `scripts/concat_vow.sh` | add `vc_hooks` to the FILES list only if a new module is created | line 23 |
| `docs/adr/2026-10-08-1422-native-verification-semantics.md` | #1415 "Hooks" bullet (560-564), Known limits (191-193), new `#1646` addendum | |
| `docs/spec/contracts.md` (l.45), `docs/spec/cli.md` ("Floats", Subset l.63), `docs/spec/errors.md` if it lists the skip | drop "stay Skipped" for the hooks; document the `unknown` reason | |
| `skills/vow/reference/{cli,contracts,errors}.md`, `compiler/main.vow` embedded help | regenerated mirrors (`scripts/generate_help.py`) | |
| `tests/verify-native/{pass,fail,unknown,skip}/` | fixtures | `skip/float_bits_hook_skipped.vow` |
| `compiler/tests/test_vc_{ops,str,gate,exec,solver}.vow`, `scripts/test_generate_operations.py` | unit coverage | `test_vc_ops.vow:57-80`, `test_generate_operations.py:612-623,2462,2493` |

## Steps (ordered; each a TDD slice, see Testing)

### 1. Catalogue + op table: both hooks are known kinds
- `compiler/vc_strops.vow`: `VC_STR_PARSE_F64() -> 21`, `VC_STR_FORMAT_F64() -> 22`; map `__vow_parse_f64_bits` /
  `__vow_format_f64_bits` in `vc_str_op_kind` (they fail the `__vow_string_` prefix guard, so test the two symbols
  before it); `vc_str_kind_is_string_op` upper bound -> 22; `vc_str_kind_creates` and
  `vc_str_kind_no_object_operand` include FORMAT (its operand is a `u64`, no object); `vc_str_kind_reads_bytes`
  includes PARSE (else `vv.content` stays false and `vc_str_eq` has no byte facts, making the consistency axiom vacuous);
  `vc_vec_kind_mutates` unchanged (neither mutates).
- `docs/spec/operations.json`: `"verifier_model": "known"`, remove both `verifier_skip_reason` lines
  (the generator rejects a skip reason on a known entry). Run `python3 scripts/generate_operations.py` to regenerate
  `catalogue_verifier_known` in `compiler/vc_ops.vow` and `--check`.
- `vc_frame_in_dv2` unchanged: neither hook has a claim.

### 2. Gate
- `compiler/vc_gate.vow` `vc_vec_call_ok`: FORMAT is handled in the `NEW`/`FROM_RAW_PARTS` group *before* the
  `t0 != ITY_PTR()` rejection: `n == 1 && inst.ty == ITY_PTR() && arg ty == ITY_U64()`. `vc_str_call_ok`: PARSE is
  `n == 1 && inst.ty == ITY_U64()` (arg 0 PTR already checked by caller).
- `vc_vec_shape_detail` (`vc_vec.vow:299-310`): FORMAT creates an object with no object operand (reuse
  `vc_str_kind_creates` / `vc_str_kind_no_object_operand`); PARSE reads `args[0]` as a string object; both inherit
  the existing "Vec passed to a call / unmodelled builtin" aliasing rules.
- `vc_inst_specific_reason` is unchanged: known hooks no longer yield `unmodeled-builtin`.

### 3. Encoders (pure, unit-testable)
New functions in `compiler/vc_str.vow` (or `compiler/vc_hooks.vow` [new] if `vc_str.vow` would pass ~450 lines; then
add to `scripts/concat_vow.sh` before `vc_gate`, after `vc_str`, and `use` it in `vc_exec.vow`):
- `VcVec` gains `hk_kind`, `hk_in_arr`, `hk_in_len`, `hk_res` (registry of earlier hook calls; parallel `Vec<i64>`,
  reuse the `memo_*` flat-array style, `vc_vec_new_state` initialises them).
- `vc_hook_parse(vv, ar, hdr, assumed, arr, len) -> i64`: if an earlier parse has the same `(arr, len)` terms
  return its result term (syntactic fast path). Else declare a fresh 64-bit constant `hp<N>` with
  `vc_agg_declare(ar, hdr, vc_vec_fresh_name(vv,"hp"), 64)`; for every earlier parse `j` assume
  `vc_str_eq(arr,len,arr_j,len_j) => r == r_j` via `vc_str_axiom` (global, not path-conditioned: sound, because a
  total function over string contents satisfies every pairwise axiom). Skip when `vc_str_eq` folds to constant false.
- `vc_hook_format(vv, ar, hdr, assumed, bits) -> (arr, len)`: memoise on the `bits` term (`vc_str_memo_find` with a
  sentinel key); else fresh array (`vc_str_fresh_array`) and fresh 64-bit length `hf<N>` (its representation fact
  `len < 2^63-1` comes for free from `vc_vec_slot`/`vc_vec_create_with` like `from_raw_parts_copy`; confirm). For
  each earlier format `j`: assume `bits == bits_j => vc_str_eq(out, out_j)` (quantifier only in a positive
  assumption, same shape as `vc_str_range_eq`).
- The `u64` side is a plain BV64 term (no cast, no `Float`): "a `u64` through `format_f64_bits` is an exact 64-bit
  vector" holds by construction; the unit test asserts the rendered declaration/equality text.

### 4. Executor
- `compiler/vc_exec.vow` `vc_walk_vec`: FORMAT branch beside `VC_STR_NEW`/`FROM_RAW_PARTS` (new object via
  `vc_walk_new_obj`, `vc_vec_create_with(vv, ar, hdr, obj, b, arr, len)` from `vc_hook_format`, return true) so
  `args[0]` is never looked up as an object. PARSE branch in `vc_walk_str`: `vc_define(w, inst, vc_hook_parse(...))`
  (result is `ITY_U64`; `vc_define` already handles a BV64 term).
- Call inside an unrolled loop / spliced callee: registry is per `VcVec`, i.e. per query, so copies are
  consistent automatically.

### 5. Replace the skip fixture; add fixtures (`// TEST:` directive style of neighbours)
- `tests/verify-native/skip/float_bits_hook_skipped.vow` -> `git mv` to `tests/verify-native/unknown/float_bits_hook_opaque_value.vow` (`ensures: result != 7u64` over `parse_f64_bits(s)`: depends on the hook's value -> `unknown`). Unmodelled-builtin skip coverage stays on `skip/unmodeled_builtin_skipped.vow` (`contains`); confirm the #1648/#1651 skip-code coverage gate (grep `scripts/` for `unmodeled-builtin`) is satisfied by it.
- `pass/parse_f64_bits_deterministic.vow`: `ensures: result == parse_f64_bits(s)` over a function returning `parse_f64_bits(s.clone())`-style; plus `pass/format_f64_bits_deterministic.vow`: `bits1 == bits2` requires -> `format(bits1) == format(bits2)` via `string_eq`/`==` on strings.
- `fail/parse_f64_bits_wrong_claim.vow`: `ensures: result != parse_f64_bits(s)` where `result = parse_f64_bits(s.clone())`, and a variant `result == parse_f64_bits(s) + 1` -- both fail for every hook value (independence re-check returns unsat, so `failed`).
- `fail/format_f64_bits_distinct_bits_equal.vow`: wrong "different bits give different strings"-style is NOT refutable (opaque) -> belongs in `unknown/`; keep one `u64`-side refutation: `fn f(bits: u64) -> u64` with `let s = format_f64_bits(bits); bits & 1` and a wrong post `result == bits` (hook not in the cone, sliced away; real `failed`).
- No fixture may depend on NaN payloads (ADR Known limits).

### 6. Independence re-check (the `sat` policy)
- `compiler/vc_native.vow`: after a hard claim returns `sat` (call sites at lines ~186 and ~335, shared helper to keep
  it one place) and the rendered claim query declares a hook symbol (reserved prefixes `hp`/`hf` via
  `vc_vec_fresh_name`; test on the sliced/rendered query), build a second query: same header and assumptions,
  scalar parameters pinned to the model values (`cs.model_*` tables, `vc_cex.vow`), String/Vec parameters left free,
  and the **goal asserted** (not negated). `unsat` => the claim fails for every hook value and every string content,
  keep `failed` with the original counterexample; `sat`/`unknown`/timeout => verdict `unknown`, reason
  `opaque builtin result: <hook>` (free-text unknown reason, like `vc_unwinding_reason`; not a skip code, so the
  closed skip-code list is untouched). Soft claims and static-sat claims are never affected.
- Cache: the second query is a different text, so `VcCache` keying (`vc_cache_peek`) needs no change; a cached
  `unsat` of the pinned query is as trustworthy as any other (`--no-cache` rule applies for "green locally").
- `--replay-cex`: a surviving `failed` is independent of the hook values, so replay reproduces it; a pinned String
  parameter case reports String params as `skipped` as today (confirm against `vc_cex.vow`).

### 7. Docs and generated mirrors
- ADR: edit the #1415 "Hooks" bullet and Known limits (191-193); add `## Addendum (#1646)` (encoding, pairwise
  axioms, independence rule, quadratic cost, no round-trip axiom, trusted base: the axioms join the simplifier/binder).
- `docs/spec/contracts.md:45`, `docs/spec/cli.md` "Floats" paragraph and Subset line: remove "stay Skipped ... because their other side is a `String`";
  state they are modelled (exact `u64`, deterministic function of the string, opaque text) and that a property of the text itself is `unknown` (`opaque builtin result`).
- `docs/spec/errors.md` / `docs/verifier-eval.md`: only if they list the hooks as skipped (grep).
- `uv run python scripts/generate_help.py` to refresh `skills/vow/reference/*` and `compiler/main.vow`;
  `python3 scripts/check_help_coverage.py`.

## Testing (TDD order; all via `build/vowc test compiler/tests/<file>.vow` unless noted)
1. RED `test_vc_ops.vow`: `check_known_classes` asserts `catalogue_verifier_known("__vow_parse_f64_bits")` and `..._format_f64_bits`, and `vc_str_op_kind` returns the new kinds; `scripts/test_generate_operations.py` (the `known` entries at 612-623 / 2462 / 2493 flip) -> GREEN with step 1.
2. RED `test_vc_gate.vow`: both hooks pass the gate with right types and are rejected with wrong arity/types (PARSE with `u64` arg; FORMAT with `ptr` arg) -> step 2.
3. RED `test_vc_str.vow` (encoder text pins, as the file already does for binder text): parse of identical operands returns the same term and no axiom; parse of two operands adds exactly one `=>` axiom over a `VC_T_QEQ` guard; format memoisation on identical bits; format pair axiom shape -> step 3.
4. RED `test_vc_exec.vow`/`test_vc_solver.vow` end-to-end with the fake/real solver: `parse(s)==parse(s.clone())` unsat; `parse(s)!=parse(s.clone())` sat; -> step 4.
5. RED fixtures (step 5) via `tests/verify-native/tests.sh` and the native corpus runner; -> step 6 turns `unknown/float_bits_hook_opaque_value` and the `fail/` pair green.
6. Gates: `python3 scripts/generate_operations.py --check`, `python3 scripts/test_generate_operations.py`, `scripts/check_help_coverage.py`, `build/vowc test compiler/ --filter test_vc_`, `scripts/bootstrap.sh --skip-cargo --no-cache` on the final head SHA (record SHA and `scripts/seed.toml` pin), then `scripts/full_test.sh` (background + poll; ~40 min). Differential: `scripts/verify_diff.py` must show these fixtures as native-decided / ESBMC-skipped, not a `soundness` finding.

## Verification surface
- Properties the native solver must prove (all quantifier-free except the byte-equality binder): determinism of parse over equal strings (incl. a clone and a built-then-compared string), determinism of format over equal bits, `u64` identity through format (hook sliced out).
- Properties it must refute: results that contradict determinism; `u64`-side arithmetic over a parse result that fails for every value.
- Properties that stay `unknown`: anything about the value of `parse` or the text of `format`.
- ESBMC/C model: unchanged (`c_model: unmodeled`); no `tests/run/` or `examples/` growth; `tests/run/query_utility_catalogue_smoke.vow` untouched.

## Risks
- **Unsound `failed` from an over-approximation** (biggest): mitigated by step 6; if pinning String params proves impractical, fall back to "hard `sat` with a hook in the cone => `unknown`" and keep only the sliced-away `u64` refutation fixture (document in ADR). Never downgrade `unsat`.
- **Quadratic axioms**: k calls give k(k-1)/2 pairwise axioms, each a possible quantified `vc_str_eq`; a hook in an unrolled loop can end `timeout`/`unknown`, never `proven`. Keep fixtures loop-free; note in ADR Cost.
- **Vacuous consistency if bytes unread**: PARSE must set `vv.content` (reads-bytes), else axioms compare unconstrained arrays and the proof is unsound. Unit-test it.
- **Global axiom on unexecuted path**: sound by the total-function argument; add a test with a call on one branch only.
- **Fixed point / determinism**: new `VcVec` fields and registry vectors use `Vec` (ordered), no `HashMap`; fresh-name counters deterministic; regenerate catalogue block rather than hand-edit (drift gate `ops/catalogue-drift`).
- **`concat_vow.sh` order** if a new module is added (bootstrap triple test breaks otherwise).
- **Gate/dispatch gaps**: FORMAT reaching the `t0 != ITY_PTR()` early return or `vc_vec_slot(args[0])` silently misclassifies; covered by gate/exec unit tests.
- Verifier-only change: no Rust twin, no C-parity or `cargo clippy` impact; `--no-cache` mandatory for the native-verifier "green locally" claim.

## Out of scope
- Any text semantics (decimal grammar, `format(parse(s)) == s`, round-trip, NaN payload).
- Other opaque builtins (`parse_*`, `int_to_string`, `trim`, ...), arena variants, `Vec<f64>`, float `%`, unary float minus (#1647), #1653 equal-built-string timeouts.
- Rust compiler / `c_emitter.{rs,vow}` / ESBMC model changes; refactors or formatting of neighbouring verifier modules.
