use crate::ast::{
    Block, Effect, Expr, ExprKind, FnDef, GenericParam, Item, Lit, Module, Param, Pat, PatKind,
    Stmt, Type, UseDecl, Visibility, VowBlock, VowClause,
};
use crate::lexer::Lexer;
use crate::span::Span;
use crate::token::{Token, TokenKind};
use vow_diag::{Blame, Diagnostic, ErrorCode, Severity, SourceLocation};

fn keyword_as_str(kind: &TokenKind) -> Option<&'static str> {
    match kind {
        TokenKind::KwFn => Some("fn"),
        TokenKind::KwLet => Some("let"),
        TokenKind::KwMut => Some("mut"),
        TokenKind::KwStruct => Some("struct"),
        TokenKind::KwEnum => Some("enum"),
        TokenKind::KwMatch => Some("match"),
        TokenKind::KwIf => Some("if"),
        TokenKind::KwElse => Some("else"),
        TokenKind::KwWhile => Some("while"),
        TokenKind::KwLoop => Some("loop"),
        TokenKind::KwBreak => Some("break"),
        TokenKind::KwReturn => Some("return"),
        TokenKind::KwPub => Some("pub"),
        TokenKind::KwUse => Some("use"),
        TokenKind::KwModule => Some("module"),
        TokenKind::KwVow => Some("vow"),
        TokenKind::KwRequires => Some("requires"),
        TokenKind::KwEnsures => Some("ensures"),
        TokenKind::KwInvariant => Some("invariant"),
        TokenKind::KwWhere => Some("where"),
        TokenKind::KwRegion => Some("region"),
        TokenKind::KwLinear => Some("linear"),
        TokenKind::KwExtern => Some("extern"),
        TokenKind::KwImpl => Some("impl"),
        TokenKind::KwTrait => Some("trait"),
        TokenKind::KwType => Some("type"),
        TokenKind::KwFor => Some("for"),
        TokenKind::KwIn => Some("in"),
        TokenKind::KwAs => Some("as"),
        TokenKind::KwRead => Some("read"),
        TokenKind::KwWrite => Some("write"),
        TokenKind::KwIO => Some("io"),
        TokenKind::KwPanic => Some("panic"),
        TokenKind::KwUnsafe => Some("unsafe"),
        _ => None,
    }
}

struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
    #[allow(dead_code)]
    source: String,
    diagnostics: Vec<Diagnostic>,
}

impl Parser {
    fn new(tokens: Vec<Token>, source: String) -> Self {
        Self {
            tokens,
            cursor: 0,
            source,
            diagnostics: Vec::new(),
        }
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.cursor]
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.tokens[self.cursor].kind
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.cursor];
        if self.cursor + 1 < self.tokens.len() {
            self.cursor += 1;
        }
        tok
    }

    fn at(&self, kind: &TokenKind) -> bool {
        self.peek_kind() == kind
    }

    fn at_end(&self) -> bool {
        matches!(self.peek_kind(), TokenKind::Eof)
    }

    fn expect(&mut self, kind: TokenKind) -> Option<Span> {
        if self.peek_kind() == &kind {
            let span = self.peek().span;
            self.advance();
            Some(span)
        } else {
            let got = self.peek().span;
            self.push_error(
                ErrorCode::UnexpectedToken,
                format!(
                    "expected {:?}, found {:?}",
                    kind,
                    self.peek_kind()
                ),
                got,
            );
            None
        }
    }

    fn expect_ident(&mut self) -> Option<(String, Span)> {
        match self.peek_kind().clone() {
            TokenKind::Ident(name) => {
                let span = self.peek().span;
                self.advance();
                Some((name, span))
            }
            _ => {
                let got = self.peek().span;
                self.push_error(
                    ErrorCode::UnexpectedToken,
                    format!("expected identifier, found {:?}", self.peek_kind()),
                    got,
                );
                None
            }
        }
    }

    fn expect_name_or_keyword(&mut self) -> Option<(String, Span)> {
        let span = self.peek().span;
        let name = keyword_as_str(self.peek_kind()).map(|s| s.to_string()).or_else(|| {
            if let TokenKind::Ident(n) = self.peek_kind() {
                Some(n.clone())
            } else {
                None
            }
        });
        match name {
            Some(n) => {
                self.advance();
                Some((n, span))
            }
            None => {
                self.push_error(
                    ErrorCode::UnexpectedToken,
                    format!("expected name, found {:?}", self.peek_kind()),
                    span,
                );
                None
            }
        }
    }

    fn push_error(&mut self, code: ErrorCode, message: String, span: Span) {
        self.diagnostics.push(Diagnostic {
            severity: Severity::Error,
            code,
            message,
            primary: SourceLocation {
                file: "<input>".to_string(),
                byte_offset: span.start,
                byte_len: span.len,
            },
            secondary: vec![],
            blame: Blame::None,
        });
    }

    fn current_span(&self) -> Span {
        self.peek().span
    }

    fn parse_module_inner(&mut self) -> Module {
        let start = self.current_span();

        self.expect(TokenKind::KwModule);
        let name = self
            .expect_ident()
            .map(|(n, _)| n)
            .unwrap_or_else(|| "<error>".to_string());

        let mut uses = Vec::new();
        while self.at(&TokenKind::KwUse) {
            if let Some(u) = self.parse_use() {
                uses.push(u);
            }
        }

        let mut items = Vec::new();
        while !self.at_end() {
            if let Some(item) = self.parse_item() {
                items.push(item);
            } else {
                break;
            }
        }

        let end = self.current_span();
        Module {
            name,
            uses,
            items,
            span: start.merge(end),
        }
    }

    fn parse_use(&mut self) -> Option<UseDecl> {
        let start = self.current_span();
        self.expect(TokenKind::KwUse)?;

        let mut path = Vec::new();
        let (first, _) = self.expect_name_or_keyword()?;
        path.push(first);

        while self.at(&TokenKind::Dot) {
            self.advance();
            let (segment, _) = self.expect_name_or_keyword()?;
            path.push(segment);
        }

        let end = self.current_span();
        Some(UseDecl {
            path,
            span: start.merge(end),
        })
    }

    fn parse_item(&mut self) -> Option<Item> {
        let vis = if self.at(&TokenKind::KwPub) {
            self.advance();
            Visibility::Public
        } else {
            Visibility::Private
        };

        match self.peek_kind() {
            TokenKind::KwFn => Some(Item::Fn(self.parse_fn(vis)?)),
            TokenKind::KwStruct
            | TokenKind::KwEnum
            | TokenKind::KwTrait
            | TokenKind::KwImpl
            | TokenKind::KwType
            | TokenKind::KwExtern
            | TokenKind::KwLinear => {
                let span = self.current_span();
                self.push_error(
                    ErrorCode::UnexpectedToken,
                    format!(
                        "item kind {:?} not yet implemented in this parser phase",
                        self.peek_kind()
                    ),
                    span,
                );
                None
            }
            _ => {
                let span = self.current_span();
                self.push_error(
                    ErrorCode::UnexpectedToken,
                    format!("expected item, found {:?}", self.peek_kind()),
                    span,
                );
                None
            }
        }
    }

    fn parse_fn(&mut self, vis: Visibility) -> Option<FnDef> {
        let start = self.current_span();
        self.expect(TokenKind::KwFn)?;

        let (name, _) = self.expect_ident()?;

        let generics = self.parse_generics();

        self.expect(TokenKind::LParen)?;
        let mut params = Vec::new();
        while !self.at(&TokenKind::RParen) && !self.at_end() {
            if let Some(p) = self.parse_param() {
                params.push(p);
            } else {
                break;
            }
            if self.at(&TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        self.expect(TokenKind::RParen)?;

        let return_ty = if self.at(&TokenKind::ThinArrow) {
            self.advance();
            self.parse_type()?
        } else {
            Type::Unit {
                span: self.current_span(),
            }
        };

        let effects = self.parse_effects();

        let vow = if self.at(&TokenKind::KwVow) {
            self.parse_vow_block()
        } else {
            None
        };

        let body = self.parse_block()?;

        let end = body.span;
        Some(FnDef {
            vis,
            name,
            generics,
            params,
            return_ty,
            effects,
            vow,
            body,
            span: start.merge(end),
        })
    }

    fn parse_param(&mut self) -> Option<Param> {
        let start = self.current_span();
        let (name, _) = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        let ty = self.parse_type()?;

        let refinement = if self.at(&TokenKind::KwWhere) {
            self.advance();
            let expr = self.parse_expr()?;
            Some(Box::new(expr))
        } else {
            None
        };

        let end = self.current_span();
        Some(Param {
            name,
            ty,
            refinement,
            span: start.merge(end),
        })
    }

    fn parse_generics(&mut self) -> Vec<GenericParam> {
        if !self.at(&TokenKind::Lt) {
            return Vec::new();
        }
        self.advance();

        let mut params = Vec::new();
        while !self.at(&TokenKind::Gt) && !self.at_end() {
            let start = self.current_span();
            let (name, name_span) = match self.expect_ident() {
                Some(n) => n,
                None => break,
            };

            let mut bounds = Vec::new();
            if self.at(&TokenKind::Colon) {
                self.advance();
                if let Some((bound, _)) = self.expect_ident() {
                    bounds.push(bound);
                }
                while self.at(&TokenKind::Plus) {
                    self.advance();
                    if let Some((bound, _)) = self.expect_ident() {
                        bounds.push(bound);
                    }
                }
            }

            let end = self.current_span();
            params.push(GenericParam {
                name,
                bounds,
                span: start.merge(name_span).merge(end),
            });

            if self.at(&TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }

        self.expect(TokenKind::Gt);
        params
    }

    fn parse_effects(&mut self) -> Vec<Effect> {
        if !self.at(&TokenKind::LBracket) {
            return Vec::new();
        }
        self.advance();

        let mut effects = Vec::new();
        while !self.at(&TokenKind::RBracket) && !self.at_end() {
            let effect = match self.peek_kind() {
                TokenKind::KwRead => Some(Effect::Read),
                TokenKind::KwWrite => Some(Effect::Write),
                TokenKind::KwIO => Some(Effect::IO),
                TokenKind::KwPanic => Some(Effect::Panic),
                TokenKind::KwUnsafe => Some(Effect::Unsafe),
                _ => {
                    let span = self.current_span();
                    self.push_error(
                        ErrorCode::UnexpectedToken,
                        format!("expected effect keyword, found {:?}", self.peek_kind()),
                        span,
                    );
                    None
                }
            };
            self.advance();
            if let Some(e) = effect {
                effects.push(e);
            }
            if self.at(&TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }

        self.expect(TokenKind::RBracket);
        effects
    }

    fn parse_vow_block(&mut self) -> Option<VowBlock> {
        let start = self.current_span();
        self.expect(TokenKind::KwVow)?;
        self.expect(TokenKind::LBrace)?;

        let mut clauses = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_end() {
            let clause_start = self.current_span();
            match self.peek_kind() {
                TokenKind::KwRequires => {
                    self.advance();
                    self.expect(TokenKind::Colon);
                    if let Some(expr) = self.parse_expr() {
                        let clause_end = expr.span;
                        clauses.push(VowClause::Requires {
                            expr,
                            span: clause_start.merge(clause_end),
                        });
                    }
                }
                TokenKind::KwEnsures => {
                    self.advance();
                    self.expect(TokenKind::Colon);
                    if let Some(expr) = self.parse_expr() {
                        let clause_end = expr.span;
                        clauses.push(VowClause::Ensures {
                            expr,
                            span: clause_start.merge(clause_end),
                        });
                    }
                }
                TokenKind::KwInvariant => {
                    self.advance();
                    self.expect(TokenKind::Colon);
                    if let Some(expr) = self.parse_expr() {
                        let clause_end = expr.span;
                        clauses.push(VowClause::Invariant {
                            expr,
                            span: clause_start.merge(clause_end),
                        });
                    }
                }
                _ => {
                    let span = self.current_span();
                    self.push_error(
                        ErrorCode::UnexpectedToken,
                        format!(
                            "expected requires, ensures, or invariant, found {:?}",
                            self.peek_kind()
                        ),
                        span,
                    );
                    break;
                }
            }
            if self.at(&TokenKind::Comma) {
                self.advance();
            }
        }

        let end = self.current_span();
        self.expect(TokenKind::RBrace)?;
        Some(VowBlock {
            clauses,
            span: start.merge(end),
        })
    }

    fn parse_block(&mut self) -> Option<Block> {
        let start = self.current_span();
        self.expect(TokenKind::LBrace)?;

        let mut stmts = Vec::new();
        let mut trailing_expr: Option<Box<Expr>> = None;

        while !self.at(&TokenKind::RBrace) && !self.at_end() {
            if self.at(&TokenKind::KwLet) {
                if let Some(stmt) = self.parse_let_stmt() {
                    stmts.push(stmt);
                } else {
                    break;
                }
            } else {
                let expr_start = self.current_span();
                if let Some(expr) = self.parse_expr() {
                    if self.at(&TokenKind::Semicolon) {
                        let semi_span = self.current_span();
                        self.advance();
                        stmts.push(Stmt::Expr {
                            span: expr_start.merge(semi_span),
                            expr,
                            has_semicolon: true,
                        });
                    } else if self.at(&TokenKind::RBrace) {
                        trailing_expr = Some(Box::new(expr));
                        break;
                    } else {
                        stmts.push(Stmt::Expr {
                            span: expr_start,
                            expr,
                            has_semicolon: false,
                        });
                        break;
                    }
                } else {
                    break;
                }
            }
        }

        let end = self.current_span();
        self.expect(TokenKind::RBrace)?;
        Some(Block {
            stmts,
            trailing_expr,
            span: start.merge(end),
        })
    }

    fn parse_let_stmt(&mut self) -> Option<Stmt> {
        let start = self.current_span();
        self.expect(TokenKind::KwLet)?;

        let is_mut = if self.at(&TokenKind::KwMut) {
            self.advance();
            true
        } else {
            false
        };

        let (name, name_span) = self.expect_ident()?;
        let pattern = Pat {
            kind: PatKind::Ident { name, is_mut },
            span: name_span,
        };

        let ty = if self.at(&TokenKind::Colon) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };

        self.expect(TokenKind::Eq)?;
        let init = self.parse_expr()?;
        let end = if self.at(&TokenKind::Semicolon) {
            let s = self.current_span();
            self.advance();
            s
        } else {
            init.span
        };

        Some(Stmt::Let {
            pattern,
            ty,
            init: Box::new(init),
            span: start.merge(end),
        })
    }

    fn parse_type(&mut self) -> Option<Type> {
        let start = self.current_span();
        match self.peek_kind().clone() {
            TokenKind::Ident(name) => {
                self.advance();
                let name_span = start;
                if self.at(&TokenKind::Lt) {
                    self.advance();
                    let mut args = Vec::new();
                    while !self.at(&TokenKind::Gt) && !self.at_end() {
                        if let Some(arg) = self.parse_type() {
                            args.push(arg);
                        } else {
                            break;
                        }
                        if self.at(&TokenKind::Comma) {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    let end = self.current_span();
                    self.expect(TokenKind::Gt);
                    Some(Type::Generic {
                        name,
                        args,
                        span: name_span.merge(end),
                    })
                } else {
                    Some(Type::Named {
                        name,
                        span: name_span,
                    })
                }
            }
            TokenKind::Amp => {
                self.advance();
                let inner = self.parse_type()?;
                let end = inner.span();
                Some(Type::Reference {
                    inner: Box::new(inner),
                    span: start.merge(end),
                })
            }
            TokenKind::LParen => {
                self.advance();
                if self.at(&TokenKind::RParen) {
                    let end = self.current_span();
                    self.advance();
                    Some(Type::Unit {
                        span: start.merge(end),
                    })
                } else {
                    let mut elems = Vec::new();
                    while !self.at(&TokenKind::RParen) && !self.at_end() {
                        if let Some(t) = self.parse_type() {
                            elems.push(t);
                        } else {
                            break;
                        }
                        if self.at(&TokenKind::Comma) {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    let end = self.current_span();
                    self.expect(TokenKind::RParen);
                    Some(Type::Tuple {
                        elems,
                        span: start.merge(end),
                    })
                }
            }
            TokenKind::Bang => {
                self.advance();
                Some(Type::Never { span: start })
            }
            _ => {
                self.push_error(
                    ErrorCode::UnexpectedToken,
                    format!("expected type, found {:?}", self.peek_kind()),
                    start,
                );
                None
            }
        }
    }

    fn parse_expr(&mut self) -> Option<Expr> {
        let span = self.current_span();
        match self.peek_kind().clone() {
            TokenKind::LitInt(v) => {
                self.advance();
                Some(Expr {
                    kind: ExprKind::Lit(Lit::Int(v)),
                    span,
                })
            }
            TokenKind::LitIntSuffixed { value, .. } => {
                self.advance();
                Some(Expr {
                    kind: ExprKind::Lit(Lit::Int(value)),
                    span,
                })
            }
            TokenKind::LitFloat(v) => {
                self.advance();
                Some(Expr {
                    kind: ExprKind::Lit(Lit::Float(v)),
                    span,
                })
            }
            TokenKind::LitBool(v) => {
                self.advance();
                Some(Expr {
                    kind: ExprKind::Lit(Lit::Bool(v)),
                    span,
                })
            }
            TokenKind::LitString(s) => {
                self.advance();
                Some(Expr {
                    kind: ExprKind::Lit(Lit::String(s)),
                    span,
                })
            }
            TokenKind::Ident(name) => {
                self.advance();
                Some(Expr {
                    kind: ExprKind::Ident(name),
                    span,
                })
            }
            _ => {
                self.push_error(
                    ErrorCode::UnexpectedToken,
                    format!("expected expression, found {:?}", self.peek_kind()),
                    span,
                );
                None
            }
        }
    }
}

pub fn parse_module(source: &str) -> (Module, Vec<Diagnostic>) {
    let tokens = match Lexer::new(source).tokenize() {
        Ok(toks) => toks,
        Err(lex_err) => {
            let diag = Diagnostic {
                severity: Severity::Error,
                code: ErrorCode::InvalidCharacter,
                message: lex_err.message,
                primary: SourceLocation {
                    file: "<input>".to_string(),
                    byte_offset: lex_err.span.start,
                    byte_len: lex_err.span.len,
                },
                secondary: vec![],
                blame: Blame::None,
            };
            let module = Module {
                name: "<error>".to_string(),
                uses: vec![],
                items: vec![],
                span: Span::new(0, 0),
            };
            return (module, vec![diag]);
        }
    };

    let mut parser = Parser::new(tokens, source.to_string());
    let module = parser.parse_module_inner();
    (module, parser.diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_module_no_items() {
        let src = "module Foo";
        let (module, diags) = parse_module(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {:?}", diags);
        assert_eq!(module.name, "Foo");
        assert!(module.uses.is_empty());
        assert!(module.items.is_empty());
    }

    #[test]
    fn parse_module_single_pure_fn() {
        let src = "module Bar fn add(x: i32, y: i32) -> i32 { x }";
        let (module, diags) = parse_module(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {:?}", diags);
        assert_eq!(module.name, "Bar");
        assert_eq!(module.items.len(), 1);
        match &module.items[0] {
            Item::Fn(f) => {
                assert_eq!(f.name, "add");
                assert_eq!(f.params.len(), 2);
                assert_eq!(f.params[0].name, "x");
                assert_eq!(f.params[1].name, "y");
                assert!(f.effects.is_empty());
                assert!(f.vow.is_none());
            }
            _ => panic!("expected Fn item"),
        }
    }

    #[test]
    fn parse_fn_with_effects() {
        let src = "module Baz fn do_io() [read, write] { 0 }";
        let (module, diags) = parse_module(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {:?}", diags);
        assert_eq!(module.items.len(), 1);
        match &module.items[0] {
            Item::Fn(f) => {
                assert_eq!(f.name, "do_io");
                assert!(f.effects.contains(&Effect::Read));
                assert!(f.effects.contains(&Effect::Write));
            }
            _ => panic!("expected Fn item"),
        }
    }

    #[test]
    fn parse_fn_with_vow_block() {
        let src = "module Qux fn safe_div(x: i32, y: i32) -> i32 vow { requires: y, ensures: result } { x }";
        let (module, diags) = parse_module(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {:?}", diags);
        assert_eq!(module.items.len(), 1);
        match &module.items[0] {
            Item::Fn(f) => {
                assert_eq!(f.name, "safe_div");
                let vow = f.vow.as_ref().expect("expected vow block");
                assert_eq!(vow.clauses.len(), 2);
                assert!(matches!(vow.clauses[0], VowClause::Requires { .. }));
                assert!(matches!(vow.clauses[1], VowClause::Ensures { .. }));
            }
            _ => panic!("expected Fn item"),
        }
    }

    #[test]
    fn parse_module_with_use() {
        let src = "module M use std.io fn f() { 0 }";
        let (module, diags) = parse_module(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {:?}", diags);
        assert_eq!(module.uses.len(), 1);
        assert_eq!(module.uses[0].path, vec!["std", "io"]);
    }
}
