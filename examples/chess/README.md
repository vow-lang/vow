# Chess UCI Demo

`examples/chess/` is a self-contained [UCI](https://en.wikipedia.org/wiki/Universal_Chess_Interface)
chess engine written in Vow. It is a real, playable engine — full legal move
generation and an alpha-beta search — not a move-list toy.

## Scope

The engine (`main.vow`) implements:

- **Board & rules** — full legal move generation for all pieces, including
  castling, en passant, pawn double-pushes, and under-promotions. Validated by
  `perft` against known node counts (startpos, Kiwipete, and two edge-case
  positions) through the depths that exercise en passant, castling legality,
  promotions, and pins.
- **Search** — negamax with alpha-beta, quiescence (captures/promotions only),
  and time-managed iterative deepening. A Zobrist-keyed transposition table
  provides cutoffs and best-move ordering across iterations. Move ordering uses
  the TT move, MVV/LVA captures, killer moves, and a history heuristic; the tree
  is pruned with null-move pruning, depth/move-count table-driven late move
  reductions (LMR), conservative shallow futility/late-quiet pruning, a
  principal-variation search, and check extensions. Root iterative deepening
  uses aspiration windows with deterministic widening on fail-low or fail-high.
- **Evaluation** — phase-tapered PeSTO piece-square tables (separate midgame and
  endgame tables blended by material phase) for material and placement, plus
  bishop pair, development/castling, pawn structure (passed / doubled /
  isolated), and rook activity (open & semi-open files, 7th rank).

### UCI protocol

The engine speaks enough of UCI to play in standard GUIs and match runners:

| Command       | Behavior                                                              |
| ------------- | -------------------------------------------------------------------- |
| `uci`         | Reports `id name Vow Chess UCI` and `uciok`.                          |
| `isready`     | Replies `readyok`.                                                    |
| `ucinewgame`  | Full per-game reset (start position today; all per-game state).       |
| `position`    | `startpos` or `fen <FEN>`, optionally followed by `moves <uci> ...`.   |
| `go`          | `depth N`, `movetime MS`, `wtime`/`btime` (+`winc`/`binc`/`movestogo`), or `infinite`. Infinite analysis searches until `stop` or `MAX_DEPTH`. Emits `info depth/score/nodes/nps/pv`. |
| `perft N`     | Node-count divide to depth `N` (move-generation self-test).           |
| `captest N`   | Differential gate: asserts the quiescence capture generator equals the tactical subset of legal moves, to depth `N` (prints mismatch count). |
| `stop`        | Polled during search (checked every 1024 nodes) and honored.          |
| `setoption`   | Accepted and ignored (no configurable options yet).                  |
| `quit`        | Exits. Polled during search and honored even without a prior `stop`.  |

Moves are read and emitted in long algebraic form (`e2e4`, `e7e8q`).
Closing stdin also interrupts an active search and exits cleanly.

## Build And Run

Use the self-hosted compiler directly:

```sh
TMPDIR=/dev/shm build/vowc build --no-verify examples/chess/main.vow -o examples/chess/.local/chess
```

> **Compiler freshness matters.** The engine relies on the region inference from
> PR #879 to keep per-node search allocations bounded. A `build/vowc` built from a
> tree *predating* #879 will place those allocations in the root region and
> exhaust memory during deep search (OOM around depth 7). Build with a compiler
> from the current tree — re-run `scripts/bootstrap.sh` if `build/vowc` is stale.

The engine reads UCI commands on `stdin`:

```sh
printf 'uci\nposition startpos\ngo depth 4\nquit\n' | examples/chess/.local/chess
```

If your environment has a tight `/tmp` quota, keep `TMPDIR=/dev/shm` for builds.

Run the deterministic search regression after search changes. It preserves the
fixed-depth scores and best moves, the tactical baseline, and the `stop` path
while requiring an aggregate node-count reduction:

```sh
python3 examples/chess/search_regression.py \
    --engine examples/chess/.local/chess
```

Run the focused UCI regression suite with:

```sh
python3 -m unittest -v examples/chess/test_uci.py
```

The suite builds its own engine binary under `examples/chess/.local/` and bounds
the compiler and engine processes to 2 GB of virtual memory.

## Playing A Match

`play_uci_match.py` drives two UCI engines against each other, alternates
colors, and (optionally) uses a Stockfish-compatible validator to confirm
moves and adjudicate unfinished games.

```sh
# Quick single game against Stockfish
python examples/chess/play_uci_match.py \
    --white examples/chess/.local/chess --black stockfish

# 10 games, alternating colors, Stockfish capped near 1350 Elo
python examples/chess/play_uci_match.py \
    --white examples/chess/.local/chess \
    --black stockfish \
    --black-option "UCI_LimitStrength=true" \
    --black-option "UCI_Elo=1350" \
    --games 10 --alternate-colors \
    --plies 200 \
    --validator stockfish \
    --log match.log
```

The `--validator` engine must support the Stockfish-specific `d` (display)
command; it is used to fetch FENs and evaluate positions and is optional.

## Strength

At **1 second per move**, the engine performs at roughly **~2110 Elo** measured
against Stockfish's `UCI_LimitStrength` ladder (alternating colours). The headline
comes from the rung nearest the 50% crossover — the most reliable data point (see
`DEVELOPMENT.md`, "anchor near the 50% crossover"): **66% over 50 games vs
`UCI_Elo=2000`** (1σ band ~2065–2170). The flanking rungs are consistent — 77% vs
1900 (~2115) and 58% vs 2100 (~2158) — but sit further from 50% and so carry less
weight. Strength scales strongly with time control; at very short controls the
engine reaches only shallow depths and plays materially weaker.

`UCI_Elo` is a rough, self-referential yardstick (it weakens Stockfish by
injecting blunders rather than by playing like a rated human), so treat this as
an operational figure on that ladder, not a calibrated CCRL/FIDE rating. Numbers
also carry wide error bars at a few dozen games per data point.

## Notes

- **Search memory is bounded.** Per-node move-list allocations are freed as the
  search unwinds, so memory stays flat across a long game or a multi-game match
  in a single reused process (`#871`). This relies on the region inference
  placing per-node `Vec<ChessMove>` allocations in freeable frame arenas rather
  than the root arena.
