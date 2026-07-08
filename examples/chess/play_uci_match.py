#!/usr/bin/env python3
"""Play N games between two UCI engines with optional color alternation,
UCI option passthrough, validator-based evaluation, and match scoring.

Example usage:

  # 10 games, alternating colors, Stockfish limited to ~1350 Elo
  python examples/chess/play_uci_match.py \
      --white examples/chess/.local/chess \
      --black stockfish \
      --black-option "UCI_LimitStrength=true" \
      --black-option "UCI_Elo=1350" \
      --games 10 --alternate-colors \
      --plies 200 \
      --validator stockfish \
      --log match.log

  # Quick single game (backwards-compatible with the old interface)
  python examples/chess/play_uci_match.py --white examples/chess/.local/chess --black stockfish
"""
from __future__ import annotations

import argparse
import re
import select
import shlex
import subprocess
import sys
from pathlib import Path


MOVE_RE = re.compile(r"^[a-h][1-8][a-h][1-8][nbrq]?$")
SCORE_RE = re.compile(r"score (cp (-?\d+)|mate (-?\d+))")


# ---------------------------------------------------------------------------
# Engine wrapper
# ---------------------------------------------------------------------------

class Engine:
    def __init__(self, cmd: list[str], cwd: Path, engine_id: str = ""):
        self.cmd = cmd
        self.name = cmd[0]
        self.engine_id = engine_id
        self.proc = subprocess.Popen(
            cmd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            bufsize=1,
            cwd=str(cwd),
        )

    def send(self, line: str) -> None:
        if self.proc.stdin is None:
            raise RuntimeError(f"{self.name}: stdin unavailable")
        self.proc.stdin.write(line + "\n")
        self.proc.stdin.flush()

    def read_until(self, token: str) -> list[str]:
        if self.proc.stdout is None:
            raise RuntimeError(f"{self.name}: stdout unavailable")
        out: list[str] = []
        while True:
            line = self.proc.stdout.readline()
            if line == "":
                raise RuntimeError(
                    f"{self.name}: EOF while waiting for {token!r}; partial={out!r}"
                )
            text = line.rstrip("\n")
            out.append(text)
            if text == token:
                return out

    def read_bestmove(self) -> tuple[str, list[str]]:
        if self.proc.stdout is None:
            raise RuntimeError(f"{self.name}: stdout unavailable")
        out: list[str] = []
        while True:
            line = self.proc.stdout.readline()
            if line == "":
                raise RuntimeError(
                    f"{self.name}: EOF while waiting for bestmove; partial={out!r}"
                )
            text = line.rstrip("\n")
            out.append(text)
            if text.startswith("bestmove "):
                parts = text.split()
                if len(parts) < 2:
                    raise RuntimeError(f"{self.name}: malformed bestmove line {text!r}")
                return parts[1], out

    def read_line_timeout(self, timeout: float = 5.0) -> str | None:
        """Read a single line with a timeout. Returns None on timeout."""
        if self.proc.stdout is None:
            raise RuntimeError(f"{self.name}: stdout unavailable")
        ready, _, _ = select.select([self.proc.stdout], [], [], timeout)
        if not ready:
            return None
        line = self.proc.stdout.readline()
        if line == "":
            return None
        return line.rstrip("\n")

    def quit(self) -> None:
        try:
            self.send("quit")
        except Exception:
            pass
        try:
            self.proc.wait(timeout=3)
        except Exception:
            self.proc.kill()
            self.proc.wait(timeout=3)


# ---------------------------------------------------------------------------
# Engine lifecycle helpers
# ---------------------------------------------------------------------------

def init_engine(engine: Engine, options: list[str] | None = None) -> None:
    engine.send("uci")
    engine.read_until("uciok")
    if options:
        for opt in options:
            engine.send(f"setoption {opt}")
    engine.send("isready")
    engine.read_until("readyok")


def new_game(engine: Engine) -> None:
    engine.send("ucinewgame")
    engine.send("isready")
    engine.read_until("readyok")


def position_command(moves: list[str]) -> str:
    if not moves:
        return "position startpos"
    return "position startpos moves " + " ".join(moves)


# ---------------------------------------------------------------------------
# Validator helpers
# ---------------------------------------------------------------------------

def fen_for_moves(engine: Engine, moves: list[str], timeout: float = 5.0) -> str:
    """Extract the FEN for a position using the Stockfish ``d`` command.

    The ``d`` (display) command is a Stockfish extension -- it is **not** part
    of the UCI standard.  The ``--validator`` engine must therefore be Stockfish
    or another engine that implements ``d``."""
    engine.send(position_command(moves))
    engine.send("d")
    fen: str | None = None
    while True:
        text = engine.read_line_timeout(timeout)
        if text is None:
            raise RuntimeError(
                f"{engine.name}: timed out after {timeout}s waiting for 'Fen:' line "
                f"(does this engine support the Stockfish-specific 'd' command?)"
            )
        if text.startswith("Fen: "):
            fen = text[5:]
            break
    # Drain remaining ``d`` output via the standard UCI sync barrier.
    engine.send("isready")
    engine.read_until("readyok")
    assert fen is not None
    return fen


def eval_position(engine: Engine, moves: list[str], movetime: int = 200) -> float:
    """Use the validator to evaluate a position. Returns score in pawns from
    white's perspective. Mate scores are mapped to +/-100."""
    engine.send(position_command(moves))
    engine.send(f"go movetime {movetime}")
    _, lines = engine.read_bestmove()
    cp = 0.0
    for line in reversed(lines):
        m = SCORE_RE.search(line)
        if m:
            if m.group(2) is not None:
                cp = int(m.group(2)) / 100.0
            elif m.group(3) is not None:
                mate_in = int(m.group(3))
                cp = 100.0 if mate_in > 0 else -100.0
            break
    # UCI scores are relative to the side to move; negate for Black's turn.
    if len(moves) % 2 == 1:
        cp = -cp
    return cp


# ---------------------------------------------------------------------------
# Game result
# ---------------------------------------------------------------------------

class GameResult:
    def __init__(
        self,
        game_num: int,
        white_name: str,
        black_name: str,
        white_is_a: bool,
        moves: list[str],
        outcome: str,
        eval_score: float | None,
        final_fen: str | None,
    ):
        self.game_num = game_num
        self.white_name = white_name
        self.black_name = black_name
        self.white_is_a = white_is_a
        self.moves = moves
        self.outcome = outcome  # "1-0", "0-1", "1/2-1/2", "unfinished"
        self.eval_score = eval_score
        self.final_fen = final_fen

    def white_score(self) -> float:
        if self.outcome == "1-0":
            return 1.0
        if self.outcome == "0-1":
            return 0.0
        if self.outcome == "1/2-1/2":
            return 0.5
        # Unfinished: use eval if available
        if self.eval_score is not None:
            if self.eval_score > 1.0:
                return 1.0
            if self.eval_score < -1.0:
                return 0.0
            return 0.5
        return 0.5


# ---------------------------------------------------------------------------
# Play one game
# ---------------------------------------------------------------------------

def play_game(
    game_num: int,
    white: Engine,
    black: Engine,
    white_go: str,
    black_go: str,
    max_plies: int,
    validator: Engine | None,
) -> GameResult:
    new_game(white)
    new_game(black)
    if validator is not None:
        new_game(validator)

    moves: list[str] = []
    prev_fen = fen_for_moves(validator, moves) if validator is not None else ""
    outcome = "unfinished"

    for ply in range(max_plies):
        if ply % 2 == 0:
            engine = white
            go_cmd = white_go
        else:
            engine = black
            go_cmd = black_go

        engine.send(position_command(moves))
        engine.send(go_cmd)
        move, _ = engine.read_bestmove()

        if move in {"(none)", "0000"}:
            # No legal move: checkmate (loss) or stalemate (draw).
            # When a validator is available, use its eval to distinguish:
            # a large score means likely checkmate; near-zero means stalemate.
            if validator is not None:
                ev = eval_position(validator, moves)
                if abs(ev) < 2.0:
                    outcome = "1/2-1/2"
                else:
                    outcome = "0-1" if ply % 2 == 0 else "1-0"
            else:
                # Without a validator we cannot distinguish; default to loss.
                outcome = "0-1" if ply % 2 == 0 else "1-0"
            break
        if MOVE_RE.match(move) is None:
            raise RuntimeError(
                f"game {game_num}: malformed bestmove {move!r} at ply {ply + 1}"
            )

        next_moves = moves + [move]
        if validator is not None:
            next_fen = fen_for_moves(validator, next_moves)
            if next_fen == prev_fen:
                raise RuntimeError(
                    f"game {game_num}: move {move!r} did not change position at ply {ply + 1}"
                )
            prev_fen = next_fen

        moves = next_moves

    eval_score = None
    final_fen = None
    if validator is not None:
        final_fen = fen_for_moves(validator, moves)
        if outcome == "unfinished":
            eval_score = eval_position(validator, moves)

    return GameResult(
        game_num=game_num,
        white_name=white.name,
        black_name=black.name,
        white_is_a=white.engine_id == "A",
        moves=moves,
        outcome=outcome,
        eval_score=eval_score,
        final_fen=final_fen,
    )


# ---------------------------------------------------------------------------
# Printing
# ---------------------------------------------------------------------------

def print_game_summary(result: GameResult) -> None:
    plies = len(result.moves)
    score_str = result.outcome
    if result.outcome == "unfinished" and result.eval_score is not None:
        score_str = f"unfinished (eval {result.eval_score:+.2f})"
    print(f"  game {result.game_num:3d}: {result.white_name} vs {result.black_name}"
          f"  {score_str}  ({plies} plies)")


def print_match_summary(
    results: list[GameResult],
    engine_a_name: str,
    engine_b_name: str,
) -> None:
    a_score = 0.0
    b_score = 0.0
    wins_a = draws = wins_b = 0

    for r in results:
        ws = r.white_score()
        if r.white_is_a:
            a_score += ws
            b_score += (1.0 - ws)
        else:
            b_score += ws
            a_score += (1.0 - ws)

        if ws == 1.0:
            if r.white_is_a:
                wins_a += 1
            else:
                wins_b += 1
        elif ws == 0.0:
            if r.white_is_a:
                wins_b += 1
            else:
                wins_a += 1
        else:
            draws += 1

    total = len(results)
    print()
    print("=" * 60)
    print(f"Match result: {engine_a_name} vs {engine_b_name}")
    print(f"  Games:  {total}")
    print(f"  Score:  {engine_a_name} {a_score:.1f} - {b_score:.1f} {engine_b_name}")
    print(f"  W/D/L:  +{wins_a} ={draws} -{wins_b} (from {engine_a_name}'s perspective)")
    if total > 0:
        pct = (a_score / total) * 100
        print(f"  Win %%:  {pct:.1f}%")
    print("=" * 60)


def write_log(path: Path, results: list[GameResult]) -> None:
    with open(path, "w") as f:
        for r in results:
            f.write(f"[Game {r.game_num}]\n")
            f.write(f"White: {r.white_name}\n")
            f.write(f"Black: {r.black_name}\n")
            f.write(f"Result: {r.outcome}\n")
            if r.eval_score is not None:
                f.write(f"Eval: {r.eval_score:+.2f}\n")
            if r.final_fen is not None:
                f.write(f"FinalFEN: {r.final_fen}\n")
            f.write(f"Moves: {' '.join(r.moves)}\n")
            f.write("\n")


# ---------------------------------------------------------------------------
# CLI helpers
# ---------------------------------------------------------------------------

def split_cmd(text: str) -> list[str]:
    return shlex.split(text)


def parse_options(raw: list[str] | None) -> list[str]:
    """Convert ['UCI_LimitStrength=true', 'UCI_Elo=1350'] to
    ['name UCI_LimitStrength value true', 'name UCI_Elo value 1350']."""
    if not raw:
        return []
    result = []
    for item in raw:
        if "=" in item:
            name, value = item.split("=", 1)
            result.append(f"name {name} value {value}")
        else:
            result.append(f"name {item}")
    return result


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main() -> int:
    parser = argparse.ArgumentParser(
        description="Play N games between two UCI engines and report match results.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    parser.add_argument("--white", required=True, help="White engine command")
    parser.add_argument("--black", required=True, help="Black engine command")
    parser.add_argument(
        "--validator",
        help="Optional Stockfish-compatible validator (must support the 'd' command; confirms moves change position, evaluates unfinished games)",
    )
    parser.add_argument("--white-go", default="go movetime 100",
                        help="UCI go command for White (default: go movetime 100)")
    parser.add_argument("--black-go", default="go movetime 100",
                        help="UCI go command for Black (default: go movetime 100)")
    parser.add_argument("--plies", type=int, default=200,
                        help="Maximum plies per game (default: 200)")
    parser.add_argument("--games", type=int, default=1,
                        help="Number of games to play (default: 1)")
    parser.add_argument("--alternate-colors", action="store_true",
                        help="Swap engine colors each game")
    parser.add_argument("--white-option", action="append", dest="white_options",
                        help="UCI option for White engine (e.g. UCI_Elo=1350). Repeatable.")
    parser.add_argument("--black-option", action="append", dest="black_options",
                        help="UCI option for Black engine (e.g. UCI_LimitStrength=true). Repeatable.")
    parser.add_argument("--validator-option", action="append", dest="validator_options",
                        help="UCI option for validator engine. Repeatable.")
    parser.add_argument("--log", type=Path,
                        help="Write per-game move log to this file")
    parser.add_argument("--cwd", default=".",
                        help="Working directory for launching engine binaries")
    args = parser.parse_args()

    cwd = Path(args.cwd).resolve()
    white_opts = parse_options(args.white_options)
    black_opts = parse_options(args.black_options)
    validator_opts = parse_options(args.validator_options)

    # Launch engines once; reuse across games.
    engine_a = Engine(split_cmd(args.white), cwd, engine_id="A")
    engine_b = Engine(split_cmd(args.black), cwd, engine_id="B")
    validator = Engine(split_cmd(args.validator), cwd) if args.validator else None

    engine_a_name = engine_a.name
    engine_b_name = engine_b.name

    try:
        init_engine(engine_a, white_opts)
        init_engine(engine_b, black_opts)
        if validator is not None:
            init_engine(validator, validator_opts)

        results: list[GameResult] = []
        for g in range(args.games):
            swap = args.alternate_colors and g % 2 == 1
            if swap:
                white, black = engine_b, engine_a
            else:
                white, black = engine_a, engine_b

            result = play_game(
                game_num=g + 1,
                white=white,
                black=black,
                white_go=args.white_go,
                black_go=args.black_go,
                max_plies=args.plies,
                validator=validator,
            )
            results.append(result)
            print_game_summary(result)

        print_match_summary(results, engine_a_name, engine_b_name)

        if args.log:
            write_log(args.log, results)
            print(f"\nGame log written to {args.log}")

        return 0
    finally:
        engine_a.quit()
        engine_b.quit()
        if validator is not None:
            validator.quit()


if __name__ == "__main__":
    sys.exit(main())
