#!/usr/bin/env python3
"""Black-box regression test for the generated opening book."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from opening_book_cases import BOOK_ENTRIES
from play_uci_match import Engine, MOVE_RE, init_engine, position_command


TRANSPOSITION_HISTORY = (
    "d2d4",
    "g8f6",
    "c2c4",
    "g7g6",
    "b1c3",
    "d7d6",
    "e2e4",
    "f8g7",
)


def check_book(engine: Engine) -> list[str]:
    failures: list[str] = []
    for history, expected in BOOK_ENTRIES:
        engine.send(position_command(list(history)))
        engine.send("go depth 1")
        actual, lines = engine.read_bestmove()
        context = "startpos" if not history else " ".join(history)
        if actual != expected:
            failures.append(f"{context}: expected {expected}, got {actual}")
        if any(line.startswith("info depth ") for line in lines):
            failures.append(f"{context}: book hit entered search")
    return failures


def check_off_book_fallback(engine: Engine) -> list[str]:
    engine.send(position_command(["a2a3"]))
    engine.send("go depth 1")
    move, lines = engine.read_bestmove()
    failures: list[str] = []
    if MOVE_RE.fullmatch(move) is None:
        failures.append(f"off-book fallback returned malformed move {move!r}")
    if not any(line.startswith("info depth 1 ") for line in lines):
        failures.append("off-book position did not enter depth-1 search")
    return failures


def check_transposition(engine: Engine) -> list[str]:
    # The stored King's Indian position reaches the same board with ...Bg7
    # before ...d6. A sequence trie would miss this alternate move order.
    engine.send(position_command(list(TRANSPOSITION_HISTORY)))
    engine.send("go depth 1")
    move, lines = engine.read_bestmove()
    failures: list[str] = []
    if move != "g1f3":
        failures.append(f"transposed King's Indian: expected g1f3, got {move}")
    if any(line.startswith("info depth ") for line in lines):
        failures.append("transposed King's Indian position entered search")
    return failures


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("engine", type=Path, help="compiled Vow chess engine")
    args = parser.parse_args()

    engine_path = args.engine.resolve()
    engine = Engine([str(engine_path)], Path.cwd(), engine_id="book-test")
    try:
        init_engine(engine)
        failures = check_book(engine)
        failures.extend(check_transposition(engine))
        failures.extend(check_off_book_fallback(engine))
    finally:
        engine.quit()

    if failures:
        for failure in failures:
            print(f"FAIL: {failure}", file=sys.stderr)
        print(f"opening book: {len(failures)} failure(s)", file=sys.stderr)
        return 1

    print(
        f"opening book: {len(BOOK_ENTRIES)} legal root hits, transposition, "
        "and fallback passed"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
