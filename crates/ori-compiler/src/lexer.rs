//! Hand-written bootstrap lexer. Produces a flat token stream consumed by
//! `parser` and `expr`. Lines and columns are 1-based to match
//! [`crate::source::Position`].

use crate::source::{SourceFile, Span};

/// Token discriminator returned by [`lex`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// Identifier or non-reserved word.
    Ident,
    /// Reserved keyword (see [`is_keyword`]).
    Keyword,
    /// Numeric literal.
    Number,
    /// `"..."` string literal (lexeme excludes the surrounding quotes).
    String,
    /// Single-character or multi-character punctuation.
    Symbol,
    /// Synthetic end-of-stream sentinel.
    Eof,
}

/// One lexed token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// Discriminator.
    pub kind: TokenKind,
    /// Verbatim source text the token was lexed from.
    pub lexeme: String,
    /// 1-based source range.
    pub span: Span,
}

fn is_keyword(s: &str) -> bool {
    matches!(
        s,
        "module"
            | "import"
            | "fn"
            | "type"
            | "let"
            | "var"
            | "mut"
            | "return"
            | "match"
            | "if"
            | "else"
            | "for"
            | "while"
            | "in"
            | "uses"
            | "service"
            | "view"
            | "actor"
            | "query"
            | "migration"
            | "capability"
            | "protocol"
            | "impl"
            | "extern"
            | "unsafe"
            | "arena"
            | "task_scope"
            | "async"
            | "await"
            | "throw"
    )
}

/// Lex `source` into a flat token stream. The lexer never fails: invalid
/// characters are dropped and downstream parsers surface any structural
/// problems as diagnostics.
pub fn lex(source: &SourceFile) -> Vec<Token> {
    let chars: Vec<char> = source.text.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;
    let mut col = 1usize;

    while i < chars.len() {
        let ch = chars[i];

        if ch == '\n' {
            i += 1;
            line += 1;
            col = 1;
            continue;
        }

        if ch.is_whitespace() {
            i += 1;
            col += 1;
            continue;
        }

        if ch == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
                col += 1;
            }
            continue;
        }

        let start_line = line;
        let start_col = col;

        if ch.is_ascii_alphabetic() || ch == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
                col += 1;
            }
            let lexeme: String = chars[start..i].iter().collect();
            let kind = if is_keyword(&lexeme) {
                TokenKind::Keyword
            } else {
                TokenKind::Ident
            };
            tokens.push(Token {
                kind,
                lexeme,
                span: Span::new(source.path.clone(), start_line, start_col, line, col),
            });
            continue;
        }

        if ch.is_ascii_digit() {
            let start = i;
            let mut seen_dot = false;
            while i < chars.len() {
                let current = chars[i];
                if current.is_ascii_digit() || current == '_' {
                    i += 1;
                    col += 1;
                } else if current == '.'
                    && !seen_dot
                    && i + 1 < chars.len()
                    && chars[i + 1].is_ascii_digit()
                {
                    seen_dot = true;
                    i += 1;
                    col += 1;
                } else {
                    break;
                }
            }
            let lexeme: String = chars[start..i].iter().collect();
            tokens.push(Token {
                kind: TokenKind::Number,
                lexeme,
                span: Span::new(source.path.clone(), start_line, start_col, line, col),
            });
            continue;
        }

        if ch == '"' {
            i += 1;
            col += 1;
            let start = i;
            let mut escaped = false;
            while i < chars.len() {
                let current = chars[i];
                if escaped {
                    escaped = false;
                    i += 1;
                    col += 1;
                } else if current == '\\' {
                    escaped = true;
                    i += 1;
                    col += 1;
                } else if current == '"' {
                    break;
                } else if current == '\n' {
                    line += 1;
                    col = 1;
                    i += 1;
                } else {
                    i += 1;
                    col += 1;
                }
            }
            let lexeme: String = chars[start..i.min(chars.len())].iter().collect();
            if i < chars.len() && chars[i] == '"' {
                i += 1;
                col += 1;
            }
            tokens.push(Token {
                kind: TokenKind::String,
                lexeme,
                span: Span::new(source.path.clone(), start_line, start_col, line, col),
            });
            continue;
        }

        let two = if i + 1 < chars.len() {
            let mut s = String::new();
            s.push(chars[i]);
            s.push(chars[i + 1]);
            s
        } else {
            String::new()
        };
        let lexeme = if matches!(
            two.as_str(),
            "->" | "=>" | "==" | "!=" | "<=" | ">=" | ".." | "::" | "&&" | "||"
        ) {
            i += 2;
            col += 2;
            two
        } else {
            i += 1;
            col += 1;
            ch.to_string()
        };
        tokens.push(Token {
            kind: TokenKind::Symbol,
            lexeme,
            span: Span::new(source.path.clone(), start_line, start_col, line, col),
        });
    }

    tokens.push(Token {
        kind: TokenKind::Eof,
        lexeme: String::new(),
        span: Span::point(source.path.clone(), line, col),
    });
    tokens
}
