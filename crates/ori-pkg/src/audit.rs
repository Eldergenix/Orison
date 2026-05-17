//! Capability/audit reporter.
//!
//! Output matches `schemas/audit-report.schema.json`. Three rules are
//! evaluated against the resolved graph:
//!
//! * `AUD0001` (`error`) — a dependency declares a capability that the root
//!   manifest does not list under `[capabilities].declared`.
//! * `AUD0002` (`info`) — the root manifest declares a capability that none
//!   of its dependencies require.
//! * `AUD0003` (`warn`) — duplicate package versions in the graph (same name
//!   appearing twice with different versions).
//!
//! Findings are returned sorted by `(severity, id, target)` to keep output
//! byte-stable.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::manifest::Manifest;
use crate::resolver::ResolvedGraph;

/// Schema identifier.
pub const AUDIT_SCHEMA: &str = "ori.audit_report.v1";

/// Finding severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditSeverity {
    /// Informational, no action required.
    Info,
    /// Should be looked at but does not block.
    Warn,
    /// Must be resolved.
    Error,
}

impl AuditSeverity {
    fn rank(self) -> u8 {
        match self {
            AuditSeverity::Error => 0,
            AuditSeverity::Warn => 1,
            AuditSeverity::Info => 2,
        }
    }
}

/// A single audit finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditFinding {
    /// Stable identifier (e.g. `AUD0001`).
    pub id: String,
    /// Severity.
    pub severity: AuditSeverity,
    /// Human-readable message.
    pub message: String,
    /// What the finding is about (e.g. `package:foo@0.1.0`).
    pub target: String,
    /// Optional remediation hint.
    pub fix_hint: Option<String>,
}

/// Summary counts. Field names match the schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditSummary {
    /// Number of audit rules that produced no findings of any severity.
    pub pass: u32,
    /// Number of `warn` findings.
    pub warn: u32,
    /// Number of `error` findings.
    pub fail: u32,
}

/// Audit report document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditReport {
    /// Schema identifier.
    pub schema: String,
    /// Summary counts.
    pub summary: AuditSummary,
    /// Findings sorted by `(severity, id, target)`.
    pub findings: Vec<AuditFinding>,
}

/// Stable identifiers of every audit rule. `pass` in the summary is the
/// count of rules in this list that produced no findings.
pub const AUDIT_RULES: &[&str] = &["AUD0001", "AUD0002", "AUD0003"];

/// Run the audit against the manifest and resolved graph.
pub fn run_audit(manifest: &Manifest, graph: &ResolvedGraph) -> AuditReport {
    let mut findings: Vec<AuditFinding> = Vec::new();

    let declared: BTreeSet<&str> = manifest
        .capabilities
        .declared
        .iter()
        .map(String::as_str)
        .collect();

    // Collect required capabilities across all dependencies (excluding root).
    let mut required: BTreeMap<String, Vec<String>> = BTreeMap::new(); // cap -> list of targets
    for (name, node) in &graph.nodes {
        if name == &graph.root {
            continue;
        }
        for cap in &node.capabilities {
            required
                .entry(cap.clone())
                .or_default()
                .push(format!("package:{}@{}", node.name, node.version));
        }
    }

    // Rule AUD0001: dependency requires capability not declared by root.
    for (cap, targets) in &required {
        if !declared.contains(cap.as_str()) {
            for target in targets {
                findings.push(AuditFinding {
                    id: "AUD0001".to_string(),
                    severity: AuditSeverity::Error,
                    message: format!(
                        "dependency requires capability `{cap}` that is not declared by root package"
                    ),
                    target: target.clone(),
                    fix_hint: Some(format!(
                        "add `{cap}` to [capabilities].declared in ori.toml or remove the dependency"
                    )),
                });
            }
        }
    }

    // Rule AUD0002: declared capability not required by any dependency.
    for cap in &manifest.capabilities.declared {
        if !required.contains_key(cap) {
            findings.push(AuditFinding {
                id: "AUD0002".to_string(),
                severity: AuditSeverity::Info,
                message: format!("capability `{cap}` is declared but no dependency requires it"),
                target: format!("package:{}", manifest.package.name),
                fix_hint: Some(
                    "consider removing the declaration to shrink the trust surface".to_string(),
                ),
            });
        }
    }

    // Rule AUD0003: duplicate package versions.
    // Build name -> set of versions across the graph.
    let mut versions: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for node in graph.nodes.values() {
        if node.version.is_empty() {
            continue;
        }
        versions
            .entry(node.name.as_str())
            .or_default()
            .insert(node.version.as_str());
    }
    for (name, vs) in &versions {
        if vs.len() > 1 {
            let list: Vec<&&str> = vs.iter().collect();
            findings.push(AuditFinding {
                id: "AUD0003".to_string(),
                severity: AuditSeverity::Warn,
                message: format!(
                    "package `{name}` appears with {} versions: {}",
                    vs.len(),
                    list.iter().copied().copied().collect::<Vec<_>>().join(", ")
                ),
                target: format!("package:{name}"),
                fix_hint: Some("unify the version pin across the dependency tree".to_string()),
            });
        }
    }

    findings.sort_by(|a, b| {
        a.severity
            .rank()
            .cmp(&b.severity.rank())
            .then_with(|| a.id.cmp(&b.id))
            .then_with(|| a.target.cmp(&b.target))
    });

    let mut summary = AuditSummary {
        pass: 0,
        warn: 0,
        fail: 0,
    };
    for f in &findings {
        match f.severity {
            AuditSeverity::Error => summary.fail += 1,
            AuditSeverity::Warn => summary.warn += 1,
            // Info findings are reported but do not change pass/warn/fail.
            AuditSeverity::Info => {}
        }
    }
    let firing_rules: BTreeSet<&str> = findings.iter().map(|f| f.id.as_str()).collect();
    summary.pass = AUDIT_RULES
        .iter()
        .filter(|rule| !firing_rules.contains(*rule))
        .count() as u32;

    AuditReport {
        schema: AUDIT_SCHEMA.to_string(),
        summary,
        findings,
    }
}
