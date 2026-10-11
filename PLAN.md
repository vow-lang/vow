# Plan: native verifier — path-conditional aggregate field writes (#1593)

## Goal
Let `vowc verify --backend native` model a `FieldSet` that is not in the block that allocates its target
(struct literals whose field values contain `if`/`match`/`unwrap`, a branch writing a pre-branch value,
a write to a parameter). Chosen approach is the issue's **option 2** (verifier-only): a write at path
condition `pc` stores `ite(pc, new, old)`. No lowerer change, so no Rust twin and no C-parity run
(CLAUDE.md "Scoped exception: the native verifier"; `vc_agg*`, `vc_exec`, `vc_induct` are verifier-only).

## Assumptions
- Option 2 over option 1 (lowerer reorder): option 1 changes IR for both compilers and the ESBMC C
  emitter input; option 2 stays inside the exempt verifier. (advisor + issue text)
- Soundness argument relied on: the walk visits blocks once in RPO over an acyclic CFG and `block_pc` is
  the exact "execution reaches this block" condition. Two blocks neither of which reaches the other
  can never both run, so an `ite` chain in walk order is exact; a read walked before a write cannot
  execute after it.
- Struct/enum params may alias in Vow (the spec has no non-aliasing rule). A write to a parameter is
  therefore only modelled when the function has exactly one `ITY_PTR` parameter; otherwise it stays
  `Skipped` with a specific detail. Conservative; a type-aware refinement is a follow-up.
- Writes whose target is a pointer **Phi** (`let mut p = if c {a} else {b}; p.f = 1;`) stay `Skipped`
  (write-through to both children under the edge conditions is a follow-up). Implementation must dump IR
  to confirm no *trivial* Phi (all Upsilons carry one SSA value) is emitted for the common
  `let c = ..; c.f = ..` shape; if one is, look through it in the gate.
- The `with_unwrap` example in the issue unwraps an unconstrained `Option` parameter, so the unwrap-None
  claim legitimately fails: expected verdict is `VerifyFailed` (counterexample `o = None`). The `pass/`
  fixture builds the `Option` locally (`Option::Some(x)`) instead.
- Reads of other-sort views of a written slot are unreachable from well-typed source (a struct field has
  one sort; enum payload views are only read after the tag check), so over-approximating them with fresh
  junk is acceptable.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `compiler/vc_agg.vow` | object model: gate rule, `vc_agg_set/get/phi_read`, header invariant comment | 1-30 (header), 33-56 (struct/new), 130-165 (set/get), 190-215 (phi_read), 223-245 (`vc_agg_shape_detail`) |
| `compiler/vc_exec.vow` | `vc_walk_agg`: REGION_ALLOC / FIELD_SET / FIELD_GET / pointer Phi; `fl.block_pc[bi]` is the pc | 341-380 (`vc_walk_agg`), 440-470 (`vc_walk_inst`) |
| `compiler/vc_induct.vow` | invariant-based loop cut: guards `vc_induct_ptr_phi`, `vc_induct_vec_written_in_loop`; call site | 211-250, 522 |
| `compiler/vc_unroll.vow` | `vc_step_applicable` already rejects any `FieldSet` in a loop (k-induction step) | 287-312 |
| `compiler/vc_gate.vow` | calls `vc_agg_shape_detail(f, tys.len())`; `vc_inst_supported` FIELD_SET/FIELD_GET | 252-262, 393 |
| `compiler/vc_vec.vow` | `vc_vec_shape_detail` ("Vec used as a struct"); Vec objects are `vc_agg_alloc` objects | 264-343 |
| `compiler/tests/test_vc_agg.vow` | unit tests: gate (`check_field_set_needs_own_block_alloc` l.88), slots (l.323), phi merge (l.272-322) | 59-110, 272-345 |
| `compiler/tests/test_vc_induct.vow` | `vc_induct_cut(...).status == VC_INDUCT_NA()` pattern | 280-340 |
| `compiler/tests/builders.vow` | `mk_inst_alloc/field_set/field_get/phi/upsilon/branch/jump`, `mk_block`, `mk_function` | 75-141 |
| `tests/verify-native/{pass,fail,skip,unknown}/` | end-to-end fixtures; `skip/field_set_in_branch.vow`, `skip/field_set_param.vow` | — |
| `docs/spec/cli.md` | "Aggregates." bullet (line ~72) | 72 |
| `docs/spec/errors.md` | `unsupported-opcode` row (line ~1411) detail examples | 1411 |
| `compiler/main.vow`, `vow/src/skill.rs`, `skills/vow/reference/cli.md` | generated copies of the cli.md bullet (`scripts/generate_help.py`) | — |

## Steps

### 1. Guard the loop cut first (soundness hole that relaxing the gate would open)
- **File**: `compiler/vc_induct.vow` (new fn next to `vc_induct_vec_written_in_loop`, call site l.522)
- **Change**: add `vc_induct_agg_written_in_loop(f, li, rpo)`: true when a `FieldSet` sits in a loop block
  and its target is not a `RegionAlloc` defined in that same block. `vc_induct_cut` returns
  `VC_INDUCT_NA()` for it (same treatment as a written Vec: the havoc copy would keep the object's entry
  state, so an invariant proof would be about the wrong state). Bounded unrolling then decides alone.
  In-loop allocs written in their own block keep today's behaviour.
- **Reuses**: `vc_chain_len(li, b)`, `vc_induct_vec_written_in_loop` shape (vc_induct.vow:237).
- `vc_step_applicable` (vc_unroll.vow:302) already rejects every `FieldSet` in a loop; no change.
- Must land before Step 4 so no commit on the branch can verify a loop-written outer object unsoundly.

### 2. Object model: births, epoch, conditional write, view invalidation
- **File**: `compiler/vc_agg.vow`
- **Change**:
  - `VcAgg` gains `births: Vec<i64>` (pc term at creation; `-1` for params/phis, pushed in
    `vc_agg_push_obj`), `sl_epoch: Vec<i64>` (parallel to slots), `epoch: i64`, and a small dirty-range
    list `dirty_obj/dirty_idx/dirty_end` for parameter objects. Update `vc_agg_new` and `vc_agg_store`.
  - New `vc_agg_write(ag, ar, hdr, obj, idx, sort, term, pc)`:
    - strong update (existing `vc_agg_set`) when `vc_tm_is_true(ar, pc)` or `pc == ag.births[obj]`
      (the write runs exactly when the object exists, so the old value is unobservable). Keeps every
      existing fixture's query byte-identical.
    - otherwise `old = vc_agg_get(...)` (junk or parameter symbol), store
      `vc_tm_app3(ar, VC_OP_ITE(), pc, term, old)` (folds when `term == old`). Other-sort views the write
      overlaps become `ite(pc, fresh_junk, view)` (view read first; a slot already `-1` stays `-1`).
    - always `ag.epoch = ag.epoch + 1`; record a dirty range when `obj` is a parameter.
  - `vc_agg_get`: a cached **Phi** slot is valid only when `sl_epoch[s] == ag.epoch` (a write may have
    changed a child; merges of merges stay polynomial because terms are hash-consed and
    `vc_flow_name` reuses the bound name). A found slot with term `-1` yields fresh junk for every kind
    (today a parameter would re-declare `p{k}f{i}s{sort}`: duplicate `declare-const`, original value
    back). An unfound view of a parameter overlapping a dirty range yields fresh junk.
  - Rewrite the header comment (l.20-27) and the `vc_agg_get` comment: the "every write precedes every
    read" invariant is replaced by the RPO/path-condition argument in Assumptions.
- **Reuses**: `vc_agg_junk`, `vc_agg_get`, `vc_tm_app3` ITE folding (vc_term.vow:511), `vc_tm_is_true`.

### 3. Executor passes the path condition
- **File**: `compiler/vc_exec.vow` (`vc_walk_agg`, l.341-380)
- **Change**: `REGION_ALLOC` sets `ag.births[obj] = fl.block_pc[bi]`; `FIELD_SET` calls
  `vc_agg_write(..., fl.block_pc[bi])` instead of `vc_agg_set`. `vc_walk_inst` already has `pc`; thread it
  or read `w.fl.block_pc[bi]`. Vec objects (`vc_agg_alloc` from `VC_VEC_NEW`) are never written here.

### 4. Relax the gate
- **File**: `compiler/vc_agg.vow` (`vc_agg_shape_detail`, l.223-245)
- **Change**: replace the same-block rule with: target must be a `RegionAlloc` or a pointer `GetArg`.
  Pointer-Phi target → `FieldSet through a merged value`; parameter target while the function has another
  `ITY_PTR` parameter → `FieldSet on a parameter that may alias another parameter`; a Vec-kind call
  target is left to `vc_vec_shape_detail` so "Vec used as a struct" still wins. Both new details are
  `unsupported-opcode`. The old `FieldSet outside the block that allocates its target` string is deleted.
- **Reuses**: the `alloc_blk` table scan already in this function (becomes `def_op` per id).

### 5. Fixtures (promote, refute, new)
- `git mv tests/verify-native/skip/field_set_in_branch.vow tests/verify-native/pass/` and
  `skip/field_set_param.vow` → `pass/`; drop their `// TEST: category` / `// TEST: skip-reason` lines
  (`ensures result.value >= 1` / `result == 0` both hold).
- New `fail/` twins (each with `counterexample-fn/-vow-id/-blame`, and `-value` where a parameter is
  named): `field_set_in_branch_wrong.vow` (`ensures result.value == 1`, refuted at `x = 2`),
  `field_set_param_wrong.vow`.
- New `pass/`: `struct_literal_if_field.vow`, `struct_literal_match_field.vow`,
  `struct_literal_unwrap_field.vow` (local `Option::Some(x)`; native-only comment as in `unwrap_some.vow`),
  `field_set_both_arms.vow` (read before the branch, in the else arm and after the join),
  `field_set_alias_through_merge.vow` (`let p = if x {a} else {b}; a.v = 7; p.v` — exercises the Phi epoch).
  Matching `fail/` twins: `struct_literal_if_field_wrong.vow`, `struct_literal_unwrap_param_none.vow`
  (the issue's literal form, `VerifyFailed`), `field_set_alias_through_merge_wrong.vow`,
  `loop_outer_field_write_wrong_post.vow` (struct built before a `while` with an `invariant`, field bumped
  in the body, `ensures result.v == 0`: must be `VerifyFailed`, would be falsely proved without Step 1).
- New `skip/`: `field_set_through_merged.vow` (reason `unsupported-opcode`),
  `field_set_param_aliased.vow` (two struct params, one written).
- Every `pass/`/`fail/` file keeps a runnable `main` (runtime differential in
  `scripts/verifier_runtime.py` runs them; `fail/` also gets `--replay-cex` in `full_test.sh`).

### 6. Docs and generated copies
- **File**: `docs/spec/cli.md` "Aggregates." bullet: replace "a field write to anything but a value
  constructed earlier in the same block (a parameter, or a value built before a branch)" with the new
  rule (writes are path-conditional; skipped: write through a merged pointer, write to a parameter when
  another struct/enum parameter may alias it, plus the unchanged nested-aggregate / `linear` items).
  `docs/spec/errors.md` `unsupported-opcode` row: add the two detail strings.
- Then `uv run python scripts/generate_help.py` (regenerates `compiler/main.vow`, `vow/src/skill.rs`,
  `skills/vow/reference/cli.md`), `cargo build --release -p vow`, `scripts/bootstrap.sh --skip-cargo`.

## Testing (TDD slices, in order — each red→green, one commit)
1. **Loop guard.** `compiler/tests/test_vc_induct.vow`: hand-built single-loop IR whose body does
   `FieldSet` on an alloc from before the loop → `vc_induct_cut(f).status == VC_INDUCT_NA()` (red today:
   returns OK). Plus: in-loop alloc written in its own block stays OK. Impl: Step 1.
2. **Write API.** `compiler/tests/test_vc_agg.vow` on `vc_agg_*` directly: same-pc write is the stored
   term; different-pc write reads as `ite(pc,new,junk)`; true-pc write to a param is strong; conditional
   param write is `ite(pc,new,symbol)`; Phi object read, write to a child, read again changes (epoch);
   param symbol declared once after a 128-bit overwrite; other-sort view after write is not the old term.
   Impl: Step 2.
3. **Executor.** `test_vc_agg.vow` two-block IR (`alloc` in B0, conditional `FieldSet` in B1, read in the
   join): `query(cs, k)` contains the `ite`, the same-block `build_insts` query is unchanged. Impl: Step 3.
4. **Gate.** Rewrite `check_field_set_needs_own_block_alloc`: param write and cross-block write → `""`;
   Phi target and aliased-param write → new details. Impl: Step 4.
5. **End to end.** Fixtures of Step 5, written failing (still `Skipped`) then green after Step 4 via
   `VOWC_BIN=build/vowc bash tests/verify-native/tests.sh` and the Section 4g loop of
   `scripts/full_test.sh` (needs `bitwuzla`; `--no-cache`, `VOW_CACHE_DIR=$(mktemp -d)`).
6. **Docs + help regeneration**, `python3 scripts/check_help_coverage.py`.

## Verification surface
- No ESBMC/C change. Run the ESBMC backend (`build/vowc verify`) on `struct_literal_if_field`,
  `struct_literal_match_field` and both promoted `field_set_*` fixtures when `esbmc` is installed; record
  agreement in the PR body ("match ESBMC where ESBMC models them"). `[panic]` functions are
  non-modelable under ESBMC (native-only).
- Proof obligations are the fixtures' `ensures`; `scripts/verifier_runtime.py` (verified ⇒ no runtime
  `VowViolation`) and `scripts/z3_crosscheck.py` pick the new `pass/`/`fail/` files up automatically.
- Regression check: record the fake-bitwuzla queries of the existing `tests/verify-native/pass` corpus
  before and after; same-block-write functions must be byte-identical.
- Dump IR (`vowc build --no-verify --dump-ir`) of the three issue functions before finalising the gate;
  `build/vowc` is absent in this worktree — bootstrap first (`scripts/bootstrap.sh`, ~5 min).

## Risks
- **Loop soundness**: relaxing the gate without Step 1 lets the induction cut prove false posts over
  outer objects. Mitigation: Step 1 first plus the `loop_outer_field_write_wrong_post` fail fixture.
- **Stale Phi cache** (alias fixture) → epoch stamp in Step 2; verify merges of merges stay linear with
  a nested-diamond fixture or unit test (no exponential recompute).
- **Aliased parameters** → single-ptr-param rule; recorded under Assumptions, covered by a skip fixture.
- **Query growth**: conditional writes add `ite`s and unused junk constants; solver sees `pc ⇒ ...`
  closed by the claim's own pc. Watch `scripts/verify_perf.py` fixtures for regressions.
- **Binary fixed point / determinism**: vectors only (no `HashMap`); new loops iterate in index order, so
  the bootstrap fixed point is unaffected. Verifier-only code, so Rust↔Vow parity and
  `c_emitter` byte-identity (Section 2c) are untouched; `cargo clippy` gate unaffected.
- **"Green locally"**: re-run `scripts/bootstrap.sh --skip-cargo --no-cache` against the final head SHA
  and record it (and the seed pin when `scripts/seed.toml` exists; it does not yet).
- Commit/PR title lower-case Conventional Commits, e.g.
  `feat(verify): model path-conditional field writes in the native verifier` (≤ 92 chars).

## Out of scope
- Lowerer reorder (option 1) and anything in `vow-ir`, `vow-verify`, `c_emitter.{rs,vow}`.
- Write-through of a pointer Phi, nested aggregate fields, type-aware parameter alias analysis.
- Aggregates crossing calls (`vc_call_scalar` stays scalar-only), Vec/String/map modelling changes.
- Refactors, formatting, unrelated skip fixtures.
