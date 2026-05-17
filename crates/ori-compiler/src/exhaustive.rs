//! Exhaustive `match` checker for the bootstrap compiler.
//!
//! This pass consumes the function bodies produced by [`crate::body`] and
//! walks every [`Expr::Match`] looking for two classes of bug:
//!
//! 1. **Non-exhaustive matches** (`E0540`) — when the scrutinee is a
//!    variant declared in this module and at least one declared
//!    constructor is missing from the arm list (and there is no wildcard
//!    arm to catch the gap).
//! 2. **Redundant arms** (`W0541`) — when the same constructor appears
//!    twice in the arm list.
//!
//! Plus a soft warning when arms appear after a `_` wildcard arm
//! (`W0542`), because those arms can never be reached.
//!
//! ## Where do declared constructors come from?
//!
//! The bootstrap parser only captures the *header line* of a `type`
//! declaration in the symbol signature. So this module supports two
//! discovery paths:
//!
//! * Single-line variant declarations (`type Status = | A | B | C`) are
//!   parsed straight out of the symbol's `signature` string.
//! * For projects that already have source on disk, the optional
//!   [`check_module_matches_with_source`] entry point re-lexes the full
//!   source so multi-line variants are recognised too.
//!
//! The base [`check_module_matches`] entry point requires only the
//! `Module` and parsed bodies; missing variant info simply means the
//! checker stays silent for that match, which preserves the
//! "no false positives in the bootstrap" promise.
//!
//! ## Diagnostic IDs
//!
//! * `E0540` — non-exhaustive match (carries an `insert_match_arm` fix).
//! * `W0541` — redundant arm.
//! * `W0542` — unreachable arm after wildcard.

use crate::ast::{Module, SymbolKind};
use crate::body::ModuleBodies;
use crate::diagnostic::{Diagnostic, Fix};
use crate::expr::{Expr, MatchArm, Pattern, Stmt};
use crate::lexer::{lex, Token, TokenKind};
use crate::source::{SourceFile, Span};
use std::collections::BTreeMap;

/// Walk every body in `bodies`, verifying that every `match` over a known
/// module-local variant covers each constructor at least once.
///
/// Variant constructors are derived from the headers of the module's
/// `Type` symbols (`type Status = | A | B`). When a match's scrutinee
/// cannot be resolved to a known variant, the checker emits nothing
/// rather than guess.
pub fn check_module_matches(module: &Module, bodies: &ModuleBodies) -> Vec<Diagnostic> {
    let variants = collect_variants_from_module(module);
    check_with_variants(module, bodies, &variants)
}

/// Like [`check_module_matches`] but also derives multi-line variant
/// declarations from the original source file. Use this when the caller
/// already has the [`SourceFile`] handy and wants the strongest
/// exhaustiveness analysis.
pub fn check_module_matches_with_source(
    source: &SourceFile,
    module: &Module,
    bodies: &ModuleBodies,
) -> Vec<Diagnostic> {
    let mut variants = collect_variants_from_module(module);
    let from_src = collect_variants_from_source(source);
    for (name, ctors) in from_src {
        // Source-derived info supersedes the (line-limited) signature
        // view because it can see multi-line variants.
        variants.insert(name, ctors);
    }
    check_with_variants(module, bodies, &variants)
}

/// Variant discovery: parse the full source for `type X = | A | B(...)`
/// declarations, with support for multi-line bodies.
///
/// Public so other passes (and tests) can introspect the same view used
/// by the exhaustive checker.
pub fn collect_variants_from_source(source: &SourceFile) -> BTreeMap<String, Vec<String>> {
    let tokens = lex(source);
    collect_variants_from_tokens(&tokens)
}

// ---------------------------------------------------------------------------
// Variant discovery
// ---------------------------------------------------------------------------

fn collect_variants_from_module(module: &Module) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    for symbol in &module.symbols {
        if symbol.kind != SymbolKind::Type {
            continue;
        }
        if let Some((name, ctors)) = parse_single_line_variant(&symbol.signature) {
            if !ctors.is_empty() && name == symbol.name {
                out.insert(name, ctors);
            }
        }
    }
    out
}

/// Parse a single-line variant declaration like
/// `type Status = | A | B(payload: T) | C` into `(type_name, [A, B, C])`.
fn parse_single_line_variant(signature: &str) -> Option<(String, Vec<String>)> {
    let trimmed = signature.trim();
    let rest = trimmed.strip_prefix("type ")?;
    // Split on `=`; the LHS is the type name (plus optional generics).
    let eq = rest.find('=')?;
    let head = rest[..eq].trim();
    let body = rest[eq + 1..].trim();
    if body.is_empty() {
        return None;
    }
    let name = head
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .find(|seg| !seg.is_empty())?
        .to_string();
    let ctors = parse_variant_arms(body);
    if ctors.is_empty() {
        None
    } else {
        Some((name, ctors))
    }
}

fn parse_variant_arms(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    // Split on `|`, ignoring a leading bar if present.
    for raw in body.split('|') {
        let segment = raw.trim();
        if segment.is_empty() {
            continue;
        }
        // Constructor name is the leading identifier; strip any `(...)`.
        let name: String = segment
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() {
            continue;
        }
        if name
            .chars()
            .next()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false)
        {
            out.push(name);
        }
    }
    out
}

fn collect_variants_from_tokens(tokens: &[Token]) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];
        if t.kind == TokenKind::Keyword && t.lexeme == "type" {
            if let Some((name, ctors, advance)) = parse_variant_decl_from_tokens(tokens, i) {
                if !ctors.is_empty() {
                    out.insert(name, ctors);
                }
                i = advance;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn parse_variant_decl_from_tokens(
    tokens: &[Token],
    start: usize,
) -> Option<(String, Vec<String>, usize)> {
    // Expect: `type Ident [generics?] = | Ctor (...)? | Ctor (...)? ...`
    let name_tok = tokens.get(start + 1)?;
    if name_tok.kind != TokenKind::Ident {
        return None;
    }
    let type_name = name_tok.lexeme.clone();
    let mut i = start + 2;
    // Skip optional generic suffix `[T, U]`
    if let Some(tok) = tokens.get(i) {
        if tok.kind == TokenKind::Symbol && tok.lexeme == "[" {
            let mut depth = 1;
            i += 1;
            while i < tokens.len() && depth > 0 {
                let inner = &tokens[i];
                if inner.kind == TokenKind::Eof {
                    return None;
                }
                if inner.kind == TokenKind::Symbol && inner.lexeme == "[" {
                    depth += 1;
                } else if inner.kind == TokenKind::Symbol && inner.lexeme == "]" {
                    depth -= 1;
                }
                i += 1;
            }
        }
    }
    // Require `=`
    let eq = tokens.get(i)?;
    if !(eq.kind == TokenKind::Symbol && eq.lexeme == "=") {
        return None;
    }
    i += 1;

    let mut ctors = Vec::new();
    let mut saw_bar = false;
    while i < tokens.len() {
        let tok = &tokens[i];
        if tok.kind == TokenKind::Eof {
            break;
        }
        // Stop when we hit another top-level item keyword on a new line.
        if is_top_level_terminator(tok)
            && (i == 0 || tokens[i - 1].span.start.line != tok.span.start.line)
        {
            break;
        }
        match (tok.kind, tok.lexeme.as_str()) {
            (TokenKind::Symbol, "|") => {
                saw_bar = true;
                i += 1;
                continue;
            }
            (TokenKind::Ident, lex) if saw_bar => {
                if lex
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_uppercase())
                    .unwrap_or(false)
                {
                    ctors.push(lex.to_string());
                }
                saw_bar = false;
                i += 1;
                // Skip an optional `(...)` payload.
                if let Some(open) = tokens.get(i) {
                    if open.kind == TokenKind::Symbol && open.lexeme == "(" {
                        let mut depth = 1;
                        i += 1;
                        while i < tokens.len() && depth > 0 {
                            let inner = &tokens[i];
                            if inner.kind == TokenKind::Eof {
                                break;
                            }
                            if inner.kind == TokenKind::Symbol && inner.lexeme == "(" {
                                depth += 1;
                            } else if inner.kind == TokenKind::Symbol && inner.lexeme == ")" {
                                depth -= 1;
                            }
                            i += 1;
                        }
                    }
                }
                continue;
            }
            // Anything else (e.g. `{` for a record body, or a new item
            // keyword) terminates this declaration.
            (TokenKind::Symbol, "{") => return None,
            (TokenKind::Keyword, kw) if is_item_kw(kw) => break,
            _ => {
                i += 1;
            }
        }
    }

    if ctors.is_empty() {
        None
    } else {
        Some((type_name, ctors, i))
    }
}

fn is_item_kw(kw: &str) -> bool {
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

fn is_top_level_terminator(tok: &Token) -> bool {
    tok.kind == TokenKind::Keyword && is_item_kw(&tok.lexeme)
}

// ---------------------------------------------------------------------------
// Core checking
// ---------------------------------------------------------------------------

fn check_with_variants(
    module: &Module,
    bodies: &ModuleBodies,
    variants: &BTreeMap<String, Vec<String>>,
) -> Vec<Diagnostic> {
    // Reverse map: constructor name → owning variant name. We assume the
    // bootstrap module keeps constructor names unique across variants;
    // collisions cause the constructor to be removed from the reverse map
    // because we cannot disambiguate without inferred types.
    let mut ctor_to_variant: BTreeMap<String, Option<String>> = BTreeMap::new();
    for (variant, ctors) in variants {
        for c in ctors {
            ctor_to_variant
                .entry(c.clone())
                .and_modify(|slot| {
                    if slot.as_deref() != Some(variant.as_str()) {
                        *slot = None;
                    }
                })
                .or_insert_with(|| Some(variant.clone()));
        }
    }

    let mut diags = Vec::new();
    for symbol in &module.symbols {
        if symbol.kind != SymbolKind::Function {
            continue;
        }
        let Some(body) = bodies.get(&symbol.id) else {
            continue;
        };
        walk_expr(
            body,
            &symbol.id,
            &symbol.span,
            variants,
            &ctor_to_variant,
            &mut diags,
        );
    }
    diags
}

fn walk_expr(
    expr: &Expr,
    symbol_id: &str,
    symbol_span: &Span,
    variants: &BTreeMap<String, Vec<String>>,
    ctor_to_variant: &BTreeMap<String, Option<String>>,
    diags: &mut Vec<Diagnostic>,
) {
    match expr {
        Expr::Match { scrutinee, arms } => {
            // Recurse into the scrutinee and arm bodies first so nested
            // matches are still checked.
            walk_expr(
                scrutinee,
                symbol_id,
                symbol_span,
                variants,
                ctor_to_variant,
                diags,
            );
            for arm in arms {
                walk_expr(
                    &arm.body,
                    symbol_id,
                    symbol_span,
                    variants,
                    ctor_to_variant,
                    diags,
                );
            }
            check_match(
                symbol_id,
                symbol_span,
                arms,
                variants,
                ctor_to_variant,
                diags,
            );
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            walk_expr(
                cond,
                symbol_id,
                symbol_span,
                variants,
                ctor_to_variant,
                diags,
            );
            walk_expr(
                then_branch,
                symbol_id,
                symbol_span,
                variants,
                ctor_to_variant,
                diags,
            );
            if let Some(e) = else_branch {
                walk_expr(e, symbol_id, symbol_span, variants, ctor_to_variant, diags);
            }
        }
        Expr::Block { stmts, tail } => {
            for s in stmts {
                walk_stmt(s, symbol_id, symbol_span, variants, ctor_to_variant, diags);
            }
            if let Some(t) = tail {
                walk_expr(t, symbol_id, symbol_span, variants, ctor_to_variant, diags);
            }
        }
        Expr::Call { callee, args } => {
            walk_expr(
                callee,
                symbol_id,
                symbol_span,
                variants,
                ctor_to_variant,
                diags,
            );
            for a in args {
                walk_expr(a, symbol_id, symbol_span, variants, ctor_to_variant, diags);
            }
        }
        Expr::Field { base, .. } => {
            walk_expr(
                base,
                symbol_id,
                symbol_span,
                variants,
                ctor_to_variant,
                diags,
            );
        }
        Expr::Return(Some(e)) => {
            walk_expr(e, symbol_id, symbol_span, variants, ctor_to_variant, diags);
        }
        Expr::Try(inner) => {
            walk_expr(
                inner,
                symbol_id,
                symbol_span,
                variants,
                ctor_to_variant,
                diags,
            );
        }
        Expr::Construct { args, .. } => {
            for a in args {
                walk_expr(a, symbol_id, symbol_span, variants, ctor_to_variant, diags);
            }
        }
        Expr::Tuple(items) => {
            for it in items {
                walk_expr(it, symbol_id, symbol_span, variants, ctor_to_variant, diags);
            }
        }
        Expr::Record { fields } => {
            for (_, v) in fields {
                walk_expr(v, symbol_id, symbol_span, variants, ctor_to_variant, diags);
            }
        }
        Expr::Lambda { body, .. } => {
            walk_expr(
                body,
                symbol_id,
                symbol_span,
                variants,
                ctor_to_variant,
                diags,
            );
        }
        Expr::Lit(_) | Expr::Var(_) | Expr::Return(None) | Expr::Error => {}
    }
}

fn walk_stmt(
    stmt: &Stmt,
    symbol_id: &str,
    symbol_span: &Span,
    variants: &BTreeMap<String, Vec<String>>,
    ctor_to_variant: &BTreeMap<String, Option<String>>,
    diags: &mut Vec<Diagnostic>,
) {
    match stmt {
        Stmt::Let { init, .. } => {
            walk_expr(
                init,
                symbol_id,
                symbol_span,
                variants,
                ctor_to_variant,
                diags,
            );
        }
        Stmt::Expr(e) | Stmt::Return(Some(e)) => {
            walk_expr(e, symbol_id, symbol_span, variants, ctor_to_variant, diags);
        }
        Stmt::Return(None) => {}
    }
}

fn check_match(
    symbol_id: &str,
    symbol_span: &Span,
    arms: &[MatchArm],
    variants: &BTreeMap<String, Vec<String>>,
    ctor_to_variant: &BTreeMap<String, Option<String>>,
    diags: &mut Vec<Diagnostic>,
) {
    // Categorise arm patterns.
    let mut wildcard_idx: Option<usize> = None;
    let mut seen: Vec<(String, usize)> = Vec::new(); // (ctor, arm_index)
    for (idx, arm) in arms.iter().enumerate() {
        match &arm.pattern {
            Pattern::Wildcard | Pattern::Binding(_) => {
                if wildcard_idx.is_none() {
                    wildcard_idx = Some(idx);
                }
            }
            Pattern::Constructor { name, .. } => {
                seen.push((name.clone(), idx));
            }
            Pattern::Literal(_) => {
                // Literal-pattern matches are not variant matches.
            }
        }
    }

    // W0542 — arms after a wildcard are unreachable.
    if let Some(w_idx) = wildcard_idx {
        for (idx, arm) in arms.iter().enumerate().skip(w_idx + 1) {
            diags.push(
                Diagnostic::warning(
                    "W0542",
                    "match arm is unreachable: a wildcard arm precedes it",
                    symbol_span.clone(),
                )
                .with_symbol(symbol_id.to_string())
                .with_found(vec![pattern_display(&arm.pattern)])
                .with_expected(vec!["arm before the wildcard `_` arm".to_string()])
                .with_agent_summary("Reorder this arm before the wildcard, or remove it.")
                .with_docs(vec!["doc:patterns.exhaustiveness".to_string()]),
            );
            let _ = idx;
        }
    }

    // W0541 — duplicate constructors.
    let mut seen_names = BTreeMap::<String, usize>::new();
    for (name, _idx) in &seen {
        let count = seen_names.entry(name.clone()).or_insert(0);
        *count += 1;
    }
    for (name, count) in &seen_names {
        if *count > 1 {
            diags.push(
                Diagnostic::warning(
                    "W0541",
                    format!("match arm for `{name}` appears more than once"),
                    symbol_span.clone(),
                )
                .with_symbol(symbol_id.to_string())
                .with_found(vec![name.clone()])
                .with_expected(vec!["each constructor at most once".to_string()])
                .with_agent_summary("Remove the duplicate arm or merge their bodies.")
                .with_docs(vec!["doc:patterns.redundant".to_string()]),
            );
        }
    }

    // Variant resolution — only proceed for exhaustiveness if we can tie
    // the arms to a single declared variant.
    let variant_name = resolve_variant(&seen, ctor_to_variant);
    let Some(variant_name) = variant_name else {
        return;
    };
    let Some(declared) = variants.get(&variant_name) else {
        return;
    };

    // E0540 — wildcard makes a match exhaustive; skip the missing-arm check.
    if wildcard_idx.is_some() {
        return;
    }

    let seen_set: std::collections::BTreeSet<&str> = seen.iter().map(|(n, _)| n.as_str()).collect();
    for ctor in declared {
        if !seen_set.contains(ctor.as_str()) {
            let fix = Fix::new(
                "insert_match_arm",
                format!("Add a match arm for `{ctor}`."),
                0.9,
            )
            .with_patch(serde_json::json!({
                "schema": "ori.patch.v1",
                "intent": format!(
                    "Add the missing `{ctor}` arm to satisfy exhaustiveness for `{variant_name}` in `{symbol_id}`."
                ),
                "operations": [{
                    "op": "insert_match_arm",
                    "target": symbol_id,
                    "pattern": ctor,
                    "body": "todo()",
                }],
                "tests": { "run": ["cargo test -p ori-compiler exhaustive"] }
            }));
            diags.push(
                Diagnostic::error(
                    "E0540",
                    format!("match is not exhaustive: missing arm `{ctor}`"),
                    symbol_span.clone(),
                )
                .with_symbol(symbol_id.to_string())
                .with_expected(declared.to_vec())
                .with_found(vec![ctor.clone()])
                .with_fix(fix)
                .with_agent_summary("Add an arm for the missing variant or use a wildcard `_` arm.")
                .with_docs(vec!["doc:patterns.exhaustiveness".to_string()]),
            );
        }
    }
}

fn resolve_variant(
    seen: &[(String, usize)],
    ctor_to_variant: &BTreeMap<String, Option<String>>,
) -> Option<String> {
    let mut found: Option<String> = None;
    for (name, _) in seen {
        let entry = ctor_to_variant.get(name)?;
        let variant = entry.clone()?;
        match &found {
            None => found = Some(variant),
            Some(existing) if existing == &variant => {}
            Some(_) => return None,
        }
    }
    found
}

fn pattern_display(pat: &Pattern) -> String {
    match pat {
        Pattern::Wildcard => "_".to_string(),
        Pattern::Binding(name) => name.clone(),
        Pattern::Literal(_) => "<literal>".to_string(),
        Pattern::Constructor { name, args } => {
            if args.is_empty() {
                name.clone()
            } else {
                format!("{name}(..)")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::parse_module_bodies;
    use crate::parser::parse_source;
    use crate::source::SourceFile;

    fn check(text: &str) -> Vec<Diagnostic> {
        let src = SourceFile::new("/exh.ori", text);
        let module = parse_source(&src).module;
        let bodies = parse_module_bodies(&src);
        check_module_matches_with_source(&src, &module, &bodies)
    }

    fn diag_ids(diags: &[Diagnostic]) -> Vec<&str> {
        diags.iter().map(|d| d.id.as_str()).collect()
    }

    // --- 1. Missing arm ---
    #[test]
    fn missing_arm_emits_e0540() {
        let text = "module a\n\
                    type Status = | A | B | C\n\
                    fn handle(s: Status) -> Int:\n\
                    \x20\x20match s | A => 1 | B => 2\n";
        let diags = check(text);
        let ids = diag_ids(&diags);
        if !ids.contains(&"E0540") {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected E0540 in {ids:?}");
            }
        }
        // Verify the missing-variant name is carried in `found` and the
        // owning symbol is set.
        let missing = diags.iter().find(|d| d.id == "E0540");
        let Some(d) = missing else {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected at least one E0540 diagnostic");
            }
            return;
        };
        assert_eq!(d.found, vec!["C".to_string()]);
        let sym = d.symbol.as_ref();
        let Some(sym) = sym else {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected symbol on E0540");
            }
            return;
        };
        assert_eq!(sym.id, "sym:a.handle");
        // The fix payload must encode an insert_match_arm operation.
        let fix = d.fixes.first();
        let Some(fix) = fix else {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected a fix on E0540");
            }
            return;
        };
        let Some(patch) = &fix.patch else {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected a patch IR payload");
            }
            return;
        };
        let op = patch
            .get("operations")
            .and_then(|o| o.get(0))
            .and_then(|o| o.get("op"))
            .and_then(|o| o.as_str())
            .unwrap_or("");
        assert_eq!(op, "insert_match_arm");
    }

    // --- 2. Redundant arm ---
    #[test]
    fn redundant_arm_emits_w0541() {
        let text = "module a\n\
                    type Status = | A | B\n\
                    fn handle(s: Status) -> Int:\n\
                    \x20\x20match s | A => 1 | A => 2 | B => 3\n";
        let diags = check(text);
        assert!(diag_ids(&diags).contains(&"W0541"));
    }

    // --- 3. Wildcard present means exhaustive ---
    #[test]
    fn wildcard_makes_match_exhaustive() {
        let text = "module a\n\
                    type Status = | A | B | C\n\
                    fn handle(s: Status) -> Int:\n\
                    \x20\x20match s | A => 1 | _ => 0\n";
        let diags = check(text);
        let ids = diag_ids(&diags);
        if ids.contains(&"E0540") {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected no E0540 with wildcard, got {ids:?}");
            }
        }
    }

    // --- 4. Multiple variants (sibling matches) ---
    #[test]
    fn multiple_variants_in_module_check_independently() {
        let text = "module a\n\
                    type Color = | Red | Green | Blue\n\
                    type Size = | Small | Large\n\
                    fn name_color(c: Color) -> Int:\n\
                    \x20\x20match c | Red => 1 | Green => 2\n\
                    fn name_size(s: Size) -> Int:\n\
                    \x20\x20match s | Small => 1 | Large => 2\n";
        let diags = check(text);
        // Color is missing Blue → E0540.
        let color_diag = diags
            .iter()
            .find(|d| d.id == "E0540" && d.found.first().map(String::as_str) == Some("Blue"));
        if color_diag.is_none() {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected E0540 for missing Blue in Color match");
            }
        }
        // Size is fully covered → no E0540 with Small/Large.
        let size_diag = diags.iter().find(|d| {
            d.id == "E0540" && d.symbol.as_ref().map(|s| s.id.as_str()) == Some("sym:a.name_size")
        });
        if size_diag.is_some() {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "did not expect E0540 for fully-covered Size match");
            }
        }
    }

    // --- 5. Payload-bearing arms still count toward coverage ---
    #[test]
    fn payload_bearing_arms_count_toward_coverage() {
        let text = "module a\n\
                    type Result2 = | Ok2 | Err2\n\
                    fn handle(r: Result2) -> Int:\n\
                    \x20\x20match r | Ok2(v) => v | Err2(e) => 0\n";
        let diags = check(text);
        // Constructors with payloads still register as covered.
        let any = diags.iter().any(|d| d.id == "E0540");
        if any {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected no E0540 with payload-bearing arms");
            }
        }
    }

    // --- 6. Nested matches are walked (constructor `Match` expressions
    // inside arm bodies are recursed into). The bootstrap parser
    // doesn't distinguish inner-vs-outer `|`-ownership without
    // parentheses, so we exercise the recursive walk by placing the
    // nested match inside an explicit `if` arm body — a sibling shape
    // produced by real Orison bodies. ---
    #[test]
    fn nested_matches_are_walked() {
        let text = "module a\n\
                    type Status = | A | B | C\n\
                    fn handle(s: Status, t: Status) -> Int:\n\
                    \x20\x20if cond: match t | A => 1 | B => 2\n";
        let diags = check(text);
        // The nested match inside the `if` body is missing `C` → E0540.
        let missing_c = diags
            .iter()
            .any(|d| d.id == "E0540" && d.found.first().map(String::as_str) == Some("C"));
        if !missing_c {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(
                    false,
                    "expected nested E0540 for missing C, got {:?}",
                    diag_ids(&diags)
                );
            }
        }
    }

    // --- 7. Unreachable arm after wildcard ---
    #[test]
    fn unreachable_arm_after_wildcard_emits_w0542() {
        let text = "module a\n\
                    type Status = | A | B\n\
                    fn handle(s: Status) -> Int:\n\
                    \x20\x20match s | A => 1 | _ => 0 | B => 2\n";
        let diags = check(text);
        assert!(diag_ids(&diags).contains(&"W0542"));
    }

    // --- 8. Unknown variant → silent (no false positives) ---
    #[test]
    fn unknown_scrutinee_variant_is_silent() {
        let text = "module a\n\
                    fn handle(x: Int) -> Int:\n\
                    \x20\x20match x | 0 => 1 | 1 => 2\n";
        let diags = check(text);
        let any = diags.iter().any(|d| d.id == "E0540");
        if any {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(
                    false,
                    "did not expect E0540 for literal match without variant"
                );
            }
        }
    }

    // --- 9. Multi-line variant declarations are recognised via source ---
    #[test]
    fn multi_line_variants_are_recognised_via_source() {
        let text = "module a\n\
                    type Status =\n\
                    \x20\x20| A\n\
                    \x20\x20| B\n\
                    \x20\x20| C\n\
                    fn handle(s: Status) -> Int:\n\
                    \x20\x20match s | A => 1 | B => 2\n";
        let diags = check(text);
        assert!(diag_ids(&diags).contains(&"E0540"));
    }

    // --- 10. Fully covered match emits nothing ---
    #[test]
    fn fully_covered_match_is_clean() {
        let text = "module a\n\
                    type Status = | A | B | C\n\
                    fn handle(s: Status) -> Int:\n\
                    \x20\x20match s | A => 1 | B => 2 | C => 3\n";
        let diags = check(text);
        let bad = diags
            .iter()
            .any(|d| d.id == "E0540" || d.id == "W0541" || d.id == "W0542");
        if bad {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(
                    false,
                    "expected no exhaustive diagnostics, got {:?}",
                    diag_ids(&diags)
                );
            }
        }
    }

    // --- 11. base API (without source) still works on single-line decls ---
    #[test]
    fn base_api_uses_signature_only() {
        let text = "module a\n\
                    type Status = | A | B\n\
                    fn handle(s: Status) -> Int:\n\
                    \x20\x20match s | A => 1\n";
        let src = SourceFile::new("/exh.ori", text);
        let module = parse_source(&src).module;
        let bodies = parse_module_bodies(&src);
        let diags = check_module_matches(&module, &bodies);
        assert!(diag_ids(&diags).contains(&"E0540"));
    }
}
