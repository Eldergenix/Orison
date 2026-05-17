//! Migration graph extraction, topological ordering, and JSON reporting.
//!
//! Each `migration` declaration in an Orison module turns into a
//! [`Migration`] node with an `id` (the symbol name) and an optional list
//! of `depends_on` ids parsed from the signature line. The bootstrap
//! surface uses an `after <id>` clause:
//!
//! ```text
//! migration add_product_search after init_products:
//!   up "..."
//!   down "..."
//! ```
//!
//! [`topological_order`] returns a deterministic apply order — ties broken
//! by id — and reports cycles explicitly via [`MigrationError`] so the CLI
//! can render them without panicking. A [`MigrationReport`] bundles the
//! result for the `ori db check --json` contract.

use crate::ast::{Module, SymbolKind};
use crate::json::to_json;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// A single migration node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Migration {
    pub id: String,
    pub depends_on: Vec<String>,
}

/// Possible failures during topological ordering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum MigrationError {
    /// At least one cycle exists in the dependency graph. The payload lists
    /// the migration ids that are part of the offending cycle(s), sorted
    /// for deterministic output.
    Cycle(Vec<String>),
}

impl MigrationError {
    pub fn as_str(&self) -> &'static str {
        match self {
            MigrationError::Cycle(_) => "cycle",
        }
    }
}

/// JSON-serialisable bundle returned by the `ori db check` contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationReport {
    pub schema: &'static str,
    pub ordered: Vec<String>,
    pub cycles: Vec<Vec<String>>,
}

impl MigrationReport {
    pub const SCHEMA: &'static str = "ori.migration_graph.v1";

    pub fn empty() -> Self {
        Self {
            schema: Self::SCHEMA,
            ordered: Vec::new(),
            cycles: Vec::new(),
        }
    }

    pub fn to_json(&self) -> String {
        to_json(self)
    }
}

/// Extract every `migration` symbol from `module` into a [`Migration`].
///
/// The returned list is sorted by id so two runs produce identical output.
/// `depends_on` is parsed from the signature: every token immediately
/// following an `after` keyword (one or more, comma-separated) becomes a
/// dependency. Unknown clauses are silently ignored — the bootstrap
/// surface is permissive on purpose.
pub fn extract_migrations(module: &Module) -> Vec<Migration> {
    let mut out: Vec<Migration> = module
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Migration)
        .map(|symbol| Migration {
            id: symbol.name.clone(),
            depends_on: parse_depends_on(&symbol.signature),
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn parse_depends_on(signature: &str) -> Vec<String> {
    let mut deps: Vec<String> = Vec::new();
    let tokens: Vec<&str> = tokenize_signature(signature);
    let mut idx = 0usize;
    while idx < tokens.len() {
        if tokens[idx] == "after" {
            idx += 1;
            let mut expect_id = true;
            while idx < tokens.len() {
                let token = tokens[idx];
                if token == "," {
                    expect_id = true;
                    idx += 1;
                    continue;
                }
                if !is_dep_identifier(token) {
                    break;
                }
                if expect_id {
                    deps.push(token.to_string());
                    expect_id = false;
                }
                idx += 1;
            }
            continue;
        }
        idx += 1;
    }
    deps
}

fn tokenize_signature(signature: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    let bytes = signature.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        if ch == ',' {
            out.push(",");
            i += 1;
            continue;
        }
        if ch.is_ascii_alphanumeric() || ch == '_' {
            let start = i;
            while i < bytes.len() {
                let c = bytes[i] as char;
                if c.is_ascii_alphanumeric() || c == '_' {
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(&signature[start..i]);
            continue;
        }
        // skip any other punctuation deterministically
        i += 1;
    }
    out
}

fn is_dep_identifier(token: &str) -> bool {
    let mut chars = token.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Kahn-style topological sort with deterministic tie-breaking (lowest
/// id first). Missing dependencies are tolerated — they simply don't
/// constrain the order — so a partial module still produces a usable
/// plan for the agent.
pub fn topological_order(migrations: &[Migration]) -> Result<Vec<String>, MigrationError> {
    let ids: BTreeSet<String> = migrations.iter().map(|m| m.id.clone()).collect();
    let mut in_degree: BTreeMap<String, usize> = BTreeMap::new();
    let mut outgoing: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for migration in migrations {
        in_degree.entry(migration.id.clone()).or_insert(0);
        for dep in &migration.depends_on {
            if !ids.contains(dep) {
                // Dependency on something we don't know — record only as a
                // permissive no-op so the order remains deterministic.
                continue;
            }
            if dep == &migration.id {
                // Self-loop counts as a 1-element cycle.
                continue;
            }
            let inserted = outgoing
                .entry(dep.clone())
                .or_default()
                .insert(migration.id.clone());
            if inserted {
                *in_degree.entry(migration.id.clone()).or_insert(0) += 1;
            }
        }
    }

    // Detect self-loops up front so a single-node cycle is reported.
    let mut self_loops: BTreeSet<String> = BTreeSet::new();
    for migration in migrations {
        if migration.depends_on.iter().any(|dep| dep == &migration.id) {
            self_loops.insert(migration.id.clone());
        }
    }

    let mut ready: BTreeSet<String> = in_degree
        .iter()
        .filter(|(id, deg)| **deg == 0 && !self_loops.contains(*id))
        .map(|(id, _)| id.clone())
        .collect();

    let mut ordered: Vec<String> = Vec::with_capacity(migrations.len());
    while let Some(next) = pop_first(&mut ready) {
        ordered.push(next.clone());
        if let Some(children) = outgoing.remove(&next) {
            for child in children {
                if let Some(entry) = in_degree.get_mut(&child) {
                    if *entry > 0 {
                        *entry -= 1;
                    }
                    if *entry == 0 && !self_loops.contains(&child) {
                        ready.insert(child);
                    }
                }
            }
        }
    }

    if ordered.len() == migrations.len() && self_loops.is_empty() {
        return Ok(ordered);
    }

    let mut offenders: BTreeSet<String> = self_loops.clone();
    for id in ids {
        if !ordered.contains(&id) {
            offenders.insert(id);
        }
    }
    Err(MigrationError::Cycle(offenders.into_iter().collect()))
}

fn pop_first(set: &mut BTreeSet<String>) -> Option<String> {
    let first = set.iter().next().cloned()?;
    set.remove(&first);
    Some(first)
}

/// Build the deterministic report contract for `ori db check`.
pub fn build_migration_report(module: &Module) -> MigrationReport {
    let migrations = extract_migrations(module);
    match topological_order(&migrations) {
        Ok(ordered) => MigrationReport {
            schema: MigrationReport::SCHEMA,
            ordered,
            cycles: Vec::new(),
        },
        Err(MigrationError::Cycle(ids)) => MigrationReport {
            schema: MigrationReport::SCHEMA,
            ordered: Vec::new(),
            cycles: vec![ids],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_source;
    use crate::source::SourceFile;

    fn parse(text: &str) -> Module {
        parse_source(&SourceFile::new("/m.ori", text)).module
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn extracts_single_migration() {
        let module = parse("module a\nmigration init_products:\n  up \"X\"\n  down \"Y\"\n");
        let migs = extract_migrations(&module);
        if migs.len() != 1 {
            assert!(false, "expected 1 migration, got {}", migs.len());
        }
        if migs[0].id != "init_products" {
            assert!(false, "wrong id: {}", migs[0].id);
        }
        if !migs[0].depends_on.is_empty() {
            assert!(false, "expected no deps, got {:?}", migs[0].depends_on);
        }
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn extracts_after_dependency_from_signature() {
        let module = parse(
            "module a\nmigration init_products:\n  up \"X\"\n  down \"Y\"\nmigration add_index after init_products:\n  up \"A\"\n  down \"B\"\n",
        );
        let migs = extract_migrations(&module);
        if migs.len() != 2 {
            assert!(false, "expected 2 migrations, got {}", migs.len());
        }
        let add_index = migs
            .iter()
            .find(|m| m.id == "add_index")
            .cloned()
            .unwrap_or(Migration {
                id: String::new(),
                depends_on: Vec::new(),
            });
        if add_index.depends_on != vec!["init_products".to_string()] {
            assert!(
                false,
                "expected dep on init_products, got {:?}",
                add_index.depends_on
            );
        }
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn topological_order_orders_dependencies_first() {
        let migs = vec![
            Migration {
                id: "b".to_string(),
                depends_on: vec!["a".to_string()],
            },
            Migration {
                id: "a".to_string(),
                depends_on: vec![],
            },
        ];
        match topological_order(&migs) {
            Ok(order) => {
                if order != vec!["a".to_string(), "b".to_string()] {
                    assert!(false, "wrong order: {order:?}");
                }
            }
            Err(err) => assert!(false, "expected ok, got {err:?}"),
        }
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn topological_order_breaks_ties_alphabetically() {
        let migs = vec![
            Migration {
                id: "z".to_string(),
                depends_on: vec![],
            },
            Migration {
                id: "a".to_string(),
                depends_on: vec![],
            },
            Migration {
                id: "m".to_string(),
                depends_on: vec![],
            },
        ];
        match topological_order(&migs) {
            Ok(order) => {
                if order != vec!["a".to_string(), "m".to_string(), "z".to_string()] {
                    assert!(false, "expected sorted order, got {order:?}");
                }
            }
            Err(err) => assert!(false, "expected ok, got {err:?}"),
        }
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn topological_order_detects_two_node_cycle() {
        let migs = vec![
            Migration {
                id: "a".to_string(),
                depends_on: vec!["b".to_string()],
            },
            Migration {
                id: "b".to_string(),
                depends_on: vec!["a".to_string()],
            },
        ];
        match topological_order(&migs) {
            Ok(order) => assert!(false, "expected cycle, got ok({order:?})"),
            Err(MigrationError::Cycle(ids)) => {
                if ids != vec!["a".to_string(), "b".to_string()] {
                    assert!(false, "unexpected cycle ids: {ids:?}");
                }
            }
        }
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn topological_order_detects_self_loop() {
        let migs = vec![Migration {
            id: "loopy".to_string(),
            depends_on: vec!["loopy".to_string()],
        }];
        match topological_order(&migs) {
            Ok(order) => assert!(false, "expected cycle, got ok({order:?})"),
            Err(MigrationError::Cycle(ids)) => {
                if ids != vec!["loopy".to_string()] {
                    assert!(false, "unexpected cycle ids: {ids:?}");
                }
            }
        }
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn build_migration_report_uses_schema_constant() {
        let module = parse(
            "module a\nmigration init_products:\n  up \"X\"\n  down \"Y\"\nmigration add_index after init_products:\n  up \"A\"\n  down \"B\"\n",
        );
        let report = build_migration_report(&module);
        if report.schema != MigrationReport::SCHEMA {
            assert!(false, "schema mismatch: {}", report.schema);
        }
        if report.ordered != vec!["init_products".to_string(), "add_index".to_string()] {
            assert!(false, "unexpected ordered: {:?}", report.ordered);
        }
        if !report.cycles.is_empty() {
            assert!(false, "expected no cycles, got {:?}", report.cycles);
        }
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn missing_dependency_is_tolerated() {
        let migs = vec![Migration {
            id: "only".to_string(),
            depends_on: vec!["ghost".to_string()],
        }];
        match topological_order(&migs) {
            Ok(order) => {
                if order != vec!["only".to_string()] {
                    assert!(false, "expected only, got {order:?}");
                }
            }
            Err(err) => assert!(false, "expected ok, got {err:?}"),
        }
    }
}
