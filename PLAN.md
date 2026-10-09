# Plan: unary `-`/`!` bind tighter than `as` (issue #674)

## Goal
Make `-x as u64` parse as `(-x) as u64` and `!b as i64` as `(!b) as i64` in BOTH compilers,
matching the "usual C/Rust precedence" the grammar claims. Today the prefix operand absorbs a
trailing `as`, so `-x as u64` is `-(x as u64)`, which for a signed `x` is a `TypeMismatch`
(negation of an unsigned type) and forces agents to write `(-x) as u64`.

## Assumptions
- Direction: match Rust (not "document the current ordering"). The current ordering was recorded
  deliberately in #1488 (grammar.md:508-511, `compiler/main.vow`, `vow/src/skill.rs`), so the
  spec sentence is rewritten, not just a test added. Reason: the corpus already works around the
  old grouping with `(-x) as u64` (`examples/sat/types.vow:106`, `tests/fixtures/contracts/
  contract_text_atoms.vow:64`), and an agent that writes the Rust spelling gets a confusing
  "negation not allowed on unsigned" error. (best guess)
- Blast radius is near zero: a corpus-wide grep (`*.vow`, `*.rs`, `*.md`, `*.py`) for
  `-ident as T`, `!ident as T` and `-<lit> as T` finds only
  `tests/fixtures/contracts/contract_text_postfix{,_canonical}.vow:80/172` (`-x as i64 == 1`)
  and `tests/run/shift_count_i8_shr_trap.vow:14` (`-1 as i8`). Neither `compiler/*.vow` nor
  `examples/`/`benchmarks/`/`stdlib` contain an affected spelling, so the bootstrap fixed point
  does not shift on source. (verified by grep)
- Meaning changes silently only at the MIN edge (`-x as i64` for narrower signed `x` at
  `x == MIN`); every other old use becomes a loud type error (`NegOnUnsigned`-style
  `TypeMismatch`). Accepted; recorded in the PR body, no `!`/BREAKING footer in the commit
  (would trigger a semantic-release major bump).
- Canonical printing keeps parentheses for the cast of a unary: `Cast{Neg x}` prints
  `(-x) as u64` (already the printer's behaviour, pinned at `vow-syntax/src/parser/expr.rs:1480`),
  and `Neg{Cast x}` now prints `-(x as u64)`. No cast-without-parens form is introduced.
  (best guess: smallest printer change, no new canonical spelling)

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `vow-syntax/src/parser/expr.rs` | Rust parser: Pratt loop, prefix arms, postfix loop; unit tests | 7 (`PREFIX_BINDING_POWER = 19`), 87-165 (`parse_expr_inner`), 167-266 (`parse_prefix`; unary arms 230-266), 431 (`KwAs` postfix), 1479-1480 (accepted-forms table) |
| `vow-syntax/src/printer.rs` | Rust canonical printer | 546-577 (`UnaryOp` arm: parenthesise `Cast` operand), 700-711 (`Cast` arm, unchanged), 1294-1303 (`test_neg_of_cast_needs_no_parens`, must be inverted) |
| `vow-syntax/tests/proptest_arb.rs`, `proptest_roundtrip.rs` | parse -> print -> parse idempotency generators | confirm `Unary{Cast}` and `Cast{Unary}` are generated |
| `compiler/parser.vow` | self-hosted parser | 871-897 (`parse_unary`), 899-945 (`parse_postfix`, `as` at ~931) |
| `compiler/contract_text.vow` | self-hosted canonical/contract text printer | 242-259 (`EXPR_UNOP` arm: parenthesise `EXPR_CAST` operand), 262-285 (`EXPR_CAST`, unchanged) |
| `compiler/tests/test_parser_expr_forms.vow` | self-hosted parse->print parity tests (`check_pair`) | 80-100 |
| `tests/fixtures/contracts/contract_text_postfix.vow` / `_canonical.vow` / `.expected` | cross-compiler contract-text fixture shared by `vow-ir/tests/contract_text_forms.rs` and `compiler/tests/test_lower_contract_text.vow` | `.vow:80`, `_canonical.vow:172`, `.expected:48` |
| `vow-types/src/check.rs` | checker special case `Neg(Cast(Lit))` | 2375-2395 (keep; still reachable via `-(1 as i8)`), 3238-3290 (Cast arm), 1833-1870 (`check_integer_literal_range` handles `Neg(Lit)`) |
| `compiler/checker.vow` | self-hosted mirror of the above | 3090 (`UNOP_NEG` over `EXPR_CAST`), 1617/1645/1772 |
| `docs/spec/grammar.md` | spec | 502-520 (Operator Precedence paragraph, sentence at 508-511) |
| `skills/vow/reference/grammar.md`, `vow/src/skill.rs`, `compiler/main.vow` | GENERATED from grammar.md | skill.rs:1907/7930, main.vow:4350/10395, grammar.md:510 |

## Steps (ordered TDD slices)

Ordering rule: the shared contract-text fixtures (`.vow`/`_canonical.vow`/`.expected`) are read by
BOTH compilers' tests, so they change last, once both compilers agree. Expected transient reds
(PR is squash-merged, so intermediate commits need not be green on the shared suites, but run them
only after Slice 3): `cargo test -p vow-ir --test contract_text_forms` is red after Slice 1 (Rust now
prints `(-x) as i64` where `.expected:48` still says `-x as i64`), and
`build/vowc test compiler/tests/test_lower_contract_text.vow` is red after Slice 2 for the same
reason; both flip green at Slice 3. Each slice's own crate/suite (`vow-syntax`;
`test_parser_expr_forms`) is green at the end of that slice.

### Slice 0. Recon + characterisation (green before and after)
- **Shape-matcher audit** (grep before touching code; record result in the PR body). Code that
  pattern-matches `UnaryOp{Neg, Cast{..}}` / `EXPR_UNOP` over `EXPR_CAST` must be reviewed because
  the AST shape of `-x as T` changes: `vow-types/src/check.rs:2375` and `compiler/checker.vow:3090`
  (special case, keep for `-(1 as i8)`), `compiler/checker.vow:1617/1645/1727/1772`,
  `compiler/lower.vow:2252/2287/2307/2625` and `vow-ir/src/lower/mod.rs:1941` (generic `Neg`
  lowering via `lower_integer_marker_as`; confirm `Cast(Neg(Lit))` lowers a negated marker to a
  narrower target), `vow-verify/src/const_fold.rs`, `compiler/const_fold.vow`,
  `vow/src/contract_quality.rs`. `vow-verify/src/c_emitter.rs:9819` ("i32 -1 as u64 sign-extends")
  builds IR directly (`test_inst`), not Vow source: unaffected.
- **Entry points**: `parse_unary` is called only from `parse_expr_prec` (parser.vow:858) and its own
  recursion; `parse_postfix` only from `parse_unary` (895). "Outermost unary" is therefore exactly
  the `parse_expr_prec` call; no pattern/predicate entry point needs separate handling.
- **Characterisation fixture** `tests/run/unary_cast_precedence.vow` [new], committed first with
  only forms valid under both groupings, stdout pinned with `// TEST: stdout`: `(-x) as u64`,
  `-(x as i64)` (signed target), `(-x) as i128`, `-1 as i8`, `-128 as i8`, and a runtime
  `i8` shift trap is already pinned by `tests/run/shift_count_i8_shr_trap.vow:14` (`-1 as i8`,
  `TEST: exit 134`). Slice 4 appends the new-grouping rows to this same file.
- **Diagnostic characterisation** (run on current build, record exact codes/spans):
  `-1 as u64`, `-129 as i8`, `-x as u64` (x: i64), `-(x as u64)`.
  Today `-1 as u64` is `-(1 as u64)` -> negation-of-unsigned `TypeMismatch`.

### Slice 1. Rust parser + printer together (red -> green, vow-syntax green at the end)
- **Test (red)** in `vow-syntax/src/parser/expr.rs` tests (accepted-forms table ~1479) and
  `vow-syntax/src/printer.rs` tests (1294): AST-shape tests via `parse_no_errors`:
  - `-a as u64` -> `Cast{ UnaryOp{Neg, a}, u64 }`; `!x as i64` -> `Cast{ UnaryOp{Not, x}, i64 }`
    (shape/print only; `bool as i64` is a type error and is NOT used in any run/contract fixture)
  - `--x as u64` -> `Cast{ Neg{Neg x} }` (NOT `Neg{Cast{Neg x}}`)
  - `-x.f as u64`, `-v[0] as i64`, `-f(a) as i64` -> `Cast{ Neg{postfix chain} }`
  - `-x as u64 + 1` -> `Binary{Add, Cast{Neg x}, 1}`; `a - b as u64` -> `Binary{Sub, a, Cast{b}}`
  - `-x as u64 as i64`, `-x as u64?`, `-x as u64 < y`, `-x as u64 << 1`
  - `-(x as u64)` -> `Neg{Cast}`; `-(if c { 1 } else { 2 }) as u64` accepted
  - rejected with the same `UnexpectedToken`: `-if c { 1 } else { 2 } as u64`,
    `3 * if c { 1 } else { 2 } as u64`
  - `&x as u64` still yields exactly one `UnsupportedFeature`.
  - Printer: invert `test_neg_of_cast_needs_no_parens` into `neg_of_cast_is_parenthesised`
    (`Neg{Cast a u64}` -> `-(a as u64)`), `Not{Cast}` -> `!(a as bool)`, `Cast{Neg a}` -> `(-a) as u64`.
  - Accepted-forms table: `("-x as u64", "(-x) as u64")`, add `("-(x as u64)", "-(x as u64)")`,
    `("!x as i64", "(!x) as i64")`. Check `proptest_arb.rs` generates both nestings.
- **Change (green)**:
  1. `parse_expr_inner`: in the postfix-operator branch, `TokenKind::KwAs` with
     `min_bp >= PREFIX_BINDING_POWER` breaks (only the three prefix arms at expr.rs:232/244/265 pass
     `PREFIX_BINDING_POWER`; every binary `rbp` is <= 18). `.`, `[`, `(`, `?` stay absorbed.
  2. Replace `postfix_ok = parenthesised || !matches!(lhs.kind, UnaryOp)` with "the unary chain did
     not end in an unparenthesised block-like operand": `parenthesised` becomes "the first token after
     the run of prefix tokens (`-`, `!`, `&`, `&&`, `mut`, via `peek_n_kind`, mod.rs:84) is `(`", and
     `postfix_ok = parenthesised || !innermost_operand(lhs).is_block_like()` where `innermost_operand`
     walks `UnaryOp` operands to the leaf.
  3. After `as` is applied to the `UnaryOp`, `postfix_ok` stays true so `-x as u64?` and
     `-x as u64 as i64` keep chaining.
  4. `printer.rs` `UnaryOp` arm (~563): add `ExprKind::Cast { .. }` to `needs_parens` next to
     `BinaryOp`. The `Cast` arm already parenthesises a `UnaryOp` operand through its `_ =>` branch.
- **Reuses**: `parse_postfix` `KwAs` arm (expr.rs:431), `parse_cast_target`; no new AST node.
- Round-trip: `Neg{Cast}` printing without parens would re-parse as `Cast{Neg}`; parser and printer
  therefore land in one commit, so the proptest never sees a half state.

### Slice 2. Self-hosted parser + printer (red -> green), same session as Slice 1
- **Test (red)**: `compiler/tests/test_parser_expr_forms.vow` `check_pair` rows mirroring the Slice 1
  table (`-x as u64` -> `(-x) as u64`, `-(x as u64)` -> `-(x as u64)`, `!x as i64` ->
  `(!x) as i64`, `--x as u64`, `-x as u64 + 1`) and a `parse_errors` row for
  `-if c { 1 } else { 2 } as u64`. Run `build/vowc test compiler/tests/test_parser_expr_forms.vow`.
- **Change (green)** `compiler/parser.vow`: move the existing `parse_postfix` loop into
  `parse_postfix_chain(p, e, span_start, allow_cast)`; the no-cast variant is used for a prefix operand.
  `parse_unary` records `span_start` before the leading prefix token, builds the unary nodes as now
  via a no-cast operand path, and at the outermost level (called from `parse_expr_prec`, the only
  entry) resumes the chain with `allow_cast = true` unless the leaf operand is an unparenthesised
  block-like (`is_block_expr`, ast.vow:184, plus a prefix-run paren lookahead with `peek_offset`,
  parser.vow:44). The resumed `Cast` span is `span_pack_to_here(p, span_start)` from the leading prefix
  token. `compiler/contract_text.vow` `EXPR_UNOP` arm (242-259): parenthesise the operand when its tag is
  `EXPR_CAST` (mirror of the Rust `needs_parens` change).

### Slice 3. Shared contract-text fixtures (atomic; both compilers already agree)
- `tests/fixtures/contracts/contract_text_postfix.vow:80`: keep `requires: -x as i64 == 1`
  (now `Cast{Neg}`), add `requires: -(x as i64) == 1` (both `x: i64`; no bool casts).
  `contract_text_postfix_canonical.vow:172`: `(-x) as i64 == 1`, add `-(x as i64) == 1`.
  `contract_text_postfix.expected:48`: `requires (-x) as i64 == 1`, add the new line in the same order.
- Consumers: `cargo test -p vow-ir --test contract_text_forms`,
  `build/vowc test compiler/tests/test_lower_contract_text.vow`; `full_test.sh` ~line 2117 runs both.

### Slice 4. End-to-end + error fixtures
- Append to `tests/run/unary_cast_precedence.vow`: `let x: i64 = 5; -x as u64` prints
  `18446744073709551611`; `-x as i64 + 1`; narrower-signed (`i32`) negate-then-widen; `-x as u64 as i64`.
- `tests/error/neg_of_unsigned_cast.vow` [new]: `-(x as u64)` ->
  `// TEST: error-code TypeMismatch` (explicit other grouping is a loud error).
- `tests/error/neg_literal_cast_unsigned.vow` [new]: `-1 as u64` -> the code BOTH compilers emit after
  the change (expected: `LiteralOutOfRange` via `CastVerdict::LiteralRange` ->
  `check_integer_value_range`, was `TypeMismatch`); record the change in the PR body.
  Confirm `-129 as i8` keeps `LiteralOutOfRange` and note any span-anchor move.
- Existing `tests/error/cast_after_*` fixtures unchanged (regression guard).

### Slice 5. Spec + generated artefacts
- `docs/spec/grammar.md` 507-511: replace with: "Unary `-` and `!` bind tighter than every
  binary operator **and tighter than `as`**, matching Rust: `-x as u64` is `(-x) as u64` and
  `!b as i64` is `(!b) as i64`. The other postfix forms (`.field`, `.method()`, `[index]`,
  `(args)`, `?`) bind tighter than unary, so `-v.len()` is `-(v.len())`, and `as Type` binds
  tighter than every binary operator: `a.len() as i64 + 1` is `(a.len() as i64) + 1`. To negate
  a cast result write `-(x as i64)`; the canonical printer prints exactly that, and prints the
  cast of a negation as `(-x) as u64`." Reword the "Operator Precedence" lead sentence if it
  implies the old behaviour.
- Regenerate: `uv run python scripts/generate_help.py` (updates `skills/vow/reference/grammar.md`,
  `vow/src/skill.rs`, `compiler/main.vow`), then `python3 scripts/check_help_coverage.py`.

## Verification surface
- Contracts / C model: no contract, codegen, or emitter change. The only verifier-visible effect is the
  *text* of contract descriptions (`vow-verify` counterexample JSON embeds contract text), which comes
  from `printer.rs` / `contract_text.vow` and is pinned by Slice 3. No ESBMC property changes; no
  contract needs weakening.
- C parity (`vow-verify/src/c_emitter.rs` vs `compiler/c_emitter.vow`): the emitters consume AST/IR, not
  tokens; `Cast(Neg x)` vs `Neg(Cast x)` are already both emitted for parenthesised source
  (`(-x) as u64`). Run `python3 scripts/parity.py c ...` via `full_test.sh` Section 2c over
  `tests/verify*/` to confirm; add no verify fixture unless a `tests/verify/` fixture
  contains an affected spelling (grep found none).
- Fixtures that must grow: Slices 0/3/4 only. `examples/` unchanged.

## Testing / gate commands (background + poll; see wall-clock memory)
- `cargo test -p vow-syntax` (parser, printer, proptests); `cargo test -p vow-ir --test contract_text_forms`
- `cargo clippy --all --all-targets -- -D warnings`; `cargo fmt --all`
- `scripts/bootstrap.sh --skip-cargo --no-cache` on the final head SHA (record SHA), then
  `build/vowc test compiler/` (or `--filter test_parser_expr_forms`) and `scripts/full_test.sh`.
- Use `VOW_CACHE_DIR=$(mktemp -d)` when validating codegen-visible fixtures.

## Risks
- **Literal negation cast path (highest)**: `-1 as i8` / `-128 as i8` move from the `Neg(Cast(Lit))`
  special case (check.rs:2375, checker.vow:3090) to `Cast(Neg(Lit))`. Slice 0 pins stdout/exit before the
  change and Slice 4 pins the new diagnostics; verify `-129 as i8` still errors with a stable code and that lowering
  (`vow-ir/src/lower`, `compiler/lower.vow`) types the negated literal. If the new path regresses,
  fix the checker in both compilers in the same slice; do not leave the special case
  half-applied. Keep the special case for `-(1 as i8)`.
- **Block-like ends the expression**: the prefix-run paren lookahead must be identical in both parsers or
  `-(if ..) as u64` / `-if .. as u64` will diverge. Slice 1 and 2 tables are the guard; the two existing
  `tests/error/cast_after_*` fixtures must keep `UnexpectedToken`.
- **`parse -> print -> parse` idempotency**: printing `Neg{Cast}` without parens would silently re-parse as
  `Cast{Neg}`; the printer change (Slice 1) and the contract_text.vow change (Slice 2) are required
  alongside their parsers, and `Cast{Neg}` must keep printing `(-x) as T`. Proptest covers it.
- **Binary fixed point**: no `compiler/*.vow` source contains an affected spelling, so stage 0 -> A -> B -> C
  stay byte-identical on source semantics. Parser code changes ARE compiler source: bootstrap must
  produce identical B and C (`scripts/bootstrap.sh`, record SHA). Avoid `HashMap` iteration in any new
  helper; none is expected.
- **Span of the resumed cast**: self-hosted `parse_unary` must stamp the `Cast` with the span from the leading
  prefix token; a wrong start offset breaks region/contract diagnostics anchors and
  `vow contracts` offsets (parity checked by `scripts/parity.py`).
- **Generated drift**: grammar.md edit without `generate_help.py` fails `check_help_coverage.py` and the
  `skills/vow/` mirror check in CI.
- **Pre-existing flakes** (see memory): e2e vow-crate run tests may SKIP-panic in the sandbox;
  `u64_marker_propagation`/`contracts_tmp_cleanup` and `concrete-block-region-parity` fail on clean main.
  Compare to clean `origin/main` before attributing.
- **Unsure about `!x as i64` typing**: with `x: bool` both groupings reject `bool as i64` (no integer width);
  tests for `!` assert AST/print shape only, not that the program compiles.

## Out of scope
- Cast of a unary printed *without* parentheses (`-x as u64` canonical form).
- Any change to `as` vs binary-operator precedence, cast target grammar, or narrowing rules.
- Removing/refactoring the `Neg(Cast(Lit))` checker special case or the `postfix_ok` machinery beyond what
  the slices need; no unrelated parser cleanup, formatting, or file splitting.
- `&` borrow-expression error recovery wording (#1488), generics in cast targets.
- Mutation-testing runs, benchmark runs, ADR (the decision is a spec correction, not a new architecture
  decision; ADR-2026-... not needed).

## Delivery
- Single PR, `fix(parser): bind unary minus and not tighter than as` (lower-case subject, < 92 chars),
  body `Closes #674`, notes the MIN-edge behaviour change and the corpus grep evidence.
  `git rm PLAN.md` before opening the PR. Both compilers land together (CLAUDE.md dual-compiler rule).
