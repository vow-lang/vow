# Plan: ground witness reads for String equality of differently built strings (#1653)

## Goal
Make `x.eq(y)` for differently built equal strings (`(a+b)+c` vs `a+(b+c)`) `proven` well inside the default
`--timeout` (native default 300 s) by making the *witness half* of the exact-equality encoding ground: the
differing-index byte of a built string is read through a per-site unfolded `ite` over the build (no quantifier),
while every existing defining axiom stays in the query untouched. Native verifier only (`compiler/vc_*.vow`): exempt
from the Rust twin and C parity (CLAUDE.md "Scoped exception", ADR-2026-10-08-1421). `c_emitter.{rs,vow}`, the `vow-*`
crates and `vow-clif-shim` are not touched.

## Evidence (spikes with the local `bitwuzla`, hand-written SMT-LIB; nothing in the repo changed)
Strings a,b,c: parameter arrays, lengths `< 2^63`, pairwise/triple sums `< 2^63` (the model's `push_str` assumption).
Goal shape is the emitted one: `(not e) or (pre and forall i<n. x2[i]==y[i])`, `e or (w<n and x2[w]!=y[w])`, assert `not e`, logic `ABV`.
| encoding | result |
|---|---|
| **today**: array-based witness + the 8 quantified append axioms | **timeout (>120 s)** |
| witness read as ground `ite` over the build, *syntactically different* index terms (`(k-la)-lb` vs `k-(la+lb)`), no axioms, QF | unsat, 80 s |
| same, offsets canonicalised, conditions `(k - off) <u len` | unsat, 2.8 s (3 operands), 10 s (4), 15 s (5) |
| canonical offsets + **absolute conditions** `k <u (sum of preceding lengths)`, no axioms, QF | unsat, 0.02 s (3), 0.03 s (5), 0.4 s (6) |
| same ground read inside the full `e`/`forall` shape, no axioms | unsat, 0.02 s (3), 0.3 s (5), 0.4 s (6) |
| **recommended**: ground witness (absolute conditions) **and all 8 axioms + array-based `forall` half kept** | **unsat, 0.015 s (3 operands)** |
Conclusions that fix the design: (1) the axioms are not what makes it slow once the witness is ground; Bitwuzla
satisfies the irrelevant quantified facts immediately, so they stay and nothing about soundness or the other
claims changes; (2) the ground read needs canonical offset sums and *absolute* branch conditions (80 s -> 0.02 s).
The spike files are reproducible: arrays `a,b,c`, `define-fun m` = `bvand _ #xff`, nested `ite` with
`(bvult w (bvadd la lb))` conditions and `(select c (bvsub w (bvadd la lb)))`.

## Assumptions
- Only the *witness half* of `vc_str_range_eq` changes semantics of the query text; the `forall` half, all append /
  slice axioms, `byte_at`, the finite-conjunction path and `VC_T_QEQ` are untouched (no binder redesign). Chosen over
  the "drop the axioms, ground everything" variant because the spike shows the extra work buys nothing and removing
  axioms would make any read that bypasses the new helper a free-array read (a spurious-`sat` risk). (best guess)
- Reads beyond a built string's length are never observable: `byte_at` past the length is a hard claim, every range op is
  guarded by `pre`, and each recursive read stays inside its operand (`i < sl` -> `sa[i]`; else `ta[i - sl]`, `i - sl < tl`).
  The ground read returns *some* deterministic value there, which is consistent with the axioms (they leave it free). (best guess)
- The expansion budget is an encoding choice (like `VC_STR_UNFOLD`), never a semantic bound: over budget the witness
  uses today's `select` on the array, which the retained axioms define, so the worst case is exactly today's query.
- Slices (`substring`/`substr`) are opaque leaves of the ground read (their axiom stays); unfolding them needs signed
  offsets where absolute conditions do not apply. Follow-up, out of scope.
- "Default `--timeout`" is the native default 300 s (`docs/spec/cli.md`); the issue's 60 s was an explicit flag.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `compiler/vc_strdeep.vow` [new] | registry of built arrays, canonical offset sums, `vc_str_read`, weight/budget | new |
| `compiler/vc_str.vow` | `vc_str_range_eq` witness (l.80-90), `vc_str_append` (l.188-200) registers its array, header comment | 33-37, 78-90, 188-200 |
| `compiler/vc_vec.vow` | `VcVec` struct: add registry fields next to `memo_key`/`memo_arr`; `vc_vec_new_state` | 33-75 |
| `compiler/tests/test_vc_str.vow` | unit tests; `check_append_defines_contents` (285), `check_identical_appends_share_an_array` (308), `main` (690) | |
| `tests/verify-native/pass/string_assoc_differently_built.vow` [new] | pass fixture pinning the issue | |
| `docs/adr/2026-10-08-1422-native-verification-semantics.md` | #1426 addendum "Sharing" bullet (l.618) names #1653 | 574-618 |
| `docs/spec/cli.md` l.74 + `skills/vow/reference/cli.md` + generated `compiler/main.vow` (2877, 3110), `vow/src/skill.rs` | "exact but not always cheap ... (a + b) + c" sentence | regenerate with `uv run python scripts/generate_help.py` |
| `docs/verifier-eval.md` | mentions the String model; update if it repeats the gap | grep `1653`/`quantified` |
| `scripts/concat_vow.sh` | module list (l.23): insert `vc_strdeep` after `vc_vec`, before `vc_str` | 23 |

## Design
1. **Registry** (`VcVec`, same lifetime/shape as `memo_key`/`memo_arr`): for every array `r` created by `vc_str_append`,
   remember `(sa, sl, ta, tl)` keyed by the term id of `r` (a `Vec<i64>` indexed by term id, grown on demand, not a
   linear scan). Registration happens even for a re-used memoised array. The array and all its axioms are emitted
   exactly as today.
2. **Canonical offsets** `vc_str_off_*`: an offset is a sorted multiset of atoms plus one folded constant, from flattening
   `bvadd` operands of the operand lengths (exact: `bvadd` is associative/commutative mod 2^64), rebuilt left-assoc with
   `vc_tm_app2(VC_OP_BVADD)`. Local to the string reads; the global simplifier is not changed.
3. **Ground read** `vc_str_read(vv, ar, arr, base, off) -> masked byte term` (real index = `base - off`):
   - registered append `sa ++ ta`: `ite(base <u canon(off+sl), read(sa, base, off), read(ta, base, canon(off+sl)))`
     (absolute condition; the right branch's offset is `off+sl`);
   - `store(arr, j, v)` (from `push_byte`): `ite(base - off == j, low8(v), read(arr, base, off))` only when a registered
     append lies below, else a plain `select` (no cost for existing push_byte-only loops);
   - `ite(c, a1, a2)` (array merged at a block join): `ite(c, read(a1..), read(a2..))`, same proviso;
   - anything else (parameter, literal, slice, `from_raw_parts` array): `bvand(select(arr, base - off), 0xff)`.
   Memoised per `(arr, base, off)` within one `vc_str_range_eq` call so shared sub-builds stay a DAG.
4. **Budget**: saturating tree-size `vc_str_weight` (append: 1+w(sa)+w(ta); store: 1+w; ite: 1+w1+w2; leaf: 1), memoised
   by term id; `VC_STR_EXPAND_BUDGET` starts at 512 and is tuned with the perf run. Over budget -> use the existing
   array `select` (so `x.push_str(x)` x k or long merge chains cannot blow up the query). No lazy axiom emission is needed.
5. **Use site**: only the witness in `vc_str_range_eq`: `differs = distinct(read(a, w, ∅), read(b, w + k, ∅))`
   (replacing `vc_str_byte(a, w)` / `vc_str_byte(b, w + k)`). Everything else keeps `vc_str_byte`.

## Steps (TDD order; one commit each, conventional lowercase subjects: `perf(verify): ...`, `test(verify): ...`, `docs(verify): ...`)

### 0. Baseline (before any edit)
- `scripts/bootstrap.sh --skip-cargo` at base `fc680f6b` (no `build/vowc` exists in the workspace yet); keep that binary as
  `$TMPDIR/vowc-base`. Record `--no-cache` wall-clock (median of 3) for **every** `tests/verify-native/{pass,fail,unknown,skip,contracts}`
  fixture and every `tests/verify*/` string user: this table closes acceptance "no fixture gets slower".
  `scripts/verify_perf.py` needs the pinned ESBMC and exits 2 without it; use it only if available.

### 1. Red: fixture + unit expectation
- `tests/verify-native/pass/string_assoc_differently_built.vow` [new]: the issue's `assoc`; a 4-operand
  `((a+b)+c)+d` vs `a+(b+(c+d))`; an append of a literal on each side; `fn main() -> i32 { 0 }`. Times out on base.
- `compiler/tests/test_vc_str.vow`: `check_assoc_witness_is_ground`: build the issue's IR with the file's helpers (`call`, `args2`, ...),
  `vc_claims`, then assert the witness assumption mentions neither `select x2_varr`-style reads of built arrays nor any
  `vq_i`, and that `(bvadd p0_vlen p1_vlen)` is the one canonical offset for the `c` read. Red now.

### 2. Canonical offsets (green on unit tests)
- `vc_strdeep.vow` [new]: `vc_str_off_empty`, `vc_str_off_add(ar, off, len)`, `vc_str_off_term(ar, base, off)`.
  Unit tests: `(la+lb)+lc`, `la+(lb+lc)`, `lc+la+lb` and constant mixes all give one term id; empty offset leaves `base` unchanged.

### 3. Registry + ground read (the fix)
- `VcVec` fields + `vc_vec_new_state`; `vc_str_append` registers (also on a memo hit); `vc_str_read` (append / store / ite / leaf)
  with the "registered append below" proviso; use it in the `vc_str_range_eq` witness. Turns step 1 green; the new fixture is `proven`.
- Keep green: `check_identical_appends_share_an_array`, `check_append_defines_contents`, all `test_vc_*`, all existing `string_*` fixtures.

### 4. Budget
- `vc_str_weight` + `VC_STR_EXPAND_BUDGET`. Tests: a chain of 12 `x.push_str(x)` and a chain of if/else merges each keep the
  query text under a size bound asserted in the unit test and still produce a sound (axiom-backed) query; add to `pass/` only if the real
  solver is quick, else leave unit-only.

### 5. Soundness negative
- `tests/verify-native/fail/string_append_not_commutative.vow` [new]: `a+b` claimed equal to `b+a` -> `sat` counterexample
  (header style of `fail/add_wrong_post.vow`: `// TEST: counterexample-fn/-vow-id/-blame/-violation`). Proves exactness is intact.

### 6. Docs, ADR, help
- `docs/spec/cli.md` l.74 + `skills/vow/reference/cli.md`: replace the "not always cheap ... `timeout`" sentence by: the byte at a differing
  index is read through the build, so differently built equal strings are decided; the cost cliff beyond the expansion budget is noted as cost, not a bound.
- ADR #1426 addendum, "Sharing" bullet: ground witness, out-of-length unobservability argument, budget fallback; drop "tracked as #1653".
- `uv run python scripts/generate_help.py`; `scripts/concat_vow.sh` gets `vc_strdeep`; `scripts/bootstrap.sh --skip-cargo`.
- Update the `vc_str.vow` header comment (the witness is ground; axioms are unchanged).

## Testing / verification (long-running: background + poll, see memory notes on gate wall-clock)
- `build/vowc test compiler/tests/test_vc_str.vow`, then `build/vowc test compiler/` (filter `vc_`).
- Section 4g of `scripts/full_test.sh` (all `tests/verify-native/*` fixtures with the real `bitwuzla`) and `VOWC_BIN=build/vowc bash tests/verify-native/tests.sh`.
- Acceptance 1: `build/vowc verify --backend native --no-cache tests/verify-native/pass/string_assoc_differently_built.vow` -> `proven`; paste wall-clock.
- Acceptance 2: re-run the step-0 table with the new binary; PR lists before/after per fixture (none slower beyond noise); run
  `scripts/verify_perf.py` if ESBMC is installed (geomean <= 1.0, 1.25x per-fixture bound).
- `scripts/bootstrap.sh --skip-cargo --no-cache` on the PR's final head SHA; record the SHA and the `scripts/seed.toml` pin.
- Re-measure the *emitted* query (not only the spike) at step 3. If it is slow: option (a) also read the `forall` half through the build with a
  body-term binder; (b) trigger-style ground instantiation of the append axioms at the witness index (issue direction 2).

## Verification surface
No Vow contract changes and no ESBMC property; no `tests/run/` or `examples/` change. Fixture contracts are the true ones
(`ensures: result == true`), with no solver-motivated bounds.

## Risks
- **Perf regression of existing string fixtures** from larger witness terms: expansion only when a registered append lies below
  a store/ite (plain `select` otherwise) + step-0/step-6 per-fixture timing table.
- **Exponential expansion** (self-append, merge chains): budget fallback to the array `select`, which the retained axioms keep exact.
- **Soundness**: the ground read and the axioms must agree in range. Guarded by the `pre`/`n <= len` discipline of every range op and
  `byte_at`'s hard claim; state that discipline in the `vc_str.vow` header so a future range op cannot read past a length. The `fail/` fixture guards exactness.
- **Index arithmetic**: absolute conditions `base <u off+len` need no wrap of `off+len`; guaranteed by the `push_str` total `< 2^63-1`
  assumption (made before the build exists) and slice clamping. Constant folding in `vc_str_off_*` must use the arena folds (`vc_fold_*`), not host ints.
- **Registry lifetime**: it must live and die with `VcVec` like `memo_key`; an unroll/induction re-walk that builds a fresh `VcVec` must not see
  stale term ids (term ids are arena-global, so keyed-by-id is safe while the arena is shared, and unreachable ids are simply never looked up).
- **Query text changes for every function with a differently built `eq`**: result/incremental caches key on text (no staleness) and
  `tests/verify-native/tests.sh` claim counts do not change (no claim is added or removed).
- **Fixed point / parity**: only `vc_*` verifier code changes; no codegen, shim, or `c_emitter` edit; keep new vectors insertion-ordered (no hash iteration);
  the new module must be in `concat_vow.sh` or the concat bootstrap breaks.

## Out of scope
Unfolding slices (signed offsets); ground reads in `byte_at` or the `forall` half; dropping any axiom; binder redesign of `VC_T_QEQ`; changing `VC_STR_UNFOLD`;
`contains`/`trim` models; the global `vc_term` simplifier; ESBMC, `c_emitter`, Rust crates; new language surface; refactors of `vc_vec`/`vc_exec`.
