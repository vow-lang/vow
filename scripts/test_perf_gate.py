#!/usr/bin/env python3
"""Behavior tests for scripts/perf_gate.py."""

import unittest

import perf_gate as pg
import verify_diff


def row(
    fixture="g/f",
    cls="match",
    comparable=True,
    gating=True,
    wall=1.0,
    rss=1.0,
    native_timeout=False,
    esbmc_timeout=False,
):
    return {
        "fixture": fixture,
        "class": cls,
        "comparable": comparable,
        "gating": gating,
        "ratio_wall": wall,
        "ratio_rss": rss,
        "esbmc": {"timed_out": esbmc_timeout},
        "native": {"timed_out": native_timeout},
    }


class StatisticsTest(unittest.TestCase):
    def test_geomean_of_reciprocal_ratios_is_one(self):
        self.assertAlmostEqual(1.0, pg.geomean([2.0, 0.5]))
        self.assertAlmostEqual(2.0, pg.geomean([1.0, 4.0]))

    def test_geomean_rejects_empty_and_non_positive(self):
        with self.assertRaises(ValueError):
            pg.geomean([])
        with self.assertRaises(ValueError):
            pg.geomean([1.0, 0.0])

    def test_ratio_is_native_over_esbmc(self):
        self.assertEqual(0.5, pg.ratio(1.0, 2.0))

    def test_ratio_rejects_zero_denominator(self):
        with self.assertRaises(ValueError):
            pg.ratio(1.0, 0.0)


class GateTest(unittest.TestCase):
    def gate(self, rows, runs=pg.RUNS):
        return pg.evaluate_gate(rows, runs)

    def test_geomean_exactly_one_passes_and_above_fails(self):
        self.assertTrue(self.gate([row(wall=1.25), row(wall=0.8)])["passed"])
        gate = self.gate([row(wall=1.001)])
        self.assertFalse(gate["passed"])
        self.assertFalse(gate["criteria"]["geomean_wall"])
        self.assertTrue(gate["criteria"]["geomean_rss"])

    def test_rss_is_gated_independently_of_wall(self):
        gate = self.gate([row(wall=0.5, rss=1.2)])
        self.assertTrue(gate["criteria"]["geomean_wall"])
        self.assertFalse(gate["criteria"]["geomean_rss"])
        self.assertFalse(gate["passed"])

    def test_worst_fixture_bound_is_inclusive_at_1_25(self):
        fast = row("g/fast", wall=0.1)
        self.assertTrue(self.gate([fast, row("g/edge", wall=1.25)])["passed"])
        gate = self.gate([fast, row("g/slow", wall=1.2501)])
        self.assertFalse(gate["criteria"]["worst_wall"])
        self.assertEqual("g/slow", gate["metrics"]["wall"]["worst"]["fixture"])

    def test_native_only_timeout_fails_the_gate(self):
        gate = self.gate(
            [
                row(),
                row(
                    "g/t",
                    cls="weaker",
                    comparable=False,
                    wall=None,
                    rss=None,
                    native_timeout=True,
                ),
            ]
        )
        self.assertFalse(gate["criteria"]["no_new_timeouts"])
        self.assertEqual(["g/t"], gate["timeouts"]["native_only"])
        self.assertFalse(gate["passed"])

    def test_timeout_on_both_backends_passes(self):
        gate = self.gate(
            [
                row(),
                row(
                    "g/t",
                    comparable=False,
                    wall=None,
                    rss=None,
                    native_timeout=True,
                    esbmc_timeout=True,
                ),
            ]
        )
        self.assertTrue(gate["passed"])
        self.assertEqual(["g/t"], gate["timeouts"]["both"])

    def test_esbmc_only_timeout_is_reported_not_gated(self):
        gate = self.gate(
            [
                row(),
                row("g/t", comparable=False, wall=None, rss=None, esbmc_timeout=True),
            ]
        )
        self.assertTrue(gate["passed"])
        self.assertEqual(["g/t"], gate["timeouts"]["esbmc_only"])

    def test_incomparable_rows_do_not_enter_ratios(self):
        gate = self.gate(
            [row(), row("g/w", cls="weaker", comparable=False, wall=9.0, rss=9.0)]
        )
        self.assertTrue(gate["passed"])
        self.assertEqual(1, gate["comparable"])
        self.assertEqual(1.0, gate["metrics"]["wall"]["geomean"])

    def test_no_comparable_rows_fails_closed(self):
        gate = self.gate([row(comparable=False, wall=None, rss=None)])
        self.assertFalse(gate["passed"])
        self.assertFalse(gate["criteria"]["enough_comparable"])
        self.assertIsNone(gate["metrics"]["wall"]["geomean"])

    def test_fewer_runs_than_the_gate_requires_is_provisional(self):
        self.assertFalse(self.gate([row()])["provisional"])
        self.assertTrue(self.gate([row()], runs=1)["provisional"])
        self.assertTrue(pg.evaluate_gate([row()], filtered=True)["provisional"])

    def test_non_gating_rows_never_enter_ratios_or_timeouts(self):
        gate = self.gate(
            [
                row(),
                row("verify-stress/s", gating=False, wall=9.0, rss=9.0),
                row(
                    "verify-stress/t",
                    gating=False,
                    comparable=False,
                    wall=None,
                    rss=None,
                    native_timeout=True,
                ),
            ]
        )
        self.assertTrue(gate["passed"])
        self.assertEqual(1, gate["comparable"])
        self.assertEqual(2, gate["non_gating"])
        self.assertEqual([], gate["timeouts"]["native_only"])


def outcome(verdict, wall=1.0, timed_out=False, unstable=False):
    return {
        "verdict": verdict,
        "wall_s": wall,
        "timed_out": timed_out,
        "unstable": unstable,
    }


def swept(fixture, esbmc, native, cls=verify_diff.MATCH, gating=True):
    excluded = pg.excluded_reason(gating, cls, esbmc, native)
    return {
        "fixture": fixture,
        "class": cls,
        "gating": gating,
        "excluded": excluded,
        "esbmc": esbmc,
        "native": native,
    }


class LikeForLikeTest(unittest.TestCase):
    def reason(self, esbmc, native, cls=verify_diff.MATCH):
        return pg.excluded_reason(True, cls, outcome(esbmc), outcome(native))

    def test_both_deciding_the_same_way_is_comparable(self):
        self.assertIsNone(self.reason("proven", "proven"))
        self.assertIsNone(self.reason("refuted", "refuted"))

    def test_neither_side_deciding_is_left_out(self):
        self.assertEqual("undecided", self.reason("skipped", "inconclusive"))
        self.assertEqual("undecided", self.reason("skipped", "skipped"))
        self.assertEqual("undecided", self.reason("inconclusive", "inconclusive"))

    def test_one_side_deciding_is_left_out_whatever_the_class(self):
        self.assertEqual("undecided", self.reason("skipped", "proven"))
        self.assertEqual("undecided", self.reason("refuted", "inconclusive"))
        self.assertEqual(
            "more_precise", self.reason("skipped", "proven", "more_precise")
        )

    def test_timeouts_and_instability_keep_their_own_reasons(self):
        slow = outcome("inconclusive", timed_out=True)
        self.assertEqual(
            "timeout", pg.excluded_reason(True, "match", slow, outcome("proven"))
        )
        flaky = outcome("proven", unstable=True)
        self.assertEqual(
            "unstable", pg.excluded_reason(True, "match", outcome("proven"), flaky)
        )
        self.assertEqual(
            "non-gating",
            pg.excluded_reason(False, "match", outcome("proven"), outcome("proven")),
        )

    def test_decided_slow_native_row_still_gates(self):
        slow = row("g/refuted", wall=1.46)
        gate = pg.evaluate_gate([row(wall=0.1), slow])
        self.assertFalse(gate["criteria"]["worst_wall"])
        self.assertEqual("g/refuted", gate["metrics"]["wall"]["worst"]["fixture"])

    def test_undecided_rows_list_both_times_and_the_ratio(self):
        rows = [
            swept("g/a", outcome("skipped", 0.005), outcome("inconclusive", 0.079)),
            swept("g/b", outcome("proven", 0.5), outcome("proven", 5.0)),
            swept(
                "g/c", outcome("skipped", 0.01), outcome("proven", 0.1), "more_precise"
            ),
        ]
        listed = pg.undecided_rows(rows)
        self.assertEqual(["g/a", "g/c"], [u["fixture"] for u in listed])
        self.assertEqual("undecided", listed[0]["excluded"])
        self.assertEqual("more_precise", listed[1]["excluded"])
        self.assertEqual({"verdict": "skipped", "wall_s": 0.005}, listed[0]["esbmc"])
        self.assertEqual(
            {"verdict": "inconclusive", "wall_s": 0.079}, listed[0]["native"]
        )
        self.assertAlmostEqual(15.8, listed[0]["ratio_wall"])

    def test_undecided_rows_skip_timeouts_unstable_and_non_gating(self):
        rows = [
            swept("g/t", outcome("inconclusive", timed_out=True), outcome("proven")),
            swept("g/u", outcome("proven", unstable=True), outcome("proven")),
            swept("g/s", outcome("skipped"), outcome("skipped"), gating=False),
        ]
        self.assertEqual([], pg.undecided_rows(rows))


if __name__ == "__main__":
    unittest.main()
