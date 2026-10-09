# Plan: #604 — stop `{` after an if/while/match head from being read as a struct/enum body

## Goal
Make `if c {}`, `while c {}`, `match x {}` (and `for _ in v {}`) parse identically in the Rust
(`vow-syntax`) and self-hosted (`compiler/parser.vow`) parsers for **every** head expression,
including heads ending in an UPPER_CASE const or an enum path, and pin it with tests in both.

## Assumptions
- **Premise partly stale.** The lowercase-ident bug in the issue is already fixed on this branch:
  `vow-syntax/src/parser/expr.rs:233-235` has the `starts_with(is_ascii_uppercase)` guard (added by
  #1488, 1a3a8ed3), a unit test at `expr.rs:1612`, and `tests/run/empty_block_after_lowercase_ident.vow`
  covers `while c {}` / `if c {}` / `for ... {}`. Not covered anywhere: `match x {}`, `if x {} else {}` unit
  test in Rust, and a self-hosted parser test. (decision: best guess)
- **Residual bug of the same class remains in BOTH parsers**, which the PascalCase guard cannot fix:
  - `const DEBUG: bool = true; if DEBUG {}` / `while FLAG {}` — UPPER_CASE consts (grammar.md:44-48 defines
    them) pass the PascalCase test, and `looks_like_struct_literal` returns true for `{ }`.
  - `if c == Color::Red {}` — `parse_enum_construct` (Rust `expr.rs:701`) / `parse_path_expr`
    (`compiler/parser.vow:1093`) treat `Path {` + `}` as an empty brace-form enum ctor, swallowing the body.
  The issue's "Additionally/alternatively … no-struct-literal context" covers exactly this. Plan implements it
  (Rust's approach) in both compilers. Keeps the PascalCase guard (still needed for non-head contexts such as
  `x {}` never being valid anyway). (decision: best guess; the alternative of only adding tests would leave
  the identical failure for consts/enum paths.)
- Language-semantics consequence: in a head expression, an unparenthesised struct literal/brace-form enum
  ctor (`if p == Point { x: 1 } { .. }`) is no longer parsed as a literal; wrap in parens. Same rule as Rust.
  Implementer must grep the corpus first (step 0) — if any in-repo program relies on the old form, fix it by
  parenthesising it in that same PR.

## Key Files
| File | Role | Lines |
|------|------|-------|
| `vow-syntax/src/parser/mod.rs` | `Parser` struct + `Parser::new` — add `no_struct: bool` | struct def, `new` |
| `vow-syntax/src/parser/expr.rs` | gate struct/enum-brace parsing; set flag in heads; clear in nested delimiters | Ident branch 227-240, `parse_paren_or_tuple` 343, postfix call/index 421-440, `parse_call_args` 478, `parse_if_expr` 492, `parse_while_expr` 526, `parse_for_expr` 547, `parse_match_expr` 592, array literal, block parse, `parse_enum_construct` 682-705, tests mod 1612 |
| `compiler/parser.vow` | same, mirrored | `Parser` 7-16 + constructor ~24, `parse_primary` 992-997, `parse_path_expr` 1082-1130, `parse_if_expr` 1135, `parse_while_expr` 1153, `parse_for_expr` 1163, `parse_match_expr` 1186, paren/call-args/index/array/block parsers |
| `compiler/tests/test_parser_expr_forms.vow` | add self-hosted check (new `check_head_exprs`, 7xx exit codes) | 182 `main` |
| `tests/run/empty_block_after_lowercase_ident.vow` | extend or sibling fixture | whole |
| `tests/run/empty_block_after_const_and_enum_path.vow` [new] | run fixture for const / enum-path heads | — |
| `docs/spec/grammar.md` | rule for heads; update "Struct Literals" 914-921 | 905-925 |
| `skills/vow/reference/grammar.md` | mirrored copy (#1488 touched it) — regenerate/sync | same section |

## Slicing (cut line)
- **Slice A (closes the issue as filed; tests only, ship alone if B grows):** Rust unit test for `x {}` heads
  (`if x {}`, `if x {} else {}`, `while x {}`, `match x {}`), self-hosted `check_head_exprs` (lowercase cases only)
  in `test_parser_expr_forms.vow`, extend `tests/run/empty_block_after_lowercase_ident.vow` if `match` is
  type-checkable. Confirm both compilers pass.
- **Slice B:** the `no_struct` change (steps 1-5, const/enum-path cases + printer). If printer/idempotency work
  outgrows one PR, land A alone and file B as a follow-up issue (`gh issue create`). Implement A first, commit it,
  then proceed to B. Do not close #604 from a PR that ships only A unless B is filed.

## Steps

### 0. Corpus check (read-only)
`rg -n '\b(if|while|match|for)\b[^{;]*\b[A-Z]\w*(::\w+)*\s*\{\s*(\w+\s*:|\})' tests examples compiler benchmarks vow-*/ docs`
List any head containing an unparenthesised struct/enum literal; each must be parenthesised in step 5.

### 1. Red: Rust unit tests (`vow-syntax/src/parser/expr.rs`, tests mod near 1612)
Add one test parsing, via `crate::parser::parse_module`, a module whose body has `match x { }`-shaped and
head-ending-in-const/enum-path code, asserting `diagnostics.is_empty()` AND the AST shape (use
`parse_expr_from_source`/`parse_no_errors` helpers at 733-749): `if x {}` → `If{condition: Ident("x")}`;
`while x {}`; `match x {}` → `Match{scrutinee: Ident, arms: []}`; `if DEBUG {}` → condition `Ident("DEBUG")`;
`if c == Color::Red {}` → condition `BinOp{rhs: EnumConstruct{fields: []}}` and then-block present;
`for _i in V {}`. Plus positive controls that must still parse as literals: `if (Point { x: 1 } == p) {}`,
`if f(Point { x: 1 }) {}`, `if v[Point { x: 1 }.x] {}`, `let p: Point = Point { x: 1 };`,
`match x { _ => Point { x: 1 } }` (arm body, flag cleared by then), `if c { Point {x:1}; }`.
Lowercase cases pass today (pinned as regressions); only the const/enum-path cases are red.

### 2. Green (Rust): `no_struct` context
- `mod.rs`: field `no_struct: bool` (init false).
- `expr.rs`: private helper `parse_head_expr(&mut self) -> Expr` = save `no_struct`, set true,
  `parse_expr_inner(0)`, restore. Use it for the condition in `parse_if_expr`, `parse_while_expr`, the
  iterable in `parse_for_expr`, and the scrutinee in `parse_match_expr`.
- Gate: Ident branch (`:233`) add `&& !self.no_struct`; `parse_enum_construct` brace branch (`:701`)
  add `&& !self.no_struct`. Keep `looks_like_struct_literal` unchanged.
- Re-enable inside nesting: helper `with_struct_literals<T>(&mut self, f)` (save, set false, run, restore)
  used in `parse_paren_or_tuple`, `parse_call_args`, index `[...]` postfix, array/vec literal, struct-literal
  field values and enum brace-args, and the block parser (so `if { Point {x:1}; true } {}` still works).
  Prefer a single `parse_delimited`-style seam if one exists; do not scatter more than these entry points.
- Match arms: `parse_match_expr` (592) parses arm bodies directly, not via the block parser. Clear the flag
  across the whole `{ arms }` region (after the scrutinee, before `expect(LBrace)`, restore after `RBrace`) —
  otherwise `if match y { A => Point { x: 1 }, _ => q } == r {}` leaves the flag true (inherited from the outer
  head) inside the arms. Same for `parse_vow_block` if it can occur inside a nested head.
- **Printer (required, not conditional).** `parse_paren_or_tuple` returns `first` bare (357-358), so the AST
  drops parens: `if (Point { x: 1 } == p) {}` parses to a BinOp with a StructLiteral, and the printer would emit
  `if Point { x: 1 } == p {}`, which no longer reparses. In `vow-syntax/src/printer.rs`, parenthesise any
  StructLiteral, or brace-form EnumConstruct with fields, that sits in an if/while/for head or match scrutinee
  and is not already inside a delimiter (call args, index, tuple, array). Reuse the #1488 paren-emission seam
  for block-like postfix operands. Check whether `compiler/contract_text.vow` can render an `if` inside a
  contract predicate; if so give it the same rule to stay text-identical with the Rust printer. Add a roundtrip
  test per step-1 positive control; check whether `proptest_arb.rs` can generate struct literals in if
  conditions (extend only if trivial).
- Run `cargo test -p vow-syntax` (incl. proptest roundtrip).

### 3. Red/Green (self-hosted): mirror
- `compiler/parser.vow`: add `no_struct: i64` to `Parser` (+ every constructor site — grep `Parser {`), a
  `parse_head_expr(p)` (save/set 1/`parse_expr`/restore) used in the four head parsers, gate
  `parse_primary:997` with `&& p.no_struct == 0` and `parse_path_expr:1093/1108` with `p.no_struct == 0`,
  and save/clear/restore in the same nesting points as Rust (paren, call args, index, array, block, struct
  field values).
- Test first in `compiler/tests/test_parser_expr_forms.vow`: new `check_head_exprs() -> i64` reusing
  `parse_errors(src)` (:105) to assert zero diagnostics for the Step-1 inputs and exact-diag-free positive
  controls; wire into `main` (:182) with 7xx failure codes; update header comment. Also assert print
  output via the existing `check_pair` (:52) where an expression-level canonical form exists.
- Run `build/vowc test compiler/tests/test_parser_expr_forms.vow` (needs `scripts/bootstrap.sh --skip-cargo`
  first to get `build/vowc` from Rust stage 0; run in foreground with explicit timeout / poll).

### 4. Run fixture
`tests/run/empty_block_after_const_and_enum_path.vow` [new] (`// TEST: stdout "..."`): `const FLAG: bool =
false; enum Color {Red, Green}`; `if FLAG {}`, `while FLAG {}`, `match c {` with arms…`}`, `if c == Color::Red {}`,
parenthesised literal control, prints `done\n`. Add `match`-empty case to the existing lowercase fixture only
if the type checker accepts an arm-less `match` (otherwise keep `match` coverage at parser-test level and
note it). Both compilers run it via `scripts/full_test.sh` Section 4 (no extra wiring).

### 5. Fix corpus fallout from step 0 (only if any), docs
- `docs/spec/grammar.md` "Struct Literals" (914-921): add "In the head of `if`, `while`, `for … in`, and
  `match`, a struct literal or brace-form enum constructor must be parenthesised; an unparenthesised
  `Name {` after the head always opens the body." Keep the PascalCase sentence. Mirror to
  `skills/vow/reference/grammar.md`. Then `uv run python scripts/generate_help.py` and check
  `scripts/check_help_coverage.py` is clean (only if the embedded skill text changed).
- `docs/equivalence/ledger.json`: check whether #1488 added a parser row that should gain this case.

## Testing / verification commands (run as separate commands)
`cargo fmt --all --check`; `cargo clippy --all --all-targets -- -D warnings`; `cargo test -p vow-syntax`;
`cargo test --all`; `scripts/bootstrap.sh --skip-cargo --no-cache` (record head SHA in PR checklist per
CLAUDE.md); `build/vowc test compiler/`; `scripts/full_test.sh` (~40 min, background + poll within turn
limits; at minimum Sections 0b/4/5 for the new fixtures). Fixed-point: compiler source now contains the new
`parse_head_expr`; bootstrap triple must still be byte-identical.

## Verification surface (ESBMC)
No contracts change; new parser functions in `compiler/parser.vow` carry no `vow` blocks (match neighbours).
If `vowc verify`/bootstrap Stage-1 verification trips on the new `Parser` field or helper, fix the code, never
the contract. No `c_emitter` change → verifier C parity untouched (still run Section 2c via full_test).

## Risks
- Fixed point/bootstrap: self-hosted parser edits are compiled by itself; a bug in the flag save/restore
  can break parsing of `compiler/*.vow` (grep corpus step 0 includes `compiler/`). Bootstrap triple is the gate.
- Missed restore on early return (Rust: wrap with save/restore not `?`-style returns; Vow: no early exits
  between set and restore) leaves flag stuck true → later struct literals mis-parse (diagnostics flood).
  Positive-control tests (step 1) guard it, including after a parse error.
- Flag left stuck by an early return during error recovery (Rust `?`/early `return` between set and restore;
  Vow `return` in helpers) turns later literals into errors. Run `check_recovery_terminates` (and a Rust
  equivalent) with heads containing parse errors, e.g. `if (Point { x: } {}`.
- `parse → print → parse` idempotency: printer must parenthesise a struct literal inside a head (check
  `vow-syntax/src/printer.rs` and `compiler/contract_text.vow` — the latter prints contract text, usually
  not heads) — proptest `proptest_roundtrip.rs` generates heads (`proptest_arb.rs:87` note).
- Dual-compiler rule: both halves must land in the same PR; clippy `-D warnings` incl. tests.
- Unparenthesised-literal-in-head programs break (step 0 enumerates; spec documents it).

## Out of scope
Refactoring the Pratt loop; generalising to a context struct; changing `looks_like_struct_literal`;
unit-struct/`Name {}` literal semantics; `loop`; type-checker handling of empty `match`; formatter changes;
closing audit-sibling issues; verifier/ESBMC changes.
