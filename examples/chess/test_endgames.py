#!/usr/bin/env python3
"""Behavior tests for the Vow chess engine's lightweight endgame knowledge.

The tests drive only the public UCI interface. Pass the already-built engine
binary with ``--engine``.
"""

from __future__ import annotations

import argparse
import random
import re
import shlex
import subprocess
import sys
import time
from dataclasses import dataclass
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
        self.new_game()
        self.set_position(fen, [])
        self.send(f"go depth {depth}")
        lines = self.read_until("bestmove ")
        for line in reversed(lines):
            match = SCORE_RE.search(line)
            if match is not None:
                return match.group(1), int(match.group(2))
        raise AssertionError(f"engine returned no score for {fen!r}: {lines!r}")

    def new_game(self) -> None:
        self.send("ucinewgame")
        self.send("isready")
        self.read_until("readyok")

    def set_position(self, fen: str, moves: list[str]) -> None:
        command = f"position fen {fen}"
        if moves:
            command += " moves " + " ".join(moves)
        self.send(command)

    def bestmove(self, fen: str, moves: list[str], depth: int) -> str:
        self.set_position(fen, moves)
        self.send(f"go depth {depth}")
        lines = self.read_until("bestmove ")
        parts = lines[-1].split()
        if len(parts) < 2:
            raise RuntimeError(f"malformed bestmove response: {lines[-1]!r}")
        return parts[1]

    def display(self, fen: str, moves: list[str]) -> tuple[str, bool]:
        self.set_position(fen, moves)
        self.send("d")
        lines = self.read_until("Checkers:")
        actual_fen = ""
        in_check = False
        for line in lines:
            if line.startswith("Fen: "):
                actual_fen = line[5:]
            if line.startswith("Checkers:"):
                in_check = len(line[len("Checkers:") :].strip()) > 0
        if not actual_fen:
            raise RuntimeError(f"validator returned no FEN: {lines!r}")
        return actual_fen, in_check

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
    "KNvK (bare king in check)": "7k/5N2/4K3/8/8/8/8/8 b - - 0 1",
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


def test_minor_mop_up(engine: Engine) -> list[str]:
    failures: list[str] = []
    _, kbb_corner = engine.score(
        "7k/8/4K3/8/8/BB6/8/8 w - - 0 1", depth=1
    )
    _, kbb_centre = engine.score(
        "8/8/8/4k3/8/BB6/2K5/8 w - - 0 1", depth=1
    )
    if kbb_corner - kbb_centre < 120:
        failures.append(
            "KBBvK: expected a corner mop-up margin of at least 120 cp, "
            f"got {kbb_corner - kbb_centre} cp"
        )

    # The c1 bishop controls a1/h8. The otherwise identical wrong-corner
    # position puts the bare king on a8, which KBN cannot mate in.
    _, kbn_right_corner = engine.score(
        "7k/8/4K3/8/8/3N4/8/2B5 w - - 0 1", depth=1
    )
    _, kbn_wrong_corner = engine.score(
        "k7/8/4K3/8/8/3N4/8/2B5 w - - 0 1", depth=1
    )
    if kbn_right_corner - kbn_wrong_corner < 100:
        failures.append(
            "KBNvK: expected the bishop-controlled corner to lead by at least "
            f"100 cp, got {kbn_right_corner - kbn_wrong_corner} cp"
        )
    return failures


def king_distance(a: int, b: int) -> int:
    return max(abs(a % 8 - b % 8), abs(a // 8 - b // 8))


def slider_attacks(piece: str, source: int, target: int, blocker: int) -> bool:
    file_delta = target % 8 - source % 8
    rank_delta = target // 8 - source // 8
    if file_delta == 0:
        file_step = 0
    else:
        file_step = 1 if file_delta > 0 else -1
    if rank_delta == 0:
        rank_step = 0
    else:
        rank_step = 1 if rank_delta > 0 else -1

    straight = file_delta == 0 or rank_delta == 0
    diagonal = abs(file_delta) == abs(rank_delta)
    if piece == "R" and not straight:
        return False
    if piece == "Q" and not (straight or diagonal):
        return False

    file = source % 8 + file_step
    rank = source // 8 + rank_step
    while file != target % 8 or rank != target // 8:
        if rank * 8 + file == blocker:
            return False
        file += file_step
        rank += rank_step
    return True


def fen_for_kx(white_king: int, piece_square: int, black_king: int, piece: str) -> str:
    occupied = {
        white_king: "K",
        piece_square: piece,
        black_king: "k",
    }
    ranks: list[str] = []
    for rank in range(7, -1, -1):
        empty = 0
        text = ""
        for file in range(8):
            value = occupied.get(rank * 8 + file)
            if value is None:
                empty += 1
            else:
                if empty:
                    text += str(empty)
                    empty = 0
                text += value
        if empty:
            text += str(empty)
        ranks.append(text)
    return "/".join(ranks) + " w - - 0 1"


def random_kx_positions(piece: str, count: int, seed: int) -> list[str]:
    rng = random.Random(seed)
    positions: list[str] = []
    seen: set[tuple[int, int, int]] = set()
    while len(positions) < count:
        white_king, piece_square, black_king = rng.sample(range(64), 3)
        signature = (white_king, piece_square, black_king)
        if signature in seen or king_distance(white_king, black_king) <= 1:
            continue
        seen.add(signature)
        # White is to move, so the preceding black move must not have left the
        # black king in check. This makes every generated FEN a legal quiet
        # starting position rather than merely a syntactically valid one.
        if slider_attacks(piece, piece_square, black_king, white_king):
            continue
        positions.append(fen_for_kx(white_king, piece_square, black_king, piece))
    return positions


@dataclass
class ConversionResult:
    converted: bool
    plies: int
    reason: str
    moves: list[str]


def play_kx_game(
    engine: Engine,
    defender: Engine,
    fen: str,
    engine_depth: int,
    defender_depth: int,
    max_plies: int,
) -> ConversionResult:
    engine.new_game()
    defender.new_game()
    moves: list[str] = []
    previous_fen, _ = defender.display(fen, moves)

    for ply in range(max_plies):
        player = engine if ply % 2 == 0 else defender
        depth = engine_depth if ply % 2 == 0 else defender_depth
        move = player.bestmove(fen, moves, depth)
        if move in {"0000", "(none)"}:
            _, in_check = defender.display(fen, moves)
            converted = ply % 2 == 1 and in_check
            reason = "checkmate" if converted else "stalemate"
            return ConversionResult(converted, ply, reason, moves)

        moves.append(move)
        actual_fen, _ = defender.display(fen, moves)
        if actual_fen == previous_fen:
            raise AssertionError(
                f"illegal move {move!r} at ply {ply + 1} from {previous_fen!r}"
            )
        previous_fen = actual_fen

    # With an even max_plies the defender (bare king) always moves on odd
    # plies, so every checkmate or stalemate it faces is detected in-loop on
    # that move. A loop that runs to completion is therefore always a
    # non-conversion: the 50-move limit was reached without mate.
    return ConversionResult(False, max_plies, "50-move limit", moves)


def test_conversion(
    engine: Engine,
    stockfish_command: str,
    pieces: str,
    positions_per_piece: int,
    seed: int,
    engine_depth: int,
    defender_depth: int,
) -> list[str]:
    defender = Engine(stockfish_command)
    failures: list[str] = []
    started = time.monotonic()
    converted = 0
    total = 0
    try:
        for piece_index, piece in enumerate(pieces):
            positions = random_kx_positions(
                piece, positions_per_piece, seed + piece_index
            )
            for fen in positions:
                total += 1
                result = play_kx_game(
                    engine,
                    defender,
                    fen,
                    engine_depth,
                    defender_depth,
                    100,
                )
                if result.converted:
                    converted += 1
                else:
                    failures.append(
                        f"K{piece}vK {fen}: {result.reason} after {result.plies} "
                        f"plies; moves={' '.join(result.moves)}"
                    )
    finally:
        defender.close()

    elapsed = time.monotonic() - started
    print(
        f"KX conversion: {converted}/{total} checkmates within 100 plies "
        f"(seed {seed}, engine depth {engine_depth}, defender depth "
        f"{defender_depth}, {elapsed:.1f}s)"
    )
    return failures


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--engine", required=True, help="UCI engine command")
    parser.add_argument("--stockfish", default="stockfish", help="Stockfish command")
    parser.add_argument("--pieces", default="QR", choices=("Q", "R", "QR"))
    parser.add_argument("--positions-per-piece", type=int, default=10)
    parser.add_argument("--seed", type=int, default=909)
    parser.add_argument("--engine-depth", type=int, default=4)
    parser.add_argument("--defender-depth", type=int, default=8)
    parser.add_argument(
        "--draws-only", action="store_true", help="skip seeded conversion games"
    )
    args = parser.parse_args()

    engine = Engine(args.engine)
    try:
        failures = test_insufficient_material(engine)
        failures.extend(test_minor_mop_up(engine))
        if not args.draws_only:
            failures.extend(
                test_conversion(
                    engine,
                    args.stockfish,
                    args.pieces,
                    args.positions_per_piece,
                    args.seed,
                    args.engine_depth,
                    args.defender_depth,
                )
            )
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
