#!/usr/bin/env python3
"""Behavior tests for scripts/verify_compiler.py."""

import io
import json
import os
import tempfile
import unittest
from unittest import mock

import verify_compiler as vc

SKIPS = {
    "byte_at_u8": {"code": "unmodeled-builtin", "issue": 1679, "reason": "narrowing"},
    "span_pack_to_here": {
        "code": "unsupported-opcode",
        "issue": 1677,
        "reason": "struct arg",
    },
}


def summary(
    functions=None,
    skipped=None,
    status="Verified",
    verify_status=None,
    counterexamples=(),
    wall=10.0,
    rss=1000.0,
    timed_out=False,
    capacity=(),
):
    return {
        "status": status,
        "verify_status": verify_status,
        "functions": dict(functions or {}),
        "skipped": dict(skipped or {}),
        "counterexamples": list(counterexamples),
        "wall_s": wall,
        "peak_rss_kb": rss,
        "timed_out": timed_out,
        "capacity_functions": list(capacity),
    }


def write(directory, name, text):
    with open(os.path.join(directory, name), "w", encoding="utf-8") as fh:
        fh.write(text)


class RootsTest(unittest.TestCase):
    def test_main_comes_first_then_unreached_modules(self):
        with tempfile.TemporaryDirectory() as d:
            write(d, "main.vow", "module Main\n\nuse lexer\nuse parser\n")
            write(d, "parser.vow", "module Parser\n\nuse lexer\n")
            write(d, "lexer.vow", "module Lexer\n")
            write(d, "orphan.vow", "module Orphan\n")
            write(d, "test_a.vow", "module TestA\n\nuse lexer\n")
            roots = vc.discover_roots(d)
        self.assertEqual(
            ["main.vow", "orphan.vow", "test_a.vow"],
            [os.path.basename(r) for r in roots],
        )

    def test_a_module_reached_only_through_another_is_not_a_root(self):
        with tempfile.TemporaryDirectory() as d:
            write(d, "main.vow", "module Main\n\nuse a\n")
            write(d, "a.vow", "module A\n\nuse b\n")
            write(d, "b.vow", "module B\n")
            roots = vc.discover_roots(d)
        self.assertEqual(["main.vow"], [os.path.basename(r) for r in roots])

    def test_use_of_something_that_is_not_a_compiler_module_is_ignored(self):
        with tempfile.TemporaryDirectory() as d:
            write(d, "main.vow", "module Main\n\nuse missing\n")
            roots = vc.discover_roots(d)
        self.assertEqual(1, len(roots))

    def test_no_main_is_an_error(self):
        with tempfile.TemporaryDirectory() as d:
            write(d, "a.vow", "module A\n")
            with self.assertRaises(ValueError):
                vc.discover_roots(d)

    def test_the_real_compiler_has_main_first_and_every_module_covered(self):
        roots = vc.discover_roots()
        self.assertEqual("main.vow", os.path.basename(roots[0]))
        self.assertEqual(len(roots), len(set(roots)))
        self.assertIn("module_io.vow", [os.path.basename(r) for r in roots])
        self.assertNotIn("vc_gate.vow", [os.path.basename(r) for r in roots])


class SkipListTest(unittest.TestCase):
    def load(self, text):
        with tempfile.TemporaryDirectory() as d:
            write(d, "skips.toml", text)
            return vc.load_skips(os.path.join(d, "skips.toml"))

    def test_a_valid_entry_loads(self):
        skips = self.load(
            '[[skip]]\nfunction = "f"\ncode = "unsupported-opcode"\nissue = 7\nreason = "why"\n'
        )
        self.assertEqual(
            {"f": {"code": "unsupported-opcode", "issue": 7, "reason": "why"}}, skips
        )

    def test_an_empty_file_is_an_empty_list(self):
        self.assertEqual({}, self.load("# nothing skipped\n"))

    def test_the_checked_in_list_is_valid(self):
        skips = vc.load_skips(vc.DEFAULT_SKIPS)
        for name, entry in skips.items():
            self.assertIn(entry["code"], vc.NATIVE_SKIP_CODES, msg=name)

    def test_every_entry_needs_a_tracking_issue_a_reason_and_a_closed_code(self):
        base = {
            "function": "f",
            "code": "unsupported-opcode",
            "issue": 1,
            "reason": "r",
        }
        bad = [
            {**base, "issue": "1677"},
            {**base, "issue": True},
            {**base, "reason": "  "},
            {**base, "code": "made-up"},
            {**base, "function": ""},
            {**base, "extra": 1},
        ]
        for entry in bad:
            body = "[[skip]]\n" + "".join(
                f"{k} = {json.dumps(v)}\n" for k, v in entry.items()
            )
            with self.assertRaises(ValueError, msg=str(entry)):
                self.load(body)

    def test_a_function_listed_twice_is_rejected(self):
        entry = '[[skip]]\nfunction = "f"\ncode = "unsupported-opcode"\nissue = 1\nreason = "r"\n'
        with self.assertRaises(ValueError):
            self.load(entry + entry)


class ParsingTest(unittest.TestCase):
    def test_progress_lines_keep_the_last_word_per_function(self):
        text = (
            "Verifying f...\n  f: BV inconclusive, retrying with --z3 --ir...\n"
            "  f: TIMEOUT (BV and IR)\n  g: PROVEN\n  h: PROVEN (IR)\n"
            "  k: SKIPPED (outside the native verifier subset)\n  bad line: PROVEN\n"
        )
        self.assertEqual(
            {"f": "TIMEOUT", "g": "PROVEN", "h": "PROVEN", "k": "SKIPPED"},
            vc.progress_verdicts(text),
        )

    def test_a_note_hint_is_not_a_function(self):
        text = "note[RegionRootEscape]: ...\n  blame: Callee\n  hint: if intentional\n  g: PROVEN\n"
        self.assertEqual({"g": "PROVEN"}, vc.progress_verdicts(text))

    def test_missing_stderr_is_empty(self):
        self.assertEqual({}, vc.progress_verdicts(None))

    def test_skipped_functions_read_the_verification_skipped_warnings(self):
        result = {
            "diagnostics": [
                {
                    "error_code": "VerificationSkipped",
                    "message": "skipped verification of `f`: unmodeled-builtin: __vow_x",
                },
                {
                    "error_code": "VerificationSkipped",
                    "message": "calls from `a` to contracted `b` were not verified: `a` cannot be modelled",
                },
                {
                    "error_code": "ArithOverflowReachable",
                    "message": "skipped verification of `z`: nope",
                },
            ]
        }
        self.assertEqual(
            {"f": "unmodeled-builtin: __vow_x"}, vc.skipped_functions(result)
        )

    def test_skip_code_is_the_closed_list_prefix_or_none(self):
        self.assertEqual(
            "unsupported-opcode", vc.skip_code("unsupported-opcode: Call[i64]")
        )
        self.assertEqual(
            "recursion-unsupported", vc.skip_code("recursion-unsupported: a -> b -> a")
        )
        self.assertIsNone(vc.skip_code("Load/Store"))


class FunctionRowsTest(unittest.TestCase):
    def rows(self, esbmc, native, skips=SKIPS):
        return {
            r["function"]: r
            for r in vc.function_rows(summary(esbmc), summary(native), skips)
        }

    def test_equal_proofs_match(self):
        rows = self.rows({"f": "PROVEN"}, {"f": "PROVEN"})
        self.assertEqual("match", rows["f"]["class"])
        self.assertEqual([], vc.unexplained(list(rows.values())))

    def test_native_proving_what_esbmc_leaves_open_is_more_precise(self):
        rows = self.rows({"f": "UNKNOWN"}, {"f": "PROVEN"})
        self.assertEqual("more_precise", rows["f"]["class"])
        self.assertEqual([], vc.unexplained(list(rows.values())))

    def test_native_skipping_what_esbmc_proves_is_weaker_unless_listed(self):
        rows = self.rows(
            {"byte_at_u8": "PROVEN", "other": "PROVEN"},
            {"byte_at_u8": "SKIPPED", "other": "SKIPPED"},
        )
        self.assertEqual("weaker", rows["byte_at_u8"]["class"])
        self.assertEqual(SKIPS["byte_at_u8"], rows["byte_at_u8"]["explained"])
        self.assertIsNone(rows["other"]["explained"])
        self.assertEqual(
            ["other"], [r["function"] for r in vc.unexplained(list(rows.values()))]
        )

    def test_both_inconclusive_is_a_match(self):
        rows = self.rows({"f": "TIMEOUT"}, {"f": "SKIPPED"})
        self.assertEqual("match", rows["f"]["class"])

    def test_native_refuting_a_function_is_weaker_because_the_compiler_is_correct(self):
        rows = self.rows({"f": "PROVEN"}, {"f": "FAILED"})
        self.assertEqual("weaker", rows["f"]["class"])
        self.assertEqual(
            ["f"], [r["function"] for r in vc.unexplained(list(rows.values()))]
        )

    def test_native_proving_what_esbmc_refutes_through_its_capacity_is_more_precise(
        self,
    ):
        rows = {
            r["function"]: r
            for r in vc.function_rows(
                summary({"f": "FAILED"}, capacity=["f"]),
                summary({"f": "PROVEN"}),
                SKIPS,
            )
        }
        self.assertEqual("more_precise", rows["f"]["class"])
        self.assertEqual([], vc.unexplained(list(rows.values())))

    def test_a_capacity_counterexample_of_another_function_does_not_excuse_this_one(
        self,
    ):
        rows = vc.function_rows(
            summary({"f": "FAILED"}, capacity=["g"]), summary({"f": "PROVEN"}), SKIPS
        )
        self.assertEqual("soundness", rows[0]["class"])

    def test_native_proving_what_esbmc_refutes_is_a_soundness_row(self):
        rows = self.rows({"f": "FAILED"}, {"f": "PROVEN"})
        self.assertEqual("soundness", rows["f"]["class"])
        self.assertEqual(1, len(vc.unexplained(list(rows.values()))))

    def test_a_worker_that_died_is_weaker(self):
        for word in ("PANICKED", "ERROR"):
            rows = self.rows({"f": "PROVEN"}, {"f": word})
            self.assertEqual("weaker", rows["f"]["class"], word)

    def test_a_function_native_never_reported_is_absent_and_weaker(self):
        rows = self.rows({"f": "PROVEN"}, {})
        self.assertEqual("absent", rows["f"]["native"])
        self.assertEqual("weaker", rows["f"]["class"])

    def test_an_absent_function_on_both_sides_is_never_listed(self):
        self.assertEqual([], vc.function_rows(summary(), summary(), SKIPS))


class NativeProblemsTest(unittest.TestCase):
    def test_a_clean_run_has_no_problem(self):
        self.assertEqual([], vc.native_problems(summary(), SKIPS))

    def test_a_listed_skip_is_acceptable(self):
        native = summary(
            status="Skipped",
            skipped={"byte_at_u8": "unmodeled-builtin: __vow_i64_to_u8_wrap"},
        )
        self.assertEqual([], vc.native_problems(native, SKIPS))

    def test_an_unlisted_skip_is_a_problem(self):
        native = summary(
            status="Skipped", skipped={"new_fn": "unsupported-opcode: Call"}
        )
        problems = vc.native_problems(native, SKIPS)
        self.assertEqual(1, len(problems))
        self.assertIn("not in the skip list", problems[0])

    def test_a_skip_for_another_reason_than_listed_is_a_problem(self):
        native = summary(
            status="Skipped", skipped={"byte_at_u8": "recursion-unsupported: a -> a"}
        )
        self.assertIn("skip list says", vc.native_problems(native, SKIPS)[0])

    def test_failures_are_problems(self):
        for native in (
            summary(status="VerifyFailed"),
            summary(status="CompileFailed"),
            summary(counterexamples=[("f", "Callee", 0)]),
            summary(verify_status="unknown"),
            summary(timed_out=True),
        ):
            self.assertTrue(vc.native_problems(native, SKIPS), native)

    def test_stale_entries_are_those_no_root_skips(self):
        natives = [summary(skipped={"byte_at_u8": "unmodeled-builtin: x"}), summary()]
        self.assertEqual(["span_pack_to_here"], vc.stale_skips(natives, SKIPS))
        self.assertEqual(
            [],
            vc.stale_skips(
                natives + [summary(skipped={"span_pack_to_here": "u"})], SKIPS
            ),
        )


def timed(root, esbmc, native, skips=SKIPS):
    return vc.timed_row(root, esbmc, native, skips)


class CompareGateTest(unittest.TestCase):
    def passing_rows(self, wall=10.0, rss=1000.0):
        esbmc = summary({"f": "PROVEN"}, wall=10.0, rss=1000.0)
        native = summary({"f": "PROVEN"}, wall=wall, rss=rss)
        return [timed("/r/compiler/main.vow", esbmc, native)]

    def gate(self, rows, perf=True, skips=SKIPS):
        return vc.compare_criteria(rows, skips, perf, 5)

    def stale_free(self):
        return {}

    def test_equal_work_passes(self):
        gate = self.gate(self.passing_rows(), skips={})
        self.assertTrue(gate["passed"], gate)

    def test_the_gate_fails_on_an_unexplained_weaker_function(self):
        esbmc = summary({"f": "PROVEN"})
        native = summary(
            {"f": "SKIPPED"},
            status="Skipped",
            skipped={"f": "unsupported-opcode: Call"},
        )
        gate = self.gate([timed("/r/main.vow", esbmc, native)], skips={})
        self.assertFalse(gate["criteria"]["verdict_parity"])
        self.assertFalse(gate["criteria"]["native_acceptable"])
        self.assertFalse(gate["passed"])

    def test_a_listed_skip_passes_the_verdict_criteria(self):
        skips = {"f": {"code": "unsupported-opcode", "issue": 1, "reason": "r"}}
        esbmc = summary({"f": "PROVEN"})
        native = summary(
            {"f": "SKIPPED"},
            status="Skipped",
            skipped={"f": "unsupported-opcode: Call"},
        )
        gate = self.gate([timed("/r/main.vow", esbmc, native, skips)], skips=skips)
        self.assertTrue(gate["passed"], gate)

    def test_a_stale_skip_entry_fails_the_gate(self):
        gate = self.gate(self.passing_rows())
        self.assertEqual(["byte_at_u8", "span_pack_to_here"], gate["stale_skips"])
        self.assertFalse(gate["criteria"]["skip_list_fresh"])
        self.assertFalse(gate["passed"])

    def test_cost_over_the_gate_fails_and_informational_mode_reports_it(self):
        slow = self.passing_rows(wall=14.0)
        self.assertFalse(self.gate(slow, skips={})["passed"])
        informational = self.gate(slow, perf=False, skips={})
        self.assertTrue(informational["passed"])
        self.assertFalse(informational["perf_passed"])

    def test_rss_is_gated_on_its_own(self):
        gate = self.gate(self.passing_rows(rss=2000.0), skips={})
        self.assertFalse(gate["criteria"]["geomean_rss"])
        self.assertTrue(gate["criteria"]["geomean_wall"])

    def test_a_root_below_the_wall_floor_is_not_gated(self):
        esbmc = summary({"f": "PROVEN"}, wall=0.2, rss=100.0)
        native = summary({"f": "PROVEN"}, wall=0.9, rss=900.0)
        row = timed("/r/span.vow", esbmc, native)
        self.assertFalse(row["comparable"])
        self.assertEqual("below-floor", row["excluded"])
        gate = self.gate(self.passing_rows() + [row], skips={})
        self.assertTrue(gate["passed"], gate)
        self.assertEqual(1, gate["comparable"])

    def test_with_nothing_comparable_the_cost_criteria_fail(self):
        esbmc = summary({"f": "PROVEN"}, wall=0.2, rss=100.0)
        native = summary({"f": "PROVEN"}, wall=0.3, rss=100.0)
        gate = self.gate([timed("/r/span.vow", esbmc, native)], skips={})
        self.assertFalse(gate["passed"])
        self.assertTrue(
            self.gate([timed("/r/span.vow", esbmc, native)], perf=False, skips={})[
                "passed"
            ]
        )

    def test_an_unpinned_esbmc_makes_the_result_provisional(self):
        rows = self.passing_rows()
        self.assertFalse(vc.compare_criteria(rows, {}, True, 5, True)["provisional"])
        self.assertTrue(vc.compare_criteria(rows, {}, True, 5, False)["provisional"])
        self.assertTrue(vc.compare_criteria(rows, {}, True, 1, True)["provisional"])

    def test_a_native_only_timeout_fails_even_when_excluded_from_the_ratios(self):
        esbmc = summary({"f": "PROVEN"})
        native = summary({"f": "UNKNOWN"}, timed_out=True, status="VerifyFailed")
        gate = self.gate([timed("/r/main.vow", esbmc, native)], skips={})
        self.assertFalse(gate["criteria"]["no_new_timeouts"])
        self.assertFalse(gate["passed"])

    def test_a_timeout_on_both_sides_is_not_a_native_regression(self):
        esbmc = summary({}, timed_out=True)
        native = summary({}, timed_out=True)
        gate = self.gate([timed("/r/main.vow", esbmc, native)], skips={})
        self.assertTrue(gate["criteria"]["no_new_timeouts"])


class PartialRootsTest(unittest.TestCase):
    def test_a_root_subset_does_not_make_skip_entries_stale(self):
        skips = {"g": {"code": "unmodeled-builtin", "issue": 1, "reason": "r"}}
        run = [summary({"f": "PROVEN"})]
        self.assertFalse(vc.stage_criteria([("a", run)], skips)["passed"])
        self.assertTrue(
            vc.stage_criteria([("a", run)], skips, all_roots=False)["passed"]
        )


class StageGateTest(unittest.TestCase):
    def stages(self, *natives_per_stage):
        return [(f"stage{i}", natives) for i, natives in enumerate(natives_per_stage)]

    def test_identical_stages_pass(self):
        run = [summary({"f": "PROVEN"})]
        gate = vc.stage_criteria(self.stages(run, run), {})
        self.assertTrue(gate["passed"], gate)

    def test_a_stage_that_disagrees_fails(self):
        a = [summary({"f": "PROVEN"})]
        b = [
            summary(
                {"f": "SKIPPED"},
                status="Skipped",
                skipped={"f": "unsupported-opcode: Call"},
            )
        ]
        skips = {"f": {"code": "unsupported-opcode", "issue": 1, "reason": "r"}}
        gate = vc.stage_criteria(self.stages(a, b), skips)
        self.assertFalse(gate["criteria"]["stages_agree"])
        self.assertEqual(1, len(gate["stage_disagreements"]))

    def test_timings_do_not_make_stages_disagree(self):
        a = [summary({"f": "PROVEN"}, wall=1.0)]
        b = [summary({"f": "PROVEN"}, wall=99.0)]
        self.assertTrue(vc.stage_criteria(self.stages(a, b), {})["passed"])

    def test_every_stage_is_held_to_the_skip_list(self):
        ok = [summary({"f": "PROVEN"})]
        bad = [
            summary(
                {"f": "SKIPPED"},
                status="Skipped",
                skipped={"f": "unsupported-opcode: Call"},
            )
        ]
        gate = vc.stage_criteria(self.stages(ok, bad), {})
        self.assertFalse(gate["criteria"]["native_acceptable"])

    def test_a_stale_entry_fails_native_only_runs_too(self):
        gate = vc.stage_criteria(self.stages([summary()]), SKIPS)
        self.assertFalse(gate["criteria"]["skip_list_fresh"])


class CapacityTest(unittest.TestCase):
    def test_only_functions_with_nothing_but_capacity_counterexamples_count(self):
        result = {
            "counterexamples": [
                {
                    "function": "a",
                    "violation": "vec exceeded the verifier's internal capacity limit",
                },
                {
                    "function": "b",
                    "violation": "vec exceeded the verifier's internal capacity limit",
                },
                {"function": "b", "violation": "array bounds violated"},
            ]
        }
        self.assertEqual(["a"], vc.capacity_functions(result))

    def test_no_result_has_no_capacity_functions(self):
        self.assertEqual([], vc.capacity_functions(None))
        self.assertEqual([], vc.capacity_functions({}))


class FingerprintTest(unittest.TestCase):
    def test_counterexamples_are_compared_as_a_set(self):
        a = summary(
            counterexamples=[
                {"fn": "a", "blame": "Callee", "vow_id": 0},
                {"fn": "b", "blame": "Caller", "vow_id": 1},
            ]
        )
        b = summary(
            counterexamples=[
                {"fn": "b", "blame": "Caller", "vow_id": 1},
                {"fn": "a", "blame": "Callee", "vow_id": 0},
            ]
        )
        self.assertEqual(vc.fingerprint(a), vc.fingerprint(b))


class MeasureTest(unittest.TestCase):
    def sample(self, wall, stderr="  f: PROVEN\n"):
        return {
            "wall_s": wall,
            "peak_rss_kb": 100.0,
            "result": {"status": "Verified", "diagnostics": [], "counterexamples": []},
            "timed_out": False,
            "stderr": stderr,
        }

    def test_measure_discards_a_warm_up_run_when_it_times_several(self):
        walls = iter([99.0, 3.0, 1.0, 2.0])
        with mock.patch.object(
            vc.verify_perf,
            "run_once",
            side_effect=lambda *a, **k: self.sample(next(walls)),
        ) as run:
            result = vc.measure("vowc", "native", "p.vow", 1, None, 100, 3)
        self.assertEqual(4, run.call_count)
        self.assertEqual(2.0, result["wall_s"])
        self.assertEqual({"f": "PROVEN"}, result["functions"])
        self.assertTrue(all(c.kwargs["capture_stderr"] for c in run.call_args_list))

    def test_measure_with_one_run_is_not_warmed_up(self):
        with mock.patch.object(
            vc.verify_perf, "run_once", return_value=self.sample(5.0)
        ) as run:
            vc.measure("vowc", "native", "p.vow", 1, None, 100, 1)
        self.assertEqual(1, run.call_count)

    def test_summary_reads_the_sample_of_the_modal_verdict(self):
        bad = self.sample(1.0, stderr="  f: FAILED\n")
        bad["result"] = {
            "status": "VerifyFailed",
            "counterexamples": [{"function": "f"}],
        }
        good = self.sample(2.0)
        result = vc.summarise([bad, good, self.sample(3.0)])
        self.assertEqual({"f": "PROVEN"}, result["functions"])


class MainTest(unittest.TestCase):
    def run_main(self, argv, **patches):
        out, err = io.StringIO(), io.StringIO()
        with mock.patch("sys.stdout", out), mock.patch("sys.stderr", err):
            with mock.patch.multiple(vc, **patches):
                code = vc.main(argv)
        return code, out.getvalue(), err.getvalue()

    def test_missing_tools_are_a_harness_error(self):
        code, _, err = self.run_main(
            ["--native-only"], preflight=lambda *a: ["`bitwuzla` is not on PATH"]
        )
        self.assertEqual(2, code)
        self.assertIn("bitwuzla", err)

    def test_a_malformed_skip_list_is_a_harness_error(self):
        with tempfile.TemporaryDirectory() as d:
            write(d, "s.toml", '[[skip]]\nfunction = "f"\n')
            code, _, err = self.run_main(
                ["--native-only", "--skips", os.path.join(d, "s.toml")],
                preflight=lambda *a: [],
            )
        self.assertEqual(2, code)
        self.assertIn("skip entry", err)

    def test_several_binaries_need_native_only(self):
        with mock.patch("sys.stderr", io.StringIO()), self.assertRaises(SystemExit):
            vc.parse_args(["--vowc", "a", "--vowc", "b"])

    def test_native_only_passes_and_writes_the_report(self):
        with tempfile.TemporaryDirectory() as d:
            write(d, "s.toml", "")
            report = os.path.join(d, "r.json")
            code, out, err = self.run_main(
                [
                    "--native-only",
                    "--skips",
                    os.path.join(d, "s.toml"),
                    "--root",
                    "x.vow",
                    "--output",
                    report,
                ],
                preflight=lambda *a: [],
                measure=lambda *a, **k: summary({"f": "PROVEN"}),
            )
            with open(report, encoding="utf-8") as fh:
                data = json.load(fh)
        self.assertEqual(0, code, err)
        self.assertEqual("", out)
        self.assertEqual("native-only", data["mode"])
        self.assertTrue(data["gate"]["passed"])
        self.assertIsNone(data["esbmc_version"])

    def test_a_failing_criterion_exits_one(self):
        with tempfile.TemporaryDirectory() as d:
            write(d, "s.toml", "")
            code, out, _ = self.run_main(
                [
                    "--native-only",
                    "--skips",
                    os.path.join(d, "s.toml"),
                    "--root",
                    "x.vow",
                ],
                preflight=lambda *a: [],
                measure=lambda *a, **k: summary({"f": "FAILED"}, status="VerifyFailed"),
            )
        self.assertEqual(1, code)
        self.assertFalse(json.loads(out)["gate"]["passed"])

    def test_compare_mode_measures_both_backends_per_root(self):
        seen = []

        def fake(vowc, backend, path, jobs, timeout, watchdog, runs):
            seen.append((backend, os.path.basename(path), runs))
            return summary({"f": "PROVEN"})

        with tempfile.TemporaryDirectory() as d:
            write(d, "s.toml", "")
            code, out, err = self.run_main(
                [
                    "--runs",
                    "2",
                    "--no-perf-gate",
                    "--skips",
                    os.path.join(d, "s.toml"),
                    "--root",
                    "a.vow",
                    "--root",
                    "b.vow",
                ],
                preflight=lambda *a: [],
                measure=fake,
            )
        self.assertEqual(0, code, err)
        self.assertEqual(
            [
                ("esbmc", "a.vow", 2),
                ("native", "a.vow", 2),
                ("esbmc", "b.vow", 2),
                ("native", "b.vow", 2),
            ],
            seen,
        )
        self.assertEqual("compare", json.loads(out)["mode"])


if __name__ == "__main__":
    unittest.main()
