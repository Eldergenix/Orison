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
    /// Recovery node. Emitted alongside a diagnostic whenever the parser
    /// hits something it cannot understand. Callers can treat it as
    /// "unknown" without crashing.
    Error,
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
        self.parse_postfix_chain()
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
            if self.eat_symbol("?") {
                expr = Expr::Try(Box::new(expr));
                continue;
            }
            break;
        }
        expr
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
                Expr::Lit(Literal::Str(token.lexeme.clone()))
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
                if !d.id.starts_with("E11") {
                    assert!(false, "diag {} not in body-parser range", d.id);
                }
            }
        }
    }
}
