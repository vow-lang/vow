#!/usr/bin/env python3
"""Behavior tests for scripts/peak_rss.py."""

import os
import subprocess
import sys
import time
import unittest

import peak_rss


class ResolvePeakTest(unittest.TestCase):
    def test_largest_sampled_reading_wins_over_a_floor_level_maxrss(self):
        self.assertEqual(
            (9000, peak_rss.SAMPLED), peak_rss.resolve_peak(9000, 7000, 40_000, 40_000)
        )

    def test_single_process_high_water_mark_covers_a_peak_between_samples(self):
        self.assertEqual(
            (8000, peak_rss.SAMPLED), peak_rss.resolve_peak(5000, 8000, 30_000, 40_000)
        )

    def test_maxrss_above_the_floor_is_a_measurement(self):
        self.assertEqual(
            (50_000, peak_rss.RU_MAXRSS), peak_rss.resolve_peak(0, 0, 50_000, 40_000)
        )

    def test_maxrss_at_or_below_the_floor_is_not_a_measurement(self):
        self.assertEqual(0, peak_rss.trusted_maxrss(40_000, 40_000))
        self.assertEqual(0, peak_rss.trusted_maxrss(1_000, 40_000))
        self.assertEqual(40_001, peak_rss.trusted_maxrss(40_001, 40_000))

    def test_unsampled_run_is_a_labelled_upper_bound_never_zero(self):
        self.assertEqual(
            (30_000, peak_rss.UPPER_BOUND), peak_rss.resolve_peak(0, 0, 30_000, 40_000)
        )

    def test_no_reading_at_all_is_an_error(self):
        with self.assertRaisesRegex(ValueError, "no peak RSS reading"):
            peak_rss.resolve_peak(0, 0, 0, 40_000)


class TreeSamplerTest(unittest.TestCase):
    def test_reads_rss_and_high_water_mark_of_a_live_process(self):
        rss, hwm = peak_rss.read_rss_hwm(os.getpid())
        self.assertGreater(rss, 0)
        self.assertGreaterEqual(hwm, rss)

    def test_gone_process_reads_as_none(self):
        self.assertIsNone(peak_rss.read_rss_hwm(2**22 + 12345))

    def test_tree_sum_includes_a_descendant(self):
        child = subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(30)"],
            start_new_session=True,
        )
        self.addCleanup(child.wait)
        self.addCleanup(child.kill)
        solo = peak_rss.read_rss_hwm(os.getpid())[0]
        deadline = time.monotonic() + 10
        tree = single = 0
        while time.monotonic() < deadline and tree <= solo:
            tree, single = peak_rss.sample_tree(os.getpid())
        self.assertGreater(tree, solo)
        self.assertGreater(single, 0)

    def test_series_skips_samples_of_an_exited_unreaped_root(self):
        child = subprocess.Popen([sys.executable, "-c", "pass"])
        self.addCleanup(child.wait)
        os.waitid(os.P_PID, child.pid, os.WEXITED | os.WNOWAIT)
        sampler = peak_rss.TreeSampler(child.pid, time.monotonic, time.monotonic())
        time.sleep(0.05)
        sampler.stop()
        self.assertEqual([], sampler.series)


if __name__ == "__main__":
    unittest.main()
