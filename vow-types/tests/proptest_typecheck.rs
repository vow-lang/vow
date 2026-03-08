// Property-based tests for the Vow type checker.
//
// These tests verify that:
// 1. The type checker never panics on syntactically valid programs.
// 2. Well-typed programs remain well-typed after roundtripping through print/parse.
// 3. Type errors are deterministic — the same input always produces the same errors.

use proptest::prelude::*;
use vow_diag::{Diagnostic, DiagnosticEmitter};
use vow_syntax::parser::parse_module;
use vow_syntax::printer::print_module;
use vow_types::check::Checker;

// ---------------------------------------------------------------------------
// Test infrastructure
// ---------------------------------------------------------------------------

/// Collects diagnostics into a Vec for inspection.
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

/// Type-check a source string and return the diagnostics.
fn typecheck_source(src: &str) -> Vec<Diagnostic> {
    let (module, parse_diags) = parse_module(src, "<proptest>");
    if !parse_diags.is_empty() {
        return parse_diags;
    }
    let mut emitter = CollectingEmitter::new();
    {
        let mut checker = Checker::new("<proptest>", &mut emitter);
        checker.check_module(&module);
    }
    emitter.diagnostics
}

// ---------------------------------------------------------------------------
// Well-typed program generators
// ---------------------------------------------------------------------------

/// Generate a simple well-typed Vow program.
fn arb_welltyped_program() -> impl Strategy<Value = String> {
    // Generate programs with varying structure but known-good typing
    let arb_int_body = (0i64..1000).prop_map(|n| format!("{}", n));
    let arb_arith = (0i64..100, 1i64..100, prop::sample::select(&["+", "-", "*"]))
        .prop_map(|(a, b, op)| format!("{} {} {}", a, op, b));

    let arb_return_expr = prop_oneof![
        3 => arb_int_body,
        1 => arb_arith,
    ];

    let arb_fn_name = prop::sample::select(&[
        "foo",
        "bar",
        "baz",
        "compute",
        "calc",
        "process",
        "transform",
        "eval",
    ]);

    // Generate a function with i64 params and i64 return
    let arb_param_count = 0usize..=3;
    let param_names = ["a", "b", "c", "d"];

    (
        arb_fn_name,
        arb_param_count,
        arb_return_expr,
        prop::bool::ANY,
    )
        .prop_map(move |(name, param_count, body, has_vow)| {
            let params: Vec<String> = (0..param_count)
                .map(|i| format!("{}: i64", param_names[i]))
                .collect();
            let params_str = params.join(", ");

            let vow_block = if has_vow && param_count > 0 {
                // Simple requires clause using the first param
                format!(" vow {{\n    requires: {} > 0\n}}", param_names[0])
            } else {
                String::new()
            };

            // Build a small body with a let binding and the return expression
            let let_binding = if param_count > 0 {
                format!("    let tmp: i64 = {};\n", param_names[0])
            } else {
                String::new()
            };

            format!(
                "module Test\n\nfn {}({}) -> i64{} {{\n{}    {}\n}}\n",
                name, params_str, vow_block, let_binding, body
            )
        })
}

/// Generate a program with a struct definition and a function that uses it.
fn arb_struct_program() -> impl Strategy<Value = String> {
    let field_count = 1usize..=3;
    let field_names = ["x", "y", "z"];

    field_count.prop_map(move |count| {
        let fields: Vec<String> = (0..count)
            .map(|i| format!("    {}: i64,", field_names[i]))
            .collect();
        let field_inits: Vec<String> = (0..count)
            .map(|i| format!("{}: {}", field_names[i], i + 1))
            .collect();

        format!(
            "module Test\n\nstruct Point {{\n{}\n}}\n\nfn make() -> i64 {{\n    let p = Point {{ {} }};\n    0\n}}\n",
            fields.join("\n"),
            field_inits.join(", ")
        )
    })
}

/// Generate a program with if-else control flow.
fn arb_if_program() -> impl Strategy<Value = String> {
    (0i64..100, 0i64..100, 0i64..100).prop_map(|(threshold, then_val, else_val)| {
        format!(
            "module Test\n\nfn choose(x: i64) -> i64 {{\n    if x > {} {{\n        {}\n    }} else {{\n        {}\n    }}\n}}\n",
            threshold, then_val, else_val
        )
    })
}

/// Generate a program with a while loop and optional invariant.
fn arb_loop_program() -> impl Strategy<Value = String> {
    (1i64..20, prop::bool::ANY).prop_map(|(limit, has_invariant)| {
        let inv = if has_invariant {
            " vow {\n        invariant: cnt >= 0\n    }"
        } else {
            ""
        };
        format!(
            "module Test\n\nfn count() -> i64 {{\n    let mut cnt: i64 = 0;\n    while cnt < {}{} {{\n        cnt = cnt + 1;\n    }}\n    cnt\n}}\n",
            limit, inv
        )
    })
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// The type checker must never panic on any syntactically valid program.
    /// It may produce type errors, but must not crash.
    #[test]
    fn typecheck_never_panics(src in arb_welltyped_program()) {
        let _ = typecheck_source(&src);
        // Just asserting it doesn't panic
    }

    /// Well-typed programs should remain well-typed after print→parse roundtrip.
    /// This verifies that the printer preserves semantic meaning.
    #[test]
    fn welltyped_roundtrip_preserves_typing(src in arb_welltyped_program()) {
        let diags1 = typecheck_source(&src);

        // Parse and reprint
        let (module, parse_diags) = parse_module(&src, "<test>");
        prop_assume!(parse_diags.is_empty());

        let reprinted = print_module(&module);
        let diags2 = typecheck_source(&reprinted);

        // Same number of errors (type-checking is deterministic on equivalent input)
        prop_assert_eq!(
            diags1.len(),
            diags2.len(),
            "Type error count changed after roundtrip.\nOriginal ({} errors):\n{}\nReprinted ({} errors):\n{}",
            diags1.len(), src,
            diags2.len(), reprinted
        );
    }

    /// Type checking is deterministic: the same source always produces the same diagnostics.
    #[test]
    fn typecheck_deterministic(src in arb_welltyped_program()) {
        let diags1 = typecheck_source(&src);
        let diags2 = typecheck_source(&src);

        prop_assert_eq!(
            diags1.len(),
            diags2.len(),
            "Type checker is non-deterministic on:\n{}",
            src
        );

        for (d1, d2) in diags1.iter().zip(diags2.iter()) {
            prop_assert_eq!(
                &d1.message, &d2.message,
                "Different error messages on same input:\n{}",
                src
            );
        }
    }

    /// Struct programs should type-check without panics.
    #[test]
    fn struct_program_typechecks(src in arb_struct_program()) {
        let _ = typecheck_source(&src);
    }

    /// If-else programs should type-check without panics.
    #[test]
    fn if_program_typechecks(src in arb_if_program()) {
        let _ = typecheck_source(&src);
    }

    /// Loop programs should type-check without panics.
    #[test]
    fn loop_program_typechecks(src in arb_loop_program()) {
        let _ = typecheck_source(&src);
    }
}

// ---------------------------------------------------------------------------
// Property: error messages are informative
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Type errors should always have non-empty messages.
    #[test]
    fn type_errors_have_messages(src in arb_welltyped_program()) {
        let diags = typecheck_source(&src);
        for diag in &diags {
            prop_assert!(
                !diag.message.is_empty(),
                "Empty error message on:\n{}",
                src
            );
        }
    }
}
