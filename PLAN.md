# Plan: issue #1276 — port `__vow_string_parse_u64_opt` verifier model to self-hosted emitter

## 1. Problem restated

`vow-verify/src/c_emitter.rs` (Rust) treats `__vow_string_parse_u64_opt` as a known, modelable
builtin end-to-end: `is_known_builtin` (line 537), `collect_option_vars` (line 416), and the C
emission arm shared with `__vow_string_parse_i64_opt`/`_in_arena` (lines 1636-1643, a nondet
`tag`/unconstrained `payload` model). `compiler/c_emitter.vow`, the self-hosted mirror that must
stay behaviorally identical, already classifies the symbol as option-returning
(`is_option_returning_extern`, line 962) but omits it from `is_known_builtin` (lines 169-237) and
has no emission arm for it (the dispatch at lines 2038-2039 only matches
`parse_i64_opt`/`_in_arena`). The net effect: a pure Vow function calling `.parse_u64()` is
modelable and verified in the Rust compiler but is reported non-modelable (verification
skipped/degraded, `Call extern` \`__vow_string_parse_u64_opt\`\` as the reported reason) in
`build/vowc`. This is a self-hosted-only parity bug in an already-correct, already-documented
builtin (`.parse_u64()` / `parse_u64` are documented at `docs/spec/grammar.md:918,1142`); it does
not change syntax, semantics, or the Rust compiler, so no `docs/spec/*.md` update is required.

Confirmed while reading the code: `__vow_string_parse_u64_opt` has **no `_in_arena` variant**
anywhere in the codebase (unlike `parse_i64_opt`) — `compiler/lower.vow:3873` only ever emits the
non-arena symbol, and neither `vow-verify/src/c_emitter.rs` nor `compiler/c_emitter.vow` list a
`_in_arena` form for it. The fix only needs to handle the single symbol
`__vow_string_parse_u64_opt`.

## 2. Files to touch

- **`compiler/c_emitter.vow`** (self-hosted emitter — the only production-code change):
  - `is_known_builtin` (lines 169-237): add `|| name == String::from("__vow_string_parse_u64_opt")`
    to the disjunction, next to the existing `parse_i64_opt`/`_in_arena` lines (214-215), mirroring
    `vow-verify/src/c_emitter.rs:537`.
  - `emit_string_op`'s dispatch (lines 2038-2047, the `__vow_string_parse_i64_opt` /
    `_in_arena` arm): extend the condition to also match `__vow_string_parse_u64_opt`, so it falls
    into the same unconstrained nondet `tag`/`payload` emission as i64, mirroring
    `vow-verify/src/c_emitter.rs:1636-1643` exactly (no payload range assumption — u64, like i64,
    has no narrower target-width model).
  - No other function needs editing: `is_option_returning_extern` (line 962) and
    `collect_option_vars` (line 976) already list the symbol; `first_unsupported_opcode_name`
    (line 431) calls `is_known_builtin` and will automatically stop flagging the symbol once fixed.
- **`compiler/tests/test_c_emitter.vow`** (self-hosted test): add one new `check_*` function
  asserting a function calling `__vow_string_parse_u64_opt` is modelable and gets the expected C
  model, wired into `main()`. Full detail in TDD slice 2 below.
- **`vow-verify/src/c_emitter.rs`**: no change. It is already correct; this issue is a
  self-hosted-only parity gap. (See "Out of scope" for why a symmetric Rust-side unit test is not
  added here either.)
- **`docs/spec/*.md`**: no change. `.parse_u64()` is already documented
  (`docs/spec/grammar.md:918,1142`); this fix does not alter syntax, semantics, types, builtins,
  operators, effects, or CLI flags — only internal verifier-modelability classification.

## 3. TDD slices

1. **Red: self-hosted test exposes the gap.**
   - Test location: `compiler/tests/test_c_emitter.vow`, new function
     `check_parse_u64_option_model() -> i64`, modeled directly on the existing
     `check_arena_parse_i64_option_model()` (lines 245-299) but with the arena argument removed
     (this symbol takes a single `String` argument, matching `fn(s: String) -> Option<u64>` per
     `docs/spec/grammar.md:1142`):
     - Build a one-block `IrFunction` with an `ITY_PTR()` string-arg (`ARG 0`), a
       `mk_inst_call_extern(1, ITY_PTR(), [0], "__vow_string_parse_u64_opt")`, a `FieldGet` on
       index 0 (the `tag`) of that result, and a `Return`.
     - Assert `non_modelable_reason(m.functions[0], m, const_fn_indices) == ""` (today this fails:
       the reason string is `` Call extern `__vow_string_parse_u64_opt` ``, since
       `is_known_builtin` doesn't yet know the symbol).
     - Assert `emit_c_module(...)` contains `__vow_option_t v1;`, `v1.tag = __VERIFIER_nondet_long();`,
       the field projection (`v2 = v1.tag;`), and does **not** contain
       `/* opcode Call not modelled */`.
     - Use unused return/error codes starting at 230 (the file's highest existing code is 222 —
       confirm with `grep -oE 'return [0-9]+;' compiler/tests/test_c_emitter.vow` before picking,
       since another slice landing concurrently could raise the high-water mark).
     - Wire the call into `main()` as the next `rN` (currently ends at `r19`), following the exact
       `let rN = check_...(); if rN != 0 { return i64_to_i32_wrap(rN); }` pattern already used.
     - Confirm red: build and run this test file alone against the *current* `compiler/c_emitter.vow`
       (before the production-code edit) and observe a non-zero exit / the specific failing
       assertion. Concretely: `build/vowc test compiler/ --filter test_c_emitter` (or equivalent
       single-file invocation resolving `use` against `compiler/`) must fail on the new check
       before slice 2, and pass after.
2. **Green: production-code fix.**
   - Apply the two edits in `compiler/c_emitter.vow` described in "Files to touch" above
     (`is_known_builtin` symbol addition; `emit_string_op` dispatch arm extension).
   - Re-run the same test invocation; the new `check_parse_u64_option_model` must now return 0.
3. **Verification, in two parts — do not conflate them:**
   - **3a. Unit-test parity gate (no bootstrap needed).** `compiler/tests/test_c_emitter.vow` does
     `use c_emitter`, so both `target/release/vow test compiler/` and `build/vowc test compiler/`
     compile `compiler/c_emitter.vow` fresh **from source** each time `vow test` runs — red→green
     for slice 1/2 depends only on the source edit, not on rebuilding `build/vowc`. Run the same
     check `scripts/full_test.sh` (~lines 1511-1519) and `.github/workflows/bootstrap.yml`
     (~lines 78-97) run:
     ```
     target/release/vow test compiler/ > "$TMPDIR/rust_test.json"
     build/vowc test compiler/ > "$TMPDIR/self_test.json"
     python3 scripts/parity.py test "$TMPDIR/rust_test.json" "$TMPDIR/self_test.json"
     ```
     This confirms the new test passes under both compilers and that nothing else regressed.
   - **3b. End-to-end fix confirmation (requires bootstrap).** The unit test in 3a only proves the
     classification helper returns the right answer when invoked directly with hand-built IR — it
     does not by itself prove a real `.vow` program's `.parse_u64()` call stops being reported
     Skipped by `build/vowc verify`. Rebuild `build/vowc` from the patched source
     (`scripts/bootstrap.sh --skip-cargo`), then write a small throwaway program (e.g.
     `$TMPDIR/parse_u64_check.vow`) with a pure function that calls `.parse_u64()` and has a
     trivial `ensures`, and run:
     ```
     VOW_CACHE_DIR=$(mktemp -d) target/release/vow verify "$TMPDIR/parse_u64_check.vow"
     VOW_CACHE_DIR=$(mktemp -d) build/vowc verify "$TMPDIR/parse_u64_check.vow"
     ```
     Assert neither reports the function Skipped/non-modelable (per the memory note on this repo,
     the compile cache ignores compiler rebuilds at a fixed source revision, so a fresh
     `VOW_CACHE_DIR` per invocation is required or a stale pre-fix object could be served). This is
     the check that actually closes the issue as filed — the user-visible verifier-coverage gap,
     not just the internal classifier.

No horizontal refactor slice is planned — this is a two-line-condition + one-arm addition plus one
test, not a redesign.

## 4. Verification surface

- This change only affects **verifier-modelability classification and C-model emission**, not
  contracts or codegen. No new ESBMC properties are introduced beyond what
  `__vow_string_parse_i64_opt` already proves today (the model is a direct copy: nondet boolean
  `tag`, unconstrained `payload` when `tag == 1`). ESBMC's job on any function that now becomes
  modelable is unchanged in kind — whatever `requires`/`ensures` that function already declares.
- No `tests/run/*.vow` or `examples/*.vow` fixtures need to grow. The issue is about
  *classification* (modelable vs. not), which is fully exercised by the IR-builder-level unit test
  in slice 1 exactly as the sibling i64 case already is (`check_arena_parse_i64_option_model`) —
  there is no existing end-to-end `tests/run/` fixture for `parse_i64_opt`'s modelability either,
  so adding one only for u64 would be an inconsistent, out-of-proportion addition.
- No contract authoring is involved: this fix does not touch `requires`/`ensures`/`invariant`
  clauses anywhere.

## 5. Risk areas

- **Binary fixed point / bootstrap triple-test:** the two edits are pure string-literal
  comparisons in an `if`/`||` chain (deterministic, no `HashMap`, no iteration-order dependency).
  Low risk, but must still run the triple-test
  (`scripts/concat_vow.sh clif` → stage 0/1/2 → `sha256sum` compare) since *any*
  `compiler/*.vow` change is bootstrap-sensitive; `scripts/full_test.sh` already covers this.
- **`build/vowc` staleness:** `build/vowc` is gitignored and only reflects `compiler/*.vow` as of
  the last `scripts/bootstrap.sh` run. Slice 3a (the unit test) does *not* need a rebuild — it
  compiles `compiler/c_emitter.vow` from source via `vow test`. Slice 3b (the end-to-end check)
  does — rebuild with `scripts/bootstrap.sh --skip-cargo` first, and use a fresh
  `VOW_CACHE_DIR` per invocation (see memory note on this repo: the compile cache ignores compiler
  rebuilds at a fixed source revision).
- **Parity harness, not clippy:** this PR touches only `compiler/*.vow` and its test, so
  `cargo clippy --all -- -D warnings` is unaffected (no Rust production code changes). The
  actual gates that matter here are the `vow test compiler/` Rust/self-hosted parity check and the
  end-to-end verify check (section 3, slices 3a/3b) — verify those, not clippy.
- **Return-code collisions in `compiler/tests/test_c_emitter.vow`:** the file uses ad hoc small
  integers as `check_*` failure codes with no central registry. Re-grep the current max
  (222 as of this writing) immediately before implementation in case another change landed
  in between planning and implementation.
- **Do not accidentally "improve" the model while porting it.** The Rust model for
  `parse_u64_opt` reuses `__VERIFIER_nondet_long()` (signed) for both `tag` and `payload`, not
  `__VERIFIER_nondet_ulong()`, unlike the narrower `parse_u32_opt` arm which does use the unsigned
  variant. This looks asymmetric but is what `vow-verify/src/c_emitter.rs:1636-1643` actually does
  today for i64/u64 alike — the self-hosted port must reproduce it byte-for-byte, not "fix" the
  signedness as a drive-by improvement. If the Rust model's signedness is judged wrong, that's a
  separate issue against `vow-verify/src/c_emitter.rs`, not this parity port.

## 6. Out of scope

- **Operation Catalogue migration.** Issue #1271 deliberately left this symbol's verifier-model
  category as a hand-listed exception in both compilers; migrating it through
  `docs/spec/operations.json` / `scripts/generate_operations.py` is explicitly called out in the
  issue as a follow-up, not part of this fix.
- **Adding a dedicated Rust-side unit test for `is_known_builtin("__vow_string_parse_u64_opt")`.**
  No such test currently exists in `vow-verify/src/c_emitter.rs` (only the shared
  `emit_arena_parse_i64_as_an_option` test exercises the shared match arm, for the i64 case). Note
  the repo does generally pair tests one-to-one across the two emitters (e.g.
  `emit_arena_parse_i64_as_an_option` ↔ `check_arena_parse_i64_option_model`), so a reviewer may
  reasonably ask for a `~15`-line Rust-side twin covering the u64 case specifically. This plan
  still adds only the self-hosted test (slice 1), because: (a) the issue explicitly scopes this PR
  to the self-hosted port and treats the Rust side as already-correct and out of scope; (b) Rust's
  behavior for u64 is already implicitly exercised by the shared match arm the i64 test covers
  (same code path, same three symbols in one `matches!`/`if` arm). Adding the Rust twin is a small,
  independent, low-risk follow-up — flagged here as a named option, not silently dropped — rather
  than bundled into this fix.
- **`__vow_string_parse_u64_opt_in_arena`.** Does not exist anywhere in the codebase (confirmed by
  grep); do not invent an arena variant while porting this.
- **Any refactor of the `is_known_builtin` / `emit_string_op` dispatch shape** (e.g., collapsing
  the growing `if`/`||` chains into a table, or converting the self-hosted emitter's dispatch into
  Operation-Catalogue-generated code). Out of scope per the issue and per "many small changes beat
  one large change" — this PR is a two-site parity fix plus a test, nothing else.
- **Formatting or unrelated cleanup** anywhere in `compiler/c_emitter.vow` or
  `compiler/tests/test_c_emitter.vow` beyond the new lines.
