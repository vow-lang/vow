# Design: Performance Guarantees via Empirical Complexity Verification

**Status:** Proposal (research + design)
**Author:** Claude
**Date:** 2026-04-06

## Motivation

Vow has correctness guarantees (`requires`, `ensures`, `invariant`) verified by ESBMC. But correctness without performance is incomplete — an agent can write a correct O(n^3) sort when O(n log n) was intended. "Accidentally quadratic" is one of the most common classes of bugs in agent-generated code, and no existing Vow mechanism catches it.

The idea: let functions carry **performance contracts** — Big-O complexity bounds — that are **verified empirically via fuzzing**, not symbolically via ESBMC. This keeps the two verification pipelines cleanly separated:

- **Correctness contracts** (`requires`/`ensures`/`invariant`) → ESBMC (sound, symbolic)
- **Performance contracts** (`complexity`) → Fuzz harness (empirical, statistical)

## Prior Art

### Static (Sound) Approaches

| System | Technique | Key Limitation for Vow |
|--------|-----------|----------------------|
| **RAML** (Hoffmann et al.) | Automatic amortized resource analysis via LP-solving in the type system | Requires new type-system axis — violates Vow principle #1 |
| **TiML** (Wang et al., OOPSLA 2017) | Type-and-effect system where effects track time complexity, SMT-verified | New type-system axis + SMT dependency beyond ESBMC |
| **Liquid Haskell** ("Liquidate Your Assets", POPL 2020) | Refinement types encode potential functions for amortized analysis | Heavy annotation burden, requires SMT solver (Z3) |
| **Nomos** (Das, Pfenning, CMU) | Resource-aware session types for concurrent programs | Polynomial bounds only, specific to message-passing |
| **Algebraic Complexity Analysis** (Kincaid et al., POPL 2024) | Compositional recurrence extraction + solving | Academic prototype, not production-ready |

**Verdict:** All static approaches require either a new type-system axis or a recurrence solver. Both violate Vow's design principle #1 (must not make verification harder). **Rejected for now**, though RAML-style inference could be revisited if Vow's type system evolves.

### Dynamic/Empirical Approaches

| System | Technique | Key Insight |
|--------|-----------|-------------|
| **Goldsmith, Aiken (ESEC/FSE 2007, "TREND")** | Instrument programs, measure op counts at increasing sizes, log-log regression | Operation counting is deterministic and machine-independent |
| **Zaparanuks & Hauswirth (PLDI 2012)** | "Algorithmic Profiling" — count abstract ops on JVM, fit candidate curves | Algorithmic profiling distinct from performance profiling |
| **Coppa et al. (PLDI 2012, "aprof")** | Input-sensitive profiler that auto-infers "input size" per function | Eliminates manual size specification |
| **`big_o` Python library** | Wall-clock timing + least-squares fit to fixed candidate classes | Simple, practical, widely used |
| **SlowFuzz** (Petsios et al., CCS 2017) | Evolutionary fuzzing maximizing instruction count | Finds worst-case inputs, not just average |
| **PerfFuzz** (Lemieux et al., ISSTA 2018) | Coverage-guided fuzzing maximizing per-location execution counts | More targeted than SlowFuzz |
| **Badger** (Noller et al., ISSTA 2018) | Hybrid fuzzing + symbolic execution for complexity | Explores different complexity behaviors via symbolic branches |

**Verdict:** Empirical complexity testing is the right fit for Vow. It doesn't touch the type system or verifier. It catches a real class of agent bugs. It makes agentic coding easier (declare complexity, get feedback). The combination of operation counting (Goldsmith) with adversarial input search (SlowFuzz/PerfFuzz) gives the strongest guarantees.

### Property Testing Connection

| Component | Complexity Testing | Property Testing |
|-----------|-------------------|-----------------|
| Input generation | Random values of type T at size n | Random values of type T |
| Running function | Time/count operations | Check postcondition |
| Shrinking | Find minimal size where complexity breaks | Find minimal input where property fails |
| Reporting | "O(n^2) observed, O(n) declared" | "ensures violated for input x=42" |

The infrastructure is nearly identical. Building a fuzzing/generation framework for complexity testing unlocks property testing (`hypothesis`/QuickCheck-style) as a future feature with minimal additional work.

## Syntax Design

### Recommended: `complexity` clause inside `vow` block

```vow
fn sort(v: Vec<i64>) -> Vec<i64> vow {
    requires: v.len() >= 0,
    ensures: result.len() == v.len(),
    complexity: O(n * log(n)) where n = v.len()
} {
    // ...
}
```

The `complexity` clause is a *vow* — a promise the function makes about its performance. It is verified differently from correctness clauses (fuzzing, not ESBMC), but it lives in the same contract block because it *is* a contract. Blame on violation: **Callee** (the implementation is too slow).

### Why not a separate block?

Three alternatives were considered:

**Option B: Separate `perf` block**
```vow
fn sort(v: Vec<i64>) -> Vec<i64> vow {
    ensures: result.len() == v.len()
} perf {
    complexity: O(n * log(n)) where n = v.len()
} {
    // ...
}
```
Rejected: two blocks between signature and body adds grammar complexity for marginal benefit. The difference in verification method is an implementation detail, not a user-facing distinction.

**Option C: Annotation syntax**
```vow
#[complexity(O(n_log_n), n = v.len())]
fn sort(v: Vec<i64>) -> Vec<i64> { ... }
```
Rejected: Vow has no attribute syntax. Introducing `#[...]` is a new grammar axis. Encoding complexity classes as strings is unparseable and ugly.

**Option D: Per-module or per-file annotations**
Rejected: Complexity is inherently per-function. Module-level annotations lose precision.

### Complexity Class Expressions

A **fixed set** of complexity classes, not arbitrary expressions:

```
complexity: O(1)                                  // constant
complexity: O(log(n)) where n = v.len()           // logarithmic
complexity: O(n) where n = v.len()                // linear
complexity: O(n * log(n)) where n = v.len()       // linearithmic
complexity: O(n * n) where n = v.len()            // quadratic
complexity: O(n * n * n) where n = v.len()        // cubic
complexity: O(n * m) where n = a.len(), m = b.len()  // multi-variable
```

Using `n * n` instead of `n^2` avoids introducing a power operator. The fixed set keeps parsing trivial (no symbolic math engine needed). This matches `big_o`'s approach.

Supported forms:

| Form | Doubling Ratio | Notes |
|------|---------------|-------|
| `O(1)` | 1.0 | Constant |
| `O(log(n))` | ~1.0 (slow growth) | Logarithmic |
| `O(n)` | 2.0 | Linear |
| `O(n * log(n))` | ~2.0 (slightly above) | Linearithmic |
| `O(n * n)` | 4.0 | Quadratic |
| `O(n * n * n)` | 8.0 | Cubic |

### The `where` clause

The `where` clause in `complexity` binds **size parameters** to expressions over function arguments. This tells the fuzzer:
1. **What to vary** (which argument controls "size")
2. **How to measure size** (the expression, e.g., `.len()`)

```vow
// Single size parameter
complexity: O(n) where n = data.len()

// Multiple size parameters
complexity: O(n * m) where n = rows.len(), m = cols.len()

// Size is the argument itself (for integer inputs)
complexity: O(n) where n = x

// Size is output-dependent (for impure functions)
complexity: O(n * log(n)) where n = result.len()
```

### Optional modifier: `amortized`

```vow
fn push_all(v: Vec<i64>, items: Vec<i64>) -> Vec<i64> vow {
    complexity: O(n) amortized where n = items.len()
} { ... }
```

The `amortized` keyword tells the fuzzer to measure *total cost over sequences of operations* rather than single-call worst case. Critical for data structures with amortized bounds (e.g., dynamic arrays, splay trees).

## Verification Mechanism

### Step 1: Input Generation

The `where` clause determines the generation strategy:

| Argument Type | Size Metric | Generator |
|---------------|------------|-----------|
| `i64` | The value itself | Use n directly |
| `Vec<T>` | `.len()` | Random Vec of length n, random elements |
| `String` | `.len()` | Random String of length n |
| `HashMap<K,V>` | `.len()` | Random HashMap with n entries |

Arguments **not** bound as size parameters get random values constrained by `requires` clauses. The generator respects preconditions — only generates inputs satisfying `requires`.

Size progression: geometric (e.g., n = 16, 32, 64, 128, 256, 512, 1024, 2048). Geometric spacing gives even distribution on log-log plots and good statistical power.

### Step 2: Measurement

Two measurement modes:

**Operation counting (preferred):** Instrument the IR to count basic block executions. Deterministic, machine-independent, no timing noise. Vow already compiles to IR — inserting counters is a natural fit.

**Wall-clock timing (fallback):** For effectful functions where instrumentation is impractical. Multiple trials per size, take median, warm up caches. Less reliable but always available.

### Step 3: Statistical Analysis

Two complementary tests:

**Doubling ratio test** (primary, from Goldsmith/Aiken):
- For each consecutive pair of sizes (n, 2n), compute T(2n)/T(n)
- Compare observed ratio to expected ratio for declared complexity class
- For O(n): expect ratio ~ 2.0. For O(n^2): expect ratio ~ 4.0
- Passes if all ratios are within tolerance (e.g., +/- 20%)

**Least-squares curve fitting** (secondary):
- Fit data to each candidate complexity class via least-squares
- Compute R^2 for each candidate
- The declared class must have R^2 > 0.90
- Additionally verify no *simpler* class fits equally well (prevents declaring O(n^2) for an O(n) function)

**Tightness check:**
- If declared O(n^2) but data fits O(n), emit a *warning* (not error): "complexity bound may not be tight — observed O(n), declared O(n^2)." This is technically correct (O(n) is O(n^2)) but unhelpful. Agents should declare tight bounds.

### Step 4: Adversarial Input Search

Random inputs test average-case behavior. To test worst-case, use evolutionary fuzzing (SlowFuzz/PerfFuzz approach):

1. Generate random inputs at each size → establish baseline curve
2. Mutate inputs to maximize operation count → find adversarial inputs
3. Re-measure with adversarial inputs → establish worst-case curve
4. **Both** curves must fit the declared complexity

This catches pathological cases like quicksort (O(n log n) average, O(n^2) worst-case on sorted inputs).

### Step 5: Verdict

```
PASS: complexity O(n * log(n)) confirmed
      Random inputs:  R^2 = 0.98, ratio = 2.12 (expected ~2.0)
      Worst-case:     R^2 = 0.96, ratio = 2.18 (expected ~2.0)

FAIL: complexity O(n) declared but observed O(n * n)
      Random inputs:  ratio = 3.87 (expected 2.0, observed ~4.0)
      Worst-case:     ratio = 4.02 (expected 2.0, observed ~4.0)
      Suggestion: actual complexity appears to be O(n * n)

WARN: complexity O(n * n) declared but may not be tight
      Random inputs:  O(n) fits with R^2 = 0.99
      Consider: complexity: O(n) where n = v.len()
```

## Handling Hard Cases

### Effectful Functions

```vow
fn read_and_process(path: String) -> Vec<i64> [io] vow {
    complexity: O(n * log(n)) where n = result.len()
} { ... }
```

- **`result` in `where`:** For IO functions, input size may depend on external data. Allowing `result.len()` as the size metric works when output size correlates with computational cost.
- **Effect mocking:** Vow's effect system tells us *which* effects to mock. For `[read]` functions, the fuzzer provides mock IO returning controlled-size data. For `[io]`, mock `print_str` as no-op.
- **Limitation:** If complexity depends on external state that can't be controlled, the clause can't be verified. The compiler warns rather than errors.
- **Pragmatic:** Most functions where complexity matters are pure. Effectful entry points rarely need complexity annotations.

### Multiple Size Parameters

```vow
fn matrix_mul(a: Vec<Vec<i64>>, b: Vec<Vec<i64>>) -> Vec<Vec<i64>> vow {
    complexity: O(n * n * m) where n = a.len(), m = b.len()
} { ... }
```

The fuzzer varies n and m independently across a grid of sizes. Multivariate least-squares regression fits the data to the declared form. Well-studied statistically.

### Non-Collection Arguments

```vow
fn process(config: Config, data: Vec<i64>) -> i64 vow {
    complexity: O(n) where n = data.len()
} { ... }
```

Arguments not bound as size parameters need random generation. This requires **type-aware generators** — auto-derivable for each Vow type since Vow is monomorphic:

- `i64`: uniform random in a reasonable range
- `bool`: uniform random
- `String`: random string of random length
- `Vec<T>`: random length, recursive generation for elements
- Structs: generate each field recursively
- Enums: pick a random variant, generate fields

The `requires` clauses constrain the generation space. Generators that produce values violating preconditions are filtered (rejection sampling) or guided (constraint-based generation, future work).

### Distinguishing O(n) from O(n log n)

This is genuinely hard empirically — the log factor grows slowly. Strategies:

1. **Use large size ranges:** n from 64 to 65536 (10 doublings). At this range, n log n differs from n by a factor of ~16.
2. **Use operation counts, not wall-clock time:** Eliminates noise.
3. **Accept ambiguity:** If the data fits both O(n) and O(n log n) equally well, accept either declaration. The practical difference is rarely consequential.
4. **Future:** If RAML-style static analysis is added later, it can disambiguate.

## Pipeline Integration

```
Source -> vow-syntax -> vow-types -> vow-ir -> vow-codegen -> executable
                                          |
                                          +-> vow-verify  -> proof / counterexample
                                          |
                                          +-> vow-perf    -> complexity report (NEW)
```

The new `vow-perf` crate runs in parallel with `vow-verify` and `vow-codegen`:

1. Extracts `complexity` clauses and `where` bindings from IR
2. Generates an instrumented test harness (operation-counting binary)
3. Runs the harness at increasing input sizes with random + adversarial inputs
4. Performs doubling ratio test + curve fitting
5. Reports pass/fail/warn with data

### CLI Integration

```bash
vowc build examples/sort.vow                   # compile + verify correctness (existing, unchanged)
vowc build --perf examples/sort.vow            # compile + verify correctness + test performance
vowc perf examples/sort.vow                    # test performance only (new subcommand)
vowc perf --sizes 16,64,256,1024 sort.vow      # custom size progression
vowc perf --iterations 50 sort.vow             # more iterations per size point
vowc perf --no-adversarial sort.vow            # skip worst-case input search (faster)
```

Performance testing is **opt-in** (`--perf` flag or `perf` subcommand). It does not slow down the default `vowc build` path. Functions without `complexity` clauses are silently skipped by `vowc perf`.

### JSON Output

```json
{
  "function": "sort",
  "declared": "O(n * log(n))",
  "size_param": "n = v.len()",
  "result": "pass",
  "data": {
    "sizes": [16, 32, 64, 128, 256, 512, 1024, 2048],
    "op_counts": [64, 160, 384, 896, 2048, 4608, 10240, 22528],
    "doubling_ratios": [2.5, 2.4, 2.33, 2.29, 2.25, 2.22, 2.2],
    "r_squared": 0.98
  },
  "adversarial": {
    "worst_case_ratio": 2.31,
    "r_squared": 0.96
  }
}
```

## Implementation Roadmap

### Phase 1: Pure functions, single Vec parameter (MVP)
- Parse `complexity` clause in vow block
- Support `O(1)`, `O(n)`, `O(n * n)`, `O(n * n * n)` with `where n = v.len()`
- Operation-counting instrumentation in IR
- Doubling ratio test
- `vowc perf` subcommand
- Random inputs only (no adversarial search)

### Phase 2: Extended complexity classes + multi-parameter
- Add `O(log(n))`, `O(n * log(n))`
- Support multiple size parameters (`where n = ..., m = ...`)
- Least-squares curve fitting + R^2 reporting
- Tightness warnings

### Phase 3: Adversarial input search
- Evolutionary mutation to maximize operation counts
- Both random and worst-case verification
- Shrinking to find minimal adversarial inputs

### Phase 4: Generators + property testing
- Auto-derived type-aware generators
- Precondition-respecting generation
- Reuse generators for `hypothesis`-style property testing (future feature)
- `amortized` modifier support

### Phase 5: Effectful functions
- Effect mocking for `[io]`, `[read]`, `[write]`
- `result` in `where` clauses
- Wall-clock fallback measurement

## Design Decisions Summary

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Static vs. dynamic | Dynamic (empirical) | No type-system changes, no ESBMC changes |
| Syntax location | Inside `vow {}` block | It's a contract; unified with existing syntax |
| Complexity classes | Fixed set | No symbolic math engine needed |
| Size binding | `where n = expr` | Explicit, unambiguous, familiar |
| Measurement | Operation counting (primary) | Deterministic, machine-independent |
| Statistical test | Doubling ratio + least-squares | Robust, well-studied, complementary |
| Default behavior | Opt-in (`--perf`) | No impact on existing workflow |
| Blame | Callee | Implementation is too slow |

## Vow Design Principles Check

1. **Does not make verification harder:** Separate pipeline, no ESBMC changes, no type-system changes, no C model changes.
2. **Eliminates a class of agent bugs:** "Accidentally quadratic" and worse — one of the most common classes of agent-generated performance bugs.
3. **Makes agentic coding easier:** Agent declares intended complexity, gets immediate empirical feedback. No external benchmarking framework to configure.

The killer feature: performance guarantees live *next to* correctness guarantees, in the same contract block, checked by the same compiler. One `vowc build --perf` and you know both *correct* and *fast enough*.
