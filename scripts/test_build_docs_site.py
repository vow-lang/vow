#!/usr/bin/env python3
"""Behavior tests for `_retarget_escaping_links` in scripts/build_docs_site.py.

This is the function that decides whether a `../`-escaping link in a canonical
spec page publishes correctly or takes down the strict docs build. The cases
here guard the two failure directions: a link that should rewrite cleanly must
not raise, and a link to a genuinely missing target must still raise loudly.
"""

import unittest

import build_docs_site as bds


class SlugifyHeadingTest(unittest.TestCase):
    def test_plain_ascii(self):
        self.assertEqual(bds._slugify_heading("The core rule"), "the-core-rule")

    def test_numeric_prefix_and_em_dash(self):
        self.assertEqual(
            bds._slugify_heading("0001. Numeric tower — narrow integer types"),
            "0001-numeric-tower--narrow-integer-types",
        )

    def test_colon(self):
        self.assertEqual(
            bds._slugify_heading("Verifier Discipline: Safe vs Unsafe Adaptive Retry"),
            "verifier-discipline-safe-vs-unsafe-adaptive-retry",
        )

    def test_inline_code_and_em_dash(self):
        self.assertEqual(
            bds._slugify_heading("5. Command Loop — EOF-Safe `stdin_read_line`"),
            "5-command-loop--eof-safe-stdin_read_line",
        )

    def test_parens_and_em_dash(self):
        self.assertEqual(
            bds._slugify_heading(
                "2. Output-range postcondition (the weak default — use sparingly)"
            ),
            "2-output-range-postcondition-the-weak-default--use-sparingly",
        )

    def test_arrow_and_em_dash(self):
        self.assertEqual(
            bds._slugify_heading("2. CEGIS Broken → Fixed — The Core Workflow"),
            "2-cegis-broken--fixed--the-core-workflow",
        )

    def test_asterisk_emphasis(self):
        self.assertEqual(
            bds._slugify_heading("WS-1 — Make verification *honest* (the C emitter)"),
            "ws-1--make-verification-honest-the-c-emitter",
        )


class RetargetEscapingLinksTest(unittest.TestCase):
    def test_plain_link_is_rewritten(self):
        out = bds._retarget_escaping_links(
            "See [details](../verifier-discipline.md).", "grammar.md"
        )
        self.assertEqual(
            out,
            f"See [details]({bds.GITHUB_BLOB}/docs/verifier-discipline.md).",
        )

    def test_link_with_fragment_is_rewritten(self):
        out = bds._retarget_escaping_links(
            "See [details](../verifier-discipline.md#some-heading).", "grammar.md"
        )
        self.assertEqual(
            out,
            f"See [details]({bds.GITHUB_BLOB}/docs/verifier-discipline.md#some-heading).",
        )

    def test_link_with_title_is_rewritten_not_rejected(self):
        # Standard Markdown title syntax: `](../f.md "title")`. Before the
        # title group was split out, the title text was folded into the path
        # capture, the existence check saw a nonexistent path, and a valid
        # link incorrectly raised SystemExit.
        out = bds._retarget_escaping_links(
            'See [details](../verifier-discipline.md "Verifier discipline").',
            "grammar.md",
        )
        self.assertEqual(
            out,
            f'See [details]({bds.GITHUB_BLOB}/docs/verifier-discipline.md "Verifier discipline").',
        )

    def test_link_with_fragment_and_title_is_rewritten(self):
        out = bds._retarget_escaping_links(
            'See [details](../verifier-discipline.md#some-heading "Verifier discipline").',
            "grammar.md",
        )
        self.assertEqual(
            out,
            f"See [details]({bds.GITHUB_BLOB}/docs/verifier-discipline.md#some-heading "
            '"Verifier discipline").',
        )

    def test_dead_target_still_raises(self):
        with self.assertRaises(SystemExit):
            bds._retarget_escaping_links(
                "See [details](../does-not-exist.md).", "grammar.md"
            )

    def test_sibling_link_is_untouched(self):
        # No `../` prefix, so it already resolves inside the copied set and
        # must not be touched by the escaping-link rewrite.
        out = bds._retarget_escaping_links(
            "See [errors](errors.md#e001).", "grammar.md"
        )
        self.assertEqual(out, "See [errors](errors.md#e001).")

    def test_reference_style_link_is_rewritten(self):
        out = bds._retarget_escaping_links(
            "[details]: ../verifier-discipline.md", "grammar.md"
        )
        self.assertEqual(
            out,
            f"[details]: {bds.GITHUB_BLOB}/docs/verifier-discipline.md",
        )

    def test_reference_style_link_with_fragment_is_rewritten(self):
        out = bds._retarget_escaping_links(
            "[details]: ../verifier-discipline.md#some-heading", "grammar.md"
        )
        self.assertEqual(
            out,
            f"[details]: {bds.GITHUB_BLOB}/docs/verifier-discipline.md#some-heading",
        )

    def test_reference_style_link_with_title_is_rewritten(self):
        out = bds._retarget_escaping_links(
            '[details]: ../verifier-discipline.md "Verifier discipline"',
            "grammar.md",
        )
        self.assertEqual(
            out,
            f"[details]: {bds.GITHUB_BLOB}/docs/verifier-discipline.md "
            '"Verifier discipline"',
        )

    def test_reference_style_dead_target_still_raises(self):
        with self.assertRaises(SystemExit):
            bds._retarget_escaping_links(
                "[details]: ../does-not-exist.md", "grammar.md"
            )

    def test_reference_style_sibling_link_is_untouched(self):
        out = bds._retarget_escaping_links("[errors]: errors.md#e001", "grammar.md")
        self.assertEqual(out, "[errors]: errors.md#e001")


if __name__ == "__main__":
    unittest.main()
