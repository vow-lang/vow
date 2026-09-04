# Plan: issue #1200 — verify `mkdocs.yml`'s `anchors: warn` setting under Zensical

## 1. Problem restated

`website/mkdocs.yml` (lines 14-19) sets `validation.links.anchors: warn`, with a comment
explaining that it exists because MkDocs reports a missing anchor at INFO severity by
default, which would let a broken cross-reference pass `--strict` CI silently. The site now
builds with Zensical instead of MkDocs (PR #1179), and Zensical's own default anchor
severity may already be `warn` (or otherwise already strict-fatal), making this setting and
its MkDocs-specific comment vestigial. Issue #1200 asks for an empirical differential build
test — not a guess — before touching the setting, because silently weakening a
still-load-bearing anchor-strictness gate would be worse than leaving stale prose in place.

## 2. Empirical verification (done during planning, evidence below)

This is a config-only issue with no unit to red-green, so this section replaces the usual
"TDD slices" with the actual experiment run and its results. The implementation stage does
not need to re-derive the answer — it needs to re-run the same commands as a pre-merge sanity
check (config drift is possible between planning and implementation) and then make the edit.

**Setup** (fresh `$TMPDIR`, exact CI-pinned dependency set):

```sh
uv venv .venv && source .venv/bin/activate
uv pip install --require-hashes -r website/requirements.txt   # zensical==0.0.57, pinned lockfile
```

**Toy-fixture test** (isolates the setting from the real site's nav/theme config): built three
minimal two-page sites, identical except for `mkdocs.yml`'s `validation` block, each containing
one page with a link to a heading that does not exist in the target page:

| `validation.links.anchors` | `zensical build --strict` | exit code |
|---|---|---|
| `warn` (current setting) | `Warning: anchor does not exist` → `Aborted because --strict flag is set` | 1 |
| *(key absent entirely)* | identical warning, identical abort | 1 |
| `ignore` | `No issues found` → `Build finished` | 0 |

The `ignore` row proves Zensical *does* honor this key (it is not dead/unparsed
configuration) — but the `warn` row is byte-for-byte identical to the no-key row. Zensical's
built-in default anchor severity is already `warn`, unlike MkDocs's `info` default that
motivated adding the setting in #1178. So `anchors: warn` is not inert — it is **redundant
with Zensical's own default**, and `--strict` fails the build on a missing anchor either way.

**Real-site test** (confirms the toy result isn't an artifact of the minimal fixture, i.e. that
nothing in the actual nav/theme/markdown-extensions config changes this):

```sh
python3 scripts/build_docs_site.py                              # assemble generated pages
zensical build --strict --config-file website/mkdocs.yml        # baseline: exit 0
# then, with the validation: block deleted from website/mkdocs.yml:
zensical build --strict --config-file website/mkdocs.yml        # exit 0 (unchanged)
# then, with a deliberately broken anchor appended to website/docs/index.md
# (`[broken](tutorial/index.md#totally-fake-heading-xyz)`), setting still removed:
zensical build --strict --config-file website/mkdocs.yml        # exit 1, "anchor does not exist"
```

All three real-site runs matched the toy-fixture predictions exactly. Removing the setting
does not weaken the strict gate: a broken anchor still fails the build with the setting gone.

**Repo-wide reference check:** `git grep -n "anchors" -- website scripts .github docs
README.md` finds no other reference to `validation.links.anchors` or this setting anywhere
in the repo (`website/README.md`, `docs.yml`, and `scripts/build_docs_site.py` do not
mention it). Nothing else needs updating in lockstep.

**Conclusion:** the setting is real (Zensical parses and would honor a different value) but
redundant under the current default, and its comment is factually wrong for Zensical (it
describes MkDocs's INFO default, which no longer applies). Remove both.

## 3. Judgment call and decision (per operating contract #1)

Two options were considered:

- **A — Remove the `validation:` block and its comment entirely.** Plain reading of the
  issue's own framing ("possibly the setting itself may be vestigial"); matches
  `CLAUDE.md`'s "no speculative defenses" and "surgical changes" guidance, since keeping a
  no-op setting as a hedge against a hypothetical future Zensical default change is exactly
  the kind of premature defensiveness the project's guidelines reject.
- **B — Keep `anchors: warn` but rewrite the comment to be Zensical-accurate** (i.e., pin the
  severity explicitly in case a future Zensical release changes its default).

**Decision: A (remove).** Rationale to carry into the PR body: the setting adds a config
line that does nothing today, and pinning against a hypothetical future upstream default
change is speculative — if Zensical ever weakens its own default, that regression should be
caught by the differential test method above being re-run (or a future dedicated regression
test, see §6), not by a silently-obsolete pin nobody will think to revisit. This mirrors why
#1200 itself was deferred rather than guessed at: config correctness here should be
re-verified empirically when it matters, not maintained defensively.

## 4. Files to touch

- `website/mkdocs.yml` — delete lines 14-20 (the explanatory comment, the `validation:`
  block, and the resulting blank line so exactly one blank line remains between `docs_dir:
  docs` and `theme:`).

No other file references this setting (confirmed by `git grep`, §2). No `crates/`,
`compiler/`, or `docs/spec/*.md` changes apply — this is a website build-tooling config file,
not compiler syntax/semantics/CLI surface.

## 5. TDD slices

Not applicable in the usual sense (no function/unit to red-green) — see §2 for the
verification already performed and the exact commands to reproduce. The implementation
stage's single slice is:

1. Delete `website/mkdocs.yml` lines 14-20 as described in §4.
2. Re-run the real-site differential build locally (baseline pass, then inject a broken
   anchor and confirm it still fails) using the exact commands in §2, to catch any config
   drift introduced between planning and implementation.
3. Confirm `git grep -n "anchors" -- website scripts .github docs README.md` still shows
   nothing else referencing the removed setting.

## 6. Verification surface

N/A — this change touches no contracts, codegen, IR, or C model. ESBMC is not involved.
`tests/run/` and `examples/` fixtures are for the Vow compiler pipeline and do not apply to
a docs-site YAML config change; nothing there needs to grow.

No automated regression test is being added in this PR. `scripts/test_build_docs_site.py`
(run in `ci.yml`'s `build-and-test` job) only unit-tests the `../`-link rewrite helper in
`scripts/build_docs_site.py`; that job does not install `zensical` at all. The actual
`zensical build --strict` step lives in the separate `.github/workflows/docs.yml`, which has
no fixture-injection hook today. Wiring either job to assert "a broken anchor fails the
strict build" would be a CI-infrastructure change, not a config cleanup, and is out of scope
here — see §7 for a follow-up recommendation.

## 7. Risk areas

- **Binary fixed point / `compiler/` codegen ordering / `BTreeMap` vs `HashMap` / stack-slot
  layout:** N/A — no compiler code touched.
- **`parse → print → parse` idempotency:** N/A — no Vow syntax/AST touched.
- **`cargo clippy --all -- -D warnings`:** N/A — no Rust code touched.
- **Actual risk:** the only way this change could be wrong is if the real Zensical build
  (not the toy fixture) behaves differently once the generated reference pages
  (`scripts/build_docs_site.py` output) are in play, or if some other doc page relies on a
  currently-passing-but-fragile anchor that only survives because of ordering quirks. This
  was directly tested in §2's real-site run (assembled the actual generated pages, built
  with `--strict`, then re-built after removing the setting) with no change in outcome — risk
  is low and empirically closed, not just argued.
- **Docs CI job selection:** confirm the PR only touches paths matched by `docs.yml`'s path
  filter (`docs/**`, `website/**`, `scripts/build_docs_site.py`, `.github/workflows/docs.yml`)
  so the Docs workflow actually runs on the PR and re-validates the strict build in CI, not
  just locally.

## 8. Out of scope

- **Adding a zensical-based CI regression test** that asserts a broken anchor fails the
  strict build (e.g., a fixture page under a test-only Zensical config, wired into either
  `ci.yml`'s `build-and-test` job or `docs.yml`). This would be valuable defense-in-depth
  against a future Zensical default change re-introducing silent anchor failures, but it
  requires adding `zensical`/`website/requirements.txt` as a dependency to a job that
  currently has none, or adding a fixture-injection step to `docs.yml`. That is CI
  infrastructure work, not the config cleanup this issue asks for. Left as a suggestion for a
  future issue, not bundled here.
- **Any other `website/mkdocs.yml` cleanup** (theme options, nav structure, markdown
  extensions) — none of it is related to the anchors question and bundling it would violate
  the "many small changes beat one large change" guideline.
- **Rewriting the comment instead of deleting it (Option B from §3)** — deliberately rejected,
  see §3.

## 9. Proposed PR title

`chore(docs): drop redundant anchor validation setting from mkdocs.yml`

(lower-case subject, no trailing period, `chore` type, well under the ~92-character budget
once GitHub appends ` (#N)`.) PR body should cite the empirical differential-build evidence
from §2 (toy fixture + real-site rebuild + broken-anchor injection all passing/failing
identically with and without the setting) and the judgment call from §3, per operating
contract #1, since a human reviewer may want to override the "remove" decision in favor of
"keep with corrected comment."
