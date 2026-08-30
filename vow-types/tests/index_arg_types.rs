//! Enforcement tests for index and builtin-method argument types (#1111).
//!
//! The unit tests in `check.rs` cover the `ArgExpect` seam as a pure function;
//! these drive real source through parse + check so the wiring at the
//! `ExprKind::Index` and `ExprKind::MethodCall` sites is covered too. Without
//! them the enforcement could stop firing while the seam's own tests stayed
//! green — which is exactly the bug #1111 fixed. The `.vow` fixtures in
//! `tests/error/` cover the same ground across both compilers, but no CI job
//! runs them, so these are the only CI-protected regression guard.

use vow_diag::{Diagnostic, DiagnosticEmitter, ErrorCode};
use vow_syntax::parser::parse_module;
use vow_types::check::Checker;

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

fn typecheck_source(src: &str) -> Vec<Diagnostic> {
    let (module, parse_diags) = parse_module(src, "<test>");
    assert!(
        parse_diags.is_empty(),
        "fixture must parse cleanly, got {parse_diags:?}"
    );
    let mut emitter = CollectingEmitter {
        diagnostics: Vec::new(),
    };
    {
        let mut checker = Checker::new("<test>", &mut emitter);
        let item_files = vec!["<test>".to_string(); module.items.len()];
        checker.check_module(&module, &item_files);
    }
    emitter.diagnostics
}

fn type_mismatches(src: &str) -> Vec<Diagnostic> {
    typecheck_source(src)
        .into_iter()
        .filter(|d| d.code == ErrorCode::TypeMismatch)
        .collect()
}

/// Wraps statements in a module so each fixture stays to the point.
fn program(body: &str) -> String {
    format!(
        "module Test\n\nfn main() -> i32 {{\n    let v: Vec<i64> = Vec::new();\n    let s: String = String::from(\"hi\");\n{body}\n    0\n}}\n"
    )
}

#[test]
fn a_non_integer_index_is_rejected() {
    for (label, index_expr) in [
        ("string", "String::from(\"x\")"),
        ("bool", "true"),
        ("unit-typed local", "u"),
    ] {
        let prelude = if label == "unit-typed local" {
            "    let u: bool = false;\n"
        } else {
            ""
        };
        let src = program(&format!("{prelude}    let a: i64 = v[{index_expr}];"));
        let diags = type_mismatches(&src);
        assert_eq!(
            diags.len(),
            1,
            "a {label} index must produce exactly one TypeMismatch, got {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        assert!(
            diags[0].message.contains("must be an integer type"),
            "unexpected message for {label} index: {}",
            diags[0].message
        );
    }
}

#[test]
fn an_index_write_is_rejected_through_the_same_site() {
    // `v[e] = x` routes its LHS through the same `ExprKind::Index` arm, so no
    // second checker location is needed — this pins that.
    let diags = type_mismatches(&program("    v[String::from(\"y\")] = 1;"));
    assert_eq!(
        diags.len(),
        1,
        "an index write with a non-integer index must be rejected once, got {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(diags[0].message.contains("must be an integer type"));
}

#[test]
fn index_shaped_method_arguments_reject_non_integers() {
    for (call, expected_count) in [
        ("let c: i64 = s.byte_at(String::from(\"x\"));", 1),
        ("let d: String = s.substring(String::from(\"a\"), true);", 2),
        ("v.truncate(String::from(\"n\"));", 1),
    ] {
        let diags = type_mismatches(&program(&format!("    {call}")));
        assert_eq!(
            diags.len(),
            expected_count,
            "`{call}` should yield {expected_count} TypeMismatch, got {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        for d in &diags {
            assert!(
                d.message.contains("expects an integer type"),
                "unexpected message: {}",
                d.message
            );
        }
    }
}

#[test]
fn value_shaped_method_arguments_require_the_declared_type() {
    for call in [
        "v.push(String::from(\"boom\"));",
        "s.push_str(42);",
        "s.push_str(true);",
    ] {
        let diags = type_mismatches(&program(&format!("    {call}")));
        assert_eq!(
            diags.len(),
            1,
            "`{call}` should yield exactly one TypeMismatch, got {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        assert!(
            diags[0].message.contains("argument has type"),
            "unexpected message: {}",
            diags[0].message
        );
    }
}

#[test]
fn map_key_and_value_arguments_are_checked() {
    let src = "module Test\n\nfn main() -> i32 {\n    let m: HashMap<i64, i64> = HashMap::new();\n    m.insert(String::from(\"k\"), true);\n    0\n}\n";
    let diags = type_mismatches(src);
    assert_eq!(
        diags.len(),
        2,
        "a bad key and a bad value are two independent violations, got {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn the_index_rule_stays_weak_across_widths_and_signedness() {
    // The #1104 migration needs `i64` and `u64` indices to coexist, so no
    // width or signedness is enforced. A regression that tightened this would
    // break the migration silently, so it is pinned here.
    let src = "module Test\n\nfn main() -> i32 {\n\
        \x20   let v: Vec<i64> = Vec::new();\n\
        \x20   let s: String = String::from(\"hello\");\n\
        \x20   let i: i64 = 0;\n\
        \x20   let u: u64 = 1;\n\
        \x20   let w: u32 = 2;\n\
        \x20   let a: i64 = v[i];\n\
        \x20   let b: i64 = v[u];\n\
        \x20   let c: i64 = v[w];\n\
        \x20   let d: i64 = v[0];\n\
        \x20   let e: i64 = s.byte_at(u);\n\
        \x20   let f: i64 = s.byte_at(0);\n\
        \x20   let g: String = s.substring(u, 3);\n\
        \x20   v[u] = 9;\n\
        \x20   0\n}\n";
    let diags = typecheck_source(src);
    assert!(
        diags.is_empty(),
        "every integer width and signedness must be accepted as an index, got {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn each_violation_produces_exactly_one_diagnostic() {
    // The acceptance repro from #1111: eight independent violations must yield
    // eight diagnostics, never a cascade. Mirrors
    // `tests/error/index_method_arg_types.vow`, which no CI job runs.
    let src = "module Test\n\nfn main() -> i32 {\n\
        \x20   let v: Vec<i64> = Vec::new();\n\
        \x20   let s: String = String::from(\"hi\");\n\
        \x20   let a: i64 = v[String::from(\"x\")];\n\
        \x20   let b: i64 = v[true];\n\
        \x20   v[String::from(\"y\")] = 1;\n\
        \x20   let c: i64 = s.byte_at(String::from(\"x\"));\n\
        \x20   let d: String = s.substring(String::from(\"a\"), true);\n\
        \x20   v.push(String::from(\"boom\"));\n\
        \x20   s.push_str(42);\n\
        \x20   0\n}\n";
    let diags = type_mismatches(src);
    assert_eq!(
        diags.len(),
        8,
        "expected exactly 8 TypeMismatch diagnostics, got {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn a_failed_call_used_as_an_index_does_not_cascade() {
    // An undefined call is bottom, so the index check stays quiet and only the
    // real error is reported. Both compilers agree here.
    let src = "module Test\n\nfn main() -> i32 {\n    let v: Vec<i64> = Vec::new();\n    let x: i64 = v[unknown_fn()];\n    0\n}\n";
    let diags = typecheck_source(src);
    assert_eq!(
        diags.len(),
        1,
        "expected only the undefined-function error, got {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(diags[0].message.contains("undefined function"));
}

#[test]
fn an_unrelated_nested_error_still_reports_the_real_argument_mismatch() {
    // Regression guard: an earlier attempt suppressed the argument check
    // whenever *anything* errored while checking the argument. That swallowed
    // this `bool`-vs-`String` mismatch, because the block contains an unrelated
    // failed call. The argument's type is perfectly determinate, so the
    // mismatch must still be reported.
    let src = "module Test\n\nfn main() -> i32 {\n    let s: String = String::from(\"s\");\n    s.push_str({ missing(); true });\n    0\n}\n";
    let diags = typecheck_source(src);
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("undefined function")),
        "the nested error must still be reported, got {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("argument has type `bool`")),
        "the real argument mismatch must not be swallowed, got {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}
