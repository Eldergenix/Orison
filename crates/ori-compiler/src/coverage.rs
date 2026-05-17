//! Static test-coverage estimator (bootstrap).
//!
//! Given a map of implementation files and a map of test files, the estimator
//! enumerates every public `function` / `query` symbol in the implementation
//! modules and reports which of them are *mentioned by name* in at least one
//! test file. Mention detection uses identifier-style word boundaries: the
//! characters on either side of a match must be neither alphanumeric nor `_`.
//!
//! The estimator is intentionally syntactic. It does not parse or interpret
//! test bodies; "mentioned" is a proxy for "exercised". This is enough to
//! drive a quick coverage gate during the bootstrap and to seed the richer
//! reports that the call-graph backend will eventually produce.
//!
//! Determinism is a hard requirement: outputs are sorted, percentages are
//! computed from sorted counts, and empty inputs return a zero report rather
//! than dividing by zero.

use crate::ast::SymbolKind;
use crate::parser::parse_source;
use crate::source::SourceFile;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Top-level coverage report for `ori coverage`.
#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct CoverageReport {
    pub schema: &'static str,
    pub root: String,
    pub total_functions: usize,
    pub covered: Vec<FunctionCoverage>,
    pub uncovered: Vec<FunctionCoverage>,
    pub percent: f32,
}

/// Per-function coverage entry.
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct FunctionCoverage {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub tests_referencing: Vec<String>,
}

impl CoverageReport {
    pub fn to_json(&self) -> String {
        crate::json::to_json(self)
    }

    /// Construct a deterministic, zero-coverage report. Useful when the input
    /// universe is empty (no implementation files) and the caller still wants
    /// to emit a valid envelope.
    pub fn empty(root: impl Into<String>) -> Self {
        Self {
            schema: "ori.coverage_report.v1",
            root: root.into(),
            total_functions: 0,
            covered: Vec::new(),
            uncovered: Vec::new(),
            percent: 0.0,
        }
    }
}

/// Compute a coverage report for the given implementation/test file maps.
///
/// `impl_files` and `test_files` are both `path -> source_text` maps so that
/// callers may build them from disk or supply synthetic fixtures from tests.
/// The function never panics; malformed input simply yields zero matches.
pub fn coverage_for_files(
    impl_files: &BTreeMap<String, String>,
    test_files: &BTreeMap<String, String>,
) -> CoverageReport {
    // Parse every implementation file and collect all public functions and
    // queries. Names starting with `_` are considered private by the Module
    // convention and are excluded from the coverage universe.
    let mut targets: Vec<TargetFn> = Vec::new();
    for (path, text) in impl_files {
        let parse = parse_source(&SourceFile::new(path.clone(), text.clone()));
        for sym in &parse.module.symbols {
            if !matches!(sym.kind, SymbolKind::Function | SymbolKind::Query) {
                continue;
            }
            if sym.name.starts_with('_') {
                continue;
            }
            targets.push(TargetFn {
                id: sym.id.clone(),
                name: sym.name.clone(),
                kind: sym.kind.as_str().to_string(),
            });
        }
    }

    // Deduplicate by symbol id (path-aware) so two impl files declaring the
    // same id don't double-count. Use a BTreeMap keyed by id to keep
    // ordering deterministic.
    let mut by_id: BTreeMap<String, TargetFn> = BTreeMap::new();
    for t in targets {
        by_id.entry(t.id.clone()).or_insert(t);
    }

    // Build the per-function coverage view. For each function, scan every
    // test file (in path order) and collect the file paths that contain the
    // function's name as a whole token. Identifier boundary check uses
    // alphanumeric-or-underscore on both sides so `add` does NOT match
    // `add_line` or `address`.
    let mut covered: Vec<FunctionCoverage> = Vec::new();
    let mut uncovered: Vec<FunctionCoverage> = Vec::new();
    for target in by_id.into_values() {
        let mut referencing: BTreeSet<String> = BTreeSet::new();
        for (test_path, test_text) in test_files {
            if mentions_identifier(test_text, &target.name) {
                referencing.insert(test_path.clone());
            }
        }
        let mut tests_referencing: Vec<String> = referencing.into_iter().collect();
        tests_referencing.sort();
        let entry = FunctionCoverage {
            id: target.id,
            name: target.name,
            kind: target.kind,
            tests_referencing,
        };
        if entry.tests_referencing.is_empty() {
            uncovered.push(entry);
        } else {
            covered.push(entry);
        }
    }

    // Sort both lists by id for stable output across runs.
    covered.sort_by(|a, b| a.id.cmp(&b.id));
    uncovered.sort_by(|a, b| a.id.cmp(&b.id));

    let total_functions = covered.len() + uncovered.len();
    let percent = if total_functions == 0 {
        0.0
    } else {
        // Deterministic: compute from sorted integer counts before dividing
        // so floating point rounding is stable across runs and platforms.
        (covered.len() as f32) * 100.0 / (total_functions as f32)
    };

    CoverageReport {
        schema: "ori.coverage_report.v1",
        root: ".".to_string(),
        total_functions,
        covered,
        uncovered,
        percent,
    }
}

struct TargetFn {
    id: String,
    name: String,
    kind: String,
}

/// Return true when `haystack` contains `needle` as a whole identifier-style
/// token: the bytes on either side of the match must be neither ASCII
/// alphanumeric nor `_`. Matching is case-sensitive.
fn mentions_identifier(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let bytes = haystack.as_bytes();
    let n_bytes = needle.as_bytes();
    let n_len = n_bytes.len();
    if n_len == 0 || bytes.len() < n_len {
        return false;
    }
    let upper_bound = bytes.len().saturating_sub(n_len);
    let mut i = 0usize;
    while i <= upper_bound {
        if &bytes[i..i + n_len] == n_bytes {
            let left_ok = i == 0 || !is_ident_byte(bytes[i - 1]);
            let right_idx = i + n_len;
            let right_ok = right_idx >= bytes.len() || !is_ident_byte(bytes[right_idx]);
            if left_ok && right_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn impls(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(p, t)| ((*p).to_string(), (*t).to_string()))
            .collect()
    }

    #[test]
    fn all_functions_covered_yields_one_hundred_percent() {
        let impls_map = impls(&[(
            "/src/cart.ori",
            "module demo.cart\nfn add_line(c: Cart) -> Cart\nfn remove_line(c: Cart) -> Cart\n",
        )]);
        let tests_map = impls(&[(
            "/tests/cart.ori",
            "module demo.tests\nfn test_a() -> Unit\nfn test_b() -> Unit\n// uses add_line and remove_line\n",
        )]);
        let report = coverage_for_files(&impls_map, &tests_map);
        assert_eq!(report.total_functions, 2);
        assert_eq!(report.covered.len(), 2);
        assert!(report.uncovered.is_empty());
        assert!((report.percent - 100.0).abs() < f32::EPSILON);
        assert_eq!(report.schema, "ori.coverage_report.v1");
    }

    #[test]
    fn no_functions_covered_yields_zero_percent() {
        let impls_map = impls(&[(
            "/src/cart.ori",
            "module demo.cart\nfn add_line(c: Cart) -> Cart\nfn remove_line(c: Cart) -> Cart\n",
        )]);
        let tests_map = impls(&[(
            "/tests/other.ori",
            "module demo.tests\nfn test_z() -> Unit\n",
        )]);
        let report = coverage_for_files(&impls_map, &tests_map);
        assert_eq!(report.total_functions, 2);
        assert!(report.covered.is_empty());
        assert_eq!(report.uncovered.len(), 2);
        assert!(report.percent.abs() < f32::EPSILON);
    }

    #[test]
    fn partial_coverage_rounds_correctly() {
        let impls_map = impls(&[(
            "/src/m.ori",
            "module demo.m\nfn alpha() -> Unit\nfn beta() -> Unit\nfn gamma() -> Unit\nfn delta() -> Unit\n",
        )]);
        let tests_map = impls(&[(
            "/tests/m.ori",
            "module demo.tests\nfn test_alpha_and_beta() -> Unit\n// calls alpha and beta\n",
        )]);
        let report = coverage_for_files(&impls_map, &tests_map);
        assert_eq!(report.total_functions, 4);
        assert_eq!(report.covered.len(), 2);
        assert_eq!(report.uncovered.len(), 2);
        // 2 of 4 -> 50.0 exactly.
        assert!((report.percent - 50.0).abs() < f32::EPSILON);
    }

    #[test]
    fn word_boundary_rejects_substring_and_underscore_neighbors() {
        // `add` must NOT be matched by `address`, `add_line`, or `_add`.
        let impls_map = impls(&[("/src/m.ori", "module demo.m\nfn add() -> Unit\n")]);
        let tests_map = impls(&[(
            "/tests/m.ori",
            "module demo.tests\nfn test_x() -> Unit\n// touches address and add_line and call_add and _add\n",
        )]);
        let report = coverage_for_files(&impls_map, &tests_map);
        assert_eq!(report.total_functions, 1);
        assert!(
            report.covered.is_empty(),
            "expected `add` to be uncovered, got covered: {:?}",
            report.covered
        );
        assert_eq!(report.uncovered.len(), 1);
    }

    #[test]
    fn word_boundary_accepts_clean_identifier_boundary() {
        let impls_map = impls(&[("/src/m.ori", "module demo.m\nfn add() -> Unit\n")]);
        let tests_map = impls(&[(
            "/tests/m.ori",
            "module demo.tests\nfn test_x() -> Unit\n// add(1, 2)\n",
        )]);
        let report = coverage_for_files(&impls_map, &tests_map);
        assert_eq!(report.covered.len(), 1);
        assert!(report.uncovered.is_empty());
    }

    #[test]
    fn matching_is_case_sensitive() {
        let impls_map = impls(&[("/src/m.ori", "module demo.m\nfn add() -> Unit\n")]);
        let tests_map = impls(&[(
            "/tests/m.ori",
            "module demo.tests\nfn test_x() -> Unit\n// ADD and Add and aDD\n",
        )]);
        let report = coverage_for_files(&impls_map, &tests_map);
        assert!(report.covered.is_empty());
        assert_eq!(report.uncovered.len(), 1);
    }

    #[test]
    fn output_is_deterministic_across_runs() {
        let impls_map = impls(&[
            (
                "/src/b.ori",
                "module demo.b\nfn beta() -> Unit\nfn alpha() -> Unit\n",
            ),
            ("/src/a.ori", "module demo.a\nfn gamma() -> Unit\n"),
        ]);
        let tests_map = impls(&[
            (
                "/tests/z.ori",
                "module demo.tz\nfn test_z() -> Unit\n// alpha\n",
            ),
            (
                "/tests/a.ori",
                "module demo.ta\nfn test_a() -> Unit\n// alpha and gamma\n",
            ),
        ]);
        let first = coverage_for_files(&impls_map, &tests_map);
        let second = coverage_for_files(&impls_map, &tests_map);
        assert_eq!(first, second);
        // tests_referencing for alpha must be sorted by path.
        let alphas: Vec<&FunctionCoverage> =
            first.covered.iter().filter(|f| f.name == "alpha").collect();
        assert_eq!(
            alphas.len(),
            1,
            "alpha should appear exactly once in covered"
        );
        let alpha = alphas[0];
        let mut sorted = alpha.tests_referencing.clone();
        sorted.sort();
        assert_eq!(alpha.tests_referencing, sorted);
        assert_eq!(
            alpha.tests_referencing,
            vec!["/tests/a.ori", "/tests/z.ori"]
        );
    }

    #[test]
    fn empty_input_produces_zero_report_without_dividing_by_zero() {
        let impls_map: BTreeMap<String, String> = BTreeMap::new();
        let tests_map: BTreeMap<String, String> = BTreeMap::new();
        let report = coverage_for_files(&impls_map, &tests_map);
        assert_eq!(report.total_functions, 0);
        assert!(report.covered.is_empty());
        assert!(report.uncovered.is_empty());
        assert!(report.percent.abs() < f32::EPSILON);
        assert!(report.percent.is_finite());
    }

    #[test]
    fn tests_referencing_counts_increment_per_file() {
        let impls_map = impls(&[("/src/m.ori", "module demo.m\nfn target() -> Unit\n")]);
        let tests_map = impls(&[
            (
                "/tests/a.ori",
                "module ta\nfn test_a() -> Unit\n// target()\n",
            ),
            (
                "/tests/b.ori",
                "module tb\nfn test_b() -> Unit\n// target()\n",
            ),
            (
                "/tests/c.ori",
                "module tc\nfn test_c() -> Unit\n// no_call\n",
            ),
        ]);
        let report = coverage_for_files(&impls_map, &tests_map);
        assert_eq!(report.covered.len(), 1);
        let target = &report.covered[0];
        assert_eq!(target.tests_referencing.len(), 2);
        assert_eq!(target.tests_referencing[0], "/tests/a.ori");
        assert_eq!(target.tests_referencing[1], "/tests/b.ori");
    }

    #[test]
    fn ignores_non_function_kinds() {
        // Types, services, modules: not counted as functions.
        let impls_map = impls(&[(
            "/src/m.ori",
            "module demo.m\ntype Cart\nservice Catalog\nfn included() -> Unit\n",
        )]);
        let tests_map = impls(&[(
            "/tests/m.ori",
            "module tm\nfn test_a() -> Unit\n// included()\n",
        )]);
        let report = coverage_for_files(&impls_map, &tests_map);
        assert_eq!(report.total_functions, 1);
        assert_eq!(report.covered.len(), 1);
        assert_eq!(report.covered[0].name, "included");
    }

    #[test]
    fn renders_valid_schema_json() {
        let report = CoverageReport::empty(".");
        let rendered = report.to_json();
        assert!(rendered.contains("\"schema\":\"ori.coverage_report.v1\""));
        assert!(rendered.contains("\"total_functions\":0"));
        assert!(rendered.contains("\"percent\":0"));
    }
}
