#!/usr/bin/env python3
"""Behavior tests for the Vow chess engine's lightweight endgame knowledge.

The tests drive only the public UCI interface. Pass the already-built engine
binary with ``--engine``.
"""

from __future__ import annotations

import argparse
import re
import shlex
import subprocess
import sys
from pathlib import Path


SCORE_RE = re.compile(r"\bscore (cp|mate) (-?\d+)\b")


class Engine:
    def __init__(self, command: str) -> None:
        self.command = shlex.split(command)
        self.proc = subprocess.Popen(
            self.command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
            cwd=Path.cwd(),
        )
        self.send("uci")
        self.read_until("uciok")
        self.send("isready")
        self.read_until("readyok")

    def send(self, line: str) -> None:
        if self.proc.stdin is None:
            raise RuntimeError("engine stdin is unavailable")
        self.proc.stdin.write(line + "\n")
        self.proc.stdin.flush()

    def read_until(self, token: str) -> list[str]:
        if self.proc.stdout is None:
            raise RuntimeError("engine stdout is unavailable")
        lines: list[str] = []
        while True:
            raw = self.proc.stdout.readline()
            if raw == "":
                stderr = ""
                if self.proc.stderr is not None:
                    stderr = self.proc.stderr.read()
                raise RuntimeError(
                    f"engine exited while waiting for {token!r}; "
                    f"output={lines!r}; stderr={stderr!r}"
                )
            line = raw.rstrip("\n")
            lines.append(line)
            if line == token or line.startswith(token):
                return lines

    def score(self, fen: str, depth: int = 2) -> tuple[str, int]:
        self.send("ucinewgame")
        self.send(f"position fen {fen}")
        self.send(f"go depth {depth}")
        lines = self.read_until("bestmove ")
        for line in reversed(lines):
            match = SCORE_RE.search(line)
            if match is not None:
                return match.group(1), int(match.group(2))
        raise AssertionError(f"engine returned no score for {fen!r}: {lines!r}")

    def close(self) -> None:
        try:
            self.send("quit")
        except (BrokenPipeError, RuntimeError):
            pass
        try:
            self.proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait(timeout=3)


DRAW_CASES = {
    "KvK": "7k/8/8/8/8/8/4K3/8 w - - 0 1",
    "KNvK": "7k/8/8/8/8/2N5/8/K7 w - - 0 1",
    "KvKN": "7k/8/5n2/8/8/8/8/K7 w - - 0 1",
    "KBvK": "7k/8/8/8/8/8/2B5/K7 w - - 0 1",
    "same-coloured KBvKB": "5b1k/8/8/8/8/8/8/K1B5 w - - 0 1",
}

LIVE_CASES = {
    "opposite-coloured KBvKB": "4b2k/8/8/8/8/8/8/K1B5 w - - 0 1",
    "KRvK": "7k/8/8/8/8/8/2R5/K7 w - - 0 1",
    "KQvK": "7k/8/8/8/8/8/2Q5/K7 w - - 0 1",
}


def test_insufficient_material(engine: Engine) -> list[str]:
    failures: list[str] = []
    for name, fen in DRAW_CASES.items():
        score_kind, score = engine.score(fen)
        if score_kind != "cp" or score != 0:
            failures.append(
                f"{name}: expected draw score cp 0, got {score_kind} {score}"
            )

    for name, fen in LIVE_CASES.items():
        score_kind, score = engine.score(fen)
        if score_kind != "mate" and score == 0:
            failures.append(f"{name}: live material was scored as a draw")
    return failures


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--engine", required=True, help="UCI engine command")
    args = parser.parse_args()

    engine = Engine(args.engine)
    try:
        failures = test_insufficient_material(engine)
    finally:
        engine.close()

    if failures:
        for failure in failures:
            print(f"FAIL: {failure}", file=sys.stderr)
        return 1

    print(
        f"insufficient material: {len(DRAW_CASES)}/{len(DRAW_CASES)} draws; "
        f"{len(LIVE_CASES)}/{len(LIVE_CASES)} live controls"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
