//! Documentation generation for Orison modules.
//!
//! The bootstrap supports two flavors:
//!
//! * [`generate_human_docs`] emits human-readable Markdown for browsing or
//!   embedding into project documentation. Each module gets a heading, an
//!   imports section, and one subsection per exported symbol with its
//!   signature, declared effects, and any well-known invariants. Cross
//!   references between symbols use the form `[#sym-id]` so they remain
//!   stable across renderers.
//!
//! * [`generate_agent_docs`] emits a budget-aware, compact Markdown view
//!   tuned for placing into LLM context windows. The output prioritises
//!   short symbol listings (`- kind name :: signature [effects ...]`) and
//!   appends a stable `[truncated]` marker once the byte budget is
//!   exceeded so downstream tooling can detect partial outputs.
//!
//! Output ordering is deterministic: modules are emitted in the order
//! supplied by the caller, but exported symbols inside each module are
//! sorted by stable identifier so that two runs over the same input always
//! produce byte-identical Markdown.

use crate::ast::{Module, Symbol, SymbolKind};

/// Stable suffix appended to agent docs when the byte budget is reached.
pub const TRUNCATION_MARKER: &str = "\n\n[truncated]\n";

/// Bootstrap invariants documented alongside every module.
///
/// These mirror the capsule contract so human and agent readers see the
/// same baseline guarantees the rest of the toolchain enforces.
const BASELINE_INVARIANTS: &[&str] = &[
    "No null values; use Option[T].",
    "No exceptions; use Result[T, E].",
];

/// Generate the human-oriented Markdown documentation for `modules`.
///
/// The output is deterministic for a given input: modules are emitted in
/// caller order; symbols inside a module are sorted by `id`.
pub fn generate_human_docs(modules: &[Module]) -> String {
    let mut out = String::new();
    out.push_str("# Orison module reference\n\n");
    out.push_str(&format!(
        "Generated reference for {} module(s).\n\n",
        modules.len()
    ));

    for module in modules {
        out.push_str(&format!("## Module `{}`\n\n", module.name));
        out.push_str(&format!("- Source: `{}`\n", module.path));
        out.push_str(&format!("- Symbol id: `[#mod:{}]`\n", module.name));

        if module.imports.is_empty() {
            out.push_str("- Imports: _none_\n\n");
        } else {
            out.push_str("- Imports:\n");
            let mut imports = module.imports.clone();
            imports.sort();
            for import in &imports {
                out.push_str(&format!("  - `{import}`\n"));
            }
            out.push('\n');
        }

        let mut exported: Vec<&Symbol> = module.exported_symbols().collect();
        exported.sort_by(|a, b| a.id.cmp(&b.id));

        if exported.is_empty() {
            out.push_str("_No exported symbols._\n\n");
        } else {
            out.push_str("### Exported symbols\n\n");
            for symbol in &exported {
                push_human_symbol(&mut out, symbol);
            }
        }

        out.push_str("### Invariants\n\n");
        for invariant in BASELINE_INVARIANTS {
            out.push_str(&format!("- {invariant}\n"));
        }
        out.push('\n');

        let dep_refs = symbol_dependency_refs(&exported);
        if !dep_refs.is_empty() {
            out.push_str("### Cross references\n\n");
            for reference in dep_refs {
                out.push_str(&format!("- {reference}\n"));
            }
            out.push('\n');
        }
    }

    out
}

/// Generate the agent-oriented compact Markdown documentation.
///
/// The output is truncated at `budget` bytes (UTF-8 codepoint aligned)
/// and ends with [`TRUNCATION_MARKER`] when truncation happens. A budget
/// of zero is treated as "produce only the truncation marker" so callers
/// can probe minimum-size behavior safely.
pub fn generate_agent_docs(modules: &[Module], budget: usize) -> String {
    let mut full = String::new();
    full.push_str("# Orison agent reference\n\n");
    full.push_str(&format!(
        "Modules: {}; baseline invariants: {}.\n\n",
        modules.len(),
        BASELINE_INVARIANTS.len()
    ));

    for module in modules {
        full.push_str(&format!("## {} `[#mod:{}]`\n", module.name, module.name));
        if !module.imports.is_empty() {
            let mut imports = module.imports.clone();
            imports.sort();
            full.push_str(&format!("deps: {}\n", imports.join(", ")));
        }
        let mut exported: Vec<&Symbol> = module.exported_symbols().collect();
        exported.sort_by(|a, b| a.id.cmp(&b.id));
        for symbol in &exported {
            full.push_str(&compact_symbol_line(symbol));
            full.push('\n');
        }
        full.push('\n');
    }

    truncate_with_marker(&full, budget)
}

fn push_human_symbol(out: &mut String, symbol: &Symbol) {
    out.push_str(&format!(
        "#### `{}` ({})\n\n",
        symbol.name,
        kind_label(symbol.kind)
    ));
    out.push_str(&format!("- Id: `[#{}]`\n", symbol.id));
    out.push_str(&format!("- Signature: `{}`\n", symbol.signature));
    if symbol.effects.is_empty() {
        out.push_str("- Effects: _pure_\n");
    } else {
        let mut effects = symbol.effects.clone();
        effects.sort();
        out.push_str(&format!("- Effects: `{}`\n", effects.join("`, `")));
    }
    out.push_str(&format!(
        "- Location: `{}` line {}\n\n",
        symbol.span.file, symbol.span.start.line
    ));
}

fn compact_symbol_line(symbol: &Symbol) -> String {
    let signature = collapse_whitespace(&symbol.signature);
    if symbol.effects.is_empty() {
        format!(
            "- {} {} :: {} [#{}]",
            kind_label(symbol.kind),
            symbol.name,
            signature,
            symbol.id
        )
    } else {
        let mut effects = symbol.effects.clone();
        effects.sort();
        format!(
            "- {} {} :: {} ![{}] [#{}]",
            kind_label(symbol.kind),
            symbol.name,
            signature,
            effects.join(","),
            symbol.id
        )
    }
}

fn kind_label(kind: SymbolKind) -> &'static str {
    kind.as_str()
}

fn collapse_whitespace(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_space = false;
    for ch in input.chars() {
        if ch.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(ch);
            last_space = false;
        }
    }
    out.trim().to_string()
}

/// Build a sorted list of `[#a] -> [#b]` cross references for symbols that
/// reference other exported symbols by id in their signature text.
fn symbol_dependency_refs(symbols: &[&Symbol]) -> Vec<String> {
    let mut refs: Vec<String> = Vec::new();
    let ids: Vec<&String> = symbols.iter().map(|symbol| &symbol.id).collect();
    for symbol in symbols {
        for other in &ids {
            if **other == symbol.id {
                continue;
            }
            // Match by trailing name segment, e.g. `sym:foo.bar` matches `bar`.
            if let Some(short) = other.rsplit('.').next() {
                if !short.is_empty() && symbol.signature.contains(short) {
                    refs.push(format!("`[#{}]` -> `[#{}]`", symbol.id, other));
                }
            }
        }
    }
    refs.sort();
    refs.dedup();
    refs
}

fn truncate_with_marker(text: &str, budget: usize) -> String {
    if budget == 0 {
        return TRUNCATION_MARKER.trim_start_matches('\n').to_string();
    }
    if text.len() <= budget {
        return text.to_string();
    }
    let marker = TRUNCATION_MARKER;
    let allowed = budget.saturating_sub(marker.len());
    let mut cutoff = allowed.min(text.len());
    while cutoff > 0 && !text.is_char_boundary(cutoff) {
        cutoff -= 1;
    }
    let mut out = String::with_capacity(cutoff + marker.len());
    out.push_str(&text[..cutoff]);
    out.push_str(marker);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Module, Symbol, SymbolKind};
    use crate::source::Span;

    fn sample_modules() -> Vec<Module> {
        let mut module = Module::new("store.users", "/store/users.ori");
        module.imports.push("std.json".to_string());
        module.imports.push("app.services".to_string());
        module.symbols.push(Symbol {
            id: "sym:store.users.User".to_string(),
            name: "User".to_string(),
            kind: SymbolKind::Type,
            signature: "type User = { id: UserId, name: Str }".to_string(),
            effects: Vec::new(),
            span: Span::new("/store/users.ori", 5, 1, 5, 10),
        });
        module.symbols.push(Symbol {
            id: "sym:store.users.fetch_user".to_string(),
            name: "fetch_user".to_string(),
            kind: SymbolKind::Function,
            signature: "fn fetch_user(id: UserId) -> Result[User, ApiErr]".to_string(),
            effects: vec!["db.read".to_string()],
            span: Span::new("/store/users.ori", 7, 1, 7, 12),
        });
        vec![module]
    }

    #[test]
    fn human_doc_roundtrip_contains_signature_effects_and_invariants() {
        let modules = sample_modules();
        let docs = generate_human_docs(&modules);
        assert!(docs.contains("## Module `store.users`"));
        assert!(docs.contains("Signature: `fn fetch_user(id: UserId) -> Result[User, ApiErr]`"));
        assert!(docs.contains("Effects: `db.read`"));
        assert!(docs.contains("No null values; use Option[T]."));
        assert!(docs.contains("`[#sym:store.users.fetch_user]`"));
        // Cross reference from fetch_user signature mentioning `User`.
        assert!(docs.contains("`[#sym:store.users.fetch_user]` -> `[#sym:store.users.User]`"));
    }

    #[test]
    fn human_doc_is_deterministic_across_runs() {
        let modules = sample_modules();
        let a = generate_human_docs(&modules);
        let b = generate_human_docs(&modules);
        assert_eq!(a, b);
    }

    #[test]
    fn agent_doc_truncates_with_stable_marker() {
        let modules = sample_modules();
        let docs = generate_agent_docs(&modules, 80);
        assert!(docs.ends_with(TRUNCATION_MARKER));
        assert!(docs.len() <= 80);
    }

    #[test]
    fn agent_doc_respects_zero_budget() {
        let modules = sample_modules();
        let docs = generate_agent_docs(&modules, 0);
        assert_eq!(docs, "[truncated]\n");
    }

    #[test]
    fn agent_doc_emits_full_output_when_under_budget() {
        let modules = sample_modules();
        let unbounded = generate_agent_docs(&modules, usize::MAX);
        assert!(!unbounded.ends_with(TRUNCATION_MARKER));
        assert!(unbounded.contains("fetch_user :: fn fetch_user"));
        assert!(unbounded.contains("![db.read]"));
        assert!(unbounded.contains("[#sym:store.users.fetch_user]"));
    }

    #[test]
    fn agent_doc_is_deterministic_across_runs() {
        let modules = sample_modules();
        let a = generate_agent_docs(&modules, 4096);
        let b = generate_agent_docs(&modules, 4096);
        assert_eq!(a, b);
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn truncation_marker_is_stable_constant() {
        if TRUNCATION_MARKER != "\n\n[truncated]\n" {
            assert!(
                false,
                "TRUNCATION_MARKER must remain stable for downstream consumers"
            );
        }
    }
}
