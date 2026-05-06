# Blog Post Outline: "Solving Project Euler with Verified Code — No Humans Required"

## Hook

> What if an AI could not only solve algorithmic problems, but *prove* its
> solutions correct?

We gave Claude Code 10 Project Euler problems, the Vow compiler, and zero
access to the compiler's source code. Here's what happened.

## The Setup (what makes this different)

### Not just "can the LLM code?"

Every LLM benchmark asks: does the code produce the right output? Vow asks
a harder question: **can you prove it never produces the wrong output?**

- Normal workflow: write code → run tests → ship
- Vow workflow: write code + contracts → ESBMC proves correctness → ship

The agent doesn't just solve Euler #1 — it writes a postcondition
(`ensures: result >= 0`) and a loop invariant, and the bounded model
checker *proves* these hold for all inputs in the contract's domain.

### The constraint: skill docs only

The agent has:
- The `vowc` binary (compiler + verifier)
- ~35KB of Vow skill documentation (grammar, contracts, CLI, examples)
- A problem spec + skeleton with contracts pre-written

It does NOT have:
- Access to the Vow compiler source code
- Access to any `.vow` example files
- Internet search
- Human guidance

This is the "installed tool" experience — exactly what a real developer
would have.

## The CEGIS Loop (show, don't tell)

Walk through one problem end-to-end with full code. Good candidate: **E005
(Smallest Multiple)** because it has multiple functions calling each other,
each with contracts.

### Iteration 1: Agent's first attempt

Show the LLM's response — a GCD/LCM implementation with loop invariants.

### Verification

Show the `vow verify` JSON output. If it fails, show the counterexample
and how the agent interprets it. If it passes, show the `"status": "Verified"`.

### Execution

Compile and run. Output: `232792560`. Matches Project Euler.

**Key insight:** The agent didn't just get the right answer — the model
checker *proved* that `gcd` always returns a positive number, that `lcm`
preserves positivity, and that the accumulator stays ≥ 1.

## Results Table

Present the full 10-problem results as a table:

| Euler # | Problem | Verified? | Correct? | CEGIS Iters | Time |
|---------|---------|-----------|----------|-------------|------|

Highlight:
- How many verified on first try (iteration 1)
- How many needed CEGIS iterations (the agent fixed its own bugs from
  counterexamples)
- Any failures — what went wrong and why

## What the Contracts Caught

Pick 2-3 interesting cases where the CEGIS loop revealed a real bug:

1. **Off-by-one in a loop bound** — counterexample showed `i = limit` when
   the invariant required `i < limit`
2. **Integer overflow** — the model checker found inputs where wrapping
   arithmetic violated a postcondition
3. **Missing base case** — a function returned wrong value for edge input

These aren't test failures — they're *proofs of incorrectness* with
concrete counterexamples.

## The Agentic Coding Thesis

Why this matters for the future of AI-assisted development:

1. **Contracts as spec.** Instead of writing tests, you write contracts.
   The verifier checks all inputs, not just the ones you thought of.

2. **CEGIS as debugging.** When the agent gets it wrong, the
   counterexample is a *proof* of the bug, not a flaky test. The agent
   can fix with confidence.

3. **Verification as CI.** In a Vow workflow, the build either passes
   formal verification or it doesn't. No "tests passed but there's a
   lurking edge case."

4. **No human in the loop.** The entire cycle — code generation,
   verification, counterexample-driven repair — runs autonomously. A human
   writes the spec (contracts); the machine does the rest.

## Numbers to Feature

- **X/10** problems formally verified
- **Y/10** produce the correct Euler answer
- **Z** average CEGIS iterations (lower = agent understood the language well)
- **W** total tokens (cost efficiency)

## Call to Action

- Link to the Vow repo
- Link to the euler/ directory with all problems and the runner
- "Try it yourself: `uv run --project euler euler/run.py run --model <your-model>`"
- Invite readers to add more Euler problems and PR them

## Possible Titles

1. "Solving Project Euler with Verified Code — No Humans Required"
2. "10 Euler Problems, 10 Formal Proofs: What Happens When AI Writes Verified Code"
3. "Beyond Tests: An AI Agent That Proves Its Code Correct"
4. "The CEGIS Loop: How an AI Debugs Itself with Math"
