#!/usr/bin/env python3
"""Behavior tests for the verifier C parity comparator (parity_c.py)."""

import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import parity_c

SCRIPT = Path(__file__).with_name("parity.py")

HARNESS = "int main(void) { vow_user_fn_{n}(0); return 0; }\n"


def c_source(body, n=0):
    return f"int vow_user_fn_{n}(int p0) {{\n{body}}}\n" + HARNESS.replace(
        "{n}", str(n)
    )


def fake_compiler(directory, name, sources):
    """A `verify` stand-in that feeds each source in `sources` to `esbmc`."""
    path = Path(directory) / name
    lines = [
        "#!/usr/bin/env bash",
        "set -eu",
        'tmp=$(mktemp "${TMPDIR:-/tmp}/fake.XXXXXX.c")',
    ]
    for source in sources:
        lines.append(f"cat > \"$tmp\" <<'EOF'\n{source}EOF")
        lines.append('esbmc "$tmp" --64 >/dev/null')
    lines.append('rm -f "$tmp"')
    path.write_text("\n".join(lines) + "\n")
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return str(path)


class HarnessFunctionTests(unittest.TestCase):
    def test_names_the_function_the_harness_calls(self):
        self.assertEqual(
            parity_c.harness_function(c_source("  return 0;\n", 7)), "vow_user_fn_7"
        )

    def test_a_source_without_a_harness_has_no_function(self):
        self.assertIsNone(parity_c.harness_function("int f(void) { return 0; }\n"))


class DiffCaptureTests(unittest.TestCase):
    def test_identical_sets_have_no_problems(self):
        capture = {"f": {"a", "b"}}
        self.assertEqual(parity_c.diff_captures(capture, {"f": {"b", "a"}}), [])

    def test_a_function_only_one_compiler_emitted_is_reported(self):
        problems = parity_c.diff_captures({"f": {"a"}}, {"f": {"a"}, "g": {"b"}})
        self.assertEqual(len(problems), 1)
        self.assertIn("g: C emitted only by the self-hosted compiler", problems[0])

    def test_a_textual_difference_shows_a_unified_diff(self):
        problems = parity_c.diff_captures({"f": {"x\ny\n"}}, {"f": {"x\nz\n"}})
        self.assertEqual(len(problems), 1)
        self.assertIn("-y", problems[0])
        self.assertIn("+z", problems[0])

    def test_a_different_number_of_variants_is_reported(self):
        problems = parity_c.diff_captures({"f": {"a"}}, {"f": {"a", "b"}})
        self.assertEqual(len(problems), 1)


class CompareFixtureTests(unittest.TestCase):
    def run_pair(self, rust_sources, self_sources):
        with tempfile.TemporaryDirectory() as directory:
            rust = fake_compiler(directory, "rust", rust_sources)
            hosted = fake_compiler(directory, "hosted", self_sources)
            return parity_c.compare_fixture(rust, hosted, Path(directory) / "x.vow")

    def test_byte_identical_c_passes(self):
        source = c_source("  return 1;\n")
        self.assertEqual(self.run_pair([source], [source]), [])

    def test_repeated_identical_invocations_count_once(self):
        source = c_source("  return 1;\n")
        self.assertEqual(self.run_pair([source, source], [source]), [])

    def test_one_byte_of_difference_fails(self):
        problems = self.run_pair(
            [c_source("  return 1;\n")], [c_source("  return 2;\n")]
        )
        self.assertEqual(len(problems), 1)
        self.assertIn("vow_user_fn_0", problems[0])

    def test_no_esbmc_invocation_in_either_compiler_is_not_a_comparison(self):
        self.assertIsNone(self.run_pair([], []))

    def test_one_compiler_skipping_what_the_other_verifies_fails(self):
        problems = self.run_pair([c_source("  return 1;\n")], [])
        self.assertEqual(len(problems), 1)
        self.assertIn("only by the rust compiler", problems[0])


class CliTests(unittest.TestCase):
    def test_c_mode_exits_nonzero_on_a_difference_and_zero_otherwise(self):
        with tempfile.TemporaryDirectory() as directory:
            same = c_source("  return 1;\n")
            rust = fake_compiler(directory, "rust", [same])
            equal = fake_compiler(directory, "equal", [same])
            other = fake_compiler(directory, "other", [c_source("  return 2;\n")])
            fixture = Path(directory) / "x.vow"
            fixture.write_text("")

            ok = subprocess.run(
                [sys.executable, str(SCRIPT), "c", rust, equal, str(fixture)],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(ok.returncode, 0, ok.stdout + ok.stderr)
            self.assertIn("OK", ok.stdout)

            bad = subprocess.run(
                [sys.executable, str(SCRIPT), "c", rust, other, str(fixture)],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(bad.returncode, 1, bad.stdout + bad.stderr)
            self.assertIn("FAIL", bad.stdout)

    def test_c_mode_without_fixtures_is_a_usage_error(self):
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "c", "a", "b"],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 2)


if __name__ == "__main__":
    unittest.main()
