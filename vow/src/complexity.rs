//! `vow complexity` — hand-built JSON; never use serde_json (byte-identity with self-hosted).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use vow_ir::{InstData, Opcode};
use vow_syntax::ast::{
    BinOp, Block, Effect, Expr, ExprKind, FnDef, Item, Lit, Stmt, UnOp, VowBlock, VowClause,
};

// Effect -> bit, matching the self-hosted EFF_* constants (io=1,panic=2,
// read=4,unsafe=8,write=16), so effect_breadth (popcount) and the canonical
// effects list are byte-identical with the self-hosted compiler.
fn effect_bit(e: &Effect) -> i64 {
    match e {
        Effect::IO => 1,
        Effect::Panic => 2,
        Effect::Read => 4,
        Effect::Unsafe => 8,
        Effect::Write => 16,
    }
}

fn cx_popcount_bits(mut mask: i64) -> i64 {
    let mut c = 0;
    while mask > 0 {
        c += mask % 2;
        mask /= 2;
    }
    c
}

fn cx_emit_effects(out: &mut String, eff: i64) {
    out.push('[');
    let mut ne = 0;
    for (bit, name) in [
        (1, "io"),
        (2, "panic"),
        (4, "read"),
        (8, "unsafe"),
        (16, "write"),
    ] {
        if (eff / bit) % 2 != 0 {
            if ne > 0 {
                out.push(',');
            }
            out.push('"');
            out.push_str(name);
            out.push('"');
            ne += 1;
        }
    }
    out.push(']');
}

// Count linear-struct literals (StructLiteral whose name is a linear struct) in
// an expression subtree. Mirrors compiler/complexity.vow cx_lv_expr's recursion.
fn lv_block(b: &Block, linear: &HashSet<String>) -> i64 {
    let mut c = 0;
    for s in &b.stmts {
        match s {
            Stmt::Let { init, .. } => c += lv_expr(init, linear),
            Stmt::Expr { expr, .. } => c += lv_expr(expr, linear),
        }
    }
    if let Some(t) = &b.trailing_expr {
        c += lv_expr(t, linear);
    }
    c
}

fn lv_expr(e: &Expr, linear: &HashSet<String>) -> i64 {
    let mut c = 0;
    match &e.kind {
        ExprKind::StructLiteral { name, fields } => {
            if linear.contains(name) {
                c += 1;
            }
            for (_, v) in fields {
                c += lv_expr(v, linear);
            }
        }
        ExprKind::BinaryOp { lhs, rhs, .. } => {
            c += lv_expr(lhs, linear);
            c += lv_expr(rhs, linear);
        }
        ExprKind::Index { base, index } => {
            c += lv_expr(base, linear);
            c += lv_expr(index, linear);
        }
        ExprKind::Assign { lhs, rhs } => {
            c += lv_expr(lhs, linear);
            c += lv_expr(rhs, linear);
        }
        ExprKind::UnaryOp { operand, .. } => c += lv_expr(operand, linear),
        ExprKind::FieldAccess { base, .. } => c += lv_expr(base, linear),
        ExprKind::Question { expr } => c += lv_expr(expr, linear),
        ExprKind::Cast { expr, .. } => c += lv_expr(expr, linear),
        ExprKind::Borrow { expr } => c += lv_expr(expr, linear),
        ExprKind::Call { callee, args } => {
            c += lv_expr(callee, linear);
            for a in args {
                c += lv_expr(a, linear);
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            c += lv_expr(receiver, linear);
            for a in args {
                c += lv_expr(a, linear);
            }
        }
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            c += lv_expr(condition, linear);
            c += lv_block(then_branch, linear);
            if let Some(e2) = else_branch {
                c += lv_expr(e2, linear);
            }
        }
        ExprKind::While {
            condition, body, ..
        } => {
            c += lv_expr(condition, linear);
            c += lv_block(body, linear);
        }
        ExprKind::ForEach { iterable, body, .. } => {
            c += lv_expr(iterable, linear);
            c += lv_block(body, linear);
        }
        ExprKind::Loop { body, .. } => c += lv_block(body, linear),
        ExprKind::Match { scrutinee, arms } => {
            c += lv_expr(scrutinee, linear);
            for arm in arms {
                c += lv_expr(&arm.body, linear);
            }
        }
        ExprKind::Return { value } => {
            if let Some(v) = value {
                c += lv_expr(v, linear);
            }
        }
        ExprKind::Break { value } => {
            if let Some(v) = value {
                c += lv_expr(v, linear);
            }
        }
        ExprKind::Block(b) => c += lv_block(b, linear),
        ExprKind::EnumConstruct { fields, .. } => {
            for v in fields {
                c += lv_expr(v, linear);
            }
        }
        ExprKind::Tuple(elems) => {
            for v in elems {
                c += lv_expr(v, linear);
            }
        }
        ExprKind::Lit(_) | ExprKind::Ident(_) | ExprKind::Continue | ExprKind::Result => {}
    }
    c
}

// Cyclomatic complexity from the IR CFG, decision-count form:
// (number of conditional branches) + 1. Mirrors compiler/complexity.vow
// cx_cyclomatic_ir (see the note there on why this is not `e - n + 2`).
fn ir_cyclomatic(f: &vow_ir::Function) -> i64 {
    let mut branches: i64 = 0;
    for b in &f.blocks {
        for inst in &b.insts {
            if matches!(inst.opcode, Opcode::Branch) {
                branches += 1;
            }
        }
    }
    branches + 1
}

// IR-derived per-function coupling/linear info (experimental tier).
struct IrInfo {
    cyclomatic: i64,
    consumes: i64,
    borrows: i64,
    fan_in: i64,
    fan_out: i64,
    effect_fanout: i64,
}

// Distinct user-function callee ids (IOP_CALL with CallTarget; externs excluded).
fn ir_callees(f: &vow_ir::Function) -> std::collections::HashSet<u32> {
    let mut s = std::collections::HashSet::new();
    for b in &f.blocks {
        for inst in &b.insts {
            if let InstData::CallTarget(fid) = &inst.data {
                s.insert(fid.0);
            }
        }
    }
    s
}

fn cx_henry_kafura(nloc: i64, fan_in: i64, fan_out: i64) -> i64 {
    let fl = fan_in.wrapping_mul(fan_out);
    cx_sat(nloc.wrapping_mul(fl).wrapping_mul(fl))
}

use crate::emit_frontend_diagnostics;
use crate::frontend::{FrontendGoal, prepare_frontend};

// Structural counts accumulated by the AST walk. Mirrors the self-hosted
// `cx_walk_*` (compiler/complexity.vow) handled-kind set exactly so the JSON
// stays byte-identical.
#[derive(Default)]
struct Acc {
    decisions: i64,
    stmts: i64,
}

fn walk_block(b: &Block, acc: &mut Acc) {
    for s in &b.stmts {
        acc.stmts += 1;
        match s {
            Stmt::Let { init, .. } => walk_expr(init, acc),
            Stmt::Expr { expr, .. } => walk_expr(expr, acc),
        }
    }
    if let Some(t) = &b.trailing_expr {
        // The trailing expression counts as one statement, matching how the
        // self-hosted parser folds a block's final expression into blk_stmts.
        acc.stmts += 1;
        walk_expr(t, acc);
    }
}

// Cognitive Complexity (Vow-adapted). Mirrors compiler/complexity.vow's
// `cog_*` exactly. `logctx` is the enclosing logical operator (for counting
// contiguous &&/|| runs once each); `selfn` is the function's own name (for
// direct-recursion detection).
#[derive(Default)]
struct CogAcc {
    cog: i64,
    max_nesting: i64,
    self_calls: i64,
}

fn cog_track_max(acc: &mut CogAcc, depth: i64) {
    if depth > acc.max_nesting {
        acc.max_nesting = depth;
    }
}

fn cog_block(b: &Block, nesting: i64, selfn: &str, acc: &mut CogAcc) {
    for s in &b.stmts {
        match s {
            Stmt::Let { init, .. } => cog_expr(init, nesting, None, selfn, acc),
            Stmt::Expr { expr, .. } => cog_expr(expr, nesting, None, selfn, acc),
        }
    }
    if let Some(t) = &b.trailing_expr {
        cog_expr(t, nesting, None, selfn, acc);
    }
}

fn cog_expr(e: &Expr, nesting: i64, logctx: Option<BinOp>, selfn: &str, acc: &mut CogAcc) {
    match &e.kind {
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            acc.cog += 1 + nesting;
            cog_track_max(acc, nesting + 1);
            cog_expr(condition, nesting, None, selfn, acc);
            cog_block(then_branch, nesting + 1, selfn, acc);
            if let Some(e2) = else_branch {
                if matches!(e2.kind, ExprKind::If { .. }) {
                    cog_expr(e2, nesting, None, selfn, acc);
                } else {
                    acc.cog += 1;
                    cog_expr(e2, nesting + 1, None, selfn, acc);
                }
            }
        }
        ExprKind::While {
            condition, body, ..
        } => {
            acc.cog += 1 + nesting;
            cog_track_max(acc, nesting + 1);
            cog_expr(condition, nesting, None, selfn, acc);
            cog_block(body, nesting + 1, selfn, acc);
        }
        ExprKind::ForEach { iterable, body, .. } => {
            acc.cog += 1 + nesting;
            cog_track_max(acc, nesting + 1);
            cog_expr(iterable, nesting, None, selfn, acc);
            cog_block(body, nesting + 1, selfn, acc);
        }
        ExprKind::Loop { body, .. } => {
            acc.cog += 1 + nesting;
            cog_track_max(acc, nesting + 1);
            cog_block(body, nesting + 1, selfn, acc);
        }
        ExprKind::Match { scrutinee, arms } => {
            acc.cog += 1 + nesting;
            cog_track_max(acc, nesting + 1);
            cog_expr(scrutinee, nesting, None, selfn, acc);
            for arm in arms {
                cog_expr(&arm.body, nesting + 1, None, selfn, acc);
            }
        }
        ExprKind::BinaryOp { op, lhs, rhs } => {
            if matches!(op, BinOp::And | BinOp::Or) {
                if Some(*op) != logctx {
                    acc.cog += 1;
                }
                cog_expr(lhs, nesting, Some(*op), selfn, acc);
                cog_expr(rhs, nesting, Some(*op), selfn, acc);
            } else {
                cog_expr(lhs, nesting, None, selfn, acc);
                cog_expr(rhs, nesting, None, selfn, acc);
            }
        }
        ExprKind::Question { expr } => {
            acc.cog += 1;
            cog_expr(expr, nesting, None, selfn, acc);
        }
        ExprKind::Call { callee, args } => {
            if matches!(&callee.kind, ExprKind::Ident(name) if name == selfn) {
                acc.self_calls += 1;
            }
            cog_expr(callee, nesting, None, selfn, acc);
            for a in args {
                cog_expr(a, nesting, None, selfn, acc);
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            cog_expr(receiver, nesting, None, selfn, acc);
            for a in args {
                cog_expr(a, nesting, None, selfn, acc);
            }
        }
        ExprKind::UnaryOp { operand, .. } => cog_expr(operand, nesting, None, selfn, acc),
        ExprKind::FieldAccess { base, .. } => cog_expr(base, nesting, None, selfn, acc),
        ExprKind::Index { base, index } => {
            cog_expr(base, nesting, None, selfn, acc);
            cog_expr(index, nesting, None, selfn, acc);
        }
        ExprKind::Return { value } => {
            if let Some(v) = value {
                cog_expr(v, nesting, None, selfn, acc);
            }
        }
        ExprKind::Break { value } => {
            if let Some(v) = value {
                cog_expr(v, nesting, None, selfn, acc);
            }
        }
        ExprKind::Block(b) => cog_block(b, nesting, selfn, acc),
        ExprKind::Assign { lhs, rhs } => {
            cog_expr(lhs, nesting, None, selfn, acc);
            cog_expr(rhs, nesting, None, selfn, acc);
        }
        ExprKind::StructLiteral { fields, .. } => {
            for (_, v) in fields {
                cog_expr(v, nesting, None, selfn, acc);
            }
        }
        ExprKind::EnumConstruct { fields, .. } => {
            for v in fields {
                cog_expr(v, nesting, None, selfn, acc);
            }
        }
        ExprKind::Cast { expr, .. } => cog_expr(expr, nesting, None, selfn, acc),
        ExprKind::Tuple(elems) => {
            for v in elems {
                cog_expr(v, nesting, None, selfn, acc);
            }
        }
        ExprKind::Borrow { expr } => cog_expr(expr, nesting, None, selfn, acc),
        ExprKind::Lit(_) | ExprKind::Ident(_) | ExprKind::Continue | ExprKind::Result => {}
    }
}

fn walk_expr(e: &Expr, acc: &mut Acc) {
    match &e.kind {
        ExprKind::Call { callee, args } => {
            walk_expr(callee, acc);
            for a in args {
                walk_expr(a, acc);
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            walk_expr(receiver, acc);
            for a in args {
                walk_expr(a, acc);
            }
        }
        ExprKind::BinaryOp { op, lhs, rhs } => {
            if matches!(op, BinOp::And | BinOp::Or) {
                acc.decisions += 1;
            }
            walk_expr(lhs, acc);
            walk_expr(rhs, acc);
        }
        ExprKind::UnaryOp { operand, .. } => walk_expr(operand, acc),
        ExprKind::FieldAccess { base, .. } => walk_expr(base, acc),
        ExprKind::Question { expr } => {
            acc.decisions += 1;
            walk_expr(expr, acc);
        }
        ExprKind::Index { base, index } => {
            walk_expr(base, acc);
            walk_expr(index, acc);
        }
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            acc.decisions += 1;
            walk_expr(condition, acc);
            walk_block(then_branch, acc);
            if let Some(e2) = else_branch {
                walk_expr(e2, acc);
            }
        }
        ExprKind::While {
            condition, body, ..
        } => {
            acc.decisions += 1;
            walk_expr(condition, acc);
            walk_block(body, acc);
        }
        ExprKind::ForEach { iterable, body, .. } => {
            acc.decisions += 1;
            walk_expr(iterable, acc);
            walk_block(body, acc);
        }
        ExprKind::Loop { body, .. } => {
            acc.decisions += 1;
            walk_block(body, acc);
        }
        ExprKind::Match { scrutinee, arms } => {
            walk_expr(scrutinee, acc);
            if arms.len() > 1 {
                acc.decisions += arms.len() as i64 - 1;
            }
            for arm in arms {
                walk_expr(&arm.body, acc);
            }
        }
        ExprKind::Return { value } => {
            if let Some(v) = value {
                walk_expr(v, acc);
            }
        }
        ExprKind::Break { value } => {
            if let Some(v) = value {
                walk_expr(v, acc);
            }
        }
        ExprKind::Block(b) => walk_block(b, acc),
        ExprKind::Assign { lhs, rhs } => {
            walk_expr(lhs, acc);
            walk_expr(rhs, acc);
        }
        ExprKind::StructLiteral { fields, .. } => {
            for (_, v) in fields {
                walk_expr(v, acc);
            }
        }
        ExprKind::EnumConstruct { fields, .. } => {
            for v in fields {
                walk_expr(v, acc);
            }
        }
        ExprKind::Cast { expr, .. } => walk_expr(expr, acc),
        ExprKind::Tuple(elems) => {
            for v in elems {
                walk_expr(v, acc);
            }
        }
        // `&x` has no node in the self-hosted AST (it is transparent), so
        // recurse through it here without counting it, to stay byte-identical.
        ExprKind::Borrow { expr } => walk_expr(expr, acc),
        // Leaves (mirror the self-hosted walk, which does not recurse these).
        ExprKind::Lit(_) | ExprKind::Ident(_) | ExprKind::Continue | ExprKind::Result => {}
    }
}

// Per-function computed metrics, collected before emission so file-level
// aggregates (max score, over-threshold count) can be written ahead of the
// functions array.
struct CxEmit {
    name: String,
    line: i64,
    nloc: i64,
    tokens: i64,
    stmts: i64,
    params: i64,
    cyclomatic: i64,
    cyclomatic_ir: i64,
    cognitive: i64,
    max_nesting: i64,
    h_n1: i64,
    h_n2: i64,
    h_bign1: i64,
    h_bign2: i64,
    h_vocab: i64,
    h_length: i64,
    h_volume_s: i64,
    h_difficulty_s: i64,
    h_effort_s: i64,
    eff: i64,
    effect_fanout: i64,
    linear_values: i64,
    linear_consumes: i64,
    linear_borrows: i64,
    score: i64,
    cog_sub_s: i64,
    size_sub_s: i64,
    base_s: i64,
    vow_bump_s: i64,
    contract: Contract,
    verif: Verif,
}

fn cx_emit_fn(out: &mut String, r: &CxEmit, thr: i64) {
    out.push_str("{\"name\":\"");
    out.push_str(&cx_json_escape(&r.name));
    out.push_str("\",\"line\":");
    out.push_str(&r.line.to_string());
    out.push_str(",\"complexity_score\":");
    out.push_str(&r.score.to_string());
    out.push_str(",\"score_factors\":{\"cognitive_sub\":");
    out.push_str(&cx_fmt_fixed(r.cog_sub_s));
    out.push_str(",\"size_sub\":");
    out.push_str(&cx_fmt_fixed(r.size_sub_s));
    out.push_str(",\"vow_bump\":");
    out.push_str(&cx_fmt_fixed(r.vow_bump_s));
    out.push_str(",\"base\":");
    out.push_str(&cx_fmt_fixed(r.base_s));
    out.push_str(",\"over_threshold\":");
    out.push_str(if r.score > thr { "true" } else { "false" });
    out.push_str("},\"size\":{\"nloc\":");
    out.push_str(&r.nloc.to_string());
    out.push_str(",\"tokens\":");
    out.push_str(&r.tokens.to_string());
    out.push_str(",\"stmts\":");
    out.push_str(&r.stmts.to_string());
    out.push_str(",\"params\":");
    out.push_str(&r.params.to_string());
    out.push_str("},\"structural\":{\"cyclomatic\":");
    out.push_str(&r.cyclomatic.to_string());
    out.push_str(",\"cyclomatic_ir\":");
    out.push_str(&r.cyclomatic_ir.to_string());
    out.push_str(",\"cognitive\":");
    out.push_str(&r.cognitive.to_string());
    out.push_str(",\"max_nesting\":");
    out.push_str(&r.max_nesting.to_string());
    out.push_str(",\"halstead\":{\"n1\":");
    out.push_str(&r.h_n1.to_string());
    out.push_str(",\"n2\":");
    out.push_str(&r.h_n2.to_string());
    out.push_str(",\"N1\":");
    out.push_str(&r.h_bign1.to_string());
    out.push_str(",\"N2\":");
    out.push_str(&r.h_bign2.to_string());
    out.push_str(",\"vocabulary\":");
    out.push_str(&r.h_vocab.to_string());
    out.push_str(",\"length\":");
    out.push_str(&r.h_length.to_string());
    out.push_str(",\"volume\":");
    out.push_str(&cx_fmt_fixed(r.h_volume_s));
    out.push_str(",\"difficulty\":");
    out.push_str(&cx_fmt_fixed(r.h_difficulty_s));
    out.push_str(",\"effort\":");
    out.push_str(&cx_fmt_fixed(r.h_effort_s));
    out.push_str("}},\"vow\":{\"tier\":\"experimental\",\"effects\":");
    cx_emit_effects(out, r.eff);
    out.push_str(",\"effect_breadth\":");
    out.push_str(&cx_popcount_bits(r.eff).to_string());
    out.push_str(",\"effect_fanout\":");
    out.push_str(&r.effect_fanout.to_string());
    out.push_str(",\"linear_values\":");
    out.push_str(&r.linear_values.to_string());
    out.push_str(",\"linear_consumes\":");
    out.push_str(&r.linear_consumes.to_string());
    out.push_str(",\"linear_borrows\":");
    out.push_str(&r.linear_borrows.to_string());
    out.push_str(",\"contract\":{\"requires\":");
    out.push_str(&r.contract.requires.to_string());
    out.push_str(",\"ensures\":");
    out.push_str(&r.contract.ensures.to_string());
    out.push_str(",\"invariants\":");
    out.push_str(&r.contract.invariants.to_string());
    out.push_str(",\"predicate_nodes\":");
    out.push_str(&r.contract.predicate_nodes.to_string());
    out.push_str(",\"predicate_depth\":");
    out.push_str(&r.contract.predicate_depth.to_string());
    out.push_str(",\"free_vars\":");
    out.push_str(&r.contract.free_vars.to_string());
    out.push_str(",\"has_vec_quantification\":");
    out.push_str(if r.contract.has_vec_quant {
        "true"
    } else {
        "false"
    });
    out.push_str("}},\"verification\":{\"tier\":\"experimental\",\"loops_total\":");
    out.push_str(&r.verif.loops_total.to_string());
    out.push_str(",\"loops_without_invariant\":");
    out.push_str(&r.verif.loops_without_invariant.to_string());
    out.push_str(",\"max_loop_nesting\":");
    out.push_str(&r.verif.max_loop_nesting.to_string());
    out.push_str(",\"contract_predicate_cost\":");
    out.push_str(&r.verif.contract_predicate_cost.to_string());
    out.push_str("}}");
}

pub(crate) fn run_complexity_command(
    source: &Path,
    cog_anchor: i64,
    nloc_anchor: i64,
    max_score: i64,
    max_cognitive: i64,
    max_cyclomatic: i64,
) {
    let frontend = match prepare_frontend(source, FrontendGoal::LoweredIr) {
        Ok(bundle) => {
            emit_frontend_diagnostics(bundle.diagnostics());
            bundle
        }
        Err(error) => {
            emit_frontend_diagnostics(error.diagnostics());
            eprintln!("vow complexity: {}", error.failure_message());
            std::process::exit(1);
        }
    };

    let src = match read_complexity_source(source) {
        Ok(src) => src,
        Err(error) => {
            eprintln!("vow complexity: {error}");
            std::process::exit(1);
        }
    };
    let file_nloc = line_at(&src, src.len());
    let module = frontend.module();
    // Report only the entry file's functions (deps' spans are foreign). Match by
    // `source`'s path — what module_loader stored for the entry's items — so this
    // mirrors the self-hosted `itf == path` and stays correct when the entry has no items.
    let item_files = frontend.item_files();
    let entry_file = source.to_string_lossy().into_owned();
    let thr = if max_score >= 0 { max_score } else { 80 };

    // IR-derived per-function info, matched to AST functions by name.
    let mut ir_info: HashMap<String, IrInfo> = HashMap::new();
    let mut fan_in_max: i64 = 0;
    let mut fan_out_max: i64 = 0;
    if let Some(m) = frontend.ir() {
        let funcs = &m.functions;
        let callees: Vec<HashSet<u32>> = funcs.iter().map(ir_callees).collect();
        let effectful: HashSet<u32> = funcs
            .iter()
            .filter(|f| !f.effects.is_empty())
            .map(|f| f.id.0)
            .collect();
        for (idx, f) in funcs.iter().enumerate() {
            let fan_out = callees[idx].len() as i64;
            let fid = f.id.0;
            // fan_in counts in-file callers only (schema), so restrict the caller
            // scan to entry-file functions — a dependency that calls an entry
            // function must not inflate the entry function's fan_in.
            let fan_in = funcs
                .iter()
                .zip(callees.iter())
                .filter(|(cf, s)| cf.source_file == entry_file && s.contains(&fid))
                .count() as i64;
            let effect_fanout = callees[idx]
                .iter()
                .filter(|c| effectful.contains(c))
                .count() as i64;
            let consumes = f
                .blocks
                .iter()
                .flat_map(|b| &b.insts)
                .filter(|x| matches!(x.opcode, Opcode::LinearConsume))
                .count() as i64;
            let borrows = f
                .blocks
                .iter()
                .flat_map(|b| &b.insts)
                .filter(|x| matches!(x.opcode, Opcode::LinearBorrow))
                .count() as i64;
            ir_info.insert(
                f.name.clone(),
                IrInfo {
                    cyclomatic: ir_cyclomatic(f),
                    consumes,
                    borrows,
                    fan_in,
                    fan_out,
                    effect_fanout,
                },
            );
        }
    }

    // Linear struct names, for counting linear-struct literals.
    let linear_structs: HashSet<String> = module
        .items
        .iter()
        .filter_map(|it| match it {
            Item::Struct(s) if s.is_linear => Some(s.name.clone()),
            _ => None,
        })
        .collect();

    // Pass 1: analyze + score every function defined in the entry file.
    let mut recs: Vec<CxEmit> = Vec::new();
    for (idx, item) in module.items.iter().enumerate() {
        if let Item::Fn(f) = item {
            if item_files.get(idx).map(String::as_str) != Some(entry_file.as_str()) {
                continue;
            }
            let start = f.span.start as usize;
            let end = (f.span.start + f.span.len) as usize;
            let first_line = line_at(&src, start);
            let nloc = line_at(&src, end) - first_line + 1;
            let mut acc = Acc::default();
            walk_block(&f.body, &mut acc);
            let cyclomatic = acc.decisions + 1;
            let info = ir_info.get(&f.name);
            let cyclomatic_ir = info.map(|x| x.cyclomatic).unwrap_or(-1);
            let effect_fanout = info.map(|x| x.effect_fanout).unwrap_or(0);
            let linear_consumes = info.map(|x| x.consumes).unwrap_or(0);
            let linear_borrows = info.map(|x| x.borrows).unwrap_or(0);
            let eff = f.effects.iter().fold(0i64, |acc, e| acc | effect_bit(e));
            let linear_values = lv_block(&f.body, &linear_structs);
            let mut cacc = CogAcc::default();
            cog_block(&f.body, 0, &f.name, &mut cacc);
            let cognitive = cacc.cog + if cacc.self_calls > 0 { 1 } else { 0 };
            let mut hacc = HalAcc::default();
            hal_block(&f.body, &mut hacc);
            let h_n1 = cx_popcount(hacc.mask);
            let h_n2 = hacc.seen.len() as i64;
            let h_vocab = h_n1 + h_n2;
            let h_length = hacc.bign1 + hacc.bign2;
            let h_volume_s = cx_sat(h_length.wrapping_mul(cx_log2_milli(h_vocab)));
            let h_difficulty_s = if h_n2 > 0 {
                cx_sat(h_n1.wrapping_mul(hacc.bign2).wrapping_mul(500) / h_n2)
            } else {
                0
            };
            let h_effort_s = cx_sat(h_difficulty_s.wrapping_mul(h_volume_s) / 1000);
            let contract = analyze_contract(f);
            let pcost = contract_predicate_cost(&contract);
            let verif = analyze_verif(f, pcost);
            let effect_breadth = cx_popcount_bits(eff);
            let v = cx_vow_bump(effect_breadth, linear_consumes, pcost);
            let sc = cx_score(cognitive, nloc, cog_anchor, nloc_anchor, v);
            recs.push(CxEmit {
                name: f.name.clone(),
                line: first_line,
                nloc,
                tokens: h_length,
                stmts: acc.stmts,
                params: f.params.len() as i64,
                cyclomatic,
                cyclomatic_ir,
                cognitive,
                max_nesting: cacc.max_nesting,
                h_n1,
                h_n2,
                h_bign1: hacc.bign1,
                h_bign2: hacc.bign2,
                h_vocab,
                h_length,
                h_volume_s,
                h_difficulty_s,
                h_effort_s,
                eff,
                effect_fanout,
                linear_values,
                linear_consumes,
                linear_borrows,
                score: sc.score,
                cog_sub_s: sc.cog_sub_s,
                size_sub_s: sc.size_sub_s,
                base_s: sc.base_s,
                vow_bump_s: sc.vow_bump_s,
                contract,
                verif,
            });
        }
    }

    // File-level aggregates + exit-gating.
    let mut file_max = 0i64;
    let mut over_count = 0i64;
    let mut exit_code = 0i32;
    for r in &recs {
        if r.score > file_max {
            file_max = r.score;
        }
        if r.score > thr {
            over_count += 1;
        }
        if max_score >= 0 && r.score > max_score {
            exit_code = 1;
        }
        if max_cognitive >= 0 && r.cognitive > max_cognitive {
            exit_code = 1;
        }
        if max_cyclomatic >= 0 && r.cyclomatic > max_cyclomatic {
            exit_code = 1;
        }
    }

    // Module-level coupling maxima, restricted to the entry file's reported
    // functions (recs) — not all lowered IR functions — so a multi-module run
    // can't attribute fan-in/out or Henry-Kafura to an imported helper.
    let mut hk_max: i64 = 0;
    for r in &recs {
        if let Some(info) = ir_info.get(&r.name) {
            hk_max = hk_max.max(cx_henry_kafura(r.nloc, info.fan_in, info.fan_out));
            fan_in_max = fan_in_max.max(info.fan_in);
            fan_out_max = fan_out_max.max(info.fan_out);
        }
    }

    // Pass 2: emit.
    let mut out = String::from(
        "{\"schema_version\":\"1\",\"kind\":\"complexity_report\",\"tool\":\"vow\",\"files\":[{\"file\":\"",
    );
    out.push_str(&cx_json_escape(&source.to_string_lossy()));
    out.push_str("\",\"complexity_score\":");
    out.push_str(&file_max.to_string());
    out.push_str(",\"functions_over_threshold\":");
    out.push_str(&over_count.to_string());
    out.push_str(",\"nloc\":");
    out.push_str(&file_nloc.to_string());
    out.push_str(",\"functions\":[");
    for (k, r) in recs.iter().enumerate() {
        if k > 0 {
            out.push(',');
        }
        cx_emit_fn(&mut out, r, thr);
    }
    out.push_str("],\"module\":{\"tier\":\"experimental\",\"functions\":");
    out.push_str(&(recs.len() as i64).to_string());
    out.push_str(",\"fan_in_max\":");
    out.push_str(&fan_in_max.to_string());
    out.push_str(",\"fan_out_max\":");
    out.push_str(&fan_out_max.to_string());
    out.push_str(",\"henry_kafura_max\":");
    out.push_str(&hk_max.to_string());
    out.push_str("}}],\"summary\":{\"functions\":");
    out.push_str(&(recs.len() as i64).to_string());
    out.push_str(",\"nloc_total\":");
    out.push_str(&file_nloc.to_string());
    out.push_str(",\"threshold\":");
    out.push_str(&thr.to_string());
    out.push_str(",\"functions_over_threshold\":");
    out.push_str(&over_count.to_string());
    out.push_str(",\"thresholds_exceeded\":[");
    let mut emitted = 0;
    for r in &recs {
        if r.score > thr {
            if emitted > 0 {
                out.push(',');
            }
            out.push('"');
            out.push_str(&cx_json_escape(&r.name));
            out.push('"');
            emitted += 1;
        }
    }
    out.push_str("]}}");
    println!("{out}");
    std::process::exit(exit_code);
}

// ---- 0-100 gate score (mirrors compiler/complexity.vow cx_score) ----
struct CxScore {
    score: i64,
    cog_sub_s: i64,
    size_sub_s: i64,
    base_s: i64,
    vow_bump_s: i64,
}

fn cx_anchor_map(x: i64, t_in: i64) -> i64 {
    let t = if t_in <= 0 { 1 } else { t_in };
    if x <= t {
        return 800i64.wrapping_mul(x) / t;
    }
    800 + 200i64.wrapping_mul(x - t) / x
}

fn cx_soft_or(c: i64, s: i64) -> i64 {
    1000 - (1000 - c).wrapping_mul(1000 - s) / 1000
}

fn cx_round_0_100(val_scaled: i64) -> i64 {
    let r = (val_scaled * 100 + 500) / 1000;
    if r > 100 { 100 } else { r }
}

fn cx_score(cognitive: i64, nloc: i64, cog_anchor: i64, nloc_anchor: i64, v: i64) -> CxScore {
    let c = cx_anchor_map(cognitive, cog_anchor);
    let s = cx_anchor_map(nloc, nloc_anchor);
    let base = cx_soft_or(c, s);
    let score = cx_round_0_100(base + v);
    CxScore {
        score,
        cog_sub_s: c,
        size_sub_s: s,
        base_s: base,
        vow_bump_s: v,
    }
}

// ---- Fixed-point integer math (byte-identical with compiler/complexity.vow) ----
const CX_SAT: i64 = 2_000_000_000;

fn cx_sat(x: i64) -> i64 {
    if x > CX_SAT { CX_SAT } else { x }
}

fn cx_log2_milli(n: i64) -> i64 {
    if n <= 1 {
        return 0;
    }
    let mut intpart: i64 = 0;
    let mut v: i64 = n;
    while v > 1 {
        v /= 2;
        intpart += 1;
    }
    let mut p2: i64 = 1;
    let mut k: i64 = 0;
    while k < intpart {
        p2 *= 2;
        k += 1;
    }
    let scale: i64 = 1000;
    let mut m: i64 = n.wrapping_mul(scale) / p2;
    let mut frac: i64 = 0;
    let mut b: i64 = scale / 2;
    let mut iter = 0;
    while iter < 10 {
        m = m.wrapping_mul(m) / scale;
        if m >= 2 * scale {
            m /= 2;
            frac += b;
        }
        b /= 2;
        iter += 1;
    }
    intpart * scale + frac
}

fn cx_fmt_fixed(scaled: i64) -> String {
    let mut s = String::new();
    let mut x = scaled;
    if x < 0 {
        s.push('-');
        x = -x;
    }
    let whole = x / 1000;
    let frac = x - whole * 1000;
    s.push_str(&whole.to_string());
    s.push('.');
    if frac < 100 {
        s.push('0');
    }
    if frac < 10 {
        s.push('0');
    }
    s.push_str(&frac.to_string());
    s
}

fn cx_popcount(mask: i64) -> i64 {
    let mut c: i64 = 0;
    let mut m = mask;
    while m > 0 {
        c += m % 2;
        m /= 2;
    }
    c
}

// ---- Halstead operator/operand counting (mirrors compiler/complexity.vow) ----
#[derive(Default)]
struct HalAcc {
    mask: i64,
    bign1: i64,
    bign2: i64,
    seen: Vec<String>,
}

fn hal_op(acc: &mut HalAcc, id: i64) {
    acc.mask |= 1i64 << id;
    acc.bign1 += 1;
}

fn hal_operand(acc: &mut HalAcc, canon: String) {
    acc.bign2 += 1;
    if !acc.seen.iter().any(|e| e == &canon) {
        acc.seen.push(canon);
    }
}

// Binop id matches the self-hosted BINOP_* constants exactly.
fn rust_binop_id(op: BinOp) -> i64 {
    match op {
        BinOp::Add => 0,
        BinOp::Sub => 1,
        BinOp::Mul => 2,
        BinOp::Div => 3,
        BinOp::Rem => 4,
        BinOp::AddChecked => 5,
        BinOp::SubChecked => 6,
        BinOp::MulChecked => 7,
        BinOp::DivChecked => 8,
        BinOp::RemChecked => 9,
        BinOp::Eq => 10,
        BinOp::Ne => 11,
        BinOp::Lt => 12,
        BinOp::Le => 13,
        BinOp::Gt => 14,
        BinOp::Ge => 15,
        BinOp::And => 16,
        BinOp::Or => 17,
        BinOp::BitXor => 18,
        BinOp::BitAnd => 19,
        BinOp::BitOr => 20,
        BinOp::Shl => 21,
        BinOp::Shr => 22,
    }
}

fn hal_block(b: &Block, acc: &mut HalAcc) {
    for s in &b.stmts {
        match s {
            Stmt::Let { init, .. } => hal_expr(init, acc),
            Stmt::Expr { expr, .. } => hal_expr(expr, acc),
        }
    }
    if let Some(t) = &b.trailing_expr {
        hal_expr(t, acc);
    }
}

fn hal_expr(e: &Expr, acc: &mut HalAcc) {
    match &e.kind {
        ExprKind::Ident(name) => hal_operand(acc, format!("v:{name}")),
        ExprKind::Lit(Lit::Int(n)) => hal_operand(acc, format!("n:{n}")),
        ExprKind::Lit(Lit::Bool(b)) => hal_operand(acc, format!("b:{}", if *b { 1 } else { 0 })),
        ExprKind::Lit(Lit::String(s)) => hal_operand(acc, format!("s:{s}")),
        ExprKind::Lit(Lit::Float(_)) => {}
        ExprKind::BinaryOp { op, lhs, rhs } => {
            hal_op(acc, rust_binop_id(*op));
            hal_expr(lhs, acc);
            hal_expr(rhs, acc);
        }
        ExprKind::UnaryOp { op, operand } => {
            let id = match op {
                UnOp::Neg => 23,
                UnOp::Not => 24,
            };
            hal_op(acc, id);
            hal_expr(operand, acc);
        }
        ExprKind::Call { callee, args } => {
            hal_op(acc, 25);
            hal_expr(callee, acc);
            for a in args {
                hal_expr(a, acc);
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            hal_op(acc, 28);
            hal_expr(receiver, acc);
            for a in args {
                hal_expr(a, acc);
            }
        }
        ExprKind::Index { base, index } => {
            hal_op(acc, 26);
            hal_expr(base, acc);
            hal_expr(index, acc);
        }
        ExprKind::FieldAccess { base, .. } => {
            hal_op(acc, 27);
            hal_expr(base, acc);
        }
        ExprKind::Cast { expr, .. } => {
            hal_op(acc, 29);
            hal_expr(expr, acc);
        }
        ExprKind::Assign { lhs, rhs } => {
            hal_op(acc, 30);
            hal_expr(lhs, acc);
            hal_expr(rhs, acc);
        }
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            hal_op(acc, 31);
            hal_expr(condition, acc);
            hal_block(then_branch, acc);
            if let Some(e2) = else_branch {
                hal_expr(e2, acc);
            }
        }
        ExprKind::While {
            condition, body, ..
        } => {
            hal_op(acc, 32);
            hal_expr(condition, acc);
            hal_block(body, acc);
        }
        ExprKind::ForEach { iterable, body, .. } => {
            hal_op(acc, 33);
            hal_expr(iterable, acc);
            hal_block(body, acc);
        }
        ExprKind::Loop { body, .. } => {
            hal_op(acc, 34);
            hal_block(body, acc);
        }
        ExprKind::Match { scrutinee, arms } => {
            hal_op(acc, 35);
            hal_expr(scrutinee, acc);
            for arm in arms {
                hal_expr(&arm.body, acc);
            }
        }
        ExprKind::Return { value } => {
            hal_op(acc, 36);
            if let Some(v) = value {
                hal_expr(v, acc);
            }
        }
        ExprKind::Question { expr } => {
            hal_op(acc, 37);
            hal_expr(expr, acc);
        }
        ExprKind::Break { value } => {
            hal_op(acc, 38);
            if let Some(v) = value {
                hal_expr(v, acc);
            }
        }
        ExprKind::Continue => hal_op(acc, 39),
        ExprKind::Block(b) => hal_block(b, acc),
        ExprKind::StructLiteral { fields, .. } => {
            for (_, v) in fields {
                hal_expr(v, acc);
            }
        }
        ExprKind::EnumConstruct { fields, .. } => {
            for v in fields {
                hal_expr(v, acc);
            }
        }
        ExprKind::Tuple(elems) => {
            for v in elems {
                hal_expr(v, acc);
            }
        }
        // `&x` is transparent in the self-hosted AST: recurse, count nothing.
        ExprKind::Borrow { expr } => hal_expr(expr, acc),
        ExprKind::Result => {}
    }
}

// ---- Contract + verification-difficulty metrics (mirror complexity.vow) ----
struct Contract {
    requires: i64,
    ensures: i64,
    invariants: i64,
    predicate_nodes: i64,
    predicate_depth: i64,
    free_vars: i64,
    has_vec_quant: bool,
}

fn pred_walk(
    e: &Expr,
    depth: i64,
    nodes: &mut i64,
    maxdepth: &mut i64,
    has_index: &mut bool,
    collect_free_vars: bool,
    seen: &mut HashSet<String>,
) {
    // Borrow has no node in the self-hosted AST: walk through it transparently.
    if let ExprKind::Borrow { expr } = &e.kind {
        pred_walk(
            expr,
            depth,
            nodes,
            maxdepth,
            has_index,
            collect_free_vars,
            seen,
        );
        return;
    }
    *nodes += 1;
    if depth > *maxdepth {
        *maxdepth = depth;
    }
    match &e.kind {
        ExprKind::Ident(name) => {
            // `result` is the ensures-result binding; the self-hosted parser
            // models it as EXPR_RESULT (not an identifier), so it is not a free
            // var. It is reserved, so excluding it here is always correct.
            if collect_free_vars && name != "result" {
                seen.insert(name.clone());
            }
        }
        ExprKind::Index { base, index } => {
            *has_index = true;
            pred_walk(
                base,
                depth + 1,
                nodes,
                maxdepth,
                has_index,
                collect_free_vars,
                seen,
            );
            pred_walk(
                index,
                depth + 1,
                nodes,
                maxdepth,
                has_index,
                collect_free_vars,
                seen,
            );
        }
        ExprKind::BinaryOp { lhs, rhs, .. } => {
            pred_walk(
                lhs,
                depth + 1,
                nodes,
                maxdepth,
                has_index,
                collect_free_vars,
                seen,
            );
            pred_walk(
                rhs,
                depth + 1,
                nodes,
                maxdepth,
                has_index,
                collect_free_vars,
                seen,
            );
        }
        ExprKind::Assign { lhs, rhs } => {
            pred_walk(
                lhs,
                depth + 1,
                nodes,
                maxdepth,
                has_index,
                collect_free_vars,
                seen,
            );
            pred_walk(
                rhs,
                depth + 1,
                nodes,
                maxdepth,
                has_index,
                collect_free_vars,
                seen,
            );
        }
        ExprKind::UnaryOp { operand, .. } => pred_walk(
            operand,
            depth + 1,
            nodes,
            maxdepth,
            has_index,
            collect_free_vars,
            seen,
        ),
        ExprKind::FieldAccess { base, .. } => pred_walk(
            base,
            depth + 1,
            nodes,
            maxdepth,
            has_index,
            collect_free_vars,
            seen,
        ),
        ExprKind::Question { expr } => pred_walk(
            expr,
            depth + 1,
            nodes,
            maxdepth,
            has_index,
            collect_free_vars,
            seen,
        ),
        ExprKind::Cast { expr, .. } => pred_walk(
            expr,
            depth + 1,
            nodes,
            maxdepth,
            has_index,
            collect_free_vars,
            seen,
        ),
        ExprKind::Call { callee, args } => {
            pred_walk(callee, depth + 1, nodes, maxdepth, has_index, false, seen);
            for a in args {
                pred_walk(
                    a,
                    depth + 1,
                    nodes,
                    maxdepth,
                    has_index,
                    collect_free_vars,
                    seen,
                );
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            pred_walk(
                receiver,
                depth + 1,
                nodes,
                maxdepth,
                has_index,
                collect_free_vars,
                seen,
            );
            for a in args {
                pred_walk(
                    a,
                    depth + 1,
                    nodes,
                    maxdepth,
                    has_index,
                    collect_free_vars,
                    seen,
                );
            }
        }
        ExprKind::Tuple(elems) => {
            for e2 in elems {
                pred_walk(
                    e2,
                    depth + 1,
                    nodes,
                    maxdepth,
                    has_index,
                    collect_free_vars,
                    seen,
                );
            }
        }
        _ => {}
    }
}

fn analyze_contract(f: &FnDef) -> Contract {
    let (mut requires, mut ensures, mut invariants) = (0i64, 0i64, 0i64);
    let mut nodes = 0i64;
    let mut maxdepth = 0i64;
    let mut has_index = false;
    let mut seen: HashSet<String> = HashSet::new();
    if let Some(vb) = &f.vow {
        for clause in &vb.clauses {
            let expr = match clause {
                VowClause::Requires { expr, .. } => {
                    requires += 1;
                    expr
                }
                VowClause::Ensures { expr, .. } => {
                    ensures += 1;
                    expr
                }
                VowClause::Invariant { expr, .. } => {
                    invariants += 1;
                    expr
                }
            };
            pred_walk(
                expr,
                1,
                &mut nodes,
                &mut maxdepth,
                &mut has_index,
                true,
                &mut seen,
            );
        }
    }
    Contract {
        requires,
        ensures,
        invariants,
        predicate_nodes: nodes,
        predicate_depth: maxdepth,
        free_vars: seen.len() as i64,
        has_vec_quant: has_index,
    }
}

fn contract_predicate_cost(ct: &Contract) -> i64 {
    ct.predicate_nodes + ct.free_vars + if ct.has_vec_quant { 1 } else { 0 }
}

struct Verif {
    loops_total: i64,
    loops_without_invariant: i64,
    max_loop_nesting: i64,
    contract_predicate_cost: i64,
}

fn loop_no_inv(vow: &Option<VowBlock>) -> bool {
    // A loop counts as lacking an invariant unless its vow block carries an
    // actual `invariant:` clause — a loop vow block with only requires/ensures
    // still forces the verifier to unwind blind.
    match vow {
        None => true,
        Some(vb) => !vb
            .clauses
            .iter()
            .any(|c| matches!(c, VowClause::Invariant { .. })),
    }
}

fn loops_block(b: &Block, nesting: i64, total: &mut i64, without: &mut i64, maxnest: &mut i64) {
    for s in &b.stmts {
        match s {
            Stmt::Let { init, .. } => loops_expr(init, nesting, total, without, maxnest),
            Stmt::Expr { expr, .. } => loops_expr(expr, nesting, total, without, maxnest),
        }
    }
    if let Some(t) = &b.trailing_expr {
        loops_expr(t, nesting, total, without, maxnest);
    }
}

fn loops_expr(e: &Expr, nesting: i64, total: &mut i64, without: &mut i64, maxnest: &mut i64) {
    match &e.kind {
        ExprKind::While {
            condition,
            vow,
            body,
        } => {
            *total += 1;
            if nesting + 1 > *maxnest {
                *maxnest = nesting + 1;
            }
            if loop_no_inv(vow) {
                *without += 1;
            }
            loops_expr(condition, nesting, total, without, maxnest);
            loops_block(body, nesting + 1, total, without, maxnest);
        }
        ExprKind::ForEach {
            iterable,
            vow,
            body,
            ..
        } => {
            *total += 1;
            if nesting + 1 > *maxnest {
                *maxnest = nesting + 1;
            }
            if loop_no_inv(vow) {
                *without += 1;
            }
            loops_expr(iterable, nesting, total, without, maxnest);
            loops_block(body, nesting + 1, total, without, maxnest);
        }
        ExprKind::Loop { vow, body } => {
            *total += 1;
            if nesting + 1 > *maxnest {
                *maxnest = nesting + 1;
            }
            if loop_no_inv(vow) {
                *without += 1;
            }
            loops_block(body, nesting + 1, total, without, maxnest);
        }
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            loops_expr(condition, nesting, total, without, maxnest);
            loops_block(then_branch, nesting, total, without, maxnest);
            if let Some(e2) = else_branch {
                loops_expr(e2, nesting, total, without, maxnest);
            }
        }
        ExprKind::Match { scrutinee, arms } => {
            loops_expr(scrutinee, nesting, total, without, maxnest);
            for arm in arms {
                loops_expr(&arm.body, nesting, total, without, maxnest);
            }
        }
        ExprKind::Block(b) => loops_block(b, nesting, total, without, maxnest),
        ExprKind::BinaryOp { lhs, rhs, .. } => {
            loops_expr(lhs, nesting, total, without, maxnest);
            loops_expr(rhs, nesting, total, without, maxnest);
        }
        ExprKind::Assign { lhs, rhs } => {
            loops_expr(lhs, nesting, total, without, maxnest);
            loops_expr(rhs, nesting, total, without, maxnest);
        }
        ExprKind::Index { base, index } => {
            loops_expr(base, nesting, total, without, maxnest);
            loops_expr(index, nesting, total, without, maxnest);
        }
        ExprKind::UnaryOp { operand, .. } => loops_expr(operand, nesting, total, without, maxnest),
        ExprKind::FieldAccess { base, .. } => loops_expr(base, nesting, total, without, maxnest),
        ExprKind::Question { expr } => loops_expr(expr, nesting, total, without, maxnest),
        ExprKind::Cast { expr, .. } => loops_expr(expr, nesting, total, without, maxnest),
        ExprKind::Borrow { expr } => loops_expr(expr, nesting, total, without, maxnest),
        ExprKind::Return { value } => {
            if let Some(v) = value {
                loops_expr(v, nesting, total, without, maxnest);
            }
        }
        ExprKind::Break { value } => {
            if let Some(v) = value {
                loops_expr(v, nesting, total, without, maxnest);
            }
        }
        ExprKind::Call { callee, args } => {
            loops_expr(callee, nesting, total, without, maxnest);
            for a in args {
                loops_expr(a, nesting, total, without, maxnest);
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            loops_expr(receiver, nesting, total, without, maxnest);
            for a in args {
                loops_expr(a, nesting, total, without, maxnest);
            }
        }
        ExprKind::StructLiteral { fields, .. } => {
            for (_, v) in fields {
                loops_expr(v, nesting, total, without, maxnest);
            }
        }
        ExprKind::EnumConstruct { fields, .. } => {
            for v in fields {
                loops_expr(v, nesting, total, without, maxnest);
            }
        }
        ExprKind::Tuple(elems) => {
            for v in elems {
                loops_expr(v, nesting, total, without, maxnest);
            }
        }
        ExprKind::Lit(_) | ExprKind::Ident(_) | ExprKind::Continue | ExprKind::Result => {}
    }
}

fn analyze_verif(f: &FnDef, predicate_cost: i64) -> Verif {
    let mut total = 0i64;
    let mut without = 0i64;
    let mut maxnest = 0i64;
    loops_block(&f.body, 0, &mut total, &mut without, &mut maxnest);
    Verif {
        loops_total: total,
        loops_without_invariant: without,
        max_loop_nesting: maxnest,
        contract_predicate_cost: predicate_cost,
    }
}

fn read_complexity_source(source: &Path) -> Result<String, String> {
    std::fs::read_to_string(source).map_err(|_| format!("cannot read {}", source.to_string_lossy()))
}

// Experimental Vow-surface score bump (§3.2a Step 3), fixed-point, capped 150.
fn cx_vow_bump(effect_breadth: i64, linear_consumes: i64, contract_predicate_cost: i64) -> i64 {
    let excess_eff = (effect_breadth - 2).max(0);
    let over_budget = (contract_predicate_cost - 20).max(0);
    let v = 50 * excess_eff + 30 * linear_consumes + 20 * over_budget;
    v.min(150)
}

// 1-based line number of a byte offset. Must match the self-hosted
// `diag_compute_line` exactly (count `\n` bytes before `offset`).
fn line_at(src: &str, offset: usize) -> i64 {
    let bytes = src.as_bytes();
    let mut line: i64 = 1;
    let mut i: usize = 0;
    while i < offset && i < bytes.len() {
        if bytes[i] == 10 {
            line += 1;
        }
        i += 1;
    }
    line
}

fn cx_json_escape(s: &str) -> String {
    // Iterate chars, not bytes: `byte as char` would map a UTF-8 lead byte like
    // 0xC3 to U+00C3 and re-encode it as two bytes (mojibake), diverging from the
    // self-hosted escaper, which appends the original UTF-8 bytes. Pushing the
    // char preserves those bytes and stays byte-identical for non-ASCII paths.
    let mut r = String::new();
    for c in s.chars() {
        match c {
            '"' => r.push_str("\\\""),
            '\\' => r.push_str("\\\\"),
            '\n' => r.push_str("\\n"),
            '\r' => r.push_str("\\r"),
            '\t' => r.push_str("\\t"),
            c if (c as u32) < 32 => r.push('?'),
            c => r.push(c),
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn read_complexity_source_reports_missing_path() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("missing.vow");

        let err = read_complexity_source(&path).unwrap_err();

        assert_eq!(err, format!("cannot read {}", path.to_string_lossy()));
    }

    #[test]
    fn read_complexity_source_allows_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("empty.vow");
        std::fs::write(&path, "").unwrap();

        let src = read_complexity_source(&path).unwrap();

        assert_eq!(src, "");
    }
}
