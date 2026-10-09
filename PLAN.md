# Plan: make `io` independent of `read`/`write` in both compilers (issue #635)

## Goal
Remove the `[io]` ⊇ `[read]`/`[write]` subsumption from the effect checkers of the Rust and self-hosted compilers so the implementation matches the documented rule ("Each effect is independent — `io` is not a superset of `read` or `write`", `docs/spec/grammar.md:1249`, `docs/vow_design.md:259`, generated `--help`). Resolution (a) from the issue; (b) rejected: the docs, `--help`, and design §4.2 (explicit semantics) all say independence.

## Assumptions
- Option (a) chosen (best guess; matches spec, `--help`, and the issue's recommendation). No spec sentence changes, so generated help/skill text only changes via the mandatory docs step 5.
- The change is **breaking for existing `.vow` sources**: ~1036 `[io]`-only declarations exist in the corpus and any that call a `read`/`write` builtin (`fs_read`, `fs_open`, `fs_read_line`, `fs_status`, `fs_read_status`, `fs_close`, `fs_exists`, `fs_listdir`, `fs_is_dir`, `fs_is_symlink`, `getenv`, `path_lookup`, `args`, `stdin_read`, `stdin_read_line`, `stdin_ready` = Read; `fs_write` = Write — `vow-types/src/env.rs:160-260`, `compiler/env.vow:307-335,525-528`) or a user fn with those effects will start failing with `EffectViolation`. The implementer must find them empirically (no `build/vowc`/`target/release/vow` is present in the workspace; build first) and add `read`/`write` to the declarations (`[io, read]`, etc., canonical order is whatever `print_effects` emits — effect lists are sorted by the printer, parser accepts any order).
- Builtin effect assignments are NOT changed (e.g. `stdin_*`/`args`/`getenv` stay `read`; `fs_mkdir`/`fs_remove*`/`fs_rename`/`mktemp_dir` stay `io`). Out of scope; see Out of scope.
- The spec `io` table row lists "stdin" though `stdin_*` are `read` builtins; fixing the wording is mandatory (step 5) because independence makes the mismatch user-visible.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `vow-types/src/effects.rs` | Rust `effect_covered` (the subsumption) + tests `io_subsumes_read`/`io_subsumes_write` | 345-353, 756-780 (test helper `env_with_read_file` ~725) |
| `compiler/env.vow` | Self-hosted `effect_covered` with `io_decl` short-circuits | 135-165 |
| `compiler/checker.vow` | Sole caller of `effect_covered` | 4603 |
| `docs/spec/grammar.md` | Effect table + independence sentence + propagation examples | 1235-1275 |
| `docs/spec/errors.md` | `EffectViolation` entry (fix text) | ~260-270 |
| `tests/error/` | Compile-error fixtures (`// TEST: error-code EffectViolation`) | model: `panic_effect_unwrap_pure_fn.vow` |
| `tests/run/` | Run fixtures; some may rely on subsumption | grep result: 15 files in `tests/` call read/write builtins |
| `compiler/*.vow` (17 files call read/write builtins), `examples/` (5), `bench/` (1), `bench/memory/programs/` | Corpus that must keep compiling; mostly `main`-style `[io]` fns | find via build errors |
| `skills/vow/reference/grammar.md`, `vow/src/skill.rs`, `compiler/main.vow` (5000, 10993) | Generated copies of the spec/help text | regenerate only if spec text changes |

## Steps (ordered so every commit stays buildable)

Ordering constraint: if either checker is tightened before `compiler/*.vow` is migrated, stage 0 and stage 1 reject the compiler's own sources and bootstrap cannot run. So: migrate first (accepted under today's checkers), then flip both checkers together.

### 1. Discover violation sites with a throwaway flip (not committed)
- Locally change `effect_covered` in `vow-types/src/effects.rs:345-353` to `declared.contains(needed)`, `cargo build --release -p vow -j2`, run `./target/release/vow --no-verify <file>` over every `.vow` under `compiler/`, `examples/`, `tests/run`, `tests/debug`, `tests/verify*`, `tests/multi`, `tests/decl`, `tests/fixtures`, `tests/mutants`, `bench/` (incl. `bench/memory/programs/`), `benchmarks/`, `docs/spec/*.md` code blocks. Collect every `EffectViolation`; fixing a function exposes its callers, so iterate to fixpoint. Then `git checkout vow-types/src/effects.rs` (revert the flip). Confirm first (grep both checkers) that neither has an unused/over-declared-effect diagnostic, so adding `read`/`write` is accepted today.

### 2. Migrate the corpus (commit 1; accepted by the unchanged checkers)
- Add the missing `read`/`write` to each offending fn's effect list (`[io, read]`, etc.), written sorted (printer canonical order). Keep behaviour unchanged. Covers `compiler/*.vow`, `examples/`, `tests/run|debug|verify*|multi|decl|fixtures|mutants`, `bench/**`, `benchmarks/*/{skeleton,reference}.vow`, and inline `.vow` strings in Rust tests (`vow/tests/`, `vow-types/tests/`, `vow-ir/src/lower/*` tests; grep `\[io\]`). Do not touch `tests/error/*` fixtures expecting other errors.
- Doc code blocks: grep `docs/spec/{examples,index,stdlib,contracts,grammar}.md` and `benchmarks/*/spec.md` for `[io]`-only fns calling Read/Write builtins and fix them (benchmark skeletons are fed to LLMs).
- Golden outputs that print effect lists: `tests/decl/` (`compiler/decl_text.vow`), IR dumps (`vow-ir/src/printer.rs`, expected `.ir`/snapshots), complexity JSON (`compiler/complexity_main.vow`, `vow/src/complexity.rs`). After migrating, grep expected-output files for effect lists of touched sources and update them for both compilers.
- Gate: `scripts/bootstrap.sh --skip-cargo --no-cache` and `cargo test --all` still green with unchanged checkers.

### 3. RED: Rust unit tests + end-to-end fixtures (commit 2, with step 4 flip)
- `vow-types/src/effects.rs`: replace `io_subsumes_read` (756-763) / `io_subsumes_write` (766-780) with `io_does_not_cover_read` / `io_does_not_cover_write` asserting exactly one `ErrorCode::EffectViolation` whose message contains `read_file` / `write_file` (mirror `pure_caller_calling_effectful_fn_emits_violation`, 737-745); add `io_and_read_covers_read` (`vec![Effect::IO, Effect::Read]` → empty). Reuses `TestEmitter`, `make_fn`, `simple_body`, `env_with_read_file`.
- `tests/error/io_does_not_cover_read.vow`, `tests/error/io_does_not_cover_write.vow` `[new]`: `// TEST: error-code EffectViolation`, `[io]` fn calling `fs_read`/`fs_write`, plus `main() [io]` (model: `tests/error/panic_effect_unwrap_pure_fn.vow`).
- `tests/run/io_read_write_declared_together.vow` `[new]`: fn with `[io, read, write]` using `print_str`, `fs_exists`, `fs_write` (path from `mktemp_dir`, cleaned up); `// TEST: exit 0`.
- `compiler/tests/test_effect_covered.vow` `[new]`: truth table (io↛read, io↛write, read↛io, {io,read} covers read, needed==0 always covered, panic/unsafe unchanged). Run: `build/vowc test compiler/tests/test_effect_covered.vow`.

### 4. GREEN: flip both checkers together (same commit as step 3)
- `vow-types/src/effects.rs:345-353`: `effect_covered` becomes `declared.contains(needed)`.
- `compiler/env.vow:135-165`: delete `io_decl`; use `has_eff_bit(needed, eff_read) && !has_eff_bit(declared, eff_read)` and symmetric write; io check as `!has_eff_bit(declared, eff_io)`. Contract unchanged (function stays verifiable). Sole caller: `compiler/checker.vow:4603`.

### 5. Docs/spec (mandatory)
- `docs/spec/grammar.md:1238-1249`: keep the independence sentence; fix the `io` row so it no longer lists "stdin" — derive wording from actual builtin assignments (`vow-types/src/env.rs:160-260`: stdin_*/args/getenv/fs reads = read; fs_write = write; print/eprint/process/time/mkdir/remove/rename/mktemp = io). Add a sentence and example: a function that prints and reads a file declares `[io, read]`.
- `docs/spec/errors.md` (~260-270) fix text: mention `[io, read]` form. Mirror in `docs/vow_design.md:259` if it holds the same row.
- `uv run python scripts/generate_help.py` (regenerates `skills/vow/reference/grammar.md`, `vow/src/skill.rs`, embedded text in `compiler/main.vow`), then `python3 scripts/check_help_coverage.py` and `python3 scripts/generate_operations.py --check`.
- No ADR (restores a documented rule); PR body notes this is a breaking tightening.

### 6. Verifier parity check
- `python3 scripts/parity.py c RUST_BIN SELF_BIN tests/verify*/*.vow`: migrated effect lists must not change emitted C (non-modelable callers skip identically in both emitters).

## Testing
- `cargo test -p vow-types effects::` (steps 1), then `cargo test --all` and `cargo clippy --all --all-targets -- -D warnings`, `cargo fmt --all --check`.
- `scripts/bootstrap.sh --skip-cargo --no-cache` on the final head SHA (record SHA in the PR checklist per CLAUDE.md) — proves `compiler/*.vow` self-compiles under the new rule and the binary fixed point holds.
- `build/vowc test compiler/` (includes new `test_effect_covered.vow`), `VOW_FULL_TEST_SKIP_CARGO=1 scripts/full_test.sh` (~40 min; background it and poll with bounded intervals), including `tests/error/io_does_not_cover_*.vow` for both compilers.
- Remember env quirks from memory: ~8 vow e2e tests fail in the sandbox without a linked runtime; use `VOW_CACHE_DIR=$(mktemp -d)` when validating after compiler rebuilds.

## Risks
- **Breadth of migration (highest):** hundreds of corpus functions may need `read`/`write`; each missed one breaks bootstrap, `full_test.sh`, or a benchmark. Mitigation: the compiler-error-driven loop in steps 1-2, plus final bootstrap + full_test.
- **Binary fixed point:** only the `.vow` source text of `compiler/` changes (effect lists), so stage-1/2 outputs must still match; effects are not codegen-relevant. Verify with bootstrap triple.
- **Parse→print idempotency:** effect list order is canonicalised by `vow-syntax/src/printer.rs` (`print_effects` sorts); migrated sources may be written in any order but should be written sorted (`[io, read, write]`) to avoid format-check noise.
- **Verifier C parity / verify fixtures:** effects make a fn non-modelable; adding `read`/`write` to a fixture fn that was previously `[io]` doesn't change that. Fixtures with no effects are unaffected. Re-run parity (step 5).
- **`bench/`, `benchmarks/*/reference.vow`, `memory/programs` bounds:** adding effects doesn't alter RSS; `validate-references` should be spot-run on any touched benchmark file.
- **Clippy gate:** test removal/addition only; `--all-targets` is enforced.
- **Third-party/agent skill text:** `skills/vow/reference/` is generated; do not hand-edit.

## Out of scope
- Self-hosted diagnostic `"effectful call from pure function"` (`compiler/checker.vow:4604`) becomes a misleading message when an `[io]` fn lacks `read`/`write`; improving it (naming the missing effect) is a follow-up.
- Changing builtin effect assignments (stdin/args/getenv as `read`, `fs_mkdir`/`fs_remove*` as `io`, or adding a finer-grained split) — file a follow-up if the doc/impl mapping of `stdin` vs `read` needs a real decision.
- Any effect-inference or `[io]`-implies shorthand (would re-introduce a hierarchy, rejected).
- Refactoring `effect_covered` into a shared table, formatting/cleanup of touched files, the unrelated `panic`/`unsafe` rules.
- Native verifier (`compiler/vc_*.vow`) changes (no effect-coverage logic there).
