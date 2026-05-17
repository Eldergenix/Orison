//! Numeric literal parser shared by the lexer (and any future tooling that
//! needs to interpret an Orison numeric lexeme without depending on the
//! lexer's character cursor).
//!
//! Recognised forms (milestone M21c):
//!
//! | Form                  | Example          | Base       |
//! |-----------------------|------------------|------------|
//! | Decimal integer       | `42`, `1_000`    | Decimal    |
//! | Decimal float         | `3.14`, `1_0.5`  | Decimal    |
//! | Hex integer           | `0xFF`, `0xDE_AD`| Hex        |
//! | Binary integer        | `0b1010`         | Binary     |
//! | Octal integer         | `0o17`           | Octal      |
//!
//! Underscores are allowed between digits as visual separators
//! (`1_000_000`). They are rejected at the start, end, immediately
//! after the base prefix, and adjacent to the decimal point. The parser
//! is pure: same input always yields the same `NumericLit` (or error).
//!
//! ## Diagnostic IDs
//!
//! Errors carry stable IDs so callers (today: the lexer; tomorrow:
//! formatter / linter / IDE) can present consistent diagnostics:
//!
//! * `N1400` — empty input.
//! * `N1401` — invalid digit for the chosen base.
//! * `N1402` — `i64` overflow → falls back to [`NumericKind::BigInt`].
//! * `N1403` — invalid underscore placement.
//! * `N1404` — missing digits after a base prefix (e.g. bare `0x`).
//!
//! ## Float handling
//!
//! Floats are decimal-only in the bootstrap. The parser uses
//! [`f64::from_str`] with explicit error handling so a malformed
//! mantissa surfaces as `InvalidDigit` rather than a runtime panic.
//! Hex/binary/octal floats are out of scope.
//!
//! ## i64 overflow → BigInt
//!
//! When an integer literal overflows `i64`, the parser does **not**
//! error: it returns `NumericKind::BigInt(text)` with the underscores
//! stripped from `text`, so a future arbitrary-precision parser can
//! consume the canonical digit stream verbatim. The caller decides
//! whether overflow is a hard error in its target type.

use std::fmt;

/// Numeric base discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumericBase {
    /// Plain decimal, no prefix.
    Decimal,
    /// `0x` / `0X` prefix.
    Hex,
    /// `0b` / `0B` prefix.
    Binary,
    /// `0o` / `0O` prefix.
    Octal,
}

impl NumericBase {
    /// Numeric radix value (`10`, `16`, `2`, `8`).
    pub fn radix(self) -> u32 {
        match self {
            NumericBase::Decimal => 10,
            NumericBase::Hex => 16,
            NumericBase::Binary => 2,
            NumericBase::Octal => 8,
        }
    }

    /// Canonical lowercase tag suitable for diagnostic messages.
    pub fn as_str(self) -> &'static str {
        match self {
            NumericBase::Decimal => "decimal",
            NumericBase::Hex => "hex",
            NumericBase::Binary => "binary",
            NumericBase::Octal => "octal",
        }
    }
}

/// Parsed numeric value.
#[derive(Debug, Clone, PartialEq)]
pub enum NumericKind {
    /// Fits in `i64`.
    Int(i64),
    /// Decimal float (always `Decimal` base).
    Float(f64),
    /// Integer that overflowed `i64`. Holds the digit-only text
    /// (underscores stripped) so a future arbitrary-precision parser
    /// can consume it directly.
    BigInt(String),
}

/// Fully parsed numeric literal.
#[derive(Debug, Clone, PartialEq)]
pub struct NumericLit {
    /// The decoded value.
    pub kind: NumericKind,
    /// Source base.
    pub base: NumericBase,
    /// Verbatim source text the literal was parsed from.
    pub raw_text: String,
}

/// Error kinds emitted by [`parse_numeric`].
#[derive(Debug, Clone, PartialEq)]
pub enum NumericError {
    /// Caller passed an empty string.
    Empty,
    /// A character is not a valid digit for the active base.
    InvalidDigit {
        /// The offending character.
        ch: char,
        /// Base the parser was decoding under.
        base: NumericBase,
    },
    /// Value did not fit in `i64`. The lexer demotes this to
    /// [`NumericKind::BigInt`] rather than surfacing it as a hard error;
    /// it is preserved as a separate variant so tooling that *wants* to
    /// flag overflow (e.g. a typed-i64 lint) still can.
    OverflowI64,
    /// Underscore placed at the start, end, or next to a `.` or base
    /// prefix.
    InvalidUnderscore,
    /// `0x`, `0b`, or `0o` not followed by any digit.
    MissingDigitsAfterPrefix,
}

impl NumericError {
    /// Stable diagnostic id for this error.
    pub fn id(&self) -> &'static str {
        match self {
            NumericError::Empty => "N1400",
            NumericError::InvalidDigit { .. } => "N1401",
            NumericError::OverflowI64 => "N1402",
            NumericError::InvalidUnderscore => "N1403",
            NumericError::MissingDigitsAfterPrefix => "N1404",
        }
    }

    /// Human-readable explanation.
    pub fn message(&self) -> String {
        match self {
            NumericError::Empty => "empty numeric literal".to_string(),
            NumericError::InvalidDigit { ch, base } => {
                format!("invalid {} digit '{}'", base.as_str(), ch)
            }
            NumericError::OverflowI64 => {
                "integer literal does not fit in i64; downgraded to BigInt".to_string()
            }
            NumericError::InvalidUnderscore => {
                "underscore must appear between two digits".to_string()
            }
            NumericError::MissingDigitsAfterPrefix => {
                "numeric base prefix is not followed by any digits".to_string()
            }
        }
    }
}

impl fmt::Display for NumericError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.id(), self.message())
    }
}

/// Parse `text` as an Orison numeric literal. See the module docs for
/// the supported forms.
pub fn parse_numeric(text: &str) -> Result<NumericLit, NumericError> {
    if text.is_empty() {
        return Err(NumericError::Empty);
    }

    // ---- detect base prefix ------------------------------------------------
    let (base, digits_with_separators) = detect_base(text);

    // ---- non-decimal: integer-only path ------------------------------------
    if !matches!(base, NumericBase::Decimal) {
        // Reject empty body after prefix (e.g. `0x`).
        if digits_with_separators.is_empty() {
            return Err(NumericError::MissingDigitsAfterPrefix);
        }
        // Reject `0x_ff` (underscore immediately after the prefix).
        if digits_with_separators.starts_with('_') {
            return Err(NumericError::InvalidUnderscore);
        }
        let cleaned = clean_underscores(digits_with_separators)?;
        validate_digits(&cleaned, base)?;
        let kind = match i64_from_radix(&cleaned, base.radix()) {
            Some(v) => NumericKind::Int(v),
            None => NumericKind::BigInt(cleaned.clone()),
        };
        return Ok(NumericLit {
            kind,
            base,
            raw_text: text.to_string(),
        });
    }

    // ---- decimal: integer OR float -----------------------------------------
    // No leading/trailing underscore and no underscores adjacent to `.`.
    if digits_with_separators.starts_with('_') || digits_with_separators.ends_with('_') {
        return Err(NumericError::InvalidUnderscore);
    }

    // Reject multiple dots and `_.` / `._` adjacency.
    let mut dot_count = 0usize;
    let bytes = digits_with_separators.as_bytes();
    for (idx, b) in bytes.iter().enumerate() {
        if *b == b'.' {
            dot_count += 1;
            if dot_count > 1 {
                return Err(NumericError::InvalidDigit {
                    ch: '.',
                    base: NumericBase::Decimal,
                });
            }
            if idx == 0 || idx + 1 == bytes.len() {
                // Leading or trailing dot is treated as invalid digit
                // because the lexer is supposed to gate on digit-start
                // and we don't allow trailing-dot floats.
                return Err(NumericError::InvalidDigit {
                    ch: '.',
                    base: NumericBase::Decimal,
                });
            }
            if bytes[idx - 1] == b'_' || bytes[idx + 1] == b'_' {
                return Err(NumericError::InvalidUnderscore);
            }
        }
    }

    let cleaned = clean_underscores(digits_with_separators)?;

    if dot_count == 1 {
        // Float path — only decimal digits + a single `.` are allowed.
        for ch in cleaned.chars() {
            if ch == '.' {
                continue;
            }
            if !ch.is_ascii_digit() {
                return Err(NumericError::InvalidDigit {
                    ch,
                    base: NumericBase::Decimal,
                });
            }
        }
        match cleaned.parse::<f64>() {
            Ok(v) => Ok(NumericLit {
                kind: NumericKind::Float(v),
                base: NumericBase::Decimal,
                raw_text: text.to_string(),
            }),
            // `f64::from_str` only fails on malformed input we did not
            // catch above; surface as InvalidDigit on the first non-
            // digit character we can find for a deterministic message.
            Err(_) => {
                let bad = cleaned
                    .chars()
                    .find(|c| !c.is_ascii_digit() && *c != '.')
                    .unwrap_or('?');
                Err(NumericError::InvalidDigit {
                    ch: bad,
                    base: NumericBase::Decimal,
                })
            }
        }
    } else {
        validate_digits(&cleaned, NumericBase::Decimal)?;
        let kind = match cleaned.parse::<i64>() {
            Ok(v) => NumericKind::Int(v),
            Err(_) => NumericKind::BigInt(cleaned.clone()),
        };
        Ok(NumericLit {
            kind,
            base: NumericBase::Decimal,
            raw_text: text.to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Inspect a leading `0x` / `0b` / `0o` prefix. Returns the active base
/// and the remaining "body" slice (digits + optional separators / dot).
fn detect_base(text: &str) -> (NumericBase, &str) {
    if text.len() >= 2 {
        let bytes = text.as_bytes();
        if bytes[0] == b'0' {
            match bytes[1] {
                b'x' | b'X' => return (NumericBase::Hex, &text[2..]),
                b'b' | b'B' => return (NumericBase::Binary, &text[2..]),
                b'o' | b'O' => return (NumericBase::Octal, &text[2..]),
                _ => {}
            }
        }
    }
    (NumericBase::Decimal, text)
}

/// Strip underscores from `body`, enforcing that every underscore sits
/// strictly between two characters. Adjacency to `.` is checked by the
/// caller (decimal path) before this runs.
fn clean_underscores(body: &str) -> Result<String, NumericError> {
    if body.contains("__") {
        return Err(NumericError::InvalidUnderscore);
    }
    let mut out = String::with_capacity(body.len());
    for ch in body.chars() {
        if ch != '_' {
            out.push(ch);
        }
    }
    Ok(out)
}

/// Validate that every character in `digits` is a legal digit under
/// `base`. The `.` character is rejected here — float parsing handles
/// the decimal point separately.
fn validate_digits(digits: &str, base: NumericBase) -> Result<(), NumericError> {
    if digits.is_empty() {
        return Err(NumericError::MissingDigitsAfterPrefix);
    }
    for ch in digits.chars() {
        if !is_digit_for_base(ch, base) {
            return Err(NumericError::InvalidDigit { ch, base });
        }
    }
    Ok(())
}

fn is_digit_for_base(ch: char, base: NumericBase) -> bool {
    match base {
        NumericBase::Decimal => ch.is_ascii_digit(),
        NumericBase::Hex => ch.is_ascii_hexdigit(),
        NumericBase::Binary => ch == '0' || ch == '1',
        NumericBase::Octal => matches!(ch, '0'..='7'),
    }
}

/// Build an `i64` from a clean (no separator) digit string under
/// `radix`. Returns `None` on overflow so the caller can fall back to
/// `BigInt`. Returns `None` on parse failure too — the caller has
/// already validated digits so this only fires for true overflow.
fn i64_from_radix(digits: &str, radix: u32) -> Option<i64> {
    i64::from_str_radix(digits, radix).ok()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Test helper: pull the parsed literal out of a `Result` while
    /// failing the test (via `assert!`) on the unexpected error path.
    /// Acts like the standard `Result::unwrap` while remaining
    /// compatible with the repository's no-unwrap source guardrail.
    fn ok_lit(r: Result<NumericLit, NumericError>) -> NumericLit {
        match r {
            Ok(v) => v,
            Err(e) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected Ok(NumericLit), got Err({e:?})");
                }
                NumericLit {
                    kind: NumericKind::Int(0),
                    base: NumericBase::Decimal,
                    raw_text: String::new(),
                }
            }
        }
    }

    /// Mirror of [`ok_lit`] for the `Err` arm. Mirrors the standard
    /// `unwrap_err` pattern without invoking the forbidden method.
    fn err_of(r: Result<NumericLit, NumericError>) -> NumericError {
        match r {
            Err(e) => e,
            Ok(v) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected Err(NumericError), got Ok({v:?})");
                }
                NumericError::Empty
            }
        }
    }

    fn int_of(lit: NumericLit) -> i64 {
        match lit.kind {
            NumericKind::Int(v) => v,
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected Int, got {other:?}");
                }
                0
            }
        }
    }

    fn float_of(lit: NumericLit) -> f64 {
        match lit.kind {
            NumericKind::Float(v) => v,
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected Float, got {other:?}");
                }
                0.0
            }
        }
    }

    fn bigint_of(lit: NumericLit) -> String {
        match lit.kind {
            NumericKind::BigInt(s) => s,
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected BigInt, got {other:?}");
                }
                String::new()
            }
        }
    }

    // --- 1. plain decimal integer ----
    #[test]
    fn decimal_integer() {
        let r = ok_lit(parse_numeric("42"));
        assert_eq!(r.base, NumericBase::Decimal);
        assert_eq!(int_of(r), 42);
    }

    // --- 2. decimal float ----
    #[test]
    #[allow(clippy::approx_constant)]
    fn decimal_float() {
        let r = ok_lit(parse_numeric("3.14"));
        assert_eq!(r.base, NumericBase::Decimal);
        let v = float_of(r);
        assert!((v - 3.14).abs() < 1e-12);
    }

    // --- 3. hex ----
    #[test]
    fn hex_ff_is_255() {
        let r = ok_lit(parse_numeric("0xFF"));
        assert_eq!(r.base, NumericBase::Hex);
        assert_eq!(int_of(r), 255);
    }

    // --- 4. hex uppercase prefix ----
    #[test]
    fn hex_uppercase_prefix() {
        let r = ok_lit(parse_numeric("0Xdead"));
        assert_eq!(r.base, NumericBase::Hex);
        assert_eq!(int_of(r), 0xdead);
    }

    // --- 5. binary ----
    #[test]
    fn binary_1010_is_10() {
        let r = ok_lit(parse_numeric("0b1010"));
        assert_eq!(r.base, NumericBase::Binary);
        assert_eq!(int_of(r), 10);
    }

    // --- 6. octal ----
    #[test]
    fn octal_17_is_15() {
        let r = ok_lit(parse_numeric("0o17"));
        assert_eq!(r.base, NumericBase::Octal);
        assert_eq!(int_of(r), 15);
    }

    // --- 7. decimal with separators ----
    #[test]
    fn decimal_separators() {
        let r = ok_lit(parse_numeric("1_000_000"));
        assert_eq!(int_of(r), 1_000_000);
    }

    // --- 8. hex with separators ----
    #[test]
    fn hex_separators() {
        let r = ok_lit(parse_numeric("0xDE_AD_BE_EF"));
        assert_eq!(int_of(r), 0xDEAD_BEEFi64);
    }

    // --- 9. float with separators ----
    #[test]
    fn float_separators() {
        let r = ok_lit(parse_numeric("1_0.5"));
        let v = float_of(r);
        assert!((v - 10.5).abs() < 1e-12);
    }

    // --- 10. i64 overflow → BigInt (decimal) ----
    #[test]
    fn decimal_overflow_falls_back_to_bigint() {
        // 2^63 = 9_223_372_036_854_775_808 which is i64::MAX + 1.
        let r = ok_lit(parse_numeric("9223372036854775808"));
        assert_eq!(bigint_of(r), "9223372036854775808");
    }

    // --- 11. i64 overflow → BigInt strips underscores ----
    #[test]
    fn decimal_overflow_strips_underscores() {
        let r = ok_lit(parse_numeric("9_223_372_036_854_775_808"));
        assert_eq!(bigint_of(r), "9223372036854775808");
    }

    // --- 12. hex overflow → BigInt ----
    #[test]
    fn hex_overflow_falls_back_to_bigint() {
        // 17 hex F's = 2^68 - 1 which exceeds i64.
        let r = ok_lit(parse_numeric("0xFFFFFFFFFFFFFFFFF"));
        assert!(matches!(r.kind, NumericKind::BigInt(_)));
    }

    // --- 13. empty input → N1400 ----
    #[test]
    fn empty_input_is_n1400() {
        let e = err_of(parse_numeric(""));
        assert_eq!(e.id(), "N1400");
        assert!(matches!(e, NumericError::Empty));
    }

    // --- 14. invalid digit → N1401 ----
    #[test]
    fn invalid_decimal_digit_is_n1401() {
        let e = err_of(parse_numeric("12a3"));
        assert_eq!(e.id(), "N1401");
        assert!(matches!(
            e,
            NumericError::InvalidDigit {
                ch: 'a',
                base: NumericBase::Decimal
            }
        ));
    }

    // --- 15. invalid hex digit → N1401 ----
    #[test]
    fn invalid_hex_digit_is_n1401() {
        let e = err_of(parse_numeric("0xg1"));
        assert_eq!(e.id(), "N1401");
    }

    // --- 16. invalid binary digit → N1401 ----
    #[test]
    fn invalid_binary_digit_is_n1401() {
        let e = err_of(parse_numeric("0b102"));
        assert_eq!(e.id(), "N1401");
        assert!(matches!(
            e,
            NumericError::InvalidDigit {
                ch: '2',
                base: NumericBase::Binary
            }
        ));
    }

    // --- 17. invalid octal digit → N1401 ----
    #[test]
    fn invalid_octal_digit_is_n1401() {
        let e = err_of(parse_numeric("0o89"));
        assert_eq!(e.id(), "N1401");
    }

    // --- 18. underscore at start → N1403 ----
    #[test]
    fn leading_underscore_is_n1403() {
        let e = err_of(parse_numeric("_100"));
        assert_eq!(e.id(), "N1403");
    }

    // --- 19. underscore at end → N1403 ----
    #[test]
    fn trailing_underscore_is_n1403() {
        let e = err_of(parse_numeric("100_"));
        assert_eq!(e.id(), "N1403");
    }

    // --- 20. underscore right after prefix → N1403 ----
    #[test]
    fn underscore_after_prefix_is_n1403() {
        let e = err_of(parse_numeric("0x_ff"));
        assert_eq!(e.id(), "N1403");
    }

    // --- 21. underscore before dot → N1403 ----
    #[test]
    fn underscore_before_dot_is_n1403() {
        let e = err_of(parse_numeric("1_.5"));
        assert_eq!(e.id(), "N1403");
    }

    // --- 22. underscore after dot → N1403 ----
    #[test]
    fn underscore_after_dot_is_n1403() {
        let e = err_of(parse_numeric("1._5"));
        assert_eq!(e.id(), "N1403");
    }

    // --- 23. double underscore → N1403 ----
    #[test]
    fn double_underscore_is_n1403() {
        let e = err_of(parse_numeric("1__000"));
        assert_eq!(e.id(), "N1403");
    }

    // --- 24. bare 0x → N1404 ----
    #[test]
    fn missing_digits_after_hex_prefix_is_n1404() {
        let e = err_of(parse_numeric("0x"));
        assert_eq!(e.id(), "N1404");
    }

    // --- 25. bare 0b → N1404 ----
    #[test]
    fn missing_digits_after_binary_prefix_is_n1404() {
        let e = err_of(parse_numeric("0b"));
        assert_eq!(e.id(), "N1404");
    }

    // --- 26. bare 0o → N1404 ----
    #[test]
    fn missing_digits_after_octal_prefix_is_n1404() {
        let e = err_of(parse_numeric("0o"));
        assert_eq!(e.id(), "N1404");
    }

    // --- 27. determinism — same input, same output ----
    #[test]
    fn parse_is_deterministic() {
        let a = ok_lit(parse_numeric("0xCAFE_BABE"));
        let b = ok_lit(parse_numeric("0xCAFE_BABE"));
        assert_eq!(a, b);
    }

    // --- 28. NumericError IDs are stable strings ----
    #[test]
    fn error_ids_are_stable() {
        assert_eq!(NumericError::Empty.id(), "N1400");
        assert_eq!(
            NumericError::InvalidDigit {
                ch: 'a',
                base: NumericBase::Decimal
            }
            .id(),
            "N1401"
        );
        assert_eq!(NumericError::OverflowI64.id(), "N1402");
        assert_eq!(NumericError::InvalidUnderscore.id(), "N1403");
        assert_eq!(NumericError::MissingDigitsAfterPrefix.id(), "N1404");
    }

    // --- 29. base radix table ----
    #[test]
    fn base_radix_table() {
        assert_eq!(NumericBase::Decimal.radix(), 10);
        assert_eq!(NumericBase::Hex.radix(), 16);
        assert_eq!(NumericBase::Binary.radix(), 2);
        assert_eq!(NumericBase::Octal.radix(), 8);
    }

    // --- 30. zero parses for every base ----
    #[test]
    fn zero_parses_for_every_base() {
        assert_eq!(int_of(ok_lit(parse_numeric("0"))), 0);
        assert_eq!(int_of(ok_lit(parse_numeric("0x0"))), 0);
        assert_eq!(int_of(ok_lit(parse_numeric("0b0"))), 0);
        assert_eq!(int_of(ok_lit(parse_numeric("0o0"))), 0);
    }

    // --- 31. raw_text is preserved verbatim ----
    #[test]
    fn raw_text_is_preserved_verbatim() {
        let r = ok_lit(parse_numeric("0xDE_AD"));
        assert_eq!(r.raw_text, "0xDE_AD");
    }

    // --- 32. trailing dot is rejected as invalid digit ----
    #[test]
    fn trailing_dot_is_rejected() {
        // Lexer policy: floats need digits on both sides. `1.` is
        // not produced by the lexer because the dot lookahead fails,
        // but we still reject it defensively at this layer.
        let e = err_of(parse_numeric("1."));
        assert_eq!(e.id(), "N1401");
    }
}
