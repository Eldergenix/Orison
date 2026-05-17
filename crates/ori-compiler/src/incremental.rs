//! Incremental change tracking and affected-test selection (bootstrap).
//!
//! The bootstrap cache is in-memory and file-hash based: callers feed in a
//! map of `path -> source text`, the cache hashes each file with FNV-1a,
//! compares against the previous run, and reports which modules changed.
//! Test selection then turns that into the set of affected test symbols.

use crate::ast::SymbolKind;
use crate::node_id::fnv1a_64;
use crate::parser::parse_source;
use crate::source::SourceFile;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Default, Clone)]
pub struct IncrementalCache {
    pub hashes: BTreeMap<String, u64>,
}

#[derive(Debug, Serialize)]
pub struct ChangeReport {
    pub schema: &'static str,
    pub changed_files: Vec<String>,
    pub changed_symbols: Vec<String>,
    pub new_files: Vec<String>,
    pub removed_files: Vec<String>,
}

impl ChangeReport {
    pub fn to_json(&self) -> String {
        crate::json::to_json(self)
    }
}

#[derive(Debug, Serialize)]
pub struct AffectedTests {
    pub schema: &'static str,
    pub root: String,
    pub total_tests: usize,
    pub selected: Vec<TestEntry>,
    pub skipped: Vec<TestEntry>,
}

#[derive(Debug, Serialize)]
pub struct TestEntry {
    pub id: String,
    pub name: String,
    pub file: String,
    pub reason: String,
}

impl AffectedTests {
    pub fn to_json(&self) -> String {
        crate::json::to_json(self)
    }
}

impl IncrementalCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, files: &BTreeMap<String, String>) -> ChangeReport {
        let mut report = ChangeReport {
            schema: "ori.build_report.v1",
            changed_files: Vec::new(),
            changed_symbols: Vec::new(),
            new_files: Vec::new(),
            removed_files: Vec::new(),
        };

        for (path, text) in files {
            let h = fnv1a_64(text.as_bytes());
            match self.hashes.get(path) {
                None => {
                    report.new_files.push(path.clone());
                    self.hashes.insert(path.clone(), h);
                }
                Some(prev) if *prev != h => {
                    report.changed_files.push(path.clone());
                    self.hashes.insert(path.clone(), h);
                }
                _ => {}
            }
        }

        let known: Vec<String> = self.hashes.keys().cloned().collect();
        for path in known {
            if !files.contains_key(&path) {
                report.removed_files.push(path.clone());
                self.hashes.remove(&path);
            }
        }

        // Collect symbol ids defined in changed/new files for downstream
        // test selection.
        for path in report.changed_files.iter().chain(report.new_files.iter()) {
            if let Some(text) = files.get(path) {
                let parse = parse_source(&SourceFile::new(path.clone(), text.clone()));
                for sym in parse
                    .module
                    .symbols
                    .iter()
                    .filter(|s| s.kind != SymbolKind::Module)
                {
                    report.changed_symbols.push(sym.id.clone());
                }
            }
        }
        report
    }
}

/// Select tests affected by a set of changed symbol ids. A test is "affected"
/// when its body mentions any changed symbol id by name (heuristic for the
/// bootstrap; real selection uses the call graph).
pub fn select_affected_tests(
    test_files: &BTreeMap<String, String>,
    changed_symbols: &[String],
) -> AffectedTests {
    let mut total = 0usize;
    let mut selected = Vec::new();
    let mut skipped = Vec::new();
    let names_only: Vec<String> = changed_symbols
        .iter()
        .map(|id| {
            id.rsplit('.')
                .next()
                .map(|s| s.to_string())
                .unwrap_or_else(|| id.clone())
        })
        .collect();

    for (path, text) in test_files {
        let parse = parse_source(&SourceFile::new(path.clone(), text.clone()));
        for sym in parse
            .module
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function && s.name.starts_with("test_"))
        {
            total += 1;
            let matched = names_only.iter().any(|name| text.contains(name));
            let entry = TestEntry {
                id: sym.id.clone(),
                name: sym.name.clone(),
                file: path.clone(),
                reason: if matched {
                    "test references at least one changed symbol".to_string()
                } else {
                    "no changed symbol referenced".to_string()
                },
            };
            if matched {
                selected.push(entry);
            } else {
                skipped.push(entry);
            }
        }
    }

    AffectedTests {
        schema: "ori.agent_tests.v1",
        root: ".".to_string(),
        total_tests: total,
        selected,
        skipped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_pass_reports_files_as_new() {
        let mut cache = IncrementalCache::new();
        let mut files = BTreeMap::new();
        files.insert("/a.ori".to_string(), "module a\nfn f() -> Unit".to_string());
        let report = cache.update(&files);
        assert!(report.new_files.contains(&"/a.ori".to_string()));
        assert!(report.changed_files.is_empty());
    }

    #[test]
    fn unchanged_files_do_not_appear_again() {
        let mut cache = IncrementalCache::new();
        let mut files = BTreeMap::new();
        files.insert("/a.ori".to_string(), "module a".to_string());
        let _ = cache.update(&files);
        let report = cache.update(&files);
        assert!(report.changed_files.is_empty());
        assert!(report.new_files.is_empty());
    }

    #[test]
    fn modified_files_show_up_as_changed() {
        let mut cache = IncrementalCache::new();
        let mut files = BTreeMap::new();
        files.insert("/a.ori".to_string(), "module a".to_string());
        let _ = cache.update(&files);
        files.insert("/a.ori".to_string(), "module a\nfn f() -> Unit".to_string());
        let report = cache.update(&files);
        assert!(report.changed_files.contains(&"/a.ori".to_string()));
        assert!(report.changed_symbols.iter().any(|s| s.contains(".f")));
    }

    #[test]
    fn select_tests_matches_by_reference_per_file() {
        // The bootstrap selector works at file granularity: every test in a
        // file that references a changed symbol is selected. (Per-test body
        // selection lands when the parser learns to recover bodies.)
        let mut tests = BTreeMap::new();
        tests.insert(
            "/touched.ori".to_string(),
            "module touched\nfn test_uses_target() -> Unit\nfn test_sibling() -> Unit".to_string(),
        );
        tests.insert(
            "/cold.ori".to_string(),
            "module cold\nfn test_other() -> Unit".to_string(),
        );
        let changed = vec!["sym:a.target".to_string()];
        let report = select_affected_tests(&tests, &changed);
        // Both tests in /touched.ori are selected because the file references
        // the changed name; /cold.ori is skipped entirely.
        assert!(report.selected.iter().any(|t| t.name == "test_uses_target"));
        assert!(report.selected.iter().any(|t| t.name == "test_sibling"));
        assert!(report.skipped.iter().any(|t| t.name == "test_other"));
    }
}
