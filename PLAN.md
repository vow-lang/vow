# Plan: pin self-hosted collect_assigned_in_expr operand-position parity with a regression test (#655)

## Goal
Issue #655's code defect is already fixed: `compiler/lower.vow:2114-2163` (`collect_assigned_in_expr`) now recurses into Assign RHS/non-ident LHS, BINOP, UNOP, CALL callee+args, METHOD receiver+args, INDEX, FIELD/QUESTION/CAST/RETURN/BREAK, IF cond, WHILE/FOR cond/iterable, MATCH scrutinee+arm bodies, TUPLE, SLIT, ECTOR. It mirrors `vow-ir/src/lower/mod.rs:1432-1512` (landed via #1496 and #1533). What is missing is a regression test for the issue's exact miscompile. Add that test, confirm both compilers pass it, and close the issue. No production change unless the new test fails.

## Assumptions
- Issue line numbers (`lower.vow:732-791`, Rust `492-569`) are stale; the current ones are above (best guess, verified by reading).
- The issue is not "already resolved, nothing to do": no `tests/run`, `tests/fixtures` or `compiler/tests` file exercises an assignment inside a call argument / binop / unop / return / if-cond under an outer branch (grep confirmed). Only `short_circuit_assign` (&&/|| RHS) and loop-carried tests exist. So plan the test-only slice (best guess).
- If the new fixture fails on either compiler, fix that compiler's collector or if-lowering in the same PR (dual-compiler rule). Do not land a one-sided fix.

## Key Files
| File | Role | Lines |
|------|------|-------|
| `compiler/lower.vow` | self-hosted collector (already correct); consumers `collect_if_mutations` (~2769) and `&&`/`||` (~2880) | 2114-2163, 2769-2775 |
| `vow-ir/src/lower/mod.rs` | Rust reference collector | 1432-1512 |
| `tests/run/short_circuit_assign.vow` | template for a run fixture with `// TEST: stdout` | all |
| `tests/run/assign_in_operand_position.vow` [new] | behavioral regression, runs under both compilers via `scripts/full_test.sh` Section 4 | - |
| `compiler/tests/test_lower_short_circuit_assign.vow` | template for an IR-level unit test (Phi counting + dominance validator) | all |
| `compiler/tests/test_lower_operand_assign_phi.vow` [new, optional slice 2] | asserts the outer `if` merges `count` through a Phi | - |

## Steps (TDD slices)

### 1. Red/green: behavioral fixture `tests/run/assign_in_operand_position.vow` [new]
- Header `// TEST: stdout "..."`; `module AssignInOperandPosition`; print results with `print_i64` / `print_str("\n")` like `short_circuit_assign.vow`.
- Each case is a function taking a bool `outer` (and `found`), mutating a `let mut count` inside an expression operand, then reading `count` AFTER the outer branch:
  1. Issue scenario: `if outer { total = compute(if found { count = count + 1; count } else { 0 }); }` -> return `count + total`. Check outer true/false, found true/false.
  2. Assign in binop operand: `if outer { total = 1 + { count = count + 1; count }; }`.
  3. Assign in unop operand: `if outer { total = -{ count = count + 1; count }; }`.
  4. Assign in if-condition: `if outer { if { count = count + 1; true } { total = 1; } }` (guards the If-condition recursion).
  5. Assign in while-condition under outer if (like `while_cond_assign` but nested in an `if outer`).
  6. Assign in method receiver / arg and in an early `return` value inside the outer branch.
- Expected stdout computed by hand (do not copy from compiler output). Include both outer=true and outer=false so a stale pre-if SSA value is distinguished from the post-if value.
- Run: `build/vowc build --no-verify tests/run/assign_in_operand_position.vow -o $TMPDIR/aiop && $TMPDIR/aiop`, and the same with the Rust `target/release/vow` (build via `scripts/bootstrap.sh`, cargo with `-j2`). Use `VOW_CACHE_DIR=$(mktemp -d)` (see memory: compile cache ignores compiler changes).
- Expected: green on both compilers, since the code is already fixed. A red result on either one is a real bug to fix in that compiler (see Risks).

### 2. (Optional, only if it stays small) IR-level pin `compiler/tests/test_lower_operand_assign_phi.vow` [new]
- Copy `lower_fixture`/`count_phis` helpers from `compiler/tests/test_lower_short_circuit_assign.vow`, lower the case-1 function from a `tests/fixtures/` file (or an inline string) and assert the outer `if` has a Phi for `count` and that the dominance validator passes.
- Run: `build/vowc test compiler/tests/test_lower_operand_assign_phi.vow`.
- Skip this slice if it needs more than ~60 lines or duplicates helpers; the run fixture already pins observable behavior. If kept, prefer reusing a shared fixture over a third copy of the helpers.

### 3. Fixture hygiene
- Check `tests/run/` fixtures are auto-discovered by `scripts/full_test.sh` Section 4 (no manifest to edit). Do not add `TEST: known-divergence`.
- Verifier: fixture lives in `tests/run/` and is not under `tests/verify*/`, so it is not in the Section 2c C-parity set. No contracts are added, so no ESBMC obligations. Run `build/vowc build tests/run/assign_in_operand_position.vow` (with verify) once to confirm it still verifies or reports nothing.

## Testing
- Both-compiler run of the new fixture (slice 1), plus `build/vowc test compiler/tests/test_lower_*assign*.vow` if slice 2 lands.
- Targeted regression: `build/vowc test --filter lower compiler/`.
- Full gate before PR: `scripts/bootstrap.sh --skip-cargo --no-cache` (record head SHA), then `VOW_FULL_TEST_SKIP_CARGO=1 scripts/full_test.sh` in the background, polled in short intervals (about 40 min wall clock). Known pre-existing failures (see memory) are not caused by this change.

## Verification surface
No contract, codegen or C-model change; no new ESBMC properties. If the fixture exposes a bug and a fix is needed, re-check byte-identical C parity (`scripts/full_test.sh` Section 2c) and the binary fixed point (`compiler_b == compiler_c`).

## Risks
- The test might fail on the Rust compiler or on the self-hosted one because nested if-as-expression inside a call argument hits a different bug (e.g. if-expression value Phi rather than the mutation set). Mitigation: diagnose with `--dump-ir`, fix in both compilers, file a separate issue if the cause is out of scope.
- Hand-computed stdout errors. Mitigation: trace each case by hand and cross-check against Rust semantics (inner `count` increments only when `found`).
- Fixed-point/`HashMap` ordering: no production change, so no impact unless a fix is needed. Collector uses `Vec` + `vec_contains_str`, which is deterministic.
- Commit/PR title must be lower-case Conventional Commits: `test(lower): pin assignment-in-operand mutation collection (#655)`; the PR body should say `Closes #655` and that the collector fix already landed via #1496/#1533. `git rm PLAN.md` before opening the PR.
- codecov/patch: test-only diff, no instrumented lines added.

## Out of scope
- Refactoring either collector (the Rust and Vow traversals could share a visitor, but the Vow language lacks the means). Reformatting. Changes to `collect_free_vars_in_expr`. Any spec change (`docs/spec/` unaffected: no semantics change). Other audit-20260610 findings.
