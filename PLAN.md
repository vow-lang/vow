# Plan: #1419 perf(verify): hash-consed term arena, simplifier and guard pruning

## Goal
Make the native verifier's term arena hash-consed with cached hashes, make every term constructor
simplify and constant-fold, name only composite values (with CSE), and drop claims the simplifier
already decided, so an unrolled loop yields a smaller query set (fewer define-funs, fewer claims, fewer
solver spawns) with identical verdicts. Native verifier only (`compiler/vc_*.vow`); no Rust twin
(ADR-2026-10-08-1421 scoped exception).

## Findings (baseline, read from the tree at 8da98607)
- `compiler/vc_term.vow:58-146`: arena is parallel `Vec`s, no dedup, no simplification; only `vc_tm_and/or/implies` fold the constant `true`.
- `compiler/vc_exec.vow:143-149` `vc_define`: **every** IR value gets a `define-fun vN` and `var_of[id]` is a fresh VAR, so constants and leaves are named and never seen through. `i = 0; i + 1; i < 8` in an unrolled loop is never folded, so no iteration guard is ever "decided" and every iteration copy emits claims.
- `compiler/vc_exec.vow:223-233` `vc_branch_outcome` already prunes on a constant `def_of` and on facts keyed by **IR value id** (`vc_flow_fact_lit(cond_id, ..)`, `vc_flow.vow:94`); identical conditions computed twice by distinct IR values do not share a fact.
- `compiler/vc_exec.vow:89-101` `vc_add_claim`/`vc_assume`: every claim (even goal `true`) becomes one solver process (`vc_native.vow:85-115`); `vc_script_prefix` (`vc_smt.vow:44`) copies the header per claim.
- Existing tests pin exact SMT text (`compiler/tests/test_vc_exec.vow:95-126`, `:70-86`, ...) and wiring fixtures use `ensures: result == result` (`tests/verify-native/tests.sh` THREE_CLAIMS, `stress_source`): both break once trivial claims fold away. This is expected churn, listed below.

## Assumptions (decided without an operator)
- Hash table: `HashMap<i64, i64>` (#1403, precedent `compiler/clif.vow:29-110`) from the node hash to the first term id with that hash, plus a `chain: Vec<i64>` next-pointer vector for collisions and a cached `hashes: Vec<i64>`. The map is only probed, never iterated, so output stays deterministic. The field-mixing function is a hand-written wrapping mixer, **not** `hash_u64`/`hash_str` (a function calling those is reported skipped by the verifier). The ESBMC collection-model capacity applies only to the verification of the compiler's own functions (a `ModelCapacityAssumed` note, same as `clif.vow`), not to runtime.
- `VAR` terms are not interned by name (each `vc_tm_var` call is a distinct leaf; callers hold the id). The lowerer emits exactly one `GET_ARG` per parameter in the prologue (`compiler/lower.vow:5879`) and unrolling only clones loop blocks, so `eq(x,x)`, CSE and facts still see one term per parameter. Rules never need a variable's sort.
- Constant folding is limited to widths <= 64. 128-bit constants stay unfolded (only width-independent rules apply). Follow-up.
- Dividing/remainder folding only when the divisor is non-zero and not signed `MIN / -1`; shifts only when the constant count `< width`; `mul` overflow predicates only for width <= 32 or an operand in {0, 1}. Anything else stays symbolic (sound by construction: unfolded = today's behavior).
- Claims whose negated goal folds to the constant `false` are not recorded (unsat under any assumptions; verdict-identical). A negated goal that folds to `true` is statically `sat` **only when `assumed_lens[k] == 0`** (no earlier assumption could be contradictory); otherwise it still goes to the solver, because contradictory `requires` make it `unsat` today. This makes the simplifier part of the trusted base; recorded in a short ADR.
- No cone-of-influence slicing, no join-pc rule, no interval reasoning (see Out of scope). One PR, ordered commits S0-S6.
- No `docs/spec/*.md`, `--help`, or skill change: no language/CLI surface changes, verdicts are unchanged.

## Files to touch
| File | Role | Lines |
|------|------|-------|
| `compiler/vc_term.vow` | arena: add hash-cons map + chain, canonical constants, simplifying constructors | 58-146 |
| `compiler/vc_bvfold.vow` [new] | pure width<=64 two's-complement fold helpers (no dependency on vc_term) | - |
| `compiler/vc_exec.vow` | `vc_define` naming policy + CSE, `vc_add_claim`/`vc_assume`/`vc_guard` elision, branch outcome | 89-149, 223-259 |
| `compiler/vc_flow.vow` | `vc_flow_name` CSE, term-keyed facts | 63-72, 94-96 |
| `compiler/vc_native.vow` | static-sat for UNWIND/ARITH claims in `vc_run_round` | 85-115 |
| `compiler/vc_agg.vow` | `vc_agg_declare`/`vc_agg_get` use constructors; confirm no `v`-name dependence | 130-190 |
| `scripts/concat_vow.sh` | add `vc_bvfold` before `vc_term` in the `clif` FILES list (line 23) | 23 |
| `compiler/tests/test_vc_term.vow` [new] | intern, fold, differential-evaluator tests | - |
| `compiler/tests/test_vc_bvfold.vow` [new] | exhaustive width-8 fold tests vs independent reference | - |
| `compiler/tests/test_vc_exec.vow` | goldens, new unrolled-loop metrics, claim-elision tests | 70-130, 291-343, 376-435, 655-735 |
| `compiler/tests/test_vc_agg.vow`, `test_vc_smt.vow`, `test_vc_cex.vow` | golden / arena-API updates | - |
| `tests/verify-native/tests.sh` | non-trivial wiring contracts, new memory-cap + query-count checks | 205-260, 405-500, 358-385 |
| `tests/verify-native/pass/`, `fail/` | new loop fixtures | - |
| `docs/adr/2026-10-10-HHMM-term-simplifier-trusted-base.md` [new] | trust-base decision (timestamp at authoring, UTC) | - |

## TDD slices (ordered; each commit leaves `build/vowc test compiler/` green)

### S0. Red metric tests on an unrolled loop (no production change)
- **File**: `compiler/tests/test_vc_exec.vow` (reuse `count_up_with_invariant` :655, `vc_unroll`, `query`, `count_of`).
- **Add** `counted_loop(bound_lit, symbolic)`-style builders: (a) literal bound 8 with `i = i + 1` and invariant `i <= 8` (as `count_up_with_invariant`, `check_unrolled_loop_claims` :693), unrolled at bound 8; (b) symbolic bound `n` param unrolled at 16.
- **Assert (red now)**: (a) `vc_claims(g).kinds.len() == 0` and `count_of(render(cs.header), "(define-fun v") == 0`; (b) define-fun count <= 70% of the baseline literal recorded in the test (measure baseline on current tree first, write it as a constant with a comment of the measured value) and claim count == original minus decided invariants.
- Also record the baseline numbers (define-funs, bytes, claims) in the PR body from a `build/vowc` built off `origin/main`.

### S1. Hash-consing with cached hashes
- **File**: `compiler/vc_term.vow` (`VcArena` :58, `vc_arena_new` :67, `vc_arena_push` :78).
- **Change**: add `hashes: Vec<i64>` (cached node hash, indexed by term id), `head: HashMap<i64, i64>` (hash -> first term id), `chain: Vec<i64>` (next term id with the same hash, -1), `named: Vec<i64>` (term -> var term that already names it, -1; used in S3). Hash = wrapping mix of `(kind, a, b, c, aux)` using an odd multiplier < 2^63 (e.g. `0x2545F4914F6CDD1D`) and u64 shifts; children are canonical ids so the hash is O(1) and never re-walks a subterm. Lookup walks the chain comparing the five fields.
- Keep `vc_arena_push` as the raw (non-dedup) constructor for tests; add `vc_arena_intern` used by every non-VAR constructor. `vc_tm_var` stays a fresh leaf (`strs` push).
- Canonicalise constants: `vc_tm_bv` masks `lo` to the width for w < 64, zeroes `hi` for w <= 64 (writer already ignores high bits, so output is unchanged), so `#xffffffff` from a sign-extended -1 interns once.
- **Tests** (`test_vc_term.vow` [new]): same `(kind,a,b,c,aux)` -> same id; different -> different; 2000+ terms (map growth) still hit; a forced hash collision (two distinct nodes with equal hash via the raw push path) resolves through the chain; `vc_tm_bv(32, -1, 0)` and `vc_tm_bv(32, 0xffffffff, 0)` are one id; vars never merge; determinism (build twice, ids identical).

### S2. Simplifying constructors + pure fold helpers
- **Files**: `compiler/vc_bvfold.vow` [new], `compiler/vc_term.vow` (`vc_tm_app1/2/3` :104-114, `vc_tm_and/or/implies` :126-140, `vc_tm_ext` :144), `scripts/concat_vow.sh:23`.
- **vc_bvfold** (names prefixed `vc_bvfold_`, unique across the concatenated namespace): `mask(w,x)`, `sext(w,x)`, add/sub/mul/and/or/xor, ult/ule, slt/sle, eq, shl/lshr/ashr (count < w), udiv/urem/sdiv/srem (guarded domain above), sadd/uadd/ssub/usub overflow for w <= 64, smul/umul overflow for w <= 32. Contracts only where they state the real domain (e.g. `1 <= w <= 64`); never a verifier bound.
- **vc_term smart constructors** keep their current names/signatures (no call-site churn in vc_int/vc_exec/vc_flow/vc_agg). Rules, all applied before interning:
  - const op const (w <= 64) -> const; ext of const with result width <= 64 -> const; `zero_extend 0`/`sign_extend 0` -> x (the amount, not the operand width, decides). No rule may need the width of a non-constant operand: the arena stores no sort for VAR/APP terms, so the only widths available are those carried by a BV constant (e.g. `x & allones` reads the width off the all-ones constant).
  - Bool: `not(not x)=x`, `not const`, `and/or` with const / `a==b` / `x and not x`, `ite(const,a,b)`, `ite(c,a,a)`, `ite(c,true,false)=c`, `ite(c,false,true)=not c`, `eq/distinct` of Bool constants.
  - Width-free identities returning an existing operand: `x+0, 0+x, x-0, x*1, 1*x, x|0, x^0, x&allones, x*0 -> the 0 operand, x&0 -> the 0 operand, shift by const 0`; `cmp(x,x)`, `eq(x,x)=true`, `distinct(x,x)=false`.
  - `vc_tm_is_true/false` unchanged (kind/aux check).
- **Tests**: `test_vc_bvfold.vow` exhaustive over all width-8 pairs x every op vs an independent straightforward reference (explicit `% 256` math), plus width 16/32/64 edge values (0, 1, MIN, MAX, -1); `test_vc_term.vow` differential test: random expression DAGs (xorshift PRNG, fixed seed) over 8-bit vars and constants built twice, once with raw `vc_arena_push` (no simplification) and once with the smart constructors, both evaluated by a small SMT-LIB-semantics evaluator under random environments; results must be equal. Include SMT total-division-by-zero semantics in the evaluator so unfolded div/rem is covered.

### S3. Name only composites, with CSE
- **Files**: `compiler/vc_exec.vow` (`vc_define` :143, `vc_value_name` :77), `compiler/vc_flow.vow` (`vc_flow_name` :63), `compiler/vc_term.vow` (`named`).
- **Change**: `vc_define(w, inst, t)`: if `t` is a leaf (VAR/BV/BOOL) set `var_of[id] = t`, emit no `define-fun`; if composite and `ar.named[t] >= 0` reuse that var term, emit nothing; else emit `define-fun vN`, create the VAR, set `ar.named[t]`. `vc_flow_name` consults/sets `named` the same way. `def_of` stays populated (a leaf `t` is stored in both tables) so `vc_branch_outcome` still compiles; S4 retargets it to `var_of` and removes `def_of`.
- **Soundness note**: header is append-only and every claim prefix contains all earlier definitions, so reusing a name defined on another path is valid (definitions are total functions of declared constants).
- **Tests**: update goldens in `test_vc_exec.vow` (:95-126 expects `v1 = #x0`; becomes inlined literal, no `v0/v1` defines), `test_vc_agg.vow` (:309-330, :381, :418), `test_vc_smt.vow` if constants fold; add `check_cse_names_identical_terms_once` (two IR values `x + y` -> one define-fun) and `check_constants_and_params_are_not_named`.

### S4. Guard pruning on terms
- **Files**: `compiler/vc_exec.vow` (`vc_branch_outcome` :223, `vc_branch_edge` :239), `compiler/vc_flow.vow` (`vc_flow_fact_lit` :94 callers).
- **Change**: (a) decided when `var_of[cond]` is the Bool constant (replaces `def_of`, which is deleted here with its stores in `vc_define`/`VcWalk`/`vc_claims`); (b) key facts by **term id** of the condition (`var_of[cond_id]`), so two IR values with CSE'd identical terms share the outcome; (c) when the condition term is `not(x)` look up `x` with inverted polarity (cheap, sound at term level). `vc_flow_extend_facts`/`VC_FACT_CAP` unchanged.
- **Tests**: `check_constant_guard_prunes_dead_arm` (:291) and `check_repeated_guard_prunes_inner_else` (:314) keep their intent but assert on a claim whose goal is not constant (they currently use constant ensures that S5 elides); add `check_negated_guard_decided_by_fact` and `check_duplicate_condition_values_share_outcome` (two distinct IR compares of the same operands, nested).

### S5. Claim elision (decided claims never reach the solver)
- **Files**: `compiler/vc_exec.vow` (`vc_add_claim` :89, `vc_assume` :103, `vc_guard` :109, `VcClaims` :59), `compiler/vc_native.vow` (`vc_run_round` :85).
- **Change**: in `vc_add_claim`, after `negated = and(pc, not goal)`: if it is the constant `false` record nothing. In `vc_assume`: skip a constant-`true` implication. Duplicate elision: keep a `seen` set of implication term ids (hash-consed, so equal id = equal formula); a claim whose `implies(pc, goal)` id is already assumed is `unsat` against the assumptions and is not recorded (still add nothing twice). In `vc_run_round`: a UNWIND or ARITH claim whose negated goal is the constant `true` **and whose `cs.assumed_lens[k] == 0`** is statically `sat` (the header holds only `declare-const` and definitional `define-fun`, so it is satisfiable; no model needed): mark `open` / record the site without a solver run. With any earlier assumption the claim still goes to the solver, since contradictory assumptions make it `unsat` today (it would otherwise turn `proven` into `unknown`, or add a spurious `ArithOverflowReachable` site). Hard claims always go to the solver (the counterexample needs model values).
- **Preserved invariants**: `vc_run_round` already proves without locating a solver when `cs.kinds.len() == 0` (`vc_native.vow:88-90`); `vc_claim_result`/`vc_has_site` untouched.
- **Tests**: `check_true_goal_claim_is_elided`, `check_duplicate_guard_claim_elided` (same `x / y` twice -> one DIV_ZERO claim), `check_second_division_assumes_first` (:142) stays green, `check_unrolled_loop_claims` (:693) updated: at bound 2 with literal bound 8 the three invariant claims fold to `true` -> 0 ENSURES, 1 UNWIND, and the UNWIND negated goal is the constant `true` (`assumed_lens == 0`); add `check_static_sat_needs_no_assumptions` (same loop behind a `requires` keeps the claim for the solver); S0 red tests go green.

### S6. Fixtures, wiring tests, ADR
- **`tests/verify-native/tests.sh`**: replace `ensures: result == result` in THREE_CLAIMS (`quot`) and `stress_source` with a true contract the simplifier does not decide (e.g. `result * b + a % b == a`; `result == x + (result - x)` for `bump`); keep the claim/query counts those tests expect (3 queries, 1 query, ite count == n); recount THREE_CLAIMS by hand: `a % b` brings its own DIV_ZERO guard, a duplicate of `quot`'s divisor guard elided by S5, so choose a contract with no division of its own that CSE cannot reduce to the body (`result == a / b` would fold to `true` and vanish). Add a case: a literal-bound loop with **no `requires`** and the fake solver in `unsat` mode asserts `queries == 0` and status `Verified` (rounds before the covering one send only statically-`sat` UNWIND claims with `assumed_lens == 0`; the covering round has no live sink). A variant with a `requires` asserts the UNWIND claim still reaches the solver (`queries >= 1`).
- **New fixtures**: `tests/verify-native/pass/loop_literal_bound_32.vow` (32 iterations, accumulating checked add + invariant, no `requires`; needs no solver after folding), `tests/verify-native/pass/loop_repeated_division_guard.vow` (loop-invariant divisor, one DIV_ZERO claim), `tests/verify-native/fail/loop_literal_bound_wrong_post.vow` (literal-bound loop, wrong ensures: counterexample must still be produced). Existing `loop_literal_bound`, `loop_bug_at_iteration_40`, `loop_literal_100` (unknown) must keep their verdicts; full_test.sh Section 4g globs the directories, no script change.
- **Memory**: keep the BIG real-Bitwuzla capped/uncapped case (`tests.sh:358-385`); it is the end-to-end worker-cap check and must still hit the cap (nothing in it folds). Add a unit test that building N terms leaves every parallel arena vector (`kinds,a,b,c,aux,hashes,chain,named`) at exactly N entries, and put the per-term footprint arithmetic (~9 words/term; terms <= unrolled instruction cap 200000 x a small constant) in the PR body and ADR. No cap-calibrated shell test (flaky).
- **ADR** `docs/adr/2026-10-10-HHMM-term-simplifier-trusted-base.md`: simplifier + claim elision are in the trusted base; differential/exhaustive tests are the guard; what is deliberately not folded (128-bit, wide mul-overflow, div edge cases).
- **Bootstrap/self-test**: `scripts/bootstrap.sh --skip-cargo --no-cache` on the final head SHA, record SHA + `scripts/seed.toml` pin in the PR checklist; `build/vowc test compiler/` and `VOW_FULL_TEST_SKIP_CARGO=1 scripts/full_test.sh` (background + poll, ~40 min).

## Verification surface
- ESBMC (Stage 1 bootstrap verification of the compiler itself) must still prove/skip the new `vc_bvfold_*`/`vc_term` functions: use wrapping arithmetic and u64 shifts only; no `hash_*` builtins (would be reported Skipped); no `HashMap`. Contracts are domain statements (`1 <= w <= 64`, result fits width), not `--unwind` artifacts. If a function cannot be proven it is marked unverifiable, never contract-weakened.
- Corpus verdicts: `tests/verify-native/{pass,fail,unknown,skip}` statuses and `counterexample-fn/blame` directives unchanged under real Bitwuzla (run `scripts/full_test.sh` Section 4g with `bitwuzla` on PATH). Soundness evidence for "unchanged": S2 differential tests + exhaustive width-8 tests.
- C parity (`c_emitter.rs`/`c_emitter.vow`): untouched; Section 2c must stay green.
- No new `tests/run/` or `examples/` fixtures.

## Risk areas
- **Simplifier unsoundness silently proves a false contract** (highest). Mitigation: only identities whose operand is returned unchanged or both-constant folds in a restricted domain; exhaustive width-8 + differential tests; SMT total div/rem semantics honoured by not folding edge cases; fixtures `fail/*` guard the counterexample direction.
- **Claim elision changes query counts** that wiring tests assert -> fixtures updated in S6; an accidental elision of a hard claim would turn a fail into proven, so S5 elides only `negated == false` and syntactically-implied duplicates.
- **Golden churn**: many exact-text assertions in `test_vc_exec.vow`/`test_vc_agg.vow`; update goldens by hand, not by loosening to `contains("define-fun")`.
- **Fixed point / determinism**: no map iteration, intern table insertion order is walk order; compile twice and compare `sha256sum` of stage1/stage2 via `scripts/bootstrap.sh`.
- **Static-sat soundness**: valid only with an empty assumption prefix (S5). Reviewer: look for any path that marks `open`/records a site without checking `assumed_lens[k] == 0`.
- **Untouched gates**: no Rust code, canonical printer or `vow-clif-shim` change, so `cargo clippy --all --all-targets -- -D warnings` and `parse -> print -> parse` idempotency are unaffected; `compiler/` codegen only changes through new functions (fixed point checked by bootstrap).
- **Module list drift**: `scripts/concat_vow.sh` enumerates modules; missing `vc_bvfold` breaks the Rust stage-0 concat build. New function names must be unique across the concatenated namespace (`vc_bvfold_` prefix).
- **Name-dependent consumers**: anything reading `vN` names (tests, `vc_agg` junk/param symbols use `p<k>f..`/`j<n>`; grep before S3) must not rely on every IR value being named.
- **Shared-state hazards**: `vc_flow_name` naming order changes with CSE (a name can first appear via the flow rather than a value); goldens with `g0`/`v7` mixes need rewriting.
- **Memory**: per-term footprint grows ~2x (hash + named + slot); terms are bounded by the unrolled instruction cap (`VC_UNWIND_INST_CAP` 200000), so worst case stays far below the 4 GiB worker cap, and dedupe lowers the count. OOM in the worker is already classified `unknown` (`vc_worker.vow:236-257`).
- **Compile-cache/gate flakes** (memory notes): validate codegen with `VOW_CACHE_DIR=$(mktemp -d)`; pre-existing failures `u64_marker_propagation`, `contracts_tmp_cleanup`, `concrete-block-region-parity` reproduce on clean main.

## Out of scope (follow-up issues)
- Cone-of-influence slicing per claim / avoiding `vc_script_prefix` header copy (O(claims x header) rendering).
- Join-point path-condition rule `or(and(p,c), and(p,not c)) -> p`, commutative operand ordering, and/or flattening.
- 128-bit constant folding, wide `mul` overflow folding, interval/known-bits reasoning.
- Skipping dead instances during `vc_unroll` itself (executor-driven lazy unrolling); incremental solving (push/pop) across unwinding rounds.
- Any Rust compiler/`vow-verify` change, `docs/spec/*`, CLI, or contract changes.
