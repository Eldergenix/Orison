//! Hand-rolled TOML subset parser.
//!
//! Accepted grammar:
//!
//! * `# line comment` (terminated by newline)
//! * `[section.name]` headers (dotted names allowed)
//! * `key = "value"` string assignments
//! * `key = ["a", "b"]` arrays of strings (single-line, trailing comma OK)
//! * Bare keys: `[A-Za-z0-9_-]+`. Tabs in keys are rejected.
//!
//! Explicitly *not* supported:
//!
//! * Nested tables, `[[arrays.of.tables]]`, inline tables.
//! * Integer/float/bool/datetime values.
//! * Multi-line strings and literal strings.
//! * Escape sequences other than `\\` and `\"`.
//!
//! Duplicate keys within the same table are an error. Unterminated strings,
//! unknown escapes, and tabs inside bare keys are errors. Every error carries
//! a 1-indexed line and column pointing at the offending character.

use std::collections::BTreeMap;
use std::fmt;

/// Parsed TOML value. The subset we accept only produces strings and arrays of
/// strings, so the value type is intentionally small.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TomlValue {
    /// Single string value (`key = "value"`).
    String(String),
    /// Array of strings (`key = ["a", "b"]`).
    Array(Vec<String>),
}

/// Categorised TOML parse error. The wrapping [`TomlError`] carries position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TomlErrorKind {
    /// A string literal ran past the end of its line without a closing `"`.
    UnterminatedString,
    /// `\\x` where `x` is not one of the supported escape characters.
    UnknownEscape(char),
    /// A bare key contained a tab character.
    TabInKey,
    /// A key was assigned more than once within the same table.
    DuplicateKey(String),
    /// A required `=` token was missing after a key.
    ExpectedEquals,
    /// Unexpected token or character while parsing a value/header.
    Unexpected(String),
    /// `[header]` was opened but not closed before end-of-line.
    UnterminatedHeader,
    /// Section header was empty or contained invalid characters.
    InvalidHeader,
    /// A value other than a string or array was found.
    UnsupportedValue,
}

impl fmt::Display for TomlErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TomlErrorKind::UnterminatedString => f.write_str("unterminated string literal"),
            TomlErrorKind::UnknownEscape(c) => write!(f, "unknown escape sequence `\\{c}`"),
            TomlErrorKind::TabInKey => f.write_str("tab character is not allowed in keys"),
            TomlErrorKind::DuplicateKey(name) => write!(f, "duplicate key `{name}`"),
            TomlErrorKind::ExpectedEquals => f.write_str("expected `=` after key"),
            TomlErrorKind::Unexpected(found) => write!(f, "unexpected `{found}`"),
            TomlErrorKind::UnterminatedHeader => f.write_str("unterminated section header"),
            TomlErrorKind::InvalidHeader => f.write_str("invalid section header"),
            TomlErrorKind::UnsupportedValue => {
                f.write_str("only string and string-array values are supported")
            }
        }
    }
}

/// TOML parse error with 1-indexed source position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TomlError {
    /// The category of the error.
    pub kind: TomlErrorKind,
    /// 1-indexed line number where the error was detected.
    pub line: usize,
    /// 1-indexed column number where the error was detected.
    pub column: usize,
}

impl fmt::Display for TomlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at line {} column {}",
            self.kind, self.line, self.column
        )
    }
}

impl std::error::Error for TomlError {}

/// Parsed TOML document. Top-level keys appear under the empty section name
/// (`""`). Section names are kept verbatim (dotted, e.g. `"package.meta"`).
pub type TomlDocument = BTreeMap<String, BTreeMap<String, TomlValue>>;

/// Parse a TOML manifest document.
///
/// The returned map is keyed by section name (with `""` for top-level keys)
/// and each section is keyed by bare key name. Ordering is deterministic
/// because [`BTreeMap`] is sorted by key.
pub fn parse_manifest(text: &str) -> Result<TomlDocument, TomlError> {
    let mut doc: TomlDocument = TomlDocument::new();
    doc.insert(String::new(), BTreeMap::new());
    let mut current_section = String::new();

    for (line_idx, raw_line) in text.split('\n').enumerate() {
        let line_number = line_idx + 1;
        let line = strip_inline_comment(raw_line);
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Section header.
        if trimmed.starts_with('[') {
            let header_col = line.find('[').unwrap_or(0) + 1;
            if !trimmed.ends_with(']') {
                return Err(TomlError {
                    kind: TomlErrorKind::UnterminatedHeader,
                    line: line_number,
                    column: header_col,
                });
            }
            let inside = &trimmed[1..trimmed.len() - 1];
            let header = inside.trim();
            if header.is_empty() || !is_valid_header(header) {
                return Err(TomlError {
                    kind: TomlErrorKind::InvalidHeader,
                    line: line_number,
                    column: header_col,
                });
            }
            current_section = header.to_string();
            doc.entry(current_section.clone()).or_default();
            continue;
        }

        // key = value
        let (key, key_end_col) = read_key(line, line_number)?;
        let after_key = &line[key_end_col - 1..];
        let after_key_trimmed = after_key.trim_start();
        if !after_key_trimmed.starts_with('=') {
            return Err(TomlError {
                kind: TomlErrorKind::ExpectedEquals,
                line: line_number,
                column: column_in_line(line, after_key_trimmed),
            });
        }
        let after_eq_offset = column_in_line(line, after_key_trimmed) + 1;
        let after_eq = &line[after_eq_offset - 1..];
        let value_part = after_eq.trim_start();
        let value_col = column_in_line(line, value_part);
        let value = parse_value(value_part, line_number, value_col)?;

        let table = doc.entry(current_section.clone()).or_default();
        if table.contains_key(&key) {
            return Err(TomlError {
                kind: TomlErrorKind::DuplicateKey(key),
                line: line_number,
                column: column_in_line(line, line.trim_start()),
            });
        }
        table.insert(key, value);
    }

    Ok(doc)
}

/// Strip a `#` comment from a line. Inline comments inside string literals are
/// not stripped: we walk the line character by character, tracking string
/// state.
fn strip_inline_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_string = false;
    let mut escape = false;
    let mut idx = 0;
    while idx < bytes.len() {
        let b = bytes[idx];
        if in_string {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_string = false;
            }
        } else if b == b'"' {
            in_string = true;
        } else if b == b'#' {
            return &line[..idx];
        }
        idx += 1;
    }
    line
}

fn is_valid_header(header: &str) -> bool {
    if header.is_empty() {
        return false;
    }
    // Allow dotted: each segment must be a valid bare key.
    header.split('.').all(|segment| {
        let s = segment.trim();
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    })
}

/// Read a bare key starting from the trimmed-left portion of `line`. Returns
/// the key plus the 1-indexed column of the first character after the key.
fn read_key(line: &str, line_number: usize) -> Result<(String, usize), TomlError> {
    let trimmed_start = line.trim_start();
    let leading = line.len() - trimmed_start.len();
    let mut end = 0usize;
    let bytes = trimmed_start.as_bytes();
    while end < bytes.len() {
        let b = bytes[end];
        if b == b'\t' {
            return Err(TomlError {
                kind: TomlErrorKind::TabInKey,
                line: line_number,
                column: leading + end + 1,
            });
        }
        if b == b' ' || b == b'=' {
            break;
        }
        if !is_bare_key_char(b) {
            return Err(TomlError {
                kind: TomlErrorKind::Unexpected((b as char).to_string()),
                line: line_number,
                column: leading + end + 1,
            });
        }
        end += 1;
    }
    if end == 0 {
        return Err(TomlError {
            kind: TomlErrorKind::Unexpected(
                trimmed_start
                    .chars()
                    .next()
                    .map(|c| c.to_string())
                    .unwrap_or_default(),
            ),
            line: line_number,
            column: leading + 1,
        });
    }
    let key = trimmed_start[..end].to_string();
    Ok((key, leading + end + 1))
}

fn is_bare_key_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Compute the 1-indexed column number of `sub` inside `line`. Both slices
/// must come from the same backing string.
fn column_in_line(line: &str, sub: &str) -> usize {
    let line_ptr = line.as_ptr() as usize;
    let sub_ptr = sub.as_ptr() as usize;
    if sub_ptr < line_ptr || sub_ptr > line_ptr + line.len() {
        return 1;
    }
    sub_ptr - line_ptr + 1
}

fn parse_value(text: &str, line: usize, column: usize) -> Result<TomlValue, TomlError> {
    let trimmed = text.trim_end();
    if let Some(rest) = trimmed.strip_prefix('"') {
        let (s, end) = parse_string(rest, line, column + 1)?;
        // Ensure nothing after the closing quote except whitespace (already trimmed).
        let consumed = end - column; // characters consumed including closing quote
        let after = &trimmed[consumed..];
        if !after.trim().is_empty() {
            return Err(TomlError {
                kind: TomlErrorKind::Unexpected(
                    after.trim().chars().next().unwrap_or(' ').to_string(),
                ),
                line,
                column: column + consumed,
            });
        }
        Ok(TomlValue::String(s))
    } else if let Some(rest) = trimmed.strip_prefix('[') {
        let items = parse_string_array(rest, line, column + 1)?;
        Ok(TomlValue::Array(items))
    } else {
        Err(TomlError {
            kind: TomlErrorKind::UnsupportedValue,
            line,
            column,
        })
    }
}

/// Parse a string literal whose opening `"` has already been consumed. Returns
/// the decoded string and the 1-indexed column directly after the closing
/// quote.
fn parse_string(rest: &str, line: usize, start_col: usize) -> Result<(String, usize), TomlError> {
    let mut out = String::new();
    let mut chars = rest.char_indices();
    while let Some((idx, ch)) = chars.next() {
        match ch {
            '"' => {
                // Position after the closing quote.
                return Ok((out, start_col + idx + 1));
            }
            '\\' => match chars.next() {
                Some((_, '\\')) => out.push('\\'),
                Some((_, '"')) => out.push('"'),
                Some((_, other)) => {
                    return Err(TomlError {
                        kind: TomlErrorKind::UnknownEscape(other),
                        line,
                        column: start_col + idx,
                    });
                }
                None => {
                    return Err(TomlError {
                        kind: TomlErrorKind::UnterminatedString,
                        line,
                        column: start_col + idx,
                    });
                }
            },
            '\n' | '\r' => {
                return Err(TomlError {
                    kind: TomlErrorKind::UnterminatedString,
                    line,
                    column: start_col + idx,
                });
            }
            other => out.push(other),
        }
    }
    Err(TomlError {
        kind: TomlErrorKind::UnterminatedString,
        line,
        column: start_col + rest.len(),
    })
}

/// Parse a string array whose opening `[` has already been consumed.
fn parse_string_array(rest: &str, line: usize, start_col: usize) -> Result<Vec<String>, TomlError> {
    let mut items = Vec::new();
    let mut cursor = 0usize;
    let bytes = rest.as_bytes();
    loop {
        // Skip whitespace.
        while cursor < bytes.len() && (bytes[cursor] == b' ' || bytes[cursor] == b'\t') {
            cursor += 1;
        }
        if cursor >= bytes.len() {
            return Err(TomlError {
                kind: TomlErrorKind::Unexpected("end of line".to_string()),
                line,
                column: start_col + cursor,
            });
        }
        if bytes[cursor] == b']' {
            // Verify nothing after `]` except whitespace.
            let after = &rest[cursor + 1..];
            if !after.trim().is_empty() {
                return Err(TomlError {
                    kind: TomlErrorKind::Unexpected(
                        after.trim().chars().next().unwrap_or(' ').to_string(),
                    ),
                    line,
                    column: start_col + cursor + 1,
                });
            }
            return Ok(items);
        }
        if bytes[cursor] != b'"' {
            return Err(TomlError {
                kind: TomlErrorKind::Unexpected((bytes[cursor] as char).to_string()),
                line,
                column: start_col + cursor,
            });
        }
        let (s, end_col) = parse_string(&rest[cursor + 1..], line, start_col + cursor + 1)?;
        items.push(s);
        // end_col is column immediately after closing quote (relative to original line).
        cursor = end_col - start_col;
        // Skip whitespace.
        while cursor < bytes.len() && (bytes[cursor] == b' ' || bytes[cursor] == b'\t') {
            cursor += 1;
        }
        if cursor >= bytes.len() {
            return Err(TomlError {
                kind: TomlErrorKind::Unexpected("end of line".to_string()),
                line,
                column: start_col + cursor,
            });
        }
        match bytes[cursor] {
            b',' => {
                cursor += 1;
            }
            b']' => {
                // Re-loop and let the closing arm handle it.
            }
            other => {
                return Err(TomlError {
                    kind: TomlErrorKind::Unexpected((other as char).to_string()),
                    line,
                    column: start_col + cursor,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::assertions_on_constants)]
    fn ok_doc(text: &str) -> TomlDocument {
        match parse_manifest(text) {
            Ok(doc) => doc,
            Err(err) => {
                // See the note in `manifest.rs::parses_full_sample` — this
                // crate forbids `panic!` / `unreachable!` so an always-false
                // assertion is the only way to surface a failure cleanly.
                assert!(false, "expected toml to parse: {err}");
                TomlDocument::new()
            }
        }
    }

    #[test]
    fn parses_basic_keys() {
        let text = "name = \"a\"\nedition = \"2027.1\"\n";
        let doc = ok_doc(text);
        let root = doc.get("").cloned().unwrap_or_default();
        assert_eq!(root.get("name"), Some(&TomlValue::String("a".into())));
        assert_eq!(
            root.get("edition"),
            Some(&TomlValue::String("2027.1".into()))
        );
    }

    #[test]
    fn parses_section_and_array() {
        let text = "[package]\nname = \"x\"\n\n[deps]\nlist = [\"a\", \"b\"]\n";
        let doc = ok_doc(text);
        let pkg = doc.get("package").cloned().unwrap_or_default();
        assert_eq!(pkg.get("name"), Some(&TomlValue::String("x".into())));
        let deps = doc.get("deps").cloned().unwrap_or_default();
        assert_eq!(
            deps.get("list"),
            Some(&TomlValue::Array(vec!["a".into(), "b".into()]))
        );
    }

    #[test]
    fn rejects_unterminated_string() {
        let err = parse_manifest("name = \"oops\n").unwrap_err();
        assert_eq!(err.kind, TomlErrorKind::UnterminatedString);
        assert_eq!(err.line, 1);
    }

    #[test]
    fn rejects_duplicate_key() {
        let err = parse_manifest("a = \"1\"\na = \"2\"\n").unwrap_err();
        assert_eq!(err.kind, TomlErrorKind::DuplicateKey("a".into()));
        assert_eq!(err.line, 2);
    }

    #[test]
    fn rejects_unknown_escape() {
        let err = parse_manifest("a = \"\\n\"\n").unwrap_err();
        assert_eq!(err.kind, TomlErrorKind::UnknownEscape('n'));
    }

    #[test]
    fn rejects_tab_in_key() {
        let err = parse_manifest("a\tb = \"x\"\n").unwrap_err();
        assert_eq!(err.kind, TomlErrorKind::TabInKey);
    }

    #[test]
    fn ignores_comments_and_inline_hash_in_strings() {
        let text = "# leading comment\nurl = \"http://x#anchor\" # trailing\n";
        let doc = ok_doc(text);
        assert_eq!(
            doc.get("").and_then(|t| t.get("url")),
            Some(&TomlValue::String("http://x#anchor".into()))
        );
    }
}
