# Plan: issue #1201 — mask code spans and fenced blocks before escaping-link rewrite

## 1. Problem restated

`_retarget_escaping_links` in `scripts/build_docs_site.py` runs `ESCAPING_LINK`
and `ESCAPING_REF_LINK` over each canonical page's raw text with no awareness of
Markdown structure. A literal Markdown-link example written inside an inline
code span (`` `[guide](../missing.md)` ``) or a fenced code block is
indistinguishable, to the regex, from a real link: it either aborts the build
via `_resolve_target`'s `SystemExit` when the shown example target doesn't
exist on disk, or — worse — silently rewrites it to a GitHub URL and corrupts
the rendered code sample when the target happens to exist. Verified empirically
(see below): none of the 7 pages currently processed (`REFERENCE_PAGES` +
`stdlib.md`) has a live instance of this, so the fix closes a structural gap
with no current visible regression to point at — it must be validated by new
targeted unit tests, not by a diff in `website/docs/`.

Confirmed during planning: a full scan of `docs/spec/{grammar,contracts,contracts-methodology,cli,errors,examples,stdlib}.md`
for `](...)`-shaped text inside fenced blocks (```` ``` ````/`~~~`) or inline
code spans returns zero matches. The only live escaping links are the three
bare `](../path.md)` occurrences in `grammar.md`, already covered by
`scripts/test_build_docs_site.py`.

## 2. Files to touch

- `scripts/build_docs_site.py` — production code. Add a structure-aware
  "protected range" computation and thread it through `_retarget_escaping_links`'s
  two substitution closures (`repl` at line 70, `ref_repl` at line 79).
- `scripts/test_build_docs_site.py` — new unit tests (see TDD slices below).
- No changes to `docs/spec/*.md`: confirmed zero live matches inside code
  spans/fences, so no canonical page needs editing.
- No changes to `crates/`, `compiler/`, or `docs/spec/` as language spec —
  this is a docs-site build script, not a Vow language or compiler change.
  The cross-cutting "update both compilers" and "update docs/spec on syntax
  change" rules in `CLAUDE.md` do not apply here.

## 3. TDD slices

Each slice is red (failing test against current `build_docs_site.py`) → green
(minimal production code) → refactor if needed. All tests live in
`scripts/test_build_docs_site.py`, run via `python3 scripts/test_build_docs_site.py`
(plain `unittest`, no `pytest`/third-party import — see Risk areas).

1. **Fenced block protects a dead-target example (abort case).**
   Test: a fenced ` ``` ` block containing `` [guide](../missing.md) `` as
   literal prose; `missing.md` does not exist under `docs/`. Assert
   `_retarget_escaping_links` returns the input **unchanged** and does **not**
   raise `SystemExit`. This is the sharper of the two regressions in the issue
   (build-aborting) and should drive the first pass at the fenced-block scanner:
   a line-oriented pass recognizing CommonMark fence open/close (`` ` `` or `~`,
   run length ≥ 3, **any leading whitespace** — not just CommonMark's ≤3-space
   rule, see note below — closing run length ≥ opening, same character),
   producing a list of `(start, end)` byte ranges to protect. Wire it into
   `repl`: if `match.start()` falls inside a protected range, return
   `match.group(0)` unchanged instead of calling `_resolve_target`.
   Note on indent: `grep -nE '^ {4,}(```|~~~)' docs/spec/*.md` returns zero
   matches today, so there is no live fence nested inside a list item at 4+
   spaces indent — but a real Markdown fence nested in a list *can* be
   indented that far, and without container-block tracking the scanner can't
   tell "4-space-indented fence inside a list" from "coincidental backticks
   after an indented-code-block line" (the latter doesn't exist in these
   docs either, per section 6). Since over-matching a fence is safe (it can
   only skip a rewrite, never mis-rewrite one) and under-matching leaves the
   reported bug half-fixed for future content, detect fence markers at any
   indent rather than gating on ≤3 spaces.

2. **Fenced block protects an existing-target example (silent-corruption case).**
   Test: same shape, but the fenced example points at a target that *does*
   exist (e.g. `` [details](../verifier-discipline.md) `` inside a fence).
   Assert the text is returned byte-for-byte unchanged (not rewritten to a
   GitHub URL). This guards the corruption failure mode specifically, since
   slice 1 alone could pass via a bug that only special-cases the
   `SystemExit` path.

3. **`~~~` fence variant.** Test: same as slice 1 but using a `~~~` fence
   instead of `` ``` ``. Confirms the fence scanner isn't hardcoded to one
   marker character.

4. **Fenced block protects a reference-style link definition.** Test: a fence
   containing a standalone line `` [details]: ../missing.md `` as an example
   of reference-link syntax. Assert unchanged, no `SystemExit`. Wire the same
   protected-range check into `ref_repl`.

5. **Inline code span (single backtick) protects the exact issue example.**
   Test: prose text `` See `` `[guide](../missing.md)` `` for the syntax. ``
   (the exact shape from the issue body). Assert unchanged, no `SystemExit`.
   Drives the inline-span scanner: walk the text outside already-protected
   fenced ranges, find backtick runs, and for each opening run of length *n*
   search forward for a closing run of exactly length *n* (CommonMark rule);
   if found, protect `[opening-run-start, closing-run-end)`; if not found,
   the backticks are literal — advance past them without protecting anything.
   Per CommonMark, a code span cannot cross a paragraph boundary, so the
   closer search must stop at the first blank line (`\n\s*\n`) after the
   opener; if no same-length closer appears before that boundary, treat the
   opening run as literal and resume scanning from just past it (not from
   end of file). Without this bound, a single stray unmatched backtick
   earlier in a page would pair with the next unrelated opener and cascade,
   silently protecting (and thus never validating) every real link after it.

6. **Double-backtick span containing a literal single backtick.** Test:
   `` ``[guide](../missing.md)` `` `` (a `` `` `` `-delimited span whose content
   itself contains one backtick). Assert unchanged. Confirms the scanner
   matches run length exactly rather than treating any backtick as a
   delimiter.

6b. **Unmatched backtick does not cascade across a paragraph boundary.**
   Test: a paragraph containing a single stray `` ` `` with no closer, a
   blank line, then a second paragraph containing a real
   `[details](../verifier-discipline.md)` link. Assert the stray backtick is
   left as literal text and the real link in the following paragraph is
   still rewritten to `f"{GITHUB_BLOB}/docs/verifier-discipline.md"`. This is
   the regression test for the closer-search bound introduced in slice 5.

7. **Adjacent-content precision (regression guard on range boundaries).**
   Test: a single line containing an unrelated code span *before* a real,
   should-be-rewritten link, e.g.
   `` Use `foo()` and see [details](../verifier-discipline.md). ``.
   Assert the code span is left alone **and** the trailing real link is still
   rewritten to `f"{GITHUB_BLOB}/docs/verifier-discipline.md"`. This is the
   slice most likely to catch an off-by-one in the protected-range interval
   (e.g. accidentally protecting "rest of the line after a span" instead of
   just the span itself).

8. **Existing suite stays green.** No new test, but rerun the full existing
   `scripts/test_build_docs_site.py` suite (11 pre-existing tests covering
   plain/fragment/title link variants, dead-target detection, sibling links,
   and reference-style equivalents) after each slice to confirm the protected-
   range logic never widens to swallow ordinary unprotected links.

9. **End-to-end sanity against the real spec tree (manual, not a unit test).**
   Run `python3 scripts/build_docs_site.py` against the live `docs/spec/`
   directory and diff `website/docs/` before/after. Expected: byte-identical
   output — the three live `grammar.md` escaping links still rewrite to GitHub
   URLs, and no page changes shape, confirming the new masking is inert on
   real content (matches the issue's own empirical finding of zero live
   matches). This is the acceptance check that closes the issue in practice,
   since the unit tests above are synthetic by necessity.

## 4. Verification surface

Not applicable in the ESBMC/contracts sense: this change touches
`scripts/build_docs_site.py`, a Python docs-tooling script outside the Vow
compiler pipeline. It does not touch `crates/`, `compiler/`, contracts,
codegen, or the C model, so there are no ESBMC properties to prove and no
`tests/run/`/`examples/` fixtures to grow. The "verification surface" for this
change is the `unittest` suite in `scripts/test_build_docs_site.py`
(slices 1–8) plus the manual real-tree diff (slice 9), both gated by
`.github/workflows/ci.yml`'s `python3 scripts/test_build_docs_site.py` step
and `.github/workflows/docs.yml`'s `python3 scripts/build_docs_site.py` step
respectively.

## 5. Risk areas

- **CI has no Python dependency install for this script.**
  `.github/workflows/ci.yml` runs `python3 scripts/test_build_docs_site.py`
  directly with no `pip install` step beforehand (unlike `bench/` or `euler/`,
  which have their own `pyproject.toml`). The implementation **must** stay
  stdlib-only (`re`, no `markdown-it-py`/`mistune`/etc.) or the CI job breaks
  with `ImportError`. This is the deciding factor for the hand-rolled
  line/backtick scanner over "switch to a proper Markdown link-node walker"
  (the issue's alternative suggestion) — see Out of scope.
- **Off-by-one in protected-range boundaries** is the main correctness risk:
  under-protecting regresses to the current bug; over-protecting (e.g.
  protecting past the end of a code span to the end of the line, or treating
  an unterminated backtick run as a span to end-of-file) could let a *real*
  escaping link inside ordinary prose pass through unrewritten and
  unvalidated, reaching `website/docs/` as a dead `../` link that
  `zensical build --strict` may not catch the same way `_resolve_target`
  does today. Slice 7 exists specifically to catch this class of bug.
- **Unbounded code-span closer search cascading past its paragraph.** An
  unmatched stray backtick anywhere earlier in a page could pair with the
  next legitimate code-span opener if the closer search isn't bounded at the
  paragraph (blank-line) boundary, silently protecting — and thus never
  validating — every real link on the rest of the page. Slice 6b is the
  regression test for this specific failure mode.
- **Fence-matching rules** (marker character, run-length ≥ opening, ≤3-space
  indent tolerance) are easy to get subtly wrong (e.g. allowing a `` ``` ``
  fence to be closed by a shorter run, or by a `~~~` of matching length).
  Slices 1 and 3 cover both marker characters; if the implementer wants
  stronger coverage, an extra test for "closing fence shorter than opening
  fence is not treated as a close" is a reasonable addition but not required
  to close the issue.
- **No impact on the Vow compiler or binary fixed point.** This change cannot
  affect `compiler/` codegen ordering, `BTreeMap`/`HashMap` choices,
  `vow-clif-shim` stack-slot layout, or `parse → print → parse` idempotency —
  none of that code is touched. Likewise `cargo clippy --all -- -D warnings`
  is unaffected (no Rust files change).
- **`.github/workflows/docs.yml` runs the real build**, not just unit tests —
  a subtle regression in the scanner could pass all synthetic unit tests yet
  still alter real page output in a way that breaks `zensical build --strict`.
  Slice 9's manual diff is the safety net for this; the implementer should run
  it before opening the PR, not rely on unit tests alone.

## 6. Out of scope

- **Indented (4-space) code block masking.** The issue's own description of
  "a correct fix" mentions masking indented code blocks alongside fences and
  spans. Checked during planning: `grep -cE '^ {4,}\S' docs/spec/*.md` shows
  68 (grammar.md), 47 (errors.md), 66 (cli.md), 155 (examples.md), 68
  (contracts.md), 19 (contracts-methodology.md), 14 (stdlib.md) lines
  matching a naive "4-space indent" heuristic — almost all of these are
  ordinary numbered/bulleted-list continuation paragraphs, not code blocks.
  CommonMark only treats 4-space indentation as a code block when it is
  *not* interpretable as a continuation of a surrounding list or other
  container block, which requires tracking container-block context — exactly
  the "proper Markdown parser" work the issue offers as the alternative to
  masking. A naive indent-based detector here would risk the same failure
  mode this issue is fixing, in reverse: masking (and thus silently letting
  through unvalidated) a real escaping link that happens to sit in indented
  list-continuation prose. Deferred as a follow-up, only worth doing if a
  live case ever appears — same "diminishing returns" judgement the original
  PR review made, now scoped down to the one remaining ambiguous construct.
- **Switching to a real Markdown parser / link-AST walker.** Rejected for
  this slice per the CI risk above (no dependency-install step exists for
  this script) and because fences + code spans are unambiguous to detect with
  a hand-rolled scanner — no parser is needed to close the reported gap.
- **Editing any `docs/spec/*.md` content.** Confirmed zero live matches
  inside code spans/fences; no canonical page needs a change.
- **Any `crates/` or `compiler/` changes.** Unrelated to Vow language
  semantics; this is a docs-site build script only, so the "update both
  compilers in the same session" rule does not apply.
- **Refactoring beyond threading the protected-range check through `repl`/
  `ref_repl`.** No unrelated cleanup of `_resolve_target`, `REFERENCE_PAGES`,
  or the `main()` copy loop.
- **Masking other CommonMark constructs that can also contain `](...)`-shaped
  text** (raw HTML blocks/comments, autolinks). Not mentioned in the issue,
  no evidence of live occurrence in the 7 processed pages; out of scope.
