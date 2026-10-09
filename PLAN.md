# Plan: implement `vowc decl` in the self-hosted driver (#595)

## Goal
`build/vowc decl [-o PATH] <source.vow>` writes the same `.vow.d` declaration stub the Rust `vow decl`
writes, instead of falling through `CMD_NONE` -> `run_legacy` and printing IR with exit 0. Implementing
(not de-advertising) is required: the self-hosted compiler is the primary compiler, `decl` is in
`docs/spec/cli.md` and in both compilers' help (generated from cli.md), and the self-hosted `use`
resolver already prefers `.vow.d` stubs, so it must be able to produce them.

## Assumptions
- Implement rather than reject (the issue's "proposed fix" lists both; CLAUDE.md's dual-compiler rule and "Rust-only landings are debt" point to implement). Best guess.
- Issue line numbers are stale. Current: `get_subcommand` compiler/main.vow:128-155, `main` :16284-16333, `run_legacy` :2427, help text `decl` entries :2586/:2938-2952/:3147/:3531/:3588 (all inside generated `GENERATE:SKILL_*` blocks), `CMD_*` consts compiler/cli_flags.vow:3-10.
- Rust behaviour is the contract. `run_decl_command` (vow/src/main.rs:738): `prepare_frontend(MergedAst)` (parse + load `use` deps + merge + typecheck) -> `print_declarations(merged)` (vow-syntax/src/printer.rs:26) -> write file -> stderr `wrote <path>`, exit 0, stdout empty. Merged module has `uses` empty, so output has no `use` lines and contains dependency items first, entry items last. Frontend diagnostics go to stderr, then `vow decl: <failure message>`, exit 1. Missing source: `vow decl: source file required (try --help)`, exit 1. Default output: append `.d` to the extension (`foo.vow` -> `foo.vow.d`; no extension -> `foo.d`). Clap flags: `-o/--output <path>`, `--help`, `--human` only (no `--module-root`, no prover flags).
- Two self-hosted AST gaps vs Rust; closing them is IN scope because silent wrong output is the very bug being fixed:
  1. `parse_module_into` (compiler/parser.vow:205) consumes `pub` and discards it; Rust prints `pub `. Add `pub_items: Vec<i64>` to `AstArena` (packed `item_pack` values of `pub` items; arena is shared across `use`d files so packed ids are unique). Chosen over changing `fn_data`/`sdef_data`/... strides (every accessor hardcodes the stride).
  2. Struct-like enum variants (`V { f: T }`): parser (compiler/parser.vow:~445) drops the field names, so they cannot be printed. Not in `docs/spec/grammar.md`'s enum section. Add `struct_variant_enums: Vec<i64>` (edef ids) to `AstArena`; `decl` fails closed (stderr error naming the enum, exit 1) when a printed enum is listed. Do not teach the AST to keep the names (out of scope).
- Struct-like variants: static read only (no `build/vowc` in this workspace to probe): checker.vow:1037 consumes the flattened `payload_lid` as a tuple payload, so the self-hosted frontend ACCEPTS `enum E { V { f: i64 } }` silently. Fail-closed in `decl` is therefore justified; the implementer should first confirm with a built `build/vowc` (if the checker turns out to reject it, drop `struct_variant_enums`, its parser hook, and the cli.md divergence note).
- `impl`/`trait` items are not parsed by the self-hosted compiler at all (`parse_item` -> "expected item"), so the decl printer needs no arms for them; `build` fails identically today.
- Output byte-parity target: Rust `print_declarations`. Golden files generated once from the Rust compiler and checked in; both compilers are compared to them.
- Printer lives in a new small module `compiler/decl_text.vow` (pure `AstArena`/`Module` -> `String`, reusing `contract_text.vow`); driver in new `compiler/decl_main.vow` (pattern: `complexity_main.vow`/`mutants_main.vow`), so the 16k-line main.vow only gains a dispatch arm.

## Files to touch
| File | Role | Lines |
|------|------|-------|
| `vow/src/main.rs` | reference behaviour (`run_decl_command`, `Command::Decl` arm); NOT modified | 738-790, 941-958 |
| `vow-syntax/src/printer.rs` | reference printer: `print_declarations`, `print_fn_decl`, `print_struct`, `print_enum(_variant)`, `print_const`, `print_type_alias`, `print_extern(_fn)`, `print_params`, `print_effects`, `print_vow_block`; NOT modified | 26-95, 112-200, 284-424 |
| `compiler/cli_flags.vow` | add `CMD_DECL() = 8`, `cf_kind_decl`, case in `cli_flag_kind`; `-o`/`--output` already in `cf_is_build_value` so `cli_flag_takes_value` needs no change | 3-10, 66-76 |
| `compiler/main.vow` | `get_subcommand` arm; `main` dispatch arm; `use decl_main` | 128-155, 16284-16333 |
| `compiler/decl_main.vow` [new] | `run_decl(argv) -> i32 [io, read, write]` | - |
| `compiler/decl_text.vow` [new] | `decl_module_text(m: Module, pub_set: HashMap<i64,bool>) -> String` + per-item printers | - |
| `compiler/contract_text.vow` | reuse `print_type_text` (:22), `print_expr_text_at` (:194), `print_block_text`, `indent_text` (:71), clause printing pattern `print_loop_vow_text` (:498) | |
| `compiler/ast.vow` | `AstArena` (+2 fields in struct and `arena_new`), accessors `fn_*` :399-405, `sdef_*`, `edef_*`, `cdef_*`, `alias_*`, `ext_*`; `Module` :557 | 94-133, 399-440 |
| `compiler/parser.vow` | record `pub`, module name, struct-like variants | 187-190, 205-209, 214-220, 427-470 |
| `compiler/frontend.vow` | `frontend_prepare_path_with_root` returns `FrontendPrep.merged` (reuse); set merged `name_sid` from entry `m` | 289-295 |
| `docs/spec/cli.md` | flesh out `vow decl` section (:180-192) | |
| `scripts/generate_help.py` | regenerate main.vow help blocks, `vow/src/skill.rs`, `skills/vow/reference/cli.md` after cli.md edit | 257, 368, 444 |
| `tests/cli-flags/tests.sh` | add `decl` unknown-flag cases | |
| `tests/decl/` [new] | `tests.sh` + fixtures + goldens | |
| `scripts/full_test.sh` | wire `tests/decl/tests.sh` for both compilers (next to Section 4h, :1456-1473) | |
| `compiler/tests/test_decl_text.vow` [new] | unit tests of the printer (model on `compiler/tests/test_checker_diag_text.vow`) | |

## TDD slices (each red -> green -> commit; commit type `feat(cli)`/`test` etc., lower-case subjects)

### Slice 1 - routing: `decl` is no longer silently legacy (red: wrong exit/stdout)
- Test: `tests/decl/tests.sh` [new], case "missing source": `vowc decl` -> exit 1, stderr `vow decl: source file required (try --help)`; case "not legacy": `vowc decl examples/hello.vow -o $T/h.vow.d` exits 0, stdout EMPTY (today it prints IR), file exists. Add to `tests/cli-flags/tests.sh`: `decl --no-verify x` and `decl --module-root d x` -> exit 2 `unexpected argument`.
- Code: `CMD_DECL` + `cf_kind_decl` (bool: help/human; value: `-o`,`--output`) in cli_flags.vow; `get_subcommand` arm; `main` arm `if cmd == CMD_DECL() { return run_decl(argv); }` placed before `maybe_auto_install_skill()`; `run_decl` first version: `frontend_path_from_argv(argv, true, "decl")`, `frontend_prepare_path(path)`, print diagnostics (`diag_ctx_print_all`, as run_contracts :15957), exit 1 on errors with `vow decl: <msg>` where Rust's msg is stage-based (vow/src/frontend.rs:119-126): `parse error` | `module load error` | `type error` | `region error` (self-hosted: map a parse failure / unresolved `use` / checker errors to the first three; `region error` is unreachable for MergedAst), and an IO failure prints `vow decl: <io message>`; test asserts stderr contains `vow decl: type error` for a type-error fixture and `vow decl: module load error` for a missing `use`; output path helper `decl_default_output` (ext + ".d"), write via `fs_write`, `eprintln_str("wrote <path>")`. Body prints only `module <name>\n` for now.
- Module name: Rust prints the ENTRY module's name (module_loader.rs:241 `name: root_module.name`). The self-hosted parser DISCARDS it (`let _nm = expect_ident(p)`, compiler/parser.vow:187-190) and both `Module` literals hard-code `name_sid: 0` (parser.vow:215, frontend.vow:290; nothing reads it). Fix: parser stores the interned name sid (`-1` when the header is absent) in `Module.name_sid`; `frontend_prepare_path_traced` builds `merged` with `name_sid: m.name_sid`. `decl` prints `module <arena_str(name_sid)>`; a header-less file prints what Rust prints for an empty name (check `parse_module` in vow-syntax/src/parser; mirror it).

### Slice 2 - function declarations (the `.vow.d` core)
- Test: `compiler/tests/test_decl_text.vow` + golden fixture `tests/decl/fn_basic.vow`: plain/pub fn, params with `where` refinement, unit vs non-unit return (`-> T` omitted for unit), effects sorted alphabetically `[io, panic, read, unsafe, write]` (`print_effects`), `vow { requires:/ensures: }` block inline (rust `push_vow_block_inline`; mirror `print_vow_block` layout exactly), terminated by `;`. Bodies never printed.
- Code: `decl_text.vow`: `decl_fn_text`, `decl_params_text` (list stride 4: name, type, refinement, span), `decl_effects_text` (bitmask `EFF_*`, ast.vow:89-93), `decl_vow_text`. Introduce `pub_items` in AstArena + parser (`pub` recorded in `parse_module_into`) in this slice; `decl_module_text` takes a `HashMap<i64,bool>` built once from `arena.pub_items` (O(n), order-independent so deterministic).

### Slice 3 - struct / enum / const / type alias / extern
- Tests: goldens `tests/decl/types.vow` (struct, `linear struct`, pub struct, empty struct, enum with unit/tuple variants, const incl. negative literal and bool, type alias, `extern "C" { vow {..} fn ...; }` with and without return). Include a generic type (`Vec<Option<i64>>`), tuple and unit types (type printer already exists).
- Code: `decl_struct_text` (fields `    name: T,\n`), `decl_enum_text`, `decl_const_text`, `decl_alias_text`, `decl_extern_text` (extern fn printed through `decl_params_text` + effects; `print_extern_fn` ends `;\n`). Item separator: blank line between items; header `module N\n` then blank line when items non-empty (printer.rs:26-43; no `use` lines since merged).
- Struct-like enum variants: add `struct_variant_enums` to AstArena, record in `parse_enum_def`, `decl` errors `vow decl: enum <Name> has a struct-like variant, which the self-hosted compiler cannot print` exit 1; test with `tests/decl/struct_variant.vow` expecting exit 1 (Rust keeps its current behaviour; document the divergence in cli.md under the self-hosted notes and add `// KNOWN` note in tests.sh for Rust-skip).

### Slice 4 - multi-module + golden parity wiring
- Test: `tests/decl/multi/` (entry `use`-ing `dep.vow`; also a `dep.vow.d` stub-preferred case copied from `tests/multi/decl_stub_preference`) - expected: dep items first, no `use` lines. `tests/decl/tests.sh` runs both `VOWC_KIND=rust|self`, for every fixture runs `decl -o $TMP/x.vow.d`, byte-diffs against `<fixture>.vow.d.expected` (goldens generated ONCE with `target/release/vow decl`, reviewed by hand) and checks default-output naming (`foo.vow` -> `foo.vow.d`, extensionless -> `foo.d`) and type-error input (exit 1, no file written, stderr has diagnostic JSON + `vow decl:`). Round-trip check: output of decl on a golden fixture, fed back as `use` stub, still `verify`s (guards the "stub hides contract" rule).
- Wire into `scripts/full_test.sh` as Section 4i beside 4h with the same pass/fail log pattern. Update the `tests/run_tests.sh` only if it enumerates dirs (check; no `TEST:` directives are needed).

### Slice 5 - spec + generated help
- `docs/spec/cli.md` `### vow decl`: document merged-dependency semantic (deps inlined, no `use` lines), stderr `wrote <path>`, exit 1 on diagnostics/IO error, overwrite behaviour, flags allowed, self-hosted struct-like-variant refusal. Then `uv run python scripts/generate_help.py` (regenerates compiler/main.vow GENERATE blocks, vow/src/skill.rs, skills/vow/reference/*); run `python3 scripts/check_help_coverage.py`. Help wording for `decl` stays "Emit declaration file (.vow.d) with type signatures only"; if generate_help changes it, both compilers change together.
- Bump nothing else; no ADR needed (no architecture decision beyond this plan). Check `docs/spec/index.md` mentions.

## Verification surface
- No new contract language features. New compiler functions are ordinary Vow; bootstrap Stage 1 verifies every `vow` block it finds - add none unless they are TRUE semantic contracts (e.g. `decl_default_output` ensures result ends with ".d"); never add bounds for ESBMC. Functions ESBMC cannot model (HashMap use, string building) are skipped by the pipeline - do not distort code to fit.
- C-parity (`c_emitter.rs`/`c_emitter.vow`): untouched; Section 2c is unaffected as no `tests/verify*/` fixture changes. Confirm by running `scripts/full_test.sh` section 2c.
- `tests/decl/` is new fixture surface; `tests/run/`, `examples/` unchanged.

## Risk areas
- Binary fixed point: compiler gains code -> `scripts/bootstrap.sh --skip-cargo --no-cache` must reach fixed point; HashMap in decl_text is not iterated for output (no ordering hazard); nothing touches `vow-clif-shim` or codegen ordering. Record head SHA in the PR checklist per CLAUDE.md.
- AstArena field additions: every `AstArena { .. }` literal must be updated - only `arena_new` (ast.vow:114) builds one (grep to confirm). `pub_items` growth is O(items) per file.
- `parse -> print -> parse` idempotency: unaffected (Rust printer not modified; self-hosted `pub` now retained but not used by `print_module`).
- Byte parity risks: effects ordering, blank-line placement, `where` refinement indentation (`print_expr_at(pred, level)`), extern `.replace(";  \n", ";\n")` quirk, integer literal rendering (`u128_limbs_to_text`), nested-block indentation (#1540 just changed this in both). Mitigated by goldens covering each.
- Merge order: Rust iterates `graph.modules` (dependency order, root last; module_loader.rs:232-238) - confirm self-hosted `all_items` (deps via `load_frontend_deps`, then root items, frontend.vow:277-287) yields the same order for diamond/transitive `use` graphs with a golden; if not, mirror Rust.
- `cargo clippy --all --all-targets -- -D warnings`: no Rust source changes expected (generate_help rewrites skill.rs strings only); run it anyway after regeneration.
- codecov/patch (blocking 95%): the cli.md edit flows through `generate_help.py` into `vow/src/skill.rs` string literals; keep the cli.md prose change small, and confirm an existing cargo test touches the skill text (vow/src/skill.rs tests / vow/tests/cli_dispatch.rs) - otherwise add one assertion that `skill::json()` still contains the `decl` usage. No other Rust lines change.
- Concurrent bootstrap flake and compile-cache staleness (memory notes): use `VOW_CACHE_DIR=$(mktemp -d)` when validating.
- Dual-compiler rule: Rust side already implements decl, so no Rust semantic change; stated explicitly in the PR body.

## Testing / commands (separate invocations, background + poll for long ones)
- `build/vowc test compiler/tests/test_decl_text.vow`; `build/vowc test compiler/`
- `bash tests/decl/tests.sh` (VOWC_BIN/VOWC_KIND for each compiler); `bash tests/cli-flags/tests.sh`
- `scripts/bootstrap.sh --skip-cargo --no-cache` (fixed point), `cargo test --all`, `cargo clippy --all --all-targets -- -D warnings`, `cargo fmt --all --check`, `python3 scripts/check_help_coverage.py`, `python3 scripts/generate_operations.py --check`, `scripts/full_test.sh` (~40 min).
- PR title e.g. `feat(cli): implement vowc decl in the self-hosted driver` (<= 92 chars, lower-case). Squash merge only. Delete `PLAN.md` (git rm) before opening the PR.

## Out of scope
- Keeping struct-variant field names in the AST; `impl`/`trait` parsing in the self-hosted compiler; changing Rust `print_declarations`/`run_decl_command`; `--module-root` for `decl`; refactoring main.vow; making `build/vowc` reject other unknown subcommands (separate #1539 line of work); regenerating unrelated help text.
