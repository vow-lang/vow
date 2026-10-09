# Plan: function-scope operand check in the Rust IR validator (#631)

## Goal
`vow_ir::validate_function` must resolve instruction operands against every instruction defined in the function, not only the user's own block, so valid cross-block SSA references (loop-header Phi read by an Upsilon in another block, branch conditions defined in a dominating block) are not reported as `UndefinedInstRef`.

## Assumptions
- Scope is the issue's stated minimum: function-wide id set. Dominance checking is not added: the self-hosted `compiler/ir_dominance.vow` (`ir_non_dominating_read`) and the Rust test helper `non_dominating_reads` (`vow-ir/src/lower/mod.rs:7618`) already cover it, and it would be a new feature rather than a fix (best guess).
- Phi operand exemption: lowered Phis carry no `args` (values flow via Upsilon, which has `args` plus `InstData::PhiTarget`), so nothing to exempt; an Upsilon's `args` stay checked like any other operand (best guess; confirm in slice 2 by lowering a while loop).
- Dual-compiler rule: no self-hosted twin of `vow-ir/src/validator.rs` exists (`grep` of `compiler/` finds only `ir_dominance.vow`, a different pass). The change creates no Rust/Vow drift, so there is nothing to mirror. Record this in the PR body.
- `validate`/`validate_function` have no non-test callers (audit L3.49, a separate issue). Wiring them into the driver is out of scope.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `vow-ir/src/validator.rs` | Fix + unit tests | `validate_function` 35-89, `validate_block` 123-161 (defect at 124/136-143; Branch cond lookup at 146-155 has the same per-block flaw), tests 163-456 (`make_inst`, `make_func` helpers) |
| `vow-ir/tests/contract_text_forms.rs` | Pattern to copy for lowering source in an integration test (`parse_module` + `lower_module_with_pattern_aggregates`) | 17-35 |
| `vow-ir/src/lower/mod.rs` | Reference only: while-loop exit Upsilon refs header Phi | ~1254-1262 |
| `vow-ir/tests/validator_cross_block.rs` [new] | Integration test: lowered multi-block fn validates | - |
| `tests/fixtures/` | Existing fixtures to reuse, e.g. `short_circuit_assign.vow` | - |

## Steps

### 1. Build a function-wide definition table in `validate_function`
- **File**: `vow-ir/src/validator.rs` (lines 35-50)
- **Change**: before the block loop, build `defs: HashMap<InstId, Ty>` from `func.blocks.iter().flat_map(|b| b.insts.iter()).map(|i| (i.id, i.ty))`. Pass `&defs` to `validate_block`. `HashMap` is fine: it is lookup-only and the error order follows the deterministic block/inst iteration. Do not iterate the map.
- **Reuses**: the existing flat_map idiom at lines 48-53 and 92-97.

### 2. Make `validate_block` check operands against `defs`
- **File**: `vow-ir/src/validator.rs` (lines 123-161)
- **Change**: change the signature to `validate_block(block, defs, errors)`. Delete the per-block `inst_ids` set. Test `defs.contains_key(&arg)` for `UndefinedInstRef`. In the Branch check, look up the condition type with `defs.get(&cond_id)` instead of `block.insts.iter().find(...)`. A cond defined in another block is now type-checked (it was silently skipped); an unknown id is already reported as `UndefinedInstRef`, so do not double-report.
- Keep terminator and `BlockNotTerminated` logic untouched.

### 3. Doc comment
- **File**: `vow-ir/src/validator.rs`
- **Change**: one short comment on `validate_block` saying references are function-scoped (SSA values cross blocks via dominance and Upsilon); dominance is checked elsewhere. Comment sparingly per CLAUDE.md.

## TDD slices (write the test first, watch it fail, then implement)

1. **Red/green: cross-block ref accepted.** `vow-ir/src/validator.rs` tests: `cross_block_reference_is_valid`. Block0: `GetArg`(0), `Jump`; Block1: `Return` with args `[InstId(0)]`. Currently yields `UndefinedInstRef`; passes after steps 1-2. Jump uses `InstData::JumpTarget(BlockId)`; Branch uses `InstData::BranchTargets { then_block, else_block }` (`vow-ir/src/types.rs:222-228`). An existing lowering test to model slice 5 on: `lower_while_loop_emits_phi_upsilon_and_backedge` (`vow-ir/src/lower/mod.rs:10642`).
2. **Upsilon/Phi across blocks.** `cross_block_upsilon_to_header_phi_is_valid`: header block has `Phi`, body block has `Upsilon(args=[phi or value], PhiTarget(phi))`, mirroring the while-loop shape. Asserts `is_ok()`. This pins the exact scenario named in the issue.
3. **Still rejected: truly undefined id.** `undefined_reference_still_reported`: operand `InstId(99)` that exists in no block yields `UndefinedInstRef { user, referenced: 99 }`, including when the user is in a multi-block function. Guards against over-loosening.
4. **Branch cond type across blocks.** `branch_condition_type_checked_across_blocks`: cond defined in block0 with `Ty::I64`, `Branch` in block1 yields `TypeMismatch`; with `Ty::Bool` yields ok. Passes after step 2's `defs.get`.
5. **End-to-end on real lowered IR.** `vow-ir/tests/validator_cross_block.rs` [new]: parse and lower a source with a `while` loop (inline string with `break`/accumulator, or an existing `tests/fixtures/` loop fixture), then `assert!(!errors.iter().any(|e| matches!(e, UndefinedInstRef{..})))` for every function. Assert only on `UndefinedInstRef`, not `is_ok()`, so unrelated pre-existing validator findings (e.g. `LinearNotConsumed` quirks) cannot make the test brittle. First run it against the unfixed validator to confirm it is red for the stated reason.

## Testing
- `cargo test -p vow-ir validator` (slices 1-4) and `cargo test -p vow-ir --test validator_cross_block` (slice 5).
- `cargo test -p vow-ir`, then `cargo clippy --all --all-targets -- -D warnings` and `cargo fmt --all --check`.
- The full `cargo test --all` is not needed beyond `vow-ir` since the validator has no callers, but run it once before the PR if time allows (note: ~8 vow-crate run tests fail in the sandbox without a linked runtime; compare to clean `origin/main`).
- `scripts/bootstrap.sh` and `full_test.sh` are not required: no lowering, codegen, `compiler/*.vow` or verifier C changes, so the binary fixed point and `parity.py c` are untouched.

## Verification surface
None. No contracts, codegen, C model, `tests/run/` or `examples/` change; ESBMC is not involved.

## Risks
- Dead-code validator: nothing exercises it in production, so regressions would be invisible; slice 5 is the guard that real lowered IR keeps passing the operand check.
- Lowered IR may legitimately contain other findings (`PhiWithoutUpsilon`, linear checks) unrelated to this issue; slice 5 deliberately filters to `UndefinedInstRef`. If lowered loops reveal genuinely undefined ids, file a follow-up issue rather than widening this PR.
- Looser check: function-wide scope accepts use-before-def and sibling-branch reads (#1407-style). Accepted, since dominance is covered by `ir_non_dominating_read` / `non_dominating_reads`; state this explicitly in the PR body and in the doc comment.
- No binary fixed point, `parse -> print -> parse`, or verifier C parity impact (Rust `vow-ir` validator only). Clippy: avoid new `needless_*` lints on the `defs` lookup; collapsible `if let` chains are already used in this file.
- Dual-compiler rule: no Vow counterpart exists to update (see Assumptions). Say so in the PR body to pre-empt drift questions.

## Out of scope
- Wiring `validate` into the CLI driver (audit L3.49) and any new diagnostics surface.
- A dominance check (def dominates use) or Phi-operand exemptions beyond what lowering produces.
- Other validator defects (`check_linear_types` at 91-121, audit line ~2076), refactors, or formatting.
- `docs/spec/*` and `docs/adr/`: no language, CLI, or architecture change.
- PR title suggestion: `fix(vow-ir): resolve validator operand refs across blocks` (subject lower-case, under 92 chars). The PR closes #631.
