#!/usr/bin/env python3
"""Black-box UCI protocol regressions for the Vow chess engine."""

from __future__ import annotations

import os
import queue
import re
import subprocess
import threading
import time
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
ENGINE_SOURCE = REPO_ROOT / "examples/chess/main.vow"
ENGINE_BINARY = REPO_ROOT / "examples/chess/.local/chess-uci-test"
VOWC = REPO_ROOT / "build/vowc"
BESTMOVE_RE = re.compile(r"^bestmove [a-h][1-8][a-h][1-8][nbrq]?$")


def bounded_command(*args: str) -> list[str]:
    return [
        "bash",
        "-c",
        'ulimit -v 2000000; exec "$@"',
        "vow-chess-test",
        *args,
    ]


class Engine:
    def __init__(self) -> None:
        self.proc = subprocess.Popen(
            bounded_command(str(ENGINE_BINARY)),
            cwd=REPO_ROOT,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        self.lines: queue.Queue[str | None] = queue.Queue()
        self.transcript: list[str] = []
        self.reader = threading.Thread(target=self._drain_stdout, daemon=True)
        self.reader.start()

    def _drain_stdout(self) -> None:
        assert self.proc.stdout is not None
        for line in self.proc.stdout:
            self.lines.put(line.rstrip("\n"))
        self.lines.put(None)

    def send(self, command: str) -> None:
        if self.proc.stdin is None:
            raise RuntimeError("engine stdin is unavailable")
        self.proc.stdin.write(command + "\n")
        self.proc.stdin.flush()

    def wait_for(self, predicate, timeout: float) -> str:
        deadline = time.monotonic() + timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise AssertionError(
                    f"timed out waiting for engine output; transcript={self.transcript!r}"
                )
            try:
                line = self.lines.get(timeout=remaining)
            except queue.Empty as error:
                raise AssertionError(
                    f"timed out waiting for engine output; transcript={self.transcript!r}"
                ) from error
            if line is None:
                stderr = ""
                if self.proc.stderr is not None:
                    stderr = self.proc.stderr.read()
                raise AssertionError(
                    f"engine exited unexpectedly with {self.proc.poll()}; "
                    f"stderr={stderr!r}; transcript={self.transcript!r}"
                )
            self.transcript.append(line)
            if predicate(line):
                return line

    def read_for(self, duration: float) -> list[str]:
        observed: list[str] = []
        deadline = time.monotonic() + duration
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return observed
            try:
                line = self.lines.get(timeout=remaining)
            except queue.Empty:
                return observed
            if line is None:
                raise AssertionError(
                    f"engine exited unexpectedly with {self.proc.poll()}; "
                    f"transcript={self.transcript!r}"
                )
            self.transcript.append(line)
            observed.append(line)

    def close(self) -> None:
        try:
            if self.proc.poll() is None:
                try:
                    self.send("stop")
                    self.send("quit")
                    self.proc.wait(timeout=5)
                except (BrokenPipeError, subprocess.TimeoutExpired):
                    self.proc.terminate()
                    try:
                        self.proc.wait(timeout=2)
                    except subprocess.TimeoutExpired:
                        self.proc.kill()
                        self.proc.wait(timeout=2)
        finally:
            if self.proc.stdin is not None:
                self.proc.stdin.close()
            self.reader.join(timeout=2)
            if self.proc.stdout is not None:
                self.proc.stdout.close()
            if self.proc.stderr is not None:
                self.proc.stderr.close()


class UciGoTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        if not VOWC.is_file():
            raise AssertionError("build/vowc is missing; run scripts/bootstrap.sh first")
        ENGINE_BINARY.parent.mkdir(parents=True, exist_ok=True)
        env = os.environ.copy()
        if Path("/dev/shm").is_dir():
            env["TMPDIR"] = "/dev/shm"
        subprocess.run(
            bounded_command(
                str(VOWC),
                "build",
                "--no-verify",
                str(ENGINE_SOURCE),
                "-o",
                str(ENGINE_BINARY),
            ),
            cwd=REPO_ROOT,
            env=env,
            check=True,
            capture_output=True,
            timeout=120,
        )

    def setUp(self) -> None:
        self.engine = Engine()
        self.engine.send("uci")
        self.engine.wait_for(lambda line: line == "uciok", timeout=5)
        self.engine.send("isready")
        self.engine.wait_for(lambda line: line == "readyok", timeout=5)

    def tearDown(self) -> None:
        self.engine.close()

    def test_go_depth_returns_a_legal_move_without_stop(self) -> None:
        self.engine.send("position startpos")
        self.engine.send("go depth 1")

        bestmove = self.engine.wait_for(
            lambda line: line.startswith("bestmove "), timeout=5
        )

        self.assertRegex(bestmove, BESTMOVE_RE)

    def test_go_movetime_returns_a_legal_move_without_stop(self) -> None:
        self.engine.send("position startpos")
        self.engine.send("go movetime 50")

        bestmove = self.engine.wait_for(
            lambda line: line.startswith("bestmove "), timeout=5
        )

        self.assertRegex(bestmove, BESTMOVE_RE)

    def test_go_infinite_waits_for_stop(self) -> None:
        self.engine.send("position startpos")
        self.engine.send("go infinite")

        before_stop = self.engine.read_for(duration=1.3)

        self.assertFalse(
            any(line.startswith("bestmove ") for line in before_stop),
            f"go infinite returned before stop: {before_stop!r}",
        )

        self.engine.send("stop")
        bestmove = self.engine.wait_for(
            lambda line: line.startswith("bestmove "), timeout=10
        )

        self.assertRegex(bestmove, BESTMOVE_RE)
        after_bestmove = self.engine.read_for(duration=0.2)
        self.assertFalse(
            any(line.startswith("bestmove ") for line in after_bestmove),
            f"engine emitted more than one bestmove: {after_bestmove!r}",
        )

    def test_go_infinite_ignores_mate_score_until_stop_or_max_depth(self) -> None:
        self.engine.send("position fen 7k/5Q2/6K1/8/8/8/8/8 w - - 0 1")
        self.engine.send("go infinite")

        before_stop = self.engine.read_for(duration=0.5)
        early_bestmove = next(
            (line for line in before_stop if line.startswith("bestmove ")), None
        )
        if early_bestmove is not None:
            self.assertTrue(
                any(line.startswith("info depth 64 ") for line in before_stop),
                f"go infinite stopped on a mate score before MAX_DEPTH: {before_stop!r}",
            )
            self.assertRegex(early_bestmove, BESTMOVE_RE)
            return

        self.engine.send("stop")
        bestmove = self.engine.wait_for(
            lambda line: line.startswith("bestmove "), timeout=10
        )

        self.assertRegex(bestmove, BESTMOVE_RE)


if __name__ == "__main__":
    unittest.main()
