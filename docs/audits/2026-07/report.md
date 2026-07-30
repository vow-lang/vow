# Vow Project Audit — Cycle 1

**Date:** 2026-07-27 · **HEAD:** `3cbfe87b` · **Scope:** design coherence, verification soundness,
self-hosted compiler parity, test/CI health, issue tracker reality, CEGIS-loop end-to-end behavior.

This is the first cycle of the recurring monthly audit (see `docs/audits/README.md` and
`.claude/commands/monthly-audit.md`). It has no prior baseline to diff against — that's `recap.md`'s job
from Cycle 2 onward. Produced by six parallel investigations (issue triage, design coherence,
verification-soundness ground truth, self-hosted bootstrap health, feature-matrix/test reality check,
CEGIS end-to-end smoke test), each independently reading code and running commands rather than trusting
prior project notes, followed by one direct empirical reproduction to reconcile a cross-agent
discrepancy (§4). Read-only — no issues closed, no code changed, no commits made during the audit itself.

## Scorecard

| Dimension | Verdict |
|---|---|
| Design coherence | Loops handled honestly; collections are not |
| Verification soundness | 2 real open holes; failure discipline elsewhere is strong |
| Self-hosted compiler | Bootstrap fixed point holds; 3 checks silently absent or miscoded |
| Tests & tooling | 602 + 1235 passing, clippy clean, no regressions |
| CEGIS loop | Real mechanism, but thin and dated evidence |
| Issue tracker | 278 open, ~40–55 already fixed but never closed |

## 1. The central tension: an honest checker undone by a quiet assumption

Vow's pitch rests on one sentence in its own docs: a contract is `proven` only when the verifier
establishes it **for all inputs**; otherwise the result is `unknown`, never a false `proven`. For loops,
that promise is kept, and kept deliberately — `solver_strategy.rs:269-284` hard-asserts that a retry may
never shrink the unwind bound, with a comment noting that doing so "would prove a weaker property." Real
engineering discipline, not a slogan.

Collections don't get the same treatment. A nondet `Vec` parameter is constrained with
`__ESBMC_assume(v.len >= 0 && v.len <= 128)` (`vow-verify/src/c_emitter.rs:1709,2028`) — an *assume*, not
an *assert*. For a function iterating to a nondet `n`, completeness is unreachable at any finite bound,
so the pipeline correctly says `unknown`. For a function iterating to `v.len()`, the same code path says
`proven` — because the assume just made completeness reachable by capping the domain at 128 elements.
Nothing in the emitted JSON distinguishes the two. `docs/spec/cli.md:446` still defines `proven` as
"ESBMC proved this contract holds for all inputs." For any function that takes a collection, that isn't
what happened.

CLAUDE.md's own contract-authoring rule is unambiguous: write the true, unbounded contract; if ESBMC
can't prove it, **mark the function unverifiable or skip it** — never smuggle a bound into the contract.
That escape hatch doesn't exist. There is no per-function opt-out anywhere in the grammar; `Skipped` is a
compiler-chosen, fail-closed outcome (exit 1), not something an author can reach for. The rule has no
compliance mechanism.

The benchmark corpus shows the predictable result: **67 of 107 reference solutions carry small-literal
bounds in their own `requires` clauses.** `benchmarks/humaneval/HE013_gcd/reference.vow:6-7` reads
`requires: a <= 50, b <= 50` — the *verbatim* construction CLAUDE.md names as forbidden, on the language's
own canonical example. 35 of those 107 are self-tagged `contract_fidelity = "partial"` or `"weak"` — an
honest internal metric, sitting right there, gating nothing.

> **Shipped to agents, contradicting itself.** `skills/vow/reference/contracts.md:291` — *"Do not add
> `requires: n <= 8`."* `skills/vow/examples/examples.md:157,190` — *"`requires: n <= 8` keeps iterations
> tractable for verification"* — and `contracts.md:139` links to that exact example as the canonical
> worked case.

This is the one finding every other thread in this audit runs into eventually: not a bug in the
verifier, but a gap between the rule Vow states about itself and the mechanism it gives authors — human
or agent — to follow that rule.

## 2. Design coherence, feature by feature

Effects, linear types, and the no-generics/no-closures/no-macros stance all hold up cleanly against the
project's own three-criteria bar. A few things don't:

- **`loop` fails criterion 1 by its own spec's sentence.** `grammar.md:508`: "ESBMC cannot verify
  unbounded `loop` constructs." A construct unverifiable by construction makes verification strictly
  harder, in the language's own words.
- **Checked (`+!`) and saturating (`add_sat_u8`) arithmetic answer the same design question twice.** One
  got a dedicated operator family, the other named intrinsics, with no stated rationale for the split.
- **Struct pass-by-pointer plus heap-type indexing-as-alias plausibly reintroduce an aliasing bug class**
  — a performance affordance working against criterion 2. This repo's own CLAUDE.md already documents
  agents tripping on exactly this shape of gotcha.
- **Narrowing intrinsics** (`<src>_to_<tgt>_{try,wrap,sat}`) are real combinatorial surface growth in an
  intentionally small language — defensible under criterion 2, but worth naming as a cost.

**Internal contradictions between spec files:**
- `cli.md:280,452` states `RegionAlloc`/`FieldGet`/`FieldSet` are modeled; `errors.md:440,446` says
  `VerificationSkipped` fires "most commonly" on those exact opcodes as unsupported. Directly opposed, on
  the fail-closed path.
- `--max-k-step` is exempted from the project's own verifier-honesty test
  (`docs/design/verifier-model-bounds.md`) by fiat, no stated justification.
- All 107 `meta.toml` files carry a dead `unwind = 10` field nothing reads — a stray bound that survived
  issue #278's own purge of prover-bounds from the CLI.

**Blame semantics: sound model, leaky surface.** The caller/callee blame model is well-specified. What
blocks unattended auto-fix:
- `cli.md:574`'s Agent Decision Tree says check the `inputs` field; the actual schema uses `values`. An
  agent following the documented tree looks for a key that was never there.
- The decision tree branches on four statuses; the status table defines five — `Skipped` (fail-closed,
  exit 1) has no prescribed agent action.
- `violating_args[].value` is sometimes `""` — a span with no value, so a caller-blame precondition
  failure can't always become a guard from the diagnostic alone.
- Blame casing is inconsistent across surfaces: `"Caller"` in one place, lowercase `"callee"` in another,
  `"Caller"` again at runtime.

## 3. Implementation reality: what each compiler actually enforces

Two claims from prior project notes turned out stale in the *good* direction. The numeric tower is no
longer "i64/u64 only": the full `Ty` enum plus a dedicated `LitInt` literal marker now runs end-to-end in
both compilers (landed 2026-07-05 and 2026-07-21), with real diagnostics and closed-by-default narrowing.
Aggregates are no longer wholesale ESBMC-skipped: struct field access/allocation became modelable under a
bump-allocator heap model when PR #887 merged 2026-07-10.

What's still missing, confirmed by direct probes compiled through both binaries rather than by grep:

| Check | Rust compiler | Self-hosted (`build/vowc`) |
|---|---|---|
| Match exhaustiveness | Detects, doesn't gate — prints `NonExhaustiveMatch`, but `ErrorCounter` (`check.rs:206`) never wraps the exhaustiveness pass, so the build still exits 0 and ships the executable | Absent — accepts a missing arm silently, 0 errors reported |
| Linear double-consume | Enforced — rejects with `LinearTypeViolation` | Absent — `v.push(h); v.push(h);` on a `linear struct` compiles and runs to exit 0. No `ConsumeState`/`LinearUse` concept exists anywhere in `compiler/*.vow` |
| Effect/purity violation | Enforced, correct code | Detected, wrong code — `checker.vow:2381` catches it, but line 2382 emits the generic error path, so agents parsing `error_code` see `TypeMismatch` instead of `EffectViolation`, blamed at the wrong span |
| Checked-arithmetic overflow (`+!` etc.) | Verifier no-op in both — `c_emitter.rs:870-887` emits `CheckedAdd` byte-identical to `WrappingAdd`; ESBMC is never asked for `--overflow-check` | (same) |
| Sanitize-mode use-after-free | Dead code in both — `freed` never set `true` anywhere; no codegen site emits a generation check | (same) |

One mechanism explains why three of these shipped silently in both compilers: `tests/error/` has zero
source-level cases tagged for `NonExhaustiveMatch` or `LinearTypeViolation`, despite the repo's own
`// TEST: error-code X` convention existing for exactly this purpose. The Rust unit tests for these passes
exist, but only at the AST level — they never exercise the actual build gate, which is precisely where
the Rust exit-0 bug lives.

Bootstrap health itself is good: the CLIF triple-test (Stage 0 → A → B → C) was re-run fresh and **B and
C are byte-identical** (sha256 `c2eddf94…`) — the fixed point genuinely holds. One hygiene note: the
`build/vowc` shipped in the main checkout is dated 11 days behind HEAD, predating the 2026-07-21
numeric-tower work — the primary compiler in daily use is not currently built from current source.

## 4. The flagship example doesn't verify what it's shown to verify

`examples/divide.vow` is the project's own worked example of a Caller-blame violation:

```vow
fn divide(x: i64, y: i64) -> i64 vow { requires: y != 0 } { x / y }
fn main() -> i32 [io] { divide(10, 0); 0 }
```

Run through the default verify path at HEAD:

```
$ vow verify examples/divide.vow
{"status":"Verified","executable":null,"diagnostics":[],"counterexamples":[]}
```

`main()` concretely calls `divide(10, 0)` against `requires: y != 0`. Static verify says `Verified`
anyway. Reading `vow/src/contracts.rs` and `vow-verify/src/c_emitter.rs` explains why: a verification
**target** is selected only from functions that carry their own `vow` block. When a contracted function
calls another contracted function, the callee's `requires` is emitted as an *assert* so the contracted
caller is blamed if it violates it — that mechanism is real and does work. But `main()` carries no
contract of its own, so it is never itself a verification target, and its calls into contracted functions
are structurally invisible to static verify.

Put plainly: `vowc build examples/divide.vow` — verify-by-default, per CLAUDE.md — reports `Verified` and
ships an executable that will only reveal the bug if someone happens to run it in `--mode debug`. This
also reconciles what looked like a contradiction between two of the parallel audits this cycle: the
caller-blame *mechanism* (issue #764) is genuinely merged and live for contracted-function-to-contracted-
function calls; the gap is a scope boundary — Vow verifies contracted functions, not whole programs —
that isn't stated anywhere a reader would find it before hitting it.

## 5. The CEGIS loop: a real mechanism, thinly and datedly evidenced

The retry loop itself is not vaporware. `bench/runner.py` genuinely loops up to 5 attempts, feeds a
curated counterexample back into the prompt (violation type, blame, values, span, hints, an anti-repeat
directive), and terminates cleanly. No stubs, no `TODO`, no `unimplemented!` anywhere in the loop, the
verifier wrapper, or the prompt curation.

A real, captured counterexample (self-hosted compiler, intentionally broken `bad_max`):

```json
{"status":"VerifyFailed","function":"bad_max",
 "counterexamples":[{"values":{"a":"-2","b":"-1",
   "_esbmc_v4":"0","_esbmc___ups_4":"0","_esbmc_v10":"0"},
 "violation":"ensures result >= a && result >= b",
 "vow_id":0,"blame":"callee"}]}
```

Actionable — function, contract, blame, and concrete falsifying inputs are all present — but noisy: raw
ESBMC SSA symbols leak into `values`, there's no value for `result`, and numbers arrive as strings. The
Rust compiler's output for the same input is strictly richer (structured span with offset/length,
populated `diagnostics[]`, hints) — meaning the *primary*, self-hosted compiler currently gives an agent
a weaker signal than the bootstrap compiler nobody runs day to day.

Current ground truth on the benchmark corpus: **80 of 103 non-stretch references verify clean** — zero
timeouts, zero OOM, all 23 failures are genuine. One, decomposed in full: `E01_absolute_value` fails on
`x = i64::MIN` — the verifier is *right*; the reference is missing the overflow guard CLAUDE.md itself
calls legitimate. The rest skew Vec-heavy, consistent with the long-standing nondet-Vec hypothesis,
unconfirmed case by case. (CLAUDE.md's own count — "40 benchmarks, 15/15/10, 36 pass" — is stale roughly
2.7×; the manifest is actually 107 benchmarks, 34/53/20 plus 4 stretch.)

The historical evidence backing the project's headline numbers is thinner than it looks: the full report
files were deleted 2026-05-20, leaving one surviving artifact — a 4.5-month-old HumanEval subset run (67
of today's 107 entries) scoring 66/67. Decomposed: of the 11 repairs that took more than one iteration,
only **3** were driven by a real counterexample; **16 of 25 failures across that entire run (64%) received
an empty counterexample** and were repaired by blind retry, not CEGIS. Spot-checks run today do produce
real counterexamples, suggesting real improvement since March — but that improvement has never been
measured by an actual benchmark run. No full 107-benchmark run with a live model exists anywhere in the
repo. A related measurement flaw: `classify_failure()` maps every `VerifyFailed` to a single
`"wrong_algorithm"` bucket — it can't distinguish a genuinely bad implementation from a verifier that
simply couldn't handle the shape of the problem.

## 6. The issue tracker measures an audit, not a backlog

**278 open issues, 0 open PRs.** 110 of those (40%) were filed on a single day — 2026-06-10, from an
internal audit doc — and 79% haven't been touched since creation. Sampling 8 of that batch found **6
already fixed in code and simply never closed.** Extrapolated, an estimated 40–55 of the 110 are
closeable today with zero engineering work — the single highest-leverage triage action available, and one
that should happen before any roadmap work, because right now the tracker has no reliable signal.

| Theme | Count |
|---|---|
| Verifier model / ESBMC / counterexample / blame | 59 |
| Checker soundness (contracts, linearity, effects, purity) | 39 |
| CLI / docs / spec-schema drift | 38 |
| Self-hosted parity | 29 |
| Compiler bugs (parser/printer/lexer/lowerer) | 28 |
| CI / build / release infra | 16 |
| Language features (numeric tower, BTreeMap, float, tuples) | 15 |
| Examples / stdlib / chess | 9 |
| Mutation testing | 6 |
| Performance + benchmarks | 8 |
| Other / deferred-from-PR polish | 32 |

**What actually blocks the dream (still open, cross-checked):**
- **#585** — checked-arithmetic overflow never modeled. Corroborated independently by three separate
  readings of the code. The most-confirmed finding in this entire audit.
- **#588** — self-hosted linear-consumption tracking absent — confirmed with a concrete, reproducible probe.
- **#589 / #656** — `where`-clause refinement predicates lower straight into `__ESBMC_assume` without
  ever being type-checked — a non-bool predicate becomes an arbitrary assumption.
- **#590** — `.unwrap()`'s panic-on-`None` obligation lowers to `ConstUnit`; an unwrap-on-`None` proves
  `Verified`.
- **#609** — callee `ensures`/`invariant` failures during a caller's verification are attributed to the
  wrong function — the agent would fix the wrong code.
- **#632** — free variables inside `MethodCall`/`Index`/`Cast`/`Match` get dropped from
  `VowViolation.values` — exactly the values a repair needs.
- **#467** — bounded quantifiers (`forall i in 0..n`) — the most-cited missing contract expressiveness
  gap, and genuinely high-value by the project's own criteria: it's expressiveness, not sugar.
- **#617** — mutation oracle has no unmutated-baseline check — and independently, no `mutants.out`
  artifact exists anywhere on the machine. The safety net for everything above has apparently never been
  fired to completion since the feature landed 2026-05-08.

**Corrections to the mechanical "already fixed, safe to close" list.** A naive close-sweep would get two
of these wrong:
- **#583** is fixed *in the Rust compiler* (fails closed with a clean error) — but the self-hosted
  compiler **crashes** (SIGABRT, `IndexOutOfBounds`) on the identical repro. Re-scope, don't close.
- **#661** was initially flagged as a live fail-open (verifier crash silently treated as pass). Closer
  reading of `compiler/verifier.vow:484-510` shows it falls through to an explicit error state — it's
  defense-in-depth against a missing exit-code cross-check, not a live hole. Downgrade.

Everything else flagged as already-fixed-but-open checked out: `#584`, `#637`, `#646`, `#676`, `#608`,
`#610/#615/#616`, `#595`, `#605`. `#397`, `#764`, `#855`, `#840`, `#848`, `#852`, `#853` are closed; PRs
`#887`, `#898`, `#776` are merged. Only `#366` and `#929` remain open and current.

Four defects surfaced *this cycle* aren't tracked anywhere yet: the Rust `ErrorCounter` not gating
exhaustiveness, self-hosted's total absence of exhaustiveness checking, the effect-violation misreported
as `TypeMismatch`, and the `main()`-calls-contracted-function static-verify blind spot. File these before
they're lost the same way the June-10 batch nearly was.

## 7. Engineering hygiene: the genuinely good news

| Check | Result |
|---|---|
| `scripts/full_test.sh` | 602 passed / 0 failed / 3 skipped (945s) |
| `cargo test --all` | 1235 passed / 0 failed / 2 ignored |
| `cargo clippy --all -- -D warnings` | Clean |
| Bootstrap fixed point (clif) | B == C, byte-identical |
| `check_help_coverage.py` / `generate_help.py --check` | In sync, exit 0 |

No regression against any historical baseline — the apparent conflict in old project notes between "195
passed" and "~398 tests" was two different snapshot dates, not a discrepancy; the only number that
matters across both, the fail count, is 0 then and 0 now.

## 8. Paths onward

**Now (cheap, days not weeks):**
1. Verify-and-close sweep over the June-10 batch — check each proposed closure against code first; the
   naive version mishandles #583 and #661.
2. Fix the shipped skill's self-contradiction (`examples.md` vs `contracts.md`) — it's actively teaching
   agents the exact anti-pattern the project forbids in writing.
3. Fix the Agent Decision Tree in `cli.md`: `inputs` → `values`, add the missing `Skipped` branch.
4. Route `checker.vow:2382` through `EC_EFFECT_VIOLATION` with the call-site span — one line, restores
   "structured output everywhere."
5. Rebuild `build/vowc` from HEAD — the shipped primary compiler is 11 days behind source.
6. Correct CLAUDE.md's own stale numbers: benchmark count (40→107), self-hosted module count (13→29),
   the "full parity" claim, the `feature-matrix.md` pointer (file no longer exists).
7. File issues for the four defects this audit found that aren't tracked yet.

**Next (real engineering, weeks):**
1. Implement checked-arithmetic overflow verification (#585) — the most-corroborated gap, and the one
   where the language's namesake safety feature currently proves nothing.
2. Port linear double-consume tracking and match exhaustiveness into the self-hosted checker — both
   silent today in the compiler actually used day to day.
3. Add `tests/error/` coverage for `NonExhaustiveMatch` and `LinearTypeViolation` specifically — their
   absence is the root mechanism that let both ship silently in both compilers.
4. Decide and implement the scope of static caller-checking — right now the docs' own flagship
   Caller-violation example is invisible to `vowc build`'s default verify path.
5. Run mutation testing to completion at least once, and commit the result — the safety net behind
   everything above has never fired.
6. Re-run the full, current 107-benchmark suite with a live model, and split `classify_failure()` into
   implementation-fault vs. verifier-incapacity — the evidence behind the project's headline claim is 4.5
   months old, covers 63% of today's suite, and was mostly blind retries.

**Strategic fork (a decision, not a bug fix):** The collection-bound problem in §1 is the deepest issue
this audit found, and it has no single obviously-correct fix:
1. **Disclose.** Keep the bounded model, but surface the bound wherever `proven` appears in output (e.g.
   a `proven_domain` field) and stop calling it "for all inputs" in the docs. Cheapest; doesn't fix the
   limitation, does stop misleading whoever reads the JSON.
2. **Build the escape hatch CLAUDE.md already mandates.** A real per-function "mark unverifiable, skip
   it" annotation, so an author of a genuinely unbounded contract (like the `gcd` example CLAUDE.md
   itself uses) has a documented way to comply with the authoring rule.
3. **Invest in genuinely unbounded reasoning over collections** — the same rigor already applied to
   nondet-bounded loops — so `proven` can mean what the docs say for the common case of a function that
   takes a `Vec`. Most expensive, and the only option that actually closes the gap between the pitch and
   the delivery.

Option 1 is close to table stakes regardless of which of 2 or 3 gets picked. A second, adjacent fork:
before more language-feature work, consider whether re-running the full benchmark suite with today's
compiler — replacing four-month-old, 63%-coverage, mostly-non-counterexample evidence — would do more to
de-risk the project's central thesis than any single new feature would.

---

*Methodology: six parallel general-purpose investigations (issue triage, design coherence,
verification-soundness ground truth, self-hosted bootstrap health, feature-matrix/test reality check,
CEGIS end-to-end smoke test), each independently reading code and running commands rather than trusting
prior notes, followed by one direct empirical reproduction to reconcile a cross-agent discrepancy (§4).
All read-only — no issues closed, no code changed, no commits made.*
