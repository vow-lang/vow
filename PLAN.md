# Plan: remove dead `__vow_print_str` runtime export (#1282)

## 1. Problem restated

`vow-runtime/src/lib.rs` exports `__vow_print_str`, a `no_mangle extern "C"` symbol that no
lowering path in either compiler ever emits a call to — `print_str` lowers to the distinct,
live `__vow_string_print` symbol in both the Rust IR lowerer (`vow-ir/src/lower/mod.rs:33`)
and the self-hosted lowerer (`compiler/lower.vow:1423`). The function is unreachable and its
name invites confusion with the real `__vow_string_print` path. **Correction to the issue's
premise:** the issue states the two dead Cranelift signature-table arms for
`"__vow_print_str"` were "already removed in #1279." Verified against the repo: **PR #1279 is
still OPEN, not merged** (`gh pr view 1279 --json state` → `OPEN`), and this branch is exactly
at `origin/main` (651327a0, `git log HEAD..origin/main` empty), so those arms are still present
on `main` today. An unfiltered `grep -rn "__vow_print_str"` (excluding `target/`, `build/`,
`.git/`) currently returns **four** hits, not one:

- `vow-runtime/src/lib.rs:138` — the export itself (the only hit the issue names)
- `vow-codegen/src/cranelift_backend.rs:2429` — dead Cranelift extern-sig match arm
- `vow-clif-shim/src/lib.rs:3347` — dead Cranelift extern-sig match arm (self-hosted backend path)
- `CLAUDE.md:181` — stale doc line listing `__vow_print_str` as a `vow-runtime` print helper

Acceptance criterion #2 ("confirm no other reference exists anywhere in the repo") is
grep-clean as the literal bar, so all four hits are in scope — not just the one the issue body
names. This is still a small, surgical chore (~10 lines total across 4 files), not a scope
creep: every added hunk is a strict subset of what PR #1279 already independently identified as
dead in its own PR description.

**Handoff note for the implementation stage:** re-run `gh pr view 1279 --json state,mergedAt`
before starting. If #1279 has merged in the meantime, `git rebase origin/main` first — the
`cranelift_backend.rs`/`vow-clif-shim/src/lib.rs` hunks below will already be gone (#1279
rewrites the whole `print_*` arm into a generated `// GENERATE:OPERATIONS` block), and the
`CLAUDE.md:181` hunk will already read `__vow_string_print` instead of `__vow_print_str` — in
that case just drop the now-redundant hunks and proceed with the `vow-runtime/src/lib.rs`
removal only. Record in the PR body *why* the diff is wider than the issue text (the four-hit
grep above) — `PLAN.md` itself is deleted before the PR opens, so this reasoning must be
restated there or it is lost.

## 2. Files to touch

| File | Change |
|---|---|
| `vow-runtime/src/lib.rs` | Delete the `__vow_print_str` function (lines ~137–144: the `#[unsafe(no_mangle)] pub unsafe extern "C" fn __vow_print_str(s: *const u8) { ... }` block, including its `#[unsafe(no_mangle)]` attribute). Leave `__vow_string_print` (line 2088), `__vow_print_i64`, `__vow_print_u64`, and `__vow_debug_str` untouched — all are live and independent of this symbol. |
| `vow-codegen/src/cranelift_backend.rs` | Delete the `"__vow_print_str" => { sig.params.push(AbiParam::new(types::I64)); // ptr }` match arm at line ~2429–2431 in `make_extern_sig`. The adjacent `"__vow_print_i64" \| "__vow_print_u64"` arm and the separate `"__vow_string_print"` arm (line 2616) are untouched. |
| `vow-clif-shim/src/lib.rs` | Delete the equivalent `"__vow_print_str" => { sig.params.push(AbiParam::new(types::I64)); }` match arm at line ~3347–3349. Same adjacency as above (`__vow_string_print` arm at line 3533 untouched). |
| `CLAUDE.md` | Line 181: change `print helpers (\`__vow_print_str\`, \`__vow_print_i64\`)` to `print helpers (\`__vow_string_print\`, \`__vow_print_i64\`)` — matching the exact wording #1279 independently lands, so whichever PR merges second is a clean no-op/trivial-conflict on this line. |

No `crates/` vs `compiler/` split applies here beyond the Cranelift-backend duplication above:
`compiler/lower.vow` and `compiler/main.vow` never reference `__vow_print_str` (confirmed by
grep) and need no change. No `docs/spec/*.md` file documents `__vow_print_str` — the spec
documents the *builtin* `print_str`, whose signature, effect (`[io]`), and behavior are
unchanged; only an unreferenced internal runtime symbol name disappears. No spec update is
required or should be added.

## 3. TDD slices

This is dead-code removal with no observable behavior change, so there is no red step in the
conventional sense — there is no test that can fail against the presence of unreachable code,
and writing one would be testing an absence, not a behavior. Treat the "green" bar as: the
build still succeeds, the full test suite still passes unchanged, and the two grep-based
verification steps below both pass. Sequence as four small, independently-revertable commits
(each individually buildable):

1. **Remove the dead export.** Delete `__vow_print_str` from `vow-runtime/src/lib.rs`.
   Verify: `cargo build -p vow-runtime` succeeds (nothing in the crate references the deleted
   fn — confirmed no internal caller via grep).
2. **Remove the dead Rust-backend signature arm.** Delete the `"__vow_print_str"` arm from
   `vow-codegen/src/cranelift_backend.rs::make_extern_sig`. Verify: `cargo build -p vow-codegen`
   succeeds; the match remains exhaustive because it is a `match sym { ... _ => {} }`-shaped
   string match (removing one literal arm just routes that never-produced string through the
   existing fallthrough — confirm the match still has a wildcard/default arm before deleting,
   so this doesn't turn into a non-exhaustive-match compile error).
3. **Remove the dead self-hosted-backend signature arm.** Delete the equivalent arm from
   `vow-clif-shim/src/lib.rs`. Verify: `cargo build -p vow-clif-shim` succeeds.
4. **Fix the stale doc line.** Update `CLAUDE.md:181`. No build implication; verify by reading
   the line back.

After all four: run the full gate (see below) once, not once per slice — these are small enough
that per-slice full-suite runs would be pure overhead.

## 4. Verification surface

None. This change touches no contract, no codegen path that's actually reachable, no CLI flag,
and no C model that ESBMC reasons about — `__vow_print_str` was never wired to any call site,
so ESBMC never had a verification condition that exercises it. No new ESBMC properties, no new
`tests/run/` or `examples/` fixtures are needed. The existing fixtures that exercise `print_str`
(e.g. `examples/hello.vow`, any `tests/run/*.vow` asserting stdout via `TEST: stdout`) continue
to exercise the *live* `__vow_string_print` path and serve as the regression check that this
removal didn't accidentally touch the real print path — no new fixture needs to be added, but
`scripts/full_test.sh` must still pass to confirm those existing fixtures are unaffected.

## 5. Risk areas

- **Binary fixed point:** unaffected. `compiler/` (the self-hosted sources compiled to
  `build/vowc`) never references `__vow_print_str`, so `scripts/bootstrap.sh`'s three-stage
  SHA-256 comparison has nothing to diverge on. Bootstrap is worth running once as a confidence
  check, not because this change is expected to touch it.
- **`vow-clif-shim` stack-slot layout / `BTreeMap` ordering:** unaffected — deleting a never-hit
  match arm changes no codegen for any function that is actually compiled.
- **`parse → print → parse` idempotency:** unaffected — no AST, token, or printer change.
- **`cargo clippy --all -- -D warnings`:** the main real risk. Deleting `__vow_print_str` could
  orphan an import in `vow-runtime/src/lib.rs` if nothing else in the file uses
  `std::io::Write` / `VowVec` / `sanitize_on_read`. Checked: `__vow_debug_str` (adjacent, kept)
  and `__vow_string_print` both use `sanitize_on_read`, `VowVec`, and `Write`, so no import
  should go unused — but the implementation stage must still run clippy to confirm rather than
  assume, since an unused-import warning under `-D warnings` fails the build. Same caution
  applies to `AbiParam`/`types::I64` imports in `cranelift_backend.rs` and
  `vow-clif-shim/src/lib.rs` — both files use these extensively elsewhere, so no orphaning is
  expected, but clippy is the actual gate, not this prediction.
- **Match exhaustiveness in the two Cranelift files:** confirm before deleting that
  `make_extern_sig`'s `match sym { ... }` has a catch-all arm (it does, per current source) —
  otherwise removing a literal arm is not just dead-code deletion, it changes match semantics.
- **Quality gate must be run as separate commands, not `&&`-chained** (per repo convention):
  `cargo build --all`, `cargo test --all`, `cargo clippy --all -- -D warnings`,
  `cargo fmt --all -- --check`, each capped with an explicit `-j` sized to the sandbox's actual
  core/memory share rather than the host default.
- **PR #1279 landing mid-flight:** see the handoff note in §1 — re-check its merge state before
  editing, since it may have already applied the `cranelift_backend.rs` / `vow-clif-shim` /
  `CLAUDE.md` hunks (rewritten into a generated block, in the first two cases).

## 6. Out of scope

- The Operation Catalogue (`docs/spec/operations.json`, `scripts/generate_operations.py`) that
  PR #1279 introduces — that's a separate, much larger refactor (epic #375) and not this issue's
  concern.
- `vow-types/src/env.rs::builtin_free_fn_signatures()` and its self-hosted counterpart — already
  centralized since #1047, untouched by this change, and not implicated by the dead symbol.
- The verifier's `is_known_builtin`/`is_modelable` classifier — `print_*` builtins are effectful
  and never reach that gate; no change needed there.
- Renaming or otherwise touching `__vow_string_print`, `__vow_print_i64`, or `__vow_print_u64` —
  all three are live and out of scope.
- Any new test fixtures under `tests/run/` or `examples/` — existing `print_str` fixtures
  already cover the live path; no new coverage is needed for a removal of unreachable code.
- General cleanup of `vow-runtime/src/lib.rs`, `cranelift_backend.rs`, or `vow-clif-shim/src/lib.rs`
  beyond the specific dead-code hunks identified above — no bundled refactors or formatting
  passes.
