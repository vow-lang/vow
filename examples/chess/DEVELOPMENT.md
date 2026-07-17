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
prefer a ply cap high enough that games actually finish. Our final figure was
23/24 decisive (checkmate / no legal move), so it was not inflated by soft
adjudication.

**6. Correctness gates come before strength.**
- `perft` against known counts proves move generation (en passant, castling
  legality, promotions, pins). It also validates make/unmake bookkeeping — e.g.
  king-square tracking is confirmed purely by perft staying exact.
- A *differential gate* proves a fast path equals a slow one (the `captest`
  command checks that the quiescence capture generator equals the tactical
  subset of the full legal move list).
- The deterministic `halfmovetest` search gate covers draw-before-quiescence,
  exact halfmove-qualified TT probes and stores (including deeper entries), and
  null-move clock advancement/restoration at the 50-move boundary. Its expected
  output is `halfmovetest mismatches: 0`.
- A tactical suite vs a Stockfish oracle catches pruning that silently drops
  tactics. A change that passes perft but drops tactics is broken.

**7. Validate the tool, not just the engine.** Two "regressions" this project
were actually broken measurements: a `select()`/buffered-`readline()` race in the
match harness, and — when driving Stockfish from a one-shot pipe — stdin EOF
aborting the `go` search so it returned a depth-1 move. Hold stdin open
(`{ printf ...; sleep N; } | stockfish`) and read with blocking reads.

### Reproducing a measurement

```sh
# One rung of the ladder: engine vs Stockfish limited to ~2000, 30 games, 1 s/move.
python examples/chess/play_uci_match.py \
    --white examples/chess/.local/chess --black stockfish \
    --black-option "UCI_LimitStrength=true" --black-option "UCI_Elo=2000" \
    --white-go "go movetime 1000" --black-go "go movetime 1000" \
    --games 30 --alternate-colors --plies 200 \
    --validator stockfish --log match.log
# implied Elo = 2000 - 400*log10(1/score - 1)
```

## Evaluation polish measurement (#912)

The mobility and king-safety coefficients were kept deliberately small:
`mobility=3`, `pawn shield=6`, and `king-zone attack=8`. Two local tuning checks
informed that choice:

- A 100-position least-squares fit against Stockfish depth-8 evaluations was
  rejected. Random legal positions made the features highly correlated and the
  fitted king-attack coefficient changed sign; RMSE remained 324–343 cp versus
  370 cp for the conservative weights. That is not a stable enough signal to
  override chess priors.
- A 12-game, 100 ms/move ablation of king-zone attacks (`8` versus `0`) finished
  6–6. With no measured benefit from a more aggressive coefficient, `8` was
  retained.

The final strength gate compared the polished evaluator directly with the
TT-mate-folding-only commit (`b2986a3`), so the mate-score correctness fix could
not be mistaken for evaluation strength. Across 30 serialized games at 100
ms/move, alternating colors with a 200-ply cap, the polished engine scored
**17.5–12.5** (`+17 =1 -12`), or **58.3% / approximately +58 Elo**. The 1-sigma
score uncertainty is about +/-9 percentage points (approximately -5 to +126 Elo
for the delta), so this is evidence of a non-negative point estimate rather than
a claim of statistical significance. Twenty-six games terminated normally; the
Stockfish validator adjudicated four at the cap (three at +100, one at 0.00).

```sh
python examples/chess/play_uci_match.py \
    --white "bash -c 'ulimit -v 2000000; exec examples/chess/.local/chess'" \
    --black "bash -c 'ulimit -v 2000000; exec examples/chess/.local/chess-tt-only'" \
    --white-go "go movetime 100" --black-go "go movetime 100" \
    --games 30 --alternate-colors --plies 200 \
    --validator stockfish --log eval-final-100ms-30.log
```

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
