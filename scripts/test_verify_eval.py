#!/usr/bin/env python3
"""Behavior tests for scripts/verify_eval.py."""

import io
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import verify_eval


class ClassifyCounterexamplesTest(unittest.TestCase):
    def test_missing_expected_counterexample_is_a_status_mismatch(self):
        exp = verify_eval.Expect("tests/verify-fail/two_failures.vow", "VerifyFailed")
        exp.cex = [
            {"fn": "first", "blame": "Caller", "vow_id": 1},
            {"fn": "second", "blame": "Callee", "vow_id": 2},
        ]
        verify_json = {
            "status": "VerifyFailed",
            "counterexamples": [
                {"function": "first", "blame": "Caller", "vow_id": 1},
            ],
        }

        verdict, detail = verify_eval.classify(exp, verify_json, verifier="/unused/vow")

        self.assertEqual(verify_eval.STATUS, verdict)
        self.assertIn("no counterexample matched", detail)

    def test_wrong_function_counterexample_is_a_status_mismatch(self):
        exp = verify_eval.Expect("tests/verify-fail/wrong_function.vow", "VerifyFailed")
        exp.cex = [{"fn": "expected", "blame": "Caller", "vow_id": 1}]
        verify_json = {
            "status": "VerifyFailed",
            "counterexamples": [
                {"function": "other", "blame": "Callee", "vow_id": 1},
            ],
        }

        verdict, detail = verify_eval.classify(exp, verify_json, verifier="/unused/vow")

        self.assertEqual(verify_eval.STATUS, verdict)
        self.assertIn("no counterexample matched", detail)

    def test_wrong_counterexample_blame_stays_a_blame_regression(self):
        exp = verify_eval.Expect("tests/verify-fail/wrong_blame.vow", "VerifyFailed")
        exp.cex = [{"fn": "f", "blame": "Caller", "vow_id": 7}]
        verify_json = {
            "status": "VerifyFailed",
            "counterexamples": [
                {"function": "f", "blame": "Callee", "vow_id": 7},
            ],
        }

        verdict, detail = verify_eval.classify(exp, verify_json, verifier="/unused/vow")

        self.assertEqual(verify_eval.BLAME, verdict)
        self.assertIn("blame want=Caller", detail)

    def test_wrong_counterexample_vow_id_stays_a_blame_regression(self):
        exp = verify_eval.Expect("tests/verify-fail/wrong_vow_id.vow", "VerifyFailed")
        exp.cex = [{"fn": "f", "blame": "Callee", "vow_id": 7}]
        verify_json = {
            "status": "VerifyFailed",
            "counterexamples": [
                {"function": "f", "blame": "Callee", "vow_id": 8},
            ],
        }

        verdict, detail = verify_eval.classify(exp, verify_json, verifier="/unused/vow")

        self.assertEqual(verify_eval.BLAME, verdict)
        self.assertIn("vow_id want=7", detail)


class ParseDirectivesKnownGapTest(unittest.TestCase):
    def test_known_soundness_gap_is_accepted_under_tests_verify(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            old_repo_root = verify_eval.REPO_ROOT
            verify_eval.REPO_ROOT = str(root)
            try:
                path = root / "tests" / "verify" / "gap.vow"
                path.parent.mkdir(parents=True)
                path.write_text(
                    '// TEST: known-soundness-gap "documented false accept" #123\n',
                    encoding="utf-8",
                )

                exp = verify_eval.parse_directives(str(path), "Verified")

                self.assertEqual("documented false accept", exp.known_gap)
                self.assertEqual("123", exp.known_gap_issue)
            finally:
                verify_eval.REPO_ROOT = old_repo_root

    def test_known_soundness_gap_is_rejected_outside_tests_verify(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            old_repo_root = verify_eval.REPO_ROOT
            verify_eval.REPO_ROOT = str(root)
            try:
                for subdir, status in (
                    ("verify-fail", "VerifyFailed"),
                    ("verify-skip", "Skipped"),
                ):
                    path = root / "tests" / subdir / "gap.vow"
                    path.parent.mkdir(parents=True)
                    path.write_text(
                        '// TEST: known-soundness-gap "documented false accept" #123\n',
                        encoding="utf-8",
                    )

                    with self.subTest(subdir=subdir):
                        with self.assertRaisesRegex(
                            ValueError, "known-soundness-gap.*tests/verify"
                        ):
                            verify_eval.parse_directives(str(path), status)
            finally:
                verify_eval.REPO_ROOT = old_repo_root


class ParseDirectivesBlameCasingTest(unittest.TestCase):
    def parse(self, directive):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "tests" / "verify-fail" / "b.vow"
            path.parent.mkdir(parents=True)
            path.write_text(f"// TEST: {directive}\n", encoding="utf-8")
            return verify_eval.parse_directives(str(path), "VerifyFailed")

    def test_canonical_blame_is_accepted(self):
        exp = self.parse('cex fn="f" blame=None vow_id=1')
        self.assertEqual("None", exp.cex[0]["blame"])

    def test_lowercase_blame_is_rejected(self):
        for directive in (
            "counterexample-blame callee",
            'cex fn="f" blame=caller vow_id=1',
        ):
            with self.subTest(directive=directive):
                with self.assertRaisesRegex(ValueError, "unknown counterexample blame"):
                    self.parse(directive)


class CorpusCountsDocsTest(unittest.TestCase):
    def test_docs_counts_match_the_corpus(self):
        self.assertEqual(0, verify_eval.sync_docs(check=True))

    def test_render_lists_every_category_by_descending_count(self):
        block = verify_eval.render_corpus_counts(3, {"bounds": 2, "overflow": 1})

        rows = [line for line in block.splitlines() if line.startswith("| ")][2:]
        self.assertEqual(len(verify_eval.CATEGORIES), len(rows))
        self.assertEqual("| bounds | 2 |", rows[0])
        self.assertEqual("| overflow | 1 |", rows[1])
        self.assertIn("3 programs", block)

    def test_render_states_full_coverage_only_when_every_category_has_programs(self):
        full = {name: 1 for name in verify_eval.CATEGORIES}
        block = verify_eval.render_corpus_counts(len(full), full)
        self.assertIn(
            f"All {len(verify_eval.CATEGORIES)} categories are represented", block
        )

        partial = dict(full)
        missing = min(verify_eval.CATEGORIES)
        del partial[missing]
        block = verify_eval.render_corpus_counts(len(partial), partial)
        self.assertNotIn("categories are represented", block)
        self.assertIn(f"no program yet for: {missing}", block)
        self.assertIn(f"| {missing} | 0 |", block)

    def test_conflicting_docs_flags_are_rejected(self):
        argv = ["verify_eval.py", "--check-docs", "--write-docs"]
        with (
            mock.patch.object(sys, "argv", argv),
            mock.patch.object(sys, "stderr", io.StringIO()) as err,
            self.assertRaises(SystemExit) as ctx,
        ):
            verify_eval.main()
        self.assertEqual(2, ctx.exception.code)
        self.assertIn("mutually exclusive", err.getvalue())

    def test_splice_replaces_only_the_marked_block(self):
        block = verify_eval.render_corpus_counts(1, {"bounds": 1})
        text = (
            f"before\n{verify_eval.DOCS_START}\nstale\n{verify_eval.DOCS_END}\nafter\n"
        )

        updated = verify_eval.splice_corpus_counts(text, block)

        self.assertTrue(updated.startswith("before\n"))
        self.assertTrue(updated.endswith("\nafter\n"))
        self.assertNotIn("stale", updated)

    def test_splice_without_markers_fails_closed(self):
        with self.assertRaisesRegex(ValueError, "markers"):
            verify_eval.splice_corpus_counts("no markers here", "x")


if __name__ == "__main__":
    unittest.main()
