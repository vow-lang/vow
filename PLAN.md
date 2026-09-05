# Plan: validate anchors on externalized `../` spec links (#1199)

## 1. Problem restated

`scripts/build_docs_site.py` copies the canonical `docs/spec/*.md` pages into
`website/docs/reference/` and rewrites any `../`-escaping link (one whose
target lives outside the copied set) into an absolute GitHub blob URL via
`_retarget_escaping_links` / `_resolve_target`. Today `_resolve_target` only
checks that the target *file* exists on disk; it ignores a trailing
`#fragment`. Because the rewritten link becomes an external URL, the site
builder's own link/anchor validation (which only covers pages it actually
builds) can never catch a stale fragment — a renamed heading in, say,
`docs/verifier-discipline.md` would ship as a silently broken GitHub deep
link. This is a pure `scripts/` (Python tooling) change: no compiler, no
`crates/`, no `compiler/`, no `docs/spec/*.md` semantics change. It closes the
gap by computing the target file's real heading anchors (replicating GitHub's
Markdown heading-to-anchor slugification) and failing loudly, the same way
`_resolve_target` already fails loudly on a missing file, when the fragment
doesn't match any of them.

## 2. Files to touch

Pure Python tooling; no Rust/`crates/`, no self-hosted `compiler/`, no
`docs/spec/*.md` content change (this changes how links are *rewritten*, not
the language or CLI surface, so `docs/spec/cli.md`/`grammar.md`/etc. are not
touched).

- `scripts/build_docs_site.py` — add the slugification/anchor-extraction
  helpers and wire them into `_resolve_target`.
- `scripts/test_build_docs_site.py` — add new test coverage; **fix two
  existing tests that will start failing** once fragment validation lands
  (see Risk Areas — they currently use `#some-heading`, which is not a real
  heading anchor in `docs/verifier-discipline.md`).

No other files need to change. `website/mkdocs.yml` is unaffected (the gap
described in the issue is that MkDocs/Zensical validation never covered these
targets in the first place — nothing to reconfigure there).

## 3. Ground truth used to design the algorithm

I fetched the actual reference implementation GitHub uses for Markdown
heading anchors (`github-slugger`, the de facto standard reimplementation of
GitHub's own slugifier: lowercase → strip a fixed set of punctuation/symbol
characters (keeping Unicode letters/digits/underscore/hyphen/space) → replace
each space with a hyphen) and ran it under Node against every real heading in
`docs/verifier-discipline.md`, `docs/adr/*.md`, and a sample from
`docs/spec/*.md` that includes em-dashes, arrows, colons, parens, numbers,
and inline code spans. I then verified that the much simpler Python
expression

```python
re.sub(r"[^\w -]", "", text.lower(), flags=re.UNICODE).replace(" ", "-")
```

reproduces `github-slugger`'s output **exactly** for all 12 sampled headings,
including the tricky ones. Use `[^\w -]` (a literal space, not `\s`) in the
strip class: `\s` also matches tab and non-breaking space, which
`github-slugger` strips but which survive `\s`-preservation and then aren't
caught by a plain `.replace(" ", "-")` — using a literal space keeps the
"exact port" claim true for the full ASCII/Latin-1 range this repo could
plausibly contain, not just the sampled headings:

| Heading (raw)                                                              | Slug                                                          |
|---|---|
| `0001. Numeric tower — narrow integer types`                                | `0001-numeric-tower--narrow-integer-types` |
| `Verifier Discipline: Safe vs Unsafe Adaptive Retry`                        | `verifier-discipline-safe-vs-unsafe-adaptive-retry` |
| `5. Command Loop — EOF-Safe \`stdin_read_line\``                            | `5-command-loop--eof-safe-stdin_read_line` |
| `2. Output-range postcondition (the weak default — use sparingly)`         | `2-output-range-postcondition-the-weak-default--use-sparingly` |
| `2. CEGIS Broken → Fixed — The Core Workflow`                               | `2-cegis-broken--fixed--the-core-workflow` |
| `WS-1 — Make verification *honest* (the C emitter)`                        | `ws-1--make-verification-honest-the-c-emitter` |

This works because Python's Unicode-aware `\w` already excludes exactly the
punctuation/symbol categories GitHub's explicit strip-list excludes (for
every character actually in this repo's headings), while `_` and Unicode
letters/digits survive — matching e.g. `stdin_read_line` surviving as one
token and inline-code backticks vanishing for free (backtick is not a `\w`
character, so no special-casing of `` `code` `` spans is needed). Note double
hyphens are the *correct*, GitHub-faithful output when an em-dash or arrow
sits between two spaces (the symbol is deleted, not replaced, so the two
surrounding spaces each become their own hyphen) — a naive "collapse
whitespace first" reimplementation would get this wrong and must not be used.

**Known, accepted gap:** GitHub renders Markdown before slugifying, so
`_italic_` (single-underscore emphasis) becomes `italic`, but underscore is a
kept character in the slug algorithm, so slugifying the raw source text of a
heading using that syntax would wrongly keep the underscores. Verified by
running `grep -nE '^#{1,6} ' docs/*.md docs/adr/*.md docs/spec/*.md | sed
's/`[^`]*`//g' | grep -E '(^|[^[:alnum:]_])_[^_ ][^_]*_([^[:alnum:]_]|$)'`
(code spans stripped first, so `` `stdin_read_line` `` doesn't false-positive)
— zero matches. Asterisk emphasis (`*italic*`) does appear in three real
headings (e.g. `WS-1 — Make verification *honest* (the C emitter)` in
`docs/roadmap-0.3.0-foundations.md`) and is included in the table above,
confirmed to slugify correctly with no special-casing since `*` is already a
non-word character. A separate `grep -nE '^#{1,6} .*\]\('
docs/*.md docs/adr/*.md docs/spec/*.md` for Markdown links inside headings
(`[label](url)`, which would leak the URL text into the slug since brackets
and parens vanish but the URL's alphanumerics don't) also returns zero
matches. Both gaps (`_italic_` emphasis and in-heading links) are documented
in code as known limitations rather than worked around with speculative
parsing — see Out of Scope.

## 4. TDD slices

All slices live in `scripts/build_docs_site.py` (production) and
`scripts/test_build_docs_site.py` (tests), run via
`python3 scripts/test_build_docs_site.py`.

1. **`_slugify_heading(text: str) -> str` — pure slugification.**
   Add a new test class `SlugifyHeadingTest` with one assertion per row of
   the table above (9 cases, taken verbatim from real repo headings) plus a
   plain-ASCII case (`"The core rule"` → `"the-core-rule"`). Implement
   `_slugify_heading` as the two-line expression validated above. No file
   I/O, no regex ports of GitHub's giant Unicode punctuation table — the
   `\w`-based approach is the whole implementation.

2. **`_heading_anchors(markdown_text: str) -> set[str]` — extract all heading
   slugs from a document, ignoring fenced code blocks.**
   Tests (new class `HeadingAnchorsTest`):
   - a doc with `## Heading One` / `### Heading Two` → both slugs present.
   - a doc where a line starting with `#` appears inside a triple-backtick
     (and separately a triple-tilde) fenced block (e.g. a shell comment
     `# build it`) → that line must **not** contribute a heading/slug.
   - an ATX heading with closing hashes (`## Heading ##`) → trailing `#`s and
     whitespace stripped before slugifying.
   - two identical headings in one doc (`## Foo` twice) → slugs `foo` and
     `foo-1`, matching GitHub's duplicate-heading suffixing.
   Implementation: line-by-line scan, toggling an `in_fence` flag on lines
   starting with ` ``` ` or `~~~` (only fence markers reset it — this mirrors
   the existing repo convention confirmed by grep, not a generic CommonMark
   parser), matching `^\s{0,3}#{1,6}\s+` for real headings (the 0–3-space
   indent tolerance mirrors `ESCAPING_REF_LINK`'s own CommonMark leeway
   elsewhere in this file), stripping a trailing closing-hash run only when
   it's preceded by whitespace (`\s+#+\s*$`, so `## Foo#` keeps its literal
   `#` per CommonMark — only `## Foo ##` has its closing hashes stripped),
   slugifying via `_slugify_heading`, and de-duplicating exactly like
   `github-slugger`'s `occurrences` map: keep a `dict[str, int]` of counts
   seen so far; for each new slug, if it's already a key, bump the counter
   and keep appending `-N` until the candidate is unused (not just `-1`),
   then register whichever candidate was finally chosen — this avoids a
   collision if a document has both a heading that slugifies to `foo` twice
   *and* a separate heading that slugifies to `foo-1`.

3. **Wire anchor validation into `_resolve_target`.**
   Extend `_resolve_target(target, anchor, page)`: after the existing
   file-existence check, if `anchor` is non-empty **and not just `"#"`**
   (GitHub treats a bare trailing `#` as a top-of-page link, not a heading
   reference, so skip validation for that case rather than requiring a
   heading literally named empty-string) and `target` ends in `.md`, read
   the target file's text, compute `_heading_anchors(...)`
   (memoized — see Verification Surface), and if `anchor[1:]` is not in that
   set, `raise SystemExit` with a message naming the page, the dead
   fragment, and the file (mirroring the existing dead-file message style;
   include the list of valid anchors so a human can fix the link without
   re-deriving the slug algorithm by hand).
   New tests in `RetargetEscapingLinksTest`:
   - `test_link_with_valid_fragment_is_rewritten` — point at a **real**
     heading anchor, e.g. `../verifier-discipline.md#the-core-rule`, assert
     clean rewrite (no raise).
   - `test_link_with_stale_fragment_raises` — `../verifier-discipline.md#renamed-heading`
     → `assertRaises(SystemExit)`.
   - `test_link_with_punctuation_heading_fragment_is_rewritten` — a fragment
     against an ADR heading containing an em-dash, e.g.
     `../adr/0001-numeric-tower-narrow-ints.md#0001-numeric-tower--narrow-integer-types`,
     to lock in the double-hyphen behavior end-to-end (not just at the pure
     `_slugify_heading` layer). This couples the test to the literal ADR
     title string, matching the existing `test_dead_target_still_raises`
     style of coupling tests to real repo files rather than fixtures — if
     that ADR's title is ever edited, this test's expected slug must be
     updated alongside it. Acceptable for this PR (keeps the test simple and
     consistent with the file's existing tests); if this friction shows up
     often, a follow-up could switch to a `tmp_path` fixture with
     `unittest.mock.patch.object(bds, "REPO", ...)`, but that's not needed
     to close this issue.
   - repeat the fragment-present/fragment-stale pair for the reference-style
     (`[label]: ../...`) path, since `ref_repl` shares `_resolve_target`.

4. **Fix existing tests broken by real validation** (see Risk Areas): update
   `test_link_with_fragment_is_rewritten` and
   `test_link_with_fragment_and_title_is_rewritten` to use `#the-core-rule`
   (a real anchor in `docs/verifier-discipline.md`) instead of the
   placeholder `#some-heading`, keeping their existing assertions about the
   rewritten URL shape.

Each slice is red→green independently: write the test(s), watch them fail
(slices 1–2 fail with `AttributeError`/`NameError` since the helpers don't
exist yet; slice 3's new tests fail because no validation happens yet and
slice 4's *existing* tests fail once slice 3 lands), then implement.

## 5. Verification surface

No contracts, codegen, or C model involved — this is Python build tooling,
not the Vow compiler or a `.vow` program. Nothing under `tests/run/` or
`examples/` changes. The only "verification" is the unit tests in slice 1–4
plus a manual end-to-end run:

```bash
python3 scripts/build_docs_site.py
```

run from repo root after the change, confirmed to still succeed (proves no
real, currently-passing link in `docs/spec/*.md` regresses). Since
`grep -n '\]( *\.\./[^)]*#' docs/spec/*.md` returns zero matches today (noted
in the issue itself), this run won't exercise the new fragment-checking path
at all — the new unit tests in slice 3 are the only coverage until a real
`#fragment` escaping link is added later, which is expected and fine.

Memoization: add a small cache (e.g. `functools.lru_cache` on a helper that
takes the resolved `Path` and returns `frozenset[str]`) so a target file
referenced by multiple fragments/multiple pages is only read and re-slugified
once per `build_docs_site.py` invocation. Cheap to add now, avoids
accidental quadratic re-parsing as more cross-references accrue — not
over-engineering, just not re-reading the same file N times in a loop that
already exists.

## 6. Risk areas

- **Existing tests will break and must be fixed in the same PR** (not a
  follow-up): `test_link_with_fragment_is_rewritten` and
  `test_link_with_fragment_and_title_is_rewritten` in
  `scripts/test_build_docs_site.py` currently assert that
  `../verifier-discipline.md#some-heading` rewrites cleanly — `#some-heading`
  is not a real anchor in that file, so slice 3 makes these tests start
  raising `SystemExit`. This is caught by the test suite itself (TDD slice 4
  above), not discovered later.
- **False-positive rejection risk** (the exact failure mode the issue warns
  about, citing PR #1176's title-parsing bug): mitigated by empirically
  validating `_slugify_heading` against every real heading currently in the
  targets this feature can reach (`docs/verifier-discipline.md`,
  `docs/adr/*.md`, sampled `docs/spec/*.md` headings) rather than trusting a
  hand-derived regex. The one known unhandled construct (`_italic_`
  single-underscore emphasis in a heading) does not occur anywhere in the
  repo today (verified by grep) and is documented as an accepted gap rather
  than guessed at.
- **Fenced-code false headings**: a `# comment` inside a bash/shell code
  block would be misidentified as a Markdown heading without fence-awareness.
  Verified no current target file has this today, but slice 2 handles it
  unconditionally since it's a basic, unconditionally-correct piece of
  CommonMark structure (not speculative).
- **Binary fixed point / ESBMC / `vow-clif-shim` / `parse → print → parse`**:
  none of these apply — this change touches zero Rust/Vow compiler code.
- **`cargo clippy --all -- -D warnings`**: unaffected (Python-only change).
  The applicable gate is this repo's Python lint step; run whatever the
  project currently pins for `scripts/*.py` (check `.github/workflows/*.yml`
  for the exact `ruff` invocation/pin at implementation time, since none of
  the workflow files searched during planning had a `ruff` step wired up for
  `scripts/` — confirm before assuming there's nothing to run) alongside
  `python3 scripts/test_build_docs_site.py`.
- **CI wiring**: no change needed — `.github/workflows/ci.yml` already runs
  `python3 scripts/test_build_docs_site.py` as the "Docs site link-rewrite
  unit tests" step; the new tests ride along automatically.

## 7. Out of scope (deliberately not bundled)

- Full port of GitHub/`github-slugger`'s exact Unicode punctuation-strip
  table (including astral-plane emoji ranges). The `\w`-based
  approximation is verified exact for everything in this repo; porting the
  full 8 KB table for characters that cannot appear in this project's
  Markdown headings is speculative complexity with no behavioral payoff.
- Handling `_italic_`/`__bold__` underscore-emphasis markers, `[label](url)`
  links inside headings (whose URL text would otherwise leak into the
  computed slug), raw inline HTML, or footnote-style references inside
  headings — none exist in the repo (confirmed by grep, see §3); documented
  as a known gap in code (short comment on `_slugify_heading`), not
  implemented defensively against a scenario that cannot happen today (per
  this repo's own "no validation for scenarios that can't happen"
  discipline).
- Setext-style headings (`Heading\n=======`) — grep confirms only ATX
  (`#`-prefixed) headings are used anywhere under `docs/`; not supported,
  not silently mishandled either (a setext heading would simply not be
  discovered as an anchor, which only makes validation *stricter*, never
  silently wrong in the unsafe direction).
- Any refactor of `_retarget_escaping_links`'s two regexes
  (`ESCAPING_LINK`/`ESCAPING_REF_LINK`) or of `main()`'s copy/reset logic —
  untouched, unrelated to this issue.
- Reformatting or restructuring `scripts/test_build_docs_site.py` beyond the
  four new/updated tests described above.
