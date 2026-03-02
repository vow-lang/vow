# Phase 9 Wave 4 — Port IR, lowering, and codegen wrapper to Vow

## Overview

Port the Vow IR data structures, the AST→IR lowering pass, and an IR printer to Vow.
Add a `--dump-ir` flag to the Rust CLI as oracle. Compare IR text output from both
pipelines on a test corpus.

**Asana parent task:** GID 1213457543393580
**Reference Rust source:** `vow-ir/src/types.rs` (500 lines), `vow-ir/src/lower/mod.rs`
(2099 lines), `vow-ir/src/lower/vow.rs` (331 lines), `vow-ir/src/printer.rs` (230 lines)

---

## Prerequisite: self-hosted parser must handle vow blocks

`parse_vow_block` in `compiler/parser.vow` (line 300) currently consumes vow blocks
by tracking brace depth without storing AST data. The `fn_data` stride-6 format has
no field for a vow block reference. `EXPR_WHILE` has no vow data either.

**Decision:** Extend the self-hosted parser to store vow blocks before implementing
the lowering. This enables testing on the full example suite including `divide.vow`
and `bisect.vow`.

---

## Task 4.0: Extend self-hosted parser with vow block support

**Asana:** GID 1213487379207175
**Depends on:** nothing
**Files:** `compiler/ast.vow`, `compiler/parser.vow`

### AST representation for vow clauses

A vow block is a list of clauses. Each clause has a kind (requires/ensures/invariant),
a predicate expression, and a span. Add new arena storage:

- [ ] Add `vow_data: Vec<i64>` to `AstArena` (stride 3: kind, expr_id, span)
- [ ] Add clause kind constants: `fn CLAUSE_REQUIRES() -> i64 { 0 }`, `CLAUSE_ENSURES() -> i64 { 1 }`, `CLAUSE_INVARIANT() -> i64 { 2 }`
- [ ] Add `fn arena_add_vow_clause(a: AstArena, kind: i64, eid: i64, span: i64) -> i64`
- [ ] Add accessors: `fn vow_clause_kind(a: AstArena, cid: i64) -> i64`, `fn vow_clause_expr(a: AstArena, cid: i64) -> i64`

### Attach vow blocks to functions

The `fn_data` currently uses stride 6. Extend to stride 7 to add a vow block field:

- [ ] Change `fn_data` stride from 6 to 7
- [ ] Field [5] = `vow_lid` (list ID of clause IDs, or -1 if no vow block)
- [ ] Field [6] = span (was previously [5])
- [ ] Add accessor: `fn fn_vow_lid(a: AstArena, fid: i64) -> i64 { a.fn_data[fid * 7 + 5] }`
- [ ] Update all existing fn_data accessors to use stride 7
- [ ] Update `arena_add_fn` to take `vow_lid` parameter

### Attach vow blocks to while expressions

EXPR_WHILE currently uses: a=cond_eid, b=body_bid, c=0. Use c for vow_lid:

- [ ] EXPR_WHILE: c = vow_lid (list of clause IDs, or -1 if no vow block)

### Parse vow clauses

Rewrite `parse_vow_block` to actually store data:

- [ ] `fn parse_vow_block(p: Parser) -> i64` — returns list ID of clause IDs, or -1
  - If not `at(p, tok_kw_vow())`: return -1
  - Consume `vow`, `{`
  - Loop: parse each clause:
    - `requires:` / `ensures:` / `invariant:` keyword → determine kind
    - Parse the predicate expression with `parse_expr`
    - Consume `;` or `}`
    - `arena_add_vow_clause(kind, expr_id, span)`
  - Return list of clause IDs
- [ ] New: `fn tok_kw_requires() -> i64`, `fn tok_kw_ensures() -> i64`, `fn tok_kw_invariant() -> i64`
  - Check if lexer already has these keywords. If not, they might be parsed as identifiers.
  - **Approach:** Inside vow block, check `at_ident(p, "requires")` etc. — no new token kinds needed.
- [ ] Update `parse_fn_def` to pass vow_lid to `arena_add_fn`
- [ ] Update `parse_while` to pass vow_lid as expr_c

### Add EXPR_RESULT node

The Rust AST has `ExprKind::Result` for the `result` keyword in ensures clauses.

- [ ] Add `fn EXPR_RESULT() -> i64 { 22 }` to ast.vow
- [ ] In vow block parsing: if ident is "result", emit EXPR_RESULT node

### Update checker

The checker currently skips vow blocks. Update it to type-check vow predicates:

- [ ] Or: leave checker unchanged for now (it doesn't need vow data to self-check)
- [ ] The lowering pass can operate on the raw AST without checker validation

### Compile and verify

- [ ] All existing self-checks still pass (0 errors on all compiler/*.vow files)
- [ ] `./compiler/main examples/divide.vow` — parses without error (vow block stored)
- [ ] `./compiler/main examples/bisect.vow` — parses without error

---

## Task 4.1: Port IR data types to Vow (`compiler/ir.vow`)

**Asana:** GID 1213487093406418
**Depends on:** nothing
**Reference:** `vow-ir/src/types.rs`

### Opcode constants

One `fn IOP_*() -> i64 { N }` per opcode, matching the order in `types.rs`:

- [ ] `IOP_CONST_I32()=0, IOP_CONST_I64()=1, IOP_CONST_F32()=2, IOP_CONST_F64()=3`
- [ ] `IOP_CONST_BOOL()=4, IOP_CONST_STR()=5, IOP_CONST_UNIT()=6`
- [ ] `IOP_GET_ARG()=7`
- [ ] `IOP_WADD_I32()=8 .. IOP_GE_I32()=19` (12 i32 ops — wrapping arith + checked arith + cmp)
- [ ] `IOP_WADD_I64()=20 .. IOP_GE_I64()=31` (12 i64 ops)
- [ ] `IOP_ADD_F32()=32 .. IOP_GE_F32()=42` (11 f32 ops)
- [ ] `IOP_ADD_F64()=43 .. IOP_GE_F64()=53` (11 f64 ops)
- [ ] `IOP_NOT()=54, IOP_AND()=55, IOP_OR()=56`
- [ ] `IOP_LOAD()=57, IOP_STORE()=58`
- [ ] `IOP_BRANCH()=59, IOP_JUMP()=60, IOP_RETURN()=61, IOP_UNREACHABLE()=62`
- [ ] `IOP_PHI()=63, IOP_UPSILON()=64`
- [ ] `IOP_VOW_REQ()=65, IOP_VOW_ENS()=66, IOP_VOW_INV()=67`
- [ ] `IOP_CALL()=68`
- [ ] `IOP_REGION_ALLOC()=69, IOP_REGION_FREE()=70`
- [ ] `IOP_LINEAR_CONSUME()=71, IOP_LINEAR_BORROW()=72`
- [ ] `IOP_FIELD_GET()=73, IOP_FIELD_SET()=74`
- [ ] Helper `fn iop_is_terminal(op: i64) -> bool` (checks Branch/Jump/Return/Unreachable)

### IR type constants

- [ ] `ITY_I32()=0, ITY_I64()=1, ITY_F32()=2, ITY_F64()=3`
- [ ] `ITY_BOOL()=4, ITY_UNIT()=5, ITY_PTR()=6, ITY_LPTR()=7`

### InstData encoding

Use a flat struct to avoid nested struct issues:

- [ ] `IDATA_NONE()=0, IDATA_CONST_I32()=1, IDATA_CONST_I64()=2, IDATA_CONST_F32()=3`
- [ ] `IDATA_CONST_F64()=4, IDATA_CONST_BOOL()=5, IDATA_ARG_IDX()=6, IDATA_PHI_TGT()=7`
- [ ] `IDATA_CONST_STR()=8, IDATA_CALL_TGT()=9, IDATA_CALL_EXT()=10, IDATA_BRANCH()=11`
- [ ] `IDATA_JUMP()=12, IDATA_REGION()=13, IDATA_VOW_ID()=14, IDATA_ALLOC()=15, IDATA_FIELD()=16`

### Structs

```
struct IrInst {
    id: i64,
    op: i64,          // IOP_* constant
    ty: i64,          // ITY_* constant
    args: Vec<i64>,   // Vec of InstIds
    dk: i64,          // IDATA_* constant (data kind)
    dv: i64,          // primary data value (ConstI64 value, ArgIndex, PhiTarget, etc.)
    dv2: i64,         // secondary data value (BranchTargets else_block, AllocSize align)
    ds: String,       // data string (CallExtern symbol name)
    ostart: i64,      // origin span start
    olen: i64,        // origin span length
}
```

- [ ] `struct IrInst` — as above
- [ ] Constructor: `fn ir_inst_new(id: i64, op: i64, ty: i64, args: Vec<i64>, dk: i64, dv: i64, dv2: i64, ds: String, ostart: i64, olen: i64) -> IrInst`
- [ ] Or simpler: `fn ir_inst(id: i64, op: i64, ty: i64, args: Vec<i64>) -> IrInst` with defaults dk=IDATA_NONE, dv/dv2=0, ds=""

```
struct IrBlock {
    id: i64,
    insts: Vec<IrInst>,
}
```

- [ ] `struct IrBlock`, `fn ir_block_new(id: i64) -> IrBlock`

```
struct IrVowEntry {
    id: i64,
    description: String,
    blame: i64,         // 0=Caller, 1=Callee
    binding_ids: Vec<i64>,
    binding_names: Vec<String>,
}
```

- [ ] `struct IrVowEntry`

```
struct IrFunction {
    id: i64,
    name: String,
    params: Vec<i64>,    // Vec of ITY_* constants
    return_ty: i64,      // ITY_* constant
    effects: i64,        // bitmask (same as ast.vow EFF_* constants)
    vows: Vec<IrVowEntry>,
    blocks: Vec<IrBlock>,
}
```

- [ ] `struct IrFunction`

```
struct IrFieldLayout {
    name: String,
    ty: i64,
}
struct IrStructLayout {
    name: String,
    fields: Vec<IrFieldLayout>,
    is_linear: i64,
}
struct IrVariantLayout {
    name: String,
    tag: i64,
    payload: Vec<IrFieldLayout>,
}
struct IrEnumLayout {
    name: String,
    variants: Vec<IrVariantLayout>,
}
```

- [ ] `struct IrFieldLayout`, `struct IrStructLayout`
- [ ] `struct IrVariantLayout`, `struct IrEnumLayout`
- [ ] `struct IrModule { name: String, functions: Vec<IrFunction>, strings: Vec<String>, struct_layouts: Vec<IrStructLayout>, enum_layouts: Vec<IrEnumLayout> }`

### Helpers on IrStructLayout / IrEnumLayout

- [ ] `fn struct_field_index(sl: IrStructLayout, fname: String) -> i64` — returns field index or -1
- [ ] `fn struct_size_bytes(sl: IrStructLayout) -> i64` — `fields.len() * 8`
- [ ] `fn enum_variant_tag(el: IrEnumLayout, vname: String) -> i64` — returns tag or -1
- [ ] `fn enum_size_bytes(el: IrEnumLayout) -> i64` — `(1 + max_payload) * 8`

### Compile and verify

- [ ] `./target/release/vow --no-verify compiler/ir.vow` compiles
- [ ] `./compiler/main compiler/ir.vow` — self-hosted lexer/parser/checker reports 0 errors

---

## Task 4.2: Port IR printer to Vow (`compiler/ir_printer.vow`)

**Asana:** GID 1213487083574436
**Depends on:** Task 4.1
**Reference:** `vow-ir/src/printer.rs`

### Functions

- [ ] `fn opcode_name(op: i64) -> String` — map every IOP_* to its string name ("ConstI32", "WrappingAddI64", etc.)
- [ ] `fn ir_ty_str(ty: i64) -> String` — "i32", "i64", "f32", "f64", "Bool", "Void", "ptr", "linear_ptr"
- [ ] `fn effect_str(e: i64) -> String` — given a single effect bit, returns "IO"/"Panic"/"Read"/"Unsafe"/"Write"
- [ ] `fn effects_str(eff: i64) -> String` — expand bitmask to comma-separated string
- [ ] `fn format_data(inst: IrInst) -> String` — format the data payload:
  - IDATA_NONE → "" (empty — caller checks for empty and omits `[...]`)
  - IDATA_CONST_I32/I64 → i64_to_str(dv)
  - IDATA_CONST_F32/F64 → f64 formatting (edge case, not needed for test corpus)
  - IDATA_CONST_BOOL → "true"/"false"
  - IDATA_CONST_STR → "@" + i64_to_str(dv)
  - IDATA_ARG_IDX → i64_to_str(dv)
  - IDATA_PHI_TGT → "%" + i64_to_str(dv)
  - IDATA_CALL_TGT → "func" + i64_to_str(dv)
  - IDATA_CALL_EXT → "extern:" + ds
  - IDATA_BRANCH → "block_" + i64_to_str(dv) + ", block_" + i64_to_str(dv2)
  - IDATA_JUMP → "block_" + i64_to_str(dv)
  - IDATA_REGION → "region_" + i64_to_str(dv)
  - IDATA_VOW_ID → "vow_" + i64_to_str(dv)
  - IDATA_ALLOC → "size=" + i64_to_str(dv) + ",align=" + i64_to_str(dv2)
  - IDATA_FIELD → "field_" + i64_to_str(dv)

- [ ] `fn print_inst(inst: IrInst) -> String`
  - Format: `"    "  + pad(ir_ty_str(inst.ty), 10) + "  %" + i64_to_str(inst.id) + " = " + name + "(" + args_str + ")"`
  - Where `name` is `opcode_name(inst.op)` + `"[" + format_data(inst) + "]"` if data is non-empty, else just `opcode_name(inst.op)`
  - `args_str` is comma-separated `"%N"` for each arg
  - **Critical:** the Rust format string is `"{:<10}  %{} = {}({})"` — 4 leading spaces, ty left-padded to 10 chars, two spaces, percent-id, equals, name, parens

- [ ] `fn pad_right(s: String, width: i64) -> String` — pad with spaces to `width` chars

- [ ] `fn print_block(blk: IrBlock, is_entry: i64) -> String`
  - Entry block: `"  entry (block " + i64_to_str(blk.id) + "):"`
  - Non-entry: `"  block_" + i64_to_str(blk.id) + ":"`
  - Then each inst on its own line

- [ ] `fn print_function(f: IrFunction) -> String`
  - Header: `"fn " + name + "(" + params_str + ") -> " + ret_ty + " [" + effects + "]:"`
  - params_str: comma-separated ir_ty_str for each param
  - Then each block

- [ ] `fn print_module(m: IrModule) -> String`
  - Strings section: `"strings:\n"` then `"  @0 = \"...\"\n  @1 = \"...\""` etc.
  - Struct layouts: `"struct Name { field: ty, ... }"`
  - Enum layouts: `"enum Name { Variant(tag=N), Variant(tag=N, ty, ...) }"`
  - Functions (separated by blank lines)
  - **Critical:** String escaping in the strings section — Rust uses `{:?}` which produces
    `\"` for double-quotes, `\\n` for newlines, etc. The Vow printer must match this format.

### String escaping for string pool

The Rust printer uses `format!("  @{i} = {:?}", s)` which calls Rust's Debug trait on String.
This produces escape sequences like `\"`, `\\`, `\n`, `\t`.

- [ ] `fn debug_escape_str(s: String) -> String` — produce Rust `Debug`-style escaping:
  - `"` → `\"`
  - `\` → `\\`
  - newline (byte 10) → `\n`
  - tab (byte 9) → `\t`
  - carriage return (byte 13) → `\r`
  - null (byte 0) → `\0`
  - Other bytes pass through

### Compile and verify

- [ ] `./target/release/vow --no-verify compiler/ir_printer.vow` compiles
- [ ] `./compiler/main compiler/ir_printer.vow` — 0 errors

---

## Task 4.3: Add `--dump-ir` flag to Rust `vow` CLI

**Asana:** GID 1213433030360896
**Depends on:** nothing (Rust-only change)
**Reference:** `vow/src/main.rs`

### Changes

- [ ] Add `#[arg(long)] dump_ir: bool` to `Args` struct (line 26)
- [ ] In `main()`, after `args.help` handling, when `args.dump_ir`:
  - call `run_pipeline_dump_ir(source, no_verify)` (new function)
  - or inline: parse → load modules → type check → lower → print → exit
- [ ] New function `run_pipeline_dump_ir(source: &Path, no_verify: bool)`:
  1. Read source file
  2. Parse: `vow_syntax::parser::parse_module(&src)`
  3. Load modules: `module_loader::load_modules` + `merge_modules`
  4. Type check: `Checker::new().check_module(&ast)`
  5. Lower: `let ir = vow_ir::lower_module(&ast);`
  6. Print: `println!("{}", vow_ir::print_module(&ir));`
  7. Exit with code 0
- [ ] Update `--help` JSON and human-readable text to document `--dump-ir`

### Test

- [ ] `cargo build --all --release`
- [ ] `./target/release/vow --dump-ir --no-verify examples/hello.vow` prints IR text
- [ ] `./target/release/vow --dump-ir --no-verify examples/countdown.vow` prints IR text
- [ ] `cargo test --all` passes (no regressions)

---

## Task 4.4: Port AST→IR lowering to Vow (`compiler/lower.vow`)

**Asana:** GID 1213487098322113
**Depends on:** Task 4.1
**Reference:** `vow-ir/src/lower/mod.rs` (2099 lines) + `lower/vow.rs` (331 lines)

This is the largest and most complex task. The Vow lowering operates on the self-hosted
AST (arena-based, accessed via `expr_tag()`, `expr_a()` etc.) and produces IR data
structures from Task 4.1.

### LowerCtx struct

```
struct LowerCtx {
    func: IrFunction,
    current_block: i64,        // BlockId
    next_inst_id: i64,
    scope_vars: Vec<String>,   // flat: [name0, name1, ...] per scope frame
    scope_vals: Vec<i64>,      // parallel: [instid0, instid1, ...]
    scope_starts: Vec<i64>,    // frame boundaries: [0, n1, n1+n2, ...]
    string_pool: Vec<String>,
    func_names: Vec<String>,   // function name → index is FuncId
    func_ret_tys: Vec<i64>,   // parallel: return ITY_* for each function
    struct_names: Vec<String>,
    struct_field_lists: Vec<Vec<String>>,   // parallel: field names per struct
    enum_names: Vec<String>,
    enum_variant_lists: Vec<Vec<String>>,   // parallel: variant names per enum
    tag_inst_ids: Vec<i64>,    // inst_struct_type keys
    tag_names: Vec<String>,    // inst_struct_type values (parallel)
    arena: AstArena,           // reference to the parsed AST
}
```

- [ ] `struct LowerCtx`
- [ ] `fn lower_ctx_new(name: String, params: Vec<i64>, return_ty: i64, effects: i64, ...) -> LowerCtx`

### Scope management (append-only with truncation-based snapshot/restore)

The Rust code uses `Vec<HashMap<String,InstId>>` with deep-cloning for snapshot/restore.
In Vow, struct field assignment copies the pointer, not the data — so we cannot simply
save `ctx.scope_vars` and restore it later (the saved pointer and the live pointer are
the same Vec).

**Solution: append-only + truncation.** Instead of cloning the Vecs, we save the
*lengths* before entering a branch. After the branch, we truncate back to those lengths.
This works because:
- `push_scope` appends a marker to `scope_starts`
- `define` appends to `scope_vars` / `scope_vals`
- `pop_scope` truncates back to the last marker
- Snapshot = save 3 integers (lengths of vars, vals, starts)
- Restore = truncate each Vec back to the saved length

This avoids deep copying entirely. The only requirement is that we never mutate
elements in-place between snapshot and restore — but `assign` does mutate vals
in-place. So after restore, the old `scope_vals` entries are unchanged (they still
hold the pre-branch values), which is exactly what we want.

```
struct ScopeSnap {
    vars_len: i64,
    vals_len: i64,
    starts_len: i64,
}
```

- [ ] `fn lctx_push_scope(ctx: LowerCtx)` — push current scope_vars.len() to scope_starts
- [ ] `fn lctx_pop_scope(ctx: LowerCtx)` — truncate vars/vals to last marker, pop starts
- [ ] `fn lctx_define(ctx: LowerCtx, name: String, id: i64)` — append to current frame
- [ ] `fn lctx_assign(ctx: LowerCtx, name: String, id: i64)` — find existing binding, update value in-place
- [ ] `fn lctx_lookup(ctx: LowerCtx, name: String) -> i64` — search backwards, return InstId or -1
- [ ] `struct ScopeSnap` with three i64 fields (lengths, not copies)
- [ ] `fn lctx_snapshot(ctx: LowerCtx) -> ScopeSnap` — save current lengths of scope_vars, scope_vals, scope_starts
- [ ] `fn lctx_restore(ctx: LowerCtx, snap: ScopeSnap)` — truncate each Vec back to saved length
  - **Note:** `assign` modifies `scope_vals[i]` in-place. After snapshot, assignments in the
    then-branch modify elements 0..snap.vals_len (pre-branch values). Restore truncates
    anything *appended* but does NOT undo in-place mutations. This is correct because:
    - Mutations to pre-branch vars are captured by Phi nodes (collect_if_mutations)
    - The restore gives us back the pre-branch bindings for the else-branch
    - After both branches, the merge block uses Phi values, not scope values
  - **Edge case:** `assign` in then-branch mutates scope_vals[i]. Before else-branch,
    restore truncates but doesn't reset scope_vals[i]. We need to save/restore mutated
    vals explicitly for the else-branch. Use `collect_if_mutations` result: before else,
    write back the pre-branch InstId for each mutated var.

### inst_struct_type management

The Rust code uses `HashMap<InstId, String>`. In Vow, use parallel Vecs.

- [ ] `fn lctx_tag_inst(ctx: LowerCtx, inst_id: i64, type_name: String)` — record type tag
- [ ] `fn lctx_get_tag(ctx: LowerCtx, inst_id: i64) -> String` — look up, return "" if not found

### Block and instruction management

- [ ] `fn lctx_new_block(ctx: LowerCtx) -> i64` — append new IrBlock to func.blocks, return id
- [ ] `fn lctx_switch_to_block(ctx: LowerCtx, block_id: i64)` — set current_block
- [ ] `fn lctx_emit(ctx: LowerCtx, op: i64, ty: i64, args: Vec<i64>, dk: i64, dv: i64, dv2: i64, ds: String, ostart: i64, olen: i64) -> i64`
  - Allocates next_inst_id, creates IrInst, pushes to current block, returns id
  - **Convenience wrappers:**
    - [ ] `fn lctx_emit_simple(ctx: LowerCtx, op: i64, ty: i64, args: Vec<i64>, ostart: i64, olen: i64) -> i64` — dk=IDATA_NONE
    - [ ] `fn lctx_emit_const_i64(ctx: LowerCtx, val: i64, ostart: i64, olen: i64) -> i64`
    - [ ] `fn lctx_emit_call_extern(ctx: LowerCtx, sym: String, ty: i64, args: Vec<i64>, ostart: i64, olen: i64) -> i64`
- [ ] `fn lctx_is_terminated(ctx: LowerCtx) -> bool` — check if current block's last inst is terminal
- [ ] `fn lctx_intern_str(ctx: LowerCtx, s: String) -> i64` — intern into string_pool
- [ ] `fn lctx_inst_ty(ctx: LowerCtx, inst_id: i64) -> i64` — look up type of emitted inst

### AST type → IR type

- [ ] `fn lower_ast_ty(a: AstArena, tid: i64) -> i64` — maps AST type ID to ITY_* constant:
  - TY_NAMED: look up name string → "i32"→ITY_I32, "i64"→ITY_I64, "f32"→ITY_F32, "f64"→ITY_F64, "bool"→ITY_BOOL, else→ITY_PTR
  - TY_UNIT/TY_NEVER → ITY_UNIT
  - else → ITY_PTR

### Builtin function mapping

- [ ] `fn builtin_to_runtime(name: String) -> i64` — returns 1 if recognized, 0 if not
- [ ] `fn builtin_sym(name: String) -> String` — "print_str"→"__vow_string_print", "print_i64"→"__vow_print_i64", etc.
- [ ] `fn builtin_ret_ty(name: String) -> i64` — "print_str"→ITY_UNIT, "fs_read"→ITY_PTR, etc.

### Binop opcode mapping

- [ ] `fn binop_opcode(op: i64) -> i64` — BINOP_ADD→IOP_WADD_I64, BINOP_EQ→IOP_EQ_I64, etc.
- [ ] `fn binop_ty(op: i64) -> i64` — arithmetic→ITY_I64, comparison/logical→ITY_BOOL

### collect_assigned_vars helper

Recursively walks AST to find variables assigned in a block. Used for while loop
Phi/Upsilon generation.

- [ ] `fn collect_assigned_vars(ctx: LowerCtx, bid: i64) -> Vec<String>`
  - Walk all stmts in block, for each STMT_EXPR recurse into expr
  - For EXPR_ASSIGN where LHS is EXPR_IDENT: add name to result (dedup)
  - Recurse into EXPR_BLOCK, EXPR_IF (then-branch, else-branch), EXPR_WHILE (body)
  - Recurse into EXPR_BINOP, EXPR_UNOP
- [ ] `fn collect_assigned_in_expr(ctx: LowerCtx, eid: i64, seen: Vec<String>, out: Vec<String>)`

### collect_if_mutations helper

Like collect_assigned_vars but for if/else branches, filtered to names in current scope.

- [ ] `fn collect_if_mutations(ctx: LowerCtx, then_bid: i64, else_eid: i64) -> Vec<String>`
  - Returns names of vars assigned in then/else AND currently in scope
- [ ] Parallel `fn collect_if_mutation_ids(ctx: LowerCtx, names: Vec<String>) -> Vec<i64>`
  - Returns pre-branch InstId for each mutated var

### backpatch_upsilon

- [ ] `fn backpatch_upsilon(ctx: LowerCtx, block_id: i64, upsilon_id: i64, phi_id: i64)`
  - Find inst with `id == upsilon_id` in block `block_id`, update its `dv` to `phi_id`
  - **Note:** needs mutable access to IrBlock.insts — since IrInst is a heap-allocated
    struct, the Vec stores pointers, so we can read the inst and modify its dv field directly

### lower_expr — the core function (~1000 lines in Rust)

Map each expression tag to the appropriate IR instruction sequence. The self-hosted AST
uses `expr_tag(a, eid)`, `expr_a(a, eid)`, `expr_b(a, eid)`, `expr_c(a, eid)` accessors
(stride-5 in `arena.expr_data`).

**Expression semantics reference** (from parser.vow and checker.vow):

| Tag | a | b | c | Notes |
|-----|---|---|---|-------|
| EXPR_LIT_INT | value | 0 | 0 | integer literal |
| EXPR_LIT_BOOL | 0 or 1 | 0 | 0 | boolean literal |
| EXPR_LIT_STR | str_id | 0 | 0 | string literal |
| EXPR_LIT_FLOAT | ??? | 0 | 0 | float literal (TBD) |
| EXPR_IDENT | str_id | 0 | 0 | variable reference |
| EXPR_BINOP | op (BINOP_*) | lhs_eid | rhs_eid | binary operation |
| EXPR_UNOP | op (UNOP_*) | operand_eid | 0 | unary operation |
| EXPR_CALL | callee_eid | args_lid | 0 | function call |
| EXPR_METHOD | recv_eid | method_sid | args_lid | method call |
| EXPR_FIELD | base_eid | field_sid | 0 | field access |
| EXPR_INDEX | base_eid | index_eid | 0 | indexing |
| EXPR_IF | cond_eid | then_bid | else_eid (-1 if none) | if-else |
| EXPR_WHILE | cond_eid | body_bid | 0 | while loop |
| EXPR_MATCH | scrut_eid | arms_lid | 0 | match expression |
| EXPR_RETURN | value_eid (-1 if none) | 0 | 0 | return |
| EXPR_BLOCK | block_bid | 0 | 0 | nested block |
| EXPR_ASSIGN | lhs_eid | rhs_eid | 0 | assignment |
| EXPR_SLIT | name_sid | fields_lid | 0 | struct literal |
| EXPR_ECTOR | path_lid | args_lid | 0 | enum construct |
| EXPR_QUESTION | inner_eid | 0 | 0 | ? operator |

**NOTE:** Verify these semantics against parser.vow before implementing. The table above
is inferred from checker.vow usage patterns but needs confirmation from the parser.

Each case:

- [ ] **EXPR_LIT_INT** → `lctx_emit(IOP_CONST_I64, ITY_I64, [], IDATA_CONST_I64, value, 0, "")`
- [ ] **EXPR_LIT_BOOL** → `lctx_emit(IOP_CONST_BOOL, ITY_BOOL, [], IDATA_CONST_BOOL, value, 0, "")`
- [ ] **EXPR_LIT_STR** → intern string, emit ConstStr, then CallExtern `__vow_string_from_cstr`, tag as "String"
- [ ] **EXPR_LIT_FLOAT** → `lctx_emit(IOP_CONST_F64, ITY_F64, [], IDATA_CONST_F64, ?, 0, "")` (float bit-cast TBD)
- [ ] **EXPR_IDENT** → `lctx_lookup(ctx, name)` — panic if not found
- [ ] **EXPR_BINOP** →
  - Lower lhs, lower rhs
  - Check if either operand is tagged "String" and op is Eq/Ne → emit `__vow_string_eq` call (+ Not for Ne)
  - Otherwise: `binop_opcode(op)` → emit
- [ ] **EXPR_UNOP** →
  - UNOP_NOT → `IOP_NOT`
  - UNOP_NEG → emit ConstI64(0), then WrappingSubI64(zero, val)
- [ ] **EXPR_CALL** →
  - Lower all args
  - Get callee name from EXPR_IDENT
  - Check func_names for user function → CallTarget(func_id)
  - Check builtin_to_runtime → CallExtern(sym)
  - Else → CallExtern(name) with ret_ty=ITY_UNIT
- [ ] **EXPR_IF** → (complex, ~100 lines in Rust)
  - collect_if_mutations for both branches
  - Lower condition
  - Branch to then_block / else_block
  - Snapshot scope
  - Lower then-branch, capture mutation vals, emit Upsilon(placeholder) + Jump(merge)
  - Restore scope
  - Lower else-branch (or emit ConstUnit), capture mutation vals, emit Upsilon + Jump
  - Restore scope
  - Switch to merge_block
  - Emit Phi for each mutated var, backpatch Upsilons from both branches
  - Handle all 4 cases: both terminated, only then, only else, neither
- [ ] **EXPR_WHILE** → (~80 lines in Rust + vow invariant)
  - collect_assigned_vars for body
  - Emit placeholder Upsilons for loop-carried vars
  - Jump to header_block
  - Header: emit Phis, backpatch pre-header Upsilons, update scope
  - **Vow invariant:** If EXPR_WHILE c field (vow_lid) != -1, iterate clause list;
    for each CLAUSE_INVARIANT: call `lower_invariant(ctx, clause_expr)` — emits
    VowInvariant + Branch(violation_block, continue_block)
  - Lower condition, Branch to body_block / exit_block
  - Body: lower body, emit back-edge Upsilons, Jump to header
  - Switch to exit_block, emit ConstUnit
- [ ] **EXPR_BLOCK** → push_scope, lower_block_inner, pop_scope
- [ ] **EXPR_RETURN** →
  - Lower value (or emit ConstUnit if no value)
  - **Vow ensures:** If current function has vow block with CLAUSE_ENSURES entries,
    for each ensures clause: call `lower_ensures(ctx, clause_expr, return_val_id)` —
    emits VowEnsures + Branch(violation_block, continue_block). The `result` keyword
    in the predicate (EXPR_RESULT) resolves to `return_val_id`.
  - Emit Return
- [ ] **EXPR_ASSIGN** →
  - Lower rhs
  - If lhs is EXPR_IDENT → lctx_assign
  - If lhs is EXPR_FIELD → lower base, look up field index, emit FieldSet
- [ ] **EXPR_FIELD** → lower base, look up struct field index via inst_struct_type, emit FieldGet
- [ ] **EXPR_SLIT** (struct literal) →
  - RegionAlloc(size=n_fields*8, align=8)
  - Tag the alloc in inst_struct_type
  - For each field: lower value, emit FieldSet
- [ ] **EXPR_ECTOR** (enum construct) →
  - Handle String::from(lit) → just lower the arg (already creates VowVec string)
  - Handle HashMap::new() → CallExtern `__vow_map_new`, tag "HashMap"
  - Handle Vec::new() → CallExtern `__vow_vec_new`
  - Handle Option::None → RegionAlloc + FieldSet(tag=0)
  - General case: RegionAlloc, FieldSet tag, FieldSet payloads
- [ ] **EXPR_METHOD** → dispatch on inst_struct_type tag:
  - String: len→`__vow_string_len`, push_str→`__vow_string_push_str`, eq→`__vow_string_eq`, byte_at→`__vow_string_byte_at`, push_byte→`__vow_string_push_byte`
  - HashMap: len→`__vow_map_len`, insert→`__vow_map_insert`, get→`__vow_map_get`, contains_key→`__vow_map_contains`, remove→`__vow_map_remove`
  - Default (Vec): len→`__vow_vec_len`, push→`__vow_vec_push_val`
- [ ] **EXPR_INDEX** → lower base + index, emit CallExtern `__vow_vec_get_val`
- [ ] **EXPR_MATCH** → (~100 lines in Rust)
  - Lower scrutinee, FieldGet tag at index 0
  - For each arm: check tag equality, branch, push scope, bind payload fields, lower body, Upsilon + Jump to merge
  - Merge: Phi from all arms, backpatch Upsilons
- [ ] **EXPR_QUESTION** → (~50 lines)
  - Lower inner, FieldGet tag, check == 0 (None)
  - Branch: early_return_block (RegionAlloc None, Return) / continue_block (FieldGet payload)

### lower_stmt

- [ ] **STMT_LET** → lower init expr, define in scope; if type annotation is Named (non-primitive) or Generic, tag in inst_struct_type
- [ ] **STMT_EXPR** → lower expr (discard result)

### lower_block / lower_block_inner

- [ ] `fn lower_block(ctx: LowerCtx, bid: i64) -> i64` — push_scope, lower_block_inner, pop_scope
- [ ] `fn lower_block_inner(ctx: LowerCtx, bid: i64) -> i64` — lower stmts, handle trailing expr
  - Walk stmts: `list_get(a, blk_stmts(a, bid), i)` for each i
  - Break on is_terminated
  - If terminated, return -1 (sentinel)
  - Self-hosted parser stores trailing expr as last STMT_EXPR with has_semi=0 in stmts list
    (NOT in blk_trail) — **or** it may be in blk_trail for some blocks
  - Check blk_trail first; if -1, check last stmt for STMT_EXPR with has_semi=0
  - If no trailing expr: emit ConstUnit

### lower_function

- [ ] `fn lower_function_vow(ctx: LowerCtx, a: AstArena, fid: i64) -> IrFunction`
  - Get params from fn_params_lid: iterate pairs [name_sid, type_id]
  - Lower param types
  - Create LowerCtx
  - Emit GetArg for each param; tag String/HashMap/Vec/user-struct params in inst_struct_type
  - Define params in scope
  - **Vow requires:** If fn_vow_lid(a, fid) != -1, iterate clause list; for each
    CLAUSE_REQUIRES: call `lower_requires(ctx, clause_expr)` — emits VowRequires +
    Branch(violation_block, continue_block). Blame = Caller.
  - Push scope, lower_block_inner on fn_body_bid, pop scope
  - If not terminated: check ensures (same as EXPR_RETURN), emit Return(trailing)
  - Return IrFunction

### Vow lowering helpers (from vow.rs)

- [ ] `fn lower_requires(ctx: LowerCtx, clause_eid: i64)` — lower predicate expr,
  emit VowRequires(pred_id), Branch to violation/continue, in violation emit
  `__vow_violation` call then Unreachable
- [ ] `fn lower_ensures(ctx: LowerCtx, clause_eid: i64, result_id: i64)` — same as
  requires but blame=Callee, and EXPR_RESULT resolves to result_id
- [ ] `fn lower_invariant(ctx: LowerCtx, clause_eid: i64)` — same as requires but
  uses VowInvariant opcode, blame=Callee
- [ ] `fn collect_free_vars(ctx: LowerCtx, eid: i64) -> Vec<String>` — walk predicate
  expr tree, collect all EXPR_IDENT names that are in scope, deduplicate
- [ ] `fn collect_vars_in_expr(ctx: LowerCtx, eid: i64, out: Vec<String>)` — recursive
  helper for collect_free_vars

### lower_module

- [ ] `fn lower_module_vow(m: Module) -> IrModule`
  - Build func_names + func_ret_tys from function items
  - Build struct_field_map from struct items
  - Build enum_variant_map from enum items (+ always add Option/Result)
  - For each function item: call lower_function_vow
  - Merge string pools (offset ConstStr indices by base)
  - Return IrModule

### Compile and verify

- [ ] `./target/release/vow --no-verify compiler/lower.vow` compiles
- [ ] `./compiler/main compiler/lower.vow` — 0 errors
- [ ] Sanity test: main.vow calls lower_module_vow on test_arith.vow, prints IR

---

## Task 4.5: Test — verify identical IR output

**Asana:** GID 1213487103468796
**Depends on:** Tasks 4.1–4.4

### Test corpus

Five programs covering all major IR constructs including vow blocks:

1. `examples/hello.vow` — one function (main), string literal, print_str call, return 0
2. `examples/countdown.vow` — two functions (countdown with while loop + main), mutable variable, binop, call
3. `compiler/test_arith.vow` (recreate) — two functions (add + main), binop, print_i64 call
4. `examples/divide.vow` — `requires: y != 0` (tests VowRequires lowering, blame=Caller)
5. `examples/bisect.vow` — `requires` + while `invariant` (tests VowInvariant in loop header)

### Wire up compiler/main.vow

- [ ] Add `use ir`, `use ir_printer`, `use lower` to main.vow
- [ ] After `check_module`, if 0 errors:
  - Call `lower_module_vow(m)` → get IrModule
  - Call `print_module(ir_mod)` → get IR text string
  - Print IR text to stdout (or write to `.vow.ir` file)
- [ ] Recompile `./compiler/main` with the updated main.vow

### Collect reference IR from Rust

- [ ] `./target/release/vow --dump-ir --no-verify examples/hello.vow > /tmp/hello_rust.ir`
- [ ] `./target/release/vow --dump-ir --no-verify examples/countdown.vow > /tmp/countdown_rust.ir`
- [ ] `./target/release/vow --dump-ir --no-verify compiler/test_arith.vow > /tmp/arith_rust.ir`
- [ ] `./target/release/vow --dump-ir --no-verify examples/divide.vow > /tmp/divide_rust.ir`
- [ ] `./target/release/vow --dump-ir --no-verify examples/bisect.vow > /tmp/bisect_rust.ir`

### Collect Vow IR

- [ ] `./compiler/main examples/hello.vow > /tmp/hello_vow.ir` (strip the "N tokens, N items, N errors" line)
- [ ] `./compiler/main examples/countdown.vow > /tmp/countdown_vow.ir`
- [ ] `./compiler/main compiler/test_arith.vow > /tmp/arith_vow.ir`
- [ ] `./compiler/main examples/divide.vow > /tmp/divide_vow.ir`
- [ ] `./compiler/main examples/bisect.vow > /tmp/bisect_vow.ir`

### Compare

- [ ] `diff /tmp/hello_rust.ir /tmp/hello_vow.ir` — empty (identical)
- [ ] `diff /tmp/countdown_rust.ir /tmp/countdown_vow.ir` — empty
- [ ] `diff /tmp/arith_rust.ir /tmp/arith_vow.ir` — empty
- [ ] `diff /tmp/divide_rust.ir /tmp/divide_vow.ir` — empty
- [ ] `diff /tmp/bisect_rust.ir /tmp/bisect_vow.ir` — empty

### Debug mismatches

If diffs are non-empty, likely causes and fixes:
- **Spacing in print_inst:** verify pad_right produces exactly the same alignment
- **Effect ordering:** Rust sorts effects; Vow bitmask must expand in same order
- **String pool Debug escaping:** verify debug_escape_str matches Rust `{:?}` exactly
- **InstId numbering:** verify both pipelines assign IDs in the same sequence
- **Trailing expr handling:** verify lower_block_inner treats has_semi=0 correctly

### Completion criteria

All three diffs are empty. Mark Asana parent task (GID 1213457543393580) complete.

---

## Open questions

1. **Float literal encoding:** `EXPR_LIT_FLOAT` stores a value in `expr_a`. How is the
   f64 value packed into an i64? (bit-cast via `f64::to_bits()`?) — Needed only if test
   corpus includes floats. Current corpus does not; defer.

2. **ScopeSnap struct: RESOLVED.** Using append-only scope with truncation-based
   snapshot/restore. `ScopeSnap` holds 3 integers (lengths), not Vec copies.
   No deep copying needed. See Task 4.4 scope management section for details.

3. **Trailing expr representation:** The Rust AST has `Block.trailing_expr` as a
   separate field. The self-hosted AST stores it in `blk_trail(a, bid)` which returns
   an expr_id or -1. But the memory notes say the self-hosted parser sometimes stores
   trailing exprs as `STMT_EXPR(has_semi=0)` in the stmts list.
   **Resolution:** Check `blk_trail(a, bid)` first. If -1, peek at last stmt: if it's
   `STMT_EXPR` with `stmt_b(a, sid) == 0`, treat it as trailing expr and exclude from
   stmt iteration. This matches how cgen.vow handled it.

4. **Vec<IrInst> and Vec<IrBlock>:** Vow's `__vow_vec_push_val` pushes an i64. For
   user structs, the i64 is a heap pointer. So `func.blocks.push(blk)` pushes the
   pointer to `blk`. Then `func.blocks[i]` returns the pointer, and field access
   works. This should be transparent — verify with a small test if uncertain.

---

## Execution order

```
Task 4.0 (parser vow blocks) ← FIRST: prerequisite for vow-aware lowering
Task 4.3 (--dump-ir flag)    ← can start immediately, Rust-only
Task 4.1 (IR data types)     ← can start immediately
    ↓
Task 4.2 (IR printer)        ← needs 4.1
Task 4.4 (lowering)          ← needs 4.0 + 4.1, can parallel with 4.2
    ↓
Task 4.5 (test)              ← needs all of the above
```

Tasks 4.0, 4.1, and 4.3 are independent and can be done in any order.
Tasks 4.2 and 4.4 both depend on 4.1 but are independent of each other.
Task 4.4 additionally depends on 4.0 (needs vow block data in the AST).
