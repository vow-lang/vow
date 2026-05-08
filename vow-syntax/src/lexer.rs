use crate::span::Span;
use crate::token::{IntSuffix, Token, TokenKind};

#[derive(Debug)]
pub struct LexError {
    pub message: String,
    pub span: Span,
}

pub struct Lexer<'src> {
    src: &'src str,
    pos: usize,
}

impl<'src> Lexer<'src> {
    pub fn new(src: &'src str) -> Self {
        Self { src, pos: 0 }
    }

    pub fn tokenize(mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace();
            if self.pos >= self.src.len() {
                tokens.push(Token::new(TokenKind::Eof, Span::new(self.pos as u32, 0)));
                break;
            }
            let token = self.next_token()?;
            tokens.push(token);
        }
        Ok(tokens)
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.src.len() {
            match self.current_byte() {
                b' ' | b'\t' | b'\n' | b'\r' => self.pos += 1,
                b'/' if self.peek_byte(1) == Some(b'/') => {
                    self.pos += 2;
                    while self.pos < self.src.len() && self.current_byte() != b'\n' {
                        self.pos += 1;
                    }
                }
                _ => break,
            }
        }
    }

    fn current_byte(&self) -> u8 {
        self.src.as_bytes()[self.pos]
    }

    fn peek_byte(&self, offset: usize) -> Option<u8> {
        self.src.as_bytes().get(self.pos + offset).copied()
    }

    fn next_token(&mut self) -> Result<Token, LexError> {
        let start = self.pos;
        let b = self.current_byte();

        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => self.lex_ident_or_keyword(start),
            b'0'..=b'9' => self.lex_number(start),
            b'"' => self.lex_string(start),
            b'+' => {
                if self.peek_byte(1) == Some(b'!') {
                    self.pos += 2;
                    Ok(Token::new(
                        TokenKind::PlusChecked,
                        Span::new(start as u32, 2),
                    ))
                } else {
                    self.pos += 1;
                    Ok(Token::new(TokenKind::Plus, Span::new(start as u32, 1)))
                }
            }
            b'-' => {
                if self.peek_byte(1) == Some(b'!') {
                    self.pos += 2;
                    Ok(Token::new(
                        TokenKind::MinusChecked,
                        Span::new(start as u32, 2),
                    ))
                } else if self.peek_byte(1) == Some(b'>') {
                    self.pos += 2;
                    Ok(Token::new(TokenKind::ThinArrow, Span::new(start as u32, 2)))
                } else {
                    self.pos += 1;
                    Ok(Token::new(TokenKind::Minus, Span::new(start as u32, 1)))
                }
            }
            b'*' => {
                if self.peek_byte(1) == Some(b'!') {
                    self.pos += 2;
                    Ok(Token::new(
                        TokenKind::StarChecked,
                        Span::new(start as u32, 2),
                    ))
                } else {
                    self.pos += 1;
                    Ok(Token::new(TokenKind::Star, Span::new(start as u32, 1)))
                }
            }
            b'/' => {
                if self.peek_byte(1) == Some(b'!') {
                    self.pos += 2;
                    Ok(Token::new(
                        TokenKind::SlashChecked,
                        Span::new(start as u32, 2),
                    ))
                } else {
                    self.pos += 1;
                    Ok(Token::new(TokenKind::Slash, Span::new(start as u32, 1)))
                }
            }
            b'%' => {
                if self.peek_byte(1) == Some(b'!') {
                    self.pos += 2;
                    Ok(Token::new(
                        TokenKind::PercentChecked,
                        Span::new(start as u32, 2),
                    ))
                } else {
                    self.pos += 1;
                    Ok(Token::new(TokenKind::Percent, Span::new(start as u32, 1)))
                }
            }
            b'=' => {
                if self.peek_byte(1) == Some(b'=') {
                    self.pos += 2;
                    Ok(Token::new(TokenKind::EqEq, Span::new(start as u32, 2)))
                } else if self.peek_byte(1) == Some(b'>') {
                    self.pos += 2;
                    Ok(Token::new(TokenKind::FatArrow, Span::new(start as u32, 2)))
                } else {
                    self.pos += 1;
                    Ok(Token::new(TokenKind::Eq, Span::new(start as u32, 1)))
                }
            }
            b'!' => {
                if self.peek_byte(1) == Some(b'=') {
                    self.pos += 2;
                    Ok(Token::new(TokenKind::BangEq, Span::new(start as u32, 2)))
                } else {
                    self.pos += 1;
                    Ok(Token::new(TokenKind::Bang, Span::new(start as u32, 1)))
                }
            }
            b'<' => {
                if self.peek_byte(1) == Some(b'=') {
                    self.pos += 2;
                    Ok(Token::new(TokenKind::LtEq, Span::new(start as u32, 2)))
                } else {
                    self.pos += 1;
                    Ok(Token::new(TokenKind::Lt, Span::new(start as u32, 1)))
                }
            }
            b'>' => {
                if self.peek_byte(1) == Some(b'=') {
                    self.pos += 2;
                    Ok(Token::new(TokenKind::GtEq, Span::new(start as u32, 2)))
                } else {
                    self.pos += 1;
                    Ok(Token::new(TokenKind::Gt, Span::new(start as u32, 1)))
                }
            }
            b'&' => {
                if self.peek_byte(1) == Some(b'&') {
                    self.pos += 2;
                    Ok(Token::new(TokenKind::AmpAmp, Span::new(start as u32, 2)))
                } else {
                    self.pos += 1;
                    Ok(Token::new(TokenKind::Amp, Span::new(start as u32, 1)))
                }
            }
            b'|' => {
                if self.peek_byte(1) == Some(b'|') {
                    self.pos += 2;
                    Ok(Token::new(TokenKind::PipePipe, Span::new(start as u32, 2)))
                } else {
                    self.pos += 1;
                    Ok(Token::new(TokenKind::Pipe, Span::new(start as u32, 1)))
                }
            }
            b':' => {
                if self.peek_byte(1) == Some(b':') {
                    self.pos += 2;
                    Ok(Token::new(
                        TokenKind::ColonColon,
                        Span::new(start as u32, 2),
                    ))
                } else {
                    self.pos += 1;
                    Ok(Token::new(TokenKind::Colon, Span::new(start as u32, 1)))
                }
            }
            b'.' => {
                if self.peek_byte(1) == Some(b'.') {
                    self.pos += 2;
                    Ok(Token::new(TokenKind::DotDot, Span::new(start as u32, 2)))
                } else {
                    self.pos += 1;
                    Ok(Token::new(TokenKind::Dot, Span::new(start as u32, 1)))
                }
            }
            b'{' => {
                self.pos += 1;
                Ok(Token::new(TokenKind::LBrace, Span::new(start as u32, 1)))
            }
            b'}' => {
                self.pos += 1;
                Ok(Token::new(TokenKind::RBrace, Span::new(start as u32, 1)))
            }
            b'(' => {
                self.pos += 1;
                Ok(Token::new(TokenKind::LParen, Span::new(start as u32, 1)))
            }
            b')' => {
                self.pos += 1;
                Ok(Token::new(TokenKind::RParen, Span::new(start as u32, 1)))
            }
            b'[' => {
                self.pos += 1;
                Ok(Token::new(TokenKind::LBracket, Span::new(start as u32, 1)))
            }
            b']' => {
                self.pos += 1;
                Ok(Token::new(TokenKind::RBracket, Span::new(start as u32, 1)))
            }
            b',' => {
                self.pos += 1;
                Ok(Token::new(TokenKind::Comma, Span::new(start as u32, 1)))
            }
            b';' => {
                self.pos += 1;
                Ok(Token::new(TokenKind::Semicolon, Span::new(start as u32, 1)))
            }
            b'?' => {
                self.pos += 1;
                Ok(Token::new(TokenKind::Question, Span::new(start as u32, 1)))
            }
            b'^' => {
                self.pos += 1;
                Ok(Token::new(TokenKind::Caret, Span::new(start as u32, 1)))
            }
            _ => {
                self.pos += 1;
                Err(LexError {
                    message: format!("unexpected character '{}'", b as char),
                    span: Span::new(start as u32, 1),
                })
            }
        }
    }

    fn lex_ident_or_keyword(&mut self, start: usize) -> Result<Token, LexError> {
        while self.pos < self.src.len() {
            match self.current_byte() {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' => self.pos += 1,
                _ => break,
            }
        }
        let text = &self.src[start..self.pos];
        let len = (self.pos - start) as u32;
        let span = Span::new(start as u32, len);

        if text == "_" {
            return Ok(Token::new(TokenKind::Underscore, span));
        }

        if let Some(kw) = TokenKind::from_keyword(text) {
            return Ok(Token::new(kw, span));
        }

        Ok(Token::new(TokenKind::Ident(text.to_string()), span))
    }

    fn lex_number(&mut self, start: usize) -> Result<Token, LexError> {
        while self.pos < self.src.len() && self.current_byte().is_ascii_digit() {
            self.pos += 1;
        }

        // Check for float: digits followed by '.' followed by digits
        if self.peek_byte(0) == Some(b'.') && self.peek_byte(1).is_some_and(|b| b.is_ascii_digit())
        {
            self.pos += 1; // consume '.'
            while self.pos < self.src.len() && self.current_byte().is_ascii_digit() {
                self.pos += 1;
            }
            let text = &self.src[start..self.pos];
            let len = (self.pos - start) as u32;
            let span = Span::new(start as u32, len);
            let value: f64 = text.parse().map_err(|_| LexError {
                message: format!("invalid float literal '{}'", text),
                span,
            })?;
            return Ok(Token::new(TokenKind::LitFloat(value), span));
        }

        let digits = &self.src[start..self.pos];
        let int_value: i128 = digits.parse().map_err(|_| LexError {
            message: format!("integer literal '{}' out of range", digits),
            span: Span::new(start as u32, (self.pos - start) as u32),
        })?;

        // Check for type suffix
        let suffix_start = self.pos;
        if self.pos < self.src.len() {
            let suffix = self.try_lex_int_suffix();
            if let Some(suffix) = suffix {
                let len = (self.pos - start) as u32;
                let span = Span::new(start as u32, len);
                return Ok(Token::new(
                    TokenKind::LitIntSuffixed {
                        value: int_value,
                        suffix,
                    },
                    span,
                ));
            }
            // Reset position if no valid suffix was consumed
            self.pos = suffix_start;
        }

        let len = (self.pos - start) as u32;
        Ok(Token::new(
            TokenKind::LitInt(int_value),
            Span::new(start as u32, len),
        ))
    }

    fn try_lex_int_suffix(&mut self) -> Option<IntSuffix> {
        let rest = &self.src[self.pos..];
        // Order matters: longer suffixes first (i128 before i16, u128 before u16, etc.)
        let suffixes: &[(&str, IntSuffix)] = &[
            ("i128", IntSuffix::I128),
            ("i64", IntSuffix::I64),
            ("i32", IntSuffix::I32),
            ("i16", IntSuffix::I16),
            ("i8", IntSuffix::I8),
            ("u128", IntSuffix::U128),
            ("u64", IntSuffix::U64),
            ("u32", IntSuffix::U32),
            ("u16", IntSuffix::U16),
            ("u8", IntSuffix::U8),
            ("usize", IntSuffix::Usize),
            ("isize", IntSuffix::Isize),
        ];

        for (suffix_str, suffix) in suffixes {
            if let Some(stripped) = rest.strip_prefix(suffix_str) {
                let after = stripped.as_bytes().first().copied();
                if after.is_none_or(|b| !b.is_ascii_alphanumeric() && b != b'_') {
                    self.pos += suffix_str.len();
                    return Some(*suffix);
                }
            }
        }
        None
    }

    fn lex_string(&mut self, start: usize) -> Result<Token, LexError> {
        self.pos += 1; // consume opening '"'
        let mut value = String::new();
        loop {
            if self.pos >= self.src.len() {
                return Err(LexError {
                    message: "unterminated string literal".to_string(),
                    span: Span::new(start as u32, (self.pos - start) as u32),
                });
            }
            let b = self.current_byte();
            if b == b'"' {
                self.pos += 1; // consume closing '"'
                let len = (self.pos - start) as u32;
                return Ok(Token::new(
                    TokenKind::LitString(value),
                    Span::new(start as u32, len),
                ));
            }
            if b == b'\\' {
                self.pos += 1;
                if self.pos >= self.src.len() {
                    return Err(LexError {
                        message: "unterminated string escape".to_string(),
                        span: Span::new(start as u32, (self.pos - start) as u32),
                    });
                }
                let esc = self.current_byte();
                match esc {
                    b'n' => value.push('\n'),
                    b't' => value.push('\t'),
                    b'r' => value.push('\r'),
                    b'\\' => value.push('\\'),
                    b'"' => value.push('"'),
                    b'0' => value.push('\0'),
                    _ => {
                        value.push('\\');
                        value.push(esc as char);
                    }
                }
                self.pos += 1;
            } else {
                value.push(b as char);
                self.pos += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::TokenKind;

    fn lex(src: &str) -> Vec<TokenKind> {
        Lexer::new(src)
            .tokenize()
            .unwrap()
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn lex_keywords() {
        let kinds = lex("fn let mut struct");
        assert_eq!(kinds[0], TokenKind::KwFn);
        assert_eq!(kinds[1], TokenKind::KwLet);
        assert_eq!(kinds[2], TokenKind::KwMut);
        assert_eq!(kinds[3], TokenKind::KwStruct);
    }

    #[test]
    fn lex_checked_ops_vs_plain() {
        let kinds = lex("+! + -! -");
        assert_eq!(kinds[0], TokenKind::PlusChecked);
        assert_eq!(kinds[1], TokenKind::Plus);
        assert_eq!(kinds[2], TokenKind::MinusChecked);
        assert_eq!(kinds[3], TokenKind::Minus);
    }

    #[test]
    fn lex_integer_with_suffix() {
        let kinds = lex("42i32 100u64 0i128");
        assert_eq!(
            kinds[0],
            TokenKind::LitIntSuffixed {
                value: 42,
                suffix: IntSuffix::I32
            }
        );
        assert_eq!(
            kinds[1],
            TokenKind::LitIntSuffixed {
                value: 100,
                suffix: IntSuffix::U64
            }
        );
        assert_eq!(
            kinds[2],
            TokenKind::LitIntSuffixed {
                value: 0,
                suffix: IntSuffix::I128
            }
        );
    }

    #[test]
    fn lex_integer_no_suffix() {
        let kinds = lex("42 0 999");
        assert_eq!(kinds[0], TokenKind::LitInt(42));
        assert_eq!(kinds[1], TokenKind::LitInt(0));
        assert_eq!(kinds[2], TokenKind::LitInt(999));
    }

    #[test]
    #[allow(
        clippy::approx_constant,
        reason = "3.14 is the lexer input, not an approximation of PI"
    )]
    fn lex_float_literal() {
        let kinds = lex("3.14 0.0");
        assert!(matches!(kinds[0], TokenKind::LitFloat(f) if (f - 3.14).abs() < 1e-10));
        assert!(matches!(kinds[1], TokenKind::LitFloat(f) if f == 0.0));
    }

    #[test]
    fn lex_string_literal() {
        let kinds = lex("\"hello world\"");
        assert!(matches!(&kinds[0], TokenKind::LitString(s) if s == "hello world"));
    }

    #[test]
    fn lex_unterminated_string_is_error() {
        let result = Lexer::new("\"unterminated").tokenize();
        assert!(result.is_err());
    }

    #[test]
    fn lex_empty_input() {
        let kinds = lex("");
        assert_eq!(kinds, vec![TokenKind::Eof]);
    }

    #[test]
    fn lex_comparison_operators() {
        let kinds = lex("== != <= >= < >");
        assert_eq!(kinds[0], TokenKind::EqEq);
        assert_eq!(kinds[1], TokenKind::BangEq);
        assert_eq!(kinds[2], TokenKind::LtEq);
        assert_eq!(kinds[3], TokenKind::GtEq);
        assert_eq!(kinds[4], TokenKind::Lt);
        assert_eq!(kinds[5], TokenKind::Gt);
    }

    #[test]
    fn lex_bool_literals() {
        let kinds = lex("true false");
        assert_eq!(kinds[0], TokenKind::LitBool(true));
        assert_eq!(kinds[1], TokenKind::LitBool(false));
    }

    #[test]
    fn lex_underscore_vs_ident() {
        let kinds = lex("_ _foo _bar123");
        assert_eq!(kinds[0], TokenKind::Underscore);
        assert_eq!(kinds[1], TokenKind::Ident("_foo".to_string()));
        assert_eq!(kinds[2], TokenKind::Ident("_bar123".to_string()));
    }

    #[test]
    fn lex_all_checked_ops() {
        let kinds = lex("+! -! *! /! %!");
        assert_eq!(kinds[0], TokenKind::PlusChecked);
        assert_eq!(kinds[1], TokenKind::MinusChecked);
        assert_eq!(kinds[2], TokenKind::StarChecked);
        assert_eq!(kinds[3], TokenKind::SlashChecked);
        assert_eq!(kinds[4], TokenKind::PercentChecked);
    }

    #[test]
    fn lex_arrows() {
        let kinds = lex("-> =>");
        assert_eq!(kinds[0], TokenKind::ThinArrow);
        assert_eq!(kinds[1], TokenKind::FatArrow);
    }

    #[test]
    fn lex_colon_variants() {
        let kinds = lex(": ::");
        assert_eq!(kinds[0], TokenKind::Colon);
        assert_eq!(kinds[1], TokenKind::ColonColon);
    }

    #[test]
    fn lex_dot_variants() {
        let kinds = lex(". ..");
        assert_eq!(kinds[0], TokenKind::Dot);
        assert_eq!(kinds[1], TokenKind::DotDot);
    }

    #[test]
    fn lex_delimiters() {
        let kinds = lex("{ } ( ) [ ]");
        assert_eq!(kinds[0], TokenKind::LBrace);
        assert_eq!(kinds[1], TokenKind::RBrace);
        assert_eq!(kinds[2], TokenKind::LParen);
        assert_eq!(kinds[3], TokenKind::RParen);
        assert_eq!(kinds[4], TokenKind::LBracket);
        assert_eq!(kinds[5], TokenKind::RBracket);
    }

    #[test]
    fn lex_boolean_operators() {
        let kinds = lex("&& ||");
        assert_eq!(kinds[0], TokenKind::AmpAmp);
        assert_eq!(kinds[1], TokenKind::PipePipe);
    }

    #[test]
    fn lex_bitwise_operators() {
        let kinds = lex("& | ^");
        assert_eq!(kinds[0], TokenKind::Amp);
        assert_eq!(kinds[1], TokenKind::Pipe);
        assert_eq!(kinds[2], TokenKind::Caret);
    }

    #[test]
    fn lex_effect_keywords() {
        let kinds = lex("read write io panic unsafe");
        assert_eq!(kinds[0], TokenKind::KwRead);
        assert_eq!(kinds[1], TokenKind::KwWrite);
        assert_eq!(kinds[2], TokenKind::KwIO);
        assert_eq!(kinds[3], TokenKind::KwPanic);
        assert_eq!(kinds[4], TokenKind::KwUnsafe);
    }

    #[test]
    fn lex_all_vow_keywords() {
        let kinds = lex("vow requires ensures invariant region linear");
        assert_eq!(kinds[0], TokenKind::KwVow);
        assert_eq!(kinds[1], TokenKind::KwRequires);
        assert_eq!(kinds[2], TokenKind::KwEnsures);
        assert_eq!(kinds[3], TokenKind::KwInvariant);
        assert_eq!(kinds[4], TokenKind::KwRegion);
        assert_eq!(kinds[5], TokenKind::KwLinear);
    }

    #[test]
    fn lex_invalid_character_is_error() {
        let result = Lexer::new("@foo").tokenize();
        assert!(result.is_err());
    }

    #[test]
    fn lex_span_correctness() {
        let tokens: Vec<Token> = Lexer::new("fn foo").tokenize().unwrap();
        assert_eq!(tokens[0].span, Span::new(0, 2));
        assert_eq!(tokens[1].span, Span::new(3, 3));
    }

    #[test]
    fn lex_eof_span() {
        let tokens = Lexer::new("").tokenize().unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Eof);
        assert_eq!(tokens[0].span, Span::new(0, 0));
    }

    #[test]
    fn lex_line_comment_only() {
        let kinds = lex("// comment\n");
        assert_eq!(kinds, vec![TokenKind::Eof]);
    }

    #[test]
    fn lex_comment_after_code() {
        let kinds = lex("fn foo // comment\n");
        assert_eq!(kinds[0], TokenKind::KwFn);
        assert_eq!(kinds[1], TokenKind::Ident("foo".to_string()));
        assert_eq!(kinds[2], TokenKind::Eof);
    }

    #[test]
    fn lex_comment_at_eof_no_newline() {
        let kinds = lex("fn foo // comment");
        assert_eq!(kinds[0], TokenKind::KwFn);
        assert_eq!(kinds[1], TokenKind::Ident("foo".to_string()));
        assert_eq!(kinds[2], TokenKind::Eof);
    }

    #[test]
    fn lex_slash_still_division() {
        let kinds = lex("a / b");
        assert_eq!(kinds[0], TokenKind::Ident("a".to_string()));
        assert_eq!(kinds[1], TokenKind::Slash);
        assert_eq!(kinds[2], TokenKind::Ident("b".to_string()));
        assert_eq!(kinds[3], TokenKind::Eof);
    }

    #[test]
    fn lex_slash_checked_still_works() {
        let kinds = lex("a /! b");
        assert_eq!(kinds[0], TokenKind::Ident("a".to_string()));
        assert_eq!(kinds[1], TokenKind::SlashChecked);
        assert_eq!(kinds[2], TokenKind::Ident("b".to_string()));
        assert_eq!(kinds[3], TokenKind::Eof);
    }

    #[test]
    fn lex_double_slash_in_string() {
        let kinds = lex("\"hello // world\"");
        assert!(matches!(&kinds[0], TokenKind::LitString(s) if s == "hello // world"));
        assert_eq!(kinds[1], TokenKind::Eof);
    }

    #[test]
    fn lex_multiple_comment_lines() {
        let kinds = lex("// a\n// b\nfn x");
        assert_eq!(kinds[0], TokenKind::KwFn);
        assert_eq!(kinds[1], TokenKind::Ident("x".to_string()));
        assert_eq!(kinds[2], TokenKind::Eof);
    }

    #[test]
    fn lex_empty_comment() {
        let kinds = lex("//\nfn x");
        assert_eq!(kinds[0], TokenKind::KwFn);
        assert_eq!(kinds[1], TokenKind::Ident("x".to_string()));
        assert_eq!(kinds[2], TokenKind::Eof);
    }

    #[test]
    fn lex_usize_isize_suffix() {
        let kinds = lex("10usize 20isize");
        assert_eq!(
            kinds[0],
            TokenKind::LitIntSuffixed {
                value: 10,
                suffix: IntSuffix::Usize
            }
        );
        assert_eq!(
            kinds[1],
            TokenKind::LitIntSuffixed {
                value: 20,
                suffix: IntSuffix::Isize
            }
        );
    }
}
