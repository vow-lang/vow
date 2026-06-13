# Code Complexity Measurement: State of the Art + `vowc complexity` Design

> Research note + design spec. The state-of-the-art survey is fact-checked: each
> empirical claim was extracted from a primary source and survived 3-vote
> adversarial verification (≥2/3 to confirm). Confidence levels and the exact
> vote are preserved. Sources are listed in the Appendix.
>
> Scope chosen with the requester: prioritize **classic structural metrics**,
> **effect & linearity surface**, and **agent-bug-risk**. Deliverable is this
> report + a concrete `vowc complexity` design — no code yet.

---

## Part 0 — Executive summary

1. **The field's own verdict on structural complexity is sobering.** Most
   structural metrics — cyclomatic complexity above all — correlate strongly
   with lines of code and add little *independent* predictive power for defects.
   Once size is statistically controlled, classic OO complexity/coupling metrics
   lose their association with fault-proneness almost entirely
   (El Emam et al., IEEE TSE 2001, *high*). Static-feature defect predictors have
   hit a documented "ceiling" (Menzies et al., ASE 2010, *high*).

2. **Cognitive Complexity is the one metric with human-validated grounding** — but
   only for the *time* to understand code, **not its correctness**. A 10-study
   meta-analysis (~24,000 human evaluations) found r = **0.54** with comprehension
   time, r = **−0.29** with subjective understandability, and essentially **zero**
   (−0.13, and 0.00 against physiological effort) with comprehension *correctness*
   (Muñoz Barón et al., ESEM 2020, *high*).

3. **For an agent-oriented language, that boundary is the whole game.** The
   outcome Vow cares about is "will an agent generate a *correct*, *verifiable*
   version of this function" — and the validated metrics demonstrably do **not**
   capture correctness. So the right move is *not* to import a metric and assert
   it predicts agent-bug-risk. It is to **measure metrics and validate them
   against Vow's own ground-truth signals** (does it verify? how many CEGIS
   rounds? does ESBMC time out?). Vow has a correctness oracle almost no other
   language has — use it (Part 4).

4. **The effect-system / linear-type / LLM-complexity literature is thin to
   absent in the verified corpus.** No primary source survived that gives a
   formula or empirical validation for effect-system complexity, linear/ownership
   complexity, or "complexity vs LLM generation success." That is itself a
   finding: Vow's effect/linearity/contract metrics are **novel territory**, to
   be shipped as *experimental, size-controlled* signals — hypotheses to
   validate, not established metrics to trust.

5. **Design consequences** (Part 2): always report size next to every metric;
   prefer nesting-aware Cognitive Complexity over raw decision counts; expose
   constituent dimensions rather than one collapsed score; keep all thresholds
   advisory and out of the language; and build the harness to validate Vow
   metrics against verification outcomes.

---

## Part 1 — State of the art (cited)

### 1.1 Classic structural metrics

#### Cyclomatic complexity — McCabe (1976)

- **Definition.** `v(G) = e − n + 2p` on the control-flow graph (e = edges,
  n = nodes, p = connected components). For a single connected program this
  reduces to **(number of decision points) + 1** (Shepperd 1988; Radon docs,
  *high*, 3-0).
- **Claims to measure.** Number of linearly independent paths → testing effort /
  control-flow intricacy. McCabe proposed an advisory limit of **10 per module**,
  explicitly "reasonable, but not magical," relaxable for large `switch`/`case`
  (*high*, 3-0).
- **Validated value.** Weak as an *independent* predictor. Shepperd's foundational
  critique: "for a large class of software it is no more than a proxy for, and in
  many cases is outperformed by, lines of code … outperformed by LOC in over a
  third of the studies considered" (Software Engineering Journal 1988, *high*,
  3-0). Shepperd & Ince (1994) and Fenton & Pfleeger (1997) reach the same
  conclusion (*high*, 3-0).
- **Weakness.** Tracks LOC; little independent information; flat `switch` with N
  arms scores the same as N deeply-nested branches despite very different
  difficulty.
- **Important nuance (do not overstate).** The strong claim that "CC ≈ LOC nearly
  perfectly, LOC explains ~90% of CC variance" was **refuted** here (0-3), as was
  "CC has *no* independent power" (refuted 1-2). The defensible position is:
  **strong correlation with size, little incremental predictive value** — not
  exact equivalence. Landman et al. (2014/2016) dispute the precise magnitude for
  individual Java methods.

#### Cognitive Complexity — SonarSource / Campbell (2017)

- **Definition (three rules).** (1) *Ignore* structures that shorthand multiple
  statements readably (e.g. a method call, null-coalescing); (2) **+1 for each
  break in linear control flow**; (3) **a nesting increment** when flow-breaking
  structures are nested inside one another (SonarSource white paper, *high*, 3-0).
- **Claims to measure.** *Understandability* — deliberately abandons McCabe's
  graph model "in favor of a set of simple rules." Two methods with identical
  cyclomatic complexity get very different scores: a nested-loop `sumOfPrimes` = 7
  vs a flat-`switch` `getWords` = 1 (*high*, 3-0).
- **Validated value.** The only solely-code-based metric empirically validated to
  reflect *some* aspects of understandability. Meta-analysis (10 studies, 427
  snippets, ~24k evaluations): **r = 0.54** with comprehension time
  (random-effects), **r = −0.29** with subjective ratings (*high*, 3-0).
- **Weakness 1 — heterogeneity.** The 0.54 is an average over wildly varying
  studies: per-study r ranges −0.03 to 0.94, I² = 85% (*high*, 3-0).
- **Weakness 2 — correctness blind spot (critical for Vow).** Across 6 studies /
  269 snippets, correlation with comprehension *correctness* was only −0.13
  (range −0.52 to +0.57), and **0.00** with physiological effort. Authors: "little
  to no support for the assumption that Cognitive Complexity correlates with the
  correctness of code comprehension" (*high*, 3-0). A 2022 JSS follow-up
  (Scalabrino et al.) further questions its added value over traditional metrics.
- **Counting caveat.** A specific worked nesting total (the "+1,+2,+3 ⇒ 9"
  example) was **refuted** (1-2) — i.e. exact increment bookkeeping is
  implementation-sensitive; pin the increment table down explicitly (we do, §3.2).

#### Halstead complexity measures — Halstead (1977)

- **Definition.** From four base counts: η1/n1 = distinct operators, η2/n2 =
  distinct operands, N1 = total operators, N2 = total operands. Then:
  - vocabulary `η = n1 + n2`
  - length `N = N1 + N2`
  - **volume** `V = N · log2(η)`
  - **difficulty** `D = (n1 / 2) · (N2 / n2)`
  - **effort** `E = D · V`
  (Radon docs, matching Halstead 1977, *high*, 3-0.)
- **Claims to measure.** Program "size in bits" (V), mental effort to (re)write
  (E). Volume feeds the Maintainability Index.
- **Weakness.** Extremely sensitive to what counts as an operator vs operand;
  cross-tool comparability is poor unless the classification is fixed and
  documented. Limited independent predictive value (folds into the size critique).

#### Maintainability Index — Oman & Hagemeyer (1992), SEI / Visual Studio variants

- **Definition (Radon's 0-100-normalized hybrid).**
  `MI = max[0, 100·(171 − 5.2·ln V − 0.23·G − 16.2·ln L + 50·sin(√(2.4·C))) / 171]`
  where V = Halstead Volume, G = total cyclomatic complexity, L = SLOC,
  C = fraction of comment lines (in radians) (*high*, 2-1).
- **Lineage caveat (the dissenting vote).** This is **not** "purely SEI." It is a
  hybrid: the 0-100 normalization is the Visual Studio contribution; the original
  SEI/Oman-Hagemeyer derivative is *unbounded* and uses log2; Visual Studio's
  variant omits the comment term. Describe MI as an **SEI + Visual-Studio hybrid**.
- **Weakness.** Radon itself labels MI "a very experimental metric." The comment
  term is widely criticized (comments can be gamed); the constants are dataset-fit
  and not theoretically motivated. (Van Deursen's "Think twice before using the
  MI" is the standard critique — blog, not counted as a verified claim.)
- **Relevance to Vow.** The comment term is *actively misleading* for Vow:
  `//` comments are stripped at lex time and carry no machine-relevant intent
  (intent lives in contracts). If MI is offered at all, **drop the comment term**.

#### NPath — Nejmeh (1988)

- **Definition.** Number of acyclic execution paths through a function — the
  *product* (not sum) of the path-multiplicities of sequential decisions.
- **Claims to measure.** Test-path explosion that cyclomatic complexity
  *understates* (CC adds where NPath multiplies).
- **Weakness.** Explodes combinatorially; must be reported as a capped / log
  value. *No NPath formula or validation survived adversarial verification in this
  corpus* — treat its predictive value as unestablished here.

#### Maximum nesting depth

- **Definition.** Deepest level of nested control structures in a function.
- **Status.** A core ingredient of Cognitive Complexity's nesting increment
  (validated *as part of* Cognitive Complexity). As a *standalone* metric, no
  independent validation survived verification here. Cheap to compute; best used
  as an input to Cognitive Complexity, not a headline number.

#### Fan-in / fan-out and Henry-Kafura information flow (1981)

- **Definition.** fan-in = number of callers (+ data read); fan-out = number of
  callees (+ data written). Henry-Kafura information-flow complexity ≈
  `length · (fan-in · fan-out)²`.
- **Status.** **No verified claim** giving the formula or empirical validation
  survived in this corpus. The closest verified result is the OO size-confounding
  finding (below), which applies to coupling metrics generally: report them, but
  demonstrate value beyond size before trusting them.

#### LOC variants (SLOC / LLOC / NLOC)

- **Definition.** Physical lines (SLOC), logical statements (LLOC), non-comment
  lines (NLOC). `lizard` reports NLOC; Radon reports SLOC and uses it in MI.
- **Status — the quiet winner.** Because nearly every structural metric is a size
  proxy, **size is the baseline every other metric must beat.** This is the single
  most actionable empirical result for the tool's design (§2).

### 1.2 Tooling landscape

| Tool | Headlines | Computes (verified) | Notes |
|---|---|---|---|
| **lizard** | Cyclomatic | Per-function **CCN, NLOC, token count, parameter count**; threshold flags (`-C`, `-a`); multi-language (*high*, 3-0) | Multi-metric despite the "CC" reputation. Good UX model for per-function tabular output. |
| **radon** (Python) | CC + Halstead + MI | CC as decision-count+1 from the AST with an **explicit per-construct increment table** (if/elif/for/while/except/with/assert/comprehension/boolean-op each +1; else/finally +0; `case` +1, `case _` +0); full Halstead suite; the MI hybrid above (*high*, 3-0) | Best reference for *exact* AST-level counting conventions. Note: counts each boolean operator +1, diverging from "pure McCabe." |
| **rust-code-analysis** (Mozilla) | Multi-metric, tree-sitter | CC, Cognitive, Halstead, MI, NOM, NARGS per AST node | Closest architectural analogue to a compiler-integrated pass over a typed AST; useful design reference (fetched, claims not individually re-verified). |
| **SonarQube** | Cognitive (origin) | Cognitive Complexity + CC | Canonical Cognitive Complexity implementation; the white paper is the spec. |
| **PMD / Understand** | Rule engines / multi-metric | CC, NPath, coupling, etc. | Named in scope; not verified against primary docs in this run. |

**Tools disagree on counting.** Radon counts each boolean operator as a decision;
"pure McCabe" does not. This is why §3.2 fixes Vow's counting conventions
explicitly and §4 makes AST-vs-IR cyclomatic agreement a self-check.

### 1.3 The empirical verdict: size confounding & the ceiling effect

- **Size confounding (El Emam, Benlarbi, Goel & Rai, IEEE TSE 2001, *high*, 3-0).**
  *Before* controlling for class size, Chidamber-Kemerer and some Lorenz-Kidd OO
  metrics were associated with fault-proneness (reproducing prior studies).
  *After* controlling for size, **none remained associated.** Of 24 proposed OO
  metrics, only 4 stayed fault-related and only 2 were useful in prediction
  models. Recommendation (verified, 3-0): **re-examine prior validations and
  always control for size.** (A rebuttal — Evanco 2003 — disputes whether size is
  formally a "confounder" but does not dispute that the associations vanish under
  size control.)
- **Ceiling effect (Menzies et al., ASE 2010, *high*, 3-0).** Defect predictors
  built on static features (McCabe + Halstead + LOC) hit a wall: "better data
  mining technology is not leading to better defect predictors … we have reached
  the limits of the standard learning goal." Progress needs **new information
  sources**, not better learners on the same static features. (Later AST/semantic
  deep-learning work challenges permanence — but by *adding* information,
  consistent with the diagnosis. Frame as a paradigm ceiling, not an absolute.)
- **Decompose, don't collapse (Hybrid Cyclomatic Complexity, arXiv 2504.00477,
  2025 preprint, *medium*).** Summing a class's own and inherited complexity into
  one number "loses important aspects regarding complexity"; the decomposed
  representation was slightly superior in an SVM defect model. Single,
  not-yet-peer-reviewed, internally inconsistent on one figure (0.33 in prose vs
  0.55 in heatmaps) → medium confidence, but the **expose-components** takeaway
  aligns with everything else.

### 1.4 Cognitive Complexity: validated for time, blind to correctness

Restating because it drives the whole agent-bug-risk question: Cognitive
Complexity is the best-validated metric we have, and it predicts how *long* code
takes to understand — not whether it is understood *correctly*, and not
physiological effort. For a tool whose telos is "help agents produce **correct**,
verified code," importing Cognitive Complexity as a proxy for bug-risk would be
adopting precisely the metric that the evidence says **does not** track
correctness. Use it as a *readability* signal; do not claim it predicts
agent-bug-risk without Vow-specific validation (Part 4).

### 1.5 Gaps in families (2) and (3)

The verified corpus contains **no** primary, validated claim for:

- effect-system complexity quantification;
- linear / affine / ownership (Rust borrow-checker) complexity metrics;
- CBO / LCOM / instability `I = Ce/(Ca+Ce)` / abstractness formulas with
  incremental value;
- Henry-Kafura / NPath exact formulas with validation;
- **complexity vs LLM/agent code-generation success, comprehension, or repair**.

The open questions returned by the research run say this plainly: families (2) and
(3) "remain unanswered by surviving claims" and "may be genuinely thin literature
or simply unsearched." For the design, the safe and honest reading is: **anything
Vow-specific is unproven and must ship as experimental + size-controlled.** (If we
want to close family (2)/(3) properly, that is a dedicated second research pass —
flagged in Part 5.)

### 1.6 Claims that were refuted (so the design doesn't repeat them)

- "CC ≈ LOC near-perfectly; LOC explains ~90% of CC variance" — **refuted 0-3.**
- "CC has no independent explanatory power at all" — **refuted 1-2.**
- "Only Halstead Volume / Nesting / #Procedures predict defects (~0.3), CC & LOC
  not significant" — **refuted 0-3.**
- "Six metrics combined explain only 27.6% of defect variance" — **refuted 0-3.**
- Cognitive Complexity "+1,+2,+3 ⇒ total 9" worked example — **refuted 1-2.**
- "lizard self-describes as measuring *apparent* complexity" — **refuted 0-3.**

Net: state the size-correlation as *strong but not equivalence*, and pin counting
conventions down rather than trusting any single worked example.

---

## Part 2 — Design principles for a Vow complexity tool

Derived directly from Part 1:

1. **Size is the baseline, always co-reported.** Every per-function record carries
   NLOC (and token count). No complexity number is ever shown or gated without its
   size sitting next to it. Rationale: size confounding (El Emam 2001) + the LOC-
   proxy critique (Shepperd 1988).
2. **Prefer nesting-aware over decision-count.** Cognitive Complexity is the
   headline structural number; cyclomatic is reported but framed as the
   testing/path proxy it is. Rationale: §1.1, §1.4.
3. **One score to *gate*, the component vector to *diagnose*.** The headline is a
   single bounded `complexity_score` (0–100) so a caller can apply a flat rule
   ("> 80 ⇒ refactor"). But the score never travels alone: every record also
   carries the component vector and the raw inputs, so once a function is flagged
   the consumer can see *why* (tangled vs merely large) and pick the right remedy.
   The single number answers "should I act?"; the components answer "how?". This
   reconciles the actionable-gate requirement with the HCC "don't lose information
   in a collapsed score" finding (§1.3): nothing is lost — the score is a *view*,
   not a replacement. The score is a **readability / refactor-priority** signal
   (matched to the refactor action), explicitly **not** a defect predictor (§1.4).
4. **Vow-specific dimensions ship as `experimental`.** Effect breadth, linear-flow
   complexity, contract/predicate complexity, coupling — all tagged experimental
   in the JSON, none claimed to predict defects until validated against Vow's own
   outcomes. Rationale: §1.5.
5. **Validate against Vow's correctness oracle, not against comprehension.** The
   unique asset is verification outcomes (verify pass/fail, ESBMC time, CEGIS
   iteration count, mutation survival). That is the agent-bug-risk outcome
   variable. Rationale: §1.4, §1.5, Part 4.
6. **Thresholds are advisory and live outside the language.** Per the project's
   "decouple language from prover" rule, no BMC bound or complexity threshold
   leaks into the language, contracts, or default CLI behavior. Gating is opt-in
   (`--max-*` flags) for CI only.
7. **Deterministic, agent-first output.** Structured JSON by default (binary
   fixed-point ethos: stable ordering via `BTreeMap`, sorted keys). Human table is
   a `--format human` convenience, mirroring lizard.
8. **Drop misleading borrowings.** No comment term in any MI-like score (comments
   are stripped at lex; intent is in contracts).

---

## Part 3 — `vowc complexity` design spec

### 3.1 Metric catalog

Three tiers, each tagged with a stability label in the JSON
(`stable` = validated in the literature; `experimental` = Vow-specific / unproven).

**Headline — `complexity_score` (0–100), the gate (tier: stable).** A single
bounded number per function (and per file), derived from the tiers below by the
normalization in §3.2a. Default policy: `> 80 ⇒ refactor/dedupe`. Always emitted
together with `score_factors` (the sub-scores it was built from) so the flag is
self-explaining.

**A. Per-function — size (tier: stable baseline)**
- `nloc` — non-comment lines spanned by the function body (from spans → line nums).
- `tokens` — token count over the function (Halstead N, reused).
- `stmts` — statement count (`blk_stmts` list lengths, recursive).
- `params` — parameter count (`list_len(fn_params_lid)`).

**B. Per-function — structural (tier: stable)**
- `cyclomatic` — decision-count form (§3.2). Authoritative value cross-checked
  against IR `e − n + 2`.
- `cognitive` — Vow-adapted Cognitive Complexity (§3.2). **Headline metric.**
- `max_nesting` — deepest control nesting (input to cognitive; also exposed).
- `npath` — acyclic path count, capped (reported as `{value, capped: bool}`).
- `halstead` — `{n1, n2, N1, N2, vocabulary, length, volume, difficulty, effort}`.

**C. Per-function — Vow surface (tier: experimental)**
- `effects` — list of effect names from the `fn_eff` bitset.
- `effect_breadth` — `popcount(fn_eff)` (0–5 today: IO/PANIC/READ/UNSAFE/WRITE).
- `effect_fanout` — number of *distinct effectful callees* invoked (call graph ×
  callee effect sets); captures "how much effect surface this function pulls in."
- `linear_values` — count of linear-typed values constructed in the function
  (struct literals whose `sdef_is_linear`), via the checker's `is_linear_ty`.
- `linear_consumes` / `linear_borrows` — `IOP_LINEAR_CONSUME` / `IOP_LINEAR_BORROW`
  counts from the IR; proxy for resource-flow bookkeeping burden.
- `contract` —
  `{requires, ensures, invariants, predicate_nodes, predicate_depth, free_vars, has_vec_quantification}`
  computed by walking each vow clause expr (`fn_vow_lid` → clause `eid`).

**D. Module-level — coupling (tier: experimental, size-controlled)**
- `fan_out` — distinct callees per function, aggregated.
- `fan_in` — distinct callers (whole-module call graph; reuse the
  `collect_callees_dfs` pattern in `c_emitter.vow`).
- `henry_kafura` — `nloc · (fan_in · fan_out)²`, reported but flagged unvalidated.
- `instability` — `I = Ce / (Ca + Ce)` from the `use`/call graph (optional).

**E. Per-function — verification difficulty (tier: experimental, Vow-unique)**
- `loops_total` and `loops_without_invariant` — loops the BMC must unwind blind.
- `max_loop_nesting` — unwind-burden proxy.
- `contract_predicate_cost` — predicate node count + free-var count + quantifier
  flag, summed across clauses. *This is the bridge to Part 4*: it is the cheap
  static signal we will validate against actual ESBMC time / CEGIS rounds.

> Tier E is the part no other language's complexity tool can have, because no
> other mainstream language ships a model checker that yields a per-function
> correctness/cost outcome. It is the reason a Vow complexity tool is worth
> building rather than just running `lizard` on transpiled output.

### 3.2 Vow-adapted formulas

**Cyclomatic (decision-count form), base 1, +1 for each:**
- `EXPR_IF` (each `if` / `else if` head — `else` adds 0)
- `EXPR_WHILE`, `EXPR_FOR`, `EXPR_LOOP`
- each `EXPR_MATCH` arm beyond the first (`#arms − 1`)
- each short-circuiting `&&` / `||` (`BINOP` logical-and / logical-or) — *Radon
  convention; documented as a choice, not "pure McCabe"*
- each `?` propagation (`EXPR_QUESTION`) — implicit error branch

Cross-check: build `v(G) = edges − nodes + 2` from `IrFunction.blocks` using
terminator edges (`IOP_BRANCH` has two successors, `IOP_JUMP` one,
`IOP_RETURN`/`IOP_UNREACHABLE` zero). AST and IR forms should agree up to the
documented `&&`/`||`/`?` conventions; disagreement beyond that is a bug (§4).

**Cognitive Complexity (Vow increment table):**
- *Structural increment* (+1, **and** subject to nesting): `if`, each `else if`,
  `while`, `for`, `loop`, `match`.
- *Hybrid/secondary increment* (+1, **no** nesting): `else`, `break`/`continue`
  **with a label/value**, a sequence of like binary logical operators (one +1 per
  contiguous run of `&&` or of `||`, switching operator starts a new run),
  and `?` propagation.
- *Nesting increment*: +1 additional for **each** enclosing structural structure
  when entering a new structural structure (the SonarSource nesting rule).
- *Recursion*: +1 per function that calls itself (SonarSource counts recursion;
  detect self-`EXPR_CALL`).
- *Ignored* (rule 1): plain method/function calls, field access, indexing, casts,
  struct/enum construction — these don't break linear flow.

> The exact increments are pinned here precisely because the literature's worked
> example was refuted (§1.6) — we own our table and test it with golden cases (§4).

**Halstead operator/operand classification for Vow (fixed & documented):**
- **Operators (η1/N1):** all `BINOP_*` (incl. the *checked* arith ops `+!`/`-!`/…
  counted distinctly from wrapping), all `UNOP_*`, assignment, call `()`, index
  `[]`, field `.`, method-call, cast `as`, and the control keywords
  `if`/`else`/`while`/`for`/`loop`/`match`/`return`/`break`/`continue`/`?`.
- **Operands (η2/N2):** identifiers (`EXPR_IDENT`), and literals
  (`EXPR_LIT_INT`/`_BOOL`/`_STR`).
- Formulas exactly per §1.1 (Halstead 1977 / Radon).

**Maintainability Index (optional, Vow-corrected):** if offered, use the
SEI+VisualStudio hybrid **without the comment term** (comments are non-semantic in
Vow): `MI' = max[0, 100·(171 − 5.2·ln V − 0.23·G − 16.2·ln L) / 171]`. Label it
experimental, exactly as Radon does.

**Contract predicate cost:** for each `requires`/`ensures`/`invariant` clause,
walk the predicate expr arena counting nodes (`predicate_nodes`), tracking max
depth (`predicate_depth`), distinct free identifiers (`free_vars`, reuse the
`VowEntry.bindings` notion), and whether it indexes/quantifies over a `Vec`
(`has_vec_quantification`). Sum across clauses.

### 3.2a The single 0–100 score (the gate)

**Goal.** Collapse the vector into one bounded, monotonic number whose `80` line is
*stable across codebases* and tied to an established threshold, so a flat rule
("score > 80 ⇒ refactor") is meaningful — not an arbitrary cutoff on an arbitrary
scale.

**Two legitimate reasons to refactor**, each able to trip the gate on its own:
- the function is **tangled** → drive from **Cognitive Complexity** (the validated
  comprehension metric, §1.4);
- the function is **too large** → drive from **NLOC** (size is the dominant real
  signal, §1.3, and the action "deduplicate / split" is a size action).

**Step 1 — sub-scores in [0,1], each anchored so its threshold maps to 0.80.**

Anchors are chosen from established conventions, *not* fit to make a number look
good:
- `COG_ANCHOR = 15` — SonarQube's default "flag this function" Cognitive
  Complexity threshold.
- `NLOC_ANCHOR = 60` — common "function is getting long" guidance (~50–60 lines).

For an anchor `T` and value `x`, a linear-then-saturating map:
```
f(x, T) = 0.80 * (x / T)                          if x <= T
        = 0.80 + 0.20 * (1 - exp(-(x - T) / T))   if x >  T   (asymptotes to 1.0)
```
- `c = f(cognitive, COG_ANCHOR)`
- `s = f(nloc,      NLOC_ANCHOR)`

**Step 2 — soft-OR combine (compounding, never exceeds 1):**
```
base = 1 - (1 - c) * (1 - s)
```
Either factor at its anchor ⇒ `base ≥ 0.80`; both high ⇒ higher than either alone.

**Step 3 — optional capped Vow-surface bump (experimental, default on, small):**
```
v = min(0.15, 0.05*excess_effect_breadth + 0.03*linear_consumes
                 + 0.02*contract_predicate_cost_over_budget)      // each term clamped >= 0
```
Capped at 0.15 so Vow-specific signals can push a *borderline* function over the
line but can never, alone, flag a small, simple, well-contracted function.

**Step 4 — final score:**
```
complexity_score = round(100 * min(1.0, base + v))
```

**Worked intuition (defaults):**
- cognitive 8, nloc 30 → c=0.43, s=0.40 → base=0.66 → **~66** (below 80, fine).
- cognitive 15, nloc 30 → c=0.80, s=0.40 → base=0.88 → **~88** (flag: tangled).
- cognitive 4, nloc 200 → c=0.21, s=0.98 → base=0.98 → **~98** (flag: too large / dedupe).
- cognitive 15, nloc 60 → c=0.80, s=0.80 → base=0.96 → **~96** (flag: both).

So "score > 80" ≈ "at or past the SonarQube Cognitive line, **or** a long
function, **or** both moderately high" — a defensible, fixed-semantics gate.

**File-level score.** `file.complexity_score = max(function scores)` — a file is as
urgent as its worst function — and also report `functions_over_threshold` (count)
and file `nloc`, so "this file has 3 functions > 80" is directly actionable.

**Anchors are flags, not magic.** Expose `--cog-anchor`, `--nloc-anchor`,
`--max-score` (default 80). Keep the *defaults* fixed so the rule is portable.

**Calibrate once, then trust.** Before shipping defaults, run the score over
`compiler/*.vow` and check the distribution: `> 80` should flag roughly the worst
~5–15% of functions (the ones a reviewer would agree need work). If it flags 40%
or 0.5%, the anchors are wrong — adjust them, don't redefine the scale. This keeps
absolute semantics honest (the size-control discipline of §1.3 applied to the
gate itself). A codebase-relative alternative (80 = the 90th percentile) is
available but sacrifices portability of the rule, so it is *not* the default.

**Honesty guardrails.** (1) The score predicts *readability / refactor priority*,
not bugs — never relabel it as defect risk. (2) It is a **soft prompt to review**,
not a hard law; the component vector, not the scalar, decides the actual fix.
(3) The Vow bump is `experimental` until §4 validation; the score is fully
defensible with `v = 0` (pure cognitive+size) if that validation is pending.

### 3.3 Substrate mapping (where each metric reads from)

From the codebase map (file refs are to the self-hosted compiler):

| Metric | Substrate | Key handles |
|---|---|---|
| nloc / stmts / params | AST + source | `fn_body_bid` → `blk_stmts` (`ast.vow:184`); `fn_params_lid` (`ast.vow:196`); spans → line numbers |
| cyclomatic (primary) | AST | decision exprs: `EXPR_IF/WHILE/FOR/LOOP/MATCH`, `BINOP` and/or, `EXPR_QUESTION` (`ast.vow:3-57`) |
| cyclomatic (cross-check) | IR | `IrFunction.blocks` + terminators `iop_is_terminal` (`ir.vow:170`, `ir.vow:242`) |
| cognitive / max_nesting | AST | recursive expr walk with a nesting counter |
| halstead / tokens | AST or token stream | `BINOP/UNOP` kinds, `EXPR_IDENT`, literals |
| effects / effect_breadth | AST **or** IR (both bitsets) | `fn_eff` (`ast.vow:86-90`), `IrFunction.effects` (`ir.vow:268`) |
| linear_values | checker | `sdef_is_linear` (`ast.vow:205`), `is_linear_ty` recursion (`checker.vow:26-154`) |
| linear_consumes/borrows | IR | `IOP_LINEAR_CONSUME=79` / `IOP_LINEAR_BORROW=80` (`ir.vow:90-98`) |
| contract.* | AST | `fn_vow_lid` clause list → clause `eid` (`ast.vow:31-33,276-285`) |
| fan_out / fan_in | IR call graph | `IOP_CALL` targets; DFS pattern `collect_callees_dfs` (`c_emitter.vow:455-494`) |
| verification difficulty | AST (+ later, real outcomes) | loop exprs, contract cost (above); Part 4 joins ESBMC/CEGIS data |

**Recommendation:** compute everything from the **AST** except cyclomatic
cross-check, `linear_consumes/borrows`, and the call graph, which are cleaner from
the **IR**. The AST is the right substrate because complexity is a
language-surface property and the AST preserves nesting structure that the IR
flattens.

### 3.4 CLI surface + JSON schema

Mirror the `vowc mutants` scaffolding (`compiler/mutants_main.vow`,
`compiler/main.vow` dispatch).

```
vowc complexity <file.vow>...            # analyze given files, JSON to stdout
vowc complexity --root DIR               # analyze all *.vow under DIR (skip test_*)
vowc complexity --root DIR --output-dir complexity.out   # batch: per-file + summary
vowc complexity <file> --format human    # lizard-style table
vowc complexity <file> --max-cognitive N --max-cyclomatic N   # CI gating (opt-in)
```

- **Flag parsing** reuses the mutants helpers (`--root`, `--output-dir`,
  `is_decimal_int_literal`). `--shard X/Y` is optional (cheap pass, likely
  unnecessary; include only if batch runtime warrants it).
- **Exit codes.** `0` always, *unless* a `--max-*` threshold is exceeded → nonzero.
  No threshold flags ⇒ pure reporting (principle 6).
- **File filtering.** Skip `test_*.vow` (mutants convention,
  `mutants_main.vow:229`). Unlike mutants, do **not** skip `// GENERATE` /
  `extern "C"` blocks — generated/FFI code still has measurable complexity; instead
  tag those functions `origin: "generated" | "extern"` so consumers can filter.

**JSON schema (v1):**

```json
{
  "schema_version": "1",
  "kind": "complexity_report",
  "tool": "vow",
  "files": [
    {
      "file": "compiler/lexer.vow",
      "complexity_score": 88,
      "functions_over_threshold": 1,
      "nloc": 642,
      "functions": [
        {
          "name": "lex_number",
          "line": 142,
          "origin": "source",
          "complexity_score": 66,
          "score_factors": { "cognitive_sub": 0.43, "size_sub": 0.40,
                             "vow_bump": 0.0, "base": 0.66, "over_threshold": false },
          "size":       { "nloc": 23, "tokens": 118, "stmts": 14, "params": 2 },
          "structural": {
            "cyclomatic": 7,
            "cyclomatic_ir": 7,
            "cognitive": 11,
            "max_nesting": 3,
            "npath": { "value": 24, "capped": false },
            "halstead": { "n1": 12, "n2": 19, "N1": 60, "N2": 58,
                          "vocabulary": 31, "length": 118,
                          "volume": 584.6, "difficulty": 18.3, "effort": 10698.2 }
          },
          "vow": {
            "tier": "experimental",
            "effects": ["panic"],
            "effect_breadth": 1,
            "effect_fanout": 0,
            "linear_values": 0, "linear_consumes": 0, "linear_borrows": 0,
            "contract": { "requires": 1, "ensures": 1, "invariants": 0,
                          "predicate_nodes": 14, "predicate_depth": 3,
                          "free_vars": 3, "has_vec_quantification": false }
          },
          "verification": {
            "tier": "experimental",
            "loops_total": 1, "loops_without_invariant": 0,
            "max_loop_nesting": 1, "contract_predicate_cost": 14
          }
        }
      ],
      "module": {
        "tier": "experimental",
        "functions": 37, "fan_in_max": 9, "fan_out_max": 12,
        "henry_kafura_max": 480, "instability": 0.62
      }
    }
  ],
  "summary": {
    "functions": 412,
    "complexity_score": { "p50": 38, "p90": 72, "max": 96, "argmax": "lower.vow:lower_expr",
                          "over_threshold": 27, "threshold": 80 },
    "cognitive": { "p50": 4, "p90": 14, "max": 41, "argmax": "lower.vow:lower_expr" },
    "cyclomatic": { "p50": 3, "p90": 11, "max": 33 },
    "nloc_total": 9821,
    "thresholds_exceeded": ["lower.vow:lower_expr", "checker.vow:check_expr"]
  }
}
```

- Every structural number sits beside its `size` (principle 1).
- `tier` fields make the experimental status machine-visible (principle 4).
- Percentile summaries (not just max) so agents can compare a function to the
  codebase distribution rather than to an absolute literature threshold.
- Stable ordering: files sorted by path, functions by source line, object keys
  fixed (binary-fixed-point ethos, principle 7).

### 3.5 Integration points

**Self-hosted (primary):**
1. `compiler/main.vow:136-142` — add `fn CMD_COMPLEXITY() -> i64 { 7 }`.
2. `compiler/main.vow:144-168` — `if sub == "complexity" { return CMD_COMPLEXITY(); }`.
3. `compiler/main.vow` `main()` dispatch — `if cmd == CMD_COMPLEXITY() { return run_complexity(argv); }`.
4. New `compiler/complexity.vow` — the metric pass (AST/IR walkers, formulas).
5. New `compiler/complexity_main.vow` — CLI plumbing, enumeration, JSON emission
   (clone `mutants_main.vow` structure: usage, flag parse, `enumerate_root`,
   `json_escape_str`, summary).
6. `skill_json()` / `skill_human()` (`main.vow:1372+`, `2212+`) — add the
   `complexity` command + `command_details`.

**Rust parity (required by CLAUDE.md — "modify BOTH compilers"):**
- Add the subcommand to the `vow` crate CLI driver.
- Implement the pass over `vow-syntax` AST (`vow-syntax/src/ast.rs`: `FnDef`,
  `ExprKind`, `VowClause`, `effects: Vec<Effect>`, `StructDef.is_linear`) and
  `vow-ir` for the IR cross-checks.
- Keep JSON byte-identical between the two (a parity test, like existing
  cross-compiler tests).

**Docs (required by CLAUDE.md — spec is source of truth):**
- `docs/spec/cli.md` — new `complexity` command, flags, JSON schema, exit codes.
- Regenerate `--help`/skill via `scripts/generate_help.py`, rebuild both compilers
  (mind the known skill-doc drift — keep the generated diff scoped).

### 3.6 What NOT to do

- **Don't collapse to a single score** as the primary output (§1.3 HCC).
- **Don't gate by default** or bake any threshold/BMC bound into the language or
  default CLI (decouple-from-prover rule).
- **Don't include a comment term** in any MI-like metric (comments are stripped).
- **Don't claim** effect/linear/contract metrics predict bugs until Part 4 shows
  it — ship them `experimental`.
- **Don't transpile-and-run-lizard.** The C model is verification scaffolding, not
  a faithful surface for language-level complexity; measure the Vow AST/IR.
- **Don't over-trust cyclomatic.** Report it, but lead with Cognitive + size.

---

## Part 4 — Validation plan

**What this score is for, and what it is *not*.** `complexity_score` measures
**agent/human comprehensibility** — "is this function hard to follow, and should
it be refactored or deduplicated for clarity." **Correctness is orthogonal and is
handled elsewhere in Vow: by tests and by `requires`/`ensures`/`invariant`
contracts (statically verified).** A function can be perfectly correct *and* score
95 (refactor it for clarity); it can be simple, score 10, *and* be wrong (the
contracts/tests catch that). The two axes do not substitute for each other, and
the gate must never be sold as a bug/defect signal — the literature is explicit
that comprehension-based metrics do not track correctness (§1.4).

**So the gate's validation target is comprehensibility, not an oracle.** The
literature's core discipline still applies — *don't trust a metric you haven't
shown beats size* (§1.3) — so validation is primarily **calibration against
refactor-worthiness judgments**: run the score over `compiler/*.vow`, confirm the
`> 80` set matches the functions a reviewer (or an agent asked "does this need
refactoring?") would independently flag, and confirm it adds signal beyond raw
NLOC. That is the whole job of the gate.

**Separately and optionally** (a curiosity, *not* the gate's validation): Vow
happens to expose correctness/cost oracles — `vow verify` outcome + ESBMC time,
CEGIS iteration count (`bench/`), mutation survival (`mutants.out`). It may be
interesting to check whether comprehensibility complexity *also* correlates with
verification cost, but that is an orthogonal study; a null result there does not
weaken the comprehensibility gate.

**Correctness/cost signals available in the repo (for the optional study only):**
- `vow verify` outcome per function: `Verified` / `Unverified` / `Skipped` /
  `VerifyFailed`, plus ESBMC wall-time.
- **CEGIS iteration count** and pass/fail from the benchmark runner (`bench/`,
  temperature-0, reproducible) — the closest thing to a direct *agent-bug-risk*
  measurement that exists.
- **Mutation survival** from `vowc mutants` (`mutants.out/outcomes.json`) — a
  per-site testability signal.

**Primary — calibrate the comprehensibility gate:**
1. **Threshold calibration.** Score `compiler/*.vow`; confirm `> 80` flags roughly
   the worst ~5–15% of functions and that a reviewer/agent asked "does this need
   refactoring for clarity?" agrees with the flagged set. Adjust the *anchors*
   (§3.2a) if the rate or the agreement is off; do not redefine the scale.
2. **Beats-size check.** Confirm the score adds signal beyond raw `nloc` (e.g. it
   flags short-but-deeply-tangled functions that NLOC alone would miss, and ranks
   them above long-but-flat ones). If the score never disagrees with NLOC ordering,
   it is just LOC in disguise (§1.3) and the cognitive factor is mis-weighted.
3. **AST↔IR cyclomatic self-check.** Continuous invariant: AST decision-count and
   IR `e−n+2` agree (modulo documented `&&`/`||`/`?` conventions). A divergence is
   a compiler bug, not a metric nuance.

**Optional & orthogonal (does NOT gate the comprehensibility score):**
4. **Complexity-vs-verification-cost curiosity.** Check whether comprehensibility
   complexity *also* correlates with ESBMC time / CEGIS rounds / mutation survival.
   Interesting if positive; irrelevant to the gate if null. Correctness remains the
   job of tests and contracts, not of this score.

**Golden tests (ship with v1):** small `.vow` files in `tests/` with
hand-computed expected metrics (decision tables make these deterministic),
including: a flat `match` vs a nested-`if` ladder of equal cyclomatic but
different cognitive (the SonarSource contrast, §1.1); a linear-struct
produce/consume chain; a function with a `Vec`-quantified `invariant`.

---

## Part 5 — Phased roadmap

- **Phase 1 — structural core (stable tier).** `complexity.vow` +
  `complexity_main.vow`; size + cyclomatic (AST) + cognitive + max_nesting +
  Halstead + params; JSON to stdout; `--format human`; golden tests; docs/spec +
  Rust parity. *Self-contained, immediately useful, fully grounded in validated
  literature.*
- **Phase 2 — IR cross-checks + coupling.** IR cyclomatic, call-graph fan-in/out,
  `linear_consumes/borrows`; AST↔IR self-check test. (experimental tier)
- **Phase 3 — Vow surface + verification difficulty.** effect breadth/fanout,
  linear_values, contract predicate cost, loops-without-invariant. (experimental)
- **Phase 4 — validation harness.** Wire metrics ↔ `vow verify` time, `bench/`
  CEGIS counts, `mutants.out` outcomes; run the size-control regressions (Part 4);
  promote any metric that beats `nloc` from `experimental` to `stable`.
- **(Optional) Phase 0.5 — close the research gap.** A dedicated second
  deep-research pass on family (2)/(3): effect-system & ownership complexity
  metrics and the emerging "code complexity vs LLM success" literature, which the
  first pass found thin. Do this if we want external grounding before trusting the
  Vow-specific dimensions; otherwise Part 4's internal validation suffices.

---

## Appendix — Sources (verified claims)

| # | Source | Used for | Conf. | Vote |
|---|---|---|---|---|
| 1 | SonarSource, *Cognitive Complexity* white paper (Campbell 2017) — sonarsource.com/resources/cognitive-complexity/ | Cognitive Complexity definition & CC contrast | high | 3-0 |
| 2 | Muñoz Barón, Wyrich & Wagner, ESEM 2020 — arxiv.org/pdf/2007.12520 | Cognitive Complexity meta-analysis (time/ratings/correctness, heterogeneity) | high | 3-0 |
| 3 | Shepperd, *A critique of cyclomatic complexity*, Software Eng. J. 1988 — cs.du.edu/~snarayan/.../cycl-1.pdf | CC as LOC proxy; v(G) formula & limit-of-10; poor foundations | high / medium | 3-0 / 2-1 |
| 4 | El Emam, Benlarbi, Goel & Rai, *Confounding Effect of Class Size*, IEEE TSE 2001 — semanticscholar.org/paper/eda92ba4… | Size confounding; control-for-size mandate | high | 3-0 |
| 5 | Menzies et al., *Defect prediction from static code features*, ASE 2010 — link.springer.com/article/10.1007/s10515-010-0069-5 | Ceiling effect; CC little independent info | high | 3-0 |
| 6 | Cernău, Dioșan & Șerban, *Unveiling Hybrid Cyclomatic Complexity*, arXiv 2504.00477 (2025 preprint) | Decompose-don't-collapse | medium | 2-1 / 3-0 |
| 7 | lizard — github.com/terryyin/lizard | Multi-metric tooling model (CCN/NLOC/token/param) | high | 3-0 |
| 8 | Radon docs — radon.readthedocs.io/en/latest/intro.html | CC decision-count table; Halstead suite; MI hybrid formula | high / 2-1 | 3-0 / 2-1 |
| — | rust-code-analysis — mozilla.github.io/rust-code-analysis/metrics.html | Compiler-integrated multi-metric design reference (fetched) | — | — |

**Honesty notes carried from the research run:** (a) family (2) effect/linear/
coupling and family (3) LLM-vs-complexity literature were *not* covered by
surviving verified claims — treat Vow-specific metrics as experimental; (b) the
CC≈LOC "near-perfect / no-independent-power" extremes were both refuted — say
"strong correlation, little incremental value"; (c) MI is an SEI+VisualStudio
hybrid and "very experimental," not pure SEI; (d) the Menzies ceiling is a
static-feature/traditional-learner paradigm ceiling, not absolute.
