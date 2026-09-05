# Plan: issue #1198 — escape reference-style `../` links in copied spec pages

## 1. Problem restated

`scripts/build_docs_site.py` copies the canonical `docs/spec/*.md` pages into
`website/docs/` and rewrites any `../`-escaping link so it points at GitHub
instead of a path that doesn't exist in the copied set. The rewrite regex
`ESCAPING_LINK` (`scripts/build_docs_site.py:45`) only matches **inline**
Markdown links — `](../foo.md)` — via `_retarget_escaping_links`
(`scripts/build_docs_site.py:48`). It does not match **reference-style** link
definitions, `[label]: ../foo.md`, which live on their own line rather than
inside a `](...)` construct. A grep of `docs/spec/` today finds zero such
definitions, so nothing is silently mis-published right now, but if one is
ever added it would pass through `_retarget_escaping_links` untouched, keep
its raw `../foo.md` path in the copied page, and only get caught later by
`zensical build --strict`'s dead-link check (a loud but distant failure,
several build steps removed from the actual cause). This is a pure Python
docs-tooling change — it does not touch the Rust compiler, the self-hosted
compiler, or `docs/spec/*.md` content itself, since no language surface is
involved.

## 2. Files to touch

- `scripts/build_docs_site.py` — add a second regex (or generalize the
  existing one) to also match and rewrite reference-style link definitions,
  plus a small helper/branch in `_retarget_escaping_links` (or a sibling
  function) to produce the reference-style replacement form.
- `scripts/test_build_docs_site.py` — new test cases for the reference-style
  form, mirroring the existing inline-link cases (plain, fragment, dead
  target, sibling-link-untouched).
- No `crates/`, no `compiler/`, no `docs/spec/*.md` changes — this is tooling
  that processes spec content, not a change to Vow language syntax,
  semantics, builtins, or CLI flags, so the "any spec change needs a
  docs/spec update" rule in `CLAUDE.md` does not apply here. `docs/spec/`
  stays the input, unchanged.

## 3. TDD slices

1. **Red:** add `test_reference_style_link_is_rewritten` to
   `scripts/test_build_docs_site.py` asserting that
   `"[details]: ../verifier-discipline.md"` run through
   `_retarget_escaping_links` becomes
   `f"[details]: {bds.GITHUB_BLOB}/docs/verifier-discipline.md"`. This fails
   today because `ESCAPING_LINK` doesn't match the `label]: ../path` shape at
   all, so the string is returned unchanged.
   **Green:** extend `_retarget_escaping_links` to also recognize and rewrite
   this shape (see design note below).

2. **Red:** add `test_reference_style_link_with_fragment_is_rewritten`
   covering `"[details]: ../verifier-discipline.md#some-heading"`.
   **Green:** reuse the same anchor-capture group logic already proven for
   inline links.

3. **Red:** add `test_reference_style_link_with_title_is_rewritten` covering
   the optional reference-definition title forms Markdown allows —
   `[details]: ../verifier-discipline.md "Verifier discipline"` — since the
   existing inline-link title bug (folding the title into the path capture)
   is exactly the kind of regression this generalization must not
   reintroduce.
   **Green:** mirror the existing `(\s+"[^"]*")?` title group in the new
   pattern.

4. **Red:** add `test_reference_style_dead_target_still_raises` — a
   reference-style link to a nonexistent `docs/` target must still raise
   `SystemExit` with the same loud-failure message shape as the inline case,
   not silently fall through.
   **Green:** route the reference-style match through the same
   existence-check branch (shared helper) used by the inline-link `repl`.

5. **Red:** add `test_reference_style_sibling_link_is_untouched` — a
   reference-style definition without a `../` prefix (e.g.
   `[errors]: errors.md#e001`) must be left alone, matching the existing
   inline-link sibling-link behavior.
   **Green:** confirm the new pattern only matches when `../` is present
   (it already anchors on `\.\./`, so this should pass once the pattern is
   scoped correctly — write the test first to prove it).

6. **Refactor:** once both shapes are covered and green, factor the shared
   "does `docs/<target>` exist, else raise" logic out of the two `repl`
   closures into one small helper (e.g. `_resolve_or_raise(target, anchor,
   title, page) -> str` returning the GitHub-URL fragment), so
   `_retarget_escaping_links` runs both regex substitutions through the same
   validation path instead of duplicating the `SystemExit` message. Re-run
   the full test file after the refactor to confirm no behavior changed.

### Design note on the regex generalization

Reference-style link definitions have the form `[label]: url "optional
title"` at the start of a line (per CommonMark, optionally indented up to 3
spaces). A minimal, non-overreaching pattern:

```python
ESCAPING_REF_LINK = re.compile(
    r'^(\s{0,3}\[[^\]]+\]:\s*)\.\./([^\s]+?)(#[^\s]*)?(\s+"[^"]*")?\s*$',
    re.MULTILINE,
)
```

capturing the `[label]: ` prefix separately so the replacement can
reconstruct `f'{prefix}{GITHUB_BLOB}/docs/{target}{anchor}{title}'` without
touching the label. Keep this as a **second** compiled pattern rather than
trying to merge it into `ESCAPING_LINK` — the two link shapes have different
surrounding syntax (`](...)`  vs. line-anchored `label]: ...`), and forcing
one regex to match both would make the pattern harder to read for near-zero
line savings. This keeps `_retarget_escaping_links` doing two clear
`.sub()` passes rather than one convoluted one.

## 4. Verification surface

None. This change has no interaction with contracts, codegen, the C model,
or ESBMC — it is a pure-Python text-rewriting utility operating on Markdown
strings at docs-build time, entirely outside the Vow compiler pipeline. No
`tests/run/` or `examples/` fixtures are relevant. The only "verification"
here is the existing `python3 scripts/test_build_docs_site.py` unittest
suite (wired into `.github/workflows/ci.yml:93`) plus, if desired as a final
manual check, running `python scripts/build_docs_site.py` against a scratch
copy of `docs/spec/` containing a synthetic reference-style escaping link to
confirm the end-to-end copy step also rewrites it (belt-and-suspenders check
beyond the unit tests, not a required CI addition).

## 5. Risk areas

- **None of the binary-fixed-point risks apply.** This PR touches no
  `compiler/` codegen, no `vow-clif-shim`, no `BTreeMap`/`HashMap` ordering,
  and no stack-slot layout — it is entirely outside the self-hosted/Rust
  compiler pair.
- **No `parse → print → parse` idempotency risk.** No Vow syntax or AST is
  involved.
- **`cargo clippy --all -- -D warnings` is unaffected** — no Rust code
  changes.
- **Actual risk is regex correctness**, specifically: (a) not
  over-matching plain prose that happens to contain `]:` followed by a
  relative path in a code fence or inline code span (mitigate by anchoring
  the reference-style pattern to line-start, matching CommonMark's own
  reference-definition grammar, and by running it only over the same
  canonical files already covered by `REFERENCE_PAGES` / `stdlib.md`); (b)
  reintroducing the title-folding bug the inline-link tests already guard
  against — avoided by copying the proven `(\s+"[^"]*")?` group verbatim;
  (c) double-processing a link that matches both patterns — not possible
  here since the two shapes are syntactically disjoint (one requires `](`,
  the other requires a line-starting `[label]:`), but worth a quick manual
  check during review that no pathological Markdown triggers both `.sub()`
  passes on overlapping spans.
- No repo-wide Python lint gate (e.g. `ruff`) was found wired into
  `.github/workflows/ci.yml` for `scripts/`, so the only required check for
  this change is running `python3 scripts/test_build_docs_site.py` directly,
  matching how CI invokes it at `ci.yml:93`.

## 6. Out of scope

- Do **not** merge `ESCAPING_LINK` and the new reference-style pattern into
  a single regex — keep them as two clear, separately testable patterns per
  the design note above.
- Do **not** add reference-style escaping links to any real `docs/spec/*.md`
  file as part of this PR — the issue is about making the tooling handle
  that link style *if and when* it appears, not about introducing it.
- Do **not** refactor `_reset`, `main`, or the schema-copying logic in
  `scripts/build_docs_site.py` — unrelated to this fix.
- Do **not** add a general CommonMark reference-link parser or pull in a
  Markdown library — the existing regex-based approach is proportionate to
  the narrow, closed set of canonical files this script processes.
- Do **not** touch `.github/workflows/ci.yml` — `test_build_docs_site.py` is
  already wired in at line 93 and needs no new invocation.
