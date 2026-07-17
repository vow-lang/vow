#!/usr/bin/env python3
"""Validate readable opening lines and emit their exact Vow Zobrist keys."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

from opening_book_cases import BOOK_ENTRIES


MASK64 = (1 << 64) - 1
SPLITMIX_SEED = 8378711321616135283
SPLITMIX_INCREMENT = 11400714819323198485
SPLITMIX_MUL1 = 13787848793156543929
SPLITMIX_MUL2 = 10723151780598845931
ZOB_SIZE = 781
MOVE_RE = re.compile(r"^[a-h][1-8][a-h][1-8][nbrq]?$")
PIECES = {
    "P": 1,
    "N": 2,
    "B": 3,
    "R": 4,
    "Q": 5,
    "K": 6,
    "p": -1,
    "n": -2,
    "b": -3,
    "r": -4,
    "q": -5,
    "k": -6,
}
PROMOTIONS = {"n": 2, "b": 3, "r": 4, "q": 5}


class Stockfish:
    def __init__(self, command: str):
        self.proc = subprocess.Popen(
            [command],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            bufsize=1,
        )
        self.send("uci")
        self.read_until("uciok")
        self.send("isready")
        self.read_until("readyok")

    def send(self, line: str) -> None:
        if self.proc.stdin is None:
            raise RuntimeError("Stockfish stdin is unavailable")
        self.proc.stdin.write(line + "\n")
        self.proc.stdin.flush()

    def read_until(self, expected: str) -> None:
        if self.proc.stdout is None:
            raise RuntimeError("Stockfish stdout is unavailable")
        for line in self.proc.stdout:
            if line.rstrip("\n") == expected:
                return
        raise RuntimeError(f"Stockfish exited before emitting {expected!r}")

    def fen(self, history: tuple[str, ...]) -> str:
        command = "position startpos"
        if history:
            command += " moves " + " ".join(history)
        self.send(command)
        self.send("d")
        if self.proc.stdout is None:
            raise RuntimeError("Stockfish stdout is unavailable")
        fen = ""
        for line in self.proc.stdout:
            if line.startswith("Fen: "):
                fen = line[5:].strip()
                break
        if not fen:
            raise RuntimeError(f"Stockfish did not report a FEN for {history!r}")
        self.send("isready")
        self.read_until("readyok")
        return fen

    def close(self) -> None:
        try:
            self.send("quit")
            self.proc.wait(timeout=3)
        except Exception:
            self.proc.kill()
            self.proc.wait(timeout=3)


def build_ztab() -> list[int]:
    table: list[int] = []
    state = SPLITMIX_SEED
    for _ in range(ZOB_SIZE):
        state = (state + SPLITMIX_INCREMENT) & MASK64
        value = state
        value = ((value ^ (value >> 30)) * SPLITMIX_MUL1) & MASK64
        value = ((value ^ (value >> 27)) * SPLITMIX_MUL2) & MASK64
        value ^= value >> 31
        table.append(value)
    return table


def square_index(square: str) -> int:
    return ord(square[0]) - ord("a") + (ord(square[1]) - ord("1")) * 8


def parse_fen_board(placement: str) -> list[int]:
    squares = [0] * 64
    ranks = placement.split("/")
    if len(ranks) != 8:
        raise ValueError(f"invalid FEN placement: {placement!r}")
    for fen_rank, encoded in enumerate(ranks):
        board_rank = 7 - fen_rank
        file_index = 0
        for char in encoded:
            if char.isdigit():
                file_index += int(char)
            elif char in PIECES and file_index < 8:
                squares[board_rank * 8 + file_index] = PIECES[char]
                file_index += 1
            else:
                raise ValueError(f"invalid FEN placement: {placement!r}")
        if file_index != 8:
            raise ValueError(f"invalid FEN placement: {placement!r}")
    return squares


def engine_ep_square(squares: list[int], history: tuple[str, ...]) -> int:
    if not history:
        return -1
    move = history[-1]
    from_square = square_index(move[0:2])
    to_square = square_index(move[2:4])
    if (
        abs(to_square - from_square) == 16
        and from_square % 8 == to_square % 8
        and abs(squares[to_square]) == 1
    ):
        return (from_square + to_square) // 2
    return -1


def zobrist_key(fen: str, history: tuple[str, ...], table: list[int]) -> int:
    fields = fen.split()
    if len(fields) < 4:
        raise ValueError(f"invalid FEN: {fen!r}")
    squares = parse_fen_board(fields[0])
    key = 0
    for square, piece in enumerate(squares):
        if piece > 0:
            piece_index = piece - 1
        elif piece < 0:
            piece_index = 5 - piece
        else:
            continue
        key ^= table[piece_index * 64 + square]
    if fields[1] == "b":
        key ^= table[768]
    castling = fields[2]
    if "K" in castling:
        key ^= table[769]
    if "Q" in castling:
        key ^= table[770]
    if "k" in castling:
        key ^= table[771]
    if "q" in castling:
        key ^= table[772]
    ep_square = engine_ep_square(squares, history)
    if ep_square >= 0:
        key ^= table[773 + ep_square % 8]
    return key if key < 1 << 63 else key - (1 << 64)


def move_key(move: str) -> int:
    promotion = PROMOTIONS.get(move[4:], 0)
    return square_index(move[0:2]) + square_index(move[2:4]) * 64 + promotion * 4096


def validated_positions(stockfish: Stockfish) -> list[tuple[tuple[str, ...], str]]:
    cache: dict[tuple[str, ...], str] = {(): stockfish.fen(())}

    def checked_fen(history: tuple[str, ...]) -> str:
        if history in cache:
            return cache[history]
        previous = checked_fen(history[:-1])
        current = stockfish.fen(history)
        if current == previous:
            raise ValueError(f"illegal move {history[-1]!r} after {' '.join(history[:-1])}")
        cache[history] = current
        return current

    positions: list[tuple[tuple[str, ...], str]] = []
    for history, move in BOOK_ENTRIES:
        if MOVE_RE.fullmatch(move) is None:
            raise ValueError(f"malformed book move: {move!r}")
        before = checked_fen(history)
        after = checked_fen((*history, move))
        if before == after:
            raise ValueError(f"illegal book move {move!r} after {' '.join(history)}")
        positions.append((history, before))
    return positions


def render_module(positions: list[tuple[tuple[str, ...], str]]) -> str:
    table = build_ztab()
    keyed: dict[int, tuple[str, tuple[str, ...]]] = {}
    for (history, fen), (_, move) in zip(positions, BOOK_ENTRIES, strict=True):
        key = zobrist_key(fen, history, table)
        previous = keyed.get(key)
        if previous is not None and previous[0] != move:
            raise ValueError(
                f"conflicting moves for Zobrist key {key}: {previous[0]} and {move}"
            )
        keyed[key] = (move, history)

    lines = [
        "module OpeningBook",
        "",
        "// Generated by generate_opening_book.py; do not edit by hand.",
        "// Keys use the fixed splitmix64 table and layout from chess/main.vow.",
        "fn opening_book_move_key(zkey: i64) -> i64 {",
    ]
    for key, (move, history) in keyed.items():
        position = "startpos" if not history else " ".join(history)
        lines.append(f"    // {position} -> {move}")
        lines.append(f"    if zkey == {key} {{ return {move_key(move)}; }}")
    lines.extend(("    -1", "}", ""))
    return "\n".join(lines)


def main() -> int:
    directory = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser()
    parser.add_argument("--stockfish", default="stockfish")
    parser.add_argument("--output", type=Path, default=directory / "opening_book.vow")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    stockfish = Stockfish(args.stockfish)
    try:
        rendered = render_module(validated_positions(stockfish))
    finally:
        stockfish.close()

    if args.check:
        current = args.output.read_text() if args.output.exists() else ""
        if current != rendered:
            print(f"stale generated opening book: {args.output}", file=sys.stderr)
            return 1
        print(f"opening book is current: {len(BOOK_ENTRIES)} legal entries")
        return 0

    args.output.write_text(rendered)
    print(f"generated {len(BOOK_ENTRIES)} legal entries in {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
