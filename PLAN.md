# Plan: issue #1212 — measure arena proof memory on macOS arm64

## 1. Problem restated

`#1156` (landed as PR #1214, "ci: verify macOS arm64 builds with ESBMC") made the ESBMC install
action and the compiler-contract verification pipeline work on macOS/arm64 runners, but it did not
touch the standalone `vow-runtime/verify/arena.c` proof: `arena-verify.yml` still runs only on
`ubuntu-latest`, and `scripts/verify_arena.sh`'s `ulimit -v 2000000` cap has only ever been
exercised against Linux's virtual-memory accounting. Darwin/XNU accounts virtual address space
differently from Linux (ASLR slides, the dyld shared cache, and malloc zones can inflate a
process's reported VSZ far beyond its resident footprint), so the existing 2 GiB cap might reject
a healthy proof outright, or — the opposite failure — might not be enforced by the kernel at all
and silently pass while doing nothing. Issue #1212 asks to *measure* the real behavior on a
macOS/arm64 runner before deciding whether to gate on it: capture peak RSS and virtual-memory
behavior with and without the cap, determine whether `ulimit -v` is a meaningful, safe control on
Darwin, and only then add a blocking macOS arena-proof job.

**Pre-flight fact, verified during planning (not left as a risk):** the pinned ESBMC 8.5 macOS
release (`esbmc-macos.zip`, sha256 `a5a9b444…4d786d`, matching `install-esbmc/action.yml`) statically
links Boolector — `strings` on `release/bin/esbmc` shows `.../boolector-src/src/btor*.c` compile-unit
paths with no corresponding Homebrew dylib reference, unlike Z3 which is dynamically linked against
`/opt/homebrew/opt/z3/lib/libz3.5.1.dylib`. `--boolector` is therefore available on the macOS runner
without any change to `install-esbmc`'s `brew install` list. The issue's premise holds; this is a
measurement-and-gating task, not a re-scope.

## 2. Files to touch

This is CI/tooling work with **no compiler semantics involved** — nothing in `crates/`, `compiler/`,
or `docs/spec/*.md` changes, and `arena.c` itself is not touched (its properties and `--unwind 5`
bound are unaffected by which OS runs the proof). `.github/actions/install-esbmc/action.yml` already
handles `macOS/ARM64` (from #1214) and needs no edit — confirmed above that Boolector doesn't need a
new `brew` package.

- `.github/workflows/arena-verify.yml` — add a `verify-macos` job alongside the existing `verify`
  job: same `needs: changes` / `if: needs.changes.outputs.arena == 'true'` gate, `runs-on:
  macos-latest`, `continue-on-error: true` while the job is in its measurement phase (see Slice 1
  below), sha-pinned actions only, `persist-credentials: false` on checkout, an explicit
  `timeout-minutes` (start generous — Homebrew installing `llvm@21` alone is slow — tighten once a
  real run's wall-clock is known).
- `scripts/verify_arena.sh` — touched **only if** the measurement shows Darwin needs a different
  code path than Linux (e.g. the cap has to be skipped, or set to a different value, or replaced
  with a post-hoc RSS assertion). Do not speculatively branch this script before there's evidence;
  the Linux path must not change either way.
- `scripts/test_verify_arena.py` — extend `WorkflowTest` with shape assertions for the new job
  (job exists, runs on `macos-latest`, is gated on the same classifier output, declares a timeout).
  The existing Linux-specific regex assertions (`test_workflow_runs_the_arena_proof` etc.) must keep
  passing unmodified — they anchor on the `verify` job, not `verify-macos`.
- `vow-runtime/verify/Makefile` — the header comment already carries a per-ESBMC-version peak-RSS
  table for Linux (8.1.0/8.3.0/8.5.0). Add the macOS/arm64/ESBMC-8.5 row once measured; this is the
  canonical place the repo already records this fact, so don't duplicate it elsewhere as the primary
  source.
- `docs/design/arena_macos_memory_profile.md` (new) — an evidence report in the same shape as
  `docs/design/arena_phase9_profile.md`: date, issue, method, raw numbers, and the resulting
  decision. This is the artifact `gh issue comment` and the PR body will point to.
- `.github/actions/install-esbmc/action.yml` / `scripts/test_install_esbmc_action.py` — not touched;
  named here only to record that they were checked and don't need it.

## 3. Work slices (this is CI measurement, not a code feature — no red/green unit-test loop applies
   to the proof itself; the "tests" here are the workflow-shape assertions and the empirical runner
   data)

**Slice 1 — land a non-blocking measurement job and gather real evidence.**

1. Add `verify-macos` to `arena-verify.yml`, `continue-on-error: true`, running (in order, so each
   step isolates one question the issue asks):
   a. `Install ESBMC` (existing composite action) then `esbmc --version` — confirms the binary loads
      and reports the Boolector-capable build.
   b. A probe step, run *without* `set -e` swallowing its result, that does `ulimit -v 2000000`
      standalone (not via `verify_arena.sh`, so a Darwin rejection doesn't abort the job before any
      data is captured) and records the exit status plus `ulimit -v` readback.
   c. `esbmc --version` again *after* the cap is set — catches the failure mode where the cap is
      accepted by `ulimit` but the dyld shared-cache reservation alone exceeds it, so nothing after
      that point can even start.
   d. `/usr/bin/time -l scripts/verify_arena.sh` (uncapped baseline: temporarily invoke
      `make -C vow-runtime/verify verify` directly for this step, since the script hardcodes the
      cap) to get Darwin's "maximum resident set size" for the real proof, plus a background
      `ps -o vsz=,rss= -p "$pid"` sampler (1s interval) piped to a log artifact, noting explicitly
      that Darwin VSZ includes the shared cache and is not comparable in magnitude to Linux VmSize.
   e. `/usr/bin/time -l scripts/verify_arena.sh` (capped, the real script, real `ulimit -v 2000000`)
      — the actual gate candidate. Record whether it passes, and if it passes, whether the sampled
      VSZ during the run exceeded 2 GiB anyway (cap accepted but not enforced).
   f. Upload the sampler log and `time -l` output as a workflow artifact (`actions/upload-artifact`,
      sha-pinned) so the numbers survive past the job for the design doc.
2. Extend `scripts/test_verify_arena.py::WorkflowTest` with the shape assertions listed in §2.
3. Commit both changes, push, and open the PR (per the run's operating contract — `gh pr create`
   with explicit flags, no `--web`). Because `arena-verify.yml` is already in `ARENA_INPUT_FILES` in
   `scripts/ci_docs_only.py`, the PR's own CI run exercises `verify-macos` — no separate
   `workflow_dispatch` is needed to get a real macOS/arm64 data point.
4. Read results from logs, **not** job status: `continue-on-error: true` reports the job green even
   when a step inside it fails, so `gh run view <id> --log` (after `gh run list --workflow
   arena-verify.yml --branch <branch>`) is the only reliable source for what actually happened.
   Budget roughly 20–30 minutes of wall-clock for the run (checkout + ESBMC cache/install + one
   `--unwind 5` Boolector proof on a fresh macOS runner).

**Slice 2 — apply the decision matrix and (conditionally) flip the job to blocking.**

Write `docs/design/arena_macos_memory_profile.md` from the Slice-1 data, then act on exactly one of:

- **Cap enforced, proof passes, headroom comparable to Linux's ~585 MiB @ 8.5.0** (i.e. sampled VSZ
  stays under 2 GiB and RSS is in the same order of magnitude as the Linux table): keep the existing
  `ulimit -v 2000000`, drop `continue-on-error: true` from `verify-macos`, and it becomes the
  blocking macOS arena-proof job the issue asks for.
- **Cap rejected outright, or kills dyld/ESBMC before the proof can run** (step 1c or 1b in Slice 1
  fails): `ulimit -v` is not a safe control on Darwin. Do not carry it into the blocking job. Instead
  gate macOS on a post-run assertion against the measured `time -l` max-RSS figure (a concrete number
  from Slice 1, not a guess), and flip to blocking on *that* check once it's proven stable across at
  least the PR run plus one nightly run.
- **Cap "passes" but the sampler shows VSZ routinely over 2 GiB during the run**: the limit is
  accepted by the kernel but not meaningfully enforced (or enforced against a definition of "virtual"
  that doesn't track this workload). Same remedy as above — RSS-based gate, not a virtual-memory cap
  — with the doc explicitly stating why the Linux-style cap doesn't transfer.
- **Solver or proof failure unrelated to memory accounting** (e.g. a Boolector version mismatch
  behaves differently on arm64, or the proof times out for a reason unconnected to the ulimit
  questions above): `verify-macos` stays advisory (`continue-on-error: true`), the doc records the
  failure mode, and a follow-up issue is filed via `gh issue create` referencing #1212. This does not
  block closing #1212 — the issue's own text only requires "capture peak RSS and virtual-memory
  behavior" and "determine whether the limit is meaningful and safe," not that the proof necessarily
  succeed on the first try.

Whichever branch applies, update the `arena-verify.yml` header comment (it already documents why the
Linux job runs where it does) with one line pointing at the macOS job and the design doc, mirroring
the existing style rather than duplicating the rationale inline.

## 4. Verification surface

No contract, codegen, or C-model properties change. `arena.c`'s assertions and its `--unwind 5`
bound are platform-independent — they describe the arena's chunk-chain invariants, not the host OS's
memory accounting — so nothing here touches what ESBMC proves. No new fixtures are needed under
`tests/run/` or `examples/`; the "test fixture" for this issue is the workflow-shape assertions in
`scripts/test_verify_arena.py` plus the empirical CI run itself. If Slice 2 lands an RSS-based
post-run check (rather than keeping the virtual-memory cap), that check is a CI script assertion, not
an ESBMC property — it does not enter `requires`/`ensures`/`invariant` territory and so is exempt
from the "don't weaken contracts to fit the verifier" rule (there is no contract here to weaken).

## 5. Risk areas

- **False negative from `continue-on-error: true` masking a real failure.** Mitigated by reading
  `gh run view --log` directly in Slice 1 step 4 rather than trusting job/check status.
- **Cap probe ordering.** Testing `ulimit -v 2000000` inside the existing `set -euo pipefail`
  `verify_arena.sh` would abort the whole script (and hide the exit code) if Darwin rejects it. The
  plan isolates that probe into its own step (Slice 1, step 1b) precisely to avoid losing the signal.
- **Runner cost/flakiness.** `macos-latest` runners are slower and pricier than Linux; keeping
  `continue-on-error: true` until the decision is evidence-backed avoids turning a measurement task
  into a new source of red, required-looking CI on every PR touching `vow-runtime/`.
- **Not a fixed-point / codegen / clippy risk.** This issue touches no Rust or self-hosted compiler
  code, so the binary fixed point (`compiler/` codegen ordering, `BTreeMap`/`HashMap` choices,
  `vow-clif-shim` stack-slot layout), `parse → print → parse` idempotency, and
  `cargo clippy --all -- -D warnings` are all unaffected. The one adjacent gate that *is* live is
  `workflow-lint.yml` (actionlint + zizmor) against the new YAML — sha-pin the same actions already
  pinned elsewhere in this file, use `persist-credentials: false`, and don't introduce a new
  `permissions:` scope beyond `contents: read`.
- **`scripts/ci_docs_only.py` path list.** `arena-verify.yml` is already in `ARENA_INPUT_FILES`, so
  editing it to add `verify-macos` is already self-gating — no change needed there. Do not add
  `vow-runtime/verify/Makefile`'s comment-only edit as a new classifier input; it's already covered
  by the existing `vow-runtime/` directory prefix.

## 6. Out of scope

- Re-pinning or bumping the ESBMC version as part of this issue — the 8.5 pin is unrelated to
  Darwin memory accounting and changing it would confound the measurement.
- Adding Boolector (or any solver) via Homebrew to `install-esbmc` — confirmed unnecessary; it's
  already statically linked into the macOS release.
- Making the Linux `verify` job's cap or `--unwind` bound conditional on anything — the Linux path is
  proven and stays untouched regardless of what the macOS measurement finds.
- Any refactor of `arena.c`'s assertions, the symbolic-loop bound, or the directed #391/#422–#426
  scenarios — none of that is implicated by which OS runs the proof.
- Wiring the eventual macOS RSS/vmem figures into `vow-perf`'s complexity-class tracking — that
  crate measures compiled-program operation counts, not CI-runner memory, and is an unrelated
  subsystem.
- A general "macOS CI cost" audit (runner minutes, whether `build-and-test-macos` should stay
  `continue-on-error`) — out of scope for this issue; only the arena proof's macOS behavior is in
  play.
