#!/usr/bin/env python3
"""Behavior tests for scripts/check_help_coverage.py.

The checker is a staleness gate: it is supposed to fail when a grammar
token is missing from its canonical `--help` JSON field, even if that same
token happens to appear elsewhere in the JSON (e.g. inside an unrelated
builtin signature). These tests build real help JSON via
generate_help.build_help_json against the actual docs/spec/*.md files, then
mutate one field at a time to reproduce that "hidden elsewhere" failure mode
and confirm the checker actually catches it.
"""

import json
import subprocess
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CHECKER = REPO_ROOT / "scripts" / "check_help_coverage.py"
GRAMMAR = REPO_ROOT / "docs" / "spec" / "grammar.md"

sys.path.insert(0, str(REPO_ROOT / "scripts"))
import generate_help  # noqa: E402


def _build_help_data() -> dict:
    grammar = GRAMMAR.read_text()
    cli = (REPO_ROOT / "docs" / "spec" / "cli.md").read_text()
    contracts = (REPO_ROOT / "docs" / "spec" / "contracts.md").read_text()
    return generate_help.build_help_json(grammar, cli, contracts)


def _run_checker(help_data: dict) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(CHECKER), str(GRAMMAR), json.dumps(help_data)],
        capture_output=True,
        text=True,
    )


class CheckHelpCoverageTest(unittest.TestCase):
    def test_unmutated_help_passes(self):
        result = _run_checker(_build_help_data())
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_missing_type_hidden_in_builtins_is_caught(self):
        data = _build_help_data()
        self.assertIn("String", data["language"]["types"])
        data["language"]["types"] = [
            t for t in data["language"]["types"] if t != "String"
        ]
        # "String" still appears elsewhere in the language object (e.g. in
        # builtin signatures that take/return String), which is exactly the
        # hole this test guards against.
        self.assertIn("String", json.dumps(data["language"]["builtins"]))

        result = _run_checker(data)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("type:String", result.stdout)

    def test_missing_effect_hidden_in_builtin_signature_is_caught(self):
        data = _build_help_data()
        self.assertIn("io", data["language"]["effects"])
        data["language"]["effects"] = [
            e for e in data["language"]["effects"] if e != "io"
        ]
        self.assertIn("io", json.dumps(data["language"]["builtins"]))

        result = _run_checker(data)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("effect:io", result.stdout)

    def test_missing_builtin_hidden_in_another_signature_is_caught(self):
        data = _build_help_data()
        builtins = data["language"]["builtins"]
        self.assertIn("print_str", builtins)
        del builtins["print_str"]
        other_key = next(iter(builtins))
        builtins[other_key] = builtins[other_key] + " print_str"

        result = _run_checker(data)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("builtin:print_str", result.stdout)


if __name__ == "__main__":
    unittest.main()
