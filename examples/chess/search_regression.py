#!/usr/bin/env python3
"""Deterministic fixed-depth regression gate for the Vow chess engine."""

from __future__ import annotations

import argparse
import re
import shlex
import time
from dataclasses import dataclass
from pathlib import Path

from play_uci_match import Engine, init_engine


ROOT = Path(__file__).resolve().parents[2]
INFO_RE = re.compile(
    r"^info depth (?P<depth>\d+) score (?P<kind>cp|mate) "
    r"(?P<score>-?\d+) nodes (?P<nodes>\d+) time (?P<time>\d+) "
    r"nps (?P<nps>\d+) pv (?P<pv>[a-h][1-8][a-h][1-8][nbrq]?)$"
)


@dataclass(frozen=True)
class FixedDepthCase:
    name: str
    position: str
    depth: int
    score_kind: str
    score: int
    bestmove: str
    nodes: int


@dataclass(frozen=True)
class SearchResult:
    score_kind: str
    score: int
    bestmove: str
    nodes: int
    elapsed_ms: int
    nps: int


FIXED_DEPTH_CASES = (
    FixedDepthCase("startpos", "position startpos", 6, "cp", 0, "g1f3", 16541),
    FixedDepthCase(
        "kiwipete",
        "position fen r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/"
        "PPPBBPPP/R3K2R w KQkq - 0 1",
        6,
        "cp",
        -30,
        "e2a6",
        356711,
    ),
    FixedDepthCase(
        "midgame",
        "position fen r1bq1rk1/pp2bppp/2n1pn2/2pp4/3P1B2/2PBPN2/"
        "PP1N1PPP/R2Q1RK1 w - - 0 9",
        6,
        "cp",
        39,
        "f3e5",
        115061,
    ),
    FixedDepthCase(
        "endgame",
        "position fen 8/2k2pp1/1p5p/p1p1P3/P1P2P2/1P4KP/8/8 w - - 0 1",
        6,
        "cp",
        25,
        "f4f5",
        4783,
    ),
)


TACTICAL_CASES = (
    (
        "2rr3k/pp3pp1/1nnqbN1p/3pN3/2pP4/2P3Q1/PPB4P/R4RK1 w - - 0 1",
        "g3g6",
        "g3g6",
    ),
    (
        "8/7p/5k2/5p2/p1p2P2/Pr1pPK2/1P1R3P/8 b - - 0 1",
        "c4c3",
        "f6f7",
    ),
    (
        "5rk1/1ppb3p/p1pb4/6q1/3P1p1r/2P1R2P/PP1BQ1P1/5RKN w - - 0 1",
        "e3g3",
        "e3g3",
    ),
    (
        "r1b1kb1r/3q1ppp/pBp1pn2/8/Np3P2/5B2/PPP3PP/R2Q1RK1 w kq - 0 1",
        "f3c6",
        "f3c6",
    ),
    (
        "r3r1k1/ppqb1ppp/8/4p1NQ/8/2P5/PP3PPP/R3R1K1 b - - 0 1",
        "d7f5",
        "d7f5",
    ),
    (
        "5k2/6pp/p1qN4/1p1p4/3P4/2PKP2Q/PP3r2/3R4 b - - 0 1",
        "c6c4",
        "c6c4",
    ),
    (
        "6k1/1b1nqpbp/pp4p1/5P2/1PN5/4Q3/P5PP/1B2B1K1 b - - 0 1",
        "g7d4",
        "g7d4",
    ),
    (
        "3r3k/2r4p/1p1b3q/p4P2/P1P4P/1P1Q3B/1RnN3K/2R5 b - - 0 1",
        "h6d2",
        "d6h2",
    ),
    (
        "2r5/2rk2pp/1pn1pb2/pN1p4/P2P4/1N2B3/nPR1KPPP/3R4 b - - 0 1",
        "c6d4",
        "c6d4",
    ),
    (
        "6k1/pp4p1/2p5/2bp4/8/P5Pb/1P3rrP/2BRRN1K b - - 0 1",
        "g2g1",
        "g2g1",
    ),
)

PRUNING_SAFEGUARDS = (
    (
        "advanced pawn push",
        "8/8/4P3/8/8/8/4K3/7k w - - 0 1",
        6,
        "e6e7",
    ),
)

STARTPOS_MOVES = {
    "a2a3",
    "a2a4",
    "b1a3",
    "b1c3",
    "b2b3",
    "b2b4",
    "c2c3",
    "c2c4",
    "d2d3",
    "d2d4",
    "e2e3",
    "e2e4",
    "f2f3",
    "f2f4",
    "g1f3",
    "g1h3",
    "g2g3",
    "g2g4",
    "h2h3",
    "h2h4",
}


def limited_command(engine_command: str) -> list[str]:
    command = shlex.split(engine_command)
    if not command:
        raise ValueError("engine command must not be empty")
    quoted = " ".join(shlex.quote(part) for part in command)
    return ["sh", "-c", f"ulimit -v 2000000; exec {quoted}"]


def run_search(engine_command: str, position: str, depth: int) -> SearchResult:
    engine = Engine(limited_command(engine_command), ROOT, "candidate")
    try:
        init_engine(engine)
        engine.send(position)
        engine.send(f"go depth {depth}")
        bestmove, lines = engine.read_bestmove()
    finally:
        engine.quit()

    info = None
    for line in lines:
        match = INFO_RE.match(line)
        if match and int(match.group("depth")) == depth:
            info = match
    if info is None:
        raise AssertionError(f"no completed info line at depth {depth}: {lines!r}")
    return SearchResult(
        score_kind=info.group("kind"),
        score=int(info.group("score")),
        bestmove=bestmove,
        nodes=int(info.group("nodes")),
        elapsed_ms=int(info.group("time")),
        nps=int(info.group("nps")),
    )


def run_bestmove(engine_command: str, fen: str, depth: int) -> str:
    engine = Engine(limited_command(engine_command), ROOT, "candidate")
    try:
        init_engine(engine)
        engine.send(f"position fen {fen}")
        engine.send(f"go depth {depth}")
        bestmove, _ = engine.read_bestmove()
        return bestmove
    finally:
        engine.quit()


def run_stop_path(engine_command: str) -> tuple[str, list[int]]:
    engine = Engine(limited_command(engine_command), ROOT, "candidate")
    try:
        init_engine(engine)
        engine.send("position startpos")
        engine.send("go depth 64")
        # Let the synchronous command loop enter search before queuing stop;
        # otherwise the runtime's line buffer can prefetch both commands before
        # stdin_ready() begins polling the underlying descriptor.
        time.sleep(0.05)
        engine.send("stop")
        bestmove, lines = engine.read_bestmove()
    finally:
        engine.quit()

    completed_depths: list[int] = []
    for line in lines:
        if not line.startswith("info "):
            continue
        match = INFO_RE.match(line)
        if match is None:
            raise AssertionError(f"malformed completed-depth info line: {line!r}")
        completed_depths.append(int(match.group("depth")))
    return bestmove, completed_depths


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", required=True, help="candidate engine command")
    parser.add_argument(
        "--max-total-nodes",
        type=int,
        default=sum(case.nodes for case in FIXED_DEPTH_CASES) - 1,
        help="strict aggregate fixed-depth node ceiling",
    )
    parser.add_argument(
        "--skip-tactics",
        action="store_true",
        help="run only the faster fixed-depth gate",
    )
    args = parser.parse_args()

    failures: list[str] = []
    total_nodes = 0
    baseline_nodes = sum(case.nodes for case in FIXED_DEPTH_CASES)
    print("fixed-depth regression:")
    for case in FIXED_DEPTH_CASES:
        result = run_search(args.engine, case.position, case.depth)
        total_nodes += result.nodes
        semantics = (result.score_kind, result.score, result.bestmove)
        expected = (case.score_kind, case.score, case.bestmove)
        if semantics != expected:
            failures.append(f"{case.name}: expected {expected}, got {semantics}")
        delta = result.nodes - case.nodes
        print(
            f"  {case.name:9} d{case.depth} {result.score_kind} {result.score:+d} "
            f"pv={result.bestmove} nodes={result.nodes} ({delta:+d}) "
            f"time={result.elapsed_ms}ms nps={result.nps}"
        )

    print(
        f"  aggregate nodes={total_nodes} "
        f"(baseline={baseline_nodes}, delta={total_nodes - baseline_nodes:+d}, "
        f"ceiling={args.max_total_nodes})"
    )
    if total_nodes > args.max_total_nodes:
        failures.append(
            f"aggregate nodes {total_nodes} exceed ceiling {args.max_total_nodes}"
        )

    if not args.skip_tactics:
        oracle_matches = 0
        print("tactical regression (depth 7):")
        for index, (fen, expected_move, oracle_move) in enumerate(TACTICAL_CASES, 1):
            move = run_bestmove(args.engine, fen, 7)
            if move != expected_move:
                failures.append(
                    f"tactical {index}: expected stable move {expected_move}, got {move}"
                )
            if move == oracle_move:
                oracle_matches += 1
            print(
                f"  {index:2}: move={move} expected={expected_move} "
                f"oracle={oracle_move}"
            )
        print(f"  oracle matches={oracle_matches}/10 (baseline=8/10)")
        if oracle_matches != 8:
            failures.append(
                f"tactical oracle result changed: expected 8/10, got {oracle_matches}/10"
            )

        print("selective-pruning safeguards:")
        for name, fen, depth, expected_move in PRUNING_SAFEGUARDS:
            move = run_bestmove(args.engine, fen, depth)
            print(f"  {name}: move={move} expected={expected_move}")
            if move != expected_move:
                failures.append(
                    f"{name}: expected stable move {expected_move}, got {move}"
                )

    stopped_move, completed_depths = run_stop_path(args.engine)
    print(
        f"stop regression: bestmove={stopped_move} "
        f"completed_depths={completed_depths}"
    )
    if stopped_move not in STARTPOS_MOVES:
        failures.append(f"stop path returned illegal startpos move {stopped_move}")
    if completed_depths != sorted(set(completed_depths)):
        failures.append(
            f"stop path published non-monotonic completed depths {completed_depths}"
        )

    if failures:
        print("FAIL:")
        for failure in failures:
            print(f"  {failure}")
        return 1
    print("PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
