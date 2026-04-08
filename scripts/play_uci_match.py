#!/usr/bin/env python3
import argparse
import re
import shlex
import subprocess
import sys
from pathlib import Path


MOVE_RE = re.compile(r"^[a-h][1-8][a-h][1-8][nbrq]?$")


class Engine:
    def __init__(self, cmd: list[str], cwd: Path):
        self.cmd = cmd
        self.name = cmd[0]
        self.proc = subprocess.Popen(
            cmd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
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
                raise RuntimeError(f"{self.name}: EOF while waiting for {token!r}; partial={out!r}")
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
                raise RuntimeError(f"{self.name}: EOF while waiting for bestmove; partial={out!r}")
            text = line.rstrip("\n")
            out.append(text)
            if text.startswith("bestmove "):
                parts = text.split()
                if len(parts) < 2:
                    raise RuntimeError(f"{self.name}: malformed bestmove line {text!r}")
                return parts[1], out

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


def init_engine(engine: Engine) -> None:
    engine.send("uci")
    engine.read_until("uciok")
    engine.send("isready")
    engine.read_until("readyok")
    engine.send("ucinewgame")


def position_command(moves: list[str]) -> str:
    if not moves:
        return "position startpos"
    return "position startpos moves " + " ".join(moves)


def fen_for_moves(engine: Engine, moves: list[str]) -> str:
    engine.send(position_command(moves))
    engine.send("d")
    if engine.proc.stdout is None:
        raise RuntimeError(f"{engine.name}: stdout unavailable")
    while True:
        line = engine.proc.stdout.readline()
        if line == "":
            raise RuntimeError(f"{engine.name}: EOF while waiting for Fen line")
        text = line.rstrip("\n")
        if text.startswith("Fen: "):
            return text[5:]


def split_cmd(text: str) -> list[str]:
    return shlex.split(text)


def main() -> int:
    parser = argparse.ArgumentParser(description="Play a short game between two UCI engines.")
    parser.add_argument("--white", required=True, help="White engine command")
    parser.add_argument("--black", required=True, help="Black engine command")
    parser.add_argument(
        "--validator",
        help="Optional validator engine command (must support the Stockfish-specific 'd' command) used to confirm that each move changes the position",
    )
    parser.add_argument("--white-go", default="go movetime 100", help="UCI go command for White")
    parser.add_argument("--black-go", default="go movetime 100", help="UCI go command for Black")
    parser.add_argument("--plies", type=int, default=20, help="Maximum plies to play")
    parser.add_argument(
        "--cwd",
        default=".",
        help="Working directory used to launch engine binaries",
    )
    args = parser.parse_args()

    cwd = Path(args.cwd).resolve()
    white = Engine(split_cmd(args.white), cwd)
    black = Engine(split_cmd(args.black), cwd)
    validator = Engine(split_cmd(args.validator), cwd) if args.validator else None

    try:
        init_engine(white)
        init_engine(black)
        if validator is not None:
            init_engine(validator)

        moves: list[str] = []
        prev_fen = fen_for_moves(validator, moves) if validator is not None else ""

        for ply in range(args.plies):
            if ply % 2 == 0:
                side = "white"
                engine = white
                go_cmd = args.white_go
            else:
                side = "black"
                engine = black
                go_cmd = args.black_go

            engine.send(position_command(moves))
            engine.send(go_cmd)
            move, _ = engine.read_bestmove()

            if move in {"(none)", "0000"}:
                print(f"{side} has no move after {' '.join(moves)}")
                break
            if MOVE_RE.match(move) is None:
                raise RuntimeError(f"{side}: malformed bestmove {move!r}")

            next_moves = moves + [move]
            if validator is not None:
                next_fen = fen_for_moves(validator, next_moves)
                if next_fen == prev_fen:
                    raise RuntimeError(
                        f"{side}: move {move!r} did not change validator position after {' '.join(moves)!r}"
                    )
                prev_fen = next_fen

            print(f"{ply + 1:02d}. {side} {move}")
            moves = next_moves

        if validator is not None:
            print(f"final_fen {prev_fen}")
        print("moves", " ".join(moves))
        return 0
    finally:
        white.quit()
        black.quit()
        if validator is not None:
            validator.quit()


if __name__ == "__main__":
    sys.exit(main())
