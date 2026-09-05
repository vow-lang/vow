#!/usr/bin/env python3
"""Behavior tests for `_retarget_escaping_links` in scripts/build_docs_site.py.

This is the function that decides whether a `../`-escaping link in a canonical
spec page publishes correctly or takes down the strict docs build. The cases
here guard the two failure directions: a link that should rewrite cleanly must
not raise, and a link to a genuinely missing target must still raise loudly.
"""

import unittest

import build_docs_site as bds


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


class MaskedMarkdownStructureTest(unittest.TestCase):
    def test_fenced_block_protects_dead_target_example(self):
        text = "Example:\n\n```\nSee [guide](../missing.md) for details.\n```\n"
        out = bds._retarget_escaping_links(text, "grammar.md")
        self.assertEqual(out, text)

    def test_fenced_block_protects_existing_target_example(self):
        text = (
            "Example:\n\n"
            "```\n"
            "See [details](../verifier-discipline.md) for details.\n"
            "```\n"
        )
        out = bds._retarget_escaping_links(text, "grammar.md")
        self.assertEqual(out, text)

    def test_tilde_fenced_block_protects_dead_target_example(self):
        text = "Example:\n\n~~~\nSee [guide](../missing.md) for details.\n~~~\n"
        out = bds._retarget_escaping_links(text, "grammar.md")
        self.assertEqual(out, text)

    def test_fenced_block_protects_reference_style_definition(self):
        text = "Example:\n\n```\n[details]: ../missing.md\n```\n"
        out = bds._retarget_escaping_links(text, "grammar.md")
        self.assertEqual(out, text)

    def test_inline_code_span_protects_issue_example(self):
        text = "See `[guide](../missing.md)` for the syntax."
        out = bds._retarget_escaping_links(text, "grammar.md")
        self.assertEqual(out, text)

    def test_double_backtick_span_with_literal_backtick_is_protected(self):
        text = "See ``[guide](../missing.md)` `` for the syntax."
        out = bds._retarget_escaping_links(text, "grammar.md")
        self.assertEqual(out, text)

    def test_unmatched_backtick_does_not_cascade_past_blank_line(self):
        text = (
            "This paragraph has a stray ` backtick with no closer.\n"
            "\n"
            "See [details](../verifier-discipline.md) in the next paragraph.\n"
        )
        out = bds._retarget_escaping_links(text, "grammar.md")
        self.assertEqual(
            out,
            (
                "This paragraph has a stray ` backtick with no closer.\n"
                "\n"
                f"See [details]({bds.GITHUB_BLOB}/docs/verifier-discipline.md) "
                "in the next paragraph.\n"
            ),
        )

    def test_code_span_before_real_link_does_not_block_rewrite(self):
        text = "Use `foo()` and see [details](../verifier-discipline.md)."
        out = bds._retarget_escaping_links(text, "grammar.md")
        self.assertEqual(
            out,
            f"Use `foo()` and see [details]({bds.GITHUB_BLOB}/docs/verifier-discipline.md).",
        )


if __name__ == "__main__":
    unittest.main()
