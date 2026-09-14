# Plan: #1277 — tag `proc_sample`'s result as heap-returning in Rust's `tag_builtin_result`

## 0. Status at this attempt (reused workspace)

This workspace was reused from an earlier attempt. Slices 1, 2, and 4 below are **already
committed** on this branch, ahead of this plan commit:

- `643d6f53` `fix(lower): tag proc_sample's result as heap-returning` — the one-line
  `tag_builtin_result` fix (slice 2) plus the `pin_to_root_proc_sample_lowers_to_string_pin` unit
  test (slice 1).
- `40c93815` `test(run): add proc_sample pin_to_root regression fixture` — the
  `tests/run/proc_sample_pin_to_root.vow` fixture (slice 4). Its commit message records that no
  runtime divergence was empirically observed for this builtin (freshly-owned allocation, not
  reused scratch), so the fixture is a regression pin rather than a crash reproduction — this
  resolves slice 4's "empirical answer" requirement.

(The run context's "Attempt: 1" doesn't match this history — the prior commits predate this
invocation. Not reconciling that; recorded here so a reviewer isn't confused by the timeline.)

**Nothing here has been pushed and no PR exists yet.** The implementation stage's remaining job
is *not* to redo slices 1/2/4 — do not re-add the `| "proc_sample"` arm or duplicate either test,
that would conflict/no-op against the existing commits. Instead it must:

1. Run the gates below (unverified by this planning pass — see the checks in §3a) and fix anything
   that fails.
2. `git rm PLAN.md`.
3. `git fetch origin main` and diff against `origin/main` (not bare `main` — the worktree's local
   `main` ref is known stale in this repo).
4. Push and `gh pr create` non-interactively with a lower-case, ≤92-char Conventional Commits
   title (squash-merge only, per repo policy).

### Gates to run before pushing (none of these were verified by this planning pass)

Run each separately, not `&&`-chained:

```bash
cargo test -p vow-ir
cargo clippy --all -- -D warnings
cargo fmt --all -- --check
```

Then build+run `tests/run/proc_sample_pin_to_root.vow` the way `scripts/full_test.sh` Section 4
does (fresh cache to avoid stale-object drift): `VOW_CACHE_DIR=$(mktemp -d) build/vowc build
--no-verify tests/run/proc_sample_pin_to_root.vow -o /tmp/proc_sample_pin && /tmp/proc_sample_pin`
— expect stdout `1`.

## 1. Problem restated

`vow-ir/src/lower/mod.rs::tag_builtin_result` (mod.rs:190-239) tags certain builtins' call
results with a heap-shape marker (`ctx.inst_struct_type` / `ctx.inst_option_elem_ty`) that later
lowering-time logic in the same module consults. `proc_sample` returns `Str`
(`vow-types/src/env.rs:216`) and is tagged `"String"` in the self-hosted mirror
(`compiler/lower.vow:2546`), but is missing from the Rust `"String"` arm (mod.rs:199-203). There
are two lowering-time consumers of `inst_struct_type` that could plausibly be affected by this —
but they are not equally affected, and the plan below is careful not to overclaim beyond what's
verifiable by reading the code:

- **`pin_to_root(proc_sample())`** (mod.rs:1713-1758): with no tag on `source_id`, both the
  `"String"` and `"Vec"` branches are skipped and the function falls through to
  `return source_id` (mod.rs:1758) — `pin_to_root` becomes a genuine no-op. This path consults
  `inst_struct_type` exclusively, with no fallback, so this **is** a confirmed real divergence.
  Its runtime consequence is more subtle than a classic scratch-buffer use-after-free, though:
  `__vow_proc_sample` (`vow-runtime/src/lib.rs:3301-3316`) returns a **freshly-owned** allocation
  via `__vow_string_new`, unlike e.g. `stdin_read_line`'s reused scratch buffer (which is exactly
  why `pin_to_root` matters for *that* builtin, per `compiler/main.vow:4927`). `pin_to_root`'s
  emitted `__vow_string_pin_to_root` call is also what the region/escape pass recognizes as an
  intrinsic root marker (`vow-ir/src/region.rs:2838`'s comment on `Root` sources) — but a targeted
  grep of `inst_struct_type` across `region.rs` returns zero hits, so the region pass never reads
  the struct-type tag itself; it reacts to the *presence of the pin call*. Skipping the pin call
  means the region pass never sees that marker for this call site, so whatever the *default*
  region-placement heuristic assigns to an untagged direct builtin-call result silently applies
  instead of the user's explicit root-placement request. Whether that default already happens to
  be root-equivalent for a freshly-owned allocation (harmless no-op) or is scope/arena-local
  (a real escape risk once the value crosses a scope boundary) is **not determined by reading code
  alone** — this is exactly the "not yet confirmed... worth investigating" TODO the issue calls
  out, and the implementation stage should resolve it empirically (see TDD slice 4) rather than
  this plan asserting an outcome.
- **`proc_sample().<method>()`** (mod.rs:3485-3891): `recv_struct` (mod.rs:3491) is looked up from
  `inst_struct_type`, falling back to `ctx.string_exprs`. Checked against
  `vow-types/src/check.rs:1922-1928`: `string_exprs` is populated by `check_expr` for **every**
  expression whose checked type is `Ty::Str`, keyed by AST-node pointer identity — not just
  literals. Since `proc_sample` statically returns `Ty::Str` and the real pipeline
  (`vow/src/frontend.rs`) hands the *same* AST nodes from checker to lowerer, `proc_sample()`
  call-expr receivers are already in `string_exprs` in production, so `recv_struct` resolves to
  `Some("String")` via the fallback regardless of the `tag_builtin_result` bug. **This path is not
  actually broken in production.** The apparent divergence only shows up if a unit test calls
  `lower_function` directly with an empty `string_exprs` set (as the existing tests in this file
  already do) — that would be a test-harness artifact, not a real compiler bug, so this plan does
  not add a regression test for it.

So there is exactly **one** confirmed real Rust/self-hosted divergence in production: the
`pin_to_root(proc_sample())` no-op. The self-hosted side already has the correct tag and needs no
production change; this plan's fix is Rust-only.

## 2. Files to touch

- **`vow-ir/src/lower/mod.rs`** — production fix (one line: add `"proc_sample"` to the
  `"String"`-tagged match arm at mod.rs:199-203) + one new regression test in the existing
  `#[cfg(test)] mod tests` block, placed next to `pin_to_root_process_stdout_lowers_to_string_pin`
  (mod.rs:7804-7849), reusing the same helpers (`call_expr`, `make_fn`, `string_ty`, `sp`).
- **`compiler/lower.vow`** — no production change (already tags `proc_sample` correctly at
  line 2546). Confirmed via `grep`; nothing to edit.
- **`docs/spec/*.md`** — no update required. `proc_sample` does not appear anywhere in
  `docs/spec/` (confirmed via grep) — it is an internal tracing-only builtin (consumed by
  `compiler/perfetto.vow`), not part of the documented public builtin surface, and this fix
  changes no signature, syntax, semantics, or CLI flag. `tag_builtin_result` is lowering-internal
  bookkeeping, not observable language behavior.
- **`CHANGELOG.md`** — no manual edit; semantic-release generates it from the Conventional
  Commits PR title.

## 3. TDD slices

1. **Red: `pin_to_root_proc_sample_lowers_to_string_pin`** (new test in
   `vow-ir/src/lower/mod.rs`, adjacent to `pin_to_root_process_stdout_lowers_to_string_pin`).
   Build `pin_to_root(proc_sample())` as the trailing expr of a fn returning `Str` with effect
   `IO` (mirror the existing test's `make_fn`/`call_expr` shape exactly, swapping
   `"process_get_stdout"` → `"proc_sample"` and `"__vow_process_get_stdout"` →
   `"__vow_proc_sample"`). Assert the lowered instructions contain
   `InstData::CallExtern("__vow_proc_sample")` **and**
   `InstData::CallExtern("__vow_string_pin_to_root")`. Today this fails: the pin call is absent
   because `tag_builtin_result` doesn't tag `proc_sample`, so lowering falls through to
   `return source_id` and no pin call is ever emitted.
   Green: adding `"proc_sample"` to the `"String"` match arm (slice 2) makes it pass.

2. **Green: production fix.** In `tag_builtin_result` (mod.rs:190-239), add `"proc_sample"` to
   the `"String"`-tagged arm's pattern list (mod.rs:199-203), i.e. extend the `|`-chain that
   already includes `"process_get_stdout" | "process_get_stderr" | "process_stdout_for" |
   "process_stderr_for"` with `| "proc_sample"`. Re-run slice 1: passes. Re-run the full
   `vow-ir` test suite (`cargo test -p vow-ir`) to confirm no other test asserts the old (buggy)
   fall-through behavior for `proc_sample`.

3. **Do not add a test for the method-call-dispatch path.** As established in §1, that path is
   already correct in production via the `string_exprs` fallback; a test built directly against
   `lower_function` with an empty `string_exprs` set would only be pinning a test-harness
   limitation, not a real bug, and would misrepresent the issue's scope if read later as "this was
   broken too." If the implementer wants extra confidence here, the right check is to confirm (by
   inspection, not a new permanent test) that `check.rs:1922-1928`'s `check_expr` really does run
   on every `proc_sample()` call site before lowering in the real `vow/src/frontend.rs` pipeline —
   not to add IR-level test infrastructure for a non-bug.

4. **Optional empirical slice (do only if slices 1-2 land cleanly and time remains):** resolve the
   "not yet confirmed" TODO from §1 by adding `tests/run/proc_sample_pin_to_root.vow`, exercising
   `pin_to_root(proc_sample())` across a scope boundary (return it from an inner block/function and
   use it in the caller), matching the existing `tests/run/fs_read_line_pin_to_root.vow` /
   `pin_to_root_string.vow` fixture style. **Constraint, see Risk Areas below:**
   `proc_sample()`'s content is live `/proc` data and is nondeterministic across runs and across
   the two compilers' separately-built binaries — the fixture must not assert on its literal
   content via `// TEST: stdout`. Instead assert only a structural invariant that doesn't depend
   on the sampled values, e.g. `print_i64(if pinned.len() > 0 { 1 } else { 0 })`, plus
   `// TEST: exit 0`, so `compare_runtime`'s cross-compiler stdout diff (`scripts/full_test.sh`
   Section 4) stays stable. Build the fixture with `--no-verify` **before** the fix lands (on a
   throwaway branch or via `git stash`) to see whether the no-op is actually observable (e.g. via
   a debug-mode crash, ASAN if available, or a wrong length after crossing the scope) — record
   whatever is found as a one-line note in the PR description, since that's the empirical answer
   the issue asks for. If nothing distinguishes tagged from untagged behavior at runtime, say so
   explicitly rather than silently dropping the fixture; keep it anyway as a straightforward
   regression pin for slice 2's fix, unconditional on what was found.

## 3a. Fixture review (checks run during this planning pass, not the gates above)

The committed `tests/run/proc_sample_pin_to_root.vow` was spot-checked against the codebase
rather than executed (planning stage doesn't build/run code):

- **`// TEST: stdout "1"` directive syntax** — matches the quoted-string format used throughout
  `tests/run/*.vow` (confirmed via grep of existing fixtures).
- **`string_starts_with(Str, Str) -> I64`** (`vow-types/src/env.rs:190`) — returns `I64`, not
  `Bool`, so `print_i64(string_starts_with(...))` type-checks as written. No bug here.
- **`String::from(...)` / `fn main() -> i32 [io]`** — matches the style of
  `tests/run/pin_to_root_string.vow`, the cited reference fixture.
- **`"compiler|"` prefix** — confirmed against the doc comment on `__vow_proc_sample`
  (`vow-runtime/src/lib.rs:3296-3298`): format is `"<group>|<rss_kb>|<cpu_pct>"`, and the
  compiler's own group is always sampled/emitted first per `collect_proc_samples`.
- **Platform gap (non-blocking for CI, worth documenting in the PR body):**
  `vow-runtime/src/lib.rs:3290-3293` — `read_proc_table` returns empty on `not(target_os =
  "linux")`, so `__vow_proc_sample` returns `""` there, `string_starts_with("", "compiler|")` is
  `0`, and this fixture would print `0` instead of `1`. `.github/workflows/full-test.yml` runs
  both jobs on `ubuntu-latest` only, so this does not break CI. It **would** break a developer
  running `tests/run_tests.sh` locally on macOS. `TEST: skip` in this harness is unconditional
  (no platform-conditional variant exists — checked `scripts/full_test.sh`'s skip-parsing and
  found no `target_os`/platform-gated skip pattern anywhere in `tests/run/*.vow`), so there's no
  cheap fix within this fixture's format. Leave as-is (matches how other Linux-only-sampling
  behavior is already handled) but call it out explicitly in the PR description rather than
  leaving it as a silent local-macOS footgun.

## 4. Verification surface

This change touches lowering metadata only — no contracts, no ESBMC-visible C model, no codegen
type changes (the fix makes `proc_sample`'s call site carry the same `"String"` tag that every
other `Str`-returning builtin already carries; it does not introduce a new instruction shape).
Nothing new for ESBMC to prove. The `tests/run/*.vow` fixture in slice 4, if added, is an
unwind-free `--no-verify` build+run comparison per `scripts/full_test.sh` Section 4, not a
verification fixture, so no `--unwind` bound or contract is involved.

## 5. Risk areas

- **`proc_sample()` output is nondeterministic** (live `/proc` sampling, per-process RSS/CPU that
  differs run to run and between the separately-built Rust and self-hosted binaries). This is the
  reason slice 4 is optional and, if attempted, must avoid asserting on literal output — see
  slice 4's constraint. This is not a fixed-point or codegen-ordering risk; it's a test-authoring
  trap specific to this one builtin.
- **Binary fixed point / `BTreeMap` vs `HashMap` / stack-slot layout:** not implicated. The fix is
  confined to `vow-ir` (Rust crate) lowering metadata; it does not touch `vow-clif-shim`,
  `compiler/clif.vow`, or any self-hosted codegen path (self-hosted already has the correct tag,
  unchanged by this PR).
- **`parse → print → parse` idempotency:** not implicated — no AST or printer changes.
- **`cargo clippy --all -- -D warnings`:** the fix is a single additional `|`-pattern in an
  existing match arm; no new lint surface.
- **Overclaiming the bug's severity in the PR description.** The issue body itself hedges
  ("not yet confirmed whether this manifests as a memory-safety issue... or 'only' a
  type/method-resolution issue"). §1 above found that the method-resolution half is a non-issue in
  production (covered by `string_exprs`), and that `proc_sample`'s allocation is already
  freshly-owned rather than reused scratch, which weakens (without ruling out) the memory-safety
  reading for the `pin_to_root` half. The PR description should state precisely what was
  confirmed and what remains open, rather than repeating the issue's speculative framing as fact.

## 6. Out of scope

- **#1271's Operation Catalogue migration.** The issue body is explicit that #1271 will make this
  class of drift structurally impossible by generating both compilers' return-shape tags from one
  source. This PR is the narrow, independent fix + regression test the issue asks for; it does not
  start or partially implement the catalogue refactor.
- **Any other builtin in `tag_builtin_result`'s match.** Only `proc_sample` is confirmed drifted
  by the issue and by re-reading both `mod.rs` and `compiler/lower.vow`'s lists side by side; no
  other name is out of sync. Do not audit or touch unrelated arms in the same match as a
  drive-by.
- **A test for the `proc_sample().<method>()` dispatch path.** As established in §1 and §3.3,
  this path is not broken in production; adding a permanent unit test for it would test a
  test-harness artifact, not the compiler, and would misstate the fix's scope for future readers.
- **Refactoring `tag_builtin_result` or `ExprKind::MethodCall`'s dispatch match** into something
  more drift-resistant (e.g. extracting a shared table). That is precisely #1271's job; bundling
  it here would violate "many small changes beat one large change."
- **The `tests/run/proc_sample_pin_to_root.vow` integration fixture (slice 4)** is optional and
  may be dropped entirely if it can't be made deterministic within scope — slice 1's unit test is
  the load-bearing regression coverage for this issue regardless of what slice 4 finds.
