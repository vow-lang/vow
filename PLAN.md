# Plan: issue #1307 — compile cache may mask stale ESBMC verdicts

## 0. Investigation summary (read this before the rest of the plan)

The issue asks to *confirm* whether the compile cache masked #1291's `span_len`
bug locally, and either fix the cache key or change the PR-checklist
convention. I confirmed this both by reading the cache implementation and by
pulling the actual CI run for #1291's merge commit. The cache is **not** the
cause; the culprit is a process gap.

**Cache soundness (why a stale verdict structurally cannot be served):**

- `vow/src/cache.rs::VerifyCache` only ever persists `FAILED` entries
  (`VERIFY_CACHE_FAILURE_HEADER`). `parse_cached_result` rejects anything that
  isn't literally `"FAILED v3"` on the first line — a `PROVEN` verdict is
  never written to disk, so a warm cache cannot *demote* a real failure to a
  pass. It can only ever serve a stale `FAILED`, which would still turn a
  build red, not green. This is deliberate ("Security: cached PROVEN results
  from disk are never trusted" — see the comment above `CachedFailure`).
- The lookup key is a hash of the **emitted C source text** for the function
  plus `max_k_step`/solver/encoding/memlimit (`VerifyCache::config_key`).
  `span_len` did not exist before PR #1291 introduced it, so there is no
  preexisting cache entry its C text could collide with across the PR's three
  commits — each edit to the function changes the C text and therefore the
  key.
- `vow/src/main.rs::compile_cache_enabled` (`!no_cache && no_verify`) already
  disables the **object** cache whenever ESBMC verification is active. That
  fail-closed gate landed in commit `468b8458` (2026-04-30), months before
  #1291 (2026-09-17). It was not the state of the code at the time of #1291;
  it already existed.
- `compiler/verifier.vow` (the self-hosted compiler) has **no on-disk verify
  cache at all** — see the block comment at `compiler/verifier.vow:1383-1394`:
  "the self-hosted compiler currently always re-runs ESBMC." `--no-cache` is
  accepted there only for CLI symmetry and has no effect. There is likewise no
  compile-object cache implementation anywhere in `compiler/*.vow` (confirmed
  by grep — the only `.o`-adjacent code is literal output-path string
  building, not a cache). So Stage 2/3 of `bootstrap.sh` (`build/vowc`
  rebuilding itself) cannot be masking anything via caching either.

**Direct evidence from the actual CI run (not just static reasoning):** the
post-merge `bootstrap` workflow run for #1291's merge commit `6735b548` is
run `35252574778` (`gh run view 35252574778 --json jobs`). Pulling the raw
job log for its `bootstrap` (ubuntu) job via
`gh api repos/vow-lang/vow/actions/jobs/105308313915/logs` (the `gh run view
--log`/`--log-failed` CLI paths truncate this job's huge
`RegionRootEscape`-heavy output around line 5000; the raw API blob does not)
shows the structured JSON build output at the point Stage 1 failed:

```json
{"status":"VerifyFailed", "function":"span_len",
 "values":{"pos":"4611686018427387904","start":"-4611686018427387904", ...},
 "violation":"ensures result >= 0","vow_id":1,
 "source":{"file":"compiler/lexer.vow","offset":9311,"length":20},
 "blame":"callee"}
```

`pos = 2^62`, `start = -2^62` satisfies `requires: (pos as i64) >= start`, but
`(pos as i64) - start = 2^63` overflows signed 64-bit and wraps to `i64::MIN`,
violating `ensures: result >= 0`. This is a deterministic, solver-independent
counterexample — it does not depend on randomized search — and this was the
**first and only** `bootstrap` workflow run ever executed against commit
`6735b548` (`gh run list --workflow=bootstrap.yml` shows no earlier run for
that SHA), on a stock `ubuntu-latest` GitHub-hosted runner whose
`~/.cache/vow` starts empty every run (no `actions/cache` step targets that
path — only `Swatinem/rust-cache`, which caches `cargo`/`target`, is present).
So this was a **cold-cache run that caught the bug on the first try**. Given
the cache's FAILED-only, content-keyed design above, *any* run against this
exact tree — local or CI, cold or warm — must produce the same verdict.

**What this rules in, and the actual explanation (not just "rules out the
cache"):** since a verified run against #1291's actual final tree is
guaranteed to fail regardless of cache state, "green locally" cannot have
been checked against that final tree as a verified build. This is confirmed,
not merely circumstantial:

- `gh pr view 1291 --json baseRefOid` returns `4b37ef2a26a0c42a9b14d18b27d7d63b127a3b8a`
  — the PR's base is the same SHA the PR body's own "Ergonomics note" cites as
  "current HEAD" when it was written (`4b37ef2a`), i.e. the state of `main`
  at the moment the PR was opened, not after any of its own commits landed.
- `git show c52264d1:compiler/lexer.vow | grep -c span_len` returns `0` —
  `span_len` does not exist at commit 1 (`c52264d1`, the PR's only commit at
  the time the body was written).
- The PR body itself is internally consistent with commit 1's tree and not
  with the final tree: it says "No contract clauses changed", true only
  before `f394565b` added `span_len`'s `requires`/`ensures`; it counts "9
  `pos - start` sites" with `as i64` casts at each one, which is what commit 1
  looks like — commit 2 (`24e839bd`) replaced those sites with calls to the
  new `span_len` helper.
- The PR's `createdAt` (`2026-09-16T10:22:32Z`) is 22 seconds after commit 1
  was authored (`10:22:10Z`). `f394565b` (the commit that adds `span_len`'s
  buggy contract) was authored the next day at `17:06:57Z`, about 18 minutes
  before the PR merged at `17:24:43Z`. The GraphQL edit history for the PR
  body (`userContentEdits`) shows zero recorded edits.

Taken together: the PR body — including its "`scripts/bootstrap.sh
--skip-cargo` — green" checklist line — was written against commit 1, before
`span_len` existed, and was never revisited after commits 2 and 3 added the
bug 18 minutes before merge. The checklist claim is accurate for the tree it
was checked against; it was simply never re-checked against the tree that
actually merged. No cache mechanism, local or otherwise, needs to be invoked
to explain the discrepancy.

**Conclusion for scope:** there is no cache-key defect to fix (first branch of
the issue's "Ask"). The actionable branch is the second one — tighten the
convention so a "green locally" claim (a) is re-checked against and pinned to
the PR's actual final head SHA (not an earlier commit the author happened to
be looking at when they last wrote the checklist), and (b) is made with a
guaranteed-cold cache as cheap defense in depth (it cannot be the fix, since
the cache was already proven sound above, but it removes any residual doubt a
reviewer might raise and costs nothing to add). This plan also adds a
regression test that pins down the "a cache hit can never turn a real failure
into a pass" guarantee as a checked, executable fact — the literal "confirm"
the issue asks for, made durable rather than re-derived by hand each time.

**Important — this finding must survive past this plan.** The implementation
stage deletes `PLAN.md` before opening the PR (`git rm PLAN.md`, per the
stage-handoff contract), so this section 0 does not itself reach the PR or
the issue thread. The implementation stage must copy the conclusion above
(run `35252574778`, the `span_len` counterexample values, the cold-runner
fact, and the commit-1-vs-final-tree timeline) into **both** the PR body (as
the answer to "confirm whether the compile cache is the cause") **and** a
`gh issue comment 1307` posted before or alongside opening the PR — otherwise
the one piece of information the issue actually asked for ("confirm whether
...") never reaches anyone who didn't read this now-deleted file.

## 1. Problem restated

PR #1291's checklist claimed a green local `scripts/bootstrap.sh --skip-cargo`
run, but the post-merge Bootstrap CI run on `main` failed — on the first and
only attempt against that tree — with a real `ensures result >= 0` violation
in `span_len` (fixed by #1306), and the same "green-locally" claim pattern
recurred on #1295 and #1299. The suspected cause — the compile cache silently
reusing a stale ESBMC verdict — does not hold up: the cache's FAILED-only,
content-keyed design (confirmed sound both by code inspection and by the cold
CI run reproducing the failure deterministically on its first and only try)
means no cache state, local or CI, could have produced a different verdict
against this exact tree. The real gap is that (a) nothing before merge
re-verifies a PR against its actual final head SHA with ESBMC, and (b) the
Bootstrap workflow that would catch it is itself post-merge-only. The
unsigned-sizes migration (#1104/#1116) has more seam PRs coming, each of which
will keep hitting this same "green locally, uncorroborated until merge" trap
unless the convention changes.

## 2. Files to touch

- `tests/verify-cache-stale-pass/tests.sh` (new) — bash integration test
  against the **real** `./target/release/vow` binary and **real** ESBMC
  (no fake stub — see slice 1 below for why), proving a cache warmed by a
  passing version of a function does not mask a later, contract-breaking
  edit to that same function.
- `scripts/full_test.sh` — wire the new test in next to the
  `tests/esbmc-path-cache/tests.sh` call site (~line 747).
- `scripts/bootstrap.sh` — add a `--no-cache` flag that forwards `--no-cache`
  to every stage's `vow build`/`vowc build` invocation; update `usage()`.
- `tests/bootstrap/tests.sh` — extend with a test asserting `--no-cache` is
  forwarded to all three stage invocations (this file already has the
  `VOW_BOOTSTRAP_TEST_LOG`/`fake_compiler.sh` harness needed; `fake_compiler.sh`
  itself does not need to change).
- `CLAUDE.md` — under "Bootstrap Commands (Rust Stage 0)" / "Bootstrap (Rust
  Compiler)", add a self-contained note (it will outlive this plan, so it
  must not reference `PLAN.md`) covering: (a)
  `.github/workflows/bootstrap.yml` only runs on push to `main` and nightly,
  never on PRs, so no PR-time CI corroborates a "bootstrap is green"
  checklist claim; (b) the convention for migration-epic (#1104/#1116) seam
  PRs is to re-run `scripts/bootstrap.sh --skip-cargo --no-cache` against the
  PR's actual final head SHA (after the last push, not an earlier one)
  immediately before ticking that checklist box, and to record the checked
  SHA in the checklist line; and (c) `--no-cache` is cheap defense in depth,
  not a fix for a confirmed cache bug — `VerifyCache` only ever caches
  `FAILED` verdicts (never `PROVEN`), so a stale cache entry can make a build
  wrongly red, never wrongly green; issue #1307 found the actual cause of a
  prior "green locally, red in CI" incident (#1291) to be a PR description
  that was written before the contract-breaking commit landed and never
  re-checked afterward, not a cache defect.
- No `docs/spec/*.md` changes: `--no-cache` already exists as a documented
  `vow`/`vowc` CLI flag (`docs/spec/cli.md`); `scripts/bootstrap.sh` is a dev
  script outside the language/CLI spec surface, and `--no-cache` on it is a
  pure passthrough, not a new capability of the compiler.

## 3. TDD slices

1. **Red/confirm — a cache warmed by a passing version must not mask a later
   contract-breaking edit to the same function.**
   - Test: `tests/verify-cache-stale-pass/tests.sh` (new). Uses the **real**
     `target/release/vow` binary and **real** ESBMC, not a fake stub: the
     scenario depends on the Rust counterexample parser accepting ESBMC's
     actual output, and `tests/esbmc-path-cache/tests.sh`'s fake-`esbmc`
     technique only works there because the self-hosted binary it targets
     has no cache and only ever needs to recognize a trivial
     `VERIFICATION SUCCESSFUL` string — reproducing a parseable *failing*
     trace well enough to fool the Rust parser is not worth the fragility
     when the real toolchain is available in this dev environment and the
     fixture is one function (seconds of wall-clock, not minutes).
   - Behavior under test, matching the actual #1291 incident shape:
     1. Write a fixture containing a function named like `span_len` with the
        **post-#1306** (correct) body: `requires: (pos as i64) >= start`,
        `ensures: result == (pos as i64) - start`, body using checked `-!`.
        Call this "version A". Run `target/release/vow verify` on it with a
        fresh `VOW_CACHE_DIR=$TMP_ROOT/cache`. Assert `Verified`.
     2. Overwrite the same fixture path with "version B": the **pre-#1306**
        (buggy) body — `ensures: result >= 0`, plain wrapping `-`. Run
        `target/release/vow verify` again, **same** `VOW_CACHE_DIR` (now
        "warm" from step 1, even though step 1 stored nothing — PROVEN is
        never cached, which is itself part of what this test pins down).
        Assert `VerifyFailed` with `violation` containing `result >= 0` (or
        equivalently a non-zero exit / non-`Verified` status in the JSON
        output).
     3. This is the literal mechanism the issue worried about: "a local
        bootstrap run can silently reuse a cached verify verdict from before
        the contract-breaking change existed." Step 2 is exactly that
        sequence, with the real toolchain, and it must stay red-for-the-bug
        (i.e. the test must pass, by correctly reporting `VerifyFailed`).
   - Before wiring this into `full_test.sh`, do a one-off sanity check (not
     committed) that the test actually has teeth: `vow/src/verification.rs`
     has **two** hit/miss sites — the primary one at line 121
     (`if let Some(ce) = vc.lookup_failure(...) { Failed(ce) } else { /* run
     ESBMC, store on Failed */ }`, inside `verify_one_function`), and a
     second one at line 283 inside `verify_contracts_only`, which is only
     reached when the *first* verdict is an `arith:`-tagged counterexample
     (see the `#585` comment at line 147 — ESBMC reports one violated
     property per run, and an arithmetic-overflow property can mask the
     contract verdict, so the driver re-asks with arith obligations
     suppressed). The `span_len`-shaped bug in this fixture does **not** hit
     that second site: wrapping `-` (as opposed to checked `-!`) has no
     overflow obligation to report, so ESBMC's one property violation is the
     `ensures` postcondition directly — confirmed by the real counterexample
     in section 0 above, whose JSON has no `arith_overflow`/`arith:` field.
     So mutating line 121 alone is sufficient for *this* fixture; do not
     assume it generalizes to a fixture that layers a checked-arithmetic
     violation on top.

     Temporarily invert line 121's branch so a cache **miss** (the `else`
     branch — which is exactly what step 2 hits, since version B's C text was
     never cached) short-circuits to `VerificationResult::Proven` *without*
     calling ESBMC, instead of running ESBMC. Rebuild, confirm the new test
     now fails (step 2 wrongly reports `Verified`), then revert the temporary
     change. This proves the test would have caught the hypothesized "cache
     masks a stale verdict" bug if it existed, rather than passing vacuously.
   - Production code: none expected — this slice is expected to go green
     immediately against current `main`, because the guarantee already holds
     (as also demonstrated by the real CI run analyzed in section 0). If it
     does *not* go green, stop and re-scope: that would mean the issue's
     premise is correct after all and the fix belongs in `vow/src/cache.rs`
     or `vow/src/verification.rs` (see the contingency note at the end of
     this section).

2. **Green — give the convention a cheap, obvious way to demand a cold run.**
   - Test: `tests/bootstrap/tests.sh`, new function
     `test_no_cache_flag_is_forwarded_to_all_stages` (red first: assert the
     three captured `fake_compiler.sh` invocations each contain `--no-cache`;
     this fails until the flag exists).
   - Production code: `scripts/bootstrap.sh` — add `--no-cache` to the
     recognized flags, append it to both `stage12_build_flags` and
     `stage3_build_flags` (unconditionally — it composes with
     `--verify-jobs 1`, `--no-verify`, and `--stage3-no-verify`, since
     `--no-cache` only ever *removes* cache consultation and never changes
     codegen), and document it in `usage()`.

3. **Docs — close the loop on the convention itself (no test; doc-only).**
   - `CLAUDE.md`: add the two notes described in "Files to touch" above. This
     is the slice that actually answers the issue's second remediation branch
     ("update the #1104/#1116 migration's PR checklist convention").

**Contingency branch (only if slice 1 surprises us):** if step 2 of the new
test actually reports `Verified` on current `main` — i.e. version B's real
bug is masked — the defect is in the **hit/miss handling itself**
(`vow/src/verification.rs` around line 121, or the cache's key derivation in
`vow/src/cache.rs`), not in a missing version/ABI seed. A version seed
(`COMPILE_CACHE_ABI_VERSION`-style) only controls whether a stale **FAILED**
entry gets evicted — reusing a stale FAILED can only produce a false *red*,
never a false *green*, so it cannot be the mechanism for this specific
failure mode and adding one would not fix it. If this branch is taken,
re-diagnose from scratch starting at the exact hit/miss branch and the key
computation that fed it, rather than reapplying the ABI-seed pattern by
analogy. This branch is not expected to be taken per the investigation in
section 0 and the real CI evidence gathered there, but is recorded here so
the implementation stage doesn't need to re-derive the "wrong direction to
fix" warning from scratch if it is.

## 4. Verification surface

This is tooling/process work, not a language or codegen change:

- No new contracts, no `requires`/`ensures` changes, no C-model changes. The
  fixture functions in the new test exist only to drive the cache; they are
  not new compiler-accepted constructs (they reuse the exact
  `requires`/`ensures` shapes already present in `compiler/lexer.vow`'s
  history, both pre- and post-#1306).
- `tests/verify-cache-stale-pass/tests.sh` uses the real ESBMC toolchain
  (required — see slice 1 for why a fake stub doesn't fit this scenario), so
  it depends on ESBMC being installed, matching every other test in
  `scripts/full_test.sh` that already assumes this.
- No `tests/run/*.vow` or `examples/` fixtures need to grow — this issue does
  not touch the compiler's accepted-language surface. The new test's fixture
  lives under `tests/verify-cache-stale-pass/` (scratch, written to a `mktemp
  -d` at test run time), not under `tests/run/` or `examples/`.

## 5. Risk areas

- **Binary fixed point:** `--no-cache` only removes cache *consultation*; it
  never changes what codegen produces for a given IR (the existing comment in
  `bootstrap.sh` already establishes this same invariant for `--no-verify`:
  "Verification does not change codegen, so the SHA-256 fixed-point check
  remains meaningful" — the same reasoning applies to the cache, since a cache
  hit/miss only decides whether codegen is *skipped and replaced with a prior
  byte-identical artifact*, never an *alternate* artifact). No change to
  `vow-clif-shim` stack-slot layout, `BTreeMap` ordering, or codegen ordering
  is in scope, so the fixed-point check is not at risk.
- **`parse → print → parse` idempotency:** unaffected; no syntax or printer
  changes.
- **`cargo clippy --all -- -D warnings`:** the only Rust-adjacent surface
  touched is test wiring in shell scripts, not new Rust source, so no new
  clippy surface. (If the contingency branch in slice 1 is taken, the new
  `VerifyCache` code must still pass `cargo clippy --all -- -D warnings` and
  keep `cache.rs`'s existing unit tests green.)
- **Self-hosted/Rust drift (CLAUDE.md "Vow Compiler" rule):** this plan makes
  no behavior change to either compiler, so the "touch both compilers in the
  same session" rule does not apply here. The one thing that could look like
  drift — `compiler/verifier.vow` having no cache while `vow/src/cache.rs`
  has one — is pre-existing, intentional, and documented in place; nothing in
  this plan narrows or widens that gap.
- **`scripts/full_test.sh` wall-clock:** the new test runs real ESBMC twice
  against a single tiny function, so it should add low single-digit seconds,
  not minutes, to the ~40-minute `full_test.sh` run — but confirm this
  empirically once written rather than assuming it; if it is slower than
  expected, that is a sign the fixture is more complex than needed, not a
  reason to fake the solver.

## 6. Out of scope

- **Making `.github/workflows/bootstrap.yml` run on pull requests.** The
  workflow's own header comment explains this was a deliberate, already-made
  tradeoff to cut PR-blocking CI time (~900s+) after `build-and-test` was cut
  to ~280s. Reversing it is a large, contentious CI-architecture change that
  belongs in its own issue if the project ever decides the tradeoff is wrong
  — not bundled into this fix.
- **Adding a verify or compile-object cache to the self-hosted compiler.**
  It deliberately has neither today, for stronger safety guarantees than the
  Rust side. Adding one would be a new feature with its own correctness
  surface, not a fix for this issue.
- **Any change to `CompileCache`/`VerifyCache`'s key composition in
  `vow/src/cache.rs`**, absent the contingency branch in section 3. The
  existing fail-closed gating and FAILED-only caching are already correct;
  don't add defensive bounds or seeds to a mechanism that isn't broken.
- **Auditing #1295/#1299 individually** for the same "green locally, red in
  CI" pattern. Once the structural explanation (no PR-gating Bootstrap check)
  is documented, re-litigating each prior PR adds no new information.
- **Fixing `scripts/bootstrap.sh`'s pre-existing "unknown flag" behavior**
  (an unrecognized flag currently prints a message and exits 0 via `usage()`
  instead of exiting non-zero). Real, but unrelated to this issue; don't
  bundle it into a cache/convention PR.
- **Posting to the #1104/#1116 epic issues.** The implementation stage may
  optionally leave a `gh issue comment` on #1104 and/or #1116 pointing at the
  new `CLAUDE.md` convention, but that is a communication nicety, not a file
  change this plan requires.
