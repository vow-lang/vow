# Plan: #1472 — `.vow.d` stubs shadow source and drop contracts

## 1. Problem restated

`vow/src/module_loader.rs::module_file_for_use` prefers a sibling `<name>.vow.d`
declaration stub over `<name>.vow` source whenever both exist, and
`vow-syntax/src/printer.rs::print_fn_decl` (the function that renders a
declaration's signature into a `.vow.d` stub) silently drops `requires`/
`ensures`/`invariant` clauses even when the underlying `FnDef.vow` carries them.
Worse, the same clause-dropping bug exists in `print_fn`'s declaration arm (the
full-module printer), so a plain `.vow` file containing
`fn f(x: i64) -> i64 vow { requires: x > 0 };` fails `parse → print → parse`
idempotency today — independent of stubs. But fixing only the printer does not
close the hole: research into `vow-types/src/check.rs`, `vow-ir/src/lower/mod.rs`
and `vow-verify/src/c_emitter.rs` (mirrored exactly in `compiler/checker.vow`,
`compiler/lower.vow`, `compiler/c_emitter.vow`) shows that **no declaration-only
function's `vow` block is ever consulted at a call site today, stub or not**:
`is_declaration` functions are excluded from `fn_items`/`func_index` during IR
lowering (`vow-ir/src/lower/mod.rs:5331-5344`), so a call to one lowers to an
untyped `InstData::CallExtern(name)` with `Ty::Unit`, never
`InstData::CallTarget`, and the verifier's "assert callee's `requires` at the
call site" machinery (`c_emitter.rs:3256-3273`) only walks `InstData::CallTarget`
callees. A non-builtin `CallExtern` makes the calling function `non_modelable`
(`Skipped`, not falsely `Verified`) — a real but easy-to-miss loss, since
`Skipped` and `Verified` are both "not `Violated`" if nothing downstream checks
per-function status. Building a sound axiomatic call-site model for bodyless
declarations (assert `requires`, havoc the result, and — the genuinely unsafe
part — *assume* `ensures` as an unverified trust boundary) is exactly the open,
unresolved "implementation/proof identity" question #1053 reserves for its own
epic. The scoped, fail-closed fix for #1472 is therefore: make stubs capable of
*saying* what contract they carry (printer fix, needed regardless), and make
the loader refuse to rely on a stub whose declarations carry a contract the
verifier cannot enforce from a bodyless signature — falling back to the sibling
`.vow` source, where the existing, already-correct intra-module
`requires`-as-assert/Caller-blame mechanism takes over unchanged.

## 2. Files to touch

**Rust compiler**
- `vow-syntax/src/printer.rs` — fix `print_fn`'s declaration arm (lines
  ~184-187) and `print_fn_decl` (lines ~58-74) to emit the `vow` block (reuse
  the existing `print_vow_block` helper, already used once elsewhere, instead
  of hand-rolling three copies of the same clause-printing loop).
- `vow-syntax/tests/proptest_arb.rs` — the `FnDef` strategy hardcodes
  `is_declaration: false` (line ~333); extend it to sometimes generate
  `is_declaration: true`, with and without a `vow` block, so
  `proptest_roundtrip.rs` exercises the fixed paths continuously instead of
  only via hand-written unit tests.
- `vow/src/module_loader.rs` — add a predicate (`module_has_unenforceable_contract`
  or similar) over a parsed `Module`, and restructure `load_deps` (not
  `module_file_for_use`, which stays a pure existence-based path choice used
  unchanged by `infer_module_root`) so that when the preferred file is a
  `.vow.d` stub, it is parsed first; if the predicate is true, re-resolve and
  parse the sibling `.vow` source instead.
- `vow/src/main.rs` — no code change expected (the `vow decl` output path
  already calls `print_declarations`/`print_fn_decl`, fixed above), but add a
  test confirming `vow decl` output now includes vow blocks verbatim.

**Self-hosted compiler**
- `compiler/frontend.vow::load_frontend_deps` (lines ~153-187) — same loader
  restructuring as `vow/src/module_loader.rs::load_deps`: after
  `resolve_use_path` picks a `.vow.d` stub and it's parsed, inspect the parsed
  module for a declaration carrying a non-empty `vow` block; if found,
  re-resolve against the `.vow` source and parse that instead. Needs a small
  AST-walking helper over `compiler/ast.vow` accessors (`fn_is_declaration`,
  `fn_vow_lid`, iterating `ITEM_FN`/`ITEM_IMPL` items) — no new arena fields.
- **No self-hosted printer change.** `compiler/main.vow` has no `decl`
  subcommand and `compiler/ir_printer.vow` only prints IR, not AST source —
  the self-hosted compiler cannot emit `.vow.d` stubs at all today. The
  printer half of #1472 is Rust-only; only the loader (consumer) half needs
  self-hosted parity, per the issue's acceptance criterion.
- `compiler/checker.vow`, `compiler/lower.vow`, `compiler/c_emitter.vow` —
  **no changes**. The fallback happens entirely at the loading step, before
  type-checking; by the time these run, they see either full source or a
  stub with no enforceable contract, so their existing `is_declaration`
  handling (already parallel to Rust, confirmed by research) is untouched.

**Tests / fixtures**
- `vow-syntax/src/printer.rs` (inline `#[cfg(test)]`) — new unit tests for
  the declaration-with-vow-block round trip, both via `print_module` and
  `print_declarations`.
- `vow/src/module_loader.rs` (inline tests) — new unit tests for the
  contract-detection predicate and the fallback-vs-stub choice.
- New fixture directory `tests/verify-fail-multi/stub_requires_violation/`
  with `dep.vow` (real source, `pub fn` with `requires`), `dep.vow.d` (the
  stub, generated-looking — written by hand to match what the fixed `vow decl`
  would emit, i.e. it **does** carry the `vow` block, proving the fallback is
  what catches this, not an already-empty stub), and `main.vow` (`use dep;`,
  calls with a `requires`-violating argument). See §4 for why this cannot live
  flatly in `tests/verify-fail/`.
- `scripts/full_test.sh` — new small section (e.g. "Section 4d: Verify-Fail
  Multi-Module Fixtures") that walks `tests/verify-fail-multi/*/`, runs both
  `$RUST verify` and `run_self verify` on each `main.vow`, and checks
  `status == VerifyFailed` plus the existing
  `// TEST: counterexample-blame caller` / `counterexample-violation`
  directive parsing already used by Section 4c — reusing, not duplicating,
  that parsing logic where possible.
- `tests/multi/decl_stub_preference/` — **unchanged**. Its stub has no `vow`
  block, so the new fallback predicate is false and current behavior (stub
  preferred, ill-typed source ignored) is preserved exactly.

**Docs**
- `docs/spec/grammar.md` (~lines 25-27) and `docs/spec/stdlib.md` (~line 33) —
  state the fallback rule: a `.vow.d` stub is used only when none of its
  declarations carry a `vow` block; otherwise the sibling `.vow` source is
  loaded instead, so a declaration's contract is never silently dropped from
  verification.
- `docs/spec/cli.md` (~line 124, `vow test` module-root inference) — add a
  one-line note that `infer_module_root`'s existence-only walk is unaffected
  (it never inspects file contents; the contract-fallback check only applies
  once a concrete `use` is actually loaded for build/verify).
- Regenerate help/skill text: `uv run python scripts/generate_help.py`, then
  `cargo build --release -p vow` and `scripts/bootstrap.sh --skip-cargo`, per
  CLAUDE.md's spec-change checklist (`vow decl` help text and
  `compiler/main.vow`'s embedded copies of `grammar.md`/`stdlib.md` prose
  reference `.vow.d` resolution and must stay in sync — `--check` in
  `scripts/check_help_coverage.py` will catch drift).

## 3. TDD slices

1. **Printer: declarations round-trip their `vow` block.**
   - Red: `vow-syntax/src/printer.rs` test `declaration_with_vow_block_prints_requires_and_ensures` —
     build a `FnDef` with `is_declaration: true` and
     `vow: Some(VowBlock { clauses: [Requires(...), Ensures(...)], .. })`,
     assert `print_module` output contains `vow {`, `requires: ...`,
     `ensures: ...` before the `;`. Fails today (both `print_fn`'s declaration
     early-return and `print_fn_decl` drop it).
   - Red: same fixture through `print_declarations` (the `.vow.d`-emission
     path) — same assertion, currently fails for the same reason in
     `print_fn_decl`.
   - Green: refactor `print_fn`'s declaration arm and `print_fn_decl` to call
     the existing `print_vow_block(vow, level + 1)` helper and emit
     `" vow {\n...}"` before the trailing `;` when `f.vow.is_some()`, matching
     the grammar `fn add(x: i64) -> i64 vow { requires: x > 0 };` already
     accepted by `parser/mod.rs::parse_fn` (confirmed: `parse_vow_block` is
     called unconditionally before the `Semicolon` check, so this is a
     printer-only fix, no parser change).
   - Refactor: once both declaration arms reuse `print_vow_block`, delete the
     now-redundant inline clause-printing loop that `print_fn`'s non-declaration
     arm still has (lines ~190-204), and switch it to the same helper too —
     three copies become one.
   - Extend `vow-syntax/tests/proptest_arb.rs`'s `FnDef` strategy to generate
     `is_declaration: true` (with and without a `vow` block) some fraction of
     the time, so the existing `proptest_roundtrip.rs` property test exercises
     this continuously.

2. **Loader: detect a stub's unenforceable contract.**
   - Red: `vow/src/module_loader.rs` unit test
     `module_has_unenforceable_contract_detects_declaration_with_requires` —
     parse a tiny module string with
     `pub fn f(x: i64) -> i64 vow { requires: x > 0 };`, assert the new
     predicate returns `true`.
   - Red: companion tests — returns `false` for a declaration with no `vow`
     block (the existing `decl_stub_preference` shape), `false` for a
     non-declaration function with a `vow` block (it has a real body, nothing
     to lose), and `true` for the same shape inside an `impl` block's method
     (iterate `Item::Impl(i).methods` the same way as top-level `Item::Fn`).
   - Green: implement the predicate over `Module.items`. **Design choice:**
     ignore `Visibility` entirely (don't special-case private declarations) —
     the self-hosted AST (`compiler/ast.vow`) has no visibility slot on
     `fn_data` at all (confirmed: `pub` is lexed and parsed but never
     recorded), so a visibility-aware predicate on the Rust side would be an
     unreproducible parity divergence from the self-hosted side for a
     vanishingly rare case (a private declaration with a contract in a
     published stub). Trading a little stub-usability for exact two-compiler
     parity is the right call here.

3. **Loader: fall back to source when the stub fails the predicate.**
   - Red: `vow/src/module_loader.rs` integration-style test building a real
     temp directory (`tempfile::TempDir`, already a dev-dependency) with
     `dep.vow` (ill-typed, mirroring `decl_stub_preference`'s poison trick, OR
     simply a different, type-correct body so the test can assert on which
     body actually ran) and `dep.vow.d` carrying a `requires`-bearing
     declaration; assert that loading `main.vow`'s `use dep` resolves to the
     `.vow` source, not the stub.
   - Red: companion test with a contract-free stub (today's
     `decl_stub_preference` shape) proving the stub is still preferred —
     this is the regression pin that the existing `tests/multi/decl_stub_preference/`
     fixture must keep passing unmodified.
   - Green: restructure `load_deps` so file selection is: resolve `vow_path`
     and `decl_path` as today; if `decl_path` exists, read and parse it; if
     parsing fails cleanly or `module_has_unenforceable_contract` is true,
     discard it and read+parse `vow_path` instead; otherwise keep the
     already-parsed stub AST (avoid a redundant re-parse). `module_file_for_use`
     itself is untouched — it is still the existence-only predicate
     `infer_module_root` relies on, and `infer_module_root` must stay
     content-blind (it probes many candidate ancestor directories; parsing
     each one would be wasteful and is not needed for its narrower job of
     picking a resolving directory).

4. **Self-hosted parity: same fallback in `load_frontend_deps`.**
   - Red: a `compiler/tests/test_frontend_decl_fallback.vow` (or extend an
     existing frontend test file) exercising `load_frontend_deps` (or the
     smallest public seam around it) against a temp-directory-style fixture
     with the same dep/stub/source shape as slice 3, asserting the merged
     module's item for `dep`'s function came from `.vow` (e.g. by checking
     `item_files` attribution, mirroring how `vow-ir`'s `item_files`
     provenance is tested on the Rust side) rather than the empty-body stub.
   - Green: port the same predicate (walk `ITEM_FN`/`ITEM_IMPL` arena entries
     for `fn_is_declaration(a, fid) && fn_vow_lid(a, fid) != -1`) and the same
     parse-then-maybe-reparse restructuring into `load_frontend_deps`.
   - Run `cargo test -p vow module_loader` and the self-hosted
     `build/vowc test compiler/` (or the relevant single file via `--filter`)
     after each half lands, per CLAUDE.md's dual-compiler-same-session rule.

5. **End-to-end regression: a caller violating a library `requires` through a
   stub is `Violated`/Caller blame, not `Verified`.**
   - Red: add `tests/verify-fail-multi/stub_requires_violation/{dep.vow,dep.vow.d,main.vow}`
     and the new `scripts/full_test.sh` section described in §2. Before the
     loader fix, this fixture's `main.vow` would either (a) load the stub,
     hit the `CallExtern`/non-modelable path, and report `Skipped` — still not
     `Verified`, but not the specific `VerifyFailed`+Caller-blame shape the
     acceptance criterion demands — or (b) if the stub predicate isn't wired
     in yet, silently stay on the stub. Either way the slice-5 assertion
     (`status == VerifyFailed`, blame `caller`) fails until slices 3-4 land.
   - Green: once the loader falls back to `dep.vow`'s real source, `dep`'s
     function becomes a normal fully-lowered callee, and the existing,
     unmodified `c_emitter.rs:3256-3273` / `compiler/c_emitter.vow` equivalent
     "assert callee's `requires` at the call site" mechanism (already proven
     by `tests/verify-fail/caller_requires_violation.vow`) does the rest with
     zero verifier changes.
   - This slice is the one that actually satisfies acceptance criterion #3; it
     depends on 1-4 and should land after them, as its own small commit
     (new fixture + harness section only).

6. **Pin the stub-only (no sibling source) case as already fail-closed.**
   - Add `tests/verify-skip/stub_only_requires_declaration.vow` (single file,
     no `use`, matching the existing flat convention in that directory) that
     directly declares `fn f(x: i64) -> i64 vow { requires: x > 0 };` with no
     body anywhere and a caller that calls it, documenting (per the existing
     `unwrap_panic_effect.vow`/`vec_of_string_skipped.vow` style comment
     convention) that a contracted declaration with no backing implementation
     is `Skipped`, not falsely `Verified` — this is the one case this plan
     deliberately does not "fix" further (see §6 Out of scope), and the test
     exists to pin that it fails closed rather than open.
   - This is a pure test-only slice; if it's already `Skipped` today (expected,
     per the research trace through `is_modelable`/`non_modelable_reason`),
     no production code changes are needed — just confirm and pin.

## 4. Verification surface

- **New ESBMC obligation, same shape as existing ones.** Once the loader falls
  back to source, `dep`'s function is verified exactly like any other
  intra-module callee — no new C-model pattern, no new ESBMC property class.
  The only new "property" from ESBMC's point of view is the *existing*
  `requires`-as-`__ESBMC_assert` at the call site, now reached via a call that
  used to be invisible to the verifier.
- **Why not `tests/multi/`:** confirmed by reading `scripts/full_test.sh`
  Section 6b — every `tests/multi/*/` fixture is built with `--no-verify`, so
  it cannot carry a verification-outcome assertion at all. The acceptance
  criterion's "`tests/verify-fail/` or `tests/multi/`" phrasing is satisfied by
  neither as-is: `tests/verify-fail/*.vow` is a *flat* glob in both
  `scripts/full_test.sh` Section 4c and `tests/run_tests.sh` Phase 3, each run
  as an independent `vow verify <file>` target expected to itself report
  `VerifyFailed` — a bare sibling `dep.vow` dropped in that directory would be
  swept up and verified standalone (trivially `Verified`, since its own
  `requires` is an assumption, not an obligation, when it's the direct
  target), which Section 4c's `actual_status != VerifyFailed → fail` check
  would flag as a harness regression. Hence the new `tests/verify-fail-multi/`
  subdirectory-per-fixture convention (slice 5), modeled on `tests/multi/`'s
  own subdirectory pattern but wired to run `verify` instead of
  `build --no-verify`.
- **No change needed to `tests/run_tests.sh`.** It is the local-dev-only
  harness (not CI-gating per CLAUDE.md); adding the new section there too is
  optional polish, not required by the acceptance criteria, and is called out
  explicitly in §6 as deliberately not bundled into this change.
- **Parity (`scripts/parity.py c`).** The new `tests/verify-fail-multi/*/main.vow`
  fixtures should also be added to Section 2c's C-parity glob once the
  harness section exists, so a future emitter change can't silently diverge
  Rust vs self-hosted C for this call shape. Flag this explicitly in the PR
  description since Section 2c's glob list is hand-maintained.

## 5. Risk areas

- **Parity divergence in the fallback predicate.** Both compilers must treat
  "stub declares a non-empty `vow` block" identically. The visibility-blind
  design in slice 2 removes the obvious divergence (self-hosted has no
  visibility tracking at all), but the Rust and self-hosted predicates are
  still two independent implementations (one walking `vow_syntax::ast::Module`,
  one walking `compiler/ast.vow`'s flat arena) — any mismatch (e.g. forgetting
  `Item::Impl` methods on one side) silently changes which file a `use`
  resolves to on only one compiler, which would surface as a
  `scripts/parity.py`/`compare_full_json` mismatch in the new harness section,
  not a crash. Write the predicate tests (slice 2 and its self-hosted mirror
  in slice 4) against the *same* fixture source text on both sides to keep
  them honest.
- **`print_fn`'s early-return fix touches a hot, heavily-proptested path.**
  `print_fn` is used for every `.vow` file the compiler ever prints
  (canonical form, `vow decl`-adjacent tooling, error-message source
  snippets in some diagnostics). Refactoring its declaration arm to share
  `print_vow_block` must not change output for the (overwhelmingly common)
  non-declaration case — keep that refactor to the two declaration arms only
  in the first pass, and only fold the non-declaration arm's duplicate loop
  into the shared helper as a mechanical, test-covered follow-up within the
  same slice (not a separate PR, but called out as its own commit so a
  bisect lands on the behavior change vs. the dedup separately).
- **`load_deps` double-parse cost.** The new design parses the `.vow.d` stub
  first and, on fallback, re-parses the `.vow` source — a small constant-factor
  cost (declaration stubs are small by construction) that's fine for
  correctness but should avoid re-reading the stub file a second time if the
  fallback path is taken (keep the already-read stub `String` only as long as
  needed to decide, then drop it).
- **`compute_may_write_table`'s existing `is_declaration`-seeded fail-open
  bit** (`compiler/checker.vow:4165-4179`, mirrored in `vow-types`) is
  unaffected by this change — it already treats any surviving declaration
  (after the new fallback) as conservatively "may write," which remains
  correct whether or not the declaration carries a contract.
- **Binary fixed point / `BTreeMap` vs `HashMap` ordering.** The fallback
  decision is made once per `use`, before any `HashMap`/`BTreeMap`-ordered
  codegen structure exists, and doesn't change iteration order of anything
  downstream (the merged module's item order is still dependency-first by
  `merge_modules`/`load_frontend_deps` as today) — low risk to the stage
  A/B/C bootstrap fixed point, but re-run `scripts/bootstrap.sh --skip-cargo`
  after the self-hosted change per CLAUDE.md, since any self-hosted module
  (`compiler/*.vow`) that happens to ship both a `.vow` and a stray
  `.vow.d` would now resolve differently.
- **`clippy --all -- -D warnings`.** New predicate/helper functions should
  stay small and single-purpose (per CLAUDE.md's deep-modules guidance) to
  avoid `too_many_arguments`/complexity lints; no `#[allow]` should be needed.

## 6. Out of scope

- **Axiomatic/modular contract verification for bodyless declarations**
  (assert `requires` at the call site, havoc the result, assume `ensures`) —
  the actual mechanism that would let a `.vow.d` stub *with* a contract be
  verified against directly, without falling back to source. This is real,
  substantial new verifier capability (new C-model pattern in both
  `c_emitter.rs` and `compiler/c_emitter.vow`, new `is_modelable`/
  `collect_modelable_callees` cases, a trust boundary around `ensures` that
  #1053 explicitly flags as unresolved "implementation/proof identity").
  Tracked as a follow-up under #1053, not this issue.
- **`vow decl` self-hosted support.** The self-hosted compiler has no `decl`
  subcommand and no AST/source printer at all; adding one is unrelated to
  "stubs hide contracts from verification" (the consumption side, which this
  plan fixes) and is its own feature.
- **Visibility tracking in the self-hosted AST.** Noted as a contributing
  factor to the predicate design (slice 2) but not fixed here — it's a
  pre-existing, separately-scoped gap.
- **Extending `tests/run_tests.sh`** with the new verify-fail-multi section —
  optional local-dev-harness parity, not required by the acceptance criteria.
- **The wider library-packaging audit** (artifact format, version
  compatibility, release manifest, proof/implementation identity) — stays in
  #1053 per the issue body.
- **Any change to `infer_module_root`** — confirmed existence-only by design
  (used by `vow test`'s root inference across many candidate directories);
  this plan does not add content-inspection to it.
