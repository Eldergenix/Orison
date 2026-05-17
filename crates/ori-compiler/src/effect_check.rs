//! Effect propagation and capability manifest extraction.
//!
//! Every function symbol carries a list of declared `uses ...` effects. This
//! module turns those declarations into:
//!
//! * a per-symbol diagnostic when an effect is unknown or mistyped;
//! * a capability manifest summarising the union of declared effects and the
//!   symbols that own each one;
//! * a "policy diff" between the package-declared capability set
//!   (`ori.toml` `[capabilities].declared`) and the effects implied by the
//!   compiled symbols.
//!
//! Effect *propagation* in the bootstrap is conservative: because the parser
//! only sees signatures, propagation through call graphs cannot yet be
//! computed precisely. Instead we report the set of declared effects per
//! symbol and emit a `policy.undeclared` finding for any effect not listed
//! in the package policy.

use crate::ast::{Module, SymbolKind};
use crate::diagnostic::Diagnostic;
use crate::effects::is_known_effect_or_capability;
use crate::json::to_json;
use crate::source::Span;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Stable schema identifier for the capability manifest envelope.
pub const CAPABILITY_SCHEMA: &str = "ori.capability.v1";

/// JSON envelope listing every effect declared on the symbols of a module
/// plus the package's declared/undeclared/unused policy breakdown.
#[derive(Debug, Serialize)]
pub struct CapabilityManifest {
    /// Stable schema identifier.
    pub schema: &'static str,
    /// Module name.
    pub module: String,
    /// Effects observed, grouped by effect name.
    pub effects: Vec<EffectEntry>,
    /// Policy diff against the declared capability set.
    pub policy: CapabilityPolicy,
}

/// One effect group: the effect name and the symbol ids that use it.
#[derive(Debug, Serialize)]
pub struct EffectEntry {
    /// Effect or capability name.
    pub name: String,
    /// Symbol ids that declare `uses <name>`.
    pub uses: Vec<String>,
}

/// Declared/undeclared/unused effect lists from the policy diff.
#[derive(Debug, Default, Serialize)]
pub struct CapabilityPolicy {
    /// Effects declared in the package policy.
    pub declared: Vec<String>,
    /// Effects observed but not declared in the policy.
    pub undeclared: Vec<String>,
    /// Effects declared but not observed in the module.
    pub unused: Vec<String>,
}

impl CapabilityManifest {
    /// Render the manifest as canonical JSON.
    pub fn to_json(&self) -> String {
        to_json(self)
    }
}

/// Build the capability manifest for `module`, diffing the observed effects
/// against the `declared_policy` capability list.
pub fn build_capability_manifest(
    module: &Module,
    declared_policy: &[String],
) -> CapabilityManifest {
    let mut by_effect: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for symbol in &module.symbols {
        for eff in &symbol.effects {
            by_effect
                .entry(eff.clone())
                .or_default()
                .insert(symbol.id.clone());
        }
    }
    let effects: Vec<EffectEntry> = by_effect
        .into_iter()
        .map(|(name, uses)| EffectEntry {
            name,
            uses: uses.into_iter().collect(),
        })
        .collect();

    let declared_set: BTreeSet<&String> = declared_policy.iter().collect();
    let used_set: BTreeSet<String> = effects.iter().map(|e| e.name.clone()).collect();
    let undeclared: Vec<String> = used_set
        .iter()
        .filter(|name| !declared_set.contains(name))
        .cloned()
        .collect();
    let unused: Vec<String> = declared_policy
        .iter()
        .filter(|name| !used_set.contains(*name))
        .cloned()
        .collect();

    CapabilityManifest {
        schema: CAPABILITY_SCHEMA,
        module: module.name.clone(),
        effects,
        policy: CapabilityPolicy {
            declared: declared_policy.to_vec(),
            undeclared,
            unused,
        },
    }
}

/// Produce per-symbol effect diagnostics for `module`. Effects not declared
/// in `declared_policy` produce `E0410`; unknown effect names produce `W0401`.
pub fn effect_diagnostics(module: &Module, declared_policy: &[String]) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let declared_set: BTreeSet<&String> = declared_policy.iter().collect();

    for symbol in &module.symbols {
        if symbol.kind == SymbolKind::Module {
            continue;
        }
        for eff in &symbol.effects {
            if !is_known_effect_or_capability(eff) {
                out.push(unknown_effect_diagnostic(
                    symbol.id.as_str(),
                    eff,
                    symbol.span.clone(),
                ));
                continue;
            }
            if !declared_policy.is_empty() && !declared_set.contains(eff) {
                out.push(undeclared_effect_diagnostic(
                    symbol.id.as_str(),
                    eff,
                    symbol.span.clone(),
                ));
            }
        }
    }
    out
}

fn unknown_effect_diagnostic(symbol_id: &str, eff: &str, span: Span) -> Diagnostic {
    Diagnostic::warning(
        "W0401",
        format!("unknown effect or capability `{eff}`"),
        span,
    )
    .with_symbol(symbol_id.to_string())
    .with_expected(
        crate::effects::KNOWN_EFFECTS
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
    )
    .with_found(vec![eff.to_string()])
    .with_agent_summary("Use a known effect name or declare a named capability.")
    .with_docs(vec!["doc:effects.known-effects".to_string()])
}

fn undeclared_effect_diagnostic(symbol_id: &str, eff: &str, span: Span) -> Diagnostic {
    Diagnostic::error(
        "E0410",
        format!(
            "effect `{eff}` is used by `{symbol_id}` but is not in the package capability policy"
        ),
        span,
    )
    .with_symbol(symbol_id.to_string())
    .with_expected(vec![format!("declare `{eff}` in [capabilities].declared")])
    .with_found(vec![eff.to_string()])
    .with_agent_summary("Add the missing capability to ori.toml or remove the effect usage.")
    .with_docs(vec!["doc:effects.policy".to_string()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_source;
    use crate::source::SourceFile;

    fn module_for(text: &str) -> Module {
        parse_source(&SourceFile::new("/t.ori", text)).module
    }

    #[test]
    fn capability_manifest_groups_symbols_by_effect() {
        let module = module_for(
            "module demo\nfn a() -> Unit uses fs.read\nfn b() -> Unit uses fs.read\nfn c() -> Unit uses net.outbound",
        );
        let manifest = build_capability_manifest(&module, &[]);
        let names: Vec<_> = manifest.effects.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"fs.read"));
        assert!(names.contains(&"net.outbound"));
        let fs_read = manifest
            .effects
            .iter()
            .find(|e| e.name == "fs.read")
            .map(|e| e.uses.len())
            .unwrap_or(0);
        assert_eq!(fs_read, 2);
    }

    #[test]
    fn policy_diff_reports_undeclared_effect() {
        let module = module_for("module demo\nfn a() -> Unit uses fs.read");
        let manifest = build_capability_manifest(&module, &["net.outbound".to_string()]);
        assert!(manifest.policy.undeclared.contains(&"fs.read".to_string()));
        assert!(manifest.policy.unused.contains(&"net.outbound".to_string()));
    }

    #[test]
    fn diagnostics_flag_undeclared_effect() {
        let module = module_for("module demo\nfn a() -> Unit uses fs.read");
        let diags = effect_diagnostics(&module, &["net.outbound".to_string()]);
        assert!(diags.iter().any(|d| d.id == "E0410"));
    }

    #[test]
    fn diagnostics_quiet_when_policy_empty() {
        let module = module_for("module demo\nfn a() -> Unit uses fs.read");
        let diags = effect_diagnostics(&module, &[]);
        assert!(!diags.iter().any(|d| d.id == "E0410"));
    }
}
