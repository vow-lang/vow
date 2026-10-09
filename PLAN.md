# Plan: fix sibling-branch value reads in both lowerers (#1407)

## Goal
Make lowering never emit a value read whose definition does not dominate it. The one confirmed
source is `&&` / `||`: a variable assigned in the RHS is read after the merge, but only the RHS
block defines it. Fix it in the self-hosted lowerer and the Rust lowerer, pin it with twin tests
over a shared fixture, and add a durable dominance sweep over `compiler/` and the test corpus.

## Assumptions
- Root cause = `&&`/`||` lowering ignores RHS mutations (best guess, from reading both lowerers; unconfirmed by running the validator, which needs a built `build/vowc` that this planning workspace lacks). Step 1 re-confirms it empirically and enumerates any other offenders before code is written.
- The mutation collectors (`collect_assigned_in_expr`/`_block`) already visit the same expression forms in both lowerers (compared line by line: Assign, BinOp, UnOp, Call, Method, Field, Index, If, While/For, Loop, Match arm bodies, Tuple, struct literal, enum ctor, Break/Return, Question, Cast, Block; Lit/Ident/Continue/Result are leaves). So the "collector covers the same forms" criterion is satisfied by a parity *test*, not a collector change.
- If the sweep finds a second offender class, fix it in this PR only when it is the same class (a branch-local value escaping through scope); otherwise file a follow-up issue and mention it in the PR.
- No spec change: this is a lowering bug fix with no change to syntax, semantics or CLI. The defensive `ir-non-dominating-read` skip stays (ADR 1430 table, row "Kept as a defensive gate after the lowerer fix").

## Key Files
| File | Role | Lines |
|------|------|-------|
| `compiler/lower.vow` | `&&`/`||` lowering (the bug); `EXPR_IF` lowering is the reference pattern; `collect_if_mutations`; `lctx_in_scope_vars`/`lctx_assign`/`lctx_lookup`/`lctx_snapshot`; `backpatch_upsilon`; `merge_phi_ty` | 2855-2905, 3178-3300, 2757, 373-424, 2486 |
| `vow-ir/src/lower/mod.rs` | Rust twin: `&&`/`||` arm; `collect_if_mutations`, `collect_assigned_in_expr`; `in_scope_vars`, `assign`, `lookup`; `merge_phi_ty`, `backpatch_upsilon`; tests module (`lower_source_to_module`, twin-fixture tests ~7440) | 1704-1782, 1606, 1391, 1224/1164/1212, 1580, 4995, 6728, 7448 |
| `compiler/ir_dominance.vow` | validator `ir_non_dominating_read(f) -> String`; used by `is_modelable` / `non_modelable_reason` in `compiler/c_emitter.vow:492,609` | 239 |
| `compiler/tests/test_lower_loop_carried_scope.vow` | model for a source-lowering test (`lower_fixture` via `parse_module_into` + `lower_module_vow`) | 1-30 |
| `tests/fixtures/loop_carried_scope.vow` | model for a fixture shared by both compilers' tests | all |
| `tests/run/match_merge_with_mutation.vow` | model for a `// TEST: stdout` run fixture | all |
| `compiler/frontend.vow` | `frontend_lower_path_with_root` for the corpus sweep test | 341 |
| `scripts/full_test.sh` | Sections 2c (verifier C parity), 4 (run), 4d (verify-skip) | 873, 1274 |

## Steps (ordered; each is one small commit-sized unit)

### 1. Reproduce and enumerate (no commits)
- `scripts/bootstrap.sh` (background, ~5 min, cap `cargo build -j2`) to get `build/vowc`; use `VOW_CACHE_DIR=$(mktemp -d)` for every codegen validation (stale compile cache).
- Sweep: `build/vowc verify` (fake `esbmc` first on `PATH`, as `scripts/parity_c.py` does) over `compiler/main.vow` and every `tests/verify*/` and `tests/run/` fixture; collect every function whose Skipped reason contains `ir-non-dominating-read`. Record the list in the PR description. Minimise the first offender to a repro; expect the `&&`/`||`-RHS-assignment shape.
- Runtime confirmation: `let mut x = 5; if false && { x = 9; true } { }` then `print x` must print 5 (the clif shim reads x's value slot, which the RHS block never wrote on the short-circuit path).

### 2. RED: shared fixture + twin lowering tests
- `tests/fixtures/short_circuit_assign.vow` [new]: functions with RHS-assigned variables read after the merge (`&&`, `||`, nested `a && (b || {x = ..; c})`, assignment inside a `while` condition `&&`, RHS that `return`s), plus one function per hidden-assignment expression form (cast, index, tuple, struct literal, enum ctor, method arg, match arm) inside an `if` branch whose variable is read after the `if`. Header comment follows `loop_carried_scope.vow`.
- `compiler/tests/test_lower_short_circuit_assign.vow` [new]: lowers the fixture (copy `lower_fixture` from `test_lower_loop_carried_scope.vow`) and asserts `ir_non_dominating_read(f) == ""` for every function (use `use ir_dominance`), plus for the `&&`/`||` functions that the operand of the post-merge read is an `IOP_PHI`. Red now.
- `vow-ir/src/lower/mod.rs` tests module: `short_circuit_rhs_assignment_reaches_merge_through_phi` (twin; reads the same fixture via `include_str!`). Rust has no validator (scoped exception does not apply, validator is verifier-only), so assert structurally: the instruction feeding the post-merge `Return`/use is `Opcode::Phi`, and the fixture's `if`-form functions likewise. Red now.
- `tests/run/short_circuit_assign.vow` [new]: `// TEST: stdout "..."` prints the post-merge value for both short-circuited and evaluated paths (runtime wrongness, exercised by both compilers via `full_test.sh` Section 4).

### 3. GREEN (self-hosted): carry RHS mutations through the `&&`/`||` merge
- `compiler/lower.vow` 2862-2905. Before emitting the Branch: `mutations = kept names of collect_assigned_in_expr(rhs_eid)` filtered by `lctx_in_scope_vars`; save `lctx_lookup` values. In `rhs_block` after `lower_expr(rhs)`: read current mutation values, emit one Upsilon per name (before the existing result Upsilon, same order as the `EXPR_IF` code), then `lctx_restore`/`lctx_assign` back to the saved values. In `short_block`: Upsilon the saved values. In `merge_block`: one `IOP_PHI` per name (`merge_phi_ty` of the two sides), `backpatch_upsilon` both sides, `lctx_assign(name, phi)`. Reuse `EXPR_IF`'s structure (3183-3300); if the duplication exceeds ~30 lines extract a small helper used only by `&&`/`||` (do not refactor the `EXPR_IF` path in this PR).
- Empty mutation list must emit byte-identical IR to today (fixed point + IR-text parity).

### 4. GREEN (Rust): same change
- `vow-ir/src/lower/mod.rs` 1704-1782, mirroring step 3 with `collect_assigned_in_expr` + `ctx.in_scope_vars`, `ctx.snapshot_scope`/`restore_scope`/`assign`, `merge_phi_ty`, `backpatch_upsilon`. Same instruction emission order as the self-hosted lowerer (IR placement parity between compilers is tested).

### 5. Durable gate: dominance sweep over the compiler and corpus
- `compiler/tests/test_ir_dominance_corpus.vow` [new, `[io]`]: `frontend_lower_path_with_root` on `compiler/main.vow` (module root `compiler/`) and on `tests/fixtures/*.vow` + representative `tests/run/*.vow` (explicit list; no directory-listing builtin assumed), assert `ir_non_dominating_read` is empty for every function; on failure return a code and print the function name. Runs under `build/vowc test compiler/`.
- Re-run the step 1 sweep over all `tests/verify*/` fixtures with the fixed `build/vowc`; expect zero `ir-non-dominating-read` skips. Record in the PR.

### 6. Verifier parity fixture
- `tests/verify/short_circuit_assign.vow` [new] with a contract over a function that assigns in an `&&` RHS and reads after the merge (true contract, no ESBMC bounds). Section 2c (`parity.py c`) then proves both emitters hand ESBMC byte-identical C for the new IR shape (Upsilon/Phi already modelled for `if`).

## Testing / gates (separate commands, backgrounded and polled; budgets in memory notes)
- `cargo test -p vow-ir`; `cargo clippy --all --all-targets -- -D warnings`; `cargo fmt --all --check`.
- `build/vowc test compiler/tests/test_lower_short_circuit_assign.vow`, `test_ir_dominance_corpus.vow`, `test_lower_loop_carried_scope.vow`, `test_ir_dominance.vow`.
- `scripts/bootstrap.sh --skip-cargo --no-cache` on the final head SHA (record SHA and `scripts/seed.toml` pin in the PR checklist); binary fixed point (stage b == stage c sha256).
- `scripts/full_test.sh` (~40 min) incl. Sections 2c, 4. Known pre-existing failures to verify against clean origin/main before blaming this change: `u64_marker_propagation`, `contracts_tmp_cleanup`, `concrete-block-region-parity`, ~8 sandbox SKIP-panic run tests.
- Commit as `fix(lower): carry && / || rhs assignments through the merge phi` (lower-case subject; PR title <= ~92 chars). `git rm PLAN.md` before opening the PR.

## Verification surface
No contract or C-model change. New IR shape = Upsilon/Phi pairs already modelled for `if`; Section 2c parity + the `tests/verify` fixture cover it. Previously Skipped functions (dominance gate) may become Verified: expect no `tests/verify-skip` fixture to depend on this reason (grep confirmed none reference it); check ESBMC proves any newly-modelable compiler functions or they stay Skipped via the existing timeout path (do not change contracts).

## Risks
- Binary fixed point / compile cache: the lowerer change alters IR only for `&&`/`||` with RHS assignments; if the compiler source itself has such code, stage b != stage a is expected and b == c must still hold. Validate with a fresh `VOW_CACHE_DIR`.
- Instruction-id shifts: extra Phis/Upsilons shift later ids; IR-text parity tests and `concrete-block-region-parity` are id-sensitive. Emission order must match between lowerers.
- RHS that diverges (`a && { return 1; }`): Upsilons land after the terminator (existing convention, `linear_successors` scans backward); keep the same convention and cover it in the fixture.
- `while` conditions containing `&&` with assignments: loop-carried Phis (`collect_loop_assigned_vars`) interact with the new merge Phis; fixture covers it.
- Stack-slot clif shim: Phi shadow slots give parallel-copy semantics; no shim change expected. `BTreeMap` determinism unaffected (no new maps).
- Verifier C parity: any IR change reaches both emitters; Section 2c must stay green.
- Rust `cargo clippy -D warnings` on new test code (`--all-targets`).

## Out of scope
- Refactoring `EXPR_IF`/`match` mutation merging into a shared helper; reworking collectors (already equivalent).
- Removing the defensive `ir-non-dominating-read` gate or porting the validator to Rust.
- Mutation-testing runs, spec/docs/ADR edits (no language or CLI change), unrelated formatting.
