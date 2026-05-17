//! SQL DSL query-shape extraction and validation for the bootstrap compiler.
//!
//! The bootstrap parser exposes each `query` declaration as a
//! [`Symbol`](crate::ast::Symbol) with a compact signature string that
//! follows the pattern:
//!
//! ```text
//! query find_user(id: UserId) -> {id: UserId, name: Str}
//! ```
//!
//! This module turns that surface text into a structured
//! [`QueryShape`] (its result-row columns) and applies two cheap but useful
//! correctness checks:
//!
//! * `Q0010` — a declared column references a type that is neither a known
//!   builtin, a type declared in this module, nor a permitted generic name.
//! * `Q0020` — two queries share the same name but declare different
//!   result-row shapes; the agent must converge on one canonical shape
//!   before the database layer can route either of them.
//!
//! The checker is intentionally conservative: if it cannot parse a shape
//! (e.g. the `query` is a stub with no `->` clause yet) it simply skips
//! that symbol so the edit-check loop keeps moving.

use crate::ast::{Module, SymbolKind};
use crate::diagnostic::Diagnostic;
use crate::types::is_builtin_type;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

/// Generic constructors that may appear unqualified in a column type and
/// must not be flagged as unknown by `Q0010`. Kept aligned with the wider
/// type checker's permitted-generic set.
const PERMITTED_GENERICS: &[&str] = &[
    "Option", "Result", "List", "Pair", "Fn", "Iter", "Query", "Map", "Set",
];

/// A single column in a query result row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QueryColumn {
    pub name: String,
    pub ty: String,
}

/// The structured shape of a `query` declaration's result row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QueryShape {
    pub columns: Vec<QueryColumn>,
}

/// Parse a compact `query` signature into a [`QueryShape`].
///
/// Accepts signatures whose return type is an inline record literal of the
/// form `{name: Type, name: Type}`. Returns `None` for any signature whose
/// return shape cannot be recovered — callers should treat that as a soft
/// "no shape declared yet" rather than an error.
pub fn extract_query_shape(signature: &str) -> Option<QueryShape> {
    let arrow = signature.find("->")?;
    let after = signature[arrow + 2..].trim();
    let open = after.find('{')?;
    let rest = &after[open + 1..];
    let close = rest.rfind('}')?;
    let inner = rest[..close].trim();
    if inner.is_empty() {
        return Some(QueryShape {
            columns: Vec::new(),
        });
    }
    let mut columns: Vec<QueryColumn> = Vec::new();
    for field in split_top_level_commas(inner) {
        let column = parse_field(field)?;
        columns.push(column);
    }
    Some(QueryShape { columns })
}

fn parse_field(field: &str) -> Option<QueryColumn> {
    let trimmed = field.trim();
    let colon = trimmed.find(':')?;
    let name = trimmed[..colon].trim();
    let ty = trimmed[colon + 1..].trim();
    if name.is_empty() || ty.is_empty() {
        return None;
    }
    if !is_ident(name) {
        return None;
    }
    Some(QueryColumn {
        name: name.to_string(),
        ty: ty.to_string(),
    })
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Split a record-literal body on commas while ignoring commas nested
/// inside `[]` or `()` so that types like `Result[A, B]` survive intact.
fn split_top_level_commas(inner: &str) -> Vec<&str> {
    let mut depth = 0i32;
    let mut last = 0usize;
    let mut out = Vec::new();
    for (idx, ch) in inner.char_indices() {
        match ch {
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' => depth -= 1,
            ',' if depth == 0 => {
                let segment = inner[last..idx].trim();
                if !segment.is_empty() {
                    out.push(segment);
                }
                last = idx + 1;
            }
            _ => {}
        }
    }
    let tail = inner[last..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

/// Walk every `SymbolKind::Query` in `module` and emit `Q0010`/`Q0020`
/// diagnostics for shape problems. Output is sorted (by symbol id) so it
/// is byte-stable for golden tests and agent caches.
pub fn check_module_queries(module: &Module) -> Vec<Diagnostic> {
    let declared_types: BTreeSet<String> = module
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Type)
        .map(|s| s.name.clone())
        .collect();
    let permitted_generics: BTreeSet<&str> = PERMITTED_GENERICS.iter().copied().collect();

    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    // Track shapes seen per query name in source order so we can detect
    // mismatching duplicates deterministically.
    let mut by_name: BTreeMap<String, Vec<(&crate::ast::Symbol, QueryShape)>> = BTreeMap::new();

    for symbol in module
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Query)
    {
        let shape = match extract_query_shape(&symbol.signature) {
            Some(shape) => shape,
            None => continue,
        };

        // Q0010 — unknown column type.
        for column in &shape.columns {
            let head = strip_generic_head(&column.ty);
            let starts_uppercase = head
                .chars()
                .next()
                .map(|c| c.is_ascii_uppercase())
                .unwrap_or(false);
            if !starts_uppercase {
                continue;
            }
            if permitted_generics.contains(head)
                || is_builtin_type(head)
                || declared_types.contains(head)
            {
                continue;
            }
            diagnostics.push(
                Diagnostic::warning(
                    "Q0010",
                    format!(
                        "query `{}` column `{}` references unknown type `{}`",
                        symbol.name, column.name, head
                    ),
                    symbol.span.clone(),
                )
                .with_symbol(symbol.id.clone())
                .with_expected(vec![format!(
                    "a builtin, a type declared in `{}`, or a permitted generic ({})",
                    module.name,
                    PERMITTED_GENERICS.join(", ")
                )])
                .with_found(vec![head.to_string()])
                .with_agent_summary(
                    "Declare the referenced type or import it before using it in a query column.",
                )
                .with_docs(vec!["doc:db.queries".to_string()]),
            );
        }

        by_name
            .entry(symbol.name.clone())
            .or_default()
            .push((symbol, shape));
    }

    // Q0020 — duplicate query names with mismatched shapes.
    for (name, instances) in &by_name {
        if instances.len() < 2 {
            continue;
        }
        let (first_symbol, first_shape) = &instances[0];
        for (other_symbol, other_shape) in &instances[1..] {
            if other_shape == first_shape {
                continue;
            }
            diagnostics.push(
                Diagnostic::error(
                    "Q0020",
                    format!(
                        "query `{}` is declared with a different shape than its earlier declaration",
                        name
                    ),
                    other_symbol.span.clone(),
                )
                .with_symbol(other_symbol.id.clone())
                .with_expected(vec![render_shape(first_shape)])
                .with_found(vec![render_shape(other_shape)])
                .with_agent_summary(
                    "Pick one canonical row shape for this query and update every declaration to match.",
                )
                .with_docs(vec!["doc:db.queries.shape".to_string()])
                .with_minimal_context(vec![first_symbol.id.clone(), other_symbol.id.clone()]),
            );
        }
    }

    diagnostics.sort_by(|a, b| {
        a.id.cmp(&b.id)
            .then_with(|| a.span.start.line.cmp(&b.span.start.line))
            .then_with(|| a.message.cmp(&b.message))
    });
    diagnostics
}

fn strip_generic_head(ty: &str) -> &str {
    let ty = ty.trim();
    match ty.find('[') {
        Some(open) => ty[..open].trim(),
        None => ty,
    }
}

fn render_shape(shape: &QueryShape) -> String {
    let mut out = String::from("{");
    for (idx, column) in shape.columns.iter().enumerate() {
        if idx > 0 {
            out.push_str(", ");
        }
        out.push_str(&column.name);
        out.push_str(": ");
        out.push_str(&column.ty);
    }
    out.push('}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_source;
    use crate::source::SourceFile;

    fn parse(text: &str) -> Module {
        parse_source(&SourceFile::new("/q.ori", text)).module
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn parses_single_column_query() {
        let shape = extract_query_shape("query find_user(id: UserId) -> {id: UserId}");
        match shape {
            Some(shape) => {
                if shape.columns.len() != 1 {
                    assert!(false, "expected 1 column, got {}", shape.columns.len());
                }
                let column = &shape.columns[0];
                if column.name != "id" || column.ty != "UserId" {
                    assert!(false, "unexpected column: {column:?}");
                }
            }
            None => assert!(false, "expected a shape, got None"),
        }
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn parses_multi_column_query() {
        let shape =
            extract_query_shape("query find_user(id: UserId) -> {id: UserId, name: Str, age: Int}");
        match shape {
            Some(shape) => {
                if shape.columns.len() != 3 {
                    assert!(false, "expected 3 columns, got {}", shape.columns.len());
                }
                let names: Vec<&str> = shape.columns.iter().map(|c| c.name.as_str()).collect();
                if names != vec!["id", "name", "age"] {
                    assert!(false, "unexpected column order: {names:?}");
                }
            }
            None => assert!(false, "expected a shape, got None"),
        }
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn parses_nested_generic_column_type() {
        let shape = extract_query_shape(
            "query list_active(scope: Scope) -> {rows: List[Product], error: Option[Str]}",
        );
        match shape {
            Some(shape) => {
                if shape.columns.len() != 2 {
                    assert!(false, "expected 2 columns, got {}", shape.columns.len());
                }
                if shape.columns[0].ty != "List[Product]" {
                    assert!(false, "first column type was {}", shape.columns[0].ty);
                }
                if shape.columns[1].ty != "Option[Str]" {
                    assert!(false, "second column type was {}", shape.columns[1].ty);
                }
            }
            None => assert!(false, "expected a shape, got None"),
        }
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn returns_none_without_record_return_type() {
        if extract_query_shape("query find_user(id: UserId) -> User").is_some() {
            assert!(false, "non-record return should yield None");
        }
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn returns_empty_shape_for_empty_record() {
        let shape = extract_query_shape("query ping() -> {}");
        match shape {
            Some(shape) => {
                if !shape.columns.is_empty() {
                    assert!(false, "expected empty columns, got {:?}", shape.columns);
                }
            }
            None => assert!(false, "expected Some(empty shape)"),
        }
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn check_flags_unknown_column_type_q0010() {
        let module = parse(
            "module catalog\nquery find_thing(id: ProductId) -> {id: ProductId, nope: Mystery}\n",
        );
        let diags = check_module_queries(&module);
        if !diags.iter().any(|d| d.id == "Q0010") {
            assert!(false, "expected Q0010 to fire, got {diags:?}");
        }
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn check_accepts_builtin_and_declared_types() {
        let module = parse(
            "module catalog\ntype ProductId\nquery find_thing(id: ProductId) -> {id: ProductId, name: Str}\n",
        );
        let diags = check_module_queries(&module);
        if diags.iter().any(|d| d.id == "Q0010") {
            assert!(false, "did not expect Q0010, got {diags:?}");
        }
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn check_flags_duplicate_query_with_different_shape_q0020() {
        // The bootstrap parser dedups by `(module, name)` so two identically-named
        // queries cannot both reach the SQL checker through the surface parser
        // — only one survives. We construct the module by hand so the SQL
        // checker's Q0020 path is exercised in the cross-module case.
        use crate::ast::{Module, Symbol, SymbolKind};
        use crate::source::Span;
        let mut module = Module::new("catalog", "/q.ori");
        module.symbols.push(Symbol {
            id: "sym:catalog.UserId".to_string(),
            name: "UserId".to_string(),
            kind: SymbolKind::Type,
            signature: "type UserId".to_string(),
            effects: Vec::new(),
            span: Span::dummy("/q.ori".to_string()),
        });
        module.symbols.push(Symbol {
            id: "sym:catalog.find_user#a".to_string(),
            name: "find_user".to_string(),
            kind: SymbolKind::Query,
            signature: "query find_user(id: UserId) -> {id: UserId, name: Str}".to_string(),
            effects: Vec::new(),
            span: Span::dummy("/q.ori".to_string()),
        });
        module.symbols.push(Symbol {
            id: "sym:catalog.find_user#b".to_string(),
            name: "find_user".to_string(),
            kind: SymbolKind::Query,
            signature: "query find_user(id: UserId) -> {id: UserId, email: Str}".to_string(),
            effects: Vec::new(),
            span: Span::dummy("/q.ori".to_string()),
        });
        let diags = check_module_queries(&module);
        if !diags.iter().any(|d| d.id == "Q0020") {
            assert!(false, "expected Q0020 to fire, got {diags:?}");
        }
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn check_accepts_duplicate_query_with_matching_shape() {
        let module = parse(
            "module catalog\nquery find_user(id: UserId) -> {id: UserId, name: Str}\nquery find_user(id: UserId) -> {id: UserId, name: Str}\n",
        );
        let diags = check_module_queries(&module);
        if diags.iter().any(|d| d.id == "Q0020") {
            assert!(
                false,
                "did not expect Q0020 for matching shapes, got {diags:?}"
            );
        }
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn check_is_deterministic_for_repeated_runs() {
        let module = parse(
            "module catalog\nquery a(id: UserId) -> {id: UserId, nope: Mystery}\nquery b(id: UserId) -> {id: UserId, alsoNope: Bogus}\n",
        );
        let first = check_module_queries(&module);
        let second = check_module_queries(&module);
        if first != second {
            assert!(false, "diagnostics were not deterministic between runs");
        }
    }
}
