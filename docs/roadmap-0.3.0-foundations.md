# Vow 0.3.0 — Foundations Stabilization Plan ("Back on Track")

**Status:** Proposed
**Author:** drafted on branch `claude/agentic-language-foundations-y4m3T`
**Supersedes (scope only):** the open-ended feature tracks in `docs/roadmap.md`
for the duration of this release.

> `v0.2.0` is already tagged and released (2026-05-20). This plan targets the
> **next** release, **0.3.0**. The exact number is not the point: 0.3.0 is a
> **hardening release, not a feature release** — see §6.

---

## 1. Diagnosis: how we derailed

Vow set out to be a *verifiable-first language for agents*. The thesis is sound
and the proof points are real: a self-hosted verified fixed-point compiler, a
CEGIS loop, structured counterexamples with blame, and strong benchmark numbers.

What went wrong is **breadth outran depth**. Between `v0.2.0` and today the open
backlog grew to **162 issues**, and the most ambitious tracks in flight are all
*new surface area*:

- **Numeric tower** (#523–#529): u8/i8/i16/u16/u32/i32/i128/u128, floats,
  BigInt/Decimal/Rational — an entire new type-system axis.
- **BTreeMap Phase 2** (#350, #530–#540): non-i64 payloads, 11 open slices, every
  one of which touches the verifier model.
- **I/O honest types, live-programming, vow-perf** (#521, #488, #487): three more
  subsystems.

Meanwhile the **two load-bearing foundations are not solid**:

1. **Arena allocation** is the memory model for *every* Vow program, yet it has a
   cluster of open soundness/robustness bugs and its own verification harness is
   both unsound in one place and a 30 GB memory bomb in another.
2. **Contract verification** — the entire reason Vow exists — is frequently
   *pointless* (91% of proven contracts are `result >= 0` on constant functions,
   #81), *intractable* (ESBMC OOMs/timeouts, #516/#546), or *broken at the C
   model layer* (#505/#506 emit C that does not compile).

This violates the project's own first principle (CLAUDE.md): a feature ships only
if it *does not make verification harder* and *eliminates a class of agent bugs*.
The numeric tower and BTreeMap-with-arbitrary-V both fail the first clause **while
the verifier is still shaky**. We are widening the verification surface faster
than we are stabilizing it.

**The fix is a deliberate narrowing.** 0.3.0 freezes net-new language/verifier
surface and spends the entire release making the two foundations *boringly
reliable*. Everything else waits.

---

## 2. Problem statement A — Arena allocation

### 2.1 What exists

- Runtime: `vow-runtime/src/lib.rs` (~5000 lines) — `VowArena`, chunk-linked bump
  allocator, oversized single-resident chunks, `try_extend`, O(chunks) close walk.
- Region inference: `vow-ir/src/region.rs` (**7993 lines**) and the self-hosted
  twin `compiler/region.vow` (**5113 lines**) — a monotone fixed-point escape
  analysis assigning every `RegionAlloc` a `RegionId` (Block / Caller(slot) /
  Root / Rodata).
- Verification harness: `vow-runtime/verify/arena.c` (~325 lines) + a 10-line
  `Makefile`.
- Normative spec: `docs/design/arena_memory.md` (~2060 lines, §10 covers ESBMC).

### 2.2 Root-cause themes (from the open issues)

| Theme | Evidence | Issues |
|---|---|---|
| **Harness is unsound** — wrapped-pointer bound; alignment never asserted; oversized-classifier edges unchecked; addr-cap should be `<` not `<=` | `arena.c` `alloc_chunk`/`align_up` assumptions | #437, #430, #422–#426, #480 |
| **Harness is intractable** — `--incremental-bmc --max-k-step 10` = 20 independent BMC runs, fresh symex+SMT per k, symbolic malloc in over-unwound loops → **~30 GB RSS / ~1.5 h**, OOM under the 2 GB CI ulimit | `verify/Makefile`, `full_test.sh` | #516, #546 |
| **RegionAlloc is unverifiable** — any vowed function whose body emits `RegionAlloc` (e.g. a functional struct update) is `SkippedNonModelable`, so bootstrap can't reach overall `Verified` without a script tolerance | `c_emitter.rs` `first_unsupported_opcode` | #397 |
| **Region inference is fragile & huge** — 8 k-line pass with a lattice fixed-point, alias-aware LUB, root-escape rewrite, AMBIGUOUS conflict gating, and a Vec-indexed lookup-table perf workaround; a steady drip of corner-case fixes (#400/#407, #372, #366–#368, #351) | `region.rs` / `region.vow` | #407, #372, #366–#368, #351 |
| **try_extend contract gaps** — frame condition / no-mutation / success postconditions claimed but not actually asserted; reserve arithmetic can wrap | `arena.c`, runtime | #431, #432, #433, #435 |

### 2.3 The core tension

The region pass is the single most complex thing in the compiler (13 k lines
across two implementations), it is *not* itself verified, and it produces an
opcode the verifier *refuses to model*. So the memory model that underpins every
program is trusted, not proven, and it keeps springing leaks. That is the exact
opposite of "verifiable first."

---

## 3. Problem statement B — Contract verification tractability

### 3.1 What exists

- C emitter: `vow-verify/src/c_emitter.rs` (**5813 lines**) + `compiler/c_emitter.vow`
  (2471 lines). Models `Vec`→`int64_t[128]`, `String`→`int8_t[256]`,
  `HashMap`→64-entry parallel arrays; `requires`→`__ESBMC_assume`,
  `ensures`/`invariant`→`__ESBMC_assert`.
- ESBMC driver: `vow-verify/src/esbmc.rs` (1957 lines) + `solver_strategy.rs`
  (902 lines) + `compiler/verifier.vow` (1020 lines). Invokes
  `esbmc <file> --no-bounds-check --no-pointer-check --incremental-bmc
  --max-k-step <N> --64` with a solver/`--ir` fallback ladder and a timeout.

### 3.2 Root-cause themes

| Theme | Evidence | Issues |
|---|---|---|
| **Contracts are often vacuous** — 354/388 proven contracts are `ensures result >= 0` on constant-returning tag functions; silent-fallback dispatchers (`tok_to_binop`, `ir_ty_to_c`) have no totality contracts. No methodology, no taxonomy, no contract-quality metric | self-hosted audit | #81 |
| **The C model emits invalid C** — `return value;` in a `void` function; `int64_t = __vow_vec_t` type mismatch. ESBMC reports `FAILED` for non-existent contract violations | SAT example | #505, #506 |
| **ESBMC is asked for too much** — `--incremental-bmc --max-k-step N` runs 2N independent verifications sharing no work; pointer-heavy arena C is the worst case; OOM/timeout under realistic limits | see §2.2 | #516, #546 |
| **Verifier-only bounds leak into contracts** — `--vec-max/--string-max/--hashmap-max/--btreemap-max` capacity flags exist as global knobs; fixtures smuggle `requires: n <= 8` (an unwind bound) into *contracts*, violating CLAUDE.md | spec + fixtures | #278, #552 |
| **Real overflow corners missed** — `lit_var` ensures `result > 0` but `-i64::MIN` wraps negative; the contract is genuinely wrong, not an ESBMC artifact | SAT example | #504 |
| **Expressiveness gaps** — no bounded quantifiers (`forall i in 0..n`), no `decreases` for recursion; both would *raise* contract value without a new type axis | roadmap | #467, #470 |
| **Status discipline** — soft `UNKNOWN`/`TIMEOUT`/`Skipped` vs hard `FAILED`; caching trust boundary; differential testing of model vs runtime | — | #334, #335, #337, #338, #376/#377, #385/#386 |

### 3.3 Should we drop C and target ESBMC IR directly?

**No — and this is already settled.** `docs/investigations/esbmc-ir-targeting.md`
evaluated three approaches:

- **A. Emit GOTO binary directly** — undocumented, version-brittle format;
  goto-transcoder was *archived*. High risk. Rejected.
- **B. Custom C++ ESBMC frontend** — the "right" long-term architecture but ESBMC
  ships no stable linkable API; premature without ESBMC-team buy-in. Rejected for now.
- **C. Keep C, make it smarter** — exploit ESBMC flags, fix the collection
  models, tighten counterexample mapping. **Recommended.**

So the C path stays. The user's instinct ("maybe we shouldn't go through C") is
worth respecting as a *long-term* option (Approach B, gated on an upstream API),
but **the near-term wins are all inside the C emitter and the ESBMC invocation**,
not in a rewrite. The bugs in #505/#506 are emitter bugs, not C-vs-IR architecture
problems — going to IR would not have prevented them, only moved them.

---

## 4. The attack plan — five workstreams

Ordered by dependency. WS-0 and WS-1 are quick, high-leverage, and unblock the
rest. WS-2/WS-3 are the deep work. WS-4 is the policy that keeps us honest.

### WS-0 — Stop the bleeding (days, not weeks)

Cheap fixes that immediately make the foundations measurable and CI green.

1. **Switch the arena harness off the memory bomb.** Replace
   `--incremental-bmc --max-k-step 10` in `vow-runtime/verify/Makefile` with the
   single-shot bound from #516: `--unwind 5 --no-bounds-check --no-pointer-check
   --64 --bitwuzla`. Expected: ~30 GB → a few hundred MB, hours → seconds. (#516, #546)
2. **Apply the same single-shot strategy to the main pipeline** where loop counts
   are statically small, behind a measured A/B (see WS-3.1). Reconcile the docs:
   `contracts.md` claims "k-induction-parallel" but the code runs plain
   `--incremental-bmc`. Pick one, document the truth. (#516)
3. **Close the already-fixed harness issues.** #437's recommended non-wrapping
   bound (`base_addr <= ADDR_CAP - total`) is *already in* `arena.c` (PR #447);
   verify and close #437, and resolve #480 (`<=` → `<`) in the same touch.
4. **Align the memory budget with reality.** ESBMC is already invoked with
   `--memlimit 4096m` (`solver_strategy.rs`), but CI caps virtual memory at **2 GB**
   (#546). ESBMC therefore believes it may use 4 GB and gets OOM-killed by the
   cgroup before its own soft limit fires. Drive `--memlimit` from the active
   ulimit so a too-large proof fails *soft* (→ `UNKNOWN`) instead of being killed.
5. **Capture verifier C for debugging.** Wire `VOW_VERIFY_DEBUG` / `--keep-verify-tmp`
   into the self-hosted verifier (`compiler/verifier.vow:310` TODO) so #505/#506
   are reproducible. Prerequisite for WS-2.
6. **Fix the two known contract bugs** that are pure correctness, not strategy:
   `lit_var`/`lit_bucket` i64::MIN overflow guard (#504).

**Exit:** `full_test.sh` arena/esbmc section passes under a 2 GB ulimit; #437,
#480, #504, #516, #546 closed or downgraded.

### WS-1 — Make verification *honest* (the C emitter)

The verifier must never report `FAILED` for a contract that wasn't violated.

1. **Fix the malformed-C bugs** #505 (`int64_t = __vow_vec_t`) and #506
   (`return` in `void`). Add a regression fixture per bug under `tests/verify/`.
2. **Audit the C emitter for every collection-payload path.** The #505 class
   (losing the `i64` payload type through a `Vec` index) is likely systemic, not a
   one-off; sweep all `EXPR_INDEX` / field-as-container lowerings in both emitters.
3. **Differential harness** (#335): a small suite that runs the *same* program
   through the runtime and through the verifier model and asserts agreement, so the
   model can't silently diverge again. Seed it from the SAT example.

**Exit:** `examples/sat/main.vow` verifies end-to-end (or is honestly `Skipped`,
never falsely `FAILED`); a differential test guards model/runtime parity.

### WS-2 — Make RegionAlloc verifiable (the deep arena fix)

This is the keystone. Today the memory model is *trusted*; the goal is to make it
*proven* — or at least to make verification not silently surrender on it.

1. **Decide the modeling strategy for `RegionAlloc`** (#397). Two viable routes,
   pick one with a short ADR:
   - **(a) Model it as a havoc-allocation** in the C emitter: a `RegionAlloc`
     becomes a fresh nondeterministic-but-distinct pointer with an
     `__ESBMC_assume` of non-aliasing. Contracts that don't depend on the
     allocator (the common case, e.g. `ir_inst_set_region`'s `requires: rgn >= 0`)
     then verify normally. Lowest risk; keeps the arena itself proven once by
     `arena.c` and treats it as a trusted axiom elsewhere.
   - **(b) Prove the in-place-mutation rewrite** so functional struct updates that
     today emit `RegionAlloc` instead emit field stores (#397 option 2), removing
     the opcode from contract-bearing functions entirely.
   Recommendation: ship **(a)** for 0.3.0 (unblocks bootstrap `Verified`), keep
   (b) as an optimization.
2. **Make `arena.c` a *complete* and *sound* axiomatization** of the invariants
   the rest of the pipeline assumes: alignment of returned pointers (#430),
   `try_extend` frame/no-mutation/success (#431/#432/#433), reserve-arithmetic
   no-wrap (#435), and the oversized-classifier edges (#422–#426). Each becomes an
   asserted property, not a comment.
3. **Bootstrap reaches `"status":"Verified"` with no script tolerance** (#397
   acceptance criterion). This is the headline exit gate for the arena track.
4. **Containment for the region pass.** We are *not* rewriting `region.rs`/
   `region.vow` in 0.3.0 (too risky), but we (i) freeze new region features,
   (ii) land the in-flight soundness fixes (#372, #407, #366–#368), and (iii) add
   the BTreeMap-backed lookup table (#351) only if it reduces, not grows, surface.

**Exit:** no vowed function in `compiler/*.vow` is `SkippedNonModelable`;
`scripts/bootstrap.sh` returns `Verified`; arena invariants in §2.2 are all
*asserted* in `arena.c` and proved in seconds.

### WS-3 — Make verification *tractable and meaningful*

Two halves: performance, and contract quality.

1. **Performance: tune the existing ESBMC strategy per function** (#516). A solver
   fallback ladder already exists (`solver_strategy.rs`: classify → BV via
   Boolector/Bitwuzla → on timeout/memlimit retry `--ir --z3`), as does a
   `--memlimit` default and a per-function solver classifier. The work is to
   *measure and tune it*, not build it: benchmark `--incremental-bmc --max-k-step N`
   vs single-shot `--unwind k` vs `--k-induction` across the fixtures + arena,
   recording peak RSS and wall time, and default to the cheapest that proves the
   corpus. Add `--smt-during-symex` where it lowers RSS. Wire `--memlimit` to the
   ulimit (WS-0.4) so runaway proofs fail soft (follow-up to the merged #502).
2. **Contract methodology** (#81) — the highest-value, lowest-code item. Produce
   `docs/spec/contracts-methodology.md` defining a **contract taxonomy** (range,
   uniqueness, round-trip, dispatch-totality, relational) with a worked example of
   each *and* a verdict on expressibility today. Then harden a small set of real
   compiler contracts (pack/unpack round-trips, dispatcher totality) as proof of
   concept, replacing volume with substance. Add a contract-quality signal to
   `vow contracts --verify` that flags trivially-true contracts.
3. **Expressiveness, surgically.** Land **bounded quantifiers** `forall i in
   0..n: P(i)` (#467) — it desugars to an unrolled assert loop, adds no type-system
   axis, and directly enables dispatch-totality and Vec contracts from the
   taxonomy. Defer `decreases` (#470) to a post-0.3.0 ADR.
4. **Stop leaking verifier bounds into contracts.** Audit and reshape the
   `tests/verify/` fixtures that carry `requires: n <= 8` (#552); design the
   removal of the `--vec-max/--string-max/...` global flags in favor of
   per-function inference or literal-derived bounds (#278) — spec-and-plan in
   0.3.0, implement may slip.

**Exit:** the fixture corpus + arena verify under a fixed memory budget
(target: < 2 GB peak, no single function > the timeout); a published
contract methodology; bounded quantifiers shipped; no contract in the repo
contains an unwind bound.

### WS-4 — Guardrails so we don't re-derail

Policy, encoded where possible as CI checks (see §5).

---

## 5. Engineering guidelines for 0.3.0

These are binding for the duration of the release.

1. **Feature freeze on net-new verification surface.** No work that *adds* types,
   payload kinds, or collection models lands until WS-0–WS-3 exit. Concretely,
   the **numeric tower (#523–#529)** and **BTreeMap non-i64 V (#350, #530–#540)**
   are *paused*, not cancelled. They resume in 0.4.0 on a hardened verifier.
   (This is just CLAUDE.md's three-criteria rule applied honestly.)
2. **No contract may contain a verifier bound.** `requires: n <= 8` to fit
   `--unwind` is forbidden. If ESBMC can't prove the true contract, mark the
   function unverifiable — never weaken the spec. Add `check_help_coverage.py`-style
   linting that flags suspiciously round literal bounds in `requires`/`ensures`.
3. **No false `FAILED`.** A verification result of `FAILED` must correspond to a
   real contract violation with a counterexample. Malformed-model failures are
   `bug`s, not `FAILED`s. The differential harness (WS-1.3) enforces this.
4. **Both compilers, same session.** Per CLAUDE.md: every verifier/region change
   lands in *both* `vow-*` and `compiler/*.vow`, with `cargo test` + bootstrap
   triple green, in the same PR.
5. **Surgical PRs.** One soundness fix per PR; no bundled refactors. The arena and
   verifier files are already too big — every change should *reduce* or hold
   surface area, never grow it. (region.rs at ~8 k lines is a standing liability;
   note it, don't expand it.)
6. **Spec is the source of truth.** Any change to ESBMC flags, collection models,
   or contract syntax updates `docs/spec/contracts.md` / `cli.md` in the same PR,
   and regenerates `--help` + skill. The current `contracts.md`/code mismatch on
   the verification *strategy* is itself a bug to fix in WS-0.
7. **Measure memory in CI.** `full_test.sh` already characterizes peak RSS (#546);
   add a regression gate so a change that pushes arena/verify back over budget
   fails CI.

---

## 6. Definition of 0.3.0 ("back on track")

0.3.0 ships when **all** of the following hold:

- [ ] `scripts/bootstrap.sh` returns `"status":"Verified"` with **no** tolerance
      for `SkippedNonModelable` (#397).
- [ ] `full_test.sh` passes end-to-end under a **2 GB** ulimit, including the
      arena verify section (#546).
- [ ] Arena harness `arena.c` *asserts* (not comments) every invariant in §2.2;
      runs in seconds; #437/#480/#430/#431/#432/#433/#435/#422–#426 closed.
- [ ] The verifier never emits invalid C; #505/#506 closed; differential
      model/runtime harness in CI (#335).
- [ ] `docs/spec/contracts-methodology.md` published; the trivial-contract count on
      the self-hosted compiler is materially reduced and a contract-quality signal
      exists (#81).
- [ ] Bounded quantifiers shipped (#467); zero unwind-bounds-as-contracts in the
      repo (#552).
- [ ] No regression in benchmark pass rate (the existing 99% combined number is a
      floor, not a target to chase).

Explicit **non-goals** for 0.3.0: numeric tower, arbitrary BTreeMap values,
live-programming, vow-perf, the ESBMC IR/GOTO rewrite (Approach B). All deferred
to 0.4.0+ and tracked in `docs/roadmap.md`.

---

## 7. Sequencing

```
Week 0   WS-0  stop-the-bleeding (harness flags, close already-fixed, #504, debug hook)
Week 1   WS-1  honest C emitter (#505/#506 + sweep + differential seed)
Week 1-3 WS-2  RegionAlloc modeling ADR + arena.c axiomatization + bootstrap Verified
Week 2-4 WS-3a perf strategy benchmark + memory budget gate
Week 2-4 WS-3b contract methodology + harden + bounded quantifiers
Week 4   WS-4  guardrail CI checks; reconcile spec/code; cut 0.3.0
```

WS-2 is the critical path. WS-1 and WS-3a can run in parallel against it. The
feature freeze (WS-4.1) starts on day 0.

---

## 8. Issue triage — foundational vs deferred

**Foundational (this release):**
Arena: #397, #437, #480, #430, #431, #432, #433, #435, #422, #423, #424, #425,
#426, #516, #546, #372, #407, #366, #367, #368, #351.
Verification: #81, #505, #506, #504, #552, #278, #467, #334, #335, #337, #338,
#376, #377, #385, #386.

**Deferred to 0.4.0+ (paused, not cancelled):**
Numeric tower #523, #524, #525, #526, #527, #528, #529. BTreeMap-V #350,
#530–#540. Live-programming #488. vow-perf #487, #486, #482, #478, #484.
I/O wave #521. ESBMC IR rewrite (Approach B, no issue yet — file post-0.3.0).

**Hygiene / parallel-safe (land opportunistically, don't gate the release):**
the `deferred-from-pr` and `ergonomics` clusters (#553–#559, #364, etc.),
CI items (#393/#399/#497), docs items.

> Recommended next administrative step (not yet done — needs owner sign-off):
> create a **`0.3.0-foundations` milestone** and attach the Foundational set above,
> and add a `paused-0.4.0` label to the deferred set. I can do this on request.
