#!/usr/bin/env python3
"""Behavior tests for scripts/verifier_runtime.py.

The two directions fail differently and must never be conflated: a SOUNDNESS
failure is a false proof, a PRECISION failure is honest-verdict-misleading-
evidence. These tests pin that distinction and the not-applicable bookkeeping
that keeps a sweep from looking like it measured more than it did.
"""

import os
import subprocess
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock
from unittest.mock import patch

import verifier_runtime


class DirectiveTest(unittest.TestCase):
    def test_skip_directive_is_recognized(self):
        m = verifier_runtime.DIRECTIVE_SKIP.search('// TEST: skip "flaky"\n')

        self.assertEqual("flaky", m.group(1))

    def test_known_soundness_gap_directive_is_recognized(self):
        # verify_eval.py's KNOWN_GAP convention: a documented gap is reported
        # loudly but must not fail the run, or the sweep is red forever and
        # nobody reads it.
        m = verifier_runtime.DIRECTIVE_KNOWN_GAP.search(
            "// TEST: known-soundness-gap #585\n"
        )

        self.assertEqual("#585", m.group(1))

    def test_known_gap_without_an_issue_still_matches(self):
        m = verifier_runtime.DIRECTIVE_KNOWN_GAP.search(
            "// TEST: known-soundness-gap\n"
        )

        self.assertIsNotNone(m)


class CorpusTest(unittest.TestCase):
    def test_default_roots_exist(self):
        for rel in ("tests/verify", "tests/verify-fail", "examples"):
            with self.subTest(root=rel):
                self.assertTrue((verifier_runtime.REPO_ROOT / rel).is_dir())

    def test_repo_root_is_the_worktree(self):
        self.assertTrue((verifier_runtime.REPO_ROOT / "CLAUDE.md").exists())


def _verify_failed(*replay_values):
    return {
        "status": "VerifyFailed",
        "counterexamples": [
            {"function": "f", "vow_id": i, "replay": r, "replay_reason": "because"}
            for i, r in enumerate(replay_values)
        ],
    }


class PrecisionTest(unittest.TestCase):
    # `confirmed` is the only evidence that the model's counterexample
    # actually reproduces at runtime; `aborted` is an honest runtime failure
    # through some other mechanism, not that evidence.
    @patch("verifier_runtime.run_json")
    def test_abort_only_is_inconclusive_not_ok(self, mock_run_json):
        mock_run_json.return_value = (_verify_failed("aborted"), None)

        result = verifier_runtime.check_precision(Path("f.vow"), "verifier", 30)

        self.assertEqual("skipped", result["verdict"])

    @patch("verifier_runtime.run_json")
    def test_confirmed_alongside_aborted_is_still_ok(self, mock_run_json):
        mock_run_json.return_value = (_verify_failed("confirmed", "aborted"), None)

        result = verifier_runtime.check_precision(Path("f.vow"), "verifier", 30)

        self.assertEqual("ok", result["verdict"])
        self.assertIn("aborted", result["detail"])

    @patch("verifier_runtime.run_json")
    def test_diverged_is_still_a_precision_finding(self, mock_run_json):
        mock_run_json.return_value = (_verify_failed("diverged", "aborted"), None)

        result = verifier_runtime.check_precision(Path("f.vow"), "verifier", 30)

        self.assertEqual("PRECISION", result["verdict"])


def _call_args(mock_run_json):
    return [c.args[1] for c in mock_run_json.call_args_list]


class BackendTest(unittest.TestCase):
    """`--backend native` reaches `verify` only; `build` rejects it."""

    @patch("verifier_runtime.run_debug_binary", return_value=(None, None))
    @patch("verifier_runtime.run_json")
    def test_soundness_passes_backend_to_verify_not_to_build(self, run_json, _dbg):
        run_json.side_effect = [
            ({"status": "Verified"}, None),
            ({"executable": "x"}, None),
        ]

        verifier_runtime.check_soundness(
            Path("f.vow"), "vowc", Path("/tmp"), 30, backend="native"
        )

        verify, build = _call_args(run_json)
        self.assertEqual(["--backend", "native"], verify[1:3])
        self.assertNotIn("--backend", build)
        self.assertIn("build", build)

    @patch("verifier_runtime.run_json")
    def test_precision_passes_backend_to_verify(self, run_json):
        run_json.return_value = (_verify_failed("confirmed"), None)

        verifier_runtime.check_precision(Path("f.vow"), "vowc", 30, backend="native")

        (args,) = _call_args(run_json)
        self.assertEqual(["--backend", "native"], args[1:3])
        self.assertIn("--replay-cex", args)

    @patch("verifier_runtime.run_json")
    def test_no_backend_leaves_the_invocation_unchanged(self, run_json):
        run_json.return_value = (_verify_failed("confirmed"), None)

        verifier_runtime.check_precision(Path("f.vow"), "verifier", 30)

        (args,) = _call_args(run_json)
        self.assertNotIn("--backend", args)

    def test_pair_review_style_positional_call_still_binds(self):
        with patch("verifier_runtime.run_json", return_value=(None, "timeout")):
            result = verifier_runtime.check_soundness(
                Path("f.vow"), "verifier", Path("/tmp"), 30
            )

        self.assertEqual("skipped", result["verdict"])


class NativeStrictPrecisionTest(unittest.TestCase):
    """The native model is checked against the runtime, not trusted."""

    def check(self, payload, backend="native"):
        with patch("verifier_runtime.run_json", return_value=(payload, None)):
            return verifier_runtime.check_precision(
                Path("f.vow"), "vowc", 30, backend=backend
            )

    def test_aborted_is_a_finding_under_native_only(self):
        payload = _verify_failed("confirmed", "aborted")

        self.assertEqual("PRECISION", self.check(payload)["verdict"])
        self.assertEqual("ok", self.check(payload, backend=None)["verdict"])

    def test_a_verifier_bug_diagnostic_is_a_finding(self):
        payload = _verify_failed("confirmed")
        payload["diagnostics"] = [{"error_code": "VerifierBug", "message": "m"}]

        result = self.check(payload)

        self.assertEqual("PRECISION", result["verdict"])
        self.assertIn("VerifierBug", result["detail"])

    def test_a_skipped_replay_is_a_gap_bucketed_by_reason(self):
        payload = _verify_failed("confirmed", "skipped")
        payload["counterexamples"][1]["replay_reason"] = "parameter 0: unsupported type"

        result = self.check(payload)

        self.assertEqual("REPLAY-GAP", result["verdict"])
        self.assertEqual(["parameter 0: unsupported type"], result["replay_skipped"])

    def test_skipped_stays_unjudged_under_esbmc(self):
        payload = _verify_failed("confirmed", "skipped")

        self.assertEqual("ok", self.check(payload, backend=None)["verdict"])

    def test_all_confirmed_is_ok(self):
        self.assertEqual("ok", self.check(_verify_failed("confirmed"))["verdict"])


class ReplayByDesignTest(unittest.TestCase):
    """Obligations that are not runtime aborts are not replay failures."""

    def check(self, *reasons):
        payload = _verify_failed(*["skipped"] * len(reasons))
        for ce, reason in zip(payload["counterexamples"], reasons):
            ce["replay_reason"] = reason
        with patch("verifier_runtime.run_json", return_value=(payload, None)):
            return verifier_runtime.check_precision(
                Path("f.vow"), "vowc", 30, backend="native"
            )

    def test_the_documented_non_abort_reasons_are_not_gaps(self):
        for reason in (
            "replay: a shift-count counterexample is a verifier obligation; "
            "the runtime masks the count (8-bit shifts excepted)",
            "replay: no runtime abort is predicted for this obligation",
            "replay: counterexample belongs to a co-emitted callee; "
            "its inputs are the caller's",
        ):
            with self.subTest(reason=reason):
                result = self.check(reason)
                self.assertEqual("skipped", result["verdict"])
                self.assertEqual([reason], result["replay_skipped"])

    def test_an_unsupported_parameter_type_is_still_a_gap(self):
        result = self.check(
            "parameter 0: unsupported type (self-hosted v1: scalars only)"
        )

        self.assertEqual("REPLAY-GAP", result["verdict"])

    def test_an_unrecognised_reason_fails_closed(self):
        self.assertEqual("REPLAY-GAP", self.check("something new")["verdict"])


class ReplaySkippedDirectiveTest(unittest.TestCase):
    def test_directive_documents_a_deliberate_skip(self):
        m = verifier_runtime.DIRECTIVE_REPLAY_SKIPPED.search(
            "// TEST: replay-skipped 128-bit parameter\n"
        )

        self.assertEqual("128-bit parameter", m.group(1))


class DisclaimedCallSiteTest(unittest.TestCase):
    """A Caller violation at a call the verifier said it did not verify."""

    def run_soundness(self, verify, violation):
        with (
            patch(
                "verifier_runtime.run_json",
                side_effect=[(verify, None), ({"executable": "x"}, None)],
            ),
            patch("verifier_runtime.run_debug_binary", return_value=(violation, None)),
        ):
            return verifier_runtime.check_soundness(
                Path("f.vow"), "vowc", Path("/tmp"), 30, backend="native"
            )

    NOTE = {
        "error_code": "VerificationSkipped",
        "message": "calls from `main` to contracted `double` were not verified: "
        "`main` cannot be modelled",
    }

    def test_caller_violation_with_a_disclaiming_note_is_not_a_false_proof(self):
        result = self.run_soundness(
            {"status": "Verified", "diagnostics": [self.NOTE]},
            {"error": "VowViolation", "vow_id": 0, "blame": "Caller"},
        )

        self.assertEqual("unverified-call-site", result["verdict"])

    def test_callee_violation_is_still_a_false_proof_despite_the_note(self):
        result = self.run_soundness(
            {"status": "Verified", "diagnostics": [self.NOTE]},
            {"error": "VowViolation", "vow_id": 0, "blame": "Callee"},
        )

        self.assertEqual("SOUNDNESS", result["verdict"])

    def test_caller_violation_without_a_note_is_a_false_proof(self):
        result = self.run_soundness(
            {"status": "Verified", "diagnostics": []},
            {"error": "VowViolation", "vow_id": 0, "blame": "Caller"},
        )

        self.assertEqual("SOUNDNESS", result["verdict"])


class NativeCorpusTest(unittest.TestCase):
    def test_native_default_roots_add_the_native_fixtures(self):
        native = verifier_runtime.default_roots("native")
        esbmc = verifier_runtime.default_roots(None)

        for d in ("pass", "fail", "unknown"):
            root = verifier_runtime.REPO_ROOT / "tests" / "verify-native" / d
            self.assertIn(root, native)
            self.assertNotIn(root, esbmc)
            self.assertTrue(root.is_dir())

    def test_collect_corpus_shards_deterministically(self):
        root = verifier_runtime.REPO_ROOT / "tests" / "verify-native" / "pass"

        whole = verifier_runtime.collect_corpus([root])
        halves = [verifier_runtime.collect_corpus([root], f"{k}/2") for k in (0, 1)]

        self.assertEqual(sorted(whole), sorted(halves[0] + halves[1]))
        self.assertTrue(set(halves[0]).isdisjoint(halves[1]))


class ExtraRootTest(unittest.TestCase):
    def test_extra_roots_extend_the_defaults(self):
        roots = verifier_runtime.resolve_roots([], ["/gen"], "native")

        self.assertEqual(verifier_runtime.default_roots("native") + ["/gen"], roots)

    def test_explicit_roots_replace_the_defaults_and_still_take_extras(self):
        roots = verifier_runtime.resolve_roots(["a"], ["/gen"], None)

        self.assertEqual(["a", "/gen"], roots)


class RelativePathTest(unittest.TestCase):
    def test_a_corpus_outside_the_repo_keeps_its_absolute_path(self):
        outside = Path("/somewhere/else/gen_0.vow")

        self.assertEqual(str(outside), verifier_runtime.relative_to_repo(outside))

    def test_a_repo_file_is_reported_repo_relative(self):
        inside = verifier_runtime.REPO_ROOT / "tests" / "verify" / "x.vow"

        self.assertEqual(
            "tests/verify/x.vow", verifier_runtime.relative_to_repo(inside)
        )


class MainBackendTest(unittest.TestCase):
    def test_native_with_the_rust_compiler_is_a_usage_error(self):
        with patch.object(
            verifier_runtime.sys,
            "argv",
            [
                "verifier_runtime.py",
                "--backend",
                "native",
                "--verifier",
                "target/release/vow",
            ],
        ):
            self.assertEqual(2, verifier_runtime.main())

    def test_native_defaults_the_verifier_to_vowc(self):
        self.assertEqual("build/vowc", verifier_runtime.default_verifier("native"))
        self.assertEqual("target/release/vow", verifier_runtime.default_verifier(None))


class DebugRunTest(unittest.TestCase):
    """A hung binary is not evidence that the verifier was right."""

    def run_with(self, exc_or_proc):
        def fake_run(argv, **kwargs):
            if isinstance(exc_or_proc, Exception):
                raise exc_or_proc
            return exc_or_proc

        with mock.patch.object(verifier_runtime.subprocess, "run", fake_run):
            return verifier_runtime.run_debug_binary(Path("/nonexistent"), 3)

    def test_a_timeout_is_reported_as_an_error(self):
        violation, err = self.run_with(
            verifier_runtime.subprocess.TimeoutExpired(cmd="x", timeout=3)
        )

        self.assertIsNone(violation)
        self.assertIn("timed out", err)

    def test_a_clean_run_is_not_an_error(self):
        violation, err = self.run_with(SimpleNamespace(stderr="", returncode=0))

        self.assertIsNone(violation)
        self.assertIsNone(err)

    def test_a_violation_is_parsed(self):
        line = '{"error":"VowViolation","vow_id":1,"blame":"Callee"}'
        violation, err = self.run_with(SimpleNamespace(stderr=line, returncode=1))

        self.assertEqual("VowViolation", violation["error"])
        self.assertIsNone(err)


class RunJsonEnvTest(unittest.TestCase):
    def test_scrubs_credentials_keeps_path_pins_cwd(self):
        with (
            mock.patch.dict(
                os.environ, {"ANTHROPIC_API_KEY": "sk-x", "PATH": "/bin"}, clear=True
            ),
            mock.patch.object(
                verifier_runtime.subprocess,
                "run",
                side_effect=subprocess.TimeoutExpired("x", 1),
            ) as run,
        ):
            verifier_runtime.run_json("verifier", [], 1)

        env = run.call_args.kwargs["env"]
        self.assertNotIn("ANTHROPIC_API_KEY", env)
        self.assertEqual("/bin", env["PATH"])
        self.assertEqual(verifier_runtime.REPO_ROOT, run.call_args.kwargs["cwd"])


class RunDebugBinaryEnvAndCwdTest(unittest.TestCase):
    def test_scrubs_credentials_and_isolates_cwd_with_an_absolute_path(self):
        with (
            mock.patch.dict(os.environ, {"OPENAI_API_KEY": "sk-x"}, clear=True),
            mock.patch.object(
                verifier_runtime.subprocess,
                "run",
                side_effect=subprocess.TimeoutExpired("x", 1),
            ) as run,
        ):
            verifier_runtime.run_debug_binary("relative/dbg", 1)

        self.assertNotIn("OPENAI_API_KEY", run.call_args.kwargs["env"])
        called_path = Path(run.call_args.args[0][0])
        self.assertTrue(called_path.is_absolute())
        self.assertEqual(Path("relative/dbg").resolve(), called_path)
        self.assertNotEqual(verifier_runtime.REPO_ROOT, run.call_args.kwargs["cwd"])


if __name__ == "__main__":
    unittest.main()
