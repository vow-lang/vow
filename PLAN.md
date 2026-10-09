# Plan: index-once receiver routing in `collect_extern_syms` (issue #619)

## Goal
Make the self-hosted import pre-pass `collect_extern_syms` (`compiler/clif.vow:339`) O(instructions) per function by building an id->inst index and a phi->upsilon-source index once per function, instead of re-scanning `f.blocks` for every routed push/insert. Behaviour (returned symbol set and order) must stay byte-identical.

## Assumptions
- Issue line numbers/names are stale: `clif_receiver_region`/`clif_value_receiver_region` are now `clif_receiver_route` (`clif.vow:231`) / `clif_value_receiver_route` (`clif.vow:149`), and routes carry a candidate tag. Fix is the same; plan targets current code (best guess).
- Rust backend needs no change: `vow-codegen/src/cranelift_backend.rs` already builds `inst_index: HashMap<InstId,&Inst>` + `PhiUpsilonData` once per function (`:527`, `:48`, `source_value_route` `:654`, `routed_vec_extern` `:769`). This is self-hosted catch-up to existing Rust behaviour; dual-compiler rule satisfied; no spec/CLI/language change.
- Design uses only `HashMap<i64,i64>` plus plain `Vec`s. `HashMap<i64,i64>` is proven in `compiler/decl_text.vow:216`; no map-valued-`Vec` and no map-typed struct fields (none exist in `compiler/`, unverified corner). Builders RETURN their map (precedent: `cx_ir_fan_in_counts` returns a `BTreeMap`, `compiler/complexity_graph.vow:12`); callee mutation of a caller-owned map is not relied on.
- `HashMap` not `BTreeMap`: maps are only probed by key, never iterated, so no fixed-point determinism risk (best guess).

## Key Files
| File | Role | Lines |
|------|------|-------|
| `compiler/clif.vow` | the whole change | `clif_find_inst` 13-28; `clif_hidden_store_target_region` 110-126; `clif_value_receiver_route` 149-229; `clif_receiver_route` 231; `clif_routed_extern_symbol` 239-311; `collect_extern_syms` 339-372 |
| `compiler/tests/test_fresh_builtin_routing.vow` | existing caller of `clif_routed_extern_symbol(f, call)` (`routed_symbol` helper, line ~102); must follow the new signature | 80-103 |
| `compiler/tests/builders.vow` | `mk_inst_arg`, `mk_inst_phi`, `mk_inst_upsilon`, `mk_block`, `mk_function`, `mk_inst_call_extern` for new tests | 13-90 |
| `vow-codegen/src/cranelift_backend.rs` | reference design only, no edit | 48, 527, 654-735, 769 |
| `bench/memory/programs/*.vow` | not touched (RSS bounds, not time) | - |

Only `collect_extern_syms` calls the routing functions outside tests (`grep` verified: no other caller of `clif_find_inst`, `clif_receiver_route`, `clif_routed_extern_symbol`; `clif_emit_module` calls `collect_extern_syms` at `clif.vow:463`).

## Steps

### 1. Add index builders (`compiler/clif.vow`)
- `clif_inst_pos(f: IrFunction) -> HashMap<i64, i64>`: one pass over blocks/insts, ordinal = running count; insert only if absent (first occurrence wins, matching `clif_find_inst:19`).
- `clif_flat_insts(f: IrFunction) -> Vec<IrInst>`: same walk, pushes every inst; ordinal in `pos` indexes it.
- Phi sources as a linked chain, no `Vec`-valued map: `clif_phi_chain_head(f) -> HashMap<i64,i64>` (phi id -> first chain slot or absent) plus `clif_phi_chain_next(f) -> Vec<i64>` and `clif_phi_chain_src(f) -> Vec<i64>` (slot -> next slot / upsilon `args[0]`). Slots = ordinal of qualifying upsilon in the same flat order, `next` = -1 terminated. Build by walking insts in REVERSE and prepending (head[dv] = slot, next[slot] = old head), which yields forward block/inst order with no tail map. Qualifying upsilon: `op == IOP_UPSILON() && dk == IDATA_PHI_TARGET() && args.len() > 0`, key `dv` (exactly the filter at 198-201). (Implementer may fold these into fewer passes; builders must stay pure and return values.)
- `clif_find_inst_indexed(flat, pos, id) -> IrInst`: `pos.get(id)` -> `flat[ord]`, else the same `ir_inst_new(-1, IOP_CONST_UNIT(), ITY_UNIT(), Vec::new(), IDATA_NONE(), 0, 0, String::from(""))` sentinel as `clif_find_inst:26`.
- Delete scanning `clif_find_inst` once no caller remains.
- Every new helper name must be unique across `compiler/*.vow` (see Risks): grep each before adding.

### 2. Thread the indexes through routing (`compiler/clif.vow`)
- `clif_value_receiver_route(f, receiver, seen)` gains `flat`, `pos`, `phi_head`, `phi_next`, `phi_src` (five index params, passed through unchanged; `f` stays for `clif_hidden_store_target_region`). Replace the two `clif_find_inst` calls (165, 178) with `clif_find_inst_indexed`; replace the Phi block-walk (187-220) with a loop over the chain from `phi_head.get(receiver.id)`, same merge / `have` / `candidate_arm` / divergent-return logic verbatim, each arm resolving `clif_find_inst_indexed(.., phi_src[slot])`.
- `clif_receiver_route`, `clif_routed_extern_symbol` take and forward the same params.

### 3. Rewire `collect_extern_syms` (`compiler/clif.vow:339`)
- Per function, before the block loop: build `flat`, `pos`, `phi_head`, `phi_next`, `phi_src` once; pass into `clif_routed_extern_symbol`. `result` order stays first-seen, so import order is unchanged.

### 4. Test caller (`compiler/tests/test_fresh_builtin_routing.vow:102`)
- `routed_symbol` itself builds the five indexes for its one-function IR (calling the Step 1 builders) and passes them to `clif_routed_extern_symbol`. No test-only wrapper is added to `clif.vow`.

## TDD slices (red -> green -> refactor)
1. **Characterization (green on current code)**: new `compiler/tests/test_clif_receiver_route.vow` [new; name unique, `ls compiler/tests` first] pins current routing via `clif_routed_extern_symbol` (through a local helper like `routed_symbol`): (a) Root arg receiver -> plain `__vow_vec_push`; (b) receiver `rgn = Block(3)` -> `__vow_vec_push_in_arena`; (c) FieldGet from Block container + string push_str -> `__vow_string_push_str_in_candidate_arena`; (d) `__vow_vec_get_val` projection receiver; (e) Phi with two Upsilon arms of the same Block region -> `_in_arena`; (f) Phi with divergent arm regions -> `receiver.rgn` fallback; (g) self-referential loop Phi terminates via `seen`; (h) missing receiver id -> Root; (i) duplicate inst ids -> first wins. Helpers from `compiler/tests/builders.vow` (`mk_inst_arg`, `mk_inst_phi`, `mk_inst_upsilon`, `mk_inst_call_extern`, `mk_block`, `mk_function`). The helper to build the function must be written against the current 2-arg signature first, then flipped in slice 3.
2. **Red**: add `check_index_builders` calling `clif_inst_pos`, `clif_flat_insts`, `clif_phi_chain_*` directly (fails to compile). Assert ordinal map, first-duplicate-wins, sentinel for missing id, upsilon source order for one phi with 3 arms across 2 blocks, and no entry for a phi with no upsilons.
3. **Green**: implement Steps 1-3, flip the test helpers (Step 4); slice-1 assertions must pass unchanged.
4. **Functional size check**: add one test with a modest generated function (N ~ 2000 pushes on a loop-header Phi) running `collect_extern_syms` on a one-function `IrModule`; assert the returned list is exactly `["__vow_vec_push"]`. No timing assertion and no large N (it cannot go red without a runner timeout, and a big function slows every `build/vowc test compiler/`). Performance evidence is the before/after wall-clock below, not a test.
5. **Refactor**: remove dead `clif_find_inst`.

## Testing / verification commands (run as separate commands, background + poll per memory notes on gate wall-clock)
- `build/vowc test compiler/tests/test_clif_receiver_route.vow`, `build/vowc test compiler/tests/test_fresh_builtin_routing.vow`, then `build/vowc test compiler/` (self-hosted unit tests).
- Performance evidence: record `time build/vowc build --no-verify compiler/main.vow -o $TMPDIR/x` with the pre-change `build/vowc` vs the freshly bootstrapped one (same fresh `VOW_CACHE_DIR`), in the PR body. Then `scripts/bootstrap.sh --skip-cargo --no-cache` confirms the fixed point.
- Bootstrap triple test (`scripts/concat_vow.sh clif` ... `sha256sum compiler_b compiler_c`) must give identical hashes. `build/vowc` itself will differ (clif.vow changed), so the invariant is narrower: old and new compiler emit the same object/binary for the same input programs (a few `tests/run` fixtures and `compiler/main.vow`), compared with a fresh `VOW_CACHE_DIR`.
- `scripts/full_test.sh` (Sections 2c C-parity untouched; Section 4 run tests) with `VOW_CACHE_DIR=$(mktemp -d)` to dodge the stale compile cache (memory note).
- No `cargo` gates required (no Rust edits); still run `cargo build --release -p vow` only if bootstrap needs it.

## Verification surface (ESBMC / C model)
- `compiler/clif.vow` carries no `vow` contracts on these functions; no new contracts are added and none weakened. The module is verified via `build/vowc build` (bootstrap verifies the compiler); new helpers should be contract-free like their neighbours, or, if the verifier flags HashMap use, mark unverifiable rather than distorting a contract.
- Not touching `c_emitter.{rs,vow}` or lowering -> verifier C parity unaffected (Section 2c runs as a regression check only).
- New fixtures: only the `compiler/tests/` unit test; no `tests/run/` or `examples/` growth needed (no language-visible behaviour change). Optionally add `bench/memory`-style program? No: out of scope (bounds are RSS).

## Risks
- **Binary fixed point**: output must be unchanged. Risk: changing iteration order of Upsilon sources or symbol insertion order changes merge results/early-return paths. Mitigation: preserve block/inst order in `srcs`, keep first-seen `result` order, never iterate a HashMap (probe only), triple-test hashes.
- **Divergent-phi early return**: result when arms diverge is `receiver.rgn` regardless of which arm triggered it, but keep verbatim loop to be safe.
- **First-match semantics** for duplicate ids (`clif_find_inst:19`) must carry over (step 1) -- tested in slice 2.
- **HashMap in `clif.vow`**: `HashMap<i64,i64>` is used elsewhere in `compiler/` (`decl_text.vow:216`) and compiled by both compilers; a failure would show immediately at bootstrap stage 0.
- **concat_vow.sh namespace**: `scripts/concat_vow.sh` merges all modules into one namespace; grep every new `clif_*` helper name (and the new test file name under `compiler/tests/`) for collisions across `compiler/*.vow` before adding.
- **Stale compile cache** may hide codegen regressions (use fresh `VOW_CACHE_DIR`).
- **Clippy gate**: no Rust change; `cargo clippy --all --all-targets -- -D warnings` unaffected.
- **Test runner time**: scale test must not slow `build/vowc test compiler/` appreciably after the fix.
- **codecov/patch**: no Rust lines changed, so not applicable.
- **Parameter growth** in routing signatures: five index params threaded unchanged; acceptable, noted (a map-holding struct is deliberately avoided).

## Out of scope
- Any change to Rust `cranelift_backend.rs` (already indexed) or `vow-clif-shim` (`inst_region_for_value` already uses maps).
- Making self-hosted lowering emit non-Root regions, or short-circuiting the scan on the Root check (alternative cheap guard; not chosen since correctness must not depend on it).
- `clif_hidden_store_target_region` recomputing `clif_hidden_store_targets` per GET_ARG receiver: cost scales with `rsum_store_effects` (params/store edges), not instruction count, and hoisting it changes another function's signature. Follow-up only.
- Optimising `seen` vector cloning, `vec_contains_str` on `result`, other `clif_compile_function` passes, or refactoring `clif_routed_extern_symbol`'s branches.
- Formatting/unrelated cleanups; docs/spec, `--help`, skill regen (no language/CLI change).
- Fixing the stale audit doc (`docs/audit-20260610`).

## Commit / PR
- Conventional commit, lower-case subject, e.g. `perf(clif): index insts and phi sources once per function in collect_extern_syms` (<=92 chars for PR title). Split into two commits if useful: tests (characterization) then perf change. Squash merge. `PLAN.md` is `git rm`'d before the PR. Record the checked head SHA when ticking the bootstrap checklist (`bootstrap.yml` doesn't run on PRs).
