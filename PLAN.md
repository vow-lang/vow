# Plan: #1267 — self-hosted checker never validates struct-literal field types or names

## 1. Problem restated

`compiler/checker.vow`'s `EXPR_SLIT` case (struct literals) type-checks each field's value
expression via `check_expr(e, m, feid)` but discards the result into `let _ftid`, so it never
compares the field's actual type against the struct's declared field type. It also never emits a
diagnostic when the struct name itself doesn't resolve (`sidx == -1` currently only changes the
*return type* of the expression, silently, to `CTY_UNKNOWN()`), and never emits a diagnostic when
a field name in the literal doesn't match any declared field — the linear search over declared
fields just falls through with no `else` branch. `vow-types/src/check.rs:2804-2830` (Rust) already
performs all three checks correctly. This is the same shape as the `EXPR_ASSIGN` gap fixed in
#1264 (commit `ed4c9b9e`): Rust validates a computed type, self-hosted computes it and drops it.

## 2. Files to touch

- `compiler/checker.vow` — the only production file. All three checks live in the single
  `EXPR_SLIT` block inside `check_expr_inner` (currently `compiler/checker.vow:3102-3140`).
- `tests/error/struct_literal_field_type_mismatch.vow` (new)
- `tests/error/struct_literal_unknown_struct.vow` (new)
- `tests/error/struct_literal_unknown_field.vow` (new)

**No `vow-types/src/check.rs` (Rust) change.** Rust is already correct — it is the oracle this fix
restores parity with, not a co-requirement. `ed4c9b9e` is direct precedent for a self-hosted-only
diff under the repo's "modify both compilers" rule: that rule targets genuine semantic changes,
not fixing a self-hosted implementation bug against already-correct Rust behavior.

**No `docs/spec/*.md` change.** No new syntax, semantics, builtin, operator, effect, or CLI flag is
introduced. `TypeMismatch` is an existing, already-documented error code
(`docs/spec/errors.md` §TypeMismatch) being emitted from a previously-silent code path.

## 3. TDD slices

**Procedure for every slice below:** don't hand-guess the expected `TEST: error-code` /
`TEST: error-count` values. For each fixture, run it through the *Rust* compiler first —
`./target/release/vow build --no-verify <fixture> 2>/dev/null | python3 -m json.tool` — and read
`diagnostics[].error_code` and the count of `severity == "error"` entries straight from that
output. Rust is the oracle; self-hosted is red until it matches Rust exactly, green once it does.
Use a fresh cache per invocation, `VOW_CACHE_DIR=$(mktemp -d -p "$TMPDIR")`, for every build during
red/green iteration — the compile cache keys on source, not compiler binary, so a stale object from
before your `compiler/checker.vow` edit will silently mask the fix (see project memory: "vow
compile cache ignores compiler changes"). Rebuild `build/vowc` after every `compiler/checker.vow`
edit via `scripts/bootstrap.sh --skip-cargo` before re-testing.

### Slice 1 — field value has the wrong type

**Fixture** `tests/error/struct_literal_field_type_mismatch.vow`:
```vow
module StructLiteralFieldTypeMismatch

struct Foo { x: i32 }

fn main() -> i32 {
  let f: Foo = Foo { x: true };
  0
}
```
`true` against a `bool`-vs-`i32` mismatch is unambiguous and has no integer-literal-range
interaction, so it isolates exactly the coercibility check.

- **Red:** rebuild `build/vowc` from `main` (pre-fix) if not already current, run the fixture with
  a fresh `VOW_CACHE_DIR` — confirm it exits 0 with no diagnostics (the bug).
- **Green — production change** in the `EXPR_SLIT` block's field loop:
  - `let _ftid: i64 = check_expr(e, m, feid);` → `let ftid: i64 = check_expr(e, m, feid);`
  - Add `let mut expected_tid: i64 = -1;` before the inner field-name search loop (reset once per
    outer-loop field, alongside the existing `field_index` declaration).
  - In the inner loop's match-found branch, alongside the existing
    `check_contextual_integer_literal_ranges(e, a, feid, expected_tid_here)` call, assign
    `expected_tid = expected_tid_here` (the `ts_arg_get(...)` result already computed there).
  - After the inner loop, guarded by `if sidx != -1 { ... }` (already the case — this whole
    block is inside that guard) and `if expected_tid != -1 { ... }`:
    ```vow
    if !is_opaque(expected_tid) && !is_coercible(e.ts, ftid, expected_tid) {
        let msg: String = String::from("field `");
        msg.push_str(fname);
        msg.push_str(String::from("` of struct `"));
        msg.push_str(sname);
        msg.push_str(String::from("` expects `"));
        msg.push_str(ty_value_display_name(e.ts, expected_tid));
        msg.push_str(String::from("`, found `"));
        msg.push_str(ty_value_display_name(e.ts, ftid));
        msg.push_str(String::from("`"));
        env_emit_error_code(e, EC_TYPE_MISMATCH(), msg, expr_span(a, feid));
    }
    ```
    Guard order (`is_opaque` on the *target*, `expected_tid`, mirroring `EXPR_ASSIGN`'s
    `!is_opaque(lhs_tid) && !is_coercible(e.ts, rhs_tid, lhs_tid)`) matters for a real reason, not
    just precedent: `is_opaque` carries `requires: tid >= 0`, so it must never be called with the
    `-1` sentinel. It's called here only inside `if expected_tid != -1`, so the precondition holds
    by construction — keep that nesting exactly as described, don't hoist the `is_opaque` check
    outside the `expected_tid != -1` guard.
  - Set the fixture's `// TEST: error-code` / `// TEST: error-count` from the Rust oracle run.
- **Green — verification:** rebuild, rerun fixture with a fresh cache dir; self-hosted's exit code,
  status, and diagnostics must match the Rust oracle. Then run
  `scripts/bootstrap.sh --skip-cargo` end-to-end (not skip-verify) — `compiler/*.vow` is the
  largest struct-literal corpus in the repo and Rust already accepts all of it, so this is the real
  regression test for the new coercion check. If it fails, `is_coercible` is stricter than
  `can_context_coerce` on some case actually present in the compiler's own source — fix that
  specific gap in `is_coercible`/`is_opaque` as part of this slice; do not weaken the new
  `EXPR_SLIT` check to work around it.

### Slice 2 — unknown struct name

**Fixture** `tests/error/struct_literal_unknown_struct.vow`:
```vow
module StructLiteralUnknownStruct

struct Foo { x: i32 }

fn main() -> i32 {
  let f: Foo = Bar { x: 1 };
  0
}
```
`Bar` is used only as a struct-literal constructor, never as a type annotation, so the only
diagnostics in play are the struct-literal ones (an `unknown type` diagnostic from `resolve_ast_ty`
would be a different error code and would break the parity comparison). Wrapping it in
`let f: Foo = ...;` deliberately exercises the return-type fix below: with a bad return type the
`let` binding's own coercion check should *also* fire, so the Rust oracle for this fixture is
expected to report **two** `TypeMismatch` diagnostics (unknown struct, then Unit-vs-`Foo`) — run it
against Rust and confirm this, don't assume.

- **Red:** confirm self-hosted exits 0. Today `sidx == -1` emits nothing and returns
  `CTY_UNKNOWN()`, which `is_opaque` treats as coercible to anything, so even the surrounding
  `let`'s own (already-correct) coercion check is silently skipped.
- **Green — production change**, both in the `EXPR_SLIT` block:
  - Immediately after `let sidx: i64 = env_lookup_struct(e, sname);`, add:
    ```vow
    if sidx == -1 {
        let umsg: String = String::from("unknown struct `");
        umsg.push_str(sname);
        umsg.push_str(String::from("`"));
        env_emit_error_code(e, EC_TYPE_MISMATCH(), umsg, expr_span(a, eid));
    }
    ```
    (once per literal, not once per field — this sits before the field loop, matching Rust's
    `None` arm which emits once then still type-checks each field's value expression.)
  - Change the trailing `if sidx == -1 { return CTY_UNKNOWN(); }` to
    `if sidx == -1 { return CTY_UNIT(); }`. This is the one change in this issue that isn't purely
    additive, so call it out precisely: Rust's `None` arm returns `Ty::Unit`, not an opaque/unknown
    marker. `CTY_UNKNOWN()` is used throughout `checker.vow` as an "I gave up, don't cascade
    errors" marker and `is_opaque` treats it as coercible to everything; using it here swallows
    every downstream use of the bogus literal (assignments, returns, further field access) that
    Rust would still catch. `CTY_UNIT()` is the ordinary "this expression has type `()`" marker
    used elsewhere in the same file (e.g. block/statement fallthrough) and participates normally in
    `is_coercible` (falls through to the `from_tid == to_tid` check, no special-casing) — it is not
    itself opaque, so downstream mismatches surface exactly as they do in Rust.
- **Green — verification:** rebuild, rerun with a fresh cache dir; self-hosted's diagnostics must
  match the Rust oracle's two-`TypeMismatch` result exactly (sorted code list, not order). Rerun
  Slice 1's fixture for no regression. Run `scripts/bootstrap.sh --skip-cargo` again — this is the
  practical check that the `CTY_UNKNOWN()` → `CTY_UNIT()` return-type change has no collateral
  effect on the existing corpus (nothing in `compiler/*.vow` should be relying on an
  already-buggy unknown-struct literal silently type-checking).

### Slice 3 — extra / misspelled field name

**Fixture** `tests/error/struct_literal_unknown_field.vow`:
```vow
module StructLiteralUnknownField

struct Foo { x: i32 }

fn main() -> i32 {
  let f: Foo = Foo { x: 1, y: 2 };
  0
}
```
`x` is present and correctly typed; `y` is not declared. Reading `vow-types/src/check.rs:2804-2830`
confirms Rust's struct-literal check only iterates the literal's own fields — it never checks that
every *declared* field was supplied — so this fixture should isolate exactly one diagnostic
("no field `y`") with no secondary "missing field `x`... wait, x is present" noise. Confirm the
exact oracle count against Rust before finalizing `TEST: error-count` — don't assume.

- **Red:** confirm self-hosted exits 0 (the inner search loop exhausts on `y` with no match and
  currently does nothing).
- **Green — production change:** reuse the `expected_tid` sentinel introduced in Slice 1. After the
  inner search loop, add the missing `else` arm:
  ```vow
  if expected_tid != -1 {
      // (Slice 1's coercion check)
  } else {
      let nfmsg: String = String::from("struct `");
      nfmsg.push_str(sname);
      nfmsg.push_str(String::from("` has no field `"));
      nfmsg.push_str(fname);
      nfmsg.push_str(String::from("`"));
      env_emit_error_code(e, EC_TYPE_MISMATCH(), nfmsg, expr_span(a, feid));
  }
  ```
  This whole block is already nested inside `if sidx != -1`, so an already-unknown struct doesn't
  also claim "has no field" for each of its fields — matching Rust's `None` arm, which skips
  per-field name/type checks entirely and only calls `check_expr`.
- **Green — verification:** rebuild, rerun with a fresh cache dir, compare to the Rust oracle.
  Rerun Slices 1 and 2's fixtures for no regression. Run `scripts/bootstrap.sh --skip-cargo` once
  more.

### Closing integration step (after all three slices are green)

- `scripts/full_test.sh` end to end (not `--skip-cargo` this run) — Section 7 walks all of
  `tests/error/*.vow` including the three new fixtures and asserts the Rust/self-hosted
  `error_code` multisets match (`scripts/parity.py::compare_error`); the bootstrap triple-test
  (fixed-point) and Section 0b (concrete block-region parity) run as part of the same script and
  must stay green.
- `tests/run_tests.sh` (developer-only harness) — its Phase 5 (`tests/error/`) is what actually
  parses and enforces the new fixtures' `TEST: error-code` / `TEST: error-count` / `TEST: stderr`
  directives against `build/vowc`; `full_test.sh`'s Section 7 does not parse these directives at
  all, it only compares the raw diagnostics JSON, so Phase 5 is the only place these annotations
  are checked mechanically.
- `cargo clippy --all -- -D warnings` — expected a no-op confirmation since no Rust file changes;
  run once anyway per the standard quality gate.
- `cargo test --all` — expected unaffected (no Rust source touched); run once for completeness.

## 4. Verification surface

No contracts, codegen, or C-model/ESBMC surface is touched. The fix operates entirely on type IDs
already computed by the existing `check_expr` / `env_lookup_struct` / struct-metadata machinery and
reuses two pre-existing, already-verified helpers (`is_coercible`, `is_opaque`) rather than
introducing new ones — same reuse pattern as `ed4c9b9e`.

The one contract-relevant detail: `is_opaque(tid: i64) -> bool vow { requires: tid >= 0 }` is a
`vow`-contracted function, and `compiler/checker.vow` is itself subject to ESBMC verification during
`scripts/bootstrap.sh`'s verify pass. The new call site (Slice 1) must never reach `is_opaque` with
the `-1` "no match" sentinel — it doesn't, because the call is nested inside
`if expected_tid != -1`, but this is exactly the kind of thing ESBMC's own `--unwind`-bounded
symbolic exploration would catch if the nesting were wrong, so preserve it structurally rather than
relying on it being "obviously true" by inspection. No new fixtures are needed under `tests/run/`
or `examples/` — this is a compile-time diagnostic path, not a runtime-checked one, so `tests/error/`
is the right home for the regression guard, matching the pattern for every other `TypeMismatch`
fixture in that directory.

## 5. Risk areas

- **Binary fixed point:** no risk by construction — the diff is added `if`/`else` branches and
  `String` concatenation inside an existing function, no new `BTreeMap`/`HashMap` usage, no
  `vow-clif-shim` or stack-slot changes, no nondeterministic iteration order. `run_bootstrap_triple`
  inside `scripts/full_test.sh` (closing integration step) is the actual gate; expect it to pass
  without special handling.
- **`parse → print → parse` idempotency:** unaffected — no AST, token, or printer changes; this is
  checker-only.
- **`cargo clippy --all -- -D warnings`:** unaffected — no Rust file is touched by this plan.
- **`is_coercible` stricter than `can_context_coerce`:** the one real risk, called out in Slice 1 —
  mitigated by running the full self-hosted bootstrap (which compiles all of `compiler/*.vow`)
  after the coercion check lands, before moving to Slice 2.
- **`CTY_UNKNOWN()` → `CTY_UNIT()` return-type change (Slice 2):** the one change in this plan that
  isn't purely additive. It restores Rust-equivalent behavior for *already-broken* code (a struct
  literal naming an undeclared struct), so no correct program's behavior should change — but "no
  correct program relies on this" isn't something the planning stage can exhaustively prove by
  inspection. The Slice 2 bootstrap rerun plus the closing `scripts/full_test.sh` full corpus sweep
  (`tests/run/`, `tests/error/`, `examples/`, `benchmarks/`) are the practical safety net; watch
  their output specifically for anything that previously compiled cleanly and no longer does.
- **Diagnostic count precision:** `tests/run_tests.sh` Phase 5's `TEST: error-count` is an exact
  match, not a lower bound — each fixture's field values must be trivial enough
  (`true`, `1`, `2`) that `check_expr` on them doesn't itself emit an unrelated diagnostic and
  inflate the count. The three fixtures above were chosen with this in mind; if the oracle run in
  any slice shows an unexpected extra diagnostic, treat that as a fixture-design problem to fix,
  not a reason to loosen `TEST: error-count`.

## 6. Out of scope

- **"Did you mean" hints** on the new unknown-struct/unknown-field diagnostics. Rust has this via
  `suggest_similar`/`emit_error_with_hints`; self-hosted has no such hint infrastructure anywhere
  yet (confirmed absent by grep). Adding it would be a separate feature-parity gap, not required by
  this issue's regression guard (which only asks for matching error *codes* and nonzero exit).
- **A "missing declared field" completeness check** (e.g. `Foo { }` when `Foo` declares `x`).
  Confirmed Rust doesn't have one today by reading `vow-types/src/check.rs:2804-2830` — it only
  iterates the literal's own fields. Adding this to self-hosted would introduce a *new* behavior
  that diverges from Rust, not fix a parity gap; if wanted, it's separate follow-up work requiring
  its own Rust-side change first.
- **Refactoring the `EXPR_SLIT` block's control-flow shape** — e.g. extracting the hand-rolled
  linear field-name search into a helper function, or replacing the "advance `field_index` to `nf`
  to exit early" idiom with an actual `break`. Keep the diff additive and in the file's existing
  style; this is a bug fix, not a cleanup pass.
- **Changes to `is_coercible`/`is_opaque` themselves**, unless Slice 1's bootstrap regression run
  turns up a concrete gap in them (see Risk areas) — and even then, fix only the specific gap
  found, don't audit or refactor either function more broadly.
- **`vow-types/src/check.rs` changes.** Rust is already correct; this plan restores self-hosted
  parity with it, per `ed4c9b9e` precedent.
