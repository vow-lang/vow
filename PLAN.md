# Plan: issue #1361 — per-clause `--multi-property` taint guard

## 0. Finding: the production guard already exists in both compilers

Before planning new production code, I audited the current branch tip (`a0734a07`,
which already includes `8b12f0ad` / PR #1362, "fix(verify): reject a tainted
VERIFICATION SUCCESSFUL from ESBMC"). That PR's diff is **not** limited to the
whole-function classifier the issue body describes — it also:

- Changed `run_esbmc_multi_property` (`vow-verify/src/esbmc.rs:985-1015`) to return
  a third `tainted: bool` alongside the verdict map.
- Changed `resolve_clause_status` (`vow/src/contracts.rs:88-106`) to take that
  `tainted` bool and only trust `Some(true)` (PASSED) when `!tainted`; `Some(false)`
  (FAILED) is still trusted unconditionally. This is pinned by
  `resolve_clause_status_distrusts_per_claim_proven_when_tainted`
  (`vow/src/contracts.rs:498-531`) and by an end-to-end fake-ESBMC test,
  `run_esbmc_multi_property_flags_a_tainted_run_and_still_reports_the_passed_claim`
  (`vow-verify/src/esbmc.rs:2478-2509`, using the
  `multi-property-solver-error-then-passed.sh` fixture added in the same PR).
- Made the identical change in the self-hosted compiler: `MultiPropertyResult`
  (`compiler/verifier.vow:62-72`) carries `tainted`; `verify_function_multi_property`
  (`compiler/verifier.vow:958-983`) computes it via `is_tainted_output`; the
  per-clause resolution loop in `compiler/main.vow:14494-14540` already guards
  `if verdict == 1 && !mpr_tainted { proven } else if verdict == 0 { failed } else { ...overall... }`.
- Documented the per-clause discipline explicitly in
  `docs/verifier-discipline.md` ("The same discipline applies to `vow contracts
  --verify`'s per-clause `--multi-property` path...").

So item 2 of the issue's "What's needed" — extend the taint guard to the
per-clause path in both compilers — is already done and already tested on the
Rust side. This plan does **not** re-implement it. It closes a real, narrower
gap: **the self-hosted compiler has no isolated, unit-testable function
mirroring `resolve_clause_status`, and therefore no unit test pinning the
taint-gating behavior** the way Rust's `resolve_clause_status_distrusts_per_claim_proven_when_tainted`
does. The self-hosted logic lives inline inside `compiler/main.vow`'s ~14,500-line
`run_contracts_command`-equivalent loop, reachable only by running the whole
verify pipeline through a real or fake ESBMC binary — which the self-hosted test
suite has no infrastructure for (no fake-ESBMC fixtures; those live only under
`vow-verify/tests/fixtures/fake-esbmc`, Rust-only, `#[cfg(unix)]`-gated).

**Ruled out during planning**, so the implementation stage does not need to
re-investigate: whether the external watchdog-timeout path
(`compiler/verifier.vow:463-474`, `exit_code == -2`) could leak partial
`--multi-property` output into `parse_multi_property_verdicts` and get a
clause falsely marked `proven`. It cannot — `run_esbmc` hardcodes
`combined: String::from("(watchdog timeout)")` on that path (verifier.vow:473),
discarding any real stdout/stderr, which matches Rust's `run_esbmc_capture`
discarding its captured buffers and returning bare `Err(Timeout)` on its own
forced-kill path (`vow-verify/src/esbmc.rs:890-893`). Both compilers are
symmetric here; no divergence.

**Explicitly out of scope, flagged as a follow-up candidate, not this PR**: a
*different* question than the one #1361 asks. `parse_multi_property_verdicts`
is called on `combined` regardless of the function's overall status — so if
ESBMC's own internal `--timeout` (not the external kernel watchdog) fires
mid-run after printing a definitive per-claim `PASSED ... vow:N` line for one
claim but before finishing others, that claim's verdict would still be trusted
(`Some(true)`, untainted by the `ERROR:`/memlimit definition of `tainted`)
even though the function overall resolves to `Timeout`/`Unknown`. Whether
ESBMC's `--multi-property` mode can print a claim's verdict as *provisionally*
decided before a later internal timeout — as opposed to only emitting a claim
verdict once it is fully and finally decided — is an ESBMC-internals question,
symmetric between both compilers (same parsing design in both), and is exactly
the kind of question item 1 of this issue asks to confirm against a real ESBMC
binary. Local ESBMC here is 8.3 (per prior session's memory) and no 8.4/8.5
binary is available, so it can't be confirmed in this session. Bundling a
guard for an unconfirmed hazard into this PR repeats the exact anti-pattern
the original #1346/#1362 plan called out ("Tracked separately... so as not to
bundle an unverified per-clause fix into the whole-function soundness fix").
Recommend filing it as a new issue once an ESBMC 8.4+ binary is available to
test against.

## 1. Problem restated

Issue #1361 asks whether ESBMC's per-claim `--multi-property` `PASSED` lines can
be tainted by the same solver-internal-exception class of bug (esbmc/esbmc#4484)
that #1346/#1362 fixed for the whole-function `VERIFICATION SUCCESSFUL` verdict,
and if so, to extend the "tainted disqualifies a trusted verdict" guard to the
per-clause path in both compilers. That guard was, in fact, already added
defensively in the same PR that fixed the whole-function case (#1362) and is
fully tested on the Rust side (`vow/src/contracts.rs`, `vow-verify/src/esbmc.rs`).
The self-hosted compiler (`compiler/verifier.vow`, `compiler/main.vow`) has the
identical production behavior but no equivalent isolated unit test, because the
gating logic is inlined in a large driver loop instead of being a separately
callable function. This plan extracts that inline logic into a small, pure,
directly-testable `resolve_clause_status` function in `compiler/verifier.vow`
(mirroring the Rust module layout) and adds unit tests mirroring Rust's three
`resolve_clause_status_*` tests, closing the test-coverage parity gap and
leaving a regression trip-wire that the current inline code does not have.

## 2. Files to touch

- `compiler/verifier.vow` — add `resolve_clause_status(vow_id: i64, verdict_ids:
  Vec<i64>, verdict_proven: Vec<i64>, overall: i64, tainted: bool) -> String`,
  placed directly after `verify_function_multi_property` (after line 983), next
  to `MultiPropertyResult`. Pure function: no `[io]` effect, no ESBMC
  invocation — mirrors `resolve_clause_status` in `vow/src/contracts.rs:88-106`
  field-for-field (same match order: per-claim `true` gated on `!tainted`,
  per-claim `false` unconditional, then the `overall` fallback ladder
  `VERIFY_PROVEN`/`VERIFY_PROVEN_IR` → `"proven"`, `VERIFY_TIMEOUT` →
  `"timeout"`, `VERIFY_UNKNOWN` → `"unknown"`, `VERIFY_FAILED` → `"unknown"`,
  else → `"error"`).
- `compiler/main.vow` — replace the inline resolution block at
  `compiler/main.vow:14510-14540` (the `while ei < e_func_ids.len()` loop that
  searches `v_ids`/`v_proven` and then branches on `verdict`/`mpr_tainted`/
  `overall`) with a single call:
  `e_statuses[ei] = resolve_clause_status(e_vow_ids[ei], v_ids, v_proven, overall, mpr_tainted);`
  No behavior change — this is a pure refactor callable from the existing call
  site, which is the only call site (`grep -n "verify_function_multi_property"
  compiler/main.vow` returns exactly one hit).
- `compiler/tests/test_verifier.vow` — add three `check_*` functions mirroring
  Rust's three `resolve_clause_status_*` tests (see TDD slices below), wired
  into `main()` as `r19`/`r20`/`r21` with new sentinel return codes starting at
  `55` (next free after the existing `check_is_tainted_output`'s `52`-`54`).
- `docs/verifier-discipline.md` — the per-clause paragraph currently credits
  `run_esbmc_multi_property` / `verify_function_multi_property` with the gating;
  those functions only *compute* the taint bit. Name the actual gating
  functions too: `resolve_clause_status` (`vow/src/contracts.rs`,
  `compiler/verifier.vow`, once extracted).
- No `docs/spec/*.md` changes — this touches no syntax, semantics, CLI flag, or
  builtin signature. `docs/spec/cli.md:470`'s `unknown` status-table row
  already describes the externally-observable outcome, not the internal
  gating function names, so it needs no edit.
- No Rust crate changes (`vow/src/contracts.rs`, `vow-verify/src/esbmc.rs`
  already have the seam, the three unit tests, and the fake-ESBMC end-to-end
  test — see Finding 0). State this explicitly in the PR description so a
  reviewer doesn't flag it against the "always modify both compilers" rule:
  this PR *reduces* Rust/self-hosted drift, it doesn't add any.

## 3. TDD slices

1. **Red**: add `check_resolve_clause_status_prefers_per_claim_verdict_over_overall`
   to `compiler/tests/test_verifier.vow`, calling a not-yet-defined
   `resolve_clause_status`. Mirrors
   `resolve_clause_status_prefers_per_claim_verdict_over_overall`
   (`vow/src/contracts.rs:476-496`): a per-claim `proven` (id 7) wins over an
   overall `VERIFY_FAILED`, and a per-claim `failed` (id 8) wins over an overall
   `VERIFY_PROVEN`. Sentinel `55`.
   **Green**: add `resolve_clause_status` to `compiler/verifier.vow` with just
   enough logic (the two `match`-equivalent arms this test exercises) to pass.
2. **Red**: add `check_resolve_clause_status_distrusts_per_claim_proven_when_tainted`.
   Mirrors `resolve_clause_status_distrusts_per_claim_proven_when_tainted`
   (`vow/src/contracts.rs:498-531`) — the test this issue exists to add. Three
   assertions: (a) id 7 `proven`, `overall = VERIFY_PROVEN`, `tainted = true` →
   `"proven"` (overall still legitimately proven); (b) id 7 `proven`,
   `overall = VERIFY_UNKNOWN`, `tainted = true` → `"unknown"` (tainted PASSED
   must not override an Unknown overall); (c) id 8 `failed`,
   `overall = VERIFY_PROVEN`, `tainted = true` → `"failed"` (FAILED trusted
   regardless of taint). Sentinel `56`.
   **Green**: complete the `!tainted` guard on the `Some(true)`-equivalent arm.
   **Verify the guard is load-bearing** (not just present): temporarily delete
   the `&& !tainted` condition, confirm this test's case (a)/(b) now fail
   (case (a) would coincidentally still pass since overall is already
   `"proven"` — check case (b) specifically flips to `"proven"` instead of
   `"unknown"`), then restore the guard. This is a manual verification step
   during implementation, not a permanent mutation test.
3. **Red**: add `check_resolve_clause_status_falls_back_to_overall_when_clause_unreported`.
   Mirrors `resolve_clause_status_falls_back_to_overall_when_clause_unreported`
   (`vow/src/contracts.rs:533-575`) — empty verdict vectors, a `vow_id` that
   never appears, cases for `VERIFY_PROVEN`/`VERIFY_PROVEN_IR` → `"proven"`,
   `VERIFY_TIMEOUT` → `"timeout"`, `VERIFY_UNKNOWN` → `"unknown"`,
   `VERIFY_FAILED` → `"unknown"` (intentionally, not `"failed"` — pin this
   surprising arm explicitly, matching the Rust test's own comment on why).
   Sentinel `57`. `VERIFY_NOT_FOUND`/error-path coverage: self-hosted's
   `overall` parameter here is never actually `VERIFY_NOT_FOUND` in the real
   call site (the `esbmc_exists` check in `main.vow:14472-14479` short-circuits
   before `verify_function_multi_property` is ever called), so pin the
   catch-all `else -> "error"` arm with one synthetic out-of-range `overall`
   value (e.g. `VERIFY_NOT_FOUND()` itself) for defensive coverage, matching
   how the Rust test pins `ToolNotFound`/`ToolError` even though those are
   similarly unreachable from the real call site.
   **Green**: complete the fallback ladder.
4. **Refactor**: wire `compiler/main.vow`'s call site to `resolve_clause_status`,
   deleting the inline block it replaces. Confirm `compiler/tests/test_verifier.vow`
   still passes (it now exercises the same function the real driver calls, not
   a parallel copy). Confirm the bootstrap triple (`scripts/bootstrap.sh`, then
   `/tmp/compiler_b`/`/tmp/compiler_c` via `scripts/concat_vow.sh`) still
   produces an identical binary — this is a pure extraction with no behavior
   change, so the fixed point must be unaffected.
5. **Docs**: update the `docs/verifier-discipline.md` paragraph (see Files to
   touch). No test — prose only.

## 4. Verification surface

No ESBMC-facing behavior changes. `resolve_clause_status` is pure string-
mapping logic over already-computed inputs (`verdict_ids`/`verdict_proven`/
`overall`/`tainted`); it emits no C code, defines no new contract, and ESBMC
never sees it. No new `tests/run/*.vow` or `examples/` fixtures are needed —
the existing Rust fake-ESBMC fixture
(`vow-verify/tests/fixtures/fake-esbmc/multi-property-solver-error-then-passed.sh`)
already exercises the equivalent Rust code path end-to-end; the self-hosted
side gets equivalent coverage at the pure-function level (slice 2 above)
rather than through a new self-hosted fake-ESBMC harness, since none exists
today and building one is out of scope for a test-parity fix (see Out of
scope).

## 5. Risk areas

- **Binary fixed point**: `resolve_clause_status` must be placed so
  `scripts/concat_vow.sh`'s flattening doesn't collide with an existing
  top-level name — confirmed clear (`grep -rn "fn resolve_clause_status"
  compiler/*.vow` currently has zero hits, and no other module defines it).
  `Vec<i64>` parameters (`verdict_ids`, `verdict_proven`) are read-only scans
  in this function, matching the existing inline loop's access pattern — no
  new mutation ordering to get wrong.
- **`BTreeMap`/`HashMap` determinism**: not applicable — this function touches
  no map type, only parallel `Vec<i64>` arrays already produced deterministically
  by `parse_multi_property_verdicts`.
- **`parse → print → parse` idempotency**: not applicable — no AST/printer
  change.
- **`cargo clippy --all -- -D warnings`**: not applicable — no Rust source
  changes in this PR.
- **Self-hosted effect checker**: `resolve_clause_status` must be declared with
  no `[io]` effect (it calls no ESBMC, does no I/O) — get this wrong and the
  self-hosted type checker will reject the extraction at the call site, since
  `update_contract_statuses`'s equivalent loop (currently pure iteration) would
  otherwise need to become effectful too.
- **Test sentinel collisions**: new `check_*` return codes must not reuse `1`-`54`,
  already claimed by existing tests in the same file — start at `55` per the
  slice list above and verify against the current file before landing.

## 6. Out of scope

- Re-implementing the taint guard itself in either compiler's production path
  — already done by #1362 (Finding 0).
- Building a self-hosted fake-ESBMC test harness (shell-script fixtures +
  subprocess plumbing) to get an end-to-end self-hosted test mirroring Rust's
  `run_esbmc_multi_property_flags_a_tainted_run_and_still_reports_the_passed_claim`.
  The pure-function unit tests in slices 1-3 give equivalent coverage of the
  actual gating logic at far lower cost; a full harness is a larger, separate
  investment not justified by this issue alone.
- The ESBMC-own-internal-`--timeout`-vs-partial-verdict question raised and
  scoped out in Finding 0 — recommend a new issue once an ESBMC 8.4+ binary is
  available locally to test against, rather than guessing at a guard for an
  unconfirmed hazard.
- Any refactor of `compiler/main.vow`'s surrounding ~14,500-line verify loop
  beyond the one block this issue's fix touches — CLAUDE.md's "surgical
  changes" principle rules out opportunistic cleanup here.
- Renaming or restructuring `MultiPropertyResult`/`EsbmcRun` or their Rust
  counterparts — out of scope, no defect found in those types.
