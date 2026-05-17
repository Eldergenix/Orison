//! Name resolution and module graph for a set of `Module` values.
//!
//! The bootstrap resolver works at the module-level: it walks every module
//! produced by [`parse_source`](crate::parser::parse_source), interns symbol
//! IDs across modules, tracks namespace separation (type vs. value vs.
//! protocol vs. service), and computes the dependency graph implied by
//! `import` declarations. Cycle detection runs Tarjan-flavoured DFS and
//! produces actionable diagnostics that name every module in the cycle.

use crate::ast::{Module, Symbol, SymbolKind};
use crate::diagnostic::Diagnostic;
use crate::source::Span;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Namespace each symbol resolves into. Two symbols may share a name as
/// long as they live in different namespaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Namespace {
    /// Value bindings: functions, queries, capabilities, migrations.
    Value,
    /// Type names.
    Type,
    /// Protocol declarations.
    Protocol,
    /// Service- or actor-like declarations.
    Service,
}

impl Namespace {
    /// Map a [`SymbolKind`] to the namespace it resolves into.
    pub fn of(kind: SymbolKind) -> Self {
        match kind {
            SymbolKind::Function | SymbolKind::Query | SymbolKind::Capability => Namespace::Value,
            SymbolKind::Type => Namespace::Type,
            SymbolKind::Actor | SymbolKind::Service => Namespace::Service,
            SymbolKind::View => Namespace::Service,
            SymbolKind::Migration => Namespace::Value,
            SymbolKind::Module | SymbolKind::Unknown => Namespace::Value,
        }
    }
}

/// One resolved symbol entry in the global table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedSymbol {
    /// Canonical symbol id.
    pub id: String,
    /// Owning module name.
    pub module: String,
    /// Bare symbol name.
    pub name: String,
    /// Symbol kind.
    pub kind: SymbolKind,
    /// Namespace the symbol resolves into.
    pub namespace: Namespace,
}

/// Module-level dependency graph implied by `import` declarations.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModuleGraph {
    /// For every module, the set of modules it imports (by name).
    pub edges: BTreeMap<String, BTreeSet<String>>,
}

impl ModuleGraph {
    /// Iterate every module name appearing as a node in the graph.
    pub fn nodes(&self) -> impl Iterator<Item = &String> {
        self.edges.keys()
    }
}

/// Output of [`resolve`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Resolution {
    /// Full symbol table keyed by canonical id.
    pub symbols: BTreeMap<String, ResolvedSymbol>,
    /// Module dependency graph.
    pub graph: ModuleGraph,
    /// Diagnostics produced while resolving.
    pub diagnostics: Vec<Diagnostic>,
}

/// Resolve names across `modules`, building the global symbol table and the
/// module dependency graph. Diagnostics in the returned [`Resolution`]
/// describe duplicate declarations, missing imports, and import cycles.
pub fn resolve(modules: &[Module]) -> Resolution {
    let mut resolution = Resolution::default();
    let mut by_namespace: BTreeMap<(String, Namespace, String), &Symbol> = BTreeMap::new();

    for module in modules {
        let imports: BTreeSet<String> = module.imports.iter().cloned().collect();
        resolution
            .graph
            .edges
            .entry(module.name.clone())
            .or_default()
            .extend(imports);

        for symbol in &module.symbols {
            if symbol.kind == SymbolKind::Module {
                continue;
            }
            let ns = Namespace::of(symbol.kind);
            let key = (module.name.clone(), ns, symbol.name.clone());
            if let Some(prev) = by_namespace.get(&key) {
                resolution
                    .diagnostics
                    .push(duplicate_diagnostic(module, symbol, prev));
                continue;
            }
            by_namespace.insert(key, symbol);
            resolution.symbols.insert(
                symbol.id.clone(),
                ResolvedSymbol {
                    id: symbol.id.clone(),
                    module: module.name.clone(),
                    name: symbol.name.clone(),
                    kind: symbol.kind,
                    namespace: ns,
                },
            );
        }
    }

    let known_modules: BTreeSet<String> = modules.iter().map(|m| m.name.clone()).collect();
    for module in modules {
        for import in &module.imports {
            if import.starts_with("core.")
                || import.starts_with("std.")
                || import.starts_with("app.")
            {
                // Standard distribution imports are always permitted; the
                // resolver simply records them in the graph.
                continue;
            }
            if !known_modules.contains(import) {
                resolution
                    .diagnostics
                    .push(unresolved_import_diagnostic(module, import));
            }
        }
    }

    for cycle in detect_cycles(&resolution.graph) {
        resolution.diagnostics.push(cycle_diagnostic(&cycle));
    }

    resolution
}

fn duplicate_diagnostic(module: &Module, current: &Symbol, previous: &Symbol) -> Diagnostic {
    Diagnostic::error(
        "E0211",
        format!(
            "duplicate `{}` declarations in module `{}`",
            current.name, module.name
        ),
        current.span.clone(),
    )
    .with_symbol(current.id.clone())
    .with_expected(vec![format!("single declaration of `{}`", current.name)])
    .with_found(vec![previous.id.clone(), current.id.clone()])
    .with_agent_summary("Rename or remove one of the duplicate declarations.")
    .with_minimal_context(vec![previous.id.clone(), current.id.clone()])
    .with_docs(vec!["doc:names.duplicates".to_string()])
}

fn unresolved_import_diagnostic(module: &Module, import: &str) -> Diagnostic {
    Diagnostic::error(
        "E0220",
        format!(
            "import `{}` does not resolve to a known module (from `{}`)",
            import, module.name
        ),
        Span::dummy(module.path.clone()),
    )
    .with_expected(vec![format!("module `{}`", import)])
    .with_found(vec![import.to_string()])
    .with_agent_summary("Add the missing module or remove the import.")
    .with_minimal_context(vec![module.name.clone(), import.to_string()])
    .with_docs(vec!["doc:modules.resolution".to_string()])
}

fn cycle_diagnostic(cycle: &[String]) -> Diagnostic {
    let chain = cycle.join(" -> ");
    Diagnostic::error(
        "E0230",
        format!("module import cycle detected: {chain}"),
        Span::dummy(format!(
            "{}.ori",
            cycle.first().cloned().unwrap_or_default()
        )),
    )
    .with_expected(vec!["acyclic module graph".to_string()])
    .with_found(vec![chain.clone()])
    .with_agent_summary("Break the cycle by extracting shared types into a leaf module.")
    .with_minimal_context(cycle.to_vec())
    .with_docs(vec!["doc:modules.cycles".to_string()])
}

fn detect_cycles(graph: &ModuleGraph) -> Vec<Vec<String>> {
    let mut on_stack: BTreeSet<String> = BTreeSet::new();
    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut stack: Vec<String> = Vec::new();
    let mut cycles: Vec<Vec<String>> = Vec::new();
    for start in graph.edges.keys() {
        if visited.contains(start) {
            continue;
        }
        dfs(
            graph,
            start,
            &mut on_stack,
            &mut visited,
            &mut stack,
            &mut cycles,
        );
    }
    cycles
}

fn dfs(
    graph: &ModuleGraph,
    node: &str,
    on_stack: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
    stack: &mut Vec<String>,
    cycles: &mut Vec<Vec<String>>,
) {
    visited.insert(node.to_string());
    on_stack.insert(node.to_string());
    stack.push(node.to_string());
    if let Some(neighbours) = graph.edges.get(node) {
        for neighbour in neighbours {
            if !graph.edges.contains_key(neighbour) {
                continue;
            }
            if on_stack.contains(neighbour) {
                if let Some(start_idx) = stack.iter().position(|n| n == neighbour) {
                    let mut cycle = stack[start_idx..].to_vec();
                    cycle.push(neighbour.to_string());
                    cycles.push(cycle);
                }
            } else if !visited.contains(neighbour) {
                dfs(graph, neighbour, on_stack, visited, stack, cycles);
            }
        }
    }
    on_stack.remove(node);
    stack.pop();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceFile;

    fn module_of(text: &str, path: &str) -> Module {
        let parse = crate::parser::parse_source(&SourceFile::new(path, text));
        parse.module
    }

    #[test]
    fn detects_duplicate_function_across_distinct_ids() {
        // Construct a module that contains two functions sharing the same
        // (namespace, name) but with distinct ids — this is what happens
        // when a resolver pass merges symbol tables from multiple frontends
        // (e.g. parser dedup happens per-file but the resolver also runs
        // across the package). The resolver must catch this.
        use crate::source::Span;
        let mut module = Module::new("demo", "/x.ori");
        module.symbols.push(Symbol {
            id: "sym:demo.f#0".to_string(),
            name: "f".to_string(),
            kind: SymbolKind::Function,
            signature: "fn f() -> Unit".to_string(),
            effects: Vec::new(),
            span: Span::dummy("/x.ori".to_string()),
        });
        module.symbols.push(Symbol {
            id: "sym:demo.f#1".to_string(),
            name: "f".to_string(),
            kind: SymbolKind::Function,
            signature: "fn f() -> Int".to_string(),
            effects: Vec::new(),
            span: Span::dummy("/x.ori".to_string()),
        });
        let res = resolve(&[module]);
        assert!(res.diagnostics.iter().any(|d| d.id == "E0211"));
    }

    #[test]
    fn flags_unresolved_import() {
        let module = module_of("module demo\nimport demo.unknown", "/x.ori");
        let res = resolve(&[module]);
        assert!(res.diagnostics.iter().any(|d| d.id == "E0220"));
    }

    #[test]
    fn allows_standard_distribution_imports() {
        let module = module_of("module demo\nimport std.json", "/x.ori");
        let res = resolve(&[module]);
        assert!(!res.diagnostics.iter().any(|d| d.id == "E0220"));
    }

    #[test]
    fn detects_module_import_cycle() {
        let m_a = module_of("module a\nimport b", "/a.ori");
        let m_b = module_of("module b\nimport a", "/b.ori");
        let res = resolve(&[m_a, m_b]);
        assert!(res.diagnostics.iter().any(|d| d.id == "E0230"));
    }

    #[test]
    fn allows_value_and_type_with_same_name() {
        // The bootstrap kinds map function -> value, type -> type, so the
        // same simple identifier may appear in two distinct namespaces.
        let module = module_of("module demo\nfn Foo() -> Unit\ntype Foo", "/x.ori");
        let res = resolve(&[module]);
        assert!(!res.diagnostics.iter().any(|d| d.id == "E0211"));
    }
}
