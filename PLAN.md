# Plan — issue #1082: scheduled equivalence-validation infrastructure

## 1. Problem restated

Equivalence between the Rust bootstrap compiler and the self-hosted `build/vowc` is only
worth anything if it is re-established on every change, so #1082 asks for a three-tier
cadence around the #1081 runner: a blocking per-PR tier over promoted fixtures plus
`vowc test compiler/`, a non-blocking sharded nightly sweep, and a credentialed monthly
adversarial review — all sharing one incremental ledger so a run re-checks only what
changed and states explicitly what it did not cover. Most of that shipped in #1086 and
#1091; what is still missing is the two Definition-of-Done bullets those PRs left open —
**tier 1 is not actually wired into per-PR CI** (`scripts/full_test.sh`, which is where
`vowc test compiler/` and the promoted fixtures live, runs in no workflow at all), and
**the nightly tier reads the ledger but never writes it**, so every divergence it finds
still needs a hand-edit of `docs/equivalence/ledger.json` before the next run stops
reporting it as new.

## 2. Current state (verified in this workspace, at `9b65f580`)

Already on `main` — do **not** rebuild any of it:

| Deliverable | Where | Status |
|---|---|---|
| Tier-2 workflow | `.github/workflows/equivalence.yml` | merged; sharded 4×, `fail-fast: false`, `timeout-minutes: 120`, `permissions: contents: read`, cron `41 4 * * *` (offset from cargo-mutants `17 2`, bootstrap `53 3`, arena-verify `06 29`), explicit exit-status triage, job summary, `upload-artifact` |
| Tier-2 runner | `scripts/equivalence.py` (1007 lines) + `scripts/test_equivalence.py` (975 lines) | merged; `--shard`, `--min-compared`, `--ledger`/`--no-ledger`, `reconcile()` |
| Tier-3 command | `.claude/commands/equivalence-review.md`, `scripts/pair_review.py` | merged |
| Ledger + schema | `docs/equivalence/ledger.json`, `ledger.schema.json` | merged; 5 pairs, 17 corpus entries |
| Tier docs | `docs/equivalence/README.md` | merged |
| Tier-1 fixtures + suppressions | `tests/run/`, `tests/error/`, `scripts/parity.py` | merged |

Evidence for the two gaps:

- `grep -rn full_test .github/workflows/` returns only a comment in `equivalence.yml`.
  `scripts/full_test.sh` is invoked by no workflow, so its Section 10b (`vowc test
  compiler/`, line 1421) and its `// TEST: known-divergence` fixture handling are
  local-only. Tier 1 is documented as "every PR / blocking" in
  `docs/equivalence/README.md` but is not in fact on the PR path.
- `scripts/equivalence.py` has `load_ledger()` and `reconcile()` but no writer. The
  nightly prints `NO LONGER DIVERGING — update docs/equivalence/ledger.json:` and stops
  there.
- The DoD's "green on `workflow_dispatch`" is evidenced today only by the scheduled run
  `33385393498` (2026-08-31, `success`); the workflow has never been dispatched. Same job
  body, but the literal bullet is unticked.

Measured in this workspace (matters for the CI-placement decision the maintainer deferred
in the 2026-08-25 issue comment):

- `./target/debug/vow test compiler/` — **268 s**, 19/19 passed. The three slowest tests
  dominate: `test_wide_literal_lexer` 77 s, `test_verifier` 74 s,
  `test_lower_inst_ty_index` 70 s.
- `bash scripts/concat_vow.sh clif` + `./target/debug/vow build --no-verify --no-cache` →
  `/tmp/vowc_concat` — **171 s**. This is the artifact `build-and-test` already builds, so
  it is not new cost *inside that job*.
- `/tmp/vowc_concat test compiler/` (all 19) — **did not finish in 18 minutes** of
  ~99%-CPU, single-threaded, 119 MB-RSS work, and was killed. Not a crash and not an
  argument error: `/tmp/vowc_concat test compiler/ --filter arith` returns
  `TestsPassed` in **0.68 s**, so the subcommand works.
- Narrowed by re-running individual tests through the self-hosted binary:
  **`--filter wide_literal` alone exceeded a 420 s timeout** (`test_wide_literal_lexer`),
  against **76.6 s** for the same test under `./target/debug/vow` — a ≥5.5× slowdown on
  one test, and enough on its own to account for the whole sweep overrunning.
  `--filter verifier` was still running when the sweep was stopped, so its number is
  unknown. Caveat on all of these: a concurrent bootstrap from another workspace was
  competing for cores throughout, so treat them as *lower bounds*, not measurements.
- Baseline PR latency today: the last five `ci.yml` runs took **5–6 minutes** wall clock
  end to end.

Two conclusions follow.

**First**, the maintainer's "~40-minute job, needs its own PR" objection does not apply to
this DoD bullet: running *the whole* `full_test.sh` is a 2276-second job that needs a
bootstrap, whereas `vowc test compiler/` needs neither.

**Second, and more important: the self-hosted side looks too slow for the PR path, and
that slowness is itself a finding.** The Rust side is 4.5 min against a 5–6 min baseline,
and #1159 moved the bootstrap off the PR path three merges ago *specifically* to protect
that latency. But `test_wide_literal_lexer` alone taking ≥420 s under `build/vowc`-class
codegen against 76.6 s under the Rust compiler is not merely a CI-budget problem — a
self-hosted compiler ≥5× slower than the reference on the same input is a divergence in an
observable `scripts/equivalence.py` does not currently measure at all. §5 slice 2
therefore states a decision rule rather than a fixed placement, and treats "too slow"
as something to file rather than to absorb.

## 3. Residual scope

Close the two open DoD bullets and nothing else:

1. `vowc test compiler/` runs in per-PR CI, comparing both compilers, blocking.
2. The nightly writes the ledger (as a schema-valid proposal artifact — it is
   `contents: read` by design and must stay that way).

Plus the documentation and the one verification run those imply.

## 4. Files to touch

Production:

- `scripts/parity.py` — add `compare_test()` and a `test` mode to `main()`.
- `scripts/equivalence.py` — add `propose_ledger()` and `--emit-ledger-update`.
- `.github/workflows/ci.yml` — one new `compiler-tests` job (see slice 2 for why a job
  rather than a step in `build-and-test`).
- `.github/workflows/equivalence.yml` — pass `--emit-ledger-update`; name the artifact
  file in the job summary.
- `scripts/full_test.sh` — Section 10b's status/total checks call the shared comparator
  instead of a second hand-rolled copy (slice 5; droppable).

Tests:

- `scripts/test_parity.py` — `compare_test` unit tests.
- `scripts/test_equivalence.py` — `propose_ledger` unit tests.
- `scripts/test_bootstrap_workflow.py` — CI and equivalence workflow-shape guards.

Docs:

- `docs/equivalence/README.md` — "who writes the ledger", and the tier-1 row gains the CI
  step it now actually has.
- `.claude/commands/equivalence-review.md` — Step 3.5 applies `ledger.proposed.json`
  instead of instructing a hand-edit.

**No `docs/spec/*.md` change is required.** Nothing here touches Vow syntax, semantics,
types, builtins, operators, effects, or a `vowc` CLI flag. `parity.py` and
`equivalence.py` are repo harnesses, not the compiler CLI; `vow test`'s flags and JSON
schema (`docs/spec/cli.md` §`vow test`) are consumed unchanged. **No `compiler/*.vow` or
`crates/` change either** — this issue is CI and harness infrastructure, so the
"both compilers in the same session" rule is not engaged. If a slice starts wanting a
compiler edit, that is a signal the slice has drifted out of scope.

## 5. TDD slices

Small, independently revertable, in dependency order. Slices 1–2 close DoD bullet 1;
slices 3–4 close bullet 2; slice 5 is hygiene and may be dropped without reopening the
issue.

### Slice 1 — `parity.py test`: one comparator for `vow test` output

*Red* — `scripts/test_parity.py`, new `CompareTestTest`:

- both `TestsPassed` with equal `total` and an equal per-test `(name, status)` multiset →
  no errors;
- `total` differs (self-hosted discovered fewer tests) → error naming both counts;
- same `total`, one test `passed` on Rust and `failed` on self-hosted → error naming the
  test — this is the #902 shape and a bare `total` check misses it;
- either side's `status` is not `TestsPassed` → error, even when both agree (two equally
  broken suites are not a green tier-1 gate);
- either process exit is non-zero → error.

*Green* — `scripts/parity.py`:

- `compare_test(rust, self_hosted, rust_exit, self_exit)` built from the existing
  `_mismatch()` helper, shaped like `compare_error()`;
- `main()` accepts `test` in its mode tuple and usage string, dispatching with no fixture
  or ledger path (a `vow test` run is not a fixture; there is no suppression registry for
  it, and inventing one is out of scope).

### Slice 2 — the per-PR CI gate, as its own parallel job

**Step 0 of this slice is a measurement, not an edit.** On an otherwise-idle machine, time
`/tmp/vowc_concat test compiler/` end to end and record which tests dominate
(`--filter <stem>` per test; the Rust-side per-test `duration_ms` in §2 is the baseline to
compare against). Everything below branches on that number, `T_self`.

**Placement decision rule.**

- **`T_rust + T_self` ≲ 12 min** → a new `compiler-tests` job in `ci.yml`, blocking,
  exactly as sketched below. This is the outcome #1082 asks for.
- **`T_self` is materially larger, or the run does not terminate** → do **not** put it on
  the PR path. Two things then happen, and both are required:
  1. File the slowness as a tier-2 finding and record it in the ledger. A self-hosted
     compiler several times slower than the Rust one on the same 19 inputs is a real
     divergence in an observable the runner does not currently measure; it is exactly the
     kind of gap this issue's ledger exists to make visible rather than to bury.
  2. Put the gate in `bootstrap.yml`'s nightly `bootstrap` job instead, which already has
     a verified `build/vowc`, a release-mode `target/release/vow`, and a 90-minute budget.
     Say plainly in the PR body and the issue comment that the DoD's "per-PR" placement was
     not met, and why. Scaling the gate down to nightly is the owner's call to confirm —
     but shipping *no* gate, or an advisory one, is not an option.

A step inside `build-and-test` is the wrong shape either way: it would push that job from
~5–6 min to ~15 min and make it the PR critical path again — the exact regression #1159
landed to avoid. A separate job gated on the same `changes` output runs *concurrently*, so
PR latency becomes `max(build-and-test, compiler-tests)` rather than their sum. It also
keeps a slow compiler-test run from delaying the Codecov upload, and makes the new gate
independently re-runnable and independently tunable. The duplicated `cargo build --all`
is cheap under the existing `Swatinem/rust-cache`.

Note that both measurements above used **debug** binaries, matching what `build-and-test`
has on hand. `full_test.sh` builds `--release` (`scripts/full_test.sh:341`), so the
nightly placement gets a much faster Rust side for free; the self-hosted binary is native
Cranelift output either way, so its cost does not move.

*Red* — `scripts/test_bootstrap_workflow.py`, extending `CiWorkflowTest` (which already
runs in CI as "CI workflow-shape unit tests"):

- `ci.yml` has a `compiler-tests` job that invokes `test compiler/` with **both**
  `./target/debug/vow` and `/tmp/vowc_concat`;
- it routes the two outputs through `scripts/parity.py test`;
- it carries no `continue-on-error` — tier 1 is blocking, and a silently advisory tier-1
  gate is the exact failure mode #1082 was filed about. Assert this on the job *and* on
  the step;
- it reuses the docs-only gate (`needs: changes`, `if: needs.changes.outputs.code ==
  'true'`), so a prose PR does not pay for it;
- it carries a `timeout-minutes` (as #988 required of every other job here);
- it builds the concat binary before it reads it — assert `concat_vow.sh` appears earlier
  in the job body than `vowc_concat test`, so a reordering fails here rather than on the
  runner;
- `build-and-test` still runs `concat_vow.sh` (the existing
  `test_pull_requests_still_compile_the_self_hosted_compiler` assertion must keep passing;
  the new job adds a gate, it does not move one).

*Green* — `.github/workflows/ci.yml`, a new job beside `build-and-test`:

```yaml
compiler-tests:
  needs: changes
  if: needs.changes.outputs.code == 'true'
  runs-on: ubuntu-latest
  timeout-minutes: 45
  steps:
    - uses: actions/checkout@v7
    - uses: dtolnay/rust-toolchain@stable
    - uses: Swatinem/rust-cache@v2
    - run: cargo build --all
    - name: Build concatenated self-hosted compiler
      run: |
        bash scripts/concat_vow.sh clif > /tmp/compiler_clif.vow
        ulimit -v 2000000
        ./target/debug/vow build --no-verify --no-cache /tmp/compiler_clif.vow -o /tmp/vowc_concat
        test -x /tmp/vowc_concat
    - name: Compiler test suite, both compilers (equivalence tier 1)
      run: |
        ulimit -v 2000000
        set +e
        ./target/debug/vow test compiler/ > /tmp/rust_test.json
        rust_exit=$?
        /tmp/vowc_concat test compiler/ > /tmp/self_test.json
        self_exit=$?
        set -e
        python3 scripts/parity.py test /tmp/rust_test.json /tmp/self_test.json \
          "$rust_exit" "$self_exit"
```

No ESBMC install, no uv, no llvm-cov: `vow test` without `--verify` never invokes ESBMC,
and adding the install step would cost more than the tests.

`set +e` around the two invocations is deliberate and mirrors `equivalence.yml`: a failing
suite exits non-zero, and the comparator's message is far more useful than a bare
`Process completed with exit code 1`. `parity.py` still fails the step. `ulimit -v
2000000` matches `full_test.sh`'s `run_self` so a memory regression is caught here rather
than by an OOM-killed runner.

The job comment must record *why* it uses the concat binary rather than `build/vowc`:
`full_test.sh` itself compares against a stage-1 binary (`$RUST --no-verify
compiler/main.vow`, `scripts/full_test.sh:344`), and a bootstrap on the PR path is exactly
what `bootstrap.yml` was created to move off it. A stage-1 comparison is weaker than a
fixed-point one — that is the documented attribution caveat in
`docs/equivalence/README.md` — and the nightly still does the fixed-point run.

Linux only. `build-and-test-macos` is `continue-on-error: true` while #501 is open, so
adding a blocking-shaped gate there buys nothing; note it in §8.

### Slice 3 — `propose_ledger()`: the nightly becomes a ledger writer

*Red* — `scripts/test_equivalence.py`, new `ProposeLedgerTest`:

- a new divergence on an untracked file adds a corpus entry with the injected date as
  `first_seen`, `status: "open"`, and exactly the observables seen;
- a new `error_code` divergence also pins `rust_error_codes` / `self_hosted_error_codes`,
  both sorted — the schema *conditionally requires* both on an active `error_code` entry,
  so a proposal that omitted them would be invalid;
- a tracked entry whose every observable stopped reproducing flips to `status: "fixed"`
  and keeps its `note`/`issue`/`fixture`;
- a tracked entry where only one of two observables stopped narrows `observable` to the
  survivor and stays `open`;
- a new observable on an already-tracked file extends `observable` to the list form rather
  than replacing it;
- the `pairs` block and every untouched corpus entry round-trip byte-identically, and key
  order is sorted (a nondeterministic proposal makes its own diff unreviewable);
- a clean run proposes a document equal to the input except `updated`;
- the proposal satisfies the same stdlib-only schema assertions `LedgerLoadTest` already
  applies to the committed ledger (required keys, allowed observable names, sorted
  multisets, `expected` ⇒ `note`+`issue`) — factor those assertions into a helper both
  test classes call, rather than adding a `jsonschema` dependency the CI step
  (`python3 scripts/test_equivalence.py`, system Python, no install) cannot rely on.

*Green* — `scripts/equivalence.py`:

- `propose_ledger(document, new, fixed, today)` — pure, total, no I/O and no
  `datetime.now()` inside. `today` is injected by the caller, matching the schema's own
  note that `updated` is "stamped by the caller, not derived in-process: workflow scripts
  here must stay deterministic";
- `--emit-ledger-update` writes `<output-dir>/ledger.proposed.json` and prints its path
  under the existing `NO LONGER DIVERGING` block. It **never** rewrites
  `docs/equivalence/ledger.json` in place: the nightly is `permissions: contents: read`,
  and a runner that edits tracked files would either fail or need write scope this
  workflow deliberately does not have;
- exit codes are unchanged. Emitting a proposal is not a verdict.

`propose_ledger` writes only the `corpus` half. `pairs` is tier 3's content-hash record
and no corpus sweep has the information to update it; say so in the docstring so a later
reader does not read the omission as a bug.

### Slice 4 — wire the proposal into the nightly and the docs

*Red* — `scripts/test_bootstrap_workflow.py`, new `EquivalenceWorkflowTest` (the module
already globs every workflow for the cron-collision assertion, so this is its natural
home):

- the sweep step passes `--emit-ledger-update`;
- the uploaded artifact path still covers the output directory the flag writes into;
- `permissions: contents: read` is unchanged — a future edit that gives this workflow
  write scope must fail a test, not slip through review.

*Green*:

- `.github/workflows/equivalence.yml` — add `--emit-ledger-update` to the sweep
  invocation; add one line to the `Summarize` step pointing at `ledger.proposed.json` when
  it exists.
- `docs/equivalence/README.md` — under "The ledger", a short "Who writes it" subsection:
  tier 2 emits `ledger.proposed.json` into its artifact (read-only workflow, so a proposal
  rather than a commit); tier 3 applies it and commits, together with the pair hashes only
  it can compute. Also correct the tier table's tier-1 row, which currently promises a
  per-PR gate that did not exist until slice 2.
- `.claude/commands/equivalence-review.md` — Step 1 gains `--emit-ledger-update`; Step 3
  item 5 becomes "apply `ledger.proposed.json`, then add the issue number and fixture path
  by hand" instead of "record it in the ledger".

### Slice 5 — Section 10b stops duplicating the comparator (droppable)

`scripts/full_test.sh` Section 10b hand-rolls the status and total comparison in inline
`uv run python -c` one-liners. Replace those two checks with `run_parity test`, keeping
its `contract_density`, `--filter` and working-directory checks, which are CLI-surface
assertions rather than equivalence ones. This is not an unrelated refactor: `parity.py`
exists precisely so the harnesses share one rule, and leaving two copies means the CI gate
and the local suite can disagree about what parity means.

Cost: verifying it means running `scripts/full_test.sh` end to end (~2276 s; ESBMC 8.3.0
is on `PATH` in this workspace, so it is runnable). If that run is not feasible in the
implementation session, **drop this slice** and open a follow-up rather than landing an
unverified edit to the suite.

## 6. Verification surface

No contracts, no codegen, no C model, no `.vow` source is touched, so **ESBMC has nothing
new to prove and no `tests/run/` or `examples/` fixture needs to grow.** The verification
surface is entirely the Python harnesses and the workflow YAML.

What must be demonstrated before the PR opens, run as separate commands (never
`&&`-chained):

1. `python3 scripts/test_parity.py`
2. `python3 scripts/test_equivalence.py`
3. `python3 scripts/test_bootstrap_workflow.py`
4. `cargo build --all` (already warm here)
5. `./target/debug/vow test compiler/` → `TestsPassed`, `total == 19`
6. `bash scripts/concat_vow.sh clif > /tmp/compiler_clif.vow`; build `/tmp/vowc_concat`;
   `/tmp/vowc_concat test compiler/` → `TestsPassed`, `total == 19`, **with the wall-clock
   time recorded** (this is slice 2's step 0; the placement branches on it)
7. `python3 scripts/parity.py test /tmp/rust_test.json /tmp/self_test.json 0 0` → `OK`
8. `python3 scripts/equivalence.py --rust target/release/vow --self <self> --emit-ledger-update --output-dir /tmp/eq`
   over a small root (e.g. `examples/`, with `--min-compared 5`), then confirm
   `ledger.proposed.json` parses and, on a clean run, differs from the committed ledger in
   `updated` only.
9. `cargo clippy --all -- -D warnings` and `cargo fmt --all -- --check` — unchanged by
   this PR but they gate the same job the new step lands in.

Step 6 is the one that can genuinely fail, and in this workspace it already did not
complete in 18 minutes (§2). `full_test.sh` compares against a *module-loaded* stage-1
binary (`$RUST --no-verify compiler/main.vow`) while CI has the *concatenated* one; they
are built from the same sources but by different paths, so confirming that
`$RUST --no-verify compiler/main.vow -o /tmp/vowc_mod` followed by
`/tmp/vowc_mod test compiler/` behaves the same is a cheap way to tell "the self-hosted
compiler is slow here" apart from "the concatenated build is slow here". If
`/tmp/vowc_concat test compiler/` is not green, do not weaken the comparator to make it
pass — report the divergence, file it, and either promote a fixture or record a ledger
entry, exactly as `docs/equivalence/README.md` prescribes.

Post-merge, to tick the DoD's last literal bullet: `gh workflow run equivalence.yml --ref
<branch>` and link the run in the issue comment. If the runner budget makes a 4-shard
dispatch unattractive, cite scheduled run `33385393498` (2026-08-31, `success`, identical
job body) instead and say so explicitly rather than claiming a dispatch that did not run.

## 7. Risk areas

- **Binary fixed point.** Untouched. No `compiler/*.vow`, no `vow-clif-shim`, no
  `BTreeMap`/`HashMap` choice, no stack-slot layout, no codegen ordering is in scope. The
  new CI step only *reads* the concat binary the job already builds.
- **`parse → print → parse` idempotency.** Untouched — no parser, printer or AST change.
- **`cargo clippy --all -- -D warnings`.** No Rust changes, so nothing new to lint. (Note
  the CI gate is `--all` without `--all-targets`; do not "fix" test-module lints it does
  not run.)
- **Per-PR wall clock — the real cost, and the one thing the owner may want to overrule.**
  Today `ci.yml` finishes in 5–6 min. `compiler-tests` will take roughly
  1 (checkout/cache) + ~1 (cached `cargo build --all`) + ~3 (concat build) + ~4.5 (Rust
  `vow test`) + `T_self` minutes, and `T_self` measured here as **>18 min** (lower bound,
  under core contention). Running it in parallel keeps PR latency at the max rather than
  the sum, but on this evidence the max would be ~4–5× today's. #1159 traded a later
  fixed-point signal for exactly this latency, so the PR body must give the measured
  number and name the alternatives (move to nightly next to `bootstrap.yml`; or run only
  the self-hosted side, since `cargo test --all` already covers the Rust one). Do **not**
  quietly resolve the tension by making the gate `continue-on-error` — an advisory tier 1
  is what #1082 exists to fix.
- **A newly-blocking gate turning the tree red on unrelated PRs.** This gate has never run
  on the PR path, so its first run is also its first evidence. Mitigation: verification
  step 6 above must be green locally *before* the PR opens. If it is not, the correct
  outcome is a filed divergence plus a `known-divergence` fixture — not a `continue-on-error`.
- **`ulimit -v 2000000` on a GitHub runner.** `full_test.sh` and the nightly both already
  apply it to self-hosted invocations, and `ci.yml` applies it to the concat build, so the
  cap is known to hold for compiler-sized work on this image. It is nonetheless the most
  likely source of a first-run surprise: an OOM there reads as an opaque kill. If it
  fires, report the memory number rather than raising the cap reflexively.
- **A stale or hand-edited ledger colliding with a proposal.** `propose_ledger` must
  round-trip unknown keys and preserve `pairs` untouched; the round-trip test is the guard.
  Applying a proposal is a human/tier-3 step precisely so a nightly cannot clobber a
  hand-written `note` or `expected` rationale.
- **`test_nightly_does_not_collide_with_the_other_scheduled_workflows`.** No cron is added
  or moved, so this stays green; do not touch the schedules.

## 8. Out of scope

Deliberately not bundled. Each is a follow-up issue, not a line in this PR.

- **Running `scripts/full_test.sh` in CI.** The full suite is 2276 s and needs a
  bootstrap. #1082's DoD asks for `vowc test compiler/`, which slice 2 delivers without
  it. The wider "promoted fixtures run per-PR" gap deserves its own issue with a
  considered placement (nightly next to `bootstrap.yml`, or a cached required check).
- **The tier-1 step on macOS.** `build-and-test-macos` is `continue-on-error: true` while
  #501 is open; a blocking-shaped step under a non-blocking job is misleading.
- **Tier-2 fuzzer integration (#905).** The issue sequences it after the runner and it is
  a separate issue.
- **Reworking `parity.py`'s suppression registries**, the `compare_json`/`VerifyFailed`
  diagnostics gap (#1138), or the `$esbmc$` prefix filter. All documented, all blocked on
  other issues.
- **`propose_ledger` touching the `pairs` block**, or any auto-filing of issue numbers /
  fixture paths. Those need judgement the sweep does not have.
- **Fixing the self-hosted `test_wide_literal_lexer` slowdown** (§2). File it, record it,
  let it drive slice 2's placement — but a compiler performance bug is not a CI-wiring PR.
- **Adding a compile-time / runtime-cost observable to `scripts/equivalence.py`.** The
  slowdown found above argues for one, and the ledger schema's `observableName` enum would
  need a new member. That is a real follow-up and a real design question (what threshold
  is a divergence rather than noise on a shared runner?), not a line item here.
- **Formatting or comment cleanups** anywhere in the touched files beyond the lines the
  slices change.
- **Any `docs/spec/` edit.** Nothing here changes the language or the `vowc` CLI; a spec
  diff in this PR would be a sign the scope drifted.
