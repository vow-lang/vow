#!/usr/bin/env python3
"""Behavior tests for scripts/verify_perf.py."""

import io
import json
import os
import stat
import sys
import tempfile
import unittest
from unittest import mock

import verify_perf as vp


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
    def test_median_of_odd_and_even_samples(self):
        self.assertEqual(3, vp.median([5, 1, 3, 2, 4]))
        self.assertEqual(2.5, vp.median([4, 1, 3, 2]))

    def test_geomean_of_reciprocal_ratios_is_one(self):
        self.assertAlmostEqual(1.0, vp.geomean([2.0, 0.5]))
        self.assertAlmostEqual(2.0, vp.geomean([1.0, 4.0]))

    def test_geomean_rejects_empty_and_non_positive(self):
        with self.assertRaises(ValueError):
            vp.geomean([])
        with self.assertRaises(ValueError):
            vp.geomean([1.0, 0.0])

    def test_ratio_is_native_over_esbmc(self):
        self.assertEqual(0.5, vp.ratio(1.0, 2.0))

    def test_ratio_rejects_zero_denominator(self):
        with self.assertRaises(ValueError):
            vp.ratio(1.0, 0.0)


class GateTest(unittest.TestCase):
    def gate(self, rows, runs=vp.RUNS):
        return vp.evaluate_gate(rows, runs)

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
        self.assertTrue(vp.evaluate_gate([row()], filtered=True)["provisional"])

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


class CorpusTest(unittest.TestCase):
    def test_stretch_benchmarks_are_excluded(self):
        manifest = {
            "benchmarks": [
                {"path": "easy/E01_x", "expected_status": "Verified"},
                {"path": "hard/H01_y", "expected_status": "Stretch"},
            ]
        }
        got = vp.benchmark_fixtures(manifest)
        self.assertEqual(
            [("benchmarks", "E01_x", "easy/E01_x/reference.vow")],
            [
                (f.group, f.name, os.path.relpath(f.path, vp.BENCHMARKS_DIR))
                for f in got
            ],
        )
        self.assertEqual("Verified", got[0].truth)

    def corpus_names(self, **kw):
        return {f"{f.group}/{f.name}" for f in vp.collect_corpus(**kw)}

    def test_repo_corpus_has_the_gate_benchmarks_and_float_fixtures(self):
        corpus = vp.collect_corpus()
        self.assertEqual(
            vp.GATE_BENCHMARKS,
            sum(1 for f in corpus if f.group == "benchmarks"),
        )
        names = {f"{f.group}/{f.name}" for f in corpus}
        for want in (
            "verify/float_arithmetic",
            "verify-fail/float_enum_payload_wrong",
            "verify-native/skip/float_skipped",
            "verify-fail-multi/stub_requires_violation",
        ):
            self.assertIn(want, names)

    def test_truth_follows_the_directory(self):
        by_name = {f"{f.group}/{f.name}": f for f in vp.collect_corpus()}
        self.assertEqual("Verified", by_name["verify-native/pass/add_exact"].truth)
        self.assertEqual(
            "VerifyFailed", by_name["verify-native/fail/add_wrong_post"].truth
        )
        self.assertEqual("Skipped", by_name["verify-native/skip/float_skipped"].truth)
        self.assertIsNone(by_name["verify-native/unknown/loop_infinite"].truth)
        multi = by_name["verify-fail-multi/stub_requires_violation"]
        self.assertEqual("VerifyFailed", multi.truth)
        self.assertTrue(multi.path.endswith("main.vow"))

    def test_stress_is_opt_in(self):
        self.assertFalse(
            any(n.startswith("verify-stress/") for n in self.corpus_names())
        )
        self.assertIn(
            "verify-stress/unwind_loop", self.corpus_names(include_stress=True)
        )

    def test_benchmark_count_drift_is_an_error(self):
        with mock.patch.object(vp, "GATE_BENCHMARKS", 22):
            with self.assertRaises(ValueError):
                vp.collect_corpus()


def write_stub(directory, body):
    path = os.path.join(directory, "stub")
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(f"#!{sys.executable}\n{body}\n")
    os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)
    return path


class RunOnceTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)

    def stub(self, body):
        return write_stub(self._tmp.name, body)

    def run_stub(self, body, timeout=None, watchdog_s=20.0, jobs=1):
        return vp.run_once(
            self.stub(body), "native", "x.vow", jobs, timeout, watchdog_s
        )

    def test_cache_off_flags_env_and_result_json(self):
        got = self.run_stub(
            "import json, os, sys\n"
            "d = os.environ['VOW_CACHE_DIR']\n"
            "print(json.dumps({'status': 'Verified', 'argv': sys.argv[1:],\n"
            "                  'cache_empty': os.listdir(d) == []}))\n",
            jobs=3,
        )
        self.assertEqual(
            [
                "verify",
                "--no-cache",
                "--backend",
                "native",
                "--verify-jobs",
                "3",
                "x.vow",
            ],
            got["result"]["argv"],
        )
        self.assertTrue(got["result"]["cache_empty"])
        self.assertFalse(got["timed_out"])
        self.assertEqual(0, got["exit_code"])
        self.assertGreater(got["wall_s"], 0)

    def test_timeout_flag_only_when_explicit(self):
        got = self.run_stub(
            "import json, sys\nprint(json.dumps({'argv': sys.argv[1:]}))\n", timeout=7
        )
        argv = got["result"]["argv"]
        self.assertEqual(["--timeout", "7"], argv[argv.index("--timeout") :][:2])

    def test_peak_rss_covers_child_allocation(self):
        got = self.run_stub(
            "import time, json\n"
            "b = bytearray(60 * 1024 * 1024)\n"
            "for i in range(0, len(b), 4096): b[i] = 1\n"
            "time.sleep(0.2)\n"
            "print(json.dumps({'status': 'Verified'}))\n"
        )
        self.assertGreaterEqual(got["peak_rss_kb"], 50 * 1024)
        self.assertGreaterEqual(got["peak_rss_kb"], got["rss_tree_kb"])

    def test_maxrss_at_or_below_the_harness_peak_is_not_trusted(self):
        with mock.patch.object(
            vp.resource,
            "getrusage",
            return_value=mock.Mock(ru_maxrss=40_000),
        ):
            self.assertEqual(0, vp.trusted_maxrss(40_000))
            self.assertEqual(0, vp.trusted_maxrss(1_000))
            self.assertEqual(40_001, vp.trusted_maxrss(40_001))

    def test_watchdog_kills_a_hung_run(self):
        got = self.run_stub("import time\ntime.sleep(60)\n", watchdog_s=0.5)
        self.assertTrue(got["timed_out"])
        self.assertEqual("timeout", got["result"]["verify_status"])
        self.assertLess(got["wall_s"], 10)

    def test_verifier_reported_timeout_counts_as_timed_out(self):
        got = self.run_stub(
            "import json\n"
            "print(json.dumps({'status': 'VerifyFailed', 'verify_status': 'timeout'}))\n"
        )
        self.assertTrue(got["timed_out"])

    def test_death_by_signal_is_crashed(self):
        got = self.run_stub("import os, signal\nos.kill(os.getpid(), signal.SIGSEGV)\n")
        self.assertEqual("crashed", got["result"]["verify_status"])
        self.assertFalse(got["timed_out"])

    def test_non_json_output_has_no_result(self):
        with mock.patch.object(sys, "stderr", io.StringIO()):
            got = self.run_stub("print('not json')\n")
        self.assertIsNone(got["result"])


def sample(
    verdict_status="Verified",
    wall=1.0,
    rss=1000,
    timed_out=False,
    verify_status=None,
    cex=None,
):
    result = {"status": verdict_status}
    if verify_status:
        result["verify_status"] = verify_status
    if cex:
        result["counterexamples"] = cex
    return {
        "wall_s": wall,
        "peak_rss_kb": rss,
        "timed_out": timed_out,
        "result": result,
        "exit_code": 0,
    }


class FixtureTest(unittest.TestCase):
    FX = vp.Fixture("verify", "x", "tests/verify/x.vow", "Verified")

    def measure(self, per_backend, runs=5, fx=None):
        calls = []

        def fake(vowc, backend, path, jobs, timeout, watchdog_s):
            calls.append((backend, watchdog_s))
            return next(per_backend[backend])

        with (
            mock.patch.object(vp, "run_once", side_effect=fake),
            mock.patch.object(vp.verify_diff, "function_count", return_value=2),
        ):
            result = vp.measure_fixture("vowc", fx or self.FX, runs, 1, None)
        return result, calls

    @staticmethod
    def feed(*samples):
        return iter(samples)

    def test_runs_are_interleaved_after_one_discarded_warmup(self):
        esbmc = self.feed(
            *[sample(wall=100)] + [sample(wall=w) for w in (3, 1, 2, 5, 4)]
        )
        native = self.feed(
            *[sample(wall=100)] + [sample(wall=w) for w in (1, 1, 1, 1, 1)]
        )
        got, calls = self.measure({"esbmc": esbmc, "native": native})
        self.assertEqual(["esbmc", "native"] * 6, [backend for backend, _ in calls])
        self.assertEqual(3, got["esbmc"]["wall_s"])
        self.assertEqual([3, 1, 2, 5, 4], got["esbmc"]["wall_samples"])
        self.assertEqual(1 / 3, got["ratio_wall"])
        self.assertEqual("match", got["class"])
        self.assertTrue(got["comparable"])

    def test_watchdog_scales_with_function_count(self):
        both = lambda: self.feed(*[sample() for _ in range(6)])  # noqa: E731
        _, calls = self.measure({"esbmc": both(), "native": both()})
        self.assertEqual(
            vp.DEFAULT_BUDGET_S * 2 + vp.verify_diff.WATCHDOG_SLACK, calls[0][1]
        )

    def test_stress_rows_are_reported_but_non_gating(self):
        stress = vp.Fixture("verify-stress", "s", "tests/verify-stress/s.vow", None)
        got, _ = self.measure(
            {
                "esbmc": self.feed(*[sample()] * 6),
                "native": self.feed(*[sample()] * 6),
            },
            fx=stress,
        )
        self.assertFalse(got["gating"])
        self.assertFalse(got["comparable"])
        self.assertEqual("non-gating", got["excluded"])

    def test_verdict_that_changes_between_runs_is_unstable(self):
        flaky = [sample()] * 3 + [sample("Skipped")] * 3
        got, _ = self.measure(
            {"esbmc": self.feed(*flaky), "native": self.feed(*[sample()] * 6)}
        )
        self.assertTrue(got["esbmc"]["unstable"])
        self.assertFalse(got["comparable"])
        self.assertEqual("unstable", got["excluded"])

    def test_non_match_class_is_excluded_from_ratios(self):
        got, _ = self.measure(
            {
                "esbmc": self.feed(*[sample()] * 6),
                "native": self.feed(*[sample("Skipped")] * 6),
            }
        )
        self.assertEqual("weaker", got["class"])
        self.assertFalse(got["comparable"])
        self.assertIsNone(got["ratio_wall"])

    def test_timeout_needs_a_strict_majority_of_runs(self):
        slow = sample("VerifyFailed", timed_out=True, verify_status="timeout")
        mostly = [sample()] * 3 + [slow] * 2
        got, _ = self.measure(
            {
                "esbmc": self.feed(sample(), *mostly),
                "native": self.feed(*[sample()] * 6),
            }
        )
        self.assertEqual(2, got["esbmc"]["timeout_runs"])
        self.assertFalse(got["esbmc"]["timed_out"])

        majority = [slow] * 3 + [sample()] * 2
        got, _ = self.measure(
            {
                "esbmc": self.feed(sample(), *majority),
                "native": self.feed(*[sample()] * 6),
            }
        )
        self.assertTrue(got["esbmc"]["timed_out"])
        self.assertFalse(got["comparable"])


class MainTest(unittest.TestCase):
    FX = [
        vp.Fixture("verify", "a", "tests/verify/a.vow", "Verified"),
        vp.Fixture("verify", "b", "tests/verify/b.vow", "Verified"),
    ]

    def run_main(self, argv, wall_by_backend, fixtures=None, problems=()):
        out, err = io.StringIO(), io.StringIO()

        def fake(vowc, backend, path, jobs, timeout, watchdog_s):
            return sample(wall=wall_by_backend[backend])

        with (
            mock.patch.object(vp, "preflight", return_value=list(problems)),
            mock.patch.object(vp, "probe_version", return_value="8.5.0"),
            mock.patch.object(vp, "collect_corpus", return_value=fixtures or self.FX),
            mock.patch.object(vp.verify_diff, "function_count", return_value=1),
            mock.patch.object(vp, "run_once", side_effect=fake),
            mock.patch.object(vp, "require_linux", return_value=True),
            mock.patch.object(sys, "stdout", out),
            mock.patch.object(sys, "stderr", err),
        ):
            code = vp.main(argv)
        return code, out.getvalue(), err.getvalue()

    def test_passing_run_reports_json_and_exits_zero(self):
        code, out, err = self.run_main([], {"esbmc": 2.0, "native": 1.0})
        report = json.loads(out)
        self.assertEqual(0, code)
        self.assertEqual(1, report["schema_version"])
        for key in (
            "vowc",
            "esbmc_version",
            "bitwuzla_version",
            "host",
            "runs",
            "budget",
            "timeout_s",
            "verify_jobs",
            "corpus",
            "rows",
            "gate",
        ):
            self.assertIn(key, report)
        self.assertEqual(5, report["runs"])
        self.assertEqual("default", report["budget"])
        self.assertIsNone(report["timeout_s"])
        self.assertEqual(1, report["verify_jobs"])
        self.assertTrue(report["gate"]["passed"])
        self.assertFalse(report["gate"]["provisional"])
        self.assertEqual(2, len(report["rows"]))
        self.assertIn("geomean", err)

    def test_failing_gate_exits_one(self):
        code, out, _ = self.run_main([], {"esbmc": 1.0, "native": 2.0})
        self.assertEqual(1, code)
        self.assertFalse(json.loads(out)["gate"]["passed"])

    def test_output_flag_writes_the_report_to_a_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            dest = os.path.join(tmp, "r.json")
            code, out, _ = self.run_main(
                ["--output", dest], {"esbmc": 1.0, "native": 1.0}
            )
            with open(dest, encoding="utf-8") as fh:
                report = json.load(fh)
        self.assertEqual(0, code)
        self.assertEqual("", out)
        self.assertTrue(report["gate"]["passed"])

    def test_reduced_runs_and_explicit_timeout_are_recorded(self):
        _, out, _ = self.run_main(
            ["--runs", "1", "--timeout", "9", "--verify-jobs", "2"],
            {"esbmc": 1.0, "native": 1.0},
        )
        report = json.loads(out)
        self.assertEqual(1, report["runs"])
        self.assertEqual("explicit", report["budget"])
        self.assertEqual(9, report["timeout_s"])
        self.assertEqual(2, report["verify_jobs"])
        self.assertTrue(report["gate"]["provisional"])

    def test_filter_selects_a_subset_and_is_provisional(self):
        _, out, _ = self.run_main(
            ["--filter", "verify/b"], {"esbmc": 1.0, "native": 1.0}
        )
        report = json.loads(out)
        self.assertEqual(["verify/b"], [r["fixture"] for r in report["rows"]])
        self.assertTrue(report["gate"]["provisional"])

    def test_preflight_problems_are_a_harness_failure(self):
        code, out, err = self.run_main(
            [], {"esbmc": 1.0, "native": 1.0}, problems=["esbmc 8.3.0 is not 8.5"]
        )
        self.assertEqual(2, code)
        self.assertEqual("", out)
        self.assertIn("esbmc 8.3.0", err)

    def test_empty_selection_is_a_harness_failure(self):
        code, _, _ = self.run_main(["--filter", "typo"], {"esbmc": 1.0, "native": 1.0})
        self.assertEqual(2, code)

    def test_corpus_drift_is_a_harness_failure(self):
        err = io.StringIO()
        with (
            mock.patch.object(vp, "preflight", return_value=[]),
            mock.patch.object(vp, "require_linux", return_value=True),
            mock.patch.object(vp, "collect_corpus", side_effect=ValueError("drift")),
            mock.patch.object(sys, "stderr", err),
        ):
            self.assertEqual(2, vp.main([]))
        self.assertIn("drift", err.getvalue())

    def test_non_linux_is_a_harness_failure(self):
        with (
            mock.patch.object(vp, "require_linux", return_value=False),
            mock.patch.object(sys, "stderr", io.StringIO()),
        ):
            self.assertEqual(2, vp.main([]))


class PreflightTest(unittest.TestCase):
    def test_missing_tools_and_wrong_esbmc_are_reported(self):
        with (
            mock.patch.object(vp.shutil, "which", return_value=None),
            mock.patch.object(vp, "probe_version", return_value=None),
        ):
            problems = vp.preflight("/nonexistent/vowc")
        text = "\n".join(problems)
        self.assertIn("vowc", text)
        self.assertIn("esbmc", text)
        self.assertIn("bitwuzla", text)

    def test_esbmc_version_must_match_the_pin(self):
        with tempfile.TemporaryDirectory() as tmp:
            vowc = write_stub(tmp, "pass")
            with mock.patch.object(vp.shutil, "which", return_value="/bin/x"):
                with mock.patch.object(vp, "probe_version", return_value="8.3.0"):
                    problems = vp.preflight(vowc)
                with mock.patch.object(vp, "probe_version", return_value="8.5.0"):
                    self.assertEqual([], vp.preflight(vowc))
        self.assertEqual(1, len(problems))
        self.assertIn("8.3.0", problems[0])

    def test_version_parser_reads_the_first_dotted_number(self):
        self.assertEqual(
            "8.5.0", vp.parse_version("ESBMC version 8.5.0 64-bit x86_64 linux")
        )
        self.assertIsNone(vp.parse_version("no digits here"))


if __name__ == "__main__":
    unittest.main()
