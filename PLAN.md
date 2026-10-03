# Plan: close mutation-testing gaps in `compiler/c_emitter.vow` (issue #1326)

## 1. Problem restated

Three `vowc mutants` survivors in the self-hosted emitter (`compiler/c_emitter.vow`) pass the
full Tier-2 oracle (`scripts/full_test.sh`) unnoticed: `str_starts_with`'s loop starts comparing
at byte index 1 instead of 0 (id 461), `collect_wide_vars`'s outer loop condition flips from
`bi < blocks.len()` to `bi >= blocks.len()` and so never runs (id 1427), and `emit_string_op`'s
literal-index bounds check `candidate.dv >= 0 && candidate.dv < strings.len()` becomes an `||`
(id 1916). Per `CLAUDE.md`'s mutation-testing policy, each survivor needs either a test that
kills it or a documented equivalence claim. Investigation below (reading the production code and
the existing hand-built-IR test idioms in `compiler/tests/test_c_emitter.vow`) confirms the first
two are real, reachable bugs and settles the issue's own open question on the third: hand-built-IR
tests already exist extensively in this file, so the issue's stated preference — "(a) only if
hand-built IR tests already exist for this module" — resolves to writing a test, not recording an
equivalence.

## 2. Files to touch

- **`compiler/tests/test_c_emitter.vow`** — the only file this issue needs to change. Three new
  `check_*` functions plus three new `let rNN: i64 = check_...(); if rNN != 0 { ... }` lines
  appended to `fn main()` (currently ends at the `r27` call, line ~997).
- **No changes to `compiler/c_emitter.vow` itself.** Production code is already correct; the
  mutants are genuine misses in test coverage, not bugs to fix.
- **No changes to `compiler/tests/builders.vow`.** The existing `ir_inst_new(id, op, ty, args, dk,
  dv, dv2, ds)` escape hatch already used in this test file for one-off opcodes (e.g.
  `check_wide_constants_are_non_modelable` building `IOP_CONST_I128()` directly, or
  `check_arena_parse_i64_option_model` building `IOP_FIELD_GET()` directly) covers both new
  opcodes needed here (`IOP_FIELD_SET()`, `IOP_CONST_STR()`). No new builder helper pulls its
  weight for a one-time use.
- **No Rust-crate changes.** `str_starts_with` and `emit_string_op`'s literal-lookup guard have no
  Rust-side counterpart with the same hand-rolled logic — `vow-verify/src/c_emitter.rs`'s
  `collect_wide_vars` is the one function that does exist on both sides, and it's a 5-line
  `.filter(...).map(...)` iterator chain with no off-by-one to mutate; there is nothing to port.
  This is a self-hosted-only test-coverage gap, not a language-semantic change, so `CLAUDE.md`'s
  "always modify both compilers" rule for behavioral changes does not apply.
- **No `docs/spec/*.md` changes.** No syntax, semantics, builtin signature, operator, effect, or
  CLI flag changes.

## 3. TDD slices

These are coverage-only additions against already-correct production code, so "red" here means
"the mutant currently survives because no test exercises the differing behavior," not "production
code is broken." Each slice's "green" bar is: (a) the new test passes against the current
`build/vowc test compiler/ --filter test_c_emitter` run (directory-scan mode — a single-file
invocation resolves `use c_emitter;` against `compiler/tests/`, which has no `c_emitter.vow`, so
it must be the directory form), and (b) hand-applying the mutation and re-running the same command
makes it fail (see "Verification surface" below for why this is safe to do in-place and why it's
enough — a full `vowc mutants run` on the original mutant ID is confirmatory, not required, since
the IDs below are already known to have drifted).

**Why a failing test actually fails the Tier-2 oracle.** It's not obvious from the issue text
alone that a `compiler/tests/*.vow` test failure propagates to `scripts/full_test.sh`'s exit code —
`scripts/full_test.sh` itself (Section 10b, ~line 1693) runs `vowc test compiler/` with *both*
compilers but only explicitly asserts on `contract_density` presence and a separately-filtered
`test_complexity_io` check; it does not itself check `status == TestsPassed` on the full run.
That check lives one layer down: `run_parity test "test/parity" ...` calls
`scripts/parity.py`'s `compare_test()` (`scripts/parity.py:506-518`), which explicitly requires
`exit_code == 0` **and** `document.get("status") == "TestsPassed"` for *both* the Rust and
self-hosted run, appending an error (→ `fail "test/parity"` → nonzero `full_test.sh` exit) if
either compiler's test run didn't cleanly pass. Since the mutation lives in the Vow *source text*
of `compiler/c_emitter.vow`, both the Rust-frontend test run (`$RUST test compiler/`) and the
self-hosted test run (`run_self test compiler/`, itself built from the mutated source) compile and
execute the same mutated `str_starts_with`/`collect_wide_vars`/`emit_string_op`, so a test that
fails under the mutation fails identically on both sides and `compare_test` still flags it (exit
code and status are checked unconditionally, independent of whether the two sides agree with each
other). No changes to `scripts/full_test.sh` or `scripts/parity.py` are needed.

### Slice 1 — `str_starts_with` (mutant 461)

Add `check_str_starts_with() -> i64` as a direct unit test of the pure helper — no IR needed,
since `str_starts_with(s: String, prefix: String) -> bool` is callable straight from the test
module like any other plain `fn` in `c_emitter.vow` (existing tests already call
`is_known_builtin`, `non_modelable_reason`, etc. the same way).

```
fn check_str_starts_with() -> i64 {
    if str_starts_with(String::from("xbc"), String::from("abc")) { return 400; }
    if !str_starts_with(String::from("abc"), String::from("abc")) { return 401; }
    if !str_starts_with(String::from("abcdef"), String::from("abc")) { return 402; }
    if !str_starts_with(String::from("abc"), String::from("")) { return 403; }
    if str_starts_with(String::from("ab"), String::from("abc")) { return 404; }
    if str_starts_with(String::from("abd"), String::from("abc")) { return 405; }
    0
}
```

The first assertion (`"xbc"` vs. prefix `"abc"`) is the one that kills mutant 461 directly: with
the real loop (`i` starts at 0), byte 0 mismatches (`'x' != 'a'`) and the function correctly
returns `false`. With the mutant (`i` starts at 1), the loop only checks bytes 1 and 2 (`'b'=='b'`,
`'c'=='c'`), never looks at byte 0, and wrongly returns `true`. The other five cases pin exact
match, prefix-shorter-than-string, empty-prefix, prefix-longer-than-string, and a non-leading
mismatch, so a future regression in any direction is caught, not just this one mutation.

No production code changes.

### Slice 2 — `collect_wide_vars` / `field_access_is_wide` (mutant 1427)

Add `check_wide_field_set_is_non_modelable() -> i64`, building a function whose only wide value
comes from a parameter (not `IOP_CONST_I128()`/`IOP_CONST_U128()`, which `iop_unsupported_name`
special-cases and would hide the `FieldSet`-specific path we're targeting), stored through
`IOP_FIELD_SET()`:

```
fn check_wide_field_set_is_non_modelable() -> i64 {
    let insts: Vec<IrInst> = Vec::new();
    insts.push(mk_inst_arg(0, ITY_PTR(), 0));
    insts.push(mk_inst_arg(1, ITY_I128(), 1));
    let fs_args: Vec<i64> = Vec::new();
    fs_args.push(0);
    fs_args.push(1);
    insts.push(ir_inst_new(2, IOP_FIELD_SET(), ITY_UNIT(), fs_args, IDATA_FIELD(), 0, 0, String::from("")));
    insts.push(mk_inst_const_i64(3, 0));
    insts.push(mk_inst_return(4, 3));

    let blocks: Vec<IrBlock> = Vec::new();
    blocks.push(mk_block(0, insts));
    let params: Vec<i64> = Vec::new();
    params.push(ITY_PTR());
    params.push(ITY_I128());
    let f: IrFunction = mk_function(410, String::from("store_wide"), params, ITY_I64(), blocks);
    let funcs: Vec<IrFunction> = Vec::new();
    funcs.push(f);
    let m: IrModule = mk_module(funcs);
    let const_fn_indices: Vec<i64> = Vec::new();

    let reason: String = non_modelable_reason(f, m, const_fn_indices);
    if reason == String::from("") { return 410; }
    if !reason.contains(String::from("FieldSet at 128-bit width")) { return 411; }

    // Companion: the gate must reject only the wide FieldSet, not every FieldSet.
    let narrow_insts: Vec<IrInst> = Vec::new();
    narrow_insts.push(mk_inst_arg(0, ITY_PTR(), 0));
    narrow_insts.push(mk_inst_const_i64(1, 0));
    let narrow_fs_args: Vec<i64> = Vec::new();
    narrow_fs_args.push(0);
    narrow_fs_args.push(1);
    narrow_insts.push(ir_inst_new(2, IOP_FIELD_SET(), ITY_UNIT(), narrow_fs_args, IDATA_FIELD(), 0, 0, String::from("")));
    narrow_insts.push(mk_inst_return(3, 1));
    let narrow_blocks: Vec<IrBlock> = Vec::new();
    narrow_blocks.push(mk_block(0, narrow_insts));
    let narrow_params: Vec<i64> = Vec::new();
    narrow_params.push(ITY_PTR());
    let narrow_f: IrFunction = mk_function(411, String::from("store_narrow"), narrow_params, ITY_I64(), narrow_blocks);
    let narrow_funcs: Vec<IrFunction> = Vec::new();
    narrow_funcs.push(narrow_f);
    let narrow_m: IrModule = mk_module(narrow_funcs);
    if non_modelable_reason(narrow_f, narrow_m, const_fn_indices) != String::from("") { return 412; }
    0
}
```

With the real `collect_wide_vars`, the parameter's id lands in `wide_vars` (its `inst.ty` is
`ITY_I128()`), so `field_access_is_wide` sees `IOP_FIELD_SET()` storing a wide id and returns
`true`; `is_modelable` rejects the function and `non_modelable_reason` reports
`"...is not modelable in the verifier (contains unsupported opcode \`FieldSet at 128-bit
width\`)"` — `opcode_name(IOP_FIELD_SET())` is confirmed to return exactly `"FieldSet"`
(`compiler/ir_printer.vow:166`), so the literal string in assertion 411 is correct, not a guess.
With the mutant (`bi >= blocks.len()`, outer loop body never runs), `wide_vars` is always empty,
`field_access_is_wide` always returns `false` for `FieldSet`, and the function is wrongly reported
modelable (`reason == ""`), which the first assertion catches.

No production code changes.

### Slice 3 — `emit_string_op`'s literal-index guard (mutant 1916)

Resolves the issue's own open question: hand-built-IR tests calling `emit_c_function` directly
(bypassing the `is_modelable` classification entirely) already exist in this file —
`check_expanding_string_helper_falls_back` (around line 410) is the template: it builds a raw
`IrFunction`/`IrModule`, calls `emit_c_function(c, f, const_fn_indices, modelable_fi_set, m, 16,
16, 16, 16, false, false, 0)` directly, and asserts on the literal substring
`"/* opcode Call not modelled */"` that `emit_unmodelled` writes. So option (a) applies, per the
issue's own stated rule.

Add `check_string_matches_literal_at_oob_index_falls_back() -> i64`:

```
fn check_string_matches_literal_at_oob_index_falls_back() -> i64 {
    let insts: Vec<IrInst> = Vec::new();
    insts.push(mk_inst_const_i64(0, 0));
    insts.push(mk_inst_const_i64(1, 0));
    insts.push(ir_inst_new(2, IOP_CONST_STR(), ITY_PTR(), Vec::new(), IDATA_CONST_STR(), 1, 0, String::from("")));
    let call_args: Vec<i64> = Vec::new();
    call_args.push(0);
    call_args.push(1);
    call_args.push(2);
    insts.push(mk_inst_call_extern(3, ITY_I64(), call_args, String::from("__vow_string_matches_literal_at")));
    insts.push(mk_inst_return(4, 3));

    let blocks: Vec<IrBlock> = Vec::new();
    blocks.push(mk_block(0, insts));
    let f: IrFunction = mk_function(420, String::from("matches_literal_oob"), Vec::new(), ITY_I64(), blocks);
    let funcs: Vec<IrFunction> = Vec::new();
    funcs.push(f);
    let m: IrModule = mk_module(funcs);
    m.strings.push(String::from("ab"));

    let c: String = String::from("");
    let const_fn_indices: Vec<i64> = Vec::new();
    let modelable_fi_set: Vec<i64> = Vec::new();
    emit_c_function(c, f, const_fn_indices, modelable_fi_set, m, 16, 16, 16, 16, false, false, 0);
    if !c.contains(String::from("/* opcode Call not modelled */")) { return 420; }

    // Companion: an in-range index on the same shape must NOT fall back.
    let insts2: Vec<IrInst> = Vec::new();
    insts2.push(mk_inst_const_i64(0, 0));
    insts2.push(mk_inst_const_i64(1, 0));
    insts2.push(ir_inst_new(2, IOP_CONST_STR(), ITY_PTR(), Vec::new(), IDATA_CONST_STR(), 0, 0, String::from("")));
    let call_args2: Vec<i64> = Vec::new();
    call_args2.push(0);
    call_args2.push(1);
    call_args2.push(2);
    insts2.push(mk_inst_call_extern(3, ITY_I64(), call_args2, String::from("__vow_string_matches_literal_at")));
    insts2.push(mk_inst_return(4, 3));
    let blocks2: Vec<IrBlock> = Vec::new();
    blocks2.push(mk_block(0, insts2));
    let f2: IrFunction = mk_function(421, String::from("matches_literal_in_range"), Vec::new(), ITY_I64(), blocks2);
    let funcs2: Vec<IrFunction> = Vec::new();
    funcs2.push(f2);
    let m2: IrModule = mk_module(funcs2);
    m2.strings.push(String::from("ab"));
    let c2: String = String::from("");
    emit_c_function(c2, f2, const_fn_indices, modelable_fi_set, m2, 16, 16, 16, 16, false, false, 0);
    if c2.contains(String::from("/* opcode Call not modelled */")) { return 421; }
    0
}
```

`m.strings` has exactly one entry, so `dv = 1` is one-past-the-end: non-negative but out of
range. With the real guard (`>= 0 && < len`), `1 < 1` is false, the search loop never marks
`found`, and `emit_string_op` falls through to `emit_unmodelled`. With the mutant (`>= 0 || <
len`), `1 >= 0` alone makes the condition true, so `literal = strings[candidate.dv]` executes an
out-of-bounds `Vec<String>` index — on the current correct code this line is provably unreached
(the `if` guards it), so the test cannot crash when run against correct code; it only crashes
under the surviving mutant, and a crash is itself an oracle failure (`vowc test` / `full_test.sh`
exits non-zero), which is a valid "caught" outcome per `docs/mutants.md`. The companion case
(`dv = 0`, in range) pins that the guard still accepts valid indices, so the test is not merely
asserting on `emit_unmodelled`'s wording without checking the opposite is possible at all.

Dispatch into `emit_string_op` for this call is purely name-based: `emit_c_function`'s instruction
loop matches `str_starts_with(ds, "__vow_string_")` (line ~1687) before routing to
`emit_string_op`, with no type-checking of the call's arguments first — confirmed by
`check_expanding_string_helper_falls_back`'s existing precedent of using placeholder
`mk_inst_const_i64` args for calls whose real parameters are strings/vecs. `s`/`pos` in this test
are likewise untyped placeholders; only `iargs[2]` (the literal pointer id) needs to resolve to a
real `IOP_CONST_STR()` instruction, which it does. (`string_model_receiver_arg`, which also lists
`__vow_string_matches_literal_at`, is unrelated metadata consumed elsewhere for modelability
classification — it plays no role in `emit_string_op`'s own dispatch.)

No production code changes.

### `main()` wiring

Append after the existing `r27` block (`compiler/tests/test_c_emitter.vow`, end of `fn main()`):

```
let r28: i64 = check_str_starts_with();
if r28 != 0 { return i64_to_i32_wrap(r28); }
let r29: i64 = check_wide_field_set_is_non_modelable();
if r29 != 0 { return i64_to_i32_wrap(r29); }
let r30: i64 = check_string_matches_literal_at_oob_index_falls_back();
if r30 != 0 { return i64_to_i32_wrap(r30); }
```

(Verified against the file: the highest return-code literal currently in use is `303`, so the
`400`/`410`/`420` ranges picked above are free; re-grep before landing in case other in-flight
work added codes in that range meanwhile.)

## 4. Verification surface

These are self-hosted-compiler unit tests of the emitter's own internals, not Vow source programs
— they do not go through ESBMC, and no `tests/run/*.vow` or `examples/` fixture is needed. The
relevant gates are:

- `build/vowc test compiler/ --filter test_c_emitter` — must pass after the three new `check_*`
  functions are added. (Must be the directory form, `compiler/`, not
  `compiler/tests/test_c_emitter.vow` — single-file mode resolves `use c_emitter;` against the
  test file's own parent directory, `compiler/tests/`, which has no `c_emitter.vow`; only the
  directory-scan form resolves `use` against `compiler/` per `compiler/main.vow:4980`'s documented
  behavior.)
- `scripts/full_test.sh` (`VOW_FULL_TEST_SKIP_CARGO=1` since only `.vow` sources change) — runs
  `vowc test compiler/` with both the Rust and self-hosted compiler and parity-checks the results
  (`scripts/full_test.sh:1691-1734`, gated by `scripts/parity.py:506-518`'s `compare_test`, see
  slice-3 note above), so the new tests get exercised by both compilers automatically; no separate
  Rust test needs writing.
- **Primary mutant-killing verification (cheap, do this first):** for each of the three
  production lines, hand-edit the single character the issue's mutation describes (`0` → `1` in
  `str_starts_with`'s initializer; `<` → `>=` in `collect_wide_vars`'s loop condition; `&&` → `||`
  in `emit_string_op`'s literal-index guard), run `build/vowc test compiler/ --filter
  test_c_emitter`, confirm it now fails (nonzero exit / `TestsFailed` in the JSON — a crash is also
  an acceptable failure signal for the slice-3 case, see "Risk areas"), then `git checkout --
  compiler/c_emitter.vow` to restore. Do this one mutation at a time so a failure is attributable.
  This is a ~seconds-per-mutation loop and is sufficient to demonstrate each test kills its target
  mutation; a full `vowc mutants run` is ~40 minutes per Tier-2 pass and is corroborating evidence,
  not the primary check.
- **Mutant IDs are stale — re-derive, don't reuse 461/1427/1916/`21926`.** Those were computed
  against commit `16a60028`; at current `HEAD` the sites have already moved (e.g.
  `collect_wide_vars`'s loop is at `compiler/c_emitter.vow:1091` here, not `:1032`), and other
  unrelated commits may have changed the total mutant count. Before citing IDs in the PR
  description, run `build/vowc mutants list --root compiler` on the branch HEAD (after the test
  additions — recall `test_*.vow` files are excluded from mutation, so adding tests doesn't shift
  other files' IDs) and match entries by `file:line:col:label`, not by the old numeric IDs. If a
  full confirmatory `vowc mutants run --root compiler --shard <id>/<new-total>` is run for the PR
  record, it needs the branch's test commit already present (mutants run uses `git worktree add
  --detach` from HEAD) and a `target/` (symlinked from the main checkout, or `--tier1-cmd
  'scripts/bootstrap.sh'` for a from-scratch worktree).

## 5. Risk areas

- **Binary fixed point.** `compiler/tests/*.vow` is not part of the self-hosted compiler's own
  module graph (`compiler/main.vow`'s `use` graph does not reach `compiler/tests/`), and
  `scripts/concat_vow.sh` (used for the Stage 0→1→2 triple-bootstrap hash check) does not include
  it either. Adding test functions here cannot change `build/vowc`'s own binary, so the triple-test
  hash comparison in `scripts/full_test.sh` is unaffected.
- **`parse → print → parse` idempotency.** This is a property test over the printer
  (`vow-syntax`), not a check keyed to this specific file; as long as the new Vow syntax added is
  ordinary (`fn`, `let`, `if`, struct-literal-free), there's no special exposure here beyond normal
  syntax correctness, which `vowc test`'s own parse step already enforces.
- **Direct field mutation (`m.strings.push(...)`).** `IrModule.strings` is a `Vec<String>` field
  pushed to directly elsewhere in this same file (e.g. `mk_module`'s `m.functions.push(...)`,
  `mk_block`'s `b.insts.push(...)`), so this is an established, safe pattern — not a new one.
- **Return-code collisions in `fn main()`.** The three new `check_*` functions introduce six new
  literal return codes (400-405, 410-412, 420-421). Re-verify uniqueness against the full file
  (`grep -oE "return [0-9]+;"`) immediately before landing, since other issues may land return
  codes in parallel on this same file.
- **Crash-as-catch for mutant 1916.** The OOB companion test is designed to never execute the
  out-of-bounds `strings[candidate.dv]` line under correct code (the guard short-circuits first),
  so it cannot flake or crash in CI under the current, unmutated compiler. Confirm this by running
  the new test locally before committing — if it crashes against real `c_emitter.vow`, the guard
  analysis above is wrong and the test needs rethinking, not a try/catch.
- **`cargo clippy --all -- -D warnings`.** Not applicable — no Rust files change.

## 6. Out of scope

- The issue's own "Scope" section: only 18 of 216 mutants on #1118's changed lines were sampled,
  and only 4,196 mutants across the rest of `c_emitter.vow`/`checker.vow` are unswept. Running a
  broader mutation sweep, or sweeping the rest of `c_emitter.vow`, is explicitly future work the
  issue defers to a possible follow-up comment — not this PR.
- No refactor of `str_starts_with`/`str_ends_with` into a shared helper, no extraction of the
  literal-lookup search loop in `emit_string_op` into a named function, and no new `builders.vow`
  helpers for `IOP_CONST_STR()`/`IOP_FIELD_SET()` beyond the one-off `ir_inst_new(...)` calls used
  here — all would be premature abstraction for a one-time test-construction need, inconsistent
  with the file's existing precedent of inlining one-off opcodes via `ir_inst_new` directly.
- No changes to `is_modelable`, `non_modelable_reason`, `first_unsupported_opcode_name`, or
  `field_access_is_wide` — all are already correct; this issue is coverage-only.
- No investigation of #1076/#1077 (closed: 128-bit struct-field codegen and `VowViolation`
  i128/u128 truncation) beyond confirming they don't overlap this issue's test — they're closed
  and about different failure modes (raw Cranelift codegen crash, and diagnostic value truncation,
  respectively) than the verifier-modelability gate this issue's slice 2 test pins.
