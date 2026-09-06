#!/usr/bin/env python3
"""Assemble the Zensical site source from the canonical specification.

The website renders the SAME markdown that lives in `docs/spec/` (and that the
compiler embeds into its agent skill via `generate_help.py`) — it must never fork
those files. This script copies the curated reference pages into `website/docs/`
and applies the few link rewrites needed for the site to build cleanly under
`zensical build --strict`.

Run this before `zensical build` / `zensical serve`. The generated pages are
gitignored; the hand-written pages (home, tutorial, reference/index) are committed.

    python scripts/build_docs_site.py
"""

from __future__ import annotations

import functools
import re
import shutil
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SPEC = REPO / "docs" / "spec"
SITE_DOCS = REPO / "website" / "docs"
REFERENCE = SITE_DOCS / "reference"

GITHUB_BLOB = "https://github.com/vow-lang/vow/blob/main"

# Canonical reference pages copied verbatim into website/docs/reference/.
REFERENCE_PAGES = [
    "grammar.md",
    "contracts.md",
    "contracts-methodology.md",
    "cli.md",
    "errors.md",
    "examples.md",
]

# Links in the canonical files are relative to `docs/spec/`. Sibling links
# (`errors.md#...`) already resolve inside the copied set; a `../` prefix always
# escapes it, so those are retargeted at GitHub — external links are not validated
# by `zensical --strict` and stay correct on the published site. The path group
# excludes whitespace so a standard Markdown title (`](../f.md "details")`) isn't
# folded into the path and mistaken for a dead target.
ESCAPING_LINK = re.compile(r'\]\(\.\./([^)#\s]+)(#[^)\s]*)?(\s+"[^"]*")?\)')

# Reference-style link *definitions* (`[label]: ../foo.md`) live on their own
# line rather than inside a `](...)` construct, so they need a second,
# line-anchored pattern (per CommonMark, optionally indented up to 3 spaces)
# rather than folding into ESCAPING_LINK above.
ESCAPING_REF_LINK = re.compile(
    r'^(\s{0,3}\[[^\]]+\]:\s*)\.\./([^\s]+?)(#[^\s]*)?(\s+"[^"]*")?\s*$',
    re.MULTILINE,
)


def _slugify_heading(text: str) -> str:
    """Reproduce GitHub's Markdown heading-to-anchor slug for `text`.

    Verified against `github-slugger` (the de facto reference reimplementation
    of GitHub's own slugifier) for every real heading currently under `docs/`,
    including em-dashes, arrows, colons, parens, numbers, and inline code
    spans. Does not handle `_italic_` underscore-emphasis or in-heading
    Markdown links — neither construct occurs in this repo's headings today.
    """
    return re.sub(r"[^\w -]", "", text.lower(), flags=re.UNICODE).replace(" ", "-")


_FENCE = re.compile(r"^\s{0,3}(```|~~~)")
_ATX_HEADING = re.compile(r"^\s{0,3}#{1,6}\s+(.*)$")
_CLOSING_HASHES = re.compile(r"\s+#+\s*$")


def _heading_anchors(markdown_text: str) -> set[str]:
    """Extract the set of GitHub anchor slugs for every heading in `markdown_text`.

    Skips lines inside fenced code blocks (``` or ~~~) so a shell comment like
    `# build it` isn't mistaken for a heading. Duplicate headings get GitHub's
    `-1`, `-2`, ... suffixing.
    """
    anchors: set[str] = set()
    occurrences: dict[str, int] = {}
    in_fence = False
    for line in markdown_text.splitlines():
        if _FENCE.match(line):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        match = _ATX_HEADING.match(line)
        if not match:
            continue
        heading = _CLOSING_HASHES.sub("", match.group(1)).strip()
        slug = _slugify_heading(heading)
        original_slug = slug
        while slug in occurrences:
            occurrences[original_slug] = occurrences.get(original_slug, 0) + 1
            slug = f"{original_slug}-{occurrences[original_slug]}"
        occurrences[slug] = 0
        anchors.add(slug)
    return anchors


@functools.lru_cache(maxsize=None)
def _heading_anchors_for(target_path: Path) -> frozenset[str]:
    return frozenset(_heading_anchors(target_path.read_text()))


# A fence line is a run of 3+ identical backticks or tildes, optionally
# followed by an info string, indented by at most 3 spaces (CommonMark's
# fence-open/close limit). A deeper indent isn't a fence, and treating it as
# one risks hitting the "unterminated fence" branch below, which protects to
# end of document and silently disables `../` link validation past it.
_FENCE_LINE = re.compile(r"^ {0,3}(`{3,}|~{3,})(.*)$")

# A code span can't cross a paragraph break (CommonMark); this bounds the
# closer search in `_inline_code_span_ranges` at the next blank line. `\r?`
# lets this also match a CRLF-terminated blank line (`\r\n\r\n`), which the
# bare `\n[ \t]*\n` form misses (the `\r` between the two `\n`s isn't in
# `[ \t]*`).
_BLANK_LINE = re.compile(r"\r?\n[ \t]*\r?\n")


def _iter_lines(text: str):
    """Yield each line of `text` (trailing `\\n` kept, if present), splitting
    on `\\n` only -- unlike `str.splitlines()`, which also breaks on `\\v`,
    `\\f` and other separators that would close a fence early."""
    start = 0
    n = len(text)
    while start <= n:
        nl = text.find("\n", start)
        if nl == -1:
            if start < n:
                yield text[start:]
            return
        yield text[start : nl + 1]
        start = nl + 1


def _fenced_block_ranges(text: str) -> list[tuple[int, int]]:
    """Character ranges of fenced code blocks (```` ``` ```` or `~~~`), start-of-open-line to end-of-close-line."""
    ranges: list[tuple[int, int]] = []
    fence_char: str | None = None
    fence_len = 0
    fence_start = 0
    offset = 0
    for line in _iter_lines(text):
        stripped = line.rstrip("\n")
        m = _FENCE_LINE.match(stripped)
        if fence_char is None:
            if m:
                fence_char = m.group(1)[0]
                fence_len = len(m.group(1))
                fence_start = offset
        elif (
            m
            and m.group(1)[0] == fence_char
            and len(m.group(1)) >= fence_len
            and m.group(2).strip() == ""
        ):
            ranges.append((fence_start, offset + len(line)))
            fence_char = None
        offset += len(line)
    if fence_char is not None:
        # Unterminated fence: protect to end of document rather than guess.
        ranges.append((fence_start, len(text)))
    return ranges


def _range_containing(
    pos: int, ranges: list[tuple[int, int]]
) -> tuple[int, int] | None:
    """The (start, end) entry in `ranges` that contains `pos`, or None."""
    for start, end in ranges:
        if start <= pos < end:
            return start, end
    return None


def _inline_code_span_ranges(
    text: str, fenced_ranges: list[tuple[int, int]]
) -> list[tuple[int, int]]:
    """Character ranges of inline code spans (backtick-delimited), outside fenced blocks."""

    def _skip_to(pos: int) -> int:
        hit = _range_containing(pos, fenced_ranges)
        return hit[1] if hit else pos

    ranges: list[tuple[int, int]] = []
    backtick_run = re.compile(r"`+")
    n = len(text)
    i = 0
    while i < n:
        opener = backtick_run.search(text, i)
        if not opener:
            break
        skipped = _skip_to(opener.start())
        if skipped != opener.start():
            i = skipped
            continue
        run_len = opener.end() - opener.start()
        # A code span cannot cross a paragraph (blank-line) boundary, or a
        # fenced block (which also interrupts a paragraph in CommonMark);
        # bound the closer search at whichever comes first. Without the
        # fence bound, a stray unmatched opener could pair with an unrelated
        # backtick run on the far side of (or inside) a fence and swallow
        # everything in between, including a real link.
        blank_line = _BLANK_LINE.search(text, opener.end())
        boundary = blank_line.start() if blank_line else n
        next_fence_start = min(
            (start for start, _ in fenced_ranges if start >= opener.end()),
            default=n,
        )
        boundary = min(boundary, next_fence_start)

        search_pos = opener.end()
        closer = None
        while search_pos < boundary:
            candidate = backtick_run.search(text, search_pos)
            if not candidate or candidate.start() >= boundary:
                break
            if candidate.end() - candidate.start() == run_len:
                closer = candidate
                break
            search_pos = candidate.end()

        if closer:
            ranges.append((opener.start(), closer.end()))
            i = closer.end()
        else:
            # No matching closer before the paragraph boundary: the run is
            # literal text, not a span delimiter. Resume right after it.
            i = opener.end()
    return ranges


def _protected_ranges(text: str) -> list[tuple[int, int]]:
    """Character ranges of fenced blocks and inline code spans, where a
    literal `](...)`-shaped Markdown example must not be treated as a real
    link."""
    fenced = _fenced_block_ranges(text)
    spans = _inline_code_span_ranges(text, fenced)
    return fenced + spans


def _is_protected(pos: int, ranges: list[tuple[int, int]]) -> bool:
    return _range_containing(pos, ranges) is not None


def _resolve_target(target: str, anchor: str, page: str) -> str:
    """Resolve a `../`-escaping target to its GitHub URL, or raise loudly."""
    target_path = REPO / "docs" / target
    if not target_path.exists():
        raise SystemExit(
            f"{page}: link '../{target}' has no target at docs/{target}. "
            "Fix the link in the canonical file."
        )
    if anchor and anchor != "#" and target_path.suffix == ".md":
        fragment = anchor[1:]
        valid_anchors = _heading_anchors_for(target_path)
        if fragment not in valid_anchors:
            raise SystemExit(
                f"{page}: link '../{target}{anchor}' has no heading '{fragment}' "
                f"in docs/{target}. Valid anchors: {sorted(valid_anchors)}"
            )
    return f"{GITHUB_BLOB}/docs/{target}{anchor}"


def _retarget_escaping_links(text: str, page: str) -> str:
    """Point `../`-prefixed links at GitHub, failing loudly on a dead target.

    A literal Markdown-link example inside a fenced code block or inline code
    span (e.g. `` `[guide](../missing.md)` `` shown as prose) is masked first,
    so it is left untouched instead of being treated as a real link.
    """

    protected = _protected_ranges(text)

    def repl(match: re.Match[str]) -> str:
        if _is_protected(match.start(), protected):
            return match.group(0)
        target, anchor, title = (
            match.group(1),
            match.group(2) or "",
            match.group(3) or "",
        )
        url = _resolve_target(target, anchor, page)
        return f"]({url}{title})"

    text = ESCAPING_LINK.sub(repl, text)

    protected = _protected_ranges(text)

    def ref_repl(match: re.Match[str]) -> str:
        if _is_protected(match.start(), protected):
            return match.group(0)
        prefix, target, anchor, title = (
            match.group(1),
            match.group(2),
            match.group(3) or "",
            match.group(4) or "",
        )
        url = _resolve_target(target, anchor, page)
        return f"{prefix}{url}{title}"

    return ESCAPING_REF_LINK.sub(ref_repl, text)


def _reset(path: Path) -> None:
    if path.exists():
        if path.is_dir():
            shutil.rmtree(path)
        else:
            path.unlink()


def main() -> None:
    if not SPEC.is_dir():
        raise SystemExit(f"canonical spec dir not found: {SPEC}")

    # Clean only generated targets so removals in docs/spec propagate, while
    # preserving the hand-written reference/index.md landing page.
    REFERENCE.mkdir(parents=True, exist_ok=True)
    for name in REFERENCE_PAGES:
        _reset(REFERENCE / name)
    _reset(REFERENCE / "schemas")
    _reset(SITE_DOCS / "stdlib.md")

    copied = 0

    for name in REFERENCE_PAGES:
        src = SPEC / name
        if not src.is_file():
            raise SystemExit(f"missing canonical page: {src}")
        (REFERENCE / name).write_text(_retarget_escaping_links(src.read_text(), name))
        copied += 1

    # Standard library reference is a single comprehensive page.
    stdlib_src = SPEC / "stdlib.md"
    if not stdlib_src.is_file():
        raise SystemExit(f"missing canonical page: {stdlib_src}")
    (SITE_DOCS / "stdlib.md").write_text(
        _retarget_escaping_links(stdlib_src.read_text(), "stdlib.md")
    )
    copied += 1

    # JSON schemas referenced by cli.md, served as static assets.
    schemas_src = SPEC / "schemas"
    schemas_dst = REFERENCE / "schemas"
    n_schemas = 0
    if schemas_src.is_dir():
        schemas_dst.mkdir(parents=True, exist_ok=True)
        for sf in sorted(schemas_src.glob("*.json")):
            shutil.copy2(sf, schemas_dst / sf.name)
            n_schemas += 1

    print(
        f"Assembled site source: {copied} reference pages, {n_schemas} schemas "
        f"-> {SITE_DOCS.relative_to(REPO)}"
    )


if __name__ == "__main__":
    main()
