# Implementation plan — issue #1088

**Self-hosted parser does not support `extern "C"` blocks (3× UnexpectedToken vs Rust's MissingContract)**

Branch: `sym/vow/1088-self-hosted-parser-does-not-support-extern-c-blocks-3x-unexpectedtoken-vs-rust-s-missingcontract`

---

## 1. Problem restated

`compiler/parser.vow`'s `parse_item` (line 229) has no `extern` branch, so any `extern "C" { ... }`
block falls into the `else` arm, which emits `UnexpectedToken` ("expected item (fn, struct, enum,
const, or type alias)") and calls `skip_unknown_item`. Because `skip_unknown_item` only consumes a
brace-balanced group when the *first* token it swallows is `{`, it eats `extern`, returns, then the
loop re-enters on `"C"` (second error), then on `{` (third error, this time consuming the block) —
producing exactly the 3× `UnexpectedToken` the issue reports, while the Rust compiler parses the
block in `vow-syntax/src/parser/items.rs:283` and reports the semantic `MissingContract` from
`vow-types/src/check.rs:1121`. The self-hosted lexer already tokenises `extern`
(`compiler/token.vow:36`, `compiler/lexer.vow:70`); only the parser, AST arena and checker are
missing. **Verified this session against a freshly built `target/release/vow`: this is an
accept/reject divergence, not merely a diagnostic one** — a *correctly contracted* extern block
compiles, links, runs (`exit 0`) and reports `Verified` under the Rust compiler, and is rejected by
`build/vowc` with three parse errors.

---

## 2. Target grammar — **mirror the Rust parser, not `docs/spec/grammar.md`**

This is the single most important instruction in this plan. `docs/spec/grammar.md:974` and
`docs/spec/contracts.md:408` document a form that **no compiler has ever accepted**:

```vow
extern "C" vow { requires: ... }
{ fn write(fd: i32, ptr: i64, len: i64) -> i64 [io] }     // ← NOT parseable
```

Empirically (this session, `target/release/vow` at 2ebcaec0): that form produces **6×
`UnexpectedToken`** from the *Rust* compiler. `docs/spec/errors.md:388` already documents the real
form. The implemented and canonically-printed grammar (`vow-syntax/src/printer.rs:370`) is:

```
extern "C" "{" [ vow "{" clause ("," | ";")* ... "}" ] ( "fn" IDENT "(" params ")" [ "->" type ] [ effects ] ";" )* "}"
```

```vow
extern "C" {
    vow {
        requires: true
    }
    fn write_thing(fd: i64, n: i64) -> i64 [io];
}
```

Points of exactness the implementer must match (`vow-syntax/src/parser/items.rs:283-340`):

- The `vow { ... }` block is **inside** the outer braces and **precedes** all `fn` declarations.
  It is parsed once, for the block; there is **no per-function `vow` block**. Do not call the
  self-hosted `parse_vow_block` after `parse_effects` in the extern-fn loop — Rust does not, and
  accepting it would create a fresh accept/reject divergence in the other direction.
- Each extern `fn` ends in `;`. A missing return type defaults to unit.
- ABI other than `"C"`: emit `UnexpectedToken` with `only extern "C" is supported, got "<abi>"`,
  spanned at the ABI literal, and **keep parsing** the block.
- A non-string-literal where the ABI belongs: `UnexpectedToken`, keep parsing
  (Rust's `expect_string_literal`, `vow-syntax/src/parser/mod.rs:660`).
- The body loop must carry a no-progress guard (Rust's `pre_iter = self.cursor` at `items.rs:305`,
  `if self.cursor > pre_iter { push } else if !at(RBrace) && !at_end { advance }`). Without it a
  malformed extern block is an infinite loop in the self-hosted parser. Non-negotiable.

Neither compiler lowers `Item::Extern` into IR (`grep 'Item::Extern'` hits only `ast.rs`,
`parser/items.rs`, `printer.rs`, `check.rs`), so the fix is frontend-only in both.

---

## 3. Files to touch

### Self-hosted (`compiler/`) — the actual fix

| File | Change |
|---|---|
| `compiler/ast.vow` | Add `ITEM_EXTERN() -> i64 { 5 }` (line ~86). Widen `item_pack`'s `requires: kind <= 4` → `kind <= 5` (line 481). Add `ext_data: Vec<i64>` to `AstArena` + its `arena_new` initialiser. Add `arena_add_ext(a, vow_lid, fns_lid, span) -> i64` (stride 3, mirroring `arena_add_alias`) and accessors `ext_vow_lid`, `ext_fns_lid`, `ext_span`. |
| `compiler/parser.vow` | Add `parse_extern(p) -> i64`. Add the `else if at(p, tok_kw_extern())` branch to `parse_item` (before the error arm, line 229-253). |
| `compiler/checker.vow` | Handle `ITEM_EXTERN` in the `register_fn` pass (line ~588): emit `EC_MISSING_CONTRACT()` when the block has no vow block and at least one fn, then `register_fn(e, m, fid)` for each extern fid. |

Not touched, deliberately: `compiler/lower.vow`, `compiler/complexity_main.vow`. Every item loop in
those files is guarded by `if ik == ITEM_FN()` / `== ITEM_CONST()` / `== ITEM_STRUCT()` etc. with no
`else` fallthrough, so `ITEM_EXTERN` is silently skipped — which is exactly what the Rust lowerer
does with `Item::Extern`. Confirmed by reading `lower.vow:5483`, `:5541`, `:5562`, `:5738`, `:5815`.

### Rust (`vow-syntax/`, `vow-types/`) — **no changes**

The Rust behaviour is already correct and is the reference. If a slice below wants a `.rs` edit,
that is a signal the slice has drifted out of scope.

### Test fixtures

| File | Purpose |
|---|---|
| `tests/error/extern_missing_contract.vow` (new) | Reject-parity: `// TEST: error-code MissingContract`. Picked up automatically by `scripts/full_test.sh:1176`. |
| `tests/run/extern_block_contracted.vow` (new) | Accept-parity: contracted, **uncalled** extern block + a `main` that prints. `// TEST: exit 0`. Picked up by Section 4 (`full_test.sh:561`). |
| `compiler/tests/test_checker.vow` (existing) | Self-hosted unit coverage for `parse_extern` + the `MissingContract` emission, if it fits the file's existing pattern; otherwise fold into the fixtures above. |

### Equivalence ledger — **required, or the nightly sweep fails *because* the fix worked**

`docs/equivalence/ledger.json:170` carries

```json
"tests/fixtures/mutants/sample_extern.vow": {
  "first_seen": "2026-08-25", "observable": "error_code", "status": "open",
  "note": "Self-hosted parser does not support `extern \"C\"` blocks: 3x UnexpectedToken vs Rust's MissingContract.",
  "issue": 1088,
  "rust_error_codes": ["MissingContract"],
  "self_hosted_error_codes": ["UnexpectedToken", "UnexpectedToken", "UnexpectedToken"]
}
```

`scripts/equivalence.py:1136` is `return 1 if (new or fixed) else 0` — an entry still marked `open`
whose divergence has stopped reproducing lands in `fixed` and **exits 1**. The Tier-2 nightly sweep
would go red on a green fix. Follow the precedent already in the file at
`ledger.json:163` (`tests/error/undefined_function.vow`): keep the entry, set
`"status": "fixed"`, drop `rust_error_codes` / `self_hosted_error_codes`, and rewrite `note` to say
what was fixed and that the entry is retained so a reappearance reads as a regression.

Leave alone: the two `tests/error/match_arm_missing_comma_{scalar,block}.vow` entries, also tagged
`"issue": 1088` but `"status": "expected"` — those are the cosmetic divergences this PR does not
touch (§7.3). `tests/error/undefined_function.vow` is already `"status": "fixed"`, so the first of
the issue's two "related, for the record" items is resolved and needs nothing.

No `// TEST: known-divergence` directive references this issue (the only two in the tree name
`#1087`), so there is no in-fixture directive to remove.

### Docs (`docs/spec/`) — correcting drift, last slice

| File | Change |
|---|---|
| `docs/spec/grammar.md` (§ Extern Blocks, ~line 974) | Replace the unparseable example with the implemented form. |
| `docs/spec/contracts.md` (§ Extern Block Contracts, ~line 408) | Same replacement. |
| `scripts/generate_help.py:641` | `"extern_blocks"` string literal also carries the wrong form; fix it. |
| Regenerated: `vow/src/skill.rs`, `compiler/main.vow`, `skills/vow/**` | Produced by `uv run python scripts/generate_help.py`. **Never hand-edit.** |

`docs/spec/errors.md` needs no change — its `MissingContract` example already uses the real form.

---

## 4. TDD slices

Each slice is independently green and independently revertible.

### Slice 1 — reject-parity fixture (red on self-hosted, green on Rust)
- **Test:** new `tests/error/extern_missing_contract.vow`, first line
  `// TEST: error-code MissingContract`, body = an uncontracted `extern "C" { fn f(x: i64) -> i64; }`
  plus a `main`. (Do not simply move `tests/fixtures/mutants/sample_extern.vow` — it is load-bearing
  for `tests/mutants/tests.sh`.)
- **Expected:** `full_test.sh` Section 7 fails with `error codes: ['MissingContract'] vs
  ['UnexpectedToken','UnexpectedToken','UnexpectedToken']`. That failure is the issue, reproduced in
  the suite.
- **Production code:** none.

### Slice 2 — AST arena support
- **Behaviour:** `ITEM_EXTERN` packs/unpacks; `arena_add_ext` round-trips `(vow_lid, fns_lid, span)`.
- **Production code:** `compiler/ast.vow` per §3. Widening `item_pack`'s `requires: kind <= 5` is a
  representation-invariant update, not a verifier accommodation — it states the true domain of the
  tag. Re-run `build/vowc verify compiler/ast.vow`-equivalent (via `scripts/bootstrap.sh`) and
  confirm `item_kind`'s `ensures: result == v / 4294967296` still discharges over the wider domain.
- **Test:** covered transitively by Slice 3; add a direct arena assertion in `compiler/tests/` only
  if the existing files already assert at that granularity.

### Slice 3 — `parse_extern`
- **Behaviour:** the grammar in §2 parses; each extern fn becomes an `arena_add_fn(...)` record with
  `is_declaration = 1`, `vow_lid = -1`, and a `-1` return tid when `->` is absent (matching
  `parse_fn_def`'s handling). The block's `vow_lid` (or `-1`) and the list of fids go into
  `arena_add_ext`. `parse_item` gains the `tok_kw_extern()` branch.
- **Production code:** `compiler/parser.vow`.
- **After this slice** the fixture from Slice 1 emits **zero** diagnostics from the self-hosted
  compiler (parse succeeds, checker not yet wired) — still red, but for a different reason. That is
  the expected intermediate state; do not "fix" it by emitting the diagnostic from the parser.

### Slice 4 — checker: `MissingContract` + signature registration
- **Behaviour:** for each `ITEM_EXTERN` item, if `ext_vow_lid == -1` and the fn list is non-empty,
  emit `EC_MISSING_CONTRACT()` with message `extern block requires a vow contract` spanned over the
  **whole block** (`ext_span`, matching Rust's `block.span`); then `register_fn(e, m, fid)` for every
  extern fid so callers type-check against the declared signatures. `register_fn`
  (`checker.vow:664`) reads only the fn arena record and never touches the body, so it is already
  correct for declaration-only fns — no change needed there.
- Registration must run in the **same pass** as ordinary `ITEM_FN` registration (`checker.vow:588`),
  so that a fn body appearing earlier in the file can still call an extern declared later.
- The `check_fn` pass (`checker.vow:598`) already filters `!fn_is_declaration`, so extern fns are
  correctly skipped there without modification.
- **Hints are out of scope for this slice.** Rust attaches one hint
  (`add a \`vow { ... }\` block ...`); `env_emit_error_code` (`env.vow:775`) has no hints parameter
  and **no self-hosted checker diagnostic currently carries a hint** (`diag_add_hint` appears only in
  `region.vow` and `main.vow`). `scripts/parity.py:_error_codes` compares error codes only, so hint
  parity is not required to close this issue. Adding a hints-carrying checker seam is a separate,
  cross-cutting change — record it in the issue comment as a follow-up.
- **Slice 1's fixture goes green here.**

### Slice 5 — accept-parity fixture
- **Test:** new `tests/run/extern_block_contracted.vow`:
  ```vow
  // TEST: exit 0
  module ExternBlockContracted

  extern "C" {
      vow {
          requires: true
      }
      fn write_thing(fd: i64, n: i64) -> i64 [io];
  }

  fn main() -> () [io] {
      print_i64(1);
  }
  ```
- **Why this is safe to link:** neither compiler lowers `Item::Extern`, so no `CallExtern` to
  `write_thing` is ever emitted and no undefined symbol reaches the linker. Verified this session:
  `vow build --no-verify` → runs, prints `1`, exits 0; `vow verify` → `Verified`.
- **Do not call the extern function from the fixture.** That would emit `CallExtern("write_thing")`
  against a symbol that does not exist, and the test would fail at link time in both compilers.
- **Production code:** none — this fixture is green the moment Slices 3+4 land, and it is the
  regression guard for the accept/reject half of the divergence.

### Slice 6 — spec correction, ledger, regeneration
- Flip the `tests/fixtures/mutants/sample_extern.vow` ledger entry to `"status": "fixed"` per §3.
  Confirm with `python3 scripts/equivalence.py --help` / a targeted sweep that the entry no longer
  lands in `no_longer_diverging`.
- Edit `docs/spec/grammar.md` and `docs/spec/contracts.md` to the §2 form; fix
  `scripts/generate_help.py:641`.
- Run, in order (never `&&`-chained):
  ```
  uv run python scripts/generate_help.py
  cargo build --release -p vow
  scripts/bootstrap.sh --skip-cargo
  ```
- `scripts/generate_help.py --check` reports **in sync** on the current tree (verified this
  session), so the regenerated diff will contain this change and nothing else. It will still be
  large: `vow/src/skill.rs`, `compiler/main.vow` and `skills/vow/reference/{grammar,contracts}.md`
  all embed the spec text verbatim. That churn is mechanical and expected.
- `scripts/check_help_coverage.py` requires the `extern_blocks` key in the `--help` language JSON;
  the key stays, only its value changes.

### Slice 7 — full gate
```
cargo build --release --all
cargo test --all
cargo clippy --all -- -D warnings
scripts/bootstrap.sh --skip-cargo
scripts/full_test.sh
```
Separate commands, never `&&`-chained. `scripts/full_test.sh` Section 9 runs the bootstrap triple
test; `sha256sum` of stages B and C must be identical.

**Build the runtime first.** `cargo build --release --all` (not just `-p vow`) is deliberate:
without `target/release/libvow_runtime.a` every link fails with
`error[LinkFailed]: could not find libvow_runtime.a`, and roughly eight `vow`-crate run tests report
FAILED. That is environmental, not a regression — this plan touches zero `.rs` files in slices 1-5,
so a run-test failure there is the missing runtime, not the fix. Confirm against a clean tree before
investigating.

**Use a scratch compile cache when hand-checking pre-existing fixtures.** Re-running
`tests/fixtures/mutants/sample_extern.vow` through a freshly rebuilt `build/vowc` at the same source
revision can be served stale objects from the compile cache, which would show the old three-error
output on a working fix. Run such checks as
`VOW_CACHE_DIR=$(mktemp -d) build/vowc build --no-verify tests/fixtures/mutants/sample_extern.vow -o /tmp/x`.
The two *new* fixtures have no cache entries and are unaffected.

---

## 5. Verification surface

No new ESBMC obligations for user programs. Neither compiler lowers extern blocks into IR, so
`vow-verify` sees nothing new and no C-model change is required. `docs/spec/contracts.md:420`'s
claim that "ESBMC uses `requires` as assumptions and `ensures` as assertions when verifying callers
of extern functions" is **unimplemented in both compilers today** — a real spec/implementation gap,
but a feature, not this parity bug. It is listed under Out of scope and will be recorded on the
issue.

The self-hosted compiler is itself a verified Vow program, so the ESBMC obligations that *do* change
are the contracts on the functions this plan edits:

- `item_pack`'s widened `requires: kind <= 5` must still discharge, together with `item_kind`'s
  `ensures: result == v / 4294967296` over the wider tag domain. Both are pure integer facts about
  the packing scheme; if either fails to discharge, the correct response is to strengthen the
  proof obligation's statement, **not** to reinstate a false `kind <= 4` bound.
- `arena_add_ext` and the new accessors follow the `arena_add_alias` / `alias_*` shape exactly, so
  they inherit the same (absent) contract surface. Do not invent index bounds that merely satisfy
  `--unwind`.

Fixture growth: two new fixtures (§3), one under `tests/error/`, one under `tests/run/`. No
`examples/` change — `extern` is not a construct an example program should model, since the callable
half is unimplemented.

---

## 6. Risk areas

- **Binary fixed point.** New code in `ast.vow` / `parser.vow` / `checker.vow` changes the compiler
  binary, so stages B and C of the triple test must still agree byte-for-byte. Nothing in this plan
  touches `BTreeMap`-vs-`HashMap` iteration order, `vow-clif-shim` stack-slot layout, or codegen
  ordering, so the exposure is the ordinary "did the new code compile itself deterministically"
  kind, not the structural kind. If B ≠ C, suspect a non-deterministic container introduced in
  `parse_extern`, not the shim.
- **`AstArena` gains a field.** `arena_new` must initialise `ext_data`. A missed initialiser is a
  self-compile failure at bootstrap, not a silent miscompile — loud, but it will stop the build.
- **`parse → print → parse` idempotency.** Untouched: the canonical printer lives only in
  `vow-syntax/src/printer.rs`, the self-hosted compiler has no AST printer, and this plan changes no
  Rust parsing or printing. Slice 6's doc edit brings `grammar.md` *into* agreement with
  `print_extern`, which strictly reduces drift.
- **`cargo clippy --all -- -D warnings`.** Slices 1-5 touch no `.rs` file at all; Slice 6 touches
  `vow/src/skill.rs` only through the generator, which emits string literals. Clippy exposure is
  effectively nil. (Per the recorded gate, CI runs without `--all-targets` — match that locally.)
- **Infinite loop on malformed extern blocks.** The `pre_iter` no-progress guard in Slice 3 is the
  only thing standing between a truncated `extern "C" { fn` and a hung self-hosted compiler.
  `scripts/equivalence.py` treats a hang as a hard failure. Test a truncated block by hand.
- **`vowc mutants` skip-list.** `compiler/mutants_sites.vow:584 add_extern_ranges` excludes
  `extern "C" { ... }` ranges by *textual* brace matching, independent of the parser. Parsing
  support does not change it, and the existing `tests/mutants/tests.sh` coverage of
  `tests/fixtures/mutants/sample_extern.vow` keeps working. One line of confirmation in the PR body,
  no code change.
- **Nightly equivalence sweep goes red on a correct fix.** Covered in §3: `equivalence.py` exits 1
  when a still-`open` ledger entry stops diverging. The ledger edit is not optional bookkeeping, it
  is part of the fix.
- **Generated-file churn in Slice 6.** Large but mechanical. Reviewers should read `docs/spec/*.md`
  and `scripts/generate_help.py` and take the rest on the generator's word;
  `scripts/generate_help.py --check` in CI is what guarantees they match.

---

## 7. Out of scope

Deliberately **not** in this PR:

1. **Extern contracts as ESBMC assumptions/assertions.** `contracts.md:420` promises it; neither
   compiler implements it (`Item::Extern` is never lowered; `vow-verify` has no extern-contract
   path). Real gap, separate feature, needs its own issue. Slice 6 will **not** delete that sentence
   from the spec — the spec is stating intent, and silently removing it would erase the gap instead
   of recording it.
2. **Making extern functions callable** (lowering `Item::Extern` to declarations, symbol
   declaration, linkage). Out of scope for a parser-parity fix, and both compilers agree today.
3. **The two cosmetic cascade-depth divergences** named in the issue —
   `tests/error/undefined_function.vow` (2 vs 1 `TypeMismatch`) and
   `tests/error/match_arm_missing_comma_{scalar,block}.vow` (2 vs 3 `UnexpectedToken`). The issue
   filed them "for the record, not blocking", both have agreeing codes, and tightening a cascade is
   a separate behavioural change.
4. **A hints-carrying seam in the self-hosted checker** (`env_emit_error_code_hinted` or similar).
   Needed for full diagnostic parity on `MissingContract`, cross-cutting across every checker error,
   and not required by `parity.py`'s code-multiset comparison. Follow-up.
5. **Refactoring `skip_unknown_item`** to consume a whole malformed item in one pass. It is the
   mechanism behind the 3× error count, but with `extern` handled the recovery path is no longer on
   this issue's critical path, and changing recovery would move error counts on unrelated fixtures.
6. **Moving or reshaping `tests/fixtures/mutants/sample_extern.vow`.** It is an input to
   `tests/mutants/tests.sh`; leave it exactly as it is.
7. Formatting, unrelated cleanups, and any `.rs` change not produced by `generate_help.py`.

---

## 8. PR shape

Squash-merge only; the PR title becomes the commit subject on `main` and must pass commitlint
(lower-case subject, no trailing period, ≤ ~92 chars before GitHub appends ` (#N)`).

Proposed title:

```
fix(parser): parse extern "C" blocks in the self-hosted compiler
```

The body should carry: the accept/reject evidence from §1, the grammar decision from §2 (with the
`grammar.md` correction called out so a reviewer does not read it as scope creep), the ledger flip
from §3, and the mutants-skip-list confirmation from §6.
