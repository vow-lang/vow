#!/usr/bin/env python3
"""Behavior tests for scripts/verify_diff.py."""

import io
import json
import os
import sys
import tempfile
import unittest
from unittest import mock

import verify_diff as vd

K = ("f", "Callee", 1)
OTHER = ("f", "Caller", 2)


class ClassifyTest(unittest.TestCase):
    def cls(self, truth, esbmc, native, ek=frozenset(), nk=frozenset()):
        return vd.classify(truth, esbmc, native, ek, nk)[0]

    def test_equal_verdicts_match(self):
        self.assertEqual(vd.MATCH, self.cls("Verified", vd.PROVEN, vd.PROVEN))
        self.assertEqual(vd.MATCH, self.cls("Skipped", vd.SKIPPED, vd.SKIPPED))
        self.assertEqual(
            vd.MATCH, self.cls("VerifyFailed", vd.REFUTED, vd.REFUTED, {K}, {K})
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

    def test_native_solver_failure_is_weaker_even_if_esbmc_is_inconclusive(self):
        for status, want in (
            ("panicked", vd.WEAKER),
            ("error", vd.WEAKER),
            ("crashed", vd.WEAKER),
            ("timeout", vd.MATCH),
        ):
            cls, _ = vd.classify(
                "Verified", vd.INCONCLUSIVE, vd.INCONCLUSIVE, native_status=status
            )
            self.assertEqual(want, cls)

    def test_counterexample_sets(self):
        def refuted(esbmc, native):
            return self.cls("VerifyFailed", vd.REFUTED, vd.REFUTED, esbmc, native)

        self.assertEqual(vd.WEAKER, refuted({K}, set()))
        self.assertEqual(vd.WEAKER, refuted({K}, {OTHER}))
        self.assertEqual(vd.MORE_PRECISE, refuted({K}, {K, OTHER}))


class CapacityArtifactTest(unittest.TestCase):
    """ESBMC refuting only through its own model capacity (#1426)."""

    def cls(self, truth, esbmc, native, reason, only):
        return vd.classify(
            truth,
            esbmc,
            native,
            frozenset(),
            frozenset(),
            None,
            reason,
            only,
        )

    def test_documented_capacity_refutation_is_an_explained_extra_proof(self):
        cls, detail = self.cls(
            "VerifyFailed", vd.REFUTED, vd.PROVEN, "push_str cap", True
        )
        self.assertEqual(vd.MORE_PRECISE, cls)
        self.assertIn("model capacity", detail)
        self.assertIn("push_str cap", detail)

    def test_without_the_directive_the_row_stays_a_soundness_finding(self):
        self.assertEqual(
            vd.SOUNDNESS, self.cls("VerifyFailed", vd.REFUTED, vd.PROVEN, None, True)[0]
        )

    def test_a_directive_does_not_excuse_a_refutation_that_is_not_a_capacity_one(self):
        self.assertEqual(
            vd.SOUNDNESS,
            self.cls("VerifyFailed", vd.REFUTED, vd.PROVEN, "why", False)[0],
        )

    def test_the_directive_excuses_only_a_native_proof(self):
        self.assertEqual(
            vd.WEAKER, self.cls("VerifyFailed", vd.REFUTED, vd.SKIPPED, "why", True)[0]
        )
        self.assertEqual(
            vd.MATCH, self.cls("VerifyFailed", vd.REFUTED, vd.REFUTED, "why", True)[0]
        )

    def test_capacity_only_needs_every_counterexample_to_be_a_capacity_assertion(self):
        cap = {"violation": "string exceeded the verifier's internal capacity limit"}
        other = {"violation": "index out of bounds"}
        self.assertTrue(vd.capacity_only({"counterexamples": [cap]}))
        self.assertTrue(vd.capacity_only({"counterexamples": [cap, cap]}))
        self.assertFalse(vd.capacity_only({"counterexamples": [cap, other]}))
        self.assertFalse(vd.capacity_only({"counterexamples": []}))
        self.assertFalse(vd.capacity_only(None))

    def test_directive_parsing(self):
        import tempfile

        with tempfile.TemporaryDirectory() as d:
            path = d + "/f.vow"
            with open(path, "w") as fh:
                fh.write(
                    '// TEST: category x\n// TEST: esbmc-capacity-artifact "a cap"\n'
                )
            self.assertEqual("a cap", vd.capacity_artifact(path))
            with open(path, "w") as fh:
                fh.write("// TEST: category x\n")
            self.assertIsNone(vd.capacity_artifact(path))
            with open(path, "w") as fh:
                fh.write("// TEST: esbmc-capacity-artifact\n")
            with self.assertRaisesRegex(ValueError, "malformed esbmc artifact"):
                vd.capacity_artifact(path)


def skip_diag(fn, severity="warning", code="VerificationSkipped"):
    return {
        "error_code": code,
        "severity": severity,
        "message": f"skipped verification of `{fn}`: not modelable",
    }


class ModelArtifactTest(unittest.TestCase):
    KEY = ("eq_transitive", "Callee", 2)

    def cls(self, truth, esbmc_keys, artifacts, esbmc=vd.REFUTED, native=vd.PROVEN):
        return vd.classify(
            truth,
            esbmc,
            native,
            frozenset(esbmc_keys),
            frozenset(),
            None,
            None,
            False,
            artifacts,
        )

    def test_declared_model_counterexample_is_an_explained_extra_proof(self):
        cls, detail = self.cls("Verified", [self.KEY], {self.KEY: "spurious string_eq"})
        self.assertEqual(vd.MORE_PRECISE, cls)
        self.assertIn("spurious string_eq", detail)

    def test_an_undeclared_counterexample_stays_a_soundness_finding(self):
        other = ("other_fn", "Callee", 0)
        cls, _ = self.cls(
            "Verified", [self.KEY, other], {self.KEY: "spurious string_eq"}
        )
        self.assertEqual(vd.SOUNDNESS, cls)

    def test_no_counterexample_key_is_never_excused(self):
        cls, _ = self.cls("Verified", [], {self.KEY: "x"})
        self.assertEqual(vd.SOUNDNESS, cls)

    def test_the_directive_does_not_override_a_corpus_label_of_incorrect(self):
        cls, _ = self.cls("VerifyFailed", [self.KEY], {self.KEY: "x"})
        self.assertEqual(vd.SOUNDNESS, cls)

    def test_the_directive_excuses_only_a_native_proof(self):
        cls, _ = self.cls("Verified", [self.KEY], {self.KEY: "x"}, native=vd.SKIPPED)
        self.assertEqual(vd.WEAKER, cls)


class FootprintTest(unittest.TestCase):
    def cls(self, esbmc_skipped, native_skipped, verdict=vd.SKIPPED):
        return vd.classify(
            "Skipped",
            verdict,
            verdict,
            esbmc_skipped=frozenset(esbmc_skipped),
            native_skipped=frozenset(native_skipped),
        )

    def test_same_skipped_functions_match(self):
        self.assertEqual(vd.MATCH, self.cls({"a", "b"}, {"a", "b"})[0])

    def test_native_analysing_what_esbmc_skips_is_more_precise_not_a_match(self):
        cls, detail = self.cls({"a", "b", "c"}, {"c"})
        self.assertEqual(vd.MORE_PRECISE, cls)
        self.assertIn("['a', 'b']", detail)

    def test_native_skipping_what_esbmc_analyses_is_weaker(self):
        cls, detail = self.cls({"c"}, {"b", "c"})
        self.assertEqual(vd.WEAKER, cls)
        self.assertIn("['b']", detail)

    def test_weaker_wins_when_both_sides_differ(self):
        self.assertEqual(vd.WEAKER, self.cls({"a"}, {"b"})[0])

    def test_native_that_stopped_at_a_refutation_did_not_analyse_later_functions(self):
        keys = frozenset({("f", "Callee", 1)})
        cls, _ = vd.classify(
            "VerifyFailed",
            vd.REFUTED,
            vd.REFUTED,
            keys,
            keys,
            esbmc_skipped=frozenset({"g"}),
        )
        self.assertEqual(vd.MATCH, cls)

    def test_native_skipping_a_function_is_weaker_even_after_a_refutation(self):
        keys = frozenset({("f", "Callee", 1)})
        cls, _ = vd.classify(
            "VerifyFailed",
            vd.REFUTED,
            vd.REFUTED,
            keys,
            keys,
            native_skipped=frozenset({"g"}),
        )
        self.assertEqual(vd.WEAKER, cls)

    def test_inconclusive_native_did_not_analyse_later_functions_either(self):
        cls, _ = vd.classify(
            "Skipped",
            vd.INCONCLUSIVE,
            vd.INCONCLUSIVE,
            esbmc_skipped=frozenset({"g"}),
        )
        self.assertEqual(vd.MATCH, cls)

    def test_skipped_functions_reads_warnings_only(self):
        result = {
            "diagnostics": [
                skip_diag("low"),
                skip_diag("main", severity="note"),
                skip_diag("other", code="ArithOverflowReachable"),
                {"error_code": "VerificationSkipped", "severity": "warning"},
            ]
        }
        self.assertEqual(frozenset({"low"}), vd.skipped_functions(result))
        self.assertEqual(frozenset(), vd.skipped_functions(None))


class ClassifyOutcomesTest(unittest.TestCase):
    def write(self, tmp, text):
        path = os.path.join(tmp, "f.vow")
        with open(path, "w") as fh:
            fh.write(text)
        return path

    def outcome(self, status, *diags, cexes=()):
        return vd.summarise(
            {
                "status": status,
                "diagnostics": list(diags),
                "counterexamples": list(cexes),
            }
        )

    def test_wide_enum_payload_shape_is_not_a_match(self):
        esbmc = self.outcome(
            "Skipped", *[skip_diag(f) for f in ("low", "second", "ok", "call_ret")]
        )
        native = self.outcome("Skipped", skip_diag("call_ret"))
        cls, _ = vd.classify_outcomes("Skipped", "/nonexistent.vow", esbmc, native)
        self.assertEqual(vd.MORE_PRECISE, cls)

    def test_directives_of_the_fixture_are_honoured(self):
        cex = {"function": "f", "blame": "Callee", "vow_id": 2, "violation": "ensures"}
        esbmc = self.outcome("VerifyFailed", cexes=[cex])
        native = self.outcome("Verified")
        with tempfile.TemporaryDirectory() as tmp:
            bare = self.write(tmp, "// TEST: category x\n")
            self.assertEqual(
                vd.SOUNDNESS, vd.classify_outcomes("Verified", bare, esbmc, native)[0]
            )
            marked = self.write(
                tmp,
                '// TEST: esbmc-model-artifact fn="f" blame=Callee vow_id=2 "why"\n',
            )
            self.assertEqual(
                vd.MORE_PRECISE,
                vd.classify_outcomes("Verified", marked, esbmc, native)[0],
            )


class NativeCorpusTest(unittest.TestCase):
    def test_native_directories_are_collected_with_their_truth(self):
        truths = {}
        for group, exp in vd.collect_native(None):
            truths.setdefault(group, set()).add(exp.expected_status)
        self.assertEqual(
            {
                "verify-native/pass": {"Verified"},
                "verify-native/fail": {"VerifyFailed"},
                "verify-native/skip": {"Skipped"},
                "verify-native/unknown": {None},
            },
            truths,
        )

    def test_collect_covers_the_eval_corpus_and_the_native_one(self):
        groups = {group for group, _ in vd.collect(None)}
        self.assertTrue({"verify", "verify-fail", "verify-skip"} <= groups)
        self.assertIn("verify-native/pass", groups)

    def test_filter_applies_to_the_native_fixtures(self):
        names = [exp.name for _, exp in vd.collect_native("string_eq_exact")]
        self.assertEqual(["string_eq_exact"], names)


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
        self.assertEqual(1, code(vd.WEAKER, vd.HARNESS))
        self.assertEqual(1, code(vd.SOUNDNESS, vd.HARNESS))
        self.assertEqual(2, code(vd.MATCH, vd.HARNESS))

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
    def run_with(self, stdout="", returncode=0, expired=False):
        proc = mock.Mock(returncode=returncode, pid=4242)
        if expired:
            proc.communicate.side_effect = [
                vd.subprocess.TimeoutExpired(cmd="vowc", timeout=1),
                ("", ""),
            ]
        else:
            proc.communicate.return_value = (stdout, "boom")
        with (
            mock.patch.object(vd.subprocess, "Popen", return_value=proc) as popen,
            mock.patch.object(vd.os, "killpg") as killpg,
            mock.patch.object(sys, "stderr", io.StringIO()),
        ):
            result, _ = vd.run_backend("vowc", "native", "x.vow", 7, 1)
        return result, popen, killpg

    def test_hung_verifier_is_killed_as_a_group_and_times_out(self):
        result, _, killpg = self.run_with(expired=True)
        killpg.assert_called_once()
        self.assertEqual(vd.INCONCLUSIVE, vd.verdict_of(result))
        self.assertEqual("timeout", result["verify_status"])

    def test_signal_death_is_a_crash_not_a_harness_error(self):
        result, _, _ = self.run_with(returncode=-11)
        self.assertEqual(vd.INCONCLUSIVE, vd.verdict_of(result))
        self.assertEqual("crashed", result["verify_status"])

    def test_non_json_output_is_an_error(self):
        result, _, _ = self.run_with(stdout="not json", returncode=1)
        self.assertEqual(vd.ERROR, vd.verdict_of(result))

    def test_backend_and_timeout_are_passed_to_vowc(self):
        _, popen, _ = self.run_with(stdout='{"status":"Verified"}')
        args = popen.call_args.args[0]
        self.assertEqual(
            [
                "vowc",
                "verify",
                "--no-cache",
                "--backend",
                "native",
                "--timeout",
                "7",
                "x.vow",
            ],
            args,
        )
        self.assertTrue(popen.call_args.kwargs["start_new_session"])


def clause(fn, kind, vow_id, status, trivial=False, desc=""):
    return {
        "function": fn,
        "kind": kind,
        "vow_id": vow_id,
        "status": status,
        "trivially_satisfiable": trivial,
        "description": desc or f"{kind} x",
    }


def report(*clauses):
    return {"contracts": list(clauses)}


class ContractsTest(unittest.TestCase):
    def cmp(self, esbmc, native, truth="Verified"):
        return vd.compare_contracts(truth, esbmc, native)[0]

    def test_equal_reports_match(self):
        r = report(
            clause("f", "ensures", 1, "proven", True),
            clause("g", "requires", 0, "unknown"),
        )
        self.assertEqual(vd.MATCH, self.cmp(r, r))

    def test_native_proving_a_failed_clause_is_soundness(self):
        e = report(clause("f", "ensures", 1, "failed"))
        n = report(clause("f", "ensures", 1, "proven"))
        self.assertEqual(vd.SOUNDNESS, self.cmp(e, n, "VerifyFailed"))

    def test_native_proving_what_esbmc_leaves_open_is_more_precise(self):
        e = report(clause("f", "ensures", 1, "unknown"))
        n = report(clause("f", "ensures", 1, "proven"))
        self.assertEqual(vd.MORE_PRECISE, self.cmp(e, n, "VerifyFailed"))

    def test_native_skipping_a_proven_clause_is_weaker(self):
        e = report(clause("f", "ensures", 1, "proven"))
        n = report(clause("f", "ensures", 1, "skipped"))
        self.assertEqual(vd.WEAKER, self.cmp(e, n))

    def test_vacuity_must_agree(self):
        e = report(clause("f", "requires", 0, "vacuous", desc="requires a > 0"))
        n = report(clause("f", "requires", 0, "proven", desc="requires a > 0"))
        self.assertEqual(vd.WEAKER, self.cmp(e, n))
        n = report(clause("f", "requires", 0, "vacuous", desc="requires a > 0"))
        self.assertEqual(vd.MATCH, self.cmp(e, n))

    def test_entry_block_only_vacuity_label_is_a_known_esbmc_divergence(self):
        d = "requires a > 0 && a < 100"
        e = report(
            clause("f", "requires", 0, "vacuous", desc=d),
            clause("f", "ensures", 1, "vacuous"),
        )
        n = report(
            clause("f", "requires", 0, "proven", desc=d),
            clause("f", "ensures", 1, "proven"),
        )
        self.assertEqual(vd.MORE_PRECISE, self.cmp(e, n))

    def test_entry_block_only_vacuity_does_not_hide_native_failures(self):
        d = "requires a > 0 && a < 100"

        def pair(status, kind_status=None):
            return report(
                clause("f", "requires", 0, kind_status or status, desc=d),
                clause("f", "ensures", 1, status),
            )

        e = pair("vacuous")
        self.assertEqual(vd.WEAKER, self.cmp(e, pair("error")))
        self.assertEqual(vd.WEAKER, self.cmp(e, pair("failed"), "Verified"))
        self.assertEqual(vd.MORE_PRECISE, self.cmp(e, pair("failed"), "VerifyFailed"))
        self.assertEqual(vd.MORE_PRECISE, self.cmp(e, pair("proven")))

    def test_native_without_a_report_is_weaker(self):
        e = report(clause("f", "ensures", 1, "proven"))
        for why in ("timeout", "crashed"):
            n = {"status": "VerifyFailed", "verify_status": why, "counterexamples": []}
            self.assertEqual(vd.WEAKER, self.cmp(e, n))

    def test_native_vacuity_esbmc_does_not_see_is_weaker(self):
        e = report(clause("f", "ensures", 1, "proven"))
        n = report(clause("f", "ensures", 1, "vacuous"))
        self.assertEqual(vd.WEAKER, self.cmp(e, n))

    def test_trivial_flag_must_agree(self):
        e = report(clause("f", "ensures", 1, "proven", True))
        n = report(clause("f", "ensures", 1, "proven", False))
        self.assertEqual(vd.WEAKER, self.cmp(e, n))
        self.assertEqual(vd.WEAKER, self.cmp(n, e))

    def test_native_error_where_esbmc_is_inconclusive_is_weaker(self):
        e = report(clause("f", "ensures", 1, "unknown"))
        n = report(clause("f", "ensures", 1, "error"))
        self.assertEqual(vd.WEAKER, self.cmp(e, n))

    def test_abort_only_failure_proves_no_clause_is_not_weaker(self):
        e = report(clause("f", "ensures", 1, "proven"))
        n = report(clause("f", "ensures", 1, "unknown"))
        self.assertEqual(vd.WEAKER, vd.compare_contracts("Verified", e, n)[0])
        self.assertEqual(
            vd.MORE_PRECISE, vd.compare_contracts("Verified", e, n, {"f"})[0]
        )
        e = report(clause("f", "ensures", 1, "failed"))
        self.assertEqual(
            vd.MORE_PRECISE, vd.compare_contracts("VerifyFailed", e, n, {"f"})[0]
        )

    def test_aborting_functions_are_refuted_without_a_failed_clause(self):
        table = vd.clause_table(
            report(
                clause("f", "ensures", 1, "unknown"),
                clause("g", "ensures", 1, "failed"),
            )
        )
        cex = [
            {"fn": "f", "blame": "Caller", "vow_id": 0},
            {"fn": "g", "blame": "Callee", "vow_id": 1},
            {"fn": "h", "blame": "None", "vow_id": 4294967293},
        ]
        with mock.patch.object(vd.verify_eval, "actual_cex", return_value=cex):
            self.assertEqual({"f", "h"}, vd.aborting_functions({}, table))
        with mock.patch.object(vd.verify_eval, "actual_cex", return_value=[]):
            self.assertEqual(set(), vd.aborting_functions({}, table))

    def test_fixture_takes_the_worst_clause(self):
        e = report(
            clause("f", "ensures", 1, "proven"), clause("g", "ensures", 1, "failed")
        )
        n = report(
            clause("f", "ensures", 1, "skipped"), clause("g", "ensures", 1, "proven")
        )
        self.assertEqual(vd.SOUNDNESS, self.cmp(e, n, "VerifyFailed"))

    def test_different_clause_sets_or_missing_reports_are_harness_rows(self):
        e = report(clause("f", "ensures", 1, "proven"))
        n = report(clause("f", "ensures", 2, "proven"))
        self.assertEqual(vd.HARNESS, self.cmp(e, n))
        self.assertEqual(vd.HARNESS, self.cmp(None, e))

    def test_summary_text(self):
        r = report(
            clause("f", "ensures", 1, "proven", True),
            clause("f", "requires", 0, "proven"),
        )
        self.assertEqual("proven:1,proven+trivial:1", vd.clause_summary(r))


class TestCommandTests(unittest.TestCase):
    def one_test(self, status):
        return {"tests": [{"status": status}]}

    def test_verdicts(self):
        self.assertEqual(vd.PROVEN, vd.test_verdict(self.one_test("passed")))
        self.assertEqual(vd.PROVEN, vd.test_verdict(self.one_test("failed")))
        self.assertEqual(vd.REFUTED, vd.test_verdict(self.one_test("verify_failed")))
        self.assertEqual(vd.SKIPPED, vd.test_verdict(self.one_test("contract_skipped")))
        self.assertEqual(vd.ERROR, vd.test_verdict(self.one_test("compile_error")))
        self.assertEqual(vd.ERROR, vd.test_verdict({"tests": []}))

    def test_test_path_failing_a_correct_program_is_not_soundness(self):
        cls, _ = vd.classify_test(
            "Verified", self.one_test("verify_failed"), self.one_test("passed")
        )
        self.assertEqual(vd.MORE_PRECISE, cls)
        cls, _ = vd.classify_test(
            "VerifyFailed", self.one_test("verify_failed"), self.one_test("passed")
        )
        self.assertEqual(vd.SOUNDNESS, cls)

    def test_synthetic_timeout_is_inconclusive(self):
        hung = {
            "status": "VerifyFailed",
            "verify_status": "timeout",
            "counterexamples": [],
        }
        self.assertEqual(vd.INCONCLUSIVE, vd.test_verdict(hung))
        cls, _ = vd.classify_test("Verified", self.one_test("passed"), hung)
        self.assertEqual(vd.WEAKER, cls)

    def test_command_lines(self):
        self.assertEqual(
            ["v", "contracts", "--verify", "--no-cache", "--backend", "native"],
            vd.command_args("v", "contracts", "native", 7, "x.vow"),
        )
        self.assertEqual(
            ["v", "test", "--verify", "--backend", "native", "x.vow"],
            vd.command_args("v", "test", "native", 7, "x.vow"),
        )

    def run_with(self, command, results):
        exp = vd.verify_eval.Expect("tests/verify/x.vow", "Verified")
        with (
            mock.patch.object(vd, "function_count", return_value=1),
            mock.patch.object(
                vd,
                "run_backend",
                side_effect=lambda v, b, p, t, n, c="verify": (results[b], 0.0),
            ),
        ):
            return vd.diff_fixture("vowc", 7, "verify", exp, command)

    def test_test_row_classes(self):
        row = self.run_with(
            "test",
            {
                "esbmc": self.one_test("passed"),
                "native": self.one_test("verify_failed"),
            },
        )
        self.assertEqual(vd.WEAKER, row["class"])
        self.assertEqual("test", row["command"])
        row = self.run_with(
            "test",
            {"esbmc": self.one_test("passed"), "native": self.one_test("passed")},
        )
        self.assertEqual(vd.MATCH, row["class"])

    def test_contracts_row(self):
        r = report(clause("f", "ensures", 1, "proven"))
        row = self.run_with("contracts", {"esbmc": r, "native": r})
        self.assertEqual(vd.MATCH, row["class"])
        self.assertEqual("proven:1", row["native"]["verdict"])


class MainTest(unittest.TestCase):
    def run_main(self, argv, results):
        exp = vd.verify_eval.Expect("tests/verify/x.vow", "Verified")
        out = io.StringIO()
        with (
            mock.patch.object(vd, "preflight", return_value=[]),
            mock.patch.object(vd, "collect", return_value=[("verify", exp)]),
            mock.patch.object(vd, "function_count", return_value=1),
            mock.patch.object(
                vd,
                "run_backend",
                side_effect=lambda v, b, p, t, n, c="verify": (results[b], 0.0),
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

    def test_empty_selection_is_a_harness_failure(self):
        with (
            mock.patch.object(vd, "preflight", return_value=[]),
            mock.patch.object(vd, "collect", return_value=[]),
            mock.patch.object(sys, "stderr", io.StringIO()),
        ):
            self.assertEqual(2, vd.main(["--filter", "typo"]))

    def test_worker_exception_is_a_harness_failure(self):
        exp = vd.verify_eval.Expect("tests/verify/x.vow", "Verified")
        with (
            mock.patch.object(vd, "preflight", return_value=[]),
            mock.patch.object(vd, "collect", return_value=[("verify", exp)]),
            mock.patch.object(vd, "run_backend", side_effect=PermissionError("denied")),
            mock.patch.object(sys, "stderr", io.StringIO()),
        ):
            self.assertEqual(2, vd.main([]))

    def test_missing_tools_are_a_harness_failure(self):
        with (
            mock.patch.object(vd.shutil, "which", return_value=None),
            mock.patch.object(sys, "stderr", io.StringIO()),
        ):
            self.assertEqual(2, vd.main(["--vowc", "/nonexistent/vowc"]))


if __name__ == "__main__":
    unittest.main()
