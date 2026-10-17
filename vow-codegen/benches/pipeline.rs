//! Compiler pipeline benchmarks: lex, parse, type-check, lower to IR, and
//! Cranelift code generation.
//!
//! Inputs are real Vow sources checked into the repository:
//! - the self-contained `benchmarks/hard/*/reference.vow` programs drive the
//!   full single-file pipeline (they need no module loading);
//! - large files of the self-hosted compiler under `compiler/` drive the
//!   lexer and parser on realistic, multi-thousand-line inputs.

use divan::{Bencher, black_box};
use vow_codegen::cranelift_backend::CraneliftBackend;
use vow_codegen::{Backend, BuildMode, TraceMode};
use vow_diag::{Diagnostic, DiagnosticEmitter};
use vow_syntax::ast::Module;
use vow_syntax::lexer::Lexer;
use vow_syntax::parser::parse_module;
use vow_types::check::Checker;

fn main() {
    divan::main();
}

const FILE: &str = "<bench>";

/// Self-contained programs that pass the full frontend on their own.
const PROGRAMS: &[(&str, &str)] = &[
    (
        "stack_push_pop",
        include_str!("../../benchmarks/hard/H01_stack_push_pop/reference.vow"),
    ),
    (
        "bounded_queue",
        include_str!("../../benchmarks/hard/H03_bounded_queue/reference.vow"),
    ),
    (
        "matrix_ops",
        include_str!("../../benchmarks/hard/H06_matrix_ops/reference.vow"),
    ),
    (
        "ring_buffer",
        include_str!("../../benchmarks/hard/H07_ring_buffer/reference.vow"),
    ),
    (
        "interval_overlap",
        include_str!("../../benchmarks/hard/H08_interval_overlap/reference.vow"),
    ),
    (
        "expression_eval",
        include_str!("../../benchmarks/hard/H10_expression_eval/reference.vow"),
    ),
];

/// Large sources from the self-hosted compiler (syntax-level benchmarks only:
/// they import sibling modules, so they are not type-checked here).
const COMPILER_SOURCES: &[(&str, &str)] = &[
    ("lexer", include_str!("../../compiler/lexer.vow")),
    ("parser", include_str!("../../compiler/parser.vow")),
    ("checker", include_str!("../../compiler/checker.vow")),
    ("lower", include_str!("../../compiler/lower.vow")),
];

const PROGRAM_NAMES: &[&str] = &[
    "stack_push_pop",
    "bounded_queue",
    "matrix_ops",
    "ring_buffer",
    "interval_overlap",
    "expression_eval",
];

const COMPILER_NAMES: &[&str] = &["lexer", "parser", "checker", "lower"];

fn lookup(table: &[(&str, &'static str)], name: &str) -> &'static str {
    table
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, src)| *src)
        .unwrap_or_else(|| panic!("unknown bench input {name}"))
}

struct CollectingEmitter {
    diagnostics: Vec<Diagnostic>,
}

impl DiagnosticEmitter for CollectingEmitter {
    fn try_emit(&mut self, diag: &Diagnostic) -> std::io::Result<()> {
        self.diagnostics.push(diag.clone());
        Ok(())
    }

    fn try_finish(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn parse(src: &str) -> Module {
    let (module, diags) = parse_module(src, FILE);
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    module
}

/// Type-check then lower a parsed module to IR.
fn check_and_lower(module: &Module) -> vow_ir::Module {
    let item_files = vec![FILE.to_string(); module.items.len()];
    let mut emitter = CollectingEmitter {
        diagnostics: Vec::new(),
    };
    let (string_exprs, pattern_aggregates, payload_scalars) = {
        let mut checker = Checker::new(FILE, &mut emitter);
        checker.check_module(module, &item_files);
        assert!(!checker.has_errors(), "type errors");
        checker.into_lowering_metadata()
    };
    vow_ir::lower::lower_module_with_pattern_aggregates(
        module,
        &item_files,
        &string_exprs,
        pattern_aggregates,
        payload_scalars,
    )
}

fn typecheck(module: &Module) -> bool {
    let item_files = vec![FILE.to_string(); module.items.len()];
    let mut emitter = CollectingEmitter {
        diagnostics: Vec::new(),
    };
    let mut checker = Checker::new(FILE, &mut emitter);
    checker.check_module(module, &item_files);
    checker.has_errors()
}

mod syntax {
    use super::*;

    #[divan::bench(args = COMPILER_NAMES)]
    fn lex(bencher: Bencher, name: &str) {
        let src = lookup(COMPILER_SOURCES, name);
        bencher.bench(|| Lexer::new(black_box(src)).tokenize().unwrap());
    }

    #[divan::bench(args = COMPILER_NAMES)]
    fn parse(bencher: Bencher, name: &str) {
        let src = lookup(COMPILER_SOURCES, name);
        bencher.bench(|| parse_module(black_box(src), FILE));
    }
}

mod frontend {
    use super::*;

    #[divan::bench(args = PROGRAM_NAMES)]
    fn typecheck(bencher: Bencher, name: &str) {
        let module = parse(lookup(PROGRAMS, name));
        bencher.bench(|| super::typecheck(black_box(&module)));
    }

    #[divan::bench(args = PROGRAM_NAMES)]
    fn check_and_lower(bencher: Bencher, name: &str) {
        let module = parse(lookup(PROGRAMS, name));
        bencher.bench(|| super::check_and_lower(black_box(&module)));
    }
}

mod codegen {
    use super::*;

    #[divan::bench(args = PROGRAM_NAMES)]
    fn cranelift_debug(bencher: Bencher, name: &str) {
        let ir = check_and_lower(&parse(lookup(PROGRAMS, name)));
        let backend = CraneliftBackend::new();
        bencher.bench(|| {
            backend
                .compile_module(black_box(&ir), BuildMode::Debug, TraceMode::Off)
                .unwrap()
        });
    }

    #[divan::bench(args = PROGRAM_NAMES)]
    fn cranelift_release(bencher: Bencher, name: &str) {
        let ir = check_and_lower(&parse(lookup(PROGRAMS, name)));
        let backend = CraneliftBackend::new();
        bencher.bench(|| {
            backend
                .compile_module(black_box(&ir), BuildMode::Release, TraceMode::Off)
                .unwrap()
        });
    }
}

/// Whole single-file pipeline: source text to object bytes.
#[divan::bench(args = PROGRAM_NAMES)]
fn end_to_end(bencher: Bencher, name: &str) {
    let src = lookup(PROGRAMS, name);
    let backend = CraneliftBackend::new();
    bencher.bench(|| {
        let module = parse(black_box(src));
        let ir = check_and_lower(&module);
        backend
            .compile_module(&ir, BuildMode::Release, TraceMode::Off)
            .unwrap()
    });
}
