use crate::ast::{BinOp, Expr, ExprKind, Lit, MatchArm, UnOp};
use crate::span::Span;
use crate::token::TokenKind;

use super::Parser;

fn infix_binding_power(kind: &TokenKind) -> Option<(u8, u8)> {
    match kind {
        TokenKind::PipePipe => Some((1, 2)),
        TokenKind::AmpAmp => Some((3, 4)),
        TokenKind::EqEq
        | TokenKind::BangEq
        | TokenKind::Lt
        | TokenKind::LtEq
        | TokenKind::Gt
        | TokenKind::GtEq => Some((5, 6)),
        TokenKind::Plus | TokenKind::Minus | TokenKind::PlusChecked | TokenKind::MinusChecked => {
            Some((7, 8))
        }
        TokenKind::Star
        | TokenKind::Slash
        | TokenKind::Percent
        | TokenKind::StarChecked
        | TokenKind::SlashChecked
        | TokenKind::PercentChecked => Some((9, 10)),
        _ => None,
    }
}

fn token_to_binop(kind: &TokenKind) -> Option<BinOp> {
    match kind {
        TokenKind::Plus => Some(BinOp::Add),
        TokenKind::Minus => Some(BinOp::Sub),
        TokenKind::Star => Some(BinOp::Mul),
        TokenKind::Slash => Some(BinOp::Div),
        TokenKind::Percent => Some(BinOp::Rem),
        TokenKind::PlusChecked => Some(BinOp::AddChecked),
        TokenKind::MinusChecked => Some(BinOp::SubChecked),
        TokenKind::StarChecked => Some(BinOp::MulChecked),
        TokenKind::SlashChecked => Some(BinOp::DivChecked),
        TokenKind::PercentChecked => Some(BinOp::RemChecked),
        TokenKind::EqEq => Some(BinOp::Eq),
        TokenKind::BangEq => Some(BinOp::Ne),
        TokenKind::Lt => Some(BinOp::Lt),
        TokenKind::LtEq => Some(BinOp::Le),
        TokenKind::Gt => Some(BinOp::Gt),
        TokenKind::GtEq => Some(BinOp::Ge),
        TokenKind::AmpAmp => Some(BinOp::And),
        TokenKind::PipePipe => Some(BinOp::Or),
        _ => None,
    }
}

impl Parser {
    pub fn parse_expr_inner(&mut self, min_bp: u8) -> Expr {
        let mut lhs = self.parse_prefix();

        if matches!(
            lhs.kind,
            ExprKind::If { .. }
                | ExprKind::While { .. }
                | ExprKind::Loop { .. }
                | ExprKind::Block(_)
                | ExprKind::Match { .. }
        ) {
            return lhs;
        }

        loop {
            let kind = self.peek_kind().clone();

            if matches!(
                kind,
                TokenKind::Question | TokenKind::LParen | TokenKind::Dot | TokenKind::LBracket
            ) {
                lhs = self.parse_postfix(lhs);
                continue;
            }

            if kind == TokenKind::Eq {
                if min_bp > 0 {
                    break;
                }
                let op_span = self.current_span();
                self.advance();
                let rhs = self.parse_expr_inner(0);
                let span = lhs.span.merge(rhs.span).merge(op_span);
                lhs = Expr {
                    kind: ExprKind::Assign {
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                    },
                    span,
                };
                continue;
            }

            if let Some((lbp, rbp)) = infix_binding_power(&kind) {
                if lbp < min_bp {
                    break;
                }
                let op_span = self.current_span();
                self.advance();
                if let Some(op) = token_to_binop(&kind) {
                    let rhs = self.parse_expr_inner(rbp);
                    let span = lhs.span.merge(rhs.span).merge(op_span);
                    lhs = Expr {
                        kind: ExprKind::BinaryOp {
                            op,
                            lhs: Box::new(lhs),
                            rhs: Box::new(rhs),
                        },
                        span,
                    };
                }
                continue;
            }

            break;
        }

        lhs
    }

    fn parse_prefix(&mut self) -> Expr {
        let start = self.current_span();
        match self.peek_kind().clone() {
            TokenKind::LitInt(v) => {
                self.advance();
                Expr {
                    kind: ExprKind::Lit(Lit::Int(v)),
                    span: start,
                }
            }
            TokenKind::LitIntSuffixed { value, .. } => {
                self.advance();
                Expr {
                    kind: ExprKind::Lit(Lit::Int(value)),
                    span: start,
                }
            }
            TokenKind::LitFloat(v) => {
                self.advance();
                Expr {
                    kind: ExprKind::Lit(Lit::Float(v)),
                    span: start,
                }
            }
            TokenKind::LitBool(v) => {
                self.advance();
                Expr {
                    kind: ExprKind::Lit(Lit::Bool(v)),
                    span: start,
                }
            }
            TokenKind::LitString(s) => {
                self.advance();
                Expr {
                    kind: ExprKind::Lit(Lit::String(s)),
                    span: start,
                }
            }
            TokenKind::Ident(name) => {
                self.advance();
                if self.at(&TokenKind::ColonColon) {
                    return self.parse_enum_construct(name, start);
                }
                if self.at(&TokenKind::LBrace) && self.looks_like_struct_literal() {
                    return self.parse_struct_literal(name, start);
                }
                Expr {
                    kind: ExprKind::Ident(name),
                    span: start,
                }
            }
            TokenKind::Bang => {
                self.advance();
                let operand = self.parse_expr_inner(13);
                let span = start.merge(operand.span);
                Expr {
                    kind: ExprKind::UnaryOp {
                        op: UnOp::Not,
                        operand: Box::new(operand),
                    },
                    span,
                }
            }
            TokenKind::Minus => {
                self.advance();
                let operand = self.parse_expr_inner(13);
                let span = start.merge(operand.span);
                Expr {
                    kind: ExprKind::UnaryOp {
                        op: UnOp::Neg,
                        operand: Box::new(operand),
                    },
                    span,
                }
            }
            TokenKind::Amp => {
                self.advance();
                let operand = self.parse_expr_inner(13);
                let span = start.merge(operand.span);
                Expr {
                    kind: ExprKind::Borrow {
                        expr: Box::new(operand),
                    },
                    span,
                }
            }
            TokenKind::LParen => self.parse_paren_or_tuple(),
            TokenKind::LBrace => {
                let block = self.parse_block_required();
                let span = block.span;
                Expr {
                    kind: ExprKind::Block(Box::new(block)),
                    span,
                }
            }
            TokenKind::KwIf => self.parse_if_expr(),
            TokenKind::KwWhile => self.parse_while_expr(),
            TokenKind::KwLoop => self.parse_loop_expr(),
            TokenKind::KwBreak => {
                self.advance();
                let value = if self.is_expr_start() {
                    Some(Box::new(self.parse_expr_inner(0)))
                } else {
                    None
                };
                let end = value.as_ref().map(|e| e.span).unwrap_or(start);
                Expr {
                    kind: ExprKind::Break { value },
                    span: start.merge(end),
                }
            }
            TokenKind::KwReturn => {
                self.advance();
                let value = if self.is_expr_start() {
                    Some(Box::new(self.parse_expr_inner(0)))
                } else {
                    None
                };
                let end = value.as_ref().map(|e| e.span).unwrap_or(start);
                Expr {
                    kind: ExprKind::Return { value },
                    span: start.merge(end),
                }
            }
            TokenKind::KwMatch => self.parse_match_expr(),
            _ => {
                let span = self.current_span();
                self.push_error(
                    vow_diag::ErrorCode::UnexpectedToken,
                    format!("expected expression, got {:?}", self.peek_kind()),
                    span,
                );
                Expr {
                    kind: ExprKind::Lit(Lit::Int(0)),
                    span,
                }
            }
        }
    }

    fn parse_paren_or_tuple(&mut self) -> Expr {
        let start = self.current_span();
        self.advance();

        if self.at(&TokenKind::RParen) {
            let end = self.advance().span;
            return Expr {
                kind: ExprKind::Tuple(vec![]),
                span: start.merge(end),
            };
        }

        let first = self.parse_expr_inner(0);

        if self.at(&TokenKind::RParen) {
            self.advance();
            return first;
        }

        if self.at(&TokenKind::Comma) {
            let mut elems = vec![first];
            while self.at(&TokenKind::Comma) {
                self.advance();
                if self.at(&TokenKind::RParen) {
                    break;
                }
                elems.push(self.parse_expr_inner(0));
            }
            let end = self.current_span();
            self.expect(TokenKind::RParen);
            return Expr {
                kind: ExprKind::Tuple(elems),
                span: start.merge(end),
            };
        }

        self.expect(TokenKind::RParen);
        first
    }

    fn parse_postfix(&mut self, lhs: Expr) -> Expr {
        let start = lhs.span;
        match self.peek_kind().clone() {
            TokenKind::Question => {
                let end = self.advance().span;
                Expr {
                    kind: ExprKind::Question {
                        expr: Box::new(lhs),
                    },
                    span: start.merge(end),
                }
            }
            TokenKind::Dot => {
                self.advance();
                let (field_name, field_span) = self
                    .expect_ident()
                    .unwrap_or(("<error>".to_string(), self.current_span()));
                if self.at(&TokenKind::LParen) {
                    self.advance();
                    let args = self.parse_call_args();
                    let end = self.current_span();
                    Expr {
                        kind: ExprKind::MethodCall {
                            receiver: Box::new(lhs),
                            method: field_name,
                            args,
                        },
                        span: start.merge(end),
                    }
                } else {
                    Expr {
                        kind: ExprKind::FieldAccess {
                            base: Box::new(lhs),
                            field: field_name,
                        },
                        span: start.merge(field_span),
                    }
                }
            }
            TokenKind::LParen => {
                self.advance();
                let args = self.parse_call_args();
                let end = self.current_span();
                Expr {
                    kind: ExprKind::Call {
                        callee: Box::new(lhs),
                        args,
                    },
                    span: start.merge(end),
                }
            }
            TokenKind::LBracket => {
                self.advance();
                let index = self.parse_expr_inner(0);
                let end = self.current_span();
                self.expect(TokenKind::RBracket);
                Expr {
                    kind: ExprKind::Index {
                        base: Box::new(lhs),
                        index: Box::new(index),
                    },
                    span: start.merge(end),
                }
            }
            _ => lhs,
        }
    }

    fn parse_call_args(&mut self) -> Vec<Expr> {
        let mut args = Vec::new();
        while !self.at(&TokenKind::RParen) && !self.at_end() {
            args.push(self.parse_expr_inner(0));
            if self.at(&TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        self.expect(TokenKind::RParen);
        args
    }

    fn parse_if_expr(&mut self) -> Expr {
        let start = self.current_span();
        self.expect(TokenKind::KwIf);
        let condition = self.parse_expr_inner(0);
        let then_branch = self.parse_block_required();
        let else_branch = if self.at(&TokenKind::KwElse) {
            self.advance();
            if self.at(&TokenKind::KwIf) {
                Some(Box::new(self.parse_if_expr()))
            } else {
                let block = self.parse_block_required();
                let span = block.span;
                Some(Box::new(Expr {
                    kind: ExprKind::Block(Box::new(block)),
                    span,
                }))
            }
        } else {
            None
        };
        let end = else_branch
            .as_ref()
            .map(|e| e.span)
            .unwrap_or(then_branch.span);
        Expr {
            kind: ExprKind::If {
                condition: Box::new(condition),
                then_branch: Box::new(then_branch),
                else_branch,
            },
            span: start.merge(end),
        }
    }

    fn parse_while_expr(&mut self) -> Expr {
        let start = self.current_span();
        self.expect(TokenKind::KwWhile);
        let condition = self.parse_expr_inner(0);
        let vow = if self.at(&TokenKind::KwVow) {
            self.parse_vow_block()
        } else {
            None
        };
        let body = self.parse_block_required();
        let end = body.span;
        Expr {
            kind: ExprKind::While {
                condition: Box::new(condition),
                vow,
                body: Box::new(body),
            },
            span: start.merge(end),
        }
    }

    fn parse_loop_expr(&mut self) -> Expr {
        let start = self.current_span();
        self.expect(TokenKind::KwLoop);
        let vow = if self.at(&TokenKind::KwVow) {
            self.parse_vow_block()
        } else {
            None
        };
        let body = self.parse_block_required();
        let end = body.span;
        Expr {
            kind: ExprKind::Loop {
                vow,
                body: Box::new(body),
            },
            span: start.merge(end),
        }
    }

    fn parse_match_expr(&mut self) -> Expr {
        let start = self.current_span();
        self.expect(TokenKind::KwMatch);
        let scrutinee = self.parse_expr_inner(0);
        self.expect(TokenKind::LBrace);
        let mut arms = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_end() {
            let arm_start = self.current_span();
            let pattern = self.parse_pat_inner();
            self.expect(TokenKind::FatArrow);
            let body = self.parse_expr_inner(0);
            let arm_end = body.span;
            arms.push(MatchArm {
                pattern,
                body,
                span: arm_start.merge(arm_end),
            });
            if self.at(&TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        let end = self.current_span();
        self.expect(TokenKind::RBrace);
        Expr {
            kind: ExprKind::Match {
                scrutinee: Box::new(scrutinee),
                arms,
            },
            span: start.merge(end),
        }
    }

    fn is_expr_start(&self) -> bool {
        matches!(
            self.peek_kind(),
            TokenKind::LitInt(_)
                | TokenKind::LitIntSuffixed { .. }
                | TokenKind::LitFloat(_)
                | TokenKind::LitBool(_)
                | TokenKind::LitString(_)
                | TokenKind::Ident(_)
                | TokenKind::Bang
                | TokenKind::Minus
                | TokenKind::Amp
                | TokenKind::LParen
                | TokenKind::LBrace
                | TokenKind::KwIf
                | TokenKind::KwWhile
                | TokenKind::KwLoop
                | TokenKind::KwMatch
        )
    }

    fn looks_like_struct_literal(&self) -> bool {
        match self.tokens.get(self.cursor + 1).map(|t| &t.kind) {
            Some(TokenKind::RBrace) => true,
            Some(TokenKind::Ident(_)) => matches!(
                self.tokens.get(self.cursor + 2).map(|t| &t.kind),
                Some(TokenKind::Colon)
            ),
            _ => false,
        }
    }

    fn parse_struct_literal(&mut self, name: String, start: Span) -> Expr {
        self.expect(TokenKind::LBrace);
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_end() {
            let field_name = self.expect_ident().map(|(n, _)| n).unwrap_or_default();
            self.expect(TokenKind::Colon);
            let value = self.parse_expr_inner(0);
            fields.push((field_name, value));
            if self.at(&TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        let end = self.current_span();
        self.expect(TokenKind::RBrace);
        Expr {
            kind: ExprKind::StructLiteral { name, fields },
            span: start.merge(end),
        }
    }

    fn parse_enum_construct(&mut self, first_segment: String, start: Span) -> Expr {
        let mut path = vec![first_segment];
        while self.at(&TokenKind::ColonColon) {
            self.advance();
            if let Some((segment, _)) = self.expect_ident() {
                path.push(segment);
            } else {
                break;
            }
        }
        let fields = if self.at(&TokenKind::LParen) {
            self.advance();
            let mut args = Vec::new();
            while !self.at(&TokenKind::RParen) && !self.at_end() {
                args.push(self.parse_expr_inner(0));
                if self.at(&TokenKind::Comma) {
                    self.advance();
                } else {
                    break;
                }
            }
            self.expect(TokenKind::RParen);
            args
        } else {
            vec![]
        };
        let end = self.current_span();
        Expr {
            kind: ExprKind::EnumConstruct { path, fields },
            span: start.merge(end),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{BinOp, Block, ExprKind, Lit, UnOp};

    fn parse_expr_from_source(src: &str) -> Expr {
        let tokens = crate::lexer::Lexer::new(src).tokenize().expect("lex error");
        let mut parser = Parser::new(tokens, String::new());
        parser.parse_expr_inner(0)
    }

    fn parse_block_from_source(src: &str) -> Block {
        let tokens = crate::lexer::Lexer::new(src).tokenize().expect("lex error");
        let mut parser = Parser::new(tokens, String::new());
        parser.parse_block_required()
    }

    fn parse(src: &str) -> Expr {
        parse_expr_from_source(src)
    }

    fn parse_no_errors(src: &str) -> Expr {
        let tokens = crate::lexer::Lexer::new(src).tokenize().expect("lex error");
        let mut parser = Parser::new(tokens, String::new());
        let expr = parser.parse_expr_inner(0);
        assert!(
            parser.diagnostics.is_empty(),
            "unexpected errors: {:?}",
            parser
                .diagnostics
                .iter()
                .map(|e| &e.message)
                .collect::<Vec<_>>()
        );
        expr
    }

    #[test]
    fn precedence_add_mul() {
        let expr = parse("1 + 2 * 3");
        match &expr.kind {
            ExprKind::BinaryOp {
                op: BinOp::Add,
                lhs,
                rhs,
            } => {
                assert!(matches!(lhs.kind, ExprKind::Lit(Lit::Int(1))));
                match &rhs.kind {
                    ExprKind::BinaryOp {
                        op: BinOp::Mul,
                        lhs: l2,
                        rhs: r2,
                    } => {
                        assert!(matches!(l2.kind, ExprKind::Lit(Lit::Int(2))));
                        assert!(matches!(r2.kind, ExprKind::Lit(Lit::Int(3))));
                    }
                    _ => panic!("expected mul on rhs"),
                }
            }
            _ => panic!("expected add at top level"),
        }
    }

    #[test]
    fn precedence_mul_add() {
        let expr = parse("2 * 3 + 1");
        match &expr.kind {
            ExprKind::BinaryOp {
                op: BinOp::Add,
                lhs,
                rhs,
            } => {
                match &lhs.kind {
                    ExprKind::BinaryOp { op: BinOp::Mul, .. } => {}
                    _ => panic!("expected mul on lhs"),
                }
                assert!(matches!(rhs.kind, ExprKind::Lit(Lit::Int(1))));
            }
            _ => panic!("expected add at top level"),
        }
    }

    #[test]
    fn unary_neg() {
        let expr = parse_no_errors("-x");
        assert!(matches!(
            &expr.kind,
            ExprKind::UnaryOp { op: UnOp::Neg, .. }
        ));
    }

    #[test]
    fn unary_not() {
        let expr = parse_no_errors("!b");
        assert!(matches!(
            &expr.kind,
            ExprKind::UnaryOp { op: UnOp::Not, .. }
        ));
    }

    #[test]
    fn function_call() {
        let expr = parse_no_errors("f(a, b)");
        match &expr.kind {
            ExprKind::Call { callee, args } => {
                assert!(matches!(&callee.kind, ExprKind::Ident(n) if n == "f"));
                assert_eq!(args.len(), 2);
                assert!(matches!(&args[0].kind, ExprKind::Ident(n) if n == "a"));
                assert!(matches!(&args[1].kind, ExprKind::Ident(n) if n == "b"));
            }
            _ => panic!("expected Call"),
        }
    }

    #[test]
    fn method_call() {
        let expr = parse_no_errors("xs.len()");
        match &expr.kind {
            ExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                assert!(matches!(&receiver.kind, ExprKind::Ident(n) if n == "xs"));
                assert_eq!(method, "len");
                assert!(args.is_empty());
            }
            _ => panic!("expected MethodCall"),
        }
    }

    #[test]
    fn field_access() {
        let expr = parse_no_errors("s.field");
        match &expr.kind {
            ExprKind::FieldAccess { base, field } => {
                assert!(matches!(&base.kind, ExprKind::Ident(n) if n == "s"));
                assert_eq!(field, "field");
            }
            _ => panic!("expected FieldAccess"),
        }
    }

    #[test]
    fn if_else() {
        let expr = parse_no_errors("if x { 1 } else { 2 }");
        match &expr.kind {
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                assert!(matches!(&condition.kind, ExprKind::Ident(n) if n == "x"));
                assert!(else_branch.is_some());
                let _ = then_branch;
            }
            _ => panic!("expected If"),
        }
    }

    #[test]
    fn if_no_else() {
        let expr = parse_no_errors("if cond { 42 }");
        match &expr.kind {
            ExprKind::If { else_branch, .. } => {
                assert!(else_branch.is_none());
            }
            _ => panic!("expected If"),
        }
    }

    #[test]
    fn while_loop() {
        let expr = parse_no_errors("while x { 1 }");
        match &expr.kind {
            ExprKind::While {
                condition,
                vow,
                body,
            } => {
                assert!(matches!(&condition.kind, ExprKind::Ident(n) if n == "x"));
                assert!(vow.is_none());
                let _ = body;
            }
            _ => panic!("expected While"),
        }
    }

    #[test]
    fn loop_expr() {
        let expr = parse_no_errors("loop { 1 }");
        assert!(matches!(&expr.kind, ExprKind::Loop { vow: None, .. }));
    }

    #[test]
    fn match_expr() {
        let expr = parse_no_errors("match x { 1 => 2, 3 => 4 }");
        match &expr.kind {
            ExprKind::Match { scrutinee, arms } => {
                assert!(matches!(&scrutinee.kind, ExprKind::Ident(n) if n == "x"));
                assert_eq!(arms.len(), 2);
            }
            _ => panic!("expected Match"),
        }
    }

    #[test]
    fn question_postfix() {
        let expr = parse_no_errors("foo?");
        match &expr.kind {
            ExprKind::Question { expr } => {
                assert!(matches!(&expr.kind, ExprKind::Ident(n) if n == "foo"));
            }
            _ => panic!("expected Question"),
        }
    }

    #[test]
    fn index_expr() {
        let expr = parse_no_errors("arr[i]");
        match &expr.kind {
            ExprKind::Index { base, index } => {
                assert!(matches!(&base.kind, ExprKind::Ident(n) if n == "arr"));
                assert!(matches!(&index.kind, ExprKind::Ident(n) if n == "i"));
            }
            _ => panic!("expected Index"),
        }
    }

    #[test]
    fn bool_literal_true() {
        let expr = parse_no_errors("true");
        assert!(matches!(&expr.kind, ExprKind::Lit(Lit::Bool(true))));
    }

    #[test]
    fn bool_literal_false() {
        let expr = parse_no_errors("false");
        assert!(matches!(&expr.kind, ExprKind::Lit(Lit::Bool(false))));
    }

    #[test]
    fn string_literal() {
        let expr = parse_no_errors("\"hello\"");
        assert!(matches!(&expr.kind, ExprKind::Lit(Lit::String(s)) if s == "hello"));
    }

    #[test]
    fn float_literal() {
        let expr = parse_no_errors("3.14");
        assert!(matches!(&expr.kind, ExprKind::Lit(Lit::Float(_))));
    }

    #[test]
    fn checked_arithmetic() {
        let expr = parse_no_errors("a +! b");
        assert!(matches!(
            &expr.kind,
            ExprKind::BinaryOp {
                op: BinOp::AddChecked,
                ..
            }
        ));
    }

    #[test]
    fn logical_and_or_precedence() {
        let expr = parse("a || b && c");
        match &expr.kind {
            ExprKind::BinaryOp {
                op: BinOp::Or, rhs, ..
            } => {
                assert!(matches!(
                    &rhs.kind,
                    ExprKind::BinaryOp { op: BinOp::And, .. }
                ));
            }
            _ => panic!("expected Or at top"),
        }
    }

    #[test]
    fn comparison_ops() {
        for src in ["a == b", "a != b", "a < b", "a <= b", "a > b", "a >= b"] {
            let expr = parse(src);
            assert!(
                matches!(&expr.kind, ExprKind::BinaryOp { .. }),
                "failed for: {}",
                src
            );
        }
    }

    #[test]
    fn borrow_expr() {
        let expr = parse_no_errors("&x");
        assert!(matches!(&expr.kind, ExprKind::Borrow { .. }));
    }

    #[test]
    fn assign_expr() {
        let expr = parse_no_errors("a = b");
        assert!(matches!(&expr.kind, ExprKind::Assign { .. }));
    }

    #[test]
    fn tuple_expr() {
        let expr = parse_no_errors("(1, 2, 3)");
        match &expr.kind {
            ExprKind::Tuple(elems) => assert_eq!(elems.len(), 3),
            _ => panic!("expected Tuple"),
        }
    }

    #[test]
    fn paren_unwrap() {
        let expr = parse_no_errors("(42)");
        assert!(matches!(&expr.kind, ExprKind::Lit(Lit::Int(42))));
    }

    #[test]
    fn return_with_value() {
        let expr = parse_no_errors("return 42");
        match &expr.kind {
            ExprKind::Return { value: Some(v) } => {
                assert!(matches!(&v.kind, ExprKind::Lit(Lit::Int(42))));
            }
            _ => panic!("expected Return with value"),
        }
    }

    #[test]
    fn break_no_value() {
        let tokens = crate::lexer::Lexer::new("break")
            .tokenize()
            .expect("lex error");
        let mut parser = Parser::new(tokens, String::new());
        let expr = parser.parse_expr_inner(0);
        assert!(matches!(&expr.kind, ExprKind::Break { value: None }));
    }

    #[test]
    fn if_else_if() {
        let expr = parse_no_errors("if a { 1 } else if b { 2 } else { 3 }");
        match &expr.kind {
            ExprKind::If {
                else_branch: Some(e),
                ..
            } => {
                assert!(matches!(&e.kind, ExprKind::If { .. }));
            }
            _ => panic!("expected If-else-if chain"),
        }
    }

    #[test]
    fn unit_tuple() {
        let expr = parse_no_errors("()");
        assert!(matches!(&expr.kind, ExprKind::Tuple(elems) if elems.is_empty()));
    }

    #[test]
    fn block_expr() {
        let block = parse_block_from_source("{ 1 + 2 }");
        assert!(block.trailing_expr.is_some());
        match block.trailing_expr.as_ref().unwrap().kind {
            ExprKind::BinaryOp { op: BinOp::Add, .. } => {}
            _ => panic!("expected add expr"),
        }
    }

    #[test]
    fn while_with_vow() {
        let expr = parse_no_errors("while x vow { invariant: x } { 1 }");
        match &expr.kind {
            ExprKind::While { vow, .. } => {
                assert!(vow.is_some());
            }
            _ => panic!("expected While"),
        }
    }

    #[test]
    fn loop_with_vow() {
        let expr = parse_no_errors("loop vow { invariant: true } { break }");
        match &expr.kind {
            ExprKind::Loop { vow, .. } => {
                assert!(vow.is_some());
            }
            _ => panic!("expected Loop"),
        }
    }
}
