# Plan: pin Rust/self-hosted block-dominator-tree parity with differential tests (#658)

## Goal
Mechanically prove that `compiler/region.vow:build_block_parent` (incremental-LCA fixpoint) and
`vow-ir/src/region.rs:BlockTree::from_function` (dominator-set intersection) produce identical
`parent[]` arrays, so a latent divergence can no longer mis-place a region open marker unnoticed.
Test-only unless the tests find a real divergence (then reconcile in BOTH compilers).

## Findings (recon, so the implementer does not re-derive them)
- Issue line numbers are stale. Current locations: Vow `build_block_parent` `compiler/region.vow:3923`,
  `block_tree_visit_root` :3976, `block_tree_dominance_parent` :4045, `block_tree_parent_lca` :4094,
  `build_block_depth` :4166. Rust `BlockTree::from_function` `vow-ir/src/region.rs:703`, `visit_root`
  :~740, `dominance_parent` :808, `compute_depth` :898.
- Forward-graph construction is the same in both: DFS from `blocks[0]`, then every unvisited block in
  `blocks` order as a new root/component; successors sorted ascending, visited smallest-first;
  edges to on-stack nodes (back edges) and cross-component edges are dropped. Terminator scan is the
  same (`last terminal`, `Branch`/`Jump` only; `Return`/`Unreachable` -> no successors).
  `iop_is_terminal` == `Opcode::is_terminal`. The forward graph is a DAG (every kept edge goes from a
  later to an earlier DFS post-order number), so its dominator tree is unique.
- Only the dominator step differs. Argument that the Vow fixpoint still yields the unique idom tree
  (to be confirmed by the tests, and worth a 3-4 line comment in `block_tree_dominance_parent`):
  initial parents are DFS-discovery predecessors, so every true dominator of v is a tree ancestor of v
  (a root->v CFG path passes through all of them); each update `parent[v] = LCA(parent[v], pred)`
  moves v to an ancestor of its old parent, and any dominator d of v dominates/equals `pred`, so d
  stays an ancestor -> "all dominators are ancestors" is invariant. At the fixpoint `parent[v]` is an
  ancestor of every pred, so by induction in topological order every ancestor dominates v. Ancestors
  == dominators, hence `parent[v] == idom(v)`. Stale-tree "overshoot" is impossible for this reason.
  Iteration bound `n*n+1` is tight but sufficient (<= n moves per node, each sweep moves >= 1).
- Rust uses O(n^2) BTreeSet memory per component; the Vow fixpoint is O(n) memory. Do NOT port the
  dom-set algorithm to Vow "to match"; if parity holds, keep both algorithms.
- Existing coverage does not pin this: `vow-ir/src/region.rs` tests (5827-5960) only check LUBs on
  4-block diamonds/disconnected roots; `compiler/tests/test_region.vow` never calls
  `build_block_parent`; `vow/tests/region_summary_equivalence.rs` defers byte-identical comparison and
  is about summaries, not placement. `scripts/full_test.sh` Section 0b already diffs `RegionAlloc
  <region=block_N>` placements on `compiler/main.vow` but not Open/Close markers and not synthetic CFGs.

## Assumptions
- Full end-to-end diff of RegionOpen/RegionClose markers over a fixture corpus is deferred (follow-up):
  Section 0b of `full_test.sh` is the natural home, but it has a known pre-existing offset failure on
  clean main (`concrete-block-region-parity`, see memory), so extending it here would conflate a
  pre-existing red with this change. The issue explicitly accepts the focused `build_block_parent` vs
  `BlockTree` test as the minimum. (best guess)
- A shared literal corpus duplicated in a Rust test and a Vow test, tied together by a pinned digest,
  is preferred over a cross-process harness: deterministic, no `build/vowc` dependency inside
  `cargo test`, runs in both `vowc test` flavours. (best guess)
- No language/CLI/spec change, so no `docs/spec/*` or ADR update. Verifier-exempt: no contract changes.

## CI wiring (verified)
`scripts/full_test.sh:2259-2262` runs `$RUST test compiler/` and `run_self test compiler/`, and
`.github/workflows/bootstrap.yml:91-93` runs both on push/nightly, so a new `compiler/tests/test_*.vow`
is auto-discovered by both compilers; no gate edit needed. No `compiler/**/*.vow` canonical-form gate
was found in `full_test.sh`; still write the file in the same style as `test_region.vow`.

## Key Files
| File | Role | Lines |
|------|------|-------|
| `vow-ir/src/region.rs` | add `#[cfg(test)] mod dominance_tests;` after `mod tests` (ends the file); production untouched | 4244 (`mod tests`), 703-960 (`BlockTree`) |
| `vow-ir/src/region/dominance_tests.rs` [new] | Rust differential tests; uses `super::BlockTree` and its own tiny CFG builder | - |
| `compiler/tests/test_region_dominance.vow` [new] | Vow twin: same corpus through `build_block_parent`/`build_block_depth` | - |
| `compiler/tests/builders.vow` | reuse `mk_inst_jump`, `mk_inst_branch`, `mk_block`, `mk_function`; define a local value-less Return via `ir_inst_new(.., IOP_RETURN(), ITY_UNIT(), Vec::new(), IDATA_NONE(), ..)` (as `mk_return_unit`, `test_region.vow:366`, which is file-local) | 29-85 |
| `compiler/region.vow` | UNTOUCHED unless step 5 finds a divergence (equivalence argument lives in the new test's header, keeping the PR test-only and bootstrap-neutral) | 4045-4092 |

## Steps (TDD slices; each is its own commit, conventional-commit subjects lower-case)

### 1. test(region): golden dominator parents for named CFG shapes in Rust
- **File**: `vow-ir/src/region/dominance_tests.rs` [new]; add `#[cfg(test)] mod dominance_tests;` to `region.rs`.
- **Builder**: `cfg(&[(id, &[succs])]) -> Function`: block order = slice order (first = entry); 0 succs ->
  `Return`, 1 -> `Jump`, 2 -> `Branch` (copy the shapes of `jump_inst`/`branch_inst`/`return_unit_inst`,
  `region.rs:4280-4310`; `function()` at :4311 — redefine locally, they are private to `mod tests`).
  `parents(&f) -> Vec<(u32, i64)>` from `BlockTree::from_function(&f).parent` (None -> -1).
- **Cases** (expected arrays hand-derived and written literally, `id -> idom`):
  straight line; diamond; if-without-else (triangle); nested diamond; **shared join with unequal
  predecessor depths** (join reached from depth-1 and depth-3 branches: idom = common branch point,
  not either pred); while loop (header + body back edge + exit); nested loops; loop with `break`
  (exit join from header and from body); loop with `continue`; self-loop; multi-return (two sinks);
  **join numbered lower than its preds and branch point higher than the join** (stresses the Vow
  per-id sweep order); non-contiguous / unsorted ids; unreachable block as second root; unreachable
  subgraph with an edge INTO the reachable component (cross-component edge must be dropped);
  **irreducible** two-entry cycle (r->a, r->b, a->b, b->a: result depends on DFS order — pin the
  expected value from the Rust output after confirming by hand on the forward DAG).
- **Production code**: none; these must pass on current code (characterization). If any fails, STOP and
  go to step 5.

### 2. test(region): the same golden table through the self-hosted pass
- **File**: `compiler/tests/test_region_dominance.vow` [new]; `module TestRegionDominance`, `use ir`,
  `use region`, `use tests.builders` (mirror the header of `compiler/tests/test_region.vow:1-6`).
- Build each CFG with `mk_block`/`mk_inst_jump`/`mk_inst_branch`/local `mk_return_unit`/`mk_block`/`mk_function`
  (`builders.vow:29-85`); assert `build_block_parent(f)[id]` equals the same literal table
  (`-1` root, `-2` absent) and that `build_block_depth` has no `-1` for existing blocks.
- Follow the file's conventions: check helpers return `i64`, `main` runs checks in order and returns the
  first non-zero (`test_region.vow:567-597`); annotate chained field reads with `let`.
- Run: `build/vowc test compiler/tests/test_region_dominance.vow` and the Rust stage-0 equivalent
  (`full_test.sh` runs `compiler/tests/*` under both compilers; the file must compile in both).

### 3. test(region): exhaustive 4-block corpus pinned by a shared digest
- **Files**: both new test files.
- Enumerate all `11^4 = 14641` graphs over 4 blocks, entry = logical block 0: per block, digit
  `k` in base 11: `0` Return; `1..=4` Jump(k-1); `5..=10` Branch over the 6 unordered pairs
  `(0,1),(0,2),(0,3),(1,2),(1,3),(2,3)` in that order (self-loops covered by Jump/…; same-target
  branch excluded — dedup'd by both compilers). Digit order: block 0 least significant. Run twice:
  identity ids and reversed ids (`id = 3 - logical`, block list order unchanged so entry stays first).
- Digest over `parent[id]` for ids `0..3` in graph order then pass order:
  `h = h * 1000003 + (p + 3)`, i64/u64 wrapping, seed `1469598103934665603` (avoid `^`/u64 literals
  > i64::MAX). Rust uses `wrapping_mul/wrapping_add`.
- Assert `digest == <PINNED_CONSTANT>` in BOTH files. Derive the constant once from the Rust side, then
  require the Vow side to match it (that is the differential). Also assert in each file that every
  non-entry reachable block has a parent chain ending at a root (no cycles) — localises tree bugs
  without needing the other side.
- Second, standard (not fallback) corpus: 5 and 6 blocks, 1000 graphs each from a fixed-seed LCG
  with the same per-block decode (base `1+n+C(n,2)`); lets nested-loop + unequal-depth joins appear.
  LCG arithmetic: `x = (x * 1103515245 + 12345) mod 2^31` computed so no negative intermediate ever
  reaches `%` (keep all values non-negative i64 below 2^62, reduce with `& 0x7fffffff` or `% 2147483648`
  on a non-negative value) — Rust and Vow differ on `%` of negatives. Separate pinned digest.
- Budget: Rust ms; Vow ~1-3s for ~16k graphs.

### 4. make the tests red once (mutation sanity, not committed)
- Vow: temporarily change `block_tree_dominance_parent` so `next_parent = pred` (drop the LCA), and
  separately cap the sweep at one iteration; both the golden table and the digest must fail.
- Rust: temporarily switch `max_by_key` to `min_by_key` in `dominance_parent` (:808+); goldens and
  digest must fail. Revert all three; `git diff` must show only the new test files + the `mod` line.
- Guards against a shared decode bug making both sides trivially agree (e.g. every graph = Return).
- Header comment of `test_region_dominance.vow` (and a short one in `dominance_tests.rs`) carries the
  equivalence argument: "dominators are always tree ancestors; at the fixpoint ancestors are
  dominators; forward graph is a DAG", and points at the other file.

### 5. (conditional) reconcile a divergence
- Only if step 1/2/3 expose a mismatch. Decide which side is wrong by hand-deriving the forward-DAG
  idom for the minimal failing graph. Fix BOTH compilers in the same PR (CLAUDE.md dual-compiler rule),
  keep the fix surgical, add the minimal failing graph as a named golden case, then re-pin the digest.
  If the Vow side is wrong prefer repairing its fixpoint/iteration order over porting the O(n^2) set
  algorithm. File a follow-up issue for anything larger.

## Testing
- Red phase: step 4 mutations must fail the new tests before the final green run.
- `cargo test -p vow-ir region::dominance_tests` then `cargo test -p vow-ir` (cap jobs: `-j2`).
- `cargo clippy --all --all-targets -- -D warnings` and `cargo fmt --all -- --check`.
- `build/vowc test compiler/tests/test_region_dominance.vow` and `build/vowc test compiler/tests/test_region.vow`
  (regression), plus the Rust stage-0 `target/release/vow test` of the same file.
- If (and only if) step 5 edits `compiler/region.vow`: `scripts/bootstrap.sh --skip-cargo --no-cache`
  on the final head SHA, record the SHA in the PR checklist line.
- Final gate: `scripts/full_test.sh` (40 min — run backgrounded with polling, per the memory note) at
  least Section 2 (`test/` suites); ignore known pre-existing reds (`concrete-block-region-parity`,
  `u64_marker_propagation`, `contracts_tmp_cleanup`) after confirming on clean main.

## Verification surface
- No contracts, C model or ESBMC property changes; `build_block_parent` has no `vow` block and none is
  added. No `tests/run/` or `examples/` fixture growth required.
- If step 5 modifies `region.vow` code, the bootstrap verification pass must stay green and the
  stage-1/stage-2 binaries must stay byte-identical.

## Risks
- Digest bridge hides *which* graph diverges: mitigated by per-file tree-validity assertions and the
  named golden cases; on digest mismatch temporarily dump per-graph parents from both sides.
- Vow arithmetic/`u64` literal semantics in the digest: keep to wrapping `*`/`+` on i64, seed < 2^63.
- Test file must compile under Rust stage-0 AND self-hosted compilers (Vow `if`-returning-i32 CLIF bug
  noted in `test_region.vow:9-14`; use `i64` check helpers).
- Irreducible case pins DFS-order-dependent output: if either compiler ever changes successor order
  both goldens move together — intended.
- Fixpoint cost is not tested for scale (the `block_parent_depth` seen-scan is O(depth^2), `block_id_exists`
  O(blocks) per visit); noted, not addressed here.
- Name collisions: `use region` imports every `region.vow` name; grep new helper names (`cfg`, `parents`,
  `digest`, `mk_return_unit`, ...) against `compiler/*.vow` before use — prefix them `dom_`.
- Binary fixed point / `BTreeMap` order / clif-shim stack slots: N/A while only test files change; if
  step 5 edits `region.vow`, bootstrap triple-check (stage1 == stage2 hash) is required.
- `parse -> print -> parse` idempotency: N/A (no syntax or printer change).
- Byte-identical verifier C (`c_emitter.{rs,vow}`): N/A (region pass is not in the verifier C path,
  no contract/IR-lowering change).
- `cargo clippy --all --all-targets -- -D warnings`: new test code is linted; avoid `needless_range_loop`
  in the enumeration loops (iterate with `enumerate`/iterators) and keep `cast_*` explicit.
- Chasing the stale `mod tests` helper visibility: `function()`/`branch_inst()` are private to
  `mod tests`; the new module redefines a minimal builder instead of widening visibility.

## Out of scope
- Extending `full_test.sh` Section 0b / `region_summary_equivalence.rs` to diff RegionOpen/RegionClose
  markers or `.vmod` summaries (follow-up; blocked by the pre-existing Section 0b offset failure).
- Replacing either dominator algorithm, scaling fixes to `block_id_exists`/`block_successors_by_id`,
  refactoring `region.rs` (10k lines) or `region.vow`, formatting or unrelated cleanups.
- Any change to `docs/spec/*`, `docs/adr/*`, `--help`/skill text, or verifier C emitters.
