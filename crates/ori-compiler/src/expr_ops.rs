//! Binary and unary operator support for the body-expression parser.
//!
//! This module is intentionally *data-only*: it owns the `BinOp` / `UnOp`
//! enums, the lexeme tables that map source tokens onto those enums, and
//! the precedence / associativity tables consumed by the Pratt loop in
//! [`crate::expr`]. The parser itself lives in `expr.rs` so that the
//! existing primary-expression machinery remains the single source of
//! truth for sub-expression parsing.
//!
//! ## Precedence table
//!
//! Higher numbers bind tighter. The full ladder is:
//!
//! | Level | Operators                              |
//! |-------|----------------------------------------|
//! | 1     | `\|\|`                                 |
//! | 2     | `&&`                                   |
//! | 3     | `==`, `!=`                             |
//! | 4     | `<`, `<=`, `>`, `>=`                   |
//! | 5     | `+`, `-`                               |
//! | 6     | `*`, `/`, `%`                          |
//! | 7     | `??`                                   |
//!
//! All operators are left-associative *except* `??`, which is
//! right-associative so that `a ?? b ?? c` parses as `a ?? (b ?? c)` —
//! the conventional shape for null-coalescing fallback chains.
//!
//! ## Diagnostics
//!
//! When the parser cannot find an operand on the right-hand side of a
//! binary operator it emits diagnostic `E1200`. That ID is owned by this
//! module and must remain stable; it is part of the structured-JSON
//! contract surfaced to agents and IDE clients.

/// Binary operator kinds recognised by the body parser. The discriminant
/// order is **not** semantically meaningful; precedence is encoded
/// separately in [`precedence`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Rem,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `&&`
    And,
    /// `||`
    Or,
    /// `??`
    Coalesce,
}

/// Unary operator kinds recognised by the body parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    /// `-x`
    Neg,
    /// `!x`
    Not,
}

/// Stable diagnostic ID for "expected operand after binary operator".
pub const E_MISSING_BIN_OPERAND: &str = "E1200";

impl BinOp {
    /// Canonical source lexeme for this operator. Round-trips with
    /// [`binop_for_lexeme`]: for every `op`, `binop_for_lexeme(op.lexeme())
    /// == Some(op)`. Used by textual codegen so an `Expr::Binary` can be
    /// re-emitted as parseable source.
    pub fn lexeme(&self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "&&",
            BinOp::Or => "||",
            BinOp::Coalesce => "??",
        }
    }
}

impl UnOp {
    /// Canonical source lexeme for this unary operator. Round-trips with
    /// [`unop_for_lexeme`]. Used by textual codegen for `Expr::Unary`.
    pub fn lexeme(&self) -> &'static str {
        match self {
            UnOp::Neg => "-",
            UnOp::Not => "!",
        }
    }
}

/// Map a lexeme onto its [`BinOp`], if any. Returns `None` for tokens
/// that are not binary operators so the caller can fall through to other
/// productions (postfix `?`, `,`, etc.).
pub fn binop_for_lexeme(lexeme: &str) -> Option<BinOp> {
    match lexeme {
        "+" => Some(BinOp::Add),
        "-" => Some(BinOp::Sub),
        "*" => Some(BinOp::Mul),
        "/" => Some(BinOp::Div),
        "%" => Some(BinOp::Rem),
        "==" => Some(BinOp::Eq),
        "!=" => Some(BinOp::Ne),
        "<" => Some(BinOp::Lt),
        "<=" => Some(BinOp::Le),
        ">" => Some(BinOp::Gt),
        ">=" => Some(BinOp::Ge),
        "&&" => Some(BinOp::And),
        "||" => Some(BinOp::Or),
        "??" => Some(BinOp::Coalesce),
        _ => None,
    }
}

/// Map a lexeme onto its [`UnOp`], if any. The same lexeme (`-`) can also
/// be a binary operator; the parser distinguishes them by position
/// (prefix vs. infix).
pub fn unop_for_lexeme(lexeme: &str) -> Option<UnOp> {
    match lexeme {
        "-" => Some(UnOp::Neg),
        "!" => Some(UnOp::Not),
        _ => None,
    }
}

/// Pratt-style precedence for the given binary operator. The single
/// `match` here is the only source of truth — every other site that
/// needs to compare precedence must call this function rather than
/// duplicating the ladder.
pub fn precedence(op: BinOp) -> u8 {
    match op {
        BinOp::Or => 1,
        BinOp::And => 2,
        BinOp::Eq | BinOp::Ne => 3,
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 4,
        BinOp::Add | BinOp::Sub => 5,
        BinOp::Mul | BinOp::Div | BinOp::Rem => 6,
        BinOp::Coalesce => 7,
    }
}

/// `true` iff `op` is right-associative. Only `??` is right-associative
/// in the bootstrap grammar; everything else folds left.
pub fn is_right_associative(op: BinOp) -> bool {
    matches!(op, BinOp::Coalesce)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Every binary operator round-trips through [`binop_for_lexeme`] using
    /// its canonical lexeme.
    #[test]
    fn binop_lexemes_round_trip() {
        let pairs: &[(&str, BinOp)] = &[
            ("+", BinOp::Add),
            ("-", BinOp::Sub),
            ("*", BinOp::Mul),
            ("/", BinOp::Div),
            ("%", BinOp::Rem),
            ("==", BinOp::Eq),
            ("!=", BinOp::Ne),
            ("<", BinOp::Lt),
            ("<=", BinOp::Le),
            (">", BinOp::Gt),
            (">=", BinOp::Ge),
            ("&&", BinOp::And),
            ("||", BinOp::Or),
            ("??", BinOp::Coalesce),
        ];
        for (lexeme, op) in pairs {
            assert_eq!(binop_for_lexeme(lexeme), Some(*op));
        }
    }

    /// Lexemes that are not binary operators return `None`.
    #[test]
    fn binop_unknown_returns_none() {
        for lexeme in [".", ",", "(", ")", "->", "=>", "?", "@"] {
            assert_eq!(binop_for_lexeme(lexeme), None);
        }
    }

    /// Unary operator lexemes round-trip.
    #[test]
    fn unop_lexemes_round_trip() {
        assert_eq!(unop_for_lexeme("-"), Some(UnOp::Neg));
        assert_eq!(unop_for_lexeme("!"), Some(UnOp::Not));
        assert_eq!(unop_for_lexeme("+"), None);
        assert_eq!(unop_for_lexeme("?"), None);
    }

    /// The precedence ladder is monotone: each level binds strictly tighter
    /// than the level above it for the operators in the milestone grammar.
    #[test]
    fn precedence_is_monotone() {
        assert!(precedence(BinOp::Or) < precedence(BinOp::And));
        assert!(precedence(BinOp::And) < precedence(BinOp::Eq));
        assert!(precedence(BinOp::Eq) == precedence(BinOp::Ne));
        assert!(precedence(BinOp::Ne) < precedence(BinOp::Lt));
        assert!(precedence(BinOp::Lt) == precedence(BinOp::Le));
        assert!(precedence(BinOp::Le) == precedence(BinOp::Gt));
        assert!(precedence(BinOp::Gt) == precedence(BinOp::Ge));
        assert!(precedence(BinOp::Ge) < precedence(BinOp::Add));
        assert!(precedence(BinOp::Add) == precedence(BinOp::Sub));
        assert!(precedence(BinOp::Sub) < precedence(BinOp::Mul));
        assert!(precedence(BinOp::Mul) == precedence(BinOp::Div));
        assert!(precedence(BinOp::Div) == precedence(BinOp::Rem));
        assert!(precedence(BinOp::Rem) < precedence(BinOp::Coalesce));
    }

    /// Only `??` is right-associative.
    #[test]
    fn associativity_only_coalesce_is_right() {
        assert!(is_right_associative(BinOp::Coalesce));
        for op in [
            BinOp::Add,
            BinOp::Sub,
            BinOp::Mul,
            BinOp::Div,
            BinOp::Rem,
            BinOp::Eq,
            BinOp::Ne,
            BinOp::Lt,
            BinOp::Le,
            BinOp::Gt,
            BinOp::Ge,
            BinOp::And,
            BinOp::Or,
        ] {
            assert!(
                !is_right_associative(op),
                "{op:?} should be left-associative"
            );
        }
    }

    /// The diagnostic ID is stable and uses the documented prefix.
    #[test]
    fn diagnostic_id_is_stable() {
        assert_eq!(E_MISSING_BIN_OPERAND, "E1200");
    }

    /// Every `BinOp::lexeme()` round-trips through `binop_for_lexeme`.
    #[test]
    fn binop_lexeme_method_round_trips() {
        for op in [
            BinOp::Add,
            BinOp::Sub,
            BinOp::Mul,
            BinOp::Div,
            BinOp::Rem,
            BinOp::Eq,
            BinOp::Ne,
            BinOp::Lt,
            BinOp::Le,
            BinOp::Gt,
            BinOp::Ge,
            BinOp::And,
            BinOp::Or,
            BinOp::Coalesce,
        ] {
            assert_eq!(binop_for_lexeme(op.lexeme()), Some(op));
        }
    }

    /// Every `UnOp::lexeme()` round-trips through `unop_for_lexeme`.
    #[test]
    fn unop_lexeme_method_round_trips() {
        for op in [UnOp::Neg, UnOp::Not] {
            assert_eq!(unop_for_lexeme(op.lexeme()), Some(op));
        }
    }
}
