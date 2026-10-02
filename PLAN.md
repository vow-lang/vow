# Plan: issue #1181 — impl.md PR body passes literal `\n`, breaking the `Closes` autolink

## 1. Problem restated

`symphonika/prompts/impl.md` instructs the implementation agent to open its PR with
`gh pr create ... --body "<summary>\n\nCloses #{{issue.number}}"`. Bash does not expand `\n`
inside double quotes, so `gh` receives the six literal characters `\n\nCloses` as part of one
argument instead of a blank line followed by `Closes`. GitHub's issue-closing-keyword matcher
(`\bcloses\s+#\d+`) never sees a `closes` token bounded correctly, so the autolink silently
fails to register. This has already happened twice in production (PRs #856, #867), both requiring
a manual issue-closing sweep days later. The fix is to replace the double-quoted `--body` with
a quoted heredoc piped through `--body-file -`, which needs no shell escaping and always produces
real newlines.

## 2. Files to touch

This is a prompt/tooling fix, not a language or compiler change — it lives entirely outside the
Rust workspace and `compiler/`. Confirmed by repo-wide grep: no script, test, or `workflow.yml`
entry inlines or parses this prompt text; `workflow.yml:44` only references the file by path
(`prompt: prompts/impl.md`), it does not embed the body.

- `symphonika/prompts/impl.md` — the only file with the bug (`grep -rn '\\n' symphonika/prompts/*.md` finds exactly one hit, line 52). Replace the `--body "...\n\n..."` line with a quoted heredoc through `--body-file -`, and add one sentence next to the block stating *why* (bash doesn't expand `\n` in double quotes; a literal `\n` breaks the `Closes` autolink), so a future edit doesn't collapse it back into a one-liner.

No other `symphonika/prompts/*.md` file contains `--body "...\n..."` (checked all five:
`autofix-pr.md`, `code-review-fix.md`, `impl.md`, `plan.md`, `resolve-conflicts.md`,
`simplify.md`). The two other `--body` call sites in `impl.md` (line 69) and `plan.md` (line 82)
are single-line `gh issue comment ... --body "<explanation>"` strings with no embedded `\n` —
out of scope (see §6).

- No `docs/spec/*.md` update — this touches no Vow syntax, semantics, type, builtin, operator,
  effect, or CLI flag.
- No `compiler/*.vow` counterpart — the dual-compiler rule in `CLAUDE.md` applies to language
  implementation changes; this is agent-prompt text consumed by Symphonika, not by either Vow
  compiler.

## 3. TDD slices

There is no existing test harness for `symphonika/prompts/*.md` (they are prompt text rendered
by Symphonika's workflow engine, not parsed or executed by this repo's test suites), and
inventing a markdown-prompt-testing framework for a one-line fix would be exactly the kind of
unrelated infrastructure `CLAUDE.md`'s "surgical changes" rule forbids. Instead, treat the
fenced shell block itself as the unit under test and verify it directly, red before green:

1. **Red — reproduce the bug as issue #1181 describes it.**
   Extract today's fenced block (`sed -n '/^gh pr create/,/^```/p' symphonika/prompts/impl.md`,
   or just re-run the issue's own repro) and confirm:
   ```sh
   python3 -c "import re,sys; print(bool(re.search(r'\bcloses\s+#\d+', sys.argv[1], re.I)))" \
     "$(printf '%s' "<summary>\n\nCloses #123")"
   ```
   prints `False` — matching the issue body's own demonstration. This is a manual confirmation
   step, not a committed test file; it establishes the failure the fix must close.

2. **Green — edit `impl.md` and verify the new block produces a real autolink match.**
   Replace lines 50–52 with:
   ```sh
   git push -u origin {{branch.name}}
   gh pr create --base main --head {{branch.name}} \
     --title "<conventional title — no agent prefix like [claude] or [codex]>" \
     --body-file - <<'BODY'
   <summary>

   Closes #{{issue.number}}
   BODY
   ```
   (closing `BODY` delimiter at column 0 — an indented terminator never closes a heredoc).
   Then verify mechanically, without invoking the real `gh` or GitHub:
   ```sh
   cat > /tmp/gh <<'STUB'
   #!/bin/sh
   if [ "$1" = pr ] && [ "$2" = create ]; then cat > /tmp/pr-body.txt; fi
   STUB
   chmod +x /tmp/gh
   # render the heredoc with a sample issue number (e.g. 1181) substituted for {{issue.number}}
   # and sample values for {{branch.name}}, run it with PATH=/tmp:$PATH so the stub captures stdin,
   # then:
   python3 -c "import re; b=open('/tmp/pr-body.txt').read(); \
     assert re.search(r'\bcloses\s+#\d+', b, re.I); assert '\n\n' in b; print('ok')"
   ```
   This must print `ok`: a real blank line precedes `Closes`, and the keyword matches the
   same regex the issue used to demonstrate the bug. This confirms the *actual edited file*
   produces a correct body — not a hand-retyped copy — which is what catches delimiter or
   indentation mistakes a visual review would miss.

3. **Refactor / consistency pass.** Re-read the other four `--body`/`--body-file` call sites in
   `symphonika/prompts/*.md` (there are none with multi-line content today) so no analogous
   literal-`\n` pattern is introduced elsewhere by accident. No code change expected from this
   step — it is a verification pass, not new work.

There is no production code to write beyond the single markdown edit; slices 1–2 are the
red/green pair, slice 3 is the guard against scope creep.

## 4. Verification surface

Not applicable in the ESBMC/contract/codegen sense — this change has zero contact with
`vow-verify`, the C model, or any `.vow` contract. No new properties for ESBMC to prove, and no
new fixtures under `tests/run/` or `examples/` (those directories hold Vow-language test
programs; this fix touches none of them).

The only "verification" this PR needs:
- The manual red/green check in §3 above, recorded in the PR description (command + `ok` output),
  since there is no committed automated test for prompt text.
- A **post-merge-adjacent self-check specific to this issue**: this very PR is itself opened by a
  human/agent following the *current* (unfixed) `impl.md`, since the fix doesn't apply to its own
  creation. Write this PR's body by hand with a real blank line (not by following the buggy
  template), and after opening it, run `gh pr view --json closingIssuesReferences` and confirm
  `#1181` is listed — the fastest possible confirmation that a correctly-formed body does what
  the issue asks.

## 5. Risk areas

None of the usual Vow-specific fixed-point risks apply:
- **Binary fixed point** (`compiler/` codegen ordering, `BTreeMap`/`HashMap`, `vow-clif-shim`
  stack slots) — untouched; no `compiler/*.vow` or Rust crate file changes.
- **`parse → print → parse` idempotency** — untouched; no `vow-syntax` changes.
- **`cargo clippy --all -- -D warnings`** — untouched; no Rust source changes, so the gate is a
  no-op for this PR (still fine to run `cargo clippy --all -- -D warnings` for a clean CI signal,
  but no diff should appear under it).

The one real risk is heredoc mechanics themselves, since this prompt is interpreted by a future
headless agent with no human to catch a shell mistake at edit time:
- The closing delimiter `BODY` must be un-indented (column 0) and quoted (`<<'BODY'`, not
  `<<BODY`) so neither variable expansion nor command substitution runs on an agent's summary
  text (which may itself contain `$`, backticks, or stray `BODY`-looking lines). Quoting also
  means the implementer never needs an escaping rule, which was the root cause of this issue.
- Markdown fencing: the heredoc body itself contains a blank line, which must render inside the
  triple-backtick fence without the Markdown renderer (or `trailing-whitespace` pre-commit hook
  with `--markdown-linebreak-ext=md`) stripping or flagging it. Verified locally with
  `pre-commit run trailing-whitespace --files symphonika/prompts/impl.md` as part of the normal
  commit flow — no special handling needed since the blank line has no trailing whitespace.

## 6. Out of scope

- The two other `--body "<explanation>"` call sites (`impl.md:69`, `plan.md:82`, both
  `gh issue comment`) — single-line strings with no embedded `\n`, nothing to fix.
- Adding a CI/pre-commit lint that greps prompt files for literal `\n` inside `--body` strings.
  Worth considering as a follow-up issue, but it's a new guard-rail mechanism, not part of
  closing #1181, and would need its own design (false-positive risk against legitimate `\n`
  mentioned in prose, e.g. this very PLAN.md).
- Any reformatting of `impl.md` beyond the touched lines (the file's wrapping width, heading
  structure, and surrounding prose stay as-is).
- Porting the fix to `pmatos/jsse` — the issue states that repo is already fixed; it's a
  separate repo and out of this PR's reach regardless.
