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


def engine_stdout(engine: Path, commands: str) -> str:
    result = subprocess.run(
        [str(engine)],
        input=commands,
        text=True,
        capture_output=True,
        check=True,
        preexec_fn=limit_virtual_memory,
    )
    return result.stdout


def run_engine(engine: Path, commands: str) -> list[list[str]]:
    searches: list[list[str]] = []
    current: list[str] = []
    for line in engine_stdout(engine, commands).splitlines():
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


def evaluation_breakdown(engine: Path, fen: str) -> dict[str, int]:
    stdout = engine_stdout(engine, f"position fen {fen}\neval\nquit\n")
    eval_lines = [line for line in stdout.splitlines() if line.startswith("eval ")]
    assert len(eval_lines) == 1, stdout
    tokens = eval_lines[0].split()
    assert len(tokens) % 2 == 1, eval_lines[0]
    return {tokens[i]: int(tokens[i + 1]) for i in range(1, len(tokens), 2)}


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
    assert final_mate_score(searches[0]) is not None, searches[0]
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
    assert final_mate_score(searches[0]) is not None, searches[0]
    assert final_mate_score(searches[1]) == -1, searches[1]


def test_quiescence_mate_score_uses_root_ply(engine: Path) -> None:
    searches = run_engine(
        engine,
        "position fen 2n5/2k1B1r1/br3b2/4N3/B7/P1K1N1n1/6P1/1q6 b - - 4 61\n"
        "go depth 1\n"
        "quit\n",
    )
    assert len(searches) == 1, searches
    assert final_mate_score(searches[0]) == 1, searches[0]


def test_evaluation_reports_piece_mobility(engine: Path) -> None:
    knight = evaluation_breakdown(
        engine, "7k/8/8/8/3N4/8/8/K7 w - - 0 1"
    )
    rook = evaluation_breakdown(
        engine, "7k/8/3P4/8/3R4/8/3p4/K7 w - - 0 1"
    )
    assert knight["mobility"] == 8, knight
    assert rook["mobility"] == 10, rook


def test_evaluation_reports_pawn_shield_balance(engine: Path) -> None:
    terms = evaluation_breakdown(
        engine, "rnbqkbnr/pppppppp/8/8/8/8/PPP3PP/RNBQKBNR w KQkq - 0 1"
    )
    assert terms["pawn_shield"] == -6, terms


def test_evaluation_reports_king_zone_attacks(engine: Path) -> None:
    terms = evaluation_breakdown(
        engine, "6k1/8/8/8/8/8/1Q6/K7 w - - 0 1"
    )
    assert terms["king_attacks"] == 2, terms
    assert terms["king_safety"] == 2, terms


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--engine", required=True, type=Path)
    args = parser.parse_args()

    test_warm_tt_preserves_mate_distance(args.engine)
    test_warm_tt_preserves_negative_mate_distance(args.engine)
    test_quiescence_mate_score_uses_root_ply(args.engine)
    test_evaluation_reports_piece_mobility(args.engine)
    test_evaluation_reports_pawn_shield_balance(args.engine)
    test_evaluation_reports_king_zone_attacks(args.engine)
    print("chess engine behavior tests passed")


if __name__ == "__main__":
    main()
