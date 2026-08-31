# Plan — #1136: parity harness never compares error codes, spans, messages, or counterexample values

## 1. Problem restated

`scripts/full_test.sh` contains the only per-PR (Tier 1) check that the Rust bootstrap compiler
and the self-hosted compiler agree, and it compares far less than its two comparator names
suggest: `compare_error` (`scripts/full_test.sh:261-305`, the whole 175-fixture `tests/error/`
suite) asserts only that both compilers exited non-zero, both reported `CompileFailed`, and both
produced at least one diagnostic — it never compares `error_code`, so the two compilers may reject
a program for entirely different reasons and the case passes; `compare_json`
(`scripts/full_test.sh:76-172`, ~309 cases across `examples/`, `tests/run/`, `tests/verify*/`,
`tests/multi/`) compares exit code, `status`, diagnostics **count**, `verify_status`, `function`,
and `counterexamples[].{function, vow_id, blame}`, and never compares `diagnostics[].error_code`,
`.blame`, or `counterexamples[].values` — the last of which is the actual output of the whole
verification pipeline and the payload the CEGIS loop feeds back to a repair agent. Two bugs found
on 2026-08-31 (#1135, and the `<type>` placeholder divergence caught during #1114) both survived
because the field that diverged was never in the compared set; this plan closes the two holes the
issue marks as landable now (items 1 and 2), and leaves items 3 and 4 ordered behind #1135 and
#1113 exactly as the issue requires — both are still **open** as of 2026-08-31, so that ordering
is honoured by omission, not by preference.

## 2. Measured fallout (do not re-derive this — it is the evidence the plan is built on)

Measured 2026-08-31 with `./target/release/vow` (release, `cargo build --all --release`) against a
stage-1 self-hosted binary built by `./target/release/vow build --no-verify` over
`scripts/concat_vow.sh clif` output — the same stage-1 target `scripts/full_test.sh:445` builds for
`$SELF`, which uses `compiler/main.vow` and module loading instead of the concatenation. ESBMC 8.3.0 on `PATH`.

### `compare_error` — sorted multiset of `diagnostics[].error_code`, 175 `tests/error/` fixtures

**12 diverge, 163 agree. All 12 are already recorded in `docs/equivalence/ledger.json`.**

| family | fixtures | Rust | self-hosted | ledger issue |
|---|---|---|---|---|
| linear-type checks phase-shifted into region inference | 9 (`linear_*`) | `LinearTypeViolation` | `RegionLinear` (8) / one fewer `TypeMismatch` (1) | #588 |
| parser error-recovery emits an extra token error | 2 (`match_arm_missing_comma_{block,scalar}`) | 2× `UnexpectedToken` | 3× `UnexpectedToken` | #1088 |

Restricting to `severity == "error"` changes nothing (all 12 still diverge), so there is no reason
to filter by severity.

The 4 synthetic fixtures Section 7 writes into `$TMPDIR` (`parse_error`, `type_error`,
`missing_module`, `const_type_mismatch`) all **agree** — they need no exemption.

One ledger entry is **stale**: `tests/error/undefined_function.vow` is listed with
`observable: "error_code"`, `status: "expected"`, issue #1088, but the two compilers agree on
`['TypeMismatch']` today. It must be flipped to `status: "fixed"` in this PR (see slice 3).

### `compare_json` — the two new comparisons, simulated over every corpus it is called on

**309 cases compared, exactly 2 would newly FAIL.**

| new comparison | cases | new failures |
|---|---|---|
| sorted multiset of `(diagnostics[].error_code, diagnostics[].blame)`, under the existing `status != "VerifyFailed"` guard | 309 | **0** |
| `counterexamples[0].values` restricted to source-named keys | 309 | **2** |

The two failures, both in `tests/verify-fail/`:

- `caller_requires_unchecked.vow` — Rust `{"x": "-1"}` vs self-hosted `{"n": "-1"}`. Filed as
  **#1139**. Both are wrong in different ways; the self-hosted labelling assigns `n = -1` to a
  function whose own `requires: n >= 0` forbids it.
- `off_by_one_bounds.vow` — Rust `{"n": "1", "v": "{ .len=0"}` vs self-hosted
  `{"n": "1", "v": "{ .len=0, .data={ 0, ... } }"}`. Rust truncates the aggregate at the first
  comma, producing unbalanced text. Filed as **#1140**.

### Divergences deliberately left outside the compared set, with evidence

- **`diagnostics[]` under `status == "VerifyFailed"`: 31/31 cases diverge.** Rust emits one
  `VowEnsuresViolated`/`VowRequiresViolated` per counterexample; the self-hosted compiler emits
  `[]`. All 21 `tests/verify-fail/`, all 8 `tests/multi/vmod_*/`, and
  `examples/{cegis_broken,vec_overcount}.vow`. This is what the existing `rs != 'VerifyFailed'`
  guard has been hiding. Filed as **#1138**; the guard stays exactly as-is in this PR and #1138
  owns removing it.
- **`_esbmc_*` keys inside `counterexamples[].values`: 8/21 `tests/verify-fail/` diverge.** The
  divergences are IR value numbering (`_esbmc_v12/_esbmc_v14` vs `_esbmc_v11/_esbmc_v13`), an
  extra self-hosted `_esbmc___vow_heap` family, and the #1140 truncation. `counterexample.schema.json`
  documents these keys as "source names **or ESBMC variable names**", and their numbering is a
  lowering-internal detail that no spec makes a parity contract. Comparing them would assert
  identical IR value numbering across two independent lowerings — a strictly stronger property than
  this issue asks for. Excluded, with the exclusion tested (slice 2).
- **`diagnostics[].message`** — self-hosted prefixes `"in fn main: "`. Needs a spec decision on
  whether message text is a parity contract; not in this PR.
- **`diagnostics[].span`** — behind #1135 (open). **`counterexamples[].violation`** — behind #1113
  (open). **`counterexamples[].source`** — legitimately typed differently per
  `counterexample.schema.json` and tracked by #613.

## 3. Files to touch

Nothing under `compiler/` or any Rust crate changes. This is a test-harness change: **the binary
fixed point, `parse → print → parse` idempotency, and `cargo clippy --all -- -D warnings` are all
untouched by construction.** No `docs/spec/*.md` change is required — no syntax, semantics, type,
builtin, operator, effect, or CLI flag changes.

| path | change |
|---|---|
| `scripts/parity.py` | **new.** The two comparators, extracted verbatim from the shell heredocs, then extended. Importable module + `main()` CLI. |
| `scripts/test_parity.py` | **new.** `unittest` over synthetic JSON. This is where the "deliberately-divergent fixture" acceptance criterion is met. |
| `scripts/full_test.sh` | `compare_json` / `compare_error` bodies replaced by a call to `scripts/parity.py`; both grow an optional fixture-path argument; `compare_json` call sites (11) pass the path they already have in scope. |
| `.github/workflows/ci.yml` | one step, `python3 scripts/test_parity.py`, beside the existing `python3 scripts/test_equivalence.py` (line 85). |
| `docs/equivalence/ledger.json` | `tests/error/undefined_function.vow` → `status: "fixed"` (stale entry, see §2). |
| `docs/equivalence/README.md` | the sentence "it compares diagnostic *counts* rather than error codes" becomes false; replace it, and document the two Tier-1 suppression mechanisms and why they differ. |
| `tests/verify-fail/caller_requires_unchecked.vow` | add `// TEST: known-cex-divergence 1139 "..."`. |
| `tests/verify-fail/off_by_one_bounds.vow` | add `// TEST: known-cex-divergence 1140 "..."`. |

### Why the code moves to `scripts/parity.py`

This is the enabling seam, not a drive-by refactor. The acceptance criterion "a deliberately-divergent
fixture proves each new comparison actually fails when it should" cannot be met by a `.vow` fixture —
a genuinely divergent fixture makes the suite red by construction, which is the outcome the issue
tells us to avoid. It has to be a unit test over synthetic JSON documents, and that needs an
importable comparator. The repo already has exactly this convention: `scripts/equivalence.py` +
`scripts/test_equivalence.py`, run in CI at `.github/workflows/ci.yml:85`, plus four more
`scripts/<mod>.py` / `scripts/test_<mod>.py` pairs. `full_test.sh` is 1582 lines; moving ~140 lines
of Python out of it also serves the "small files, smaller functions" rule in CLAUDE.md.

### Why two different suppression mechanisms

This looks arbitrary and is not; state the reason in the code comment.

- **`error_code` divergences use `docs/equivalence/ledger.json`.** All 12 are already there with
  issue numbers and rationale. Duplicating them as per-fixture comments would create a second
  registry for facts the project already records in one place, and #1136's whole complaint is that
  parity facts live in prose instead of machine-checked artifacts. Tier 1 and Tier 2 pointing at
  the same ledger is the fix in spirit.
- **`counterexamples[].values` divergences use a per-fixture directive.** The ledger's
  `observableName` enum is Tier 2's vocabulary, and `reconcile()`
  (`scripts/equivalence.py:757-813`) reports any tracked observable a run did **not** produce as
  `fixed` and fails on it. Adding a `counterexample_values` member that `equivalence.py` never
  emits would make every Tier-2 run fail. Teaching Tier 2 to emit it is a real change to a
  different tier and belongs in its own PR (follow-up 6). Until then the directive is the correct
  home, and `full_test.sh` already owns exactly this directive shape — `compare_runtime`'s
  `// TEST: known-divergence <issue> "<why>"` (`scripts/full_test.sh:176-236`), including the
  fails-once-fixed rule.

## 4. TDD slices

Each slice is red → green → refactor and leaves the suite green. Run
`python3 scripts/test_parity.py` after every slice; run `bash scripts/full_test.sh` at slices 1, 3
and 5.

### Slice 1 — extract the comparators unchanged (characterization)

- **Test:** `scripts/test_parity.py`, new. Port the *existing* assertions as characterization
  tests before changing any behaviour: exit-code mismatch, `status` mismatch, diagnostics-count
  mismatch under a non-`VerifyFailed` status, soft-`VerifyFailed` (`verify_status` set) requiring
  zero counterexamples and matching `function`, hard-`VerifyFailed` requiring non-empty
  counterexamples on both sides and matching `counterexamples[0].{function, blame}`, the
  `vow_id in (0, -1, None)` both-sides escape hatch, malformed-JSON handling, and for
  `compare_error` the three current assertions. Red first: the module does not exist.
- **Production:** create `scripts/parity.py` with `compare_json(rust, self, rust_exit, self_exit)`
  and `compare_error(...)` returning an errors list, plus a `main()` dispatching on
  `argv[1] in ("json", "error")` that reads two JSON paths and prints `OK` / `FAIL: ...`. Body
  copied verbatim from the two heredocs — **no behaviour change in this slice.**
- **Then** replace both heredocs in `scripts/full_test.sh` with
  `python3 scripts/parity.py json "$rust_f" "$self_f" "$rust_exit" "$self_exit"`, keeping the
  existing `if result=$(...)` / `pass` / `fail` shape. Gate: `bash scripts/full_test.sh` produces
  the same PASS/FAIL/SKIP counts as before the slice — capture them first.

### Slice 2 — `compare_json`: `diagnostics[].error_code` + `.blame`, and `counterexamples[].values`

- **Test (`scripts/test_parity.py`):**
  - a pair whose diagnostics agree on count but differ on `error_code` → FAIL naming both
    multisets. (This is the #1112 case the harness could not re-check.)
  - a pair whose diagnostics agree on `error_code` but differ on `blame` (`"caller"` vs `"callee"`,
    and `"callee"` vs absent) → FAIL.
  - the same pair under `status == "VerifyFailed"` on both sides → **no** diagnostics finding
    (the #1138 guard is deliberate and must be pinned by a test, or a future edit will silently
    remove it).
  - a `VerifyFailed`/`VerifyFailed` pair whose `counterexamples[0].values` differ on a
    source-named key → FAIL.
  - the same pair differing **only** on `_esbmc_*` keys → **no** finding, with a comment naming
    #1140 and the IR-numbering rationale.
  - a pair whose `values` dicts have the same content in different insertion order → no finding
    (Python dict equality is order-insensitive; pin it so nobody "fixes" it into a list compare).
  - a directive-bearing fixture path whose `values` diverge → SKIP outcome, not FAIL.
  - a directive-bearing fixture path whose `values` **agree** → FAIL, "no longer reproduces —
    remove the directive".
- **Production (`scripts/parity.py`):**
  - replace the diagnostics-count check with a sorted multiset of `(error_code, blame)` tuples,
    under the **unchanged** `rs != 'VerifyFailed'` guard. The multiset subsumes the count, so the
    count check is deleted, not kept alongside.
  - add `values` (source-named keys only, i.e. keys not starting with `_esbmc`) to the `rc[0]`
    field comparison in the hard-`VerifyFailed` branch, and to the per-index field loop in the
    `else` branch.
  - accept an optional fixture path; read `// TEST: known-cex-divergence <issue> "<why>"` from it
    with the same semantics as `compare_runtime`'s directive: reproduces → SKIP naming the issue;
    no longer reproduces → FAIL demanding removal. Scope the directive to the `values` check only —
    a directive-bearing fixture that starts diverging on `status` or `blame` must still FAIL, the
    same rule `compare_runtime` enforces for exit codes.
- **Then** thread the fixture path through `compare_json` in `scripts/full_test.sh` as an optional
  6th argument and pass it at all 11 call sites (each already has `$vow_file`, `$main_file`, or
  `$fixture_path` in scope), and add the two directives to
  `tests/verify-fail/{caller_requires_unchecked,off_by_one_bounds}.vow`.
- **Expected result:** `bash scripts/full_test.sh` green, with 2 new loud SKIPs.

### Slice 3 — `compare_error`: sorted multiset of `diagnostics[].error_code`, ledger-aware

- **Test (`scripts/test_parity.py`):**
  - both reject with different codes, path untracked → FAIL naming both multisets.
  - both reject with the same codes but different **counts** (`['UnexpectedToken']×2` vs `×3`) →
    FAIL. This is the `match_arm_missing_comma_*` shape; a set comparison would miss it, so pin
    the multiset.
  - a path with a ledger entry `{observable: "error_code", status: "expected"}` and diverging
    codes → SKIP naming the issue number.
  - the same path with **agreeing** codes → FAIL, "no longer diverging — update
    docs/equivalence/ledger.json". (This is `reconcile()`'s GAP_FIXED discipline, and the reason
    the stale `undefined_function.vow` entry must be corrected.)
  - a ledger entry with `status: "fixed"` and diverging codes → FAIL, not SKIP (a `fixed` entry is
    retained precisely so a reappearance reads as a regression).
  - a ledger entry tracking only `runtime` and diverging codes → FAIL (matching is on
    `(file, observable)`, never on the path alone).
  - an absolute path outside the repo (the `$TMPDIR` synthetic fixtures) → never consults the
    ledger, strict comparison.
- **Production (`scripts/parity.py`):** `from equivalence import load_ledger, tracked_observables,
  REPO_ROOT` — `equivalence.py` is import-safe (`main()` is behind `if __name__ == "__main__"`, no
  import-time side effects) and both helpers are already unit-tested. Do **not** copy them. Resolve
  the fixture path with `os.path.relpath(os.path.abspath(p), REPO_ROOT)` and treat any result
  starting with `..` as untracked. Suppression applies only when
  `entry["status"] in ("open", "expected")` **and** `"error_code" in tracked_observables(entry)`.
- **Then** pass `"$fixture_path"` from Section 7 (`scripts/full_test.sh:1135`), and flip
  `tests/error/undefined_function.vow` to `status: "fixed"` in `docs/equivalence/ledger.json`.
- **Expected result:** `bash scripts/full_test.sh` green, with 12 new loud SKIPs across
  `tests/error/` (9× #588, 2× #1088, and none for `undefined_function`).

### Slice 4 — CI wiring and docs

- Add `python3 scripts/test_parity.py` to `.github/workflows/ci.yml` beside the equivalence-runner
  step (line 85). Verify locally with `python3 scripts/test_parity.py`.
- `docs/equivalence/README.md`: replace "it compares diagnostic *counts* rather than error codes"
  (now false), and add a short "Tier-1 suppressions" subsection stating that `error_code`
  exemptions live in `ledger.json` and `counterexamples[].values` exemptions live as
  `// TEST: known-cex-divergence` directives, with the `reconcile()` reason for the split.

### Slice 5 — full gate

Run as separate commands, never `&&`-chained:

```
python3 scripts/test_parity.py
python3 scripts/test_equivalence.py
cargo clippy --all -- -D warnings
cargo test --all
bash scripts/full_test.sh
scripts/bootstrap.sh --skip-cargo
```

`cargo clippy` and `cargo test` are unchanged-by-construction but are the project's stated gate.
`scripts/bootstrap.sh --skip-cargo` must still reach the SHA-256 fixed point (it will — no
`compiler/` source changes).

## 5. Verification surface

**None.** This change touches no contract, no codegen path, and no C model. ESBMC is invoked only
as it already is, by `full_test.sh` running the existing `tests/verify*` corpora — no new
properties are asked of it and no `--unwind`/`--timeout` budget changes.

No new fixtures under `tests/run/` or `examples/`. The acceptance criterion's
"deliberately-divergent fixture" is deliberately **not** a `.vow` file: a genuinely divergent
`.vow` fixture would make `full_test.sh` red by construction. It is instead a synthetic JSON pair
in `scripts/test_parity.py`, one per new comparison, which is what actually proves the comparator
fails when it should.

Two existing fixtures gain a comment line
(`tests/verify-fail/{caller_requires_unchecked,off_by_one_bounds}.vow`). `//` line comments are
stripped at lex time, so neither the verification result nor the canonical-print idempotency of
those files changes.

## 6. Risk areas

| risk | assessment |
|---|---|
| **Binary fixed point** (`compiler/` codegen ordering, `BTreeMap` vs `HashMap`, `vow-clif-shim` stack-slot layout) | Not at risk. No `compiler/*.vow` and no Rust crate is modified. `scripts/bootstrap.sh` and `scripts/concat_vow.sh` are untouched. Still verified in slice 5. |
| **`parse → print → parse` idempotency** | Not at risk. The only `.vow` edits are two `//` comment lines, stripped at lex time. |
| **`cargo clippy --all -- -D warnings`** | Not at risk. No Rust changes. Still run in slice 5. |
| **Regression from the extraction itself** | The real risk of this PR. Mitigated by slice 1 being a pure verbatim move with characterization tests written first, and by capturing `full_test.sh`'s PASS/FAIL/SKIP counts before and after that slice. Do not fold slices 1 and 2 into one commit. |
| **`$TMPDIR` paths reaching the ledger lookup** | Section 7 feeds `compare_error` four absolute `$TMPDIR` paths. The `os.path.relpath` + `..` guard is the defence; it has its own test in slice 3. A bug here would *suppress* a real divergence, which is the exact failure mode this issue exists to remove. |
| **`full_test.sh` runtime** | Unchanged. Same number of `python3` processes; the ledger JSON parse per `compare_error` call (179 calls) is a ~7 KB read. |
| **The suite hangs rather than fails** | Pre-existing and now known: #1141 (self-hosted parser loops forever on `fn f( -> i32 { 0 }`). `run_self` has a `ulimit -v` but no wall-clock timeout. Not fixed here; noted in #1141. |
| **Over-broad SKIPs hiding a regression** | 14 new SKIPs is a real cost. Each names an issue number, each hard-FAILs once the divergence stops reproducing, and Tier 2 (`scripts/equivalence.py`, nightly) independently reconciles the ledger. |

## 7. Out of scope — deliberately not bundled

- **Fixing any of the divergences the new comparisons expose.** #1138, #1139, #1140 are compiler
  bugs; fixing them in a test-harness PR would bundle a compiler change into a harness change.
- **#1141** (self-hosted parser hang) — found incidentally while measuring, unrelated to the
  comparators.
- **`diagnostics[].span`** — issue item 3, ordered behind #1135, which is open. **That ordering is
  load-bearing.** Adding it now turns the whole `tests/error/` suite red.
- **`counterexamples[].violation`** — issue item 4, ordered behind #1113, which is open. Same
  reason.
- **`counterexamples[].source`**, **`.execution_path`**, **`.branch_decisions`**,
  **`.call_sites`**, **`.violating_args`** — `source` is typed differently by design per
  `counterexample.schema.json` (#613); the rest are Rust-only or need a spec decision first.
- **`diagnostics[].message` / `.hints` / `.secondary`** — needs a decision on whether diagnostic
  prose is a parity contract. Self-hosted prefixes `"in fn main: "` and emits `line`/`column`
  fields the Rust side does not.
- **Schema staleness.** `docs/spec/schemas/diagnostic.schema.json` is missing 7 emitted error
  codes and forbids `blame`/`hints`/`secondary` via `additionalProperties: false`; the
  `CompileFailed` `message` field is absent from self-hosted output though the schema requires it.
  Already tracked by #611, #612, #616, #652. Touching them here would widen a harness fix into a
  spec change.
- **Teaching `scripts/equivalence.py` a `counterexample_values` observable** so Tier 1 and Tier 2
  share one registry — the right end state, but a change to a different tier.
- **Any reformatting, renaming, or restructuring of `scripts/full_test.sh`** beyond replacing the
  two heredocs and threading one optional argument.

## 8. Follow-ups this PR should reference, not do

| # | what | unblocks |
|---|---|---|
| #1138 | self-hosted emits no `diagnostics[]` on `VerifyFailed` | removing the `rs != 'VerifyFailed'` guard, so `error_code`/`blame` are compared on all 309 cases |
| #1139 | caller-blame `values` name different variables | removing the `caller_requires_unchecked.vow` directive |
| #1140 | Rust truncates aggregate `values` at the first comma | removing the `off_by_one_bounds.vow` directive; also the `_esbmc_*` half |
| #1141 | self-hosted parser hangs on a malformed parameter list | a wall-clock timeout around `run_self` |
| #1135 | self-hosted parser drops spans on 23 expression kinds | issue item 3: `span.offset` / `span.length` |
| #1113 | self-hosted cast printer ` as <type>` placeholder | issue item 4: `counterexamples[].violation` |
| — | `equivalence.py` gains a `counterexample_values` observable | one registry for both tiers; retires the directive |

## 9. PR

Squash-merge only; the PR title is the commit message that lands on `main` and must satisfy
Conventional Commits (lower-case subject, no trailing period, header ≤ 100 chars **including** the
` (#N)` GitHub appends — so ~92 chars to work with).

Proposed title:

```
test(parity): compare error codes, diagnostic blame, and counterexample values
```

The implementation stage `git rm`s this `PLAN.md` before opening the PR.
