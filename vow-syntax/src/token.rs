use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Keywords
    KwFn,
    KwLet,
    KwMut,
    KwStruct,
    KwEnum,
    KwMatch,
    KwIf,
    KwElse,
    KwWhile,
    KwLoop,
    KwBreak,
    KwReturn,
    KwPub,
    KwUse,
    KwModule,
    KwVow,
    KwRequires,
    KwEnsures,
    KwInvariant,
    KwWhere,
    KwRegion,
    KwLinear,
    KwExtern,
    KwImpl,
    KwTrait,
    KwType,
    KwFor,
    KwIn,
    KwAs,
    KwConst,

    // Effect keywords
    KwRead,
    KwWrite,
    KwIO,
    KwPanic,
    KwUnsafe,

    // Literals
    LitInt(i128),
    LitFloat(f64),
    LitBool(bool),
    LitString(String),

    LitIntSuffixed { value: i128, suffix: IntSuffix },

    // Identifiers
    Ident(String),

    // Arithmetic operators (wrapping, default)
    Plus,    // +
    Minus,   // -
    Star,    // *
    Slash,   // /
    Percent, // %

    // Checked arithmetic operators
    PlusChecked,    // +!
    MinusChecked,   // -!
    StarChecked,    // *!
    SlashChecked,   // /!
    PercentChecked, // %!

    // Comparison operators
    EqEq,   // ==
    BangEq, // !=
    Lt,     // <
    Gt,     // >
    LtEq,   // <=
    GtEq,   // >=

    // Boolean operators
    AmpAmp,   // &&
    PipePipe, // ||
    Bang,     // !

    // Other operators
    Amp,       // &
    Question,  // ?
    FatArrow,  // =>
    ThinArrow, // ->
    Eq,        // =

    // Delimiters
    LBrace,   // {
    RBrace,   // }
    LParen,   // (
    RParen,   // )
    LBracket, // [
    RBracket, // ]

    // Punctuation
    Comma,      // ,
    Colon,      // :
    Semicolon,  // ;
    Dot,        // .
    ColonColon, // ::
    DotDot,     // ..

    // Special
    Underscore, // _
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntSuffix {
    I8,
    I16,
    I32,
    I64,
    I128,
    U8,
    U16,
    U32,
    U64,
    U128,
    Usize,
    Isize,
}

impl TokenKind {
    pub fn from_keyword(s: &str) -> Option<TokenKind> {
        match s {
            "fn" => Some(TokenKind::KwFn),
            "let" => Some(TokenKind::KwLet),
            "mut" => Some(TokenKind::KwMut),
            "struct" => Some(TokenKind::KwStruct),
            "enum" => Some(TokenKind::KwEnum),
            "match" => Some(TokenKind::KwMatch),
            "if" => Some(TokenKind::KwIf),
            "else" => Some(TokenKind::KwElse),
            "while" => Some(TokenKind::KwWhile),
            "loop" => Some(TokenKind::KwLoop),
            "break" => Some(TokenKind::KwBreak),
            "return" => Some(TokenKind::KwReturn),
            "pub" => Some(TokenKind::KwPub),
            "use" => Some(TokenKind::KwUse),
            "module" => Some(TokenKind::KwModule),
            "vow" => Some(TokenKind::KwVow),
            "requires" => Some(TokenKind::KwRequires),
            "ensures" => Some(TokenKind::KwEnsures),
            "invariant" => Some(TokenKind::KwInvariant),
            "where" => Some(TokenKind::KwWhere),
            "region" => Some(TokenKind::KwRegion),
            "linear" => Some(TokenKind::KwLinear),
            "extern" => Some(TokenKind::KwExtern),
            "impl" => Some(TokenKind::KwImpl),
            "trait" => Some(TokenKind::KwTrait),
            "type" => Some(TokenKind::KwType),
            "for" => Some(TokenKind::KwFor),
            "in" => Some(TokenKind::KwIn),
            "as" => Some(TokenKind::KwAs),
            "const" => Some(TokenKind::KwConst),
            "read" => Some(TokenKind::KwRead),
            "write" => Some(TokenKind::KwWrite),
            "io" => Some(TokenKind::KwIO),
            "panic" => Some(TokenKind::KwPanic),
            "unsafe" => Some(TokenKind::KwUnsafe),
            "true" => Some(TokenKind::LitBool(true)),
            "false" => Some(TokenKind::LitBool(false)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_keyword_all_keywords() {
        assert_eq!(TokenKind::from_keyword("fn"), Some(TokenKind::KwFn));
        assert_eq!(TokenKind::from_keyword("vow"), Some(TokenKind::KwVow));
        assert_eq!(
            TokenKind::from_keyword("requires"),
            Some(TokenKind::KwRequires)
        );
        assert_eq!(
            TokenKind::from_keyword("ensures"),
            Some(TokenKind::KwEnsures)
        );
        assert_eq!(
            TokenKind::from_keyword("invariant"),
            Some(TokenKind::KwInvariant)
        );
        assert_eq!(TokenKind::from_keyword("not_a_keyword"), None);
    }
}
