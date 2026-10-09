#!/usr/bin/env python3
"""Behavior tests for scripts/generate_help.py.

The `-o, --output` default reported by `--help` must come from the
docs/spec/cli.md option-table row, not from a string hardcoded in the
generator, so the help text cannot drift from the spec.
"""

import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SPEC = REPO_ROOT / "docs" / "spec"

sys.path.insert(0, str(REPO_ROOT / "scripts"))
import generate_help  # noqa: E402


def _help_data(cli: str | None = None) -> dict:
    grammar = (SPEC / "grammar.md").read_text()
    cli = cli if cli is not None else (SPEC / "cli.md").read_text()
    contracts = (SPEC / "contracts.md").read_text()
    return generate_help.build_help_json(grammar, cli, contracts)


def _output_option(data: dict, command: str) -> dict:
    options = data["command_details"][command]["options"]
    matches = [o for o in options if o["form"] == "-o, --output <path>"]
    assert len(matches) == 1, matches
    return matches[0]


class OutputDefaultTest(unittest.TestCase):
    def test_build_output_default_is_build_stem(self):
        option = _output_option(_help_data(), "build")
        self.assertEqual(option["default"], "build/<stem>")
        self.assertTrue(option["description"].endswith("(default: build/<stem>)"))

    def test_build_output_default_tracks_cli_md(self):
        cli = (SPEC / "cli.md").read_text()
        self.assertIn("`build/<stem>`", cli)
        option = _output_option(
            _help_data(cli.replace("`build/<stem>`", "`out/<stem>.bin`", 1)),
            "build",
        )
        self.assertEqual(option["default"], "out/<stem>.bin")
        self.assertTrue(option["description"].endswith("(default: out/<stem>.bin)"))

    def test_decl_output_default_is_source_vow_d(self):
        option = _output_option(_help_data(), "decl")
        self.assertEqual(option["default"], "<source>.vow.d")
        self.assertEqual(
            option["description"],
            "Output declaration file path (default: <source>.vow.d)",
        )


if __name__ == "__main__":
    unittest.main()
