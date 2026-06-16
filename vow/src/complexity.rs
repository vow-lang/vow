//! `vow complexity` — per-function complexity metrics emitted as JSON.
//!
//! The JSON must be **byte-identical** to the self-hosted compiler's
//! (`compiler/complexity.vow` + `compiler/complexity_main.vow`). To preserve
//! that, this module hand-builds the JSON string with integer-only arithmetic
//! and never uses `serde_json` or native float formatting.

use std::collections::HashMap;
use std::path::Path;

use vow_ir::{InstData, Opcode};
use vow_syntax::ast::{BinOp, Block, Expr, ExprKind, Item, Lit, Stmt, UnOp};

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
use crate::frontend::{prepare_frontend, FrontendGoal};

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
        ExprKind::Lit(_)
        | ExprKind::Ident(_)
        | ExprKind::Break { .. }
        | ExprKind::Continue
        | ExprKind::Result => {}
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
        ExprKind::Lit(_)
        | ExprKind::Ident(_)
        | ExprKind::Break { .. }
        | ExprKind::Continue
        | ExprKind::Result => {}
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
    linear_consumes: i64,
    linear_borrows: i64,
    score: i64,
    cog_sub_s: i64,
    size_sub_s: i64,
    base_s: i64,
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
    out.push_str(",\"vow_bump\":0.000,\"base\":");
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
    out.push_str("}},\"vow\":{\"tier\":\"experimental\",\"linear_consumes\":");
    out.push_str(&r.linear_consumes.to_string());
    out.push_str(",\"linear_borrows\":");
    out.push_str(&r.linear_borrows.to_string());
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

    let src = std::fs::read_to_string(source).unwrap_or_default();
    let file_nloc = line_at(&src, src.len());
    let module = frontend.module();
    let thr = if max_score >= 0 { max_score } else { 80 };

    // IR-derived per-function info, matched to AST functions by name.
    let mut ir_info: HashMap<String, IrInfo> = HashMap::new();
    let mut fan_in_max: i64 = 0;
    let mut fan_out_max: i64 = 0;
    if let Some(m) = frontend.ir() {
        let funcs = &m.functions;
        let callees: Vec<std::collections::HashSet<u32>> = funcs.iter().map(ir_callees).collect();
        for (idx, f) in funcs.iter().enumerate() {
            let fan_out = callees[idx].len() as i64;
            let fid = f.id.0;
            let fan_in = callees.iter().filter(|s| s.contains(&fid)).count() as i64;
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
            fan_in_max = fan_in_max.max(fan_in);
            fan_out_max = fan_out_max.max(fan_out);
            ir_info.insert(
                f.name.clone(),
                IrInfo {
                    cyclomatic: ir_cyclomatic(f),
                    consumes,
                    borrows,
                    fan_in,
                    fan_out,
                },
            );
        }
    }

    // Pass 1: analyze + score every function.
    let mut recs: Vec<CxEmit> = Vec::new();
    for item in &module.items {
        if let Item::Fn(f) = item {
            let start = f.span.start as usize;
            let end = (f.span.start + f.span.len) as usize;
            let first_line = line_at(&src, start);
            let nloc = line_at(&src, end) - first_line + 1;
            let mut acc = Acc::default();
            walk_block(&f.body, &mut acc);
            let cyclomatic = acc.decisions + 1;
            let info = ir_info.get(&f.name);
            let cyclomatic_ir = info.map(|x| x.cyclomatic).unwrap_or(-1);
            let linear_consumes = info.map(|x| x.consumes).unwrap_or(0);
            let linear_borrows = info.map(|x| x.borrows).unwrap_or(0);
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
            let h_effort_s = h_difficulty_s.wrapping_mul(h_volume_s) / 1000;
            let sc = cx_score(cognitive, nloc, cog_anchor, nloc_anchor);
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
                linear_consumes,
                linear_borrows,
                score: sc.score,
                cog_sub_s: sc.cog_sub_s,
                size_sub_s: sc.size_sub_s,
                base_s: sc.base_s,
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

    // Module-level Henry-Kafura max (needs AST nloc + IR fan-in/out by name).
    let mut hk_max: i64 = 0;
    for r in &recs {
        if let Some(info) = ir_info.get(&r.name) {
            hk_max = hk_max.max(cx_henry_kafura(r.nloc, info.fan_in, info.fan_out));
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
    if r > 100 {
        100
    } else {
        r
    }
}

fn cx_score(cognitive: i64, nloc: i64, cog_anchor: i64, nloc_anchor: i64) -> CxScore {
    let c = cx_anchor_map(cognitive, cog_anchor);
    let s = cx_anchor_map(nloc, nloc_anchor);
    let base = cx_soft_or(c, s);
    let score = cx_round_0_100(base);
    CxScore {
        score,
        cog_sub_s: c,
        size_sub_s: s,
        base_s: base,
    }
}

// ---- Fixed-point integer math (byte-identical with compiler/complexity.vow) ----
const CX_SAT: i64 = 2_000_000_000;

fn cx_sat(x: i64) -> i64 {
    if x > CX_SAT {
        CX_SAT
    } else {
        x
    }
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
        ExprKind::Break { .. } => hal_op(acc, 38),
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
    let mut r = String::new();
    for b in s.bytes() {
        match b {
            34 => r.push_str("\\\""),
            92 => r.push_str("\\\\"),
            10 => r.push_str("\\n"),
            13 => r.push_str("\\r"),
            9 => r.push_str("\\t"),
            x if x < 32 => r.push('?'),
            x => r.push(x as char),
        }
    }
    r
}
