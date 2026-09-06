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


class HeadingAnchorsTest(unittest.TestCase):
    def test_two_headings(self):
        text = "## Heading One\n\nbody\n\n### Heading Two\n"
        self.assertEqual(bds._heading_anchors(text), {"heading-one", "heading-two"})

    def test_fenced_code_block_hash_ignored(self):
        text = "## Real Heading\n\n```\n# build it\n```\n"
        self.assertEqual(bds._heading_anchors(text), {"real-heading"})

    def test_tilde_fenced_code_block_hash_ignored(self):
        text = "## Real Heading\n\n~~~\n# build it\n~~~\n"
        self.assertEqual(bds._heading_anchors(text), {"real-heading"})

    def test_closing_hashes_stripped(self):
        text = "## Heading ##\n"
        self.assertEqual(bds._heading_anchors(text), {"heading"})

    def test_trailing_whitespace_stripped(self):
        text = "## Foo   \n"
        self.assertEqual(bds._heading_anchors(text), {"foo"})

    def test_duplicate_headings_get_numeric_suffix(self):
        text = "## Foo\n\n## Foo\n"
        self.assertEqual(bds._heading_anchors(text), {"foo", "foo-1"})

    def test_duplicate_slug_collision_avoided(self):
        # "Foo" x2 produces "foo" and "foo-1"; a third, separate heading that
        # slugifies directly to "foo-1" must not collide with it.
        text = "## Foo\n\n## Foo\n\n## Foo 1\n"
        self.assertEqual(bds._heading_anchors(text), {"foo", "foo-1", "foo-1-1"})


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
            "See [details](../verifier-discipline.md#the-core-rule).", "grammar.md"
        )
        self.assertEqual(
            out,
            f"See [details]({bds.GITHUB_BLOB}/docs/verifier-discipline.md#the-core-rule).",
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
            'See [details](../verifier-discipline.md#the-core-rule "Verifier discipline").',
            "grammar.md",
        )
        self.assertEqual(
            out,
            f"See [details]({bds.GITHUB_BLOB}/docs/verifier-discipline.md#the-core-rule "
            '"Verifier discipline").',
        )

    def test_dead_target_still_raises(self):
        with self.assertRaises(SystemExit):
            bds._retarget_escaping_links(
                "See [details](../does-not-exist.md).", "grammar.md"
            )

    def test_link_with_valid_fragment_is_rewritten(self):
        out = bds._retarget_escaping_links(
            "See [details](../verifier-discipline.md#the-core-rule).", "grammar.md"
        )
        self.assertEqual(
            out,
            f"See [details]({bds.GITHUB_BLOB}/docs/verifier-discipline.md#the-core-rule).",
        )

    def test_link_with_stale_fragment_raises(self):
        with self.assertRaises(SystemExit):
            bds._retarget_escaping_links(
                "See [details](../verifier-discipline.md#renamed-heading).",
                "grammar.md",
            )

    def test_link_with_punctuation_heading_fragment_is_rewritten(self):
        # Locks in the double-hyphen slugification behavior end-to-end
        # (em-dash between two spaces deletes to two literal hyphens), not
        # just at the pure _slugify_heading layer.
        out = bds._retarget_escaping_links(
            "See [details](../adr/0001-numeric-tower-narrow-ints.md"
            "#0001-numeric-tower--narrow-integer-types).",
            "grammar.md",
        )
        self.assertEqual(
            out,
            f"See [details]({bds.GITHUB_BLOB}/docs/adr/0001-numeric-tower-narrow-ints.md"
            "#0001-numeric-tower--narrow-integer-types).",
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
            "[details]: ../verifier-discipline.md#the-core-rule", "grammar.md"
        )
        self.assertEqual(
            out,
            f"[details]: {bds.GITHUB_BLOB}/docs/verifier-discipline.md#the-core-rule",
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

    def test_reference_style_link_with_valid_fragment_is_rewritten(self):
        out = bds._retarget_escaping_links(
            "[details]: ../verifier-discipline.md#the-core-rule", "grammar.md"
        )
        self.assertEqual(
            out,
            f"[details]: {bds.GITHUB_BLOB}/docs/verifier-discipline.md#the-core-rule",
        )

    def test_reference_style_link_with_stale_fragment_raises(self):
        with self.assertRaises(SystemExit):
            bds._retarget_escaping_links(
                "[details]: ../verifier-discipline.md#renamed-heading", "grammar.md"
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

    def test_stray_backtick_before_fence_does_not_swallow_real_link(self):
        # A stray unmatched backtick opener, with no closer before the fence,
        # must not pair with an unrelated same-length backtick run inside the
        # fence -- that would wrongly protect (and skip rewriting) the real
        # link sitting between the two.
        text = (
            "Prose with `stray backtick and a real link "
            "[details](../verifier-discipline.md) right before a fence.\n"
            "```\n"
            "foo ` bar\n"
            "```\n"
        )
        out = bds._retarget_escaping_links(text, "grammar.md")
        self.assertEqual(
            out,
            (
                "Prose with `stray backtick and a real link "
                f"[details]({bds.GITHUB_BLOB}/docs/verifier-discipline.md) "
                "right before a fence.\n"
                "```\n"
                "foo ` bar\n"
                "```\n"
            ),
        )

    def test_stray_backtick_does_not_swallow_link_through_closing_fence(self):
        # Mirror of the case above: the false closer sits AFTER the fence
        # entirely (the fence itself has no backticks in its content).
        text = (
            "Prose with `stray and a link "
            "[details](../verifier-discipline.md) before fence.\n"
            "```\n"
            "code here, no backticks\n"
            "```\n"
            "After fence text with a closer ` right here.\n"
        )
        out = bds._retarget_escaping_links(text, "grammar.md")
        self.assertEqual(
            out,
            (
                "Prose with `stray and a link "
                f"[details]({bds.GITHUB_BLOB}/docs/verifier-discipline.md) "
                "before fence.\n"
                "```\n"
                "code here, no backticks\n"
                "```\n"
                "After fence text with a closer ` right here.\n"
            ),
        )

    def test_over_indented_pseudo_fence_does_not_suppress_validation(self):
        # CommonMark caps a fence's own indentation at 3 spaces; an 8-space
        # indented ``` is not a fence at all. It must not be treated as an
        # (unterminated) one -- that would silently protect, and skip
        # validating, every `../` link for the rest of the document.
        text = (
            "Some intro text.\n\n"
            "        ```\n"
            "Some further prose.\n\n"
            "See [guide](../missing.md) later in the document.\n"
        )
        with self.assertRaises(SystemExit):
            bds._retarget_escaping_links(text, "grammar.md")

    def test_form_feed_inside_fence_does_not_close_it_early(self):
        # `_fenced_block_ranges` must treat only `\n` as a line break, not
        # every separator `str.splitlines()` recognizes -- a form feed in
        # fenced content must not look like a line boundary that exposes a
        # literal reference-style definition inside the fence.
        text = "```\nlet x = 1;\x0c```\n[details]: ../missing.md\n```\n"
        out = bds._retarget_escaping_links(text, "grammar.md")
        self.assertEqual(out, text)

    def test_crlf_blank_line_bounds_stray_backtick(self):
        # A CRLF blank line must bound the closer search exactly like an LF
        # one does, so a stray backtick in one paragraph can't pair with a
        # real code span's opener in a later paragraph and swallow the link
        # in between.
        text = (
            "This paragraph has a stray ` backtick with no closer.\r\n"
            "\r\n"
            "See [details](../verifier-discipline.md) in the next paragraph, "
            "then a `closer`.\r\n"
        )
        out = bds._retarget_escaping_links(text, "grammar.md")
        self.assertEqual(
            out,
            (
                "This paragraph has a stray ` backtick with no closer.\r\n"
                "\r\n"
                f"See [details]({bds.GITHUB_BLOB}/docs/verifier-discipline.md) "
                "in the next paragraph, then a `closer`.\r\n"
            ),
        )


if __name__ == "__main__":
    unittest.main()
