# Architecture review — vow — 2026-09-16

**Scope**: `vow-types/src/check.rs` and `vow-ir/src/lower/mod.rs` — the two hottest compiler
files in the last 120 commits (check.rs 14 touches, lower/mod.rs 10, the latter freshly churned
by #1288 two days ago). Hot-spot inference per step 1; no path argument was given.
**Picked**: `builtin-result-tag` — see [PR #1290](https://github.com/vow-lang/vow/pull/1290) and `.architecture/backlog.md`
**Degradations**: none. `gh` authenticated; explore sub-agent available; advisor available for
adjudication.

Diagram convention (replaces the upstream HTML legend): **solid edges are the interface** a caller
sees; **dashed edges are inside the implementation**, behind the seam.

## Candidates

### builtin-result-tag — extract the builtin-result heap/Option classifier  ·  Strong  ·  score 22/25

- **Files** — `vow-ir/src/lower/mod.rs`: the `tag_builtin_result` policy at `mod.rs:201-251`
  (single caller `mod.rs:1808`), sited beside the already-pure `narrow_intrinsic_target`
  (`mod.rs:161-183`) and the `vow_static_builtin_to_runtime` name table (`mod.rs:~60-159`).
  File-count estimate: **1**.
- **Score** — **22/25** (leverage 4, locality 4, blast radius 1, heat 5)
  - *leverage 4*: mirrors the landed `builtin_constructor_spec` seam (#1268) exactly — one dispatch
    site, a name-keyed classification table, a pure spec output — which scored leverage 4 and merged.
    The ~40-name policy is currently reachable only through full lowering, so its whole test surface
    is unlocked by the extraction, not merely shrunk.
  - *locality 4*: after the seam, a change to which builtins return heap/Option values is a one-line
    edit in one pure function, and its correctness is checkable in one unit test rather than through
    an end-to-end lowering fixture.
  - *blast radius 1*: one file, no published interface; `BuiltinResultTag` is crate-private.
  - *heat 5*: `lower/mod.rs` is the second-hottest file (10/120 commits) and `tag_builtin_result`
    itself was edited **two days ago** by #1288.
- **Problem** — `tag_builtin_result` is a **shallow** module: its interface (`name -> mutations on
  ctx`) is entangled with a large, pure classification policy (~40 builtin names → one of three tag
  kinds: a `String` heap struct-type, a `Vec` heap struct-type, or `Option<elem_ty>`). The decision
  is a total function of `name` alone, but it can only be reached by constructing a `LowerCtx` and an
  `InstId` and inspecting side effects afterwards — so it has **no locality of test**. The list has
  **already drifted**: #1288 was a bug fix that had to hand-add `proc_sample` to the String-heap arm,
  the exact class of miss a pure, testable table prevents.
- **Deletion test** — deleting `tag_builtin_result` and inlining it at the caller would **move**
  complexity into the already-large call site and scatter the name policy; extracting the pure
  classifier out of it instead **concentrates** the policy in one testable place. The seam passes.
- **Solution** — introduce a crate-private `enum BuiltinResultTag { StringHeap, VecHeap,
  OptionOf(Ty) }` and a pure `fn builtin_result_tag(name: &str) -> Option<BuiltinResultTag>` that
  performs the `_try`+`narrow_intrinsic_target` early path first, then the explicit name match.
  `tag_builtin_result` becomes a thin `&mut self`-style wrapper that matches on the tag and applies
  the `ctx.inst_struct_type` / `ctx.inst_option_elem_ty` inserts. Rust-only; `compiler/lower.vow`
  is untouched (the change is behaviour-preserving, so it introduces no new drift — the existing
  "keep in sync" invariant is unchanged, and the sync comment moves to sit above the seam).
- **Benefits** — **leverage**: the entire name→tag policy becomes unit-testable with a plain `&str`,
  no `LowerCtx`. **locality**: the tag decision and its verification live in one function.
  **test surface**: goes from zero direct coverage to exhaustively pinnable, including the two
  fall-through traps `i16_to_u8_try` / `i64_to_i32_try` (whose `u8`/`i32` targets are *not* in
  `narrow_intrinsic_target`'s supported set, so they must reach the explicit arms) and the
  early-path-only `u64_to_u32_try`.

Before — the policy is fused into the ctx-mutating function; the caller and any test must go through
lowering:

```mermaid
graph LR
  C[lower_call caller] --> T[tag_builtin_result]
  T --> P1[_try/narrow early path]
  T --> P2[String-heap names]
  T --> P3[Vec-heap names]
  T --> P4[Option-of-Ty names]
  T --> M[ctx.inst_struct_type / inst_option_elem_ty inserts]
```

After — the pure classifier is one seam; the wrapper only applies the ctx inserts; tests hit the
seam directly:

```mermaid
graph LR
  C[lower_call caller] --> T[tag_builtin_result wrapper]
  U[unit test] --> S[builtin_result_tag name -> Option BuiltinResultTag]
  T --> S
  T -.-> M[ctx inserts by tag kind]
  S -.-> P1[_try/narrow early path]
  S -.-> P2[String-heap]
  S -.-> P3[Vec-heap]
  S -.-> P4[Option-of-Ty]
```

- **Recommendation strength** — Strong.

### call-argument-coercion-action — pure call-arg coercion decision  ·  Worth exploring  ·  score 22/25

- **Files** — `vow-codegen/src/cranelift_backend.rs` (`coerce_call_argument`, ~L398-432). Estimate 1.
- **Score** — 22/25 (leverage 4, locality 4, blast radius 1, heat 5). Ties the pick on total.
- **Problem** — a pure coercion decision (`actual_bits`, `expected_bits`, `is_i128`, `signed` →
  extend/truncate/none) fused with `builder.ins()` emission.
- **Deletion test** — concentrates: a genuine pure seam.
- **Pick note** — **lost the 22-22 tie on the deterministic tie-break**: equal blast (1) and heat
  (5), then most-recently-touched file — `lower/mod.rs` (#1288, 2026-09-14) postdates
  `cranelift_backend.rs` (#1279, 2026-09-14, earlier in the log). Kept `proposed`. Soft note: a
  codegen change also carries a byte-identical-bootstrap obligation whose triple-test is heavier to
  run than a single-crate `cargo test`; that is a scheduling caution, not a rubric filter.

### coerce-context-argument-epilogue — collapse the contextual-coercion epilogue  ·  Speculative  ·  score 21/25

- **Files** — `vow-types/src/check.rs`, ~10 sites. Estimate 1.
- **Score** — 21/25 (leverage 3, locality 5, blast radius 1, heat 5).
- **Problem** — a repeated `check_expr + check_contextual_integer_literal_ranges + can_context_coerce
  + emit-mismatch` epilogue.
- **Deletion test** — mostly moves: the proposed helper is a shallow 4-param wrapper of 3 statements
  and stays `&mut self`, so the **test surface does not improve**. Runner-up by score, but a weaker
  *deepening* than the pick; the top two are within 1 point (22 vs 21), so this is the natural next
  firing only if its leverage is re-argued.

### negation-verdict — pure unary-negation verdict  ·  Worth exploring  ·  score 20/25

- **Files** — `vow-types/src/check.rs:2161-2183` (`UnaryOp::Neg` arm). Estimate 1.
- **Score** — 20/25 (leverage 3, locality 4, blast radius 1, heat 5). **Rescored 18→20**: the
  explore pass confirmed it is a *total pure function of `operand_ty`* (3-way
  `{Unsigned, NonNumeric, Ok}`, both error branches return `Unit`, Ok returns the operand type),
  testable with zero `&mut self` — leverage 2→3. The literal-folding directly above (L2138-2158)
  stays at the call site.
- **Deletion test** — concentrates. Clean textbook seam; the strongest runner-up *by deepening
  quality* even though it scores below the epilogue candidate.

### arm-pattern-support-classifier — pure unsupported-arm classifier  ·  Worth exploring  ·  score 20/25

- **Files** — `vow-types/src/check.rs:3167-3225` (`validate_arm_pattern`). Estimate 1.
- **Score** — 20/25 (leverage 3, locality 4, blast radius 1, heat 5).
- **Problem** — 8 unsupported-arm reasons already computed as a neutral `Option<(msg, hint)>` then
  emitted separately; the classifier match never touches `self`.
- **Deletion test** — concentrates, but the honest gain is "exercise the classifier without a
  `Checker`", not "untangle policy from emission" — the value is already half-realised. The correct
  seam returns an enum (caller maps variant → wording), not the `(msg, hint)` text.

## Dropped

| Candidate | Dropped because |
|---|---|
| `esbmc-ce-description-heuristic` | Leverage 1 — inert; the only caller destructures `Failed(_)` and discards the description, so extraction deepens nothing observable. Re-checked 2026-09-16: caller unchanged. Do not re-surface. |
| `solver-classify-function` | Already a pure, unit-tested seam (`test_classify_*`). No shallowness to remove. Re-checked 2026-09-16. |

(Hard filters only. `comparison-operand-verdict` was **rescored down** — leverage 3→2, total 18 —
because the explore pass found the tautology half already extracted (`zero_comparison_verdict`
check.rs:565, `never_negative_operand` check.rs:1927) and the composite verdict cannot be fully pure
(`never_negative_operand` reads `self.nonneg_casts`). It stays `proposed`, not vetoed — leverage 2
is eligible, just low.)

## Too large to automate

| Candidate | Why |
|---|---|
| `clif-shim-region-parity` | Blast radius 4 — crosses the `vow-clif-shim` / `vow-codegen` crate seam and touches codegen output. Re-checked 2026-09-16: still large. A human should schedule it. |
| `division-abort-spec` | Blast radius 4 — triplicated policy across `vow-codegen` / `vow-verify` / `vow-runtime` plus byte-identical-bootstrap risk. Human-scheduled. |

## Pick

**`builtin-result-tag`**, 22/25. It tied `call-argument-coercion-action` on total (22) and won the
rubric's deterministic tie-break: equal blast radius (1) and heat (5), broken by most-recently-touched
file — `lower/mod.rs` was edited by #1288 on 2026-09-14, after `cranelift_backend.rs`'s last touch
(#1279, same day, earlier in the log). Independently, it is the cleaner unattended pick: its
behaviour is pinnable by a single-crate `cargo test -p vow-ir` on a pure seam, whereas the codegen
candidate's byte-identical-bootstrap obligation is heavier to discharge in one firing. The top two
were within 1 point of the runner-up *candidate* by score (`coerce-context-argument-epilogue`, 21),
which is noted per ranking.md; but that runner-up is a shallow `&mut self` epilogue dedup with no
test-surface gain, so `builtin-result-tag` is also the stronger *deepening*.

## Design

Design-it-twice: four interfaces were produced in parallel by sub-agents (A/B/C/D), then adjudicated
against fixed criteria in order — **depth → locality → seam placement → test surface → blast radius**
— by the advisor. All four expose the same pure test surface (`name -> tag`, no `LowerCtx`), preserve
the `_try`+`narrow_intrinsic_target` early-path ordering, and keep the load-bearing `ends_with("_try")`
guard (without it `_wrap`/`_sat` names would wrongly classify as `Option`, since
`narrow_intrinsic_target` also parses those modes). There is exactly one caller, so the seam's value
is testability and locality, not multiple adapters.

### Winner — Design A (enum verdict)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuiltinResultTag { StringHeap, VecHeap, OptionOf(Ty) }

fn builtin_result_tag(name: &str) -> Option<BuiltinResultTag> { /* _try+narrow early path, then name match */ }

fn tag_builtin_result(ctx: &mut LowerCtx, name: &str, result: InstId) {
    match builtin_result_tag(name) { /* apply ctx.inst_struct_type / inst_option_elem_ty inserts */ }
}
```

- **Hides**: the two-phase precedence, the ~40-name table, and the narrow-target gap (u8/i32 not
  supported → those `_try` names fall through) behind one `&str` signature.
- **Seam placement**: the volatile axis is *name → result shape*; A puts exactly that behind the seam
  and keeps the ctx-encoding strings (`"String"`/`"Vec"`/`"Option"`) **outside** it, in the wrapper —
  the policy answers a typed fact, the string is merely how today's `HashMap<InstId, String>` spells
  it. Composes the existing pure `narrow_intrinsic_target` rather than duplicating its parse.
- **Test surface**: illegal states unrepresentable (an `Option` result always carries its element
  `Ty`); testable with a plain `&str`.
- **Blast radius**: one file — enum + fn + wrapper + test + comment move.

### Runner-up design — C (const data table)

Same pure seam as A, but the explicit-name policy is a `const &[(&str, BuiltinResultTag)]` with a
linear-scan lookup. **Lost on blast radius after tying A on depth/locality/seam/test-surface.** Its
one unique gain over A — an *iterable* table a future test could walk to mechanically check
`compiler/lower.vow` parity — is out of scope for this behaviour-preserving change, so it pays blast
radius (const table + a keys-unique test to recover the `unreachable_patterns` lint A keeps for free,
plus ~20 repeated `Heap("String")` rows vs A's `|`-grouped arm) for leverage this PR cannot cash. The
C sub-agent reached the same conclusion: "adopt the pure seam regardless; pick the table only if the
parity check is on the roadmap."

### Losing designs

- **B — fold the tag into `vow_static_builtin_to_runtime`** (a third tuple field across ~110 rows).
  Buys a genuinely attractive anti-drift invariant (a builtin has a heap tag **iff** its runtime
  return is `Ty::Ptr`) and true one-row builtin adds. **Lost on seam placement and blast radius**: it
  welds a codegen concern (extern symbol / IR return type) to a heap-tracking concern (the tag), and
  introduces a precedence flip that is behaviour-neutral only under a name-set disjointness nothing
  enforces — a latent hazard. It also rewrites two large existing tests, carries a catalogue-`Plain`
  trap (a future generated heap-returning op would silently lose its tag), and weakens structural
  parity with `compiler/lower.vow`. Its invariant is preserved as a follow-up finding below rather
  than adopted.
- **D — minimal tuple** `Option<(&'static str, Option<Ty>)>` carrying the ctx struct-name string
  across the seam. Smallest wrapper. **Lost on test surface**: the tuple permits illegal states —
  `("Option", None)` yields a half-tagged `Option` (a real `pin_to_root` bug class) that A's enum
  makes unrepresentable — and couples the seam to the ctx representation.

### Follow-up finding (not implemented; carried to the PR body)

Design B surfaced a real latent invariant worth recording: every `Ty::Ptr`-returning static builtin
in `vow_static_builtin_to_runtime` has a heap/Option tag, and no non-`Ptr` row does. A future
table-fold, or a C-style iterable table, could **assert** this and turn the hand-maintained
`compiler/lower.vow` sync comment into a mechanical check. Out of scope here (behaviour-preserving,
one-file); noted for a human to schedule. No ADR proposed.
