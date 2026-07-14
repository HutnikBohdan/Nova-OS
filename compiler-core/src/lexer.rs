use crate::{CompileError, ErrorCode, Span};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Eof,
    Ident,
    Integer,
    Fn,
    Let,
    Mut,
    If,
    Else,
    While,
    Return,
    True,
    False,
    I64,
    Bool,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Colon,
    Semicolon,
    Comma,
    Arrow,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Equal,
    EqualEqual,
    Bang,
    BangEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    AndAnd,
    OrOr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token<'s> {
    pub kind: TokenKind,
    pub text: &'s str,
    pub span: Span,
}

pub struct Lexer<'s> {
    source: &'s str,
    at: usize,
}

impl<'s> Lexer<'s> {
    pub const fn new(source: &'s str) -> Self {
        Self { source, at: 0 }
    }
    pub fn next(&mut self) -> Result<Token<'s>, CompileError> {
        let b = self.source.as_bytes();
        while self.at < b.len() {
            if b[self.at].is_ascii_whitespace() {
                self.at += 1;
                continue;
            }
            if b[self.at] == b'/' && b.get(self.at + 1) == Some(&b'/') {
                self.at += 2;
                while self.at < b.len() && b[self.at] != b'\n' {
                    self.at += 1;
                }
                continue;
            }
            break;
        }
        let start = self.at;
        if start == b.len() {
            return Ok(Token {
                kind: TokenKind::Eof,
                text: "",
                span: Span::new(start, start),
            });
        }
        let c = b[self.at];
        self.at += 1;
        if c.is_ascii_alphabetic() || c == b'_' {
            while self.at < b.len() && (b[self.at].is_ascii_alphanumeric() || b[self.at] == b'_') {
                self.at += 1;
            }
            let text = &self.source[start..self.at];
            let kind = match text {
                "fn" => TokenKind::Fn,
                "let" => TokenKind::Let,
                "mut" => TokenKind::Mut,
                "if" => TokenKind::If,
                "else" => TokenKind::Else,
                "while" => TokenKind::While,
                "return" => TokenKind::Return,
                "true" => TokenKind::True,
                "false" => TokenKind::False,
                "i64" => TokenKind::I64,
                "bool" => TokenKind::Bool,
                _ => TokenKind::Ident,
            };
            return Ok(Token {
                kind,
                text,
                span: Span::new(start, self.at),
            });
        }
        if c.is_ascii_digit() {
            while self.at < b.len() && b[self.at].is_ascii_digit() {
                self.at += 1;
            }
            return Ok(Token {
                kind: TokenKind::Integer,
                text: &self.source[start..self.at],
                span: Span::new(start, self.at),
            });
        }
        let (kind, two) = match c {
            b'(' => (TokenKind::LParen, false),
            b')' => (TokenKind::RParen, false),
            b'{' => (TokenKind::LBrace, false),
            b'}' => (TokenKind::RBrace, false),
            b':' => (TokenKind::Colon, false),
            b';' => (TokenKind::Semicolon, false),
            b',' => (TokenKind::Comma, false),
            b'+' => (TokenKind::Plus, false),
            b'*' => (TokenKind::Star, false),
            b'/' => (TokenKind::Slash, false),
            b'%' => (TokenKind::Percent, false),
            b'-' if b.get(self.at) == Some(&b'>') => (TokenKind::Arrow, true),
            b'-' => (TokenKind::Minus, false),
            b'=' if b.get(self.at) == Some(&b'=') => (TokenKind::EqualEqual, true),
            b'=' => (TokenKind::Equal, false),
            b'!' if b.get(self.at) == Some(&b'=') => (TokenKind::BangEqual, true),
            b'!' => (TokenKind::Bang, false),
            b'<' if b.get(self.at) == Some(&b'=') => (TokenKind::LessEqual, true),
            b'<' => (TokenKind::Less, false),
            b'>' if b.get(self.at) == Some(&b'=') => (TokenKind::GreaterEqual, true),
            b'>' => (TokenKind::Greater, false),
            b'&' if b.get(self.at) == Some(&b'&') => (TokenKind::AndAnd, true),
            b'|' if b.get(self.at) == Some(&b'|') => (TokenKind::OrOr, true),
            _ => {
                return Err(CompileError::new(
                    ErrorCode::UnexpectedCharacter,
                    Span::new(start, self.at),
                    "невідомий символ у коді",
                ))
            }
        };
        if two {
            self.at += 1;
        }
        Ok(Token {
            kind,
            text: &self.source[start..self.at],
            span: Span::new(start, self.at),
        })
    }
}
