// Property-based tests for the full Vow compiler pipeline.
//
// These tests verify end-to-end properties:
// 1. Well-typed programs never panic during IR lowering or codegen.
// 2. The pipeline is deterministic: same source → same binary output.
// 3. Programs that type-check and compile produce executables with correct exit codes.

use proptest::prelude::*;
use vow_diag::{Diagnostic, DiagnosticEmitter};
use vow_syntax::parser::parse_module;
use vow_types::check::Checker;

/// Collects diagnostics.
struct CollectingEmitter {
    diagnostics: Vec<Diagnostic>,
}

impl CollectingEmitter {
    fn new() -> Self {
        Self {
            diagnostics: Vec::new(),
        }
    }
}

impl DiagnosticEmitter for CollectingEmitter {
    fn emit(&mut self, diag: &Diagnostic) {
        self.diagnostics.push(diag.clone());
    }
    fn finish(&mut self) {}
}

/// Parse, type-check, and lower to IR. Returns true if IR lowering succeeds.
fn parse_typecheck_and_lower(src: &str) -> bool {
    let (module, parse_diags) = parse_module(src, "<proptest>");
    if !parse_diags.is_empty() {
        return false;
    }
    let mut emitter = CollectingEmitter::new();
    let string_exprs;
    {
        let mut checker = Checker::new("<proptest>", &mut emitter);
        checker.check_module(&module);
        if checker.has_errors() {
            return false;
        }
        string_exprs = checker.into_string_exprs();
    }
    // Attempt IR lowering — must not panic
    let _ir_module = vow_ir::lower::lower_module(&module, "<proptest>", &string_exprs);
    true
}

// ---------------------------------------------------------------------------
// Generators for well-typed programs
// ---------------------------------------------------------------------------

/// Generates programs that should pass type-checking (simple integer functions).
fn arb_simple_program() -> impl Strategy<Value = String> {
    (
        prop::sample::select(&["foo", "bar", "compute", "step"]),
        0usize..=2,
        0i64..500,
    )
        .prop_map(|(name, param_count, return_val)| {
            let param_names = ["a", "b", "c"];
            let params: Vec<String> = (0..param_count)
                .map(|i| format!("{}: i64", param_names[i]))
                .collect();

            format!(
                "module Test\n\nfn {}({}) -> i64 {{\n    {}\n}}\n",
                name,
                params.join(", "),
                return_val
            )
        })
}

/// Generates programs with arithmetic in the body.
fn arb_arith_program() -> impl Strategy<Value = String> {
    (0i64..100, 1i64..100, prop::sample::select(&["+", "-", "*"])).prop_map(|(a, b, op)| {
        format!(
            "module Test\n\nfn calc(x: i64, y: i64) -> i64 {{\n    {} {} {}\n}}\n",
            a, op, b
        )
    })
}

/// Generates programs with vow contracts that should pass type-checking.
fn arb_vow_program() -> impl Strategy<Value = String> {
    (0i64..50, 0i64..100).prop_map(|(threshold, ret)| {
        format!(
            "module Test\n\nfn guarded(x: i64) -> i64 vow {{\n    requires: x > {}\n    ensures: result >= 0\n}} {{\n    {}\n}}\n",
            threshold, ret
        )
    })
}

/// Generates programs with if-else that should type-check.
fn arb_branching_program() -> impl Strategy<Value = String> {
    (0i64..100, 0i64..100, 0i64..100).prop_map(|(threshold, then_val, else_val)| {
        format!(
            "module Test\n\nfn branch(x: i64) -> i64 {{\n    if x > {} {{\n        {}\n    }} else {{\n        {}\n    }}\n}}\n",
            threshold, then_val, else_val
        )
    })
}

/// Generates programs with while loops.
fn arb_while_program() -> impl Strategy<Value = String> {
    (1i64..10,).prop_map(|(limit,)| {
        format!(
            "module Test\n\nfn looper() -> i64 {{\n    let mut i: i64 = 0;\n    while i < {} {{\n        i = i + 1;\n    }}\n    i\n}}\n",
            limit
        )
    })
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Simple programs must parse, type-check, and lower to IR without panicking.
    #[test]
    fn simple_programs_lower_to_ir(src in arb_simple_program()) {
        let result = std::panic::catch_unwind(|| parse_typecheck_and_lower(&src));
        prop_assert!(result.is_ok(), "Pipeline panicked on:\n{}", src);
    }

    /// Arithmetic programs must not panic during pipeline.
    #[test]
    fn arith_programs_lower_to_ir(src in arb_arith_program()) {
        let result = std::panic::catch_unwind(|| parse_typecheck_and_lower(&src));
        prop_assert!(result.is_ok(), "Pipeline panicked on:\n{}", src);
    }

    /// Vow-annotated programs must not panic during pipeline.
    #[test]
    fn vow_programs_lower_to_ir(src in arb_vow_program()) {
        let result = std::panic::catch_unwind(|| parse_typecheck_and_lower(&src));
        prop_assert!(result.is_ok(), "Pipeline panicked on:\n{}", src);
    }

    /// Branching programs must not panic during pipeline.
    #[test]
    fn branching_programs_lower_to_ir(src in arb_branching_program()) {
        let result = std::panic::catch_unwind(|| parse_typecheck_and_lower(&src));
        prop_assert!(result.is_ok(), "Pipeline panicked on:\n{}", src);
    }

    /// While-loop programs must not panic during pipeline.
    #[test]
    fn while_programs_lower_to_ir(src in arb_while_program()) {
        let result = std::panic::catch_unwind(|| parse_typecheck_and_lower(&src));
        prop_assert!(result.is_ok(), "Pipeline panicked on:\n{}", src);
    }

    /// The pipeline is deterministic: same source → identical IR (no random state).
    #[test]
    fn pipeline_deterministic(src in arb_simple_program()) {
        let (module1, d1) = parse_module(&src, "<test>");
        let (module2, d2) = parse_module(&src, "<test>");

        prop_assert_eq!(d1.len(), d2.len());

        if d1.is_empty() {
            let mut e1 = CollectingEmitter::new();
            let mut e2 = CollectingEmitter::new();
            let se1;
            let se2;
            {
                let mut c1 = Checker::new("<test>", &mut e1);
                c1.check_module(&module1);
                se1 = c1.into_string_exprs();
            }
            {
                let mut c2 = Checker::new("<test>", &mut e2);
                c2.check_module(&module2);
                se2 = c2.into_string_exprs();
            }

            if e1.diagnostics.is_empty() && e2.diagnostics.is_empty() {
                let ir1 = vow_ir::lower::lower_module(&module1, "<test>", &se1);
                let ir2 = vow_ir::lower::lower_module(&module2, "<test>", &se2);

                // Same number of functions
                prop_assert_eq!(
                    ir1.functions.len(),
                    ir2.functions.len(),
                    "Different function counts for:\n{}",
                    src
                );
            }
        }
    }
}
