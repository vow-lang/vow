#!/usr/bin/env python3

import errno
import os
from pathlib import Path
import pty
import select
import subprocess
import tempfile
import time
import unittest


ROOT = Path(__file__).resolve().parents[1]
RELEASE_SCRIPT = ROOT / "scripts" / "release.sh"
EXPECTED_PROMPT = b"Proceed? [yes/No] "


class ReleasePromptTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp_dir.cleanup)

        temp_path = Path(self.temp_dir.name)
        self.bin_dir = temp_path / "bin"
        self.bin_dir.mkdir()
        self.gh_log = temp_path / "gh.log"

        self.write_executable(
            "git",
            """#!/usr/bin/env bash
set -euo pipefail

case "$*" in
    "fetch --quiet origin main") exit 0 ;;
    "show origin/main:vow/Cargo.toml") printf 'version = "1.2.3"\\n' ;;
    *) printf 'unexpected git arguments: %s\\n' "$*" >&2; exit 64 ;;
esac
""",
        )
        self.write_executable(
            "gh",
            """#!/usr/bin/env bash
set -euo pipefail

printf '%s\\n' "$*" >> "$GH_LOG"
""",
        )

    def write_executable(self, name, contents):
        path = self.bin_dir / name
        path.write_text(contents)
        path.chmod(0o755)

    def run_release(self, reply):
        self.gh_log.unlink(missing_ok=True)
        env = os.environ.copy()
        env["PATH"] = f"{self.bin_dir}{os.pathsep}{env['PATH']}"
        env["GH_LOG"] = str(self.gh_log)

        master_fd, slave_fd = pty.openpty()
        process = subprocess.Popen(
            ["bash", str(RELEASE_SCRIPT), "rev"],
            cwd=ROOT,
            env=env,
            stdin=slave_fd,
            stdout=slave_fd,
            stderr=slave_fd,
            close_fds=True,
        )
        os.close(slave_fd)

        output = bytearray()
        try:
            prompt = self.read_prompt(master_fd, process, output)
            os.write(master_fd, reply.encode() + b"\n")
            try:
                return_code = process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
                self.fail(f"release script did not exit; output: {output.decode(errors='replace')}")
            self.drain(master_fd, output)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            os.close(master_fd)

        dispatch = self.gh_log.read_text() if self.gh_log.exists() else ""
        return prompt, return_code, dispatch, bytes(output)

    def read_prompt(self, master_fd, process, output):
        deadline = time.monotonic() + 5
        marker = b"Proceed? ["
        while time.monotonic() < deadline:
            start = output.find(marker)
            if start != -1:
                end = output.find(b"] ", start)
                if end != -1:
                    return bytes(output[start : end + 2])

            if process.poll() is not None:
                break
            readable, _, _ = select.select([master_fd], [], [], 0.1)
            if readable:
                self.read_available(master_fd, output)

        self.fail(f"release prompt not emitted; output: {output.decode(errors='replace')}")

    @staticmethod
    def read_available(master_fd, output):
        try:
            output.extend(os.read(master_fd, 4096))
        except OSError as error:
            if error.errno != errno.EIO:
                raise

    def drain(self, master_fd, output):
        while select.select([master_fd], [], [], 0)[0]:
            before = len(output)
            self.read_available(master_fd, output)
            if len(output) == before:
                break

    def test_prompt_advertises_every_accepted_affirmative_response(self):
        for reply in ("y", "Y", "yes", "YES"):
            with self.subTest(reply=reply):
                prompt, return_code, dispatch, _ = self.run_release(reply)

                self.assertEqual(prompt, EXPECTED_PROMPT)
                self.assertEqual(return_code, 0)
                self.assertEqual(
                    dispatch,
                    "workflow run release.yml --ref main -f bump=rev\n",
                )

    def test_empty_response_remains_the_negative_default(self):
        prompt, return_code, dispatch, output = self.run_release("")

        self.assertEqual(prompt, EXPECTED_PROMPT)
        self.assertEqual(return_code, 1)
        self.assertEqual(dispatch, "")
        self.assertIn(b"Aborted.", output)


if __name__ == "__main__":
    unittest.main()
