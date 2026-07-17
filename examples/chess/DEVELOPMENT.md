# Developing the engine: measurement discipline & lessons

This engine went from **~1520 to ~2110 Elo** (on the Stockfish `UCI_Elo` ladder at
1 s/move). Most of the difficulty was not writing chess code — it was *knowing
whether a change actually helped*. This file records the measurement methodology
and the lessons, so the next round of work doesn't relearn them.

## Progression

| Milestone | Change | ~Elo @ 1 s/move |
| --- | --- | --- |
| Baseline | depth-3-capped, material-ish eval | ~1520 |
| Search rewrite | TT+Zobrist, killers/history, null-move/LMR/PVS, real time mgmt | ~1800 |
| Speed pass | alloc-free `square_attacked`, king-square tracking | (folded in) |
| PeSTO eval | tapered midgame/endgame piece-square tables | ~1950 |
| Draw detection | repetition + 50-move | **~2110** |
| Search depth | aspiration windows, table LMR, shallow quiet pruning | **~2115** |
| Lightweight endgames | insufficient material + KX mop-up | not Elo-measured |

## Endgame acceptance (#909)

The lightweight endgame change was measured separately from Elo because its
acceptance criterion is conversion of elementary mates. With seed `909`, ten
legal KQvK and ten legal KRvK positions, the Vow engine searching depth 4, and
Stockfish searching depth 8 as deterministic defender, the draw-only baseline
mated **11/20** positions before 100 plies. The mop-up evaluation mated
**20/20**. The same run recognised all six representative insufficient-
material FENs (including both colours and a bare king in check) and kept three
live-material controls non-drawn.

```sh
ulimit -v 2000000
python3 examples/chess/test_endgames.py \
    --engine examples/chess/.local/chess \
    --stockfish stockfish \
    --pieces QR --positions-per-piece 10 --seed 909 \
    --engine-depth 4 --defender-depth 8
```

This is a targeted conversion measurement, not an Elo sample. No Elo delta is
claimed: the 20-position endgame suite is intentionally biased toward KX
positions, while an Elo estimate requires the 20–50+ full games near the
Stockfish ladder crossover described below.

## Measurement discipline

**1. Two modes — pick the right one.**
- **Fixed-depth node counts** (`go depth N`, read `nodes` from the `info` line).
  With no transposition table hit across processes this is *deterministic* and
  therefore **contention-immune** — no Stockfish, no games. Use it for any change
  whose effect is on the search tree:
  - *Move-ordering* changes must leave the node count **identical** (only time
    drops) — that is the proof they are semantics-preserving.
  - *Raw-speed* changes (fewer allocations, cheaper per-node work) leave the node
    count the same and **raise nps**.
  - *Pruning* changes (null-move, LMR, better ordering) **reduce** the node count.
- **Fixed-time matches vs Stockfish** for actual playing strength, where speed,
  eval, and search all compound. Reserve these for milestones.

**2. Anchor near the 50 % crossover.** Elo is a logistic curve: at 15 % or 85 %
it is nearly flat, so a real +150 Elo barely moves the score and reads as noise.
Bracket `UCI_Elo` until you find where you score **30–70 %**, and measure there.

**3. Respect the error bars.** ~12 games gives a ±10 % (1σ) band — useless for
detecting anything smaller than a couple hundred Elo. Use 20–50+ games per point
and pool across runs. State the band, not just the point estimate.

**4. Never run matches in parallel.** `movetime` is wall-clock; CPU contention
makes every engine search fewer nodes in the same wall-clock, weakening and
adding noise to *both* matches. Serialize matches; do A/B during development with
the deterministic fixed-depth mode instead.

**5. Trust decisive results, audit adjudicated ones.** `play_uci_match.py` scores
an unfinished game by the validator's eval with a **±1.0** threshold — a +1.0
position at the ply cap counts as a *full point*. Before believing a headline
number, check the outcome breakdown (`--log`, then look at `Result:` lines) and
prefer a ply cap high enough that games actually finish. The current 50-game
headline run was 45/50 decisive (checkmate / no legal move), and all five
adjudicated games were engine losses, so the score was not inflated by soft
adjudication.

**6. Correctness gates come before strength.**
- `perft` against known counts proves move generation (en passant, castling
  legality, promotions, pins). It also validates make/unmake bookkeeping — e.g.
  king-square tracking is confirmed purely by perft staying exact.
- A *differential gate* proves a fast path equals a slow one (the `captest`
  command checks that the quiescence capture generator equals the tactical
  subset of the full legal move list).
- A tactical suite vs a Stockfish oracle catches pruning that silently drops
  tactics. A change that passes perft but drops tactics is broken.

**7. Validate the tool, not just the engine.** Two "regressions" this project
were actually broken measurements: a `select()`/buffered-`readline()` race in the
match harness, and — when driving Stockfish from a one-shot pipe — stdin EOF
aborting the `go` search so it returned a depth-1 move. Hold stdin open
(`{ printf ...; sleep N; } | stockfish`) and read with blocking reads.

### Reproducing a measurement

```sh
# One rung of the ladder: engine vs Stockfish limited to ~2000, 50 games, 1 s/move.
ulimit -v 2000000
python3 examples/chess/play_uci_match.py \
    --white examples/chess/.local/chess --black stockfish \
    --black-option "UCI_LimitStrength=true" --black-option "UCI_Elo=2000" \
    --white-go "go movetime 1000" --black-go "go movetime 1000" \
    --games 50 --alternate-colors --plies 200 \
    --validator stockfish --log match.log
# implied Elo = 2000 - 400*log10(1/score - 1)
```

### Search-depth tuning regression (#911)

The deterministic gate launches a fresh engine for each position, freezes the
depth-6 score and best move, requires the ten depth-7 tactical moves to remain
unchanged, exercises `stop`, and enforces a strict aggregate node ceiling:

```sh
python3 examples/chess/search_regression.py \
    --engine examples/chess/.local/chess
```

The tuned search keeps every fixed-depth score and best move unchanged while
reducing the representative depth-6 tree:

| Position | Baseline nodes | Tuned nodes | Reduction |
| --- | ---: | ---: | ---: |
| Start position | 16,541 | 10,483 | 36.6% |
| Kiwipete | 356,711 | 313,524 | 12.1% |
| Middlegame | 115,061 | 93,989 | 18.3% |
| Pawn endgame | 4,783 | 4,133 | 13.6% |
| **Aggregate** | **493,096** | **422,129** | **14.4%** |

The tactical result remained 8/10 against the Stockfish oracle, with all ten
pre-change engine moves identical. The advanced seventh-rank pawn safeguard and
the asynchronous `stop` path also pass. The move-generation gates remained
unchanged: start position `perft 4 = 197281`, Kiwipete `perft 3 = 97862`,
position 3 `perft 4 = 43238`, position 5 `perft 3 = 59412`, and all four
`captest 3` runs report zero mismatches.

For the fixed-time crossover, the parent and tuned binaries were run
sequentially with the command above, substituting `/tmp/chess-911-baseline` and
then `/tmp/chess-911-candidate` for `--white` and using corresponding separate
log paths. The processes were never concurrent.

| Engine | W/D/L | Score | Implied Elo | 1σ band | Decisive W/D/L | Adjudicated W/D/L |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Baseline | 25/3/22 | 53.0% | 2021 | 1972–2071 | 24/0/20 | 1/3/2 |
| Tuned | 33/0/17 | 66.0% | 2115 | 2065–2170 | 33/0/12 | 0/0/5 |

This is a +13 percentage-point score change and a **+94 Elo point-estimate
delta** at the crossover. The independent 1σ bands still overlap, so the match
supports the acceptance direction rather than a precise Elo-gain claim.

## Lessons learnt

- **Root-cause before fixing.** The dramatic "OOM at depth 7" was a *stale
  compiler* (pre-#879 region inference), not an engine bug — a fresh compiler cut
  peak memory ~700×. Confirm the toolchain is current before diagnosing.
- **The instrument can be blind.** Early on, three genuinely different versions
  all read "16.7 % over 12 games." That was the *measurement* failing to resolve
  the difference, not the changes failing. Fixing the instrument (crossover
  anchor + more games) is not a detour — it is the thing that turns the next five
  changes from guesses into knowledge.
- **Depth is this engine's dominant lever, and it is time-control-sensitive.**
  The same binary read ~1700 at 0.2 s/move and ~1800 at 1 s/move. Always report
  the time control with any Elo claim.
- **Time management is easy to get subtly wrong.** Discarding an unfinished
  deepening iteration silently wasted ~half the clock. Committing the best move
  found so far (once the principal-variation move has been searched) was a large,
  nearly free gain.
- **Design the correctness gate into the feature.** Computing the Zobrist key
  per-node (instead of incrementally through make/unmake) traded a little speed
  to eliminate a whole class of hard-to-debug hashing bugs; storing the full
  64-bit key makes index collisions self-correcting.
- **Generate error-prone data, don't type it.** The 1536 PeSTO table values were
  emitted by a script (with a board-orientation sanity check), never hand-typed
  into the source.
- **Know when to stop.** At the target, on a validated engine, each new feature
  is margin and fresh regression surface. Opening book / endgame knowledge were
  deliberately left as future work.
