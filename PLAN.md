# Plan: IR dominance validator for the self-hosted IR (#1405)

## Goal
Add `compiler/ir_dominance.vow`, a pure well-formedness pass that finds value reads whose definition does not dominate the use (sibling-branch reads included), and wire it into the verifier's modelability gate so such a function is `Skipped` with reason `ir-non-dominating-read` (ADR 2026-10-08-1430 §4, epic #1398 D13). Codegen and compilation are untouched. Self-hosted only (ADR 2026-10-08-1421 exempts the dominance validator from the dual-compiler rule; no Rust twin).

## Assumptions
- Hook point is the existing modelability gate, not a new call in `main.vow`: every verify path already funnels through `is_modelable` / `non_modelable_reason` (`compiler/main.vow:1028` via `skip_if_non_modelable` at 1098/1711/2145, `main.vow:15840`, `compiler/verifier.vow:1217` `caller_precondition_role`, callees via `collect_callees_dfs`). One hook there satisfies "runs on every function the verifier considers" including callees and caller-precondition targets. (best guess)
- No `reason_code` JSON field now: the ADR assigns `reason_code` to P1. The code string `ir-non-dominating-read` appears in the human `reason` text, built from one constant, so P1 can lift it into the field later. (best guess)
- A use in an unreachable block is ignored (the lowerer can leave dead blocks after `return`); a reachable use of a value defined in an unreachable block is a violation. (best guess)
- An operand id with no defining instruction is reported as a violation under the same code (the IR is malformed; fail closed). If the corpus survey (Step 9) shows legitimate IR with sentinel/undefined operand ids, narrow this rule to the observed sentinel and record it in the PR body.
- `IrVowEntry.binding_ids` are not checked: their only consumers are `compiler/clif.vow:416` (runtime value capture for `VowViolation`) and `.vmod` serialization (`module_io.vow:887`); the verifier's C emitter never reads them, so Skipping a function over display-only data would be wrong. A non-dominating binding is a codegen concern, out of scope. (best guess)
- Closure of #1405 vs #1407: #1407 is blocked-by #1405 (the validator is the tool that produces its repro), so a no-go at Step 9 is not a deadlock. If Step 9 is no-go, the PR says `Part of #1405` (not `Closes`) because acceptance criterion 2 stays open until wiring lands. (decided from the issue graph)
- Real lowered IR may currently contain sibling-branch reads (that is bug #1407, open, `fix(lower): sibling-branch value reads in the self-hosted lowerer`). Such functions will become `Skipped` once the gate is wired; that is the intended fail-closed behaviour. Step 9 decides whether wiring is safe for the existing corpus.
- The new module carries no `vow` blocks: bootstrap runs the verifier over `compiler/*.vow` and a vowed function that is non-modelable (collections passed to user fns, `Vec<Vec<i64>>`) is `Skipped` and fails the build. Contracts would add no value on a pure checker whose tests are the oracle. (best guess)

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `compiler/ir_dominance.vow` [new] | validator: CFG, dominators, def/use check | — |
| `compiler/c_emitter.vow` | hook in `is_modelable` (369-) and `non_modelable_reason` (588-); add `use ir_dominance` | 3-6, 369-390, 588-635 |
| `compiler/tests/test_ir_dominance.vow` [new] | hand-built IR + lowered-source fixtures | — |
| `compiler/tests/builders.vow` | add `mk_inst_branch/jump/phi/upsilon` helpers (additive) | 1-80 |
| `scripts/concat_vow.sh` | insert `ir_dominance` into the `clif` FILES list before `c_emitter` (bootstrap triple test) | 23 |
| `compiler/ir.vow` | read-only reference: `IrInst` (224), `IrBlock` (238), `IrFunction` (258), `iop_is_terminal` (129) | |
| `compiler/region.vow` | read-only reference: `linear_successors` (625), block-tree dominance (3930-4140) | |
| `compiler/lower.vow` | read-only reference: Upsilon/Phi/short-circuit/loop emission (2725, 2853-2875, 3180-3257, 3360-3480) | |

## Semantics the validator encodes
- Blocks keyed by `IrBlock.id`; remap id→index with a `Vec<i64>` sized `max_id+1` (hand-built IR need not have id == index).
- Successors: scan the block backward for the last terminal (Upsilons are appended after Jump/Branch), `Branch` → `dv`,`dv2`; `Jump` → `dv` (same logic as `linear_successors`, `region.vow:625`). Do not import `region` (5.8k lines, loop-ignoring approximation); re-derive locally.
- Entry = `f.blocks[0]`. Dominators via Cooper–Harvey–Kennedy over reverse postorder (iterative DFS, no recursion); then pre/post numbering of the dominator tree so `dominates(a,b)` is O(1). Cost O(B·α + I); no O(B²) bitsets (compiler functions have thousands of blocks).
- Defs: `def_block[inst.id]`, `def_pos[inst.id]` (Vec sized max inst id+1). A `Phi` is a def at its own block/position (read by uses like any value).
- Uses checked: every `inst.args[k]` of every inst, including `Upsilon` (its value arg must dominate the Upsilon's position), `Branch` cond, `Return`, `Vow*` args. Violation iff def block ≠ use block and def block does not dominate use block, or same block and `def_pos >= use_pos`, or no def.
- Back-edge values flow only through Phi/Upsilon, so a loop body def read in the header/exit is correctly rejected; no special-casing.
- Output: `ir_non_dominating_read(f: IrFunction) -> String`: `""` when well-formed, else one sentence naming the first offending use (`inst %U in block B reads %D (block C)`). `fn SKIP_IR_NON_DOMINATING_READ() -> String` is the reason code; the sentence starts with it. Deterministic (ascending block index/inst order, no hash maps).

## Steps (TDD slices, each red → green; commit per slice)
Language gotchas: no closures/generics; chained field access on struct values needs annotated `let` (`let blk: IrBlock = f.blocks[i];`); keep functions short (small-files rule), split CFG / dominator tree / use check into separate small fns inside the one module (deep module: one exported entry point + the code constant).

### 1. Straight-line block (red: module missing)
- **Files**: `compiler/tests/test_ir_dominance.vow` [new], `compiler/ir_dominance.vow` [new], `compiler/tests/builders.vow`.
- **Tests**: single block accepted; use-before-def in same block rejected; operand with no def rejected.
- **Code**: module skeleton, def table, per-inst use check, `SKIP_IR_NON_DOMINATING_READ()`.
- **Reuses**: `mk_inst*`, `mk_block`, `mk_function`, `mk_module` (`builders.vow:9-80`), `ir_inst_new` (`ir.vow`).

### 2. If/else diamond (core acceptance: sibling-branch read)
- **Tests** (hand-built, blocks entry→then/else→merge): accept merge reading entry defs and a Phi fed by Upsilons in then/else; reject else-block reading a then-block def (sibling); reject merge reading a then-only def.
- **Code**: successor extraction, RPO, CHK idom, dominator-tree pre/post numbering, `dominates`.

### 3. Loops
- **Tests**: while-shaped CFG (preheader→header⇄body, header→exit) with loop-carried Phi: accept header-Phi reads in body/exit and preheader defs in body; reject body def read in exit; reject body def read in header (back-edge value not via Upsilon). Nested loop and `break`-style edge to exit.
- **Code**: none new if CHK handles back edges (iterate to fixpoint); add only if red.

### 4. Match
- **Tests**: test-chain CFG (each arm test is a Branch; arms jump to merge): accept scrutinee reads in every arm and Phi at merge; reject arm-A def read in arm-B and in a later test block.

### 5. Short-circuit `&&` / `||`
- **Tests**: shape from `lower.vow:2853-2875` (Branch on lhs → rhs block / merge, Upsilons in rhs and short-circuit blocks, Phi in merge): accept; reject merge reading the rhs-block value directly instead of through the Phi.

### 6. Unreachable blocks, Upsilon/Vow operands, malformed CFG
- **Tests**: dead block's own reads ignored; reachable read of an unreachable def rejected; `VOW_REQ/ENS` arg and `Upsilon` value arg are checked; branch target to a nonexistent block id does not crash (treated as no successor); empty function (`blocks.len() == 0`) returns `""`.

### 6b. False-positive guards (real dominance, not "sibling" matching)
- **Tests**: if/else where the else arm ends in `Return` (or `Unreachable`) and the merge reads the then-arm's def directly: accepted (then dominates merge because else never reaches it). Same for a `match` arm that returns. These pin that the check is dominance-based; a syntactic sibling check would reject them.

### 7. Lowered-source positives
- **Test**: in `test_ir_dominance.vow`, copy the `lower_fixture` helper from `compiler/tests/test_lower_loop_carried_scope.vow:15-30` (same local-helper convention as the other lower tests; do not widen `builders.vow` with lowerer imports). Inline sources covering if/else (value and statement), `match` over enum, `while` and `for` with mutated variables, `&&`/`||` in conditions and as values. Assert `ir_non_dominating_read` is `""` for every lowered function.
- If any construct is flagged, that is #1407's repro: do not weaken the validator and do not xfail silently. Drop that construct from this PR, add its source to a comment on #1407 (`gh issue comment 1407`), and mention it in the PR body.

### 8. Wire the gate (red: reason is empty for a bad function)
- **File**: `compiler/c_emitter.vow` + `use ir_dominance` (line 5 area).
- **Change**: in `is_modelable`, after the `f.effects != 0` check (cache entry already set to 0 at ~384), `if ir_non_dominating_read(f).len() > 0 { return false; }`. In `non_modelable_reason`, after the reserved-symbol and effects branches and before `is_modelable`, return the standard sentence `function \`f\` is not modelable in the verifier (ir-non-dominating-read: …)`. Order: reserved symbol, effects, dominance, opcodes/callees. A callee with a bad read makes callers non-modelable through the existing "calls a non-modelable callee" path (ADR code `non-modelable-callee`).
- **Tests** (`test_ir_dominance.vow`, imports `ir`, `ir_dominance`, `c_emitter`, `verifier`, `builders`): `non_modelable_reason` on the bad hand-built fn contains `ir-non-dominating-read` and is `""` for the good one; a caller of the bad fn is non-modelable; a function with effects keeps the effects reason; `caller_precondition_role` returns 2 for an uncontracted caller whose own IR is bad. Existing `compiler/tests/test_c_emitter.vow` must stay green.
- **Also**: `scripts/concat_vow.sh` clif list gets `ir_dominance` before `c_emitter`.

### 9. Corpus survey and fixed-point check (gate on wiring)
- With a freshly built `build/vowc` (use `VOW_CACHE_DIR=$(mktemp -d)`), run `vowc verify` before and after Step 8 over `tests/verify/`, `tests/verify-fail/`, `tests/verify-skip/`, `tests/verify-stress/` and `compiler/main.vow`; the set of `Skipped` functions must be identical. Section 4b–4d of `scripts/full_test.sh` compare Rust-vs-self JSON, so a newly flagged fixture would fail parity because stage 0 does not validate.
- Cheapest tripwire: `python3 scripts/parity.py c RUST_BIN SELF_BIN tests/verify*/...` (full_test Section 2c) puts a fake `esbmc` on `PATH`; a function newly Skipped by the self-hosted compiler is a missing C source, with no solver time. Run it before and after Step 8; any diff is a hit.
- Bootstrap matters too: `scripts/bootstrap.sh` Stage 2/3 build the compiler with the self-hosted `build/vowc` *with verification on* (`run_self_stage`, `stage12_build_flags="--verify-jobs 1"`), so a vowed compiler function flagged by the new gate would turn Stage 2/3 red (Skipped exits 1).
- If clean: ship the gate. If any real function is flagged: keep Steps 1–7 (module + tests), do not land Step 8's wiring, and `gh issue comment 1405` + `1407` with the flagged functions and IR block shapes; the wiring follows #1407. Never weaken a contract or silence a hit.
- `scripts/bootstrap.sh --skip-cargo --no-cache` on the final head SHA; record SHA and `scripts/seed.toml` pin (if present) in the PR checklist.

## Testing
- `build/vowc test compiler/tests/test_ir_dominance.vow` (iterate), then `build/vowc test compiler/` and `build/vowc test compiler/tests/test_c_emitter.vow`.
- Quality gates run as separate commands: `scripts/bootstrap.sh --skip-cargo --no-cache`, then `scripts/full_test.sh` in the background (~40 min; poll, do not use the default timeout). `cargo` gates are unaffected (no Rust change) but `cargo test --all` still runs inside `full_test.sh`; ~8 vow-crate run tests failing in the sandbox is environmental (see memory notes).
- Coverage: `codecov/patch` (95%, blocking) measures only the Rust crates instrumented by `cargo llvm-cov`; this change touches no Rust, so it is not exercised. Still cover each bounds-guard branch with a Step 6 test.
- Commit/PR title: `feat(verify): add dominance validator for the self-hosted IR` (lower-case subject, ≤92 chars). Remove `PLAN.md` with `git rm` before opening the PR.

## Verification surface
- Native ESBMC proofs: none required (no contracts in the new module, by design). ESBMC will still analyse the new functions only if they become caller-precondition targets; avoid calling contracted functions (`region_pack`, etc.) from the module so nothing is `Skipped` in bootstrap.
- No new `tests/run/` or `examples/` fixtures: a source program cannot reach the validator's reject path without exploiting #1407, and a `tests/verify-skip/` fixture would diverge from the Rust twin in the Section 4d JSON parity compare. Hand-built IR unit tests are the oracle.
- C parity (`scripts/parity.py c`): unaffected for modelable functions; the Rust `c_emitter.rs` is deliberately not changed.

## Risks
- **Real IR flagged by the gate** (the main risk): bootstrap Stage 2/3 (self-hosted, verifying) and full_test Rust-vs-self parity (Sections 2c, 4b–4d) both fail on a newly Skipped function; Step 9 is an explicit go/no-go.
- **Binary fixed point**: new module and `c_emitter.vow` edit change the compiler source; they must be deterministic (no HashMap iteration; sorted/index-ordered loops only) and be added to `concat_vow.sh` or the triple test breaks.
- **Quadratic behaviour**: avoid `region_vec_contains_i64`-style linear membership in hot loops; use index-addressed `Vec<i64>` tables and tree numbering. Mind that `non_modelable_reason`/`is_modelable` are called per function repeatedly with fresh caches: the pass is linear-ish, but do not call it per inst or per callee edge beyond the existing cache.
- **Recursion**: Vow stack depth on deep CFGs — all traversals iterative.
- **Message text divergence** from Rust `non_modelable_reason`: affects only IR that stage 0 does not validate; documented here and in the PR body.
- **Unreachable/malformed IR** (dangling branch targets, ids ≥ table size): bounds-guard every table access; never panic in a gate that runs on every function.
- No `parse → print → parse` impact (no syntax change), no clif-shim or `BTreeMap` change, no clippy exposure (no Rust change).

## Out of scope
- The lowerer fix for sibling-branch reads (#1407) and any Rust-compiler change.
- `reason_code` JSON field, schema updates, and `docs/spec/*`, `--help`, skill regeneration (ADR defers the surface rewrite to P1/P3; this change alters no CLI flag, syntax or builtin).
- A Rust twin of the validator; refactoring `region.vow` dominance code or deduplicating the `lower_fixture` test helper; making the ESBMC emitter handle non-dominating reads.
- Using the validator in codegen or the clif backend ("no behaviour change for compilation").
