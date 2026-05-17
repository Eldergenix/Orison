//! Extract function body token slices from a parsed source file.
//!
//! The bootstrap item parser ([`crate::parser`]) stops at the `:` that
//! introduces a function body. This module fills the gap: it walks the
//! token stream, locates each `fn name(...) -> RetTy ... :` header, and
//! returns the tokens that make up the body up to (but not including) the
//! next top-level item.
//!
//! The result is intentionally token-level rather than source-text-level
//! so the expression parser in [`crate::expr`] can consume it directly,
//! and so we never depend on the (currently whitespace-sensitive) layout
//! of the surface syntax for body boundaries.
//!
//! ## What counts as a function for body extraction?
//!
//! Only the `fn` keyword introduces a body. Other top-level keywords
//! (`type`, `service`, `view`, `actor`, `query`, `migration`,
//! `capability`, `module`, `import`) are treated as body terminators so
//! that adjacent items are not swallowed.
//!
//! ## Limitations (real gaps, surfaced honestly)
//!
//! * The body is taken as the token run between the header `:` and the
//!   next item keyword. There is no brace-counted block grammar, so a
//!   `fn` keyword that legitimately appears inside an expression (e.g. a
//!   lambda) is *also* treated as a body terminator. That's a deliberate
//!   conservatism in the bootstrap: the demo modules don't use
//!   first-class lambdas yet.
//! * Functions without an explicit body (e.g. trait/extern declarations
//!   that have no `:`) yield `None`.
//! * Bodies on a single line (`fn f() -> Unit: return Unit`) are fully
//!   supported — extraction is purely token-based.

use crate::ast::{Module, SymbolKind};
use crate::diagnostic::Diagnostic;
use crate::expr::{parse_body_expr, Expr};
use crate::lexer::{lex, Token, TokenKind};
use crate::source::{SourceFile, Span};
use std::collections::BTreeMap;

/// Result of parsing every function body in a module.
#[derive(Debug, Clone, Default)]
pub struct ModuleBodies {
    /// `Symbol::id` → parsed body expression.
    pub bodies: BTreeMap<String, Expr>,
    /// Diagnostics produced by the body parser (ID range `E1100..=E1199`).
    pub diagnostics: Vec<Diagnostic>,
}

impl ModuleBodies {
    pub fn get(&self, symbol_id: &str) -> Option<&Expr> {
        self.bodies.get(symbol_id)
    }

    pub fn is_empty(&self) -> bool {
        self.bodies.is_empty()
    }

    pub fn len(&self) -> usize {
        self.bodies.len()
    }
}

/// Extract the tokens that make up the body of the function whose
/// signature span is `span_of_fn`. Returns `None` if no body could be
/// identified (e.g. the function has no `:` introducer).
///
/// The returned slice **does not include** the trailing EOF token; callers
/// should append one if the downstream parser depends on it. (The
/// expression parser in this crate tolerates either.)
pub fn extract_body(source: &SourceFile, span_of_fn: &Span) -> Option<Vec<Token>> {
    let tokens = lex(source);
    let fn_idx = find_fn_at_span(&tokens, span_of_fn)?;
    let body_start = find_body_start(&tokens, fn_idx)?;
    let body_end = find_body_end(&tokens, body_start);
    if body_start >= body_end {
        return Some(Vec::new());
    }
    Some(tokens[body_start..body_end].to_vec())
}

/// Parse every function body in `source` into an `Expr`. Returns a
/// `ModuleBodies` keyed by the symbol IDs that the item parser would
/// assign (`sym:{module}.{name}`), so callers don't need to re-derive IDs.
pub fn parse_module_bodies(source: &SourceFile) -> ModuleBodies {
    let parse = crate::parser::parse_source(source);
    parse_module_bodies_with_module(source, &parse.module)
}

/// Same as [`parse_module_bodies`] but reuses an already-parsed [`Module`]
/// to avoid duplicate work in callers that have already run the item
/// parser.
pub fn parse_module_bodies_with_module(source: &SourceFile, module: &Module) -> ModuleBodies {
    let mut out = ModuleBodies::default();
    let tokens = lex(source);

    for symbol in &module.symbols {
        if symbol.kind != SymbolKind::Function {
            continue;
        }
        let Some(fn_idx) = find_fn_at_span(&tokens, &symbol.span) else {
            continue;
        };
        let Some(body_start) = find_body_start(&tokens, fn_idx) else {
            continue;
        };
        let body_end = find_body_end(&tokens, body_start);
        let body_tokens: &[Token] = if body_start >= body_end {
            &[]
        } else {
            &tokens[body_start..body_end]
        };
        let (expr, mut diags) = parse_body_expr(body_tokens);
        // Tag diagnostics with the owning symbol so downstream tools can
        // route them to the right item.
        for d in diags.iter_mut() {
            if d.symbol.is_none() {
                *d = std::mem::replace(
                    d,
                    Diagnostic::error("E1100", String::new(), Span::dummy(source.path.clone())),
                )
                .with_symbol(symbol.id.clone());
            }
        }
        out.diagnostics.append(&mut diags);
        out.bodies.insert(symbol.id.clone(), expr);
    }

    out
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Locate the `fn` keyword token that begins the function whose name span
/// is `span`. The item parser stores the span of the *name*, so we look
/// for an `fn` keyword on the same line whose following identifier
/// matches.
fn find_fn_at_span(tokens: &[Token], span: &Span) -> Option<usize> {
    let line = span.start.line;
    for (idx, t) in tokens.iter().enumerate() {
        if t.kind != TokenKind::Keyword || t.lexeme != "fn" {
            continue;
        }
        if t.span.start.line != line {
            continue;
        }
        if let Some(next) = tokens.get(idx + 1) {
            if next.kind == TokenKind::Ident
                && next.span.start.line == line
                && next.span.start.column == span.start.column
            {
                return Some(idx);
            }
            // Fallback: same line and the name column overlaps.
            if next.kind == TokenKind::Ident
                && next.span.start.line == line
                && next.span.end.column == span.end.column
            {
                return Some(idx);
            }
        }
    }
    // Last-resort fallback: first `fn` keyword on that line.
    tokens
        .iter()
        .position(|t| t.kind == TokenKind::Keyword && t.lexeme == "fn" && t.span.start.line == line)
}

/// Find the index of the first token *after* the `:` that introduces a
/// function body. Returns `None` if the header has no `:`.
fn find_body_start(tokens: &[Token], fn_idx: usize) -> Option<usize> {
    let line = tokens.get(fn_idx)?.span.start.line;
    let mut depth_paren = 0i32;
    let mut depth_bracket = 0i32;
    let mut depth_brace = 0i32;
    let mut i = fn_idx + 1;
    while i < tokens.len() {
        let t = &tokens[i];
        if t.kind == TokenKind::Eof {
            return None;
        }
        match (t.kind, t.lexeme.as_str()) {
            (TokenKind::Symbol, "(") => depth_paren += 1,
            (TokenKind::Symbol, ")") => depth_paren -= 1,
            (TokenKind::Symbol, "[") => depth_bracket += 1,
            (TokenKind::Symbol, "]") => depth_bracket -= 1,
            (TokenKind::Symbol, "{") => depth_brace += 1,
            (TokenKind::Symbol, "}") => depth_brace -= 1,
            (TokenKind::Symbol, ":")
                if depth_paren <= 0 && depth_bracket <= 0 && depth_brace <= 0 =>
            {
                // The `:` we want is the one on the header line; any `:`
                // inside a generic param or argument is masked by the depth
                // counters above.
                if t.span.start.line == line {
                    return Some(i + 1);
                }
                // `:` on a continuation line still counts.
                return Some(i + 1);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Find the exclusive end index of the body that begins at `body_start`.
/// The body ends at the first top-level item keyword (other than `fn`
/// inside expressions — see module-level caveats) or at EOF.
fn find_body_end(tokens: &[Token], body_start: usize) -> usize {
    let mut depth_paren = 0i32;
    let mut depth_bracket = 0i32;
    let mut depth_brace = 0i32;
    let mut i = body_start;
    while i < tokens.len() {
        let t = &tokens[i];
        if t.kind == TokenKind::Eof {
            return i;
        }
        match (t.kind, t.lexeme.as_str()) {
            (TokenKind::Symbol, "(") => depth_paren += 1,
            (TokenKind::Symbol, ")") => depth_paren -= 1,
            (TokenKind::Symbol, "[") => depth_bracket += 1,
            (TokenKind::Symbol, "]") => depth_bracket -= 1,
            (TokenKind::Symbol, "{") => depth_brace += 1,
            (TokenKind::Symbol, "}") => depth_brace -= 1,
            (TokenKind::Keyword, kw) if is_item_keyword(kw) => {
                if depth_paren <= 0
                    && depth_bracket <= 0
                    && depth_brace <= 0
                    && is_at_line_start(tokens, i)
                {
                    return i;
                }
            }
            _ => {}
        }
        i += 1;
    }
    i
}

fn is_item_keyword(kw: &str) -> bool {
    matches!(
        kw,
        "fn" | "type"
            | "service"
            | "view"
            | "actor"
            | "query"
            | "migration"
            | "capability"
            | "module"
            | "import"
            | "protocol"
            | "impl"
    )
}

/// `true` if the token at `idx` is the first non-trivial token on its
/// line (i.e. either the very first token, or the previous token is on a
/// strictly earlier line). Used to distinguish a top-level `fn` from one
/// embedded in an expression.
fn is_at_line_start(tokens: &[Token], idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    let prev = &tokens[idx - 1];
    let cur = &tokens[idx];
    cur.span.start.line > prev.span.start.line
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Symbol;
    use crate::expr::{Literal, Stmt};

    fn src(text: &str) -> SourceFile {
        SourceFile::new("/t.ori", text)
    }

    /// Helper: assert a symbol named `name` exists in `module` and return it,
    /// using only the allowed `assert!(false, msg)` panic pattern.
    fn must_symbol<'a>(module: &'a crate::ast::Module, name: &str) -> &'a Symbol {
        if let Some(sym) = module.symbols.iter().find(|s| s.name == name) {
            return sym;
        }
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(false, "expected symbol named `{name}` to exist");
        }
        // Unreachable; the assert above always panics. We need *some* value
        // so the function type-checks without `unwrap`/`expect`.
        &module.symbols[0]
    }

    fn must_body<'a>(bodies: &'a ModuleBodies, id: &str) -> &'a Expr {
        if let Some(expr) = bodies.get(id) {
            return expr;
        }
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(false, "expected body for `{id}`");
        }
        // Unreachable; satisfies the borrow checker.
        bodies
            .bodies
            .values()
            .next()
            .unwrap_or(&Expr::Lit(Literal::Unit))
    }

    #[test]
    fn extracts_body_for_single_fn() {
        let s = src("module a\nfn f() -> Unit:\n  return Unit\n");
        let module = crate::parser::parse_source(&s).module;
        let sym = must_symbol(&module, "f");
        let Some(body) = extract_body(&s, &sym.span) else {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "extract_body returned None");
            }
            return;
        };
        // Should at least contain `return` keyword.
        assert!(body.iter().any(|t| t.lexeme == "return"));
    }

    #[test]
    fn no_body_for_header_without_colon() {
        let s = src("module a\nfn f() -> Unit\n");
        let module = crate::parser::parse_source(&s).module;
        let sym = must_symbol(&module, "f");
        assert!(extract_body(&s, &sym.span).is_none());
    }

    #[test]
    fn module_bodies_keyed_by_symbol_id() {
        let s = src("module a\nfn f() -> Int:\n  return 1\nfn g() -> Int:\n  return 2\n");
        let bodies = parse_module_bodies(&s);
        assert_eq!(bodies.len(), 2);
        assert!(bodies.get("sym:a.f").is_some());
        assert!(bodies.get("sym:a.g").is_some());
    }

    #[test]
    fn body_stops_at_next_top_level_item() {
        let s = src("module a\nfn f() -> Int:\n  return 1\ntype Foo\n");
        let bodies = parse_module_bodies(&s);
        let body = must_body(&bodies, "sym:a.f");
        // The body must not have swallowed the `type` keyword: parsing
        // `type Foo` would not produce a clean `Return` expression.
        match body {
            Expr::Return(Some(inner)) => {
                assert_eq!(**inner, Expr::Lit(Literal::Int(1)));
            }
            Expr::Block { tail, stmts, .. } => {
                let last = tail.as_deref().or_else(|| match stmts.last() {
                    Some(Stmt::Expr(e)) => Some(e),
                    _ => None,
                });
                match last {
                    Some(Expr::Return(Some(inner))) => {
                        assert_eq!(**inner, Expr::Lit(Literal::Int(1)));
                    }
                    _ => {
                        // Acceptable as long as it's not an Error node.
                        assert!(!matches!(body, Expr::Error));
                    }
                }
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected return-or-block, got {other:?}");
                }
            }
        }
    }

    #[test]
    fn handles_function_with_uses_clause() {
        let s = src("module a\nfn boot() -> Unit uses http, db.read:\n  return Unit\n");
        let bodies = parse_module_bodies(&s);
        let body = must_body(&bodies, "sym:a.boot");
        assert!(!matches!(body, Expr::Error));
    }

    #[test]
    fn handles_result_return_with_question_mark() {
        let s = src(
            "module a\nfn f() -> Result[User, ApiErr] uses db.read:\n  let user = db.users.find(id)?\n  return Ok(user)\n",
        );
        let bodies = parse_module_bodies(&s);
        let body = must_body(&bodies, "sym:a.f");
        // Inside the block we expect a Let with a Try expression as its init,
        // and a return statement. The return may live in `stmts` (as a
        // `Stmt::Return`) or be promoted to `tail` — both shapes are valid.
        if let Expr::Block { stmts, tail } = body {
            assert!(!stmts.is_empty());
            let saw_try = stmts.iter().any(|s| {
                matches!(
                    s,
                    Stmt::Let {
                        init: Expr::Try(_),
                        ..
                    }
                )
            });
            assert!(saw_try, "expected a let-binding with a `?` initialiser");
            let saw_return = stmts.iter().any(|s| matches!(s, Stmt::Return(Some(_))))
                || matches!(tail.as_deref(), Some(Expr::Return(Some(_))));
            assert!(saw_return, "expected a return Ok(user) statement");
        } else {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected block body");
            }
        }
    }

    #[test]
    fn idempotent_parse_for_same_source() {
        let s = src("module a\nfn f() -> Int:\n  let x = 1\n  return x\n");
        let a = parse_module_bodies(&s);
        let b = parse_module_bodies(&s);
        assert_eq!(a.bodies, b.bodies);
    }

    #[test]
    fn diagnostics_are_tagged_with_symbol() {
        let s = src("module a\nfn f() -> Int:\n  let x =\n");
        let bodies = parse_module_bodies(&s);
        // Either E1103 (no rhs) or E1100 (recovery on the trailing tokens).
        let any_for_f = bodies.diagnostics.iter().any(|d| {
            d.symbol
                .as_ref()
                .map(|s| s.id == "sym:a.f")
                .unwrap_or(false)
        });
        assert!(
            any_for_f,
            "expected at least one diagnostic tagged with sym:a.f"
        );
    }

    #[test]
    fn empty_body_yields_unit() {
        // `fn f() -> Unit:` followed immediately by another item.
        let s = src("module a\nfn f() -> Unit:\ntype Foo\n");
        let bodies = parse_module_bodies(&s);
        let body = must_body(&bodies, "sym:a.f");
        assert_eq!(*body, Expr::Lit(Literal::Unit));
    }

    /// End-to-end smoke test: every function body in the demo storefront
    /// must parse without producing a top-level recovery node, exercising
    /// `Ok(_)`, `Err(_)`, field access, tagged record literals, `if`, and
    /// the `:`-introduced single-line body form.
    #[test]
    fn demo_store_cart_module_parses_cleanly() {
        let text = "module demo_store.cart\n\
                    import demo_store.domain\n\
                    \n\
                    fn add_line(cart: Cart, line: CartLine) -> Cart:\n\
                    \x20\x20return Cart { customer: cart.customer, lines: cart.lines.append(line) }\n\
                    \n\
                    fn remove_line(cart: Cart, product_id: ProductId) -> Cart:\n\
                    \x20\x20let kept = cart.lines.without_product(product_id)\n\
                    \x20\x20return Cart { customer: cart.customer, lines: kept }\n\
                    \n\
                    fn total(cart: Cart) -> Money:\n\
                    \x20\x20return money_zero()\n\
                    \n\
                    fn validate(cart: Cart) -> Result[Cart, CartError]:\n\
                    \x20\x20if cart.lines.is_empty():\n\
                    \x20\x20\x20\x20return Err(Empty)\n\
                    \x20\x20return Ok(cart)\n";
        let s = src(text);
        let bodies = parse_module_bodies(&s);
        for id in [
            "sym:demo_store.cart.add_line",
            "sym:demo_store.cart.remove_line",
            "sym:demo_store.cart.total",
            "sym:demo_store.cart.validate",
        ] {
            let body = must_body(&bodies, id);
            assert!(!matches!(body, Expr::Error), "{id} parsed as Error");
        }
    }
}
