#!/usr/bin/env python3
"""Behavior tests for scripts/verifier_runtime.py.

The two directions fail differently and must never be conflated: a SOUNDNESS
failure is a false proof, a PRECISION failure is honest-verdict-misleading-
evidence. These tests pin that distinction and the not-applicable bookkeeping
that keeps a sweep from looking like it measured more than it did.
"""

import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock
from unittest.mock import patch

import verifier_runtime


class DirectiveTest(unittest.TestCase):
    def test_skip_directive_is_recognized(self):
        m = verifier_runtime.DIRECTIVE_SKIP.search('// TEST: skip "flaky"\n')

        self.assertEqual("flaky", m.group(1))

    def test_known_soundness_gap_directive_is_recognized(self):
        # verify_eval.py's KNOWN_GAP convention: a documented gap is reported
        # loudly but must not fail the run, or the sweep is red forever and
        # nobody reads it.
        m = verifier_runtime.DIRECTIVE_KNOWN_GAP.search(
            "// TEST: known-soundness-gap #585\n"
        )

        self.assertEqual("#585", m.group(1))

    def test_known_gap_without_an_issue_still_matches(self):
        m = verifier_runtime.DIRECTIVE_KNOWN_GAP.search(
            "// TEST: known-soundness-gap\n"
        )

        self.assertIsNotNone(m)


class CorpusTest(unittest.TestCase):
    def test_default_roots_exist(self):
        for rel in ("tests/verify", "tests/verify-fail", "examples"):
            with self.subTest(root=rel):
                self.assertTrue((verifier_runtime.REPO_ROOT / rel).is_dir())

    def test_repo_root_is_the_worktree(self):
        self.assertTrue((verifier_runtime.REPO_ROOT / "CLAUDE.md").exists())


class ViolationParsingTest(unittest.TestCase):
    def test_memory_limit_matches_the_repo_convention(self):
        # `ulimit -v 2000000` is the repo-wide rule for self-compiled binaries;
        # expressed in bytes here because setrlimit takes bytes.
        self.assertEqual(2_000_000 * 1024, verifier_runtime.SELF_MEM_LIMIT)


def _verify_failed(*replay_values):
    return {
        "status": "VerifyFailed",
        "counterexamples": [
            {"function": "f", "vow_id": i, "replay": r, "replay_reason": "because"}
            for i, r in enumerate(replay_values)
        ],
    }


class PrecisionTest(unittest.TestCase):
    # `confirmed` is the only evidence that the model's counterexample
    # actually reproduces at runtime; `aborted` is an honest runtime failure
    # through some other mechanism, not that evidence.
    @patch("verifier_runtime.run_json")
    def test_abort_only_is_inconclusive_not_ok(self, mock_run_json):
        mock_run_json.return_value = (_verify_failed("aborted"), None)

        result = verifier_runtime.check_precision(Path("f.vow"), "verifier", 30)

        self.assertEqual("skipped", result["verdict"])

    @patch("verifier_runtime.run_json")
    def test_confirmed_alongside_aborted_is_still_ok(self, mock_run_json):
        mock_run_json.return_value = (_verify_failed("confirmed", "aborted"), None)

        result = verifier_runtime.check_precision(Path("f.vow"), "verifier", 30)

        self.assertEqual("ok", result["verdict"])
        self.assertIn("aborted", result["detail"])

    @patch("verifier_runtime.run_json")
    def test_diverged_is_still_a_precision_finding(self, mock_run_json):
        mock_run_json.return_value = (_verify_failed("diverged", "aborted"), None)

        result = verifier_runtime.check_precision(Path("f.vow"), "verifier", 30)

        self.assertEqual("PRECISION", result["verdict"])


class DebugRunTest(unittest.TestCase):
    """A hung binary is not evidence that the verifier was right."""

    def run_with(self, exc_or_proc):
        def fake_run(argv, **kwargs):
            if isinstance(exc_or_proc, Exception):
                raise exc_or_proc
            return exc_or_proc

        with mock.patch.object(verifier_runtime.subprocess, "run", fake_run):
            return verifier_runtime.run_debug_binary(Path("/nonexistent"), 3)

    def test_a_timeout_is_reported_as_an_error(self):
        violation, err = self.run_with(
            verifier_runtime.subprocess.TimeoutExpired(cmd="x", timeout=3)
        )

        self.assertIsNone(violation)
        self.assertIn("timed out", err)

    def test_a_clean_run_is_not_an_error(self):
        violation, err = self.run_with(SimpleNamespace(stderr="", returncode=0))

        self.assertIsNone(violation)
        self.assertIsNone(err)

    def test_a_violation_is_parsed(self):
        line = '{"error":"VowViolation","vow_id":1,"blame":"Callee"}'
        violation, err = self.run_with(SimpleNamespace(stderr=line, returncode=1))

        self.assertEqual("VowViolation", violation["error"])
        self.assertIsNone(err)


if __name__ == "__main__":
    unittest.main()
