# Vow Language & Self-Hosted Compiler — Comprehensive Audit

**Date:** 2026-06-10  ·  **Scope:** `main` @ `d220ad2` (+ open PRs/issues cross-referenced)  ·  **Target:** the Vow surface language, the verification pipeline, the self-hosted compiler (`compiler/*.vow`), and the Rust bootstrap (`vow-*` crates), plus benchmarks, mutation testing, docs/spec, and the agent-facing tool surface.

Evaluated against `docs/vow_design.md` (authoritative). Deliberate exclusions per §3 / §5.9 (generics, traits, closures, macros, operator overloading, subtyping, exceptions, `null`, statement-level `assert`/`assume`, `async`, interactive LSP/debugger, multiple visibility gradations) are treated as **design boundaries, not gaps**, and are not reported as missing features.

---

## How this audit was produced (methodology & caveats)

A multi-agent workflow fanned out **40 finders** across the three lanes plus peripheral subsystems. Every finder was required to load the Vow skill + `docs/spec/*` + `docs/vow_design.md`, cite exact `file:line` ranges, quote the offending code, and dedupe against the ~150 open GitHub issues. Each finding was then sent to an **adversarial cross-check** (3 independent skeptics for high/critical claims — *reproducible-from-code*, *design-consistency*, *impact-and-severity* lenses — and 1 for medium/low), instructed to **refute by default**.

The run was interrupted twice: first a harness OOM (V8 heap, during peak Verify), then — on resume — a hard **account usage limit** that killed the remaining live verifiers and the dedup/synthesis agents. **All 41 finder outputs (188 findings) and 185 verifier verdicts were preserved on disk** and salvaged directly from the workflow transcripts. A **follow-up bounded confirmation pass** (a dedicated 77-agent workflow, one static adversarial verifier per previously-unverified finding, severity-ordered, no builds) then adjudicated every remaining item: **all 77 now carry a verdict** (63 confirmed, 14 refuted). This document is assembled from that data plus the author's own dynamic reproductions. Consequences for how to read it:

- **Status legend:** ✅ *survived* = adversarial reviewer(s) confirmed it; ❌ *refuted* = reviewer(s) could not reproduce it or judged it a non-issue (kept here, clearly marked, for transparency — do not file as-is). ⚠️ *unverified* should no longer appear; if it does, that finding's reviewer was lost and it needs a manual confirm.
- **Severity** shown is the finder's claim; the per-finding line surfaces the reviewers' `severity_adjusted` votes where they differ. Reviewers consistently downgraded several soundness findings (notably the verifier-panic pair) from critical→high on the grounds that they are gated on a specific trigger (an internal panic, a non-zero `u64`, an overflowing input) rather than firing on routine input — the "critical" label reflects worst-case **trust** impact, the reviewer "high" reflects conditional reachability. Both views are shown.
- **Dynamically reproduced** findings (built `target/release/vow` and ran them) are called out explicitly; treat those as the highest-confidence items.
- A handful of findings are the *same root cause* seen from different lanes; these are cross-referenced in each lane's intro rather than silently merged.

**Counts:** 188 findings — by severity: 11 critical, 53 high, 68 medium, 56 low · by cross-check: 165 survived, 0 unverified, 23 refuted · by lane: L1=32, L2=32, L3=100, P=24.


---

## Executive summary — top 5 by impact

The audit's defining result: **multiple paths let unverified code be reported as `Verified`/`Unverified`-but-OK** — the exact failure mode Vow exists to prevent (§2.1: "verification is the primary trust mechanism"). Two of the five below were reproduced live by the author against `target/release/vow`.

1. **Refinement-type predicates are silently dropped → false `Verified`** (`critical`, Lane 2, **dynamically reproduced**). A return/parameter refinement `{ r: i64 || r > 0 }` is fully parsed and round-trips through the canonical printer, but `vow-types/src/env.rs:903` resolves `Type::Refinement` to its base type and discards the predicate — the *only* consumer of refinements in the entire pipeline. No `ensures`/`requires` obligation is ever generated. Live repro: `fn make_positive(x: i64) -> { r: i64 || r > 0 } { x }` returns `{"status":"Verified"}`, while the identical `ensures: result > 0` returns `VerifyFailed` with counterexample `x = -1`. §8 lists this as "Partial"; in practice it is an unsound false proof, not a benign gap. An agent reading the JSON believes a postcondition is proven that is not.

2. **The self-hosted (production) verifier pins every `u64` parameter to the constant `0`** (`critical`, Lane 2, ESBMC-reproduced by the cross-checker). `esbmc_nondet_call` in `compiler/verifier.vow:163` has no `ITY_U64` arm and falls through to the literal `"0"`, so ESBMC explores only the single input `0` for every `u64` parameter. A contract that is false for some non-zero `u64` (e.g. `ensures: result >= x` on `x + x`) is reported `Verified` by `build/vowc` while the Rust compiler correctly reports `VerifyFailed`. `cli.md` explicitly promises `u64` contracts are checked "using `uint64_t` and unsigned nondet values." The Rust harness (`vow-verify/src/esbmc.rs:78`) is correct; only the self-hosted copy regressed.

3. **Checked arithmetic (`+!`,`-!`,`*!`,`/!`,`%!`) is modeled identically to wrapping in ESBMC** (`critical`, Lanes 1 & 2). Both C emitters fold the checked opcodes into the same arms as the wrapping opcodes and emit a bare C operator, and ESBMC is invoked with **no** `--overflow-check`. The runtime (Cranelift) *does* emit an abort path, so the verifier proves postconditions over a wrapped result the program never actually produces (it aborts first), and never reports the abort as reachable. §5.7's "abort with `ArithmeticOverflow`" is unmodeled; §8 "builtin panic/unsafe effect coverage: Partial" understates this — it is a silently dropped safety obligation (relates to #335).

4. **The self-hosted compiler — the production `build/vowc` — enforces neither vow-clause purity nor linear single-consumption** (`critical`, Lane 1). It has no equivalent of the Rust `check_vow_purity`, so an effectful predicate like `requires: fs_read(path) == "x"` on a "pure" function compiles clean and is then sent to ESBMC with an obligation it cannot model (`compiler/checker.vow:610`). Separately, its only linear check is an idempotent IR-region liveness pass, so double-consume / consume-in-loop / partial-path consume of a `linear` value produce **no** `LinearTypeViolation` (`compiler/region.vow:428`) — a silent double-free of the underlying resource, contradicting §5.6 and the §8 "Implemented" status (which holds only for the Rust bootstrap, not the self-hosting validation target).

5. **The trust surface fails open in two more ways** (`critical`/`high`, Lane 3): a **verifier-thread panic is silently degraded to `Unverified` + exit 0** with the linked binary already written (`vow/src/main.rs:9206`, `unwrap_or((VerifyOutcome::Skipped, …))`; relates to #413) — indistinguishable from a deliberate `--no-verify` skip; and **method-call arguments are never type- or arity-checked** (`vow-types/src/check.rs:1010`), so `v.push(String::from("x"))` on a `Vec<i64>` type-checks (**dynamically reproduced**: compiles with no error), storing a heap pointer into an `i64` slot that contracts then read back as an integer. `vow contracts --verify` likewise exits `0` on failed contracts (#479).


---

## Dynamic reproduction (author-run, against `target/release/vow`)

The stage-0 Rust compiler was built (`cargo build --release -p vow`) and the following were run under `ulimit -v 2000000`. These are the highest-confidence findings in the report.

| # | Finding | Program | Result | Verdict |
|---|---------|---------|--------|---------|
| R1 | Refinement predicate dropped (Lane 2) | `fn make_positive(x:i64) -> { r:i64 \|\| r>0 } { x }` | `{"status":"Verified"}` | **BUG** — predicate `r>0` never checked |
| R1′ | (control) same as `ensures` | `fn make_positive(x:i64) -> i64 vow { ensures: result>0 } { x }` | `VerifyFailed`, cex `x=-1` | correct — proves the refinement path specifically drops the obligation |
| R2 | Method-call arg type confusion (Lane 3) | `let v: Vec<i64> = Vec::new(); v.push(String::from("x"));` | compiles, `executable` produced, **no type error** | **BUG** — `String` stored into `Vec<i64>` slot |

Additionally, the adversarial cross-checkers independently reproduced (their transcripts cite live ESBMC/compiler runs): the **u64-pinned-to-0** hole (#2 above) and the **checked-arithmetic-as-wrapping** hole (#3 above).


---

## "Diverges from design doc" — extending §8's status table

Columns: design area · the status `docs/vow_design.md` §8 claims · what the audit actually found · severity · representative finding.

| Area | §8 status | What the audit found | Severity | Finding |
|------|-----------|----------------------|----------|---------|
| Refinement type predicates in verification | **Partial** | Not "incomplete" but **unsound**: predicate silently dropped at `env.rs:903`, producing false `Verified` (reproduced). The §8 wording undersells a trust-breaking hole. | critical | L2-refinement-fwd |
| Builtin panic/unsafe effect coverage | **Partial** | Checked-arith overflow abort and `.unwrap()` panic are **never modeled** in ESBMC → dropped safety obligations; effect checker also misses indexing/division panics. | critical/high | L2-builtin-effects, L1-arith |
| Linear resource types | **Implemented** | True only for the Rust bootstrap. The **self-hosted production compiler never emits `LinearTypeViolation`** (idempotent region pass); double-consume is silent. | critical | L1-linear |
| Effect propagation (user-defined fns) | **Implemented** | Self-hosted compiler has **no vow-clause purity check** at all; both compilers make `[io]` subsume `[read]`/`[write]`, contradicting §5.5 "each effect is independent." | critical/medium | L1-effects, L3-effects-rust |
| Quantifiers in contracts | **Target** | `is_modelable` gate admits functions the C emitter then models with unconstrained nondet stubs rather than failing closed (#572). | high | L2-quantifiers-loops |
| Parameter `where` refinements | **Implemented** | Lowered to `requires`, but the `where`/refinement predicate expression is **never type-checked** — a non-bool or ill-typed predicate is accepted. | high | L1-transition-debt |
| Canonical form (parse→print→parse idempotent) | (implied Implemented) | Multiple printer bugs break idempotence: `&(a+b)`→`&a+b`, `(a+b)?`→`a+b?`, nested unary `-!x` is unparseable, all block bodies printed at indent 0. | high | L1-syntax-canonical, L3-printer-rust |
| Arena-per-scope memory model | **In Progress** | AMBIGUOUS-region allocation can escape via a direct `FieldSet`/`Store`/extern store without the rewrite that guards `Call` results (`region.rs:3314`). (#437's arena-bound issue is already fixed.) | high | L3-ir-region-rust |
| Debug-mode contract/boundary traces | **Partial** | `VowViolation` JSON is assembled with no escaping / no finite-number handling (#436); `u64` captures emit the `i32` tag + zero payload (#439) in both backends. | high/medium | L3-runtime, L2-debug-runtime |
| `u64` contracts (cli.md promise) | — | Self-hosted verifier checks them only at input `0` (see top-5 #2). | critical | L2-soundness-skip |
| Verification status reporting (fail-closed, §6.4 / cli.md) | — | Verifier-thread panic → `Unverified` + exit 0 (#413); `vow contracts --verify` exits 0 on failed contracts (#479); self-hosted `verify_collect` ignores ESBMC exit code. | critical/high | L3-vsec, L3-driver-rust |
| Module-level vow blocks | **Target** | Confirmed not implemented — consistent with §8, not a finding. | — | — |
| Mandatory contracts on extern decls | **Target** | Confirmed not enforced — consistent with §8, not a finding. | — | — |


---

## Lane 1 — Language Design for Agentic Coding

**Root-cause clusters (cross-lane):** the *checked-arithmetic-as-wrapping* model hole appears here (L1-arith) and in Lane 2 (L2-builtin-effects, L2-lowering-rust) — one fix. The *self-hosted has no vow-purity check* finding (L1-effects) is the same defect as L3-effects-rust. *`io` subsumes `read`/`write`* appears as both an L1 and an L3 finding. Treat each cluster as a single work item.

_32 findings — 4C / 12H / 12M / 4L._

#### L1.1 Checked arithmetic family (+!, -!, *!, /!, %!) abort-on-overflow semantics are NOT modeled by the verifier — verified code can abort at runtime — `CRITICAL` · ✅ survived cross-check · **Duplicate of #335**
**finder:** `L1-arith` · **kind:** soundness · **verdicts:** 3 · reviewer severity votes: high×1, critical×1, medium×1  
**Files:** `vow-verify/src/c_emitter.rs:864-908`, `vow-verify/src/solver_strategy.rs:84-98`, `vow-verify/src/esbmc.rs:420-426`  
**Design ref:** §5.7 (checked operators abort with ArithmeticOverflow on overflow); §4.5 (counterexample reporting is part of the contract); divergence from intended verifier-faithful model  
**Evidence:** The C emitter folds wrapping AND checked opcodes into identical plain-C operators. Add: `Opcode::WrappingAddI64 | Opcode::CheckedAddI64 | ... => { out.push_str(&format!("  v{} = v{} + v{};\n", id, a, b)); }` (c_emitter.rs:864-872); the same pattern for Sub (873-881), Mul (882-890), Div (891-899), Rem (900-908). The ESBMC invocation is `--no-bounds-check --no-pointer-check --incremental-bmc --max-k-step N --64` plus only `--solver`/`--ir`/`--memlimit` (esbmc.rs:420-426; solver_strategy.rs:84-98 esbmc_args). There is NO `--overflow-check`, and ESBMC does not enable arithmetic-overflow checking by default. So `x +! y`, `x *! y`, `x -! y`, and `i64::MIN /! -1` are all modeled as silently wrapping C arithmetic that continues execution. At runtime the codegen for checked ops calls `__vow_arithmetic_overflow` and traps (cranelift_backend.rs:895-924, emit_overflow_check:1677-1693). Design §5.7 states `+!`,`-!`,`*!` "are checked and abort with ArithmeticOverflow on overflow". The verifier therefore proves postconditions using a wrapped result that the runtime never produces (it aborts first), and never reports the abort as a reachable program behavior. A function `fn f(a:i64,b:i64)->i64 vow { ensures: result >= a } { a +! b }` would be reported PROVEN under the wrapping model for inputs where the runtime aborts. This is verified-but-aborts divergence — unverified runtime behavior reported as verified.

**Proposed fix:** Model checked ops faithfully in the C harness: for `+!`/`-!`/`*!` emit a built-in overflow guard (e.g. `__builtin_add_overflow(a,b,&v)` with `if(__of) __VERIFIER_error();` or an explicit `__ESBMC_assume`/assert encoding) so ESBMC treats overflow as a reachable abort, and for `/!`/`%!` add the `INT_MIN/-1` overflow guard in addition to the existing div-by-zero behavior. Alternatively pass `--overflow-check` and emit checked ops with signed types while emitting wrapping ops with explicit modular/unsigned arithmetic so the two families are distinguishable. The wrapping family must remain modeled as defined wraparound (not flagged).

#### L1.2 Self-hosted compiler performs NO purity check on vow-clause predicates (requires/ensures/invariant) — `CRITICAL` · ✅ survived cross-check
**finder:** `L1-effects` · **kind:** soundness · **verdicts:** 3 · reviewer severity votes: high×2, medium×1 · **Filed as #586**  
**Files:** `compiler/checker.vow:610-635`, `compiler/checker.vow:665-690`, `compiler/checker.vow:1799-1825`, `vow-types/src/effects.rs:228-271`  
**Design ref:** §5.5 ('Expressions inside vow clauses must be pure. Contract checking must not itself perform I/O or hidden state changes.'); §2.2; grammar.md:669-671 (Contract Purity)  
**Evidence:** `check_fn` in the self-hosted compiler validates vow clauses ONLY for being `bool`-typed, never for purity. `check_vow_clause` (checker.vow:610-635) does: `let clause_tid: i64 = check_expr(e, m, clause_eid); if clause_tid != CTY_BOOL() && !is_opaque(clause_tid) { ... must be \`bool\` }` — no effect/purity analysis. The only effect pass, `check_effects_fn` (checker.vow:1799-1825), runs `collect_calls_in_block(e, m, body_bid, calls)` on the function BODY only (`let body_bid: i64 = fn_body_bid(a, fid); ... collect_calls_in_block(e, m, body_bid, calls);`) — vow clauses are never traversed. There is no equivalent of the Rust compiler's `check_vow_purity` anywhere in compiler/*.vow (grep for `pure`/`purity` finds only the help-text strings in main.vow and the `effectful call from pure function` body diagnostic). The Rust bootstrap compiler DOES check this (effects.rs:223-271: `if let Some(vow_block) = &fn_def.vow { check_vow_purity(...) }`). Because the self-hosted `build/vowc` is the production compiler, an agent can write `fn f(...) -> bool vow { requires: fs_read(path) == "x" }` on a pure function and it compiles clean. The verifier model is restricted to pure functions (c_emitter.vow:424-431: `if f.effects != 0 { return ... \`has effects; the verifier model is restricted to pure functions\` }`), so a pure function with an effectful predicate IS sent to ESBMC — with a contract obligation the verifier cannot faithfully model, silently weakening the proof.

**Proposed fix:** Port `check_vow_purity` into compiler/checker.vow: after `check_vow_clause`, run `collect_calls_in_expr` over each clause expression and emit `EffectViolation` (blame Callee) for any callee whose `e.fn_effects[fidx] != 0`, and for any `.unwrap()` (`__unwrap__`) found. Wire it for both function-level vow (checker.vow:665-682) and loop invariants. Add a self-hosted test under tests/run/ that an impure contract is rejected.

#### L1.3 Self-hosted (primary) compiler never emits LinearTypeViolation: double-consume, consume-in-loop, and partial-path consume of linear values are silently accepted — `CRITICAL` · ✅ survived cross-check
**finder:** `L1-linear` · **kind:** soundness · **verdicts:** 3 · reviewer severity votes: high×2, critical×1 · **Filed as #588**  
**Files:** `compiler/region.vow:428-453`, `compiler/checker.vow:357-692`, `compiler/diag.vow:17-17`  
**Design ref:** §5.6 (Linear types: 'must be consumed exactly once'); §2.4/§7 (self-hosted compiler is the validation target and must have feature parity); grammar.md:462 ('the caller cannot access it afterward')  
**Evidence:** The self-hosted compiler (`build/vowc`, the primary day-to-day compiler) performs its ONLY linear-usage check in `region.vow::check_linear_regions_function`, a forward liveness dataflow that reports a value still live at region close (`linear_emit_live_errors`, EC_REGION_LINEAR=19). Consumption is modeled by `linear_remove_origins` -> `linear_remove_i64`, which is idempotent: `fn linear_remove_i64(v: Vec<i64>, val: i64) { ... if v[i] == val { ...; return; } ... }` returns silently when the id is absent. So a SECOND consume of the same origin removes nothing and produces no error; at region close the value is not live, so RegionLinear does not fire either. There is NO AST-level consume-state tracking in `compiler/checker.vow` (grep for `ConsumeState`/`in_loop`/`already consumed` returns 0 hits). The error code IS defined — `compiler/diag.vow:17 fn EC_LINEAR_TYPE_VIOLATION() -> i64 { 6 }` and is name-mapped at diag.vow:192-193 — but it is NEVER emitted anywhere in the self-hosted sources (only the Rust `vow-types/src/linear.rs::consume_var` emits `already consumed`/loop/`may already be consumed`). Concretely, for `fn f(h: Handle) { consume(h); consume(h); }` the self-hosted lowering emits two `IOP_LINEAR_CONSUME` on the same origin (lower.vow:206-212, 2888-2892); region.vow removes once, no-ops on the second, no diagnostic. The embedded skill/errors doc claims the opposite: main.vow:4254 / docs/spec/errors.md:104 state LinearTypeViolation catches "consuming it twice, consuming it inside a loop that may execute more than once, or consuming it after only some control-flow paths already consumed it." Because `LinearConsume` is a pure no-op marker in codegen (cranelift_backend.rs:1594-1597 emits a unit; c_emitter.vow:313 marks it verifier-non-modelable), the entire single-consumption guarantee is static-only — so a missed double-consume is a silent double-free/double-close of the underlying resource with zero diagnostics in the production compiler.

**Proposed fix:** Port the AST-level single-consumption check from `vow-types/src/linear.rs` (consume-state per variable with Available/Consumed/MaybeConsumed, in-loop rejection, branch/match merge) into the self-hosted front-end (a new pass invoked from checker.vow over each FnDef body), emitting EC_LINEAR_TYPE_VIOLATION (code 6) for double-consume / loop-consume / partial-path consume. The IR region pass should remain only the 'unconsumed at region close' (RegionLinear) backstop. Add tests/error/*.vow fixtures for double-consume, consume-in-loop, and partial-branch-then-reuse mirroring the Rust unit tests, since none currently exist.

#### L1.4 Canonical printer emits unparseable `-!x` for Neg(Not(x)); parse→print→parse fails — `CRITICAL` · ❌ refuted by cross-check
**finder:** `L1-syntax-canonical` · **kind:** soundness · **verdicts:** 3 · reviewer severity votes: low×1, high×1, medium×1  
**Files:** `vow-syntax/src/printer.rs:519-529`, `vow-syntax/src/lexer.rs:77-83`  
**Design ref:** §5.2 (compiler enforces canonical source form), §6.5 (deterministic, diff-stable canonicalization), spec/grammar.md Canonical Form ('parse → print → parse is idempotent')  
**Evidence:** printer.rs `print_expr` UnaryOp arm only parenthesizes a `BinaryOp` operand:
```
ExprKind::UnaryOp { op, operand } => {
    let op_str = match op { UnOp::Neg => "-", UnOp::Not => "!" };
    let inner = match &operand.kind {
        ExprKind::BinaryOp { .. } => format!("({})", print_expr(operand)),
        _ => print_expr(operand),
    };
    format!("{}{}", op_str, inner)
}
```
A nested unary operand is NOT parenthesized, so `Neg(Not(x))` prints `-!x`. But the lexer (lexer.rs:77-83) lexes `-!` as a single `MinusChecked` token:
```
b'-' => { if self.peek_byte(1) == Some(b'!') { self.pos += 2; Ok(Token::new(TokenKind::MinusChecked, ...)) }
```
VERIFIED dynamically: real source `- !x` parses to Neg(Not(x)); the canonical printer emits `-!x`; reparsing that output yields parse errors: `["expected expression, got MinusChecked"]`. The canonicalizer thus produces output the compiler itself rejects.

**Proposed fix:** In the UnaryOp printer arm, parenthesize the operand whenever it is itself a UnaryOp (or at minimum when op==Neg and operand is UnOp::Not), e.g. print `-(!x)`. Alternatively widen the existing `BinaryOp =>` arm to also match `UnaryOp { .. }`. Add a roundtrip test for `- !x`/Neg(Not(x)).

#### L1.5 Checked division/remainder (/!, %!) do not check the INT_MIN/-1 overflow case the design requires — `HIGH` · ✅ survived cross-check
**finder:** `L1-arith` · **kind:** bug · **verdicts:** 2 · reviewer severity votes: medium×1, high×1 · **Filed as #599**  
**Files:** `vow-codegen/src/cranelift_backend.rs:925-954`, `vow-clif-shim/src/lib.rs:1880-1895`  
**Design ref:** §5.7 ("/! and %! ... abort on zero divisor or overflow") — implementation omits the overflow half  
**Evidence:** CheckedDivI64 only guards the zero divisor, then issues a raw sdiv: `let is_zero = builder.ins().icmp(IntCC::Equal, arg!(1), zero); emit_overflow_check(builder, is_zero, ctx)?; let val = builder.ins().sdiv(arg!(0), arg!(1));` (cranelift_backend.rs:925-932). Same for CheckedRemI64 (940-947) and the self-hosted shim CDIV/CREM (vow-clif-shim/src/lib.rs:1880-1895). Design §5.7 explicitly says: "`/!` and `%!` are checked division and remainder (abort on zero divisor OR overflow)". The `i64::MIN /! -1` (and `i64::MIN %! -1`) overflow case is not detected by any controlled check; it instead hits Cranelift's implicit sdiv/srem hardware trap (int_ovf), so the abort is an uncontrolled SIGFPE rather than the structured `__vow_arithmetic_overflow` path the zero case uses. Checked operators thus have two different abort mechanisms for the two failure modes the design lists together, and the overflow mode bypasses the runtime violation handler entirely.

**Proposed fix:** In CheckedDivI32/I64 and CheckedRemI32/I64 (both compilers), add an explicit guard for signed-min-over-negative-one (`a == TYPE_MIN && b == -1`) routed through emit_overflow_check before the sdiv/srem, mirroring the existing is_zero guard, so both failure modes abort via the same controlled `__vow_arithmetic_overflow` path. U64 variants are unaffected (no signed-overflow case).

#### L1.6 counterexample.schema.json mandates `inputs` but both compilers emit `values`; schema forbids the actual CEGIS-rich fields (blame, call_sites, violating_args, execution_path, branch_decisions) — `HIGH` · ✅ survived cross-check
**finder:** `L1-diagnostics-agent` · **kind:** design-divergence · **verdicts:** 3 · **Filed as #610**  
**Files:** `docs/spec/schemas/counterexample.schema.json:7-45`, `vow/src/main.rs:7936-7951`, `compiler/diag.vow:429-462`  
**Design ref:** §6.4 (ESBMC counterexamples as first-class repair inputs; CEGIS workflow), §6.5 (structured diagnostics and outputs)  
**Evidence:** counterexample.schema.json line 7 requires `["function", "inputs", "violation", "vow_id", "source"]` with `"additionalProperties": false` (line 44), and the only map field declared is `"inputs"` (line 13-17). But the Rust `CounterexampleJson` (main.rs:7936-7951) declares `pub values: BTreeMap<String,String>` (NOT `inputs`), `pub blame: String`, plus `call_sites`, `violating_args`, `execution_path`, `branch_decisions` — none of which the schema permits. The test at main.rs:11999 asserts `ces[0]["values"]["y"]` and main.rs:11994 asserts `ces[0]["blame"]`, proving the real wire field is `values` and that `blame` is emitted. The self-hosted `diag_ce_to_json` (diag.vow:432) also emits `"values":{...}` and `"blame":"..."` (diag.vow:454-459). Result: every real counterexample fails validation against the published schema — the schema requires a field (`inputs`) that is never emitted and forbids (`additionalProperties:false`) every field that actually carries the repair-critical data. An agent that validates ESBMC counterexamples against the shipped schema (or extracts violating values from `inputs` per the schema) gets nothing usable from the most CEGIS-load-bearing output.

**Proposed fix:** Make schema and impl agree on one canonical shape. Either rename the emitted map field to `inputs` in both `CounterexampleJson` (main.rs:7938) and `diag_ce_to_json` (diag.vow:432), or change the schema's required field to `values`. Then add the actually-emitted fields (`blame`, `call_sites`, `violating_args`, `execution_path`, `branch_decisions`) to counterexample.schema.json (and the embedded copy in main.rs:7301), or relax `additionalProperties`. Update the cli.md example (line 286) to the chosen canonical name.

#### L1.7 diagnostic.schema.json `error_code` enum is missing 4 codes that the compiler actually emits (BTreeMapKeyTypeMustBeI64, BTreeMapValueMustBeNonLinear, RegionLiteralMutation, VerificationSkipped) — `HIGH` · ✅ survived cross-check
**finder:** `L1-diagnostics-agent` · **kind:** design-divergence · **verdicts:** 3 · reviewer severity votes: medium×3 · **Filed as #611**  
**Files:** `docs/spec/schemas/diagnostic.schema.json:11-34`, `vow-diag/src/lib.rs:36-80`, `vow/src/main.rs:8060`  
**Design ref:** §6.5 (stable error codes; structured diagnostics describe themselves to agents)  
**Evidence:** The wire `error_code` is produced by `format!("{:?}", d.code)` (main.rs:8060), so every `ErrorCode` Debug name can appear in `diagnostics[]`. The enum (vow-diag/src/lib.rs:36-80) defines `BTreeMapKeyTypeMustBeI64`, `BTreeMapValueMustBeNonLinear`, `RegionLiteralMutation`, and `VerificationSkipped`, and all four are emitted as real diagnostics: `code: ErrorCode::BTreeMapKeyTypeMustBeI64` (vow-types/src/check.rs:1080,1994), `BTreeMapValueMustBeNonLinear` (check.rs:2003), `RegionLiteralMutation` (vow-ir/src/region.rs:3119), `VerificationSkipped` (main.rs:8825). But diagnostic.schema.json's `error_code` enum (lines 11-34) lists none of them. All four ARE documented in errors.md (lines 180, 196, 390, 322), so the schema is the outlier. An agent that validates compiler diagnostics against the schema will reject valid output — e.g. a `Skipped` build always carries a `VerificationSkipped` warning whose `error_code` is not in the enum.

**Proposed fix:** Add `BTreeMapKeyTypeMustBeI64`, `BTreeMapValueMustBeNonLinear`, `RegionLiteralMutation`, and `VerificationSkipped` to the `error_code` enum in docs/spec/schemas/diagnostic.schema.json and the embedded copy in main.rs (~line 7360). Add a CI check (extend scripts/check_help_coverage.py) that asserts the schema enum is a superset of the `ErrorCode` Debug names to prevent future drift.

#### L1.8 diagnostic.schema.json forbids fields the compiler intentionally emits (`blame`, `hints`, `secondary`), via additionalProperties:false — `HIGH` · ✅ survived cross-check
**finder:** `L1-diagnostics-agent` · **kind:** design-divergence · **verdicts:** 3 · reviewer severity votes: medium×3 · **Filed as #612**  
**Files:** `docs/spec/schemas/diagnostic.schema.json:7-57`, `vow/src/main.rs:7892-7903`, `compiler/diag.vow:334-353`  
**Design ref:** §6.4 (blame information on contract failures), §6.5 (structured diagnostics)  
**Evidence:** diagnostic.schema.json declares only `error_code/message/severity/span` and sets `"additionalProperties": false` (line 57), with the inner `span` object also `additionalProperties:false` allowing only `file/offset/length` (line 54). But `DiagnosticJson` (main.rs:7892-7903) emits `hints`, `secondary`, and `blame` whenever non-empty, and the Rust test at main.rs:11942-11943 explicitly documents these as intended optional fields ("no extra fields surface beyond the optional `secondary`/`hints`/`blame`"). The verify path always sets blame/hints on contract-violation diagnostics (main.rs:8890-8901: `blame: blame_to_diag_blame(...)`, `hints`). The self-hosted compiler additionally injects `"line"` and `"column"` into the span object (diag.vow:327-330) and emits `"blame"`/`"hints"` (diag.vow:334-353) — none allowed by the schema. Every diagnostic that carries blame/hints (i.e. all verification failures) fails schema validation; the self-hosted output additionally fails on the span-level `line`/`column`.

**Proposed fix:** Add `blame` (enum: caller/callee), `hints` (array of string), and `secondary` (array of span objects) as optional properties in diagnostic.schema.json and the embedded copy. Decide whether `line`/`column` are part of the contract: either add them to the span sub-schema and emit them from the Rust compiler too, or remove them from compiler/diag.vow (diag.vow:322-331) so the two compilers agree.

#### L1.9 Self-hosted counterexample emits `source` as a bare string (file path); Rust compiler and schema emit `source` as object {file,offset,length} or null — cross-compiler divergence in CEGIS payload — `HIGH` · ✅ survived cross-check
**finder:** `L1-diagnostics-agent` · **kind:** design-divergence · **verdicts:** 3 · reviewer severity votes: high×2, medium×1 · **Filed as #613**  
**Files:** `compiler/diag.vow:452-453`, `compiler/main.vow:206-214`, `vow/src/main.rs:7941`, `docs/spec/schemas/counterexample.schema.json:27-42`  
**Design ref:** §6.4 (counterexamples as first-class repair inputs; fault localization), §2.4/§7 (self-hosting fixed point — outputs must agree)  
**Evidence:** Self-hosted `diag_ce_to_json` serializes source as a JSON string: `r.push_str(String::from("\",\"source\":\"")); r.push_str(diag_json_escape_str(ce.source));` (diag.vow:452-453), and `ce.source` is set to the file path String (`source: path,` at main.vow:212; `path: String` param at main.vow:185). The Rust compiler emits `source: Option<SpanJson>` i.e. an object `{file,offset,length}` or null (main.rs:7941), matching counterexample.schema.json's `source` `oneOf [object{file,offset,length}, null]` (schema lines 27-42). So the same logical field (`source`) has two incompatible types across the two compilers that are supposed to be a verified fixed point. An agent that reads `source.file`/`source.offset` from Rust-compiler output (per schema) gets a plain string from self-hosted output and cannot extract the clause location it needs to localize the repair.

**Proposed fix:** Make the self-hosted counterexample emit `source` as the schema object (or null): thread the vow clause span into `VerifyCE` and have `diag_ce_to_json` emit `"source":{"file":...,"offset":...,"length":...}`. Add a differential test (e.g. in the existing region_summary_equivalence-style harness) asserting Rust and self-hosted counterexample JSON are byte-identical for a known failing fixture.

#### L1.10 `.unwrap()` inside a vow clause is not flagged as impure in either compiler (Panic effect ignored in purity check) — `HIGH` · ✅ survived cross-check
**finder:** `L1-effects` · **kind:** soundness · **verdicts:** 3 · reviewer severity votes: medium×3 · **Filed as #601**  
**Files:** `vow-types/src/effects.rs:241-269`, `compiler/checker.vow:610-635`  
**Design ref:** §5.5 (vow clauses must be pure); grammar.md:648 (`panic` = 'May panic (unwrap, etc.)'), 671  
**Evidence:** In the Rust compiler `check_vow_purity` collects both `calls` and `panic_exprs` (effects.rs:241-243: `let mut panic_exprs = Vec::new(); collect_calls_in_expr(expr, &mut calls, &mut panic_exprs);`) but the loop that emits diagnostics only iterates `for (callee_expr, callee_name) in &calls` (effects.rs:245) — `panic_exprs` is never inspected. So a predicate like `ensures: opt.unwrap() == 5` passes purity checking even though `.unwrap()` carries the `[panic]` effect (grammar.md:591, 648) and is therefore impure — a panicking, partial expression inside a contract that must be a pure total predicate. The self-hosted compiler does not check clause purity at all (see related finding), so it is also missed there. This lets a partial/aborting expression masquerade as a pure contract predicate.

**Proposed fix:** In `check_vow_purity` (and the new self-hosted equivalent), after the `calls` loop, also iterate `panic_exprs` and emit an `EffectViolation` ('vow predicate must be pure but uses `.unwrap()` which may panic; use a total predicate such as `opt.is_some() && ...`'). Add a unit test mirroring `vow_purity_impure_predicate_emits_violation` but with `.unwrap()` in the clause.

#### L1.11 Loop-invariant vow clauses are never purity-checked (while / for-each / loop) — `HIGH` · ✅ survived cross-check
**finder:** `L1-effects` · **kind:** soundness · **verdicts:** 3 · reviewer severity votes: medium×2, high×1 · **Filed as #602**  
**Files:** `vow-types/src/check.rs:544-563`, `vow-types/src/check.rs:1349-1400`, `vow-types/src/effects.rs:223-226`  
**Design ref:** §5.1 (loop invariants are core contract mechanism), §5.5 (vow clauses must be pure); grammar.md:671  
**Evidence:** `check_vow_purity` is invoked only for the function-level vow, from `check_fn_effects` (effects.rs:223-225: `if let Some(vow_block) = &fn_def.vow { check_vow_purity(vow_block, env, file, emitter); }`). Loop invariants flow through a different path: `ExprKind::While { vow, .. } => { if let Some(vow) = vow { self.check_vow_clauses(vow, "while loop"); } }` (check.rs:1349-1356), likewise for ForEach (1388-1389) and Loop (1397-1399). `check_vow_clauses` (check.rs:544-563) only checks each clause is `bool` — it never calls `check_vow_purity`. So `while c { invariant: side_effecting_io() } { ... }` is accepted, and the impure invariant is lowered into a loop verification obligation. The self-hosted compiler likewise has no purity path for invariants.

**Proposed fix:** Have `check_vow_clauses` call `check_vow_purity` (or factor the per-clause purity logic into a shared helper) for every clause it processes, so while/for/loop invariants get the same purity enforcement as function-level vow. Mirror in compiler/checker.vow.

#### L1.12 Linear single-consumption is defeated by storing a linear struct in Vec<T> or HashMap<_,V>: obligation silently discharged on insert, then duplicated via copy-semantics indexing or lost on overwrite — `HIGH` · ✅ survived cross-check
**finder:** `L1-linear` · **kind:** soundness · **verdicts:** 3 · **Filed as #614**  
**Files:** `vow-types/src/check.rs:1982-2024`, `compiler/checker.vow:200-235`, `tests/run/linear_region_ok.vow:7-41`  
**Design ref:** §5.6 (linear values 'must be consumed exactly once'); §5.4 (Vec/HashMap intrinsics); grammar.md:571-574 / contracts.md (BTreeMap non-linear-value rule); §4.4 (local reasoning)  
**Evidence:** Only `BTreeMap` rejects linear values: check.rs:2000-2005 emits `BTreeMapValueMustBeNonLinear` and the self-hosted equivalent is checker.vow:226-230 — both gated specifically on BTreeMap. `Vec<T>` and `HashMap<K,V>` carry NO such guard (grep for VecElementMust/HashMapValueMust returns nothing). The intended-to-pass fixture `tests/run/linear_region_ok.vow` exercises exactly this: `let v: Vec<Handle> = Vec::new(); v.push(h);` (lines 9-10, 16-18, 49-50) and `v[0] = h;` (line 39) where `Handle` is `linear struct`. The region pass treats `v.push(h)` as consuming `h` (origin removed from live), so the obligation is discharged forever — but the handle now lives in a `Vec` whose slots are plain 8-byte pointers. Per grammar.md the same bitwise/duplication hazard the docs cite for BTreeMap applies: indexing 'uses copy semantics: `v[i]` copies the 8-byte slot value ... base container is not consumed', so `let h2 = v[0]` aliases/duplicates the linear handle undetected, and `v[0] = h` (line 39) overwrites the `Handle{fd:0}` pushed on line 38, silently leaking that obligation. errors.md/grammar.md:572-574 justify the BTreeMap rejection as 'runtime/verifier shift values bitwise and would silently duplicate a linear obligation' — identical reasoning applies to Vec and HashMap, which are unguarded.

**Proposed fix:** Either (a) reject `Vec`/`HashMap` (and any non-BTreeMap container) whose element/value type is or transitively contains a `linear struct`, reusing `is_linear_ty`, with a generalized error code (e.g. ContainerValueMustBeNonLinear), in BOTH compilers; or (b) if linear containers are intended, model container insert as transferring the obligation into the container and require the container itself to be linear and tracked. Update tests/run/linear_region_ok.vow (currently codifies the unsound pattern as expected-pass) and grammar.md accordingly.

#### L1.13 Borrow printer drops parentheses: `&(a + b)` becomes `&a + b`, changing the AST on reparse — `HIGH` · ✅ survived cross-check
**finder:** `L1-syntax-canonical` · **kind:** soundness · **verdicts:** 3 · reviewer severity votes: low×1, medium×2 · **Filed as #593**  
**Files:** `vow-syntax/src/printer.rs:689-689`, `vow-syntax/src/parser/expr.rs:237-247`  
**Design ref:** §6.5 (canonicalization must be meaning-preserving and diff-stable); spec/grammar.md Canonical Form  
**Evidence:** printer.rs:689 prints borrow with no precedence handling: `ExprKind::Borrow { expr } => format!("&{}", print_expr(expr))`. The parser (expr.rs:237-247) parses `&` as a prefix op at `PREFIX_BINDING_POWER = 19`, tighter than every binary op (max 18). So `Borrow(BinaryOp(a,+,b))` prints as `&a + b`, which reparses as `BinaryOp(Borrow(a), +, b)`. VERIFIED dynamically: top-level AST kind changes from `Borrow` to `BinaryOp` across print→reparse for `&(a + b)`. The printed text is self-stable (idempotent) but the *program meaning* changed during the first print, violating semantic idempotence.

**Proposed fix:** In the Borrow arm, parenthesize the operand when it is a BinaryOp (and other lower-than-prefix-precedence forms), mirroring the Cast arm's logic at printer.rs:691-704. Add roundtrip tests covering Borrow/Question of binary ops.

#### L1.14 Question (`?`) printer drops parentheses: `(a + b)?` becomes `a + b?`, changing the AST on reparse — `HIGH` · ✅ survived cross-check
**finder:** `L1-syntax-canonical` · **kind:** soundness · **verdicts:** 3 · reviewer severity votes: high×1, medium×2 · **Filed as #594**  
**Files:** `vow-syntax/src/printer.rs:690-690`, `vow-syntax/src/parser/expr.rs:83-96`  
**Design ref:** §6.5 (meaning-preserving canonicalization); spec/grammar.md Canonical Form  
**Evidence:** printer.rs:690: `ExprKind::Question { expr } => format!("{}?", print_expr(expr))` with no precedence guard. `?` is parsed as a postfix operator (expr.rs:83-96 handles `TokenKind::Question` in the postfix loop, binding tighter than binary ops). So `Question(BinaryOp(a,+,b))` prints `a + b?`, which reparses as `BinaryOp(a, +, Question(b))`. VERIFIED dynamically: top-level AST kind changes from `Question` to `BinaryOp` across print→reparse for `(a + b)?`. Meaning is silently altered by the canonicalizer.

**Proposed fix:** In the Question arm, parenthesize the inner expression when it is a BinaryOp/UnaryOp/Assign etc. (anything binding looser than postfix `?`). Reuse the precedence-aware helper already present for Cast.

#### L1.15 Refinement-type predicate `{ x: T || pred }` is silently erased with no diagnostic (parsed, type-checked as base, never lowered to a verification obligation) — `HIGH` · ✅ survived cross-check
**finder:** `L1-transition-debt` · **kind:** soundness · **verdicts:** 3 · reviewer severity votes: low×1, high×2 · **Filed as #583**  
**Files:** `vow-types/src/env.rs:903-903`, `vow-syntax/src/parser/types.rs:107-124`, `vow-ir/src/lower/vow.rs:221-246`  
**Design ref:** §5.3 (Refinement properties live above the base type system; status: 'Refinement type syntax parsed but not fully forwarded to verification: Partial'); §8 ('Refinement type predicates in verification | Partial'); §6.4 (structured diagnostics are part of the contract)  
**Evidence:** The parser accepts a refinement type in any type position (`parse_type_inner` -> `LBrace => self.parse_refinement_type(start)`), producing `Type::Refinement { binding, base, predicate, .. }`. But the type resolver throws the predicate away: `vow-types/src/env.rs:903`:

    AstType::Refinement { base, .. } => self.resolve(base),

The IR lowering path that turns parameter refinements into verification obligations (`lower_param_refinements`) only reads `param.refinement` — the `where`-clause expression — not the parameter *type* being a `Type::Refinement`:

    pub(crate) fn lower_param_refinements(ctx: &mut LowerCtx, params: &[Param]) {
        for param in params {
            if let Some(ref refinement) = param.refinement { ... emit VowRequires ... }
        }
    }

A grep across vow-ir, vow-verify and vow-types/src/check.rs for `Type::Refinement` / `.predicate` returns nothing — the predicate is never consumed downstream. Critically, unlike traits/impls (which `check.rs:526-539` rejects with an explicit `UnsupportedFeature` error), a refinement type produces NO diagnostic at the use site: the function type-checks and compiles cleanly as if the predicate were verified. An agent writing `fn f(x: { v: i64 || v > 0 }) -> i64` will believe `v > 0` is a checked precondition; it is silently dropped, so verification reports SUCCESS while proving nothing about the predicate.

**Proposed fix:** Until refinement-type predicates are forwarded to verification, make the erasure non-silent: in the type checker, when a parameter/return/let type resolves through `AstType::Refinement`, either (a) lower the predicate into a `requires`/`ensures`-equivalent obligation the same way `where` clauses are (preferred — `where` already proves the mechanism exists), or (b) emit a hard `UnsupportedFeature` diagnostic at the refinement-type span (mirroring the trait/impl rejection at check.rs:526) so an agent is never misled into thinking the predicate is verified. The current middle ground — accept, erase, report success — is the worst option for an agent-first verified language.

#### L1.16 `where`-clause refinement expression is never type-checked, yet is lowered into a verifier `__ESBMC_assume`; non-bool / sibling-referencing clauses are silently accepted (both compilers) — `HIGH` · ✅ survived cross-check
**finder:** `L1-transition-debt` · **kind:** soundness · **verdicts:** 3 · **Filed as #589**  
**Files:** `vow-types/src/check.rs:565-639`, `compiler/checker.vow:637-690`, `vow-ir/src/lower/vow.rs:221-244`, `vow-verify/src/c_emitter.rs:1012-1015`  
**Design ref:** §5.3 ('Parameter `where` clauses are treated as sugar for `requires`'; status Implemented); grammar.md:98 / contracts.md:198-211 (where becomes additional `requires`; can only reference its own parameter). Divergence: the sugar is not held to the type-discipline of what it desugars to.  
**Evidence:** `check_fn` (check.rs:565-639) defines params then type-checks `vow` `requires`/`ensures` clauses (lines 587-637), but never visits `param.refinement` (the `where` expr). Grepping check.rs for `refinement` only finds `refinement: None` constructions. The self-hosted `check_fn` (checker.vow:637-690) is identical: it reads param slots `i*3` (name) and `i*3+1` (type) but never slot `i*3+2` (the where expr). Yet lowering DOES turn the where expr into a verification obligation: `lower_param_refinements` (lower/vow.rs:224-243) builds `VowClause::Requires { expr: (**refinement).clone() }` and emits `Opcode::VowRequires`, which the C emitter renders as `__ESBMC_assume(v{pred})` (c_emitter.rs:1014). I proved silent acceptance with a probe test: `fn f(a: i64, b: i64 where a + 1) -> i64 { a }` type-checks with NO error (checker.has_errors() == false). This where clause (1) has type i64, not bool — a real `requires: a + 1` is rejected with ContractTypeMismatch (check.rs:591-598) — and (2) references sibling param `a`, which grammar.md:98 explicitly forbids ('Each `where` clause can only reference its own parameter'). The malformed clause is silently emitted as `__ESBMC_assume(a + 1)`, i.e. the verifier silently assumes `a != -1`, a wrong precondition that was never validated.

**Proposed fix:** In both `check_fn` (Rust check.rs and self-hosted checker.vow), type-check `param.refinement` exactly like a `requires` clause: resolve it in a scope where only the owning parameter is bound (to enforce the spec's self-reference-only rule and reject sibling/undefined references), and require its type to be `bool` (emit ContractTypeMismatch otherwise). This closes the gap where a malformed `where` clause becomes an unchecked, possibly-wrong ESBMC assumption.

#### L1.17 grammar.md classifies / and % as plain "wrapping" with no trap; design §5.7 says they trap on zero divisor — `MEDIUM` · ✅ survived cross-check
**finder:** `L1-arith` · **kind:** design-divergence · **verdicts:** 1 · reviewer severity votes: low×1 · **Filed as #651**  
**Files:** `docs/spec/grammar.md:186-196`, `vow-codegen/src/cranelift_backend.rs:875-889`  
**Design ref:** §5.7 ("/ and % trap on zero divisor"; "no undefined behavior") vs grammar.md §Operators  
**Evidence:** grammar.md §Operators lists `/` and `%` under "Wrapping Arithmetic (default)": `| / | Div (wrapping) |`, `| % | Rem (wrapping) |`, and states "Wrapping operators silently wrap on overflow" (grammar.md:186-196) with no mention of zero-divisor trapping. The authoritative design §5.7 says "`/` and `%` trap on zero divisor" and "There is no undefined behavior". Codegen emits a bare `sdiv`/`srem` with no controlled check (cranelift_backend.rs:875-889), relying on the implicit Cranelift int_divz hardware trap (SIGFPE) — which technically traps, but the spec text an agent reads (grammar.md) tells it `/` merely "wraps", giving no signal that `x / 0` aborts. For Vow's agent-first goal, an agent reading grammar.md cannot locally infer that `/` aborts on zero, and may write `a / b` expecting a defined wrapping result instead of reaching for a guarded `b != 0` precondition or `/!`. This is exactly the "reach for the wrong operator" surface gap in the lane focus.

**Proposed fix:** Update grammar.md to state that `/` and `%` trap (abort) on a zero divisor (and on INT_MIN/-1 if codegen is hardened), distinguishing them from the always-defined wrapping +,-,*. Clarify whether the trap is a controlled VowViolation/abort or a raw hardware trap, and align the codegen accordingly (a controlled __vow_violation would match Vow's structured-diagnostics principle better than a bare SIGFPE).

#### L1.18 cli.md agent decision tree tells agents to read `inputs` for violating values, but the field is named `values` — documented CEGIS repair path points at a non-existent field — `MEDIUM` · ✅ survived cross-check
**finder:** `L1-diagnostics-agent` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #648**  
**Files:** `docs/spec/cli.md:286-296`, `docs/spec/cli.md:486-490`  
**Design ref:** §6.4 (CEGIS workflow: agent repairs code against the counterexample), §2.3/§4.5 (tool behavior is part of the programming model)  
**Evidence:** cli.md's VerifyFailed example shows the counterexample with `"inputs": { "a": "...", "b": "0" }` (cli.md:286) and the Agent Decision Tree instructs: `status == "VerifyFailed" -> Read counterexamples[]` then `├── Check inputs for the violating values` (cli.md:487). But the actual emitted field is `values` (proven by main.rs:11999 `ces[0]["values"]["y"]` and diag.vow:432 `"values":{...}`). An agent that follows the canonical CLI spec to drive its CEGIS loop will look up `counterexamples[0].inputs`, find it absent, and lose the violating argument values — the single most important input to the repair step.

**Proposed fix:** Resolve in tandem with the counterexample-schema finding: pick one canonical field name (`values` or `inputs`) and make cli.md (lines 286, 487), the schema, and both emitters agree. Given both compilers already emit `values`, the lowest-churn fix is to change cli.md and the schema to `values`.

#### L1.19 Inconsistent `blame` string casing across output surfaces: build/verify diagnostics+counterexamples use lowercase ("caller"), but `vow contracts` and runtime VowViolation use PascalCase ("Caller") — `MEDIUM` · ✅ survived cross-check
**finder:** `L1-diagnostics-agent` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #649**  
**Files:** `vow/src/main.rs:8046-8048`, `vow/src/main.rs:9929-9933`, `docs/spec/cli.md:327-364`  
**Design ref:** §6.5 (stable error codes and blame categories), §4.1 (single canonical way)  
**Evidence:** Diagnostic JSON blame is lowercase: `Blame::Caller => Some("caller".to_string())` (main.rs:8046-8048); counterexample blame is lowercase too (`Blame::Caller => "caller"`, main.rs:8274, asserted at main.rs:11994 `ces[0]["blame"] == "caller"`). But the `vow contracts` output uses PascalCase: `Blame::Caller => "Caller"` (main.rs:9929-9931), documented in cli.md:327/363 as `"blame": "Caller"`. The runtime VowViolation is also PascalCase (`"blame":"Caller"`, vow-violation.schema.json enum `["Caller","Callee"]`). So an agent consuming blame must branch on casing depending on which JSON surface it parsed (build diagnostics vs. contracts vs. runtime), defeating the §6.5 goal of stable, uniform blame categories and forcing brittle, surface-specific normalization.

**Proposed fix:** Pick one canonical casing for `blame` across all JSON surfaces. The runtime VowViolation schema fixes PascalCase as a public contract, so the simplest convergence is to make build/verify diagnostics and counterexamples emit `"Caller"`/`"Callee"` (change main.rs:8047/8048 and 8274/8275), or document an explicit per-surface casing convention in cli.md if divergence is intentional. Update schema enums to match.

#### L1.20 Both compilers make `io` subsume `read`/`write`, contradicting the documented effect-independence rule — `MEDIUM` · ✅ survived cross-check
**finder:** `L1-effects` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #635**  
**Files:** `vow-types/src/effects.rs:6-14`, `compiler/env.vow:58-87`  
**Design ref:** §5.5 ('Each effect is independent — io is not a superset of read or write'); grammar.md:651  
**Evidence:** Rust `effect_covered` (effects.rs:6-14): `if (needed == &Effect::Read || needed == &Effect::Write) && declared.contains(&Effect::IO) { return true; }` — declaring `[io]` silently satisfies a `[read]` or `[write]` requirement. Self-hosted `effect_covered` (env.vow:77, 83) does the same: `if has_eff_bit(needed, eff_read) && !io_decl && !has_eff_bit(declared, eff_read) { return false; }` and the symmetric `eff_write` line — both short-circuit on `io_decl`. This directly contradicts the design doc §5.5: 'Each effect is independent — `io` is not a superset of `read` or `write`,' restated verbatim in grammar.md:651. The behaviour is even locked in by Rust tests `io_subsumes_read` / `io_subsumes_write` (effects.rs:377-400). Consequence: a function declared `[io]` can perform filesystem reads/writes without admitting `[read]`/`[write]`, so the effect signature understates the function's capability surface — exactly the agent-facing precision §2.2 relies on.

**Proposed fix:** Either (a) remove the io->read/write subsumption from both `effect_covered` implementations and update the two Rust tests, restoring true independence per the spec; or (b) if subsumption is genuinely intended, change the design doc §5.5 and grammar.md:651 to document the hierarchy. Given §4.2 (explicit semantics) and the doc's emphasis, (a) is the aligned choice. Fix must land in both compilers in the same change per CLAUDE.md.

#### L1.21 Self-hosted effect propagation skips `break <expr>` values, missing effectful calls inside break — `MEDIUM` · ✅ survived cross-check
**finder:** `L1-effects` · **kind:** bug · **verdicts:** 1 · **Filed as #636**  
**Files:** `compiler/checker.vow:1616-1776`, `compiler/parser.vow:834-841`, `vow-types/src/effects.rs:101-105`  
**Design ref:** N/A — pure impl bug (self-hosted diverges from Rust reference; §5.5 effect propagation correctness)  
**Evidence:** The self-hosted `collect_calls_in_expr` (checker.vow:1616-1776) enumerates expr kinds explicitly (CALL, METHOD, BINOP, UNOP, FIELD, QUESTION, INDEX, IF, WHILE, LOOP, FOR, MATCH, RETURN, BLOCK, ASSIGN, SLIT, ECTOR, CAST, TUPLE) but has NO arm for `EXPR_BREAK` (kind 15, ast.vow:18). The parser stores the broken value's expr id in `expr_a`: `arena_add_expr(p.arena, EXPR_BREAK(), val, 0, 0, 0)` (parser.vow:840). Therefore in `loop { break effectful_io_call() }` the call inside the break value is never collected, and `check_effects_fn` never sees it — effect propagation silently misses it, so a pure function can perform an effect via a break value with no diagnostic. The Rust compiler handles this correctly: `ExprKind::Break { value } => { if let Some(v) = value { collect_calls_in_expr(v, ...) } }` (effects.rs:101-105). Loop bodies themselves are traversed (EXPR_LOOP arm at checker.vow:1695), so the break node IS reachable — only its child is dropped.

**Proposed fix:** Add an `EXPR_BREAK` arm to the self-hosted `collect_calls_in_expr`: `if tag == EXPR_BREAK() { let val_eid: i64 = expr_a(a, eid); if val_eid != -1 { collect_calls_in_expr(e, m, val_eid, calls); } return; }` — mirroring the existing `EXPR_RETURN` arm (checker.vow:1719-1725). Add a tests/run/ case with an effectful call in a break value inside a pure function and assert the EffectViolation.

#### L1.22 Rust linear.rs rejects sound loop-local create-and-consume (in_loop false positive), diverging from the self-hosted compiler which accepts it — `MEDIUM` · ✅ survived cross-check
**finder:** `L1-linear` · **kind:** bug · **verdicts:** 1 · **Filed as #650**  
**Files:** `vow-types/src/linear.rs:197-218`, `vow-types/src/linear.rs:301-318`  
**Design ref:** §2.4 (self-hosting/parity as sufficiency test); §2.2 (agents must predict compiler behavior — divergence defeats this); §5.6 (resource discipline)  
**Evidence:** In `check_expr`, the While/ForEach/Loop arms set `tracker.in_loop = true` for the whole body. `consume_var` then unconditionally flags ANY consumption of an Available linear value while `in_loop`: `Some(ConsumeState::Available(_)) => { if tracker.in_loop { emit_violation(... "linear value `{name}` cannot be consumed inside a loop (would be consumed multiple times)" ...) } }`. But a linear value DECLARED inside the loop body via `let h: Handle = open();` is registered as Available by `register_pattern_linear` (linear.rs:127-142, reached through check_block->check_stmt), and consuming it in the same iteration (`close(h)`) is sound — created once, consumed once per iteration. The check has no notion of 'declared within this loop scope', so it false-positives on the canonical per-iteration acquire/use/release pattern (e.g. open a file handle each iteration and close it). The self-hosted region pass accepts this exact program (a loop-local origin removed in the same body never reaches region close live), so the two compilers DISAGREE: Rust rejects a valid program the self-hosted compiler compiles, blocking an idiomatic agent resource pattern and breaking dual-compiler parity.

**Proposed fix:** Track the loop nesting depth at which each linear variable was registered (e.g. store a `loop_depth` in ConsumeState::Available). In consume_var, only emit the in-loop error when the variable was registered at a SHALLOWER loop depth than the current consumption site (i.e. it is an outer value consumed inside the loop). Values declared and consumed within the same loop iteration are sound and must be accepted, matching the self-hosted region pass. Add a positive test for loop-local acquire/consume.

#### L1.23 Nested control-flow blocks render at absolute indentation level 0 (broken canonical indentation) — `MEDIUM` · ✅ survived cross-check
**finder:** `L1-syntax-canonical` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #591**  
**Files:** `vow-syntax/src/printer.rs:553-688`, `vow-syntax/src/printer.rs:226-232`  
**Design ref:** §6.5 (deterministic, diff-stable code formatting via canonicalization); spec/grammar.md Canonical Form ('indentation uses 4 spaces')  
**Evidence:** `print_expr` is context-free: every control-flow expression hard-codes level 0 when printing its block, e.g. If (printer.rs:569) `print_block(then_branch, 0)`, While (615) `print_block(body, 0)`, Loop (676), ForEach (648), Match (553-559) uses `print_match_arm(arm, 1)`. `print_block` (226-232) then prints the body at `level+1` and the closing brace at the passed level. Because the passed level is always 0 regardless of nesting depth, a function-body `if` produces structurally wrong indentation. VERIFIED dynamically, canonical output for `fn f() -> i64 { if a>0 {1} else if a<0 {2} else {3} }` is:
```
fn f() -> i64 {
    if a > 0 {
    1
} else if a < 0 {
    2
} else {
    3
}
}
```
The `1`/`2`/`3` are at 4 spaces (should be 8) and the inner closing braces sit at column 0. Output is textually idempotent (so the proptest passes) but directly violates the documented 'indentation uses 4 spaces' canonical form and the diff-stability goal.

**Proposed fix:** Thread the current indentation `level` into `print_expr` (or a level-aware sibling) so control-flow blocks and match arms render relative to their nesting depth, instead of always using 0. Update the corresponding self-hosted source printer if/when one is added.

#### L1.24 Self-hosted parser accepts `;`, `,`, or nothing between vow clauses; Rust parser accepts only `,`/nothing — compiler divergence and multiple idioms — `MEDIUM` · ✅ survived cross-check
**finder:** `L1-syntax-canonical` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #626**  
**Files:** `compiler/parser.vow:443-448`, `vow-syntax/src/parser/mod.rs:479-494`  
**Design ref:** §4.1 (single canonical way), §2.4/§7 (self-hosting equivalence); spec/grammar.md ('Multiple clauses are separated by commas')  
**Evidence:** Self-hosted `parse_vow_block` (parser.vow:443-448) consumes a trailing semicolon AND a trailing comma between clauses:
```
if at(p, tok_semicolon()) { let _sc: Token = advance(p); }
if at(p, tok_comma()) { let _cm: Token = advance(p); }
```
The Rust `parse_vow_block` (mod.rs:492-494) only consumes a comma; a semicolon falls into the `_ =>` error arm (mod.rs:479-490) `"expected requires, ensures, or invariant, found {:?}"`. VERIFIED dynamically: the Rust parser rejects `requires: x > 0;` with `["expected requires, ensures, or invariant, found Semicolon", "expected RBrace, found Semicolon", ...]`, while the self-hosted parser accepts it. A program valid under build/vowc would be rejected by the Rust bootstrap compiler, and both compilers admit three equivalent separator idioms (`;`, `,`, whitespace) where grammar.md documents only the comma. This undermines §4.1 single-canonical-way and the self-hosting equivalence story.

**Proposed fix:** Pick one separator policy and apply it to BOTH compilers. Recommended: drop the semicolon acceptance in compiler/parser.vow:443-445 so both compilers accept only comma (and the canonical printer's no-separator output), matching grammar.md. Add a parity test that a semicolon-separated vow block is rejected by both.

#### L1.25 Round-trip proptest generator omits Borrow/Question/Cast and nested-unary nodes, so printer parenthesization bugs go undetected — `MEDIUM` · ✅ survived cross-check
**finder:** `L1-syntax-canonical` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #628**  
**Files:** `vow-syntax/tests/proptest_arb.rs:82-172`, `vow-syntax/tests/proptest_roundtrip.rs:329-364`  
**Design ref:** §6.5 (canonicalization correctness is part of the tool contract); §2.4 (self-hosting validation relies on these guarantees)  
**Evidence:** `arb_expr_inner` (proptest_arb.rs:88-94) only generates leaf, BinaryOp, UnaryOp, and Call:
```
prop_oneof![ 5 => arb_expr_leaf(), 2 => arb_binop_expr(depth-1), 1 => arb_unop_expr(depth-1), 1 => arb_call_expr(depth-1) ]
```
and `arb_unop_expr` (160-167) only applies unary ops to LEAF operands (`arb_expr_leaf()`), never to binary or other unary ops. No `Borrow`, `Question`, or `Cast` node is ever generated. The roundtrip property (proptest_roundtrip.rs:329-364) checks text idempotence + AST equality, but cannot exercise the printer arms that are actually broken (Borrow drops parens, Question drops parens, Neg(Not(x)) emits unparseable `-!x`). This is why those soundness bugs pass CI. Related to (but distinct from) #427/#429 which concern the vow-types typecheck proptest diagnostic vacuity.

**Proposed fix:** Extend arb_expr to generate Borrow, Question, and Cast nodes, and allow arb_unop_expr/arb_binop operands to be arbitrary subexpressions (parenthesizing as needed in the generator's intent), then ensure the roundtrip property asserts span-stripped AST equality (not just text idempotence). These additions would have caught all four printer findings above.

#### L1.26 Canonical printer emits refinement type with single `|` but parser requires `||`, breaking the documented `parse -> print -> parse` idempotency — `MEDIUM` · ✅ survived cross-check
**finder:** `L1-transition-debt` · **kind:** bug · **verdicts:** 1 · **Filed as #620**  
**Files:** `vow-syntax/src/printer.rs:407-419`, `vow-syntax/src/parser/types.rs:113-117`  
**Design ref:** §4.1 / §5.2 (single canonical form; compiler enforces canonical source form; parse->print->parse idempotency is a stated guarantee). N/A — pure impl bug against that guarantee.  
**Evidence:** Printer (printer.rs:413-418) emits a single pipe:

    format!("{{ {}: {} | {} }}", binding, print_type(base), print_expr(predicate))

Parser (parser/types.rs:114) requires the double-pipe token:

    self.expect(TokenKind::PipePipe);

The lexer maps `|`->`Pipe` and `||`->`PipePipe` (token.rs:92,100; lexer.rs:179,182), so they are distinct tokens. I proved the round-trip break with a probe test: parsing `{ x: i64 || x > 0 }`, printing it (yields `{ x: i64 | x > 0 }`), then re-parsing produced diagnostics `["expected PipePipe, found Pipe", "expected expression, got Pipe"]`. The unit test `printer::tests::test_type_refinement` even hard-codes the broken output: `assert_eq!(print_type(&ty), "{ x: i64 | x > 0 }")`. The proptest round-trip generator (vow-syntax/tests/proptest_arb.rs) never produces `Type::Refinement` (only `refinement: None`), so the idempotency property is never exercised for this form and the regression is uncaught. The embedded skill/--help text states this is an invariant (vow/src/main.rs:2256): 'The canonical printer normalizes source: parse -> print -> parse is idempotent.'

**Proposed fix:** Pick one operator and make printer and parser agree. Since `||` is logical-or elsewhere and `|` reads as the refinement separator, prefer the parser accepting `|` (single Pipe) and keep the printer's `|`, OR change the printer to emit `||` to match the current parser. Then add a proptest arbitrary for `Type::Refinement` so the round-trip property covers it, and fix `test_type_refinement` to assert the chosen canonical form.

#### L1.27 Wrapping / and % use uncontrolled hardware traps on zero divisor, bypassing the structured violation path (no diagnostic, no debug/release symmetry) — `MEDIUM` · ❌ refuted by cross-check
**finder:** `L1-arith` · **kind:** diagnostics-quality · **verdicts:** 1  
**Files:** `vow-codegen/src/cranelift_backend.rs:875-889`, `vow-clif-shim/src/lib.rs:1855-1862`  
**Design ref:** §4.5 (structured diagnostics part of the language contract); §5.7 (no UB); §4.2 (one semantic interpretation)  
**Evidence:** WrappingDivI64/WrappingRemI64 emit `builder.ins().sdiv(arg!(0), arg!(1))` / `srem(...)` with no zero guard (cranelift_backend.rs:875-889; self-hosted shim 1855-1862). On `x / 0` this produces a raw CPU divide fault (SIGFPE) at runtime regardless of build mode, with none of Vow's structured output: no `__vow_violation` JSON, no blame, no description, no `values` capture. Design §4.5 makes "structured diagnostics ... counterexample reporting ... part of the intended agent workflow" and §4.2 demands one clear semantic interpretation. An agent that triggers `x / 0` gets an opaque OS-level crash it cannot parse, contradicting the agent-first, structured-output identity. Note checked div-by-zero on `/!` DOES go through emit_overflow_check, but that helper traps with `TrapCode::INTEGER_OVERFLOW` (cranelift_backend.rs:1693) — mislabeling a division-by-zero as an integer overflow.

**Proposed fix:** Route zero-divisor (and INT_MIN/-1) aborts for both wrapping and checked div/rem through the structured __vow_violation / __vow_arithmetic_overflow handler with a distinct DivisionByZero reason rather than reusing INTEGER_OVERFLOW, so the failure surfaces as parseable JSON with blame. At minimum, give zero-division its own TrapCode/reason distinct from the overflow label, and document the runtime abort shape in cli.md/errors.md.

#### L1.28 Checked-arithmetic runtime abort handler (__vow_arithmetic_overflow) is only declared in debug mode; release-mode checked ops degrade to bare traps — `MEDIUM` · ❌ refuted by cross-check
**finder:** `L1-arith` · **kind:** design-divergence · **verdicts:** 1  
**Files:** `vow-codegen/src/cranelift_backend.rs:1690-1693`, `vow-codegen/src/cranelift_backend.rs:2947-2963`  
**Design ref:** §5.7 (checked ops abort with ArithmeticOverflow — unconditional, not a verifiable-contract check that release may drop)  
**Evidence:** `__vow_arithmetic_overflow` is only imported when `mode.has_debug_checks()` is true: `let (vow_violation_id, overflow_id) = if mode.has_debug_checks() { ... declare_function("__vow_arithmetic_overflow", Linkage::Import, ...) ... }` (cranelift_backend.rs:2947-2963). emit_overflow_check then does `if let Some(overflow_ref) = ctx.overflow_ref { builder.ins().call(overflow_ref, &[]); } builder.ins().trap(TrapCode::INTEGER_OVERFLOW);` (1690-1693) — so in release mode overflow_ref is None and a checked-op overflow emits only a bare trap with no handler call, no message. Design §5.7 states checked ops "abort with ArithmeticOverflow" with no mode qualifier — abort-on-overflow is a semantic property of `+!`/`-!`/`*!`/`/!`/`%!`, not a debug-only check like the runtime vow-contract checks that release legitimately omits. An agent compiling release gets a bare SIGILL/trap with no ArithmeticOverflow diagnostic, contradicting the named-abort guarantee.

**Proposed fix:** Declare and call the arithmetic-overflow handler in all modes (it is a hard semantic abort, distinct from optional debug vow-contract checks), or at minimum emit a structured abort/message in release too. Keep the trap as the hard stop but ensure the ArithmeticOverflow reason is reported in every build mode so the named-abort guarantee in §5.7 holds.

#### L1.29 errors.md error catalog omits `IoError`, leaving a documented-vs-emittable gap in the agent-facing error reference — `LOW` · ✅ survived cross-check
**finder:** `L1-diagnostics-agent` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #682**  
**Files:** `docs/spec/errors.md:9-448`, `vow-diag/src/lib.rs:67-68`, `docs/spec/schemas/diagnostic.schema.json:30`  
**Design ref:** §6.5 (the language and tools should be understandable as an operational skill surface), §4.5  
**Evidence:** `IoError` is a real `ErrorCode` variant (vow-diag/src/lib.rs:67-68) and is listed in diagnostic.schema.json's enum (line 30), so it can appear as a wire `error_code`. But errors.md (whose section headers, enumerated by grep, run UnterminatedString...LoweringWarning) has no `### IoError` entry. errors.md opens (line 3) with "This document lists all error codes, their phase, meaning, an example trigger, and how to fix them" — making the omission a coverage gap. An agent that receives `error_code: …[truncated]

**Proposed fix:** Add an `### IoError` section to docs/spec/errors.md documenting its phase (driver/IO), meaning (file read/write or process I/O failure during compilation), and fix. Consider a CI assertion that every `ErrorCode` Debug name has a matching `### <name>` header in errors.md to prevent catalog drift.

#### L1.30 consume_var does not transition MaybeConsumed state on use, producing duplicate diagnostics for a single repaired site — `LOW` · ✅ survived cross-check
**finder:** `L1-linear` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #684**  
**Files:** `vow-types/src/linear.rs:289-300`  
**Design ref:** §6.4 (structured diagnostics for CEGIS repair); §4.5 (tooling is part of the language contract)  
**Evidence:** In `consume_var`, the `Some(ConsumeState::MaybeConsumed(_))` arm emits 'linear value `{name}` may already be consumed' but does NOT update `tracker.vars` — the variable stays `MaybeConsumed`. (Contrast the `Available` arm at lines 301-317 which transitions to `Consumed`.) Consequently a `MaybeConsumed` value used N more times produces N identical diagnostics for what is logically one ownership defect, and never settles into a definite state. For the agent-repair workflow (§6.4) this multiplies c …[truncated]

**Proposed fix:** After emitting the MaybeConsumed diagnostic, transition the variable to `Consumed(span)` (or a terminal MaybeConsumed-already-reported state) so subsequent uses do not re-emit. This yields one diagnostic per ownership defect, improving repair legibility without weakening soundness.

#### L1.31 Integer-suffix handling: non-u64 suffixes silently dropped; `100u64` rewritten to `100 as u64` (two idioms collapse, info loss) — `LOW` · ✅ survived cross-check
**finder:** `L1-syntax-canonical` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #627**  
**Files:** `vow-syntax/src/parser/expr.rs:155-178`, `vow-syntax/src/ast.rs:335-341`  
**Design ref:** §4.1 (single canonical way), §4.2 (no implicit conversions / one semantic interpretation); spec/grammar.md Integer Literals (only `42u64` documented)  
**Evidence:** `Lit` (ast.rs:336-341) carries no suffix field. In expr.rs:155-178 the parser specially handles only `u64`:
```
TokenKind::LitIntSuffixed { value, suffix } => {
    if suffix == IntSuffix::U64 { /* build Cast(Lit::Int(value), u64) */ }
    else { Expr { kind: ExprKind::Lit(Lit::Int(value)), .. } }  // suffix discarded
}
```
VERIFIED dynamically: `42i32` canonicalizes to `42` (suffix lost — becomes a default-i64 literal), `5u8` → `5`, while `100u64` canonicalizes to `100 as u64`. So source `100u6 …[truncated]

**Proposed fix:** Either (a) make the lexer/parser reject suffixes other than the documented `u64` so unsupported forms surface a diagnostic instead of silently retyping, or (b) add a `suffix` field to `Lit::Int` and have the printer emit it canonically. Decide a single canonical literal form (`100u64` vs `100 as u64`) and document it; align both compilers.

#### L1.32 Refinement-type surface form is accepted by the Rust bootstrap compiler but unparsable by the self-hosted compiler (cross-compiler surface divergence) — `LOW` · ✅ survived cross-check
**finder:** `L1-transition-debt` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #662**  
**Files:** `vow-syntax/src/parser/types.rs:33-34`, `compiler/parser.vow:454-506`  
**Design ref:** §2.4 / §7 (self-hosting as sufficiency test; same surface in both compilers); §5.3 (refinement syntax is Partial). CLAUDE.md: 'always modify BOTH the Rust compiler and the self-hosted compiler.'  
**Evidence:** The Rust parser handles the brace refinement form in `parse_type_inner` (types.rs:34): `TokenKind::LBrace => self.parse_refinement_type(start)`. The self-hosted `parse_type` (parser.vow:454-506) has branches for `(`, `!`, `&`, `[`, and ident/generic, but NO `{` (LBrace) branch — a refinement type in parameter position falls through to `expect_ident(p)` at parser.vow:491 and produces a parse error. So the identical source (`fn f(x: { v: i64 || v > 0 }) -> i64 { ... }`) is accepted (predicate sile …[truncated]

**Proposed fix:** Bring the two compilers into agreement on this Partial feature. The cleanest path, pending full verification forwarding, is to reject the refinement-type form uniformly: either teach the self-hosted `parse_type` to parse it and have both compilers emit a single `UnsupportedFeature`/erasure diagnostic (see finding 1), or remove the brace-refinement branch from the Rust parser so neither compiler accepts an undocumented form (the brace syntax is not documented in docs/spec/grammar.md — only `where` clauses are).

---

## Lane 2 — Verification Pipeline

This is the highest-stakes lane: every finding here is about whether a `Verified` result can be trusted. **Clusters:** *u64-pinned-to-0* = L2-lowering-self ≡ L2-soundness-skip (one fix in `compiler/verifier.vow`); *refinement-dropped* = L2-refinement-fwd (+ the L1 transition-debt view); *checked-arith-as-wrapping* shared with Lane 1.

_32 findings — 4C / 7H / 10M / 11L._

#### L2.1 Checked-arithmetic overflow abort (`+!`,`-!`,`*!`,`/!`,`%!`) is never modeled in the ESBMC verification model — a verified function can still abort at runtime — `CRITICAL` · ✅ survived cross-check
**finder:** `L2-builtin-effects` · **kind:** soundness · **verdicts:** 3 · reviewer severity votes: critical×1, high×1, low×1 · **Filed as #585**  
**Files:** `vow-verify/src/c_emitter.rs:864-908`, `compiler/c_emitter.vow:975-994`, `vow-verify/src/esbmc.rs:419-426`, `vow-codegen/src/cranelift_backend.rs:893-953`  
**Design ref:** §5.7 (checked operators abort on overflow); §5.5 + §8 "Builtin panic/unsafe effect coverage: Partial"; §2.1 (verification is the primary trust mechanism)  
**Evidence:** The Cranelift codegen backend emits a genuine abort path for checked arithmetic (cranelift_backend.rs:895-897 `let (result, overflow) = builder.ins().sadd_overflow(...); emit_overflow_check(builder, overflow, ctx)?;`), matching spec §5.7 / grammar.md:208 "Checked operators abort with `ArithmeticOverflow` on overflow." But the verification C emitter collapses CHECKED and WRAPPING ops to the same plain C operator with NO overflow assertion. Rust c_emitter.rs:864-872: `Opcode::WrappingAddI64 | Opcode::CheckedAddI64 | ... => { ... out.push_str(&format!("  v{} = v{} + v{};\n", id, a, b)); }` — CheckedAdd is in the same arm as WrappingAdd and emits a bare `+`. Self-hosted c_emitter.vow:975 is identical: `if op == IOP_WADD_I32() || ... || op == IOP_CADD_I64() || ... { emit_binop(out, ty, id, iargs[0], iargs[1], String::from("+")); }`. ESBMC is invoked (esbmc.rs:421-426) with `--no-bounds-check --no-pointer-check --incremental-bmc --max-k-step N --64` and NO `--overflow-check`; the local ESBMC `--help` confirms `--overflow-check` / `--unsigned-overflow-check` are opt-in (off by default), so signed/unsigned overflow is never checked. `grep -c overflow vow-verify/src/c_emitter.rs` returns 0 and no overflow obligation is injected anywhere in vow-verify or vow-ir. Net effect: a function `fn f(x:i64)->i64 vow { ensures: ... } { x +! x }` that aborts at runtime for large `x` is reported VERIFIED with no proof obligation for the abort path — a silently dropped safety obligation, exactly the verified-when-it-can-abort hole §2.1 forbids.

**Proposed fix:** In both C emitters, split the CHECKED arithmetic opcodes out of the wrapping arms and emit an explicit ESBMC obligation before the operation, e.g. for CheckedAddI64: `__ESBMC_assert(!__builtin_add_overflow_p(v{a}, v{b}, (int64_t)0), "vow:arith-overflow");` (or an equivalent INT_MIN/INT_MAX range guard) so ESBMC proves the abort is unreachable or reports a counterexample. For CheckedDiv/Rem also assert the divisor != 0 and the INT_MIN/-1 case. This brings the verification model in line with the runtime abort that cranelift_backend.rs already emits.

#### L2.2 Self-hosted ESBMC harness pins every u64 parameter to the constant 0 — u64 contracts are verified only at input 0 — `CRITICAL` · ✅ survived cross-check
**finder:** `L2-lowering-self` · **kind:** soundness · **verdicts:** 3 · **Filed as #584**  
**Files:** `compiler/verifier.vow:163-193`  
**Design ref:** §6.2 (verification-condition lowering); §7 (self-hosting must verify against the same model). N/A — pure impl bug / Rust-vs-self-hosted divergence  
**Evidence:** `esbmc_nondet_call` in the self-hosted verifier has no `ITY_U64` arm and falls through to the constant string "0":

```vow
fn esbmc_nondet_call(ty: i64) -> String {
    if ty == ITY_I32() { return String::from("__VERIFIER_nondet_int()"); }
    if ty == ITY_I64() { return String::from("__VERIFIER_nondet_long()"); }
    if ty == ITY_F32() { return String::from("__VERIFIER_nondet_float()"); }
    if ty == ITY_F64() { return String::from("__VERIFIER_nondet_double()"); }
    if ty == ITY_BOOL() { return String::from("__VERIFIER_nondet_bool()"); }
    String::from("0")
}
```

`emit_harness` (lines 172-193) calls `esbmc_nondet_call(pty)` for each non-Unit parameter to build `int main(void) { f(<args>); return 0; }`. For a `u64` parameter (`ITY_U64()` == 8), this emits the literal `0` instead of a nondeterministic value. The C function declares the parameter as `uint64_t p0` (via `ir_ty_to_c`, c_emitter.vow:507) and the corresponding GetArg flows `p0` straight into the body (`emit_c_function`, c_emitter.vow:2066-2075, the scalar else-branch). ESBMC therefore explores only the single input `p0 == 0` for every u64 parameter. The Rust harness does NOT have this bug — `vow-verify/src/esbmc.rs:74-84` has an explicit `Ty::U64 => "__VERIFIER_nondet_unsigned_long()"` arm. A contract such as the documented `fn safe_add(a: u64, b: u64) -> u64 vow { requires: a <= 1000 as u64 ... ensures: result >= a }` (docs/spec/contracts.md:324-341) that is FALSE for some nonzero u64 input is reported VERIFICATION SUCCESSFUL by the self-hosted compiler because only a==0,b==0 is checked, while the Rust compiler would correctly find the counterexample. This is unverified code reported as verified.

**Proposed fix:** Add `if ty == ITY_U64() { return String::from("__VERIFIER_nondet_unsigned_long()"); }` to `esbmc_nondet_call` in compiler/verifier.vow, matching the Rust `esbmc_nondet_call`. Add a runtime/verify regression test with a u64-parameter contract that is false for a nonzero u64 (e.g. `ensures: result != 5` with body `x`) and assert the self-hosted verifier reports FAILED, not Verified. (Note: `c_nondet_suffix` in c_emitter.vow:519 already maps U64 correctly; only the harness copy in verifier.vow is wrong.)

#### L2.3 Refinement-type predicate is silently dropped during type resolution, producing false "Verified" results (unverified code reported as verified) — `CRITICAL` · ✅ survived cross-check
**finder:** `L2-refinement-fwd` · **kind:** soundness · **verdicts:** 1 · reviewer severity votes: high×1 · **Filed as #583**  
**Files:** `vow-types/src/env.rs:903`, `vow-syntax/src/parser/types.rs:34,107-124`, `vow-syntax/src/printer.rs:407-420`  
**Design ref:** §5.3 (Refinement properties live above the base type system; "logical predicates are carried through contracts and verifier obligations"), §8 ("Refinement type predicates in verification | Partial | Syntax exists, full semantic forwarding is incomplete"), §2.1 (verification is the primary trust mechanism). This is design-divergence/transition-debt per the §8 "Partial" status, BUT the manifestation …[truncated]  
**Evidence:** The standalone refinement-type syntax `{ binding: base || predicate }` is fully parsed (`vow-syntax/src/parser/types.rs:34` dispatches `LBrace` to `parse_refinement_type`, which at 118-123 builds `Type::Refinement { binding, base, predicate, span }`) and round-trips canonically via the printer (407-420). But at type resolution the predicate is DISCARDED with no diagnostic:

  vow-types/src/env.rs:903 -> `AstType::Refinement { base, .. } => self.resolve(base),`

This is the ONLY consumer of `Type::Refinement` in the entire pipeline (grep across vow-types, vow-ir, vow-verify, vow-codegen returns exactly this one site). No VowRequires/VowEnsures obligation is ever generated from the predicate, and the c_emitter never sees it.

Reproduced with the stage-0 Rust compiler (target/release/vow):

  fn make_positive(x: i64) -> { r: i64 || r > 0 } { x }

`vow verify` returns `{"status":"Verified","diagnostics":[],"counterexamples":[]}` even though `make_positive(-5) == -5` violates `r > 0`. The IDENTICAL predicate written as `ensures: result > 0` is correctly caught: it returns `VerifyFailed` with counterexample `x = -1`. So the refinement path is a genuine false proof, not merely "incomplete": unverified code is reported as Verified with zero warnings. An agent reading the structured JSON would believe the postcondition is proven.

The parameter-position dual is equally unsound: `fn use_pos(x: { n: i64 || n > 0 }) -> i64 vow { ensures: result > 0 } { x }` — the refinement should establish `x > 0` as an assumption (making the ensures provable) and should reject the call `use_pos(-5)`. Instead the refinement is absent in both directions.

**Proposed fix:** A parsed-but-unforwarded refinement predicate must never produce a silent "Verified". Two acceptable fixes: (a) Forward the predicate: when a `Type::Refinement` appears in return position, emit a VowEnsures over the bound variable; in parameter position, emit a VowRequires (mirroring the working `where`-clause path in vow-ir/src/lower/vow.rs:221 lower_param_refinements) AND assume it in the callee body; for struct fields/locals, emit the corresponding obligation. (b) If forwarding is deferred, REJECT the syntax at parse/check time with a clear ErrorCode (e.g. UnsupportedFeature "refinement-type predicates are not yet forwarded to verification; use a `where` clause or an explicit requires/ensures"), so no program can be silently reported Verified while a refinement is dropped. Until (a) lands, (b) is mandatory to preserve the trust mechanism. Add a verify-level regression test asserting the make_positive example does NOT report Verified.

#### L2.4 Self-hosted ESBMC harness passes literal 0 for u64 parameters instead of a nondet unsigned value — unsound contracts on u64 functions reported Verified — `CRITICAL` · ✅ survived cross-check
**finder:** `L2-soundness-skip` · **kind:** soundness · **verdicts:** 2 · **Filed as #584**  
**Files:** `compiler/verifier.vow:163-193`  
**Design ref:** §2.1 (verification is the primary trust mechanism); §6.4 (counterexample-driven repair) — VERIFICATION-PATH SECURITY: unverified code reported as VERIFIED. Also a Rust↔self-hosted design-divergence (CLAUDE.md: both compilers must match).  
**Evidence:** `esbmc_nondet_call` in compiler/verifier.vow has no branch for `ITY_U64()` and falls through to the literal `0`:

```vow
fn esbmc_nondet_call(ty: i64) -> String {
    if ty == ITY_I32() { return String::from("__VERIFIER_nondet_int()"); }
    if ty == ITY_I64() { return String::from("__VERIFIER_nondet_long()"); }
    if ty == ITY_F32() { return String::from("__VERIFIER_nondet_float()"); }
    if ty == ITY_F64() { return String::from("__VERIFIER_nondet_double()"); }
    if ty == ITY_BOOL() { return String::from("__VERIFIER_nondet_bool()"); }
    String::from("0")   // <-- ITY_U64() (type 8) lands here
}
```

`emit_harness` (verifier.vow:172-193) builds `int main(void){ f(<nondet...>); }` by calling `esbmc_nondet_call(pty)` for every non-unit param, so a vowed pure function `fn safe_add(a: u64, b: u64)` is verified by ESBMC against the single concrete call `safe_add(0, 0)` rather than all u64 inputs.

The Rust compiler does this correctly — vow-verify/src/esbmc.rs:78 has `Ty::U64 => "__VERIFIER_nondet_unsigned_long()"`. The two backends therefore disagree on what was proved.

Concrete soundness impact: an unsound contract that only holds at the zero point is reported PROVEN/Verified by `build/vowc`. E.g. `fn double(x: u64) -> u64 vow { ensures: result >= x } { x + x }` wraps for large `x` (a real bug) but `double(0)=0 >= 0` holds, so the self-hosted verifier emits `"status":"Verified"` while the Rust verifier emits `VerifyFailed`. cli.md ("Unsigned Integer Contracts") explicitly promises `u64` contracts are verified "using uint64_t and unsigned nondet values".

Confirmed reachable: `u64` lowers to `ITY_U64()` (compiler/lower.vow:489), all u64 IR ops are listed as modelable (compiler/c_emitter.vow:266-270), so a pure u64 function is NOT skipped and does reach ESBMC. Test fixture tests/verify/u64_verify.vow uses only *correct* contracts, so full_test.sh's compare_json sees Rust=Verified / self=Verified and the divergence is masked; no fixture stresses a wrong-for-large-u64 contract.

**Proposed fix:** Add `if ty == ITY_U64() { return String::from("__VERIFIER_nondet_unsigned_long()"); }` to `esbmc_nondet_call` in compiler/verifier.vow, mirroring vow-verify/src/esbmc.rs:78. Add a tests/verify-fail/ fixture with a u64 contract that is unsound only for large values (e.g. unguarded `ensures: result >= x` on `x + x`) so the parity check (compare_json) fails if either backend regresses; and a tests/verify/ positive fixture whose correctness genuinely depends on the full u64 range.

#### L2.5 Callee preconditions are emitted as __ESBMC_assume inside the callee body, so a caller violating a callee's `requires` is never statically detected (Caller blame unenforceable across call boundaries) — `HIGH` · ✅ survived cross-check
**finder:** `L2-blame-cex` · **kind:** soundness · **verdicts:** 3 · **Filed as #608**  
**Files:** `vow-verify/src/c_emitter.rs:1012-1026`, `vow-verify/src/c_emitter.rs:1567-1589`, `vow-verify/src/c_emitter.rs:2255-2277`, `compiler/c_emitter.vow:1112-1118`, `compiler/c_emitter.vow:1196-1206`  
**Design ref:** §5.1 (requires blames caller), §6.4 (blame information on contract failures; fault localization), contracts.md Blame Model  
**Evidence:** `VowRequires` always lowers to an assumption, never an assertion:
```
Opcode::VowRequires => {
    let pred = inst.args[0].0;
    out.push_str(&format!("  __ESBMC_assume(v{});\n", pred));
}
```
When a verified function `f` calls a modelable callee `g`, the emitter emits a direct C call (c_emitter.rs:1579-1586 `v{} = {}({})`), and `g`'s full body is emitted ahead of `f` via `emit_c_function_full` (c_emitter.rs:2255-2277). That body contains `g`'s own `requires` lowered to `__ESBMC_assume(...)`. Consequently, when ESBMC verifies `f`, any arguments `f` passes to `g` that violate `g`'s precondition are silently assumed valid rather than asserted. There is no call-site emission of `g`'s `requires` as an assertion anywhere in the emitter (grep confirms VowRequires has only the assume arm; no `assert`-requires path exists). contracts.md:61 and docs/vow_design.md:127 promise `requires` violations are blamed on (and therefore detected for) the Caller; statically this obligation is dropped at every function-call boundary. Debug-mode codegen DOES check requires at runtime (cranelift_backend.rs:1268-1277 emits Blame::Caller), confirming this is specifically a static-verification gap, not a language design that abandons the check.

**Proposed fix:** At each modelable call site, before emitting the C call to `g`, emit `__ESBMC_assert(<g.requires substituted with the actual argument values>, "vow:<caller-context id>")` so the caller is checked against the callee's precondition with Caller blame; only the callee's own in-body verification should treat its `requires` as an assumption. Alternatively, when verifying `f`, replace each modelable call with a contract stub: assert the callee's `requires` (Caller blame), then `assume` its `ensures` and havoc outputs, instead of inlining the callee body verbatim. Mirror in both vow-verify/src/c_emitter.rs and compiler/c_emitter.vow.

#### L2.6 Callee `ensures`/`invariant` failures during a caller's verification are attributed to the WRONG contract because vow_ids are per-function (0-based) but the counterexample is always resolved against the top-level function's vows — `HIGH` · ✅ survived cross-check
**finder:** `L2-blame-cex` · **kind:** diagnostics-quality · **verdicts:** 3 · reviewer severity votes: high×2, medium×1 · **Filed as #609**  
**Files:** `vow-ir/src/lower/mod.rs:399-410`, `vow-verify/src/esbmc.rs:140-156`, `vow/src/main.rs:8248-8287`, `compiler/main.vow:185-214`  
**Design ref:** §6.4 (ESBMC counterexamples as first-class repair inputs; blame information), contracts.md Interpreting Counterexamples  
**Evidence:** Vow ids are allocated per-function starting at 0: `let id = VowId(self.func.vows.len() as u32);` (lower/mod.rs:400). The callee `g`'s `ensures` is emitted verbatim as `__ESBMC_assert(v, "vow:<g's local id>")` (c_emitter.rs:1016-1026) inside `g`'s body, which is co-emitted with the target `f` (c_emitter.rs:2255-2277). The harness calls only the target `f` (esbmc.rs:86-98). If `g`'s assertion fails, `extract_vow_id` returns `g`'s local id (e.g. 0), but `build_structured_counterexample` is invoked with `func = f` (main.rs:8633, `verify_one_function`) and resolves it against `f`'s vows:
```
let vow_entry = ce.vow_id.and_then(|id| func.vows.iter().find(|v| v.id.0 == id));
...
let blame = vow_entry.map(|v| match v.blame { ... }).unwrap_or("none");
let source = ce.vow_id.and_then(|id| find_vow_span(func, id)).map(...);
```
When `f` also has a vow with that id, the reported `violation` description, `blame`, and `source` span all come from `f`'s unrelated clause (active misattribution); when it does not, blame degrades to "none" and `violation` falls back to the raw ESBMC line. In all cases the emitted `function` field is `f`, not `g`. The self-hosted path is identical (main.vow:199 `if ve.id == vow_id` over `f.vows`). This corrupts the exact blame/source metadata an agent needs for CEGIS repair (§6.4).

**Proposed fix:** Make the vow_id reported by ESBMC globally disambiguating, e.g. emit `"vow:<func_id>:<local_id>"` and have extract_vow_id/parse_vow_id parse both components, then resolve the counterexample against the function the assertion actually belongs to (and set `function` to that callee). Minimal alternative: assign globally-unique VowIds across the whole module so a callee assertion never collides with an unrelated clause in the target, and have build_structured_counterexample search all module functions for the matching VowEntry.

#### L2.7 `.unwrap()` panic-on-None/Err obligation is never modeled in verification — lowered to ConstUnit, so unwrap on an empty Option verifies clean — `HIGH` · ✅ survived cross-check
**finder:** `L2-builtin-effects` · **kind:** soundness · **verdicts:** 3 · reviewer severity votes: high×1, low×1 · **Filed as #590**  
**Files:** `vow-ir/src/lower/mod.rs:2699-2705`, `compiler/lower.vow:2451-2459`, `vow-verify/src/c_emitter.rs:853-855`  
**Design ref:** §5.5 + §8 "Builtin panic/unsafe effect coverage: Partial"; §2.1; grammar.md:591  
**Evidence:** Both lowerers route `.unwrap()` through the method catch-all that discards it. Rust lower/mod.rs:2699-2704: `_ => { for a in args { lower_consumed_expr(ctx, a); } ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span) }` — there is no `"unwrap"` arm, so `opt.unwrap()` becomes a `ConstUnit`. Self-hosted lower.vow:2451-2459 is identical: after the push/pop/clear/truncate arms it falls through to `... return lctx_emit(ctx, IOP_CONST_UNIT(), ITY_UNIT(), u_args, IDATA_NONE(), 0, 0, String::from(""));`. ConstUnit is a fully-modeled opcode in the C emitter (c_emitter.rs:853-855 `Opcode::ConstUnit => { out.push_str(&format!("  v{} = 0;\n", id)); }`), so it does NOT trigger the `emit_unmodelled`/`UNSUPPORTED_OP_VOW_ID` skip path. grammar.md:591 says `.unwrap()` "panics on None"; the verifier never reads the Option/Result tag, never asserts `tag == Some`, and emits no obligation. Contrast with the `?` operator, which the same lowerer DOES model by loading the tag (lower/mod.rs:2722-2747). A verified function containing `vec.get(i).unwrap()` therefore carries no proof that the unwrap cannot panic.

**Proposed fix:** Lower `.unwrap()` to an explicit IR sequence that loads the Option/Result discriminant (FieldGet 0) and emits a verification obligation `__ESBMC_assert(tag == 1, "vow:unwrap-none")` before producing the payload (FieldGet 1) — analogous to the existing `?` lowering — in BOTH lower/mod.rs and lower.vow, with a matching arm in both C emitters. This makes the documented panic-on-None a proof obligation rather than a silently dropped one.

#### L2.8 Top-level build status flattens overflow-blind `ProvenIr` (Z3+IR fallback) into `Verified`, so an agent sees a fully-proven verdict for proofs that did not model machine-integer overflow — `HIGH` · ✅ survived cross-check · **Duplicate of #337**
**finder:** `L2-esbmc-invoke` · **kind:** soundness · **verdicts:** 3 · reviewer severity votes: high×1, medium×2  
**Files:** `vow/src/main.rs:8651`, `vow/src/main.rs:8771-8782`, `vow/src/main.rs:8942`, `vow-verify/src/solver_strategy.rs:253-285`, `compiler/main.vow:419-422`  
**Design ref:** §2.1 (verification is the primary trust mechanism), §5.7 (no mode-dependent arithmetic semantics; checked/overflow semantics are part of the language), §6.5 (stable status for agents)  
**Evidence:** When BV verification hits a resource budget (timeout / memlimit) in Auto mode, `run_with_fallback` retries with `--z3 --ir` (integer/unbounded encoding) and, on success, returns `VerificationResult::ProvenIr` (solver_strategy.rs:276 `VerificationResult::Proven => (VerificationResult::ProvenIr, ir_config)`). The `VerificationResult::ProvenIr` variant's own docstring in esbmc.rs:32-33 states: `/// Proven under integer arithmetic (--ir mode); overflow not modeled.` In the CLI driver, `ProvenIr` is collapsed with `Proven`: `VerificationResult::Proven | VerificationResult::ProvenIr => PerFuncResult::Ok` (main.rs:8651). A `PerFuncResult::Ok` produces no `Halt`, so the aggregate becomes `VerifyOutcome::Proven` (main.rs:8780), which maps to `VerifyOutcome::Proven => (BuildStatus::Verified, ...)` (main.rs:8942) with exit code 0. The self-hosted compiler behaves identically: `if st == VERIFY_PROVEN_IR() { ... return 1; }` (compiler/main.vow:419-421) counts proven-ir as proven for the `all_proven` aggregate. The per-function JSON does carry a distinct `"proven-ir"` status (main.rs:9879-9881), but the TOP-LEVEL `status: "Verified"` and exit 0 carry no marker that any function was proven only under the overflow-blind IR encoding. For the function under test, parameters are nondeterministic in the harness and constrained only by its own `requires`; an `ensures` clause whose truth depends on bitvector wrap-around (e.g. it holds for unbounded integers but is violated by 64-bit overflow) can be proven SUCCESSFUL under IR while BV would (correctly) refute it. The mitigating in-code rationale `"overflow is not modeled by IR, but the BV caller preconditions still guard against it"` (docs at main.rs:2632) does not hold for the top-level function-under-test, which has no caller preconditions beyond its own `requires`.

**Proposed fix:** Do not silently flatten `ProvenIr` into the top-level `Verified` status. Either (a) introduce a top-level build status / boolean (e.g. `VerifiedModuloOverflow` or a `proven_ir_count` field) so an agent consuming only the top-level `status` learns that some contracts were proven only under an overflow-blind encoding, or (b) for any function whose body contains overflow-sensitive arithmetic referenced by an `ensures`/`invariant`, treat an IR-only proof as inconclusive (soft `unknown`) rather than passing. Document the chosen discipline alongside issue #337.

#### L2.9 Checked arithmetic (`+!`/`-!`/`*!`/`/!`/`%!`) is modeled identically to wrapping arithmetic in the ESBMC C model — abort-on-overflow semantics dropped, producing spurious counterexamples and breaking the documented CEGIS repair — `HIGH` · ✅ survived cross-check
**finder:** `L2-lowering-rust` · **kind:** design-divergence · **verdicts:** 3 · reviewer severity votes: high×2, medium×1 · **Filed as #585**  
**Files:** `vow-verify/src/c_emitter.rs:864-908`, `vow-verify/src/c_emitter.rs:513-532`  
**Design ref:** §5.7 ("`+!`, `-!`, `*!` are checked and abort with `ArithmeticOverflow` on overflow"); contracts.md:229-243 documents `+!` as the CEGIS fix  
**Evidence:** In `emit_inst`, the checked opcodes share the *same* match arms as the wrapping opcodes and emit a plain C operator with no overflow modeling:

```rust
Opcode::WrappingAddI32 | Opcode::WrappingAddI64
| Opcode::CheckedAddI32 | Opcode::CheckedAddI64
| Opcode::WrappingAddU64 | Opcode::CheckedAddU64 => {
    let (a, b) = (inst.args[0].0, inst.args[1].0);
    out.push_str(&format!("  v{} = v{} + v{};\n", id, a, b));
}
```
(identical pattern for Sub/Mul/Div/Rem). The Cranelift backend, by contrast, models the real Vow semantics — `CheckedAddI64` lowers to `sadd_overflow` + `emit_overflow_check` which aborts (vow-codegen/src/cranelift_backend.rs:895-924). So the runtime ABORTS on overflow while the C/ESBMC model WRAPS in 2's-complement (default `--encoding bv`, esbmc.rs:419-426 passes no `--overflow-check`).

Soundness direction: because the wrapping model admits the post-overflow wrapped value into downstream reasoning whereas the real program would have aborted (never returning), the C model explores a strict SUPERSET of the program's return-states. For `ensures`/`invariant` asserts this means the model is STRONGER than Vow semantics → it rejects correct programs with FALSE counterexamples (it does NOT report unverified code as verified). Concrete false-reject: `fn f(x: i32, y: i32) -> i32 vow { requires: y >= 0, ensures: result >= x } { x +! y }` is correct in Vow (on overflow it aborts, so every *returning* path has result == x+y >= x), but the model computes wrapping `x + y`, lets it wrap negative, and reports `result >= x` violated.

This also breaks the documented CEGIS repair: contracts.md:229-243 ("Wrapping Arithmetic Overflow") tells the agent the fix for a wrap-induced counterexample is to "use checked arithmetic (`+!`)". Because the model treats `+!` exactly like `+`, switching to `+!` does NOT change the ESBMC verdict — the same counterexample reappears, silently defeating the prescribed repair loop. The self-hosted emitter has the identical defect (compiler/c_emitter.vow:975-983), so both compilers diverge the same way.

**Proposed fix:** Give the Checked* opcodes their own emit arms that model abort-on-overflow as an assumption that no overflow occurs on the continuing path, e.g. for `CheckedAddI64`: `int64_t v{id}; _Bool __ovf = __builtin_add_overflow(v{a}, v{b}, &v{id}); __ESBMC_assume(!__ovf);` (and the analogous `sub`/`mul`; for `/!`/`%!` add `__ESBMC_assume(v{b} != 0)` plus the INT_MIN/-1 guard). This makes the model match Vow's runtime semantics: paths that would abort are pruned, so contracts that rely on the no-overflow guarantee of `+!` verify, and the contracts.md CEGIS fix actually works. Apply the same change to compiler/c_emitter.vow to keep the two compilers in sync.

#### L2.10 is_modelable gate accepts functions that the C emitter then models with emit_unmodelled (silent nondet, no assertion) — fail-open precision/soundness gap for structured values in contracts — `HIGH` · ✅ survived cross-check · **Duplicate of #572**
**finder:** `L2-quantifiers-loops` · **kind:** soundness · **verdicts:** 3 · reviewer severity votes: medium×1, low×1  
**Files:** `vow-verify/src/c_emitter.rs:632-639`, `vow-verify/src/c_emitter.rs:1605-1672`, `compiler/c_emitter.vow:302-309`, `compiler/c_emitter.vow:1231-1293`  
**Design ref:** §2.1 (verification is the primary trust mechanism); §6.3 (contract obligations must be consumed identically by codegen and verify). N/A for the quantifier sub-question — pure impl gate gap.  
**Evidence:** The verifier draws a sharp line between two unsupported-handling helpers: `emit_unsupported_for_verification` fails CLOSED (`__ESBMC_assert(0, "vow:UNSUPPORTED_OP_VOW_ID")`, c_emitter.rs:1727-1739) while `emit_unmodelled` fails OPEN (emits `v{id} = __VERIFIER_nondet_*();` with NO assert, c_emitter.rs:1662-1672 / c_emitter.vow:856-865). Soundness therefore hinges on `emit_unmodelled` being unreachable for any value that feeds a contract predicate in a function the gate deemed modelable. The gate (`is_modelable`) classifies a `FieldGet` as modelable when its result/arg is a tracked vec/string/hashmap/btreemap/option var (c_emitter.rs:632-639; c_emitter.vow:302-309), but the var-classification (`collect_typed_vars`) does not trace structured values that arrive as parameters or are returned/passed without a typed-receiver use. Such an unclassified structured value passes the gate (function is verified, not skipped) yet hits `emit_unmodelled` / the FieldGet `emit_unmodelled` tail (c_emitter.rs:1645/1648; c_emitter.vow:1277), so the contract is evaluated against a fresh nondet value rather than the real model — yielding a bogus verdict instead of a clean Skipped. This is the exact class enumerated in open issue #572 (param-typed `Vec<Vec<i64>>`, untyped `get_val` results, and map/btreemap structured keys/values).

**Proposed fix:** Per #572: key the non-scalar / unmodelable check off the instruction's IR type (Ptr/struct vs scalar) rather than the `collect_typed_vars` sets, and extend it to map/btreemap key+value operands. Any structured operand the emitter cannot precisely model must route the whole function to `emit_unsupported_for_verification` / `non_modelable_reason` (fail-closed Skipped), never to `emit_unmodelled`. Mirror the fix in both c_emitter.rs and c_emitter.vow.

#### L2.11 Standalone refinement-type syntax { x: T || pred } is accepted by the parser/printer but is not in the spec, and silently unsound — `HIGH` · ❌ refuted by cross-check
**finder:** `L2-refinement-fwd` · **kind:** design-divergence · **verdicts:** 2 · reviewer severity votes: low×1  
**Files:** `vow-syntax/src/parser/types.rs:107-124`, `docs/spec/grammar.md:87-98`, `docs/spec/contracts.md:196-211`  
**Design ref:** §2.5 (canonical form is more valuable than stylistic flexibility), §5.3 (where clauses are the documented sugar for requires). CLAUDE.md spec-source-of-truth rule. design-divergence / transition debt.  
**Evidence:** The spec documents ONLY the parameter `where`-clause form. grammar.md:87 heading is "Where Clauses (Refinement Types on Parameters)" and grammar.md:98 says "`where` constraints on parameters become additional `requires`"; contracts.md:198 says "Where clauses on parameters become refinement types (additional `requires` for verification)". Neither grammar.md nor contracts.md documents the standalone type form `{ binding: base || predicate }`.

Yet the parser implements it as a first-class type producible in ANY type position (param, return, struct field, type alias, const, slice/tuple inner) via `parse_type_inner` -> LBrace -> `parse_refinement_type` (vow-syntax/src/parser/types.rs:34,107-124), and the printer canonicalizes it as `{ x: T | pred }` (printer.rs:407-420) so it survives the idempotent parse->print->parse pass — making it look like a supported, blessed feature. CLAUDE.md states the spec is the source of truth and "Any change to Vow syntax MUST include a corresponding spec update." Here surface syntax exists with NO spec entry and (per the sibling finding) with unsound semantics.

This violates §2.5 canonical form: there are now two surface forms for a parameter refinement (`x: i64 where x>0` which works, and `x: { n: i64 || n>0 }` which is silently dropped) that look equivalent to an agent but behave oppositely.

**Proposed fix:** Decide one direction and make spec + compiler agree: either (1) document the `{ x: T || pred }` type form in grammar.md and contracts.md AND implement its forwarding (then the sibling soundness finding is also resolved), or (2) remove `parse_refinement_type` and the `Type::Refinement` AST node from the surface language, keeping only the spec-documented `where`-clause form, so there is exactly one canonical way to write a parameter refinement. Given §2.5 and the soundness hole, (2) is the lower-risk choice until full forwarding is designed.

#### L2.12 Static verification can only ever produce Callee-blame counterexamples; the entire Caller-blame reporting path (call_sites, violating_args) is unreachable dead code in the static pipeline — `MEDIUM` · ✅ survived cross-check
**finder:** `L2-blame-cex` · **kind:** design-divergence · **verdicts:** 1 · reviewer severity votes: low×1 · **Filed as #646**  
**Files:** `vow/src/main.rs:8272-8334`, `vow-verify/src/c_emitter.rs:1012-1026`  
**Design ref:** §6.4 (blame information on contract failures; CEGIS repair), §5.1 (requires blames caller)  
**Evidence:** `build_structured_counterexample` builds caller-oriented evidence only when `blame == "caller"`:
```
let sites_raw = if blame == "caller" { call_site_index.get(&func.name)... } else { vec![] };
...
let violating_args = if blame == "caller" { ... per-binding param/arg spans ... } else { vec![] };
```
But `blame` is "caller" only when the matched VowEntry has Blame::Caller, i.e. a `requires` clause. `requires` is universally lowered to `__ESBMC_assume` (c_emitter.rs:1014) and is NEVER asserted (no requires->assert path exists; confirmed by grep and by the test `emit_vow_requires_as_assume` asserting `!c.contains("__ESBMC_assert")`). A function's own `requires` is assumed during its verification (so it cannot fail), and callee `requires` is also assumed (Finding 1). Therefore the static verifier cannot emit a counterexample whose vow_id maps to a Caller-blame clause, making the call_sites/violating_args branches dead for the static path. This is a real divergence from §6.4's promise of blame-aware repair input: the toolchain advertises caller-blame static counterexamples (schema in main.rs requires `blame` with Caller/Callee, and CounterexampleJson carries call_sites/violating_args) b …[truncated]

**Proposed fix:** Resolve jointly with Finding 1: once callee preconditions are asserted at call sites with Caller blame, this path becomes reachable and the call_sites/violating_args machinery becomes meaningful. Until then, either (a) document that static caller-blame is not produced and the runtime debug path is the only caller-blame detector, or (b) add a regression test that actually drives a static Caller-blame counterexample so the path is proven live.

#### L2.13 Non-`vow:` model assertions (vec/string/hashmap/btreemap capacity & bounds) surface with vow_id defaulting to 0, mislabeling the failure as the function's first contract — `MEDIUM` · ✅ survived cross-check
**finder:** `L2-blame-cex` · **kind:** diagnostics-quality · **verdicts:** 1 · reviewer severity votes: low×1 · **Filed as #647**  
**Files:** `vow-verify/src/c_emitter.rs:1112-1130`, `vow-verify/src/esbmc.rs:140-156`, `vow/src/main.rs:8255-8287`  
**Design ref:** §6.4 (structured, actionable failure info), contracts.md Collection Models for Verification (capacities are verifier-internal, not contracts)  
**Evidence:** Model assertions are emitted with non-numeric labels, e.g.:
```
__ESBMC_assert(v{vec}.len < {vec_max}, "vec capacity");
__ESBMC_assert(v{idx} >= 0 && v{idx} < v{vec}.len, "vec bounds");
```
(and analogous `"string capacity"`, `"hashmap capacity"`, `"btreemap capacity"`). When one of these fails, `extract_vow_id` finds no `vow:N` line under `Violated property:` and returns `None` (esbmc.rs:140-156). `build_structured_counterexample` then does `let vid = ce.vow_id.unwrap_or(0);` (main.rs:8255) and `vow_entry = None`, so the cex is reported with `vow_id: 0`, `blame: "none"`, and `violation` falling back to `ce.description` (the raw ESBMC line). For a function whose vow 0 is, say, a `requires`, the agent sees a counterexample tagged `vow_id: 0` that does not correspond to the model-capacity assertion that actually failed — there is no distinct signal that this is a verifier model-bound trip rather than a user-contract violation. The UNSUPPORTED_OP path got a reserved sentinel (UNSUPPORTED_OP_VOW_ID, c_emitter.rs:1725) precisely to avoid this fallthrough, but the capacity/bounds asserts did not.

**Proposed fix:** Give the model-capacity/bounds assertions a reserved non-user label range (e.g. a MODEL_BOUND_VOW_ID sentinel analogous to UNSUPPORTED_OP_VOW_ID, or a `model:<kind>` tag parsed alongside `vow:`), and have build_structured_counterexample synthesize an explicit diagnostic ('verifier collection model capacity reached; this is a prover bound, not a contract violation') instead of defaulting vow_id to 0 and blaming the first contract. Mirror in compiler/verifier.vow + compiler/main.vow.

#### L2.14 Effect checker only flags `.unwrap()` for `[panic]`; out-of-bounds indexing and checked-arith abort sites are never required to admit `[panic]` — `MEDIUM` · ✅ survived cross-check
**finder:** `L2-builtin-effects` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #622**  
**Files:** `vow-types/src/effects.rs:35-47`, `vow-types/src/effects.rs:88-91`, `compiler/checker.vow:1639-1677`, `compiler/checker.vow:1807-1822`  
**Design ref:** §5.5 (effect vocabulary incl. `[panic]`); §8 (Builtin panic/unsafe effect coverage: Partial); grammar.md:536, grammar.md:208; docs/roadmap.md §24.3  
**Evidence:** The panic-effect detection is hard-wired to the single method name `unwrap`. effects.rs:44 `if method == "unwrap" { panic_exprs.push(expr); }` is the ONLY place `panic_exprs` is populated. `ExprKind::Index` (effects.rs:88-91) and the checked-arith `ExprKind::BinaryOp` (effects.rs:30-33) merely recurse into children and never register a panic site. The self-hosted checker mirrors this exactly: collect_calls_in_expr pushes `"__unwrap__"` only for `mname == "unwrap"` (checker.vow:1651-1652), while `EXPR_INDEX` (checker.vow:1673-1677) and `EXPR_BINOP` (checker.vow:1657-1661) only recurse; check_effects_fn (checker.vow:1810-1812) flags only `__unwrap__`. Per grammar.md:536 `v[i]` "panics if out of bounds" and §5.7 checked operators abort, these are panic sources of equal standing to `.unwrap()`, yet a pure (effect-free) function can index a Vec or use `+!` with no `[panic]` requirement. docs/roadmap.md:395-398 confirms: "`[Panic]` effect exists in the grammar but no builtins are annotated with it. Division by zero, array out-of-bounds, and `.unwrap()` are all silent panic sources." This is exactly the §8 'Builtin panic/unsafe effect coverage: Partial' gap. Note: indexing's `[panic]` req …[truncated]

**Proposed fix:** Decide and document (grammar.md/contracts.md) which builtin panic sources require `[panic]`: at minimum out-of-bounds-capable indexing `v[i]`/`v[i]=val` and checked arithmetic `+!`/`-!`/`*!`/`/!`/`%!` (and `/`,`%` zero-divisor trap). Then extend collect_calls_in_expr in BOTH effects.rs and checker.vow to register these expression kinds as panic sites so the existing `[panic]`-coverage diagnostic fires uniformly. Keep the spec the single source of truth for which builtins carry `[panic]`.

#### L2.15 U64 vow-binding captures emit wrong tag (TAG_I32) and zero payload in VowViolation values — runtime value diverges from verified obligation — `MEDIUM` · ✅ survived cross-check · **Duplicate of #439**
**finder:** `L2-debug-runtime` · **kind:** diagnostics-quality · **verdicts:** 1 · reviewer severity votes: low×1  
**Files:** `vow-codegen/src/cranelift_backend.rs:1700-1709`, `vow-codegen/src/cranelift_backend.rs:1801-1811`, `vow-clif-shim/src/lib.rs:2824-2833`, `vow-clif-shim/src/lib.rs:2888-2898`  
**Design ref:** §5.1 (debug-mode runtime checks mirror obligations); §6.4 (captured values are repair inputs). CLAUDE.md VowViolation diagnostic shape: `values` must contain runtime values of free variables.  
**Evidence:** Both backends recognize U64 as an IR type (`IrTy::U64`/`ITY_U64`, mapped to cranelift `I64` in `ir_ty_to_cranelift`/`ity_to_cranelift`), but the VowViolation capture encoding ignores it. In the Rust backend `tag_for_ir_ty` (cranelift_backend.rs:1700) has arms only for I32/I64/F32/F64/Bool and `_ => 0`, so a u64 binding gets `tag = 0` which the runtime's `fmt_payload` (vow-runtime/src/lib.rs:58-67) interprets as `TAG_I32 => format!("{}", payload as i32)`. Worse, the payload `match ir_ty` at lines 1801-1811 has `_ => builder.ins().iconst(types::I64, 0)`, so the captured payload for a u64 is hardcoded to literal 0 (the real runtime value is never stored). The self-hosted shim mirrors the identical bug: `tag_for_ir_ty` (lib.rs:2824) `_ => 0`, and the payload `match *ir_ty` (lib.rs:2888-2897) `_ => builder.ins().iconst(types::I64, 0)`. The runtime even defines `const TAG_U64: u8 = 5` with correct unsigned formatting (`format!("{payload}")`), but neither codegen path ever emits it. Net effect: a failing contract that captures a `u64` free variable reports `"x": 0` in the VowViolation `values` map regardless of x's actual value (e.g. a value above i64::MAX), giving the agent a false count …[truncated]

**Proposed fix:** Add `IrTy::U64 => 5` (TAG_U64) to `tag_for_ir_ty` in both cranelift_backend.rs:1700 and vow-clif-shim/src/lib.rs:2824, and add a `IrTy::U64 => *cl_val` (store the I64 bit pattern) arm to the payload match in both emit_vow_violation_body (cranelift_backend.rs:1801) and the shim's emit_vow_check (lib.rs:2888). Add a debug-mode runtime test triggering a u64 contract violation with a value > i64::MAX and assert the JSON values map shows the unsigned decimal, not 0.

#### L2.16 VowViolation JSON assembled without escaping; non-finite float captures (NaN/inf) emit invalid JSON — `MEDIUM` · ✅ survived cross-check · **Duplicate of #436**
**finder:** `L2-debug-runtime` · **kind:** diagnostics-quality · **verdicts:** 1  
**Files:** `vow-runtime/src/lib.rs:58-68`, `vow-runtime/src/lib.rs:92-123`  
**Design ref:** §4.5 / §6.5 (structured machine-readable diagnostics are part of the language contract); §6.4 (counterexamples as first-class repair inputs).  
**Evidence:** `__vow_violation` (vow-runtime/src/lib.rs:114-116) builds the structured envelope by raw string interpolation: `format!(r#"{{\"error\":\"VowViolation\",...,\"description\":\"{desc}\",\"file\":\"{file}\",...}}"#)`. `desc` is the printed predicate expression (vow-ir/src/lower/vow.rs:17-28 `clause_description` -> `print_expr`), which can contain string literals — a contract such as `requires: s.matches_literal_at(0, "a\"b")` yields a description containing a literal double-quote, corrupting the JSON. `file` is a filesystem path that on Linux may contain `"`, `\`, or control bytes. No characters are escaped. Separately, `fmt_payload` (lines 58-67) formats float captures with Rust `Display`: `TAG_F32 => format!("{}", f32::from_bits(...))`, `TAG_F64 => format!("{}", f64::from_bits(...))`. For `f64::NAN`/`f64::INFINITY` this prints `NaN`/`inf`, which is not valid JSON numeric syntax, so the first stderr line (the JSON envelope agents parse) fails to parse. Both the values map and the envelope become unparseable, defeating the structured-diagnostics contract.

**Proposed fix:** Route `desc`, `file`, and binding `name` through a small JSON string-escaping helper (escape `\"`, `\\`, control chars). In `fmt_payload`, render non-finite floats as a documented JSON-safe form (e.g. JSON `null` or a quoted string like `"NaN"`/`"Infinity"`). Add a subprocess test that invokes `__vow_violation` with a quoted/control-char desc and a NaN float binding and asserts `serde_json` parses the first line.

#### L2.17 ESBMC `--memlimit 4096m` is hardcoded well above the documented 2 GB CI/run ulimit, so the cgroup SIGKILLs ESBMC before its own soft limit fires (no `out of memory` text emitted) — `MEDIUM` · ✅ survived cross-check · **Duplicate of #546**
**finder:** `L2-esbmc-invoke` · **kind:** diagnostics-quality · **verdicts:** 1 · reviewer severity votes: low×1  
**Files:** `vow-verify/src/solver_strategy.rs:33-45`, `vow-verify/src/solver_strategy.rs:96-100`, `compiler/verifier.vow:339-341`, `compiler/verifier.vow:376-382`  
**Design ref:** §6.4 (structured diagnostics / honest failure classification); docs/design/verifier-model-bounds.md "Failure semantics"  
**Evidence:** `pub const DEFAULT_ESBMC_MEMLIMIT_MB: u32 = 4096;` (solver_strategy.rs:35) is emitted unconditionally as `--memlimit 4096m` (esbmc_args, solver_strategy.rs:96-100), and the self-hosted compiler mirrors it: `fn DEFAULT_ESBMC_MEMLIMIT_ARG() -> String { String::from("4096m") }` (verifier.vow:341), pushed unconditionally in `append_memlimit_arg` (verifier.vow:376-382, comment: `Self-hosted Vow has no memlimit opt-out CLI today; emit the default cap unconditionally`). The project's own CLAUDE.local.md memory and issue #546 document that all self-compiled binaries run under `ulimit -v 2000000` (2 GB), and #546 records an actual arena/esbmc OOM under that cap. When the cgroup/ulimit kills ESBMC first, it dies via SIGKILL producing no `out of memory` / `bad_alloc` text, so `is_memory_limit_output` (esbmc.rs:537-546) does not fire; the empty output falls through to `VerificationResult::ToolError` (esbmc.rs:529) / `VERIFY_ERROR` (verifier.vow parse_verify_status:493). The verdict fails closed (good — not a false PASS), but it is reported as an opaque tool error rather than the honest `unknown`/`memory limit exceeded` classification the model-bounds spec calls for, degrading the structured-re …[truncated]

**Proposed fix:** Drive the emitted `--memlimit` from the active virtual-memory limit (e.g. read RLIMIT_AS / the run ulimit and pass `min(active_limit, 4096m)` minus headroom) so ESBMC's soft limit fires first and yields a clean `memory limit exceeded` → `unknown` classification instead of an externally-killed `ToolError`. This is WS-0.4 in docs/roadmap-0.3.0-foundations.md.

#### L2.18 Collection-typed cross-function call results and HashMap/BTreeMap value payloads are not guarded by the non-modelable check, allowing struct-typed values to be emitted as/assigned from `int64_t` (ill-typed C model) — `MEDIUM` · ✅ survived cross-check · **Duplicate of #572**
**finder:** `L2-lowering-rust` · **kind:** bug · **verdicts:** 1  
**Files:** `vow-verify/src/c_emitter.rs:1568-1589`, `vow-verify/src/c_emitter.rs:213-245`, `vow-verify/src/c_emitter.rs:1419-1445`  
**Design ref:** §6.2/§6.3 (shared IR/C model must faithfully represent collection values); §5.4 (intrinsic built-ins Vec/String/HashMap have fixed semantics)  
**Evidence:** The non-scalar guard `vec_op_carries_non_scalar` (lines 213-245) only inspects `__vow_vec_*` store/load ops; it does not cover (a) a modelable callee that RETURNS a collection, nor (b) HashMap/BTreeMap value payloads. For case (a), the modelable-call arm emits `v{id} = callee(...)` whenever `inst.ty != Ty::Unit`:

```rust
if inst.ty != Ty::Unit {
    out.push_str(&format!("  v{} = {}({});\n", id, callee.name, args_str.join(", ")));
}
```
A callee returning `Vec<i64>`/`String` has `func.return_ty == Ty::Ptr`, so its C signature returns `int64_t` and its body does `return 0; /* modelled type return */` (lines 1050-1067). If the caller subsequently uses that result as a collection receiver (e.g. `.len()`), `collect_typed_vars` puts the result id into `vec_vars`, so it is *declared* `__vow_vec_t v{id};` — and `v{id} = callee()` becomes `__vow_vec_t = int64_t`, an ill-typed assignment. For case (b), `__vow_map_insert` emits `v{m}.vals[..] = v{v};` (line 1432) with `vals` typed `int64_t[]` (preamble line 2128), so a structured `v{v}` (Vec/String/Option) is silently stored as `int64_t`.

This tends to fail closed (the CBMC frontend rejects the ill-typed C, surfacing as a ToolError rather …[truncated]

**Proposed fix:** Extend the non-modelable guard so a function is rejected (non-modelable, hence skipped rather than mis-modeled) when (1) a `CallTarget` result whose `inst.ty == Ty::Ptr/LinearPtr` is later treated as a collection receiver, and (2) a map insert/get carries a structured (non-scalar) key or value. This generalizes `vec_op_carries_non_scalar` to CallTarget results and to map/btreemap key/value args, mirroring the Vec element check.

#### L2.19 Self-hosted verifier skips every function using String::parse_i64()/parse_u64() that the Rust verifier models and proves — `MEDIUM` · ✅ survived cross-check
**finder:** `L2-lowering-self` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #621**  
**Files:** `compiler/c_emitter.vow:108-164`, `compiler/c_emitter.vow:1474-1768`  
**Design ref:** §6.2; §7 (self-hosted compiler must reach feature parity with the Rust verifier). N/A — Rust-vs-self-hosted divergence  
**Evidence:** `__vow_string_parse_i64_opt` / `__vow_string_parse_u64_opt` are the lowered form of the surface builtins `String::parse_i64()` / `parse_u64()` (grammar.md:554-555; lower.vow:2242,2249). The self-hosted `is_known_builtin` (c_emitter.vow:108-164) does NOT list them — the list ends with the btreemap entries and never mentions `parse_i64_opt`/`parse_u64_opt`. Consequently `is_modelable` (c_emitter.vow:277-280, `is_known_builtin(inst.ds)` fails) returns false for any function calling them, and `skip_if_non_modelable` (main.vow:228-247) skips the whole function (reported `Skipped`). They ARE recognised by `collect_option_vars` (c_emitter.vow:800-805) so the intent was clearly to model them, but `emit_string_op` (c_emitter.vow:1474-1768) has no arm for them either, so even if reached they would fall to `emit_unmodelled` (silent nondet, no sentinel). The Rust emitter handles both: `is_known_builtin` lists `"__vow_string_parse_i64_opt" | "__vow_string_parse_u64_opt"` (vow-verify/src/c_emitter.rs:450-451) and `emit_string_op` models them as a nondet Option with `tag == 0 || tag == 1` plus a nondet payload (vow-verify/src/c_emitter.rs:1376-1382). Net effect: a vowed function that parses a str …[truncated]

**Proposed fix:** Add `__vow_string_parse_i64_opt` and `__vow_string_parse_u64_opt` to the self-hosted `is_known_builtin`, and add an arm in `emit_string_op` mirroring the Rust model: `v{id}.tag = __VERIFIER_nondet_long(); __ESBMC_assume(v{id}.tag == 0 || v{id}.tag == 1); if (v{id}.tag == 1) { v{id}.payload = __VERIFIER_nondet_long(); }`. Add a verify test that parses a string under a contract and asserts the self-hosted compiler runs ESBMC (not Skipped).

#### L2.20 Verification strategy is plain --incremental-bmc, but spec/CLI/help all claim "k-induction-parallel (incremental BMC + k-induction proof)" — loop invariants are bounded-checked, never inductively proven — `MEDIUM` · ✅ survived cross-check
**finder:** `L2-quantifiers-loops` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #624**  
**Files:** `vow-verify/src/esbmc.rs:419-426`, `compiler/verifier.vow:388-398`, `docs/spec/contracts.md:16-21`, `compiler/main.vow:2196-2308`  
**Design ref:** §6.5 (machine-readable self-description must not require hidden knowledge / must be accurate); §2.1; contracts.md Verification Pipeline  
**Evidence:** The only ESBMC strategy flag actually passed is `--incremental-bmc`:

vow-verify/src/esbmc.rs:419-426 — `cmd.arg(tmp.path()).arg("--no-bounds-check").arg("--no-pointer-check").arg("--incremental-bmc").arg("--max-k-step").arg(max_k_step.to_string()).arg("--64");`

verifier.vow:390-395 — `esbmc_args.push(String::from("--incremental-bmc")); ... esbmc_args.push(String::from("--max-k-step")); esbmc_args.push(max_k_step); esbmc_args.push(String::from("--64"));`

There is NO `--k-induction` / `--k-induction-parallel` flag anywhere (grep confirms only `--max-k-step` occurrences). Yet the documentation and machine-readable capability surface assert a k-induction proof:

contracts.md:18 — "Verification strategy: **k-induction-parallel** (incremental BMC + k-induction proof)"
main.vow:2196 — `"strategy": "k-induction-parallel",` and main.vow:2308 — `Strategy ... : k-induction-parallel (incremental BMC + k-induction proof)`.

ESBMC's `--incremental-bmc` runs a base-case check plus a forward-condition (full-unwind) check per k (confirmed in open issue #516, citing esbmc_parseoptions.cpp:1485/1534). It does NOT run the k-induction inductive step, which is what `--k-induction[-parallel]` enables. …[truncated]

**Proposed fix:** Either (a) change the ESBMC invocation to actually request k-induction (`--k-induction` or `--k-induction-parallel`) if inductive proofs of unbounded loops are intended, or (b) correct the spec/help/self-description to state the real strategy: "incremental BMC (base case + forward condition) up to --max-k-step; loops beyond the bound report UNKNOWN." Keep generate_help.py:303/742, contracts.md:18, and main.vow:2196/2308 in sync with whichever is chosen.

#### L2.21 where-clause (parameter refinement) predicate is never type-checked; non-bool predicates pass silently and are lowered to a requires obligation — `MEDIUM` · ✅ survived cross-check
**finder:** `L2-refinement-fwd` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #656**  
**Files:** `vow-types/src/check.rs:579-601`, `compiler/checker.vow:655-663`, `vow-ir/src/lower/vow.rs:221-246`  
**Design ref:** §5.3 (parameter where clauses are sugar for requires — they should therefore get identical type discipline). docs/spec/contracts.md:98 ("each where clause can only reference its own parameter") — that restriction is also unenforced, same root cause. Pure impl bug / diagnostics-quality gap (consistent across both compilers).  
**Evidence:** In `check_fn` the explicit `requires`/`ensures` clauses ARE type-checked to be `bool` (vow-types/src/check.rs:589-598 for requires, 626-635 for ensures, emitting ContractTypeMismatch otherwise). But the parameter loop (579-585) only binds `param.name` and NEVER visits `param.refinement` — the `where`-clause predicate is never type-checked at all. The self-hosted compiler mirrors this gap: compiler/checker.vow:655-663 iterates params and calls `env_define_var` but never checks the refinement at `i*3+2`.

The predicate is still lowered into a real VowRequires obligation (vow-ir/src/lower/vow.rs:221-246 lower_param_refinements builds a `VowClause::Requires` from `param.refinement` and emits Opcode::VowRequires). So an ill-typed `where` predicate bypasses the contract type discipline and is handed straight to the C emitter / ESBMC.

Reproduced: `fn f(x: i64 where x) -> i64 vow { ensures: result == x } { x }` returns `{"status":"Verified","diagnostics":[]}`. The predicate `x` has type i64, not bool — written as `requires: x` it would be rejected with ContractTypeMismatch, but as a `where` clause it passes with no diagnostic. Inconsistent contract type-checking across two syntactic forms …[truncated]

**Proposed fix:** In check_fn, after binding params, type-check each `param.refinement` predicate exactly like a `requires` clause: it must evaluate to `bool`, else emit ContractTypeMismatch. Also enforce the spec restriction that a `where` clause may only reference its own parameter (reject references to other params/result). Apply the same in compiler/checker.vow check_fn (process the `i*3+2` refinement slot). Add a checker test that `x: i64 where x` is rejected, matching the existing `requires: x` rejection.

#### L2.22 Panic-effect detection keys on the bare method name `unwrap`, so it misses any non-method unwrap form and any future renamed/aliased panic builtin — `LOW` · ✅ survived cross-check
**finder:** `L2-builtin-effects` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #622**  
**Files:** `vow-types/src/effects.rs:35-47`, `compiler/checker.vow:1639-1654`  
**Design ref:** §5.5; §4.5 (tooling/diagnostics are part of the language contract); §8 (Partial)  
**Evidence:** Both checkers detect the panic effect purely by string-matching the method selector `unwrap`. effects.rs:44 `if method == "unwrap"` only matches `ExprKind::MethodCall` whose method is literally `unwrap`; a call-form such as `Option::unwrap(x)` (an `ExprKind::Call` with callee ident `Option::unwrap`) is collected as a normal call (effects.rs:22-26) and looked up via `env.lookup_fn`, which has no entry for the builtin, so no `[panic]` is required. checker.vow:1651 likewise only matches `mname == S …[truncated]

**Proposed fix:** Resolve unwrap/panic-producing builtins to a canonical builtin identity (or a dedicated IR opcode) during type-check rather than matching the surface selector string, and drive the `[panic]` requirement off that resolved identity. This makes the panic-effect check robust to call-form and to any future panic builtins, and keeps it consistent between the two compilers.

#### L2.23 Self-hosted compiler drops file/offset from VowViolation; debug binaries it produces report empty source location, diverging from the Rust backend — `LOW` · ✅ survived cross-check
**finder:** `L2-debug-runtime` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #687**  
**Files:** `vow-clif-shim/src/lib.rs:2912-2926`, `vow-clif-shim/src/lib.rs:1115-1121`, `compiler/clif.vow:399-408`  
**Design ref:** §6.4 (fault localization / source-located repair inputs); CLAUDE.md self-hosted parity requirement: "full feature parity with the Rust compiler: ... structured diagnostics".  
**Evidence:** The self-hosted IR carries source location per vow clause: `compiler/ir.vow` `struct IrVowEntry` has `file: String` (line 253) and `offset: i64` (line 254). But `compiler/clif.vow` (lines 400-404) forwards only `ctx, ve.id, ve.description, bids, bnames` to `__vow_clif_fn_vow` — `ve.file` and `ve.offset` are never passed. Correspondingly the FFI entry `__vow_clif_fn_vow` (vow-clif-shim/src/lib.rs:1115-1121) has no file/offset parameters, and the shim's `emit_vow_check` hardcodes them: `// No file …[truncated]

**Proposed fix:** Extend the `__vow_clif_fn_vow` FFI signature with `file_vec: i64` and `offset: i64`; have `compiler/clif.vow` pass `ve.file` and `ve.offset`; in the shim, intern the file string as a null-terminated data global (as already done for desc at lib.rs:1544-1556) and store the real offset, so `emit_vow_check` passes them to `__vow_violation`. Add a runtime test that asserts a self-hosted-built debug binary's VowViolation JSON contains a non-empty `file` and the correct `offset`.

#### L2.24 Self-hosted `--max-k-step` value is forwarded verbatim to ESBMC with no numeric validation — `LOW` · ✅ survived cross-check
**finder:** `L2-esbmc-invoke` · **kind:** bug · **verdicts:** 1 · **Filed as #668**  
**Files:** `compiler/main.vow:513-517`, `compiler/verifier.vow:393-394`  
**Design ref:** §6.5 (deterministic, self-describing agent tooling; parity between the two compilers)  
**Evidence:** `let max_k_step_str: String = get_flag_arg(argv, "--max-k-step"); let max_k_step: String = { if max_k_step_str.len() > 0 { max_k_step_str } else { String::from("50") } };` (compiler/main.vow:513-517) takes the raw flag string and threads it straight into `esbmc_args.push(String::from("--max-k-step")); esbmc_args.push(max_k_step);` (verifier.vow:393-394) with no check that it is a positive integer. A non-numeric (`--max-k-step abc`), negative, or `0` value is passed unchecked to ESBMC. The Rust p …[truncated]

**Proposed fix:** Validate `--max-k-step` in both compilers: reject non-numeric and `< 1` values with a structured diagnostic (mirroring the existing `--verify-jobs must be >= 1` check at compiler/main.vow:325), and apply a clap `value_parser` range on the Rust side. Keep the two compilers in parity per the CLAUDE.md dual-implementation rule.

#### L2.25 `__vow_string_eq` is modeled by length-equality plus an unconstrained nondet bool, ignoring statically-known byte content — verifier cannot prove or refute equality of literals of equal length (imprecision, spurious counterexamples) — `LOW` · ✅ survived cross-check
**finder:** `L2-lowering-rust` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #677**  
**Files:** `vow-verify/src/c_emitter.rs:1263-1275`, `vow-verify/src/c_emitter.rs:1971-1982`  
**Design ref:** §2.1/§6.4 (the verifier and its counterexamples are part of the programming model; spurious counterexamples degrade the CEGIS loop). N/A — pure impl/precision gap  
**Evidence:** For distinct operands, `__vow_string_eq` is modeled as `v{id} = (v{a}.len == v{b}.len) ? __str_eq_{lo}_{hi} : 0;` where `__str_eq_{lo}_{hi}` is a per-pair `__VERIFIER_nondet_bool()` (declared at lines 1978-1981). The byte arrays (`.data[]`) are never compared, even when both strings have statically-known literal content (which the emitter DOES materialize for `__vow_string_literal`, lines 1183-1186). Consequently `String::from("ab").eq(String::from("cd"))` (both len 2) is modeled as a free nonde …[truncated]

**Proposed fix:** When both operands have statically-known bytes (literal-derived, like the existing `const_str_indices`/`inst_by_id` lookups used by `matches_literal_at`), emit a concrete byte-by-byte comparison (`len equal && for all i data[i]==data[i]`) instead of the nondet bool. Retain the length-gated nondet only for the genuinely-opaque case (nondet/param strings). This recovers precise equality for the common literal case while staying sound.

#### L2.26 detect_const_fns classifies u64-returning constant functions as const-fns; the Rust detector does not (inlining-vs-call divergence) — `LOW` · ✅ survived cross-check
**finder:** `L2-lowering-self` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #663**  
**Files:** `compiler/c_emitter.vow:628-639`  
**Design ref:** §6.2; §6.3 (shared IR / consistent obligations). N/A — Rust-vs-self-hosted divergence  
**Evidence:** The self-hosted `detect_const_fns` accepts `IOP_CONST_U64()` as a constant-function body:

```vow
if ci.op == IOP_CONST_I32() || ci.op == IOP_CONST_I64() || ci.op == IOP_CONST_U64() || ci.op == IOP_CONST_BOOL() {
    result.push(fi);
}
```

and `const_fn_value_str` / `const_fn_type_str` emit it as a `...ULL` literal of C type `uint64_t` (c_emitter.vow:661-663, 682). The Rust `detect_constant_functions` only matches `ConstI32`, `ConstI64`, `ConstBool` — there is no `ConstU64` arm (vow-verify/src/ …[truncated]

**Proposed fix:** Align the two detectors: either add a `ConstU64` arm to the Rust `detect_constant_functions` (preferred, since u64 const-fns are legitimate), or drop `IOP_CONST_U64()` from the self-hosted `detect_const_fns`. Add a cross-compiler test that emits the same C for a u64 const-fn under both backends.

#### L2.27 for-each loop invariant is lowered at the header before the element binding is in scope — invariants over the loop element are silently unavailable — `LOW` · ✅ survived cross-check
**finder:** `L2-quantifiers-loops` · **kind:** diagnostics-quality · **verdicts:** 1 · reviewer severity votes: medium×1 · **Filed as #664**  
**Files:** `vow-ir/src/lower/mod.rs:1416-1485`, `compiler/lower.vow:1660-1700`  
**Design ref:** §5.1 (loop invariants for simple predicates: Implemented); §5.2 (for-each desugars to while). N/A — lowering/scoping limitation.  
**Evidence:** In the for-each lowering the invariant is emitted at the top of the header block:

vow-ir/src/lower/mod.rs:1416-1418 — `// Lower vow invariant at top of header (before condition)\n if let Some(wv) = for_vow { vow::lower_invariant(ctx, wv); }`

but the per-iteration element binding is only defined later, inside the body block:

vow-ir/src/lower/mod.rs:1466-1485 — `ctx.switch_to_block(body_block); let elem_id = ctx.emit(Opcode::Call, ... "__vow_vec_get_val" ...); ... ctx.push_scope(); ctx.define(b …[truncated]

**Proposed fix:** Document the restriction in contracts.md/grammar.md (for-each invariants may reference loop-carried variables and the iterable, but not the per-iteration element binding), or lower a header-visible projection of the current element so element-referencing invariants are expressible and checked at the header. Keep Rust and self-hosted lowering aligned.

#### L2.28 Self-hosted parse_type cannot parse the refinement-type syntax, diverging from the Rust compiler (one accepts+drops, the other errors) — `LOW` · ✅ survived cross-check
**finder:** `L2-refinement-fwd` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #690**  
**Files:** `compiler/parser.vow:454-506`, `vow-syntax/src/parser/types.rs:33-34`  
**Design ref:** §2.4 / §6 self-hosting feature parity; CLAUDE.md "When implementing changes across Vow compilers, always modify BOTH." design-divergence / transition debt.  
**Evidence:** The self-hosted `parse_type` (compiler/parser.vow:454-506) handles `(`/`!`/`&`/`[`/ident-with-generics, but has NO `tok_lbrace()` branch — there is no equivalent of the Rust `TokenKind::LBrace => self.parse_refinement_type(start)` (vow-syntax/src/parser/types.rs:34). So a program containing a `{ x: T || pred }` type that the Rust stage-0 compiler accepts (and silently drops, per the critical finding) would fail to parse under build/vowc. CLAUDE.md requires both compilers to maintain feature pari …[truncated]

**Proposed fix:** Resolve in tandem with finding #2: if the refinement-type form is removed from the Rust surface, no self-hosted change is needed (parity restored). If it is kept and forwarded, add a `tok_lbrace()` -> parse_refinement_type branch to compiler/parser.vow:parse_type plus the corresponding AST/lower handling, so both compilers agree.

#### L2.29 Self-hosted harness encodes pointer/linear-pointer (ITY_PTR/ITY_LPTR) parameters as literal 0, diverging from c_nondet_suffix and risking spurious proofs if such a param-shaped function is ever classified modelable — `LOW` · ✅ survived cross-check
**finder:** `L2-soundness-skip` · **kind:** soundness · **verdicts:** 1 · **Filed as #584**  
**Files:** `compiler/verifier.vow:163-193`, `compiler/c_emitter.vow:511-521`  
**Design ref:** §2.1; §6.4 — defense in depth on the verification harness. N/A — pure impl bug / parity gap with c_nondet_suffix.  
**Evidence:** The same `esbmc_nondet_call` fall-through that mishandles u64 also emits the literal `0` for `ITY_PTR()` (type 6) and `ITY_LPTR()` (type 7) parameters, since neither has a branch. Yet the in-body nondet helper `c_nondet_suffix` (compiler/c_emitter.vow:511-521) DOES distinguish them (`ITY_PTR => "long"`, `ITY_LPTR => "long"`, `ITY_U64 => "unsigned_long"`), showing the harness and the body model disagree on how these types are realized. Today most pointer-typed params correspond to structured valu …[truncated]

**Proposed fix:** Make `esbmc_nondet_call` exhaustive against every scalar IR type that can appear as a verified parameter and keep it in lock-step with `c_nondet_suffix`: add explicit branches for ITY_U64 (unsigned long), and decide deliberately for ITY_PTR/ITY_LPTR (either a nondet long or an assertion that such params are unreachable in modelable functions). Prefer a single shared type→nondet table consumed by both the harness and the body emitter so they cannot drift.

#### L2.30 Counterexample variable-value capture stops at the first 'Violated property:' line, dropping ESBMC assignments printed after it; reduces free-variable completeness for CEGIS — `LOW` · ❌ refuted by cross-check
**finder:** `L2-blame-cex` · **kind:** diagnostics-quality · **verdicts:** 1  
**Files:** `vow-verify/src/esbmc.rs:158-180`, `compiler/verifier.vow:567-606`  
**Design ref:** §6.4 (counterexamples as first-class repair inputs), CLAUDE.md VowViolation diagnostic shape (values = all free variables)  
**Evidence:** `extract_variable_assignments` hard-breaks at the first `Violated property:` marker:
```
if trimmed == "Violated property:" { break; }
if !in_counterexample { continue; }
```
The self-hosted `parse_assignments` mirrors this (verifier.vow:577 sets `in_ce = false` at `Violated property:`). ESBMC's counterexample trace can list the assignments of the operands involved in the violated expression in the state block(s) at/after the violated property region; any variable value appearing after the first …[truncated]

**Proposed fix:** Do not break at the first `Violated property:`; instead parse all `[Counterexample]` state assignments through end of the counterexample section (or to `VERIFICATION FAILED`), deduplicating by variable name keeping the last assignment (ESBMC's final value). Keep the `Violated property:` marker only for vow_id extraction, not as a hard cutoff for value harvesting. Apply to both esbmc.rs and verifier.vow.

#### L2.31 Self-hosted counterexample output omits call_sites, violating_args, execution_path, and branch_decisions present in the Rust path — blame/fault-localization fidelity is not at parity — `LOW` · ❌ refuted by cross-check
**finder:** `L2-blame-cex` · **kind:** design-divergence · **verdicts:** 1  
**Files:** `compiler/main.vow:185-214`, `compiler/verifier.vow:16-22`, `vow/src/main.rs:8336-8399`  
**Design ref:** §6.4 (fault localization; blame-aware counterexamples), CLAUDE.md self-hosted parity requirement  
**Evidence:** The Rust `build_structured_counterexample` produces execution_path (from block visits), branch_decisions, call_sites, and violating_args (main.rs:8336-8399), which §6.4 calls out as fault-localization aids. The self-hosted `VerifyCE` (built by `build_ce_from_result`, main.vow:206-214) carries only `function, var_names, var_values, violation, vow_id, source, blame` — no execution_path, branch_decisions, call_sites, or violating_args; and `VerifyResult` (verifier.vow:16-22) only tracks `vow_id` an …[truncated]

**Proposed fix:** Either bring compiler/verifier.vow + compiler/main.vow to parity (capture block visits/branch decisions and build call_sites/violating_args), or explicitly document the self-hosted counterexample as a reduced schema in cli.md and gate the schema's optional fields. Given build/vowc is primary, parity is the correct target.

#### L2.32 Self-hosted run_verify_loop drops all later vowed functions once one fails — no Skipped/gate evaluation for them (divergent fail-fast vs Rust's deterministic aggregation) — `LOW` · ❌ refuted by cross-check
**finder:** `L2-quantifiers-loops` · **kind:** diagnostics-quality · **verdicts:** 1  
**Files:** `compiler/main.vow:254-287`  
**Design ref:** §6.4 (structured, fault-localized diagnostics); §2.5 (canonical, mechanically comparable output). N/A — impl divergence, not a soundness hole.  
**Evidence:** In run_verify_loop the launch/skip logic is guarded by `all_proven == 1`:

main.vow:259 — `if vows.len() > 0 && all_proven == 1 {` and main.vow:273 — `if all_proven == 1 {` (only then is `skip_if_non_modelable` called and `verify_start` launched).

Once any in-flight collection sets `all_proven = 0` (main.vow:270), every subsequent vowed function in the `while fi < fns.len()` sweep is neither verified, nor gate-checked for non-modelability, nor reported — it is silently skipped from processing. …[truncated]

**Proposed fix:** Mirror the Rust aggregation: continue gate-checking (`skip_if_non_modelable`) and collecting per-function outcomes for all vowed functions even after the first failure, then report the deterministic lowest-index halt plus the full set of Skipped warnings. Do not gate the skip-evaluation on `all_proven == 1`.

---

## Lane 3 — Implementation Quality

**Clusters:** *verifier-panic→exit 0* = L3-vsec ≡ L3-driver-rust (#413); *self-hosted checker does no arg/operand type-checking* spans L3-checker-self + L3-divergence (#490); *`vow contracts --verify` exits 0 on failure* = L3-driver-rust ≡ L3-vsec ≡ L3-driver-self (#479); *`u64` violation capture tag bug (#439)* spans L3-codegen-rust + L3-shim + L2-debug-runtime; *struct-style enum payloads dropped (#575)* = L3-parser-self ≡ L3-divergence.

_100 findings — 3C / 27H / 37M / 33L._

#### L3.1 Method-call arguments are never type- or arity-checked (type confusion into flat slots) — `CRITICAL` · ✅ survived cross-check
**finder:** `L3-checker-rust` · **kind:** soundness · **verdicts:** 3 · reviewer severity votes: critical×2, high×1 · **Filed as #587**  
**Files:** `vow-types/src/check.rs:1010-1177`  
**Design ref:** §2.1 (verification is the primary trust mechanism), §5.3 (decidable nominal type system); spec docs/spec/grammar.md Vec<T>/HashMap method signatures  
**Evidence:** In `ExprKind::MethodCall` the only thing done with arguments is `for arg in args { self.check_expr(arg); }` (lines 1016-1018) — each argument is type-checked for its *own* validity, but its type is NEVER compared against the receiver method's expected parameter type, and the argument *count* is never checked. The known-method table (lines 1029-1140) maps method name -> result type only; there is no parameter-type list and no per-arg loop comparing `arg_ty` against an expected type (contrast the free-function path at lines 993-1007 which does exactly that). Consequences: `let v: Vec<i64> = Vec::new(); v.push(String::from("x"));` type-checks, and IR lowering then stores the String pointer verbatim into the i64 element slot (`vow-ir/src/lower/mod.rs:2649-2662` `__vow_vec_push_val` with `args.first()` lowered as-is, no cast/validation). The same slot is later read back as `i64` by `v[i]`/`v.get(i)` (`check.rs:1112-1119`, `1243`) — a heap pointer reinterpreted as an integer, which feeds arithmetic and `requires`/`ensures` contracts. Wrong arity is also accepted: `v.push()` type-checks here, and lowering substitutes `ConstUnit` for the missing element (`vow-ir/src/lower/mod.rs:2650-2655`). `HashMap::insert`, `String::push_byte`, etc. are all equally unchecked.

**Proposed fix:** Give each builtin method an explicit parameter-type signature (parameterised by the receiver's type arguments, e.g. Vec<T>.push expects T, HashMap<K,V>.insert expects (K,V)) and run the same arg-count + per-argument coercion check already used for free-function calls (check.rs:976-1007), including the I32-literal carve-out only where appropriate. Emit TypeMismatch on mismatched element/arg types and on wrong arity.

#### L3.2 Verification thread panic is silently degraded to Unverified/exit 0 (fail-open) — `CRITICAL` · ✅ survived cross-check · **Duplicate of #413**
**finder:** `L3-driver-rust` · **kind:** soundness · **verdicts:** 1 · reviewer severity votes: high×1  
**Files:** `vow/src/main.rs:9149-9162`, `vow/src/main.rs:9206-9214`, `vow/src/main.rs:9276-9279`  
**Design ref:** §6.4 (diagnostics/repair must be trustworthy) / §6.5 (verification is part of the programming model); also docs/spec/cli.md Exit Codes 'fail closed'  
**Evidence:** The full build pipeline spawns verification on a dedicated thread:

  let verify_handle = thread::spawn(move || -> (VerifyOutcome, Vec<SkippedFunction>) {
      if no_verify { return (VerifyOutcome::Skipped, Vec::new()); }
      run_verification_sync(...)
  });

On the two success paths (cache-hit and normal codegen) the result is harvested with:

  let (verify_outcome, skipped) = verify_handle
      .join()
      .unwrap_or((VerifyOutcome::Skipped, Vec::new()));   // line 9206-9208 and 9276-9278

`std::thread::spawn` returns `Err` from `join()` when the spawned thread *panics* (the workspace has no `panic = "abort"`, so panics unwind to the thread boundary, not the process). The `unwrap_or` then substitutes `VerifyOutcome::Skipped`, which `verify_outcome_to_output_with_skipped` maps to:

  VerifyOutcome::Skipped => (BuildStatus::Unverified, vec![], None, None),   // line 8940

and `run_build_command` does NOT exit non-zero for `Unverified`:

  if matches!(&result.status, BuildStatus::CompileFailed{..} | BuildStatus::VerifyFailed{..} | BuildStatus::Skipped) { std::process::exit(1); }   // line 9688-9693

So a panic anywhere inside `run_verification_sync` turns a *verified* build into `status:"Unverified"` with exit 0 — reported to the agent as 'Compiled but ESBMC was not invoked'. This path is reachable: the parallel coordinator uses `.expect()` on the `halts`/`skipped_acc` mutexes (lines 8754, 8758, 8770, 8774, 8783-8785); any worker panic inside `verify_one_function` (e.g. an `expect`/slice-index/OOM-adjacent panic in `emit_verify_c_source` or the vow-verify call) makes `thread::scope` re-panic, which propagates out of the spawned verify thread and trips exactly this `unwrap_or`. The whole point of fail-closed semantics (spec cli.md: "both fail closed on Skipped") is defeated: every proof obligation is silently dropped and the run still exits 0.

**Proposed fix:** Treat a panicked verification thread as a hard failure, never as Unverified. Replace `.unwrap_or((VerifyOutcome::Skipped, ..))` at both success-path sites with handling that, on `Err`, produces a fail-closed outcome — e.g. `VerifyOutcome::Error { function: String::new(), message: "verification thread panicked".into() }` (maps to VerifyFailed, exit 1) — or re-panic/abort the whole process. The `no_verify` skip must remain the ONLY producer of VerifyOutcome::Skipped.

#### L3.3 Verifier-thread panic is silently reported as Unverified (exit 0 + linked binary) on the default verify path — `CRITICAL` · ✅ survived cross-check · **Duplicate of #413**
**finder:** `L3-vsec` · **kind:** soundness · **verdicts:** 1 · reviewer severity votes: high×1  
**Files:** `vow/src/main.rs:9206-9214`, `vow/src/main.rs:9276-9279`, `vow/src/main.rs:8940-8942`, `vow/src/main.rs:9688-9693`  
**Design ref:** §2.1 (verification is the primary trust mechanism); §6.4 (fail-closed reporting). cli.md Exit Codes: builds must fail closed when ESBMC was asked to verify but could not prove.  
**Evidence:** In `run_pipeline_from_frontend` the verification thread result is recovered with:
```rust
let (verify_outcome, skipped) = verify_handle
    .join()
    .unwrap_or((VerifyOutcome::Skipped, Vec::new()));   // lines 9206-9208 (cache-hit path) and 9276-9278 (normal codegen path)
```
If the spawned verification thread *panics* (rather than returning), `join()` yields `Err(JoinError)` and `unwrap_or` substitutes `VerifyOutcome::Skipped`. The verify thread can panic: `run_verification_sync` runs an inner `thread::scope` whose workers call `.lock().expect("verify halts mutex poisoned")` (lines 8754,8758,8770,8774,8785) and `verify_one_function` -> `emit_verify_c_source`/counterexample parsing; a panic in any scoped worker poisons the mutex, the `.expect()` re-panics, and `thread::scope` re-raises it through `run_verification_sync` into the outer `verify_handle` closure. `VerifyOutcome::Skipped` then maps at line 8940 to `BuildStatus::Unverified`. `run_build_command` (lines 9688-9692) only fails closed on `CompileFailed | VerifyFailed | Skipped` — `Unverified` is NOT in that set, so the process emits `{"status":"Unverified"}` and exits 0 with the linked executable already written. `VerifyOutcome::Skipped` is documented (line 7829) as meaning "ESBMC not invoked (`--no-verify`)", so a verifier crash is indistinguishable from a deliberate skip: a CEGIS agent reading the JSON sees `Unverified` (decision tree: "compiled but ESBMC not available / --no-verify") and treats the binary as merely unverified rather than as a verification crash. This also affects `vow test --verify`, which calls the same `run_pipeline_from_frontend` with `no_verify = !verify` (main.rs:9447-9459): a verifier panic yields `Unverified`, hits the `_ => {}` arm at 9501, and the test is then executed and can be reported `passed`.

**Proposed fix:** Match on the JoinError explicitly instead of `unwrap_or`. On `Err(_)`, return a fail-closed outcome (e.g. a new `VerifyOutcome::Error{function:"<verifier>", message:"verification thread panicked"}` mapping to `BuildStatus::VerifyFailed`) with `executable: None` (or delete the produced binary), so the run exits 1 and emits a diagnostic. Add a regression test that forces the verify thread to panic and asserts non-zero exit + non-Unverified status.

#### L3.4 ConstF64 emission produces invalid C for inf/-inf/NaN and out-of-range float literals — `HIGH` · ✅ survived cross-check
**finder:** `L3-cemit-rust` · **kind:** bug · **verdicts:** 1 · reviewer severity votes: medium×1 · **Filed as #606**  
**Files:** `vow-verify/src/c_emitter.rs:839-847`  
**Design ref:** §6.2/§6.3 (Vow IR -> lower verification conditions -> ESBMC); N/A for the float design discussion in #527 — this is a pure impl bug in the C emitter  
**Evidence:** ConstF32/ConstF64 are emitted by piping the Rust float through `Display`:
```
Opcode::ConstF32 => {
    if let InstData::ConstF32(v) = inst.data {
        out.push_str(&format!("  v{} = {}f;\n", id, v));   // line 841
    }
}
Opcode::ConstF64 => {
    if let InstData::ConstF64(v) = inst.data {
        out.push_str(&format!("  v{} = {};\n", id, v));    // line 846
    }
}
```
Rust `Display` renders non-finite f64 as the bare tokens `inf`, `-inf`, `NaN` — none of which are valid C floating-constants. I confirmed reachability end-to-end: the lexer parses a float literal with `text.parse::<f64>()` (vow-syntax/src/lexer.rs:293) with NO range check, and `f64::parse` overflows large `digits.digits` literals to `inf` (verified: a 400-digit `1...1.0` literal parses to `Ok(inf)`). The checker types it F64 (vow-types/src/check.rs:749) and the lowerer emits `InstData::ConstF64(*v)` unchanged (vow-ir/src/lower/mod.rs:610-614). I added a unit test calling `emit_inst` with `InstData::ConstF64(f64::INFINITY)`; it emitted exactly `"  v7 = inf;\n"`. gcc rejects this: `error: 'inf' undeclared`. ESBMC's frontend would likewise fail to parse the model, so `run_esbmc` falls through to `VerificationResult::ToolError` (esbmc.rs:528), which the CLI turns into `Halt(VerifyOutcome::Error)` (vow/src/main.rs:8640) — a hard verification failure for an otherwise-valid program. (The f32 path with `{}f` would also emit `inff`/`NaNf`, but ConstF32 is not currently produced from source, so that variant is latent.)

**Proposed fix:** Format float constants with explicit handling of non-finite and edge values rather than `Display`. For finite values, use a C-valid representation that round-trips (e.g. hex float `{:a}`-style or a guaranteed-decimal form with `LL`-style suffix correctness). For non-finite values, emit `(1.0/0.0)`, `(-1.0/0.0)`, `(0.0/0.0)` or `__builtin_inf()/__builtin_nan("")`. Better still, reject literals that overflow to infinity at lex/type-check time with a structured diagnostic, since a non-finite literal is almost certainly a program error.

#### L3.5 Modelable callee taking/returning a collection (Vec/String/Map) emits struct-to-int64_t argument passing => uncompilable C model — `HIGH` · ✅ survived cross-check · **Duplicate of #572**
**finder:** `L3-cemit-rust` · **kind:** bug · **verdicts:** 1  
**Files:** `vow-verify/src/c_emitter.rs:1567-1589`, `vow-verify/src/c_emitter.rs:1809-1834`  
**Design ref:** §6.2 (cross-function lowering of verification conditions to ESBMC) — impl bug in the C emitter; cross-function generalization of the structured-value class in issue #572  
**Evidence:** The modelable-callee call site passes the caller's SSA values verbatim with no type adaptation:
```
let mut args_str = Vec::new();
for (i, arg) in inst.args.iter().enumerate() {
    if i < callee.params.len() && callee.params[i] != Ty::Unit {
        args_str.push(format!("v{}", arg.0));            // line 1575
    }
}
...
out.push_str(&format!("  v{} = {}({});\n", id, callee.name, args_str.join(", ")));  // 1579-1584
```
A modelable callee with a `Ty::Ptr` (Vec/String/Map) parameter has its signature emitted as `int64_t p{i}` (params builder lines 1817-1834 maps Ptr->int64_t). But at the caller, if the argument value is classified as a collection it is declared `__vow_vec_t v{arg}` and passed directly. I built a two-function module (caller creates a Vec via `__vow_vec_new`, passes it to a modelable `vec_helper(Vec) -> i64` that calls `__vow_vec_len`) and ran `emit_c_module_with_callees`. It emitted:
```
int64_t vec_helper(int64_t p0);
...
int64_t caller(void) {
  __vow_vec_t v0;
  ...
  v0.len = 0;
  v1 = vec_helper(v0);   // <-- __vow_vec_t passed to int64_t p0
```
gcc rejects this: `error: incompatible type for argument 1 of 'vec_helper' ... expected 'int64_t' but argument is of type '__vow_vec_t'`. ESBMC's parser fails the same way, yielding `ToolError` -> `Halt(Error)` (vow/src/main.rs:8640) and blocking verification of a perfectly ordinary pure helper pattern. Separately, even the callee side is unsound: the Ptr param is never read (the GetArg path at lines 1857-1862 havocs `v0.len` to nondet instead of binding `p0`), so the callee is modeled with a fresh nondeterministic Vec divorced from the caller's value. The return side has the identical defect when a modelable callee returns a Vec/String (`return 0;` at line 1060 assigned into a struct-typed result).

**Proposed fix:** Key the C type of call args/params/results off the callee's IR parameter/return types and the caller's value classification jointly. For a collection param, either emit the callee with a `__vow_vec_t`/`__vow_string_t`/`__vow_*` parameter type and pass the struct by value/pointer consistently, or (simpler and consistent with #572's guidance) treat any modelable function that has a Ptr/struct parameter or return as NON-modelable in `is_modelable`, so such calls are conservatively skipped/havoced rather than emitted as ill-typed C.

#### L3.6 Exhaustiveness checker treats a bare-identifier binding pattern as a unit-variant match, masking catch-all mis-dispatch — `HIGH` · ✅ survived cross-check
**finder:** `L3-checker-rust` · **kind:** soundness · **verdicts:** 3 · reviewer severity votes: medium×2, high×1 · **Filed as #603**  
**Files:** `vow-types/src/exhaustiveness.rs:106-131`, `vow-types/src/check.rs:1911-1916`  
**Design ref:** §5.2 (`match` is exhaustive), §4.2 (explicit semantics over convenience — one meaning per construct)  
**Evidence:** A bare identifier in a pattern (no `::`, `(`, or `{`) parses as `PatKind::Ident { name }` — a binding/catch-all (vow-syntax/src/parser/types.rs:299-305). `collect_enum_patterns` then does: `PatKind::Ident { name, .. } => { if let Some(&vn) = variant_names.iter().find(|&&n| n == name.as_str()) { covered.insert(vn); } }` (exhaustiveness.rs:119-123). So in `match color { Red => .., Green => .., Blue => .. }` (bare idents), each arm is recorded as covering the like-named variant and the match is declared exhaustive. But these are *binding* patterns: `bind_arm_pattern` binds the whole scrutinee to the name (check.rs:1913-1914), and IR lowering treats `PatKind::Ident` exactly like `Wildcard` — an unconditional catch-all (`vow-ir/src/lower/mod.rs:2247-2251`). The result is that the FIRST arm matches *all* inputs and the remaining arms are dead, yet the type checker reports the match as a well-formed exhaustive variant match. Exhaustiveness reasoning is therefore unsound: it certifies as 'covers every variant' a match that actually never inspects the discriminant.

**Proposed fix:** When the scrutinee type is an enum, resolve a bare `PatKind::Ident` against the enum's variant set: if it names a unit variant, treat it as that variant pattern (covering it, binding nothing); if it does not, treat it as a true catch-all binding (which alone makes the match exhaustive). Apply the same disambiguation in `bind_arm_pattern` and in IR match lowering so dispatch, binding, and exhaustiveness agree. Alternatively require the path form (`Enum::Variant`) for variant patterns and emit a diagnostic when a bare ident shadows a variant.

#### L3.7 Self-hosted checker performs no argument type-checking on function calls (only arity); Rust stage0 rejects — `HIGH` · ✅ survived cross-check · **Duplicate of #490**
**finder:** `L3-checker-self` · **kind:** soundness · **verdicts:** 3  
**Files:** `compiler/checker.vow:981-1052`, `vow-types/src/check.rs:993-1007`  
**Design ref:** §2.5 (single canonical form / multi-compiler agreement), §7 (self-hosting as design validation); §5.3 (decidable nominal type system)  
**Evidence:** Self-hosted EXPR_CALL handler checks only argument *count*, never argument *types*:
```
if e.fn_param_count[fidx] != n_args {
    env_emit_error(e, String::from("wrong argument count"), expr_span(a, eid));
}
return e.fn_ret_tids[fidx];
```
The per-arg loop at 986-991 calls `check_expr` purely for side effects, discarding `_arg_tid`, and the parameter tids stored in `e.fn_param_lids[fidx]` are never consulted. The Rust checker iterates `args.iter().zip(param_tys.iter())` and emits `"argument has type `{arg_ty}` but function expects `{expected_ty}`"` (check.rs:993-1007). Consequence: a self-hosted call like `use_u64(some_string)` or `f(true)` where `f` expects `i64` type-checks clean, but stage0 rejects it. This is strictly broader than the integer-width symptom: any type can be passed for any parameter.

**Proposed fix:** In the EXPR_CALL ident branch, after resolving `fidx`, load the parameter lid via `e.fn_param_lids[fidx]` and for each `i < min(n_args, param_count)` compare `is_coercible(e.ts, arg_tid_i, ts_arg_get(e.ts, param_lid, i))`, emitting EC_TYPE_MISMATCH on failure. Capture arg tids in the existing loop instead of discarding them.

#### L3.8 is_coercible lets ANY i64 value (not just literals) coerce to any integer type; root cause of #490 and a stage0 divergence — `HIGH` · ✅ survived cross-check · **Duplicate of #490**
**finder:** `L3-checker-self` · **kind:** design-divergence · **verdicts:** 3 · reviewer severity votes: medium×1, high×2  
**Files:** `compiler/types.vow:216-232`, `compiler/checker.vow:900-902`, `vow-types/src/check.rs:748,673-675`  
**Design ref:** §5.3 type system; §2.5 canonical/multi-compiler agreement  
**Evidence:** Self-hosted `is_coercible`:
```
if from_tag == CTY_I64() && ty_is_integer(ts, to_tid) {
    return true;
}
```
This admits coercion from `i64` to `u8/u16/u32/u64/...` for *any* i64-typed expression. Integer literals in the self-hosted checker are typed `CTY_I64()` (checker.vow:901: `if tag == EXPR_LIT_INT() { return CTY_I64(); }`), so the checker cannot distinguish a literal from a real i64 value such as `v.len()`. The Rust checker types integer literals as `Ty::I32` (check.rs:748) and only coerces `init_ty == Ty::I32 && ann_ty.is_integer()` (check.rs:673-675) — i.e. only literals coerce, real i64 values do not. Hence `let n: u64 = v.len();` is accepted self-hosted but rejected by stage0 (issue #490 case a).

**Proposed fix:** Introduce a literal-int marker distinct from i64 (mirror Rust's I32-for-literals), or restrict the coercion to syntactically literal/constant integer expressions (reuse `expr_is_lit_int_const`) at each coercion site rather than blanket-coercing all i64. The blanket `from_tag == CTY_I64() && ty_is_integer` rule should be removed.

#### L3.9 Self-hosted checker omits binop operand type-checking: arithmetic/comparison/logical operands never validated for matching/numeric/bool types — `HIGH` · ✅ survived cross-check · **Duplicate of #490**
**finder:** `L3-checker-self` · **kind:** soundness · **verdicts:** 3 · reviewer severity votes: medium×1, high×2  
**Files:** `compiler/checker.vow:928-966`, `vow-types/src/check.rs:777-832,1840-1909`  
**Design ref:** §5.3 type system (explicit semantics, no implicit conversions §4.2); §2.5/§7 multi-compiler agreement  
**Evidence:** Self-hosted EXPR_BINOP: for `&&`/`||` it unconditionally `return CTY_BOOL()` (934-936) with no check that operands are bool; for comparisons it unconditionally `return CTY_BOOL()` (937-939) with no check that operands have matching/comparable types; for arithmetic it does `if ty_is_numeric(e.ts, lhs_tid) { return lhs_tid; } return CTY_I64();` (962-965) with no check that lhs and rhs types match. The Rust checker routes arithmetic through `check_same_numeric` which emits `"arithmetic operands have different types: `{lhs}` and `{rhs}`"` (check.rs:1864-1872), comparisons emit `"comparison operands have different types"` (check.rs:789-808), and `&&`/`||` emit `"logical operator requires `bool`"` (check.rs:813-830). Self-hosted thus accepts `string_a == int_b`, `bool_x && int_y`, `u64_a + i64_b`, all of which stage0 rejects. (Comparison hole is also #490 case b: `0u64 < v.len()`.)

**Proposed fix:** In EXPR_BINOP: for `&&`/`||` verify both operands are CTY_BOOL or opaque; for comparisons verify lhs/rhs are coercible to each other (mirroring the I32/literal rule); for arithmetic add a same-numeric check matching `check_same_numeric` (numeric + matching, with literal coercion). Emit EC_TYPE_MISMATCH otherwise.

#### L3.10 Self-hosted EXPR_IF does not check condition is bool nor that then/else branch types are compatible — `HIGH` · ✅ survived cross-check
**finder:** `L3-checker-self` · **kind:** soundness · **verdicts:** 3 · **Filed as #607**  
**Files:** `compiler/checker.vow:1256-1273`, `vow-types/src/check.rs:1304-1347`  
**Design ref:** §5.2 (`if` is an expression; both branches same type); §5.3; §7 multi-compiler agreement  
**Evidence:** Self-hosted EXPR_IF:
```
let _cond_tid: i64 = check_expr(e, m, cond_eid);
let then_tid: i64 = check_block(e, m, then_bid);
if else_eid == -1 { return CTY_UNIT(); }
let else_tid: i64 = check_expr(e, m, else_eid);
if is_opaque(then_tid) { return else_tid; }
if is_opaque(else_tid) { return then_tid; }
return then_tid;
```
The condition type `_cond_tid` is discarded — no `if condition must be bool` check. When both branches are concrete, the function returns `then_tid` with no comparison to `else_tid` — no `if branches have different types` check. The Rust checker emits `"if condition must be `bool`"` (check.rs:1305-1312) and `"if branches have different types: `{then_ty}` vs `{else_ty}`"` (check.rs:1322-1333). Self-hosted thus accepts `if 5 { ... } else { ... }` and `if c { 1i64 } else { String::from(\"x\") }`, returning a wrong inferred type for the latter.

**Proposed fix:** Check `_cond_tid` is CTY_BOOL or opaque, emitting EC_TYPE_MISMATCH otherwise. When both branches concrete, verify `is_coercible(then,else) || is_coercible(else,then)`; on mismatch emit EC_TYPE_MISMATCH and pick the non-literal branch type to mirror Rust's I32-coercion result selection.

#### L3.11 Float arithmetic is non-functional: float binops lower to integer opcodes; backend float arms are dead code — `HIGH` · ✅ survived cross-check
**finder:** `L3-codegen-rust` · **kind:** bug · **verdicts:** 1 · **Filed as #600**  
**Files:** `vow-ir/src/lower/mod.rs:2882-2912`, `vow-codegen/src/cranelift_backend.rs:1027-1085`  
**Design ref:** §5.3 (primitive floats f32/f64), §5.7 (f32/f64 with IEEE 754 semantics)  
**Evidence:** `binop_opcode` in the IR lowerer dispatches only on `is_u64` vs a default i64 path — there is NO float case:
```
fn binop_opcode(op: BinOp, operand_ty: &Ty) -> (Opcode, Ty) {
    let is_u64 = *operand_ty == Ty::U64;
    match op {
        BinOp::Add => { if is_u64 { (Opcode::WrappingAddU64, Ty::U64) } else { (Opcode::WrappingAddI64, Ty::I64) } }
        ...
```
Float literals lower to `ConstF64`/Ty::F64 (lower/mod.rs:610-616) and the type checker accepts `f64 + f64` (vow-types/src/check.rs:781-782 via `check_same_numeric`). So `let c: f64 = a + b;` lowers to `WrappingAddI64` over two F64-typed Cranelift values. The backend DOES have correct float arms (`AddF32|AddF64 => fadd`, `EqF64 => fcmp`, etc. at cranelift_backend.rs:1027-1085) but a repo-wide grep shows NO site in vow-ir ever emits `Opcode::AddF*/SubF*/MulF*/DivF*/EqF*/...` — those backend arms are unreachable. The emitted `iadd`/`icmp` over F64 operands is rejected/ill-typed by Cranelift (integer-only ops on float values), so float arithmetic fails to compile (or miscompiles); there are no float runtime tests in tests/run/ to catch it. This breaks a first-class, IEEE-754 language feature (§5.3 primitive floats, §5.7 IEEE 754 semantics).

**Proposed fix:** In `binop_opcode`, branch on float operand types and emit the float opcodes the backend already supports: AddF32/AddF64, SubF*, MulF*, DivF*, and EqF*/NeF*/LtF*/LeF*/GtF*/GeF* for comparisons (returning Ty::Bool), keyed off `operand_ty == Ty::F32 | Ty::F64`. Add tests/run float-arithmetic coverage to prevent regression. Mirror in the self-hosted lowerer for parity.

#### L3.12 Self-hosted checker omits operand-type match check for arithmetic operators (+ - * / % and checked variants); Rust rejects mixed/non-numeric operands — `HIGH` · ✅ survived cross-check
**finder:** `L3-divergence` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #597**  
**Files:** `compiler/checker.vow:928-965`, `vow-types/src/check.rs:781-788,1840-1874`  
**Design ref:** §7 (differential-verification trust); §2.1; §4.1  
**Evidence:** Self-hosted check_expr_inner handles ALL arithmetic binops by falling through every special case to:
```vow
        if ty_is_numeric(e.ts, lhs_tid) {
            return lhs_tid;
        }
        return CTY_I64();
```
(checker.vow:962-965). There is NO check that lhs_tid == rhs_tid, and NO error when lhs is non-numeric (it silently returns CTY_I64). The only 'matching type' error in the whole self-hosted checker is for bitwise ops (checker.vow:957). The Rust compiler routes Add/Sub/Mul/Div/Rem (and *Checked) to check_same_numeric (check.rs:781-788), which emits TypeMismatch when `!lhs.is_numeric()` (check.rs:1855-1862) and when `lhs != rhs` (check.rs:1864-1872). Concrete divergent input: `fn f() -> i64 { let a: i64 = 5; let b: u64 = 3 as u64; a + (b as i64) }` is fine in both, but `fn f(a: i64, b: u64) -> i64 { a + b }` — self-hosted: lhs_tid=i64 is numeric, returns i64 (accepted, then coercible to i64 return); Rust: `arithmetic operands have different types: i64 and u64` (rejected). Also `fn f(s: String) -> i64 { s + 1 }` — self-hosted: lhs String not numeric, silently returns CTY_I64 (accepted); Rust: `arithmetic operator requires a numeric type, found String` (rejected).

**Proposed fix:** In checker.vow EXPR_BINOP, after the comparison/logical/bitwise branches, add an arithmetic branch mirroring check_same_numeric: error if lhs not numeric; apply the I64-literal-coercion symmetrically; error if lhs_tid != rhs_tid after coercion; return the unified type.

#### L3.13 Vec::get / HashMap::get return-type divergence between Rust and self-hosted checkers — `HIGH` · ✅ survived cross-check
**finder:** `L3-divergence` · **kind:** design-divergence · **verdicts:** 1 · reviewer severity votes: medium×1 · **Filed as #598**  
**Files:** `compiler/checker.vow:1110-1115,1138-1146`, `vow-verify/../vow-types/src/check.rs:1062-1122`  
**Design ref:** §7 (differential trust); docs/spec/grammar.md §HashMap/Vec methods  
**Evidence:** For Vec<T>.get, self-hosted returns the bare element type:
```vow
                    if mname == String::from("get") { return inner_tid; }
``` (checker.vow:1115) whereas Rust returns Option<T>: `"get" => Some(Ty::Applied(Box::new(Ty::Enum("Option"...)), vec![<elem>]))` (check.rs:1112-1119). For HashMap<K,V>.get, self-hosted returns the value type `if mname == String::from("get") { return val_tid; }` (checker.vow:1146), whereas Rust returns Ty::I64 unconditionally: `"get" => Some(Ty::I64)` (check.rs:1067). Concrete divergent inputs: (1) `fn f(v: Vec<i64>) -> i64 { v.get(0) }` — self-hosted: get→i64, accepted; Rust: get→Option<i64>, rejected (type mismatch). (2) `fn f(m: HashMap<i64,bool>) -> bool { m.get(0) }` — self-hosted: get→bool, accepted; Rust: get→i64, rejected. The grammar lists HashMap.get as `(K)->V` (grammar.md:563) and does NOT list Vec.get at all (grammar.md:526-538, Vec uses indexing v[i]); so both compilers accept an undocumented Vec.get AND assign it conflicting types, and Rust's HashMap.get=i64 contradicts the spec V signature.

**Proposed fix:** Pick the spec-correct semantics and make both agree: HashMap.get should return V (Rust's `Some(Ty::I64)` is wrong per grammar.md:563 — fix to value_ty). Vec.get is not in the grammar — either add it to grammar.md with one canonical return type (Option<T>) and align both checkers and add lowering, or reject `.get` on Vec in both compilers and require indexing.

#### L3.14 Self-hosted is_coercible permits any i64-typed value to coerce to any integer width on let-binding; Rust restricts coercion to I32 literals — `HIGH` · ✅ survived cross-check · **Duplicate of #490**
**finder:** `L3-divergence` · **kind:** soundness · **verdicts:** 1 · reviewer severity votes: medium×1  
**Files:** `compiler/types.vow:216-232`, `vow-types/src/check.rs:669-710,745-770`  
**Design ref:** §7; §2.1; §5.3 (decidable nominal types)  
**Evidence:** Self-hosted is_coercible allows `from_tag == CTY_I64() && ty_is_integer(ts, to_tid)` (types.vow:228-230), i.e. ANY i64-typed expression coerces to any integer width (u8, u64, i8, ...). Because integer literals are typed CTY_I64 in the self-hosted (checker.vow:901 `EXPR_LIT_INT() return CTY_I64()`), this was intended to model unsuffixed-literal coercion, but it applies to ALL i64 values, not just literals. Rust restricts the let-binding coercion to `init_ty == Ty::I32` (check.rs:673-675), and integer literals are I32 in Rust — so only literals coerce. Concrete divergent input: `fn f(n: i64) -> u8 { let x: u8 = n; x }` — self-hosted: i64 coercible to u8, accepted; Rust: `let binding annotated as u8 but initializer has type i64`, rejected. This is the same mechanism as #490 (Vec::len i64 → u64) but generalized to every i64 value, and it is a soundness concern because a 64-bit value is silently bound to a narrower type with no truncation diagnostic.

**Proposed fix:** Tighten is_coercible so i64→narrower-integer coercion only applies to actual unsuffixed integer literals (or constant integer expressions), not arbitrary i64 values — mirror Rust by either giving literals a distinct 'untyped int' tag or gating the coercion on expr_is_lit_int_const at the let-binding site.

#### L3.15 Self-hosted parser discards struct-style enum variant payloads (Variant { field: Type }), diverging from intended linearity/construction checks — `HIGH` · ✅ survived cross-check · **Duplicate of #575**
**finder:** `L3-divergence` · **kind:** design-divergence · **verdicts:** 1 · reviewer severity votes: medium×1  
**Files:** `compiler/parser.vow:324-336`  
**Design ref:** §7; docs/spec/grammar.md (enum variant struct)  
**Evidence:** parse_enum_def, on encountering `Variant { ... }`, parses and THROWS AWAY the field names and types, pushing an empty payload list:
```vow
        } else if at(p, tok_lbrace()) {
            let _lb2: Token = advance(p);
            while !at(p, tok_rbrace()) && !at_end(p) {
                let _fsid: i64 = expect_ident(p);
                let _colon: bool = expect(p, tok_colon());
                let _ftid: i64 = parse_type(p);
                ...
            }
            ...
            let empty_lid: i64 = arena_add_list(p.arena, Vec::new());
            variant_items.push(empty_lid);
``` (parser.vow:324-336). The grammar lists struct-style enum variant patterns `Shape::Named { x, y }` (grammar.md:511) as supported, so the payload is part of the language; dropping it means construction/match/linearity checks over those payloads are skipped by the self-hosted compiler relative to the Rust path.

**Proposed fix:** Store the parsed (field name, type) pairs for struct-style variants in the AST so the checker can validate construction/match payloads and linearity, matching the Rust representation.

#### L3.16 `vow contracts --verify` exits 0 even when contracts are failed/timeout/unknown/error/skipped — `HIGH` · ✅ survived cross-check · **Duplicate of #479**
**finder:** `L3-driver-rust` · **kind:** soundness · **verdicts:** 1 · reviewer severity votes: low×1  
**Files:** `vow/src/main.rs:9950-9978`  
**Design ref:** §6.5 agent-facing tooling (stable, fail-closed verdicts); docs/spec/cli.md §vow contracts  
**Evidence:** In run_contracts_command:

  let mut exit_code = 0;
  if verify {
      if find_esbmc().is_none() {
          for entry in &mut entries { entry.status = "error".to_string(); }
          exit_code = 1;
      } else {
          let verify_cache = if no_cache { None } else { VerifyCache::new() };
          update_contract_statuses(&mut entries, ir_module, verify_cache.as_ref(), limits, config);
      }
  }
  ...
  if exit_code != 0 { std::process::exit(exit_code); }

`exit_code` is only ever set to 1 in the ESBMC-missing branch. The ESBMC-present branch calls `update_contract_statuses`, which writes per-entry `status` of `"failed"`, `"timeout"`, `"unknown"`, `"error"`, or `"skipped"` (lines 9876-9900) but never touches `exit_code`. An agent invoking `vow contracts --verify` and receiving JSON with `"status":"failed"` contracts still observes process exit 0. Unlike `vow build`/`vow verify` (which fail-close on VerifyFailed/Skipped), this verification entry point treats counterexample-proven contract violations as a zero exit.

**Proposed fix:** After `update_contract_statuses`, set `exit_code = 1` if any entry has a non-proven, non-not_verified status (at minimum `failed`; ideally also `timeout`/`unknown`/`error`/`skipped` for consistency with build/verify). Update docs/spec/cli.md Status Values table accordingly. (Tracked by #479, which cites these exact lines.)

#### L3.17 Self-hosted driver advertises `vow decl` in --help/skill JSON but does not implement it; `vowc decl file.vow` silently prints IR and exits 0 — `HIGH` · ✅ survived cross-check
**finder:** `L3-driver-self` · **kind:** design-divergence · **verdicts:** 1 · reviewer severity votes: medium×1 · **Filed as #595**  
**Files:** `compiler/main.vow:144-168`, `compiler/main.vow:9122-9143`, `compiler/main.vow:1406-1406`  
**Design ref:** §6.5 (agent-facing tooling: "tools describe themselves to agents"; the self-description must not lie); cli.md §`vow decl`  
**Evidence:** `get_subcommand` (compiler/main.vow:144-168) recognizes only build/verify/test/contracts/skill/mutants:
```
    if sub == "contracts" { return CMD_CONTRACTS(); }
    if sub == "skill" { return CMD_SKILL(); }
    if sub == "mutants" { return CMD_MUTANTS(); }
    CMD_NONE()
```
There is no `decl` case, and `main()` has no CMD_DECL dispatch (compiler/main.vow:9122-9143). So `vowc decl foo.vow` returns CMD_NONE and falls through to `run_legacy(argv)`. In `run_legacy`, `get_source_path` returns "foo.vow" (the `decl` token is silently ignored), and with no `-o`/`--verify`/`--emit-c` flags the function prints IR text and returns 0 (compiler/main.vow:1355-1359). Yet the self-hosted skill/help payload explicitly advertises the command: compiler/main.vow:1406 emits `"decl": "Emit declaration file (.vow.d) with type signatures only"`, and lines 1713/1725/1730/2213 advertise `vow decl [OPTIONS] <source.vow>` with `-o, --output` and `<source>.vow.d` default. The Rust driver implements it (`run_decl_command`, vow/src/main.rs:9696 + `Command::Decl` arm at 10093-10109). cli.md §`vow decl` (lines 156-168) documents it as a real command. The self-hosted compiler thus claims a capability it lacks, and instead of erroring it silently does something completely different (prints IR, no .vow.d file) with a success exit code.

**Proposed fix:** Either implement `decl` in the self-hosted driver (add CMD_DECL to get_subcommand, a `run_decl` that emits the `.vow.d` declaration file, and dispatch in main), or — if decl is intentionally Rust-only like mutants is self-hosted-only — remove `decl` from every self-hosted skill/help surface AND make `get_subcommand` reject the bare `decl` token with an explicit "unimplemented in self-hosted compiler" error and non-zero exit, rather than silently routing it through run_legacy.

#### L3.18 Legacy bare form `vow <file.vow>` does not verify (and does not emit a binary) by default, contradicting the documented "equivalent to vow build" contract — `HIGH` · ✅ survived cross-check
**finder:** `L3-driver-self` · **kind:** design-divergence · **verdicts:** 1 · reviewer severity votes: medium×1 · **Filed as #596**  
**Files:** `compiler/main.vow:1251-1362`, `compiler/main.vow:9142-9143`  
**Design ref:** §6.4/§6.5 (verification is part of the programming model; tools describe themselves honestly); cli.md:11,91 — legacy bare form documented as equivalent to `vow build`  
**Evidence:** `main()` routes any unrecognized leading token (including a bare source file) to `run_legacy` (compiler/main.vow:9142-9143):
```
    maybe_auto_install_skill();
    run_legacy(argv)
```
In `run_legacy`, verification is opt-IN: `let do_verify: bool = has_flag(argv, "--verify");` (compiler/main.vow:1266). With no flags the function just prints IR (lines 1356-1358); with `-o` it emits a binary and reports `Unverified` with NO verification at all (lines 1327-1354: `clif_emit_module(...)` then `diag_emit_build_json(String::from("Unverified"), out, dctx)`). Contrast the Rust driver: the no-subcommand `None` arm (vow/src/main.rs:10169-10220) builds a `mode/trace/config` and calls `run_build_command(... args.no_verify ...)` — i.e. it is fully equivalent to `vow build`, which verifies by default (no_verify defaults false) and fails closed (run_build_command exits 1 on VerifyFailed/Skipped, vow/src/main.rs:9688-9693). cli.md:11 documents `vow [OPTIONS] <source.vow>` as `# legacy (equivalent)` to `vow build` (which "Verifies contracts by default"), and cli.md:91 ties the bare `vow <source.vow>` form to the build path's auto-install. An agent or script using the documented legacy form on the self-hosted compiler gets an unverified binary (or just IR) while believing it ran the verifying build path.

**Proposed fix:** Make the self-hosted no-subcommand path mirror Rust: route bare `vow <file.vow>` (and `-o`) into the same code as `run_build` (verify by default, `--no-verify` to opt out, emit build-result JSON, fail closed on VerifyFailed/Skipped). If the IR-dump/`--emit-c` behaviors are still wanted for bootstrap, gate them behind explicit flags (`--dump-ir`, `--emit-c`) within that build path rather than making no-verify the silent default.

#### L3.19 Self-hosted compiler has NO vow-predicate purity check — effectful calls inside requires/ensures/invariant are entirely unenforced (parity gap with Rust) — `HIGH` · ✅ survived cross-check
**finder:** `L3-effects-rust` · **kind:** design-divergence · **verdicts:** 3 · reviewer severity votes: medium×2, high×1 · **Filed as #586**  
**Files:** `compiler/checker.vow:610-635`, `vow-types/src/effects.rs:228-271`  
**Design ref:** §5.5 (contract purity); grammar.md:669-671; CLAUDE.md ('modify BOTH the Rust compiler and the self-hosted compiler')  
**Evidence:** The Rust frontend enforces contract purity via `check_vow_purity` (effects.rs:228-271), which flags `requires:/ensures:/invariant:` predicates that call effectful functions. The self-hosted compiler's clause handler does NOT:

```vow
fn check_vow_clause(e: CheckEnv, m: Module, vow_lid: i64, kind: i64, context: String) [io] {
    ...
    let clause_tid: i64 = check_expr(e, m, clause_eid);
    if clause_tid != CTY_BOOL() && !is_opaque(clause_tid) { ... CONTRACT_TYPE_MISMATCH ... }
    ...
}
```

It only checks the clause's *type* (must be bool). There is no scan for effectful calls. `check_effects_fn` (compiler/checker.vow:1799-1825) scans only `fn_body_bid` (`collect_calls_in_block(e, m, body_bid, calls)`, line 1804), never the vow clause expressions. A grep across `compiler/*.vow` for 'must be pure'/'predicate'/'vow.*effect' finds no purity enforcement on contract clauses. Therefore on `build/vowc` (the PRIMARY day-to-day compiler), a predicate like `requires: read_file(p).len() > 0` type-checks and passes — an effectful, possibly-failing computation is admitted into a contract, defeating the design guarantee and diverging from the Rust frontend whose `--help` output (compiler/main.vow:3105) tells agents 'Contract expressions ... must be pure -- they cannot call effectful functions.'

**Proposed fix:** Port `check_vow_purity` to `compiler/checker.vow`: in `check_vow_clause`, after type-checking the clause expression, run `collect_calls_in_expr` over the clause and emit an EffectViolation (Blame::Callee) for any called function whose `fn_effects` is non-zero, plus any `__unwrap__` panic site. Add self-hosted regression tests mirroring `vow_purity_impure_predicate_emits_violation`.

#### L3.20 Self-hosted lowerer hardcodes ITY_I64 for every mutation/loop/match Phi and Upsilon type; Rust reference uses the real inst type — `HIGH` · ✅ survived cross-check
**finder:** `L3-ir-lower-self` · **kind:** design-divergence · **verdicts:** 1 · reviewer severity votes: medium×1 · **Filed as #618**  
**Files:** `compiler/lower.vow:1286-1289`, `compiler/lower.vow:1215-1218`, `compiler/lower.vow:1339-1356`, `compiler/lower.vow:1488-1528`, `compiler/lower.vow:1655-1702`, `compiler/lower.vow:2596-2603`, `compiler/lower.vow:2691-2702`  
**Design ref:** §7 (self-hosting must reach byte-identical fixed point with Rust; divergence is transition debt) and §6.3 (IR carries uniform types); also a latent codegen-soundness bug  
**Evidence:** Every cross-block merge Phi for a mutated/loop variable in the self-hosted lowerer is emitted with a hardcoded ITY_I64 type, and its feeding Upsilons are emitted ITY_UNIT or ITY_I64, regardless of the variable's real type. Examples:

if-else mutation phi (lower.vow:1286): `let phi_id: i64 = lctx_emit(ctx, IOP_PHI(), ITY_I64(), phi_args, IDATA_NONE(), 0, 0, String::from(""));` with the feeding upsilons at 1215/1260 emitted as `IOP_UPSILON(), ITY_UNIT()`.
while loop var phi (1356): `lctx_emit(ctx, IOP_PHI(), ITY_I64(), ...)`; pre-header upsilon (1342) `IOP_UPSILON(), val_ty` where `val_ty: i64 = ITY_I64()` (1339).
loop (1505), for (1680), and match mutation phi (2691) and match RESULT phi (2702) all hardcode ITY_I64.

The Rust reference (vow-ir/src/lower/mod.rs) types every one of these from the actual value: if-else `let phi_ty = ctx.inst_ty(t_val);` (mod.rs:1029); while/loop `let ty = ctx.inst_ty(*pre_val);` (mod.rs:1216,1246); match `let phi_ty = ctx.inst_ty(arm_results[0].3[i]);` (mod.rs:2318) and result `arm_results.first().map(|(_,_,ty,_)| *ty)` (mod.rs:2334-2337).

Why this miscompiles: the clif shim reloads a cross-block Phi value using the Phi instruction's own type: `let orig_ty = inst_ty_map.get(&arg_inst_id).and_then(|&t| ity_to_cranelift(t)).unwrap_or(types::I64); let val = load_slotted_value(&mut builder, slot, orig_ty);` (vow-clif-shim/src/lib.rs:1744-1748). The Upsilon store keeps the value's true type (store_slotted_value, lib.rs:2815-2820: f64/f32 stored raw). So for an f64/f32 mutated/loop variable the value is stored as a float but reloaded as I64 (load_slotted_value default arm `stack_load(types::I64,...)`, lib.rs:2811) — a type-confused value that either trips Cranelift's verifier or silently reinterprets the float bits. The same hardcoded-I64 Phi type also flows into VowViolation diagnostics: the capture's ir_ty is `*inst_ty_map.get(&b.inst_id)` (lib.rs:2214) which for a captured loop/mutated variable is the Phi (I64), so emit_vow_check tags an f64 capture as I64 (tag_for_ir_ty, lib.rs:2824) and extracts the payload as raw i64 instead of bitcasting the float (lib.rs ~2890), reporting wrong runtime values. Note the SAME function already types the if-else *result* phi correctly via `lctx_inst_ty` (lower.vow:1299,1306,1312), proving the I64 hardcoding on the mutation phis is an oversight, not intent.

**Proposed fix:** Replace the hardcoded ITY_I64 on every mutation/loop/match Phi and its feeding Upsilons with the real type of the incoming value, mirroring the Rust lowerer: compute `let ty = lctx_inst_ty(ctx, pre_val_or_mut_val);` and use it for both the Phi and each Upsilon. Apply at lower.vow:1215,1260,1286 (if-else), 1339-1356/1376-1388 (while), 1488-1528 (loop), 1655-1702 (for), 2596-2603/2641-2648/2669-2676 and 2691/2702 (match). Add tests/run cases that mutate an f64/f32 (and i32) variable across an if-else, a while/for/loop, and a match, asserting runtime values (extends the spirit of issue #471).

#### L3.21 AMBIGUOUS-region allocation escaping via a direct FieldSet/Store/extern-push into a parameter container is never rejected (use-after-free) — `HIGH` · ✅ survived cross-check · **Duplicate of #368**
**finder:** `L3-ir-region-rust` · **kind:** soundness · **verdicts:** 1  
**Files:** `vow-ir/src/region.rs:3314-3363`, `vow-ir/src/region.rs:3395-3417`, `vow-codegen/src/cranelift_backend.rs:392-414`  
**Design ref:** §4.4 ('Any subsequent parameter-rooted store of that directly fresh heap value MUST be rejected with RegionConflict') and §5.1 representation promise  
**Evidence:** `lub_to_region_id` mints the AMBIGUOUS sentinel when a fresh allocation's marker set spans more than one hidden caller slot (e.g. it is both returned and stored into a parameter container):
```rust
// region.rs:2676-2681
if has_legacy_caller && !caller_slots.is_empty() {
    return RegionId::Caller(HiddenRegionIdx::AMBIGUOUS);
}
if caller_slots.len() > 1 {
    return RegionId::Caller(HiddenRegionIdx::AMBIGUOUS);
}
```
The ONLY place an AMBIGUOUS source is rejected is `check_store_conflict_semantic` (region.rs:3397 `Some(RegionId::Caller(idx)) if idx.is_ambiguous()`), and that is reached ONLY from `check_store_conflicts_post_inference`, which iterates exclusively `Opcode::Call` with `InstData::CallTarget` and only for `AliasOf(p)` store effects:
```rust
// region.rs:3322-3341
if inst.opcode != Opcode::Call { continue; }
let InstData::CallTarget(callee) = &inst.data else { continue; };
...
if let InternalReturnRegion::Published(RegionConstraint::AliasOf(p)) = source_constraint {
```
No post-inference conflict check covers a direct `Opcode::FieldSet`/`Opcode::Store` into a parameter, nor a `CallExtern` push edge (`__vow_vec_push_val` etc.) into a parameter, within the analyzed function itself. Meanwhile codegen silently routes an AMBIGUOUS allocation to slot 0:
```rust
// cranelift_backend.rs:402-406
let resolved_idx = if idx.is_ambiguous() { 0 } else { idx.0 as usize };
```
with the comment 'If the value is actually used at a store, the build has already been rejected' — an assumption that holds only for the AliasOf(p) cross-call path. Concretely, `fn f(c: Container) -> Item { let it = Item{..}; c.items.push(it) /* or c.field = it */; return it }` makes `it` FreshInCaller (return slot 0) AND a store-target source into param `c` (slot 1) => region(it)=Caller(AMBIGUOUS). The direct FieldSet/extern-push into `c` is in f's own body, so `check_store_conflicts_post_inference` never sees it; codegen allocates `it` in the slot-0 (return) arena, but a pointer to it is stored into `c`'s (slot-1) container. If the caller's `c` outlives the return arena, that pointer dangles.

**Proposed fix:** Extend the post-inference reject to cover all parameter-rooted stores of an AMBIGUOUS source, not just AliasOf(p) cross-call effects. Iterate `Opcode::FieldSet`/`Opcode::Store` (args=[target,source]) and `for_each_extern_store_edge` store edges in the analyzed function; for each whose `target` traces to a parameter (`trace_param`) and whose `source`'s `region_map` entry is `Caller(idx)` with `idx.is_ambiguous()` and is heap-producing, emit `RegionConflict`. Mirror in compiler/region.vow. Alternatively, have codegen hard-error on any AMBIGUOUS region that is actually allocated, rather than falling back to slot 0.

#### L3.22 Self-hosted parser silently discards struct-style enum variant payloads — `HIGH` · ✅ survived cross-check · **Duplicate of #575**
**finder:** `L3-parser-self` · **kind:** design-divergence · **verdicts:** 3 · reviewer severity votes: medium×3  
**Files:** `compiler/parser.vow:324-340`  
**Design ref:** §5.2 (surface syntax: enums/match), §7 (self-hosting differential behavior). grammar.md lines 474, 512: enum struct-variant `Named { x: i64 }` and pattern `Shape::Named { x, y }`.  
**Evidence:** In parse_enum_def, the struct-variant arm parses the fields but throws them away and pushes an EMPTY payload list:
```vow
} else if at(p, tok_lbrace()) {
    let _lb2: Token = advance(p);
    while !at(p, tok_rbrace()) && !at_end(p) {
        let _fsid: i64 = expect_ident(p);
        let _colon: bool = expect(p, tok_colon());
        let _ftid: i64 = parse_type(p);
        if at(p, tok_comma()) { let _c: Token = advance(p); }
    }
    let _rb2: bool = expect(p, tok_rbrace());
    let empty_lid: i64 = arena_add_list(p.arena, Vec::new());  // fields dropped
    variant_items.push(empty_lid);
}```
The tuple-variant arm (lines 311-323) correctly stores `payload_lid`, but the struct-variant arm stores `empty_lid`. The Rust parser (vow-syntax/src/parser/items.rs:82-106) preserves these as `VariantKind::Struct(fields)`. The AST also has no notion of named-field variants — `arena_add_edef` only stores a flat variant list, so a struct-style variant `Named { x: i64 }` is recorded as a payload-less unit variant. This makes the self-hosted compiler unable to construct/match/lower struct-style enum variants and silently differs from the Rust frontend.

**Proposed fix:** Extend the enum AST encoding to record per-variant kind + named-field metadata (mirror VariantKind::Struct), and store the parsed field list in parse_enum_def's struct-variant arm instead of an empty list. Then thread it through checker/lower/match. This is exactly the gap tracked in #575.

#### L3.23 Canonical printer ignores nesting level for all block-bodied expressions, producing malformed indentation — `HIGH` · ✅ survived cross-check
**finder:** `L3-printer-rust` · **kind:** design-divergence · **verdicts:** 3 · reviewer severity votes: medium×3 · **Filed as #591**  
**Files:** `vow-syntax/src/printer.rs:553-560`, `vow-syntax/src/printer.rs:561-688`, `vow-syntax/src/printer.rs:215-232`  
**Design ref:** §4.1 (Single canonical way: "the compiler canonicalizes source form"), §5.2 ("The compiler enforces a canonical source form ... The source form itself is part of the mechanical interface")  
**Evidence:** `print_expr` takes no `level` parameter, so every block-bodied expression hard-codes the indentation level of its body. In the `If`/`While`/`ForEach`/`Loop`/`Block`/`Match` arms the body is always printed at level 0/1:
  line 556: `out.push_str(&print_match_arm(arm, 1));`
  line 569: `print_block(then_branch, 0)`
  line 577: `print_block(b, 0)` (else block)
  line 615/648/676: `out.push_str(&print_block(body, 0));` (while/for/loop)
  line 688: `ExprKind::Block(b) => print_block(b, 0),`
and loop vow clauses hard-code `indent(1)` (lines 597,599,603,630-637,660-665).
Because `print_block_body` (line 221) and `print_stmt` (lines 245,253) recurse through `print_expr`, any nested control-flow construct is emitted at the wrong column. Concrete probe (parse -> print) of
  `fn f(x: i64) -> i64 { while x > 0 { if x > 5 { x = x - 1; } else { x = x - 2; } } x }`
produces:
```
    while x > 0 {
    if x > 5 {
    x = x - 1;
} else {
    x = x - 2;
}
}
    x
```
The inner `if`, its statements, and the closing braces are all dedented to column 0/4 regardless of depth; loop `invariant:` clauses likewise print at a fixed column. The output is idempotent (print->print stable) and re-parses to the same AST only because Vow is brace-delimited, so it is not a soundness hole — but it is not a canonical source form.

**Proposed fix:** Thread a `level: usize` parameter through `print_expr` (and `print_expr_with_parens`), and replace every hard-coded `print_block(.., 0)` / `print_match_arm(.., 1)` / `indent(1)` with the actual current level (e.g. `print_block(body, level)`, `print_match_arm(arm, level + 1)`, `indent(level + 1)` for vow clauses). `print_block`/`print_block_body` already accept a level; the callers in `print_stmt`/`print_block_body` just need to pass it down. Add round-trip + golden tests asserting correct nested indentation (the existing proptest generator excludes nested control flow, so this regressed undetected).

#### L3.24 VowViolation JSON assembled without escaping desc/file/binding-names; non-finite floats emit invalid JSON — `HIGH` · ✅ survived cross-check · **Duplicate of #436**
**finder:** `L3-runtime` · **kind:** diagnostics-quality · **verdicts:** 2 · reviewer severity votes: medium×2  
**Files:** `vow-runtime/src/lib.rs:58-123`  
**Design ref:** §6.4 (structured diagnostics / counterexamples as first-class repair inputs); §6.5 (structured outputs, stable error codes)  
**Evidence:** `__vow_violation` builds the structured diagnostic by raw string interpolation:
```rust
json_pairs.push_str(&format!(r#""{name}":{val}"#));
...
let json = format!(r#"{{"error":"VowViolation","vow_id":{vow_id},"blame":"{blame_str}","description":"{desc}","file":"{file}","offset":{offset}{values_json}}}"#);
```
`desc` and `file` come from `CStr::from_ptr(...).to_string_lossy()` (lines 81-90) and are program-controlled source text (contract description strings, file paths). If they contain a `"`, `\`, or control char, the first stderr line is no longer parseable JSON, so an agent's JSON parser breaks exactly on the failure it most needs to read. Separately, `fmt_payload` (lines 58-67) renders float bindings with Rust Display: `TAG_F32 => format!("{}", f32::from_bits(...))`, `TAG_F64 => format!("{}", f64::from_bits(...))`. A NaN/Inf payload emits the bare token `NaN`/`inf` into `"values":{...}` as a numeric position — invalid JSON. The runtime tests never parse `__vow_violation` output with a JSON parser.

**Proposed fix:** Route every JSON string field (name, desc, file, sanitizer details) through a small escaping helper that escapes `\"`, `\\`, and control chars; render non-finite floats as a JSON-safe form (e.g. `"NaN"`/`"Infinity"` strings or null) per a documented convention. Add a subprocess test that feeds a quote/newline desc and a NaN f64 binding and asserts serde_json parses the first stderr line.

#### L3.25 Vec reserve capacity arithmetic overflows: silent under-reserve (OOB write) and non-terminating doubling before allocator guards run — `HIGH` · ✅ survived cross-check · **Duplicate of #435**
**finder:** `L3-runtime` · **kind:** soundness · **verdicts:** 1 · reviewer severity votes: medium×1  
**Files:** `vow-runtime/src/lib.rs:1271-1295`  
**Design ref:** §5.6 / §5.7 (memory model, no UB); CLAUDE.md production-quality (no shortcuts, scalability)  
**Evidence:** `vec_reserve_in_arena_no_null_check` performs unchecked usize arithmetic before any allocator limit is consulted:
```rust
let required = v.len + additional;        // (1) wraps
if required <= v.cap { return; }
let mut new_cap = if v.cap == 0 { VEC_INITIAL_CAP } else { v.cap };
while new_cap < required { new_cap *= 2; } // (2) wraps to 0 -> infinite spin / under-cap
let old_size = v.cap * elem_size;          // (3) wraps
let new_size = new_cap * elem_size;        // (4) wraps -> under-allocated backing
```
(1) With e.g. `v.len=1, additional=usize::MAX`, `required` wraps to 0, `required <= v.cap` is true, the function returns having reserved nothing; a subsequent `vec_push_no_sanitize_in_arena` writes at `v.ptr.add(v.len*elem_size)` past the real backing — a heap OOB write in release. (2) For a large `additional` that doesn't wrap `required`, `new_cap *= 2` can wrap past `required` to 0 and loop forever, or (4) `new_cap*elem_size` wraps to a small `new_size`, under-allocating the chunk while `v.cap` records the huge value, so later element writes overrun. The only overflow guard is inside `__vow_arena_alloc`, reached too late and with the already-wrapped `new_size`. `Vec::reserve(n)` and `String::push_str` (via `__vow_vec_reserve_in_arena(arena, dest, src_len, ...)`) reach this with caller-influenced sizes.

**Proposed fix:** Use checked_add for `required` and checked_mul for `new_cap*elem_size`/`v.cap*elem_size`; guard the doubling loop with checked_mul (break to oom_trap on overflow). On any overflow past the allocator size limit call `oom_trap("Vec::reserve")` before mutating the descriptor. Add a subprocess trap test calling `__vow_vec_reserve_in_arena(arena, vec, usize::MAX, 8, 8)` asserting an OutOfMemory envelope within a bounded timeout.

#### L3.26 Parser misparses `if cond {}` / `while cond {}` / `match x {}` — empty block after bare identifier is swallowed as a struct literal — `HIGH` · ✅ survived cross-check
**finder:** `L3-syntax-rust` · **kind:** bug · **verdicts:** 3 · reviewer severity votes: high×2, medium×1 · **Filed as #604**  
**Files:** `vow-syntax/src/parser/expr.rs:200-212`, `vow-syntax/src/parser/expr.rs:600-609`  
**Design ref:** §5.2 (if/while/match are core forms; canonical single form), §2.4/§7 self-hosting parity; grammar.md "Struct literal names must be PascalCase"  
**Evidence:** In parse_prefix, any identifier is eligible for struct-literal parsing:
```
TokenKind::Ident(name) => {
    self.advance();
    if self.at(&TokenKind::ColonColon) { ... }
    if self.at(&TokenKind::LBrace) && self.looks_like_struct_literal() {
        return self.parse_struct_literal(name, start);
    }
    ...
}
```
and looks_like_struct_literal returns true whenever the token after `{` is `}`:
```
fn looks_like_struct_literal(&self) -> bool {
    match self.tokens.get(self.cursor + 1).map(|t| &t.kind) {
        Some(TokenKind::RBrace) => true,                       // <-- empty `{}` always matches
        Some(TokenKind::Ident(_)) => matches!(... Some(TokenKind::Colon)),
        _ => false,
    }
}
```
parse_if_expr / parse_while_expr / parse_match_expr all parse the condition/scrutinee with parse_expr_inner(0), with no "no-struct-literal" context. I confirmed dynamically:
- `fn f(x: bool) -> i64 { if x {} 0 }` parses the condition as `StructLiteral { name: "x", fields: [] }` and then emits `expected LBrace, found LitInt(0)` (the empty then-block is consumed as the struct body).
- `while x {}` produces the identical failure.
- `match x {}` produces `expected LBrace, found RBrace` + `expected RBrace, found Eof`.
The self-hosted parser does NOT have this bug: compiler/parser.vow gates the struct-literal branch on a PascalCase first byte (`first_byte >= 65 && first_byte <= 90`, line 794), so `if x {}` is accepted there. The Rust bootstrap parser has no such guard, so the two compilers disagree on valid programs (parity break against the byte-identical/self-hosting goal).

**Proposed fix:** Mirror the self-hosted parser: only treat `Ident {` as a struct literal when the identifier is PascalCase (uppercase first byte), matching grammar.md and compiler/parser.vow line 794. Additionally/alternatively parse `if`/`while`/`match` conditions in a no-struct-literal context (Rust's approach). Add regression tests for `if x {}`, `while x {}`, `match x {}`.

#### L3.27 Integer literals exceeding the i64 range are silently truncated with no diagnostic — `HIGH` · ✅ survived cross-check
**finder:** `L3-syntax-rust` · **kind:** bug · **verdicts:** 3 · **Filed as #605**  
**Files:** `vow-syntax/src/lexer.rs:300-330`, `vow-ir/src/lower/mod.rs:603-609`  
**Design ref:** §5.7 (fixed-width integers, no UB, explicit numeric semantics); §2.1/§6.4 (verification trust; diagnostics). N/A as deliberate exclusion.  
**Evidence:** lex_number parses the digits into an i128 and only rejects values that overflow i128:
```
let int_value: i128 = digits.parse().map_err(|_| LexError {
    message: format!("integer literal '{}' out of range", digits),
    ...
})?;
...
Ok(Token::new(TokenKind::LitInt(int_value), ...))
```
The value is carried as i128 all the way to IR lowering, where it is truncated with a plain `as i64`:
```
Lit::Int(v) => ctx.emit(Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(*v as i64), span),
```
Nothing in between range-checks the literal against the target integer width. I confirmed dynamically that `fn g() -> i64 { 99999999999999999999999 }` produces ZERO diagnostics, and (via a standalone i128->i64 probe) `99999999999999999999999 as i64 == 200376420520689663` — a completely different value. Likewise `9223372036854775808` (i64::MAX+1) silently becomes i64::MIN. For an agent-first language whose entire premise is that the compiler proves the program the agent wrote is correct, silently compiling a different numeric value than the source spells out is a correctness/soundness-adjacent hole: ESBMC then verifies the truncated constant, not the written one. This is squarely a literal-width handling bug, distinct from the i64/u64 context issue tracked in #490.

**Proposed fix:** Range-check integer literals against their resolved width at lex/parse/check time. Minimum: in the lexer, reject unsuffixed literals that do not fit in i64 (or in u64 for the documented annotation-coercion case) with an out-of-range diagnostic; for suffixed literals, validate against the suffix's width (e.g. `256u8` must error). Emit ErrorCode for overflow rather than truncating in lowering. Add tests covering i64::MAX+1, values in (i64::MAX, u64::MAX], and per-suffix overflow.

#### L3.28 `vow contracts --verify` exits 0 even when ESBMC reports failed/timeout/unknown/error contracts (both drivers) — `HIGH` · ✅ survived cross-check · **Duplicate of #479**
**finder:** `L3-vsec` · **kind:** soundness · **verdicts:** 1 · reviewer severity votes: low×1  
**Files:** `vow/src/main.rs:9950-9978`, `compiler/main.vow:8973-9106`  
**Design ref:** §2.1; §6.4. cli.md §`vow contracts --verify` and Exit Codes (failures should be non-zero).  
**Evidence:** Rust `run_contracts_command`:
```rust
let mut exit_code = 0;
if verify {
    if find_esbmc().is_none() { ... exit_code = 1; }
    else { update_contract_statuses(&mut entries, ...); }   // mutates entry.status to "failed"/"timeout"/"unknown"/"error"/"skipped" but NEVER touches exit_code
}
...
if exit_code != 0 { std::process::exit(exit_code); }    // lines 9976-9978
```
`exit_code` is set to 1 only in the ESBMC-missing branch. When ESBMC runs and a contract comes back `failed` (or `timeout`/`unknown`/`error`/`skipped`), `exit_code` stays 0, so the process exits 0 while emitting `"status":"failed"` entries. The self-hosted `run_contracts` (compiler/main.vow) has the identical shape: `let mut exit_code: i32 = 0;` then `if !esbmc_exists() { ... exit_code = 1; }` — the `else` branch populates per-contract statuses (`failed`, `timeout`, `unknown`, `error`, `skipped`) but never updates `exit_code`, and the function returns `exit_code` (line 9105) as the process exit code. An agent running `vow contracts --verify` and getting `failed` contracts therefore sees exit 0, contradicting the fail-closed contract surface used everywhere else.

**Proposed fix:** After verification, scan the resulting per-contract statuses; if any is `failed` (and arguably `timeout`/`unknown`/`error`/`skipped`, to mirror `vow verify`'s fail-closed-on-Skipped policy), set `exit_code = 1`. Apply symmetrically in both `vow/src/main.rs:run_contracts_command` and `compiler/main.vow:run_contracts`, and add a test that a program with a violated contract exits non-zero under `vow contracts --verify`.

#### L3.29 Wrapping division `/` traps (SIGFPE) on `i64::MIN / -1` instead of wrapping — `HIGH` · ❌ refuted by cross-check
**finder:** `L3-codegen-rust` · **kind:** bug · **verdicts:** 1 · reviewer severity votes: medium×1  
**Files:** `vow-codegen/src/cranelift_backend.rs:875-878`  
**Design ref:** §5.7 (arithmetic: `/` traps on zero divisor; `+`/`-`/`*`/`/`/`%` are wrapping)  
**Evidence:** WrappingDivI32/I64 lower to a bare Cranelift `sdiv`:
```
Opcode::WrappingDivI32 | Opcode::WrappingDivI64 => {
    let val = builder.ins().sdiv(arg!(0), arg!(1));
    ctx.value_map.insert(inst.id, val);
}
```
Cranelift's `sdiv` is documented (cranelift-codegen-meta shared/instructions.rs:1956-1961) to trap when `x = -2^(B-1), y = -1`, and the x64 lowering (isa/x64/lower.isle:4461-4471) emits `x64_idiv ... TrapCode.INTEGER_OVERFLOW`, i.e. the hardware `idiv` #DE fault. The runtime (vow-runtime/src/lib.rs) installs only a SIGSEGV handler (line 443) — there is no SIGFPE handler — so `i64::MIN / -1` (and `i32::MIN / -1`) under the wrapping operator terminates the process with SIGFPE. Per grammar.md:196 "Wrapping operators silently wrap on overflow" and design §5.7 (`/` traps only on zero divisor); the correct result is `i64::MIN` (matching Rust's `wrapping_div`). The value is fully attacker/data-controlled and not statically excludable, so this is a reachable crash on a documented-as-total operation.

**Proposed fix:** Guard the signed wrapping div: compute `is_min_over_neg1 = (lhs == TYPE_MIN) && (rhs == -1)` and select `lhs` (the wrapped result) in that case, otherwise `sdiv`. Equivalently force the divisor away from -1 via a select before `sdiv`. Apply the same to WrappingDivU64? No — udiv only traps on zero, which is the intended behavior. Only the signed div needs the INT_MIN/-1 wrap.

#### L3.30 Self-hosted lexer silently wraps overflowing integer literals (no out-of-range diagnostic) — `HIGH` · ❌ refuted by cross-check
**finder:** `L3-parser-self` · **kind:** bug · **verdicts:** 3 · reviewer severity votes: low×3  
**Files:** `compiler/lexer.vow:185-201`  
**Design ref:** §7 (self-hosting differential behavior); §2.2 (predict compiler behavior). N/A — pure impl/divergence bug vs Rust frontend.  
**Evidence:** The integer-literal branch accumulates with wrapping arithmetic and never checks for overflow:
```vow
let mut int_val: i64 = 0;
while pos < src_len && is_digit(src.byte_at(pos)) {
    let d: i64 = src.byte_at(pos) - 48;
    int_val = int_val * 10 + d;   // `*` and `+` are WRAPPING in Vow
    pos = pos + 1;
}
```
In Vow, `*`/`+` are wrapping (grammar.md §Wrapping Arithmetic), so a literal like `99999999999999999999999` silently produces a garbage i64 value with no diagnostic. The Rust lexer parses into i128 and emits an error on out-of-range (vow-syntax/src/lexer.rs:300-304: `"integer literal '{}' out of range"`). A self-hosted-compiled program with an over-large literal would miscompile to a different (wrapped) constant than the Rust compiler rejects — a silent frontend divergence that can corrupt verification/codegen inputs.

**Proposed fix:** Detect overflow during accumulation (e.g. flag when `int_val` would exceed i64::MAX before the multiply/add, comparing against a precomputed bound) and route to a structured diagnostic via the parser's diag channel, matching the Rust `IntegerLiteralOutOfRange` behavior. At minimum emit an error rather than silently wrapping.

#### L3.31 Self-hosted C-emitter omits __vow_string_parse_i64_opt/parse_u64_opt from is_known_builtin, silently skipping verification of any pure function that calls String::parse_i64/parse_u64 (Rust emitter models them) — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-cemit-self` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #621**  
**Files:** `compiler/c_emitter.vow:108-164`, `compiler/c_emitter.vow:1474-1768`  
**Design ref:** §6.2 (shared IR/C model across codegen+verify), §7 (self-hosting fixed point: same source must behave identically); contracts.md verification pipeline  
**Evidence:** The self-hosted `is_known_builtin` list (compiler/c_emitter.vow:108-164) enumerates every modelable extern but does NOT include `__vow_string_parse_i64_opt` or `__vow_string_parse_u64_opt`. The Rust emitter's `is_known_builtin` (vow-verify/src/c_emitter.rs:404-468) DOES include them:
  `| "__vow_string_parse_i64_opt"
   | "__vow_string_parse_u64_opt"` (rs lines 450-451).
Consequence chain in the self-hosted modelable gate (compiler/c_emitter.vow:277-281):
  `} else if op == IOP_CALL() {
       if inst.dk == IDATA_CALL_EXTERN() {
           is_known_builtin(inst.ds) && !vec_op_carries_non_scalar(...)`
Returns false for these calls, so `is_modelable` returns false, and the driver `skip_if_non_modelable` (compiler/main.vow:228-235, 274-276) reports a Warning and SKIPS verification of the whole function. These externs are live: lower.vow:2242 and :2249 emit `__vow_string_parse_i64_opt`/`__vow_string_parse_u64_opt` for `str.parse_i64()`/`str.parse_u64()`. Additionally, `emit_string_op` (compiler/c_emitter.vow:1474-1768) has NO case for these names and would fall through to `emit_unmodelled` (which emits `v{id} = __VERIFIER_nondet_long();` into a struct-typed `__vow_option_t v{id}` slot …[truncated]

**Proposed fix:** Add `__vow_string_parse_i64_opt` and `__vow_string_parse_u64_opt` to the self-hosted `is_known_builtin` (compiler/c_emitter.vow:108-164), and add a matching case in `emit_string_op` that mirrors the Rust model (set `.tag` nondet ∈{0,1}, conditionally set `.payload` nondet), exactly as vow-verify/src/c_emitter.rs:1376-1382. Add a regression fixture exercising `String::parse_i64()` in a pure verified function.

#### L3.32 `Ty::I32` is overloaded as both the integer-literal sentinel and the real `i32` type, letting genuine i32 values coerce into u64/i64 without a sign-aware cast — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-checker-rust` · **kind:** soundness · **verdicts:** 1 · **Filed as #637**  
**Files:** `vow-types/src/check.rs:747-748`, `vow-types/src/check.rs:789-809`, `vow-types/src/check.rs:993-1007`, `vow-types/src/check.rs:1476-1496`  
**Design ref:** §5.3 (decidable, nominal type system; no implicit conversions per §4.2); related to issue #490 (i64-in-u64) and #525 (make i32 first-class)  
**Evidence:** Integer literals are typed `Lit::Int(_) => Ty::I32` (check.rs:748), and every integer-coercion rule keys on `Ty::I32` to mean 'untyped literal', e.g. the comparison rule `let coercible = (lhs_ty == Ty::I32 && rhs_ty.is_integer()) || (rhs_ty == Ty::I32 && lhs_ty.is_integer());` (check.rs:790-791) and the call-arg rule `let coercible = arg_ty == Ty::I32 && expected_ty.is_integer() && *expected_ty != Ty::I32;` (check.rs:995-996). But a *genuine* `i32` value (from `let x: i32 = ...`, a param of type `i32`, or a call to `fn f() -> i32`, all of which resolve to `Ty::I32` via env.resolve) is indistinguishable from a literal. Therefore `x < some_u64` (genuine i32 vs u64) is accepted, and IR lowering selects the comparison opcode from the LHS type (`vow-ir/src/lower/mod.rs:757-769` -> `binop_opcode`, which uses signed `*I64` for anything that is not exactly `Ty::U64`, lines 2882-2992), so an i32/u64 ordering compiles to a *signed* comparison over a logically-unsigned operand. Likewise passing an `i32`-returning call where a `u64` parameter is expected type-checks (check.rs:995-1007) and lowering passes the value with no `CastI64ToU64` (the cast is only inserted for `let _: u64 = ...` and `e …[truncated]

**Proposed fix:** Introduce a distinct sentinel for unsuffixed integer literals (e.g. `Ty::IntLit`) separate from `Ty::I32`, and restrict the wide integer coercion to that sentinel only. Genuine `i32` values must then require an explicit `as` cast to reach `i64`/`u64`, and an i32/u64 mix must error like i64/u64 already does — closing the sign-confused-comparison and uncast-argument paths.

#### L3.33 `check_block` discards `Never` from `return EXPR;` statements, mistyping divergent function bodies as `()` — `MEDIUM` · ✅ survived cross-check · **Duplicate of #491**
**finder:** `L3-checker-rust` · **kind:** diagnostics-quality · **verdicts:** 1  
**Files:** `vow-types/src/check.rs:650-661`  
**Design ref:** §5.2 surface form; matches open issue #491  
**Evidence:** `check_block` ignores the type of every non-trailing statement: `for stmt in &block.stmts { self.check_stmt(stmt); }` (the result of `check_stmt` is `()`), then `let ty = match &block.trailing_expr { Some(expr) => self.check_expr(expr), None => Ty::Unit };`. When the body is `{ return 3i32; }` (a semicolon-terminated `return`, parsed as a statement, not a trailing expression) the block is unconditionally typed `Ty::Unit`, even though the `Return` rule (check.rs:1476-1496) already evaluates the expression to `Ty::Never`. `check_fn` then reports `function body has type \`()\` but declared return type is \`i32\`` (check.rs:603-620) for the natural `{ return …; }` style, rejecting valid divergent bodies.

**Proposed fix:** Track divergence in `check_block`: if any statement is `Never`-typed (have `check_stmt` return the statement's type, or detect terminating `Return`/`Break`/`Continue`/diverging-call statements), type the block `Ty::Never` when there is no trailing expression (and treat the rest as dead code), instead of unconditionally `Ty::Unit`.

#### L3.34 HashMap `.get()` is hardcoded to `Ty::I64` regardless of the value type V, diverging from the `.get -> V` spec — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-checker-rust` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #638**  
**Files:** `vow-types/src/check.rs:1062-1072`, `vow-types/src/env.rs:856-867`  
**Design ref:** §5.4 (HashMap is an admitted intrinsic with fixed semantics); docs/spec/grammar.md HashMap `.get -> V`  
**Evidence:** `env.resolve` accepts any V for HashMap: `"HashMap" => ... Ty::Applied(Box::new(Ty::Struct("HashMap")), vec![self.resolve(k)?, self.resolve(v)?])` (env.rs:856-867), so `HashMap<i64, String>` and `HashMap<i64, Point>` type-check. But the method table ignores the receiver's type arguments entirely and returns a fixed scalar: `"get" => Some(Ty::I64)` (check.rs:1067) — contrast the BTreeMap branch which correctly reads `args.get(1)` to type get/insert as `Option<V>` (check.rs:1085-1099). IR lowering matches this with `Opcode::Call, Ty::I64, ... __vow_map_get` (vow-ir/src/lower/mod.rs:2604-2610). docs/spec/grammar.md (line 564) specifies `.get(k) | (K) -> V`. The effect is that for any non-i64 V the get result is the raw 8-byte slot reinterpreted as i64 (a heap pointer for String/struct V), and the program cannot retrieve the stored value at its declared type without a type error elsewhere — silent type unsoundness for `HashMap<_, non-i64>` plus a spec divergence.

**Proposed fix:** Type `HashMap::get`/`insert`/`remove` from the receiver's V type argument (mirroring the BTreeMap branch and the documented signature), and either support non-i64 V end-to-end in lowering/runtime or reject non-i64 V at type-formation with a clear diagnostic (as is already done for BTreeMap keys via BTreeMapKeyTypeMustBeI64).

#### L3.35 Self-hosted EXPR_ASSIGN performs no lhs/rhs type-compatibility check — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-checker-self` · **kind:** soundness · **verdicts:** 1 · **Filed as #644**  
**Files:** `compiler/checker.vow:1473-1479`, `vow-types/src/check.rs:1528-1545`  
**Design ref:** §5.3 type system; §7 multi-compiler agreement  
**Evidence:** Self-hosted EXPR_ASSIGN:
```
let _lhs_tid: i64 = check_expr(e, m, lhs_eid);
let _rhs_tid: i64 = check_expr(e, m, rhs_eid);
return CTY_UNIT();
```
Both operand types are discarded. The Rust checker computes `coercible = (rhs_ty == Ty::I32 && lhs_ty.is_integer()) || lhs_ty == rhs_ty || ... Never` and emits `"assignment type mismatch: left is `{lhs_ty}` but right is `{rhs_ty}`"` (check.rs:1531-1543). Self-hosted accepts `let mut x: i64 = 0; x = String::from(\"y\");` and `x = some_struct;`, which stage0 rejects.

**Proposed fix:** After computing lhs/rhs tids, emit EC_TYPE_MISMATCH unless `is_coercible(e.ts, rhs_tid, lhs_tid)` (or either is opaque).

#### L3.36 Self-hosted checker never reports undefined variables; env_lookup_var silently returns Unit — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-checker-self` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #645**  
**Files:** `compiler/env.vow:381-391`, `compiler/checker.vow:913-926`, `vow-types/src/check.rs:757-775`  
**Design ref:** §6.4 structured diagnostics / fault localization; §2.2 predict compiler behavior; §7 multi-compiler agreement  
**Evidence:** `env_lookup_var` returns `CTY_UNIT()` when no binding matches:
```
while i >= 0 { if e.var_names[i] == name && scope_visible(...) { return e.var_tids[i]; } i = i - 1; }
CTY_UNIT()
```
EXPR_IDENT (checker.vow:913-926) calls `env_lookup_var` and returns the result with no presence test, so a misspelled/undeclared identifier silently types as `()`. `grep` confirms no `undefined variable` diagnostic exists anywhere in checker.vow/env.vow. The Rust checker emits `"undefined variable `{name}`"` with a `did you mean` hint (check.rs:759-773). For an agent-first language whose CEGIS loop depends on structured diagnostics (§6.4), a typo'd variable producing no diagnostic (and a spurious Unit type that can cascade into misleading downstream errors) is a real repair-loop hazard and a stage0 divergence.

**Proposed fix:** Make `env_lookup_var` distinguish 'not found' (e.g. return -1) from a real Unit-typed binding; in EXPR_IDENT, when lookup fails after the const scan, emit EC_TYPE_MISMATCH `undefined variable` and return CTY_UNKNOWN (opaque) to suppress cascades.

#### L3.37 Self-hosted EXPR_FIELD silently returns Never for unknown field or non-struct receiver instead of an error — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-checker-self` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #633**  
**Files:** `compiler/checker.vow:1204-1232`, `vow-types/src/check.rs:1179-1235`  
**Design ref:** §6.4 structured diagnostics; §5.3; §7 multi-compiler agreement  
**Evidence:** Self-hosted EXPR_FIELD only resolves a field when the receiver is `CTY_STRUCT` and the named field exists; in every other case (receiver not a struct, or field name not found in the struct) it falls through to `return CTY_NEVER();` (line 1232). Because Never is opaque, the bogus access is silently accepted and downstream checks are suppressed. The Rust checker emits `"field access on non-struct type `{base_ty}`"` (check.rs:1194-1201) and `"struct `{struct_name}` has no field `{field}`"` with a suggestion hint (check.rs:1206-1226). A typo'd field name in self-hosted code produces no diagnostic and an opaque type, undermining the repair loop and diverging from stage0.

**Proposed fix:** When `recv_tag != CTY_STRUCT` (and not opaque) emit `field access on non-struct type`; when the field loop completes without a match emit `struct X has no field Y`. Return CTY_UNKNOWN rather than CTY_NEVER so the failure is opaque-but-flagged rather than silently divergent.

#### L3.38 Checked division `/!` on `i64::MIN / -1` raw-traps (SIGFPE/SIGILL) instead of structured `ArithmeticOverflow` — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-codegen-rust` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #599**  
**Files:** `vow-codegen/src/cranelift_backend.rs:925-939`  
**Design ref:** §5.7 (`/!` checked: abort on zero divisor OR overflow)  
**Evidence:** CheckedDivI32/I64 only check the zero-divisor case, then emit `sdiv`:
```
Opcode::CheckedDivI32 | Opcode::CheckedDivI64 => {
    let cl_ty = ir_ty_to_cranelift(inst.ty).unwrap_or(types::I64);
    let zero = builder.ins().iconst(cl_ty, 0);
    let is_zero = builder.ins().icmp(IntCC::Equal, arg!(1), zero);
    emit_overflow_check(builder, is_zero, ctx)?;
    let val = builder.ins().sdiv(arg!(0), arg!(1));
    ...
}
```
For `i64::MIN / -1` the explicit `is_zero` check passes (divisor != 0), then `sdiv` traps via x64 `idiv` `TrapCode.INTEGER_OVERFLOW` (lower.isle:4471) → SIGFPE. Design §5.7 states `/!` is "checked division ... (abort on zero divisor or overflow)", and the intended abort path is `__vow_arithmetic_overflow` (vow-runtime/src/lib.rs:170, emits `{"error":"ArithmeticOverflow"}`). The INT_MIN/-1 overflow case bypasses that handler entirely, producing a raw fatal signal with no structured diagnostic — defeating the CEGIS-facing diagnostic contract for the one overflow case division can hit.

**Proposed fix:** Extend the checked-div guard to also detect `lhs == TYPE_MIN && rhs == -1` and route it through `emit_overflow_check` (the same path the zero check uses) so it calls `__vow_arithmetic_overflow` and emits the structured `ArithmeticOverflow` envelope, rather than relying on the hardware `idiv` fault.

#### L3.39 Checked remainder `%!` silently returns 0 on `i64::MIN % -1` instead of aborting with `ArithmeticOverflow` — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-codegen-rust` · **kind:** soundness · **verdicts:** 1 · reviewer severity votes: low×1 · **Filed as #634**  
**Files:** `vow-codegen/src/cranelift_backend.rs:940-954`  
**Design ref:** §5.7 (`%!` checked: abort on zero divisor OR overflow)  
**Evidence:** CheckedRemI32/I64 only guard the zero divisor, then emit `srem`:
```
Opcode::CheckedRemI32 | Opcode::CheckedRemI64 => {
    let cl_ty = ir_ty_to_cranelift(inst.ty).unwrap_or(types::I64);
    let zero = builder.ins().iconst(cl_ty, 0);
    let is_zero = builder.ins().icmp(IntCC::Equal, arg!(1), zero);
    emit_overflow_check(builder, is_zero, ctx)?;
    let val = builder.ins().srem(arg!(0), arg!(1));
    ...
}
```
Unlike `sdiv`, Cranelift's `srem` does NOT trap on `INT_MIN % -1` — its x64 lowering uses a checked sequence (isa/x64/inst/emit.rs:229-235: "Check if the divisor is -1 ... otherwise the divisor is -1 and the result is always 0") that returns 0 without faulting. Design §5.7 says `%!` is "checked ... remainder (abort on zero divisor or overflow)". So `INT_MIN % -1` under `%!` silently produces 0 and continues, rather than aborting with `ArithmeticOverflow`. The checked operator's contract ("abort on overflow") is therefore not enforced for remainder. (Mathematically the remainder of INT_MIN/-1 is 0, but the spec explicitly classifies this as an overflow condition that must abort under the checked operator.)

**Proposed fix:** Before `srem`, detect `lhs == TYPE_MIN && rhs == -1` and route it through `emit_overflow_check` so `%!` aborts via `__vow_arithmetic_overflow`, matching the documented checked-operator semantics (or, if the project decides INT_MIN%-1==0 is acceptable for `%!`, amend §5.7 to say `%!` only aborts on zero divisor — but code and spec must agree).

#### L3.40 U64 vow-violation captures are emitted with the i32 tag and a zero payload (wrong counterexample value) — `MEDIUM` · ✅ survived cross-check · **Duplicate of #439**
**finder:** `L3-codegen-rust` · **kind:** diagnostics-quality · **verdicts:** 1 · reviewer severity votes: low×1  
**Files:** `vow-codegen/src/cranelift_backend.rs:1700-1709`, `vow-codegen/src/cranelift_backend.rs:1801-1811`  
**Design ref:** §6.4 (counterexamples/blame as first-class repair inputs)  
**Evidence:** `tag_for_ir_ty` has no U64 case and falls through to tag 0 (= TAG_I32 in vow-runtime/src/lib.rs:43):
```
fn tag_for_ir_ty(ty: IrTy) -> u8 {
    match ty { IrTy::I32 => 0, IrTy::I64 => 1, IrTy::F32 => 2, IrTy::F64 => 3, IrTy::Bool => 4, _ => 0 }
}
```
and the payload match also has no U64 arm, so a U64 binding's payload is hard-coded to 0:
```
let payload: Value = match ir_ty {
    IrTy::I32 => builder.ins().sextend(types::I64, *cl_val),
    IrTy::I64 => *cl_val,
    ... IrTy::Bool => *cl_val,
    _ => builder.ins().iconst(types::I64, 0),  // U64 lands here
};
```
U64 bindings are NOT filtered out by the capture filter (lines 1296-1302 only drop Ptr/LinearPtr/Unit), so a `u64` free variable in a failed predicate (e.g. `requires: n > 0` with `n: u64`) is reported in the VowViolation `values` map as `0` (and via the wrong tag the runtime `fmt_payload` would print it as signed i32 even if a real value were stored). The runtime even has a correct `TAG_U64 = 5` path (lib.rs:48,64) that the Rust backend never emits. Since `values` drives CEGIS repair (CLAUDE.md VowViolation shape), the agent gets a wrong counterexample for u64 variables.

**Proposed fix:** Add `IrTy::U64 => 5` to `tag_for_ir_ty` and `IrTy::U64 => *cl_val` to the payload match (the I64 bit pattern is the correct u64 payload). This matches the runtime's existing TAG_U64 handling. Already filed as #439 against the self-hosted shim with an explicit note to fix the Rust backend mirror; this is that mirror.

#### L3.41 collect_extern_syms incurs O(instructions^2) per function on every native build via clif_find_inst linear scans (and full block walks for Phi receivers) — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-codegen-self` · **kind:** optimization · **verdicts:** 1 · **Filed as #619**  
**Files:** `compiler/clif.vow:13-28`, `compiler/clif.vow:135-192`, `compiler/clif.vow:329-358`  
**Design ref:** §7 (self-hosting is the hardest in-project workload); CLAUDE.md Production Quality: 'Scalability is a requirement. Data structures, algorithms, and compiler passes must be chosen for reasonable asymptotic behavior, not just correctness on small inputs.'  
**Evidence:** `collect_extern_syms` walks every instruction of every function and, for every routed receiver-based extern (`__vow_vec_push`, `__vow_vec_push_val`, `__vow_string_push_str`, `__vow_string_push_byte`, `__vow_map_insert`), calls `clif_routed_extern_symbol` which unconditionally calls `clif_receiver_region`:

  `if sym == String::from("__vow_vec_push") {\n        let rr2: i64 = clif_receiver_region(f, inst);` (clif.vow:216-217)

`clif_receiver_region` then resolves the receiver with a full linear scan:

  `let receiver: IrInst = clif_find_inst(f, inst.args[0]);` (clif.vow:190)

where `clif_find_inst` is O(blocks * insts):

  `while bi < blocks.len() { ... while ii < insts.len() { if insts[ii].id == id { return insts[ii]; } ...` (clif.vow:16-24)

Worse, when the receiver is a Phi (extremely common: the self-hosted compiler's many `while`-loop bodies push into vectors threaded through loop-header phis), `clif_value_receiver_region` walks ALL blocks/insts of the function looking for matching upsilons, per call:

  `if receiver.op == IOP_PHI() { let blocks: Vec<IrBlock> = f.blocks; ... while bi < blocks.len() { let insts: Vec<IrInst> = blocks[bi].insts; ... while ii < insts.len() {` (clif …[truncated]

**Proposed fix:** Mirror the Rust backend: in `collect_extern_syms`, build a per-function `HashMap<i64, IrInst>` (id -> inst) and a `HashMap<i64, Vec<i64>>` (phi-target -> upsilon arg0 sources) once per function, then have `clif_find_inst`/`clif_value_receiver_region`/`clif_receiver_region` take those maps instead of re-scanning `f.blocks`. This makes the pass O(insts) per function and matches the shim's own `inst_region_for_value` design (which already uses these maps at codegen time, lib.rs:1205-1229).

#### L3.42 Self-hosted checker omits operand validation for comparison (== != < <= > >=) and logical (&& ||) operators; Rust rejects ill-typed operands — `MEDIUM` · ✅ survived cross-check · **Duplicate of #490**
**finder:** `L3-divergence` · **kind:** design-divergence · **verdicts:** 1  
**Files:** `compiler/checker.vow:934-939`, `vow-types/src/check.rs:789-831`  
**Design ref:** §7 (differential-verification trust); §4.1  
**Evidence:** Self-hosted unconditionally returns BOOL with zero operand checking:
```vow
        if op == BINOP_AND() || op == BINOP_OR() {
            return CTY_BOOL();
        }
        if op == BINOP_EQ() || op == BINOP_NE() || op == BINOP_LT() || op == BINOP_LE() || op == BINOP_GT() || op == BINOP_GE() {
            return CTY_BOOL();
        }
``` (checker.vow:934-939). The Rust compiler, for Eq/Ne/Lt/Le/Gt/Ge, emits TypeMismatch `comparison operands have different types` unless equal/coercible (check.rs:789-809), and for And/Or emits `logical operator requires bool` unless each operand is Bool/Never (check.rs:813-830). Concrete divergent inputs: (1) `fn f() -> bool { 5 == true }` — self-hosted accepts (returns BOOL), Rust rejects (`comparison operands have different types: i64 and bool`). (2) `fn f() -> bool { 1 && 2 }` — self-hosted accepts (returns BOOL), Rust rejects (`logical operator requires bool, found i64`). (3) `fn f(a: i64, b: u64) -> bool { a < b }` — self-hosted accepts, Rust rejects.

**Proposed fix:** In checker.vow EXPR_BINOP, before returning CTY_BOOL for comparisons, replicate Rust's coercible check (lhs==rhs or one side I32-literal) and emit an error otherwise; for AND/OR, require both operands to be CTY_BOOL (or NEVER) and emit `logical operator requires bool` otherwise.

#### L3.43 Self-hosted checker silently accepts unknown/non-struct field access (returns CTY_NEVER) where Rust emits TypeMismatch — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-divergence` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #633**  
**Files:** `compiler/checker.vow:1204-1233`, `vow-types/src/check.rs:1179-1238`  
**Design ref:** §7 (differential trust); §6.4 (structured diagnostics); §4.4 (local reasoning)  
**Evidence:** Self-hosted EXPR_FIELD falls through to `return CTY_NEVER();` (checker.vow:1232) for BOTH a non-struct receiver and a struct receiver whose field name is not found (the while loop at 1221-1229 finds nothing and exits). No diagnostic is emitted. Rust's FieldAccess emits TypeMismatch `field access on non-struct type` (check.rs:1194-1200), `struct X has no field Y` (check.rs:1220-1226), and `unknown struct X` (check.rs:1230-1234). Because CTY_NEVER is opaque/coercible to anything (types.vow:216-232, is_coercible returns true when from==NEVER), the self-hosted then accepts code the Rust rejects. Concrete divergent input: `struct S { a: i64 } fn f(s: S) -> i64 { s.nope }` — self-hosted: field not found → CTY_NEVER → coercible to i64 → accepted with NO error; Rust: `struct S has no field nope` (rejected). This is exactly the agent-typo class of bug the diagnostics are supposed to catch.

**Proposed fix:** In checker.vow EXPR_FIELD, when the receiver is not a struct, or the struct has no matching field, emit a TypeMismatch / EC_UNKNOWN_FIELD diagnostic (mirroring Rust) before returning a recovery type; do not return CTY_NEVER silently.

#### L3.44 Stage 0 (Rust) check_block ignores Never propagated by `return EXPR;` while self-hosted propagates CTY_NEVER, causing accept/reject divergence on divergent bodies — `MEDIUM` · ✅ survived cross-check · **Duplicate of #491**
**finder:** `L3-divergence` · **kind:** design-divergence · **verdicts:** 1  
**Files:** `vow-types/src/check.rs:650-661`, `compiler/checker.vow:1420-1432`  
**Design ref:** §7 (differential trust); §2.1  
**Evidence:** Rust check_block returns only the trailing-expr type (or Unit) and never observes Never from inner `return` statements:
```rust
        let ty = match &block.trailing_expr {
            Some(expr) => self.check_expr(expr),
            None => Ty::Unit,
        };
``` (check.rs:655-660). The self-hosted EXPR_RETURN correctly evaluates to CTY_NEVER (checker.vow:1432) and Never is coercible to the function return type. This is the divergence captured by issue #491; reported here as a confirmed cross-compiler accept/reject difference for divergent function bodies (e.g. a function whose only statement is `return foo();` with no trailing expr).

**Proposed fix:** Per #491, make Rust check_block (and check_stmt for `return`/`break`) propagate Ty::Never so divergent bodies type the same in both compilers.

#### L3.45 Self-hosted driver never rejects unknown flags or invalid flag values; bad `--mode`/`--debug-trace`/`--encoding`/`--solver`/`--max-k-step` silently fall back to defaults or are forwarded to ESBMC — `MEDIUM` · ✅ survived cross-check · **Duplicate of #580**
**finder:** `L3-driver-self` · **kind:** design-divergence · **verdicts:** 1 · reviewer severity votes: low×1  
**Files:** `compiler/main.vow:15-36`, `compiler/main.vow:691-703`, `compiler/main.vow:726-737`  
**Design ref:** §2.2, §6.5 (predictable, self-describing tooling); cli.md flag tables  
**Evidence:** `has_flag`/`get_flag_arg` (compiler/main.vow:15-36) only test exact-string membership; nothing enumerates the legal flag set, so any unknown flag is silently ignored. Invalid *values* also fall back silently: `run_build` maps `--mode` with `else { 0 }` (compiler/main.vow:691-697) so `--mode bogus`/`--mode typo` becomes release, and `--debug-trace` with `else { 0 }` (698-703) so a typo becomes `off`. `--solver`/`--encoding` are not validated against their enum — only the special `encoding == "ir"` case is checked (726-737); any other value (e.g. `--solver bogus`, `--encoding bogus`) is forwarded verbatim to ESBMC as `--bogus` by verifier.vow append_solver_args, and `--max-k-step` is passed through as a raw string. The Rust driver uses clap `value_enum` for mode/solver/encoding/debug-trace and a `u32` for max_k_step (vow/src/main.rs:108-127, 158-177), so it rejects unknown flags and invalid enum/numeric values with a clean error before doing any work. Result: on the self-hosted compiler an agent's flag typo is silently accepted and the wrong (or default) behavior runs, with no diagnostic — directly at odds with §2.2 ("predict compiler and verifier behavior") and §6.5.

**Proposed fix:** Add an allow-list validation pass over argv per subcommand (matching the Rust clap surface), erroring with a non-zero exit on unknown flags. Additionally validate `--mode`/`--debug-trace`/`--solver`/`--encoding` against their enums and `--max-k-step`/`--timeout`/`--verify-jobs` as numerics, emitting an explicit error instead of a silent fallback. This is the parity work tracked by #580.

#### L3.46 `vow contracts --verify` exits 0 even when a contract is `failed`/`timeout`/`unknown`/`skipped` — `MEDIUM` · ✅ survived cross-check · **Duplicate of #479**
**finder:** `L3-driver-self` · **kind:** diagnostics-quality · **verdicts:** 1 · reviewer severity votes: low×1  
**Files:** `compiler/main.vow:8972-9028`, `compiler/main.vow:9105-9106`  
**Design ref:** §6.4/§6.5 (structured, stable, fail-closed gating); cli.md §`vow contracts` (only documents fail-closed for ESBMC-missing)  
**Evidence:** In `run_contracts`, `exit_code` is initialized to 0 and only ever set to 1 in the ESBMC-missing branch (compiler/main.vow:8973-8981):
```
    let mut exit_code: i32 = 0;
    if do_verify {
        if !esbmc_exists() { ... exit_code = 1; }
        else { ... per-contract statuses set to proven/failed/timeout/unknown/error/skipped ... }
    }
```
The else-branch (8982-9027) can assign `failed` (9009), `timeout` (9014), `unknown` (9011/9016), `error` (9018), or `skipped` (8993) to a contract's status, but never touches `exit_code`. The function then returns `exit_code` (9105) — i.e. 0 — even though a contract counterexample was found. The build/verify gates fail closed on these outcomes (run_build/run_verify return 1), but `contracts --verify` reports a counterexample in JSON while signalling success at the shell level. An agent or CI step that keys off the exit code of `vow contracts --verify` will treat a falsified contract as a pass. (The Rust `run_contracts_command` has the same gap, vow/src/main.rs:9950-9978, so this is a shared bug, not a self-hosted-only regression.) Tracked as #479.

**Proposed fix:** After computing statuses, set `exit_code = 1` whenever any contract status is `failed` (and consider `timeout`/`unknown`/`error`/`skipped`, matching build/verify fail-closed policy). Mirror the same change in the Rust `run_contracts_command`. Per #479, at minimum `failed` must produce a non-zero exit.

#### L3.47 vow predicate purity check silently ignores `.unwrap()` (panic) — collected into `panic_exprs` but never inspected — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-effects-rust` · **kind:** bug · **verdicts:** 1 · **Filed as #659**  
**Files:** `vow-types/src/effects.rs:228-271`  
**Design ref:** §5.5 ('Expressions inside vow clauses must be pure. Contract checking must not itself perform I/O or hidden state changes.'); grammar.md:669-671  
**Evidence:** `check_vow_purity` collects BOTH effectful calls and panic sites, but only ever inspects `calls`:

```rust
let mut calls = Vec::new();
let mut panic_exprs = Vec::new();
collect_calls_in_expr(expr, &mut calls, &mut panic_exprs);

for (callee_expr, callee_name) in &calls {        // line 245
    if let Some(sig) = env.lookup_fn(callee_name)
        && !sig.effects.is_empty() { ... emit EffectViolation ... }
}
```

`panic_exprs` (populated only for `.unwrap()` in `collect_calls_in_expr`, lines 44-46) is collected on line 243 and then never read again in the function — it is dead. Consequently a contract predicate such as `requires: opt.unwrap() > 0` or `ensures: result.unwrap() == x` passes the purity check unflagged. The body scan in `check_fn_effects` does not catch it either, because that scan only walks `fn_def.body` (line 161: `collect_calls_in_block(&fn_def.body, ...)`), never the vow block. So a `.unwrap()` embedded in a `requires`/`ensures`/`invariant` is checked by neither path: it is not reported as an impure predicate, and it does not force the enclosing function to declare `[panic]`. `.unwrap()` carries the `[panic]` effect (grammar.md:591) and introduces a control-flow di …[truncated]

**Proposed fix:** In `check_vow_purity`, after the `calls` loop, iterate `panic_exprs` and emit an `EffectViolation` (Blame::Callee) for each, since a panicking `.unwrap()` is not a pure predicate. The hint should direct the author to replace `.unwrap()` in the predicate with a total expression (e.g. an explicit `match`/`is_some` guard). Add a regression test analogous to `vow_purity_impure_predicate_emits_violation` using `body_with_unwrap`-style input.

#### L3.48 Effect checker makes `[io]` subsume `[read]` and `[write]`, contradicting design + grammar + agent-facing --help that say effects are independent — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-effects-rust` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #635**  
**Files:** `vow-types/src/effects.rs:6-14`, `vow-types/src/effects.rs:376-400`  
**Design ref:** §5.5; §4.5; grammar.md:651  
**Evidence:** `effect_covered` treats `[io]` as a superset of `[read]` and `[write]`:

```rust
fn effect_covered(declared: &[Effect], needed: &Effect) -> bool {
    if declared.contains(needed) { return true; }
    if (needed == &Effect::Read || needed == &Effect::Write) && declared.contains(&Effect::IO) {
        return true;
    }
    false
}
```

and tests `io_subsumes_read` (line 377-383) and `io_subsumes_write` (line 386-400) assert NO error when an `[io]`-only function calls a `[read]`/`[write]` function (`assert!(emitter.0.is_empty())`). The self-hosted compiler implements the same subsumption (compiler/env.vow:58-87: `if has_eff_bit(needed, eff_read) && !io_decl && ... return false`). But the authoritative design doc states the opposite in §5.5: 'Each effect is independent — `io` is not a superset of `read` or `write`.' grammar.md:651 repeats it verbatim, and the compiler's own generated agent-facing `--help` prints the same sentence (compiler/main.vow:3085 and :6218: 'Each effect is independent -- `io` is not a superset of `read` or `write`.'). An agent that models effects per the documented/`--help` contract will write a function annotated `[io]` and the checker will silently permit it …[truncated]

**Proposed fix:** Resolve the contradiction in ONE direction and align all four sites. Either (a) remove the IO→Read/Write subsumption from `effect_covered` (Rust) and the `io_decl` short-circuits in `compiler/env.vow:effect_covered`, delete/invert tests `io_subsumes_read`/`io_subsumes_write`, keeping the design/grammar as written; or (b) if subsumption is genuinely intended, update docs/vow_design.md §5.5, grammar.md:651, and the generated `--help` text (regenerate via scripts/generate_help.py) to state that `[io]` implies `[read]` and `[write]`. Do not leave the implemented semantics and the agent-facing contract in conflict.

#### L3.49 IR validator (validate/validate_function) is dead code — never invoked anywhere in the build/verify/codegen pipeline — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-ir-lower-rust` · **kind:** diagnostics-quality · **verdicts:** 1 · reviewer severity votes: low×1 · **Filed as #630**  
**Files:** `vow-ir/src/validator.rs:27-89`, `vow-ir/src/lib.rs:21-21`  
**Design ref:** §6.3 (IR: 'Phi/Upsilon style SSA handling', 'explicit contract obligations in IR'); §6.4 (structured diagnostics). N/A — pure impl/wiring gap  
**Evidence:** `vow-ir/src/lib.rs:21` exports `pub use validator::{ValidationError, ValidationResult, validate, validate_function};`. A repo-wide grep for call sites returns ONLY the internal recursion inside the module itself: `grep -rn "ir::validate|validate_function|validator::validate"` (excluding tests/pub-use/defs) yields a single hit: `vow-ir/src/validator.rs:30: errors.extend(validate_function(func).errors);`. No call appears in the `vow` CLI driver, `vow-codegen`, or `vow-verify`. The only other `.validate()` calls in the tree are unrelated (`config.validate()` in vow/src/main.rs:87 and `SolverStrategy::validate()` in vow-verify). Thus the IR structural sanity checks (block termination, single terminator, Phi/Upsilon pairing, linearity, undefined refs) never run on real lowered IR — malformed IR proceeds straight to codegen and verification with no guard.

**Proposed fix:** Invoke `vow_ir::validate(&module)` after `lower_module` in the CLI driver (before codegen + verification dispatch), surfacing any `ValidationError` as a structured diagnostic / ICE. Before wiring it in, fix the validator's own defects (see sibling findings) so it does not reject valid IR. Add an integration test that lowers a multi-block function and asserts `validate` passes.

#### L3.50 Validator UndefinedInstRef check is per-block only — would reject all valid cross-block SSA references (Upsilon args, branch/return args from dominating blocks) — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-ir-lower-rust` · **kind:** bug · **verdicts:** 1 · reviewer severity votes: low×1 · **Filed as #631**  
**Files:** `vow-ir/src/validator.rs:123-161`  
**Design ref:** §6.3 (Pizlo-style SSA with Phi/Upsilon; cross-block value references are intrinsic to this IR). N/A — pure impl bug  
**Evidence:** In `validate_block`, the reference set is built from a SINGLE block: `let inst_ids: HashSet<InstId> = block.insts.iter().map(|i| i.id).collect();` (line 124), and every operand is checked against only that set: `for &arg in &inst.args { if !inst_ids.contains(&arg) { errors.push(ValidationError::UndefinedInstRef { user: inst.id, referenced: arg }); } }` (lines 136-143). Lowered Vow IR routinely references values defined in dominating blocks: e.g. the while-loop natural-exit Upsilon emitted in `cond_block` references the header Phi (`vow-ir/src/lower/mod.rs:1254-1262`, `vec![header_phi]`), and break/back-edge Upsilons reference loop-body values targeting exit/header Phis in other blocks. Such legitimate IR would be flagged `UndefinedInstRef`. The flaw is masked only because the validator is never run (sibling finding) and because every existing validator test uses single-block functions with same-block or empty args (validator.rs:204-456).

**Proposed fix:** Build the defined-id set across the WHOLE function (all blocks), not per-block, and validate operand references against that function-wide set. A proper SSA check would additionally verify dominance (def dominates use) and exempt Phi operands, which are merged via Upsilon rather than directly referenced; at minimum switch to function-scope id collection so valid cross-block refs are not rejected.

#### L3.51 collect_vars_in_expr drops free variables inside MethodCall / Index / Cast / Match predicate sub-expressions — VowViolation `values` omits them — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-ir-lower-rust` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #632**  
**Files:** `vow-ir/src/lower/vow.rs:48-129`  
**Design ref:** §6.4 ('ESBMC counterexamples as first-class repair inputs'; structured diagnostics). CLAUDE.md: VowViolation `values` 'contains runtime values of all free variables in the predicate'  
**Evidence:** `collect_vars_in_expr` matches `Ident`, `Result`, `BinaryOp`, `UnaryOp`, `Call`, `Block`, `If`, `FieldAccess`, then `ExprKind::Lit(_) | ExprKind::Break{..} | ExprKind::Return{..} => {}` and a catch-all `_ => {}` (lines 126-127). It does NOT recurse into `ExprKind::MethodCall`, `ExprKind::Index`, `ExprKind::Cast`, `ExprKind::Match`, `ExprKind::Question`, `ExprKind::Tuple`, `ExprKind::StructLiteral`, or `ExprKind::EnumConstruct` (all confirmed present in vow-syntax/src/ast.rs:264-332). Common, spec-endorsed contract predicates use exactly these forms — `docs/spec/contracts.md` shows `requires: i < v.len()` (line 125), `requires: v.len() <= 128` (line 44), and `as u64` casts in contracts (line 326). For `i < v.len()`, the `BinaryOp` arm captures `i` but the `MethodCall` receiver `v` is reached only via the catch-all, so `v` is silently omitted from the vow's `bindings`. These bindings drive the runtime `values` map in the `VowViolation` JSON (consumed by codegen at vow-codegen/src/cranelift_backend.rs:1996 `for (idx, (name, _)) in vow_entry.bindings.iter()...`). Result: a debug-mode contract failure on `i < v.len()` reports `i` but not `v`, degrading the structured counterexample the …[truncated]

**Proposed fix:** Add recursion arms in `collect_vars_in_expr` for `MethodCall` (receiver + args), `Index` (base + index), `Cast` (expr), `Question` (expr), `Match` (scrutinee + arm bodies), `Tuple`, `StructLiteral` (field exprs), and `EnumConstruct` (field exprs). Mirror the existing recursive arms. Add a unit test asserting that `requires: i < v.len()` yields bindings for both `i` and `v`.

#### L3.52 collect_assigned_in_expr (self-hosted) omits BinaryOp/UnaryOp/Call/Method/Return and Assign-RHS / if-while condition recursion that the Rust reference traverses, dropping mutation Phis — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-ir-lower-self` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #655**  
**Files:** `compiler/lower.vow:732-791`  
**Design ref:** §7 (self-hosting fixed point with Rust), §6.4 (contract checks must read correct values); soundness-adjacent because stale values feed downstream vow obligations  
**Evidence:** The self-hosted mutation collector only descends a narrow set of expression forms:
```
fn collect_assigned_in_expr(ctx, eid, out) {
    ... if tag == EXPR_ASSIGN() { /* checks LHS ident only; never recurses into RHS */ }
    else if tag == EXPR_BLOCK() { ... }
    else if tag == EXPR_IF() { collect then-block + else-expr; /* NOT the condition */ }
    else if tag == EXPR_WHILE() { collect body; /* NOT the condition */ }
    else if tag == EXPR_LOOP() {...} else if tag == EXPR_FOR() {...}
    else if tag == EXPR_MATCH() { collect arm bodies }
    /* no EXPR_BINOP, EXPR_UNOP, EXPR_CALL, EXPR_METHOD, EXPR_RETURN cases */
}
```
The Rust reference (vow-ir/src/lower/mod.rs:492-569) additionally recurses into Assign RHS (`collect_assigned_in_expr(rhs,...)` line 500), the If condition (line 515), the While condition (line 529), BinaryOp lhs+rhs (553-555), UnaryOp operand (557), and Return value (558-560). EXPR tags BINOP=5, UNOP=6, CALL=7, METHOD=8 exist (compiler/ast.vow:8-11). The same self-hosted file already knows how to traverse these forms — `collect_free_vars_in_expr` (lower.vow:3040-3065) handles EXPR_BINOP/EXPR_UNOP/EXPR_CALL/EXPR_CAST/EXPR_IF-cond — confirming the omission in col …[truncated]

**Proposed fix:** Bring collect_assigned_in_expr to parity with the Rust collect_assigned_in_expr: add recursion for EXPR_ASSIGN RHS, EXPR_BINOP (both operands), EXPR_UNOP operand, EXPR_CALL/EXPR_METHOD arguments+receiver, EXPR_RETURN value, and the EXPR_IF/EXPR_WHILE conditions. Reuse the traversal already present in collect_free_vars_in_expr. Add a tests/run case with an assignment nested in a call-argument/binary-op inside an outer branch and assert the post-branch value.

#### L3.53 Internal-Call and extern-store-into-parameter heap results over-approximated to Root (memory leak / transient values pinned for process lifetime) — `MEDIUM` · ✅ survived cross-check · **Duplicate of #407**
**finder:** `L3-ir-region-rust` · **kind:** design-divergence · **verdicts:** 1  
**Files:** `vow-ir/src/region.rs:1511-1528`, `vow-ir/src/region.rs:1919-1937`  
**Design ref:** §4.4 (Accidental root-region placement is prevented by §4.4: region inference rejects rather than over-approximates to root), §6.3, §5.6  
**Evidence:** Two places rewrite a precisely-inferred block/caller region up to Root. (1) Every internal `Call` heap producer is collapsed to Root regardless of its LUB:
```rust
// region.rs:1511-1528
if inst.opcode == Opcode::Call
    && !matches!(&inst.data, InstData::CallExtern(sym) if heap_producing_extern(sym))
{
    if matches!(region_id, RegionId::Block(_) | RegionId::Caller(_)) {
        ...
        region_id = RegionId::Root;
    }
}
```
(2) For extern store edges whose target traces to a parameter, the stored element's marker is forced to Root rather than the precise caller store-target slot:
```rust
// region.rs:1922-1927
let marker = if trace_param(target_id, inst_lookup).is_some() {
    MustOutliveMarker::Root
} else {
    target_region_marker(target_id, inst_lookup, summaries)
};
add_marker(must_outlive, source_id, marker);
```
Root never closes (§6.2), so a transient Vec/String/HashMap element returned from an internal call, or pushed into a parameter Vec, is retained for the whole process lifetime instead of being reclaimed when its block/caller arena closes. Not a soundness hole (Root is always live) but violates the design's reclamation goal and the §4.4 'reject rather than ove …[truncated]

**Proposed fix:** Per issue #407, narrow the internal-Call rewrite so it only fires for the genuinely-unsupported aggregate-projection-and-repackage shape it cites, not for every internal-Call heap producer; and route extern-store-into-parameter element sources through the precise `CallerStoreTarget(p)` slot (as the internal AliasOf path already does) instead of Root, once slot-aware codegen for the push element is wired.

#### L3.54 Self-hosted dominator-tree construction uses an incremental-LCA fixpoint instead of region.rs's dominator-set intersection, with no test pinning Rust/self-hosted parity — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-ir-region-self` · **kind:** design-divergence · **verdicts:** 1 · reviewer severity votes: low×1 · **Filed as #658**  
**Files:** `compiler/region.vow:3291-3337`, `vow-ir/src/region.rs:665-753`  
**Design ref:** §5.6 region placement/escape analysis is compiler-owned; §2.4/§7 self-hosting requires the Vow impl to match the Rust impl; §10 open question "How precise does region escape analysis need to be"  
**Evidence:** The two compilers compute the block dominator tree (the foundation for every region-placement LCA and is_ancestor close-marker decision) by DIFFERENT algorithms. Rust `dominance_parent` computes true dominator SETS via iterative intersection and then picks idom as the strict dominator with the largest dom-set:
```
new_dom = dom[first_pred]; for pred in rest { new_dom = new_dom ∩ dom[pred]; } new_dom.insert(node);
... idom = strict_doms.max_by_key(|c| dom[c].len())
```
Self-hosted `block_tree_dominance_parent` instead iterates per-edge `parent[succ] = (old==-2 ? pred : block_tree_parent_lca(old_parent, pred))` to a fixpoint, where `block_tree_parent_lca` walks the *partially-built parent tree*:
```
let next_parent: i64 = if old_parent == -2 { pred } else { block_tree_parent_lca(parent, old_parent, pred) };
```
This is the Cooper-Harvey-Kennedy shape, which converges to the same idom tree for reducible CFGs *provided* the forward graph is a DAG (both build it by stripping back/on-stack edges) and it is iterated to fixpoint over all edges. For structured Vow CFGs this should coincide, but the equivalence is unproven here and there is NO regression test that compares the two parents: ` …[truncated]

**Proposed fix:** Add a differential test (the deferred work in region_summary_equivalence.rs): emit per-inst regions and RegionOpen/RegionClose markers from both compilers on a corpus of branchy/loopy fixtures and assert byte-identical placement. If full diffing is too heavy, at minimum add a focused unit test on `build_block_parent` vs the Rust `BlockTree::dominance_parent` over hand-built irreducible-ish CFGs (nested loops, shared join with unequal predecessor depths). Until parity is mechanically verified, treat the self-hosted dominator pass as a divergence to reconcile.

#### L3.55 Linear tracker ignores branch divergence (return/?/break): early-return-then-consume valid programs get a spurious LinearTypeViolation — `MEDIUM` · ✅ survived cross-check · **Duplicate of #491**
**finder:** `L3-linear-rust` · **kind:** bug · **verdicts:** 1  
**Files:** `vow-types/src/linear.rs:173-177`, `vow-types/src/linear.rs:350-388`  
**Design ref:** §2.2 (agents must predict compiler behavior / repair via structured feedback) — a false reject blocks valid CEGIS repairs; pure impl bug in the linear pass  
**Evidence:** `check_if_branches` merges the then/else trackers with no notion of a diverging branch. The `Return` arm only consumes its value, it never marks the path dead:
```rust
ExprKind::Return { value } => {
    if let Some(v) = value {
        check_expr(v, tracker, env, file, emitter, true);
    }
}
```
In the no-else path of `check_if_branches`, a then-branch that consumed `h` flips `tracker` to MaybeConsumed:
```rust
if let Some(span) = state_may_be_consumed(then_state)
    && matches!(tracker.vars.get(name), Some(ConsumeState::Available(_)))
{
    tracker.vars.insert(name.clone(), ConsumeState::MaybeConsumed(span));
}
```
So for the valid program `fn f(h: Handle) -> Handle { if cond { return consume(h); } consume(h) }` — where `h` is consumed exactly once on every real path because the then-branch RETURNS — `h` becomes MaybeConsumed after the `if`, and the trailing `consume(h)` then hits the MaybeConsumed arm of `consume_var` and emits a hard error. I added a temporary unit test reproducing exactly this shape and it FAILED with: `LinearTypeViolation: "linear value 'h' may already be consumed"`. The region dataflow pass (vow-ir/src/region.rs:240-246) correctly clears `live` on Return, …[truncated]

**Proposed fix:** Track divergence in the linear pass. Give `check_block`/`check_expr` a way to report that a branch ends in `return`/`break`/`continue` (or otherwise has type Never). In `check_if_branches` and `check_match_arms`, when a branch diverges, exclude its tracker from the post-construct merge (the path cannot fall through), so the surviving state after the `if` is taken solely from non-diverging branches. This mirrors the already-correct region-pass behavior of clearing live linear obligations at a Return terminator.

#### L3.56 Linear tracker does not re-arm a linear local on assignment: reassign-after-consume yields a spurious "already consumed", and overwriting a live linear is silently ignored — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-linear-rust` · **kind:** bug · **verdicts:** 1 · **Filed as #657**  
**Files:** `vow-types/src/linear.rs:220-223`, `vow-types/src/linear.rs:268-319`  
**Design ref:** §5.6 / §2.1 (single-consumption resource discipline) — pure impl bug in the linear pass  
**Evidence:** The `Assign` arm visits the LHS non-consuming and the RHS consuming, but never updates the consume-state of an assigned linear LHS:
```rust
ExprKind::Assign { lhs, rhs } => {
    check_expr(lhs, tracker, env, file, emitter, false);
    check_expr(rhs, tracker, env, file, emitter, true);
}
```
Assignment to a linear local is type-legal (vow-types/src/check.rs:1528-1544 accepts `Handle = Handle` via `lhs_ty == rhs_ty`). So for the valid program `fn f(h: Handle) { consume(h); h = open(); consume(h); }` the second `consume(h)` reads `h` while the tracker still has it Consumed (the assignment did not reset it to Available), so `consume_var`'s Consumed arm fires. I added a temporary unit test for exactly this and it FAILED with: `LinearTypeViolation: "linear value 'h' already consumed"` even though the reassignment produced a fresh value. The region pass models reassignment correctly (a fresh Call/RegionAlloc origin), so it accepts the program; only the AST pass falsely rejects. The dual problem is also present: assigning a fresh value over a still-Available linear (`let h=open(); h=open2();` leaking the first) is silently ignored by linear.rs because the Available LHS is never flagged b …[truncated]

**Proposed fix:** In the `Assign` arm, detect when the LHS is an `Ident` naming a tracked linear var. Before overwriting, if its current state is Available, emit the never-consumed/leak diagnostic at the LHS span (the old obligation is being dropped). Then set the var's state to Available(assign-span) so the freshly-assigned value is tracked from scratch (re-arming after a prior consume). This makes reassignment behave like a new binding for linearity purposes.

#### L3.57 Self-hosted parser cannot parse the prefix borrow operator `&expr` — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-parser-self` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #640**  
**Files:** `compiler/parser.vow:678-690`  
**Design ref:** §5.2 (Rust-like surface syntax). grammar.md lines 251-262: `&` is a documented unary borrow operator; `Single & is overloaded by position: prefix &expr is borrow`.  
**Evidence:** parse_unary only handles `!` and `-`; there is no `&` (borrow) arm:
```vow
fn parse_unary(p: Parser) -> i64 {
    if at(p, tok_bang()) { ... UNOP_NOT() ... }
    else if at(p, tok_minus()) { ... UNOP_NEG() ... }
    else { parse_postfix(p) }
}
```
A leading `&` therefore falls through to parse_primary, which has no `tok_amp()` case (lines 756-874), hitting the catch-all error arm: `push_error(p, "unexpected token in expression")`. The Rust parser handles prefix `&` as `ExprKind::Borrow` (vow-syntax/src/parser/expr.rs:237-247). There is also no borrow tag in ast.vow (only TY_REF exists for types) and no borrow handling in lower.vow (EXPR_UNOP only maps UNOP_NOT/UNOP_NEG). So `&expr` is unsupported end-to-end in the self-hosted compiler.

**Proposed fix:** Add an EXPR_BORROW (or UNOP_BORROW) tag to ast.vow, parse prefix `&` in parse_unary mirroring the `!`/`-` arms, and lower it (currently borrow is a no-op / identity at the value level in the Rust pipeline). If borrow is intentionally unsupported in self-hosted Vow, emit a clear unsupported-feature diagnostic instead of the generic 'unexpected token'.

#### L3.58 Self-hosted parser drops string-literal patterns in match arms — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-parser-self` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #641**  
**Files:** `compiler/parser.vow:1006-1018`  
**Design ref:** §5.2 (match exhaustiveness/patterns). grammar.md line 508: Literal pattern examples include `"hello"`.  
**Evidence:** parse_pattern handles int, suffixed-int, and bool literal patterns but not string literals:
```vow
} else if tag == tok_lit_int() || tag == tok_lit_int_suffixed() {
    ... PAT_LIT(), EXPR_LIT_INT() ...
} else if tag == tok_lit_bool() {
    ... PAT_LIT(), EXPR_LIT_BOOL() ...
} else if tag == tok_ident() {
```
There is no `tok_lit_string()` arm. A pattern like `match s { "hello" => 1, _ => 0 }` falls to the catch-all (lines 1081-1084) → `push_error(p, "unexpected token in pattern")` and yields a PAT_WILD, silently mis-parsing the arm. The Rust parser supports `PatKind::Lit(Lit::String(s))` (vow-syntax/src/parser/types.rs:187-193).

**Proposed fix:** Add a `tag == tok_lit_string()` arm in parse_pattern that interns the string and produces a PAT_LIT carrying EXPR_LIT_STR with the interned sid, mirroring the int/bool arms and the Rust LitString pattern handling.

#### L3.59 Self-hosted lexer silently accepts unterminated string literals and unknown characters (no diagnostics) — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-parser-self` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #642**  
**Files:** `compiler/lexer.vow:202-226`  
**Design ref:** §4.5/§6.4 (structured diagnostics are part of the language contract); §7 (differential behavior with the Rust frontend). grammar.md §String Literals.  
**Evidence:** On an unterminated string, the loop runs to EOF and then emits a token with no error:
```vow
while pos < src_len && src.byte_at(pos) != 34 { ... }
if pos < src_len { pos = pos + 1; }   // no closing quote => no advance, no error
let tok: Token = make_token(tok_lit_string(), 0, 0, s, start, pos - start);
```
Unknown characters are likewise skipped silently in the final else (line 423-425): `} else { pos = pos + 1; }`. The Rust lexer returns hard errors in both cases: `"unterminated string literal"` (vow-syntax/src/lexer.rs:366-371) and `"unexpected character '{}'"` (lines 246-252). `lex(src: String) -> Vec<Token>` has no diagnostic channel at all, so the self-hosted lexer structurally cannot report any lexical error — malformed input that the Rust frontend rejects is silently consumed.

**Proposed fix:** Give the lexer a way to surface diagnostics (return a status/error list or take a DiagCtx) and emit structured errors for unterminated string literals, unterminated escapes, and unknown characters, matching the Rust lexer's UnexpectedCharacter / unterminated-string errors.

#### L3.60 Self-hosted lexer never produces float literals; float syntax silently mis-parses — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-parser-self` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #643**  
**Files:** `compiler/lexer.vow:185-201`, `compiler/parser.vow:756-875`  
**Design ref:** §5.3/§5.7 (f32/f64 IEEE-754 are language types). grammar.md lines 154-159 (Float Literals `3.14`, `-0.5`) and lines 118-119 (f32/f64 primitive types).  
**Evidence:** The digit branch in lex() only handles integers (with optional suffix); there is no float branch. A source `3.14` lexes as `tok_lit_int(3)`, then `tok_dot`, then `tok_lit_int(14)`, so it is parsed as field access `(3).14` rather than a float literal. The Rust lexer has an explicit float path (vow-syntax/src/lexer.rs:283-298, `TokenKind::LitFloat`). Notably the AST/checker/lower DO model floats — ast.vow:6 `EXPR_LIT_FLOAT()=3`, checker.vow:909-910 maps it to CTY_F64(), lower.vow:872 handles EXPR_LIT_FLOAT — but the parser never emits an EXPR_LIT_FLOAT node and the lexer never emits a float token, so that handling is dead and float literals cannot reach it.

**Proposed fix:** Add a float-lexing path: after consuming integer digits, if the next byte is `.` followed by a digit, consume the fraction and emit a float token (new tok_lit_float kind carrying the value), then add a parse_primary arm producing EXPR_LIT_FLOAT — matching the Rust lexer/parser. If floats are intentionally out of scope for self-hosted Vow, remove the dead EXPR_LIT_FLOAT handling and document the exclusion.

#### L3.61 String-literal printer emits raw control bytes into canonical source (asymmetric/incomplete escape set) — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-printer-rust` · **kind:** diagnostics-quality · **verdicts:** 1 · reviewer severity votes: low×1 · **Filed as #623**  
**Files:** `vow-syntax/src/printer.rs:748-763`, `vow-syntax/src/lexer.rs:384-405`  
**Design ref:** §4.1 / §5.2 (canonical, mechanically-comparable source form). Also a lexer/printer escape-set asymmetry (pure impl bug component).  
**Evidence:** `print_lit` only escapes six characters and passes every other character through verbatim:
```
Lit::String(s) => {
    let mut out = "\"".to_string();
    for ch in s.chars() {
        match ch {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\0' => out.push_str("\\0"),
            c => out.push(c),   // <-- raw byte, including BEL/ESC/VT/FF/DEL
        }
    }
    ...
```
A probe printing a string containing `\u{7}` (BEL) and `\u{1b}` (ESC) yields output bytes `[34, 97, 7, 98, 27, 99, 34]` i.e. the raw control bytes are embedded directly in the emitted "canonical" source. The lexer (`lex_string`, lines 393-404) only DECODES `\n \t \r \\ \" \0`; any other escape sequence is preserved literally as backslash+char (lines 400-403), so there is no `\x`/`\u` form the printer could emit that the lexer would re-read. The round-trip happens to succeed (the lexer accepts raw non-quote/non-backslash bytes), but the canonical form for a Vow program can now contain unprintable control characters, which corrupts diffs, termina …[truncated]

**Proposed fix:** Make the lexer/printer escape sets symmetric and complete: add `\xNN` (or `\u{..}`) decoding to `lex_string`, and have `print_lit` escape all non-printable / non-ASCII-graphic control bytes (e.g. anything `< 0x20` other than the named ones, plus `0x7f`) using that same form. This guarantees canonical source is plain printable text and that every byte has a single canonical escaped representation.

#### L3.62 Returned allocation alignment never asserted in the ESBMC arena verify harness — `MEDIUM` · ✅ survived cross-check · **Duplicate of #430**
**finder:** `L3-runtime` · **kind:** diagnostics-quality · **verdicts:** 1 · reviewer severity votes: low×1  
**Files:** `vow-runtime/verify/arena.c:256-270`  
**Design ref:** §5.6 (arena memory model); §6.4 (verification as part of the model)  
**Evidence:** The C mirror that ESBMC bounded-model-checks (`vow-runtime/verify/arena.c` `main`) makes `align` symbolic and constrains it to {1,8,16,4096}, then after each `__vow_arena_alloc` asserts only:
```c
assert(a.cursor <= a.chunk_end);
assert((uintptr_t)p >= (uintptr_t)a.current_chunk + CHUNK_LINK_BYTES);
assert((uintptr_t)p + sz <= a.chunk_end);
...
assert(a.last_alloc_size == sz);
```
There is no `assert(((uintptr_t)p & (align - 1)) == 0)`. A broken `align_up` returning an in-bounds but under-aligned pointer (e.g. returning `addr` unchanged) would still satisfy every existing assertion, including for align=4096 — so the alignment half of the allocator contract is never actually verified. The Rust unit tests (`arena_alignment_respected`, line ~4355; `arena_large_alignment_takes_oversized_path`, line ~4366) do check `p % align == 0`, but only for fixed alignments and outside the ESBMC proof path.

**Proposed fix:** After each harness-checked allocation add `assert(((uintptr_t)p & (align - 1)) == 0);` for the symbolic-align case, plus directed fixed-alignment assertions. Validate by temporarily breaking `align_up` and confirming `make verify` rejects it.

#### L3.63 U64 vow-violation captures emit wrong type tag and a hardcoded payload of 0 (debug/sanitize diagnostics) — `MEDIUM` · ✅ survived cross-check · **Duplicate of #439**
**finder:** `L3-shim` · **kind:** diagnostics-quality · **verdicts:** 1 · reviewer severity votes: low×1  
**Files:** `vow-clif-shim/src/lib.rs:2824-2833`, `vow-clif-shim/src/lib.rs:2884-2901`, `vow-clif-shim/src/lib.rs:2213-2219`  
**Design ref:** §6.4 (counterexamples / blame-carrying VowViolation values as first-class repair inputs); N/A — pure impl bug otherwise  
**Evidence:** `tag_for_ir_ty` has no ITY_U64 arm:
```
fn tag_for_ir_ty(ty: i64) -> i64 {
    match ty {
        ITY_I32 => 0, ITY_I64 => 1, ITY_F32 => 2, ITY_F64 => 3, ITY_BOOL => 4,
        _ => 0,            // <- ITY_U64 (=8) falls here, mislabeled as TAG_I32
    }
}
```
and `emit_vow_check`'s payload match also lacks a U64 arm:
```
let payload: Value = match *ir_ty {
    ITY_I32 => builder.ins().sextend(types::I64, *cl_val),
    ITY_I64 => *cl_val,
    ITY_F32 => { ... }
    ITY_F64 => builder.ins().bitcast(types::I64, MemFlags::new(), *cl_val),
    ITY_BOOL => *cl_val,
    _ => builder.ins().iconst(types::I64, 0),   // <- ITY_U64 => literal 0
};
```
The capture filter at 2213-2219 only excludes ITY_PTR|ITY_LPTR|ITY_UNIT, so a `u64` free variable IS captured and reaches both default arms. The runtime (`vow-runtime/src/lib.rs:48,65`) defines `TAG_U64 = 5` and formats it as an unsigned decimal, so the correct tag exists but is never emitted. Net effect: every `u64` binding in a debug/sanitize VowViolation reports type-tag I32 and value 0 regardless of the real runtime value. The Rust backend twin (`vow-codegen/src/cranelift_backend.rs:1700-1709,1801-1811`) shares the identical defect, so it is …[truncated]

**Proposed fix:** Add `ITY_U64 => 5` to `tag_for_ir_ty` and `ITY_U64 => *cl_val` (the value is already an i64-shaped Cranelift value) to the payload match in `emit_vow_check`. Mirror the same two arms in `vow-codegen/src/cranelift_backend.rs`. Add a debug-mode runtime regression test that fails a `requires`/`ensures` over a `u64` free variable with a value above i64::MAX and asserts the JSON `values` map shows the correct unsigned decimal.

#### L3.64 Call and MethodCall AST spans overshoot past the closing `)` into the following token — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-syntax-rust` · **kind:** diagnostics-quality · **verdicts:** 1 · reviewer severity votes: low×1 · **Filed as #639**  
**Files:** `vow-syntax/src/parser/expr.rs:389-400`, `vow-syntax/src/parser/expr.rs:362-388`, `vow-syntax/src/parser/expr.rs:430-442`  
**Design ref:** §6.4 (structured diagnostics, fault localization, counterexamples mapped to source); §6.5 stable agent-facing output  
**Evidence:** In parse_postfix the Call arm computes `end` AFTER parse_call_args has already consumed the closing `)`:
```
TokenKind::LParen => {
    self.advance();
    let args = self.parse_call_args();   // <-- consumes the `)`
    let end = self.current_span();       // <-- span of the token AFTER `)`
    Expr { kind: ExprKind::Call { ... }, span: start.merge(end) }
}
```
The MethodCall arm has the identical pattern (`let args = self.parse_call_args(); let end = self.current_span();`). I confirmed dynamically:
- `f(1) + 2`: the call's span text is "f(1) +" (includes the trailing `+`).
- `a.len() + 2`: the method call's span text is "a.len() +".
Contrast with the Index arm, which captures `end` BEFORE consuming `]` and is correct (`a[0]` span text = "a[0]"). Because Vow's debugging model is span-driven (§6.4: counterexamples and fault localization map back to source via spans), an over-wide call span makes blame/counterexample highlighting point at the wrong range — directly degrading the agent-facing diagnostic surface.

**Proposed fix:** Capture the closing-paren span inside parse_call_args and return it, or have parse_call_args return the `)` span; then build `end` from the `)` rather than from current_span() after consumption. Simplest: change parse_call_args to return `(Vec<Expr>, Span)` where Span is the consumed `)` (mirroring the Index arm which reads current_span() before expect(RBracket)). Add span-extent tests for Call/MethodCall like the existing item-span parity tests.

#### L3.65 Suffixed integer literals other than u64 are accepted but their width/sign is discarded (e.g. 256u8, -1 via u32 suffix) — no width validation — `MEDIUM` · ✅ survived cross-check
**finder:** `L3-syntax-rust` · **kind:** bug · **verdicts:** 1 · **Filed as #627**  
**Files:** `vow-syntax/src/parser/expr.rs:155-178`, `vow-syntax/src/lexer.rs:332-360`  
**Design ref:** §5.7 fixed-width integers; §4.2 explicit semantics over convenience; grammar.md "Suffixed integer literals: 42u64" (only u64 documented)  
**Evidence:** parse_prefix only specially handles the U64 suffix (wrapping it in a `as u64` cast); every other suffix (i8/i16/i32/i128/u8/u16/u32/u128/usize/isize) is dropped entirely and the literal collapses to a bare i64-typed `Lit::Int`:
```
if suffix == IntSuffix::U64 {
    Expr { kind: ExprKind::Cast { expr: ...Lit::Int(value)..., target_ty: u64 }, ... }
} else {
    Expr { kind: ExprKind::Lit(Lit::Int(value)), span: start }   // <-- suffix thrown away
}
```
The lexer faithfully records `LitIntSuffixed { value, suffix }` for all 12 suffixes (try_lex_int_suffix), but the parser then erases all of them except U64. Consequences: (1) `42u8` is treated identically to `42i64`, so the declared width carries no type or range meaning, and an out-of-range value like `256u8` is silently accepted (no width check anywhere, see related literal-overflow finding); (2) grammar.md §Integer Literals only documents `42u64`, so the lexer accepts suffixes the language does not define behavior for. This conflicts with §5.7's fixed-width-integer model and the explicit-semantics principle (§4.2).

**Proposed fix:** Decide and enforce one behavior: either (a) only `u64` suffixes are legal (reject all others at lex/parse with a diagnostic, matching what grammar.md documents and the parser actually supports), or (b) thread every suffix into the type system as a typed literal with width/range validation. Do not lex a suffix the parser silently discards. Update grammar.md accordingly and add per-suffix tests.

#### L3.66 Parallel verify 'lowest-indexed halt' determinism guarantee is not actually upheld — `MEDIUM` · ❌ refuted by cross-check
**finder:** `L3-driver-rust` · **kind:** diagnostics-quality · **verdicts:** 1  
**Files:** `vow/src/main.rs:8710-8789`  
**Design ref:** §6.5 'deterministic, diff-stable code formatting' / agent-facing reproducibility; §2.5 canonical form  
**Evidence:** The coordinator comment promises determinism:

  // Stop after first halt-class outcome (Failed/Error/Timeout/ToolNotFound);
  // return lowest-indexed halt for deterministic reporting.

but workers claim indices via `next.fetch_add` and break early on the stop flag:

  loop {
      if stop.load(Ordering::Acquire) { break; }
      let idx = next.fetch_add(1, Ordering::AcqRel);
      if idx >= vowed.len() { break; }
      ... match verify_one_function(...) {
          PerFuncResult::Halt(out) => { guard[idx] = Some(out); ...; stop.store(true, Ordering::Release); }
      }
  }

After the harvest:

  let outcome = halts.into_iter().flatten().next().unwrap_or_else(...);

If the worker on a higher index (e.g. idx=5) halts first, `stop` is set before a sibling claims a lower failing index (e.g. idx=2); that lower index is never verified, so `halts[2]` stays None and `halts[..].flatten().next()` returns the idx=5 outcome. The reported failing `function`, `counterexample`, and `verify_status` therefore depend on thread scheduling whenever two or more vowed functions would halt. The default CLI path runs with jobs = num_cpus/2 (>1), so this is the common case, not an edge case. This contrad …[truncated]

**Proposed fix:** Either (a) drop the over-promising comment and document that any halt is reported, or (b) make selection truly deterministic: after the scope joins, deterministically re-scan claimed-but-unverified lower indices, or run a final single-threaded lowest-index pass over functions that were skipped due to the stop flag, so the reported halt is always the lowest source-order failing function regardless of scheduling.

#### L3.67 Self-hosted IR printer renders float constants as raw IEEE-754 bit pattern, not the float value (diverges from Rust golden printer) — `MEDIUM` · ❌ refuted by cross-check
**finder:** `L3-printer-self` · **kind:** design-divergence · **verdicts:** 1 · reviewer severity votes: low×1  
**Files:** `compiler/ir_printer.vow:238-239`, `compiler/lower.vow:872-875`  
**Design ref:** §6.3 (self-hosting / byte-identical fixed point), §4.1 (single canonical form). Divergence between Rust and self-hosted printer output; does NOT affect the codegen binary fixed point (print_module text is consumed only by --dump-ir / IR-text dump in main.vow:707 and main.vow:1357, never re-parsed or fed to codegen).  
**Evidence:** compiler/ir_printer.vow format_data maps float-const data kinds to a *signed-integer* render of inst.dv:
```
if dk == IDATA_CONST_F32() { return i64_to_str(inst.dv); }
if dk == IDATA_CONST_F64() { return i64_to_str(inst.dv); }
```
But inst.dv for a float literal holds the IEEE-754 bit pattern, not a value to print as a decimal. compiler/lower.vow:872-875 lowers EXPR_LIT_FLOAT by storing the raw `bits`:
```
if tag == EXPR_LIT_FLOAT() {
    let bits: i64 = expr_a(a, eid);
    ...
    return lctx_emit(ctx, IOP_CONST_F64(), ITY_F64(), args, IDATA_CONST_F64(), bits, 0, String::from(""));
}
```
The Rust golden printer (vow-ir/src/printer.rs:157-158) prints the actual float: `InstData::ConstF64(v) => Some(v.to_string())` where the lowerer stores `InstData::ConstF64(*v)` (vow-ir/src/lower/mod.rs:610-614). Concrete divergence verified: for source literal `1.5`, the Rust `--dump-ir` emits `ConstF64[1.5]` while the self-hosted emits `ConstF64[4609434218613702656]` (the i64 reinterpretation of the f64 bits, computed via rustc). The two `vowc --dump-ir` outputs are therefore NOT byte-identical for any program containing a float literal. This is not caught by scripts/full_test.sh Section 0b, whi …[truncated]

**Proposed fix:** Make the self-hosted printer reconstruct the decimal float from the stored bits before printing (mirroring Rust's `v.to_string()`), or at minimum print it in a form that round-trips to the same text as Rust. Add a float literal to the --dump-ir parity comparison in full_test.sh (or a dedicated golden test) so any future divergence is caught. Until a float-formatting routine exists in the self-hosted runtime, the printer should not pretend the bit pattern is a decimal value.

#### L3.68 Preamble declares 5 __VERIFIER_nondet_* externs but emitter/harness can emit __VERIFIER_nondet_unsigned_long (U64) — `LOW` · ✅ survived cross-check
**finder:** `L3-cemit-rust` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #676**  
**Files:** `vow-verify/src/c_emitter.rs:2112-2116`, `vow-verify/src/c_emitter.rs:1749-1760`  
**Design ref:** §6.2 — impl/diagnostics-quality of the emitted C model  
**Evidence:** The preamble declares exactly five nondet intrinsics:
```
out.push_str("extern int __VERIFIER_nondet_int(void);\n");
out.push_str("extern long __VERIFIER_nondet_long(void);\n");
out.push_str("extern float __VERIFIER_nondet_float(void);\n");
out.push_str("extern double __VERIFIER_nondet_double(void);\n");
out.push_str("extern _Bool __VERIFIER_nondet_bool(void);\n\n");
```
but `c_nondet_suffix(Ty::U64)` returns `"unsigned_long"` (line 1753), so `emit_unmodelled`/`emit_unsupported_for_verification` …[truncated]

**Proposed fix:** Add `extern unsigned long __VERIFIER_nondet_unsigned_long(void);` to `emit_c_preamble`, keeping the declared intrinsic set in sync with `c_nondet_suffix`/`esbmc_nondet_call`. Consider a shared single source of truth for the nondet-suffix-to-extern mapping so the two cannot drift.

#### L3.69 const-fn detection diverges: self-hosted detect_const_fns inlines U64 constant functions, Rust detect_constant_functions does not — produces different verified C models for the same module — `LOW` · ✅ survived cross-check
**finder:** `L3-cemit-self` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #667**  
**Files:** `compiler/c_emitter.vow:609-687`  
**Design ref:** §7 self-hosting (Rust and Vow emitters must produce equivalent verified models); CLAUDE.md 'modify BOTH compilers in the same session'  
**Evidence:** Self-hosted `detect_const_fns` accepts U64 const bodies (compiler/c_emitter.vow:635):
  `if ci.op == IOP_CONST_I32() || ci.op == IOP_CONST_I64() || ci.op == IOP_CONST_U64() || ci.op == IOP_CONST_BOOL() {`
and `const_fn_value_str` / `const_fn_type_str` emit `ULL` / `uint64_t` for the U64 case (compiler/c_emitter.vow:661-663, 682). The Rust `detect_constant_functions` only recognizes I32/I64/Bool (vow-verify/src/c_emitter.rs:39-44):
  `(Opcode::ConstI32, InstData::ConstI32(v)) => ConstantValue::I3 …[truncated]

**Proposed fix:** Make the two const-fn detectors agree: either add U64 to the Rust `ConstantValue`/`detect_constant_functions`, or drop `IOP_CONST_U64()` from the self-hosted `detect_const_fns` accept-list (compiler/c_emitter.vow:635) and `const_fn_value_str`/`const_fn_type_str`. Add a shared unit test asserting both classify the same U64-const fn identically.

#### L3.70 Self-hosted `?` operator (EXPR_QUESTION) does not require Option/Result receiver — `LOW` · ✅ survived cross-check
**finder:** `L3-checker-self` · **kind:** soundness · **verdicts:** 1 · **Filed as #680**  
**Files:** `compiler/checker.vow:1570-1577`, `vow-types/src/check.rs:1510-1526`  
**Design ref:** §5.4 intrinsic Option/Result semantics; §7 multi-compiler agreement  
**Evidence:** Self-hosted EXPR_QUESTION:
```
let inner_tid: i64 = check_expr(e, m, inner_eid);
if is_opaque(inner_tid) { return inner_tid; }
return CTY_UNIT();
```
It accepts any non-opaque inner type and returns Unit, with no check that the operand is `Option<T>`/`Result<T,E>`, and no propagation of the unwrapped payload type. The Rust checker emits `"the `?` operator requires `Option<T>` or `Result<T,E>`, found `{inner_ty}`"` (check.rs:1512-1523). Self-hosted thus accepts `let x = some_i64?;`. (It also retu …[truncated]

**Proposed fix:** Inspect the receiver tid: require CTY_APPLIED over an Option/Result base (or opaque); emit EC_TYPE_MISMATCH otherwise. Return the first applied type argument (the T payload) instead of CTY_UNIT.

#### L3.71 Self-hosted check_module lacks the item_files/items length-guard the Rust mirror asserts — `LOW` · ✅ survived cross-check · **Duplicate of #569**
**finder:** `L3-checker-self` · **kind:** bug · **verdicts:** 1  
**Files:** `compiler/checker.vow:357-373`  
**Design ref:** N/A — pure impl bug / robustness parity  
**Evidence:** `check_module` runs five per-item passes that index `item_files[i]` in lockstep with `m.items[i]` (e.g. `let cur: String = item_files[i]; ... let item: i64 = m.items[i];`) but never checks `item_files.len() == m.items.len()`. If the two vectors ever drift, the first `item_files[i]` read past the shorter vector traps with `IndexOutOfBounds` at runtime rather than a clear invariant failure. The Rust mirror guards the same invariant with `assert_eq!` per #569. Invariant holds by construction today; …[truncated]

**Proposed fix:** Add an early guard in check_module: if `item_files.len() != m.items.len()` emit an internal/invariant diagnostic (or process_exit with a clear message) before entering the indexed passes.

#### L3.72 Receiver-region GetArg test in clif.vow diverges from the shim (requires op==GET_ARG; shim keys only on dk==ARG_INDEX), an asymmetry between the two lockstep backends — `LOW` · ✅ survived cross-check
**finder:** `L3-codegen-self` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #660**  
**Files:** `compiler/clif.vow:144-149`  
**Design ref:** §6.1 / §7 (self-hosted backend must reproduce the Rust/shim backend; CLAUDE.md: 'modify BOTH the Rust compiler and the self-hosted compiler in the same session')  
**Evidence:** clif.vow gates the hidden-caller-region lookup on BOTH the opcode and the data kind:

  `if receiver.op == IOP_GET_ARG() && receiver.dk == IDATA_ARG_INDEX() {\n        let hidden: i64 = clif_hidden_store_target_region(f, receiver.dv);\n        if region_kind(hidden) == REGION_KIND_CALLER() { return hidden; }` (clif.vow:144-148)

The shim's equivalent (`inst_region_for_value_inner`) keys ONLY on the data kind, with no opcode check:

  `if let Some(&(dk, dv)) = inst_data_by_id.get(&inst_id)\n …[truncated]

**Proposed fix:** Align the two implementations: either drop the `receiver.op == IOP_GET_ARG()` conjunct in clif.vow (matching the shim's dk-only test), or add the same `op == GetArg` guard to the shim's `inst_region_for_value_inner`. Pick one canonical form and apply it to both backends so the region-resolution algorithm is byte-for-byte equivalent.

#### L3.73 Self-hosted method-call argument types are computed but never checked (arity/type) — broader silent-accept than Rust, no arg validation in either but receiver-method tables diverge — `LOW` · ✅ survived cross-check
**finder:** `L3-divergence` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #671**  
**Files:** `compiler/checker.vow:1054-1066`, `vow-types/src/check.rs:1010-1018`  
**Design ref:** §6.4 (structured diagnostics); docs/spec/grammar.md method signatures  
**Evidence:** Both compilers walk method-call args for type inference but neither validates arity or argument types against the builtin method signature (self-hosted: `let _arg_tid: i64 = check_expr(e, m, arg_eid);` discarded, checker.vow:1062-1066; Rust: `for arg in args { self.check_expr(arg); }`, check.rs:1016-1018). This is parallel and not itself a divergence, BUT combined with the divergent return-type tables it means e.g. `v.push()` (zero args) or `v.push(1, 2)` (two args) are accepted by BOTH with no …[truncated]

**Proposed fix:** Add arity + argument-type validation for builtin methods in both checkers, keyed off the spec signatures, so malformed builtin method calls produce structured diagnostics rather than being silently accepted and lowered.

#### L3.74 Self-hosted CLI does not reject unknown flags; Rust clap path does — parity gap on argument validation — `LOW` · ✅ survived cross-check · **Duplicate of #580**
**finder:** `L3-divergence` · **kind:** design-divergence · **verdicts:** 1  
**Files:** `compiler/main.vow:40-75`, `vow/src/main.rs:1-89`  
**Design ref:** §6.5 (agent-facing tooling, explicit command boundaries)  
**Evidence:** The self-hosted argv scanner (main.vow:40-75) only special-cases a fixed list of value-bearing flags (`-o`, `--mode`, `--max-k-step`, `--timeout`, `--solver`, `--encoding`, `--verify-jobs`, `--module-root`, etc.) and otherwise treats unrecognized tokens as positional inputs rather than erroring, whereas the Rust driver uses clap which rejects unknown flags. A flag the self-hosted does not know (e.g. `--bogus`) is silently ignored / mis-classified rather than rejected. This is issue #580 (reject …[truncated]

**Proposed fix:** Per #580, add an explicit unknown-flag check in the self-hosted argv parser that errors (non-zero exit + structured diagnostic) on any leading-dash token not in the known set, matching clap's behavior.

#### L3.75 Per-test `skipped` status is unreachable: schema/summary advertise it but no code path emits it — `LOW` · ✅ survived cross-check
**finder:** `L3-driver-rust` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #683**  
**Files:** `vow/src/main.rs:9586-9590`, `vow/src/main.rs:9622-9622`  
**Design ref:** §6.5 machine-readable outputs must be honest; docs/spec/cli.md §vow test  
**Evidence:** run_test_command computes per-test status only as:

  let status = match exit_code {
      Some(0) => "passed",
      Some(_) => "failed",
      None => "timeout",
  };

and the only other statuses pushed are `"compile_error"` and `"verify_failed"`. No path ever assigns `"skipped"`. Yet the summary counts it:

  let skipped = entries.iter().filter(|e| e.status == "skipped").count();

and both the embedded test-result schema (`"enum": ["passed","failed","timeout","skipped","compile_error","verify …[truncated]

**Proposed fix:** Either remove `skipped` from the per-test status enum/summary/spec, or wire an actual skip path (e.g. tests with no `main`, or filtered-but-counted tests) that emits `"skipped"`. Keep the schema, summary field, and spec in lockstep with the code.

#### L3.76 `vow test --mode profile|sanitize|<invalid>` silently degrades to debug instead of erroring (diverges from Rust which rejects `profile` for test) — `LOW` · ✅ survived cross-check
**finder:** `L3-driver-self` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #669**  
**Files:** `compiler/main.vow:940-944`  
**Design ref:** §2.5 (single canonical behavior across the two compilers); cli.md §`vow test` (only debug/release documented)  
**Evidence:** `run_test` reduces `--mode` to a binary release/debug switch (compiler/main.vow:940-944):
```
    let mode_str: String = get_flag_arg(argv, "--mode");
    let mode: i64 = {
        if mode_str == String::from("release") { 0 }
        else { 1 }
    };
```
Every non-`release` value — `profile`, `sanitize`, and any typo — maps to `1` (debug). The Rust driver explicitly errors on `--mode profile` for the test subcommand (vow/src/main.rs:10071-10074: `eprintln!("Error: --mode profile is not supporte …[truncated]

**Proposed fix:** Validate `--mode` in run_test against {debug, release}; reject `profile`/`sanitize`/unknown with an explicit error and non-zero exit, matching the Rust driver. Fold into the broader flag-value validation work (#580).

#### L3.77 Embedded skill/help payload dominates main.vow (~80% of the file, ~7,300 push_str calls), inflating compile time, codegen size, and verification surface — `LOW` · ✅ survived cross-check · **Duplicate of #177**
**finder:** `L3-driver-self` · **kind:** optimization · **verdicts:** 1  
**Files:** `compiler/main.vow:1365-8706`, `compiler/main.vow:2368-2407`  
**Design ref:** §6.5 (embedded, version-matched help is intended) vs CLAUDE.md "Small files, smaller functions" / Development Discipline; tracked by #177  
**Evidence:** The skill/help generators span compiler/main.vow:1365 (`skill_json`) through 8706 (`skill_support_contents`) — ~7,341 of the file's 9,144 lines (~80%). They are built one string-literal append at a time: `grep -c push_str compiler/main.vow` = 7303, e.g. `skill_bundle` (2368-2407+) is hundreds of consecutive `r.push_str(String::from("...\n"));` statements. Each `push_str` is an emitted IR call plus a `.rodata` literal, so the driver's codegen and the bootstrap/verify passes over main.vow carry an …[truncated]

**Proposed fix:** Store each help/skill document as a single string constant (one literal per document) or load it from an embedded data section, instead of thousands of per-line push_str calls. This keeps the version-matched-help guarantee while collapsing ~7k IR calls and shrinking the file the bootstrap/verify pipeline must process. Coordinate with the generate_help.py generator so both compilers stay in sync.

#### L3.78 Validator linear-consume check is control-flow-insensitive — flags branch-exclusive single consumes as consumed-twice — `LOW` · ✅ survived cross-check
**finder:** `L3-ir-lower-rust` · **kind:** bug · **verdicts:** 1 · **Filed as #670**  
**Files:** `vow-ir/src/validator.rs:91-121`  
**Design ref:** §5.6 / linear types ('linear struct values must be consumed exactly once'). N/A — pure impl bug  
**Evidence:** `check_linear_types` sums `LinearConsume` occurrences across ALL blocks unconditionally: `for block in &func.blocks { for inst in &block.insts { if inst.opcode == Opcode::LinearConsume { for &arg in &inst.args { if let Some(n) = consume_count.get_mut(&arg) { *n += 1; } } } } }` (lines 103-113), then `_ => errors.push(ValidationError::LinearConsumedTwice(id))` for count > 1 (line 118). A linear value consumed once in a `then` block and once in a mutually-exclusive `else` block (exactly one execut …[truncated]

**Proposed fix:** Make the linearity check path-aware: verify that each linear value is consumed exactly once on every path from definition to function exit (e.g. a per-path / per-successor dataflow over the CFG), rather than summing a global occurrence count. Until that is implemented, document the limitation and do not rely on this check as the linearity enforcement point (the type checker remains the source of truth).

#### L3.79 Linear-region check emits hard Error regardless of region kind; spec mandates a warning for Root-regioned unconsumed linear values — `LOW` · ✅ survived cross-check
**finder:** `L3-ir-region-rust` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #681**  
**Files:** `vow-ir/src/region.rs:187-254`, `vow-ir/src/region.rs:823-859`  
**Design ref:** §9.1 (region(v)=Root => warning-not-error)  
**Evidence:** `check_function_linear_regions` performs a liveness dataflow and `emit_live_linear_errors` unconditionally pushes `Severity::Error` for any linear value live at a `Return`, never consulting the value's `inst.region`:
```rust
// region.rs:842-847
diagnostics.push(Diagnostic {
    severity: Severity::Error,
    code: ErrorCode::RegionLinear,
    message: format!("linear value `{name}` is not consumed before its region closes"),
```
Spec §9.1 distinguishes by region: `region(v) = Root` must emit a …[truncated]

**Proposed fix:** Thread each live linear origin's inferred `inst.region` into `emit_live_linear_errors`; when it is `RegionId::Root`, emit `Severity::Warning` with the pin_to_root hint per §9.1, and when it is `RegionId::Rodata` reject with the §9.1 'linear values cannot have Rodata region' message. Mirror in compiler/region.vow.

#### L3.80 Dead duplicate concrete-block region LUB cluster (~290 lines) in self-hosted region.vow — never wired in, drifts silently from live value_lub_region — `LOW` · ✅ survived cross-check
**finder:** `L3-ir-region-self` · **kind:** optimization · **verdicts:** 1 · **Filed as #691**  
**Files:** `compiler/region.vow:2709-2750`, `compiler/region.vow:2752-2868`, `compiler/region.vow:2883-2898`, `compiler/region.vow:2961-2999`  
**Design ref:** §5.6 (region escape/placement); §2.4 self-hosting parity; CLAUDE.md "Small files, smaller functions" / deep-modules. N/A — pure impl/maintainability debt  
**Evidence:** Four mutually-recursive functions form a self-referential island with no external caller. `value_lub_concrete_block` (line 2709) is the cluster entry point and is referenced nowhere outside the cluster:
```
fn value_lub_concrete_block(
    f: IrFunction, id: i64, defining_block: i64, ...) -> i64 {
    ...
    sort_i64_vec_inplace(markers);
    block_tree_lca_all(block_parent, block_depth, markers)
}
```
Grep confirms the only references to `value_lub_concrete_block`/`direct_markers_collect_concr …[truncated]

**Proposed fix:** Delete the four orphaned functions (`value_lub_concrete_block`, `direct_markers_collect_concrete_blocks`, `target_marker_concrete_block`, `call_store_effects_collect_concrete_blocks`). `push_region_work` at line 2870 is shared with the live path and must be kept. Re-run the bootstrap triple to confirm byte-identical output after removal.

#### L3.81 Self-hosted region-marker ICE diagnostic omits the 'file an issue' guidance hint present in the Rust internal_compiler_error — `LOW` · ✅ survived cross-check
**finder:** `L3-ir-region-self` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #692**  
**Files:** `compiler/region.vow:175-186`, `vow-ir/src/region.rs:1231-1247`  
**Design ref:** §6.4 structured diagnostics; §6.5 agent-facing tooling. N/A — pure impl divergence  
**Evidence:** When the SCC fixpoint exceeds its monotone iteration bound, both compilers emit a fail-closed error (error_count is incremented, so the build fails — sound). But the diagnostics diverge. Self-hosted:
```
let d: Diagnostic = diag_error(
    EC_REGION_CONFLICT(),
    String::from("internal compiler error: region inference SCC exceeded monotone iteration bound"),
    String::from(""), 0, 0
);
diag_ctx_emit(dctx, d);
```
The Rust `internal_compiler_error` carries an actionable hint the self-hosted o …[truncated]

**Proposed fix:** Add `diag_add_hint(d, String::from("this indicates a bug in the region inference pass; please file an issue"));` before `diag_ctx_emit` so the self-hosted ICE matches the Rust diagnostic shape.

#### L3.82 insert_region_markers_module lacks the whole-function 'no block regions' early-out, doing dominator/back-edge/phi-home work unconditionally — `LOW` · ✅ survived cross-check
**finder:** `L3-ir-region-self` · **kind:** optimization · **verdicts:** 1 · **Filed as #693**  
**Files:** `compiler/region.vow:4886-4910`, `vow-ir/src/region.rs:1023-1034`  
**Design ref:** §5.6; CLAUDE.md Production Quality (scalability). N/A — pure impl perf divergence  
**Evidence:** Rust `insert_region_markers` collects block regions and short-circuits the whole function when none exist:
```
if block_regions.is_empty() { continue; }
let block_tree = BlockTree::from_function(func);
... // only built when there is at least one block region
```
The self-hosted mirror collects `block_regions` (line 4887-4901) but never tests it for emptiness; it unconditionally builds `build_block_parent`, `build_block_depth`, `detect_loop_back_edges`, `forward_graph_without_back_edges`, `rever …[truncated]

**Proposed fix:** After collecting `block_regions` (line 4901), add `if block_regions.len() == 0 { fi = fi + 1; continue; }` to mirror the Rust early-out and skip the dominator/back-edge/phi-home construction for marker-free functions.

#### L3.83 Self-hosted parser has no Or-pattern (`A | B`) support despite PAT_OR tag — `LOW` · ✅ survived cross-check
**finder:** `L3-parser-self` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #641**  
**Files:** `compiler/parser.vow:1006-1085`, `compiler/ast.vow:79-79`  
**Design ref:** §5.2 (match patterns). grammar.md line 513: `Or pattern | 0 \| 1 \| 2`.  
**Evidence:** ast.vow defines `fn PAT_OR() -> i64 ... { 6 }` but parse_pattern never produces it — there is no handling of a `|` between sub-patterns. The Rust parser builds `PatKind::Or` (vow-syntax/src/parser/types.rs:130-141), though it keys off `PipePipe` (`||`). grammar.md line 513 documents Or patterns `0 | 1 | 2` (single `|`). So Or-patterns are (a) entirely absent from the self-hosted parser and (b) inconsistently tokenized between Rust (`||`) and the documented grammar (`|`). A `match x { 0 | 1 => .. …[truncated]

**Proposed fix:** Decide the canonical Or-pattern token (`|` per grammar.md) and implement it in both frontends: in parse_pattern, after parsing a single pattern, loop while at `tok_pipe()` collecting alternatives into a PAT_OR list; align the Rust parser to use single `|` to match the documented grammar.

#### L3.84 parse_for_expr consumes the loop variable without checking it is an identifier — `LOW` · ✅ survived cross-check
**finder:** `L3-parser-self` · **kind:** bug · **verdicts:** 1 · **Filed as #678**  
**Files:** `compiler/parser.vow:966-979`  
**Design ref:** §6.4 (structured, blame-aware diagnostics). N/A — pure impl robustness bug.  
**Evidence:** ```vow
fn parse_for_expr(p: Parser) -> i64 {
    let _kw: bool = expect(p, tok_kw_for());
    let name_tok: Token = advance(p);            // blindly consumes whatever follows `for`
    let name_sid: i64 = arena_intern_str(p.arena, name_tok.str_val);
    let _in: bool = expect(p, tok_kw_in());
```
If the binding is missing/malformed (e.g. `for in vec { }`), `advance` consumes the `in` keyword as the binding name (interning its empty str_val), then `expect(tok_kw_in())` fails on `vec`, cascading …[truncated]

**Proposed fix:** Replace the blind `advance(p)` with `expect_ident(p)` (handling the -1 error sentinel), so a missing for-loop binding produces a precise 'expected identifier' diagnostic instead of mis-consuming the `in` keyword.

#### L3.85 try_suffix passes byte_at's out-of-bounds sentinel (-1) into is_ident_cont, violating its requires:b>=0 precondition — `LOW` · ✅ survived cross-check
**finder:** `L3-parser-self` · **kind:** bug · **verdicts:** 1 · **Filed as #679**  
**Files:** `compiler/lexer.vow:91-137`, `compiler/lexer.vow:34-39`  
**Design ref:** §5.5 contract purity / blame; CLAUDE.md overflow-guard discipline. N/A — pure impl/contract bug.  
**Evidence:** try_suffix probes bytes past the literal, e.g. `!is_ident_cont(src.byte_at(pos + 2))`. `__vow_string_byte_at` returns -1 for out-of-range indices (vow-runtime/src/lib.rs:1812-1813: `if idx < 0 || idx as usize >= v.len { return -1; }`). When a suffix sits at end-of-input, `byte_at(pos+k)` returns -1, which is then fed to `is_ident_cont`/`is_alpha`/`is_digit`, all of which carry `requires: b >= 0` (lexer.vow:13-39). In debug/verify mode this is a Caller-blame contract violation; in release it happ …[truncated]

**Proposed fix:** Either widen the predicates' preconditions to `requires: b >= -1` (treating -1 as a sentinel) and keep their bodies correct for -1, or guard the call sites in try_suffix to short-circuit when `byte_at` returns -1 before calling is_ident_cont. The former is the smaller change and documents the EOF sentinel explicitly.

#### L3.86 debug_escape_str diverges from Rust {:?} on control and non-ASCII bytes (raw passthrough vs \u{..} escapes) — `LOW` · ✅ survived cross-check
**finder:** `L3-printer-self` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #672**  
**Files:** `compiler/ir_printer.vow:37-69`  
**Design ref:** §4.1 (single canonical form), §6.3 (Rust/self-hosted output parity). Diagnostics/text-dump only; not on the codegen path.  
**Evidence:** compiler/ir_printer.vow debug_escape_str only special-cases `"`, `\`, `\n`, `\r`, `\t`, and NUL; every other byte is emitted verbatim:
```
} else if b == 0 {
    out.push_byte(92);
    out.push_byte(48);
} else {
    out.push_byte(b);
}
```
The Rust golden printer formats the string pool with `{:?}` (vow-ir/src/printer.rs:237: `format!("  @{i} = {:?}", s)`). Rust's `{:?}` for str escapes control and non-printable bytes as `\u{..}`. Verified with rustc: byte 0x01 -> `"\u{1}"`, 0x07 -> `"\u{7}"`, …[truncated]

**Proposed fix:** Extend debug_escape_str to escape all bytes < 0x20 (other than the already-handled \n/\r/\t) and 0x7f as `\u{..}` (hex, matching Rust's str Debug), and decide a deliberate policy for high bytes (>= 0x80). Add a golden test with a string-pool entry containing a control byte to lock parity.

#### L3.87 U64 constant printed via signed i64_to_str; values above i64::MAX would render negative (diverges from Rust unsigned formatting) — `LOW` · ✅ survived cross-check
**finder:** `L3-printer-self` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #673**  
**Files:** `compiler/ir_printer.vow:294-298`  
**Design ref:** §6.3 (Rust/self-hosted printer parity). Diagnostics/text-dump only; not on the codegen fixed-point path. Distinct from issue #490 (typechecker accepting i64 in u64 contexts) — this is a printer-rendering bug.  
**Evidence:** compiler/ir_printer.vow format_data renders a u64 constant by treating inst.dv as signed:
```
if dk == IDATA_CONST_U64() {
    let out: String = i64_to_str(inst.dv);
    out.push_str(String::from("u64"));
    return out;
}
```
i64_to_str prints a leading '-' for negative i64 (ir_printer.vow:9-12). The Rust golden printer holds the value as a true `u64` (vow-ir/src/types.rs:209 `ConstU64(u64)`) and prints it unsigned: `InstData::ConstU64(v) => Some(format!("{v}u64"))` (vow-ir/src/printer.rs:156). …[truncated]

**Proposed fix:** Render u64 constants with an unsigned i64->decimal routine in the self-hosted printer (treat inst.dv's bit pattern as unsigned) so the output matches Rust's `{v}u64` across the entire u64 range. Add a regression case with a large u64 value once such literals are reachable.

#### L3.88 Trace-event JSON interpolates function names without escaping (same class as #436, distinct functions) — `LOW` · ✅ survived cross-check · **Duplicate of #436**
**finder:** `L3-runtime` · **kind:** diagnostics-quality · **verdicts:** 1  
**Files:** `vow-runtime/src/lib.rs:244-273`  
**Design ref:** §6.4 (debug-mode trace output for function and contract boundaries); §6.5 (structured outputs)  
**Evidence:** `__vow_trace_enter` / `__vow_trace_exit` / `__vow_trace_vow` emit machine-readable trace JSON by raw interpolation of `to_string_lossy()` names:
```rust
let name = unsafe { CStr::from_ptr(fn_name_ptr) }.to_string_lossy();
let _ = writeln!(std::io::stderr(), r#"{{"event":"enter","fn":"{name}"}}"#);
```
Same root cause as #436 (no JSON escaping). Lower risk because `fn` names are codegen-emitted `.rodata` identifiers normally restricted to identifier characters, so quotes/backslashes are not expec …[truncated]

**Proposed fix:** When fixing #436, route the trace `fn` name (and the sanitizer `details` strings at lines ~3237-3360) through the same JSON string-escaping helper so all machine-readable stderr lines are guaranteed parseable.

#### L3.89 HashMap and BTreeMap growth use unchecked capacity multiplies (same overflow class as #435, distinct functions) — `LOW` · ✅ survived cross-check · **Duplicate of #435**
**finder:** `L3-runtime` · **kind:** soundness · **verdicts:** 1  
**Files:** `vow-runtime/src/lib.rs:2986-2993`, `vow-runtime/src/lib.rs:3155-3162`  
**Design ref:** §5.4 (HashMap as the shakiest intrinsic; must keep justifying its cost); §5.7 (no UB)  
**Evidence:** `__vow_map_insert_in_arena` grows with unchecked arithmetic:
```rust
let old_size = m.cap * MAP_ENTRY_BYTES;
let new_cap = m.cap * 2;
let new_size = new_cap * MAP_ENTRY_BYTES;
```
and `__vow_btreemap_insert` does the same (`new_cap = m.keys_cap * 2; new_size = new_cap * BTREEMAP_ENTRY_BYTES;`). Like #435, on a sufficiently large map these wrap `new_cap`/`new_size`, under-allocating the backing while `cap` records the doubled value, then the parallel-array writes (`entries[m.len*2]=key`) overrun. …[truncated]

**Proposed fix:** When applying the #435 checked-arithmetic fix to `vec_reserve_in_arena_no_null_check`, apply the same `checked_mul` + oom_trap guards to the `__vow_map_insert_in_arena` and `__vow_btreemap_insert` growth blocks for consistency.

#### L3.90 inst_region_for_value performs unmemoized recursion with per-source `seen.clone()`, giving super-linear cost on Phi DAGs — `LOW` · ✅ survived cross-check · **Duplicate of #367**
**finder:** `L3-shim` · **kind:** optimization · **verdicts:** 1  
**Files:** `vow-clif-shim/src/lib.rs:309-367`, `vow-clif-shim/src/lib.rs:2286-2300`, `vow-clif-shim/src/lib.rs:2385-2406`  
**Design ref:** §6.1/§7 (self-hosting backend must scale to the compiler's own workload); N/A — pure impl/perf bug  
**Evidence:** Inside the Phi-merge branch every Upsilon source is resolved with a freshly cloned visited-set and a full recursive descent, and there is no result cache across the whole pass:
```
for &source_id in sources {
    let mut source_seen = seen.clone();           // O(|seen|) clone per source
    let rgn = inst_region_for_value_inner(source_id, ..., &mut source_seen);
    ...
}
```
The top-level entry also allocates a brand-new `HashSet` on every call (line 305) and the function is invoked once per e …[truncated]

**Proposed fix:** Memoize results in a `HashMap<i64,i64>` carried through `inst_region_for_value_inner` (keyed by inst_id, storing the resolved region once a node fully resolves), and replace the per-source `seen.clone()` with a single shared `seen` set used for cycle detection plus the memo for already-resolved nodes. This collapses the descent to O(V+E) per top-level query and lets the memo be reused across the multiple call/store-target queries within one function. This also addresses the unit-test gap tracked in #367.

#### L3.91 Cast (`as`) binds tighter than prefix unary `-`/`!`, diverging from the documented "usual C/Rust precedence" — `LOW` · ✅ survived cross-check
**finder:** `L3-syntax-rust` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #674**  
**Files:** `vow-syntax/src/parser/expr.rs:225-247`, `vow-syntax/src/parser/expr.rs:83-96`, `vow-syntax/src/parser/expr.rs:414-425`  
**Design ref:** §5.2 source form; grammar.md §Operator Precedence ("usual C/Rust precedence") and §Unary Operators; §4.2 explicit semantics  
**Evidence:** Prefix `-`/`!`/`&` recurse with parse_expr_inner(PREFIX_BINDING_POWER) to parse their operand, but postfix `as` is applied inside that recursion's loop unconditionally (it is in the `matches!(kind, ... KwAs)` postfix set, gated by no binding power):
```
TokenKind::Minus => { self.advance(); let operand = self.parse_expr_inner(PREFIX_BINDING_POWER); ... UnOp::Neg ... }
...
if matches!(kind, ... | TokenKind::KwAs) { lhs = self.parse_postfix(lhs); continue; }
```
So the operand of unary `-` absorbs …[truncated]

**Proposed fix:** Make unary `-`/`!`/`&` bind tighter than `as` to match Rust, or explicitly document and test the chosen ordering in grammar.md. If matching Rust, restructure parse_prefix so the cast postfix is not absorbed into the unary operand (e.g. parse the unary operand at a binding power that excludes `as`, then let `as` apply to the unary result in the outer loop). Add tests for `-a as u64` and `!x as i64`.

#### L3.92 Lexer error for non-ASCII leading byte reports a 1-byte span mid-UTF-8 and mojibake character — `LOW` · ✅ survived cross-check
**finder:** `L3-syntax-rust` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #675**  
**Files:** `vow-syntax/src/lexer.rs:246-252`  
**Design ref:** §6.4 structured diagnostics / reliable handling of bad input; §6.5 stable agent-facing output  
**Evidence:** The catch-all arm of next_token treats a single raw byte as a char and reports a length-1 span:
```
_ => {
    self.pos += 1;
    Err(LexError {
        message: format!("unexpected character '{}'", b as char),   // b is one byte
        span: Span::new(start as u32, 1),                            // len 1
    })
}
```
For a multi-byte UTF-8 character (e.g. `é` = 0xC3 0xA9), `b as char` yields a mojibake char ('Ã') and the span has start at byte 0 with len 1 — i.e. it points into the middle of a …[truncated]

**Proposed fix:** Decode the full UTF-8 char at self.pos (e.g. via self.src[self.pos..].chars().next()), advance by its byte length, set the span len to that byte length, and report the actual char. This yields a correct span and message and avoids any boundary-slicing panic in diagnostic rendering. Add a test with a multi-byte invalid leading character.

#### L3.93 Self-hosted verify_collect ignores ESBMC process exit code (relies solely on stdout text), unlike a defense-in-depth check — `LOW` · ✅ survived cross-check
**finder:** `L3-vsec` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #661**  
**Files:** `compiler/verifier.vow:966-991`, `vow-verify/src/esbmc.rs:510-530`  
**Design ref:** §2.1 (verification is the trust mechanism) — defense in depth. N/A — not a present soundness bug.  
**Evidence:** In `verify_collect` the child exit code is captured but only the watchdog sentinel is acted on:
```vow
let exit_code: i64 = process_wait_timeout(handle, watchdog_ms_for(effective_timeout));
if exit_code == -2 { ...VERIFY_TIMEOUT()... }
...
let status: i64 = parse_verify_status(combined);   // line 991: classification is purely textual; a normal non-zero ESBMC exit or exit_code == -1 (wait error) is never consulted
```
The Rust path (esbmc.rs:514-530) is the same — it classifies purely on the pre …[truncated]

**Proposed fix:** When classifying `PROVEN`, additionally require the captured ESBMC exit code to be 0 (in both `compiler/verifier.vow:verify_collect`/`run_esbmc` and `vow-verify/src/esbmc.rs:run_esbmc_with_max_k_step`); on a success banner with a non-zero exit, downgrade to `ERROR`/`ToolError` so the run fails closed.

#### L3.94 emit_unmodelled assigns a scalar __VERIFIER_nondet to struct-typed (Vec/String/Option) result slots, producing C that ESBMC's frontend rejects — fail-closed but yields opaque verify failures instead of clean skips — `LOW` · ❌ refuted by cross-check
**finder:** `L3-cemit-self` · **kind:** diagnostics-quality · **verdicts:** 1  
**Files:** `compiler/c_emitter.vow:856-865`, `compiler/c_emitter.vow:1768-1769`, `compiler/c_emitter.vow:2098-2124`  
**Design ref:** §6.4 (structured, blame-aware diagnostics; verifier limitations should be reported as skips, not opaque failures)  
**Evidence:** `emit_unmodelled` (compiler/c_emitter.vow:856-865) emits a scalar assignment keyed only on `inst.ty`:
  `if inst.ty != ITY_UNIT() {
     out.push_str(... " = __VERIFIER_nondet_", str2(c_nondet_suffix(inst.ty), "();\n"));
   }`
But `emit_c_function` declares the result slot as a struct when the id is a tracked Vec/String/HashMap/BTreeMap/Option var (compiler/c_emitter.vow:2102-2111, e.g. `__vow_string_t v{id};`). The fall-through callers `emit_string_op`->`emit_unmodelled` (line 1768) and `emit_v …[truncated]

**Proposed fix:** In `emit_unmodelled`, branch on whether `inst.id` is a tracked structured var (reuse the vec/string/hashmap/btreemap/option var sets already threaded into emit_inst) and, for struct results, emit a nondet on `.len` (and `.tag`/`.payload` for option) with the appropriate capacity assume, mirroring the FieldGet model path, instead of a scalar assignment. Apply the same fix to the Rust `emit_unmodelled`.

#### L3.95 Checked-arithmetic overflow in Release/Profile mode aborts via raw SIGILL trap, losing the `ArithmeticOverflow` diagnostic — `LOW` · ❌ refuted by cross-check
**finder:** `L3-codegen-rust` · **kind:** diagnostics-quality · **verdicts:** 1  
**Files:** `vow-codegen/src/cranelift_backend.rs:1677-1698`  
**Design ref:** §5.7 (checked operators abort with ArithmeticOverflow — no mode qualifier), §6.4 (structured diagnostics)  
**Evidence:** `emit_overflow_check` only calls the structured overflow handler when `ctx.overflow_ref` is `Some`:
```
if let Some(overflow_ref) = ctx.overflow_ref {
    builder.ins().call(overflow_ref, &[]);
}
builder.ins().trap(TrapCode::INTEGER_OVERFLOW);
```
But `overflow_id`/`overflow_ref` is only declared when `mode.has_debug_checks()` (compile_module, lines 2947-2968) — in Release/Profile, `overflow_ref` is `None`, so a `+!`/`-!`/`*!`/`/!` overflow emits only `trap(INTEGER_OVERFLOW)` → Cranelift `ud2` → …[truncated]

**Proposed fix:** Declare `__vow_arithmetic_overflow` (and pass `overflow_ref`) in all build modes, not just `has_debug_checks()`, so checked-arithmetic overflow always emits the structured `ArithmeticOverflow` envelope before terminating. The handler is tiny and non-allocating, so the release-size cost is negligible. Alternatively, document in §5.7 that the structured diagnostic is debug-only and the release contract is merely "aborts".

#### L3.96 Self-hosted checker silently accepts indexing of non-indexable types (returns CTY_UNIT) where Rust emits TypeMismatch — `LOW` · ❌ refuted by cross-check
**finder:** `L3-divergence` · **kind:** diagnostics-quality · **verdicts:** 1  
**Files:** `compiler/checker.vow:1235-1254`, `vow-types/src/check.rs:1239-1257`  
**Design ref:** §7 (differential trust); §6.4 (structured diagnostics)  
**Evidence:** Self-hosted EXPR_INDEX returns CTY_I64 for String, the element type for Applied (Vec/HashMap), and otherwise `return CTY_UNIT();` (checker.vow:1253) with no diagnostic for non-indexable receivers. Rust's Index arm emits TypeMismatch `index operation on non-indexable type` for any base that is not Ty::Applied (check.rs:1244-1255). Concrete divergent input: `fn f(x: i64) -> i64 { x[0] }` — self-hosted: recv_tag is neither APPLIED nor STR → returns CTY_UNIT, no error (then a downstream coercion may …[truncated]

**Proposed fix:** In checker.vow EXPR_INDEX, emit a TypeMismatch diagnostic when recv_tag is neither CTY_APPLIED (Vec/HashMap) nor CTY_STR, mirroring Rust's index-on-non-indexable error, instead of silently returning CTY_UNIT.

#### L3.97 Vec::clear / String::clear publish a spurious store effect, forcing an unnecessary hidden-arena ABI parameter — `LOW` · ❌ refuted by cross-check
**finder:** `L3-ir-region-rust` · **kind:** optimization · **verdicts:** 1  
**Files:** `vow-ir/src/region.rs:1796-1813`, `vow-ir/src/region.rs:1938-1945`  
**Design ref:** §5.2 (Receiver-growth effects), §3.5 (empty-region elision)  
**Evidence:** `extern_growth_target` returns the receiver for clear operations:
```rust
// region.rs:1799,1804
"__vow_vec_clear" if !args.is_empty() => Some(args[0]),
...
"__vow_string_clear" if !args.is_empty() => Some(args[0]),
```
and `handle_inst` then records a ConstantGlobal store effect on that receiver param:
```rust
// region.rs:1938-1945
if let Some(target_id) = extern_growth_target(sym, &inst.args)
    && let Some(target_param) = trace_param(target_id, inst_lookup)
{
    summary.store_effects.inser …[truncated]

**Proposed fix:** Remove `__vow_vec_clear` and `__vow_string_clear` from `extern_growth_target` (they should still remain in the rodata-mutation check list via a separate predicate if literal-clear must trap). Equivalently, gate the store-effect insertion at region.rs:1938 on `extern_mutation_operation` being an allocating op. Mirror in compiler/region.vow.

#### L3.98 Arena verification harness wrapped-pointer-bound (#437) is already fixed; assumes now use a non-wrapping form — `LOW` · ❌ refuted by cross-check · **Duplicate of #437**
**finder:** `L3-ir-region-rust` · **kind:** soundness · **verdicts:** 1  
**Files:** `vow-runtime/verify/arena.c:73-92`  
**Design ref:** §10.4 (verified invariants of the arena primitive)  
**Evidence:** Issue #437 reported `(uintptr_t)base + total <= 1 << 62` as an assume that wraps before comparison, letting ESBMC prove range assertions over wrapped arithmetic. The current harness uses the non-wrapping form recommended in the issue:
```c
// arena.c:81-86
__ESBMC_assume(total <= ARENA_VERIFY_ADDR_CAP);
__ESBMC_assume(base_addr <= ARENA_VERIFY_ADDR_CAP - total);
assert(base_addr + total <= ARENA_VERIFY_ADDR_CAP);
```
Because `total <= ARENA_VERIFY_ADDR_CAP` (=2^62) is assumed first, `ARENA_VERIF …[truncated]

**Proposed fix:** No code change required for #437 itself; verify the issue can be closed. If desired, add the same non-wrapping pattern note to any other harness that relies on bounded pointer arithmetic (the comment at arena.c:54-60 already documents the requirement).

#### L3.99 String::push_str self-append guard only checks descriptor identity, not shared backing buffer — `LOW` · ❌ refuted by cross-check
**finder:** `L3-runtime` · **kind:** bug · **verdicts:** 1  
**Files:** `vow-runtime/src/lib.rs:1729-1774`  
**Design ref:** §5.6 (arena memory model / no dangling); arena_memory.md shallow-descriptor-copy note (~line 1375)  
**Evidence:** `__vow_string_push_str_in_arena` protects against the dest==src UAF (where the reserve's `arena_grow_backing` may libc::free the abandoned oversized chunk) using only pointer-identity of the *descriptors*:
```rust
let src_is_dest = std::ptr::eq(src as *const VowVec, dest as *const VowVec);
...
let src_ptr_before_reserve = if src_is_dest { core::ptr::null() }
    else { unsafe { (*(src as *const VowVec)).ptr as *const u8 } };
unsafe { __vow_vec_reserve_in_arena(arena, dest, src_len, 1, 1) };
... …[truncated]

**Proposed fix:** Detect aliasing by backing-pointer overlap, not descriptor identity: after the reserve, if `src.ptr` (captured pre-reserve) falls inside the freed/old backing range, re-read from the post-reserve `dest.ptr` prefix as in the self-append branch; or, more simply, snapshot the `src` bytes into a temporary before reserving whenever `src.ptr` lies in the same chunk as `dest.ptr`. Add a test with two distinct descriptors sharing one oversized backing.

#### L3.100 make_extern_sig silently fabricates a no-arg/no-return signature for any unrecognized runtime symbol — `LOW` · ❌ refuted by cross-check
**finder:** `L3-shim` · **kind:** soundness · **verdicts:** 1  
**Files:** `vow-clif-shim/src/lib.rs:3505-3509`, `vow-clif-shim/src/lib.rs:925-938`  
**Design ref:** §6.5 (stable, machine-readable error reporting; tools must not silently produce wrong output); N/A — pure impl bug  
**Evidence:** The fallback arm of the giant symbol→signature match only logs a warning and returns an empty signature:
```
_ => {
    eprintln!("clif_shim: unknown extern sig for '{sym}', using no-arg no-return");
}
```
and `__vow_clif_declare_extern` accepts the result unconditionally (no error return):
```
let sig = make_extern_sig(&sym, &ctx.obj_module);
let cl_id = ctx.obj_module.declare_function(&sym, Linkage::Import, &sig).expect("declare extern");
ctx.extern_func_ids.insert(sym, cl_id);
```
If the self …[truncated]

**Proposed fix:** Make the fallback arm fatal: have `make_extern_sig` return `Option<Signature>` (or a Result), and have `__vow_clif_declare_extern` return a nonzero error code (the FFI already uses `i64` returns elsewhere) when the symbol is unknown, so the driver aborts with a structured diagnostic instead of declaring a mismatched signature. Alternatively, drive extern signatures from a single shared table also consumed by `vow-runtime`/`vow-codegen` so an omission is impossible.

---

## Peripheral — Benchmarks, Mutation Testing, Docs/Spec, Agent Surface

_24 findings — 0C / 7H / 9M / 8L._

#### P.1 Benchmark harness reports a program VERIFIED with no check that the submitted contracts match the skeleton; LLM can weaken/delete contracts to force-accept — `HIGH` · ✅ survived cross-check · **Duplicate of #485**
**finder:** `P-bench` · **kind:** soundness · **verdicts:** 1 · reviewer severity votes: medium×1  
**Files:** `bench/runner.py:108-157`, `bench/runner.py:32-59`, `bench/run.py:260-304`  
**Design ref:** §2.1 (verification is the trust mechanism); §7 (self-hosting/validation honesty); Contract Authoring policy in CLAUDE.md  
**Evidence:** runner.run_benchmark extracts whatever the model returns (`code = extract_vow_code(resp.content)`) and verifies that string verbatim:
```python
vr = run_verify(vow_binary, code, timeout=verify_timeout, memory_limit=memory_limit)
...
if vr.status == "Verified":
    ... status="verified" ...
```
The only instruction not to alter contracts is in the *prompt* text (prompts.py build_initial_user_prompt: "Do not change the module name, function signatures, or contracts."). Nothing in runner.py / verifier.py / run.py ever compares the submitted code's `requires`/`ensures`/`invariant` clauses (or signature) against `bench.skeleton_vow`/`bench.reference_vow`. A grep across bench/*.py for any contract/diff/signature equality check finds none. Consequently a model that rewrites `ensures: result == a || result == b` to `ensures: true`, deletes a `requires`, or trivializes a postcondition gets scored `status=="verified"` and counted into `summary.verified` / `verification_rate` (_compute_summary). The harness rewards a verifier false-accept of an under-specified program — exactly the failure class flagged for this lane (proptests passing on rejected programs).

**Proposed fix:** After extraction, parse the submitted file and assert its vow blocks (requires/ensures/invariant text, normalized via the compiler's canonical printer) and function signatures are byte-identical to the skeleton's, before calling run_verify. If they differ, classify the attempt as `contract_tampered` (a failure), never `verified`. Easiest robust implementation: run `vow contracts` (JSON) on both skeleton and submission and require the contract set to match.

#### P.2 Counterexample JSON emits key `values` but every spec copy (schema + cli.md + contracts.md + embedded skill) requires `inputs` — `HIGH` · ✅ survived cross-check
**finder:** `P-docs-spec` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #615**  
**Files:** `vow/src/main.rs:7936-7951`, `vow/src/main.rs:8079-8132`, `vow/src/main.rs:11992-12002`, `docs/spec/schemas/counterexample.schema.json:7-15`, `docs/spec/cli.md:283-296`, `docs/spec/contracts.md:304-312`  
**Design ref:** §6.5 (structured diagnostics/outputs so tools self-describe to agents without hidden training-set knowledge); schema drift breaks this goal  
**Evidence:** The emitted `CounterexampleJson` struct (main.rs:7936) declares `pub values: BTreeMap<String, String>` with no serde rename, and `from_structured` (main.rs:8083) populates `values: ce.values...`. The in-tree test at main.rs:11999 asserts the live wire shape: `assert_eq!(ces[0]["values"]["y"], "0");` — confirming the key is `values`. But `counterexample.schema.json` declares `"required": ["function", "inputs", "violation", "vow_id", "source"]` with `"additionalProperties": false`, and the `inputs` property: `"inputs": { "type": "object", ... "description": "Map of parameter names to counterexample values" }`. cli.md's VerifyFailed example shows `"inputs": { "a": "-9223372036854775808", "b": "0" }` and contracts.md repeats `"inputs"`. The embedded skill copy of the schema shipped to agents (main.rs:4189) ALSO requires `inputs`. A strict JSON-Schema validator rejects real compiler output on two counts: missing `inputs`, and the extra `values` key under `additionalProperties:false`. An agent following the documented schema looks for `inputs` and finds nothing.

**Proposed fix:** Pick one canonical name and make spec + impl agree. Least-risk: rename the spec to `values` (counterexample.schema.json, the embedded schema at main.rs:4189/7307, cli.md:283-296, contracts.md:304-312) since `values` is what is emitted and tested. Alternatively add `#[serde(rename = "inputs")]` on the field — but then update the test at main.rs:11999. Either way, regenerate the embedded skill schema so the shipped copy matches.

#### P.3 counterexample.schema.json (and embedded copy) omits emitted fields `blame`, `call_sites`, `violating_args`, `execution_path`, `branch_decisions` while forbidding additional properties — `HIGH` · ✅ survived cross-check
**finder:** `P-docs-spec` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #616**  
**Files:** `vow/src/main.rs:7935-7951`, `vow/src/main.rs:11994-11997`, `docs/spec/schemas/counterexample.schema.json:6-45`, `vow/src/main.rs:4183-4227`  
**Design ref:** §6.5 (stable structured outputs / error codes / blame categories for agents)  
**Evidence:** `CounterexampleJson` (main.rs:7936) serializes `blame: String` (always present) plus `call_sites`, `violating_args`, `execution_path`, `branch_decisions` (each `skip_serializing_if = "Vec::is_empty"`). The in-tree test confirms these reach the wire: main.rs:11994 `assert_eq!(ces[0]["blame"], "caller");` and main.rs:11995-11997 read `ces[0]["call_sites"][0]["caller_function"]`. But `counterexample.schema.json` defines only `function/inputs/violation/vow_id/source` and sets `"additionalProperties": false` (line 44). The same is true of the embedded schema at main.rs:4226. So a real counterexample that carries blame or a non-empty call_sites array fails schema validation. The build-result example in cli.md (lines 283-296) also omits `blame`/`call_sites`, so the prose is stale too.

**Proposed fix:** Add `blame` (enum Caller/Callee/None or string), and optional `call_sites`, `violating_args`, `execution_path`, `branch_decisions` array properties to counterexample.schema.json and the embedded copy, mirroring the CeCallSiteJson/CeViolatingArgJson/CePathStepJson/CeBranchDecisionJson field shapes in main.rs:7905-7934. Update the cli.md/contracts.md examples to show a representative blame + call_sites payload.

#### P.4 diagnostic.schema.json error_code enum omits 4 real ErrorCode variants (BTreeMapKeyTypeMustBeI64, BTreeMapValueMustBeNonLinear, VerificationSkipped, RegionLiteralMutation) — `HIGH` · ✅ survived cross-check
**finder:** `P-docs-spec` · **kind:** design-divergence · **verdicts:** 1 · reviewer severity votes: medium×1 · **Filed as #611**  
**Files:** `docs/spec/schemas/diagnostic.schema.json:11-35`, `vow/src/main.rs:4243-4266`, `vow-diag/src/lib.rs:36-80`, `vow/src/main.rs:8060-8060`, `vow-types/src/check.rs:1994-2003`, `vow-ir/src/region.rs:3117-3119`  
**Design ref:** §6.5 (stable error codes; schema is the agent-facing contract); HARD-RULE: VerificationSkipped is the fail-closed skip diagnostic, so its schema omission directly weakens the agent's ability to detect unproved-but-reported contracts  
**Evidence:** `error_code` is serialized verbatim from the enum: main.rs:8060 `error_code: format!("{:?}", d.code)`. The `ErrorCode` enum (vow-diag/src/lib.rs:36-80) defines `BTreeMapKeyTypeMustBeI64`, `BTreeMapValueMustBeNonLinear`, `VerificationSkipped`, and `RegionLiteralMutation`, all of which are constructed as compile-time `Diagnostic`s (check.rs:1994 `ErrorCode::BTreeMapKeyTypeMustBeI64`, check.rs:2003 `ErrorCode::BTreeMapValueMustBeNonLinear`, region.rs:3119 `code: ErrorCode::RegionLiteralMutation` with `severity: Severity::Error`, main.rs:8825 `code: vow_diag::ErrorCode::VerificationSkipped`). errors.md documents all four as emitted diagnostics. Yet both the standalone diagnostic.schema.json enum (lines 12-33) and the embedded copy (main.rs:4244-4265) list only 21 codes and exclude these four. A strict validator rejects valid compiler output (e.g. a BTreeMap<bool,i64> type error or a skipped-verification warning).

**Proposed fix:** Add `BTreeMapKeyTypeMustBeI64`, `BTreeMapValueMustBeNonLinear`, `VerificationSkipped`, and `RegionLiteralMutation` to the error_code enum in both docs/spec/schemas/diagnostic.schema.json and the embedded schema in main.rs (lines 4243-4266 and 7358-region). Regenerate the skill. Consider a test that asserts every ErrorCode Debug string is present in the schema enum to prevent recurrence.

#### P.5 Mutation oracle has no baseline (unmutated-tree) check; a globally-broken Tier-1 oracle silently scores every mutant as caught — `HIGH` · ✅ survived cross-check
**finder:** `P-mutants` · **kind:** diagnostics-quality · **verdicts:** 1 · reviewer severity votes: medium×1 · **Filed as #617**  
**Files:** `compiler/mutants_main.vow:487-659`, `compiler/mutants_oracle.vow:33-60`  
**Design ref:** §2.3 (tools are part of the programming model) / §6.5 (agent-facing tooling must be trustworthy); N/A — impl/tooling correctness bug  
**Evidence:** `run_mutants_run` never runs the oracle once on the UNMUTATED worktree before the per-mutant loop. The loop classifies Tier-1 results purely by exit code: `} else if rc1 != 0 { status = ST_CAUGHT(); }` (mutants_main.vow:606-607). The default Tier-1 oracle is `scripts/bootstrap.sh --skip-cargo` (mutants_main.vow:496). With `--skip-cargo`, Stage 1 runs `./target/release/vow build ...` (scripts/bootstrap.sh:199) using a RELATIVE path, and the per-mutant command first does `cd <workdir>` (cd_with_log, mutants_main.vow:313). The fresh worktree's `target/` is empty (docs/mutants.md:37 states this), so `./target/release/vow` does not exist there, the oracle exits nonzero for EVERY mutant, and every mutant is scored `caught`. The summary would read e.g. `caught: N, missed: 0` — reporting a flawless test suite when in fact NOTHING was tested. cargo-mutants prevents exactly this by running the unmutated tree first and aborting if it fails. There is no such guard here (grep for `baseline`/`unmutated` finds nothing in the runner).

**Proposed fix:** Before the per-mutant loop, run the Tier-1 (and ideally Tier-2) oracle once against the unmutated worktree. If it does not exit 0, abort the run with a clear diagnostic (`baseline oracle failed; mutation results would be meaningless`) instead of proceeding. This converts the silent 100%-caught failure mode into a loud, actionable error.

#### P.6 Machine-readable --help reports the wrong default output path for `-o, --output` (says "source without .vow extension", actual is `build/<stem>`) — `HIGH` · ✅ survived cross-check
**finder:** `P-skill-help` · **kind:** bug · **verdicts:** 1 · reviewer severity votes: medium×1 · **Filed as #592**  
**Files:** `scripts/generate_help.py:230-235`, `vow/src/main.rs:391-396`, `compiler/main.vow:1426-1431`  
**Design ref:** §6.5 (machine-readable --help must describe the tool without hidden knowledge); §2.3 (surface language and tools form one system)  
**Evidence:** generate_help.py hardcodes a stale default at line 233:
  `output_default="source without .vow extension" if flag == "-o, --output" else None,`
This flows into the canonical agent interface. `vow --help` JSON (and the embedded copies in both compilers) therefore emit:
  main.rs:391 `"description": "Output executable path (default: source without .vow extension)",`
  main.rs:396 `"default": "source without .vow extension"`
but the ACTUAL compiler default is `build/<stem>`. Rust main.rs:9164-9166:
  `let output_path = output.map(...).unwrap_or_else(|| { let stem = source.file_stem()...; Path::new("build").join(stem) });`
Self-hosted compiler/main.vow:127 `let r: String = String::from("build/");` in `default_output`.
The spec (docs/spec/cli.md:18) and the bundled `reference/cli.md` agree with the code: `| -o, --output | build/<stem> | Output executable path |`. I reproduced the live discrepancy: `vow --help` printed `"default": "source without .vow extension"` while the binary actually lands at `build/<stem>`. git history confirms transition debt: commit bb0f416 ("default output to build/<stem> instead of next to source") moved the real default and cli.md was updated, but the hardcoded string in generate_help.py (originally from db00bda) was never updated. An agent that parses `--help` to locate the compiled binary will look at `examples/divide` when the binary is actually at `build/divide`, breaking the write→build→run loop the skill prescribes.

**Proposed fix:** In scripts/generate_help.py line 233 change the hardcoded `output_default` to `"build/<stem>"` (matching cli.md:18), then regenerate via `uv run python scripts/generate_help.py` and rebuild both compilers so the embedded JSON/human help and the bundled cli.md agree. Better: stop hardcoding the default at all and read it from the cli.md table row (column 1) so it can never diverge from the spec again.

#### P.7 Benchmark reference/skeleton contracts are under-specified (weak postconditions), so trivially-wrong implementations score as VERIFIED — overstating Vow's verified-correctness rate — `HIGH` · ❌ refuted by cross-check
**finder:** `P-bench` · **kind:** diagnostics-quality · **verdicts:** 1 · reviewer severity votes: medium×1  
**Files:** `benchmarks/medium/M13_gcd/skeleton.vow:3-11`, `benchmarks/easy/E02_max_of_two/skeleton.vow:3-7`, `benchmarks/easy/E03_min_of_two/skeleton.vow:3-6`, `benchmarks/easy/E04_clamp/skeleton.vow:3-8`  
**Design ref:** §2.1; CLAUDE.md Contract Authoring ("Postconditions should be tight")  
**Evidence:** M13 gcd contract is `ensures: result > 0` with body skeleton `{ 1 }`. The reference is supposed to be ground truth, yet `fn gcd(a,b) { 1 }` satisfies `result > 0` for every valid input, so a constant-1 'gcd' verifies and scores as a pass. E02 `max_of`: `ensures: result >= a, ensures: result >= b` admits any value >= max(a,b) (e.g. `a + b + 100` for positives), not `result == a || result == b`. E03 `min_of`: symmetric (`result <= a, result <= b`). E04 `clamp`: `ensures: result >= lo, ensures: result <= hi` is satisfied by `fn clamp(x,lo,hi){ lo }` which ignores `x` entirely and is wrong. These exactly match the bad-contract example the repo's own Contract Authoring rule forbids: "`min(a, b)` must ensure `result == a || result == b`, not just `result <= a && result <= b`. A weak postcondition that admits incorrect implementations is a bad contract." Combined with the harness scoring any verifying body as a pass, the suite rewards verifier-accepts of incorrect programs and inflates the headline rate compared against Dafny/Verus/Lean.

**Proposed fix:** Tighten the postconditions to the true semantic spec: max -> `(result == a || result == b) && result >= a && result >= b`; min -> dual; clamp -> `(result == x || result == lo || result == hi) && (lo <= x && x <= hi -> result == x)`; gcd -> divisibility + maximality (or at minimum `result divides a && result divides b`). Update reference.vow, skeleton.vow, and spec.md together. Add a CI guard that a stub body returning a constant/default for each benchmark FAILS verification (negative test) so weak contracts can't silently regress.

#### P.8 OpenAI reasoning models configured in the suite (o3) will crash the harness: llm.py sends max_tokens and temperature=0.0, both rejected by o-series chat completions — `MEDIUM` · ✅ survived cross-check
**finder:** `P-bench` · **kind:** bug · **verdicts:** 1 · reviewer severity votes: low×1 · **Filed as #629**  
**Files:** `bench/llm.py:27-32`, `bench/llm.py:73-92`, `bench/config.toml:18-20`  
**Design ref:** §6.5 (agent-facing tooling must actually run); N/A — pure impl bug  
**Evidence:** config.toml lists `id = "o3-2025-04-16"` and `_get_provider` routes `gpt/o1/o3/o4` to OpenAI. `_chat_openai` calls:
```python
resp = client.chat.completions.create(
    model=config.model_id,
    max_tokens=config.max_tokens,
    temperature=config.temperature,  # 0.0
    messages=oai_messages,
)
```
OpenAI o1/o3/o4 reasoning models reject the `max_tokens` parameter (they require `max_completion_tokens`) and reject any `temperature` other than the default (1.0). Running `--model o3-2025-04-16` (or `--all`, which would include OpenAI models if extended) raises a 400 BadRequest, aborting the whole model run with no result rows written. The harness therefore cannot benchmark the very reasoning models it advertises.

**Proposed fix:** In _chat_openai, detect reasoning models (model_id startswith o1/o3/o4) and (a) use `max_completion_tokens` instead of `max_tokens`, and (b) omit `temperature` (or set it to 1.0). Optionally encode per-model param quirks in config.toml. Add a smoke test that constructs the request kwargs for an o3 config and asserts no forbidden keys are present.

#### P.9 diagnostic.schema.json forbids additionalProperties but the compiler emits `secondary` and `blame` on diagnostics — `MEDIUM` · ✅ survived cross-check
**finder:** `P-docs-spec` · **kind:** design-divergence · **verdicts:** 1 · **Filed as #652**  
**Files:** `docs/spec/schemas/diagnostic.schema.json:6-57`, `vow/src/main.rs:7891-7903`, `vow/src/main.rs:8890-8901`  
**Design ref:** §6.5 (structured diagnostics + stable blame categories must be reflected in the agent-facing schema)  
**Evidence:** `DiagnosticJson` (main.rs:7891) serializes `span`, plus `hints` (skip-if-empty), `secondary: Vec<SpanJson>` (skip-if-empty), and `blame: Option<String>` (skip-if-none). VerifyFailed diagnostics populate both: main.rs:8898-8899 sets `secondary` (from counterexample call_sites) and `blame: blame_to_diag_blame(&sce.blame)`. But diagnostic.schema.json declares only `error_code/message/severity/span` and sets `"additionalProperties": false` (line 57). It does not define `hints`, `secondary`, or `blame`, so any diagnostic carrying a blame or secondary span (every contract-violation diagnostic) fails strict validation. cli.md's CompileFailed example (lines 253-272) likewise shows no `blame`/`secondary`/`hints`.

**Proposed fix:** Add optional `hints` (array of string), `secondary` (array of the same span object), and `blame` (enum ["Caller","Callee","None"] or string) properties to diagnostic.schema.json and the embedded copy in main.rs. These are already part of the diagnostic contract documented in CLAUDE.md's Blame model.

#### P.10 Tier-2 budget starvation misclassifies real coverage gaps as TIMEOUT instead of UNRUN — `MEDIUM` · ✅ survived cross-check
**finder:** `P-mutants` · **kind:** diagnostics-quality · **verdicts:** 1 · reviewer severity votes: low×1 · **Filed as #653**  
**Files:** `compiler/mutants_main.vow:608-628`  
**Design ref:** §6.5 (explicit coverage gaps over silent ones); design-divergence vs docs/mutants.md §Limitations (budget→unrun)  
**Evidence:** When a Tier-1 survivor is reached and `t2_used_ms < t2_budget_ms` but the remaining budget is tiny, the per-mutant Tier-2 timeout is clamped to the leftover budget: `let remaining_ms = t2_budget_ms - t2_used_ms; let this_t2_to_ms = if t2_to_ms < remaining_ms { t2_to_ms } else { remaining_ms };` (lines 618-619). With only, say, a few hundred ms left, `full_test.sh` cannot complete; `run_shell` clamps to a 1s `timeout(1)` (mutants_oracle.vow:34-35), the oracle is killed, `run_shell` returns -2, and `classify_oracle_rc(-2)` yields `ST_TIMEOUT()` (line 627; mutants_main.vow:272-277). The mutant is recorded `timeout` — visually a "slow test" — even though the true cause is budget exhaustion. The docs promise budget exhaustion surfaces as `unrun` ("surviving Tier-1 mutants beyond the budget are emitted with status:\"unrun\"", docs/mutants.md:123). A mutant that is actually MISSED (a genuine coverage hole) is thus buried in the timeout bucket rather than the missed/unrun buckets the operator scans.

**Proposed fix:** Before launching Tier-2, require a minimum viable slice of budget (e.g. compare `remaining_ms` against an estimate / floor; the simplest correct rule is: if `remaining_ms < t2_to_ms` then mark `unrun` rather than running a guaranteed-to-fail truncated oracle). That keeps the `unrun` semantics the docs promise and avoids polluting the `timeout` bucket with budget artifacts.

#### P.11 A `cd <workdir>` failure inside the oracle command is scored as `caught` without running any test — `MEDIUM` · ✅ survived cross-check
**finder:** `P-mutants` · **kind:** diagnostics-quality · **verdicts:** 1 · reviewer severity votes: low×1 · **Filed as #654**  
**Files:** `compiler/mutants_main.vow:312-330`, `compiler/mutants_main.vow:595-607`  
**Design ref:** §6.5 (trustworthy tool outputs); N/A — impl bug  
**Evidence:** The oracle command is `cd <workdir> && (echo --- tier1 --- && <cmd>) > <log> 2>&1` (cd_with_log, lines 313-329). The compound's exit status is `cd`'s when `cd` fails: if the worktree is removed/inaccessible mid-run (NFS hiccup, external `git worktree prune`, disk error), `cd <workdir>` exits nonzero and the `&&` short-circuits — the oracle body never runs. Back in the loop, `rc1 != 0` → `status = ST_CAUGHT()` (lines 606-607). Every subsequent mutant is then silently scored `caught` despite never being tested. Unlike the restore-failure path which aborts loudly (lines 634-641), there is no detection of this infrastructure failure; it masquerades as perfect kill coverage.

**Proposed fix:** Separate infrastructure failure from oracle verdict: have the wrapper emit a sentinel when `cd` fails (e.g. `cd <workdir> || exit 125; ...`) and treat exit 125 (or a missing-workdir probe) as an abort condition rather than `caught`. A periodic `fs_exists(workdir)` check or a baseline re-probe between mutants would also surface a vanished worktree loudly.

#### P.12 No staleness detector cross-references cli.md (CLI flags/defaults/commands) against --help JSON; --check only validates the skills/ dir, not embedded help — `MEDIUM` · ✅ survived cross-check
**finder:** `P-skill-help` · **kind:** diagnostics-quality · **verdicts:** 1 · reviewer severity votes: low×1 · **Filed as #625**  
**Files:** `scripts/check_help_coverage.py:65-127`, `scripts/generate_help.py:1125-1144`, `scripts/full_test.sh:756-778`  
**Design ref:** §6.5 (explicit command boundaries; tools describe themselves without hidden knowledge); §4.5 (tooling is part of the language contract)  
**Evidence:** check_help_coverage.py takes only `<grammar.md> <help-json>` (line 67 usage string, lines 70-73) and validates exclusively the `language` section — primitive types, parameterized types, effects, builtins, and a fixed `required_keys` list (lines 88-127). It never reads cli.md and never inspects `commands`, `command_details`, `build_options`, `--mode` values, defaults, exit codes, or status values. CLAUDE.md itself only claims grammar coverage: "will catch drift between grammar.md and --help." Separately, `generate_help.py --check` (lines 1125-1144) only calls `check_skills_dir(...)` — it confirms the on-disk `skills/vow/` mirror matches, but does NOT confirm the *embedded* `skill_json`/`skill_human` blocks inside main.rs / main.vow match the generator output. full_test.sh:772 runs exactly that `--check`. Net effect: the entire CLI surface (every flag, default, mode, command) has zero automated drift protection, and the embedded help could be hand-edited or left un-regenerated without detection. This is the root cause that let the `-o, --output` default divergence (separate finding) ship unnoticed.

**Proposed fix:** Extend the staleness check to (a) re-derive the help JSON from grammar.md+cli.md and assert it byte-equals the embedded `skill_json`/`skill_human` blocks in main.rs and main.vow (the inject_rust/inject_vow round-trip already exists — just compare instead of write under --check), and (b) cross-reference each cli.md options-table row (flag, default) against the corresponding `command_details[*].options` entry so a spec/help default mismatch fails CI.

#### P.13 `skill` command present in --help `commands` but absent from `command_details`; the skill subcommand grammar (print/--bundle/install/--local/--global) is undiscoverable from the canonical interface — `MEDIUM` · ✅ survived cross-check · **Duplicate of #419**
**finder:** `P-skill-help` · **kind:** design-divergence · **verdicts:** 1 · reviewer severity votes: low×1  
**Files:** `scripts/generate_help.py:343-443`, `docs/spec/cli.md:72-91`  
**Design ref:** §6.5 (machine-readable --help, explicit command boundaries); related to open issue #458 (surface --bundle in entrypoint) but distinct: that issue targets the human SKILL.md, this targets the JSON command_details  
**Evidence:** generate_help.py `commands` (lines 343-350) advertises six commands including `"skill": "Generate or install the Claude Code skill document..."` (line 349), but `command_details` (lines 351-443) only documents five: build, verify, test, decl, contracts. There is no `command_details["skill"]`. An agent reading the machine-readable `--help` therefore cannot discover the subcommand structure that cli.md:72-91 documents in detail: `vow skill print`, `vow skill print --bundle` (the self-contained bundle for raw-API harnesses), `vow skill install --local`, `vow skill install --global`. Per §6.5 the toolchain is supposed to "describe itself to agents without requiring hidden training-set knowledge" — here the only way to learn `--bundle` or the install scopes is to read prose (cli.md), defeating the self-describing contract for a command the JSON explicitly lists.

**Proposed fix:** Add a `command_details["skill"]` entry describing the `print [--bundle]` and `install [--local|--global]` subcommands, their stdout/side-effects, and the auto-install-on-build behavior (cli.md:91), so the canonical JSON interface is complete. Derive it from the cli.md `vow skill` section rather than hardcoding.

#### P.14 SKILL.md live `--help` command fails the skill load (non-zero exit) when neither `vow` nor `build/vowc` is resolvable — `MEDIUM` · ✅ survived cross-check · **Duplicate of #582**
**finder:** `P-skill-help` · **kind:** bug · **verdicts:** 1 · reviewer severity votes: low×1  
**Files:** `skills/vow/SKILL.md:19-21`, `scripts/generate_help.py:866-868`  
**Design ref:** §6.5 (tooling is part of the language contract; the skill is the operational surface); §2.3  
**Evidence:** SKILL.md:21 (generated from generate_help.py:868) embeds a live command:
  `!`(command -v vow >/dev/null 2>&1 && vow --help 2>/dev/null | head -200) || (command -v build/vowc >/dev/null 2>&1 && build/vowc --help 2>/dev/null | head -200)``
When `vow` is not on PATH and `build/vowc` is not resolvable relative to cwd, the whole `||` chain exits non-zero. Claude Code treats a non-zero exit from a `!`backtick`` skill command as a hard failure, producing exactly the error reported in #582: `Error: Shell command failed for pattern "!`(command -v vow ...)`"`. The skill thus fails to load in any project where the compiler hasn't been bootstrapped yet — i.e. precisely the moment an agent most needs the skill to learn how to build it. (Note: `command -v build/vowc` is also a relative path check that only succeeds when cwd is the repo root, so the fallback is fragile even when the binary exists.)

**Proposed fix:** Make the live command always exit 0, e.g. append `|| echo '(vow toolchain not found on PATH; run scripts/bootstrap.sh, then build/vowc --help)'` to the chain, and/or fall back to printing the embedded reference/cli.md so the skill body is never empty. Track under #582.

#### P.15 Benchmark contracts bake ESBMC --unwind bounds into requires clauses (e.g. gcd requires a<=8,b<=8; vec ops len<=8; collatz n<=4), violating the contracts-are-semantic policy and proving only a bounded slice while reporting 'Verified' — `MEDIUM` · ❌ refuted by cross-check · **Duplicate of #552**
**finder:** `P-bench` · **kind:** design-divergence · **verdicts:** 1 · reviewer severity votes: low×1  
**Files:** `benchmarks/medium/M13_gcd/skeleton.vow:3-10`, `benchmarks/medium/M03_vec_sum/skeleton.vow:5`, `benchmarks/medium/M15_collatz_bounded/skeleton.vow:5`, `benchmarks/README.md:60-66`  
**Design ref:** §2.1; §4.3 (verification tractability gates features, not contracts); CLAUDE.md Contract Authoring ("ESBMC bounds are not contracts")  
**Evidence:** M13 gcd: `requires: a > 0, requires: b > 0, requires: a <= 8, requires: b <= 8`. M03/M05/M06/M14: `requires: v.len() <= 8` / `requires: n <= 8`. M15: `requires: n <= 4`. benchmarks/README.md states it outright: "All loop-based problems include `requires: n <= 8` or similar bounds to stay within the unwind limit." The repo's own policy (CLAUDE.md Contract Authoring + memory note feedback_contracts_no_verifier_bounds) explicitly forbids this and uses gcd as the canonical example: "`gcd(a, b)` ... does not require `a <= 50` — that is a verifier limitation, not a property of Euclid's algorithm." The design doc §2.1 and the decouple-language-from-prover principle require BMC bounds never to leak into contracts. Effect: each such benchmark proves correctness only for tiny inputs yet the harness reports the function as `Verified` with no indication the proof is bounded, again overstating the trust claim.

**Proposed fix:** Remove the `<= N` unwind-driven bounds from `requires`. Keep only genuine semantic constraints (e.g. gcd: a>0,b>0). Address ESBMC's unwind limit at the tool layer (per-benchmark `--max-k-step`/`--unwind` settings in meta.toml consumed by the verifier wrapper), not in the program's contract. If a function genuinely cannot be proven unbounded, mark it Stretch/unverifiable rather than weakening its contract.

#### P.16 Docs/CLAUDE.md claim "union of mutants.out/ across shards is well-defined" but outputs overwrite each other when shards share an --output-dir — `MEDIUM` · ❌ refuted by cross-check
**finder:** `P-mutants` · **kind:** design-divergence · **verdicts:** 1 · reviewer severity votes: low×1  
**Files:** `compiler/mutants_main.vow:422-484`, `compiler/mutants_main.vow:593-594`, `docs/mutants.md:113-117`  
**Design ref:** §6.5 (deterministic, stable agent-facing outputs); design-divergence vs docs/mutants.md §Determinism guarantee  
**Evidence:** docs/mutants.md:117 states: "the union of `mutants.out/` across shards is well-defined, so partial nightly progress accumulates over runs" and CLAUDE.md repeats it. But every output artifact is written with `fs_write` (truncating overwrite) at a FIXED path inside output_dir: `write_mutants_json`→`join_path(output_dir, "mutants.json")` (line 423), `write_outcomes_json`→`outcomes.json` (line 435), `write_status_lists`→`<status>.txt` (line 479, comment at 456-457 even admits cargo-mutants appends these but "we write them all at once"). Per-mutant diff/log filenames use the SHARD-LOCAL index `i`: `diff_path_for(output_dir, i)` and `log_path_for(output_dir, i)` (lines 593-594), and the schema confirms `Outcome.id` is "Position of this mutant within the shard's mutants.json array" (mutants-result.schema.json). Running shard 1/8 into the same output_dir after 0/8 therefore overwrites mutants.json/outcomes.json/*.txt and collides diff/0.diff & logs/0.log between unrelated mutants. No union accumulates; only the last shard's results survive.

**Proposed fix:** Either (a) update docs/mutants.md and CLAUDE.md to require a distinct `--output-dir` per shard (the only currently-correct usage), or (b) make the union real: namespace per-shard files (e.g. `mutants.<X>-<Y>.json`, `diff/<globalid>.diff`) and make `<status>.txt` append-or-merge so a multi-shard run into one dir actually unions. Pick one and make code + docs agree.

#### P.17 benchmarks/README.md is stale: claims 40 benchmarks (E01–E15/M01–M15/H01–H10) while the manifest defines 107 (incl. 67 HumanEval); difficulty counts and tier ranges are wrong — `LOW` · ✅ survived cross-check
**finder:** `P-bench` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #665**  
**Files:** `benchmarks/README.md:3-26`, `benchmarks/manifest.toml:1-5`  
**Design ref:** §6.5 (machine/agent-facing docs must be accurate); CLAUDE.md spec-is-source-of-truth  
**Evidence:** README: "A suite of 40 specification-driven programming problems"; tier table "Easy (E01–E15) 15 | Medium (M01–M15) 15 | Hard (H01–H10) 10". manifest.toml header: `total = 107`, `stretch_count = 4`; counting entries: 107 `[[benchmarks]]`, of which 67 have ids starting `HE` (HumanEval). grep of difficulty: 34 easy, 53 medium, 20 hard. The README's counts, id ranges, and the absence of any mention of the 67-benchmark HumanEval set are all incorrect, so an agent reading the README to understand the …[truncated]

**Proposed fix:** Regenerate the README counts and tier ranges from manifest.toml (or have a small script assert README counts == manifest counts in full_test.sh). Document the HumanEval subset and clarify whether the headline verification_rate includes HE benchmarks, since report.py compares the combined rate directly against Dafny/Verus/Lean paper numbers.

#### P.18 Report compares Vow's combined verification_rate (E/M/H + 67 HumanEval, with weak/bounded contracts) directly against Dafny/Verus/Lean paper rates — an apples-to-oranges headline — `LOW` · ✅ survived cross-check
**finder:** `P-bench` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #666**  
**Files:** `bench/report.py:44-79`, `bench/run.py:291-304`  
**Design ref:** §7 (self-hosting/validation must be an honest test); N/A — reporting honesty  
**Evidence:** _compute_summary produces a single `verification_rate = verified / total` over ALL applicable benchmarks (E+M+H+HE). report.generate_report prints that rate in the same table as hard-coded rows `| Dafny (paper) | ... | 82% |`, `| Verus/Rust (paper) | ... | 44% |`, `| Lean (paper) | ... | 27% |`. The Vow denominator includes 67 HumanEval benchmarks and the weak/bounded-contract Easy/Medium problems documented in the other findings, so the single-number comparison against the Vericoding paper rate …[truncated]

**Proposed fix:** Report vow-suite (E/M/H) and HumanEval rates as separate rows, and only compare the vow-suite rate against the Vericoding paper numbers (which are not HumanEval). Footnote that several easy/medium contracts are intentionally weak/bounded so readers can interpret the rate. summary already has humaneval_rate; surface it in the table instead of folding HE into the headline.

#### P.19 `vow test --mode` accepts profile/sanitize at parse time, but cli.md and --help document only debug|release — `LOW` · ✅ survived cross-check
**finder:** `P-docs-spec` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #685**  
**Files:** `docs/spec/cli.md:109-110`, `vow/src/main.rs:222-224`, `vow/src/main.rs:631-633`, `vow/src/main.rs:10068-10076`  
**Design ref:** §6.5 (machine-readable --help should accurately enumerate accepted flag values)  
**Evidence:** `TestArgs.mode` (main.rs:223) is `#[arg(long, value_enum, default_value = "debug")] mode: ModeArg`, and `ModeArg` has four values (Debug/Release/Profile/Sanitize, main.rs:36-41). So `vow test --mode profile` and `--mode sanitize` both parse successfully; profile is then rejected at runtime (main.rs:10071 `eprintln!("Error: --mode profile is not supported for test subcommand")`) while sanitize silently runs. But cli.md only documents `--mode debug` and `--mode release` (lines 109-110) and the --h …[truncated]

**Proposed fix:** Either restrict the test subcommand to a 2-value enum so clap rejects profile/sanitize at parse time with a clear error, or document the full `<debug|release|profile|sanitize>` set in cli.md and the --help JSON and state that profile is unsupported for tests. Prefer the former (parse-time rejection) for agent clarity.

#### P.20 Rust `vow mutants` exits with code 2, an exit code cli.md's Exit Codes table does not list — `LOW` · ✅ survived cross-check
**finder:** `P-docs-spec` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #686**  
**Files:** `vow/src/main.rs:10162-10168`, `docs/spec/cli.md:212-222`  
**Design ref:** §6.5 (explicit, documented command boundaries / exit semantics for agents)  
**Evidence:** The Rust bootstrap compiler's `mutants` handler does `std::process::exit(2)` (main.rs:10167) after printing the redirect message. cli.md's Exit Codes section (lines 213-218) documents only `0` (Success) and `1` (Failure), and the prose at cli.md:170-172 says the Rust path 'emits an error pointing the user to build/vowc' without specifying the exit code. An agent that maps exit codes per the spec table will treat 2 as undefined.

**Proposed fix:** Document exit code 2 (usage/unsupported-subcommand) in cli.md's Exit Codes table, or change the mutants redirect to exit 1 for consistency with the documented failure code.

#### P.21 compute_skip_ranges scans GENERATE markers without string/comment awareness; an unterminated GENERATE:START leaves its region mutable — `LOW` · ✅ survived cross-check
**finder:** `P-mutants` · **kind:** bug · **verdicts:** 1 · **Filed as #688**  
**Files:** `compiler/mutants_sites.vow:543-579`, `compiler/mutants_sites.vow:667-692`  
**Design ref:** §7 (self-hosting test discipline); N/A — impl robustness bug  
**Evidence:** `compute_skip_ranges` does a raw line-by-line scan with no string/comment awareness (lines 547-577): any line that `string_starts_with("// GENERATE:")` and `string_ends_with(":START")` is treated as a marker, even if it occurs inside a string literal (e.g. a Vow string `"// GENERATE:X:START"` emitted by the help generator). More importantly, if `find_generate_end` finds no matching `:END` it returns -1 (line 691) and the range is NOT added (guard `if end_close > line_end`, line 569), so an unter …[truncated]

**Proposed fix:** Make compute_skip_ranges' marker scan string/comment-aware (mirror add_extern_ranges' handling), and on an unmatched GENERATE:START emit a stderr warning rather than silently leaving the region mutable, so a malformed marker is surfaced instead of degrading skip-list coverage.

#### P.22 enumerate_root does not recurse into subdirectories; sites under a nested --root are silently dropped — `LOW` · ✅ survived cross-check
**finder:** `P-mutants` · **kind:** diagnostics-quality · **verdicts:** 1 · **Filed as #689**  
**Files:** `compiler/mutants_main.vow:222-246`  
**Design ref:** §6.5 (no silent gaps); design-divergence vs docs/mutants.md flag description  
**Evidence:** `enumerate_root` calls `fs_listdir(read_dir)` and mutates only direct-child `.vow` files (`if ends_with(name, ".vow") && !starts_with(name, "test_")`, line 229); there is no recursion into subdirectories. The default `compiler/` is flat so this is fine today, but `--root DIR` is documented generically as "Directory whose `*.vow` files are mutated" (docs/mutants.md:21) with no stated flatness restriction. Pointing `--root` at any tree with subdirectories (e.g. `lib/` with `lib/math/*.vow`) silent …[truncated]

**Proposed fix:** Either recurse into subdirectories (skipping `test_*.vow` at every level) or document explicitly that `--root` is single-level and warn when the root contains subdirectories that will be ignored.

#### P.23 Oracle exit codes 124/137 from inside the test suite are misclassified as TIMEOUT, hiding genuine catches — `LOW` · ❌ refuted by cross-check
**finder:** `P-mutants` · **kind:** diagnostics-quality · **verdicts:** 1  
**Files:** `compiler/mutants_oracle.vow:51-59`  
**Design ref:** §6.5 (accurate structured outputs); N/A — impl heuristic limitation  
**Evidence:** `run_shell` maps BOTH 124 and 137 to the -2 TIMEOUT sentinel: `if rc == 124 || rc == 137 { return -2; }` (mutants_oracle.vow:56-57). The comment assumes these only ever come from the OUTER coreutils `timeout(1)` wrapper. But `full_test.sh` (the Tier-2 oracle) can legitimately produce these from its own children: a kernel OOM-kill of a test process yields 137 (128+SIGKILL), and any nested `timeout` inside the suite yields 124. A mutation that makes memory blow up or a sub-step hang is a REAL dete …[truncated]

**Proposed fix:** Distinguish outer-wrapper timeouts from inner exit codes structurally rather than by value. E.g. detect the wrapper timeout via `process_wait_timeout`'s own -2 / the wrapper's distinct signaling, and pass through 124/137 originating inside the oracle as nonzero→caught. At minimum, document that 124/137 from the suite are folded into timeout so operators don't misread the bucket.

#### P.24 `vow mutants` command is invisible in --help JSON and human help, yet its result schema is referenced — incomplete self-describing surface — `LOW` · ❌ refuted by cross-check
**finder:** `P-skill-help` · **kind:** design-divergence · **verdicts:** 1  
**Files:** `scripts/generate_help.py:327-350`, `docs/spec/cli.md:170-197`  
**Design ref:** §6.5 (explicit command boundaries for build/verify/debug/contract inspection; self-describing surface)  
**Evidence:** The help JSON references the mutants result schema (generate_help.py:327 `"mutants_result": "schemas/mutants-result.schema.json"`) but `mutants` appears nowhere in `commands` (lines 343-350) or `command_details`, and the human help (build_help_human) never mentions it — I verified `'mutants' in human help: False` and that the only JSON occurrence is the schema ref. cli.md:170-197 fully documents `vow mutants version|list|run` with all flags. So the canonical interface advertises a schema for a c …[truncated]

**Proposed fix:** Add a `mutants` entry to `commands` and a `command_details["mutants"]` (or a clearly-labeled `"availability": "self-hosted only"` note) derived from the cli.md mutants section, so the JSON surface is internally consistent with the schema it already ships. At minimum, document in the JSON why the command is omitted from the Rust path.

---

## Appendix — Filed Issues

Every surviving finding was filed on milestone **0.4.0 - Tighten** (or linked to a pre-existing issue). Issues opened by this audit are marked **new**; findings that duplicate an existing issue link to it (`pre-existing`). The 23 refuted findings were **not** filed. The index is deduplicated by issue number — clustered findings (same root cause / same `file:line`) share a single issue whose body carries each finder's view.

**Totals:** 111 new issues opened, 22 pre-existing issues linked.


### Lane 1 — Language Design for Agentic Coding  _(29 issues)_

| Issue | Severity | Title | Origin |
|---|---|---|---|
| #335 | critical | Checked arithmetic family (+!, -!, *!, /!, %!) abort-on-overflow semantics are NOT modeled by the verifier … | pre-existing |
| #586 | critical | Self-hosted compiler performs NO purity check on vow-clause predicates (requires/ensures/invariant) | **new** |
| #588 | critical | Self-hosted (primary) compiler never emits LinearTypeViolation: double-consume, consume-in-loop, and partia… | **new** |
| #583 | high | Refinement-type predicate `{ x: T \|\| pred }` is silently erased with no diagnostic (parsed, type-checked … | **new** |
| #589 | high | `where`-clause refinement expression is never type-checked, yet is lowered into a verifier `__ESBMC_assume`… | **new** |
| #593 | high | Borrow printer drops parentheses: `&(a + b)` becomes `&a + b`, changing the AST on reparse | **new** |
| #594 | high | Question (`?`) printer drops parentheses: `(a + b)?` becomes `a + b?`, changing the AST on reparse | **new** |
| #599 | high | Checked division/remainder (/!, %!) do not check the INT_MIN/-1 overflow case the design requires | **new** |
| #601 | high | `.unwrap()` inside a vow clause is not flagged as impure in either compiler (Panic effect ignored in purity… | **new** |
| #602 | high | Loop-invariant vow clauses are never purity-checked (while / for-each / loop) | **new** |
| #610 | high | counterexample.schema.json mandates `inputs` but both compilers emit `values`; schema forbids the actual CE… | **new** |
| #611 | high | diagnostic.schema.json `error_code` enum is missing 4 codes that the compiler actually emits (BTreeMapKeyTy… | **new** |
| #612 | high | diagnostic.schema.json forbids fields the compiler intentionally emits (`blame`, `hints`, `secondary`), via… | **new** |
| #613 | high | Self-hosted counterexample emits `source` as a bare string (file path); Rust compiler and schema emit `sour… | **new** |
| #614 | high | Linear single-consumption is defeated by storing a linear struct in Vec<T> or HashMap<_,V>: obligation sile… | **new** |
| #591 | medium | Nested control-flow blocks render at absolute indentation level 0 (broken canonical indentation) | **new** |
| #620 | medium | Canonical printer emits refinement type with single `\|` but parser requires `\|\|`, breaking the documente… | **new** |
| #626 | medium | Self-hosted parser accepts `;`, `,`, or nothing between vow clauses; Rust parser accepts only `,`/nothing —… | **new** |
| #628 | medium | Round-trip proptest generator omits Borrow/Question/Cast and nested-unary nodes, so printer parenthesizatio… | **new** |
| #635 | medium | Both compilers make `io` subsume `read`/`write`, contradicting the documented effect-independence rule | **new** |
| #636 | medium | Self-hosted effect propagation skips `break <expr>` values, missing effectful calls inside break | **new** |
| #648 | medium | cli.md agent decision tree tells agents to read `inputs` for violating values, but the field is named `valu… | **new** |
| #649 | medium | Inconsistent `blame` string casing across output surfaces: build/verify diagnostics+counterexamples use low… | **new** |
| #650 | medium | Rust linear.rs rejects sound loop-local create-and-consume (in_loop false positive), diverging from the sel… | **new** |
| #651 | medium | grammar.md classifies / and % as plain "wrapping" with no trap; design §5.7 says they trap on zero divisor | **new** |
| #627 | low | Integer-suffix handling: non-u64 suffixes silently dropped; `100u64` rewritten to `100 as u64` (two idioms … | **new** |
| #662 | low | Refinement-type surface form is accepted by the Rust bootstrap compiler but unparsable by the self-hosted c… | **new** |
| #682 | low | errors.md error catalog omits `IoError`, leaving a documented-vs-emittable gap in the agent-facing error re… | **new** |
| #684 | low | consume_var does not transition MaybeConsumed state on use, producing duplicate diagnostics for a single re… | **new** |

### Lane 2 — Verification Pipeline  _(23 issues)_

| Issue | Severity | Title | Origin |
|---|---|---|---|
| #583 | critical | Refinement-type predicate is silently dropped during type resolution, producing false "Verified" results (u… | **new** |
| #584 | critical | Self-hosted ESBMC harness pins every u64 parameter to the constant 0 — u64 contracts are verified only at i… _(+2 merged)_ | **new** |
| #585 | critical | Checked-arithmetic overflow abort (`+!`,`-!`,`*!`,`/!`,`%!`) is never modeled in the ESBMC verification mod… _(+1 merged)_ | **new** |
| #337 | high | Top-level build status flattens overflow-blind `ProvenIr` (Z3+IR fallback) into `Verified`, so an agent see… | pre-existing |
| #572 | high | is_modelable gate accepts functions that the C emitter then models with emit_unmodelled (silent nondet, no … _(+1 merged)_ | pre-existing |
| #590 | high | `.unwrap()` panic-on-None/Err obligation is never modeled in verification — lowered to ConstUnit, so unwrap… | **new** |
| #608 | high | Callee preconditions are emitted as __ESBMC_assume inside the callee body, so a caller violating a callee's… | **new** |
| #609 | high | Callee `ensures`/`invariant` failures during a caller's verification are attributed to the WRONG contract b… | **new** |
| #436 | medium | VowViolation JSON assembled without escaping; non-finite float captures (NaN/inf) emit invalid JSON | pre-existing |
| #439 | medium | U64 vow-binding captures emit wrong tag (TAG_I32) and zero payload in VowViolation values — runtime value d… | pre-existing |
| #546 | medium | ESBMC `--memlimit 4096m` is hardcoded well above the documented 2 GB CI/run ulimit, so the cgroup SIGKILLs … | pre-existing |
| #621 | medium | Self-hosted verifier skips every function using String::parse_i64()/parse_u64() that the Rust verifier mode… | **new** |
| #622 | medium | Effect checker only flags `.unwrap()` for `[panic]`; out-of-bounds indexing and checked-arith abort sites a… _(+1 merged)_ | **new** |
| #624 | medium | Verification strategy is plain --incremental-bmc, but spec/CLI/help all claim "k-induction-parallel (increm… | **new** |
| #646 | medium | Static verification can only ever produce Callee-blame counterexamples; the entire Caller-blame reporting p… | **new** |
| #647 | medium | Non-`vow:` model assertions (vec/string/hashmap/btreemap capacity & bounds) surface with vow_id defaulting … | **new** |
| #656 | medium | where-clause (parameter refinement) predicate is never type-checked; non-bool predicates pass silently and … | **new** |
| #663 | low | detect_const_fns classifies u64-returning constant functions as const-fns; the Rust detector does not (inli… | **new** |
| #664 | low | for-each loop invariant is lowered at the header before the element binding is in scope — invariants over t… | **new** |
| #668 | low | Self-hosted `--max-k-step` value is forwarded verbatim to ESBMC with no numeric validation | **new** |
| #677 | low | `__vow_string_eq` is modeled by length-equality plus an unconstrained nondet bool, ignoring statically-know… | **new** |
| #687 | low | Self-hosted compiler drops file/offset from VowViolation; debug binaries it produces report empty source lo… | **new** |
| #690 | low | Self-hosted parse_type cannot parse the refinement-type syntax, diverging from the Rust compiler (one accep… | **new** |

### Lane 3 — Implementation Quality  _(73 issues)_

| Issue | Severity | Title | Origin |
|---|---|---|---|
| #413 | critical | Verifier-thread panic is silently reported as Unverified (exit 0 + linked binary) on the default verify path _(+1 merged)_ | pre-existing |
| #587 | critical | Method-call arguments are never type- or arity-checked (type confusion into flat slots) | **new** |
| #368 | high | AMBIGUOUS-region allocation escaping via a direct FieldSet/Store/extern-push into a parameter container is … | pre-existing |
| #435 | high | Vec reserve capacity arithmetic overflows: silent under-reserve (OOB write) and non-terminating doubling be… _(+1 merged)_ | pre-existing |
| #436 | high | VowViolation JSON assembled without escaping desc/file/binding-names; non-finite floats emit invalid JSON _(+1 merged)_ | pre-existing |
| #479 | high | `vow contracts --verify` exits 0 even when ESBMC reports failed/timeout/unknown/error contracts (both drivers) _(+2 merged)_ | pre-existing |
| #490 | high | Self-hosted is_coercible permits any i64-typed value to coerce to any integer width on let-binding; Rust re… _(+4 merged)_ | pre-existing |
| #572 | high | Modelable callee taking/returning a collection (Vec/String/Map) emits struct-to-int64_t argument passing =>… | pre-existing |
| #575 | high | Self-hosted parser discards struct-style enum variant payloads (Variant { field: Type }), diverging from in… _(+1 merged)_ | pre-existing |
| #586 | high | Self-hosted compiler has NO vow-predicate purity check — effectful calls inside requires/ensures/invariant … | **new** |
| #591 | high | Canonical printer ignores nesting level for all block-bodied expressions, producing malformed indentation | **new** |
| #595 | high | Self-hosted driver advertises `vow decl` in --help/skill JSON but does not implement it; `vowc decl file.vo… | **new** |
| #596 | high | Legacy bare form `vow <file.vow>` does not verify (and does not emit a binary) by default, contradicting th… | **new** |
| #597 | high | Self-hosted checker omits operand-type match check for arithmetic operators (+ - * / % and checked variants… | **new** |
| #598 | high | Vec::get / HashMap::get return-type divergence between Rust and self-hosted checkers | **new** |
| #600 | high | Float arithmetic is non-functional: float binops lower to integer opcodes; backend float arms are dead code | **new** |
| #603 | high | Exhaustiveness checker treats a bare-identifier binding pattern as a unit-variant match, masking catch-all … | **new** |
| #604 | high | Parser misparses `if cond {}` / `while cond {}` / `match x {}` — empty block after bare identifier is swall… | **new** |
| #605 | high | Integer literals exceeding the i64 range are silently truncated with no diagnostic | **new** |
| #606 | high | ConstF64 emission produces invalid C for inf/-inf/NaN and out-of-range float literals | **new** |
| #607 | high | Self-hosted EXPR_IF does not check condition is bool nor that then/else branch types are compatible | **new** |
| #618 | high | Self-hosted lowerer hardcodes ITY_I64 for every mutation/loop/match Phi and Upsilon type; Rust reference us… | **new** |
| #407 | medium | Internal-Call and extern-store-into-parameter heap results over-approximated to Root (memory leak / transie… | pre-existing |
| #430 | medium | Returned allocation alignment never asserted in the ESBMC arena verify harness | pre-existing |
| #439 | medium | U64 vow-violation captures are emitted with the i32 tag and a zero payload (wrong counterexample value) _(+1 merged)_ | pre-existing |
| #491 | medium | Stage 0 (Rust) check_block ignores Never propagated by `return EXPR;` while self-hosted propagates CTY_NEVE… _(+2 merged)_ | pre-existing |
| #580 | medium | Self-hosted driver never rejects unknown flags or invalid flag values; bad `--mode`/`--debug-trace`/`--enco… _(+1 merged)_ | pre-existing |
| #599 | medium | Checked division `/!` on `i64::MIN / -1` raw-traps (SIGFPE/SIGILL) instead of structured `ArithmeticOverflow` | **new** |
| #619 | medium | collect_extern_syms incurs O(instructions^2) per function on every native build via clif_find_inst linear s… | **new** |
| #621 | medium | Self-hosted C-emitter omits __vow_string_parse_i64_opt/parse_u64_opt from is_known_builtin, silently skippi… | **new** |
| #623 | medium | String-literal printer emits raw control bytes into canonical source (asymmetric/incomplete escape set) | **new** |
| #627 | medium | Suffixed integer literals other than u64 are accepted but their width/sign is discarded (e.g. 256u8, -1 via… | **new** |
| #630 | medium | IR validator (validate/validate_function) is dead code — never invoked anywhere in the build/verify/codegen… | **new** |
| #631 | medium | Validator UndefinedInstRef check is per-block only — would reject all valid cross-block SSA references (Ups… | **new** |
| #632 | medium | collect_vars_in_expr drops free variables inside MethodCall / Index / Cast / Match predicate sub-expression… | **new** |
| #633 | medium | Self-hosted checker silently accepts unknown/non-struct field access (returns CTY_NEVER) where Rust emits T… _(+1 merged)_ | **new** |
| #634 | medium | Checked remainder `%!` silently returns 0 on `i64::MIN % -1` instead of aborting with `ArithmeticOverflow` | **new** |
| #635 | medium | Effect checker makes `[io]` subsume `[read]` and `[write]`, contradicting design + grammar + agent-facing -… | **new** |
| #637 | medium | `Ty::I32` is overloaded as both the integer-literal sentinel and the real `i32` type, letting genuine i32 v… | **new** |
| #638 | medium | HashMap `.get()` is hardcoded to `Ty::I64` regardless of the value type V, diverging from the `.get -> V` spec | **new** |
| #639 | medium | Call and MethodCall AST spans overshoot past the closing `)` into the following token | **new** |
| #640 | medium | Self-hosted parser cannot parse the prefix borrow operator `&expr` | **new** |
| #641 | medium | Self-hosted parser drops string-literal patterns in match arms _(+1 merged)_ | **new** |
| #642 | medium | Self-hosted lexer silently accepts unterminated string literals and unknown characters (no diagnostics) | **new** |
| #643 | medium | Self-hosted lexer never produces float literals; float syntax silently mis-parses | **new** |
| #644 | medium | Self-hosted EXPR_ASSIGN performs no lhs/rhs type-compatibility check | **new** |
| #645 | medium | Self-hosted checker never reports undefined variables; env_lookup_var silently returns Unit | **new** |
| #655 | medium | collect_assigned_in_expr (self-hosted) omits BinaryOp/UnaryOp/Call/Method/Return and Assign-RHS / if-while … | **new** |
| #657 | medium | Linear tracker does not re-arm a linear local on assignment: reassign-after-consume yields a spurious "alre… | **new** |
| #658 | medium | Self-hosted dominator-tree construction uses an incremental-LCA fixpoint instead of region.rs's dominator-s… | **new** |
| #659 | medium | vow predicate purity check silently ignores `.unwrap()` (panic) — collected into `panic_exprs` but never in… | **new** |
| #177 | low | Embedded skill/help payload dominates main.vow (~80% of the file, ~7,300 push_str calls), inflating compile… | pre-existing |
| #367 | low | inst_region_for_value performs unmemoized recursion with per-source `seen.clone()`, giving super-linear cos… | pre-existing |
| #569 | low | Self-hosted check_module lacks the item_files/items length-guard the Rust mirror asserts | pre-existing |
| #660 | low | Receiver-region GetArg test in clif.vow diverges from the shim (requires op==GET_ARG; shim keys only on dk=… | **new** |
| #661 | low | Self-hosted verify_collect ignores ESBMC process exit code (relies solely on stdout text), unlike a defense… | **new** |
| #667 | low | const-fn detection diverges: self-hosted detect_const_fns inlines U64 constant functions, Rust detect_const… | **new** |
| #669 | low | `vow test --mode profile\|sanitize\|<invalid>` silently degrades to debug instead of erroring (diverges fro… | **new** |
| #670 | low | Validator linear-consume check is control-flow-insensitive — flags branch-exclusive single consumes as cons… | **new** |
| #671 | low | Self-hosted method-call argument types are computed but never checked (arity/type) — broader silent-accept … | **new** |
| #672 | low | debug_escape_str diverges from Rust {:?} on control and non-ASCII bytes (raw passthrough vs \u{..} escapes) | **new** |
| #673 | low | U64 constant printed via signed i64_to_str; values above i64::MAX would render negative (diverges from Rust… | **new** |
| #674 | low | Cast (`as`) binds tighter than prefix unary `-`/`!`, diverging from the documented "usual C/Rust precedence" | **new** |
| #675 | low | Lexer error for non-ASCII leading byte reports a 1-byte span mid-UTF-8 and mojibake character | **new** |
| #676 | low | Preamble declares 5 __VERIFIER_nondet_* externs but emitter/harness can emit __VERIFIER_nondet_unsigned_lon… | **new** |
| #678 | low | parse_for_expr consumes the loop variable without checking it is an identifier | **new** |
| #679 | low | try_suffix passes byte_at's out-of-bounds sentinel (-1) into is_ident_cont, violating its requires:b>=0 pre… | **new** |
| #680 | low | Self-hosted `?` operator (EXPR_QUESTION) does not require Option/Result receiver | **new** |
| #681 | low | Linear-region check emits hard Error regardless of region kind; spec mandates a warning for Root-regioned u… | **new** |
| #683 | low | Per-test `skipped` status is unreachable: schema/summary advertise it but no code path emits it | **new** |
| #691 | low | Dead duplicate concrete-block region LUB cluster (~290 lines) in self-hosted region.vow — never wired in, d… | **new** |
| #692 | low | Self-hosted region-marker ICE diagnostic omits the 'file an issue' guidance hint present in the Rust intern… | **new** |
| #693 | low | insert_region_markers_module lacks the whole-function 'no block regions' early-out, doing dominator/back-ed… | **new** |

### Peripheral — Benchmarks, Mutation Testing, Docs/Spec, Agent Surface  _(19 issues)_

| Issue | Severity | Title | Origin |
|---|---|---|---|
| #485 | high | Benchmark harness reports a program VERIFIED with no check that the submitted contracts match the skeleton;… | pre-existing |
| #592 | high | Machine-readable --help reports the wrong default output path for `-o, --output` (says "source without .vow… | **new** |
| #611 | high | diagnostic.schema.json error_code enum omits 4 real ErrorCode variants (BTreeMapKeyTypeMustBeI64, BTreeMapV… | **new** |
| #615 | high | Counterexample JSON emits key `values` but every spec copy (schema + cli.md + contracts.md + embedded skill… | **new** |
| #616 | high | counterexample.schema.json (and embedded copy) omits emitted fields `blame`, `call_sites`, `violating_args`… | **new** |
| #617 | high | Mutation oracle has no baseline (unmutated-tree) check; a globally-broken Tier-1 oracle silently scores eve… | **new** |
| #419 | medium | `skill` command present in --help `commands` but absent from `command_details`; the skill subcommand gramma… | pre-existing |
| #582 | medium | SKILL.md live `--help` command fails the skill load (non-zero exit) when neither `vow` nor `build/vowc` is … | pre-existing |
| #625 | medium | No staleness detector cross-references cli.md (CLI flags/defaults/commands) against --help JSON; --check on… | **new** |
| #629 | medium | OpenAI reasoning models configured in the suite (o3) will crash the harness: llm.py sends max_tokens and te… | **new** |
| #652 | medium | diagnostic.schema.json forbids additionalProperties but the compiler emits `secondary` and `blame` on diagn… | **new** |
| #653 | medium | Tier-2 budget starvation misclassifies real coverage gaps as TIMEOUT instead of UNRUN | **new** |
| #654 | medium | A `cd <workdir>` failure inside the oracle command is scored as `caught` without running any test | **new** |
| #665 | low | benchmarks/README.md is stale: claims 40 benchmarks (E01–E15/M01–M15/H01–H10) while the manifest defines 10… | **new** |
| #666 | low | Report compares Vow's combined verification_rate (E/M/H + 67 HumanEval, with weak/bounded contracts) direct… | **new** |
| #685 | low | `vow test --mode` accepts profile/sanitize at parse time, but cli.md and --help document only debug\|release | **new** |
| #686 | low | Rust `vow mutants` exits with code 2, an exit code cli.md's Exit Codes table does not list | **new** |
| #688 | low | compute_skip_ranges scans GENERATE markers without string/comment awareness; an unterminated GENERATE:START… | **new** |
| #689 | low | enumerate_root does not recurse into subdirectories; sites under a nested --root are silently dropped | **new** |