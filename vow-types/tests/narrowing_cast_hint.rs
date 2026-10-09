//! The `NarrowingCastNotAllowed` hint only names `_try` intrinsics that the
//! checker's own function table registers.

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

fn narrowing_hint(src_ty: &str, tgt_ty: &str) -> String {
    let src = format!(
        "module Test\n\nfn f(x: {src_ty}) -> {tgt_ty} {{\n    x as {tgt_ty}\n}}\n\nfn main() -> i32 {{\n    0\n}}\n"
    );
    let (module, parse_diags) = parse_module(&src, "<test>");
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
    let diags = emitter.diagnostics;
    assert_eq!(diags.len(), 1, "expected one diagnostic, got {diags:?}");
    let diag = &diags[0];
    assert_eq!(diag.code, ErrorCode::NarrowingCastNotAllowed);
    assert_eq!(
        diag.hints.len(),
        1,
        "expected one hint, got {:?}",
        diag.hints
    );
    diag.hints[0].clone()
}

#[test]
fn registered_narrowing_pairs_name_their_try_intrinsic() {
    for (src, tgt) in [
        ("i64", "i32"),
        ("u64", "u8"),
        ("u128", "u32"),
        ("i128", "u8"),
        ("i128", "i32"),
        ("i32", "i8"),
    ] {
        assert_eq!(
            narrowing_hint(src, tgt),
            format!("use a `{src}_to_{tgt}_try`, `_wrap`, or `_sat` narrowing intrinsic"),
        );
    }
}

#[test]
fn pairs_without_a_registered_family_do_not_name_a_missing_intrinsic() {
    for (src, tgt) in [
        ("i128", "i64"),
        ("u128", "u64"),
        ("i128", "u64"),
        ("u128", "i64"),
    ] {
        assert_eq!(
            narrowing_hint(src, tgt),
            format!(
                "no narrowing intrinsic from {src} to {tgt} exists; narrow to i32 or smaller with `{src}_to_i32_try`, or keep the value as {src}"
            ),
        );
    }
}
