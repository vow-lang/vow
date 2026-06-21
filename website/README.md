# Vow documentation website

The public documentation site, built with [Material for MkDocs](https://squidfunk.github.io/mkdocs-material/).
It has three pillars: a human-first **Tutorial**, the **Language** reference, and the
**Standard Library** reference.

## Single source of truth

The Language and Standard Library pages are **generated** from the canonical
specification in [`../docs/spec/`](../docs/spec) — the same markdown the compiler
embeds into its agent skill (`scripts/generate_help.py`). Do **not** edit the
generated pages; edit the canonical files and re-run the assembly step.

`scripts/build_docs_site.py` copies the curated reference pages into `docs/` and
applies the few link rewrites needed for a clean strict build. Generated paths
(`docs/reference/*.md` except `index.md`, `docs/reference/schemas/`, `docs/stdlib.md`,
`site/`) are gitignored.

What's authored directly here (and tracked):

- `docs/index.md` — home page
- `docs/tutorial/` — the tutorial
- `docs/reference/index.md` — the reference landing page
- `mkdocs.yml` — site config and navigation

## Build / serve locally

Always assemble the generated pages first, then run MkDocs.

### Zero-install (uv)

```sh
python3 scripts/build_docs_site.py
uvx --with mkdocs-material mkdocs serve --config-file website/mkdocs.yml
```

### With pip

```sh
python -m pip install --require-hashes -r website/requirements.txt
python3 scripts/build_docs_site.py
mkdocs serve --config-file website/mkdocs.yml      # live preview at http://127.0.0.1:8000
mkdocs build --strict --config-file website/mkdocs.yml   # one-shot build into website/site
```

`--strict` turns broken links and nav problems into errors — CI uses it, so build with
it locally before pushing.

### Refreshing dependencies

`requirements.txt` is generated and hash-pinned for reproducible deploys. Keep
direct dependencies in `requirements.in`, then refresh the lockfile with:

```sh
uvx --python 3.13 --from pip-tools pip-compile --generate-hashes --strip-extras --output-file website/requirements.txt website/requirements.in
```

## Deployment

`.github/workflows/docs.yml` assembles the source, builds with `--strict`, and deploys
to GitHub Pages on every push to `main` that touches the docs (or via
`workflow_dispatch`). One-time setup: in the repository settings, set
**Pages → Build and deployment → Source = GitHub Actions**, and set the custom
domain to `docs.vow-lang.com` (a `CNAME` file is committed at `docs/CNAME` and copied
into the build). The site publishes to <https://docs.vow-lang.com/>.
