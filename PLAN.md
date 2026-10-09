# Plan: reject `;` between vow clauses in the self-hosted parser (issue #626)

## Goal
Make both parsers accept the same clause separators in a `vow { ... }` block: an optional `,` (or nothing). A `;` must be a parse error in both, with matching diagnostics, so a program accepted by `build/vowc` is never rejected by the Rust bootstrap compiler.

## Assumptions
- Policy: keep "optional comma / nothing" (what the Rust parser, the canonical printer and grammar.md already do), drop `;`. Dropping the self-hosted `;` is the one-sided, non-breaking direction (best guess; matches the issue's recommendation). Making the comma mandatory is out of scope: it would change the Rust parser and reject currently valid programs.
- Nothing in the repo relies on `;` between clauses (grep of `*.vow`/`*.md`/`*.rs` found only the three `--help` strings below), so no corpus fixture needs migration.
- Error-recovery shape must match the Rust parser, because `scripts/full_test.sh` Section 3 (`tests/error/*.vow`) compares the multiset of `error_code`s (and counts) between compilers. Rust: on the stray token it pushes `UnexpectedToken` and `break`s (no token skipped), then `expect(RBrace)` pushes a second error. The self-hosted loop instead pushes an error and *skips* the token, which would recover silently after `;` and yield 1 error vs Rust's >=2. The implementer must observe real output from both and align recovery (see Step 2).

## Findings that shape Step 2
- `scripts/parity.py::compare_error` compares the **sorted multiset of `error_code`s** (so the full diagnostic count), then spans (drop-detection only) and hints. It does NOT compare messages. A different number of errors between the compilers fails the `tests/error` gate, so the cascade after `;` must match in count.
- Rust: stray `;` -> `UnexpectedToken`, `break`, then `expect(RBrace)?` pushes a 2nd error and returns `None` (vow block lost); the caller then continues and may add further errors (verified in the issue: >=3 messages). Self-hosted: one error + skip token, then silent recovery. The self-hosted `push_error` already emits `EC_UNEXPECTED_TOKEN` (parser.vow:138-139), matching Rust's code, so the fixture header is `// TEST: error-code UnexpectedToken`.
- Single clause-parsing site per compiler: `parse_vow_block` (parser.vow:563, called at 327/395/1169/1177/1189; Rust mod.rs:452, called from mod.rs:325 and items.rs:237/303). No second loop accepts `;`.
- No existing `tests/error` fixture contains "expected requires", so there is no precedent proving the two recovery strategies already agree.
- Changing the self-hosted error arm to `break` alters recovery for *every* bad token in a vow block, not only `;`; Step 2 must re-run the whole `tests/error/*.vow` parity pass.

## Key Files
| File | Role | Lines |
|------|------|-------|
| `compiler/parser.vow` | self-hosted `parse_vow_block`; remove `;` acceptance | 563-611 (`;` at 597-599, `,` at 600-602, error arm 592-595) |
| `vow-syntax/src/parser/mod.rs` | Rust `parse_vow_block`; reference behaviour (no change expected) | 450-518 (error arm 479-490, comma 492-494) |
| `tests/error/` | new fixture: `;`-separated clauses rejected | new file |
| `scripts/generate_help.py` | hard-coded help JSON `vow_function` string uses `;` | 574 |
| `compiler/main.vow` | generated copy of that string | 3165 |
| `vow/src/skill.rs` | generated copy of that string | 866 |
| `docs/spec/grammar.md` | clause separator rule | 99 |
| `docs/spec/errors.md` | only if a separator note is warranted | - |
| `vow-syntax/src/parser/mod.rs` `mod tests` | Rust unit test for `;` rejection | 738+ |
| `compiler/tests/test_parser_spans.vow` (reference for style) | pattern for a self-hosted parser unit test | 1-30 |

## Steps (TDD slices, ordered)

### 1. RED: error fixture + Rust unit test
- **File**: `tests/error/vow_clause_semicolon.vow` [new]; `vow-syntax/src/parser/mod.rs` `mod tests`.
- **Change**: fixture is a function with `vow { requires: x > 0; ensures: result > 0 }` and header `// TEST: error-code UnexpectedToken` (check the exact code the self-hosted `push_error` emits and what Rust emits; parity requires they agree). Also add a Rust unit test `parse_vow_block_rejects_semicolon_separator` asserting `parse_module` returns diagnostics for the `;` form and none for the `,` form and the no-separator form (pins the Rust half so the policy is tested on both sides). Add a positive twin fixture only if one is not already covered: `tests/run/` already exercises comma and no-separator forms (verify with grep before adding; do not duplicate).
- **Run (no cargo-wide runs needed)**: build both compilers per CLAUDE.md (`cargo build --release -p vow`, `scripts/bootstrap.sh --skip-cargo`, `-j` capped), then `build/vowc build --no-verify tests/error/vow_clause_semicolon.vow` and `target/release/vow --no-verify ...`. Expect Rust: error; self-hosted: accepted (the bug), confirming RED.

### 2. GREEN: remove `;` acceptance in the self-hosted parser
- **File**: `compiler/parser.vow` 597-599.
- **Change**: delete the `if at(p, tok_semicolon()) { advance }` block; keep the comma block. Then diff diagnostics for the fixture between the two compilers (`python3 scripts/parity.py` / `scripts/full_test.sh` Section 3 helper `compare_error`). If error counts or codes differ (parity compares the full code multiset, so a count difference is a failure; expected: self-hosted skips the bad token via `let _skip: Token = advance(p)` at line 595 and recovers with one error; Rust breaks and emits a second `expected RBrace` error), change the self-hosted `else` arm to mirror Rust: push the error and `break` out of the loop without advancing, letting the existing `expect(p, tok_rbrace())` emit the second error. Check the loop construct supports `break` here (it does elsewhere in parser.vow; grep). Do not change the Rust parser unless the observed divergence cannot be fixed on the Vow side; in that case adjust both and say so in the PR.
- **Reuses**: existing `push_error`, `expect`, `tok_comma` helpers in `compiler/parser.vow`.

### 3. (Decided: no separate self-hosted unit test)
- The `tests/error` fixture runs through both compilers with count/code parity, and the Rust unit test pins the Rust half; a third test in `compiler/tests/` would be redundant. Do not add one.

### 4. Fix the `--help` / skill text that teaches `;` as a separator
- **Files**: `scripts/generate_help.py:574`, then regenerate `compiler/main.vow:3165` and `vow/src/skill.rs:866`.
- **Change**: replace `requires: <expr>; ensures: <expr>` with `requires: <expr>, ensures: <expr>` in the generator; run `uv run python scripts/generate_help.py` (do not hand-edit generated files) and confirm `python3 scripts/check_help_coverage.py` is clean. Check `skills/vow/` and any golden help snapshot (`grep -rn "<expr>; ensures"` after regeneration must return nothing).

### 5. Spec
- **File**: `docs/spec/grammar.md:99`.
- **Change**: state explicitly that clauses are separated by commas, the comma may be omitted, and a semicolon is a parse error; keep the existing example. If `errors.md` lists parse errors by message, no change is needed (no new code).

### 6. Quality gates (separate commands, background + poll per memory notes)
- `cargo fmt --all --check`, `cargo clippy --all --all-targets -- -D warnings`, `cargo test -p vow-syntax`, `scripts/bootstrap.sh --skip-cargo --no-cache` (record final head SHA), `build/vowc test compiler/` or `--filter parser`, and `scripts/full_test.sh` Section 3 (error parity) / `VOW_FULL_TEST_TIER15_ONLY=1`.

## Verification surface
No contract, codegen or C-model change; ESBMC proves nothing new. `compiler/parser.vow` is verified at bootstrap as before; removing a branch cannot weaken any contract on `parse_vow_block` (it has none). C emitter parity (Section 2c) is unaffected since no fixture under `tests/verify*/` uses `;` between clauses.

## Risks
- Error-parity divergence (count/codes) between recovery strategies: handled in Step 2 by empirical diff and mirroring Rust's break-then-expect.
- Binary fixed point: the edit changes parser behaviour only for previously-invalid-in-Rust input; compiler sources contain no `;` clause separators, so stage 1/2 output stays identical. Confirm with bootstrap.
- Generated-file drift: help string must be changed only in `generate_help.py` and regenerated, else `check_help_coverage.py` / CI drift check fails.
- `parse -> print -> parse` idempotency unaffected (printer never emits `;`).
- Clippy: the new Rust unit test must pass `cargo clippy --all --all-targets -- -D warnings` (no unused bindings; use `assert!`/`assert_eq!`, not ad-hoc `panic!` beyond the file's existing style).
- Pre-existing failures (see memory: concrete-block-region-parity, u64_marker, e2e SKIP-panic) are not caused by this change; verify against clean origin/main before attributing.
- Commit/PR title must be lower-case conventional commit, e.g. `fix(parser): reject semicolon between vow clauses in self-hosted parser`.

## Out of scope
- Making the comma mandatory or forbidding it (language change; not needed to close the issue).
- Any other parser-parity divergence, parser refactors, formatting, or recovery rework beyond this error arm.
- Changing the Rust parser's recovery strategy unless Step 2 proves it unavoidable.
