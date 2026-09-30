# Plan: fix(parser) — `span_pack`'s `start < 65536` requires is a verifier bound

## 1. Problem restated

`span_pack` (`compiler/parser.vow:102-110`) packs a byte offset (`start`) and a
length (`len`) into a single `i64` as `start * 65536 + len`, and its contract
carries `requires: start < 65536` in addition to `requires: len < 65536`. The
`start` bound is not a real domain fact — `start` is a byte offset into a
source file and has no reason to stay under 64 KiB — and it is already
silently violated in practice: `compiler/main.vow` (1,087,249 bytes) has
~100 `vow {` occurrences past byte offset 65536, and every one of them calls
`span_pack_to_here`, which clamps `len` to `[0, 65535]` but passes
`span_start` straight through unclamped. Nothing has caught this yet only
because `scripts/bootstrap.sh` builds `build/vowc` in release mode (no
runtime vow checks) and `compiler/parser.vow` is not currently an ESBMC
verify target. The first `--mode debug` build that parses a real compiler
source file gets a spurious Caller-blamed `VowViolation` inside `span_pack`.

## 2. Root-cause analysis (why the issue's literal suggested fix is half right)

Verified by hand, not assumed:

- **`start < 65536` is unconditionally unnecessary.** Unpacking is
  `span_unpack_start(packed) = packed / 65536` (integer division) and
  `span_unpack_len(packed) = packed - (packed/65536)*65536` (`compiler/diag.vow:132-133`).
  For `packed = start*65536 + len` with `0 <= len < 65536`, integer division
  gives `packed/65536 = start + floor(len/65536) = start` for **any**
  non-negative `start`, and `packed mod 65536 = len`. The round-trip is
  correct regardless of how large `start` is (short of `i64` overflow, see
  below). So `start < 65536` was never load-bearing — it's a pure verifier
  artifact exactly as the issue says.
- **`len < 65536` is *not* the same kind of artifact — dropping it breaks
  correctness.** It is the representation invariant of a base-65536
  positional encoding: unpacking only recovers `start`/`len` correctly when
  `0 <= len < 65536`. `docs/spec/contracts.md`'s own rule agrees:
  "Lengths... belong in contracts only when they express the function's real
  domain, result, or **representation invariant**." This bound expresses
  exactly that. The issue's suggested fix ("drop the two `< 65536` requires
  clauses... and stop clamping `len`") is factually wrong about `len`: doing
  so would let `span_pack_to_here` (which still clamps `len` to `[0,65535]`,
  unaffected by this fix) keep working, but the two other call sites
  (`compiler/parser.vow:508`, `compiler/parser.vow:1187`) pass a token's own
  `span_len` straight through — that's always tiny in practice (no single
  token approaches 64 KiB) but nothing currently *proves* it, and removing
  the bound would silently corrupt the packed value for any call site that
  ever violates it, with no runtime signal at all in release builds. Keeping
  `len < 65536` is the correct, minimal call: it is a genuine representation
  fact, not a verifier bound, so CLAUDE.md's contract-authoring rule does
  not ask us to remove it.
- **Removing `start`'s bound reopens `docs/spec/contracts.md`'s own
  "Wrapping Arithmetic Overflow" / "Verification-Driven Bounds" anti-pattern**
  if left naively unchecked. Contract expressions are checked against exact
  (non-wrapping) arithmetic while `start * 65536 + len` wraps at runtime; for
  a sufficiently large `start` (unreachable today, but no longer excluded by
  `requires` once the 65536 bound is gone) the wrapped `result` would stop
  matching `ensures: result == start * 65536 + len`, i.e. exactly the
  "Wrapping Arithmetic Overflow" failure mode the spec documents. The
  documented fix or a bound is a checked operator, not a bound
  (`docs/spec/contracts.md:103-118`, `:266-293`, `:349`, and the precedent
  fixture `tests/verify/checked_arith_abort_modelled.vow`, e.g. its `scale`
  function: `requires: x > 0, ensures: result >= x { x *! 3 }` — no upper
  bound on `x`, because the abort rules out the overflow case instead).
  `span_pack` should follow the same pattern: switch the body to checked
  `*!`/`+!` so an astronomically large `start` aborts cleanly instead of
  wrapping into a false postcondition, and add **no** new bound. This keeps
  the fix minimal (it is inseparable from correctly closing the reported
  hole — leaving the multiply unchecked while removing its only bound would
  reintroduce the exact "distorted/incomplete contract" anti-pattern
  CLAUDE.md and the issue itself are about) rather than a stylistic add-on.

**Resulting corrected contract** (only file touched: `compiler/parser.vow`):

```vow
fn span_pack(start: i64, len: i64) -> i64 vow {
    requires: start >= 0,
    requires: len >= 0,
    requires: len < 65536,
    ensures: result == start * 65536 + len
} {
    start *! 65536 +! len
}
```

Diff from current code: delete `requires: start < 65536,`; change the body
from `start * 65536 + len` to `start *! 65536 +! len`. Nothing else in
`span_pack`'s signature, callers, or the `len` clamp in `span_pack_to_here`
changes.

**This was verified empirically during planning, not just reasoned about.**
The exact edit above was applied to this worktree's `compiler/parser.vow`
(temporarily, then reverted — see Section 4 slice 2) and run through
`./target/release/vow verify compiler/parser.vow` (built locally via `cargo
build --release -p vow`, since neither `build/vowc` nor
`target/release/vow` pre-existed in this worktree). Result: `"status":
"Verified"`, with exactly one non-pre-existing diagnostic —
`{"error_code":"ArithOverflowReachable","message":"checked arithmetic in
\`span_pack\` can abort: multiplication overflows", ...}` — confirming the
"abort is reachable but the postcondition holds whenever the function
returns" verdict predicted above, precisely mirroring the `scale`/`twice`
functions in `tests/verify/checked_arith_abort_modelled.vow`. The unmodified
contract (with `start < 65536` still present) produces `"Verified"` with
**no** `ArithOverflowReachable` diagnostic at all — confirming the bound
was silently making the proof *easier*, exactly the anti-pattern the issue
describes, and giving TDD slice 2 below a real, empirically-grounded
red/green signal (0 warnings vs. 1) rather than a guess.

## 3. Files to touch

- **`compiler/parser.vow`** (the only production-code file):
  - `span_pack` (currently lines 102-110): drop `requires: start < 65536,`;
    change body to use checked `*!`/`+!`.
  - Leave `span_pack_to_here` (lines 121-128), and the two direct call sites
    at line 508 and line 1187, untouched — they become correct automatically
    once `span_pack`'s over-tight `requires` is gone; no clamp logic changes.
- **`compiler/tests/test_parser_span_pack.vow`** (new file): unit regression
  test, see TDD slice 1.
- **`scripts/full_test.sh`**: one new assertion block verifying
  `compiler/parser.vow` with ESBMC and checking `span_pack` proves, see TDD
  slice 2. (Follows the existing `arith_status`/`arith_warns` helper pattern
  already used for `tests/verify/checked_arith_abort_modelled.vow` around
  line 900-970 of the same file — reuse those helpers, do not duplicate
  them.)
- **No Rust crate changes.** `span_pack` is an internal implementation
  helper inside the self-hosted compiler's own Vow source
  (`compiler/parser.vow`), not a language feature or builtin — there is no
  Rust-side counterpart to keep in parity (the Rust bootstrap compiler's
  `vow-syntax::Span { start: u32, len: u32 }` is a plain struct, not a
  packed `i64`, and is untouched by this bug). CLAUDE.md's
  "modify both compilers" rule governs *language* changes; this is a bugfix
  inside one Vow source file that both compilers compile identically. The
  fixed `compiler/parser.vow` is exercised by both stage-0 (Rust) and the
  bootstrapped self-hosted binary via the normal bootstrap pipeline — there
  is nothing to duplicate.
- **No `docs/spec/*.md` changes.** This is an internal representation detail
  of the self-hosted parser (not public Vow syntax, semantics, a builtin, an
  operator, an effect, or a CLI flag), so none of the doc-sync obligations in
  CLAUDE.md's "Canonical Source of Truth" section apply.

## 4. TDD slices

1. **Red: runtime regression test proving the spurious violation, green:
   requires removal fixes it.**
   - Test file: `compiler/tests/test_parser_span_pack.vow` (new), following
     the existing `compiler/tests/test_lexer_float_overflow.vow` pattern
     (`module ...`, `use parser; use diag;`, `fn check_...() -> i32`,
     `fn main() -> i32` returning 0 on success).
   - Behavior under test: `span_pack(70000, 10)` — a `start` past the old
     64 KiB bound — must return `70000 * 65536 + 10 = 4587520010` without
     triggering a `VowViolation`, and `span_unpack_start`/`span_unpack_len`
     (already in `compiler/diag.vow`) must recover `70000`/`10` from it.
   - Run first against the **unmodified** `compiler/parser.vow` via
     `build/vowc test compiler/ --filter test_parser_span_pack` (default
     `--mode debug`, so the `requires` is live) — expect a `Panicked`/failed
     result (Caller-blamed `VowViolation`, `start < 65536`). This is the red
     step and directly demonstrates the bug described in the issue.
   - Apply the `span_pack` fix from Section 2. Re-run the same command —
     expect `TestsPassed`. This is the green step.
   - Production code: the `span_pack` edit in `compiler/parser.vow` (Section
     3) — no other file makes this test pass.
   - Optional, non-blocking complementary case in the same test file: lex
     and parse a synthetic in-memory source (>65536 bytes of leading
     whitespace/comment padding, then a real `fn ... vow { requires: ... }`
     block) and confirm `span_pack_to_here` produces a correct, non-panicking
     span for the trailing token. This exercises the actual call sites named
     in the issue (`compiler/parser.vow:508`, `:1187`,
     `span_pack_to_here`) rather than calling `span_pack` directly, closer
     to the issue's literal "parsing `compiler/main.vow` past byte 65536"
     scenario. Add only if slice 1's direct call doesn't feel sufficient
     during implementation — the direct call is already a valid, minimal
     red/green and matches this repo's existing `compiler/tests/` style
     (e.g. `test_lexer_float_overflow.vow` calls `lex()` directly rather
     than building a large synthetic fixture).

2. **Verification regression: prove the fixed contract is actually sound
   under ESBMC, and that the fix is distinguishable from the bug.**
   - Location: a new assertion block in `scripts/full_test.sh`, placed next
     to the existing checked-arithmetic-model assertions (~line 900-970),
     reusing the `arith_status`/`arith_warns` bash helpers already defined
     there (`arith_warns "$j" span_pack` counts `ArithOverflowReachable`
     diagnostics whose message contains `` `span_pack` ``, matching how that
     section already checks `scale`/`twice`/`doomed`).
   - **Expected outcome (empirically confirmed, not predicted): `Verified`
     status, plus exactly one `ArithOverflowReachable` diagnostic naming
     `span_pack`.** This is the correct final state, not a contingency —
     `requires` only constrains `start`'s sign, not its magnitude, so
     `start *! 65536` is reachable-overflowable for `start` near
     `i64::MAX / 65536 ≈ 1.4e14`, exactly like `scale`'s unbounded `x *! 3`
     in the precedent fixture. The assertion should therefore be
     `[ "$(arith_status "$j")" = "Verified" ]` and
     `[ "$(arith_warns "$j" span_pack)" = "1" ]`, not zero.
   - **This does guard the exact regression the issue is about.** If
     `start < 65536` is ever re-added (with or without reverting the
     checked operators), the `ArithOverflowReachable` diagnostic for
     `span_pack` disappears — verified empirically: the original,
     unmodified contract (`start < 65536` present, plain `*`/`+`) produces
     `Verified` with **zero** `span_pack`-named `ArithOverflowReachable`
     diagnostics. So `arith_warns "$j" span_pack` flips `1 -> 0` exactly
     when the bogus bound comes back, making `= "1"` a real, empirically
     verified tripwire, not a check that "still passes" either way.
   - Before the fix: this exact command was never run in CI (the issue's
     "not currently an ESBMC verify target" claim, which is about ESBMC
     specifically — see the *separate*, already-existing static
     contract-quality gate in Section 5, which does already cover
     `span_pack`). There is no prior green baseline in `full_test.sh` for
     this specific assertion, so wiring it in for the first time as part of
     this PR is itself the "red" step this slice adds; the paired
     before/after ESBMC runs described above are what makes it a real
     red/green cycle during implementation, not the CI history.
   - Empirically confirmed additional facts worth encoding as comments in
     the new `full_test.sh` block, since they are non-obvious: `vow verify
     compiler/parser.vow` succeeds even though `compiler/parser.vow` has no
     `main` (verify-only, no codegen, so that's fine); it resolves
     `parser.vow`'s `use token`/`use ast`/`use diag` against the file's own
     parent directory and completes in a few seconds; and its `diagnostics`
     array additionally contains ~51 pre-existing `RegionRootEscape` notes
     (region-allocator analysis, unrelated to `span_pack` or this issue) —
     the assertion must filter by `error_code == "ArithOverflowReachable"`
     and the function-name substring (which `arith_warns` already does), not
     by total diagnostic count.
   - Run this assertion against **both** compilers, matching the existing
     `if [ "$compiler" = rust ]; ... else run_self ...` pattern in that
     section of `full_test.sh` — `target/release/vow` and `build/vowc` are
     both general-purpose Vow compilers capable of verifying any `.vow`
     file, not just self-verifying themselves, so both should independently
     confirm this contract.
   - Production code: same `span_pack` edit; this slice only adds the
     regression assertion to `scripts/full_test.sh`.

3. **(Contingency, not expected to trigger)** If a future change to
   `span_pack`'s contract or body ever makes ESBMC report the abort as
   *unreachable* (i.e. `arith_warns` drops to 0 without `start`'s bound
   being reintroduced), or reports a genuine counterexample instead of a
   clean `Verified`, do not add a magnitude bound on `start` to silence it
   — investigate why the model changed instead. This is a documentation
   note for future readers of this contract, not a coded branch in this
   PR.

## 5. Verification surface

- **Property ESBMC needs to prove:** for all `start >= 0`, `0 <= len <
  65536`, `span_pack(start, len)` returns `start * 65536 + len` without the
  checked `*!`/`+!` operators aborting on any input the `requires` admits
  that a real caller could ever supply — or, if the abort is theoretically
  reachable only at astronomical `start` magnitudes, that ESBMC reports it
  as an honest `ArithOverflowReachable` warning rather than crashing the
  proof.
- **New fixtures:** one new `compiler/tests/test_parser_span_pack.vow` (TDD
  slice 1) exercising the runtime behavior; no new file under `tests/run/`
  or `tests/verify/` — `span_pack` is an internal compiler helper, not
  reachable from ordinary `.vow` programs compiled by `build/vowc`, so a
  `tests/run/*.vow` fixture cannot exercise it directly (that's exactly why
  `compiler/tests/*.vow` exists as a separate harness — see
  `docs/spec/cli.md`'s `vow test` module-resolution note). The verification
  regression (TDD slice 2) targets the real `compiler/parser.vow` file
  directly via `vow verify`, so no separate standalone fixture under
  `tests/verify/` is needed either — reuse the real source file as its own
  fixture, matching how `full_test.sh` already verifies other real
  `compiler/*.vow` files in-place (e.g. the `checked.vow` reference at
  line ~941) rather than duplicating logic into `tests/verify/`. Note:
  `span_pack` is the only `vow {}`-contracted function directly in
  `compiler/parser.vow` (confirmed by grep), but `vow verify
  compiler/parser.vow` also pulls in its transitively-`use`d modules
  (`token`, `ast`, `diag`) and reports ~51 unrelated `RegionRootEscape`
  notes from that wider graph (confirmed empirically, Section 4 slice 2) —
  harmless pre-existing noise, but the new assertion must filter on
  `error_code`/function-name, not treat any non-empty `diagnostics` array as
  failure.
- **Operator precedence for `*!`/`+!` confirmed empirically, not assumed.**
  `start *! 65536 +! len` (no parens) compiled and verified with the
  intended semantics (`ensures: result == start * 65536 + len` proved
  against it) — whatever precedence Vow's grammar assigns to checked
  operators, it parses this specific expression as `(start *! 65536) +!
  len` as intended. No parens needed in the implementation.
- **`build/vowc` is absent in this worktree** (only checked during
  planning, not built — `ls build/vowc` found nothing, and neither did
  `target/release/vow`). The implementation stage will need to build at
  least `target/release/vow` (`cargo build --release -p vow`, ~1 minute,
  confirmed during planning) to run `vow test`/`vow verify` against the
  fix, and per CLAUDE.md's "Vow Compiler" rule should also validate through
  the self-hosted `build/vowc` (`scripts/bootstrap.sh`, ~5 minutes per
  project memory) before treating the fix as verified on both compilers.
- **Caching:** `vow verify` caches results by default; the new
  `scripts/full_test.sh` assertion should pass `--no-cache` if the existing
  helpers don't already, to avoid a stale-cache false pass while iterating
  (see project memory: compile/verify caches keyed on source content have
  been a source of false confidence before — confirm the existing
  `arith_status`/`arith_warns` call sites' flags before copying them
  verbatim).
- **An existing, unrelated static gate already covers `span_pack` and was
  empirically re-run against the fix during planning.**
  `scripts/check_contract_quality.py` is a *static* (no ESBMC) weak/
  tautological-contract ratchet, run in `full_test.sh` on
  `compiler/main.vow`'s `use` graph with baseline `weak=0, tautological=0`.
  `compiler/main.vow` -> `frontend.vow` -> `parser.vow` (confirmed by
  grepping `use` lines), so `span_pack`'s contract is already inside this
  gate's scope today — and the script's own docstring names `span_pack`
  explicitly as one of the "parametric bit-packers... hardened with exact
  functional / enumerated postconditions" under issue #81 that brought the
  baseline to 0. Ran `./target/release/vow contracts compiler/main.vow |
  uv run python scripts/check_contract_quality.py --label compiler/main.vow`
  with the fix applied: `weak=0 (max 0), tautological=0 (max 0),
  substantive=90, total=90`, exit 0 — unaffected, as expected (removing a
  `requires` clause doesn't make the surviving `ensures` more tautological;
  the classifier's axis is orthogonal to "is this bound a verifier
  artifact"). No baseline-constant edit expected in
  `scripts/check_contract_quality.py`; implementation stage should still
  re-run this exact command once after editing `span_pack` as a cheap
  double-check, since it's already CI-gating and free to confirm.
- **A grammar.md caveat about checked arithmetic not being modelled was
  checked and found not to apply here.** `docs/spec/grammar.md` (~670-680,
  in the "`-` inside the guard, `-!` outside it" loop-decrement section)
  says "the verifier does not model checked arithmetic at all: `n -! 1` and
  `n - 1`... yield byte-identical counterexamples" — but that claim is
  scoped to *loop-invariant* decrement guards specifically (a narrower,
  separate concern from whole-function `requires`/`ensures` verification).
  `span_pack` has no loop. The empirical run above (Section 2) directly
  contradicts a blanket reading of that caveat: `*!`/`+!` in a straight-line
  function *was* modelled distinctly from `*`/`+` (the `ArithOverflowReachable`
  diagnostic only appears with the checked operators), consistent with
  CLAUDE.md's "Checked operators abort with `ArithmeticOverflow` on
  overflow... the abort is modelled, not ignored." Trust the empirical
  result for this fix; the grammar.md caveat is real but does not apply to
  a loop-free function like `span_pack`.

## 6. Risk areas

- **Binary fixed point:** `span_pack`'s only change is deleting one
  `requires` clause and swapping two operators for their checked
  equivalents in a leaf helper with no control-flow or data-structure
  changes. This does not touch `vow-clif-shim` stack-slot layout,
  `BTreeMap` ordering, or any codegen ordering — negligible risk to the
  triple-bootstrap fixed point. Still worth running the standard triple
  bootstrap (`scripts/concat_vow.sh` + three-stage compile +
  `sha256sum` compare) once before merging, since it's cheap insurance for
  any self-hosted-compiler change, not because this diff is expected to
  perturb it.
- **`parse → print → parse` idempotency:** unaffected — `span_pack` is not
  part of the canonical printer path; span values are metadata attached to
  already-parsed AST nodes, not something the printer re-derives from packed
  span integers.
- **`cargo clippy --all -- -D warnings` gate:** unaffected — no Rust files
  change.
- **Checked-operator semantics surprise:** `*!`/`+!` abort with
  `ArithmeticOverflow` at runtime in debug mode instead of wrapping. Since
  `span_pack` is called from every span-producing site in the parser, if
  TDD slice 1's chosen test value (or, more importantly, real parsing of
  `compiler/main.vow`/`compiler/region.vow` in `--mode debug`) ever hit an
  input that overflows `start *! 65536` in practice, the parser would now
  abort loudly instead of silently corrupting a span. That is the intended,
  correct behavior change (per Section 2's anti-pattern analysis) but is
  worth calling out explicitly in the PR description as an observable
  behavior change from "wraps silently" to "aborts loudly" for the
  (currently unreached) overflow case, distinct from the primary fix (the
  bogus `start < 65536` requires going away).
- **`vow test compiler/` wall-clock:** per project memory, self-hosted
  `vow test` runs roughly 3 min/file; adding one small new test file to
  `compiler/tests/` is a marginal, not multiplicative, cost since `vow test
  compiler/` already recurses over the whole directory in one invocation
  (confirmed via `scripts/full_test.sh:1521`, `run_self test compiler/`) —
  budget for this in the implementation stage rather than the default 2-min
  tool timeout.

## 7. Out of scope (deliberately not bundled)

- **Widening the packing scheme to 32-bit fields** (matching Rust's
  `Span { start: u32, len: u32 }`) to support individual span *lengths*
  past 64 KiB. Investigated and rejected for this PR: no evidence any single
  AST node's span (as opposed to cumulative file offset, which this fix
  already handles) currently exceeds 64 KiB in the self-hosted compiler's
  own sources, `span_pack_to_here`'s existing `len` clamp is unaffected and
  still correct, and this would be a representation-scheme change (touching
  `compiler/diag.vow`'s unpack helpers, `compiler/complexity.vow`, and
  `compiler/lower.vow`'s consumers) well beyond the reported bug. Worth a
  follow-up issue only if a real span length is ever observed to hit the
  clamp.
- **`span_len` in `compiler/lexer.vow`** — checked during planning and found
  already correct (`requires: (pos as i64) >= start, ensures: result ==
  (pos as i64) - start`, checked `-!` operator, no artificial bound). Not
  touched.
- **Wiring `compiler/parser.vow` (or any other self-hosted module) into a
  blocking, always-on CI verify gate beyond the one targeted assertion in
  TDD slice 2.** The issue is about a specific unsound contract, not about
  standing up parser-wide verification coverage; that is a separate,
  larger initiative.
- **Any formatting, refactor, or cleanup of surrounding `span_pack_to_here`
  or the two other `span_pack` call sites.** They are already correct once
  `span_pack`'s over-tight bound is removed; touching them would be
  unrelated churn.

## Exit note for implementation stage

Section 2's finding that the issue's suggested fix is **half wrong** (`len <
65536` must stay; `start`'s bound must go, paired with checked operators —
not a blanket "drop both bounds, stop clamping") is a judgment call, not a
literal reading of the issue text. Per the operating contract, the
implementation stage should post a `gh issue comment 1308` explaining this
deviation (dropping only `start`'s bound, keeping `len`'s, switching to
checked arithmetic) and the reasoning in Section 2, so a reviewer can
override if they disagree.
