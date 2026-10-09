#!/usr/bin/env python3
"""Behavior tests for scripts/verify_diff.py."""

import io
import json
import sys
import unittest
from unittest import mock

import verify_diff as vd

K = ("f", "callee", 1)


class ClassifyTest(unittest.TestCase):
    def cls(self, truth, esbmc, native, ek=(), nk=()):
        return vd.classify(truth, esbmc, native, ek, nk)[0]

    def test_equal_verdicts_match(self):
        self.assertEqual(vd.MATCH, self.cls("Verified", vd.PROVEN, vd.PROVEN))
        self.assertEqual(vd.MATCH, self.cls("Skipped", vd.SKIPPED, vd.SKIPPED))
        self.assertEqual(
            vd.MATCH, self.cls("VerifyFailed", vd.REFUTED, vd.REFUTED, [K], [K])
        )

    def test_native_proving_a_refuted_program_is_soundness(self):
        self.assertEqual(vd.SOUNDNESS, self.cls("VerifyFailed", vd.REFUTED, vd.PROVEN))
        self.assertEqual(vd.SOUNDNESS, self.cls("Verified", vd.REFUTED, vd.PROVEN))

    def test_native_proving_an_incorrect_program_esbmc_cannot_judge_is_soundness(self):
        self.assertEqual(vd.SOUNDNESS, self.cls("VerifyFailed", vd.SKIPPED, vd.PROVEN))
        self.assertEqual(
            vd.SOUNDNESS, self.cls("VerifyFailed", vd.INCONCLUSIVE, vd.PROVEN)
        )

    def test_shared_known_gap_is_not_a_divergence(self):
        self.assertEqual(vd.MATCH, self.cls("VerifyFailed", vd.PROVEN, vd.PROVEN))

    def test_native_less_conclusive_is_weaker(self):
        for native in (vd.SKIPPED, vd.INCONCLUSIVE):
            self.assertEqual(vd.WEAKER, self.cls("Verified", vd.PROVEN, native))
            self.assertEqual(vd.WEAKER, self.cls("VerifyFailed", vd.REFUTED, native))

    def test_native_refuting_a_proven_program_depends_on_truth(self):
        self.assertEqual(
            vd.MORE_PRECISE, self.cls("VerifyFailed", vd.PROVEN, vd.REFUTED)
        )
        self.assertEqual(vd.WEAKER, self.cls("Verified", vd.PROVEN, vd.REFUTED))

    def test_native_concluding_where_esbmc_does_not_is_more_precise(self):
        self.assertEqual(vd.MORE_PRECISE, self.cls("Skipped", vd.SKIPPED, vd.PROVEN))
        self.assertEqual(
            vd.MORE_PRECISE, self.cls("VerifyFailed", vd.INCONCLUSIVE, vd.REFUTED)
        )

    def test_native_refuting_a_correct_program_esbmc_skips_is_weaker(self):
        self.assertEqual(vd.WEAKER, self.cls("Verified", vd.SKIPPED, vd.REFUTED))

    def test_neither_concluding_matches(self):
        self.assertEqual(vd.MATCH, self.cls("Skipped", vd.SKIPPED, vd.INCONCLUSIVE))

    def test_missing_verdict_is_a_harness_error(self):
        self.assertEqual(vd.HARNESS, self.cls("Verified", vd.ERROR, vd.PROVEN))
        self.assertEqual(vd.HARNESS, self.cls("Verified", vd.PROVEN, vd.ERROR))

    def test_counterexample_sets(self):
        other = ("f", "caller", 2)
        self.assertEqual(
            vd.WEAKER, self.cls("VerifyFailed", vd.REFUTED, vd.REFUTED, [K], [])
        )
        self.assertEqual(
            vd.WEAKER, self.cls("VerifyFailed", vd.REFUTED, vd.REFUTED, [K], [other])
        )
        self.assertEqual(
            vd.MORE_PRECISE,
            self.cls("VerifyFailed", vd.REFUTED, vd.REFUTED, [K], [K, other]),
        )


class VerdictTest(unittest.TestCase):
    def test_status_mapping(self):
        self.assertEqual(vd.PROVEN, vd.verdict_of({"status": "Verified"}))
        self.assertEqual(vd.SKIPPED, vd.verdict_of({"status": "Skipped"}))
        cex = {"function": "f", "blame": "Callee", "vow_id": 1}
        self.assertEqual(
            vd.REFUTED,
            vd.verdict_of({"status": "VerifyFailed", "counterexamples": [cex]}),
        )
        self.assertEqual(
            vd.INCONCLUSIVE,
            vd.verdict_of(
                {
                    "status": "VerifyFailed",
                    "verify_status": "timeout",
                    "counterexamples": [],
                }
            ),
        )
        self.assertEqual(vd.ERROR, vd.verdict_of(None))
        self.assertEqual(vd.ERROR, vd.verdict_of({"status": "CompileFailed"}))


def fake_row(cls):
    side = {
        "verdict": vd.PROVEN,
        "seconds": 0.0,
        "status": "Verified",
        "verify_status": None,
        "counterexamples": [],
    }
    return {
        "fixture": "verify/x",
        "truth": "Verified",
        "known_gap": False,
        "class": cls,
        "detail": None if cls == vd.MATCH else "d",
        "esbmc": side,
        "native": side,
    }


class ReportTest(unittest.TestCase):
    def test_exit_codes(self):
        def code(*classes):
            return vd.exit_code(vd.build_report("vowc", [fake_row(c) for c in classes]))

        self.assertEqual(0, code(vd.MATCH, vd.MORE_PRECISE))
        self.assertEqual(1, code(vd.MATCH, vd.WEAKER))
        self.assertEqual(1, code(vd.SOUNDNESS))
        self.assertEqual(2, code(vd.WEAKER, vd.HARNESS))

    def test_summary_counts(self):
        report = vd.build_report("vowc", [fake_row(vd.MATCH), fake_row(vd.WEAKER)])
        self.assertEqual(
            {
                "total": 2,
                "match": 1,
                "more_precise": 0,
                "weaker": 1,
                "soundness": 0,
                "harness": 0,
            },
            report["summary"],
        )


class RunBackendTest(unittest.TestCase):
    def test_hung_verifier_is_an_inconclusive_timeout(self):
        expired = vd.subprocess.TimeoutExpired(cmd="vowc", timeout=1)
        with mock.patch.object(vd.subprocess, "run", side_effect=expired):
            result, _ = vd.run_backend("vowc", "native", "x.vow", 1)
        self.assertEqual(vd.INCONCLUSIVE, vd.verdict_of(result))

    def test_non_json_output_is_an_error(self):
        proc = mock.Mock(stdout="not json", stderr="boom")
        with (
            mock.patch.object(vd.subprocess, "run", return_value=proc),
            mock.patch.object(sys, "stderr", io.StringIO()),
        ):
            result, _ = vd.run_backend("vowc", "native", "x.vow", 1)
        self.assertEqual(vd.ERROR, vd.verdict_of(result))

    def test_backend_and_timeout_are_passed_to_vowc(self):
        proc = mock.Mock(stdout='{"status":"Verified"}', stderr="")
        with mock.patch.object(vd.subprocess, "run", return_value=proc) as run:
            vd.run_backend("vowc", "native", "x.vow", 7)
        args = run.call_args.args[0]
        self.assertEqual(
            ["--backend", "native", "--timeout", "7"],
            args[3:7] if args[2] == "--no-cache" else None,
        )


class MainTest(unittest.TestCase):
    def run_main(self, argv, results):
        exp = vd.verify_eval.Expect("tests/verify/x.vow", "Verified")
        out = io.StringIO()
        with (
            mock.patch.object(vd, "preflight", return_value=[]),
            mock.patch.object(
                vd.verify_eval, "collect", return_value=[("verify", exp)]
            ),
            mock.patch.object(
                vd, "run_backend", side_effect=lambda v, b, p, t: (results[b], 0.0)
            ),
            mock.patch.object(sys, "stdout", out),
            mock.patch.object(sys, "stderr", io.StringIO()),
        ):
            code = vd.main(argv)
        return code, out.getvalue()

    def test_report_is_json_and_weaker_row_fails(self):
        code, out = self.run_main(
            [], {"esbmc": {"status": "Verified"}, "native": {"status": "Skipped"}}
        )
        report = json.loads(out)
        self.assertEqual(1, code)
        self.assertEqual(vd.WEAKER, report["rows"][0]["class"])
        self.assertEqual("Skipped", report["rows"][0]["native"]["status"])

    def test_agreement_passes(self):
        code, out = self.run_main(
            [], {"esbmc": {"status": "Verified"}, "native": {"status": "Verified"}}
        )
        self.assertEqual(0, code)
        self.assertEqual(1, json.loads(out)["summary"]["match"])

    def test_missing_tools_are_a_harness_failure(self):
        with (
            mock.patch.object(vd.shutil, "which", return_value=None),
            mock.patch.object(sys, "stderr", io.StringIO()),
        ):
            self.assertEqual(2, vd.main(["--vowc", "/nonexistent/vowc"]))


if __name__ == "__main__":
    unittest.main()
