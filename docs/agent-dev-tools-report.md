# Agent Developer Tools: What's Missing?

*A report on toolchain features that become essential when the developer is an AI agent, not a human.*

## Premise

Vow's design assumes agents write code and humans run executables. The current toolchain already has excellent agent-facing affordances: structured JSON output, machine-readable `--help`, counterexamples with concrete inputs, and a blame model that distinguishes caller vs. callee faults. But the toolchain was still designed through a human lens — a human who happens to prefer JSON. This report identifies capabilities that only make sense (or become critically important) when the developer fixing bugs, writing features, and interpreting verification failures is an LLM agent.

---

## 1. Semantic Diff on Counterexamples (`vow verify --explain`)

### The Problem

When verification fails, the agent gets a counterexample:

```json
{
  "function": "safe_sub",
  "inputs": { "a": "-9223372036854775808", "b": "0" },
  "violation": "ensures result >= 0",
  "vow_id": 1
}
```

This tells the agent *what* failed but not *why*. The agent must mentally simulate `safe_sub(-9223372036854775808, 0)` to understand that `0 - (-9223372036854775808)` wraps around to a negative number due to two's complement overflow. For an agent, this "mental simulation" costs tokens and is error-prone — especially with multi-step arithmetic, nested function calls, or loop unrolling.

### The Proposal

Add a `--explain` flag (or make it the default) to `vow verify` that, for each counterexample, emits a **symbolic execution trace** showing the concrete values of every intermediate expression along the failing path:

```json
{
  "function": "safe_sub",
  "inputs": { "a": "-9223372036854775808", "b": "0" },
  "violation": "ensures result >= 0",
  "trace": [
    { "expr": "a - b", "value": "-9223372036854775808", "span": { "offset": 120, "length": 5 } },
    { "expr": "result", "value": "-9223372036854775808" },
    { "expr": "result >= 0", "value": "false" }
  ]
}
```

### Why This Matters for Agents Specifically

A human developer would glance at `i64_MIN` and immediately think "overflow." An agent has no such intuition — it has pattern matching over tokens. An explicit trace that shows the intermediate values collapses a multi-step reasoning problem into a lookup problem. This is the single highest-leverage improvement because it directly reduces the number of CEGIS iterations (the benchmark runner currently allows up to 5 iterations, and many failures are agents failing to *interpret* the counterexample correctly, not failing to know how to fix it).

### Implementation Path

ESBMC already computes a full execution trace internally (it's a bounded model checker — the counterexample IS a trace). The work is in `vow-verify`: extract intermediate assignments from the ESBMC counterexample XML, map them back to source expressions via the `Origin` metadata already threaded through the IR, and emit them in the JSON output.

---

## 2. Fix Suggestions as Structured Data (`diagnostics[].suggestions`)

### The Problem

Current diagnostics say *what's wrong*:

```json
{
  "error_code": "EffectViolation",
  "message": "function 'greet' calls 'print_str' which requires [io] but 'greet' declares no effects"
}
```

The agent must then figure out that the fix is to add `[io]` to `greet`'s signature. For `EffectViolation` this is straightforward. But for errors like `TypeMismatch` in a complex expression, or `LinearTypeViolation` where the fix could be "consume the value" or "make the type non-linear" or "pass it to a different function," the fix isn't obvious from the diagnostic alone.

### The Proposal

For every diagnostic where the fix is unambiguous or nearly so, emit a `suggestions` array containing concrete text edits:

```json
{
  "error_code": "EffectViolation",
  "message": "function 'greet' calls 'print_str' which requires [io] but 'greet' declares no effects",
  "suggestions": [
    {
      "description": "Add [io] effect to function signature",
      "edits": [
        {
          "span": { "offset": 18, "length": 0 },
          "replacement": " [io]"
        }
      ]
    }
  ]
}
```

### Why This Matters for Agents Specifically

Humans read error messages and apply judgment. Agents can *apply text edits mechanically*. A suggestion with a concrete span and replacement text can be applied with zero reasoning — the agent doesn't need to figure out where in the function signature the effect annotation goes, it just applies the edit. This turns a multi-step "understand error → locate fix site → generate fix → verify fix is correct" pipeline into a single "apply patch" step.

### Which Diagnostics Get Suggestions

Not all errors have unambiguous fixes. Priority list:

| Error Code | Suggestion | Confidence |
|---|---|---|
| `EffectViolation` | Add missing effect to signature | High |
| `NonExhaustiveMatch` | Add missing arms | High |
| `UnterminatedString` | Insert closing `"` | High |
| `MissingDelimiter` | Insert closing delimiter | High |
| `TypeMismatch` (return type) | Change return type annotation | Medium |
| `UnknownMethod` | Suggest closest Levenshtein match | Medium |
| `LinearTypeViolation` | Suggest consumption site | Low |

---

## 3. Contract Strength Analysis (`vow verify --analyze-contracts`)

### The Problem

Agents frequently write contracts that are either too strong (verification fails on edge cases the agent didn't consider) or too weak (verification passes trivially but the contract doesn't actually say anything useful). The current toolchain gives binary feedback: verified or not. It doesn't tell the agent "your `ensures` clause is a tautology" or "your `requires` clause excludes 99.9% of the input domain."

### The Proposal

Add `--analyze-contracts` to `vow verify` that emits contract quality metrics:

```json
{
  "contract_analysis": [
    {
      "function": "safe_add",
      "contracts": [
        {
          "clause": "requires: a >= 0",
          "kind": "requires",
          "coverage": "50.0%",
          "note": "Excludes half of i64 domain"
        },
        {
          "clause": "ensures: result >= 0",
          "kind": "ensures",
          "strength": "weak",
          "note": "Tautologically true given requires clauses — does not constrain the implementation"
        }
      ]
    }
  ]
}
```

### Why This Matters for Agents Specifically

Human developers use contracts as documentation — they're useful even if weak. Agent developers use contracts as *specifications to verify against*. A tautological contract wastes verification time and gives false confidence. Agents don't have the mathematical intuition to notice that `ensures: result >= 0` is trivially implied by `requires: a >= 0, requires: b >= 0` for an addition function. Explicit feedback about contract strength helps agents write meaningful specifications.

### Implementation Path

This is achievable by running additional ESBMC queries: for each `ensures` clause, check if it's satisfiable given the `requires` clauses (tautology detection). For `requires` clauses, estimate the fraction of the domain they exclude by random sampling or interval arithmetic.

---

## 4. Interactive Hypothesis Testing (`vow check-claim`)

### The Problem

During debugging, agents frequently form hypotheses: "I think the loop invariant needs `i <= n` instead of `i < n`." Currently, testing this hypothesis requires modifying the source file, recompiling, and re-verifying — a full round trip that costs ~10-30 seconds. If the agent is wrong, it must undo the change and try again.

### The Proposal

Add a `vow check-claim` command that takes a function name and a standalone predicate, and checks whether it holds:

```bash
vow check-claim safe_add "a + b >= 0"          # does a + b >= 0 hold for all valid inputs?
vow check-claim safe_add "result == a + b"      # is this postcondition provable?
vow check-claim --assuming "a >= 0, b >= 0" safe_add "a + b >= a"  # with extra assumptions
```

Output:

```json
{
  "claim": "a + b >= 0",
  "function": "safe_add",
  "result": "refuted",
  "counterexample": { "a": "9223372036854775807", "b": "1" },
  "note": "Overflow: a + b wraps to -9223372036854775808"
}
```

### Why This Matters for Agents Specifically

Human developers can sketch proofs on paper or reason informally. Agents cannot. But agents *can* generate many candidate hypotheses quickly and test them if the round-trip cost is low. `check-claim` turns the verifier into an interactive oracle that the agent can query rapidly without modifying source code. This is especially powerful in a CEGIS loop — instead of guessing a fix, modifying the file, and re-verifying, the agent can first test whether its proposed invariant even holds.

### Implementation Path

Reuse the C emission pipeline but inject the claim as an `__ESBMC_assert()` into the generated C model at the appropriate point (function exit for postconditions, loop head for invariants). No need to run codegen — just the verification frontend.

---

## 5. Mutation-Based Contract Adequacy Testing (`vow mutate`)

### The Problem

An agent writes code, writes contracts, and verification passes. But did the contracts actually catch anything? A function `fn f(x: i64) -> i64 { x }` with contract `ensures: result == result` will verify, but the contract is useless — any implementation would satisfy it. The agent has no signal that its contracts are meaningful.

### The Proposal

Add `vow mutate` that automatically generates mutations of the function body and checks whether the contracts catch them:

```bash
vow mutate safe_add
```

Output:

```json
{
  "function": "safe_add",
  "mutations_tested": 8,
  "mutations_caught": 6,
  "mutations_survived": 2,
  "adequacy_score": 0.75,
  "surviving_mutations": [
    {
      "description": "Replaced 'a + b' with 'b + a'",
      "note": "Contract does not distinguish commutativity — likely fine"
    },
    {
      "description": "Replaced 'a + b' with 'a + b + 0'",
      "note": "Trivially equivalent mutation"
    }
  ]
}
```

### Why This Matters for Agents Specifically

Human developers review contracts by reading them and thinking "does this cover my intent?" Agents cannot do this. Mutation testing provides *mechanized feedback* about whether contracts are meaningful. An agent that sees an adequacy score of 0.25 knows it needs to add more contracts before its code is trustworthy. This closes the loop on contract quality in a way that agents can act on.

### Implementation Path

Generate standard mutation operators (negate conditions, swap operators, off-by-one, replace constants, remove statements) on the IR level, re-emit C for each mutation, and run ESBMC. This is embarrassingly parallel and could be run alongside normal verification.

---

## 6. Dependency-Aware Incremental Verification (`vow verify --incremental`)

### The Problem

Currently, `vow verify` re-verifies the entire file from scratch on every invocation. In a CEGIS loop where the agent changes one function, verification of all unchanged functions is redundant. ESBMC invocations are expensive (seconds to tens of seconds), and multiplied across 5 CEGIS iterations, verification time dominates the development loop.

### The Proposal

Track which functions depend on which contracts (via the call graph and contract inheritance). When a function body changes but its contract doesn't, only re-verify the changed function. When a contract changes, re-verify the function and all callers.

```json
{
  "status": "VerifyFailed",
  "incremental": {
    "functions_checked": ["safe_add"],
    "functions_skipped": ["main", "helper"],
    "reason": "Only safe_add body changed since last verification"
  }
}
```

### Why This Matters for Agents Specifically

Human developers can wait 30 seconds between edits — they're thinking. Agents iterate at machine speed. The bottleneck is verification latency, not coding latency. Cutting verification time from 30 seconds to 3 seconds for single-function changes means the agent can explore 10x more candidate fixes in the same wall-clock budget. The existing `--no-cache` flag suggests there's already some caching infrastructure; this extends it to function-level granularity.

---

## 7. Dead Code and Unreachable Path Detection (`vow verify --reachability`)

### The Problem

Agents sometimes generate code with unreachable branches, dead assignments, or tautological conditions. These don't cause verification failures (the contracts may still hold), but they indicate that the agent's "mental model" of the program is wrong. The classic example: an agent adds a `requires: x >= 0` and then also adds `if x < 0 { ... }` — the branch is dead because the precondition rules it out. The agent doesn't realize it wrote contradictory logic.

### The Proposal

As a side-channel during verification, detect and report unreachable paths:

```json
{
  "reachability_warnings": [
    {
      "kind": "dead_branch",
      "span": { "offset": 200, "length": 30 },
      "message": "Branch 'x < 0' is unreachable given requires clause 'x >= 0'",
      "suggestion": "Remove the dead branch or weaken the requires clause"
    }
  ]
}
```

### Why This Matters for Agents Specifically

Dead code is a *signal of confused reasoning*. For a human, dead code is a minor style issue. For an agent, it's evidence of an internal contradiction in the agent's understanding — a red flag that other parts of the code may also be wrong. Surfacing this information gives the agent an opportunity to reconsider its approach before the code ships.

---

## 8. Contextual `--help` Per Error Code (`vow explain E001`)

### The Problem

When an agent encounters `EffectViolation`, it currently must have the full Vow specification in its context window to know what effects are and how to fix the violation. The current `--help` output describes the language globally but doesn't provide targeted guidance per error.

### The Proposal

```bash
vow explain EffectViolation
```

Output (JSON, for agents):

```json
{
  "error_code": "EffectViolation",
  "summary": "A function calls an effectful function without declaring the required effect",
  "detail": "In Vow, effects are part of the function type. If function A calls function B which has effect [io], then A must also declare [io] in its signature.",
  "fix_pattern": "Add the missing effect to the calling function's signature: fn name(...) -> Type [effect]",
  "example": {
    "bad": "fn greet() -> () {\n    print_str(\"hi\");\n}",
    "good": "fn greet() -> () [io] {\n    print_str(\"hi\");\n}"
  },
  "see_also": ["grammar.md#effects", "errors.md#EffectViolation"]
}
```

### Why This Matters for Agents Specifically

Agents have finite context windows. Shoving the entire grammar.md + contracts.md + errors.md into the system prompt costs ~35KB of tokens (as the benchmark runner currently does). A targeted `explain` command lets the agent load *only the information relevant to the error it's currently facing*. This is essentially RAG built into the compiler — the compiler knows exactly which documentation is relevant to the current error.

---

## 9. Structured Diff Between Verification Attempts (`vow diff-verify`)

### The Problem

In a CEGIS loop, the agent modifies code, re-verifies, and gets a new set of results. But it's hard for the agent to understand whether it's making *progress*. Did the change fix one counterexample but introduce another? Did the same counterexample change inputs? Did new functions start failing?

### The Proposal

```bash
vow diff-verify previous_result.json current_result.json
```

Output:

```json
{
  "resolved": [
    { "function": "safe_add", "was": "VerifyFailed", "now": "Verified" }
  ],
  "new_failures": [
    { "function": "helper", "violation": "ensures result > 0", "inputs": { "x": "0" } }
  ],
  "persistent": [
    {
      "function": "safe_sub",
      "same_violation": true,
      "input_changed": true,
      "previous_inputs": { "a": "-9223372036854775808", "b": "0" },
      "current_inputs": { "a": "0", "b": "1" }
    }
  ],
  "progress_summary": "1 fixed, 1 new, 1 persistent (inputs changed)"
}
```

### Why This Matters for Agents Specifically

Humans can eyeball two error outputs and notice differences. Agents are bad at comparing two large JSON blobs token-by-token. A structured diff gives the agent explicit signal about whether its last change was an improvement, a regression, or neutral. This is critical for effective CEGIS: the agent needs to know whether to keep going in the same direction or try something different.

---

## 10. Automatic Minimal Reproducer (`vow minimize`)

### The Problem

When a verification failure occurs in a large file with many functions, the counterexample points to one function, but the root cause may be in a helper function, a contract on a callee, or an incorrect type definition. The agent must read and understand the entire file to isolate the bug.

### The Proposal

```bash
vow minimize file.vow --function safe_sub
```

This produces a minimal `.vow` file that still exhibits the same verification failure — removing all unrelated functions, types, and imports:

```json
{
  "minimal_file": "/tmp/minimal_safe_sub.vow",
  "removed": ["main", "helper", "struct Config"],
  "lines": 12,
  "original_lines": 150
}
```

### Why This Matters for Agents Specifically

Agents pay per token. A 150-line file in the context window is expensive to reason about. A 12-line minimal reproducer lets the agent focus on exactly the relevant code. This is especially valuable when the agent is operating in a CEGIS loop with limited context budget — the less code it needs to consider, the better its fix attempts will be.

---

## Priority Ranking

Ordered by impact per implementation effort:

| Priority | Feature | Impact | Effort |
|---|---|---|---|
| 1 | Fix Suggestions (#2) | Eliminates reasoning for ~40% of compile errors | Medium |
| 2 | Counterexample Traces (#1) | Directly reduces CEGIS iterations | Medium-High |
| 3 | `check-claim` (#4) | Enables interactive hypothesis testing | Medium |
| 4 | Incremental Verification (#6) | 5-10x faster CEGIS loops | Medium |
| 5 | Verification Diff (#9) | Better CEGIS loop steering | Low |
| 6 | `explain` per error (#8) | Reduces context window pressure | Low |
| 7 | Contract Strength Analysis (#3) | Catches useless contracts early | Medium |
| 8 | Mutation Testing (#5) | Mechanized contract adequacy | High |
| 9 | Dead Code Detection (#7) | Catches confused reasoning | Medium |
| 10 | Minimal Reproducer (#10) | Reduces reasoning cost | Medium |

---

## Meta-Observation: The Compiler as Copilot

The unifying theme is that when the developer is an agent, **the compiler's job expands from gatekeeper to collaborator**. A human-facing compiler says "no" (error) or "yes" (success). An agent-facing compiler should say "no, and here's exactly what to change" (suggestions), "yes, but your contracts are weak" (strength analysis), "you're making progress" (verification diff), and "try asking me this" (check-claim).

Every feature above reduces the number of tokens an agent must spend reasoning about the problem. Tokens spent reasoning are tokens not spent generating correct code. The compiler has perfect knowledge of the program — it should share that knowledge in the most actionable format possible rather than requiring the agent to re-derive it.

This is a fundamental shift: the compiler stops being a tool *used by* developers and starts being a *partner in* development. That's the toolchain Vow deserves.
