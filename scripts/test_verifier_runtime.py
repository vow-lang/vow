#!/usr/bin/env python3
"""Behavior tests for scripts/verifier_runtime.py.

The two directions fail differently and must never be conflated: a SOUNDNESS
failure is a false proof, a PRECISION failure is honest-verdict-misleading-
evidence. These tests pin that distinction and the not-applicable bookkeeping
that keeps a sweep from looking like it measured more than it did.
"""

import unittest
from pathlib import Path

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


if __name__ == "__main__":
    unittest.main()
