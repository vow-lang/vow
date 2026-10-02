# Plan: issue #1352 — defense-in-depth `is_finite()` guards for `ConstF64`/`ConstF32` C emitters

## 1. Problem restated

`vow-verify/src/c_emitter.rs` and `compiler/c_emitter.vow` render IR float constants
(`Opcode::ConstF64`/`IOP_CONST_F64()`, and nominally `ConstF32`) directly into C double/float
literals for the ESBMC verification model. Issue #1255 closed the only currently-reachable path
to a non-finite constant by rejecting overflowing `f64` literals at lex time, but the emitters
themselves still trust that every `ConstF64`/`ConstF32` instruction they're handed is finite. If
a future pass (e.g. constant folding) ever produces `inf`/`NaN` as a `ConstF64`, today's emitter
would print `v1 = inf;` — not valid C — straight into the ESBMC input, with no second line of
defense. This issue adds that second line of defense directly in the emitter, in both compilers,
independent of the lexer.

## 2. Design decision (read this before touching code)

**Mechanism: reuse the existing `emit_unsupported_for_verification` fail-closed sentinel, not a
process abort/panic.**

Investigated and rejected alternatives:

- **Hard abort / `.unwrap()`-style panic in the self-hosted emitter.** `emit_inst` and every
  function up its call chain (`emit_c_function` → `emit_c_module` / `emit_c_module_with_callees`
  → call sites in `main.vow` and `verifier.vow`) currently declare **no effects**. Per
  `docs/spec/grammar.md`, only `.unwrap()` requires `[panic]` — plain indexing does not. Making
  the guard abort would force threading `[panic]` through that entire call chain, and testing an
  intentionally-aborting path is itself hard: `vow test` treats a test file as a single
  subprocess whose exit code must be 0 to "pass" (per `docs/spec/cli.md`'s `vow test` section),
  so a test file that deliberately aborts can't report itself as passing without inventing a
  self-reexec harness. No such harness exists anywhere in `compiler/tests/*.vow` today. This is
  exactly the "not a small addition" cost the issue flagged — and it's avoidable.
- **Silent `emit_unmodelled` (nondet substitution)**, the pattern used for `ConstI128`/`ConstU128`
  (`vow-verify/src/c_emitter.rs:985`, `compiler/c_emitter.vow` mirror). Rejected: this makes
  ESBMC treat the non-finite value as an unconstrained nondet double — sound as an
  over-approximation, but **silent**. A future regression that starts emitting `inf` would pass
  every test and never surface. That defeats "defense in depth."
- **Chosen: `emit_unsupported_for_verification`** (`vow-verify/src/c_emitter.rs:2072`,
  `compiler/c_emitter.vow:1102`). This is a pure string-emission helper (no new effects, no
  process interaction) that writes a C comment plus
  `__ESBMC_assert(0, "vow:<UNSUPPORTED_OP_VOW_ID>")` — a reserved sentinel vow_id already wired
  end-to-end: `compiler/main.vow:531` and `:780` (and the Rust equivalent in `vow-verify`) already
  classify this sentinel as a verifier-limitation and lift the build status to `Skipped`
  (exit 1), per the existing documented behavior in `docs/spec/errors.md:638`
  ("Why the build fails closed"). This is the **same mechanism already used for exactly this
  class of problem** — e.g. `emit_checked_arith`'s existing fallback at
  `vow-verify/src/c_emitter.rs:2884-2889`: "128-bit and non-integer checked arithmetic is
  rejected ... Fail closed rather than emit a guard for the wrong width." Our guard is the same
  shape: "Fail closed rather than emit an `inf`/`NaN` literal for a non-finite constant."
  No new effects, no new runtime machinery, no new diagnostic plumbing — it fails loud (ESBMC
  reports `Skipped`, not a silently-wrong proof) without the panic-ripple cost.

**Scope cut: guard `ConstF64` only, in both compilers. Defer `ConstF32`.**

- Neither compiler's lowering ever constructs `Opcode::ConstF32`/`IOP_CONST_F32()` from real
  source today (confirmed: no construction site in `vow-ir/src/lower/mod.rs` or
  `compiler/lower.vow`; `f32` has no literal syntax — `docs/spec/grammar.md:136` lists it as
  "limited support"). It's dead code in both emitters, present only for completeness/Cranelift.
- Worse: the self-hosted `ConstF32` arm's `dv` encoding is **already inconsistent** with
  `ConstF64`'s. `compiler/c_emitter.vow:1335-1338` and `compiler/ir_printer.vow:207` both treat
  `inst.dv` as a plain decimal integer (`i64_to_string(inst.dv)`), while `ConstF64` treats `dv` as
  an IEEE-754 bit pattern (`format_f64_bits(inst.dv as u64)`). There is no defined convention to
  write a *meaningful* finite check against for F32 on the self-hosted side without first fixing
  that representation — which is a separate, real bug (also pre-existing and also currently
  unreachable: the Rust side's `ConstF32(f32)` prints via `{}f` directly and would mis-emit
  `format!("{}f", 2.0f32)` as `2f`, fine, but a fractional value like `1.5f32` already round-trips
  correctly on the *Rust* side only because Rust's `InstData::ConstF32` stores a real `f32`, not
  bits — the two compilers don't even agree on what `ConstF32`'s payload means).
- Per `CLAUDE.md`'s dual-compiler rule: "if a change cannot be expressed in Vow ... do not land
  the Rust half alone — drop or re-scope." Since a correct self-hosted `ConstF32` guard requires
  first deciding/fixing the `dv` convention (out of scope for a "chore: add a guard" issue), this
  plan **drops ConstF32 from scope** and files it as an explicit follow-up (see §6).

## 3. Files to touch

- `vow-verify/src/c_emitter.rs` — guard inside the `Opcode::ConstF64` arm (~line 991-995).
- `vow-verify/src/c_emitter.rs` — new/extended `#[cfg(test)]` unit test(s) near
  `emit_const_variants` (~line 4538).
- `compiler/c_emitter.vow` — guard inside the `IOP_CONST_F64()` branch of `emit_inst`
  (~line 1340-1344).
- `compiler/tests/test_c_emitter.vow` — new check function modeled on
  `check_expanding_string_helper_falls_back` (~line 387) and wired into `main()` (~line 730).
- `docs/spec/*.md` — **no changes planned.** This hardens an internal verifier-model code path;
  it changes no syntax, semantics, builtin signature, effect, or CLI flag. The sentinel's
  user-facing behavior (`Skipped`, exit 1) is already documented generically in
  `docs/spec/errors.md:631-638` and is not opcode-specific, so no edit is needed there either.
- No changes to `vow-ir/src/region.rs` or `compiler/region.vow` — `ConstF64` stays classified as
  opcode-level "modelable"; only the runtime *value* triggers the escape hatch, which is decided
  inside the render arm, not the classifier.

## 4. TDD slices

1. **Rust: red.** In `vow-verify/src/c_emitter.rs`, add a test (extend `emit_const_variants` or
   add a sibling test) that builds a func with
   `inst(N, Opcode::ConstF64, Ty::F64, vec![], InstData::ConstF64(f64::INFINITY))` (and a second
   case for `f64::NAN`), calls `emit_c_function(&func, &HashMap::new(), &VerifyLimits::default())`,
   and asserts the output contains `&format!("vow:{UNSUPPORTED_OP_VOW_ID}")` and does **not**
   contain the literal substrings `"inf"` or `"NaN"` as an assignment RHS. Confirm it fails (no
   guard exists yet — today's code emits `v{id} = inf;` / `v{id} = NaN;`).
2. **Rust: green.** In the `Opcode::ConstF64` arm, check `v.is_finite()` before rendering; on
   failure, call `emit_unsupported_for_verification(inst, out)` instead:
   ```rust
   Opcode::ConstF64 => {
       if let InstData::ConstF64(v) = inst.data {
           if v.is_finite() {
               out.push_str(&format!("  v{} = {};\n", id, v));
           } else {
               emit_unsupported_for_verification(inst, out);
           }
       }
   }
   ```
   Re-run the test from slice 1; also re-run `emit_const_variants` unchanged to confirm the
   finite case (`ConstF64(2.0)`) still renders exactly as before (regression guard against a
   false positive).
3. **Self-hosted: red.** In `compiler/tests/test_c_emitter.vow`, add
   `check_nonfinite_f64_const_fails_closed()` modeled on
   `check_expanding_string_helper_falls_back` (~line 387): hand-build an `IrFunction` with one
   block containing `ir_inst_new(0, IOP_CONST_F64(), ITY_F64(), Vec::new(), IDATA_CONST_F64(), <bits>, 0, String::from(""))`
   followed by `mk_inst_return(1, 0)`, for each of the bit-pattern vectors below, call
   `emit_c_function(...)`, and assert the output contains
   `unsupported_op_vow_id_str()` (via the existing `vow:` sentinel format already used elsewhere
   in this file) and does not contain a bare `inf`/`nan`-shaped literal. Wire the new check into
   `main()`'s sequence alongside `check_wide_constants_are_non_modelable()`. Confirm it fails.
   Test vectors (`dv` as the i64 reinterpretation of the f64 bit pattern — cross-check each
   against the Rust test via `<value>.to_bits() as i64`):
   - `+inf` → `9218868437227405312`
   - `-inf` → `-4503599627370496`
   - `NaN`  → `9221120237041090560`
   Regression vector (must still render normally, no sentinel):
   - `f64::MAX` → `9218868437227405311`
4. **Self-hosted: green.** In `compiler/c_emitter.vow`'s `emit_inst`, inside the
   `if op == IOP_CONST_F64() { ... }` branch, add the finite check on the bit pattern *before*
   the existing render line, without touching the existing line's indentation:
   ```vow
   if op == IOP_CONST_F64() {
       if ((inst.dv >> 52) & 2047) == 2047 {
           emit_unsupported_for_verification(out, inst);
           return;
       }
       out.push_str(str4(String::from("  v"), i64_to_string(id), String::from(" = "),
           cemit_str2(format_f64_bits(inst.dv as u64), String::from(";\n"))));
       return;
   }
   ```
   (`(bits >> 52) & 0x7FF == 0x7FF` is the standard IEEE-754 double check: all-ones exponent means
   `inf` or `NaN`, regardless of sign/mantissa.) Re-run slice 3's test; also re-run the existing
   `check_wide_constants_are_non_modelable` and any other existing `test_c_emitter.vow` checks
   unchanged to confirm no regression.
5. **Both: full gate.** Run `cargo test -p vow-verify` (Rust), then rebuild
   `build/vowc` (`scripts/bootstrap.sh --skip-cargo`) and run
   `build/vowc test compiler/tests/test_c_emitter.vow` (self-hosted), confirming both pass and
   that no other `test_c_emitter.vow` check regressed.

## 5. Verification surface

- No ESBMC-provable *contract* changes — nothing here touches `requires`/`ensures` on any
  user-facing or compiler-internal function; this is pure C-emission logic.
- The new behavior IS part of the verification surface in the sense that it changes what the
  **verifier's own C model** does for a hypothetical non-finite `ConstF64`: instead of emitting
  invalid C (undefined behavior for ESBMC's parser), it emits a well-formed, always-false
  `__ESBMC_assert`, which ESBMC will dutifully report as a violated property tagged with the
  reserved sentinel vow_id, already classified by `main.vow`'s existing dispatch as a verifier
  limitation (→ `Skipped` build status). No new ESBMC properties to reason about beyond what the
  existing `UNSUPPORTED_OP_VOW_ID` machinery already proves/handles for every other fail-closed
  site in this file.
- No `tests/run/*.vow` or `examples/` fixtures need to grow. Those exercise the normal
  lex→parse→lower pipeline, which (per #1255 and confirmed above) can never produce a non-finite
  `ConstF64` from real source — hand-built IR in the two `c_emitter`-local test files is the only
  way to exercise this path, consistent with the issue's own framing.

## 6. Risk areas

- **Effect system / binary fixed point:** none. The guard adds zero new effects and zero new
  runtime calls — `emit_unsupported_for_verification` is already a plain, effect-free, pure
  string-builder function called from many existing sites in both files. No ripple into
  `emit_c_function`/`emit_c_module`/their callers' signatures.
- **`BTreeMap`/`HashMap`/stack-slot layout:** unaffected — no IR shape change, no new instruction
  kind, no new codegen path. `vow-clif-shim` is untouched (this code never runs through
  Cranelift; it only feeds ESBMC's C model).
- **`parse → print → parse` idempotency:** unaffected — no syntax change, no printer change.
- **`cargo clippy --all -- -D warnings`:** the added `if v.is_finite() { .. } else { .. }` branch
  is ordinary; no new lint surface expected. Double-check the self-hosted inline `if` block
  doesn't leave a now-unreachable `return` warning in the Rust-mirrored logic — it won't, since
  this is Vow source, not Rust, and the self-hosted compiler has no clippy-equivalent gate wired
  into CI today.
- **Dual-compiler drift:** the two implementations use slightly different finiteness tests
  (`f64::is_finite()` on a real Rust `f64` vs. a manual exponent-bit-mask check on the self-hosted
  `i64` bit pattern) because the two IRs store the value differently (native `f64` vs. raw bits).
  This is not a drift risk in the "equivalence review" sense — both compilers reject exactly the
  same set of IEEE-754 bit patterns (all-ones exponent), just via different but equivalent
  mechanisms dictated by each IR's existing representation. Note this explicitly in the PR body
  so a reviewer doesn't flag the asymmetry as unintended.
- **Test placement:** the self-hosted test must live in `compiler/tests/test_c_emitter.vow` (not
  under `tests/run/`), because only `vow test`'s directory-scan module resolution lets a test file
  `use` internal compiler modules like `c_emitter`/`ir` (confirmed via
  `docs/spec/cli.md`'s "Module resolution for directory scans" note and the existing
  `compiler/tests/test_region.vow` precedent). A `tests/run/*.vow` fixture would not resolve
  `use c_emitter;` without extra, unsupported module-root wiring.

## 7. Out of scope (do not bundle into this PR)

- **`ConstF32` guard** — deferred per §2's scope cut. File a follow-up issue: "fix
  `ConstF32`'s self-hosted `dv` encoding (decimal vs. bit-pattern) before adding a finite guard;
  also confirm/fix the Rust `ConstF32` formatter's integer-valued-float edge case
  (`format!(\"{}f\", 2.0f32)` → `2f`, which is valid C by coincidence but untested)." Do not
  silently fix the `dv` convention bug as a side effect of this PR — it's a pre-existing,
  currently-unreachable, unrelated correctness issue, not part of "add a finite guard."
- **Any change to `is_modelable`/`non_modelable_reason`/`first_unsupported_opcode_name`** in
  either `region.rs`/`region.vow` or `c_emitter.rs`/`c_emitter.vow` — `ConstF64` correctly stays
  "modelable" at the opcode-classification level; this PR only adds a value-level escape hatch
  inside the existing render arm.
- **No new abort/panic infrastructure** for the self-hosted compiler (self-reexec test harness,
  `[panic]` effect threading, etc.) — rejected in §2; do not revisit in this PR.
- **No refactor of `emit_inst`'s 21-parameter signature** — tempting given its size, but unrelated
  to this issue and would bundle an unrelated cleanup into a defense-in-depth chore.
- **No touch to `ConstF32`/`ConstF64` handling anywhere outside the two `c_emitter` files** (e.g.
  `vow-codegen/src/cranelift_backend.rs`, `compiler/clif.vow`) — those are real-codegen backends,
  not the ESBMC verification model this issue targets, and are unaffected by this guard.
