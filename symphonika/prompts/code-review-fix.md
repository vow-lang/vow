# Code review pass for issue #{{issue.number}}

You are running autonomously inside the existing issue workspace at
`{{workspace.path}}` on branch `{{branch.name}}`. A pull request was just
opened from this branch. Your job is to run an automated code review pass
against it, apply any fixes it finds, and exit.

## What to do

**Run `/code-review --fix` against the diff for the currently open pull
request on branch `{{branch.name}}`.** Let it apply its own findings to the
working tree.

If it made changes, commit and push them to `{{branch.name}}`.

Discover the PR yourself with `gh pr list --head {{branch.name}} --state open`
if you need the PR number — do not assume one. Stay on branch
`{{branch.name}}`. Do not open a second PR.

## Constraints

- This run is unattended. No operator will respond to prompts. Behavior
  that depends on a human answering mid-run is a failure mode.
- Use the local `gh` CLI for every GitHub mutation. Do **not** call the
  GitHub MCP connector tools — they elicit operator approval and end the
  run with `terminal_reason="provider requested input"`.
- Do not modify operational labels in the `sym:*` namespace. Do not
  self-apply `sym:human-needed` — the orchestrator applies that automatically
  when a run ends up blocked.
- If `/code-review --fix` genuinely cannot proceed (e.g. no open PR found
  for this branch), post a `gh pr comment` explaining what blocked you, then
  **write `BLOCKED.md` in the workspace root** (uncommitted) with the same
  explanation and exit 0. A Bash tool call's `exit 1` only ends that
  subshell, not the provider session, so it cannot make `provider_success`
  false. The FSM instead gates this state's advance on `BLOCKED.md` not
  existing; writing that file is what routes the run to its blocked exit.

## Exit

Exit 0 once `/code-review --fix` has run and any fixes it made are pushed
(or it found nothing to fix), and no `BLOCKED.md` exists in the workspace.
The orchestrator will advance to the next state on success.
