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
CI_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "ci.yml"


class VerifyArenaTest(unittest.TestCase):
    def test_pull_request_ci_runs_the_arena_proof(self) -> None:
        workflow = CI_WORKFLOW.read_text(encoding="utf-8")

        self.assertRegex(
            workflow,
            re.compile(
                r"^[ \t]*(?:-[ \t]+)?run:[ \t]*scripts/verify_arena\.sh[ \t]*$",
                re.MULTILINE,
            ),
        )

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
