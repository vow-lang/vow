---
name: investigate
description: >-
  This skill should be used when the user asks to "investigate issue",
  "investigate #<number>", "fix issue #<number>", "fix bug #<number>",
  "debug issue", "look into issue #<number>", "triage issue",
  "reproduce and fix GitHub issue", "close issue #<number>",
  "investigate PR #<number>", "review PR #<number>", "check PR #<number>",
  or invokes /investigate <number>. For issues: runs the full end-to-end bug
  investigation workflow (reproduce, fix, test, commit, close). For PRs:
  checks CI, triages review comments, runs code reviews, and posts an
  Investigation Report.
args: number
user_invocable: true
---

# Investigate GitHub Issue or Pull Request

This skill handles both GitHub issues and pull requests. The argument is a number —
the skill detects whether it refers to an issue or a PR and follows the appropriate workflow.

## Detection

Determine whether the argument refers to a PR or an issue:

```bash
NUM="$ARGUMENTS"
if gh pr view "$NUM" --json number >/dev/null 2>&1; then
  # → follow the PR Workflow below
elif gh issue view "$NUM" --json number >/dev/null 2>&1; then
  # → follow the Issue Workflow below
else
  echo "Error: #$NUM is neither a PR nor an issue in this repo" >&2
  exit 1
fi
```

- If `gh pr view` succeeds → follow the **PR Workflow** below.
- If `gh pr view` fails but `gh issue view` succeeds → follow the **Issue Workflow** below.
- If both fail → stop and report the error (do not silently fall through).

---

# PR Workflow

Set the PR number from the argument at the start:
```bash
PR="$ARGUMENTS"
```

End-to-end review workflow for an open pull request. Checks CI, triages review comments,
runs independent code reviews, and posts a consolidated Investigation Report.

## PR Phase 1 — Fetch the PR

Fetch PR details with:
```bash
gh pr view "$PR" --json title,body,headRefName,baseRefName,state,labels,reviewRequests,reviews,statusCheckRollup,files
```

Extract:
- PR title and description
- Source and target branches
- Changed files list
- Current CI status
- Existing reviews and their verdicts

Check out the PR branch locally:
```bash
gh pr checkout "$PR"
```

## PR Phase 2 — Check CI

Examine the CI status from the PR data fetched in Phase 1.

If checks are still running, poll with:
```bash
gh pr checks "$PR" --watch
```

- If CI **passes**: note this and proceed.
- If CI **fails**: fetch the failing check logs with `gh run view <run-id> --log-failed`,
  identify the failure, and include it prominently in the Investigation Report.
  Still proceed with the remaining phases — CI failure does not short-circuit the review.

  **Finding the run ID:** In the `statusCheckRollup` array from Phase 1, find entries with
  `conclusion: "FAILURE"`. The `detailsUrl` contains the run ID — e.g.,
  `https://github.com/.../actions/runs/12345/job/...` → run ID is `12345`.
  Alternatively: `gh run list --branch <headRefName> --status failure --limit 5 --json databaseId,name,conclusion`.

## PR Phase 3 — Triage Review Comments

Fetch all review comments (repo-relative paths — `gh api` infers the current repo):
```bash
gh api "pulls/$PR/comments" --paginate
gh api "pulls/$PR/reviews" --paginate
```

For each review comment:
1. **Clearly actionable** (bug report, correctness issue, missing test, style violation
   with project rule citation) → mark as **actionable**.
2. **Clearly non-actionable** (praise, acknowledgement, "nit" with no substance,
   already-addressed feedback) → mark as **skip**.
3. **Ambiguous** (subjective suggestion, debatable design choice, unclear scope) →
   run `/codex-2nd-opinion` with the comment text and surrounding code context to get
   an independent judgment. Use the Codex verdict to classify as actionable or skip.

**Getting code context for comments:** Each review comment from the API includes `path`
(file), `line`/`original_line` (position), `start_line` (for multi-line comments), and
`diff_hunk` (the surrounding diff context). Use `path` and line numbers to read the
relevant source with the Read tool, and include `diff_hunk` as additional context.

Produce a summary table of all comments with their classification and rationale.

## PR Phase 4 — Code Review

Run the code-review skill on the PR's changes:

```
/code-review:code-review
```

This reviews the diff against project guidelines (CLAUDE.md) and coding standards.
Capture the full output for inclusion in the Investigation Report.

## PR Phase 5 — Codex Review

Run the Codex review skill scoped to the PR branch:

```
/codex:review --background --scope branch
```

This provides an independent review from Codex. Capture the full output for inclusion
in the Investigation Report.

## PR Phase 6 — Investigation Report

Merge all findings from Phases 2–5 into a single consolidated comment on the PR.

Post the comment with:
```bash
gh pr comment "$PR" --body "<report>"
```

The comment must follow this structure:

```markdown
## Investigation Report

### CI Status
<pass/fail summary; if failed, include the failure reason and affected check>

### Review Comment Triage
<table of existing review comments: comment excerpt | classification | rationale>
<count of actionable vs skipped comments>

### Code Review Findings
<summary of findings from /code-review:code-review>

### Codex Review Findings
<summary of findings from /codex:review>

### Consolidated Action Items
<numbered list of all actionable items from all sources, deduplicated>
```

## PR Phase 7 — Plan Mode Prompt

After posting the Investigation Report, present the consolidated action items to the user
and ask:

> "Investigation Report posted. There are N action items. Would you like to enter plan mode
> to address them?"

If the user agrees, enter plan mode with the action items as the starting context.
Do **not** start fixing anything automatically — wait for the user's decision.

---

# Issue Workflow

Set the issue number from the argument at the start:
```bash
ISSUE="$ARGUMENTS"
```

End-to-end workflow for reproducing, fixing, testing, and closing a Vow compiler bug
reported as a GitHub issue. This workflow modifies **both** the Rust compiler and the
self-hosted compiler, as required by the dual-compiler rule in CLAUDE.md.

## Workflow

### Phase 1 — Understand the Issue

Fetch the issue with `gh issue view "$ISSUE" --json title,body,labels,comments`.

Extract:
- Reported bug behavior and symptoms
- Reproduction steps or code snippets
- Expected vs actual behavior
- Which compiler stage is likely affected (syntax, types, IR, codegen, verify, runtime)

### Phase 2 — Reproduce the Bug

Write a minimal `.vow` file that triggers the reported bug. Compile and run with:

```bash
ulimit -v 2000000; build/vowc build <test-file>.vow
```

- For verification bugs, use `ulimit -v 2000000; build/vowc verify <test-file>.vow`.
- For runtime bugs, use `ulimit -v 2000000; build/vowc build --mode debug <test-file>.vow`.
- Confirm the bug manifests before proceeding.

**Critical:** Always use `ulimit -v 2000000` when running `build/vowc` or any binary it produces.

Keep the reproduction file for fix validation and regression testing in later phases.

### Phase 3 — Create Task List

Convert the investigation into a task list using TaskCreate before executing any fix.
Typical tasks:

1. Reproduce the bug (Phase 2 output)
2. Identify root cause in Rust compiler
3. Implement fix in Rust compiler
4. Run `cargo test --all` and `cargo clippy --all -- -D warnings` until green
5. Identify corresponding code in self-hosted compiler
6. Implement equivalent fix in `compiler/*.vow`
7. Rebuild self-hosted compiler and run `ulimit -v 2000000; scripts/full_test.sh`
8. Verify reproduction case is fixed
9. Run `/codex:review` and address feedback
10. Add regression test to appropriate `tests/` category
11. Commit, push, and comment on issue

Set up dependency relationships between tasks.

### Phase 4 — Fix Both Compilers

Fix the compilers **sequentially** — Rust first, then self-hosted. Do not use parallel
sub-agents; work incrementally and directly.

**Step 1 — Rust Compiler Fix:**
- Identify the root cause in the relevant crate (vow-syntax, vow-types, vow-ir, vow-codegen, vow-verify, vow-clif-shim, vow-runtime, vow-diag, or vow)
- Implement the fix
- Iterate: run `cargo test --all` until all tests pass
- Run `cargo clippy --all -- -D warnings` to ensure no lint warnings
- Run `cargo fmt --all` to ensure formatting is clean

**Step 2 — Self-Hosted Compiler Fix (if applicable):**
- Identify the corresponding code in `compiler/*.vow` modules (see Crate-to-Module Mapping below)
- Implement the equivalent fix
- Rebuild: `scripts/bootstrap.sh --skip-cargo`
- Iterate: run `ulimit -v 2000000; scripts/full_test.sh` until all tests pass

**Critical:** Always use `ulimit -v 2000000` when running any self-compiled binary.

### Phase 5 — Verify the Fix

After both compilers are fixed:

1. Re-run the Phase 2 reproduction `.vow` file to confirm the bug is fixed:
   ```bash
   ulimit -v 2000000; build/vowc build <test-file>.vow
   ```
2. Run the full Rust test suite:
   ```bash
   cargo test --all
   ```
3. Run clippy and check formatting:
   ```bash
   cargo clippy --all -- -D warnings
   cargo fmt --all --check
   ```
4. Run the full self-hosted test suite:
   ```bash
   ulimit -v 2000000; scripts/full_test.sh
   ```
5. If any step fails, iterate on the fix.

### Phase 6 — Codex Review

After verifying the fix, request a Codex review of the changes by running `/codex:review`.

Address all issues raised by the review before proceeding. Iterate until the review passes
cleanly — re-run verification (Phase 5) after any changes made in response to review feedback.

### Phase 7 — Add Regression Test

Every bug fix **must** include a regression test. Move the reproduction `.vow` file from
Phase 2 into the appropriate test category:

| Test Category | Directory | When to Use |
|---|---|---|
| Runtime tests | `tests/run/` | Bug produced wrong runtime output or crash |
| Error tests | `tests/error/` | Bug was a missing or wrong compile-time error |
| Verify tests | `tests/verify/` | Bug caused verification to incorrectly fail |
| Verify-fail tests | `tests/verify-fail/` | Bug caused verification to incorrectly pass |
| Debug tests | `tests/debug/` | Bug related to `--mode debug` behavior |
| Multi-module tests | `tests/multi/` | Bug related to multi-module compilation |

Name the test file descriptively (e.g., `tests/run/vec_index_oob.vow`).

After adding the test, re-run the full test suite to confirm it's picked up:
```bash
ulimit -v 2000000; scripts/full_test.sh
```

### Phase 8 — Commit and Push

1. Stage all changed files (Rust sources, `compiler/*.vow`, and the new test file).
2. Create a single commit with message format: `fix: <concise description> (#$ISSUE)`
   - Follow the existing commit style (see `git log --oneline`).
   - Never mention Claude or AI in the commit message.
3. Push to the current branch.

### Phase 9 — Comment on the Issue

Post a comment on the issue using:

```bash
gh issue comment "$ISSUE" --body "<summary>"
```

The comment must include:
- **Root cause**: What caused the bug and where in the codebase.
- **Fix**: What was changed in both compilers and why.
- **Testing**: What regression test was added and what test suites passed.

### Phase 10 — Close the Issue

After confirming the fix is pushed and the comment is posted, close the issue:

```bash
gh issue close "$ISSUE"
```

If the fix is partial or needs further review, skip this step and inform the user.

## Key Project Rules

- **Dual-compiler rule**: Every change to the Rust compiler must have a corresponding change in the self-hosted compiler (`compiler/*.vow`), except for `vow-runtime` which has no self-hosted equivalent.
- **Memory safety**: Always use `ulimit -v 2000000` when running `build/vowc` or any binary it produces.
- **Task tracking**: Convert plans to task lists (TaskCreate) before execution.
- **Bootstrap rebuild**: After changing `compiler/*.vow`, rebuild with `scripts/bootstrap.sh --skip-cargo`.
- **Sequential workflow**: Fix Rust compiler first, then self-hosted. Do not use parallel sub-agents.
- **Codex review**: Run `/codex:review` after implementation, address all feedback before committing.
- **Regression tests**: Every bug fix must add a test to the appropriate `tests/` category.

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
| vow-clif-shim | (FFI shims consumed by compiler/clif.vow) |
| vow-runtime | (linked runtime; no self-hosted equivalent) |
| vow-diag | (integrated in main.vow/checker.vow) |
| vow (CLI driver) | compiler/main.vow |
