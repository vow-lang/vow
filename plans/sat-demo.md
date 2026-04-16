# Plan: SAT Demo

> Source PRD: SAT demo design discussion in this thread (2026-04-14 to 2026-04-15)

## Architectural decisions

Durable decisions that apply across all phases:

- **Artifact boundary**: The entire demo lives under `examples/sat/`, including source, scripts, fixtures, tests, benchmark tooling, and README.
- **Execution model**: The deliverable is a CLI demo binary, not a reusable library API.
- **Input contract**: Accept standard DIMACS CNF only, from a positional path argument or from `stdin` when no path is provided.
- **Output contract**: Print `SAT` or `UNSAT` on `stdout`; for satisfiable inputs print one or more `v ... 0` assignment lines; use exit codes `10` for `SAT`, `20` for `UNSAT`, and `1` for parse/I/O/internal errors.
- **Implementation boundary**: Pure Vow only. No FFI or C helpers.
- **Core solver architecture**: Deterministic CDCL with flat clause storage, stable clause ids, watched literals, 1-based variable ids, trail/decision levels/reasons, first-UIP learning, non-chronological backtracking, phase saving, Luby restarts, learned-clause deletion, and batched compaction.
- **Preprocessing scope**: Model-preserving only in v1. Parser-time tautology and duplicate cleanup are allowed; elimination-based reconstruction is deferred.
- **Verification style**: Use many small, meaningful local contracts and loop invariants on helpers and data-structure operations; do not attempt a single global proof of solver correctness in milestone 1.
- **Testing model**: Keep tests local to the demo. Use a self-contained shell runner with fixture DIMACS cases and expected CLI behavior.
- **Benchmarking model**: Use a local manifest plus self-contained Python scripts under `examples/sat/`; detect baseline solvers on `PATH`; keep downloaded corpora and results under ignored local directories.
- **Performance policy**: Track performance locally and manually at first; do not make it a hard CI gate during the initial implementation phases.
- **Correctness policy for benchmarks**: Independently validate reported `SAT` assignments in the harness; treat `UNSAT` correctness as manifest-driven for curated instances.
- **Toolchain assumptions**: Proceed assuming issue `#156` is fixed before implementation begins; record the assumption clearly at kickoff. Track heap / priority-queue follow-up needs under issue `#165` if benchmarking shows linear scans are no longer sufficient.
- **Float audit**: Milestone 1 uses integer activities only, but the demo should explicitly record what future float support would enable for SAT heuristics and where current float support is still immature.

---

## Phase 1: Demo Shell

**User stories**: Run the SAT demo on DIMACS input from file or `stdin`; get strict SAT-solver-compatible output and error handling; keep the demo self-contained under `examples/sat/`.

### What to build

Build the thin end-to-end CLI path for the demo: input selection, strict DIMACS parsing entrypoint, SAT/UNSAT output contract, exit codes, local README guidance, and local shell-driven regression tests for success and malformed-input cases.

### Acceptance criteria

- [ ] The demo accepts DIMACS CNF from a positional path argument or `stdin` when no path is provided.
- [ ] The demo prints SAT-solver-compatible `stdout` and uses the agreed exit codes for `SAT`, `UNSAT`, and error conditions.
- [ ] Malformed DIMACS cases are rejected with clear error behavior.
- [ ] The demo remains fully self-contained under `examples/sat/`, including local test runner and fixtures.

---

## Phase 2: Root-Level Solver

**User stories**: Solve trivial `SAT` and `UNSAT` inputs correctly; simplify formulas safely before search; independently check satisfiable outputs in development tests.

### What to build

Build the first real solving slice beneath the CLI: strict clause ingestion, parser-time simplification, root-level unit handling, early contradiction detection, and assignment checking for small cases. This phase should already solve obvious instances correctly end to end.

### Acceptance criteria

- [ ] Duplicate literals and tautological clauses are handled according to the agreed model-preserving policy.
- [ ] Root-level unit propagation runs before any decision is made and can return `UNSAT` immediately on contradiction.
- [ ] Small hand-crafted `SAT` and `UNSAT` fixtures succeed through the CLI.
- [ ] Development tests can independently verify that a reported `SAT` assignment satisfies the parsed formula.

---

## Phase 3: Minimal Real CDCL

**User stories**: Solve nontrivial instances with a real SAT architecture; avoid a throwaway DPLL path; keep the solver structurally credible as a Vow demo.

### What to build

Build the first full end-to-end CDCL slice: watched propagation, trail and level tracking, reason clauses, first-UIP conflict analysis, learned clauses, and non-chronological backtracking. This phase establishes the solver architecture that later phases will refine rather than replace.

### Acceptance criteria

- [ ] The solver uses the agreed watched-literal and trail-based CDCL architecture.
- [ ] First-UIP learning and non-chronological backtracking are exercised by local regression cases.
- [ ] Nontrivial small and medium fixtures solve correctly through the CLI.
- [ ] The implementation remains deterministic across repeated runs on the same inputs.

---

## Phase 4: Deterministic Search Quality

**User stories**: Improve solve behavior without introducing nondeterminism; make the solver measurably stronger before benchmarking against external baselines.

### What to build

Refine the core solver with deterministic search-quality mechanisms: integer activity scoring, saved phases, Luby restarts, learned-clause protection rules, restart-boundary reduction, and batched compaction while preserving stable clause ids.

### Acceptance criteria

- [ ] The solver includes deterministic activity scoring, phase saving, and Luby restarts.
- [ ] Learned-clause reduction operates only at restart boundaries and respects the agreed protection policy.
- [ ] Batched compaction preserves stable clause identities and keeps the solver behavior correct.
- [ ] Local regression tests cover behavior that depends on restarts, learning, and clause retention policy.

---

## Phase 5: Model-Preserving Simplification

**User stories**: Improve performance on larger formulas without introducing assignment-reconstruction complexity; keep the runtime solver output directly aligned with the original variable set.

### What to build

Add model-preserving preprocessing and inprocessing beyond the parser-time cleanup: root-safe simplification and solver-time transformations that improve the search space while preserving direct SAT assignment reporting.

### Acceptance criteria

- [ ] Added simplifications are model-preserving and do not require reconstruction of eliminated variables.
- [ ] The solver still prints assignments for all declared variables in the original DIMACS header range.
- [ ] Simplification passes are covered by targeted regression cases.
- [ ] The new passes improve or at least do not regress behavior on the local curated benchmark subset.

---

## Phase 6: Benchmark Harness

**User stories**: Run the SAT demo on a curated SAT Competition subset; compare against local baseline solvers; classify failures clearly and validate satisfiable outputs independently.

### What to build

Build the self-contained local benchmarking workflow under `examples/sat/`: benchmark manifest, download/materialization tooling, execution harness, baseline-solver detection, result classification, SAT-assignment validation, and compact report generation across `smoke`, `core`, and `stretch` tiers.

### Acceptance criteria

- [ ] The harness can materialize a curated local benchmark subset from the committed manifest without checking large corpora into git.
- [ ] The harness detects baseline solvers on `PATH` or via explicit flags and skips missing baselines cleanly.
- [ ] Results distinguish solved cases from timeouts, crashes, parse failures, malformed output, and skipped runs.
- [ ] Reported `SAT` answers from the demo are validated independently by the harness.

---

## Phase 7: Pressure-Test Vow

**User stories**: Use the SAT demo to assess Vow’s readiness for serious systems workloads; identify concrete library, runtime, codegen, and language gaps revealed by the implementation and benchmarks.

### What to build

Capture the SAT demo’s value as a language/toolchain assessment: document discovered limitations, benchmark bottlenecks, workarounds, and follow-on issues; summarize what Vow currently supports well and what remains immature, including the planned float-support audit for future SAT heuristics.

### Acceptance criteria

- [ ] The demo README documents the scope, workflow, benchmark policy, and known toolchain assumptions clearly.
- [ ] The SAT work records concrete Vow/toolchain pain points and follow-on issue candidates discovered during implementation and benchmarking.
- [ ] The float-support audit is captured explicitly, including what later SAT heuristics would want from Vow and what current float support still lacks.
- [ ] The final local benchmark report is usable both as a solver-status artifact and as evidence about Vow/runtime/codegen readiness.
