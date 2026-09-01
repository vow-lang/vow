# Plan: issue #1096 — `Vec<T>` index reads for narrow `T` lower as `i64`

## 1. Problem restated

`__vow_vec_get_val` always returns an untyped 8-byte `i64` (every `Vec` slot is 8 bytes), and
`ExprKind::Index` lowering in both compilers emits that call with IR type `Ty::I64` and never
narrows it, even though it already knows the element's declared type. Because binary-operator
lowering derives its operand width directly from the IR type of its already-lowered operands
(`vow-ir/src/lower/mod.rs:1559` reads `ctx.inst_ty(lhs_id)`), a `v[0] - v[1] >> 1` expression over
a `Vec<u32>` computes the subtraction and the shift at 64-bit signed width and only narrows to
`u32` at the very end (if at all) — by which point `Shr` has already sign-extended instead of
zero-filling, and the result is silently wrong. The fix is localized: narrow the `Index` result to
the element's real type *at the read site*, immediately after the `__vow_vec_get_val` call, so
every downstream operator sees the correct type the same way it already does for a `let`-bound
narrow local (`let a: u32 = v[0];`, which is the documented workaround). This reuses machinery the
codebase already has for exactly this purpose in both compilers — no new type-system concept, no
spec change: `grammar.md` already specifies `-` as wrapping and `>>` as logical-for-unsigned; this
is a conformance bug, not a semantics change.

## 2. Files to touch

Production code:
- `vow-ir/src/lower/mod.rs` — `ExprKind::Index` arm (currently ~lines 3888–3904).
- `compiler/lower.vow` — `EXPR_INDEX()` branch inside `lower_expr` (currently ~lines 4052–4067).

Tests:
- `vow-ir/src/lower/mod.rs` — new `#[test]` fn(s) in the existing inline test module (style: see
  `narrow_enum_payload_fieldget_stays_i64`), asserting the IR shape (an `IntCast` right after the
  `__vow_vec_get_val` `Call`, and `WrappingSub`/`Shr`/`WrappingMul`/`WrappingDiv` emitted at the
  narrow `Ty`, not `I64`).
- `tests/run/*.vow` — one or two new fixtures using the issue's reproducer shapes (see §3, §4).
  These run against **both** compilers automatically via `scripts/full_test.sh` Section 4 (it globs
  `tests/run/*.vow` and checks `// TEST: stdout "..."` against both `vowr` and `build/vowc`) — no
  separate self-hosted test-list file needs updating.
- `compiler/test_vec.vow` — optionally extend this existing self-hosted smoke test (already
  exercises `Vec<i64>` push/index/assign) with a narrow-element case, matching its existing
  "return a distinct nonzero code per failing assertion" convention. Not required for coverage
  (the `tests/run/` fixture already covers both compilers) — only worth doing if it's cheap.

No `docs/spec/*.md` changes: this fixes an implementation defect against already-documented
semantics (wrapping `-`/`*`, logical `>>` for unsigned types). Nothing about syntax, semantics,
builtins, operators, effects, or CLI surface changes.

## 3. TDD slices

Each slice is small, vertical, and independently reviewable/bisectable.

1. **Red: add the regression fixture.**
   `tests/run/vec_index_narrow_arith.vow` — encode the issue's `Vec<u32>` reproducer
   (`(v[0] - v[1]) >> 1` expecting `2147483647`) plus the multiply/shift and multiply/div variants
   from the issue body (`(v[0] * v[0]) >> 4` and `(v[0] * v[0]) / 7u32` on `4294967295u32`,
   expecting `0` for both), with `// TEST: stdout "..."` directives. Confirm it currently fails
   against both `target/release/vow` and `build/vowc` (run each manually first — don't wait for a
   full `full_test.sh` pass to see red). This is the shared oracle the next two slices turn green.

2. **Red→Green (Rust): narrow the `Index` result in `vow-ir/src/lower/mod.rs`.**
   Add an IR-level unit test first (red): assert that for `let v: Vec<u32> = ...; v[0] - v[1]`,
   the lowered `WrappingSub` instruction has `Ty::U32` (not `Ty::I64`), and that an `IntCast[I64 ->
   U32]` sits between the `__vow_vec_get_val` `Call` and the first use of its result. Then fix the
   `ExprKind::Index` arm: after computing `elem_ast_type` (already computed today, just unused for
   this) and the raw `i64` call result, convert `elem_ast_type` to an `Ty` via the existing
   `lower_ty_with_linear(ast_type, &ctx.linear_owner_names, &ctx.type_aliases)` (same helper the
   `Cast` arm already uses), and run the raw result through the existing `lower_narrow_literal(ctx,
   expr, result, elem_ty)` helper (same helper `let`-with-annotation already uses) — it's already a
   no-op for non-narrow `Ty` (`I64`/`U64`/`Bool`/`F32`/`F64`/unknown), so wide/scalar-typed Vecs are
   provably unaffected. **Move the `propagate_vec_element_metadata` call and the
   `ctx.inst_declared_ast_types.insert` onto the post-cast `InstId`, not the pre-cast one** — if
   `lower_narrow_literal` emits a new `IntCast` instruction, any metadata attached to the old
   (now-unreferenced) id becomes silently unreachable for chained accesses (`Vec<Vec<u32>>`,
   `Vec<Option<u32>>`, etc.). Confirm the unit test goes green, then confirm the Rust half of the
   `tests/run/` fixture from slice 1 passes (`cargo build --release -p vow`, run the fixture by
   hand).

3. **Red→Green (self-hosted): mirror the fix in `compiler/lower.vow`.**
   The `EXPR_INDEX()` branch already computes `elem_ast_tid` the same way
   (`lctx_generic_ast_arg_tid(ctx, lctx_get_declared_ast_tid(ctx, vec_ptr), String::from("Vec"), 0)`)
   but only uses it for `lctx_tag_declared_ast_tid` metadata. Convert `elem_ast_tid` to an IR type
   via `lctx_lower_ast_ty(ctx, elem_ast_tid)` (the self-hosted analog of `lower_ty_with_linear`,
   already used by the `EXPR_CAST()` branch), and route the raw result through
   `lower_narrow_literal(ctx, a, eid, result, elem_ty)` (`eid` here is the enclosing `lower_expr`
   parameter — the whole `Index` expression — matching the Rust call's `expr` argument). Move
   `lctx_propagate_vec_elem` and `lctx_tag_declared_ast_tid` onto the narrowed id for the same
   reason as slice 2. Rebuild (`scripts/bootstrap.sh --skip-cargo`) and confirm the self-hosted half
   of the slice-1 fixture passes under `build/vowc`.

4. **Green: full regression sweep.**
   Run `scripts/full_test.sh` (or at minimum its Section 4 run-tests loop) end to end so the new
   fixture is checked against both compilers in the same harness that will gate CI, and so any
   fixture that happened to rely on the old (buggy) width — none are expected, since no existing
   `tests/run/*.vow` fixture exercises narrow-typed `Vec` index reads inside an operator chain
   without an intermediate `let` — is caught immediately. Run `cargo test --all` and
   `cargo clippy --all -- -D warnings` for the Rust side.

5. **Optional: checked-arithmetic follow-on regression.**
   The same mislowering silently suppressed overflow traps for checked ops reached through a Vec
   read: `v[0] +! v[1]` on `Vec<u32>` computed the checked add at `i64` width, where a `u32`-range
   overflow essentially never overflows `i64`, so `ArithmeticOverflow` never fired. Once slices 2–3
   land this starts trapping correctly. Add one small `tests/run/*.vow` fixture (`// TEST: exit
   134` or whatever this repo's convention is for an expected `VowViolation` abort — check an
   existing checked-arithmetic-through-container fixture, e.g. `narrow_checked_expression_overflow.vow`,
   for the exact convention) asserting `v[i] +! v[j]` now traps at the narrow width. This is a
   direct, minimal consequence of the same fix, not scope creep — but keep it as its own commit so
   it's separately revertable if the exact trap-exit-code convention needs adjusting.

## 4. Verification surface

- No contracts change, no new `requires`/`ensures`/`invariant` are introduced, and no ESBMC-facing
  IR shape changes in a way that needs new coverage beyond what CEGIS already proves for
  `WrappingSub`/`Shr`/`WrappingMul`/`WrappingDiv`/`CheckedAdd` etc. at each narrow `Ty` — those
  operator semantics per-`Ty` are already verified elsewhere; this fix only makes `Vec` index reads
  select the *correct* `Ty` for them, the same `Ty` a `let`-bound narrow local already gets.
- One indirect verification-soundness improvement is worth flagging, not fixing further: any
  existing `vow` contract that reasons about arithmetic over a `Vec<narrow-int>` element read
  directly in a `requires`/`ensures` predicate (without an intermediate narrow `let`) was previously
  verified against the *wrong* (64-bit) width. After this fix such a predicate is checked at the
  *correct* width, which can only turn a previously-accepted-by-luck proof into either (a) still
  provable at the correct width, or (b) a new counterexample. This is desired soundness, not
  something to work around — if it surfaces in an existing benchmark or example, the fix is to
  correct the contract or the implementation, not to weaken the contract to fit ESBMC.
- `tests/run/` grows by 1–2 fixtures (§3.1, §3.5); no `benchmarks/` or `examples/` changes are
  needed — this is a compiler-internals fix, not a language-surface addition.

## 5. Risk areas

- **Binary fixed point (`compiler/lower.vow` / `vow-clif-shim`):** the fix adds one extra `IntCast`
  instruction (and one extra stack slot) per narrow-typed `Vec` index read. This is the same kind of
  instruction the codebase already emits routinely (e.g. at every narrow `let` binding), so it does
  not introduce a new instruction shape, a new `BTreeMap`/`HashMap` choice, or a new slot-allocation
  path — it just makes an existing path fire at a new call site. Still, re-run the triple-bootstrap
  fixed-point check (`scripts/concat_vow.sh` → stage A/B/C → `sha256sum`) after the self-hosted
  change lands, since it touches `compiler/lower.vow` codegen ordering directly.
- **`parse → print → parse` idempotency:** not at risk — this change is entirely inside IR lowering
  (`lower.vow` / `lower/mod.rs`), not the parser or printer. No AST shape changes.
- **`cargo clippy --all -- -D warnings`:** keep the Rust-side change minimal and pattern-matched to
  the existing `lower_narrow_literal`/`lower_ty_with_linear` call shapes used elsewhere in the same
  file, to avoid introducing new lints (unnecessary clones, etc.).
- **Metadata ordering bug (self-inflicted risk to watch for while implementing):** as called out in
  slices 2–3, attaching `propagate_vec_element_metadata` / `inst_declared_ast_types` /
  `lctx_propagate_vec_elem` / `lctx_tag_declared_ast_tid` to the *pre-cast* raw-`i64` id instead of
  the *post-cast* narrowed id would silently break metadata lookups for any code chaining another
  operation off a narrow-typed `Vec` element (nested `Vec<Vec<T>>`, `Vec<Option<T>>`, `Vec<enum>`).
  This is easy to get backwards because the existing code was written when `result` and "the id
  metadata attaches to" were the same value; after the fix they usually are not. The IR unit test in
  slice 2 should assert on the *returned* instruction, not just "some `IntCast` exists somewhere",
  to catch this ordering mistake directly.
- **Scope discipline:** issue #1030 lists five *other* narrow-width call sites (return-position
  literals, field/aggregate assignment RHS, `Option<T>` locally-constructed payloads, `if`-branch
  Phi merges) that are explicitly out of scope here and tracked separately. Do not fix those as
  part of this PR even though the shared-helper reuse (`lower_narrow_literal`) makes it tempting —
  each is its own reviewable slice per #1030's own triage.
- **`u64`/`i64` Vec elements are not touched.** `lower_narrow_literal` is a no-op outside
  `{I8,U8,I16,U16,I32,U32,I128,U128}`, so `Vec<u64>` index reads keep their current (already
  logically-i64-shaped, bit-compatible) behavior. Whether `Vec<u64>` index reads have an analogous
  signed-vs-unsigned `Shr`/`Div`/`Rem` bug is a plausible but *unconfirmed* adjacent question — it
  is not in the issue's reproducer table and should be filed as a separate issue if confirmed, not
  folded into this fix.

## 6. Out of scope

- The five other narrow-width call sites from issue #1030 (return-position literals,
  field/aggregate-assignment RHS narrowing, `Option<T>`-payload propagation for locally-constructed
  values, `if`-branch Phi-merge narrowing, and the checker-level `check_integer_literal_range` gaps
  in `vow-types/src/check.rs` / `compiler/checker.vow`). Separate issue, separate PR.
- Any potential `Vec<u64>` analog of this bug (see Risk Areas) — investigate and file separately if
  real; do not speculatively fix without a confirmed reproducer.
- `Vec` index *writes* (`v[i] = expr`) and the existing `known_index_assignment_ty` /
  `lctx_...assignment...` machinery that already handles `i128`/`u128` width on the LHS-assignment
  path — reads and writes are different bugs with different fix shapes; this issue and its
  reproducer are exclusively about reads feeding surrounding arithmetic.
- No refactor of `propagate_vec_element_metadata` / `known_expr_ast_type` / `lower_narrow_literal`
  into a single more-generic helper, even though this fix is the second or third call site to reuse
  them by hand. That kind of consolidation is exactly the shape of change #1030 asks for across *all*
  six-plus sites at once — doing it piecemeal here would preempt that follow-up's design space for
  no benefit to this bug fix.
- No changes to `__vow_vec_get_val`/`__vow_vec_set_val` runtime signatures or the 8-byte Vec slot
  layout (`vow-runtime/src/lib.rs`) — the fix is entirely in lowering, not the runtime ABI.
