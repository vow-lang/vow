# Plan: chunk the embedded help/skill payload in compiler/main.vow (issue #177)

## Goal
Cut the front-end/IR/codegen cost of the generated help/skill payload in `compiler/main.vow` by emitting each
payload as a few hundred bounded string-literal chunks instead of ~13,200 one-line `push_str` statements, with
byte-identical `--help`, `--help --human`, `skill print [--bundle]` and `skill install` output.

## Findings that drive the design
- `compiler/main.vow` is 1,335,549 B / 16,708 lines. The three GENERATE blocks are ~1.19 MB (89%):
  `SKILL_JSON` 2636-3617 (76 KB), `SKILL_HUMAN` 3619-3741 (17 KB), `SKILL_FULL` 3743-15969 (1.09 MB: `skill_entrypoint`,
  `skill_bundle` ~6k stmts at 3799-9887, 15 `skill_support_content_*` fns at 9913-15943).
- 13,183 payload statements `r.push_str(String::from("<line>\n"));`. Each is lexer tokens + AST call/path/literal +
  type-check + IR const-str/call/call + one clif-shim FFI call per inst + a stack slot, all inside ~17 huge functions.
- `compiler/lower.vow:1056` `lctx_intern_str` is a linear scan over the per-function string pool (reset at
  `lower.vow:6401`), so `skill_bundle` (~6k distinct literals) is O(n^2) string compares in the self-hosted compiler.
  Rust twin (`vow-ir/src/lower/mod.rs:1264`) uses a HashMap.
- **Trap:** the verifier C model sizes every string buffer by the longest literal in the *whole module*:
  `string_max_for_literals` (`compiler/c_emitter.vow:3760`, called from `verifier.vow:265`, `c_emitter.vow:3811,3832`)
  and its Rust twin `vow-verify/src/c_emitter.rs:3378` (`module.strings.iter().map(len).max()`). Emitting each payload
  as ONE literal (up to ~100 KB) would inflate `VOW_STRING_MAX` for every function verified in the compiler
  (bootstrap Stage 1 verification) and change the C handed to ESBMC. Chunking at line boundaries with a cap at or below the
  current longest *decoded* literal leaves that maximum, and so all other functions' C, unchanged. (The 3,540 figure
  from `awk` is a source-line length including the `    r.push_str(String::from("` prefix and `\"` escapes; it is NOT
  the decoded literal length the emitter sees. Step 0 measures the real decoded maximum.)
- Existing emission quirk to preserve: `_vow_pushstr_body` (`scripts/generate_help.py:892`) appends `\n` to *every*
  element of `text.split("\n")`, so emitted bytes == `text.replace("—","--") + "\n"`. Do not "fix" it here.
- Rust `vow/src/skill.rs` already uses one raw string per payload; it is not touched.
- Lexer (`compiler/lexer.vow:356-389`, Rust twin) handles `\n \t \r \\ \" \0`; the generator only emits `\\`, `\"`,
  `\n`. Each chunk must stay on one source line (`scripts/concat_vow.sh:13` strips blank lines and lines starting
  `use `/`module `, so raw newlines inside literals would corrupt the concat build).

## Assumptions
- Issue wording "moving out of main.vow *or* equivalent" is satisfied by cost reduction; moving the payload to its own
  module reduces no compile cost (single DFS-loaded program) and has a wider blast radius, so it is a follow-up (best guess).
- No language, CLI or spec change => no `docs/spec/*.md` update (best guess; `check_help_coverage.py` and
  `generate_help.py --check` still run as gates).
- Dual-compiler rule: the change is to the *generated Vow source* that both compilers consume; there is no compiler
  code change, so no Rust/self-hosted drift is introduced.

## Files to touch
| File | Role | Lines |
|------|------|-------|
| `scripts/generate_help.py` | emitter: new chunked payload emitter; used by `inject_vow`, `inject_skill_vow` | 892-930, 1120-1200 |
| `scripts/test_generate_help.py` | new tests for the emitter (CI runs it: `.github/workflows/ci.yml:105`) | whole file |
| `compiler/main.vow` | regenerated output only (never hand-edited) | 2636-15969 |
| `compiler/lower.vow` | context only (`lctx_intern_str`) | 1056-1067 |
| `compiler/c_emitter.vow`, `vow-verify/src/c_emitter.rs` | context only (string-max coupling) | 3760; 3378 |
| `scripts/generate_operations.py` | `check_doc_facts` greps main.vow for `\"name\": \"sig eff\"`; must keep passing | 633-663 |
| `vow/src/skill.rs`, `skills/vow/**` | must be byte-unchanged by regeneration | - |

## Steps (TDD slices; one PR, conventional title e.g. `perf(driver): chunk embedded help/skill payload in main.vow`)

### 0. Baseline capture (no commit)
- Build baseline at HEAD: `cargo build --release -p vow -j2` then `scripts/bootstrap.sh --skip-cargo` (background + poll
  a done-marker file; ~5 min). Keep `$TMPDIR/base/vowc` (copy of `build/vowc`) and `cp -r compiler $TMPDIR/base/compiler`.
- With `HOME=$TMPDIR/home` and a temp cwd (avoid `maybe_auto_install_skill` side effects, `main.vow:16119`) record sha256 of:
  `vowc --help`, `vowc --help --human`, `vowc skill print`, `vowc skill print --bundle`, and a hash of the tree from
  `vowc skill install --local`. Also the Rust `target/release/vow` equivalents.
- Record cost of compiling the OLD source with the baseline binary: wall time and peak RSS of
  `$TMPDIR/base/vowc build --no-verify $TMPDIR/base/compiler/main.vow -o $TMPDIR/x` (no GNU `time` in this sandbox:
  wrap with `python3 -I -c 'import resource,subprocess,sys,time;...ru_maxrss of RUSAGE_CHILDREN'`; 3 runs, take median);
  also count payload statements and `--dump-ir` instruction count for `skill_bundle`.
- Record the pre-change **decoded** maximum literal length over the whole compiled program (every module in the use graph,
  because `m.strings` is program-wide): from the `--dump-ir` string table or a Python unescape of all string literals in
  `compiler/*.vow`. Call it `BASE_MAX_LITERAL`. Requirement: `VOW_LITERAL_CHUNK_BYTES <= BASE_MAX_LITERAL` (2048 is
  expected to satisfy this; if not, lower the cap).
- Step-0 baseline binary only needs `scripts/bootstrap.sh --skip-cargo --no-verify`; the full verifying
  `--no-cache` run is reserved for step 4 on the final head SHA.

### 1. RED: emitter round-trip + bounded-literal tests
- File: `scripts/test_generate_help.py`, new `class VowPayloadEmissionTest`.
- Tests (all fail until the new helper exists):
  - `test_round_trip_preserves_bytes`: for texts `"a"`, `"a\n"`, `"a\n\nb"`, text with `"`, `\`, `—`, empty
    string, a 10 KB single line, and ~100 lines: decode the emitted statements with a small Python mirror of the lexer
    unescape rules (`\n \t \r \\ \" \0`) and assert result == `text.replace("—","--") + "\n"`.
  - `test_chunks_never_split_lines_and_respect_cap`: every literal is a whole number of source lines; its decoded
    length <= max(cap, longest line+1).
  - `test_injected_block_round_trips`: run `inject_vow` and `inject_skill_vow` on a small fixture file containing the
    GENERATE markers, decode every `String::from("...")` / `push_str` literal in the returned function bodies, and assert
    the concatenation equals `text.replace("\u2014","--") + "\n"` per function. This catches template bugs the helper-only
    tests cannot: both templates currently hard-code `String::from("{first}\\n")` (`scripts/generate_help.py:909,918,1134,1140,1165`)
    and would append a spurious extra `\n` once `first` is a multi-line chunk.
  - `test_chunk_cap_invariant_on_real_payload`: over the real generated payloads, every decoded literal has
    length <= max(`VOW_LITERAL_CHUNK_BYTES`, longest single line + 1) and `VOW_LITERAL_CHUNK_BYTES <= BASE_MAX_LITERAL`
    (constant pinned from step 0, with a comment saying why).
  - `test_one_statement_per_source_line`: no raw newline inside a literal; first statement is `let r: String = String::from(...)`.
  - `test_doc_fact_substring_survives`: `\"print_str\": \"fn(s: String) -> () [io]\"` present after emission of a JSON snippet.
- Code to make it pass (step 2): `scripts/generate_help.py`.

### 2. GREEN: chunked emitter
- File: `scripts/generate_help.py`. Replace `_vow_pushstr_body` (892-902) by `_vow_payload_stmts(text) -> tuple[str, str]` returning
  `(first_literal_escaped_with_its_own_trailing_escapes, rest_statements)`, and change the four templates in `inject_vow`
  (909, 918) and `inject_skill_vow` (1134, 1140, and the per-support-file loop at 1161-1165) to drop the hard-coded `\\n` after
  `{first}` -- the helper now owns every newline.
  Add module constant `VOW_LITERAL_CHUNK_BYTES = 2048`.
- Algorithm: `vow_text = text.replace("—","--") + "\n"`; split into lines keeping each line's trailing `\n`
  (identical bytes to the old per-line literals); greedily accumulate whole lines until adding the next would exceed
  `VOW_LITERAL_CHUNK_BYTES` (a line longer than the cap is its own chunk); escape `\\`, `"`, `\n` per chunk; first chunk
  goes into `let r: String = String::from("...");`, the rest into `    r.push_str(String::from("..."));`.
- Keep a one-line comment on the constant explaining it must stay <= the longest per-line literal (verifier string max).
- Update module docstring line 7 ("push_str calls") to say chunked literals.

### 3. Regenerate and prove byte-for-byte parity
- `uv run python scripts/generate_help.py`; then `git diff --stat` must show ONLY `compiler/main.vow`
  (`vow/src/skill.rs` and `skills/vow/**` unchanged) and `git status` clean of anything else.
- `python3 scripts/generate_help.py --check`, `python3 scripts/test_generate_help.py`, `python3 scripts/test_generate_operations.py`,
  `python3 scripts/generate_operations.py --check`, `uv run python scripts/check_help_coverage.py docs/spec/grammar.md <help json>`.
- Assert statement count (`grep -c 'r.push_str(String::from("'` over the three blocks) fell from 13,183 to a few hundred and
  that the program-wide decoded maximum literal (same measurement as step 0) is still exactly `BASE_MAX_LITERAL`.

### 4. Behavioural verification of both compilers
- `cargo build --release -p vow -j2`; `scripts/bootstrap.sh --skip-cargo --no-cache` (records stage1/2/3 fixed point).
- Re-capture the step-0 hashes with the NEW `build/vowc` and the Rust `vow`: all must be identical to baseline.
- `build/vowc test compiler/` is not needed (no compiler logic change) but run `--filter` smoke for lexer/lower tests.
- Run the relevant `scripts/full_test.sh` sections (background + poll; ~40 min full): help sections ~1900-1935, IR parity
  ~622 (Rust vs self-hosted `--dump-ir` of main.vow must still match), 2c C parity, ops/catalogue-drift.
- Record the checked head SHA (CLAUDE.md "green locally" rule) in the PR body.

### 5. Measure improvement (acceptance criterion 4)
- Compile NEW source with the SAME baseline binary (`$TMPDIR/base/vowc build --no-verify compiler/main.vow`) and compare
  wall time + peak RSS against step-0 numbers for the old source; also new `build/vowc` on new source. Report medians
  of 3 runs in the PR body (expect large wins in self-hosted lowering from removing the O(n^2) pool scan and ~12.6k fewer
  stmts/insts). If wall time does not improve measurably, stop and report rather than lowering the cap blindly.
- Optional: `--perfetto <path>` trace of both for phase breakdown.

## Verification surface
- No contracts change; payload functions have no `vow` blocks. `skill_support_path` / `skill_support_content_index_guard`
  keep their `requires: index < skill_support_count()` untouched.
- C parity (`vow-verify/src/c_emitter.rs` vs `compiler/c_emitter.vow`): not edited. The invariant to defend is that
  `model_string_max` for the compiler module is unchanged; step 3's max-literal assertion is the proof. `scripts/full_test.sh`
  Section 2c runs over `tests/verify*` fixtures (unchanged).
- Stage 1 verification (bootstrap without `--no-verify`) must still pass in comparable time: compare its wall time to baseline.
- No new `tests/run/` or `examples/` fixtures needed; the generator tests and the CLI hash comparison cover behaviour.

## Risk areas
- **String-max inflation** if someone later raises the chunk cap or emits whole-document literals: pinned by
  `test_no_literal_exceeds_longest_line_literal` and the step-3 assertion.
- **Binary fixed point:** string-pool contents/order change but both compilers intern first-occurrence order; Stage 2 == Stage 3
  still holds. Rust-vs-self IR parity (`full_test.sh:622`) compares per-function pool indices; verify it stays green.
- **Trailing-newline quirk** (`+ "\n"` always) and em dash -> `--`: preserved deliberately; any "cleanup" changes CLI bytes.
- **`concat_vow.sh` / `typos` / `trailing-whitespace` hooks:** chunk lines are single-line, non-empty, start with 4 spaces,
  end with `));`; `typos` already scans this file. Commit with hooks enabled (commitlint: lower-case subject, <= 92 chars).
- **Wall-clock/memory budget:** bootstrap ~5 min, full_test ~40 min; cap builds with `-j2`, keep scratch in `$TMPDIR`, never
  `pgrep -f`-wait. Known env failures (vow e2e SKIP-panic, concrete-block-region-parity, u64_marker) pre-exist on clean main.
- **parse -> print -> parse:** no new literal shape (old per-line literals already carried `\n`, `\"`, `\\` escapes); only length
  grows, to <= 2 KB. Covered by the existing vow-syntax printer idempotency tests and `full_test.sh` ~2135 fixtures; no new test.
- **clif-shim stack slots / fixed point:** fewer insts and slots per function; `BTreeMap` `slot_map` ordering is unchanged
  and deterministic, so no shim or codegen-ordering edit is needed.
- **clippy / Rust gates:** no Rust source changes, so `cargo clippy --all --all-targets -- -D warnings` is unaffected.
  Python changes must pass the pinned ruff (`uvx ruff@<pin>`), `ruff-format` and `pydoclint` hooks.
- **codecov/patch (95%, blocking):** `codecov.yml` ignores `compiler` and `scripts` and no Rust is touched, so no
  instrumented lines enter the patch; expected N/A. Confirm on the PR.
- **Stale compile cache** can serve old objects after a compiler rebuild: use `VOW_CACHE_DIR=$(mktemp -d)` for measurements.
- `generate_help.py --check` does not currently diff main.vow/skill.rs against regenerated content, so drift of those
  blocks is only caught by regeneration + `git diff`; step 3 does this manually.

## Out of scope (follow-ups to list in PR body, file as issues if absent)
- Moving the payload into a dedicated module (`compiler/skill_payload.vow`): needs `scripts/concat_vow.sh` FILES lists (ir + clif),
  `generate_help.py` `MAIN_VOW` target, `generate_operations.py:640` + `test_generate_operations.py:1256` fixture,
  CLAUDE.md module list, mutants skip-list check. Reduces file size for agents, not compile cost.
- Replacing `lctx_intern_str`'s linear scan with a hash index (still quadratic for other huge functions).
- Making `generate_help.py --check` verify main.vow/skill.rs blocks, or fixing the trailing-newline/em-dash quirks.
- Any change to Rust `skill.rs`, `docs/spec/*`, contracts or verifier code.
