# Plan: issue #1030 — narrow-integer contextual range/width lowering applied per-site

## 0. Status check (do this before writing any code)

The issue's empirical repro was captured against PR #995's branch at `54507bf9`. That PR has
since merged as `f5d2a16e`, and **findings #1 and #2 (return-position / trailing-position
literal range checks) are already fixed** — in both compilers, predating #995 entirely
(`1dbef4daa`, 2026-08-21, "feat(ir): add two-limb i128 and u128 literal constants (#1063)").
Verified empirically on current `HEAD` (`90e57098`):

```
fn g() -> i16 { return 32768; }   // compile-time LiteralOutOfRange, both ./target/release/vow and build/vowc
fn f() -> i8 { 200 }              // compile-time LiteralOutOfRange, both compilers
```

Do **not** re-fix findings #1/#2. Do add one small regression-lock slice (Slice 1 below) since
no `tests/error/*_return_literal_out_of_range.vow`-style fixture exists yet for the narrow
(i8/i16/u16/u32) widths at return/trailing position — only `i128_return_literal_out_of_range.vow`
covers this shape today. Losing this silently to a future refactor is the actual risk.

Findings **#3, #4, #5 are still live** — confirmed by building both compilers from `HEAD`
(`cargo build --release -p vow -p vow-runtime`, then `scripts/bootstrap.sh --no-verify`) and
running repros in `--mode debug`. Additionally, **probing sibling sites during planning turned
up three more instances of the same defect that the issue did not name**: struct-literal field
initialization, `Vec<T>` index assignment, and `match`-expression result narrowing when all arms
are literal/arithmetic markers. All six are the same root cause, confirmed by a second probe
round using `u64` in place of the narrow types (see §2): **every one of these sites already
works correctly for `u64`**, because `u64` routes through a context-propagation mechanism
(`record_wide_expected_ast_context` / `record_wide_marker_context` / `wide_literal_contexts`)
that narrow integers (i8/i16/i32/u8/u16/u32) never plug into. The bug is not "narrowing is
missing at six places" — it is "one propagation mechanism recognizes three widths (`u64`,
`i128`, `u128`) and excludes the other seven," which is the single root cause this issue's title
names ("applied per-site instead of uniformly").

## 1. Problem restated

Vow's checked arithmetic operators (`+!`, `-!`, etc.) must trap on overflow at the *declared*
operand width, not at the `i64` width every bare integer literal is speculatively lowered at.
For wide types (`u64`, `i128`, `u128`) a context-propagation mechanism already pushes the
declared target type down into literals and control-flow branches *before* they are lowered, so
the checked operator picks the correct width on the first pass. For narrow types
(`i8`/`i16`/`u16`/`u32`, and to a lesser extent `i32`/`u8`), that mechanism's type-recognition is
hardcoded to `{u64, i128, u128}` at four leaf points, so the same literals are lowered
speculatively at `i64`, the checked op never traps, and a later truncating cast (or, for `Option`
payload reads, no cast at all) silently produces a wrapped or outright wrong value. A smaller,
independent gap means a `let`-annotated local `Option<T>`'s element type is never tagged for
downstream `match` reads unless `T` came through a function argument (`GetArg`), so even once the
construction-time trap is fixed, `Some(v) => v + 1`-style post-extraction arithmetic still runs
at the wrong width.

## 2. Empirical defect inventory (file:line, both compilers, confirmed by running `u64` analogs)

All `u64`-analog repros below (e.g. `fn f(b: bool) -> u64 { if b { 0 -! 1 } else { 1 } }`) were
built with `target/release/vow` and `build/vowc` (`--no-verify --no-cache --mode debug`) and
**all trap correctly** (`{"error":"ArithmeticOverflow"}`, exit 134). The narrow (`i8`) versions
of the same six shapes do **not** trap — they print the wrapped/raw value. This is the go/no-go
signal for Slice 2.

| Site | Rust (`vow-ir/src/lower/mod.rs`) | Self-hosted (`compiler/lower.vow`) | Status |
|---|---|---|---|
| Field assignment `s.x = expr` | `2246-2253` (`Ty::I128\|U128` filter) | `3719-3723` (**already unfiltered/correct**) | Rust-only fix |
| Struct-literal init `S { x: expr }` | `3044-3058` (`Ty::I128\|U128` filter, both the pre-record at 3052 and narrow call at 3057) | `3871-3877` (`field_ty == ITY_I128()\|\|ITY_U128()`) | both |
| `Vec<T>` index assign `v[i] = expr` | `known_index_assignment_ty`, `4405-4414`, line 4413 `.filter(matches!(Ty::I128\|U128))` | `3744-3751` (`elem_ty == ITY_I128()\|\|ITY_U128()`) | both |
| Enum/Option/Result payload construct | `3253-3275` (`payload_tys` per-field filter) + `3290-3303` (`contextual_wide_payload_ty`) | `4019-4049` (same shape) | both — see §3 for why Option/Result need a *different* fix than user enums |
| `if`-expr both branches markers | `lower_integer_marker_as`, `4656-4691` — no `ExprKind::If` arm | `2385-2431` — no `EXPR_IF()` arm | both |
| `match`-expr all arms markers | same function, no `ExprKind::Match` arm; `expr_is_coercible_int_marker` (`1419-1457`) also never recognizes `Match` at all | same, `2068-2101` | both |

Confirmed via direct repro (not just reading): `nested if`, `return if …`, and
`match o { Option::Some(_) => 127 +! 1, Option::None => 0 }` all hit the same missing-arm gap as
the bare `if`, since they all funnel through `lower_narrow_literal` → `lower_integer_marker_as`
at the outer `return`/function-trailing-expression site (`mod.rs:2161-2163`, `5114-5125`).
`match` with *literal* patterns (`true => …`) is not valid Vow syntax
(`UnsupportedPattern`) — use enum/`Option` patterns for match regression fixtures.

User-defined (non-Option/Result) enum payloads go through `ctx.enum_variant_payload_tys`
(`mod.rs:3238-3243`, populated only from user `enum` declarations, `5344-5396`) — confirmed
broken via a standalone repro (`enum Wrapper { Val(i8) } … Wrapper::Val(127 +! 1)`, prints `128`
untrapped, both compilers). `Option`/`Result` are **not** in that table (they are not
user-declared), so their construction-site narrowing is governed by the external
"expected ast type" propagation instead — see next section.

## 3. Root-cause mechanism and the two fixes it implies

### 3a. The wide-context propagation family (primary fix, both compilers)

Four tightly-coupled functions already exist specifically to push a *known* target integer type
down into a speculatively-lowered marker expression **before** it is lowered, so the checked
operator picks the right width the first time, with zero post-hoc patching:

- `record_wide_expected_ast_context(ctx, expr, expected: &AstType)` (`mod.rs:4487-4557`) — walks
  `Block`/`If`/`Match`/`Loop`/`Generic` (including `Option<T>`/`Result<T,E>` payload extraction,
  `4537-4554`) down to a leaf `AstType::Named`, then matches the leaf name against
  `"u64"|"i128"|"u128"` only (`4525-4536`).
- `record_wide_marker_context(ctx, expr, ty)` (`4433-4477`) — the `Ty`-level twin; recurses into
  `UnaryOp(Neg)`/`BinaryOp`/`Block`/`If`/`Match`/`Loop`; gated to `matches!(ty, U64|I128|U128)`
  (`4434`), with an *additional* `wide_context_contains_control_flow` gate for `I128`/`U128` only
  (`4481`) — `U64` records unconditionally.
- `record_wide_control_flow_context` (`4479-4485`) — thin wrapper, same gate.
- The `Lit::Int` arm of `lower_expr` (`1558-1567`) — consults `ctx.wide_literal_contexts`, emits
  a real-width constant via `emit_narrow_integer_constant` only for `Some(ty @ (U64|I128|U128))`.

**Entry points already call these, unfiltered, at every site in the table above**:
`known_assignment_ast_type` (`4333-4340`, delegates to `known_expr_ast_type`, **no type filter**)
feeds field/index-assignment RHS; `Stmt::Let`'s `record_wide_expected_ast_context(ctx, init,
expected)` (`4866-4868`, **no type filter**) feeds every annotated `let`; struct-literal's
`record_wide_expected_ast_context(ctx, field_expr, &expected)` (`3050`, **no type filter**) feeds
every field init. This is why `u64` already works at all six sites without any per-site special
case — the plumbing is universal; only the four leaf functions' *type recognition* excludes
narrow widths.

**Recommended fix**: broaden the four leaf points to recognize all ten integer widths (use
`narrow_int_width(ty).is_some() || ty == Ty::U64` as the unified gate — this is the existing
`vow-ir/src/lower/mod.rs:4230-4234` helper already used by the already-correct `let`/ident-assign
sites), **keeping the `wide_context_contains_control_flow` sub-gate for every narrow type**
(mirror `I128`/`U128`'s model, *not* `U64`'s unconditional one). Reasoning: straight-line narrow
markers (`let x: i8 = 127 +! 1;`, bare `return 127 +! 1;`, plain call arguments) are **already
correct today** via the separate post-hoc `narrow_int_width`/`lower_narrow_literal` path
(`2201-2203`, `4897-4900`, call-args) — only control-flow-shaped markers (`if`/`match`/`loop`
whose arms are themselves markers) are broken, because that post-hoc path's
`lower_integer_marker_as` (`4656-4691`) has no `If`/`Match` recursion arm and never will need
one if the eager path already resolves the width. Gating the new narrow recording to
control-flow-containing expressions only avoids double-recording (and thus double-lowering) the
straight-line cases that already work.

**Mandatory companion change — the short-circuit guard**: `lower_narrow_literal`
(`mod.rs:4697-4726`, specifically `4701-4708`; self-hosted `compiler/lower.vow:2436-2456`,
specifically `2443-2446`) early-returns `original` unmodified when `wide_literal_contexts`
already recorded a matching type for this expression, **currently gated to
`matches!(ty, U64|I128|U128)`**. If the leaf gates above are broadened without broadening this
guard in lockstep, every narrow marker that the new propagation handles eagerly will *also* be
re-lowered via `lower_integer_marker_as` at the outer call site — emitting the checked operator
**twice** (once from the eager pass, once from the redundant post-hoc re-lowering), doubling
`ArithOverflowReachable` verification conditions and, if the duplicate is actually reachable in
codegen, double-executing the checked op (harmless for a value that's about to be discarded, but
wasteful IR and a verifier-surface regression). Broaden this guard's match arm in the same
commit as the leaf gates — do not land one without the other.

**This changes a documented design decision.** The doc comment at `mod.rs:4693-4696`
(`compiler/lower.vow:2433-2435`) currently says: "Marker expressions are re-lowered at that
width so checked operations keep their overflow behavior; **control-flow results are explicitly
reduced after their Phi**." After this fix, that is no longer true for *newly-covered*
control-flow markers — they are resolved *before* lowering, like straight-line markers always
were. Update the comment; do not leave it describing stale behavior.

**Keep b64bf731's `renarrow_if_branch_result`/match-arm renarrowing untouched.** It still
handles the case this fix does not cover: one branch is a genuine narrow-typed value and the
*other* is a marker (e.g. `if v > 200 { 200 } else { v }` where `v: u8`) — there the merge's own
`phi_ty` is resolved by comparing sibling branch types to each other, not by external context,
and the existing post-merge re-narrow is the only mechanism for it.

### 3b. Option/Result post-extraction tagging (required regardless of 3a, both compilers)

3a fixes the *construction-time* trap (`Option::Some(127 +! 1)` now overflows correctly) but
does **not** fix the issue's literal repro, which is about a *wrapping* read after extraction:

```vow
let o: Option<i8> = Option::Some(127);
match o { Option::Some(v) => v + 1, Option::None => 0 }   // must wrap to -128, not compute at i64
```

`variant_payload_ty(...).or_else(|| ctx.inst_option_elem_ty.get(&ptr_id).copied())
.unwrap_or(Ty::I64)` (`mod.rs:3415-3417`) falls back to `Ty::I64` whenever `inst_option_elem_ty`
was never tagged — which today only happens for `GetArg` (`5088-5092`, self-hosted
`5585-5590`), not for a `let`-annotated local built from a direct `Option::Some(...)`
construction. Fix: in `Stmt::Let`'s `AstType::Generic` handling (`mod.rs:~4935-4954`, currently
has a `"Vec"` arm populating `inst_vec_elem_types` and no `"Option"`/`"Result"` arm; self-hosted
twin `compiler/lower.vow:~5051-5067`, `atag == TY_GENERIC()`), add an arm that calls the already
general-purpose `option_named_elem_type(ast_ty, &ctx.type_aliases)` (`mod.rs:870-878`, already
unrestricted — takes any scalar) / self-hosted `lower_ast_type_option_elem_ty`, then
`ctx.inst_option_elem_ty.insert(val, elem_ty)` / `lctx_tag_option_elem(ctx, val, elem_ty)` —
tagging straight from the **declared annotation**, exactly like `GetArg` already does, rather
than trying to infer the type from the (speculatively-i64) already-lowered payload value at the
construction site. This is annotation-driven and therefore correct regardless of how the payload
was computed (literal, binop, function call).

### 3c. Fallback (only if 3a regresses or under-delivers)

If implementation reveals that broadening the wide-context family is unsafe in some corner (e.g.
interacts badly with the `i128`/`u128` two-limb constant representation, or the control-flow gate
doesn't cleanly separate from `wide_context_contains_control_flow`'s existing semantics), the
per-site mechanical fallback is fully specified in the table in §2: broaden each hardcoded
`Ty::I128 | Ty::U128` / `ITY_I128()||ITY_U128()` pattern to `narrow_int_width(ty).is_some()`
(Rust) / `narrow_int_width(ty) != 0` (self-hosted) at each listed file:line, individually. This
is strictly more sites to touch and does not fix the `if`/`match` marker-merge case (§3a's
unique contribution) — it would need the `lower_integer_marker_as` `If`/`Match` arms added
separately via a new side table (considered and rejected as primary: would need a new
`ctx.inst_marker_if_merge: HashMap<InstId, …>` populated at every if/match-merge site, more
surface area for the same outcome §3a already reaches through existing plumbing).

## 4. Files to touch

- `vow-ir/src/lower/mod.rs` — `record_wide_expected_ast_context`, `record_wide_marker_context`,
  `record_wide_control_flow_context`, the `Lit::Int` arm of `lower_expr`, `lower_narrow_literal`'s
  short-circuit guard, the `Stmt::Let` `AstType::Generic` arm, the doc comment at `4693-4696`.
- `compiler/lower.vow` — the twins of all of the above (search by the `// Twin of` comments
  already present at `2035`, `2480` etc.).
- `vow/src/cache.rs` — bump `COMPILE_CACHE_ABI_VERSION` (currently
  `"...wide-guard-index-v7"`, `cache.rs:~25`) to a new suffix, add the matching
  `compile_cache_key_invalidates_pre_*` regression test (follow `1cac4480`'s pattern exactly —
  that commit did this for the previous narrow-Vec-index fix). Without this, users with a warm
  `VOW_CACHE_DIR` silently relink pre-fix miscompiled objects.
- `tests/run/*.vow` — new fixtures (§5).
- `tests/error/*.vow` — one new regression-lock fixture for findings #1/#2 (§5, Slice 1).
- No `docs/spec/*.md` changes. This closes a conformance gap against already-documented
  semantics: ADR 0001 decision 5 ("checked / traps on overflow... no new operator family") and
  `docs/spec/grammar.md`'s checked-operators section ("widths i8/u8 through i64/u64 are
  modelled") already state the intended behavior; no syntax, semantics, or CLI surface changes.

## 5. TDD slices

Each slice is a separate commit. Build both compilers once at the start
(`cargo build --release -p vow -p vow-runtime -j<N>`, then `scripts/bootstrap.sh --skip-cargo
--no-verify`) and re-run `tests/run_tests.sh` / the relevant `cargo test -p vow-ir` after every
slice — regressing any existing `narrow_*`, `i64_*wide*`, `i128_*`, `u64_*` fixture is an
immediate stop.

1. **Regression-lock findings #1/#2 (no production change).** Add
   `tests/error/i8_return_literal_out_of_range.vow` (`fn f() -> i8 { return 200; }`, `// TEST:
   error-code LiteralOutOfRange`) and `tests/error/i16_trailing_literal_out_of_range.vow`
   (`fn f() -> i16 { 32768 }`). Confirms current (already-fixed) behavior and prevents silent
   regression during the slices below. Go/no-go: both already pass with zero production changes.

2. **3a core: broaden the wide-context family, Rust side only, behind the existing test suite.**
   Change the four leaf points in `vow-ir/src/lower/mod.rs` listed in §3a (gate +
   `wide_context_contains_control_flow` requirement for every narrow type) and the
   `lower_narrow_literal` short-circuit guard in the same commit. New fixtures:
   `tests/run/narrow_if_branch_checked_overflow.vow` (`fn f(b: bool) -> i8 { if b { 127 +! 1 }
   else { 0 } }`, `// TEST: exit 134`, `// TEST: stderr
   "{\"error\":\"ArithmeticOverflow\"}"` — follow `tests/run/narrow_checked_expression_overflow.vow`'s
   exact header convention), plus nested-if, `return if…`, and `match o { Option::Some(_) => 127
   +! 1, Option::None => 0 }` variants in the same file or siblings. Add unsigned marker-shape
   siblings (`u8`/`u16`/`u32`, `0 -! 1` inside the same shapes) — the signed probes above do not
   by themselves establish unsigned correctness. **Go/no-go**: every existing
   `tests/run/if_expr_merge_width.vow`, `narrow_annotated_local.vow`, `match_arm_merge_width.vow`,
   and every `tests/error/i128_*`/`i64_*wide*` fixture must stay green — if any regresses, stop
   and fall back to §3c for this slice specifically rather than debugging the shared propagation
   path under time pressure.
3. **3a mirror: self-hosted `compiler/lower.vow`.** Same four functions' twins. Run the same new
   `tests/run/*.vow` fixtures through `build/vowc` (rebuild via `scripts/bootstrap.sh --skip-cargo
   --no-verify` first) — the shared harness (`scripts/full_test.sh`'s `run_promoted_run_tests`,
   Section 4) exercises both `./target/release/vow` and the self-hosted binary against the same
   fixture, so one `.vow` file covers both compilers once this slice lands.
4. **3a fallout check: struct-literal, field-assign, Vec-index sites.** These should now pass
   *without further code changes* if 3a's propagation reaches them (§2 confirms
   `record_wide_expected_ast_context` is already called unfiltered at all three call sites). Add
   `tests/run/narrow_field_assignment_overflow.vow`, `tests/run/narrow_struct_init_overflow.vow`,
   `tests/run/narrow_vec_index_assign_overflow.vow` (same `exit 134`/`ArithmeticOverflow`
   convention) to confirm. If any of the three still fails after Slice 2/3, that specific site's
   §2 table entry is the fallback patch (§3c) — apply it as its own small commit, it does not
   block the others.
5. **User-defined enum payload.** Add `tests/run/narrow_enum_payload_overflow.vow`
   (`enum Wrapper { Val(i8) }`, construct with `127 +! 1`). Expected to pass from 3a via
   `enum_variant_payload_ast_types` (populated for user enums, `mod.rs:5344-5396`) feeding
   `record_wide_expected_ast_context` the same way struct fields do — confirm; if not, the
   fallback is broadening `payload_tys`'s filter at `mod.rs:3260-3269` directly (§2 table).
6. **3b: Option/Result post-extraction tagging.** Add the `"Option"`/`"Result"` arm to
   `Stmt::Let`'s `AstType::Generic` handling in both compilers (§3b). Regression fixture: the
   issue's own literal repro, as a **stdout** fixture (not a trap fixture — this is wrapping `+`,
   not checked `+!`):
   `tests/run/narrow_option_annotated_local.vow` — `let o: Option<i8> = Option::Some(127); match
   o { Option::Some(v) => print_i64((v + 1) as i64), Option::None => print_i64(0) }`,
   `// TEST: stdout "-128\n"` (mirrors the already-passing `GetArg`-based
   `tests/run/narrow_option_parameter.vow`, same expected wraparound, just for a locally
   constructed value instead of a parameter). Also add a `+!`-checked companion
   (`tests/run/narrow_option_annotated_local_overflow.vow`) asserting the trap, to cover both the
   3a (construction-time trap) and 3b (post-extraction width) halves of the same bug
   independently.
7. **Cache ABI bump.** Bump `COMPILE_CACHE_ABI_VERSION` in `vow/src/cache.rs`, add the
   `compile_cache_key_invalidates_pre_*` test following `1cac4480`. Do this last, after all
   codegen-affecting slices land, so the version string's doc comment can name this fix
   specifically (follow the existing comment's pattern of listing each prior fix it supersedes).
8. **`vow-ir` unit test coverage (coverage-gate requirement).** Per project memory, `codecov/patch`
   is a blocking 95%-of-new-lines gate and the `.vow` corpus runs an uninstrumented binary — the
   `tests/run/*.vow` fixtures above do **not** count toward it. Add `#[cfg(test)]` unit tests in
   `vow-ir/src/lower/mod.rs` directly asserting the lowered `InstData::Integer` width on the
   checked-add instruction for at least one marker-in-if-branch case and one
   Option-local-construction case (follow the existing style at `narrow_int_width_and_divergence_are_exhaustive_over_ty`,
   `~6223`, and the `lower_assignment_updates_identifier_binding`-style tests, `~7854`, for the
   harness pattern: build a tiny `FnDef` AST, lower it, assert on `ctx.func`/`InstData`).

## 6. Verification surface

No new ESBMC modeling is required. `docs/spec/grammar.md`'s checked-operators section already
states "widths i8/u8 through i64/u64 are modelled" — the verifier C model already treats checked
arithmetic as width-parametric; this fix makes the *lowering* finally emit the width the verifier
already knows how to model, it does not add a new proof obligation shape. Expect new
`ArithOverflowReachable` warnings (`docs/spec/errors.md#arithoverflowreachable`) on existing
`.vow` programs that have an `if`/`match`-merged checked op in a narrow-typed position and were
previously silently emitting unreachable-looking i64-width ops — this is correct, not a
regression; any such warning in the existing `tests/run/`/`examples/` corpus should be reviewed
per-case (either the overflow is genuinely reachable, in which case the fixture's contract or
inputs need adjusting, or it's a false-tight bound, in which case leave it — do not weaken a
contract to silence it, per CLAUDE.md's Contract Authoring rules). Run `vow build` (verify-on, not
`--no-verify`) over the full `tests/run/` and `examples/` corpus once after Slice 6 specifically
to catch this.

## 7. Risk areas

- **Binary fixed point.** Every slice must pass `scripts/bootstrap.sh`'s stage2/stage3 SHA-256
  match. The self-hosted `compiler/lower.vow` mirror (Slice 3) is not optional parallel work —
  land it in the *same* commit as the Rust change it mirrors, or the bootstrap triple-test's
  stage-1/stage-2 binaries diverge in behavior (not just hash) until both land.
- **`BTreeMap` determinism in `vow-clif-shim`.** This fix does not touch stack-slot allocation or
  `slot_map` — it only changes *which width* a value is computed at, not the IR's block/Upsilon
  structure. Low risk, but confirm no new `HashMap` is introduced in `LowerCtx` (use
  `BTreeMap`/`Vec` if any new per-function ordering-sensitive state is needed — it shouldn't be,
  since `wide_literal_contexts` already exists and is a `HashMap<usize, Ty>` keyed by expression
  pointer identity, not iterated in codegen order).
- **`parse → print → parse` idempotency.** Unaffected — this is a lowering-only fix, no AST or
  printer changes.
- **`cargo clippy --all -- -D warnings`.** The broadened match arms in the four leaf functions
  will likely trip `clippy::match_same_arms` if the narrow/wide cases end up identical — collapse
  with `|` patterns rather than suppressing.
- **Scope creep via `3c`.** Do not let a single stubborn fallback site (§5 Slice 4/5) turn into
  debugging the entire wide-context mechanism under time pressure — land the working slices,
  file a fast-follow issue for whichever single site doesn't fall out "for free" from 3a, same as
  #995 did for this class of bug.
- **The `wide_context_contains_control_flow` gate's narrow-type behavior is a judgment call,
  not a settled fact.** §3a recommends mirroring i128/u128's gated model over u64's unconditional
  one, reasoned from "straight-line narrow markers already work via the post-hoc path." Slice 2's
  go/no-go check (existing fixtures must stay green) is the actual arbiter — if gating the wrong
  way causes a regression, the fix is changing the gate condition, not the overall architecture.

## 8. Out of scope (do not bundle)

- Packed/naturally-aligned struct field layout for narrow ints (ADR 0001, explicitly deferred
  there, unrelated to this bug).
- Any change to 128-bit (`i128`/`u128`) codegen, verification, or the "refused at access" rule
  for 128-bit struct fields/enum payloads (`docs/spec/grammar.md`'s 128-bit section) — this fix
  must not alter i128/u128 behavior at all; the broadened gates add narrow widths alongside the
  existing wide ones, they do not change the wide ones' own gating logic.
- `saturating`/other new arithmetic operator families — not implicated by this issue.
- Reformatting or renaming the `wide_literal_contexts`/`record_wide_*` family to reflect its new,
  broader scope (e.g. renaming to `record_contextual_marker_context`). Tempting for clarity, but
  a pure rename bundled with a behavior change makes the diff harder to review and bisect. File a
  follow-up `refactor:` commit/issue if the names read wrong after this lands.
