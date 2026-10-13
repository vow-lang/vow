# Plan: ground (quantifier-free) reads of built Strings in the native verifier (#1653)

## Goal
Make `x.eq(y)` for differently built equal strings (`(a+b)+c` vs `a+(b+c)`) `proven` well inside the default
`--timeout` (300 s) by replacing the quantified definition of every `push_str` array with a *ground, per-site
unfolded read* (a nested `ite` over the index), so no second quantified definition has to be related to the
first. Native verifier only (`compiler/vc_*.vow`): exempt from the Rust twin and from C parity (CLAUDE.md,
ADR-2026-10-08-1421). `c_emitter.{rs,vow}`, `vow-*` crates, `vow-clif-shim` are not touched.

## Evidence (spikes run with the local `bitwuzla`, hand-written SMT-LIB, nothing in the repo changed)
Strings a,b,c: params with arrays + lengths `< 2^63`, per-pair sums `< 2^63` (the model's `push_str` assumption).
| encoding of `x2 = (a++b)++c` vs `y = a++(b++c)` witness query (`k < n`, `x2[k] != y[k]`) | result |
|---|---|
| ground `ite` read, conditions `(k - off) <u len`, index `((k - la) - lb)` vs `(k - (la+lb))` | unsat, **80 s** (index terms differ syntactically) |
| same, index offsets canonicalised (`k - (la+lb)` on both sides) | unsat, 2.8 s (3 operands), 10 s (4), 15 s (5) |
| **canonical offsets + absolute conditions `k <u (sum of preceding lengths)`** | unsat, **0.02 s (3), 0.02 s (4), 0.03 s (5), 0.4 s (6)** |
| full emitted shape: `(not e) or forall i<n. dx(i)==dy(i)`, `e or (k<n and dx(k)!=dy(k))`, goal `not e`, logic `ABV`, same absolute-condition reads | unsat, 0.02 s (3), 0.3 s (5), 0.4 s (6) |
Lessons that drive the design: (1) no axioms for the built arrays may remain in the query (they are what Bitwuzla
cannot close); (2) offsets must be *canonical sums* (flatten `bvadd`, sort atoms; exact since `bvadd` is
associative/commutative mod 2^64); (3) branch conditions must compare the *absolute* index against prefix sums
(`k <u off+len`), not subtract inside the condition; (4) the positive-polarity `forall` half does not hurt when
its body is ground-read.

## Assumptions
- Slices (`substring`/`substr`) keep today's axiom (an opaque fresh array defined by one `forall`), but the
  axiom's body reads its *operand* through the new ground read. Unfolding a slice needs signed offsets
  (`k + start`), where the absolute-condition trick does not apply; out of scope, listed as follow-up. (best guess)
- Reading beyond a built string's length returns a deterministic value instead of "unconstrained". Sound because
  nothing observes it: `byte_at` past the length is a hard claim, and every range op (`eq`, `starts_with`,
  `ends_with`, `matches_literal_at`, axiom bodies) is guarded by `pre`/`n <= len`, and each recursive read stays
  within its operand's length (`i < sl` -> `sa[i]`; else `ta[i - sl]`, `i - sl < tl`). (best guess, restated in the ADR)
- The unfolding budget is an encoding choice, never a semantic bound (same wording as `VC_STR_UNFOLD`); exceeding
  it falls back to exactly today's axiom form, so no fixture can regress below current behaviour. (best guess)
- Acceptance's "default `--timeout`" is the native default 300 s (`docs/spec/cli.md`); the issue's 60 s was an explicit run flag.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `compiler/vc_str.vow` | `vc_str_byte`, `vc_str_all_eq`, `vc_str_range_eq`, `vc_str_append`, `vc_str_slice`, memo, header comment | 33-37, 55-98, 188-232 |
| `compiler/vc_strdeep.vow` [new] | built-array registry, canonical offsets, `vc_str_read`, weight/budget, lazy axiom forcing | new |
| `compiler/vc_vec.vow` | `VcVec` struct (add registry fields next to `memo_key`/`memo_arr`), `vc_vec_new_state` | 33-75 |
| `compiler/vc_exec.vow` | `byte_at` read at 440; `push_str` -> `vc_str_append` at 453; substring at 478; logic choice 628/740 | 436-480 |
| `compiler/vc_term.vow` | `VC_T_QEQ` node doc (11-17), `vc_tm_qeq` (592) | |
| `compiler/vc_smt.vow` | `vc_qeq_var/byte/text`, `vc_term_text` QEQ branch | 224-266 |
| `compiler/vc_slice.vow` | `vc_slice_push_children` QEQ children | 73-78 |
| `compiler/tests/test_vc_str.vow` | unit tests incl. `check_append_defines_contents` (285), `check_identical_appends_share_an_array` (308), `check_binder_text` (675), `main` (690) | |
| `tests/verify-native/pass/string_assoc_differently_built.vow` [new] | pass fixture pinning the issue | |
| `docs/adr/2026-10-08-1422-native-verification-semantics.md` | #1426 addendum, "Sharing" bullet names #1653 | 574-618 |
| `docs/spec/cli.md` (l.74), `skills/vow/reference/cli.md`, `compiler/main.vow` (2877, 3110), `vow/src/skill.rs` | "exact but not always cheap ... (a+b)+c" sentence | regenerate via `scripts/generate_help.py` |
| `docs/verifier-eval.md` | mentions string equality limits (grep `1653`/`quantified`) | |
| `scripts/concat_vow.sh` | module list line 23: insert `vc_strdeep` between `vc_vec` and `vc_str` | 23 |

## Design
1. **Built-array registry** (`VcVec`): for each array created by `vc_str_append`, record
   `(array term, sa, sl, ta, tl, axiomatized)`. Lookup by array term id through a `Vec<i64>` indexed by term id
   (grown on demand; not a linear scan like `vc_str_memo_find`). The memo stays (equal builds still share one
   array, `vc_str_eq`'s `aa == ab && la == lb` shortcut keeps working).
2. **Ground read** `vc_str_read(vv, ar, arr, base, off) -> masked byte term`, where the real index is `base - off`
   and `off` is a canonical sum (`vc_str_off_*`: flatten `bvadd` operands of the lengths, fold constants, sort by
   term id, rebuild left-assoc). Cases on the array *term*:
   - registered append `r = sa ++ ta`: `ite(base <u canon(off+sl), read(sa, base, off), read(ta, base, canon(off+sl)))`
     (absolute condition; the right branch's offset is `off+sl`);
   - `store(arr, j, v)` (push_byte): `ite(base - off == j, v&0xff-masked, read(arr, base, off))`;
   - `ite(c, a1, a2)` (block merge of arrays): `ite(c, read(a1..), read(a2..))`;
   - anything else (parameter array, literal array, slice array, opaque built array): `bvand(select(arr, base - off), 0xff)`.
   Memoise per `(arr, base, off)` triple within one function walk so DAG sharing is kept.
3. **Binder with a body term**: `VC_T_QEQ(body, n)` where `body` is a Bool term over the reserved free variable
   `vq_i` (a `VC_T_VAR` leaf that is declared nowhere). Printed `(forall ((vq_i (_ BitVec 64))) (=> (bvult vq_i n) body))`.
   Needed because the forall half and the slice/opaque axiom bodies must contain ground-read terms, which the
   current `(a, b, k, n)` node (arrays only) cannot hold. Invariant to keep and assert in a test: a body term is
   built only in `vc_str`, never passed to `vc_define`/named, never has `vq_i` outside a body. `vc_slice` pushes
   `body` and `n` as children; confirm a leaf with no declaration is inert in `vc_slicer_link`.
4. **`vc_str_append` no longer emits axioms** for a registered array. `vc_str_all_eq` / `vc_str_range_eq` /
   `matches_literal` / `byte_at` read through `vc_str_read` instead of `vc_str_byte`; the witness half of
   `vc_str_range_eq` becomes ground (`w` read at `base=w`), the forall half uses `base = vq_i` (+ `k` for the `b` side).
5. **Budget + lazy forcing (keeps worst cases at today's behaviour).** `vc_str_weight(arr)` = saturating tree size
   of the expansion (append: 1+w(sa)+w(ta); store: 1+w; ite: 1+w1+w2; leaf: 1), memoised by term id.
   `VC_STR_EXPAND_BUDGET` (start 512, tune with the perf run). A read whose weight exceeds it does *not* expand:
   it emits `select(arr, idx)` and calls `vc_str_force_axioms(arr)`, which walks the term, and for every
   registered-but-unaxiomatized append emits today's two defining facts (now as body-term QEQs over operand
   reads) and marks it `axiomatized` (read then stops there as an opaque base). Prevents the `x.push_str(x)` /
   branch-merge exponential blowup while staying exact. `vv.quantified` is set whenever a binder is built or
   forced axioms are emitted.
6. **Slices**: unchanged shape (fresh array + one axiom, memoised), but the axiom body reads `sa` via
   `vc_str_read(sa, vq_i, -start)`: implement as `base = vq_i + start`, `off = empty`.
7. Docs/ADR: describe ground reads, the exact-beyond-length argument, the budget fallback; remove the "#1653
   times out" sentence. Regenerate help (`uv run python scripts/generate_help.py`).

## Steps (TDD order; every slice is one commit, `fix(verify): ...`/`perf(verify): ...` lowercase subjects)

### 1. Pin the failure: unit + fixture (red)
- `tests/verify-native/pass/string_assoc_differently_built.vow` [new]: the issue's `assoc` fn plus a 4-operand
  variant `((a+b)+c)+d` vs `a+(b+(c+d))`, and `a + literal` variants; `main` returns 0. Red now (timeout).
- `compiler/tests/test_vc_str.vow`: add `check_assoc_query_is_ground_and_closed` building the same IR with
  `vc_claims` and asserting (a) no `_varr` axiom for appended arrays in `cs.assumed`, (b) canonical offset text
  `(bvadd p0_vlen p1_vlen)` appears once as the `c` offset. Red now.

### 2. Canonical offsets (green on a unit test)
- `compiler/vc_strdeep.vow` [new]: `vc_str_off_add(ar, off, len)`, `vc_str_off_term(ar, base, off)`;
  tests in `test_vc_str.vow`: `(la+lb)+lc`, `la+(lb+lc)`, and a constant mix yield the same term id.

### 3. Binder with a body term (refactor, behaviour-preserving for existing tests)
- `vc_term.vow`: `vc_tm_qeq(ar, body, n)`; header comment updated. `vc_smt.vow`: print the new form
  (`vc_qeq_text`/`vc_qeq_byte` shrink to the one `forall` template). `vc_slice.vow`: children = `body`, `n`.
- `vc_str.vow`: `vc_str_all_eq` builds `body` over `vq_i` via a `vc_str_byte`-equivalent. Update
  `check_binder_text` (675), `check_append_defines_contents` (285, line 114 expectation), run all of
  `build/vowc test compiler/tests/test_vc_*.vow`. Existing pass fixtures stay green (spot-check with real bitwuzla).

### 4. Registry + ground read for appends (the fix)
- `VcVec` fields + `vc_vec_new_state` (vc_vec.vow 33-75); `vc_str_append` registers and stops emitting axioms;
  `vc_str_read` for append/store/ite/leaf; swap callers (`vc_exec.vow:440`, `vc_str_all_eq`, `vc_str_range_eq`,
  `vc_str_matches_literal`). Turns slice-1 tests green and the new fixture `proven`.
- Regression guard: `check_identical_appends_share_an_array` (308) must still hold (memo untouched).

### 5. Budget + lazy axiom forcing
- `vc_str_weight`, `vc_str_force_axioms`, `VC_STR_EXPAND_BUDGET`. Tests: self-append chain `x.push_str(x)` x12
  stays small (query text length below a bound asserted in the unit test) and still declares the forced
  `_varr` axioms; a branch-merge chain does the same; result remains exact (fixture with `x.push_str(x)` then
  `x.len() == a.len()*2^k`-free equality such as `x.eq(x)`; add to `pass/` only if bitwuzla closes it quickly,
  otherwise to `unknown/` with a comment).

### 6. Slice axiom reads its operand through `vc_str_read`
- `vc_str_slice`: body reads `sa` at `vq_i + start`. Test: `substring` of an appended string
  (`(a+b).substring(0, a.len()).eq(a)`) -> add as a pass fixture line in the new fixture if solver-fast,
  else keep as unit-only and note in `unknown/`.

### 7. Soundness negatives
- `tests/verify-native/fail/string_append_not_commutative.vow` [new]: `a+b` vs `b+a` claimed equal -> `sat`
  (exactness, no spurious proof), with the standard `// TEST: counterexample-*` header (copy header style of
  `fail/add_wrong_post.vow`).

### 8. Docs, ADR, help regeneration
- `docs/spec/cli.md` l.74 and `skills/vow/reference/cli.md`: replace the "not always cheap ... timeout" sentence
  (differently built strings are now decided; the quantifier remains only in the positive `forall` half; the
  budget fallback is mentioned as a cost cliff, not a bound). ADR addendum (#1426) "Sharing" bullet: ground
  reads, why out-of-length reads are unobservable, budget. `docs/verifier-eval.md` if it quotes the gap.
- `uv run python scripts/generate_help.py`, then rebuild (`scripts/bootstrap.sh --skip-cargo`) so `main.vow` help
  text is in sync (`scripts/check_help_coverage.py` runs in `full_test.sh`).
- `scripts/concat_vow.sh`: add `vc_strdeep` (before `vc_str`, after `vc_vec`).

## Testing / verification commands (run in background, poll; see memory notes on gate wall-clock)
- `build/vowc test compiler/tests/test_vc_str.vow` (also `test_vc_vec`, `test_vc_slice`, `test_vc_gate`, `test_vc_ops`).
- Every `tests/verify-native/{pass,fail,unknown,skip,contracts}` fixture with real `bitwuzla`
  (section 4g of `scripts/full_test.sh`; `VOWC_BIN=build/vowc bash tests/verify-native/tests.sh` for wiring).
- Acceptance 1: `build/vowc verify --backend native --no-cache tests/verify-native/pass/string_assoc_differently_built.vow`
  -> `proven`, wall-clock recorded in the PR.
- Acceptance 2 (no fixture slower): time every `tests/verify-native/**/string_*.vow` and `bench`-corpus string
  user before/after with `--no-cache`; then `scripts/verify_perf.py --vowc build/vowc` (geomean <= 1.0 and the
  1.25x per-fixture bound; `--no-cache` is implicit) and record the result.
- `scripts/bootstrap.sh --skip-cargo --no-cache` on the final head SHA (CLAUDE.md "green locally" rule); record the SHA
  and the `scripts/seed.toml` pin in the PR. `uv run python scripts/test_verify_diff.py` is unaffected but cheap.

## Verification surface
No Vow contract changes; no ESBMC property; no `tests/run/` fixture needed. The "proof obligations" are the
native fixtures above. Contracts in the new fixture are the true semantic ones (`ensures: result == true`), with no
bounds added for the solver.

## Risks
- **Hidden regression of existing string fixtures** (`string_build_and_read`, `string_eq_exact`, `string_equal_builds`,
  `string_fill_loop`, `string_ranges_and_literals`): ground reads over `push_byte` stores in loops could produce larger
  terms than `select(store ...)`. Mitigation: only expand through a store/ite when a *registered append lies below*
  (`vc_str_weight`'s memo records "contains a registered append"); otherwise emit the plain `select` as today.
  Measure each fixture before/after (slice 4 gate).
- **Exponential expansion** (self-append, merge chains): budget + lazy forcing (slice 5); this is the part to review hardest.
- **Bound variable capture**: `vq_i` appears in terms built for several binders; hash-consing makes bodies shared
  only when textually identical, which is fine inside any binder. It must never reach `vc_define`/naming
  (`vc_term.vow:231`) or the `named` let-lifting; assert in a unit test that no body term has `ar.named >= 0`.
- **Soundness of out-of-length reads** (assumption above): guarded by `byte_at` claim and `pre` in every range op;
  every new range op added later must keep that discipline: state it in the `vc_str.vow` header.
- **Canonical sum is only used for index/offset terms**; the `bvadd` flatten is exact mod 2^64, but the absolute
  condition `base <u off+len` needs no-wrap: guaranteed by the existing `push_str` assumption (`total <u 2^63-1`)
  and slice clamping; keep that `vc_assume` ahead of the first read.
- Bitwuzla timing is from hand-written spikes; the real emitted query (arena simplifier, `ite` flags, masks) may
  differ. Step 4 must re-measure the real query before moving on; if it regresses, fallback options in order:
  (a) trigger-style ground instantiation of the existing axioms at witness indices (issue direction 2), (b) split
  the read ites with an explicit lemma on the length ordering.
- Result cache / incremental sessions: query text changes for every string-reading function; caches are keyed by text,
  so no staleness, but `tests/verify-native/tests.sh` golden counts of queries must stay unchanged (the number of
  claims does not change).
- Fixed point: only verifier (`vc_*`) code changes; no codegen/clif change; `BTreeMap`/ordering concerns do not
  arise, but keep the new vectors deterministic (no hash iteration). `module` file must be in `concat_vow.sh`.

## Out of scope
Unfolding slice arrays (signed offsets); `contains`/`trim` etc. models; changing `VC_STR_UNFOLD`; the global
`vc_term` simplifier (offset canonicalisation stays local to the string reads); ESBMC / `c_emitter` / Rust crates;
new language surface; refactors of `vc_vec`/`vc_exec` beyond the call-site swaps.
