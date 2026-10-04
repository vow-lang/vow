//! Parameter `where` refinements are type-checked like `requires` clauses: they
//! resolve names in a scope holding only their own parameter, must be pure
//! `bool` predicates, and may not contain tuple expressions. Before this check
//! an undefined name or a tuple reached IR lowering and panicked.

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

fn typecheck(items: &str) -> Vec<Diagnostic> {
    let src = format!("module Test\n\n{items}\nfn main() -> i32 {{ 0 }}\n");
    let (module, parse_diags) = parse_module(&src, "<test>");
    assert!(
        parse_diags.is_empty(),
        "must parse cleanly: {parse_diags:?}"
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

fn codes(diags: &[Diagnostic]) -> Vec<ErrorCode> {
    diags.iter().map(|d| d.code).collect()
}

#[test]
fn valid_refinements_are_accepted() {
    let diags = typecheck(
        "const LIMIT: i64 = 100;\n\
         fn is_small(n: i64) -> bool { n < 10 }\n\
         fn a(x: i64 where x >= 0 && x <= LIMIT) -> i64 { x }\n\
         fn b(x: i64 where is_small(x)) -> i64 { x }\n\
         fn c(x: i64 where { let t: i64 = x + 1; t > 0 }) -> i64 { x }\n\
         fn d(a: i64 where a > 0, b: i64 where b > 0) -> i64 { a + b }\n",
    );
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn undefined_name_is_a_single_type_error_with_a_suggestion() {
    let diags = typecheck("fn f(x: i64 where y > 0) -> i64 { x }\n");
    assert_eq!(codes(&diags), vec![ErrorCode::TypeMismatch], "{diags:?}");
    assert!(diags[0].message.contains("undefined variable `y`"));
    assert_eq!(diags[0].hints, vec!["did you mean `x`?".to_string()]);
}

#[test]
fn sibling_parameter_is_out_of_scope() {
    let diags = typecheck("fn f(a: i64, b: i64 where b > a) -> i64 { b }\n");
    assert_eq!(codes(&diags), vec![ErrorCode::TypeMismatch], "{diags:?}");
    assert!(diags[0].message.contains("undefined variable `a`"));
    assert!(diags[0].hints[0].contains("only reference its own parameter `b`"));
}

#[test]
fn result_is_out_of_scope() {
    let diags = typecheck("fn f(x: i64 where result > 0) -> i64 { x }\n");
    assert_eq!(codes(&diags), vec![ErrorCode::TypeMismatch], "{diags:?}");
    assert!(diags[0].hints[0].contains("`ensures`"));
}

#[test]
fn non_bool_predicate_is_a_contract_type_mismatch() {
    let diags = typecheck("fn f(x: i64 where x + 1) -> i64 { x }\n");
    assert_eq!(
        codes(&diags),
        vec![ErrorCode::ContractTypeMismatch],
        "{diags:?}"
    );
    assert_eq!(
        diags[0].hints,
        vec!["parameter `where` clauses must evaluate to `bool`".to_string()]
    );
}

#[test]
fn tuple_expression_is_unsupported() {
    let diags = typecheck("fn f(x: i64 where (x, 1) != (2, 3)) -> i64 { x }\n");
    assert!(!diags.is_empty());
    assert!(
        diags
            .iter()
            .all(|d| d.code == ErrorCode::UnsupportedFeature),
        "{diags:?}"
    );
}

#[test]
fn effectful_call_is_rejected() {
    let diags = typecheck(
        "fn noisy(x: i64) -> bool [io] { print_i64(x); true }\n\
         fn f(x: i64 where noisy(x)) -> i64 { x }\n",
    );
    assert_eq!(codes(&diags), vec![ErrorCode::EffectViolation], "{diags:?}");
}

#[test]
fn literal_is_range_checked_against_the_parameter_type() {
    let diags = typecheck("fn f(x: u8 where x < 300) -> u8 { x }\n");
    assert_eq!(
        codes(&diags),
        vec![ErrorCode::LiteralOutOfRange],
        "{diags:?}"
    );
}

#[test]
fn declaration_only_function_refinement_is_checked() {
    let diags = typecheck("fn external(x: i64 where n > 0) -> i64;\n");
    assert_eq!(codes(&diags), vec![ErrorCode::TypeMismatch], "{diags:?}");
}

#[test]
fn extern_function_refinement_is_checked() {
    let diags = typecheck(
        "extern \"C\" {\n    vow {\n        requires: true\n    }\n    fn abs(x: i32 where n > 0) -> i32;\n}\n",
    );
    assert_eq!(codes(&diags), vec![ErrorCode::TypeMismatch], "{diags:?}");
}
