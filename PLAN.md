# Plan: #641 self-hosted parser accepts string-literal match patterns

## Goal
Make `compiler/parser.vow::parse_pattern` parse `"hello"` as a `PAT_LIT` carrying `EXPR_LIT_STR`, matching the Rust parser (`vow-syntax/src/parser/types.rs:198-203`). `match s { "hello" => 1, _ => 0 }` then reaches the checker and fails with `UnsupportedPattern` in both compilers instead of mis-parsing in the self-hosted one.

## Scope decision (assumptions)
- **View 1 (string literal pattern) is the whole PR.** It is a pure self-hosted parity bug. The Rust parser is already correct, so no Rust production change is needed. This does not break the dual-compiler rule: the Rust side already behaves correctly and gets only a regression test.
- **View 2 (or-patterns) is split into a follow-up issue, not bundled** (best guess; surgical-changes rule). Reasons:
  - The `||` vs `|` choice is a language-syntax decision. Rust `parse_pat_inner` (types.rs:129-141) keys off `PipePipe`, but `print_pat` (printer.rs:819-822) prints ` | `, so `parse -> print -> parse` is already non-idempotent for or-patterns.
  - The checker rejects `PAT_OR` / `PatKind::Or` with `UnsupportedPattern` in both compilers (checker.vow:2299, check.rs:3504), so no program can use it today. Fixing the token buys only a better diagnostic.
  - The issue cites grammar.md line 513 as documenting `0 | 1 | 2`. That is stale: grammar.md "Pattern Matching" (~l.1019-1043) lists or-patterns as *not implemented*, and the match section documents no `|` token.
  - Recommended follow-up, recorded in a `gh issue comment` on #641 at PR time: canonical token is single `|`, because the printer already emits it and it matches the issue's proposal. That change needs a Rust parser change (`PipePipe` -> `Pipe`), a self-hosted `PAT_OR` list loop in `parse_pattern`, the `parse_pat_inner` test update (types.rs:623), a grammar.md update, and a check that `|` inside `parse_pattern` cannot conflict with closure or refinement `||` (types.rs:114 uses `PipePipe` for refinement types; leave that alone).
- The self-hosted checker already emits the correct `UnsupportedPattern` for `PAT_LIT` (checker.vow:2293-2294) and ignores it in binding (checker.vow:2664), so no checker change is needed.

## Key Files
| File | Role | Lines |
|------|------|-------|
| `compiler/parser.vow` | add `tok_lit_string()` arm to `parse_pattern` | 1222-1301 (new arm after bool arm at 1232-1235) |
| `compiler/parser.vow` | reference: expression string literal does `arena_intern_str(p.arena, t.str_val)` + `EXPR_LIT_STR()` | 991-994 |
| `compiler/ast.vow` | `PAT_LIT`, `EXPR_LIT_STR`, `arena_add_pat(a, tag, av, bv, span)` | 76, 458 |
| `compiler/checker.vow` | existing `PAT_LIT` rejection, unchanged | 2293, 2664 |
| `vow-syntax/src/parser/types.rs` | Rust parser, already correct; `pat_lit_string` test exists | 198-203, 633 |
| `tests/error/` | new fixture | [new] `match_string_literal_pattern.vow` |
| `compiler/tests/` | new self-hosted parser unit test | [new] `test_parser_patterns.vow` |
| `scripts/parity.py` | span-parity fixture list (line ~59, next to `match_bool_literal_pattern.vow`) | edit if the list is the set of error fixtures compared for identical spans |

## Steps (TDD slices)

### Slice 1 (red): error fixture, both compilers
- Add `tests/error/match_string_literal_pattern.vow`, modelled on `tests/error/match_bool_literal_pattern.vow`:
  ```vow
  // TEST: error-code UnsupportedPattern
  module MatchStringLiteralPattern

  fn f(s: String) -> i64 {
      match s {
          "hello" => 1,
          _ => 0,
      }
  }

  fn main() -> i32 {
      0
  }
  ```
  Optionally add `// TEST: build-json x['span']['offset'] > 0 and x['span']['length'] > 0`, as `match_integer_literal_pattern.vow` does.
- Confirm red on self-hosted: `build/vowc build --no-verify tests/error/match_string_literal_pattern.vow` currently yields an `unexpected token in pattern` parse error and no `UnsupportedPattern`. Confirm the Rust compiler (`target/release/vow`) already emits `UnsupportedPattern`.
- Hazard: if the Rust checker emits a scrutinee-type error before pattern rejection for a `String` scrutinee (the grammar says the scrutinee must be an enum), pick whatever scrutinee type makes both compilers emit `UnsupportedPattern`. Compare outputs first; the fixture must produce the same code and span in both.

### Slice 2 (red): self-hosted parser unit test
- Add `compiler/tests/test_parser_patterns.vow`, using the `parse_errors` pattern from `compiler/tests/test_parser_expr_forms.vow:105-110`.
- Parse a module containing `match s { "hello" => 1, _ => 0 }`.
- Assert zero parse diagnostics, and that the first arm pattern has `pat_tag == PAT_LIT()`, `pat_a == EXPR_LIT_STR()`, and `arena_str(a, pat_b) == "hello"`.
- Add a second case with an empty string `""` and one with an escape, `"a\"b"`, comparing against the same lexer string value that the expression path produces.
- Distinct exit codes per failure, as in the neighbouring tests. Run it with `build/vowc test compiler/tests/test_parser_patterns.vow`.
- Walking arms needs the match-expression accessors (`EXPR_MATCH` a=scrutinee, b=arms list; arm layout from `parse_match`, parser.vow ~1195-1220). Read that layout before writing the assertions.

### Slice 3 (green): the parser arm
- In `parse_pattern` (`compiler/parser.vow`), insert after the bool arm (line 1235):
  ```vow
  } else if tag == tok_lit_string() {
      let _adv: Token = advance(p);
      let sid: i64 = arena_intern_str(p.arena, t.str_val);
      arena_add_pat(p.arena, PAT_LIT(), EXPR_LIT_STR(), sid, span_pack_to_here(p, span_start))
  }
  ```
- Reuses the interning idiom at parser.vow:991-994. No comment needed.
- No change to `ast.vow`, the checker, lowering, or `contract_text.vow` (PAT_LIT is rejected before lowering).

### Slice 4: Rust-side regression and parity registration
- Rust parser test `pat_lit_string` (types.rs:633) already covers parsing; no Rust production change.
- If `scripts/parity.py:~59` lists error fixtures compared for span parity, add `match_string_literal_pattern.vow` there. If it only holds span-exception cases, skip it. Read the surrounding set definition first.

## Testing / verification commands
- `build/vowc test compiler/tests/test_parser_patterns.vow` (new) and `build/vowc test compiler/tests/test_parser_expr_forms.vow` (regression).
- Fixture under both compilers via the `tests/error` section of `scripts/full_test.sh`; run `python3 scripts/parity.py` for the error-fixture span comparison if the list is touched.
- After the `compiler/parser.vow` edit: `scripts/bootstrap.sh --skip-cargo --no-cache`. The binary fixed point must hold; record the PR head SHA in the checklist, per CLAUDE.md. Budget about 5 min for bootstrap, so run it in the foreground with an explicit timeout.
- `cargo test -p vow-syntax` and `cargo clippy --all --all-targets -- -D warnings` as a sanity run (no Rust production edits expected).
- Run in the background and poll: `scripts/full_test.sh` takes about 40 min. The known pre-existing failures in memory (`concrete-block-region-parity`, `u64_marker_propagation`, `contracts_tmp_cleanup`, Rust e2e SKIP-panics) are not caused by this change; verify against clean main before blaming it.

## Verification surface
- No contracts, codegen, or C-model change. The verifier C parity rule (`c_emitter.rs` vs `c_emitter.vow`) is unaffected, because a program with a string pattern never reaches lowering.
- ESBMC: `parse_pattern` is an existing function; check that `build/vowc build` of the compiler still verifies or skips it as it does today. The new arm adds no loops, arithmetic, or indexing.
- No `examples/` or `tests/run/` growth; this is an error-path change.

## Risks
- Binary fixed point: the parser change alters the self-hosted compiler's own code. The compiler sources contain no string-literal patterns, so Stage 1 and Stage 2 outputs must stay byte-identical. Confirm via bootstrap (SHA of `compiler_b` vs `compiler_c`).
- Diagnostic parity: the self-hosted result for the fixture must match Rust's code and span (`span_pack_to_here` covers just the literal, like the Rust `start` span). Compare JSON from both compilers before finalising the fixture.
- Interning order: `arena_intern_str` runs at pattern-parse time. Only the interned id table order is affected, and only for programs that already fail, so there is no fixed-point impact.
- `parse -> print -> parse` idempotency is unaffected (Rust printer's `print_lit` already handles strings, printer.rs:759).
- Docs: no `docs/spec` change. grammar.md already states that string literal patterns are unsupported and yield `UnsupportedPattern`, and this change makes the self-hosted compiler conform. Do not run `generate_help.py`.

## Out of scope
- Or-pattern parsing and the `||` -> `|` Rust change (View 2): separate follow-up issue, see Scope decision.
- Float literal patterns in the self-hosted parser (the Rust parser has `LitFloat`; a similar parity gap, which the implementer may mention in the issue comment but must not fix here).
- Making string, int, or bool literal patterns *supported* by the checker or lowering.
- `contract_text.vow` stale comment ("the parser has no or-pattern") and any refactors or formatting.
