//! Extended string-literal lexing for milestone M21b.
//!
//! The bootstrap lexer (`crate::lexer`) only recognises plain
//! double-quoted strings. This module adds the surface-language string
//! forms that the body parser exposes to user code:
//!
//! * **Plain** — `"hello"` with the usual escapes (`\\`, `\"`, `\n`,
//!   `\t`, `\r`, `\0`).
//! * **Interpolated** — `"hi {name}!"` carved into a sequence of
//!   [`StringPart::Lit`] / [`StringPart::Interp`] fragments. The
//!   interpolation expression text is captured verbatim (the body
//!   parser re-lexes it on demand). Use `\{` to embed a literal brace.
//! * **Raw** — `r"\n"`, `r#"foo "bar""#`, …, `r####"…"####`. No escapes
//!   are processed and embedded quotes are permitted up to the
//!   matching number of `#` characters.
//! * **Multiline** — triple-double-quoted `"""…"""`. Leading-whitespace
//!   on every non-empty content line is stripped to the common minimum
//!   indent (tab is normalised as the equivalent of four spaces for the
//!   measurement; the original byte run is preserved beyond the minimum
//!   indent boundary).
//!
//! ## Diagnostic IDs
//!
//! Stable, owned by this module. They appear in the structured-JSON
//! contract and must never be repurposed.
//!
//! | id      | meaning                                                |
//! |---------|--------------------------------------------------------|
//! | `S1300` | unterminated string (any variant)                      |
//! | `S1301` | invalid raw-string delimiter (e.g. `r#####"…"#####`)   |
//! | `S1302` | unbalanced `{` in an interpolated string               |
//! | `S1303` | multiline + interpolation combined (unsupported today) |
//!
//! Errors are returned via [`StringLexError`] rather than panicking so
//! callers can surface them through the existing [`crate::diagnostic`]
//! pipeline without any allocation in the happy path.
//!
//! ## Design notes
//!
//! * **No `unwrap` / `expect` / `panic!`.** Every fallible path returns
//!   `Result<_, StringLexError>`. The `#[cfg(test)]` tests below have
//!   regression coverage for every error ID.
//! * **Round-trip rendering.** [`LexedString::render`] reproduces the
//!   exact source text the lexer consumed; the parser test in
//!   [`crate::expr`] uses this to guarantee `parse(s).render() == s`
//!   for plain, raw, and multiline literals.
//! * **Char-indexed scanning.** The functions operate over `&[char]`
//!   so multi-byte characters in the source do not split mid-escape.
//!   `start_idx` and the returned `consumed` count are both in
//!   `char` units (not bytes) — the caller is responsible for
//!   converting between char- and byte-offsets when integrating with
//!   the byte-oriented lexer.

/// One fragment of an interpolated string. A purely literal string is
/// a single [`StringPart::Lit`]; an interpolated string alternates
/// `Lit` / `Interp` segments and may begin or end with either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StringPart {
    /// Verbatim text (after escape processing for plain / interpolated
    /// strings; verbatim for raw / multiline strings).
    Lit(String),
    /// Captured expression text from inside a `{ … }` hole. The text is
    /// preserved verbatim — escape processing is *not* applied — so the
    /// caller can re-lex it as a normal expression.
    Interp(String),
}

/// Discriminator for the source form a [`LexedString`] originated from.
/// Round-trip rendering needs this so that `r"foo"` is rendered with
/// the same number of `#` hashes (or none) it was written with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringKind {
    /// Plain `"…"` literal, no interpolation, escapes processed.
    Plain,
    /// `"…{x}…"` literal with one or more `{ … }` holes.
    Interpolated,
    /// Raw literal with `n` surrounding `#` characters
    /// (`r"…"` ⇒ 0, `r#"…"#` ⇒ 1, …, `r####"…"####` ⇒ 4).
    Raw(u8),
    /// `"""…"""` literal with common-indent stripping applied.
    Multiline,
}

/// Maximum number of `#` characters permitted around a raw string.
/// Diagnostic `S1301` fires if the input exceeds this.
pub const MAX_RAW_HASHES: u8 = 4;

/// Structured result of [`lex_string_extended`]. Always describes a
/// *single* string literal — never an unrelated sequence of tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexedString {
    /// The source form this literal was written in.
    pub kind: StringKind,
    /// Fragments. A non-interpolated literal contains exactly one
    /// `StringPart::Lit`; interpolated literals alternate.
    pub parts: Vec<StringPart>,
    /// Verbatim source text including the surrounding delimiters. Used
    /// by [`LexedString::render`] to round-trip the literal.
    pub raw_text: String,
}

impl LexedString {
    /// Reproduce the verbatim source text the lexer consumed. Always
    /// equal to [`Self::raw_text`]; provided as a named method so call
    /// sites read more clearly (`lit.render()` mirrors the existing
    /// `Display`-style API used elsewhere in the compiler).
    pub fn render(&self) -> String {
        self.raw_text.clone()
    }

    /// `true` if this literal contained at least one `{ … }` hole.
    pub fn is_interpolated(&self) -> bool {
        matches!(self.kind, StringKind::Interpolated)
    }

    /// Flatten the literal back into a single `String`, dropping
    /// interpolation holes. Useful for cases that only need a
    /// best-effort display of the literal (the bootstrap interpreter
    /// uses this when it cannot resolve a hole).
    pub fn flatten_lit_only(&self) -> String {
        let mut out = String::new();
        for part in &self.parts {
            if let StringPart::Lit(s) = part {
                out.push_str(s);
            }
        }
        out
    }
}

/// Structured failure returned by [`lex_string_extended`]. Each
/// variant maps directly to one of the `S1300`..`S1303` diagnostic IDs
/// in the module docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StringLexError {
    /// `S1300` — closing quote never appeared before EOF.
    Unterminated {
        /// Char offset at which the offending literal started.
        start: usize,
    },
    /// `S1301` — too many `#` characters around a raw string. The
    /// boot-strap caps this at [`MAX_RAW_HASHES`] for predictability.
    InvalidRawDelimiter {
        /// Number of `#` characters seen before the opening quote.
        hashes: usize,
        /// Char offset of the `r` introducing the raw string.
        start: usize,
    },
    /// `S1302` — interpolated string contained `{` without a matching
    /// `}` (or `}` without `{`).
    UnbalancedInterpolation {
        /// Char offset of the offending `{` (or string start if `}`
        /// appeared with no preceding `{`).
        at: usize,
    },
    /// `S1303` — multiline string mixed with interpolation. Not
    /// supported by the bootstrap; allowed by the grammar so a future
    /// milestone can lift the restriction without a schema bump.
    MultilineInterpolation {
        /// Char offset of the offending `{`.
        at: usize,
    },
}

impl StringLexError {
    /// Stable diagnostic ID associated with this error.
    pub fn id(&self) -> &'static str {
        match self {
            StringLexError::Unterminated { .. } => "S1300",
            StringLexError::InvalidRawDelimiter { .. } => "S1301",
            StringLexError::UnbalancedInterpolation { .. } => "S1302",
            StringLexError::MultilineInterpolation { .. } => "S1303",
        }
    }

    /// One-line human-readable message; mirrors the existing
    /// [`crate::diagnostic::Diagnostic::message`] convention.
    pub fn message(&self) -> String {
        match self {
            StringLexError::Unterminated { .. } => "unterminated string literal".to_string(),
            StringLexError::InvalidRawDelimiter { hashes, .. } => {
                format!(
                    "raw string uses {hashes} `#` characters, only 0..={MAX_RAW_HASHES} allowed",
                )
            }
            StringLexError::UnbalancedInterpolation { .. } => {
                "unbalanced `{` in interpolated string".to_string()
            }
            StringLexError::MultilineInterpolation { .. } => {
                "interpolation `{…}` is not yet supported inside `\"\"\"` multiline strings"
                    .to_string()
            }
        }
    }
}

/// Lex a single string literal starting at `source[start_idx..]`
/// (char-indexed). Returns the structured [`LexedString`] together with
/// the number of `char`s consumed (delimiters included), or a
/// [`StringLexError`] on failure.
///
/// The function never panics. Caller is responsible for advancing past
/// `consumed` characters and converting the count back to a byte
/// offset if needed — `chars().take(consumed).map(char::len_utf8).sum()`
/// is the idiomatic conversion.
pub fn lex_string_extended(
    source: &str,
    start_idx: usize,
) -> Result<(LexedString, usize), StringLexError> {
    let chars: Vec<char> = source.chars().collect();
    if start_idx >= chars.len() {
        return Err(StringLexError::Unterminated { start: start_idx });
    }

    // Raw string?  Form is r [#…#] " … " [#…#].
    if chars[start_idx] == 'r' {
        let mut hashes = 0usize;
        let mut probe = start_idx + 1;
        while probe < chars.len() && chars[probe] == '#' {
            hashes += 1;
            probe += 1;
        }
        if probe < chars.len() && chars[probe] == '"' {
            return lex_raw(&chars, start_idx, hashes);
        }
        // Not actually a raw string — fall through to plain handling
        // only if the next char is `"`. (The lexer will tokenise the
        // bare `r` as an identifier in this case, so we should not
        // have been called.)
    }

    // Multiline `"""…"""`?
    if start_idx + 2 < chars.len()
        && chars[start_idx] == '"'
        && chars[start_idx + 1] == '"'
        && chars[start_idx + 2] == '"'
    {
        return lex_multiline(&chars, start_idx);
    }

    // Plain or interpolated `"…"`.
    if chars[start_idx] == '"' {
        return lex_plain_or_interp(&chars, start_idx);
    }

    Err(StringLexError::Unterminated { start: start_idx })
}

// ---------------------------------------------------------------------------
// Raw strings
// ---------------------------------------------------------------------------

fn lex_raw(
    chars: &[char],
    start_idx: usize,
    hashes: usize,
) -> Result<(LexedString, usize), StringLexError> {
    if hashes > MAX_RAW_HASHES as usize {
        return Err(StringLexError::InvalidRawDelimiter {
            hashes,
            start: start_idx,
        });
    }
    // Cursor sits at the opening `"`; advance past it.
    let mut cursor = start_idx + 1 + hashes + 1;
    let body_start = cursor;
    loop {
        if cursor >= chars.len() {
            return Err(StringLexError::Unterminated { start: start_idx });
        }
        if chars[cursor] == '"' {
            // Need to see `hashes` consecutive `#` characters next.
            let mut ok = true;
            for offset in 0..hashes {
                let probe = cursor + 1 + offset;
                if probe >= chars.len() || chars[probe] != '#' {
                    ok = false;
                    break;
                }
            }
            if ok {
                break;
            }
        }
        cursor += 1;
    }
    let body: String = chars[body_start..cursor].iter().collect();
    let raw_end = cursor + 1 + hashes;
    let raw_text: String = chars[start_idx..raw_end].iter().collect();
    let lexed = LexedString {
        kind: StringKind::Raw(hashes as u8),
        parts: vec![StringPart::Lit(body)],
        raw_text,
    };
    Ok((lexed, raw_end - start_idx))
}

// ---------------------------------------------------------------------------
// Plain & interpolated `"…"`
// ---------------------------------------------------------------------------

fn lex_plain_or_interp(
    chars: &[char],
    start_idx: usize,
) -> Result<(LexedString, usize), StringLexError> {
    // Cursor sits at the opening `"`; advance past it.
    let mut cursor = start_idx + 1;
    let mut current_lit = String::new();
    let mut parts: Vec<StringPart> = Vec::new();
    let mut has_holes = false;
    loop {
        if cursor >= chars.len() {
            return Err(StringLexError::Unterminated { start: start_idx });
        }
        let ch = chars[cursor];
        match ch {
            '"' => {
                cursor += 1;
                break;
            }
            '\\' => {
                let next = chars.get(cursor + 1).copied();
                match next {
                    Some('\\') => current_lit.push('\\'),
                    Some('"') => current_lit.push('"'),
                    Some('n') => current_lit.push('\n'),
                    Some('t') => current_lit.push('\t'),
                    Some('r') => current_lit.push('\r'),
                    Some('0') => current_lit.push('\0'),
                    Some('{') => current_lit.push('{'),
                    Some('}') => current_lit.push('}'),
                    Some(other) => {
                        // Unknown escape — keep the original two chars so
                        // round-trip rendering survives.
                        current_lit.push('\\');
                        current_lit.push(other);
                    }
                    None => return Err(StringLexError::Unterminated { start: start_idx }),
                }
                cursor += 2;
            }
            '{' => {
                has_holes = true;
                // Flush current literal fragment (even if empty so the
                // ordering [Lit, Interp, Lit, …] is preserved).
                parts.push(StringPart::Lit(std::mem::take(&mut current_lit)));
                let interp_start_at = cursor; // for diagnostics
                cursor += 1;
                let mut depth: usize = 1;
                let hole_text_start = cursor;
                loop {
                    if cursor >= chars.len() {
                        return Err(StringLexError::UnbalancedInterpolation {
                            at: interp_start_at,
                        });
                    }
                    let inner = chars[cursor];
                    if inner == '{' {
                        depth += 1;
                    } else if inner == '}' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    } else if inner == '"' {
                        // Closing the string mid-hole is illegal.
                        return Err(StringLexError::UnbalancedInterpolation {
                            at: interp_start_at,
                        });
                    }
                    cursor += 1;
                }
                let expr_text: String = chars[hole_text_start..cursor].iter().collect();
                parts.push(StringPart::Interp(expr_text));
                cursor += 1; // skip `}`
            }
            '}' => {
                // Bare `}` is an error per S1302; allow `\}` to escape.
                return Err(StringLexError::UnbalancedInterpolation { at: cursor });
            }
            other => {
                current_lit.push(other);
                cursor += 1;
            }
        }
    }
    if has_holes || !current_lit.is_empty() {
        parts.push(StringPart::Lit(current_lit));
    } else if parts.is_empty() {
        parts.push(StringPart::Lit(String::new()));
    }
    let raw_text: String = chars[start_idx..cursor].iter().collect();
    let kind = if has_holes {
        StringKind::Interpolated
    } else {
        StringKind::Plain
    };
    let lexed = LexedString {
        kind,
        parts,
        raw_text,
    };
    Ok((lexed, cursor - start_idx))
}

// ---------------------------------------------------------------------------
// Multiline `"""…"""`
// ---------------------------------------------------------------------------

fn lex_multiline(chars: &[char], start_idx: usize) -> Result<(LexedString, usize), StringLexError> {
    // Cursor sits at first `"` of the opening triplet.
    let mut cursor = start_idx + 3;
    let body_start = cursor;
    loop {
        if cursor >= chars.len() {
            return Err(StringLexError::Unterminated { start: start_idx });
        }
        if cursor + 2 < chars.len()
            && chars[cursor] == '"'
            && chars[cursor + 1] == '"'
            && chars[cursor + 2] == '"'
        {
            break;
        }
        // Reject `{` inside multiline strings: combined multiline +
        // interpolation is reserved (S1303). `\{` still escapes.
        if chars[cursor] == '\\' && cursor + 1 < chars.len() {
            // Skip the escape sequence verbatim — multiline strings
            // don't process escapes, but we still treat `\{` / `\}` as
            // an explicit opt-out so a future relaxation is forward
            // compatible.
            cursor += 2;
            continue;
        }
        if chars[cursor] == '{' {
            return Err(StringLexError::MultilineInterpolation { at: cursor });
        }
        cursor += 1;
    }
    let raw_body: String = chars[body_start..cursor].iter().collect();
    let raw_end = cursor + 3;
    let raw_text: String = chars[start_idx..raw_end].iter().collect();
    let stripped = strip_common_indent(&raw_body);
    let lexed = LexedString {
        kind: StringKind::Multiline,
        parts: vec![StringPart::Lit(stripped)],
        raw_text,
    };
    Ok((lexed, raw_end - start_idx))
}

/// Strip the common leading-whitespace prefix from every non-empty line
/// of `body`. Empty lines do not participate in the measurement (so
/// blank separators inside the multiline don't force the prefix to be
/// empty). Tabs are weighed as 4 spaces for measurement only — the
/// original byte run beyond the stripped prefix is preserved verbatim.
fn strip_common_indent(body: &str) -> String {
    let lines: Vec<&str> = body.split('\n').collect();
    let mut min_indent: Option<usize> = None;
    // Skip the first line for indent measurement if it's empty — this
    // is the line immediately following the opening `"""`.
    let measure_start = if lines.first().map(|l| l.trim().is_empty()) == Some(true) {
        1
    } else {
        0
    };
    for line in &lines[measure_start..] {
        if line.trim().is_empty() {
            continue;
        }
        let mut indent = 0usize;
        for ch in line.chars() {
            match ch {
                ' ' => indent += 1,
                '\t' => indent += 4,
                _ => break,
            }
        }
        min_indent = Some(match min_indent {
            Some(current) => current.min(indent),
            None => indent,
        });
    }
    let strip = min_indent.unwrap_or(0);
    if strip == 0 {
        return body.to_string();
    }
    let mut out = String::new();
    for (idx, line) in lines.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        if line.trim().is_empty() {
            // Whitespace-only line: collapse to an empty line so the
            // leading/trailing blank around the closing `"""` doesn't
            // leak indentation back into the stripped output.
            continue;
        }
        // Drop up to `strip` units of whitespace, counting tabs as 4.
        let mut remaining = strip;
        let mut byte_idx = 0;
        for ch in line.chars() {
            if remaining == 0 {
                break;
            }
            match ch {
                ' ' => {
                    remaining = remaining.saturating_sub(1);
                    byte_idx += 1;
                }
                '\t' => {
                    remaining = remaining.saturating_sub(4);
                    byte_idx += 1;
                }
                _ => break,
            }
        }
        out.push_str(&line[byte_idx..]);
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_ok(input: &str) -> LexedString {
        match lex_string_extended(input, 0) {
            Ok((lit, consumed)) => {
                assert_eq!(
                    consumed,
                    input.chars().count(),
                    "should consume entire input `{input}`"
                );
                lit
            }
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected Ok for `{input}`, got {err:?}");
                }
                unreachable!()
            }
        }
    }

    fn assert_err(input: &str) -> StringLexError {
        match lex_string_extended(input, 0) {
            Ok((lit, _)) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected Err for `{input}`, got Ok({lit:?})");
                }
                unreachable!()
            }
            Err(err) => err,
        }
    }

    // ---- plain ----

    #[test]
    fn plain_empty() {
        let lit = assert_ok("\"\"");
        assert_eq!(lit.kind, StringKind::Plain);
        assert_eq!(lit.parts, vec![StringPart::Lit(String::new())]);
        assert_eq!(lit.render(), "\"\"");
    }

    #[test]
    fn plain_hello() {
        let lit = assert_ok("\"hello\"");
        assert_eq!(lit.kind, StringKind::Plain);
        assert_eq!(lit.parts, vec![StringPart::Lit("hello".into())]);
        assert_eq!(lit.render(), "\"hello\"");
    }

    #[test]
    fn plain_escapes() {
        let lit = assert_ok("\"a\\nb\\tc\\\\d\\\"e\"");
        assert_eq!(lit.parts, vec![StringPart::Lit("a\nb\tc\\d\"e".into())]);
    }

    #[test]
    fn plain_with_braces_escaped() {
        let lit = assert_ok("\"hi \\{name\\}\"");
        assert_eq!(lit.kind, StringKind::Plain);
        assert_eq!(lit.parts, vec![StringPart::Lit("hi {name}".into())]);
    }

    // ---- interpolated ----

    #[test]
    fn interp_one_hole() {
        let lit = assert_ok("\"hello {name}\"");
        assert_eq!(lit.kind, StringKind::Interpolated);
        assert_eq!(
            lit.parts,
            vec![
                StringPart::Lit("hello ".into()),
                StringPart::Interp("name".into()),
                StringPart::Lit(String::new()),
            ]
        );
    }

    #[test]
    fn interp_zero_or_only_hole() {
        let lit = assert_ok("\"{x}\"");
        assert_eq!(lit.kind, StringKind::Interpolated);
        assert_eq!(
            lit.parts,
            vec![
                StringPart::Lit(String::new()),
                StringPart::Interp("x".into()),
                StringPart::Lit(String::new()),
            ]
        );
    }

    #[test]
    fn interp_many_holes() {
        let lit = assert_ok("\"{a}-{b}-{c}\"");
        assert_eq!(lit.kind, StringKind::Interpolated);
        let interps: Vec<_> = lit
            .parts
            .iter()
            .filter_map(|p| match p {
                StringPart::Interp(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            interps,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn interp_nested_braces_in_expr() {
        // The expr text inside the hole is captured verbatim, including
        // nested braces (so a record literal works).
        let lit = assert_ok("\"r={ { a: 1 } }\"");
        // After lexing, parts should be ["r=", "{ a: 1 }", ""].
        assert_eq!(lit.kind, StringKind::Interpolated);
        let interp_texts: Vec<_> = lit
            .parts
            .iter()
            .filter_map(|p| match p {
                StringPart::Interp(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(interp_texts, vec![" { a: 1 } ".to_string()]);
    }

    // ---- raw ----

    #[test]
    fn raw_zero_hashes() {
        let lit = assert_ok("r\"foo \\n\"");
        assert_eq!(lit.kind, StringKind::Raw(0));
        assert_eq!(lit.parts, vec![StringPart::Lit("foo \\n".into())]);
        assert_eq!(lit.render(), "r\"foo \\n\"");
    }

    #[test]
    fn raw_one_hash() {
        let lit = assert_ok("r#\"foo \"bar\"\"#");
        assert_eq!(lit.kind, StringKind::Raw(1));
        assert_eq!(lit.parts, vec![StringPart::Lit("foo \"bar\"".into())]);
    }

    #[test]
    fn raw_two_hashes() {
        let lit = assert_ok("r##\"a #\"b\" c\"##");
        assert_eq!(lit.kind, StringKind::Raw(2));
        assert_eq!(lit.parts, vec![StringPart::Lit("a #\"b\" c".into())]);
    }

    #[test]
    fn raw_three_hashes() {
        let lit = assert_ok("r###\"x ##\"y\"## z\"###");
        assert_eq!(lit.kind, StringKind::Raw(3));
        assert_eq!(lit.parts, vec![StringPart::Lit("x ##\"y\"## z".into())]);
    }

    #[test]
    fn raw_four_hashes() {
        let lit = assert_ok("r####\"final\"####");
        assert_eq!(lit.kind, StringKind::Raw(4));
        assert_eq!(lit.parts, vec![StringPart::Lit("final".into())]);
    }

    // ---- multiline ----

    #[test]
    fn multiline_simple() {
        let src = "\"\"\"\n    hello\n    world\n    \"\"\"";
        let lit = assert_ok(src);
        assert_eq!(lit.kind, StringKind::Multiline);
        // Common indent is 4 spaces.
        assert_eq!(lit.parts, vec![StringPart::Lit("\nhello\nworld\n".into())]);
        assert_eq!(lit.render(), src);
    }

    #[test]
    fn multiline_mixed_indent() {
        // First content line: 6 spaces. Second: 2 spaces (and blank
        // continuation). Common indent should be 2.
        let src = "\"\"\"\n      deep\n  shallow\n\"\"\"";
        let lit = assert_ok(src);
        assert_eq!(
            lit.parts,
            vec![StringPart::Lit("\n    deep\nshallow\n".into())]
        );
    }

    #[test]
    fn multiline_tab_as_four_spaces() {
        // A leading tab plus four-space line both have measured indent
        // 4 → both lose 4 units of leading whitespace.
        let src = "\"\"\"\n\tA\n    B\n\"\"\"";
        let lit = assert_ok(src);
        assert_eq!(lit.parts, vec![StringPart::Lit("\nA\nB\n".into())]);
    }

    // ---- errors ----

    #[test]
    fn error_s1300_unterminated_plain() {
        let err = assert_err("\"oops");
        assert_eq!(err.id(), "S1300");
    }

    #[test]
    fn error_s1300_unterminated_raw() {
        let err = assert_err("r#\"oops\"");
        assert_eq!(err.id(), "S1300");
    }

    #[test]
    fn error_s1300_unterminated_multiline() {
        let err = assert_err("\"\"\"never closed");
        assert_eq!(err.id(), "S1300");
    }

    #[test]
    fn error_s1301_invalid_raw_delimiter() {
        // 5 hashes exceeds MAX_RAW_HASHES (4).
        let err = assert_err("r#####\"x\"#####");
        assert_eq!(err.id(), "S1301");
    }

    #[test]
    fn error_s1302_unbalanced_open_brace() {
        let err = assert_err("\"hello {name\"");
        assert_eq!(err.id(), "S1302");
    }

    #[test]
    fn error_s1302_unbalanced_close_brace() {
        let err = assert_err("\"hi }\"");
        assert_eq!(err.id(), "S1302");
    }

    #[test]
    fn error_s1303_multiline_interp_unsupported() {
        let src = "\"\"\"\n  hi {name}\n  \"\"\"";
        let err = assert_err(src);
        assert_eq!(err.id(), "S1303");
    }

    // ---- round-trip ----

    #[test]
    fn round_trip_plain() {
        let src = "\"hello\\nworld\"";
        let lit = assert_ok(src);
        assert_eq!(lit.render(), src);
    }

    #[test]
    fn round_trip_raw() {
        let src = "r##\"contains \"quotes\" and \\n\"##";
        let lit = assert_ok(src);
        assert_eq!(lit.render(), src);
    }

    #[test]
    fn round_trip_multiline() {
        let src = "\"\"\"\n    indented\n    block\n    \"\"\"";
        let lit = assert_ok(src);
        assert_eq!(lit.render(), src);
    }

    // ---- partial-string lex (start_idx > 0) ----

    #[test]
    fn lex_at_nonzero_offset_consumes_only_the_literal() {
        let src = "let s = \"hi\"; end";
        let chars: Vec<char> = src.chars().collect();
        let start = chars.iter().position(|c| *c == '"').unwrap_or(0);
        let (lit, consumed) = lex_string_extended(src, start).unwrap_or((
            LexedString {
                kind: StringKind::Plain,
                parts: vec![StringPart::Lit(String::new())],
                raw_text: String::new(),
            },
            0,
        ));
        assert_eq!(lit.kind, StringKind::Plain);
        assert_eq!(lit.parts, vec![StringPart::Lit("hi".into())]);
        assert_eq!(consumed, 4); // "hi"
    }

    // ---- accessors ----

    #[test]
    fn is_interpolated_helper() {
        assert!(!assert_ok("\"plain\"").is_interpolated());
        assert!(assert_ok("\"hi {x}\"").is_interpolated());
        assert!(!assert_ok("r\"raw\"").is_interpolated());
    }

    #[test]
    fn flatten_lit_only_drops_holes() {
        let lit = assert_ok("\"a-{x}-b\"");
        assert_eq!(lit.flatten_lit_only(), "a--b");
    }

    #[test]
    fn error_message_is_non_empty() {
        for err in [
            StringLexError::Unterminated { start: 0 },
            StringLexError::InvalidRawDelimiter {
                hashes: 9,
                start: 0,
            },
            StringLexError::UnbalancedInterpolation { at: 0 },
            StringLexError::MultilineInterpolation { at: 0 },
        ] {
            assert!(!err.message().is_empty());
            assert!(err.id().starts_with("S13"));
        }
    }
}
