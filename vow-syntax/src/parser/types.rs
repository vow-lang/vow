use crate::ast::{Lit, Pat, PatKind, Type};
use crate::token::TokenKind;

use super::Parser;

impl Parser {
    pub fn parse_type_inner(&mut self) -> Type {
        let start = self.current_span();
        match self.peek_kind().clone() {
            TokenKind::Bang => {
                self.advance();
                Type::Never { span: start }
            }
            TokenKind::Amp => {
                self.advance();
                let inner = self.parse_type_inner();
                let span = start.merge(inner.span());
                Type::Reference {
                    inner: Box::new(inner),
                    span,
                }
            }
            TokenKind::LBracket => {
                self.advance();
                let inner = self.parse_type_inner();
                let end = self.current_span();
                self.expect(TokenKind::RBracket);
                Type::Slice {
                    inner: Box::new(inner),
                    span: start.merge(end),
                }
            }
            TokenKind::LParen => self.parse_tuple_or_unit_type(start),
            TokenKind::LBrace => self.parse_refinement_type(start),
            TokenKind::Ident(name) => {
                self.advance();
                if self.at(&TokenKind::Lt) {
                    self.advance();
                    let mut args = Vec::new();
                    while !self.at(&TokenKind::Gt) && !self.at_end() {
                        args.push(self.parse_type_inner());
                        if self.at(&TokenKind::Comma) {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    let end = self.current_span();
                    self.expect(TokenKind::Gt);
                    Type::Generic {
                        name,
                        args,
                        span: start.merge(end),
                    }
                } else {
                    Type::Named { name, span: start }
                }
            }
            _ => {
                let span = self.current_span();
                self.push_error(
                    vow_diag::ErrorCode::UnexpectedToken,
                    format!("expected type, got {:?}", self.peek_kind()),
                    span,
                );
                Type::Named {
                    name: "<error>".to_string(),
                    span,
                }
            }
        }
    }

    fn parse_tuple_or_unit_type(&mut self, start: crate::span::Span) -> Type {
        self.advance();

        if self.at(&TokenKind::RParen) {
            let end = self.advance().span;
            return Type::Unit {
                span: start.merge(end),
            };
        }

        let first = self.parse_type_inner();

        if self.at(&TokenKind::RParen) {
            self.advance();
            return first;
        }

        let mut elems = vec![first];
        while self.at(&TokenKind::Comma) {
            self.advance();
            if self.at(&TokenKind::RParen) {
                break;
            }
            elems.push(self.parse_type_inner());
        }
        let end = self.current_span();
        self.expect(TokenKind::RParen);
        Type::Tuple {
            elems,
            span: start.merge(end),
        }
    }

    fn parse_refinement_type(&mut self, start: crate::span::Span) -> Type {
        self.advance();
        let (binding, _) = self
            .expect_ident()
            .unwrap_or(("<error>".to_string(), start));
        self.expect(TokenKind::Colon);
        let base = self.parse_type_inner();
        self.expect(TokenKind::PipePipe);
        let predicate = self.parse_expr_inner(0);
        let end = self.current_span();
        self.expect(TokenKind::RBrace);
        Type::Refinement {
            binding,
            base: Box::new(base),
            predicate: Box::new(predicate),
            span: start.merge(end),
        }
    }

    pub fn parse_pat_inner(&mut self) -> Pat {
        let start = self.current_span();
        let first = self.parse_single_pat();

        if self.at(&TokenKind::PipePipe) {
            let mut pats = vec![first];
            while self.at(&TokenKind::PipePipe) {
                self.advance();
                pats.push(self.parse_single_pat());
            }
            let end = self.current_span();
            return Pat {
                kind: PatKind::Or(pats),
                span: start.merge(end),
            };
        }

        first
    }

    fn parse_single_pat(&mut self) -> Pat {
        let start = self.current_span();
        match self.peek_kind().clone() {
            TokenKind::Underscore => {
                self.advance();
                Pat {
                    kind: PatKind::Wildcard,
                    span: start,
                }
            }
            TokenKind::KwMut => {
                self.advance();
                let (name, end) = self
                    .expect_ident()
                    .unwrap_or(("<error>".to_string(), start));
                Pat {
                    kind: PatKind::Ident { name, is_mut: true },
                    span: start.merge(end),
                }
            }
            TokenKind::LitInt(v) => {
                self.advance();
                Pat {
                    kind: PatKind::Lit(Lit::Int(v)),
                    span: start,
                }
            }
            TokenKind::LitFloat(v) => {
                self.advance();
                Pat {
                    kind: PatKind::Lit(Lit::Float(v)),
                    span: start,
                }
            }
            TokenKind::LitBool(v) => {
                self.advance();
                Pat {
                    kind: PatKind::Lit(Lit::Bool(v)),
                    span: start,
                }
            }
            TokenKind::LitString(s) => {
                self.advance();
                Pat {
                    kind: PatKind::Lit(Lit::String(s)),
                    span: start,
                }
            }
            TokenKind::LParen => {
                self.advance();
                if self.at(&TokenKind::RParen) {
                    let end = self.advance().span;
                    return Pat {
                        kind: PatKind::Tuple(vec![]),
                        span: start.merge(end),
                    };
                }
                let mut pats = vec![self.parse_pat_inner()];
                while self.at(&TokenKind::Comma) {
                    self.advance();
                    if self.at(&TokenKind::RParen) {
                        break;
                    }
                    pats.push(self.parse_pat_inner());
                }
                let end = self.current_span();
                self.expect(TokenKind::RParen);
                Pat {
                    kind: PatKind::Tuple(pats),
                    span: start.merge(end),
                }
            }
            TokenKind::Ident(name) => {
                self.advance();

                if self.at(&TokenKind::ColonColon) {
                    let mut path = vec![name];
                    while self.at(&TokenKind::ColonColon) {
                        self.advance();
                        let (segment, _) = self
                            .expect_ident()
                            .unwrap_or(("<error>".to_string(), start));
                        path.push(segment);
                    }
                    let inner = if self.at(&TokenKind::LParen) {
                        self.advance();
                        let mut inner_pats = Vec::new();
                        while !self.at(&TokenKind::RParen) && !self.at_end() {
                            inner_pats.push(self.parse_pat_inner());
                            if self.at(&TokenKind::Comma) {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                        self.expect(TokenKind::RParen);
                        inner_pats
                    } else {
                        Vec::new()
                    };
                    let end = self.current_span();
                    return Pat {
                        kind: PatKind::EnumVariant { path, inner },
                        span: start.merge(end),
                    };
                }

                if self.at(&TokenKind::LParen) {
                    self.advance();
                    let mut inner_pats = Vec::new();
                    while !self.at(&TokenKind::RParen) && !self.at_end() {
                        inner_pats.push(self.parse_pat_inner());
                        if self.at(&TokenKind::Comma) {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    self.expect(TokenKind::RParen);
                    let end = self.current_span();
                    return Pat {
                        kind: PatKind::EnumVariant {
                            path: vec![name],
                            inner: inner_pats,
                        },
                        span: start.merge(end),
                    };
                }

                if self.at(&TokenKind::LBrace) {
                    self.advance();
                    let mut fields = Vec::new();
                    while !self.at(&TokenKind::RBrace) && !self.at_end() {
                        let (field_name, _) = self
                            .expect_ident()
                            .unwrap_or(("<error>".to_string(), start));
                        self.expect(TokenKind::Colon);
                        let field_pat = self.parse_pat_inner();
                        fields.push((field_name, field_pat));
                        if self.at(&TokenKind::Comma) {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    self.expect(TokenKind::RBrace);
                    let end = self.current_span();
                    return Pat {
                        kind: PatKind::Struct { name, fields },
                        span: start.merge(end),
                    };
                }

                Pat {
                    kind: PatKind::Ident {
                        name,
                        is_mut: false,
                    },
                    span: start,
                }
            }
            _ => {
                let span = self.current_span();
                self.push_error(
                    vow_diag::ErrorCode::UnexpectedToken,
                    format!("expected pattern, got {:?}", self.peek_kind()),
                    span,
                );
                Pat {
                    kind: PatKind::Wildcard,
                    span,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Lit, PatKind};

    fn parse_type(src: &str) -> Type {
        let tokens = crate::lexer::Lexer::new(src).tokenize().expect("lex error");
        let mut parser = Parser::new(tokens, String::new(), "<test>".to_string());
        let ty = parser.parse_type_inner();
        assert!(
            parser.diagnostics.is_empty(),
            "unexpected errors: {:?}",
            parser
                .diagnostics
                .iter()
                .map(|e| &e.message)
                .collect::<Vec<_>>()
        );
        ty
    }

    fn parse_pat(src: &str) -> Pat {
        let tokens = crate::lexer::Lexer::new(src).tokenize().expect("lex error");
        let mut parser = Parser::new(tokens, String::new(), "<test>".to_string());
        let pat = parser.parse_pat_inner();
        assert!(
            parser.diagnostics.is_empty(),
            "unexpected errors: {:?}",
            parser
                .diagnostics
                .iter()
                .map(|e| &e.message)
                .collect::<Vec<_>>()
        );
        pat
    }

    #[test]
    fn type_unit() {
        let ty = parse_type("()");
        assert!(matches!(ty, Type::Unit { .. }));
    }

    #[test]
    fn type_never() {
        let ty = parse_type("!");
        assert!(matches!(ty, Type::Never { .. }));
    }

    #[test]
    fn type_named() {
        let ty = parse_type("i64");
        assert!(matches!(ty, Type::Named { ref name, .. } if name == "i64"));
    }

    #[test]
    fn type_reference() {
        let ty = parse_type("&i64");
        match ty {
            Type::Reference { inner, .. } => {
                assert!(matches!(*inner, Type::Named { ref name, .. } if name == "i64"));
            }
            _ => panic!("expected Reference"),
        }
    }

    #[test]
    fn type_slice() {
        let ty = parse_type("[i64]");
        match ty {
            Type::Slice { inner, .. } => {
                assert!(matches!(*inner, Type::Named { ref name, .. } if name == "i64"));
            }
            _ => panic!("expected Slice"),
        }
    }

    #[test]
    fn type_tuple() {
        let ty = parse_type("(i64, bool)");
        match ty {
            Type::Tuple { elems, .. } => {
                assert_eq!(elems.len(), 2);
                assert!(matches!(&elems[0], Type::Named { name, .. } if name == "i64"));
                assert!(matches!(&elems[1], Type::Named { name, .. } if name == "bool"));
            }
            _ => panic!("expected Tuple"),
        }
    }

    #[test]
    fn type_generic() {
        let ty = parse_type("Vec<i64>");
        match ty {
            Type::Generic { name, args, .. } => {
                assert_eq!(name, "Vec");
                assert_eq!(args.len(), 1);
                assert!(matches!(&args[0], Type::Named { name, .. } if name == "i64"));
            }
            _ => panic!("expected Generic"),
        }
    }

    #[test]
    fn type_nested_generic() {
        let ty = parse_type("Vec<Option<i64>>");
        match ty {
            Type::Generic { name, args, .. } => {
                assert_eq!(name, "Vec");
                assert_eq!(args.len(), 1);
                match &args[0] {
                    Type::Generic {
                        name,
                        args: inner_args,
                        ..
                    } => {
                        assert_eq!(name, "Option");
                        assert_eq!(inner_args.len(), 1);
                        assert!(
                            matches!(&inner_args[0], Type::Named { name, .. } if name == "i64")
                        );
                    }
                    _ => panic!("expected inner Generic"),
                }
            }
            _ => panic!("expected outer Generic"),
        }
    }

    #[test]
    fn type_grouping() {
        let ty = parse_type("(i64)");
        assert!(matches!(ty, Type::Named { ref name, .. } if name == "i64"));
    }

    #[test]
    fn type_refinement_with_pipepipe() {
        let ty = parse_type("{ x: i64 || x }");
        match ty {
            Type::Refinement {
                binding,
                base,
                predicate,
                ..
            } => {
                assert_eq!(binding, "x");
                assert!(matches!(*base, Type::Named { ref name, .. } if name == "i64"));
                let _ = predicate;
            }
            _ => panic!("expected Refinement"),
        }
    }

    #[test]
    fn type_multi_generic() {
        let ty = parse_type("Map<i64, bool>");
        match ty {
            Type::Generic { name, args, .. } => {
                assert_eq!(name, "Map");
                assert_eq!(args.len(), 2);
            }
            _ => panic!("expected Generic"),
        }
    }

    #[test]
    fn type_double_close_angle_generic() {
        let ty = parse_type("Vec<Vec<i64>>");
        match ty {
            Type::Generic { name, args, .. } => {
                assert_eq!(name, "Vec");
                assert_eq!(args.len(), 1);
                assert!(matches!(args[0], Type::Generic { .. }));
            }
            _ => panic!("expected nested Generic"),
        }
    }

    #[test]
    fn pat_wildcard() {
        let pat = parse_pat("_");
        assert!(matches!(pat.kind, PatKind::Wildcard));
    }

    #[test]
    fn pat_ident() {
        let pat = parse_pat("x");
        assert!(matches!(&pat.kind, PatKind::Ident { name, is_mut: false } if name == "x"));
    }

    #[test]
    fn pat_mut_ident() {
        let pat = parse_pat("mut y");
        assert!(matches!(&pat.kind, PatKind::Ident { name, is_mut: true } if name == "y"));
    }

    #[test]
    fn pat_lit_int() {
        let pat = parse_pat("42");
        assert!(matches!(&pat.kind, PatKind::Lit(Lit::Int(42))));
    }

    #[test]
    fn pat_lit_bool() {
        let pat = parse_pat("true");
        assert!(matches!(&pat.kind, PatKind::Lit(Lit::Bool(true))));
    }

    #[test]
    fn pat_tuple() {
        let pat = parse_pat("(a, b)");
        match &pat.kind {
            PatKind::Tuple(pats) => {
                assert_eq!(pats.len(), 2);
                assert!(matches!(&pats[0].kind, PatKind::Ident { name, .. } if name == "a"));
                assert!(matches!(&pats[1].kind, PatKind::Ident { name, .. } if name == "b"));
            }
            _ => panic!("expected Tuple pat"),
        }
    }

    #[test]
    fn pat_enum_variant_path() {
        let pat = parse_pat("Option::None");
        match &pat.kind {
            PatKind::EnumVariant { path, inner } => {
                assert_eq!(path, &vec!["Option".to_string(), "None".to_string()]);
                assert!(inner.is_empty());
            }
            _ => panic!("expected EnumVariant"),
        }
    }

    #[test]
    fn pat_enum_variant_with_inner() {
        let pat = parse_pat("Option::Some(x)");
        match &pat.kind {
            PatKind::EnumVariant { path, inner } => {
                assert_eq!(path, &vec!["Option".to_string(), "Some".to_string()]);
                assert_eq!(inner.len(), 1);
                assert!(matches!(&inner[0].kind, PatKind::Ident { name, .. } if name == "x"));
            }
            _ => panic!("expected EnumVariant with inner"),
        }
    }

    #[test]
    fn pat_struct() {
        let pat = parse_pat("Point { x: a, y: b }");
        match &pat.kind {
            PatKind::Struct { name, fields } => {
                assert_eq!(name, "Point");
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].0, "x");
                assert_eq!(fields[1].0, "y");
            }
            _ => panic!("expected Struct pat"),
        }
    }

    #[test]
    fn pat_or() {
        let pat = parse_pat("a || b");
        match &pat.kind {
            PatKind::Or(pats) => {
                assert_eq!(pats.len(), 2);
            }
            _ => panic!("expected Or pat"),
        }
    }

    #[test]
    fn pat_lit_string() {
        let pat = parse_pat("\"hello\"");
        assert!(matches!(&pat.kind, PatKind::Lit(Lit::String(s)) if s == "hello"));
    }

    #[test]
    fn pat_unit_tuple() {
        let pat = parse_pat("()");
        assert!(matches!(&pat.kind, PatKind::Tuple(pats) if pats.is_empty()));
    }

    #[test]
    fn pat_enum_variant_no_path() {
        let pat = parse_pat("Some(x)");
        match &pat.kind {
            PatKind::EnumVariant { path, inner } => {
                assert_eq!(path, &vec!["Some".to_string()]);
                assert_eq!(inner.len(), 1);
            }
            _ => panic!("expected EnumVariant"),
        }
    }
}
