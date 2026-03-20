---
name: investigate
description: >-
  This skill should be used when the user asks to "investigate issue",
  "investigate #<number>", "fix issue #<number>", "fix bug #<number>",
  "debug issue", "look into issue #<number>", "triage issue",
  "reproduce and fix GitHub issue", "close issue #<number>",
  or invokes /investigate <number>. Runs the full end-to-end bug
  investigation workflow: reproduce, fix both Rust and self-hosted
  compilers, test, commit, push, and comment on the GitHub issue.
args: issue_number
user_invocable: true
---

# Investigate GitHub Issue

End-to-end workflow for reproducing, fixing, testing, and closing a Vow compiler bug
reported as a GitHub issue. This workflow modifies **both** the Rust compiler and the
self-hosted compiler, as required by the dual-compiler rule in CLAUDE.md.

## Workflow

### Phase 1 — Understand the Issue

Fetch the issue with `gh issue view $ARGUMENTS --json title,body,labels,comments`.

Extract:
- Reported bug behavior and symptoms
- Reproduction steps or code snippets
- Expected vs actual behavior
- Which compiler stage is likely affected (syntax, types, IR, codegen, verify, runtime)

### Phase 2 — Reproduce the Bug

Write a minimal `.vow` file that triggers the reported bug. Compile and run with:

```bash
ulimit -v 2000000; ./vowc build <test-file>.vow
```

- For verification bugs, use `./vowc verify`.
- For runtime bugs, use `--mode debug`.
- Confirm the bug manifests before proceeding.

Keep the reproduction file for fix validation in Phase 5.

### Phase 3 — Create Task List

Convert the investigation into a task list using TaskCreate before executing any fix.
Typical tasks:

1. Reproduce the bug (Phase 2 output)
2. Identify root cause in Rust compiler
3. Implement fix in Rust compiler
4. Run `cargo test --all` until green
5. Identify corresponding code in self-hosted compiler
6. Implement equivalent fix in `compiler/*.vow`
7. Rebuild self-hosted compiler and run `scripts/full_test.sh`
8. Verify reproduction case is fixed
9. Commit, push, and comment on issue

Set up dependency relationships between tasks.

### Phase 4 — Fix Both Compilers

Spawn **two sub-agents in parallel** using the Agent tool. Each agent works on
independent files (Rust `.rs` files vs self-hosted `.vow` files), so there are no
conflicts. If sub-agents encounter issues (stale worktrees, integration mismatches),
fall back to sequential execution: fix Rust first, then self-hosted.

**Agent 1 — Rust Compiler Fix:**
- Identify the root cause in the relevant crate (vow-syntax, vow-types, vow-ir, vow-codegen, vow-verify, vow-clif-shim, vow-runtime, vow-diag, or vow)
- Implement the fix
- Iterate: run `cargo test --all` until all tests pass
- Run `cargo clippy --all -- -D warnings` to ensure no lint warnings
- Report back: files changed, root cause summary, test results

**Agent 2 — Self-Hosted Compiler Fix:**
- Identify the corresponding code in `compiler/*.vow` modules (see Crate-to-Module Mapping below)
- Implement the equivalent fix
- Rebuild: `scripts/bootstrap.sh --no-verify --skip-cargo`
- Iterate: run `ulimit -v 2000000; scripts/full_test.sh` until all tests pass
- Report back: files changed, root cause summary, test results

**Critical:** Both agents MUST use `ulimit -v 2000000` when running any self-compiled binary.

### Phase 5 — Verify the Fix

After both agents complete:

1. Re-run the Phase 2 reproduction `.vow` file to confirm the bug is fixed:
   ```bash
   ulimit -v 2000000; ./vowc build <test-file>.vow
   ```
2. Run the full Rust test suite:
   ```bash
   cargo test --all
   ```
3. Run the full self-hosted test suite:
   ```bash
   ulimit -v 2000000; scripts/full_test.sh
   ```
4. If either fails, iterate on the fix.

### Phase 6 — Commit and Push

1. Stage all changed files (both Rust and `compiler/*.vow` changes).
2. Create a single commit with message format: `fix: <concise description> (#$ARGUMENTS)`
   - Follow the existing commit style (see `git log --oneline`).
   - Never mention Claude or AI in the commit message.
3. Push to the current branch.

### Phase 7 — Comment on the Issue

Post a comment on the issue using:

```bash
gh issue comment $ARGUMENTS --body "<summary>"
```

The comment must include:
- **Root cause**: What caused the bug and where in the codebase.
- **Fix**: What was changed in both compilers and why.
- **Testing**: What tests were run to validate the fix.

### Phase 8 — Close the Issue

After confirming the fix is pushed and the comment is posted, close the issue:

```bash
gh issue close $ARGUMENTS
```

If the fix is partial or needs further review, skip this step and inform the user.

## Key Project Rules

- **Dual-compiler rule**: Every change to the Rust compiler must have a corresponding change in the self-hosted compiler (`compiler/*.vow`).
- **Memory safety**: Always use `ulimit -v 2000000` when running `./vowc` or any binary it produces.
- **Commit authorship**: Never mention Claude or AI in commits for `pmatos@igalia.com`.
- **Task tracking**: Convert plans to task lists (TaskCreate) before execution.
- **Bootstrap rebuild**: After changing `compiler/*.vow`, rebuild with `scripts/bootstrap.sh --no-verify --skip-cargo`.

## Crate-to-Module Mapping

Use this to find the self-hosted equivalent of a Rust crate:

| Rust Crate | Self-Hosted Module |
|---|---|
| vow-syntax (lexer) | compiler/lexer.vow |
| vow-syntax (AST) | compiler/ast.vow |
| vow-syntax (tokens) | compiler/token.vow |
| vow-types | compiler/types.vow, env.vow, checker.vow |
| vow-ir | compiler/ir.vow, ir_printer.vow |
| vow-ir (lowering) | compiler/lower.vow |
| vow-codegen | compiler/clif.vow |
| vow (CLI driver) | compiler/main.vow |
| vow-diag | (integrated in main.vow/checker.vow) |
