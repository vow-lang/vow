#!/usr/bin/env python3
"""Behavior tests for the Rust/self-hosted parity comparators."""

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import parity

SCRIPT = Path(__file__).with_name("parity.py")
REPO_ROOT = SCRIPT.parent.parent


def document(status="Unverified", **fields):
    return {"status": status, "diagnostics": [], "counterexamples": [], **fields}


def hard_failure(**values):
    """A VerifyFailed document whose single counterexample carries `values`."""
    return document(
        "VerifyFailed",
        counterexamples=[{"function": "bad", "blame": "caller", "values": values}],
    )


def run_parity_cli(
    mode,
    rust,
    self_hosted,
    rust_exit,
    self_exit,
    fixture_path=None,
    fixture_text=None,
):
    """Invoke parity.py as full_test.sh does, over freshly written JSON files.

    `fixture_text` writes a throwaway fixture carrying a `// TEST:` directive
    and passes it as the fixture argument; `fixture_path` names an existing one.
    """
    with tempfile.TemporaryDirectory() as directory:
        rust_path = Path(directory) / "rust.json"
        self_path = Path(directory) / "self.json"
        rust_path.write_text(_as_json(rust))
        self_path.write_text(_as_json(self_hosted))
        if fixture_text is not None:
            fixture_path = Path(directory) / "known.vow"
            fixture_path.write_text(fixture_text)
        args = [
            sys.executable,
            str(SCRIPT),
            mode,
            str(rust_path),
            str(self_path),
            str(rust_exit),
            str(self_exit),
        ]
        if fixture_path is not None:
            args.append(str(fixture_path))
        return subprocess.run(
            args,
            check=False,
            capture_output=True,
            text=True,
        )


def _as_json(document_or_text):
    """Serialize a document, or pass a raw string through so a test can send
    JSON the parser must reject."""
    if isinstance(document_or_text, str):
        return document_or_text
    return json.dumps(document_or_text)


class CompareJsonCharacterizationTest(unittest.TestCase):
    def test_process_exit_codes_must_match(self):
        errors = parity.compare_json(document(), document(), 0, 1)

        self.assertEqual(["exit code: 0 vs 1"], errors)

    def test_statuses_must_match(self):
        errors = parity.compare_json(
            document("CompileFailed"), document("Unverified"), 1, 1
        )

        self.assertEqual(["status: CompileFailed vs Unverified"], errors)

    def test_non_verify_failure_diagnostics_must_match(self):
        rust = document(diagnostics=[{"error_code": "A"}])
        self_hosted = document(diagnostics=[])

        errors = parity.compare_json(rust, self_hosted, 0, 0)

        self.assertEqual(["diagnostics: [('A', None)] vs []"], errors)

    def test_soft_verify_failures_must_agree_without_counterexamples(self):
        rust = document(
            "VerifyFailed",
            verify_status="timeout",
            function="left",
            counterexamples=[{"function": "f"}],
        )
        self_hosted = document(
            "VerifyFailed",
            verify_status="unknown",
            function="right",
            counterexamples=[{"function": "g"}, {"function": "h"}],
        )

        errors = parity.compare_json(rust, self_hosted, 1, 1)

        self.assertEqual(
            [
                "verify_status: timeout vs unknown",
                "rust soft VerifyFailed has 1 counterexamples",
                "self soft VerifyFailed has 2 counterexamples",
                "function: left vs right",
            ],
            errors,
        )

    def test_hard_verify_failures_require_counterexamples(self):
        errors = parity.compare_json(
            document("VerifyFailed"), document("VerifyFailed"), 1, 1
        )

        self.assertEqual(
            [
                "rust has no counterexamples for VerifyFailed",
                "self has no counterexamples for VerifyFailed",
            ],
            errors,
        )

    def test_hard_verify_failure_counterexample_fields_must_match(self):
        rust = document(
            "VerifyFailed",
            counterexamples=[{"function": "f", "blame": "caller"}],
        )
        self_hosted = document(
            "VerifyFailed",
            counterexamples=[{"function": "g", "blame": "callee"}],
        )

        errors = parity.compare_json(rust, self_hosted, 1, 1)

        self.assertEqual(
            [
                "counterexample[0].function: f vs g",
                "counterexample[0].blame: caller vs callee",
            ],
            errors,
        )

    def test_contract_counterexample_violation_must_match(self):
        rust = document(
            "VerifyFailed",
            counterexamples=[
                {"function": "f", "blame": "Caller", "violation": "x as u64 > 0"}
            ],
        )
        self_hosted = document(
            "VerifyFailed",
            counterexamples=[
                {"function": "f", "blame": "Caller", "violation": "x as i64 > 0"}
            ],
        )

        errors = parity.compare_json(rust, self_hosted, 1, 1)

        self.assertEqual(
            ["counterexample[0].violation: x as u64 > 0 vs x as i64 > 0"],
            errors,
        )

    def test_unattributed_counterexample_violation_is_not_compared(self):
        rust = document(
            "VerifyFailed",
            counterexamples=[
                {"function": "f", "blame": "none", "violation": "[Counterexample]"}
            ],
        )
        self_hosted = document(
            "VerifyFailed",
            counterexamples=[{"function": "f", "blame": "none", "violation": ""}],
        )

        self.assertEqual([], parity.compare_json(rust, self_hosted, 1, 1))

    def test_unknown_vow_ids_are_equivalent(self):
        rust = document(counterexamples=[{"function": "f", "vow_id": 0}])
        self_hosted = document(counterexamples=[{"function": "f", "vow_id": -1}])

        self.assertEqual([], parity.compare_json(rust, self_hosted, 0, 0))


class CompareJsonDiagnosticParityTest(unittest.TestCase):
    def test_error_codes_must_match_when_diagnostic_counts_match(self):
        rust = document(diagnostics=[{"error_code": "TautologicalComparison"}])
        self_hosted = document(diagnostics=[{"error_code": "TypeMismatch"}])

        errors = parity.compare_json(rust, self_hosted, 0, 0)

        self.assertEqual(
            [
                (
                    "diagnostics: [('TautologicalComparison', None)] vs "
                    "[('TypeMismatch', None)]"
                )
            ],
            errors,
        )

    def test_diagnostic_blame_must_match(self):
        rust = document(
            diagnostics=[{"error_code": "VowRequiresViolated", "blame": "caller"}]
        )
        self_hosted = document(
            diagnostics=[{"error_code": "VowRequiresViolated", "blame": "callee"}]
        )

        errors = parity.compare_json(rust, self_hosted, 1, 1)

        self.assertEqual(
            [
                (
                    "diagnostics: [('VowRequiresViolated', 'caller')] vs "
                    "[('VowRequiresViolated', 'callee')]"
                )
            ],
            errors,
        )

    def test_verify_failed_diagnostics_remain_outside_the_comparison(self):
        rust = document(
            "VerifyFailed",
            diagnostics=[{"error_code": "VowRequiresViolated", "blame": "caller"}],
            counterexamples=[{"function": "f", "blame": "caller"}],
        )
        self_hosted = document(
            "VerifyFailed",
            diagnostics=[],
            counterexamples=[{"function": "f", "blame": "caller"}],
        )

        self.assertEqual([], parity.compare_json(rust, self_hosted, 1, 1))


class CompareJsonCounterexampleValuesTest(unittest.TestCase):
    def test_source_level_counterexample_values_must_match(self):
        errors = parity.compare_json(hard_failure(x="-1"), hard_failure(n="-1"), 1, 1)

        self.assertEqual(
            ["counterexample[0].values: {'x': '-1'} vs {'n': '-1'}"], errors
        )

    def test_esbmc_internal_values_are_not_a_parity_contract(self):
        # `$esbmc$*` names are ESBMC's own temporaries, not the agent-facing
        # CEGIS payload the two compilers owe each other. Their values track
        # internal encoding choices, so comparing them would bind parity to
        # something neither compiler promises.
        rust = hard_failure(**{"x": "-1", "$esbmc$v12": "0"})
        self_hosted = hard_failure(**{"x": "-1", "$esbmc$v99": "1"})

        self.assertEqual([], parity.compare_json(rust, self_hosted, 1, 1))

    def test_source_values_using_the_old_internal_prefix_must_match(self):
        rust = hard_failure(_esbmc_x="1")
        self_hosted = hard_failure(_esbmc_x="2")

        errors = parity.compare_json(rust, self_hosted, 1, 1)

        self.assertEqual(
            ["counterexample[0].values: {'_esbmc_x': '1'} vs {'_esbmc_x': '2'}"],
            errors,
        )

    def test_values_are_compared_beyond_the_first_hard_failure_counterexample(self):
        # The CEGIS payload of a second violated contract is no less load-bearing
        # than the first; comparing only [0] would hide it.
        rust = document(
            "VerifyFailed",
            counterexamples=[
                {"function": "first", "blame": "caller", "values": {"x": "1"}},
                {"function": "second", "blame": "callee", "values": {"y": "2"}},
            ],
        )
        self_hosted = document(
            "VerifyFailed",
            counterexamples=[
                {"function": "first", "blame": "caller", "values": {"x": "1"}},
                {"function": "second", "blame": "callee", "values": {"y": "3"}},
            ],
        )

        errors = parity.compare_json(rust, self_hosted, 1, 1)

        self.assertEqual(["counterexample[1].values: {'y': '2'} vs {'y': '3'}"], errors)

    def test_value_key_order_does_not_affect_parity(self):
        rust = hard_failure(x="-1", limit="0")
        self_hosted = hard_failure(limit="0", x="-1")

        self.assertEqual([], parity.compare_json(rust, self_hosted, 1, 1))

    def test_values_are_compared_for_each_non_failure_counterexample(self):
        rust = document(
            counterexamples=[
                {"function": "first", "vow_id": 3, "values": {"x": "1"}},
                {"function": "second", "vow_id": 4, "values": {"y": "2"}},
            ]
        )
        self_hosted = document(
            counterexamples=[
                {"function": "first", "vow_id": 3, "values": {"x": "1"}},
                {"function": "second", "vow_id": 4, "values": {"y": "3"}},
            ]
        )

        errors = parity.compare_json(rust, self_hosted, 0, 0)

        self.assertEqual(["counterexample[1].values: {'y': '2'} vs {'y': '3'}"], errors)


class CompareJsonCounterexampleCountTest(unittest.TestCase):
    def test_non_failure_count_mismatch_message_is_stable(self):
        rust = document(counterexamples=[{"function": "first"}, {"function": "second"}])
        self_hosted = document(counterexamples=[{"function": "first"}])

        errors = parity.compare_json(rust, self_hosted, 0, 0)

        self.assertEqual(["counterexamples count: 2 vs 1"], errors)

    def test_hard_failure_count_mismatch_is_an_error(self):
        rust = document(
            "VerifyFailed",
            counterexamples=[
                {"function": "first", "blame": "caller"},
                {"function": "second", "blame": "callee"},
            ],
        )
        self_hosted = document(
            "VerifyFailed",
            counterexamples=[{"function": "first", "blame": "caller"}],
        )

        errors = parity.compare_json(rust, self_hosted, 1, 1)

        self.assertEqual(["counterexamples count: 2 vs 1"], errors)


class CompareErrorCharacterizationTest(unittest.TestCase):
    def test_both_compilers_must_reject(self):
        rust = document("CompileFailed", diagnostics=[{"error_code": "A"}])
        self_hosted = document("CompileFailed", diagnostics=[{"error_code": "A"}])

        errors = parity.compare_error(rust, self_hosted, 0, 0)

        self.assertEqual(
            [
                "rust exited 0, expected failure",
                "self exited 0, expected failure",
            ],
            errors,
        )

    def test_both_compilers_must_report_compile_failed(self):
        rust = document("Unverified", diagnostics=[{"error_code": "A"}])
        self_hosted = document("Verified", diagnostics=[{"error_code": "A"}])

        errors = parity.compare_error(rust, self_hosted, 1, 1)

        self.assertEqual(
            [
                "rust status=Unverified, expected CompileFailed",
                "self status=Verified, expected CompileFailed",
            ],
            errors,
        )

    def test_both_compilers_must_emit_a_diagnostic(self):
        errors = parity.compare_error(
            document("CompileFailed"), document("CompileFailed"), 1, 1
        )

        self.assertEqual(["rust has no diagnostics", "self has no diagnostics"], errors)


class CompareTestTest(unittest.TestCase):
    @staticmethod
    def suite(status="TestsPassed", total=2, tests=None):
        return document(
            status,
            total=total,
            tests=(
                [
                    {"name": "test_arith", "status": "passed"},
                    {"name": "test_parser", "status": "passed"},
                ]
                if tests is None
                else tests
            ),
        )

    def test_passing_suites_with_the_same_test_results_agree(self):
        rust = self.suite()
        self_hosted = self.suite(
            tests=list(reversed(rust["tests"])),
        )

        self.assertEqual([], parity.compare_test(rust, self_hosted, 0, 0))

    def test_discovered_test_counts_must_match(self):
        errors = parity.compare_test(
            self.suite(total=2),
            self.suite(
                total=1,
                tests=[{"name": "test_arith", "status": "passed"}],
            ),
            0,
            0,
        )

        self.assertIn("total: 2 vs 1", errors)

    def test_both_suites_must_report_tests_passed(self):
        errors = parity.compare_test(
            self.suite("TestsFailed"), self.suite("TestsFailed"), 1, 1
        )

        self.assertIn("rust status=TestsFailed, expected TestsPassed", errors)
        self.assertIn("self status=TestsFailed, expected TestsPassed", errors)

    def test_each_process_must_exit_zero(self):
        errors = parity.compare_test(self.suite(), self.suite(), 3, 4)

        self.assertIn("rust exited 3, expected 0", errors)
        self.assertIn("self exited 4, expected 0", errors)

    def test_an_empty_suite_is_never_parity(self):
        # Two compilers that discovered nothing agree on every observable, so
        # equality alone would let a silently broken `vow test` pass the
        # blocking gate. Discovering zero tests is itself the failure.
        empty = self.suite(total=0, tests=[])

        errors = parity.compare_test(empty, empty, 0, 0)

        self.assertIn("rust total=0, expected a non-empty suite", errors)

    def test_a_malformed_test_entry_is_a_parity_error_not_a_crash(self):
        # A suite result missing `name` mixes None with str; sorting without an
        # order that tolerates both aborts the CI step with a traceback rather
        # than reporting the divergence.
        malformed = self.suite(
            tests=[{"status": "passed"}, {"name": "test_parser", "status": "passed"}]
        )

        errors = parity.compare_test(self.suite(), malformed, 0, 0)

        self.assertTrue(errors)
        self.assertIn("tests:", " ".join(errors))

    def test_only_the_differing_tests_are_reported(self):
        # The full lists would be kilobytes of log for `compiler/`.
        errors = parity.compare_test(
            self.suite(),
            self.suite(
                tests=[
                    {"name": "test_arith", "status": "passed"},
                    {"name": "test_parser", "status": "failed"},
                ]
            ),
            0,
            0,
        )

        self.assertEqual(
            [
                "tests: rust-only [('test_parser', 'passed')] "
                "vs self-only [('test_parser', 'failed')]"
            ],
            errors,
        )

    def test_cli_exposes_test_mode_without_a_fixture(self):
        completed = run_parity_cli("test", self.suite(), self.suite(), 0, 0)

        self.assertEqual((0, "OK"), (completed.returncode, completed.stdout.strip()))


class CompareErrorCodeParityTest(unittest.TestCase):
    def test_rejections_with_different_error_codes_fail(self):
        rust = document(
            "CompileFailed",
            diagnostics=[{"error_code": "TautologicalComparison"}],
        )
        self_hosted = document(
            "CompileFailed", diagnostics=[{"error_code": "TypeMismatch"}]
        )

        errors = parity.compare_error(rust, self_hosted, 1, 1)

        self.assertEqual(
            ["error codes: ['TautologicalComparison'] vs ['TypeMismatch']"],
            errors,
        )

    def test_active_ledger_entry_is_a_loud_skip(self):
        rust = document(
            "CompileFailed", diagnostics=[{"error_code": "LinearTypeViolation"}]
        )
        self_hosted = document(
            "CompileFailed", diagnostics=[{"error_code": "RegionLinear"}]
        )

        completed = run_parity_cli(
            "error",
            rust,
            self_hosted,
            1,
            1,
            REPO_ROOT / "tests/error/linear_region_unconsumed.vow",
        )

        self.assertEqual(
            (0, "SKIP: known error-code divergence (#588)"),
            (completed.returncode, completed.stdout.strip()),
        )

    def test_active_ledger_entry_does_not_hide_a_different_code_gap(self):
        rust = document(
            "CompileFailed", diagnostics=[{"error_code": "LinearTypeViolation"}]
        )
        self_hosted = document(
            "CompileFailed", diagnostics=[{"error_code": "TypeMismatch"}]
        )

        completed = run_parity_cli(
            "error",
            rust,
            self_hosted,
            1,
            1,
            REPO_ROOT / "tests/error/linear_region_unconsumed.vow",
        )

        self.assertEqual(
            (
                1,
                "FAIL: error codes: ['LinearTypeViolation'] vs ['TypeMismatch']",
            ),
            (completed.returncode, completed.stdout.strip()),
        )

    def test_stale_active_ledger_entry_fails(self):
        rejection = document(
            "CompileFailed", diagnostics=[{"error_code": "LinearTypeViolation"}]
        )

        completed = run_parity_cli(
            "error",
            rejection,
            rejection,
            1,
            1,
            REPO_ROOT / "tests/error/linear_region_unconsumed.vow",
        )

        self.assertEqual(
            (
                1,
                (
                    "FAIL: error_code divergence tracked by #588 no longer diverges — "
                    "update docs/equivalence/ledger.json"
                ),
            ),
            (completed.returncode, completed.stdout.strip()),
        )

    def test_only_an_active_error_code_entry_suppresses(self):
        # Every other shape of fixture — an entry marked fixed, one tracking a
        # different observable, or a file the ledger cannot key at all — must
        # leave the same divergence reported.
        rust = document("CompileFailed", diagnostics=[{"error_code": "TypeMismatch"}])
        self_hosted = document(
            "CompileFailed", diagnostics=[{"error_code": "UnexpectedToken"}]
        )
        with tempfile.TemporaryDirectory() as directory:
            outside_repo = Path(directory) / "synthetic.vow"
            outside_repo.write_text("module Synthetic\n")
            unsuppressed = {
                "fixed ledger entry": REPO_ROOT / "tests/error/undefined_function.vow",
                "entry for another observable": (
                    REPO_ROOT / "tests/run/euclid_gcd_swap_loop.vow"
                ),
                "fixture outside the repo": outside_repo,
                "no fixture at all": None,
            }
            for reason, fixture_path in unsuppressed.items():
                with self.subTest(reason=reason):
                    completed = run_parity_cli(
                        "error", rust, self_hosted, 1, 1, fixture_path
                    )

                    self.assertEqual(
                        (
                            1,
                            (
                                "FAIL: error codes: ['TypeMismatch'] vs "
                                "['UnexpectedToken']"
                            ),
                        ),
                        (completed.returncode, completed.stdout.strip()),
                    )

    def test_duplicate_error_code_counts_must_match(self):
        rust = document(
            "CompileFailed",
            diagnostics=[
                {"error_code": "UnexpectedToken"},
                {"error_code": "UnexpectedToken"},
            ],
        )
        self_hosted = document(
            "CompileFailed",
            diagnostics=[
                {"error_code": "UnexpectedToken"},
                {"error_code": "UnexpectedToken"},
                {"error_code": "UnexpectedToken"},
            ],
        )

        errors = parity.compare_error(rust, self_hosted, 1, 1)

        self.assertEqual(
            [
                (
                    "error codes: ['UnexpectedToken', 'UnexpectedToken'] vs "
                    "['UnexpectedToken', 'UnexpectedToken', 'UnexpectedToken']"
                )
            ],
            errors,
        )


KNOWN_CEX_FIXTURE = (
    '// TEST: known-cex-divergence 1139 "variable names differ" '
    "rust-name=x self-name=n\n"
)
KNOWN_CEX_COUNT_FIXTURE = (
    '// TEST: known-cex-count-divergence 1155 "Rust stops after first failure"\n'
)
KNOWN_CEX_COMBINED_FIXTURE = KNOWN_CEX_FIXTURE + KNOWN_CEX_COUNT_FIXTURE


class ParityCliCharacterizationTest(unittest.TestCase):
    def test_malformed_json_fails_closed(self):
        completed = run_parity_cli("json", "{", document(), 0, 0)

        self.assertEqual(1, completed.returncode)
        self.assertIn("FAIL: JSON parse error:", completed.stdout)

    def test_known_counterexample_value_divergence_is_a_loud_skip(self):
        completed = run_parity_cli(
            "json",
            hard_failure(x="-1"),
            hard_failure(n="-1"),
            1,
            1,
            fixture_text=KNOWN_CEX_FIXTURE,
        )

        self.assertEqual(
            (0, "SKIP: known counterexample divergence (#1139: variable names differ)"),
            (completed.returncode, completed.stdout.strip()),
        )

    def test_known_label_divergence_does_not_hide_a_corrupted_value(self):
        completed = run_parity_cli(
            "json",
            hard_failure(x="-1"),
            hard_failure(n="42"),
            1,
            1,
            fixture_text=KNOWN_CEX_FIXTURE,
        )

        self.assertEqual(
            (
                1,
                "FAIL: counterexample[0].values: {'x': '-1'} vs {'n': '42'}",
            ),
            (completed.returncode, completed.stdout.strip()),
        )

    def test_known_value_divergence_does_not_suppress_a_count_mismatch(self):
        rust = document(
            "VerifyFailed",
            counterexamples=[
                {"function": "first", "blame": "caller"},
                {"function": "second", "blame": "callee"},
            ],
        )
        self_hosted = document(
            "VerifyFailed",
            counterexamples=[{"function": "first", "blame": "caller"}],
        )

        completed = run_parity_cli(
            "json",
            rust,
            self_hosted,
            1,
            1,
            fixture_text=KNOWN_CEX_FIXTURE,
        )

        self.assertEqual(
            (1, "FAIL: counterexamples count: 2 vs 1"),
            (completed.returncode, completed.stdout.strip()),
        )

    def test_known_counterexample_count_divergence_is_a_loud_skip(self):
        rust = document(
            "VerifyFailed",
            counterexamples=[{"function": "first", "blame": "caller"}],
        )
        self_hosted = document(
            "VerifyFailed",
            counterexamples=[
                {"function": "first", "blame": "caller"},
                {"function": "second", "blame": "callee"},
            ],
        )

        completed = run_parity_cli(
            "json",
            rust,
            self_hosted,
            1,
            1,
            fixture_text=KNOWN_CEX_COUNT_FIXTURE,
        )

        self.assertEqual(
            (
                0,
                (
                    "SKIP: known counterexample-count divergence "
                    "(#1155: Rust stops after first failure)"
                ),
            ),
            (completed.returncode, completed.stdout.strip()),
        )

    def test_known_counterexample_directives_compose_by_observable(self):
        rust = document(
            "VerifyFailed",
            counterexamples=[
                {
                    "function": "first",
                    "blame": "caller",
                    "values": {"x": "-1"},
                },
                {"function": "second", "blame": "callee"},
            ],
        )
        self_hosted = document(
            "VerifyFailed",
            counterexamples=[
                {
                    "function": "first",
                    "blame": "caller",
                    "values": {"n": "-1"},
                }
            ],
        )

        completed = run_parity_cli(
            "json",
            rust,
            self_hosted,
            1,
            1,
            fixture_text=KNOWN_CEX_COMBINED_FIXTURE,
        )

        self.assertEqual(
            (
                0,
                (
                    "SKIP: known counterexample divergence "
                    "(#1139: variable names differ); "
                    "known counterexample-count divergence "
                    "(#1155: Rust stops after first failure)"
                ),
            ),
            (completed.returncode, completed.stdout.strip()),
        )

    def test_composed_directives_do_not_hide_a_stale_observable(self):
        completed = run_parity_cli(
            "json",
            hard_failure(x="-1"),
            hard_failure(n="-1"),
            1,
            1,
            fixture_text=KNOWN_CEX_COMBINED_FIXTURE,
        )

        self.assertEqual(
            (
                1,
                (
                    "FAIL: known-cex-count-divergence "
                    "(#1155: Rust stops after first failure) no longer reproduces — "
                    "remove the directive"
                ),
            ),
            (completed.returncode, completed.stdout.strip()),
        )

    def test_stale_counterexample_count_divergence_directive_fails(self):
        verified_failure = hard_failure(x="-1")

        completed = run_parity_cli(
            "json",
            verified_failure,
            verified_failure,
            1,
            1,
            fixture_text=KNOWN_CEX_COUNT_FIXTURE,
        )

        self.assertEqual(
            (
                1,
                (
                    "FAIL: known-cex-count-divergence "
                    "(#1155: Rust stops after first failure) no longer reproduces — "
                    "remove the directive"
                ),
            ),
            (completed.returncode, completed.stdout.strip()),
        )

    def test_count_directive_is_dormant_without_a_hard_verification_failure(self):
        no_counterexamples = document("Unverified")

        completed = run_parity_cli(
            "json",
            no_counterexamples,
            no_counterexamples,
            0,
            0,
            fixture_text=KNOWN_CEX_COUNT_FIXTURE,
        )

        self.assertEqual((0, "OK"), (completed.returncode, completed.stdout.strip()))

    def test_count_directive_cannot_suppress_a_non_failure_count_mismatch(self):
        rust = document(counterexamples=[{"function": "first"}, {"function": "second"}])
        self_hosted = document(counterexamples=[{"function": "first"}])

        completed = run_parity_cli(
            "json",
            rust,
            self_hosted,
            0,
            0,
            fixture_text=KNOWN_CEX_COUNT_FIXTURE,
        )

        self.assertEqual(
            (1, "FAIL: counterexamples count: 2 vs 1"),
            (completed.returncode, completed.stdout.strip()),
        )

    def test_stale_counterexample_divergence_directive_fails(self):
        verified_failure = hard_failure(x="-1")

        completed = run_parity_cli(
            "json",
            verified_failure,
            verified_failure,
            1,
            1,
            fixture_text=KNOWN_CEX_FIXTURE,
        )

        self.assertEqual(
            (
                1,
                (
                    "FAIL: known-cex-divergence (#1139: variable names differ) "
                    "no longer reproduces — remove the directive"
                ),
            ),
            (completed.returncode, completed.stdout.strip()),
        )

    def test_directive_is_not_stale_when_no_counterexamples_were_compared(self):
        # The same fixture is reachable through invocations that produce no
        # counterexamples (a --no-verify build). Those runs never compared
        # values, so they cannot testify that the directive is stale.
        no_counterexamples = document("Unverified")

        completed = run_parity_cli(
            "json",
            no_counterexamples,
            no_counterexamples,
            0,
            0,
            fixture_text=KNOWN_CEX_FIXTURE,
        )

        self.assertEqual((0, "OK"), (completed.returncode, completed.stdout.strip()))


if __name__ == "__main__":
    unittest.main()
