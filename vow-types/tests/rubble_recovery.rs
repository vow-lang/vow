//! Error-recovery discipline tests (#1130).
//!
//! A failed expression yields `Ty::Unknown`, which every later check absorbs, so
//! one mistake produces one diagnostic. `Ty::Never` is a different thing — the
//! type of `Option::None`, `Vec::new()` and diverging expressions — and must not
//! be absorbed where it would stand in for a scalar.

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

fn program(body: &str) -> String {
    format!(
        "module Test\n\nfn main() -> i32 {{\n    let v: Vec<i64> = Vec::new();\n    let s: String = String::from(\"s\");\n    let c: bool = true;\n{body}\n    0\n}}\n"
    )
}

fn messages(diags: &[Diagnostic]) -> Vec<&str> {
    diags.iter().map(|d| d.message.as_str()).collect()
}

#[test]
fn one_mistake_yields_one_diagnostic() {
    for (label, body) in [
        ("unknown method as index", "    let x: i64 = v[s.nope()];"),
        ("unknown field as index", "    let x: i64 = v[s.nofield];"),
        (
            "undefined function in a branch",
            "    let x: i64 = if c { missing() } else { 5 };",
        ),
        (
            "unknown method in arithmetic",
            "    let x: i64 = s.nope() + 1;",
        ),
        (
            "undefined variable in arithmetic",
            "    let x: i64 = nope + 1;",
        ),
        ("undefined variable negated", "    let x: i64 = -nope;"),
        ("undefined variable as condition", "    if (nope) { }"),
        (
            "undefined variable in comparison",
            "    let b: bool = nope < 3;",
        ),
        ("unknown method then method", "    s.nope().len();"),
        (
            "unknown field then field",
            "    let x: i64 = s.nofield.deeper;",
        ),
        ("unknown method as argument", "    s.push_str(s.nope());"),
        ("undefined variable as index", "    let x: i64 = v[nope];"),
        ("undefined variable in for-each", "    for e in (nope) { }"),
    ] {
        let diags = typecheck_source(&program(body));
        assert_eq!(
            diags.len(),
            1,
            "{label}: expected exactly one diagnostic, got {:?}",
            messages(&diags)
        );
    }
}

#[test]
fn option_none_is_not_an_index_or_a_builtin_argument() {
    for (label, body) in [
        ("index", "    let x: i64 = v[Option::None];"),
        ("index write", "    v[Option::None] = 1;"),
        ("push_str", "    s.push_str(Option::None);"),
        (
            "String::from",
            "    let t: String = String::from(Option::None);",
        ),
        ("byte_at", "    let b: i64 = s.byte_at(Option::None);"),
        ("Vec::new as index", "    let x: i64 = v[Vec::new()];"),
    ] {
        let diags = typecheck_source(&program(body));
        assert_eq!(
            diags.len(),
            1,
            "{label}: expected exactly one diagnostic, got {:?}",
            messages(&diags)
        );
        assert_eq!(diags[0].code, ErrorCode::TypeMismatch, "{label}");
    }
}

#[test]
fn bottom_typed_values_still_fill_aggregate_slots() {
    let src = program(
        "    let o: Option<i64> = Option::None;\n    let m: HashMap<i64, i64> = HashMap::new();\n    let nested: Vec<Vec<i64>> = Vec::new();\n    nested.push(Vec::new());\n    let opts: Vec<Option<i64>> = Vec::new();\n    opts.push(Option::None);",
    );
    let diags = typecheck_source(&src);
    assert!(diags.is_empty(), "got {:?}", messages(&diags));
}

#[test]
fn a_diverging_expression_is_still_accepted_as_an_index_or_argument() {
    let src = program("    s.push_str(return 1);\n    let x: i64 = v[return 2];");
    let diags = typecheck_source(&src);
    assert!(diags.is_empty(), "got {:?}", messages(&diags));
}

#[test]
fn member_access_on_the_bottom_type_is_rejected_once_each() {
    for (label, body) in [
        ("field", "    let x: i64 = Option::None.field;"),
        ("index", "    let x: i64 = Option::None[0];"),
        ("method", "    let x: i64 = Option::None.bogus();"),
        ("map method", "    let x: i64 = HashMap::new().bogus(1);"),
    ] {
        let diags = typecheck_source(&program(body));
        assert_eq!(
            diags.len(),
            1,
            "{label}: expected exactly one diagnostic, got {:?}",
            messages(&diags)
        );
    }
}
