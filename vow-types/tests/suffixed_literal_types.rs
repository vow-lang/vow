//! An integer-literal suffix fixes the literal's type and is range-checked
//! exactly like the `NNN as T` cast it desugars to.

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

fn body(ret: &str, expr: &str) -> String {
    format!("module Test\n\nfn f() -> {ret} {{\n    {expr}\n}}\n")
}

#[test]
fn in_range_suffixed_literals_are_accepted() {
    let cases = [
        ("u8", "255u8"),
        ("u8", "0u8"),
        ("i8", "127i8"),
        ("i8", "-128i8"),
        ("i16", "-32768i16"),
        ("u16", "65535u16"),
        ("i32", "-2147483648i32"),
        ("u32", "4294967295u32"),
        ("i64", "-9223372036854775808i64"),
        ("i64", "9223372036854775807i64"),
        ("u64", "18446744073709551615u64"),
        ("u8", "-0u8"),
    ];
    for (ret, expr) in cases {
        assert_eq!(codes(&body(ret, expr)), vec![], "{expr}");
    }
}

#[test]
fn out_of_range_suffixed_literals_are_rejected() {
    let cases = [
        ("u8", "256u8"),
        ("i8", "128i8"),
        ("i8", "-129i8"),
        ("i16", "32768i16"),
        ("u16", "65536u16"),
        ("i32", "2147483648i32"),
        ("u32", "4294967296u32"),
        ("i64", "9223372036854775808i64"),
        ("i64", "-9223372036854775809i64"),
    ];
    for (ret, expr) in cases {
        assert_eq!(
            codes(&body(ret, expr)),
            vec![ErrorCode::LiteralOutOfRange],
            "{expr}"
        );
    }
}

#[test]
fn negating_an_unsigned_suffixed_literal_is_a_type_error() {
    for (ret, expr) in [("u32", "-1u32"), ("u8", "-1u8")] {
        assert_eq!(
            codes(&body(ret, expr)),
            vec![ErrorCode::TypeMismatch],
            "{expr}"
        );
    }
}

#[test]
fn a_suffix_fixes_the_type_instead_of_coercing_to_context() {
    for src in [
        "module Test\n\nfn f() -> i64 {\n    let x: i64 = 5u8;\n    x\n}\n",
        "module Test\n\nfn f() -> u8 {\n    let x: u8 = 5i64;\n    x\n}\n",
        "module Test\n\nfn f(x: i64) -> i64 {\n    x + 1i32\n}\n",
    ] {
        assert_eq!(codes(src), vec![ErrorCode::TypeMismatch], "{src}");
    }
}

#[test]
fn a_suffixed_literal_is_accepted_at_its_own_type() {
    let src = "module Test\n\nfn f(x: u8) -> u8 {\n    let y: u8 = 7u8;\n    x + y + 1u8\n}\n";
    assert_eq!(codes(src), vec![]);
}
