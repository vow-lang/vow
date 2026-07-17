#!/usr/bin/env python3
"""Behavior regressions for the compiled Vow chess engine."""

from __future__ import annotations

import argparse
import re
import resource
import subprocess
from pathlib import Path


MATE_SCORE_RE = re.compile(r"\bscore mate (-?\d+)\b")


def limit_virtual_memory() -> None:
    limit = 2_000_000 * 1024
    resource.setrlimit(resource.RLIMIT_AS, (limit, limit))


def run_engine(engine: Path, commands: str) -> list[list[str]]:
    result = subprocess.run(
        [str(engine)],
        input=commands,
        text=True,
        capture_output=True,
        check=True,
        preexec_fn=limit_virtual_memory,
    )
    searches: list[list[str]] = []
    current: list[str] = []
    for line in result.stdout.splitlines():
        if line.startswith("info "):
            current.append(line)
        elif line.startswith("bestmove "):
            current.append(line)
            searches.append(current)
            current = []
    return searches


def final_mate_score(search: list[str]) -> int | None:
    for line in reversed(search):
        match = MATE_SCORE_RE.search(line)
        if match:
            return int(match.group(1))
    return None


def test_warm_tt_preserves_mate_distance(engine: Path) -> None:
    searches = run_engine(
        engine,
        "position fen 8/8/8/8/3K4/8/2Q5/k7 w - - 0 1\n"
        "go depth 8\n"
        "position fen 8/8/8/8/3K4/8/4Q3/1k6 w - - 2 2\n"
        "go depth 6\n"
        "quit\n",
    )
    assert len(searches) == 2, searches
    assert final_mate_score(searches[0]) == 3, searches[0]
    assert final_mate_score(searches[1]) == 2, searches[1]


def test_warm_tt_preserves_negative_mate_distance(engine: Path) -> None:
    searches = run_engine(
        engine,
        "position fen 8/8/8/8/3K4/8/4Q3/k7 b - - 1 1\n"
        "go depth 7\n"
        "position fen 8/8/8/8/8/2K5/4Q3/1k6 b - - 3 2\n"
        "go depth 5\n"
        "quit\n",
    )
    assert len(searches) == 2, searches
    assert final_mate_score(searches[0]) == -2, searches[0]
    assert final_mate_score(searches[1]) == -1, searches[1]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--engine", required=True, type=Path)
    args = parser.parse_args()

    test_warm_tt_preserves_mate_distance(args.engine)
    test_warm_tt_preserves_negative_mate_distance(args.engine)
    print("chess engine behavior tests passed")


if __name__ == "__main__":
    main()
