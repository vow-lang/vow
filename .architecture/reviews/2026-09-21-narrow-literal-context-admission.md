# Architecture review — vow — 2026-09-21

**Scope**: hot spots inferred from the last 120 commits (`vow-types/src/check.rs` 28 commits/90d,
`vow-codegen/src/cranelift_backend.rs` 27, `vow-ir/src/lower/mod.rs` 21, `vow-runtime/src/lib.rs` 21,
`vow-verify/src/c_emitter.rs` 16, `vow-verify/src/esbmc.rs` 13, `vow-ir/src/region.rs` 11), scanned by
two parallel sub-agents against the 40-odd candidates already carded in `.architecture/backlog.md`.
`CONTEXT.md` does not exist in this repo; domain vocabulary is taken from `CLAUDE.md` and
`docs/spec/`. ADRs 0001–0003 were read; none is contradicted by the pick.

**Picked**: `narrow-literal-context-admission` — see `.architecture/backlog.md`
**Degradations**: none. `gh` authenticated, sub-agents available, advisor available.

**Diagram legend**: solid edges are the interface a caller must learn; dashed edges are inside the
implementation.

## Candidates

### narrow-literal-context-admission — one narrowing-admission rule, spelled seven ways · Strong · score 22/25

- **Files**: `vow-ir/src/lower/mod.rs` — the admission predicate at `:1698-1701` (binop operand),
  `:1796-1806` (call argument), `:2125-2135` (assignment to ident), `:3538-3541` (match-result Phi
  re-narrow), `:4598-4602` (`lower_narrow_literal`'s own gate), `:4723-4739` (`let` with a type
  annotation, as an 8-branch `if/else` over type-name strings), `:4961-4964` (fn trailing return).
  13 call sites of `lower_narrow_literal` (`:4597`). **File-count estimate: 1.**
- **Score**: 22/25 — leverage 4, locality 4, blast radius 1, heat 5
  - *Leverage 4*: seven sites stop restating the membership and one deeply-nested caller
    (`lower_narrow_literal`, which already self-gates) stops being second-guessed by four redundant
    pre-filters that silently disagree with it.
  - *Locality 4*: changing the narrowing policy today means finding and editing seven `matches!`
    lists and an `if/else` chain scattered over 3,200 lines; afterwards it is one table.
  - *Blast radius 1*: contained — one file, no published interface changes. `lower_narrow_literal`
    is a private free function; `NarrowContext` would be private to the module.
  - *Heat 5*: `lower/mod.rs` is 21 commits in 90 days, last touched 2026-09-18 (three days ago) by
    the `builtin_method_spec` seam. **This cell is load-bearing**: at heat 4 the candidate scores 21
    and `loop-scope-break-policy` wins outright. Heat 5 is the same grade the 2026-09-18 firing gave
    this same file on the same evidence, so the rubric stays deterministic across firings.
- **Problem**: the question *"must a value of type `T` be re-lowered at its native narrow width
  here?"* is answered in seven places, in three mutually incompatible ways. Four sites spell an
  **8-type** list that omits `U64`; two spell a **9-type** list that includes it; one spells the
  8-type set as an `if/else` chain over `&str` type names, duplicating the `&str -> Ty` map that
  already exists at `:691-724` as `scalar_ty_for_field_type_name`. The interface is as complex as
  the implementation: every caller must know the membership to call the function correctly, even
  though the function already self-gates.
- **Deletion test**: **concentrates.** Deleting the seven inline spellings in favour of one table
  removes the ability to disagree about `U64` at a distance. Deleting `lower_narrow_literal` itself
  would scatter the marker-relowering machinery back to 13 sites — it is already deep; the shallow
  part is the admission predicate wrapped around it.
- **Divergence the table makes visible**: `U64` is handled three ways in one file.
  - *Rejected before the call* at `:1700`, `:1805`, `:2134`, `:3540` — `U64` is simply absent.
  - *Admitted, then conditionally escaped* at `:4600`, by a `U64`-specific guard at `:4604-4611`
    that yields to an explicit `wide_literal_contexts` entry.
  - *Admitted outright* at `:4961-4964` (fn trailing return) and at the nine call sites that
    pre-filter nothing at all.

    So a `u64` **binop operand, call argument, assignment RHS or match arm** is never re-narrowed,
    while a `u64` **trailing return, field-set value, vec-element write and `?`-result** all are.
    The `wide_literal_contexts` escape shows the *intended* policy is "`U64` is admitted but yields
    to an explicit wide context"; the four 8-type pre-filters predate that escape and were never
    updated. This review does **not** change that behaviour — the table reproduces today's
    per-context membership row for row, turning an accident of omission into a reviewable row.
- **Solution**: a pure, context-keyed seam answering the one question, with the four pre-filtered
  contexts and the unfiltered default as explicit rows; the `let` chain composes the existing
  `scalar_ty_for_field_type_name` with the same seam instead of hand-rolling a second name map.
  `lower_narrow_literal` keeps its `wide_literal_contexts` escape and its marker re-lowering; the
  `Shl`/`Shr` right-operand exception at `:1702-1704` stays at the call site, being an operand rule
  rather than a type rule.
- **Benefits**: **leverage** — one row per context replaces seven restatements, and a new narrow
  type is added once rather than seven times. **Locality** — the `U64` question is decided in one
  place, so the next firing that asks "should a `u64` call argument re-narrow?" edits one row.
  **Test surface** — the admission rule becomes directly unit-testable as a pure function; today it
  can only be reached by lowering a whole `FnDef` and inspecting the emitted IR, which is why the
  `u8` and `u64` rows of the existing table test `integer_literals_lower_at_their_native_ir_width`
  (`:6002-6069`) are missing and the `U64` divergence went unnoticed.

```mermaid
graph LR
  B[binop operand] --> P1["matches! 8 types"]
  A[call argument] --> P2["matches! 8 types"]
  S[assign to ident] --> P3["matches! 8 types"]
  M[match Phi] --> P4["matches! 8 types"]
  L["let annotation"] --> P5["if/else over 8 names"]
  R[fn return] --> P6["matches! 9 types"]
  P1 --> LN[lower_narrow_literal]
  P2 --> LN
  P3 --> LN
  P4 --> LN
  P5 --> LN
  P6 --> LN
  LN --> P7["matches! 9 types, self-gate"]
```

```mermaid
graph LR
  B[binop operand] --> T["narrow_literal_target(ctx, ty)"]
  A[call argument] --> T
  S[assign to ident] --> T
  M[match Phi] --> T
  L["let annotation"] --> T
  R[fn return] --> T
  T -.-> LN[lower_narrow_literal]
  LN -.-> W[wide_literal_contexts escape]
  LN -.-> K[marker re-lowering]
```

### loop-scope-break-policy — three loop arms, one of them forgets the break stack · Strong · score 22/25

- **Files**: `vow-types/src/check.rs:2671-2686` (`While`), `:2687-2718` (`ForEach`), `:2719-2755`
  (`Loop`); consumers `:2756-2784` (`Break`), `:2785-2794` (`Continue`); state at `:980` (`in_loop`)
  and `:983` (`break_types_stack`). **File-count estimate: 1.**
- **Score**: 22/25 — leverage 4, locality 4, blast radius 1, heat 5
  - *Leverage 4*: three arms simplify and the `Break`/`Continue` consumers stop reaching past the
    seam into two raw fields whose push/pop pairing nothing enforces.
  - *Locality 4*: loop-scoping policy becomes one table plus one applier that owns push/pop
    symmetrically.
  - *Blast radius 1*: one file, three arms, private state.
  - *Heat 5*: `check.rs` is the repo's highest-churn Rust file at 28 commits/90d, last 2026-09-14.
- **Problem**: each arm hand-rolls `in_loop += 1` / `break_types_stack.push(..)` / `check_block` /
  pop / `in_loop -= 1`, and the pairing is enforced only by eye. **`ForEach` increments `in_loop`
  but never pushes onto `break_types_stack`** (`:2713-2715`), where `While` pushes `None`
  (`:2680-2684`) and `Loop` pushes `Some(Vec::new())` (`:2723-2727`).
- **Deletion test**: **concentrates.** One applier owning the stack discipline replaces three
  hand-paired push/pop sites.
- **Three wrong behaviours that fall out of the one missing line** (verified, not inferred):
  `loop { for i in v { break 42; } break 7; }` merges `42` into the **outer** `loop`'s result type,
  while `vow_syntax::ast::loop_break_values` (`vow-syntax/src/ast.rs:402`, `:431`) never walks a
  for-each body — so the type checker and lowering (`vow-ir/src/lower/mod.rs:4371`, `:4417`) compute
  different break-value sets for the same program. `while c { for i in v { break 42; } }` blames the
  `while` for a break inside a for-each. `fn f(v: Vec<i64>) { for i in v { break 42; } }` is
  **silently accepted** with no diagnostic, the stack being empty.
- **Solution**: `loop_kind_spec(kind) -> LoopSpec { vow_context, break_slot }` where `break_slot` has
  three rows — `Push(None)`, `Push(Some(vec![]))` and `NoPush` — so today's `ForEach` anomaly is
  preserved *exactly* and becomes a visible, commentable row rather than a missing line. A private
  `with_loop_scope` applier owns `in_loop` and the stack; each arm keeps its own type computation.
- **Benefits**: **leverage** — the stack discipline is written once. **Locality** — the fix for the
  `ForEach` anomaly becomes a one-row edit in a follow-up. **Test surface** — `ExprKind::ForEach`
  appears in `check.rs` only twice outside tests (`:1876`, `:2687`) and in **zero** tests; the buggy
  arm is entirely unpinned today.

```mermaid
graph LR
  W[While arm] --> ST["in_loop / break_types_stack"]
  F[ForEach arm] --> ST
  L[Loop arm] --> ST
  BR[Break] --> ST
  CO[Continue] --> ST
```

```mermaid
graph LR
  W[While arm] --> WS["with_loop_scope(kind, ..)"]
  F[ForEach arm] --> WS
  L[Loop arm] --> WS
  WS -.-> SP["loop_kind_spec(kind)"]
  WS -.-> ST["in_loop / break_types_stack"]
  BR[Break] --> ST
  CO[Continue] --> ST
```

### builtin-arg-layout-spec — one arg-index convention, thirteen restatements · Worth exploring · score 21/25

- **Files**: `vow-verify/src/c_emitter.rs` — five lookup tables at `:218-227`
  (`vec_model_receiver_arg`), `:233-240` (`vec_op_value_arg`), `:296-318`
  (`string_model_receiver_arg`), `:320-326` (`string_model_extra_arg`), `:332-339`
  (`map_model_receiver_arg`); seven inline destructurings inside `emit_inst` at `:1346-1351`,
  `:1413-1418`, `:1452-1457`, `:1474-1479`, `:1489-1494`, `:1588-1594`, `:1613-1619`; and one
  arena-offset closure at `:1704-1714`. **File-count estimate: 1.**
- **Score**: 21/25 — leverage 4, locality 4, blast radius 1, heat 4 (`c_emitter.rs`, 16 commits/90d,
  last 2026-09-02).
- **Problem**: thirteen sites answer "for extern `name`, which `inst.args[i]` is the receiver, the
  stored value, the extra operand" — under one invariant convention (plain symbol ⇒ receiver is
  `args[0]`; `_in_arena` ⇒ arena is `args[0]` and receiver is `args[1]`) that is re-derived by hand
  every time, seven of them as ad-hoc `if name == "..._in_arena" { (1,2) } else { (0,1) }`.
- **Deletion test**: **concentrates** — one row per symbol replaces thirteen hand-derivations.
- **Divergence**: `__vow_vec_pin_to_root_val` is absent from `vec_model_receiver_arg` (`:218-226`)
  while its twin `__vow_string_pin_to_root` **is** present in `string_model_receiver_arg` (`:307`).
  It *is* in `is_vec_model_creator` (`:178`), so its result becomes a `__vow_vec_t`, but
  `collect_typed_vars` (`:366-370`) never marks its source `args[0]`, so `emit_inst` (`:1341-1344`)
  can emit `__vow_vec_t v{id} = int64_t v{source};` — exactly the `int64_t = __vow_vec_t` class this
  file's own comment at `:260-262` cites issue #505 for. Latent: in the common flow the source is
  already a vec creator's result.
- **Solution**: `builtin_arg_layout(name) -> Option<ArgLayout { arena_offset, receiver, value, extra }>`,
  with `emit_inst` using one uniform `let arg = |i| inst.args[layout.arena_offset + i].0`.
- **Benefits**: **leverage** across 13 sites; **locality** — a new `_in_arena` extern is one row
  rather than up to three table edits; **test surface** — the convention becomes assertable without
  building an IR function.

```mermaid
graph LR
  E[emit_inst] --> T1[vec_model_receiver_arg]
  E --> T2[vec_op_value_arg]
  E --> T3[string_model_receiver_arg]
  E --> T4[string_model_extra_arg]
  E --> T5[map_model_receiver_arg]
  E --> I["7 inline _in_arena ternaries"]
  E --> C[arena_offset closure]
```

```mermaid
graph LR
  E[emit_inst] --> L["builtin_arg_layout(name)"]
  CT[collect_typed_vars] --> L
  L -.-> R[receiver / value / extra]
  L -.-> AO[arena_offset]
```

### parse-opt-payload-spec — seven identical Option-havoc arms, one calling an undeclared intrinsic · Worth exploring · score 21/25

- **Files**: `vow-verify/src/c_emitter.rs:1636-1686` (7 arms); the same 9-name set re-spelled at
  `:414-422` (`collect_option_vars`) and `:535-543` (`is_known_builtin`). **File-count estimate: 1.**
- **Score**: 21/25 — leverage 4, locality 4, blast radius 1, heat 4.
- **Problem**: every arm emits the identical three-line skeleton — nondet tag, `assume(tag==0||tag==1)`,
  conditional payload nondet — differing only in `(payload_nondet, Option<min>, Option<max>)`.
- **Deletion test**: **concentrates.**
- **Divergence**: `:1677` emits `__VERIFIER_nondet_ulong()`, which appears exactly once in the repo
  outside its own test. `emit_c_preamble` (`:2950-2972`) declares the 64-bit form as
  `__VERIFIER_nondet_unsigned_long` (`:2966`), which is also what `c_nondet_suffix` (`:2094-2112`)
  says is canonical. Every other unsigned sibling (`u8` `:1649`, `u16` `:1670`) uses plain
  `__VERIFIER_nondet_long()`. Two tests pin the two spellings independently (`:3690`, `:4475`) and
  neither compiles the emitted C, so nothing catches it.
- **Solution**: `parse_opt_model(name) -> Option<OptionParseModel { payload_nondet, payload_range }>`;
  the two other sites ask `parse_opt_model(name).is_some()` instead of re-listing nine strings.
- **Benefits**: **locality** — the undeclared-intrinsic bug becomes a one-cell edit; **test surface** —
  the emitted-C contract is already well pinned (`:4437-4477`, `:4481-4519`, `:3687-3693`).

```mermaid
graph LR
  EI[emit_inst] --> A["7 arms, inline skeleton"]
  CO[collect_option_vars] --> N1["9 names"]
  KB[is_known_builtin] --> N2["9 names"]
```

```mermaid
graph LR
  EI[emit_inst] --> P["parse_opt_model(name)"]
  CO[collect_option_vars] --> P
  KB[is_known_builtin] --> P
  P -.-> ND[payload nondet]
  P -.-> RG[payload range]
```

### extern-heap-origin-kind — 36 symbol rows fanned into five predicates, with rows missing · Worth exploring · score 20/25

- **Files**: `vow-ir/src/region.rs:1917-1977` (five predicates plus the `||` chain
  `heap_producing_extern`); consumers `:1694`, `:2029-2048`, `:3049`, `:3195`.
  **File-count estimate: 1.**
- **Score**: 20/25 — leverage 4, locality 4, blast radius 1, heat 3 (`region.rs`, 11 commits/90d,
  last touched 2026-08-17 — five weeks cold, which is what holds it below the two 22s).
- **Problem**: four of the five predicates have no caller other than the `||` chain; the *kind* each
  one establishes is discarded at the join, which is precisely why gaps are invisible.
- **Deletion test**: **concentrates.**
- **Divergences**: `__vow_btreemap_new` is missing from `map_creation_extern` (`:1967-1969`) though
  lowering emits it at `vow-ir/src/lower/mod.rs:3100` exactly parallel to `__vow_map_new` at `:3088`
  — so every `BTreeMap::new()` is classified non-heap. The eight sibling
  `__vow_string_parse_{i8,i16,i32,u8,u16,u32,u64}_opt` symbols are absent from
  `option_creation_extern` (`:1960-1965`), which lists only the `i64` pair.
  `vec_creation_extern` (`:1925`) omits the `_in_arena` variants that `string_creation_extern`
  spells out for all 14 of its families.
- **Solution**: `extern_heap_origin(sym) -> Option<HeapOriginKind>`, one row per family with a
  single `_in_arena` suffix rule; `heap_producing_extern` collapses to `.is_some()`.
- **Benefits**: **locality** — one table to audit against the runtime; **test surface** — a table
  test over the full symbol→kind map, where today only graph-level behaviour is pinned.

```mermaid
graph LR
  H[heap_producing_extern] --> P1[extern_fresh_in_caller]
  H --> P2[vec_creation_extern]
  H --> P3[string_creation_extern]
  H --> P4[option_creation_extern]
  H --> P5[map_creation_extern]
  TO[trace_origin_inner] --> H
  OI[origin_to_internal_inner] --> H
```

```mermaid
graph LR
  TO[trace_origin_inner] --> E["extern_heap_origin(sym)"]
  OI[origin_to_internal_inner] --> E
  IH[is_heap_producing] --> E
  E -.-> R["one row per family"]
  E -.-> S["_in_arena suffix rule"]
```

### extern-container-op-spec — the same container convention, five more spellings · Speculative · score 20/25

- **Files**: `vow-ir/src/region.rs:1979-1994`, `:1996-2013`, `:2015-2027`, `:2406-2414`,
  `:2416-2435`. **File-count estimate: 1.**
- **Score**: 20/25 — leverage 4, locality 4, blast radius 1, heat 3.
- **Problem**: five functions each re-derive "which arg is the container, which is the stored value,
  what do we call this operation", under the same `_in_arena`-shifts-by-one convention.
- **Deletion test**: **concentrates.**
- **Divergences**: `__vow_vec_push_val` / `_in_arena` / `__vow_vec_set_val` (`:1981-1983`) are known
  store edges and element writes but appear in **neither** `extern_growth_target` (`:1996-2013`) nor
  `extern_mutation_operation` (`:2015-2027`), while their non-`_val` twins do (`:1998`, `:2017`) —
  so pushing into a rodata-backed `Vec` via `__vow_vec_push_val` escapes
  `check_literal_mutations_post_inference` (`:3358-3414`). Arity guards drift on the identical
  symbol pair: `args.len() >= 3` at `:1984` vs `!args.is_empty()` at `:2007`/`:2010`.
- **Solution**: `extern_container_op(sym) -> Option<ContainerOp { receiver_arg, stored_value_args, label, grows }>`
  with the five functions becoming thin appliers.
- **Benefits**: **locality** and a removable `unwrap_or("container mutation")` fallback (`:3379`)
  that today licenses silent drift between two sets that happen to be equal.
- Marked *Speculative* rather than *Worth exploring* only because it overlaps
  `extern-heap-origin-kind` in the same file: whichever lands first re-anchors the other's lines.

```mermaid
graph LR
  R[region inference] --> F1[for_each_extern_store_edge]
  R --> F2[extern_growth_target]
  R --> F3[extern_mutation_operation]
  R --> F4[vec_clear_targets]
  R --> F5[vec_element_write_source]
```

```mermaid
graph LR
  R[region inference] --> O["extern_container_op(sym)"]
  O -.-> RA[receiver_arg]
  O -.-> SV[stored_value_args]
  O -.-> LB[label]
  O -.-> GR[grows]
```

## Dropped

| Candidate | Dropped because |
|---|---|
| `extern-abi-spec-table` (`vow-codegen/src/cranelift_backend.rs:2451-3075`, 132 arms / 625 lines, scored 23/25 before the filter) | **Competes with a recorded architectural direction.** `CLAUDE.md` designates `docs/spec/operations.json` + `scripts/generate_operations.py` as the single checked source of runtime-symbol/ABI facts, splicing `catalogue_extern_sig` into *this very function* between `GENERATE:OPERATIONS` markers (`:2392-2410`), and states that follow-ups #1271–#1275 "should add entries and target files to the existing generator rather than inventing a new mechanism." A hand-rolled parallel Rust table is that invention. Recorded as a **judgement** drop citing that directive — **not** the ADR hard filter, which `docs/adr/0001-0003` do not trigger — so it is reversible once the catalogue absorbs this table. |
| `model-capacity-bound-spec` (`vow-verify/src/c_emitter.rs`, 12 sites) | Leverage 3, score 19/25. The `<`-at-create / `<=`-at-havoc split is coherent-but-undocumented policy, not a defect; the one self-inconsistency (`:1421` `<` vs `:1483` `<=`, both on strings) is a behaviour fork the autonomy contract reserves for a human. |
| `fs-path-arg-prologue` (`vow-runtime/src/lib.rs`, 9 `__vow_fs_*` functions) | Leverage 3, score 19/25. A real 8-line repeated prologue, but the seam is an argument-decoding helper rather than a policy table, and the error sentinel differs per function. |
| `main-help-prologue` (`vow/src/main.rs:826-1084`) | Leverage 2, score ~16/25. Eight byte-identical `--help` prologues, but the seam is a formatting/exit helper, not a policy. Overlaps the existing `driver-command-epilogue` entry. |
| `narrowing-conversion-matrix` (`vow-types/src/env.rs:352-461` vs `vow-ir/src/lower/mod.rs:161-184`) | Leverage 1 — **fails the deletion test.** Both sides were enumerated (27 source→target pairs × 3 modes = 81 names each) with **no live drift**; complexity would move, not concentrate. |
| `method-table-triple` (`vow-types/src/check.rs:337-367`, `:380-420`, `:427-455`) | Already in the backlog under `builtin-method-result-type-seam` (landed), `builtin-receiver-kind` and `builtin-method-arity-check`. |
| `builtin-method-spec` | Already in the backlog — reconciled to `landed` this firing (PR #1299 merged 2026-09-18). |

## Too large to automate

Nothing new this firing. `clif-shim-region-parity` remains the standing blast-radius-4 entry
(re-checked 2026-09-21: still ~20+ files crossing the `vow-codegen`/`vow-clif-shim` tier seam). Its
tractable slice is carded separately as `hidden-region-store-targets` (19/25).

`solver-classify-function` was also re-checked this firing (rubric reconciliation step 4): still a
pure, unit-tested seam with no shallowness to remove. Both `dropped` entries stay `dropped`.

## Pick

**`narrow-literal-context-admission`, 22/25.**

**The pick was close — a tie at 22 with `loop-scope-break-policy`**, broken deterministically by the
rubric: blast radius is 1 for both and heat is 5 for both, so the tie fell to *the candidate whose
files were touched most recently* — `vow-ir/src/lower/mod.rs` on 2026-09-18 against
`vow-types/src/check.rs` on 2026-09-14. `loop-scope-break-policy` is therefore the **runner-up
candidate** and the natural next firing; its `ForEach` anomaly is a live correctness gap and is
worth a bug issue independent of the deepening.

Two further reasons the tie-break lands somewhere defensible rather than arbitrary:

1. **It is purely behaviour-preserving.** The table reproduces today's per-context membership row for
   row, including the `U64` asymmetry. `loop-scope-break-policy` can be made behaviour-preserving too
   (via a `NoPush` row), but its value is mostly in the bug it exposes, which is a human's call to fix.
2. **It sits in `vow-ir`, where the change can be checked differentially.** IR output for the
   213-program `tests/run/` corpus can be diffed against the parent commit — the same verification the
   `builtin-method-spec` firing used. `vow-codegen` and `vow-verify` candidates have no equivalent
   IR-level diff to lean on.

Three carried-over candidates also sit at 21 (`vec-reserve-next-capacity-seam`, `arena-variant-rule`,
`esbmc-auto-timeout-policy`) and are unchanged from the 2026-09-18 firing; all remain `proposed`.

**Rust-only, as with every prior firing.** `compiler/lower.vow` mirrors the same `let`-annotation
chain at `:4867-4888` and calls its own `lower_narrow_literal` at `:2108`. Because this change is
behaviour-preserving, leaving the self-hosted compiler untouched introduces no new drift — the same
precedent `builtin-result-tag` and `builtin-method-spec` set.

## Design

Written at step 4, after this report was first committed. See below.
