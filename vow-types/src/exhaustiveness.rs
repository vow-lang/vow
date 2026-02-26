use vow_diag::{Blame, Diagnostic, DiagnosticEmitter, ErrorCode, Severity, SourceLocation};
use vow_syntax::ast::{Lit, MatchArm, PatKind};
use vow_syntax::span::Span;

use crate::env::TypeEnv;
use crate::types::Ty;

pub fn check_exhaustive(
    scrutinee_ty: &Ty,
    arms: &[MatchArm],
    env: &TypeEnv,
    span: Span,
    file: &str,
    emitter: &mut dyn DiagnosticEmitter,
) {
    if arms
        .iter()
        .any(|arm| matches!(arm.pattern.kind, PatKind::Wildcard))
    {
        return;
    }

    match scrutinee_ty {
        Ty::Bool => check_bool_exhaustive(arms, span, file, emitter),
        Ty::Enum(name) => check_enum_exhaustive(name, arms, env, span, file, emitter),
        Ty::Applied(base, _) => {
            if let Ty::Enum(name) = base.as_ref() {
                check_enum_exhaustive(name, arms, env, span, file, emitter);
            }
        }
        _ => {}
    }
}

fn check_bool_exhaustive(
    arms: &[MatchArm],
    span: Span,
    file: &str,
    emitter: &mut dyn DiagnosticEmitter,
) {
    let mut has_true = false;
    let mut has_false = false;

    for arm in arms {
        collect_bool_patterns(&arm.pattern.kind, &mut has_true, &mut has_false);
    }

    let mut missing = Vec::new();
    if !has_true {
        missing.push("true");
    }
    if !has_false {
        missing.push("false");
    }

    if !missing.is_empty() {
        emit_non_exhaustive(missing.join(", "), span, file, emitter);
    }
}

fn collect_bool_patterns(kind: &PatKind, has_true: &mut bool, has_false: &mut bool) {
    match kind {
        PatKind::Lit(Lit::Bool(true)) => *has_true = true,
        PatKind::Lit(Lit::Bool(false)) => *has_false = true,
        PatKind::Or(pats) => {
            for p in pats {
                collect_bool_patterns(&p.kind, has_true, has_false);
            }
        }
        _ => {}
    }
}

fn check_enum_exhaustive(
    name: &str,
    arms: &[MatchArm],
    env: &TypeEnv,
    span: Span,
    file: &str,
    emitter: &mut dyn DiagnosticEmitter,
) {
    let info = match env.lookup_enum(name) {
        Some(info) => info,
        None => return,
    };

    let all_variant_names: Vec<&str> = info.variants.iter().map(|v| v.name.as_str()).collect();

    let mut covered: std::collections::HashSet<&str> = std::collections::HashSet::new();

    for arm in arms {
        collect_enum_patterns(&arm.pattern.kind, &all_variant_names, &mut covered);
    }

    let missing: Vec<String> = all_variant_names
        .iter()
        .filter(|n| !covered.contains(*n))
        .map(|n| n.to_string())
        .collect();

    if !missing.is_empty() {
        emit_non_exhaustive(missing.join(", "), span, file, emitter);
    }
}

fn collect_enum_patterns<'a>(
    kind: &'a PatKind,
    variant_names: &[&'a str],
    covered: &mut std::collections::HashSet<&'a str>,
) {
    match kind {
        PatKind::EnumVariant { path, .. } => {
            if let Some(variant) = path.last()
                && let Some(&vn) = variant_names.iter().find(|&&n| n == variant.as_str())
            {
                covered.insert(vn);
            }
        }
        PatKind::Ident { name, .. } => {
            if let Some(&vn) = variant_names.iter().find(|&&n| n == name.as_str()) {
                covered.insert(vn);
            }
        }
        PatKind::Or(pats) => {
            for p in pats {
                collect_enum_patterns(&p.kind, variant_names, covered);
            }
        }
        _ => {}
    }
}

fn emit_non_exhaustive(
    missing: String,
    span: Span,
    file: &str,
    emitter: &mut dyn DiagnosticEmitter,
) {
    emitter.emit(&Diagnostic {
        severity: Severity::Error,
        code: ErrorCode::NonExhaustiveMatch,
        message: format!("non-exhaustive match: missing patterns: {missing}"),
        primary: SourceLocation {
            file: file.to_string(),
            byte_offset: span.start,
            byte_len: span.len,
        },
        secondary: vec![],
        blame: Blame::None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::{EnumInfo, VariantInfo, VariantKind};
    use vow_syntax::ast::{Lit, MatchArm, Pat, PatKind};
    use vow_syntax::span::Span;

    struct TestEmitter(Vec<Diagnostic>);

    impl DiagnosticEmitter for TestEmitter {
        fn emit(&mut self, d: &Diagnostic) {
            self.0.push(d.clone());
        }
        fn finish(&mut self) {}
    }

    fn dummy_span() -> Span {
        Span::new(0, 1)
    }

    fn make_arm(kind: PatKind) -> MatchArm {
        MatchArm {
            pattern: Pat {
                kind,
                span: dummy_span(),
            },
            body: vow_syntax::ast::Expr {
                kind: vow_syntax::ast::ExprKind::Lit(Lit::Int(0)),
                span: dummy_span(),
            },
            span: dummy_span(),
        }
    }

    #[test]
    fn wildcard_is_always_exhaustive() {
        let mut emitter = TestEmitter(vec![]);
        let env = TypeEnv::new();
        let arms = vec![make_arm(PatKind::Wildcard)];
        check_exhaustive(
            &Ty::Bool,
            &arms,
            &env,
            dummy_span(),
            "test.vow",
            &mut emitter,
        );
        assert!(emitter.0.is_empty());
    }

    #[test]
    fn bool_both_covered() {
        let mut emitter = TestEmitter(vec![]);
        let env = TypeEnv::new();
        let arms = vec![
            make_arm(PatKind::Lit(Lit::Bool(true))),
            make_arm(PatKind::Lit(Lit::Bool(false))),
        ];
        check_exhaustive(
            &Ty::Bool,
            &arms,
            &env,
            dummy_span(),
            "test.vow",
            &mut emitter,
        );
        assert!(emitter.0.is_empty());
    }

    #[test]
    fn bool_missing_false() {
        let mut emitter = TestEmitter(vec![]);
        let env = TypeEnv::new();
        let arms = vec![make_arm(PatKind::Lit(Lit::Bool(true)))];
        check_exhaustive(
            &Ty::Bool,
            &arms,
            &env,
            dummy_span(),
            "test.vow",
            &mut emitter,
        );
        assert_eq!(emitter.0.len(), 1);
        assert_eq!(emitter.0[0].code, ErrorCode::NonExhaustiveMatch);
        assert!(emitter.0[0].message.contains("false"));
    }

    fn make_enum_env(name: &str, variants: &[&str]) -> TypeEnv {
        let mut env = TypeEnv::new();
        let info = EnumInfo {
            variants: variants
                .iter()
                .map(|v| VariantInfo {
                    name: v.to_string(),
                    kind: VariantKind::Unit,
                })
                .collect(),
            generics: vec![],
        };
        env.define_enum(name, info);
        env
    }

    #[test]
    fn enum_all_variants_covered() {
        let mut emitter = TestEmitter(vec![]);
        let env = make_enum_env("Color", &["Red", "Green", "Blue"]);
        let arms = vec![
            make_arm(PatKind::EnumVariant {
                path: vec!["Red".to_string()],
                inner: vec![],
            }),
            make_arm(PatKind::EnumVariant {
                path: vec!["Green".to_string()],
                inner: vec![],
            }),
            make_arm(PatKind::EnumVariant {
                path: vec!["Blue".to_string()],
                inner: vec![],
            }),
        ];
        check_exhaustive(
            &Ty::Enum("Color".to_string()),
            &arms,
            &env,
            dummy_span(),
            "test.vow",
            &mut emitter,
        );
        assert!(emitter.0.is_empty());
    }

    #[test]
    fn enum_missing_variant() {
        let mut emitter = TestEmitter(vec![]);
        let env = make_enum_env("Color", &["Red", "Green", "Blue"]);
        let arms = vec![
            make_arm(PatKind::EnumVariant {
                path: vec!["Red".to_string()],
                inner: vec![],
            }),
            make_arm(PatKind::EnumVariant {
                path: vec!["Green".to_string()],
                inner: vec![],
            }),
        ];
        check_exhaustive(
            &Ty::Enum("Color".to_string()),
            &arms,
            &env,
            dummy_span(),
            "test.vow",
            &mut emitter,
        );
        assert_eq!(emitter.0.len(), 1);
        assert_eq!(emitter.0[0].code, ErrorCode::NonExhaustiveMatch);
        assert!(emitter.0[0].message.contains("Blue"));
    }
}
