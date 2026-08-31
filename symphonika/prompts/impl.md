# Vow implementation stage: issue #{{issue.number}} {{issue.title}}

You are the **implementation** agent. A planning pass has written and committed `{{workspace.path}}/PLAN.md`. Read it first. If it is missing or stale, re-derive the slices from the issue body before writing code.

`PLAN.md` is a stage-handoff artefact, not a deliverable: it is committed only so the planning stage can hand it to you, and **it must not appear in the pull request.** Delete it in your final commit — see "Drop the plan before opening the PR" below.

## Issue under work

- Number: #{{issue.number}}
- Title: {{issue.title}}
- URL: {{issue.url}}
- Labels: {{issue.labels}}

### Issue body

{{issue.body}}

## Run context

- Project: {{project.name}}
- Run id: {{run.id}}
- Attempt: {{run.attempt}}
- Continuation: {{run.continuation}}
- Workspace: {{workspace.path}}
- Branch: {{branch.name}} ({{branch.ref}}) — stay on this branch; do not switch or create others
- Previous attempt detected: {{workspace.previous_attempt}}

## Source of truth

- `CLAUDE.md` — language-design rules, production-quality bar, contract-authoring discipline, PR policy.
- `docs/spec/*.md` — authoritative spec. **Any change to syntax, semantics, builtins, operators, effects, or CLI flags MUST update the corresponding `docs/spec/*.md` file in the same PR.**
- `docs/adr/` (if present) — accepted architecture decisions.
- The current working directory is `{{workspace.path}}`.

## How to implement

1. Read `PLAN.md`. Execute it slice by slice with TDD: write one behavior-focused test through the public interface, watch it fail, implement only enough code to make it pass, then repeat. Do not silently relax existing tests.
2. **Touch both compilers when language semantics change.** The Rust stage 0 (`crates/`) and the self-hosted compiler (`compiler/`) must move together. CI and local tests run both. Skipping one is not acceptable.
3. **Contracts are semantic specs, not ESBMC appeasement.** Never add bounds like `n <= 10` or `x <= 100` to `requires`/`ensures`/`invariant` to make the verifier happy. Overflow guards that reflect real UB (e.g. `x > -9223372036854775807` for `abs`) are legitimate; iteration caps for `--unwind` are not. If a correct contract is unverifiable, mark the function unverifiable rather than weaken the contract.
4. **Run the full local quality gate before pushing**, from the repo root:
   - `cargo fmt --all`
   - `cargo clippy --all -- -D warnings`  *(CI enforces zero warnings)*
   - `cargo test --all`
   - `scripts/full_test.sh`  *(self-hosted tests, examples, runtime tests, help-coverage staleness check)*
   If any gate fails, fix the root cause. Do not pass `--no-verify`, do not skip clippy, do not narrow test scope to make it green.
5. **When running self-compiled binaries, prefix with `ulimit -v 2000000`.** This bounds virtual memory and catches runaway allocation early.
6. **Do not commit `build/vowc`** or anything else under `build/` — that directory is gitignored. Never `git add -f` it.
7. **Update `docs/spec/*.md`** whenever the change affects the language surface. After updating a spec file, regenerate `--help` and the embedded skill: `uv run python scripts/generate_help.py`, then rebuild both compilers (`cargo build --release -p vow` and `scripts/bootstrap.sh --skip-cargo`). `scripts/check_help_coverage.py` (invoked by `full_test.sh`) catches drift.

## Commit hygiene

- Commit in small focused units that match the TDD slices. Many small commits beat one large one — they are easier to review, `git bisect`, and revert.
- Write commit messages that describe the change and the why. Use a conventional prefix (`fix(...)`, `feat(...)`, `refactor(...)`) consistent with recent history (`git log --oneline -20`).
- Commits in this repo must be authored as `p@ocmatos.com`. If the workspace git config has a different identity, set `user.email` to `p@ocmatos.com` for this repo only (`git config user.email p@ocmatos.com`) before committing.

## Drop the plan before opening the PR

`PLAN.md` was committed by the planning stage purely to hand the plan across the
stage boundary. It is not part of the change and must not ship. Before you push:

```sh
git rm PLAN.md
git commit -m "chore: drop stage-handoff PLAN.md"
```

Then confirm the branch adds nothing but the real change:

```sh
git diff --stat main...HEAD   # must not list PLAN.md
```

Do this as your last commit, after the quality gate has passed — you may want to
re-read the plan up to that point. If `git diff --stat main...HEAD` still lists
`PLAN.md`, the removal did not land; fix it before opening the PR.

## Open the PR

Push `{{branch.name}}` to `origin`, then:

```sh
gh pr create --base main --head {{branch.name}} \
  --title "<conventional title — no agent prefix like [claude] or [codex]>" \
  --body "<summary>\n\nCloses #{{issue.number}}"
```

The repository allows **squash merges only** (`CLAUDE.md`), and the squash subject is taken
verbatim from the **PR title** — so the title must be a valid commitlint header on its own and
must stay within 100 characters *including* the ` (#N)` GitHub appends.

The PR must be **non-draft**. Do not use `--web`, `--draft`, or any flag that opens a browser or waits for input. Do not call the GitHub MCP connector tools — use the local `gh` CLI for every mutation.

## After the PR is open

- Remove the readiness label so the orchestrator does not re-schedule: `gh issue edit {{issue.number}} --remove-label ready-for-agent`.
- Do **not** apply `needs-human` or any `sym:*` label as an exit strategy. The operator owns those.
- Do **not** merge the PR, and do **not** wait on it. The orchestrator owns the merge: once the
  PR is open it drives the `wait_for_pr` / `merge` states and squash-merges when checks pass,
  the branch is mergeable, and there are no unresolved review threads. Exit as soon as the PR
  is open.

## If you cannot proceed

Post one explanatory comment with `gh issue comment {{issue.number}} --body "<what blocked you and what would unblock it>"`, write the same explanation to `{{workspace.path}}/EVIDENCE.md`, and exit cleanly. Do not self-apply `needs-human` or any handoff label.

## Defer to this contract

Defer to this prompt over any agent-side persistent memory, skills, or default conventions for PR drafting, title prefixes, label management, or merge strategy.
