//! Per-symbol query engine for the bootstrap incremental layer (M7).
//!
//! The bootstrap incremental cache in [`crate::incremental`] works at file
//! granularity: any edit to a `.ori` file invalidates every symbol in that
//! file. This module narrows the blast radius to a single *symbol* by hashing
//! the structural fingerprint of each [`Symbol`] (id + kind + signature +
//! sorted effects) and storing per-symbol computed values in a
//! [`QueryCache`].
//!
//! The cache is intentionally small and dependency-free:
//!
//! * [`symbol_fingerprint`] is FNV-1a over the canonical (id, kind, signature,
//!   sorted effects) tuple. Identical symbols hash identically; signature or
//!   effect edits change the fingerprint.
//! * [`QueryCache::get_or_compute`] returns a cached `serde_json::Value` for a
//!   `(symbol_id, fingerprint)` pair, computing it lazily via a closure on
//!   miss. The cache cannot panic on a missing key — every code path is total.
//! * [`changed_symbols`] diffs a previous fingerprint table against the
//!   current [`Module`] and reports both modified and newly added ids.
//! * [`invalidated_dependents`] performs **one-hop** import propagation on a
//!   [`ModuleGraph`] — a downstream module is invalidated if it imports a
//!   module that contains any changed symbol. Multi-hop / transitive
//!   propagation is a known limitation of the bootstrap and is documented on
//!   the function.
//!
//! The engine deliberately avoids any global state: callers construct a
//! [`QueryCache`] per session and pass it explicitly. All public outputs use
//! `BTreeMap` / `BTreeSet` or are explicitly sorted so that JSON contracts
//! remain deterministic.

use crate::ast::{Module, Symbol, SymbolKind};
use crate::node_id::fnv1a_64;
use crate::resolver::ModuleGraph;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Per-symbol structural fingerprint.
///
/// The hash is FNV-1a over the canonical tuple `(id, kind, normalized
/// signature, sorted effects)`. Effects are sorted so that re-ordering them
/// in source does not change the fingerprint; only the *set* of effects
/// matters. The bootstrap parser bundles the `uses <effect-list>` clause into
/// `Symbol::signature`, so the signature is *normalized* by stripping that
/// clause before hashing — otherwise effect re-ordering would change the
/// fingerprint twice (once via the sorted-effects list and once via the raw
/// signature) and the two paths would disagree.
///
/// Identical inputs yield identical fingerprints across processes and across
/// hosts (FNV-1a is platform-independent and dependency-free — see
/// [`crate::node_id::fnv1a_64`]).
pub fn symbol_fingerprint(symbol: &Symbol) -> u64 {
    let mut effects: Vec<&str> = symbol.effects.iter().map(String::as_str).collect();
    effects.sort_unstable();
    let kind = symbol.kind.as_str();
    let normalized_signature = normalize_signature(&symbol.signature);
    let mut buffer = String::with_capacity(
        symbol.id.len()
            + normalized_signature.len()
            + kind.len()
            + 8
            + effects.iter().map(|effect| effect.len() + 1).sum::<usize>(),
    );
    buffer.push_str(&symbol.id);
    buffer.push('|');
    buffer.push_str(kind);
    buffer.push('|');
    buffer.push_str(&normalized_signature);
    buffer.push('|');
    for (idx, effect) in effects.iter().enumerate() {
        if idx > 0 {
            buffer.push(',');
        }
        buffer.push_str(effect);
    }
    fnv1a_64(buffer.as_bytes())
}

/// Strip the `uses <...>` clause from a signature so that effect re-ordering
/// is not double-counted by the fingerprint. The match is whitespace-tolerant
/// and case-sensitive (consistent with the lexer). Returns the original string
/// unchanged when no `uses` clause is present.
fn normalize_signature(signature: &str) -> String {
    // Search for the standalone keyword `uses` preceded by whitespace and
    // followed by whitespace. We deliberately avoid `str::find` on the raw
    // substring "uses" because that would match identifiers like `houses`.
    let bytes = signature.as_bytes();
    let mut i = 0usize;
    while i + 4 <= bytes.len() {
        let preceded_by_ws_or_start = i == 0 || bytes[i - 1].is_ascii_whitespace();
        let candidate = &bytes[i..i + 4];
        let followed_by_ws =
            i + 4 < bytes.len() && (bytes[i + 4].is_ascii_whitespace() || bytes[i + 4] == b':');
        if preceded_by_ws_or_start && candidate == b"uses" && followed_by_ws {
            let head = signature[..i].trim_end();
            return head.to_string();
        }
        i += 1;
    }
    signature.to_string()
}

/// Build the canonical `symbol_id -> fingerprint` table for a module.
///
/// The module-level pseudo-symbol (`kind == Module`) is excluded so that an
/// unrelated edit to the module header line does not invalidate every
/// downstream consumer.
pub fn module_fingerprints(module: &Module) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for symbol in &module.symbols {
        if symbol.kind == SymbolKind::Module {
            continue;
        }
        out.insert(symbol.id.clone(), symbol_fingerprint(symbol));
    }
    out
}

/// In-memory cache of per-symbol computed values keyed by `(id, fingerprint)`.
///
/// The cache stores `serde_json::Value` so that arbitrary downstream queries
/// (effect summaries, type traces, openapi entries, etc.) can share one cache
/// without coupling the engine to any specific result shape.
///
/// The cache is intentionally append-only at the symbol level: when the
/// fingerprint for an id changes, the prior entry is dropped and the value is
/// re-computed. There is no panic path; a missing or stale entry simply
/// triggers a re-compute.
#[derive(Debug, Default, Clone)]
pub struct QueryCache {
    entries: BTreeMap<String, CacheEntry>,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    fingerprint: u64,
    value: Value,
}

impl QueryCache {
    /// Construct an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of entries currently cached.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache holds any entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Lookup the cached value for `symbol_id` if its fingerprint matches.
    ///
    /// Returns `None` when the id is unknown or the stored fingerprint differs
    /// from the requested one. This method never panics.
    pub fn get(&self, symbol_id: &str, fingerprint: u64) -> Option<&Value> {
        self.entries
            .get(symbol_id)
            .filter(|entry| entry.fingerprint == fingerprint)
            .map(|entry| &entry.value)
    }

    /// Drop the cached entry for `symbol_id`, if any.
    pub fn invalidate(&mut self, symbol_id: &str) {
        self.entries.remove(symbol_id);
    }

    /// Return a cached value, or compute and store one via `compute` on miss.
    ///
    /// The cache is keyed by `(symbol_id, fingerprint)`. If the stored
    /// fingerprint differs from the requested one, the prior value is replaced
    /// with the freshly computed one. The closure runs at most once per call
    /// regardless of which branch is taken.
    pub fn get_or_compute<F>(&mut self, symbol_id: &str, fingerprint: u64, compute: F) -> Value
    where
        F: FnOnce() -> Value,
    {
        if let Some(entry) = self.entries.get(symbol_id) {
            if entry.fingerprint == fingerprint {
                return entry.value.clone();
            }
        }
        let value = compute();
        self.entries.insert(
            symbol_id.to_string(),
            CacheEntry {
                fingerprint,
                value: value.clone(),
            },
        );
        value
    }
}

/// Compute the sorted list of symbol ids whose fingerprint changed, plus any
/// ids newly added in the current module.
///
/// Removed ids (present in `prev` but missing from `current`) are *not*
/// returned here; the CLI surface tracks removed ids separately. The result is
/// sorted lexicographically for deterministic JSON output.
pub fn changed_symbols(prev: &BTreeMap<String, u64>, current: &Module) -> Vec<String> {
    let mut out = BTreeSet::new();
    for symbol in &current.symbols {
        if symbol.kind == SymbolKind::Module {
            continue;
        }
        let fingerprint = symbol_fingerprint(symbol);
        match prev.get(&symbol.id) {
            None => {
                out.insert(symbol.id.clone());
            }
            Some(prev_fp) if *prev_fp != fingerprint => {
                out.insert(symbol.id.clone());
            }
            _ => {}
        }
    }
    out.into_iter().collect()
}

/// Propagate invalidation along import edges in the module graph.
///
/// Given a set of changed *symbol* ids, returns the sorted list of module
/// names that should be re-checked because they import a module that contains
/// one of those changed symbols.
///
/// The bootstrap heuristic extracts each symbol's owning module from the
/// canonical id (everything before the last `.` in the segment following the
/// `sym:` or `node:` prefix). Symbols without a parseable module prefix are
/// ignored — they cannot trigger import-level invalidation by definition.
///
/// **Limitation:** this is a one-hop walk. A change in `a` invalidates
/// modules that `import a`, but not modules that `import b` where `b` itself
/// imports `a`. Multi-hop / fixpoint propagation lands when the query engine
/// graduates beyond the bootstrap. For the current edit-check-repair loop a
/// single hop is sufficient because every level eventually runs through the
/// same cache during the test/run cycle.
pub fn invalidated_dependents(graph: &ModuleGraph, changed: &[String]) -> Vec<String> {
    let changed_modules: BTreeSet<String> = changed
        .iter()
        .filter_map(|id| module_of_symbol_id(id))
        .collect();
    if changed_modules.is_empty() {
        return Vec::new();
    }
    let mut out: BTreeSet<String> = BTreeSet::new();
    for (module, imports) in &graph.edges {
        if changed_modules.contains(module) {
            // The changed module itself is reported by `changed_symbols`; we
            // only want strictly *downstream* dependents here.
            continue;
        }
        for import in imports {
            if changed_modules.contains(import) {
                out.insert(module.clone());
                break;
            }
        }
    }
    out.into_iter().collect()
}

/// Structured report for the `ori agent changed` CLI surface.
///
/// All arrays are sorted lexicographically so the JSON output is byte-stable
/// across runs and across hosts. `notes` collects any non-fatal warnings
/// (e.g. unreadable previous fingerprint file) so the CLI never has to fail
/// just because the cache was missing or corrupt.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentChangedReport {
    pub schema: &'static str,
    pub added: Vec<String>,
    pub modified: Vec<String>,
    pub removed: Vec<String>,
    pub dependents: Vec<String>,
    pub notes: Vec<String>,
}

impl AgentChangedReport {
    /// Build a fresh report with the canonical schema id and empty arrays.
    pub fn empty() -> Self {
        Self {
            schema: "ori.agent_changed.v1",
            added: Vec::new(),
            modified: Vec::new(),
            removed: Vec::new(),
            dependents: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// Render the report as a single-line JSON string using the shared
    /// serializer that cannot panic.
    pub fn to_json(&self) -> String {
        crate::json::to_json(self)
    }
}

/// Compose a [`AgentChangedReport`] from a previous fingerprint table and the
/// current set of parsed modules.
///
/// * `prev` — the previously-persisted `symbol_id -> fingerprint` table. When
///   absent (first run, unreadable cache, etc.) the caller should pass an
///   empty map and push an explanatory entry into `notes` *before* calling
///   this helper so the reason surfaces in the final JSON.
/// * `modules` — the parsed modules for this run.
/// * `graph` — the resolver's module graph used for one-hop dependent
///   propagation.
/// * `notes` — pre-populated notes (e.g. "previous fingerprints file was
///   unreadable; treating all symbols as new") that should be preserved in
///   the report.
pub fn build_agent_changed_report(
    prev: &BTreeMap<String, u64>,
    modules: &[Module],
    graph: &ModuleGraph,
    notes: Vec<String>,
) -> AgentChangedReport {
    let mut added: BTreeSet<String> = BTreeSet::new();
    let mut modified: BTreeSet<String> = BTreeSet::new();
    let mut current: BTreeMap<String, u64> = BTreeMap::new();
    for module in modules {
        for symbol in &module.symbols {
            if symbol.kind == SymbolKind::Module {
                continue;
            }
            let fingerprint = symbol_fingerprint(symbol);
            current.insert(symbol.id.clone(), fingerprint);
            match prev.get(&symbol.id) {
                None => {
                    added.insert(symbol.id.clone());
                }
                Some(prev_fp) if *prev_fp != fingerprint => {
                    modified.insert(symbol.id.clone());
                }
                _ => {}
            }
        }
    }
    let removed: BTreeSet<String> = prev
        .keys()
        .filter(|id| !current.contains_key(id.as_str()))
        .cloned()
        .collect();
    let mut changed: Vec<String> = added.iter().chain(modified.iter()).cloned().collect();
    changed.sort();
    changed.dedup();
    let dependents = invalidated_dependents(graph, &changed);
    AgentChangedReport {
        schema: "ori.agent_changed.v1",
        added: added.into_iter().collect(),
        modified: modified.into_iter().collect(),
        removed: removed.into_iter().collect(),
        dependents,
        notes,
    }
}

/// Combine fingerprint tables from many modules into a single
/// `symbol_id -> fingerprint` map. The result is suitable for persisting to a
/// `.ori/fingerprints.json` cache and feeding back into
/// [`build_agent_changed_report`] on the next run.
pub fn combined_fingerprints(modules: &[Module]) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for module in modules {
        for (id, fingerprint) in module_fingerprints(module) {
            out.insert(id, fingerprint);
        }
    }
    out
}

/// Extract the module name from a canonical symbol id of the form
/// `sym:<module>.<name>` or `node:<module>.<kind>.<name>.<hash>`.
fn module_of_symbol_id(id: &str) -> Option<String> {
    let body = id
        .strip_prefix("sym:")
        .or_else(|| id.strip_prefix("node:"))
        .unwrap_or(id);
    let last_dot = body.rfind('.')?;
    let prefix = &body[..last_dot];
    if prefix.is_empty() {
        return None;
    }
    Some(prefix.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_source;
    use crate::source::SourceFile;

    fn module_of(text: &str, path: &str) -> Module {
        parse_source(&SourceFile::new(path.to_string(), text.to_string())).module
    }

    #[test]
    fn fingerprint_is_stable_for_same_symbol() {
        let m = module_of(
            "module demo\nfn greet() -> Unit uses io.stdout",
            "/demo.ori",
        );
        let fps_a = module_fingerprints(&m);
        let fps_b = module_fingerprints(&m);
        assert_eq!(fps_a, fps_b);
        assert!(!fps_a.is_empty());
    }

    #[test]
    fn fingerprint_ignores_effect_order() {
        let m_a = module_of(
            "module demo\nfn greet() -> Unit uses io.stdout, db.read",
            "/demo.ori",
        );
        let m_b = module_of(
            "module demo\nfn greet() -> Unit uses db.read, io.stdout",
            "/demo.ori",
        );
        let a = module_fingerprints(&m_a);
        let b = module_fingerprints(&m_b);
        assert_eq!(a, b, "effect order must not change the fingerprint");
    }

    #[test]
    fn identical_input_reports_no_changes() {
        let m = module_of("module demo\nfn greet() -> Unit", "/demo.ori");
        let prev = module_fingerprints(&m);
        let changed = changed_symbols(&prev, &m);
        assert!(
            changed.is_empty(),
            "no symbols should change between identical inputs"
        );
    }

    #[test]
    fn signature_edit_reports_only_that_symbol() {
        let before = module_of(
            "module demo\nfn greet() -> Unit\nfn other() -> Unit",
            "/demo.ori",
        );
        let after = module_of(
            "module demo\nfn greet() -> Int\nfn other() -> Unit",
            "/demo.ori",
        );
        let prev = module_fingerprints(&before);
        let changed = changed_symbols(&prev, &after);
        assert_eq!(
            changed.len(),
            1,
            "exactly one symbol should change, got: {changed:?}"
        );
        let only = changed.first().map(String::as_str).unwrap_or_default();
        assert!(
            only.contains("greet"),
            "the changed id should belong to greet, got: {only}"
        );
    }

    #[test]
    fn newly_added_symbol_is_reported() {
        let before = module_of("module demo\nfn greet() -> Unit", "/demo.ori");
        let after = module_of(
            "module demo\nfn greet() -> Unit\nfn fresh() -> Unit",
            "/demo.ori",
        );
        let prev = module_fingerprints(&before);
        let changed = changed_symbols(&prev, &after);
        assert_eq!(
            changed.len(),
            1,
            "only the new symbol should be reported, got: {changed:?}"
        );
        let only = changed.first().map(String::as_str).unwrap_or_default();
        assert!(only.contains("fresh"));
    }

    #[test]
    fn cache_get_or_compute_returns_cached_value_on_hit() {
        let m = module_of("module demo\nfn greet() -> Unit", "/demo.ori");
        let fps = module_fingerprints(&m);
        let (sym_id, fp) = fps
            .iter()
            .next()
            .map(|(k, v)| (k.clone(), *v))
            .unwrap_or_else(|| (String::new(), 0));
        let mut cache = QueryCache::new();
        let mut calls = 0usize;
        let first = cache.get_or_compute(&sym_id, fp, || {
            calls += 1;
            serde_json::json!({"hit": false})
        });
        let second = cache.get_or_compute(&sym_id, fp, || {
            calls += 1;
            serde_json::json!({"hit": false})
        });
        assert_eq!(first, second);
        assert_eq!(calls, 1, "compute closure should run exactly once on a hit");
    }

    #[test]
    fn cache_recomputes_when_fingerprint_changes() {
        let mut cache = QueryCache::new();
        let id = "sym:demo.greet";
        let _ = cache.get_or_compute(id, 1, || serde_json::json!("v1"));
        let v2 = cache.get_or_compute(id, 2, || serde_json::json!("v2"));
        assert_eq!(v2, serde_json::json!("v2"));
        assert_eq!(cache.get(id, 2), Some(&serde_json::json!("v2")));
        assert!(
            cache.get(id, 1).is_none(),
            "stale fingerprint must not return a value"
        );
    }

    #[test]
    fn cache_get_missing_key_returns_none() {
        let cache = QueryCache::new();
        assert!(cache.get("sym:nonexistent", 12345).is_none());
    }

    #[test]
    fn dependent_module_is_reported_on_one_hop_import() {
        // Module `b` imports `a`. When a symbol in `a` changes, `b` is
        // invalidated; `c` (imports nothing related) is not.
        let m_a = module_of("module a\nfn target() -> Unit", "/a.ori");
        let m_b = module_of("module b\nimport a", "/b.ori");
        let m_c = module_of("module c", "/c.ori");
        let resolution = crate::resolver::resolve(&[m_a, m_b, m_c]);
        let dependents = invalidated_dependents(&resolution.graph, &["sym:a.target".to_string()]);
        assert!(
            dependents.contains(&"b".to_string()),
            "b should be invalidated, got: {dependents:?}"
        );
        assert!(
            !dependents.contains(&"c".to_string()),
            "c should not be invalidated, got: {dependents:?}"
        );
        assert!(
            !dependents.contains(&"a".to_string()),
            "a itself is not its own dependent"
        );
    }

    #[test]
    fn dependents_are_sorted_and_unique() {
        let m_a = module_of("module a\nfn x() -> Unit", "/a.ori");
        let m_b = module_of("module b\nimport a", "/b.ori");
        let m_c = module_of("module c\nimport a", "/c.ori");
        let resolution = crate::resolver::resolve(&[m_a, m_b, m_c]);
        let dependents = invalidated_dependents(
            &resolution.graph,
            &["sym:a.x".to_string(), "sym:a.x".to_string()],
        );
        assert_eq!(dependents, vec!["b".to_string(), "c".to_string()]);
    }
}
