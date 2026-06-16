//! `vow complexity` — per-function complexity metrics emitted as JSON.
//!
//! The JSON must be **byte-identical** to the self-hosted compiler's
//! (`compiler/complexity.vow` + `compiler/complexity_main.vow`). To preserve
//! that, this module hand-builds the JSON string with integer-only arithmetic
//! and never uses `serde_json` or native float formatting.

use std::path::Path;

use vow_syntax::ast::{BinOp, Block, Expr, ExprKind, Item, Stmt};

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

pub(crate) fn run_complexity_command(source: &Path) {
    let frontend = match prepare_frontend(source, FrontendGoal::MergedAst) {
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
    let module = frontend.module();
    let mut out = String::from(
        "{\"schema_version\":\"1\",\"kind\":\"complexity_report\",\"tool\":\"vow\",\"files\":[{\"file\":\"",
    );
    out.push_str(&cx_json_escape(&source.to_string_lossy()));
    out.push_str("\",\"functions\":[");
    let mut count: i64 = 0;
    for item in &module.items {
        if let Item::Fn(f) = item {
            if count > 0 {
                out.push(',');
            }
            let start = f.span.start as usize;
            let end = (f.span.start + f.span.len) as usize;
            let first_line = line_at(&src, start);
            let last_line = line_at(&src, end);
            let nloc = last_line - first_line + 1;
            let mut acc = Acc::default();
            walk_block(&f.body, &mut acc);
            let cyclomatic = acc.decisions + 1;
            out.push_str("{\"name\":\"");
            out.push_str(&cx_json_escape(&f.name));
            out.push_str("\",\"line\":");
            out.push_str(&first_line.to_string());
            out.push_str(",\"size\":{\"nloc\":");
            out.push_str(&nloc.to_string());
            out.push_str(",\"stmts\":");
            out.push_str(&acc.stmts.to_string());
            out.push_str(",\"params\":");
            out.push_str(&(f.params.len() as i64).to_string());
            out.push_str("},\"structural\":{\"cyclomatic\":");
            out.push_str(&cyclomatic.to_string());
            out.push_str("}}");
            count += 1;
        }
    }
    out.push_str("]}],\"summary\":{\"functions\":");
    out.push_str(&count.to_string());
    out.push_str("}}");
    println!("{out}");
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
