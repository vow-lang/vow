#!/usr/bin/env python3
"""Unit tests for scripts/check_memory_bounds.py."""

import os
import stat
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

import check_memory_bounds as cmb  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parent.parent


class ReadBoundTests(unittest.TestCase):
    def write(self, text: str) -> Path:
        handle = tempfile.NamedTemporaryFile("w", suffix=".vow", delete=False)
        handle.write(text)
        handle.close()
        self.addCleanup(os.unlink, handle.name)
        return Path(handle.name)

    def test_reads_the_first_line_annotation(self):
        self.assertEqual(
            cmb.read_bound(self.write("// BENCH: max-rss-kb 7500\nmodule M\n")), 7500
        )

    def test_rejects_a_misplaced_or_malformed_annotation(self):
        self.assertIsNone(
            cmb.read_bound(self.write("module M\n// BENCH: max-rss-kb 7500\n"))
        )
        self.assertIsNone(cmb.read_bound(self.write("// BENCH: max-rss-kb lots\n")))
        self.assertIsNone(cmb.read_bound(self.write("")))

    def test_every_checked_in_program_is_annotated(self):
        sources = sorted(cmb.PROGRAM_DIR.glob("*.vow"))
        self.assertGreater(len(sources), 0)
        for source in sources:
            self.assertIsNotNone(
                cmb.read_bound(source), f"{source.name} lacks a BENCH bound"
            )


@unittest.skipUnless(sys.platform.startswith("linux"), "needs /proc")
class SamplingTests(unittest.TestCase):
    def test_peak_of_a_process_running_the_executable(self):
        peak = cmb.vm_hwm_kb(os.getpid(), Path(sys.executable))
        self.assertIsNotNone(peak)
        self.assertGreater(peak, 0)

    def test_a_process_not_yet_running_the_executable_is_not_sampled(self):
        self.assertIsNone(cmb.vm_hwm_kb(os.getpid(), Path("/bin/true")))

    def test_a_missing_process_is_not_sampled(self):
        self.assertIsNone(cmb.vm_hwm_kb(2**22 + 1, Path(sys.executable)))


@unittest.skipUnless(sys.platform.startswith("linux"), "needs /proc")
class CheckProgramTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="check_memory_bounds_test_"))
        self.addCleanup(
            lambda: __import__("shutil").rmtree(self.tmp, ignore_errors=True)
        )
        self.compiler = self.tmp / "fake_compiler.sh"
        self.compiler.write_text('#!/bin/sh\ncp /bin/true "$6"\n')
        self.compiler.chmod(self.compiler.stat().st_mode | stat.S_IXUSR)
        self.source = self.tmp / "prog.vow"
        self.source.write_text("// BENCH: max-rss-kb 100\nmodule Prog\n")

    def test_a_program_within_its_bound_passes(self):
        with mock.patch.object(cmb, "run_and_measure", return_value=(0, 50)):
            result = cmb.check_program(self.compiler, self.source, self.tmp)
        self.assertEqual(result["status"], "pass", result)
        self.assertEqual(result["bound_kb"], 100)

    def test_a_program_over_its_bound_fails(self):
        with mock.patch.object(cmb, "run_and_measure", return_value=(0, 101)):
            result = cmb.check_program(self.compiler, self.source, self.tmp)
        self.assertEqual(result["status"], "rss_exceeded")
        self.assertEqual(result["max_rss_kb"], 101)

    def test_a_program_at_its_bound_passes(self):
        with mock.patch.object(cmb, "run_and_measure", return_value=(0, 100)):
            result = cmb.check_program(self.compiler, self.source, self.tmp)
        self.assertEqual(result["status"], "pass")

    def test_a_program_that_was_never_sampled_does_not_pass(self):
        with mock.patch.object(cmb, "run_and_measure", return_value=(0, 0)):
            result = cmb.check_program(self.compiler, self.source, self.tmp)
        self.assertEqual(result["status"], "unsampled")

    def test_a_failing_program_is_reported_before_its_memory(self):
        with mock.patch.object(cmb, "run_and_measure", return_value=(3, 1)):
            result = cmb.check_program(self.compiler, self.source, self.tmp)
        self.assertEqual(result["status"], "run_failed")

    def test_a_failed_build_is_reported(self):
        failing = self.tmp / "failing_compiler.sh"
        failing.write_text("#!/bin/sh\nexit 1\n")
        failing.chmod(failing.stat().st_mode | stat.S_IXUSR)
        result = cmb.check_program(failing, self.source, self.tmp)
        self.assertEqual(result["status"], "build_failed")

    def test_a_missing_annotation_is_reported(self):
        self.source.write_text("module Prog\n")
        result = cmb.check_program(self.compiler, self.source, self.tmp)
        self.assertEqual(result["status"], "missing_annotation")


if __name__ == "__main__":
    unittest.main()
