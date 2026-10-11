# Plan: fold `x - (x / y) * y` into `x % y` in the native verifier's term simplifier (#1560)

## Goal
`vowc verify --backend native tests/verify/modulo_safe.vow` answers `Verified` instead of `timeout`, by
rewriting the Euclidean-remainder idiom into a single `bvsrem`/`bvurem` before the query is printed.
Self-hosted only (scoped native-verifier exception in CLAUDE.md: no Rust twin, no C-emitter change).

## Findings (measured with Bitwuzla 0.9.1 on this machine)
- The issue's query times out because `bvsdiv`+`bvmul`+`bvsub` over 64 bits is a nonlinear chain.
  The same query with `v10 := (bvsrem v0 v1)` is `unsat` in 5 ms.
- The identity `x - (x div y) * y == x rem y` holds for EVERY width and EVERY y (SMT-LIB total division):
  y = 0 gives `x - (-1 or 1)*0 = x = x rem 0`; signed `MIN / -1` wraps to MIN so both sides are 0.
  Bitwuzla proves the unsigned form at 8/16/32/64 bits in ~4 ms (so unsigned is not the bottleneck; the
  `bvurem` rule is for symmetry and cheaper queries) and the signed form only at width 8 (0.3 s); signed
  16/32/64 time out at 60 s -- so the signed rewrite cannot be delegated to the solver and its soundness
  must be pinned by tests. `0 <= x srem m < m` under `x >= 0, m > 0` is `unsat` in 7 ms at 64 bits.
- Operands reach the simplifier as bound variables (`v8 := bvsdiv v0 v1`, `v9 := bvmul v8 v1`, see
  `vc_define`, `compiler/vc_exec.vow:210`; `vc_flow_name`, `compiler/vc_flow.vow:64`), so the rule must see
  through names the way `vc_tm_negated` does (`compiler/vc_term.vow:280`).
- Division-by-zero and `MIN / -1` abort claims are separate hard claims built by the executor from the
  operands, not from the rewritten term; they are unaffected.

## Assumptions
- Minimal slice = this one rule (best guess). The dual `q*d + x rem d -> x`, a `x / y * y <= x` lemma,
  interval abstraction of `bvmul`/`bvsdiv`, incremental solving and Bitwuzla option tuning are follow-ups,
  not bundled (surgical-changes rule). The issue's "ideas" list is explicitly undecided.
- The fix lives in the simplifier (already in the trusted base per ADR 2026-10-10-2041), not in a
  solver option, because options do not make signed 16..64-bit div+mul tractable.
- ADR: `docs/adr/README.md` defines naming only, no amendment policy; an in-place dated `Amendment` section is used
  rather than a superseding ADR because the decision is unchanged (best guess).
- No spec change: no syntax, builtin, CLI flag or verdict semantics change. `docs/spec/cli.md` needs no edit.

## Key Files
| File | Role | Lines |
|------|------|-------|
| `compiler/vc_term.vow` | simplifying constructors; add rule | `vc_tm_negated` 280, `vc_simp_identity` 391, `vc_tm_app2` ~455 |
| `compiler/tests/test_vc_term.vow` | rule checks + exhaustive width-8 semantic check | `check_rules` 108, `ev_value` 246, `main` 573 |
| `tests/verify-native/pass/modulo_identity.vow` [new] | end-to-end pass fixture | |
| `tests/verify-native/fail/modulo_negative_dividend.vow` [new] | counterexample direction | header format: `fail/add_wrong_post.vow` |
| `docs/adr/2026-10-10-2041-term-simplifier-trusted-base.md` | add dated "Amendment" listing the rule + guards | Decision 2, Trust |

## Steps (TDD slices, red -> green, each independently committable)

### 0. Reproduce on current HEAD (gate)
- The issue's query predates #1611 (hash-consing: leaves are no longer `define-fun`'d), #1615 (slicing) and
  #1616 (k-induction), and this worktree has no `build/vowc`. Run `scripts/bootstrap.sh --skip-cargo`, then put
  a `bitwuzla` wrapper first on `PATH` (copies each query to `$TMPDIR`, then `exec`s the real solver; same
  trick as the fake in `tests/verify-native/tests.sh`) and run
  `build/vowc verify --backend native --no-cache --timeout 30 tests/verify/modulo_safe.vow`.
- Confirm the `ensures` query's `bvsub` operands are a parameter and a variable bound to
  `bvmul(var bound to bvsdiv(p0, p1), p1)`. If the file already verifies, comment on #1560 and stop.

### 1. Rule unit test (red)
- **File**: `compiler/tests/test_vc_term.vow`, in `check_rules` (after return code 97; pick the next free codes).
- **Change**: with fresh vars `x`,`y`, build `q = vc_tm_app2(BVSDIV,x,y)`, `p = vc_tm_app2(BVMUL,q,y)`,
  `r = vc_tm_app2(BVSUB,x,p)`; assert `r == vc_tm_app2(BVSREM,x,y)`. Same for `BVUDIV -> BVUREM`, mul
  operands commuted (`y*q`), and through names: `vc_tm_bind` both `q` and `p`, build `sub(x, pvar)`.
  Negative cases must NOT rewrite: `x - v` with `v := vc_tm_bind(vc_tm_ext(ar, VC_EXT_LOW(), 8, w))` (a bound
  `VC_T_EXT` node; guards the EXT-vs-op code collision) and a different dividend (`x2 - (x/y)*y`), different divisor (`x - (x/z)*y`),
  mismatched signedness (`bvsdiv` with `bvurem` is never produced), `bvadd` in place of `bvsub`.
  Also 128-bit-sorted vars: the rule is width-independent (assert it fires on constants-free 128-bit terms
  by building via a 128-bit constant divisor `vc_tm_bv(ar,128,3,0)` with a var dividend).
- **Run**: `build/vowc test compiler/tests/test_vc_term.vow`.

### 2. Implement the rule (green)
- **File**: `compiler/vc_term.vow`, new helpers next to `vc_tm_negated`, call site in `vc_tm_app2`.
- **Change**:
  - `vc_tm_def(ar, t)`: resolve `t` to a node `d` (`t` itself, or `ar.named[t]` when `t` is a `VC_T_VAR`;
    default -1 from `vc_arena_push_hashed`) and return `d` only when `d >= 0 && ar.kinds[d] == VC_T_APP()`,
    else -1. The kind re-check after following the name is mandatory: a bound variable can stand for a
    `VC_T_EXT` node, whose `aux` is the extension kind and collides with op codes
    (`VC_EXT_SIGN == 1 == VC_OP_BVSUB`, `VC_EXT_LOW == 2 == VC_OP_BVMUL`), and whose `b` is an amount, not a
    term id. Same guard as `vc_tm_negated` (`vc_term.vow:280`).
  - `vc_tm_canon(ar, t)`: the bound definition id when `t` is a bound variable, else `t`; two operands are the
    same term when canon ids are equal (a name and its definition are different ids).
  - `vc_simp_sub_quotient(ar, x, y)`: let `m = vc_tm_def(y)`; require `ar.aux[m] == VC_OP_BVMUL()`; for
    each operand order, `d = vc_tm_def(operand)` with `aux[d]` BVSDIV (-> `VC_OP_BVSREM()`) or BVUDIV
    (-> `VC_OP_BVUREM()`), `canon(a[d]) == canon(x)` and `canon(b[d]) == canon(other mul operand)`; return
    `vc_tm_app2(ar, rem_op, x, b[d])` (recursion re-simplifies: rem by 1, constant folds), else -1.
  - In `vc_tm_app2`, after `vc_simp_identity` returns -1, when `op == VC_OP_BVSUB()` try the new rule.
- **Reuses**: `vc_tm_negated` pattern (`vc_term.vow:280`), `vc_tm_app2` recursion, the width-free rule
  discipline from ADR Decision 2 (no sort lookup: both sides share the operand sort by construction).

### 3. Semantic guards for the rule (red-green, soundness)
- **File**: `compiler/tests/test_vc_term.vow`.
- **Random differential (the guard the ADR names)**: add a `make_dag` choice (next to the `choice <= 4` binary
  op branch, line ~437) that builds `sub(a, mul(div(a, b), c))` from random same-width pool operands, with
  `c == b` half the time, both mul orders, signed and unsigned div. This exercises the matcher on near-miss
  shapes (different divisor, different dividend, shared subterms) under the raw-vs-smart check in `check_dag`.
- **Exhaustive width-8 check**: `check_divrem_identity()` wired into `main`: build the raw and simplified DAG
  for `x - (x/y)*y` ONCE per op pair/mul order, then loop `(a, b)` over all 65 536 width-8 pairs varying only
  `env` (do not rebuild arenas per pair: ~3 min/file test budget). Includes `b = 0` and `(-128, -1)`;
  `ev_value` is the independent SMT-LIB reference. Do not bind names here: `ev` reads a variable from `env`,
  not from its bound definition, so see-through is covered by id equality in step 1 only.
- **Why not the solver**: Bitwuzla cannot prove the signed identity at 16+ bits (see Findings).

### 4. End-to-end fixtures
- **`tests/verify-native/pass/modulo_identity.vow` [new]**: three functions with the true contract
  (`i64`: the issue's `mod_positive`, plus a variant with `ensures: result >= 0 && result < m`; `u64`: `ensures: result < m` with `requires: m > 0`;
  `i32` with `requires: x >= 0, m > 0`, `ensures: result >= 0 && result < m`) and a `main` like
  `pass/div_positive.vow`. No bounds added to contracts (Contract Authoring rule).
- **`tests/verify-native/fail/modulo_negative_dividend.vow` [new]**: `requires: m > 0` only,
  `ensures: result >= 0` -> counterexample with `x < 0`; header directives `counterexample-fn`,
  `counterexample-vow-id`, `counterexample-blame Callee`, `counterexample-violation` take their VALUES from an
  actual run (the function has a `requires` before the `ensures`, so the `vow_id` is not `add_wrong_post`'s 0);
  only the directive format is copied from `fail/add_wrong_post.vow`. Pins that the rewrite does not turn a `sat` into `unsat` and that
  `--replay-cex` (full_test.sh:1520) still reproduces it.
- Both are picked up by the existing `full_test.sh` Section 4g glob; no harness edit.
- **Manual check**: `build/vowc verify --backend native --no-cache --timeout 30 tests/verify/modulo_safe.vow`
  -> `Verified` (record wall-clock in the PR).

### 5. ADR amendment
- **File**: `docs/adr/2026-10-10-2041-term-simplifier-trusted-base.md`: append `## Amendment (#1560)`:
  rule statement, why it is width-independent, that the guard is the exhaustive width-8 test (step 3),
  and the follow-up list. No new ADR: the decision (simplifier in trusted base, rules valid at every sort)
  is unchanged.

## Verification surface
- No ESBMC property changes: `vow-verify/src/c_emitter.rs` / `compiler/c_emitter.vow` untouched, so the
  Section 2c C-parity gate is unaffected. Native verifier only.
- Gates: `build/vowc test compiler/tests/test_vc_term.vow`, then `build/vowc test compiler/` (vc_* suites),
  `scripts/bootstrap.sh --skip-cargo --no-cache` (record head SHA + `scripts/seed.toml` pin),
  `scripts/full_test.sh` Section 4g (needs `bitwuzla` on PATH) incl. `--replay-cex` and the
  `VOW_VERIFY_NO_SLICE=1` re-run.

## Risks
- **Soundness (highest)**: a wrong rewrite turns a false contract into `proven`. Mitigated by the
  exhaustive width-8 evaluator test, negative-shape tests, and the `fail/` fixture. Rule only fires on the
  exact shape with canonically equal dividend and divisor.
- **Name see-through bug**: comparing a bound var's id with its definition id would silently never fire
  (perf only, not soundness); the through-names unit test in step 1 catches it.
- **Fixed point / determinism**: the compiler's own `vc_*` code is verified by itself only via the seed
  pin; no `BTreeMap`/ordering change, arena stays append-only, so binary fixed point is unaffected.
  The simplifier changes only verification queries, not codegen.
- **Counterexample text**: unaffected -- models are read for parameters only; intermediate names are not
  printed. Re-run `tests/verify-native/fail/*` to confirm (step 4 gate).
- **Fixture over-reach**: `u64`/`i32` variants must each be seen passing before committing; if one still
  times out, drop it from the fixture and report it as a follow-up rather than weakening a contract.
- **clippy gate**: no Rust change, so the `cargo clippy -D warnings` gate is not touched.

## Out of scope
- Dual rule `(x/y)*y + x%y -> x`, `x / y * y <= x` lemma, interval domain for `bvmul`/`bvsdiv`,
  incremental solving, Bitwuzla option tuning (`--abstraction-*`, `-S kissat`), `bvmul` by constants.
- Checked variant `x -! (x /! m) *! m`: it still emits `bvsmulo`/`bvssubo` overflow claims over the div/mul
  chain and may still time out; follow-up, deliberately no fixture here.
- Closing the title's general claim: the rule closes the reproducer. The implementation stage runs
  `gh issue create` for a follow-up on general nonlinear div/mul (`x / m * m <= x`, interval domain,
  dual identity) and links it from the PR.
- Any Rust compiler change, spec/CLI change, or reformatting of `vc_term.vow`.
- Splitting `vc_term.vow` (527 lines) -- the added code is ~40 lines; a split is a separate refactor.
