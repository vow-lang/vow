# Plan: issue #1329 — `bind_arm_pattern` mutant 3897 survives

## 1. Problem restated (with the answer the issue asked for)

Mutant 3897 (`compiler/checker.vow:2012:54`, `<=` → `>` in the `pat_*` padding loop of
`bind_arm_pattern`) was reported `missed` at Tier 2, and the issue could not tell whether the
buggy path is unreachable (explanations 1/2) or something about the oracle hides it (3).

**It is explanation 3, and it was reproduced live in this planning run** (scratch copies under
`$TMPDIR/repro`, nothing in the tree touched; unmutated vs. mutated `compiler/` built with the
installed self-hosted compiler using an isolated `VOW_CACHE_DIR`):

* The mutated compiler **does crash**: `{"error":"IndexOutOfBounds"}`, exit **134**, empty stdout,
  on e.g. `tests/run/btreemap_enum_value.vow`, `match_option_payload.vow`,
  `match_aggregate_phi_metadata.vow`, `linear_match_binding_shadow.vow`. So the runtime reading in
  the issue is right, and the branch *is* reached with a growth-needed `pid`
  (the arrays start empty, `pid` is an arena-wide id, so `len <= pid` on the first hit; with
  `>` the loop never runs and the indexed store aborts. If `len > pid` it would instead loop
  forever — either way the branch is broken, there is no benign case).
* Census over `tests/run/*.vow`, `tests/error/*.vow`, `examples/*.vow` (433 files, `build
  --no-verify`, 2 GB `ulimit -v`, as `full_test.sh` runs them): unmutated compiler → **0** empty
  outputs; mutated compiler → **19** empty outputs, all exit 134:
  `tests/run/` {btreemap_enum_value, btreemap_option_value, linear_match_binding_shadow,
  linear_match_catchall_transfer, linear_match_exhausted_payload_catchall,
  linear_question_payload, match_aggregate_phi_metadata, match_option_payload,
  match_supported_catchalls, nested_option_payload_width} and `tests/error/`
  {linear_alias_option_payload_duplicate, linear_enum_payload_duplicate,
  linear_empty_variant_phi_duplicate, linear_match_catchall_remaining_payload,
  linear_match_payload_partial_duplicate, linear_match_payload_return_after_consume,
  linear_option_payload_duplicate, match_enum_variant_payload_arity,
  non_linear_struct_linear_owner_field}. (Scratch data is under `$TMPDIR`, which is cleaned up —
  this list is the durable record; re-derive with the same census if needed.)
* **Why the oracle says `missed`:**
  1. *Tier 1* (`bootstrap.sh --skip-cargo`) cannot see it: the compiler's own source has no
     `match` arm binding an aggregate payload (its `match`es are `Option<i64>`/`Result` scalar
     payloads), so the mutated stage-1 compiler never reaches the branch. Confirmed: the mutated
     compiler compiling the unmutated tree exits 0 and yields a binary **byte-identical to the
     unmutated build** (`cmp` equal).
  2. *Tier 2* (`full_test.sh`) does run the mutated self-hosted compiler over those 19 fixtures,
     but every "build with both compilers" site does
     `if [ -z "$rust_json" ] || [ -z "$self_json" ]; then skip "…/empty output…"; continue`
     (`scripts/full_test.sh:249-251` Section 4, `:346-349` Section 7, and 8 more sites). A crash of
     the self-hosted compiler produces exactly "empty stdout" and is therefore a **SKIP**, which
     does not fail the run (`print_summary` returns non-zero only for `FAIL > 0`). No other
     Tier 2 section covers it: the unit-test suite (`vowc test compiler/`, Section 10b, which *does*
     hard-fail via `compare_test` → `status == TestsPassed`) has no test that exercises
     `bind_arm_pattern`'s `PAT_IDENT` aggregate branch.

**Probe: Section 10b is not subject to the same masking.** Section 10b also has an
empty-output→SKIP guard (`full_test.sh:1516`), so the unit-test route only works if a test binary
that *aborts* still yields non-empty `vow test` JSON. Verified with the installed self-hosted
compiler on a scratch copy of `compiler/` plus a probe test whose `main` does an OOB `v[0] = 5`:
`vow test <copy> --filter zzprobe` → exit 1, stdout
`{"status":"TestsFailed","total":1,"passed":0,"failed":1,…,"exit_code":134,"stderr":"{\"error\":\"IndexOutOfBounds\"}…"}`.
So a crashing unit test is a `TestsFailed` document → `compare_test` hard-fails
(`status != TestsPassed`), not a SKIP. The Rust-side `vow test` was not built in the planning run;
Slice 1 must repeat this probe with `target/release/vow` (see §3) — if it ever yields empty stdout
for an aborting test, the unit test alone cannot close #1329 and the closing slice must also
harden the empty-output guard at that single Section 10b site (keep the other sites in the
follow-up).

So mutant 3897 is **not equivalent**; per CLAUDE.md "Mutation Testing" the actionable response is
(a) a test that catches it. A separate, bigger finding — Tier 2 silently downgrades a
self-hosted-compiler crash on the fixture corpus to SKIP — is real but is a different defect
(CI harness policy); it is split into a follow-up issue instead of bundled here.

**Scope of this PR:** one new self-hosted unit test (`compiler/tests/`), plus issue bookkeeping.
No production code, no Rust code, no spec change.

## 2. Files to touch

| Path | Change |
|---|---|
| `compiler/tests/test_checker_pattern_metadata.vow` | **new** — direct unit test of the `pat_*` sparse-array metadata written by `bind_arm_pattern` (and, slice 2, the `question_*` twin). |
| *(nothing else)* | No `compiler/checker.vow`/`env.vow`/`lower.vow` edit. No Rust crate edit. No `docs/spec/*` edit (no syntax/semantics/CLI change). |

Both-compilers rule (CLAUDE.md "Vow Compiler"): satisfied without a Rust edit — the change is a
test *program* in `compiler/tests/`, and `vow test compiler/` (Rust) and `build/vowc test compiler/`
(self-hosted) both compile and run it; `full_test.sh` Section 10b requires both to report
`TestsPassed` with matching counts. State this explicitly in the PR body so the Rust-side
absence is not read as drift.

Follow-up issues to file (via `gh issue create`, not part of this PR): see §6.

## 3. TDD slices

Discovery is by file: `vow test compiler/` runs every `compiler/tests/test_*.vow` `main` (exit 0 =
pass; return distinct non-zero codes per check, as `test_checker_float_literal.vow` does). Model the
new file on `compiler/tests/test_checker_float_literal.vow` (`parse_module_into` → `env_new` →
`check_module`, then inspect the `CheckEnv`).

Fast red/green loop without touching the tree: copy `compiler/` to `$TMPDIR/…`, apply the mutation
there with `sed` (`sed -i '2012s/ <= pid {/ > pid {/'`), and run
`target/release/vow test <copy> --filter checker_pattern_metadata` (and `~/.local/bin/vow test …`
for the self-hosted side). **No `vowc` rebuild is needed per slice**: the mutation lives in the
source under test, pulled into the test binary via `use checker`, so `vow test <mutated copy>`
already links the mutated loop. Use a fresh `VOW_CACHE_DIR=$(mktemp -d -p $TMPDIR)` per run (the
compile cache ignores compiler changes — see memory note). Gotcha for the assertions: chained
field access on struct values reads the wrong field (CLAUDE.md "Gotchas"), so bind first —
`let names: Vec<String> = check_env.pat_aggregate_names;`, `let paths: Vec<Vec<String>> =
check_env.pat_vec_elem_paths;`, etc. — before indexing.

**Slice 0 — Rust-side probe (no commit).** Build the Rust compiler (`cargo build --release -p vow
-j4`) and repeat the §1 abort probe with `target/release/vow test <scratch copy> --filter
zzprobe`. Expect non-empty `TestsFailed` JSON. If stdout is empty, stop and apply the
single-site Section 10b guard described in §1 before continuing.

**Slice 1 — pin the PAT_IDENT aggregate metadata (kills 3897).**
* Test: `check_pat_ident_aggregate_metadata()` in the new file. Source under test (mirror
  `tests/run/match_option_payload.vow` / `match_supported_catchalls.vow` for accepted syntax):
  a `struct Box { v: i64 }`, `enum Payload { Boxed(Box), Empty }`, and one `fn` whose `match` has
  a qualified-variant arm binding the aggregate payload (`Payload::Boxed(b) => …`) **and** a final
  bare-identifier catch-all arm (`other => …`) over the enum scrutinee. Both hit
  `bind_arm_pattern`'s `PAT_IDENT` + non-empty `pattern_aggregate_name` branch; the arena is
  guaranteed non-empty before them, so their `pid` is `> 0` and the padding loop must run.
* Assertions (distinct return codes): checker reports 0 errors; `pat_aggregate_names` contains a
  non-empty `"Box"` entry and a non-empty `"Payload"` entry; the seven parallel arrays
  (`pat_aggregate_names`, `pat_vec_elem_paths`, `pat_vec_option_elem_ty_paths`,
  `pat_vec_variant_payload_ty_paths`, `pat_option_elem_tys`, `pat_variant_payload_tys`,
  `pat_aggregate_linear`) have equal length (the invariant `lower.vow:776`'s
  `pid < len` guard relies on); every placeholder slot (empty name) has `-1` / `0` / empty-Vec
  values, i.e. the sparse-array contract.
* **Red first:** run against the mutant copy (`2012: <= → >`) → the test binary must abort
  (`IndexOutOfBounds`, non-zero) and the suite reports failure; **green** on the unmutated tree.
  Also confirm the test is red for two neighbouring mutants of the same block that the assertions
  are meant to pin (e.g. padding `push(-1)`/`push(0)` const-flips) — keep only assertions that
  demonstrably fail on some mutant; drop the rest (no speculative asserts).
* Production code: none. The "green" step is just the test passing on the existing checker.

**Slice 2 — cover the `question_*` twin (`record_question_aggregate`, `checker.vow:1982-1999`).**
Same padding idiom, same `<= pid` loop at line 1987, reached by `?`/`.unwrap()` on an
aggregate-payload `Option`/`Result`. Extend the same test with a `fn` doing
`let b: Box = opt.unwrap();` (check `tests/run/unwrap_some_payload.vow` for the accepted shape) and
assert `question_aggregate_names` has a non-empty `"Box"` entry and the five `question_*` arrays
have equal length. **Gate:** first reproduce that the sibling mutant (`1987: <= → >`) actually
makes the new assertions fail in a scratch copy; if it does, keep the slice; if the sibling is
already killed by an existing unit test or cannot be made red, drop slice 2 (no test that cannot
fail). Keep it a separate commit so it can be dropped cleanly.

**Slice 3 — full gate + bookkeeping (no code).**
* Run the gate as separate commands (never `&&`-chained): `cargo build --release -p vow -j4`,
  `scripts/bootstrap.sh --skip-cargo`, `target/release/vow test compiler/`,
  `build/vowc test compiler/`, then `scripts/full_test.sh` (Section 10b must show
  `test/parity` and `test/filter` passing — see §5 for the `--filter arith` trap).
  Cap build parallelism (`-j4`); scratch under `$TMPDIR`.
* Before `git rm PLAN.md`, commit the test with a Conventional-Commit message, e.g.
  `test(checker): pin bind_arm_pattern aggregate-binding metadata growth`.
* PR title (squash subject, ≤ 92 chars, lower-case subject):
  `test(checker): pin bind_arm_pattern aggregate-binding metadata growth`. Body: `Closes #1329`,
  the evidence table from §1, and the note that no Rust change is needed (tests run under both
  compilers).
* `gh issue comment 1329` with the findings from §1 (mechanism = Tier-2 skip masking + Tier-1
  blind spot; mutant is real; fixed by unit test) and the exact red/green commands, so #1326's
  "more mysterious than it is" note is resolved. Also comment on #1326 with a one-line pointer.
* File the follow-up issue(s) from §6.

## 4. Verification surface

* **ESBMC / contracts / C model:** none. The change is a test program; no `vow { }` blocks are
  added, no contracts written or weakened, no codegen/IR/C-emitter change. (`vowc verify` is not run
  over `compiler/tests/`.) `scripts/check_contract_quality.py` is unaffected (no contracts added);
  the `contract_density` parity field in Section 10b changes identically for both compilers.
* **Fixtures:** no new `tests/run/` or `examples/` fixture. Existing fixtures already exercise the
  path but are masked by the skip; the unit test is the direct, hard-failing pin. (Adding another
  fixture would still be swallowed by the same skip and would not help.)
* **Mutation proof:** demonstrate red on mutant 3897 (and 1987 if slice 2 lands) in scratch
  copies, green on the tree; record the commands and outcomes in the PR body. A full `vowc mutants
  run` is not needed (there is no per-mutant-id selector; the shard sweep is multi-hour and local
  only).

## 5. Risk areas

* **Binary fixed point / determinism:** none — no `compiler/*.vow` production module changes, so
  the stage-2/3 binaries are unaffected. (Adding a file under `compiler/tests/` is not part of the
  `concat_vow.sh` bootstrap input.)
* **`parse → print → parse` idempotency:** N/A (no parser/printer change), but the embedded source
  strings in the test must be valid Vow the current parser accepts; copy syntax from an existing
  passing fixture rather than inventing it.
* **`Section 10b test/filter` trap:** it asserts `--filter arith` selects exactly 1 test. Do not
  put `arith` in the new file/test names.
* **`test/parity` (Section 10b):** requires Rust and self-hosted `vow test compiler/` to agree on
  totals and per-test entries. A new file adds one entry to both; no divergence expected. If the
  Rust compiler mis-handles something in the test, that is a real parity bug to report, not to
  paper over.
* **Test fragility:** the test inspects `CheckEnv.pat_*` internals. That couples it to the field
  layout, which is deliberate (the mutant lives in that write) — keep it to the seven arrays and
  the `question_*` five, no other internals. Do not hard-code a specific `pid` value (arena ids
  shift with unrelated parser changes); find populated slots by scanning.
* **Hang vs. crash:** on the mutant, `len > pid` would loop forever if it were ever true; in the
  test the arrays are empty on the first hit so it aborts immediately. Still run the mutant red
  check under `timeout 120` and `ulimit -v 2000000` so a hang cannot wedge the run.
* **clippy gate:** untouched (no Rust changes). **Codecov patch gate:** no Rust/instrumented lines
  changed.
* **Environmental noise:** ~8 `vow` crate run tests SKIP-panic in the sandbox and concurrent
  bootstraps can flake `cargo test` (see memory notes); verify any red against clean `origin/main`
  before blaming this change. Diff against `origin/main`, not the stale local `main`.

## 6. Out of scope (deliberately not bundled)

* **Harden `scripts/full_test.sh` so "exactly one compiler produced no output" is a FAIL, not a
  SKIP** (sites: Sections 1, 4, 4-verify-only, 6, 6b, 7 and the verify sections 2/4b/4c/4d/10).
  This is the real root-cause hazard exposed here (any self-hosted crash on the fixture corpus is
  invisible to the CI-gating harness — 19 fixtures in this reproduction), but it is a CI-policy
  change touching ~10 sites with unknown Rust-side asymmetries, and per "surgical changes" it gets
  its own issue/PR. File it with the census evidence; its own plan should (i) run a
  both-compilers census on clean `main` first, (ii) keep both-empty as SKIP, (iii) put the
  classification in `scripts/parity.py` with tests in `scripts/test_parity.py` (already CI-run),
  and (iv) decide per section whether verify-mode sites (ESBMC availability) may stay lenient.
* **Deduplicate the sparse-array growth idiom** (`pat_*` ×7 and `question_*` ×5 hand-rolled
  `while len <= id { push placeholder… }` loops) into one `ensure`-style seam. Legitimate
  deepening candidate, but a refactor; not a bug-fix bundle. Only proposable if expressible in Vow
  (both compilers must move together per CLAUDE.md).
* Any change to `__vow_vec_set_val` / runtime OOB behaviour (already correct: hard abort).
* Any change to `docs/mutants.md`, `scripts/bootstrap.sh`, or the mutants oracle (e.g. making
  Tier 2 treat SKIP-count increases as caught) — covered by the harness follow-up above.
* Tests for other surviving mutants from #1326 (`c_emitter.vow` 461/1427/1916) — tracked there.
