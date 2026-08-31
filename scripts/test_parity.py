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


def document(status="Unverified", **fields):
    return {"status": status, "diagnostics": [], "counterexamples": [], **fields}


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

        self.assertEqual(["diagnostics count: 1 vs 0"], errors)

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


if __name__ == "__main__":
    unittest.main()
