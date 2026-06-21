#!/usr/bin/env python3
"""Assemble the MkDocs site source from the canonical specification.

The website renders the SAME markdown that lives in `docs/spec/` (and that the
compiler embeds into its agent skill via `generate_help.py`) — it must never fork
those files. This script copies the curated reference pages into `website/docs/`
and applies the few link rewrites needed for the site to build cleanly under
`mkdocs build --strict`.

Run this before `mkdocs build` / `mkdocs serve`. The generated pages are gitignored;
the hand-written pages (home, tutorial, reference/index) are committed.

    python scripts/build_docs_site.py
"""
from __future__ import annotations

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

# Targeted link rewrites: relative links in the canonical files that point outside
# the copied set must be retargeted to absolute GitHub URLs (external links are not
# validated by mkdocs --strict and stay correct on the site).
LINK_REWRITES = {
    "grammar.md": [
        (
            "](../adr/0001-numeric-tower-narrow-ints.md)",
            f"]({GITHUB_BLOB}/docs/adr/0001-numeric-tower-narrow-ints.md)",
        ),
    ],
}


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
        text = src.read_text()
        for old, new in LINK_REWRITES.get(name, []):
            if old not in text:
                raise SystemExit(
                    f"link-rewrite target not found in {name!r}: {old!r}\n"
                    "The canonical file changed; update LINK_REWRITES."
                )
            text = text.replace(old, new)
        (REFERENCE / name).write_text(text)
        copied += 1

    # Standard library reference is a single comprehensive page.
    stdlib_src = SPEC / "stdlib.md"
    if not stdlib_src.is_file():
        raise SystemExit(f"missing canonical page: {stdlib_src}")
    (SITE_DOCS / "stdlib.md").write_text(stdlib_src.read_text())
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
