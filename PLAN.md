# Plan: wrong/right fix snippet for every errors.md entry (#454)

## Goal
Give every diagnostic entry in `docs/spec/errors.md` a concrete, compiler-validated **Wrong** / **Right** pair so agents get the fix shape instead of prose only. Documentation-only: no compiler, grammar, CLI, or verifier change; the only non-doc files touched are the generator's mechanical mirrors of the spec.

## Assumptions
- There are no `E###` codes. `errors.md` is keyed by PascalCase `error_code` names (47 `###` entries: 39 compile-time/verification, 7 runtime, 1 warning; `UnsupportedFeature` has 5 `####` sub-cases). "Every E### entry" = every `###` entry. No renaming/renumbering (that would be a diagnostics change, out of scope). (best guess)
- Format: label **every** Wrong block that gets a Right with `**Wrong:**` (one-line insert before the existing fence, existing block content untouched) and add a `**Right:**` fenced block directly after the corresponding `**Fix:**` paragraph. Entries with several wrong blocks (TypeMismatch, EffectViolation, UnsupportedPattern, UnsupportedFeature sub-cases, TautologicalComparison, InvalidCharacter) get one Right per Wrong block that is a code shape; prose-only sub-cases are skipped. The ≤6-line cap applies to **new** blocks only; existing blocks (e.g. LinearTypeViolation, the second EffectViolation block, the tuple UnsupportedPattern block) are not trimmed. No new `###`/`####` headings (keeps `build_docs_site.py` anchors stable). (best guess)
- Entries with no source-level fix (`EsbmcNotFound`, `LinkFailed`, `IoError`, `CodegenFailed`, `VerifierBug`, `OutOfMemory`, `RegionRootEscape`, `ModelCapacityAssumed`) use a `sh` block or a one-line "No source rewrite applies" instead of fabricated Vow. (best guess)
- Existing prose (including `VowEnsuresViolated`'s "or weaken the `ensures`", which sits badly with CLAUDE.md "Contract Authoring") is not rewritten; the new snippets demonstrate fixing the body / caller, never weakening a contract. Flag as follow-up. (best guess)
- `docs/spec/index.md` is not touched; it only links errors.md. (verify in Step 7)

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `docs/spec/errors.md` | the only hand-edited file; canonical spec | 1-1062 (all `###` entries) |
| `skills/vow/reference/errors.md` | byte-identical mirror, written by `generate_help.py` | whole file |
| `vow/src/skill.rs` | embeds skill bundle + support files (Rust raw strings) via `inject_skill_rust` | GENERATE markers |
| `compiler/main.vow` | embeds same payload as 2048-byte chunked literals via `inject_skill_vow` | GENERATE markers |
| `scripts/generate_help.py` | regenerator + `--check` drift gate (`build_skill_support_files` L1077, `main` L1260) | read-only |
| `scripts/full_test.sh` | `help/skills-dir-drift` gate (L2007-2012) runs `generate_help.py --check` | read-only |
| `bench/prompts.py`, `euler/run.py` | read errors.md as LLM prompt context | read-only (prompt grows) |

## Steps

### 0. Build the validator (not committed)
- Build exactly what `scripts/bootstrap.sh` Stage 0 builds so the runtime archive and clif-shim exist: `cargo build --all --release -j2` (bootstrap.sh L199-201; no `build/vowc` in this workspace). Keep scratch under `$TMPDIR`; use `VOW_CACHE_DIR=$(mktemp -d)`. Prefer frontend-only checks where possible (`--no-verify --dump-ir` prints IR with no codegen/link, `docs/spec/cli.md` L23) so a missing runtime cannot mask a result.
- Write a throwaway script in `$TMPDIR` (outside the repo) that, per entry, extracts the Wrong/Right fenced blocks from `docs/spec/errors.md` and checks them with `target/release/vow`. The script carries a **per-entry table** (not one heuristic) of: phase, wrapper (bare statements like LiteralOutOfRange's top-level `let`/`const` need wrapping in a fn/module — inside the script only, never in the doc), expected `error_code`, expected severity, and mode:
  - compile-time (Lexer/Parser/Type Checker/Region/Codegen): `--no-verify`; Wrong emits `error_code` with the right severity; Right compiles clean.
  - verification-phase: `vow verify` (ESBMC at `~/.local/bin/esbmc`); Wrong yields the documented code; Right is `Verified`. Wrong callers must be pure and modelable — an effectful `main` caller yields a `VerificationSkipped` note, not a counterexample (VowRequiresViolated/EnsuresViolated/InvariantViolated).
  - notes/warnings (`RegionRootEscape`, `ModelCapacityAssumed`, `VerificationSkipped` note form, `ArithOverflowReachable`, `LoweringWarning`): assert `severity`, and that the Right snippet no longer emits it.
  - runtime (7 entries): build Wrong with `--no-verify --mode debug` (without `--no-verify` the verifier rejects several statically), run it, assert stderr JSON `error` field and exit status 134; Right exits 0. `RegionLiteralMutation` is a compile-time rejection for statically traced literals — assert that path.
  - exempt (documented, no trigger): `ModelCapacityAssumed` (Wrong is an anti-pattern), `OutOfMemory`, `CodegenFailed`, `VerifierBug`, `EsbmcNotFound`/`LinkFailed`/`IoError` (environment; check the `sh` Right syntactically only).
- Run it against the *existing* examples first (baseline, red). Any existing example that no longer triggers its code (candidates: `RegionConflict` — coverage note says many routings are now accepted; `CodegenUnsupported` f64 `%`) is replaced with one that does. Record each correction in the commit body.

### 1. Lexer/parser entries — `docs/spec/errors.md` L9-92
UnterminatedString (close the `"`), InvalidCharacter (replace `@`; float: in-range literal), InvalidIntSuffix (`5u64`), UnexpectedToken (valid module header per `grammar.md`), MissingDelimiter (add `}`).

### 2. Type-checker group A — L94-263
TypeMismatch (four wrongs: return type, `v[i as u64]`, annotated `HashMap<i64, i64>`, `requires: b > a`), LiteralOutOfRange (`let x: u8 = 255;`), NarrowingCastNotAllowed (`i64_to_u8_wrap(big)` for a `-> u8` fn), ShiftCountOutOfRange (widen then `u32_to_u8_wrap`), TautologicalComparison (tight `ensures: result == n`; keep the `u32`-widening wrong/ok triple as-is), StaticLiteralRequired (literal `"name"`).

### 3. Effects, linear, regions, patterns, mutability, methods — L265-466
EffectViolation (`[io]`; `ensures: result.x == x`), LinearTypeViolation (consume once), RegionLinear (return the `Handle`), NonExhaustiveMatch (`Option::None => 0`), UnsupportedPattern (three wrongs: `if`/`else`; `match` on the enum; tuple-literal initializer), ImmutableAssignment (`let mut`), UnusedMut (drop `mut`), UnknownMethod (`v.push(42)`).

### 4. UnsupportedFeature, BTreeMap, contract-shape entries — L468-643
UnsupportedFeature main + sub-cases (trait → free fns; `&x` → `x`; tuple-in-contract → `a != 1 || b != 2`; map key → `u64` handle; linear Vec element → `Vec<i64>` handles; slice → `Vec<i64>`; reserved name → rename; unit param → remove), BTreeMapKeyTypeMustBeI64 (`BTreeMap<i64, i64>`), BTreeMapValueMustBeNonLinear (id handle), MissingContract (add `vow { ... }` with a *real* semantic clause, not a placeholder `true`), ContractTypeMismatch (boolean clause that is a real precondition).

### 5. Verification-phase entries — L645-920
VowRequiresViolated / VowEnsuresViolated / VowInvariantViolated currently have **no** snippet: add both Wrong and Right (caller guard; corrected body; corrected loop body — contract unchanged in every Right). EsbmcNotFound (`sh`: `vowc build --no-verify f.vow`), RegionConflict, RegionRootEscape (return instead of store into param), VerificationSkipped (split allocation from the contract-bearing computation), ArithOverflowReachable (genuine overflow-guard `requires`, or wrapping operator), VerifierAssertionUnattributed (`requires: d != 0`), VerifierBug (one line: report; do not weaken), ModelCapacityAssumed (Wrong: `requires: v.len() <= 128`; Right: no change).

### 6. Backend, IO, runtime, warning — L734-1062
CodegenUnsupported, CodegenFailed, LinkFailed (`VOW_RUNTIME_PATH=… `), IoError (fix `use` path); runtime: VowViolation (guard at call site), ArithmeticOverflow (zero-divisor guard / wrapping op), UnwrapOnNone (`match`), IndexOutOfBounds (`requires: i < v.len()`), RegionLiteralMutation (`String::from(value)` before mutation), StackOverflow (base case), OutOfMemory (no source rewrite); LoweringWarning (`let x: MyStruct = …`).

### 7. Final gate
- Regeneration happens in **every** slice commit (see TDD slices), so this step is a re-check: `uv run python scripts/generate_help.py --check`, `python3 scripts/test_generate_help.py`, `python3 scripts/test_build_docs_site.py`, `cargo fmt --all -- --check` (generator emits rustfmt shape for `vow/src/skill.rs`).
- Re-run the Step 0 validator on the final file (all entries).
- `VOW_CACHE_DIR=$(mktemp -d) scripts/bootstrap.sh --skip-cargo --no-cache` (workspace already built in Step 0); confirm the fixed point holds since `compiler/main.vow` payload literals changed. Record the checked head SHA.
- `git diff --stat origin/main`: only `docs/spec/errors.md`, `skills/vow/reference/errors.md`, `vow/src/skill.rs`, `compiler/main.vow`.
- Final commit/PR title: `docs(spec): add wrong/right fix snippets to every error entry` (lowercase subject, ≤92 chars). `git rm PLAN.md` before opening the PR.

## TDD slices
Docs-only, so red = validator fails (or Right missing). Each of Steps 1-6 is one slice: (a) add the group's Wrong/Right blocks to `docs/spec/errors.md`; (b) run the Step 0 validator restricted to that group's entries (`--only <Entry>`), red→green; (c) run `uv run python scripts/generate_help.py` so the mirrors regenerate (keeps every commit bisectable and `help/skills-dir-drift` green); (d) commit `docs(spec): add wrong/right snippets for <group> errors`. Test location: throwaway script in `$TMPDIR`, not committed. Production code that makes it pass: `docs/spec/errors.md` only; regeneration is mechanical.

## Testing
- Per-snippet: Wrong emits documented `error_code`; Right compiles (and `vow verify` passes for contract entries).
- Gates: `generate_help.py --check` (the CI `help/skills-dir-drift` check), `test_generate_help.py`, `test_build_docs_site.py`, bootstrap fixed point.
- No fixtures under `tests/` or `examples/` grow; a committed doc-snippet CI check is a follow-up issue, not this PR.

## Verification surface
No contract, codegen, IR, or C-model change; ESBMC proves nothing new. Verify-phase Right snippets are checked with `vow verify` locally only. Verifier C parity (`c_emitter.{rs,vow}`) untouched. Dual-compiler rule: no semantics change; both compilers receive the identical generated payload in the same commit.

## Risks
- A "Right" snippet that does not compile is worse than none (agents copy it; bench/euler feed errors.md to LLMs): mitigated by mandatory validator in steps 0/7.
- Existing Wrong examples may be stale (RegionConflict, CodegenUnsupported): baseline run in Step 0 finds them; correct, don't copy forward.
- Generated payload: a snippet containing `"#` forces a longer Rust raw-string delimiter (`_rust_raw_string` handles it); Vow chunk escaping handles `"`/`\`/newline. `compiler/main.vow` diff will be large and mechanical; bootstrap fixed point must be rechecked. `generate_help.py` must be run with the repo's `uv` env; hand-editing the mirrors causes `help/skills-dir-drift`.
- `docs/spec/` + `skills/` are classified as code by `scripts/ci_docs_only.py`, so full CI runs; no docs-only fast path.
- Concurrent bootstrap/cargo contention can produce a lone flaky failure (see repo memory); re-run before blaming the doc.
- Snippets must not weaken contracts to please ESBMC (CLAUDE.md Contract Authoring): Right blocks for Requires/Ensures/Invariant keep the contract and fix code.

- `parse → print → parse` idempotency, binary fixed point codegen ordering, `vow-clif-shim` stack slots, C emitter parity: untouched (no compiler logic changes); only the embedded string payload in `compiler/main.vow` changes, covered by the bootstrap re-check.
- `cargo clippy --all --all-targets -- -D warnings`: no Rust logic changes; `vow/src/skill.rs` only gains literal text. Run `cargo fmt --all -- --check` to confirm generated shape.
- codecov/patch (95% gate): regenerated lines in `vow/src/skill.rs`/`compiler/main.vow` count as changed lines. Before pushing, check how a recent spec-only PR that regenerated the payload fared (`gh pr checks` on #1601 or #1592) and expect the same; not a blocker.
- Pre-commit hooks that touch markdown: `trailing-whitespace`, `end-of-file-fixer`, `typos` (deliberate misspellings like `psh` already exist in the file; keep new ones to a minimum or reuse them), `commitlint` on the commit-msg.

## Out of scope
- Renaming entries to `E###` or changing `error_code` values.
- Rewriting existing prose (incl. the "weaken the ensures" advice), restructuring errors.md, or splitting it.
- A committed doc-snippet CI harness; `check_help_coverage.py` changes.
- Any compiler, spec-grammar, CLI, schema, or verifier change; `docs/comparisons/index.html` status edits.
