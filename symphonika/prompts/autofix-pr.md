# Autofix pull request for issue #{{issue.number}}

You are running autonomously inside the existing issue workspace at
`{{workspace.path}}` on branch `{{branch.name}}`. A pull request was opened
from this branch earlier and is now failing CI or has unresolved reviewer
feedback. Your job is to drive that PR to a clean state, then exit.

## What to do

**Use the pm-autofix-pr skill to fix the failing checks and reviewer
feedback on the open pull request for branch `{{branch.name}}`.**

The skill iterates the standard local-CLI autofix loop: identifies the PR,
reads failing check logs and unresolved review threads, makes targeted
code edits, runs the local quality gate, commits, pushes, and repeats until
the PR is clean.

Discover the PR yourself with `gh pr list --head {{branch.name}} --state open`
if you need the PR number — do not assume one. Stay on branch
`{{branch.name}}`. Do not open a second PR.

## Constraints

- This run is unattended. No operator will respond to prompts. Behavior
  that depends on a human answering mid-run is a failure mode.
- Use the local `gh` CLI for every GitHub mutation. Do **not** call the
  GitHub MCP connector tools — they elicit operator approval and end the
  run with `terminal_reason="provider requested input"`.
- Do not modify operational labels in the `sym:*` namespace.
- If you genuinely cannot make progress (e.g. the failure requires a
  product decision), post a `gh pr comment` on the PR explaining what
  blocked you, then **write `BLOCKED.md` in the workspace root**
  (uncommitted) with the same explanation and exit 0. A Bash tool call's
  `exit 1` only ends that subshell, not the provider session, so it cannot
  make `provider_success` false — exiting non-zero here would silently
  return the FSM to `wait_for_pr`, which would observe the same failing PR
  signals and route straight back into this state, an infinite loop.
  Writing `BLOCKED.md` is what the FSM actually gates this state's advance
  on. Do not self-apply `sym:human-needed`.

## Exit

Exit 0 once the local quality gate passes and you have pushed the fix, and
no `BLOCKED.md` exists in the workspace. The orchestrator will re-enter the
wait state, re-check the PR, and either loop back here, advance to merge,
or terminate based on the next PR signal snapshot.
