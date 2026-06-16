//! `vow complexity` — per-function complexity metrics emitted as JSON.
//!
//! The JSON must be **byte-identical** to the self-hosted compiler's
//! (`compiler/complexity.vow` + `compiler/complexity_main.vow`). To preserve
//! that, this module hand-builds the JSON string with integer-only arithmetic
//! and never uses `serde_json` or native float formatting.

use std::path::Path;

use vow_syntax::ast::Item;

use crate::emit_frontend_diagnostics;
use crate::frontend::{prepare_frontend, FrontendGoal};

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
            out.push_str("{\"name\":\"");
            out.push_str(&cx_json_escape(&f.name));
            out.push_str("\",\"line\":");
            out.push_str(&first_line.to_string());
            out.push_str(",\"size\":{\"nloc\":");
            out.push_str(&nloc.to_string());
            out.push_str(",\"params\":");
            out.push_str(&(f.params.len() as i64).to_string());
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
