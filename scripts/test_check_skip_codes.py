#!/usr/bin/env python3
"""Behavior tests for scripts/check_skip_codes.py."""

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_skip_codes as c  # noqa: E402

VC_OPS = """
fn VC_SKIP_A() -> String { String::from("alpha") }
fn VC_SKIP_B() -> String { SKIP_BETA() }
fn vc_other() -> String { String::from("not-a-code") }
"""
DOMINANCE = 'fn SKIP_BETA() -> String { String::from("beta") }\n'
ADR = "### 4. Closed list of `Skipped` reasons\n\n| Code | When |\n|---|---|\n| `alpha` | x |\n| `beta` | y |\n\n### 5. Timeline\n| `gamma` | z |\n"


class SkipCodeTests(unittest.TestCase):
    def test_source_codes_resolve_literals_and_indirections(self):
        self.assertEqual(c.source_codes(VC_OPS, DOMINANCE), {"alpha", "beta"})

    def test_unresolvable_indirection_is_an_error(self):
        with self.assertRaises(SystemExit):
            c.source_codes(VC_OPS, "")

    def test_adr_codes_stop_at_the_next_section(self):
        self.assertEqual(c.adr_codes(ADR), {"alpha", "beta"})

    def test_fixture_codes_read_the_directive(self):
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "a.vow").write_text("// TEST: skip-reason alpha\nmodule A\n")
            (Path(d) / "b.vow").write_text("// TEST: category unverifiable\n")
            self.assertEqual(c.fixture_codes(Path(d)), {"alpha"})

    def test_uncovered_code_is_reported(self):
        out = c.problems(
            {"alpha", "ir-non-dominating-read"},
            {"alpha", "ir-non-dominating-read"},
            set(),
        )
        self.assertEqual(len(out), 1)
        self.assertIn("`alpha`", out[0])

    def test_exempt_code_needs_no_fixture(self):
        codes = {"alpha", "ir-non-dominating-read"}
        self.assertEqual(c.problems(codes, codes, {"alpha"}), [])

    def test_source_and_adr_must_agree(self):
        out = c.problems(
            {"alpha", "beta"}, {"alpha", "gamma"}, {"alpha", "beta", "gamma"}
        )
        self.assertTrue(any("`beta` is in compiler/vc_ops.vow" in line for line in out))
        self.assertTrue(any("`gamma` is in the ADR" in line for line in out))

    def test_fixture_pinning_an_unknown_code_is_reported(self):
        codes = {"alpha", "ir-non-dominating-read"}
        out = c.problems(codes, codes, {"alpha", "typo"})
        self.assertEqual(len(out), 1)
        self.assertIn("`typo`", out[0])

    def test_repository_is_consistent(self):
        self.assertEqual(c.main(), 0)


if __name__ == "__main__":
    unittest.main()
