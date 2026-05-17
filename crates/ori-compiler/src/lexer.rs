//! Hand-written bootstrap lexer. Produces a flat token stream consumed by
//! `parser` and `expr`. Lines and columns are 1-based to match
//! [`crate::source::Position`].

use crate::diagnostic::Diagnostic;
use crate::numeric_lit::{self, NumericError};
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
/// problems as diagnostics. Numeric-literal diagnostics (invalid digit,
/// stray underscore, missing-digits-after-prefix) are silently discarded
/// here; callers that want them should use [`lex_with_diagnostics`].
pub fn lex(source: &SourceFile) -> Vec<Token> {
    lex_with_diagnostics(source).0
}

/// Same as [`lex`], but also returns the numeric-literal diagnostics
/// produced by [`crate::numeric_lit::parse_numeric`] for every
/// digit-started token. The token stream itself is identical to the
/// one returned by `lex` so existing callers can adopt this signature
/// incrementally.
pub fn lex_with_diagnostics(source: &SourceFile) -> (Vec<Token>, Vec<Diagnostic>) {
    let chars: Vec<char> = source.text.chars().collect();
    let mut tokens = Vec::new();
    let mut diagnostics = Vec::new();
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
            let end = scan_numeric(&chars, i);
            // Numeric literals never contain a newline, so a single
            // column advance is sufficient.
            col += end - i;
            i = end;
            let lexeme: String = chars[start..i].iter().collect();
            let span = Span::new(source.path.clone(), start_line, start_col, line, col);
            if let Err(err) = numeric_lit::parse_numeric(&lexeme) {
                // Overflow is intentionally demoted to BigInt by
                // parse_numeric, so we never see N1402 here. Every
                // other variant becomes a soft diagnostic — the token
                // is still emitted so downstream parsing can continue.
                if !matches!(err, NumericError::OverflowI64) {
                    diagnostics.push(Diagnostic::error(err.id(), err.message(), span.clone()));
                }
            }
            tokens.push(Token {
                kind: TokenKind::Number,
                lexeme,
                span,
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
    (tokens, diagnostics)
}

/// Walk forward over `chars` starting at `start` (already known to be
/// an ASCII digit) and return the index one past the last character
/// that belongs to the numeric lexeme. Recognises the `0x`/`0b`/`0o`
/// prefixes so non-decimal literals are captured as a single token
/// before being handed off to [`numeric_lit::parse_numeric`].
fn scan_numeric(chars: &[char], start: usize) -> usize {
    let mut i = start;
    let len = chars.len();

    // Detect and consume an optional base prefix. Only the leading
    // `0` at `start` can introduce a prefix, so we check directly.
    let has_base_prefix =
        i + 1 < len && chars[i] == '0' && matches!(chars[i + 1], 'x' | 'X' | 'b' | 'B' | 'o' | 'O');

    if has_base_prefix {
        i += 2;
        // Non-decimal: greedily consume alphanumeric + underscores so
        // that bad digits (e.g. `2` in `0b102`) stay part of the same
        // lexeme. `parse_numeric` then flags them as `InvalidDigit`
        // rather than us silently splitting the token.
        while i < len {
            let c = chars[i];
            if c == '_' || c.is_ascii_alphanumeric() {
                i += 1;
            } else {
                break;
            }
        }
        return i;
    }

    // Decimal: digits, underscores, and a single dot (with a digit on
    // the right so we don't swallow `1..n` ranges).
    let mut seen_dot = false;
    while i < len {
        let c = chars[i];
        if c.is_ascii_digit() || c == '_' {
            i += 1;
        } else if c == '.' && !seen_dot && i + 1 < len && chars[i + 1].is_ascii_digit() {
            seen_dot = true;
            i += 1;
        } else {
            break;
        }
    }
    i
}

// ---------------------------------------------------------------------------
// Tests — lexer-level integration for numeric literals.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn lex_text(text: &str) -> (Vec<Token>, Vec<Diagnostic>) {
        let src = SourceFile::new("<test>", text);
        lex_with_diagnostics(&src)
    }

    #[test]
    fn lex_hex_literal_is_one_token() {
        let (toks, diags) = lex_text("0xFF");
        // 0xFF + Eof
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].kind, TokenKind::Number);
        assert_eq!(toks[0].lexeme, "0xFF");
        assert!(diags.is_empty());
    }

    #[test]
    fn lex_binary_literal_is_one_token() {
        let (toks, diags) = lex_text("0b1010");
        assert_eq!(toks[0].lexeme, "0b1010");
        assert_eq!(toks[0].kind, TokenKind::Number);
        assert!(diags.is_empty());
    }

    #[test]
    fn lex_octal_literal_is_one_token() {
        let (toks, diags) = lex_text("0o17");
        assert_eq!(toks[0].lexeme, "0o17");
        assert_eq!(toks[0].kind, TokenKind::Number);
        assert!(diags.is_empty());
    }

    #[test]
    fn lex_decimal_with_separators() {
        let (toks, diags) = lex_text("1_000_000");
        assert_eq!(toks[0].lexeme, "1_000_000");
        assert!(diags.is_empty());
    }

    #[test]
    fn lex_invalid_digit_yields_diagnostic_but_still_token() {
        let (toks, diags) = lex_text("0b102");
        assert_eq!(toks[0].kind, TokenKind::Number);
        assert_eq!(toks[0].lexeme, "0b102");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].id, "N1401");
    }

    #[test]
    fn lex_missing_digits_after_prefix_yields_diagnostic() {
        // `0x ` — bare prefix with no body. Whitespace stops the
        // numeric scan; the token is `0x` which is invalid.
        let (toks, diags) = lex_text("0x ");
        assert_eq!(toks[0].kind, TokenKind::Number);
        assert_eq!(toks[0].lexeme, "0x");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].id, "N1404");
    }

    #[test]
    fn lex_overflow_does_not_emit_diagnostic() {
        let (toks, diags) = lex_text("9223372036854775808");
        assert_eq!(toks[0].kind, TokenKind::Number);
        assert!(
            diags.is_empty(),
            "overflow demotes silently to BigInt, expected no diagnostic, got {diags:?}"
        );
    }

    #[test]
    fn lex_existing_decimal_still_works() {
        // Regression guard for the pre-M21c behaviour: plain decimals
        // and `..` ranges still tokenise cleanly.
        let (toks, _) = lex_text("123 1..5");
        assert_eq!(toks[0].lexeme, "123");
        // Tokens: 123, 1, .., 5, Eof
        let lexemes: Vec<&str> = toks.iter().map(|t| t.lexeme.as_str()).collect();
        assert_eq!(lexemes, vec!["123", "1", "..", "5", ""]);
    }
}
