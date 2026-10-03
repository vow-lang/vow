//! Every integer type is a valid `const` declaration type, with the same
//! compile-time range check as an integer literal in a typed context.

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

fn codes(src: &str) -> Vec<ErrorCode> {
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
    emitter.diagnostics.iter().map(|d| d.code).collect()
}

fn const_program(ty: &str, value: &str) -> String {
    format!("module Test\n\nconst X: {ty} = {value};\n\nfn main() -> i32 {{\n    0\n}}\n")
}

#[test]
fn every_integer_type_accepts_an_in_range_const() {
    let cases = [
        ("i8", "-128"),
        ("i8", "127"),
        ("i16", "-32768"),
        ("i16", "32767"),
        ("i32", "-2147483648"),
        ("i32", "2147483647"),
        ("i64", "-9223372036854775808"),
        ("i64", "9223372036854775807"),
        ("i128", "-170141183460469231731687303715884105728"),
        ("i128", "170141183460469231731687303715884105727"),
        ("u8", "255"),
        ("u16", "65535"),
        ("u32", "4294967295"),
        ("u64", "18446744073709551615"),
        ("u128", "340282366920938463463374607431768211455"),
    ];
    for (ty, value) in cases {
        let found = codes(&const_program(ty, value));
        assert!(found.is_empty(), "const X: {ty} = {value}; got {found:?}");
    }
}

#[test]
fn out_of_range_const_is_literal_out_of_range() {
    let cases = [
        ("i8", "128"),
        ("i8", "-129"),
        ("u8", "256"),
        ("u16", "65536"),
        ("u32", "4294967296"),
        ("u64", "18446744073709551616"),
        ("u64", "-1"),
        ("u128", "-1"),
        ("i128", "170141183460469231731687303715884105728"),
    ];
    for (ty, value) in cases {
        let found = codes(&const_program(ty, value));
        assert_eq!(
            found,
            vec![ErrorCode::LiteralOutOfRange],
            "const X: {ty} = {value};"
        );
    }
}

#[test]
fn non_integer_const_type_for_integer_value_is_type_mismatch() {
    for ty in ["bool", "String", "f64"] {
        let found = codes(&const_program(ty, "42"));
        assert!(
            found.contains(&ErrorCode::TypeMismatch),
            "const X: {ty} = 42; got {found:?}"
        );
    }
}

#[test]
fn u64_const_indexes_a_vec_without_a_cast() {
    let src = "module Test\n\nconst IDX: u64 = 2;\n\nfn main() -> i32 {\n    let v: Vec<i64> = Vec::new();\n    v.push(1);\n    let x: i64 = v[IDX];\n    0\n}\n";
    let found = codes(src);
    assert!(found.is_empty(), "got {found:?}");
}

#[test]
fn i64_const_index_is_still_rejected() {
    let src = "module Test\n\nconst IDX: i64 = 2;\n\nfn main() -> i32 {\n    let v: Vec<i64> = Vec::new();\n    v.push(1);\n    let x: i64 = v[IDX];\n    0\n}\n";
    let found = codes(src);
    assert!(found.contains(&ErrorCode::TypeMismatch), "got {found:?}");
}

#[test]
fn const_keeps_its_declared_type_in_expressions() {
    let src = "module Test\n\nconst LIMIT: u64 = 10;\n\nfn f(n: u64) -> bool {\n    n < LIMIT\n}\n\nfn g(n: i64) -> bool {\n    n < LIMIT\n}\n";
    let found = codes(src);
    assert_eq!(found, vec![ErrorCode::TypeMismatch], "got {found:?}");
}
