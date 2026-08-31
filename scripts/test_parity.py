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


def run_parity_cli(mode, rust, self_hosted, rust_exit, self_exit, fixture_path=None):
    with tempfile.TemporaryDirectory() as directory:
        rust_path = Path(directory) / "rust.json"
        self_path = Path(directory) / "self.json"
        rust_path.write_text(json.dumps(rust))
        self_path.write_text(json.dumps(self_hosted))
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


class CompareJsonCharacterizationTest(unittest.TestCase):
    def test_process_exit_codes_must_match(self):
        errors = parity.compare_json(document(), document(), 0, 1)

        self.assertEqual(["exit code: 0 vs 1"], errors)

    def test_statuses_must_match(self):
        errors = parity.compare_json(
            document("CompileFailed"), document("Unverified"), 1, 1
        )

        self.assertEqual(["status: CompileFailed vs Unverified"], errors)

    def test_non_verify_failure_diagnostic_counts_must_match(self):
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
            [
                "counterexample[0].violation: "
                "x as u64 > 0 vs x as i64 > 0"
            ],
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
                "diagnostics: [('TautologicalComparison', None)] vs "
                "[('TypeMismatch', None)]"
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
        rust = document(
            "VerifyFailed",
            counterexamples=[
                {"function": "bad", "blame": "caller", "values": {"x": "-1"}}
            ],
        )
        self_hosted = document(
            "VerifyFailed",
            counterexamples=[
                {"function": "bad", "blame": "caller", "values": {"n": "-1"}}
            ],
        )

        errors = parity.compare_json(rust, self_hosted, 1, 1)

        self.assertEqual(
            ["counterexample[0].values: {'x': '-1'} vs {'n': '-1'}"], errors
        )

    def test_esbmc_internal_values_are_not_a_parity_contract(self):
        # The suffix is IR numbering chosen by independent lowerings. Comparing
        # it would turn #1140's internal noise into false parity failures.
        rust = document(
            "VerifyFailed",
            counterexamples=[
                {
                    "function": "bad",
                    "blame": "caller",
                    "values": {"x": "-1", "_esbmc_v12": "0"},
                }
            ],
        )
        self_hosted = document(
            "VerifyFailed",
            counterexamples=[
                {
                    "function": "bad",
                    "blame": "caller",
                    "values": {"x": "-1", "_esbmc_v99": "1"},
                }
            ],
        )

        self.assertEqual([], parity.compare_json(rust, self_hosted, 1, 1))

    def test_value_key_order_does_not_affect_parity(self):
        rust = document(
            "VerifyFailed",
            counterexamples=[
                {
                    "function": "bad",
                    "blame": "caller",
                    "values": {"x": "-1", "limit": "0"},
                }
            ],
        )
        self_hosted = document(
            "VerifyFailed",
            counterexamples=[
                {
                    "function": "bad",
                    "blame": "caller",
                    "values": {"limit": "0", "x": "-1"},
                }
            ],
        )

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
                "diagnostics: [('VowRequiresViolated', 'caller')] vs "
                "[('VowRequiresViolated', 'callee')]"
            ],
            errors,
        )


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
                "FAIL: error_code divergence tracked by #588 no longer diverges — "
                "update docs/equivalence/ledger.json",
            ),
            (completed.returncode, completed.stdout.strip()),
        )

    def test_fixed_ledger_entry_does_not_suppress_a_regression(self):
        rust = document("CompileFailed", diagnostics=[{"error_code": "TypeMismatch"}])
        self_hosted = document(
            "CompileFailed", diagnostics=[{"error_code": "UnexpectedToken"}]
        )

        completed = run_parity_cli(
            "error",
            rust,
            self_hosted,
            1,
            1,
            REPO_ROOT / "tests/error/undefined_function.vow",
        )

        self.assertEqual(
            (1, "FAIL: error codes: ['TypeMismatch'] vs ['UnexpectedToken']"),
            (completed.returncode, completed.stdout.strip()),
        )

    def test_ledger_entry_for_another_observable_does_not_suppress(self):
        rust = document("CompileFailed", diagnostics=[{"error_code": "TypeMismatch"}])
        self_hosted = document(
            "CompileFailed", diagnostics=[{"error_code": "UnexpectedToken"}]
        )

        completed = run_parity_cli(
            "error",
            rust,
            self_hosted,
            1,
            1,
            REPO_ROOT / "tests/run/euclid_gcd_swap_loop.vow",
        )

        self.assertEqual(
            (1, "FAIL: error codes: ['TypeMismatch'] vs ['UnexpectedToken']"),
            (completed.returncode, completed.stdout.strip()),
        )

    def test_fixture_outside_the_repo_is_compared_strictly(self):
        rust = document("CompileFailed", diagnostics=[{"error_code": "TypeMismatch"}])
        self_hosted = document(
            "CompileFailed", diagnostics=[{"error_code": "UnexpectedToken"}]
        )
        with tempfile.TemporaryDirectory() as directory:
            fixture_path = Path(directory) / "synthetic.vow"
            fixture_path.write_text("module Synthetic\n")
            completed = run_parity_cli("error", rust, self_hosted, 1, 1, fixture_path)

        self.assertEqual(
            (1, "FAIL: error codes: ['TypeMismatch'] vs ['UnexpectedToken']"),
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
                "error codes: ['UnexpectedToken', 'UnexpectedToken'] vs "
                "['UnexpectedToken', 'UnexpectedToken', 'UnexpectedToken']"
            ],
            errors,
        )


class ParityCliCharacterizationTest(unittest.TestCase):
    def test_malformed_json_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            rust_path = Path(directory) / "rust.json"
            self_path = Path(directory) / "self.json"
            rust_path.write_text("{")
            self_path.write_text(json.dumps(document()))

            completed = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "json",
                    str(rust_path),
                    str(self_path),
                    "0",
                    "0",
                ],
                check=False,
                capture_output=True,
                text=True,
            )

        self.assertEqual(1, completed.returncode)
        self.assertIn("FAIL: JSON parse error:", completed.stdout)

    def test_known_counterexample_value_divergence_is_a_loud_skip(self):
        rust = document(
            "VerifyFailed",
            counterexamples=[
                {"function": "bad", "blame": "caller", "values": {"x": "-1"}}
            ],
        )
        self_hosted = document(
            "VerifyFailed",
            counterexamples=[
                {"function": "bad", "blame": "caller", "values": {"n": "-1"}}
            ],
        )
        with tempfile.TemporaryDirectory() as directory:
            rust_path = Path(directory) / "rust.json"
            self_path = Path(directory) / "self.json"
            fixture_path = Path(directory) / "known.vow"
            rust_path.write_text(json.dumps(rust))
            self_path.write_text(json.dumps(self_hosted))
            fixture_path.write_text(
                '// TEST: known-cex-divergence 1139 "variable names differ"\n'
            )

            completed = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "json",
                    str(rust_path),
                    str(self_path),
                    "1",
                    "1",
                    str(fixture_path),
                ],
                check=False,
                capture_output=True,
                text=True,
            )

        self.assertEqual(
            (0, "SKIP: known counterexample divergence (#1139: variable names differ)"),
            (completed.returncode, completed.stdout.strip()),
        )

    def test_stale_counterexample_divergence_directive_fails(self):
        verified_failure = document(
            "VerifyFailed",
            counterexamples=[
                {"function": "bad", "blame": "caller", "values": {"x": "-1"}}
            ],
        )
        with tempfile.TemporaryDirectory() as directory:
            rust_path = Path(directory) / "rust.json"
            self_path = Path(directory) / "self.json"
            fixture_path = Path(directory) / "known.vow"
            rust_path.write_text(json.dumps(verified_failure))
            self_path.write_text(json.dumps(verified_failure))
            fixture_path.write_text(
                '// TEST: known-cex-divergence 1139 "variable names differ"\n'
            )

            completed = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "json",
                    str(rust_path),
                    str(self_path),
                    "1",
                    "1",
                    str(fixture_path),
                ],
                check=False,
                capture_output=True,
                text=True,
            )

        self.assertEqual(
            (
                1,
                "FAIL: known-cex-divergence (#1139: variable names differ) "
                "no longer reproduces — remove the directive",
            ),
            (completed.returncode, completed.stdout.strip()),
        )


if __name__ == "__main__":
    unittest.main()
