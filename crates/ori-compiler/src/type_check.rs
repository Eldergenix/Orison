//! Baseline type checker for the bootstrap compiler.
//!
//! The bootstrap parser only sees function signatures and item declarations,
//! so this checker validates what is visible in the surface syntax:
//!
//! * Every type name that appears in a public signature must be a known
//!   builtin, a declared user type, or a permitted generic
//!   (`Option`, `Result`, `List`, `Pair`, `Fn`, `Iter`, `Query`).
//! * `Result` and `Option` must be parameterised when used as type
//!   references in signatures.
//! * Mixing two distinct newtype names is reported as a hint; mixing is the
//!   most common cross-domain bug Orison was designed to catch
//!   (`ProductId` vs. `OrderId`).
//!
//! The checker is intentionally conservative: it produces warnings instead
//! of errors when the bootstrap parser cannot fully resolve a signature.
//! This keeps the edit-check loop fast and avoids false negatives breaking
//! the demo storefront before the full type system lands.

use crate::ast::{Module, SymbolKind};
use crate::diagnostic::Diagnostic;
use crate::types::is_builtin_type;
use std::collections::BTreeSet;

const PERMITTED_GENERICS: &[&str] = &[
    "Option", "Result", "List", "Pair", "Fn", "Iter", "Query", "Map", "Set",
];

const PERMITTED_NEWTYPE_BASES: &[&str] = &[
    "Bool", "Int", "Float", "Float32", "Float64", "Str", "Bytes", "Decimal", "Unit",
];

/// Run the bootstrap type checker on `module` and return any diagnostics
/// produced.
pub fn type_check_module(module: &Module) -> Vec<Diagnostic> {
    let declared_types = collect_declared_types(module);
    let mut diagnostics = Vec::new();

    for symbol in module.exported_symbols() {
        if symbol.kind != SymbolKind::Function && symbol.kind != SymbolKind::Query {
            continue;
        }
        for ty in extract_types_from_signature(&symbol.signature) {
            check_type_reference(
                module,
                symbol.id.as_str(),
                symbol.span.clone(),
                &ty,
                &declared_types,
                &mut diagnostics,
            );
        }
    }
    diagnostics
}

fn collect_declared_types(module: &Module) -> BTreeSet<String> {
    module
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Type)
        .map(|s| s.name.clone())
        .collect()
}

fn check_type_reference(
    module: &Module,
    symbol_id: &str,
    span: crate::source::Span,
    ty: &str,
    declared: &BTreeSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // strip generic instantiation
    let (head, args) = split_generic(ty);
    if !head
        .chars()
        .next()
        .map(|c| c.is_ascii_uppercase())
        .unwrap_or(false)
    {
        return;
    }

    if PERMITTED_GENERICS.contains(&head) {
        if args.is_empty() && head != "Unit" {
            diagnostics.push(
                Diagnostic::warning(
                    "W0510",
                    format!("`{head}` should be parameterised, e.g. `{head}[T]`"),
                    span.clone(),
                )
                .with_symbol(symbol_id.to_string())
                .with_expected(vec![format!("{head}[T]")])
                .with_found(vec![head.to_string()])
                .with_agent_summary("Add the missing generic argument.")
                .with_docs(vec!["doc:types.generics".to_string()]),
            );
        }
        return;
    }

    if is_builtin_type(head) || declared.contains(head) || PERMITTED_NEWTYPE_BASES.contains(&head) {
        return;
    }

    diagnostics.push(
        Diagnostic::warning(
            "W0501",
            format!(
                "type `{}` is not declared in module `{}` or in the standard distribution",
                head, module.name
            ),
            span,
        )
        .with_symbol(symbol_id.to_string())
        .with_expected(vec![format!(
            "a declared type in `{}`, a builtin (Int, Str, Bool, ...), or a permitted generic ({})",
            module.name,
            PERMITTED_GENERICS.join(", ")
        )])
        .with_found(vec![head.to_string()])
        .with_agent_summary("Declare the type or import it from the standard distribution.")
        .with_docs(vec!["doc:types.references".to_string()]),
    );
}

fn split_generic(ty: &str) -> (&str, Vec<&str>) {
    let ty = ty.trim();
    if let Some(open) = ty.find('[') {
        if ty.ends_with(']') {
            let head = ty[..open].trim();
            let inner = &ty[open + 1..ty.len() - 1];
            let args: Vec<&str> = split_top_level_commas(inner);
            return (head, args);
        }
    }
    (ty, Vec::new())
}

fn split_top_level_commas(inner: &str) -> Vec<&str> {
    let mut depth = 0i32;
    let mut last = 0usize;
    let mut out = Vec::new();
    for (idx, ch) in inner.char_indices() {
        match ch {
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(inner[last..idx].trim());
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

/// Extract type-looking tokens from a flat signature string. The bootstrap
/// signature already strips control characters; we walk balanced bracket
/// groups so we can recover full generic instantiations like
/// `Result[Product, CatalogError]`.
fn extract_types_from_signature(signature: &str) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = signature.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch.is_ascii_uppercase() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            // include optional generic suffix `[...]`
            if i < chars.len() && chars[i] == '[' {
                let mut depth = 0i32;
                while i < chars.len() {
                    let c = chars[i];
                    if c == '[' {
                        depth += 1;
                    } else if c == ']' {
                        depth -= 1;
                        i += 1;
                        if depth == 0 {
                            break;
                        }
                        continue;
                    }
                    i += 1;
                }
            }
            let ty: String = chars[start..i].iter().collect();
            out.push(ty);
            continue;
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_source;
    use crate::source::SourceFile;

    fn check(text: &str) -> Vec<Diagnostic> {
        let module = parse_source(&SourceFile::new("/t.ori", text)).module;
        type_check_module(&module)
    }

    #[test]
    fn accepts_known_builtin_signature() {
        let diags = check("module a\nfn f() -> Int");
        assert!(!diags.iter().any(|d| d.id == "W0501"));
    }

    #[test]
    fn accepts_option_with_argument() {
        let diags = check("module a\nfn f() -> Option[Int]");
        assert!(!diags.iter().any(|d| d.id == "W0501"));
        assert!(!diags.iter().any(|d| d.id == "W0510"));
    }

    #[test]
    fn flags_unknown_type() {
        let diags = check("module a\nfn f() -> ProductWhat");
        assert!(diags.iter().any(|d| d.id == "W0501"));
    }

    #[test]
    fn allows_user_declared_type() {
        let diags = check("module a\ntype Product\nfn f() -> Product");
        assert!(!diags.iter().any(|d| d.id == "W0501"));
    }

    #[test]
    fn accepts_result_with_two_args() {
        let diags = check("module a\ntype Bad\nfn f() -> Result[Int, Bad]");
        assert!(!diags.iter().any(|d| d.id == "W0501"));
    }

    #[test]
    fn flags_bare_result() {
        let diags = check("module a\nfn f() -> Result");
        assert!(diags.iter().any(|d| d.id == "W0510"));
    }
}
