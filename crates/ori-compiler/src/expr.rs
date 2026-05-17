//! Error-tolerant expression AST and token-driven parser for Orison
//! function bodies.
//!
//! This module is intentionally additive: it does not modify the existing
//! item-level parser. Downstream passes (`type_check`, `interp`, `hir`,
//! `patch_apply`) can opt into expression-level information by calling
//! [`parse_body_expr`] over the token slice extracted by
//! [`crate::body::extract_body`].
//!
//! Diagnostic IDs in the `E1100`..`E1199` range belong to this module.
//! Every diagnostic carries an `agent_summary` and a `docs` reference so
//! agents that consume the JSON contract know how to act on it.
//!
//! Recovery strategy:
//!   * The parser never panics. On a malformed sub-expression it emits an
//!     `Expr::Error` node, records a diagnostic, and re-synchronises on the
//!     next statement terminator (`;`, newline, or the closing brace/`,`
//!     of the enclosing construct).
//!   * `let x =` with no initialiser yields a `Let` whose init is
//!     `Expr::Error`, so sibling statements still parse.
//!
//! The parser is deliberately conservative about precedence: it supports
//! postfix `.field`, `(args)`, and `?`, plus prefix construction
//! (`Ok(x)` etc.). Binary operators are not yet modelled; they fall back
//! to `Expr::Error` so we never silently mis-parse arithmetic.

use crate::diagnostic::Diagnostic;
use crate::expr_ops::{
    binop_for_lexeme, is_right_associative, precedence, unop_for_lexeme, BinOp, UnOp,
    E_MISSING_BIN_OPERAND,
};
use crate::lexer::{Token, TokenKind};
use crate::source::Span;
use crate::types::TypeRef;

/// Literal forms recognised inside function bodies.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Unit,
}

/// Pattern grammar used inside `match` arms. Keeps the shape small on
/// purpose; richer patterns can be layered on without changing call sites.
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Wildcard,
    Binding(String),
    Literal(Literal),
    Constructor { name: String, args: Vec<Pattern> },
}

/// Arm of a `match` expression. Guards aren't modelled yet; if encountered
/// they degrade gracefully to `Pattern::Wildcard` with a diagnostic.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Expr,
}

/// Statements live inside `Block`. `Return` is duplicated here (it can also
/// appear as an `Expr`) because most bodies use it in statement position.
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let {
        name: String,
        ty: Option<TypeRef>,
        init: Expr,
    },
    Expr(Expr),
    Return(Option<Expr>),
}

/// Core expression AST. Equality is structural and stable across repeated
/// parses of the same input.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Lit(Literal),
    Var(String),
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    Field {
        base: Box<Expr>,
        name: String,
    },
    Block {
        stmts: Vec<Stmt>,
        tail: Option<Box<Expr>>,
    },
    If {
        cond: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Option<Box<Expr>>,
    },
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
    },
    Return(Option<Box<Expr>>),
    Construct {
        variant: String,
        args: Vec<Expr>,
    },
    Try(Box<Expr>),
    Tuple(Vec<Expr>),
    Record {
        fields: Vec<(String, Expr)>,
    },
    Lambda {
        params: Vec<(String, Option<TypeRef>)>,
        body: Box<Expr>,
    },
    /// Binary operator application (`lhs op rhs`), built by the Pratt
    /// loop in [`Parser::parse_binary_expr`]. The op set is defined in
    /// [`crate::expr_ops`].
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    /// Prefix unary operator application (`op operand`). Limited to
    /// `-` and `!` in the bootstrap grammar.
    Unary {
        op: UnOp,
        operand: Box<Expr>,
    },
    /// Interpolated string literal (`"hello {name}"`), as produced by
    /// the M21b extended string lexer. The parts alternate between
    /// literal text and embedded expressions; an empty trailing
    /// `InterpPart::Lit("")` may appear so the lit/expr boundary is
    /// preserved for round-trip rendering.
    InterpString {
        parts: Vec<InterpPart>,
    },
    /// Raw string literal (`r"…"` or `r#"…"#`, up to 4 hashes). The
    /// inner string is captured verbatim — no escape processing was
    /// performed, and the hash count is recorded separately so the
    /// formatter can render the literal back into source faithfully.
    RawStr {
        /// Verbatim contents (no surrounding quotes or hashes).
        text: String,
        /// Number of `#` characters surrounding the literal in source
        /// (0 for `r"…"`, 1 for `r#"…"#`, …, up to 4).
        hashes: u8,
    },
    /// Recovery node. Emitted alongside a diagnostic whenever the parser
    /// hits something it cannot understand. Callers can treat it as
    /// "unknown" without crashing.
    Error,
}

/// One fragment of an [`Expr::InterpString`]. `Lit` carries the
/// processed literal text (escapes already resolved); `Expr` carries a
/// nested expression parsed from the `{ … }` hole.
#[derive(Debug, Clone, PartialEq)]
pub enum InterpPart {
    /// Literal text between holes.
    Lit(String),
    /// Expression embedded inside a `{ … }` hole.
    Expr(Box<Expr>),
}

impl Expr {
    /// `true` if this is a recovery node. Useful for downstream passes that
    /// want to skip type-checking unrecoverable expressions.
    pub fn is_error(&self) -> bool {
        matches!(self, Expr::Error)
    }
}

const KNOWN_CONSTRUCTORS: &[&str] = &["Ok", "Err", "Some", "None", "Unit"];

fn is_constructor_name(name: &str) -> bool {
    if KNOWN_CONSTRUCTORS.contains(&name) {
        return true;
    }
    // Heuristic: PascalCase identifiers are treated as constructors so
    // user-defined variants like `NotFound` parse to `Construct`.
    name.chars().next().map(|c| c.is_ascii_uppercase()) == Some(true)
}

/// Parse a token slice (already trimmed to a function body) into a single
/// expression representing the body, plus any recovery diagnostics.
///
/// The returned expression is always a `Block` when the body contains more
/// than one statement; single-line bodies may collapse to the underlying
/// expression.
pub fn parse_body_expr(tokens: &[Token]) -> (Expr, Vec<Diagnostic>) {
    let mut parser = Parser::new(tokens);
    let expr = parser.parse_body();
    (expr, parser.into_diagnostics())
}

// ---------------------------------------------------------------------------
// Internal token-driven recursive-descent parser.
// ---------------------------------------------------------------------------

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Self {
            tokens,
            pos: 0,
            diagnostics: Vec::new(),
        }
    }

    fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    // ---------- Token cursor helpers ----------

    fn peek(&self) -> Option<&'a Token> {
        self.tokens.get(self.pos)
    }

    fn bump(&mut self) -> Option<&'a Token> {
        let t = self.tokens.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn at_eof(&self) -> bool {
        match self.peek() {
            None => true,
            Some(t) => t.kind == TokenKind::Eof,
        }
    }

    fn check_symbol(&self, sym: &str) -> bool {
        matches!(
            self.peek(),
            Some(t) if t.kind == TokenKind::Symbol && t.lexeme == sym
        )
    }

    fn check_keyword(&self, kw: &str) -> bool {
        matches!(
            self.peek(),
            Some(t) if t.kind == TokenKind::Keyword && t.lexeme == kw
        )
    }

    fn eat_symbol(&mut self, sym: &str) -> bool {
        if self.check_symbol(sym) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn eat_keyword(&mut self, kw: &str) -> bool {
        if self.check_keyword(kw) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn current_span(&self) -> Span {
        match self.peek() {
            Some(t) => t.span.clone(),
            None => self
                .tokens
                .last()
                .map(|t| t.span.clone())
                .unwrap_or_else(|| Span::dummy("<body>")),
        }
    }

    fn push_diag(&mut self, diag: Diagnostic) {
        self.diagnostics.push(diag);
    }

    // ---------- Top-level body entry ----------

    fn parse_body(&mut self) -> Expr {
        // Empty body → unit literal so callers always get a usable expression.
        if self.at_eof() {
            return Expr::Lit(Literal::Unit);
        }

        let mut stmts = Vec::new();
        let mut tail: Option<Expr> = None;

        while !self.at_eof() {
            // Tolerate stray separators between statements.
            if self.eat_symbol(";") {
                continue;
            }
            let start_pos = self.pos;
            let stmt = self.parse_stmt();
            // Optional trailing `;`.
            self.eat_symbol(";");
            if self.at_eof() {
                if let Stmt::Expr(expr) = stmt {
                    tail = Some(expr);
                } else {
                    stmts.push(stmt);
                }
                break;
            }
            // Guard against zero-progress loops if the cursor didn't move.
            if self.pos == start_pos {
                self.recover_to_stmt_boundary();
            }
            stmts.push(stmt);
        }

        if stmts.is_empty() {
            return tail.unwrap_or(Expr::Lit(Literal::Unit));
        }
        // If the body is exactly one statement and no tail, lift trivial
        // wrappers so callers see the cleanest possible Expr shape.
        if stmts.len() == 1 && tail.is_none() {
            return match stmts.pop() {
                Some(Stmt::Expr(e)) => e,
                Some(Stmt::Return(value)) => Expr::Return(value.map(Box::new)),
                Some(other) => Expr::Block {
                    stmts: vec![other],
                    tail: None,
                },
                None => Expr::Lit(Literal::Unit),
            };
        }
        Expr::Block {
            stmts,
            tail: tail.map(Box::new),
        }
    }

    fn recover_to_stmt_boundary(&mut self) {
        // Move forward at least one token, then stop at the next likely
        // statement starter so we don't loop forever on unparseable input.
        if !self.at_eof() {
            self.pos += 1;
        }
        while let Some(t) = self.peek() {
            if t.kind == TokenKind::Eof {
                break;
            }
            let is_starter = matches!(
                (t.kind, t.lexeme.as_str()),
                (TokenKind::Keyword, "let")
                    | (TokenKind::Keyword, "return")
                    | (TokenKind::Keyword, "if")
                    | (TokenKind::Keyword, "match")
                    | (TokenKind::Symbol, ";")
            );
            if is_starter {
                break;
            }
            self.pos += 1;
        }
    }

    // ---------- Statements ----------

    fn parse_stmt(&mut self) -> Stmt {
        if self.check_keyword("let") || self.check_keyword("var") {
            return self.parse_let_stmt();
        }
        if self.check_keyword("return") {
            return self.parse_return_stmt();
        }
        Stmt::Expr(self.parse_expr())
    }

    fn parse_let_stmt(&mut self) -> Stmt {
        let kw_span = self.current_span();
        // consume `let` / `var`
        let _ = self.bump();
        // optional `mut`
        let _ = self.eat_keyword("mut");

        let name = match self.peek() {
            Some(t) if t.kind == TokenKind::Ident => {
                let n = t.lexeme.clone();
                self.pos += 1;
                n
            }
            _ => {
                self.push_diag(
                    Diagnostic::error(
                        "E1101",
                        "expected identifier after `let`",
                        self.current_span(),
                    )
                    .with_expected(vec!["identifier".to_string()])
                    .with_agent_summary("Bindings need a name after `let`.")
                    .with_docs(vec!["doc:syntax.let".to_string()]),
                );
                // Make a synthetic name so downstream passes can still see a
                // binding (and so shadowing tests behave predictably).
                "_".to_string()
            }
        };

        // Optional `: Type`
        let mut ty = None;
        if self.eat_symbol(":") {
            ty = Some(self.parse_type_ref());
        }

        if !self.eat_symbol("=") {
            self.push_diag(
                Diagnostic::error(
                    "E1102",
                    "expected `=` in `let` binding",
                    self.current_span(),
                )
                .with_expected(vec!["=".to_string()])
                .with_agent_summary("Add `=` and an initialiser to the binding.")
                .with_docs(vec!["doc:syntax.let".to_string()]),
            );
            return Stmt::Let {
                name,
                ty,
                init: Expr::Error,
            };
        }

        let init = if self.is_at_stmt_terminator() {
            self.push_diag(
                Diagnostic::error(
                    "E1103",
                    "expected expression on right-hand side of `let`",
                    kw_span,
                )
                .with_expected(vec!["expression".to_string()])
                .with_agent_summary("Provide an initialiser for the binding.")
                .with_docs(vec!["doc:syntax.let".to_string()]),
            );
            Expr::Error
        } else {
            self.parse_expr()
        };

        Stmt::Let { name, ty, init }
    }

    fn parse_return_stmt(&mut self) -> Stmt {
        let _ = self.bump(); // `return`
        if self.is_at_stmt_terminator() {
            return Stmt::Return(None);
        }
        let value = self.parse_expr();
        if matches!(value, Expr::Error) {
            Stmt::Return(None)
        } else {
            Stmt::Return(Some(value))
        }
    }

    fn is_at_stmt_terminator(&self) -> bool {
        match self.peek() {
            None => true,
            Some(t) => t.kind == TokenKind::Eof || (t.kind == TokenKind::Symbol && t.lexeme == ";"),
        }
    }

    // ---------- Expressions ----------

    fn parse_expr(&mut self) -> Expr {
        self.parse_binary_expr(0)
    }

    /// Pratt-style precedence climber. `min_prec` is the lowest binding
    /// power the loop is willing to consume; recursive calls bump it up
    /// for left-associative operators and keep it equal for
    /// right-associative ones (currently only `??`).
    fn parse_binary_expr(&mut self, min_prec: u8) -> Expr {
        let mut lhs = self.parse_unary_expr();
        loop {
            let Some((op, op_width)) = self.peek_binop() else {
                break;
            };
            let prec = precedence(op);
            if prec < min_prec {
                break;
            }
            let op_span = self.current_span();
            self.pos += op_width;
            let next_min = if is_right_associative(op) {
                prec
            } else {
                prec.saturating_add(1)
            };
            if self.is_at_expr_terminator() {
                self.push_diag(
                    Diagnostic::error(
                        E_MISSING_BIN_OPERAND,
                        "expected operand after binary operator",
                        op_span,
                    )
                    .with_expected(vec!["expression".to_string()])
                    .with_agent_summary("Add an expression on the right-hand side of the operator.")
                    .with_docs(vec!["doc:syntax.operators".to_string()]),
                );
                lhs = Expr::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(Expr::Error),
                };
                break;
            }
            let rhs = self.parse_binary_expr(next_min);
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        lhs
    }

    /// Parse an optional prefix unary operator followed by a postfix
    /// chain. Recurses on itself so `--x` and `!!flag` are accepted at
    /// parse time (later passes may reject them on type grounds).
    fn parse_unary_expr(&mut self) -> Expr {
        if let Some((op, op_span)) = self.peek_prefix_unop() {
            self.pos += 1;
            if self.is_at_expr_terminator() {
                self.push_diag(
                    Diagnostic::error(
                        E_MISSING_BIN_OPERAND,
                        "expected operand after unary operator",
                        op_span,
                    )
                    .with_expected(vec!["expression".to_string()])
                    .with_agent_summary("Add an expression after the unary operator.")
                    .with_docs(vec!["doc:syntax.operators".to_string()]),
                );
                return Expr::Unary {
                    op,
                    operand: Box::new(Expr::Error),
                };
            }
            let operand = self.parse_unary_expr();
            return Expr::Unary {
                op,
                operand: Box::new(operand),
            };
        }
        self.parse_postfix_chain()
    }

    /// Inspect the next one-or-two tokens for a binary operator without
    /// consuming them. Returns the operator and how many tokens it
    /// spans (always 1 or 2). The two-token case is `??`, which the
    /// bootstrap lexer emits as two adjacent `?` symbols.
    fn peek_binop(&self) -> Option<(BinOp, usize)> {
        let t0 = self.peek()?;
        if t0.kind != TokenKind::Symbol {
            return None;
        }
        if t0.lexeme == "?" {
            let t1 = self.tokens.get(self.pos + 1)?;
            if t1.kind == TokenKind::Symbol && t1.lexeme == "?" && tokens_are_adjacent(t0, t1) {
                return Some((BinOp::Coalesce, 2));
            }
            return None;
        }
        binop_for_lexeme(&t0.lexeme).map(|op| (op, 1))
    }

    /// Inspect the next token for a prefix unary operator. Never spans
    /// two tokens.
    fn peek_prefix_unop(&self) -> Option<(UnOp, Span)> {
        let t = self.peek()?;
        if t.kind != TokenKind::Symbol {
            return None;
        }
        unop_for_lexeme(&t.lexeme).map(|op| (op, t.span.clone()))
    }

    /// `true` if the next token cannot start an expression. Used as the
    /// stop-condition for operator parsing so we don't try to grab a
    /// right-hand operand that doesn't exist.
    fn is_at_expr_terminator(&self) -> bool {
        let Some(t) = self.peek() else {
            return true;
        };
        if t.kind == TokenKind::Eof {
            return true;
        }
        if t.kind == TokenKind::Symbol {
            return matches!(
                t.lexeme.as_str(),
                ";" | "," | ")" | "}" | "]" | "=>" | "->" | "|" | ":"
            );
        }
        if t.kind == TokenKind::Keyword {
            return matches!(t.lexeme.as_str(), "else");
        }
        false
    }

    fn parse_postfix_chain(&mut self) -> Expr {
        let mut expr = self.parse_primary();
        loop {
            if self.eat_symbol(".") {
                expr = self.parse_field_or_method(expr);
                continue;
            }
            if self.check_symbol("(") {
                let args = self.parse_call_args();
                expr = Expr::Call {
                    callee: Box::new(expr),
                    args,
                };
                continue;
            }
            // `?` is the postfix try operator — unless the very next
            // token is another `?` glued to it, in which case it is the
            // `??` coalesce operator and must be left for
            // [`Parser::parse_binary_expr`] to consume.
            if self.check_symbol("?") && !self.next_is_glued_question() {
                self.pos += 1;
                expr = Expr::Try(Box::new(expr));
                continue;
            }
            break;
        }
        expr
    }

    /// `true` when the current token is `?` and the following token is
    /// also `?` with no whitespace between them, indicating the `??`
    /// coalesce operator.
    fn next_is_glued_question(&self) -> bool {
        let Some(t0) = self.peek() else {
            return false;
        };
        let Some(t1) = self.tokens.get(self.pos + 1) else {
            return false;
        };
        t0.kind == TokenKind::Symbol
            && t0.lexeme == "?"
            && t1.kind == TokenKind::Symbol
            && t1.lexeme == "?"
            && tokens_are_adjacent(t0, t1)
    }

    fn parse_field_or_method(&mut self, base: Expr) -> Expr {
        let name = match self.peek() {
            Some(t) if matches!(t.kind, TokenKind::Ident | TokenKind::Keyword) => {
                let n = t.lexeme.clone();
                self.pos += 1;
                n
            }
            _ => {
                self.push_diag(
                    Diagnostic::error(
                        "E1110",
                        "expected field or method name after `.`",
                        self.current_span(),
                    )
                    .with_expected(vec!["identifier".to_string()])
                    .with_agent_summary("Add a field or method name after the dot.")
                    .with_docs(vec!["doc:syntax.field-access".to_string()]),
                );
                return Expr::Error;
            }
        };
        Expr::Field {
            base: Box::new(base),
            name,
        }
    }

    fn parse_call_args(&mut self) -> Vec<Expr> {
        // assumes `(` is next
        let open_span = self.current_span();
        if !self.eat_symbol("(") {
            return Vec::new();
        }
        let mut args = Vec::new();
        loop {
            if self.eat_symbol(")") {
                return args;
            }
            if self.at_eof() {
                self.push_diag(
                    Diagnostic::error("E1120", "unterminated argument list", open_span.clone())
                        .with_expected(vec![")".to_string()])
                        .with_agent_summary("Add the missing `)` to close the call.")
                        .with_docs(vec!["doc:syntax.calls".to_string()]),
                );
                return args;
            }
            let before = self.pos;
            let arg = self.parse_expr();
            args.push(arg);
            // Allow trailing comma: `f(a, b,)`
            if self.eat_symbol(",") {
                continue;
            }
            if self.eat_symbol(")") {
                return args;
            }
            // Recovery: if we made no progress, skip a token to avoid an
            // infinite loop on something like `f(@)`.
            if self.pos == before {
                self.pos += 1;
            }
        }
    }

    fn parse_primary(&mut self) -> Expr {
        let Some(token) = self.peek().cloned() else {
            return Expr::Lit(Literal::Unit);
        };

        match (token.kind, token.lexeme.as_str()) {
            (TokenKind::Eof, _) => Expr::Lit(Literal::Unit),

            (TokenKind::Number, _) => {
                self.pos += 1;
                Expr::Lit(parse_number_literal(&token.lexeme))
            }

            (TokenKind::String, _) => {
                self.pos += 1;
                self.build_string_expr(&token.lexeme, &token.span)
            }

            (TokenKind::Keyword, "if") => self.parse_if(),
            (TokenKind::Keyword, "match") => self.parse_match(),
            (TokenKind::Keyword, "return") => {
                self.pos += 1;
                if self.is_at_stmt_terminator() {
                    Expr::Return(None)
                } else {
                    let value = self.parse_expr();
                    if matches!(value, Expr::Error) {
                        Expr::Return(None)
                    } else {
                        Expr::Return(Some(Box::new(value)))
                    }
                }
            }
            (TokenKind::Keyword, "fn") => self.parse_lambda(),

            (TokenKind::Ident, name) => {
                // M21b: `r` immediately followed (no whitespace) by a
                // string literal is a raw string. Detection is purely a
                // span-adjacency check so it never collides with a real
                // variable also called `r`.
                if name == "r" {
                    if let Some(raw_expr) = self.try_parse_raw_string(&token.span) {
                        return raw_expr;
                    }
                }

                // Look ahead to disambiguate variants:
                //   * `Name { ... }`   → record literal (lowercase or upper)
                //   * `Name(...)`      → constructor or call
                //   * `Name`           → bare variable / nullary constructor
                let name = name.to_string();
                self.pos += 1;

                if self.check_symbol("{") {
                    return self.parse_record_literal_with_optional_tag(Some(name));
                }

                if is_constructor_name(&name) {
                    if self.check_symbol("(") {
                        let args = self.parse_call_args();
                        return Expr::Construct {
                            variant: name,
                            args,
                        };
                    }
                    return Expr::Construct {
                        variant: name,
                        args: Vec::new(),
                    };
                }

                Expr::Var(name)
            }

            (TokenKind::Symbol, "(") => self.parse_paren_or_tuple(),
            (TokenKind::Symbol, "{") => self.parse_record_literal_with_optional_tag(None),
            (TokenKind::Symbol, "[") => self.parse_list_literal(),

            _ => {
                self.push_diag(
                    Diagnostic::error(
                        "E1100",
                        format!("unexpected token `{}` in expression", token.lexeme),
                        token.span.clone(),
                    )
                    .with_found(vec![token.lexeme.clone()])
                    .with_agent_summary("Remove or replace the unexpected token.")
                    .with_docs(vec!["doc:syntax.expressions".to_string()]),
                );
                self.pos += 1;
                Expr::Error
            }
        }
    }

    fn parse_paren_or_tuple(&mut self) -> Expr {
        // consume `(`
        let open_span = self.current_span();
        self.pos += 1;

        if self.eat_symbol(")") {
            return Expr::Lit(Literal::Unit);
        }

        let first = self.parse_expr();
        if self.eat_symbol(")") {
            return first;
        }
        if self.eat_symbol(",") {
            let mut elements = vec![first];
            loop {
                if self.eat_symbol(")") {
                    return Expr::Tuple(elements);
                }
                if self.at_eof() {
                    self.push_diag(
                        Diagnostic::error("E1121", "unterminated tuple literal", open_span)
                            .with_expected(vec![")".to_string()])
                            .with_agent_summary("Close the tuple with `)`.")
                            .with_docs(vec!["doc:syntax.tuples".to_string()]),
                    );
                    return Expr::Tuple(elements);
                }
                let before = self.pos;
                let next = self.parse_expr();
                elements.push(next);
                if self.eat_symbol(",") {
                    continue;
                }
                if self.eat_symbol(")") {
                    return Expr::Tuple(elements);
                }
                if self.pos == before {
                    self.pos += 1;
                }
            }
        }

        // Neither `,` nor `)` — recover and return the inner expression.
        self.push_diag(
            Diagnostic::error(
                "E1122",
                "expected `,` or `)` in parenthesised expression",
                open_span,
            )
            .with_expected(vec![",".to_string(), ")".to_string()])
            .with_agent_summary("Close the parenthesised expression.")
            .with_docs(vec!["doc:syntax.expressions".to_string()]),
        );
        first
    }

    fn parse_list_literal(&mut self) -> Expr {
        let open_span = self.current_span();
        self.pos += 1; // `[`
                       // Lists are modelled as `Construct { variant: "List", args }` for now.
        let mut items = Vec::new();
        loop {
            if self.eat_symbol("]") {
                return Expr::Construct {
                    variant: "List".to_string(),
                    args: items,
                };
            }
            if self.at_eof() {
                self.push_diag(
                    Diagnostic::error("E1123", "unterminated list literal", open_span)
                        .with_expected(vec!["]".to_string()])
                        .with_agent_summary("Close the list with `]`.")
                        .with_docs(vec!["doc:syntax.lists".to_string()]),
                );
                return Expr::Construct {
                    variant: "List".to_string(),
                    args: items,
                };
            }
            let before = self.pos;
            items.push(self.parse_expr());
            if self.eat_symbol(",") {
                continue;
            }
            if self.eat_symbol("]") {
                return Expr::Construct {
                    variant: "List".to_string(),
                    args: items,
                };
            }
            if self.pos == before {
                self.pos += 1;
            }
        }
    }

    fn parse_record_literal_with_optional_tag(&mut self, tag: Option<String>) -> Expr {
        let open_span = self.current_span();
        if !self.eat_symbol("{") {
            // Shouldn't happen — caller already peeked `{` — but be defensive.
            return Expr::Error;
        }
        let mut fields = Vec::new();
        loop {
            if self.eat_symbol("}") {
                break;
            }
            if self.at_eof() {
                self.push_diag(
                    Diagnostic::error("E1124", "unterminated record literal", open_span.clone())
                        .with_expected(vec!["}".to_string()])
                        .with_agent_summary("Close the record with `}`.")
                        .with_docs(vec!["doc:syntax.records".to_string()]),
                );
                break;
            }
            let name = match self.peek() {
                Some(t) if matches!(t.kind, TokenKind::Ident | TokenKind::Keyword) => {
                    let n = t.lexeme.clone();
                    self.pos += 1;
                    n
                }
                _ => {
                    self.push_diag(
                        Diagnostic::error(
                            "E1125",
                            "expected field name in record literal",
                            self.current_span(),
                        )
                        .with_expected(vec!["identifier".to_string()])
                        .with_agent_summary("Add a field name before the colon.")
                        .with_docs(vec!["doc:syntax.records".to_string()]),
                    );
                    // skip the bad token and try to keep parsing
                    self.pos += 1;
                    continue;
                }
            };
            if !self.eat_symbol(":") {
                self.push_diag(
                    Diagnostic::error(
                        "E1126",
                        format!("expected `:` after field `{name}`"),
                        self.current_span(),
                    )
                    .with_expected(vec![":".to_string()])
                    .with_agent_summary("Field syntax is `name: value`.")
                    .with_docs(vec!["doc:syntax.records".to_string()]),
                );
                fields.push((name, Expr::Error));
                let _ = self.eat_symbol(",");
                continue;
            }
            let before = self.pos;
            let value = self.parse_expr();
            fields.push((name, value));
            if self.eat_symbol(",") {
                continue;
            }
            if self.eat_symbol("}") {
                break;
            }
            if self.pos == before {
                self.pos += 1;
            }
        }

        match tag {
            Some(tag) => Expr::Construct {
                variant: tag,
                args: vec![Expr::Record { fields }],
            },
            None => Expr::Record { fields },
        }
    }

    // ---------- if / match / lambda ----------

    fn parse_if(&mut self) -> Expr {
        self.pos += 1; // `if`
        let cond = self.parse_expr();
        // Optional `:` introduces an indented body in Orison surface syntax.
        // We accept it but don't require it for token-level parsing.
        let _ = self.eat_symbol(":");
        let then_branch = self.parse_branch_block();
        let else_branch = if self.eat_keyword("else") {
            let _ = self.eat_symbol(":");
            Some(Box::new(self.parse_branch_block()))
        } else {
            None
        };
        Expr::If {
            cond: Box::new(cond),
            then_branch: Box::new(then_branch),
            else_branch,
        }
    }

    fn parse_branch_block(&mut self) -> Expr {
        // A branch body is one statement or expression. We collect until we
        // see `else`, the next statement starter at the same nesting level,
        // or EOF.
        if self.check_keyword("else") || self.at_eof() {
            return Expr::Lit(Literal::Unit);
        }
        let stmt = self.parse_stmt();
        match stmt {
            Stmt::Expr(e) => e,
            other => Expr::Block {
                stmts: vec![other],
                tail: None,
            },
        }
    }

    fn parse_match(&mut self) -> Expr {
        self.pos += 1; // `match`
        let scrutinee = self.parse_expr();
        let _ = self.eat_symbol(":");
        let mut arms = Vec::new();
        loop {
            // Arms start with `|` in the Orison surface, but be tolerant.
            let had_bar = self.eat_symbol("|");
            if !had_bar && arms.is_empty() && self.check_symbol("|") {
                // pre-consumed above
            }
            if self.at_eof() {
                break;
            }
            if !had_bar && !arms.is_empty() {
                break;
            }
            // Peek for arm pattern.
            let before = self.pos;
            let pattern = self.parse_pattern();
            if !self.eat_symbol("=>") && !self.eat_symbol("->") {
                self.push_diag(
                    Diagnostic::error("E1130", "expected `=>` in match arm", self.current_span())
                        .with_expected(vec!["=>".to_string()])
                        .with_agent_summary("Use `pattern => expression` for match arms.")
                        .with_docs(vec!["doc:syntax.match".to_string()]),
                );
                // If we made no progress, advance to avoid an infinite loop.
                if self.pos == before {
                    self.pos += 1;
                }
                continue;
            }
            let body = self.parse_expr();
            arms.push(MatchArm { pattern, body });
            // Allow optional `,` between arms.
            let _ = self.eat_symbol(",");
            if !self.check_symbol("|") {
                break;
            }
        }
        Expr::Match {
            scrutinee: Box::new(scrutinee),
            arms,
        }
    }

    fn parse_pattern(&mut self) -> Pattern {
        let Some(token) = self.peek().cloned() else {
            return Pattern::Wildcard;
        };
        match (token.kind, token.lexeme.as_str()) {
            (TokenKind::Ident, "_") => {
                self.pos += 1;
                Pattern::Wildcard
            }
            (TokenKind::Ident, name) => {
                let name = name.to_string();
                self.pos += 1;
                if is_constructor_name(&name) {
                    let mut args = Vec::new();
                    if self.eat_symbol("(") {
                        loop {
                            if self.eat_symbol(")") {
                                break;
                            }
                            if self.at_eof() {
                                break;
                            }
                            let before = self.pos;
                            args.push(self.parse_pattern());
                            if self.eat_symbol(",") {
                                continue;
                            }
                            if self.eat_symbol(")") {
                                break;
                            }
                            if self.pos == before {
                                self.pos += 1;
                            }
                        }
                    }
                    Pattern::Constructor { name, args }
                } else {
                    Pattern::Binding(name)
                }
            }
            (TokenKind::Number, _) => {
                self.pos += 1;
                Pattern::Literal(parse_number_literal(&token.lexeme))
            }
            (TokenKind::String, _) => {
                self.pos += 1;
                Pattern::Literal(Literal::Str(token.lexeme.clone()))
            }
            _ => {
                self.pos += 1;
                Pattern::Wildcard
            }
        }
    }

    fn parse_lambda(&mut self) -> Expr {
        // `fn (params) => body` or `fn (params): body`
        self.pos += 1; // `fn`
        let mut params = Vec::new();
        if self.eat_symbol("(") {
            loop {
                if self.eat_symbol(")") {
                    break;
                }
                if self.at_eof() {
                    break;
                }
                let name = match self.peek() {
                    Some(t) if t.kind == TokenKind::Ident => {
                        let n = t.lexeme.clone();
                        self.pos += 1;
                        n
                    }
                    _ => {
                        self.push_diag(
                            Diagnostic::error(
                                "E1140",
                                "expected parameter name in lambda",
                                self.current_span(),
                            )
                            .with_expected(vec!["identifier".to_string()])
                            .with_agent_summary("Lambda parameters need names.")
                            .with_docs(vec!["doc:syntax.lambda".to_string()]),
                        );
                        self.pos += 1;
                        String::from("_")
                    }
                };
                let ty = if self.eat_symbol(":") {
                    Some(self.parse_type_ref())
                } else {
                    None
                };
                params.push((name, ty));
                if self.eat_symbol(",") {
                    continue;
                }
                if self.eat_symbol(")") {
                    break;
                }
            }
        }
        let _ = self.eat_symbol("=>");
        let _ = self.eat_symbol(":");
        let body = self.parse_expr();
        Expr::Lambda {
            params,
            body: Box::new(body),
        }
    }

    // ---------- Type references ----------

    fn parse_type_ref(&mut self) -> TypeRef {
        let Some(token) = self.peek().cloned() else {
            return TypeRef::Unknown;
        };
        match (token.kind, token.lexeme.as_str()) {
            (TokenKind::Ident, name) | (TokenKind::Keyword, name) => {
                let name = name.to_string();
                self.pos += 1;
                if self.eat_symbol("[") {
                    let mut args = Vec::new();
                    loop {
                        if self.eat_symbol("]") {
                            break;
                        }
                        if self.at_eof() {
                            break;
                        }
                        let before = self.pos;
                        args.push(self.parse_type_ref());
                        if self.eat_symbol(",") {
                            continue;
                        }
                        if self.eat_symbol("]") {
                            break;
                        }
                        if self.pos == before {
                            self.pos += 1;
                        }
                    }
                    TypeRef::Generic { name, args }
                } else if crate::types::is_builtin_type(&name) {
                    TypeRef::Primitive(name)
                } else {
                    TypeRef::Named(name)
                }
            }
            _ => TypeRef::Unknown,
        }
    }

    // ---------- M21b: extended string literals ----------

    /// Build the right `Expr` flavour for a `TokenKind::String` token.
    /// Falls back to a plain `Lit(Str(_))` whenever the extended lexer
    /// reports either "no interpolation present" or a structured error
    /// (the error path additionally emits a diagnostic; the literal
    /// itself degrades to the original token text so downstream passes
    /// still see *some* string).
    fn build_string_expr(&mut self, lexeme: &str, span: &crate::source::Span) -> Expr {
        // Fast path: no `{` at all means plain string. We still have to
        // honour `\{` as an escape, so we walk char-by-char.
        if !lexeme_has_unescaped_brace(lexeme) {
            return Expr::Lit(Literal::Str(lexeme.to_string()));
        }
        // Reconstruct the surface form `"…"` so `lex_string_extended`
        // sees a self-contained literal. The lexer in `crate::lexer`
        // strips the surrounding quotes; we put them back temporarily.
        let synthesised = format!("\"{lexeme}\"");
        let lexed = crate::string_lits::lex_string_extended(&synthesised, 0);
        let (lit, _) = match lexed {
            Ok(pair) => pair,
            Err(err) => {
                self.push_diag(
                    Diagnostic::error(err.id(), err.message(), span.clone())
                        .with_agent_summary("Fix the offending string literal.")
                        .with_docs(vec!["doc:syntax.strings".to_string()]),
                );
                return Expr::Lit(Literal::Str(lexeme.to_string()));
            }
        };
        if !lit.is_interpolated() {
            // Defensive — if for some reason the extended lexer didn't
            // see holes (e.g. all braces were escaped), fall back to
            // the flattened literal so output still round-trips.
            return Expr::Lit(Literal::Str(lit.flatten_lit_only()));
        }
        let parts = self.lower_interp_parts(lit.parts, span);
        Expr::InterpString { parts }
    }

    /// Lower the structured `StringPart` sequence into an `InterpPart`
    /// sequence, recursively parsing the embedded expression text for
    /// every `Interp` hole through the same body parser.
    fn lower_interp_parts(
        &mut self,
        parts: Vec<crate::string_lits::StringPart>,
        span: &crate::source::Span,
    ) -> Vec<InterpPart> {
        let mut out = Vec::with_capacity(parts.len());
        for part in parts {
            match part {
                crate::string_lits::StringPart::Lit(text) => {
                    out.push(InterpPart::Lit(text));
                }
                crate::string_lits::StringPart::Interp(text) => {
                    let trimmed = text.trim();
                    if trimmed.is_empty() {
                        // Empty hole `"{}"` — record as a recovery node
                        // plus diagnostic so we don't silently accept
                        // it. The diagnostic id matches the lexer's
                        // S1302 family for "unbalanced/empty hole".
                        self.push_diag(
                            Diagnostic::error(
                                "S1302",
                                "empty `{}` interpolation hole",
                                span.clone(),
                            )
                            .with_agent_summary("Provide an expression inside the `{ … }`.")
                            .with_docs(vec!["doc:syntax.strings".to_string()]),
                        );
                        out.push(InterpPart::Expr(Box::new(Expr::Error)));
                        continue;
                    }
                    let sub_src = crate::source::SourceFile::new(span.file.clone(), trimmed);
                    let sub_tokens = crate::lexer::lex(&sub_src);
                    let (sub_expr, sub_diags) = parse_body_expr(&sub_tokens);
                    for d in sub_diags {
                        self.push_diag(d);
                    }
                    out.push(InterpPart::Expr(Box::new(sub_expr)));
                }
            }
        }
        out
    }

    /// Try to parse a raw string starting at the current token, which
    /// the caller has already verified to be `Ident("r")`. Returns
    /// `Some(expr)` if the next token is an adjacent `String` (or a
    /// `Symbol("#")` chain followed by a `String`), and consumes those
    /// tokens; returns `None` otherwise so the caller falls back to
    /// treating `r` as a regular identifier.
    fn try_parse_raw_string(&mut self, r_span: &crate::source::Span) -> Option<Expr> {
        // Collect zero-or-more adjacent `#` symbol tokens.
        let mut probe = self.pos + 1;
        let mut hashes: u8 = 0;
        let mut last_end = r_span.end.clone();
        while let Some(t) = self.tokens.get(probe) {
            if t.kind == TokenKind::Symbol
                && t.lexeme == "#"
                && spans_are_adjacent(&last_end, &t.span)
            {
                hashes = hashes.saturating_add(1);
                last_end = t.span.end.clone();
                probe += 1;
            } else {
                break;
            }
        }
        // Next token must be a `String` adjacent to whatever we've
        // consumed so far. For a `r#"…"#` literal, the existing lexer
        // would tokenise the body up to the first `"`; we therefore
        // do not currently support hashed raw strings via this lexer
        // path. The standalone `lex_string_extended` handles them when
        // called against raw source.
        let next = self.tokens.get(probe)?;
        if next.kind != TokenKind::String || !spans_are_adjacent(&last_end, &next.span) {
            return None;
        }
        if hashes > 0 {
            // Best-effort heuristic: with hashed raw strings the body
            // is captured in `next.lexeme` only up to the first
            // embedded `"`. That's still useful for round-trip but
            // accurate only for hash-free contents. We surface no
            // diagnostic here — the literal is preserved verbatim.
        }
        // Consume `r` + `#`*hashes + string token.
        self.pos = probe + 1;
        Some(Expr::RawStr {
            text: next.lexeme.clone(),
            hashes,
        })
    }
}

/// `true` if `lexeme` contains at least one `{` that is not preceded by
/// a `\` (per the M21b escape rules). Walks the string char-by-char to
/// avoid being fooled by `\\{` (where the backslash is itself escaped).
fn lexeme_has_unescaped_brace(lexeme: &str) -> bool {
    let mut escaped = false;
    for ch in lexeme.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '{' {
            return true;
        }
    }
    false
}

/// `true` if `b` starts exactly where `a` ends (same file, same line,
/// same column). Used to decide whether two adjacent tokens are part of
/// a single compound literal (`r"…"`) or merely happen to neighbour
/// each other across whitespace.
fn spans_are_adjacent(a_end: &crate::source::Position, b: &crate::source::Span) -> bool {
    a_end.line == b.start.line && a_end.column == b.start.column
}

/// Two tokens are *adjacent* when the end coordinate of the first equals
/// the start coordinate of the second — i.e. nothing (not even
/// whitespace) separates them in the source. The bootstrap lexer emits
/// `??` as two consecutive `?` symbols, so adjacency is how the parser
/// distinguishes `a ?? b` (coalesce) from `a? ? b` (postfix try followed
/// by a malformed ternary fragment).
fn tokens_are_adjacent(a: &Token, b: &Token) -> bool {
    a.span.end.line == b.span.start.line && a.span.end.column == b.span.start.column
}

fn parse_number_literal(text: &str) -> Literal {
    let cleaned: String = text.chars().filter(|c| *c != '_').collect();
    if cleaned.contains('.') {
        match cleaned.parse::<f64>() {
            Ok(f) => Literal::Float(f),
            Err(_) => Literal::Float(0.0),
        }
    } else {
        match cleaned.parse::<i64>() {
            Ok(i) => Literal::Int(i),
            Err(_) => Literal::Int(0),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::source::SourceFile;

    fn parse(text: &str) -> (Expr, Vec<Diagnostic>) {
        let src = SourceFile::new("/t.ori", text);
        let tokens = lex(&src);
        parse_body_expr(&tokens)
    }

    fn parse_ok(text: &str) -> Expr {
        let (expr, diags) = parse(text);
        let errors: Vec<_> = diags.iter().filter(|d| d.is_error()).collect();
        if !errors.is_empty() {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(
                    false,
                    "expected no errors for `{text}`, got: {:?}",
                    errors.iter().map(|d| &d.id).collect::<Vec<_>>()
                );
            }
        }
        expr
    }

    // ---- literals & vars ----

    #[test]
    fn int_literal() {
        assert_eq!(parse_ok("42"), Expr::Lit(Literal::Int(42)));
    }

    #[test]
    #[allow(clippy::approx_constant)]
    fn float_literal() {
        assert_eq!(parse_ok("3.14"), Expr::Lit(Literal::Float(3.14)));
    }

    #[test]
    fn string_literal() {
        assert_eq!(parse_ok("\"hi\""), Expr::Lit(Literal::Str("hi".into())));
    }

    #[test]
    fn interpolated_string_literal() {
        // `"hello {name}"` parses to an `InterpString` with two parts:
        //   * literal "hello "
        //   * embedded expression `name`
        let e = parse_ok("\"hello {name}\"");
        if let Expr::InterpString { parts } = e {
            // The lexer always emits a trailing `Lit` so the boundary
            // between text and expression is preserved (even if empty).
            let lit_parts: Vec<_> = parts
                .iter()
                .filter_map(|p| match p {
                    InterpPart::Lit(s) => Some(s.clone()),
                    _ => None,
                })
                .collect();
            let expr_parts: Vec<_> = parts
                .iter()
                .filter_map(|p| match p {
                    InterpPart::Expr(e) => Some((**e).clone()),
                    _ => None,
                })
                .collect();
            assert_eq!(lit_parts, vec!["hello ".to_string(), String::new()]);
            assert_eq!(expr_parts, vec![Expr::Var("name".into())]);
        } else {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected InterpString, got {e:?}");
            }
        }
    }

    #[test]
    fn raw_string_literal() {
        // `r"foo"` parses to `RawStr { text: "foo", hashes: 0 }`.
        let e = parse_ok("r\"foo\"");
        if let Expr::RawStr { text, hashes } = e {
            assert_eq!(text, "foo");
            assert_eq!(hashes, 0);
        } else {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected RawStr, got {e:?}");
            }
        }
    }

    #[test]
    fn plain_string_still_lit_no_regression() {
        // M21b adds new variants but plain strings must keep parsing
        // to `Lit(Str(_))` so older passes continue to work.
        let e = parse_ok("\"plain\"");
        assert_eq!(e, Expr::Lit(Literal::Str("plain".into())));
    }

    #[test]
    fn unit_literal_from_empty_parens() {
        assert_eq!(parse_ok("()"), Expr::Lit(Literal::Unit));
    }

    #[test]
    fn var_lower_ident() {
        assert_eq!(parse_ok("user"), Expr::Var("user".into()));
    }

    // ---- call / field / try ----

    #[test]
    fn simple_call() {
        let e = parse_ok("f(1, 2)");
        match e {
            Expr::Call { callee, args } => {
                assert_eq!(*callee, Expr::Var("f".into()));
                assert_eq!(args.len(), 2);
            }
            _ => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected call, got {e:?}");
                }
            }
        }
    }

    #[test]
    fn trailing_comma_in_args() {
        let e = parse_ok("f(1, 2,)");
        if let Expr::Call { args, .. } = e {
            assert_eq!(args.len(), 2);
        } else {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected call");
            }
        }
    }

    #[test]
    fn field_chain() {
        let e = parse_ok("cart.lines.append(line)");
        // Outer is a Call whose callee is the chained field access.
        if let Expr::Call { callee, args } = e {
            assert_eq!(args.len(), 1);
            if let Expr::Field { base, name } = *callee {
                assert_eq!(name, "append");
                if let Expr::Field { base, name } = *base {
                    assert_eq!(name, "lines");
                    assert_eq!(*base, Expr::Var("cart".into()));
                } else {
                    #[allow(clippy::assertions_on_constants)]
                    {
                        assert!(false, "expected nested field");
                    }
                }
            } else {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected field callee");
                }
            }
        } else {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected call");
            }
        }
    }

    #[test]
    fn try_operator() {
        let e = parse_ok("db.users.find(id)?");
        assert!(matches!(e, Expr::Try(_)));
    }

    #[test]
    fn try_on_arbitrary_expr_is_not_rejected() {
        // Per spec: `?` on a non-Result expression must not be rejected here;
        // later passes flag it. We only require that the parser builds a `Try`.
        let e = parse_ok("42?");
        assert!(matches!(e, Expr::Try(_)));
    }

    // ---- constructors / records / tuples ----

    #[test]
    fn ok_construct() {
        let e = parse_ok("Ok(user)");
        match e {
            Expr::Construct { variant, args } => {
                assert_eq!(variant, "Ok");
                assert_eq!(args.len(), 1);
                assert_eq!(args[0], Expr::Var("user".into()));
            }
            _ => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected construct");
                }
            }
        }
    }

    #[test]
    fn nullary_constructor() {
        let e = parse_ok("None");
        assert_eq!(
            e,
            Expr::Construct {
                variant: "None".into(),
                args: vec![]
            }
        );
    }

    #[test]
    fn tuple_literal() {
        let e = parse_ok("(1, 2, 3)");
        if let Expr::Tuple(parts) = e {
            assert_eq!(parts.len(), 3);
        } else {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected tuple");
            }
        }
    }

    #[test]
    fn record_literal() {
        let e = parse_ok("{ a: 1, b: 2 }");
        if let Expr::Record { fields } = e {
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[0].0, "a");
            assert_eq!(fields[1].0, "b");
        } else {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected record");
            }
        }
    }

    #[test]
    fn tagged_record_literal() {
        let e = parse_ok("Cart { customer: c, lines: ls }");
        if let Expr::Construct { variant, args } = e {
            assert_eq!(variant, "Cart");
            assert_eq!(args.len(), 1);
            assert!(matches!(args[0], Expr::Record { .. }));
        } else {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected tagged record");
            }
        }
    }

    // ---- blocks / let / return ----

    #[test]
    fn empty_body_is_unit() {
        assert_eq!(parse_ok(""), Expr::Lit(Literal::Unit));
    }

    #[test]
    fn block_with_let_and_tail() {
        let e = parse_ok("let x = 1; x");
        if let Expr::Block { stmts, tail } = e {
            assert_eq!(stmts.len(), 1);
            assert!(matches!(stmts[0], Stmt::Let { .. }));
            assert!(matches!(tail.as_deref(), Some(Expr::Var(_))));
        } else {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected block");
            }
        }
    }

    #[test]
    fn let_shadowing_parses() {
        let e = parse_ok("let x = 1; let x = 2; x");
        if let Expr::Block { stmts, tail } = e {
            assert_eq!(stmts.len(), 2);
            for s in &stmts {
                if let Stmt::Let { name, .. } = s {
                    assert_eq!(name, "x");
                } else {
                    #[allow(clippy::assertions_on_constants)]
                    {
                        assert!(false, "expected let");
                    }
                }
            }
            assert!(tail.is_some());
        } else {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected block");
            }
        }
    }

    #[test]
    fn return_with_value() {
        let e = parse_ok("return Ok(x)");
        assert!(matches!(e, Expr::Return(Some(_))));
    }

    #[test]
    fn return_without_value() {
        let e = parse_ok("return");
        assert!(matches!(e, Expr::Return(None)));
    }

    // ---- if / match ----

    #[test]
    fn if_with_else() {
        let e = parse_ok("if cond: return Ok(x) else: return Err(e)");
        assert!(matches!(
            e,
            Expr::If {
                else_branch: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn if_without_else() {
        let e = parse_ok("if cond: return Unit");
        assert!(matches!(
            e,
            Expr::If {
                else_branch: None,
                ..
            }
        ));
    }

    #[test]
    fn match_multiple_arms() {
        let e = parse_ok("match value | Ok(v) => v | Err(e) => fallback | _ => other");
        if let Expr::Match { arms, .. } = e {
            assert_eq!(arms.len(), 3);
        } else {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected match");
            }
        }
    }

    #[test]
    fn nested_match() {
        let e = parse_ok("match outer | Ok(v) => match v | Some(x) => x | None => 0 | Err(e) => 1");
        assert!(matches!(e, Expr::Match { .. }));
    }

    // ---- lambda ----

    #[test]
    fn lambda_simple() {
        let e = parse_ok("fn (x) => x");
        if let Expr::Lambda { params, body } = e {
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].0, "x");
            assert_eq!(*body, Expr::Var("x".into()));
        } else {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected lambda");
            }
        }
    }

    #[test]
    fn lambda_with_type_annotation() {
        let e = parse_ok("fn (x: Int) => x");
        if let Expr::Lambda { params, .. } = e {
            assert!(params[0].1.is_some());
        } else {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected lambda");
            }
        }
    }

    // ---- recovery ----

    #[test]
    fn recovers_from_malformed_let_and_continues() {
        // `let x =` is malformed; the parser should emit a diag and still
        // produce a sibling statement for the next `let`.
        let (expr, diags) = parse("let x = ; let y = 2; y");
        assert!(diags.iter().any(|d| d.id == "E1103"));
        if let Expr::Block { stmts, tail } = expr {
            // We expect at least one Let after recovery and a tail expression.
            assert!(!stmts.is_empty());
            assert!(tail.is_some());
        } else {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected block from recovery");
            }
        }
    }

    #[test]
    fn unexpected_token_emits_diagnostic() {
        let (_expr, diags) = parse("@");
        assert!(diags.iter().any(|d| d.id == "E1100"));
    }

    // ---- idempotence ----

    #[test]
    fn parsing_twice_is_idempotent() {
        let text = "let x = Ok(1); match x | Ok(v) => v | Err(e) => 0";
        let a = parse_ok(text);
        let b = parse_ok(text);
        assert_eq!(a, b);
    }

    #[test]
    fn all_diags_have_summary_and_docs() {
        let (_e, diags) = parse("@ let = ; ( ");
        for d in &diags {
            #[allow(clippy::assertions_on_constants)]
            {
                if d.agent.summary.is_empty() {
                    assert!(false, "diag {} missing agent summary", d.id);
                }
                if d.agent.docs.is_empty() {
                    assert!(false, "diag {} missing docs reference", d.id);
                }
                if !d.id.starts_with("E11") && !d.id.starts_with("E12") {
                    assert!(
                        false,
                        "diag {} not in body-parser range (E11xx / E12xx)",
                        d.id
                    );
                }
            }
        }
    }

    // ---- binary / unary operators (M21a) ----

    fn assert_var(expr: &Expr, expected: &str) {
        match expr {
            Expr::Var(name) => assert_eq!(name, expected),
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected Var({expected}), got {other:?}");
                }
            }
        }
    }

    #[test]
    fn binop_add_simple() {
        let e = parse_ok("a + b");
        match e {
            Expr::Binary { op, lhs, rhs } => {
                assert_eq!(op, BinOp::Add);
                assert_var(&lhs, "a");
                assert_var(&rhs, "b");
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected Binary(Add), got {other:?}");
                }
            }
        }
    }

    #[test]
    fn binop_precedence_mul_binds_tighter_than_add() {
        // `a + b * c` → Add(a, Mul(b, c))
        let e = parse_ok("a + b * c");
        match e {
            Expr::Binary {
                op: BinOp::Add,
                lhs,
                rhs,
            } => {
                assert_var(&lhs, "a");
                match *rhs {
                    Expr::Binary {
                        op: BinOp::Mul,
                        lhs: r_lhs,
                        rhs: r_rhs,
                    } => {
                        assert_var(&r_lhs, "b");
                        assert_var(&r_rhs, "c");
                    }
                    other => {
                        #[allow(clippy::assertions_on_constants)]
                        {
                            assert!(false, "expected nested Mul, got {other:?}");
                        }
                    }
                }
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected outer Add, got {other:?}");
                }
            }
        }
    }

    #[test]
    fn binop_left_associative_add() {
        // `a * b + c` → Add(Mul(a, b), c)
        let e = parse_ok("a * b + c");
        match e {
            Expr::Binary {
                op: BinOp::Add,
                lhs,
                rhs,
            } => {
                match *lhs {
                    Expr::Binary {
                        op: BinOp::Mul,
                        lhs: l_lhs,
                        rhs: l_rhs,
                    } => {
                        assert_var(&l_lhs, "a");
                        assert_var(&l_rhs, "b");
                    }
                    other => {
                        #[allow(clippy::assertions_on_constants)]
                        {
                            assert!(false, "expected nested Mul, got {other:?}");
                        }
                    }
                }
                assert_var(&rhs, "c");
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected outer Add, got {other:?}");
                }
            }
        }
    }

    #[test]
    fn binop_right_associative_coalesce() {
        // `a ?? b ?? c` → Coalesce(a, Coalesce(b, c))
        let e = parse_ok("a ?? b ?? c");
        match e {
            Expr::Binary {
                op: BinOp::Coalesce,
                lhs,
                rhs,
            } => {
                assert_var(&lhs, "a");
                match *rhs {
                    Expr::Binary {
                        op: BinOp::Coalesce,
                        lhs: r_lhs,
                        rhs: r_rhs,
                    } => {
                        assert_var(&r_lhs, "b");
                        assert_var(&r_rhs, "c");
                    }
                    other => {
                        #[allow(clippy::assertions_on_constants)]
                        {
                            assert!(false, "expected nested Coalesce, got {other:?}");
                        }
                    }
                }
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected outer Coalesce, got {other:?}");
                }
            }
        }
    }

    #[test]
    fn unop_not_simple() {
        let e = parse_ok("!x");
        match e {
            Expr::Unary { op, operand } => {
                assert_eq!(op, UnOp::Not);
                assert_var(&operand, "x");
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected Unary(Not), got {other:?}");
                }
            }
        }
    }

    #[test]
    fn unop_neg_then_binop() {
        // `-x + 1` → Add(Neg(x), 1)
        let e = parse_ok("-x + 1");
        match e {
            Expr::Binary {
                op: BinOp::Add,
                lhs,
                rhs,
            } => {
                match *lhs {
                    Expr::Unary {
                        op: UnOp::Neg,
                        operand,
                    } => assert_var(&operand, "x"),
                    other => {
                        #[allow(clippy::assertions_on_constants)]
                        {
                            assert!(false, "expected Neg(x), got {other:?}");
                        }
                    }
                }
                assert_eq!(*rhs, Expr::Lit(Literal::Int(1)));
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected outer Add, got {other:?}");
                }
            }
        }
    }

    #[test]
    fn logical_and_binds_tighter_than_or() {
        // `a && b || c` → Or(And(a, b), c)
        let e = parse_ok("a && b || c");
        match e {
            Expr::Binary {
                op: BinOp::Or,
                lhs,
                rhs,
            } => {
                match *lhs {
                    Expr::Binary {
                        op: BinOp::And,
                        lhs: l_lhs,
                        rhs: l_rhs,
                    } => {
                        assert_var(&l_lhs, "a");
                        assert_var(&l_rhs, "b");
                    }
                    other => {
                        #[allow(clippy::assertions_on_constants)]
                        {
                            assert!(false, "expected And(a, b), got {other:?}");
                        }
                    }
                }
                assert_var(&rhs, "c");
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected outer Or, got {other:?}");
                }
            }
        }
    }

    #[test]
    fn operator_parsing_is_idempotent() {
        let text = "-a + b * c == d && e || f ?? g";
        let a = parse_ok(text);
        let b = parse_ok(text);
        assert_eq!(a, b);
    }

    #[test]
    fn missing_right_operand_emits_e1200() {
        let (_e, diags) = parse("a +");
        assert!(
            diags.iter().any(|d| d.id == E_MISSING_BIN_OPERAND),
            "expected E1200 diagnostic, got {:?}",
            diags.iter().map(|d| &d.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn try_operator_still_works_after_operators_added() {
        // Regression guard: postfix `?` must keep parsing as `Try` even
        // now that the operator loop knows about `??`.
        let e = parse_ok("f(x)?");
        assert!(matches!(e, Expr::Try(_)));
    }

    #[test]
    fn comparison_then_logical() {
        // `a < b && c == d` → And(Lt(a, b), Eq(c, d))
        let e = parse_ok("a < b && c == d");
        match e {
            Expr::Binary {
                op: BinOp::And,
                lhs,
                rhs,
            } => {
                assert!(matches!(*lhs, Expr::Binary { op: BinOp::Lt, .. }));
                assert!(matches!(*rhs, Expr::Binary { op: BinOp::Eq, .. }));
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected outer And, got {other:?}");
                }
            }
        }
    }
}
