//! Tokens produced by the lexer

use std::fmt::Display;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token<'s> {
    Ident(&'s str),

    // === Punctuation ===
    AmpAmp,
    AngleLeftEq,
    AngleRightEq,
    Arrow,
    Bang,
    BangEq,
    Colon,
    Comma,
    Eq,
    EqEq,
    FatArrow,
    Hash,
    Hyphen,
    HyphenHyphen,
    Period,
    Pipe,
    PipePipe,
    Plus,
    QuestionMark,
    SemiColon,
    Slash,
    /// A possibly nested `/* ... */` block comment.
    ///
    /// Roto rejects this syntax, but retaining it as one token prevents
    /// tooling from interpreting the contents as code.
    BlockComment(&'s str),
    Star,
    Percent,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,

    // === Delimiters ===
    AngleLeft,
    AngleRight,
    CurlyLeft,
    CurlyRight,
    RoundLeft,
    RoundRight,
    SquareLeft,
    SquareRight,

    // === Keywords ===
    Keyword(Keyword),

    // === Literals ===
    String(&'s str),
    /// A raw regex literal, including its `r` prefix and matching delimiters.
    Regex(&'s str),
    /// A raw regex literal that reaches EOF without its closing delimiter.
    UnterminatedRegex(&'s str),
    Char(&'s str),
    Integer(&'s str, &'s str),
    Float(&'s str, &'s str),
    Hex(&'s str),
    Asn(&'s str),
    IpV4(&'s str),
    IpV6(&'s str),
    Bool(bool),

    /// An f-string start token signals to the parser that an f-string is coming up
    FStringStart,

    /// Literal text before an interpolated expression in an f-string.
    FStringText(&'s str),

    /// Final literal text and the closing quote of an f-string.
    FStringEnd(&'s str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FStringToken<'s> {
    /// The final part of a string, including its closing quote.
    ///
    /// This is the token from the current position to the end of the string.
    /// For non-f-strings, this will be the entire string.
    StringEnd(&'s str),

    /// An intermediate part of an f-string, until the next `{` token.
    StringIntermediate(&'s str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keyword {
    Accept,
    Const,
    Dep,
    Else,
    Enum,
    Filter,
    FilterMap,
    For,
    Fn,
    If,
    Import,
    In,
    Let,
    Match,
    Pkg,
    Record,
    Reject,
    Return,
    Std,
    Super,
    Test,
    While,
}

impl Display for Token<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Token::Ident(s) => s,

            // Punctuation
            Token::AmpAmp => "&&",
            Token::AngleLeftEq => "<=",
            Token::AngleRightEq => ">=",
            Token::Arrow => "->",
            Token::Bang => "!",
            Token::BangEq => "!=",
            Token::Colon => ":",
            Token::Comma => ",",
            Token::Eq => "=",
            Token::EqEq => "==",
            Token::FatArrow => "=>",
            Token::Hash => "#",
            Token::Hyphen => "-",
            Token::HyphenHyphen => "--",
            Token::Period => ".",
            Token::Pipe => "|",
            Token::PipePipe => "||",
            Token::Plus => "+",
            Token::QuestionMark => "?",
            Token::SemiColon => ";",
            Token::Slash => "/",
            Token::BlockComment(_) => "/*",
            Token::Star => "*",
            Token::Percent => "%",
            Token::PlusEq => "+=",
            Token::MinusEq => "-=",
            Token::StarEq => "*=",
            Token::SlashEq => "/=",
            Token::PercentEq => "%=",

            // Delimiters
            Token::AngleLeft => "<",
            Token::AngleRight => ">",
            Token::CurlyLeft => "{",
            Token::CurlyRight => "}",
            Token::RoundLeft => "(",
            Token::RoundRight => ")",
            Token::SquareLeft => "[",
            Token::SquareRight => "]",

            // Keywords
            Token::Keyword(k) => k.as_str(),

            // Literals
            Token::String(s) => s,
            Token::Regex(s) | Token::UnterminatedRegex(s) => s,
            Token::Char(s) => s,
            Token::Integer(s, suffix) => {
                f.write_str(s)?;
                return f.write_str(suffix);
            }
            Token::Float(s, suffix) => {
                f.write_str(s)?;
                return f.write_str(suffix);
            }
            Token::Hex(s) => s,
            Token::Asn(s) => s,
            Token::IpV4(s) => s,
            Token::IpV6(s) => s,
            Token::Bool(true) => "true",
            Token::Bool(false) => "false",

            Token::FStringStart => "f\"",
            Token::FStringText(s) | Token::FStringEnd(s) => s,
        };

        f.write_str(s)
    }
}

impl Keyword {
    /// Return the keyword's canonical source spelling.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Keyword::Accept => "accept",
            Keyword::Const => "const",
            Keyword::Dep => "dep",
            Keyword::Else => "else",
            Keyword::Enum => "enum",
            Keyword::Filter => "filter",
            Keyword::FilterMap => "filtermap",
            Keyword::For => "for",
            Keyword::Fn => "fn",
            Keyword::If => "if",
            Keyword::Import => "import",
            Keyword::In => "in",
            Keyword::Let => "let",
            Keyword::Match => "match",
            Keyword::Pkg => "pkg",
            Keyword::Record => "record",
            Keyword::Reject => "reject",
            Keyword::Return => "return",
            Keyword::Std => "std",
            Keyword::Super => "super",
            Keyword::Test => "test",
            Keyword::While => "while",
        }
    }
}
