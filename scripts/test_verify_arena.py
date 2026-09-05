#!/usr/bin/env python3
"""Integration tests for the standalone arena verification entry point."""

from __future__ import annotations

import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest


REPO_ROOT = Path(__file__).resolve().parent.parent
RUNNER = REPO_ROOT / "scripts" / "verify_arena.sh"
ARENA_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "arena-verify.yml"
CI_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "ci.yml"


class WorkflowTest(unittest.TestCase):
    """The arena proof is path-gated, so these guard the gate itself.

    The proof no longer runs on every pull request: it runs when the changeset
    touches an arena input, and nightly regardless. Both halves have to hold.
    Gating alone would let the proof rot on a pull request that misses the path
    list; the nightly run alone would let a regression land and be attributed
    to a day of commits instead of the one that caused it.
    """

    def setUp(self) -> None:
        self.workflow = ARENA_WORKFLOW.read_text(encoding="utf-8")

    def test_workflow_runs_the_arena_proof(self) -> None:
        self.assertRegex(
            self.workflow,
            re.compile(
                r"^[ \t]*(?:-[ \t]+)?run:[ \t]*scripts/verify_arena\.sh[ \t]*$",
                re.MULTILINE,
            ),
        )

    def test_workflow_runs_nightly(self) -> None:
        self.assertRegex(
            self.workflow,
            re.compile(r"^\s*-\s*cron:\s*[\"']\S+ \S+ \* \* \*[\"']", re.MULTILINE),
        )

    def test_workflow_is_gated_on_the_classifier(self) -> None:
        self.assertIn("needs.changes.outputs.arena == 'true'", self.workflow)

    def test_ci_workflow_no_longer_duplicates_the_proof(self) -> None:
        ci = CI_WORKFLOW.read_text(encoding="utf-8")

        self.assertNotIn("verify_arena.sh", ci)

    def test_workflow_has_a_macos_measurement_job(self) -> None:
        """Issue #1212: a non-blocking job measures the proof on macOS/arm64.

        It must share the Linux job's gate (same classifier output) and
        declare its own timeout, but is not required to run the proof the
        same way -- it measures both capped and uncapped behavior, which
        `scripts/verify_arena.sh` alone does not expose.
        """
        self.assertRegex(
            self.workflow,
            re.compile(r"^\s*verify-macos:\s*$", re.MULTILINE),
        )
        self.assertRegex(
            self.workflow,
            re.compile(r"^\s*runs-on:\s*macos-latest\s*$", re.MULTILINE),
        )

    def test_macos_job_is_gated_on_the_same_classifier(self) -> None:
        macos_job = self.workflow.split("verify-macos:", 1)[1]

        self.assertIn("needs.changes.outputs.arena == 'true'", macos_job)

    def test_macos_job_declares_a_timeout(self) -> None:
        macos_job = self.workflow.split("verify-macos:", 1)[1]

        self.assertRegex(
            macos_job, re.compile(r"^\s*timeout-minutes:\s*\d+\s*$", re.MULTILINE)
        )


class VerifyArenaTest(unittest.TestCase):
    def test_runner_caps_memory_and_invokes_the_arena_proof(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_path = Path(temp_dir)
            capture_path = temp_path / "invocation.txt"
            fake_esbmc = temp_path / "esbmc"
            fake_esbmc.write_text(
                """#!/usr/bin/env bash
set -euo pipefail
{
    printf 'vmem_kb=%s\\n' "$(ulimit -v)"
    printf 'working_directory=%s\\n' "$PWD"
    printf 'arguments='
    printf ' <%s>' "$@"
    printf '\\n'
} > "$ARENA_TEST_CAPTURE"
""",
                encoding="utf-8",
            )
            fake_esbmc.chmod(0o755)

            env = os.environ.copy()
            env["ARENA_TEST_CAPTURE"] = str(capture_path)
            env["ESBMC"] = str(fake_esbmc)

            subprocess.run([RUNNER], cwd=temp_path, env=env, check=True)

            self.assertEqual(
                capture_path.read_text(encoding="utf-8"),
                "\n".join(
                    [
                        "vmem_kb=2000000",
                        f"working_directory={REPO_ROOT / 'vow-runtime' / 'verify'}",
                        "arguments= <arena.c> <--unwind> <5> <--no-bounds-check> "
                        "<--no-pointer-check> <--64> <--boolector>",
                        "",
                    ]
                ),
            )


if __name__ == "__main__":
    unittest.main()
