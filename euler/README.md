# Euler Problems Demo: Verified Solutions by an AI Agent

This demo showcases Vow's agentic coding workflow: an AI agent (Claude Code)
solves Project Euler problems using only the `vowc` binary and language skill
documentation — no access to compiler source code, no hand-holding.

Every solution is **formally verified** by ESBMC, not just tested. The agent
writes contracts (preconditions, postconditions, loop invariants) and the
bounded model checker proves them correct.

## How It Works

```
┌─────────────┐     ┌──────────┐     ┌──────────┐     ┌──────────┐
│  Euler spec  │────▶│  Claude   │────▶│  vowc    │────▶│  ESBMC   │
│  + skeleton  │     │  Code     │     │  verify  │     │  proof   │
└─────────────┘     └──────────┘     └──────────┘     └──────────┘
                         │                                   │
                         │◀──── counterexample ──────────────┘
                         │         (CEGIS loop)
```

1. The agent receives a problem spec and a Vow skeleton with contracts
2. It fills in the implementation using only the Vow skill docs
3. `vowc verify` runs ESBMC to check all contracts
4. If verification fails, the agent gets the counterexample and iterates

## Problem Set

| #  | Euler | Problem               | Difficulty | Key Vow Feature           |
|----|-------|-----------------------|------------|---------------------------|
| 1  | E001  | Multiples of 3 or 5   | Easy       | Loop + accumulator ensures |
| 2  | E002  | Even Fibonacci        | Easy       | Fibonacci invariant        |
| 3  | E006  | Sum square difference | Easy       | Pure arithmetic ensures    |
| 4  | E005  | Smallest multiple     | Medium     | GCD/LCM contracts         |
| 5  | E007  | 10001st prime         | Medium     | Nested loops, primality    |
| 6  | E009  | Pythagorean triplet   | Medium     | Algebraic ensures          |
| 7  | E010  | Summation of primes   | Medium     | Large loop, accumulator    |
| 8  | E014  | Longest Collatz       | Hard       | Complex loop invariant     |
| 9  | E015  | Lattice paths         | Hard       | Checked arithmetic         |
| 10 | E021  | Amicable numbers      | Hard       | Nested function contracts  |

## Running

```bash
# Run all problems against a model
uv run --project euler euler/run.py run --model claude-sonnet-4-20250514

# Run a single problem
uv run --project euler euler/run.py run --model claude-sonnet-4-20250514 --problem E001

# Generate results report
uv run --project euler euler/run.py report
```

## Blog Post Material

After a run, `euler/results/` contains:
- Per-problem JSON with the agent's code, CEGIS iterations, and verification output
- A markdown report suitable for embedding in a blog post
- Statistics: verification rate, average iterations, token usage
