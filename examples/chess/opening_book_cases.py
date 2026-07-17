"""Readable source positions for the generated Vow opening book.

Each entry is ``(move history from startpos, next move)`` in UCI notation.
Histories model the common opponent branches while each stored position has one
deterministic reply.  ``generate_opening_book.py`` validates and compiles this
data; ``test_opening_book.py`` exercises the same entries through the UCI seam.
"""

from __future__ import annotations


BookEntry = tuple[tuple[str, ...], str]


BOOK_ENTRIES: tuple[BookEntry, ...] = (
    # White starts with 1.e4; as Black, answer 1.e4 with the Sicilian.
    ((), "e2e4"),
    (("e2e4",), "c7c5"),
    (("e2e4", "c7c5"), "g1f3"),
    (("e2e4", "c7c5", "g1f3"), "d7d6"),
    (("e2e4", "c7c5", "g1f3", "d7d6"), "d2d4"),
    (("e2e4", "c7c5", "g1f3", "d7d6", "d2d4"), "c5d4"),
    (("e2e4", "c7c5", "g1f3", "d7d6", "d2d4", "c5d4"), "f3d4"),
    # Open Sicilian branches when the opponent chooses 2...Nc6 or 2...e6.
    (("e2e4", "c7c5", "g1f3", "b8c6"), "d2d4"),
    (("e2e4", "c7c5", "g1f3", "b8c6", "d2d4"), "c5d4"),
    (("e2e4", "c7c5", "g1f3", "b8c6", "d2d4", "c5d4"), "f3d4"),
    (("e2e4", "c7c5", "g1f3", "e7e6"), "d2d4"),
    (("e2e4", "c7c5", "g1f3", "e7e6", "d2d4"), "c5d4"),
    (("e2e4", "c7c5", "g1f3", "e7e6", "d2d4", "c5d4"), "f3d4"),
    # 1...e5: Ruy Lopez, Morphy Defence.
    (("e2e4", "e7e5"), "g1f3"),
    (("e2e4", "e7e5", "g1f3"), "b8c6"),
    (("e2e4", "e7e5", "g1f3", "b8c6"), "f1b5"),
    (("e2e4", "e7e5", "g1f3", "b8c6", "f1b5"), "a7a6"),
    (("e2e4", "e7e5", "g1f3", "b8c6", "f1b5", "a7a6"), "b5a4"),
    (("e2e4", "e7e5", "g1f3", "b8c6", "f1b5", "a7a6", "b5a4"), "g8f6"),
    (("e2e4", "e7e5", "g1f3", "b8c6", "f1b5", "a7a6", "b5a4", "g8f6"), "e1g1"),
    # French Defence, Classical main line.
    (("e2e4", "e7e6"), "d2d4"),
    (("e2e4", "e7e6", "d2d4"), "d7d5"),
    (("e2e4", "e7e6", "d2d4", "d7d5"), "b1c3"),
    (("e2e4", "e7e6", "d2d4", "d7d5", "b1c3"), "g8f6"),
    (("e2e4", "e7e6", "d2d4", "d7d5", "b1c3", "g8f6"), "e4e5"),
    # Caro-Kann, Advance variation.
    (("e2e4", "c7c6"), "d2d4"),
    (("e2e4", "c7c6", "d2d4"), "d7d5"),
    (("e2e4", "c7c6", "d2d4", "d7d5"), "e4e5"),
    (("e2e4", "c7c6", "d2d4", "d7d5", "e4e5"), "c8f5"),
    (("e2e4", "c7c6", "d2d4", "d7d5", "e4e5", "c8f5"), "g1f3"),
    # As Black, answer 1.d4 with 1...Nf6 and enter a Nimzo-Indian.
    (("d2d4",), "g8f6"),
    (("d2d4", "g8f6"), "c2c4"),
    (("d2d4", "g8f6", "c2c4"), "e7e6"),
    (("d2d4", "g8f6", "c2c4", "e7e6"), "b1c3"),
    (("d2d4", "g8f6", "c2c4", "e7e6", "b1c3"), "f8b4"),
    (("d2d4", "g8f6", "c2c4", "e7e6", "b1c3", "f8b4"), "e2e3"),
    # King's Indian branch after an opponent's 2...g6.
    (("d2d4", "g8f6", "c2c4", "g7g6"), "b1c3"),
    (("d2d4", "g8f6", "c2c4", "g7g6", "b1c3"), "f8g7"),
    (("d2d4", "g8f6", "c2c4", "g7g6", "b1c3", "f8g7"), "e2e4"),
    (("d2d4", "g8f6", "c2c4", "g7g6", "b1c3", "f8g7", "e2e4"), "d7d6"),
    (("d2d4", "g8f6", "c2c4", "g7g6", "b1c3", "f8g7", "e2e4", "d7d6"), "g1f3"),
    # Queen's Gambit Declined.
    (("d2d4", "d7d5"), "c2c4"),
    (("d2d4", "d7d5", "c2c4"), "e7e6"),
    (("d2d4", "d7d5", "c2c4", "e7e6"), "b1c3"),
    (("d2d4", "d7d5", "c2c4", "e7e6", "b1c3"), "g8f6"),
    (("d2d4", "d7d5", "c2c4", "e7e6", "b1c3", "g8f6"), "c1g5"),
    # Slav branch after an opponent's 2...c6.
    (("d2d4", "d7d5", "c2c4", "c7c6"), "g1f3"),
    (("d2d4", "d7d5", "c2c4", "c7c6", "g1f3"), "g8f6"),
    (("d2d4", "d7d5", "c2c4", "c7c6", "g1f3", "g8f6"), "b1c3"),
)
