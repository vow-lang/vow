#!/usr/bin/env python3
"""Behavior tests for scripts/verify_eval.py."""

import tempfile
import unittest
from pathlib import Path

import verify_eval


class ClassifyCounterexamplesTest(unittest.TestCase):
    def test_missing_expected_counterexample_is_a_status_mismatch(self):
        exp = verify_eval.Expect("tests/verify-fail/two_failures.vow", "VerifyFailed")
        exp.cex = [
            {"fn": "first", "blame": "caller", "vow_id": 1},
            {"fn": "second", "blame": "callee", "vow_id": 2},
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
        exp = verify_eval.Expect(
            "tests/verify-fail/wrong_function.vow", "VerifyFailed"
        )
        exp.cex = [{"fn": "expected", "blame": "caller", "vow_id": 1}]
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
        exp.cex = [{"fn": "f", "blame": "caller", "vow_id": 7}]
        verify_json = {
            "status": "VerifyFailed",
            "counterexamples": [
                {"function": "f", "blame": "Callee", "vow_id": 7},
            ],
        }

        verdict, detail = verify_eval.classify(exp, verify_json, verifier="/unused/vow")

        self.assertEqual(verify_eval.BLAME, verdict)
        self.assertIn("blame want=caller", detail)

    def test_wrong_counterexample_vow_id_stays_a_blame_regression(self):
        exp = verify_eval.Expect("tests/verify-fail/wrong_vow_id.vow", "VerifyFailed")
        exp.cex = [{"fn": "f", "blame": "callee", "vow_id": 7}]
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


if __name__ == "__main__":
    unittest.main()
