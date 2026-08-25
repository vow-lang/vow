#!/usr/bin/env python3
"""Behavior tests for scripts/pair_review.py.

The confirmation gate is the reason this harness is trustworthy, so the tests
concentrate on it: an unconfirmed claim must never be counted as a finding, and
a pair must never be reported as reviewed when it was skipped or truncated.
"""

import hashlib
import tempfile
import unittest
from pathlib import Path

import pair_review


class PairSpecTest(unittest.TestCase):
    def test_every_declared_pair_exists_on_disk(self):
        # A typo'd path would silently review nothing.
        for name, (rust_paths, self_path) in pair_review.PAIRS.items():
            with self.subTest(pair=name):
                self.assertTrue(
                    (pair_review.REPO_ROOT / self_path).exists(), self_path
                )
                for spec in rust_paths:
                    self.assertTrue(
                        (pair_review.REPO_ROOT / spec).exists(), spec
                    )

    def test_pair_hash_changes_when_self_hosted_side_changes(self):
        rust_paths, self_path = pair_review.PAIRS["lexer"]
        before = pair_review.hash_pair(rust_paths, self_path)
        p = pair_review.REPO_ROOT / self_path
        original = p.read_bytes()
        try:
            p.write_bytes(original + b"\n// touched\n")
            after = pair_review.hash_pair(rust_paths, self_path)
        finally:
            p.write_bytes(original)

        self.assertNotEqual(before, after)

    def test_pair_hash_is_stable_across_calls(self):
        rust_paths, self_path = pair_review.PAIRS["c_emitter"]

        self.assertEqual(
            pair_review.hash_pair(rust_paths, self_path),
            pair_review.hash_pair(rust_paths, self_path),
        )

    def test_directory_pair_hashes_every_rust_file(self):
        # `parser` and `lower` are directories on the Rust side; a hash that
        # only covered mod.rs would miss a change in expr.rs.
        rust_paths, self_path = pair_review.PAIRS["parser"]
        target = pair_review.REPO_ROOT / "vow-syntax/src/parser/expr.rs"
        before = pair_review.hash_pair(rust_paths, self_path)
        original = target.read_bytes()
        try:
            target.write_bytes(original + b"\n// touched\n")
            after = pair_review.hash_pair(rust_paths, self_path)
        finally:
            target.write_bytes(original)

        self.assertNotEqual(before, after)


class ReadPairTest(unittest.TestCase):
    def test_truncation_is_flagged_and_marked_in_the_text(self):
        # A model that does not know it saw half a file will reason
        # confidently about the half it never got.
        body, truncated = pair_review.read_pair(
            ["vow-verify/src/c_emitter.rs"], "compiler/c_emitter.vow", 1000
        )

        self.assertTrue(truncated)
        self.assertIn("TRUNCATED", body)

    def test_untruncated_pair_reports_false(self):
        _, truncated = pair_review.read_pair(
            ["vow-syntax/src/token.rs"], "compiler/span.vow", 10_000_000
        )

        self.assertFalse(truncated)

    def test_both_sides_are_labelled(self):
        body, _ = pair_review.read_pair(
            ["vow-syntax/src/token.rs"], "compiler/span.vow", 10_000_000
        )

        self.assertIn("=== RUST:", body)
        self.assertIn("=== SELF-HOSTED:", body)


class SystemPromptTest(unittest.TestCase):
    def test_prompt_demands_a_module_header(self):
        # Without this the model emits header-less programs that the Rust
        # compiler rejects outright, and every candidate comes back as an
        # accept/reject divergence for the wrong reason.
        self.assertIn("module M", pair_review.SYSTEM)

    def test_prompt_says_only_error_codes_are_compared(self):
        self.assertIn("error CODE", pair_review.SYSTEM)

    def test_prompt_allows_an_empty_answer(self):
        # A model pushed to produce findings will invent them.
        self.assertIn("empty findings list", pair_review.SYSTEM)


if __name__ == "__main__":
    unittest.main()
