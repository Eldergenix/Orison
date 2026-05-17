//! Minimal UI manifest extraction with accessibility hints.
//!
//! Treats `view <Name>(props)` declarations as the source of truth for the
//! UI surface and emits a [`UiManifest`] matching
//! `schemas/ui-manifest.schema.json`. Accessibility findings are very
//! conservative (label heuristic only) — the real a11y pass lands when the
//! view-tree IR exists.

use crate::ast::{Module, SymbolKind};
use crate::json::to_json;
use serde::Serialize;

/// Stable schema id for the UI manifest envelope.
pub const UI_MANIFEST_SCHEMA: &str = "ori.ui_manifest.v1";

/// JSON envelope listing every `view` symbol in a module.
#[derive(Debug, Serialize)]
pub struct UiManifest {
    /// Stable schema identifier.
    pub schema: &'static str,
    /// One [`ViewEntry`] per declared view.
    pub views: Vec<ViewEntry>,
}

/// One declared view inside a [`UiManifest`].
#[derive(Debug, Serialize)]
pub struct ViewEntry {
    /// Symbol id of the view.
    pub symbol: String,
    /// View name.
    pub name: String,
    /// Optional route the view is mounted on.
    pub route: Option<String>,
    /// Declared props parsed from the signature.
    pub props: Vec<PropEntry>,
    /// Design tokens referenced by the view (placeholder list for the
    /// bootstrap).
    pub tokens_used: Vec<String>,
    /// Accessibility heuristic findings produced by the bootstrap.
    pub accessibility_findings: Vec<A11yFinding>,
}

/// One prop entry inside a [`ViewEntry`].
#[derive(Debug, Serialize)]
pub struct PropEntry {
    /// Prop binding name.
    pub name: String,
    /// Declared type.
    pub r#type: String,
}

/// One accessibility heuristic finding.
#[derive(Debug, Serialize)]
pub struct A11yFinding {
    /// Severity string (`info`, `warning`, ...).
    pub severity: &'static str,
    /// Human-readable explanation.
    pub message: String,
}

impl UiManifest {
    /// Render the manifest as canonical JSON.
    pub fn to_json(&self) -> String {
        to_json(self)
    }
}

/// Build the UI manifest for `module`.
pub fn build_ui_manifest(module: &Module) -> UiManifest {
    let views: Vec<ViewEntry> = module
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::View)
        .map(|s| {
            let props = parse_props(&s.signature);
            let findings = a11y_findings(&s.name, &props);
            ViewEntry {
                symbol: s.id.clone(),
                name: s.name.clone(),
                route: None,
                props,
                tokens_used: Vec::new(),
                accessibility_findings: findings,
            }
        })
        .collect();
    UiManifest {
        schema: UI_MANIFEST_SCHEMA,
        views,
    }
}

fn parse_props(signature: &str) -> Vec<PropEntry> {
    let open = match signature.find('(') {
        Some(idx) => idx,
        None => return Vec::new(),
    };
    let close = match signature[open..].find(')') {
        Some(off) => open + off,
        None => return Vec::new(),
    };
    let body = &signature[open + 1..close];
    if body.trim().is_empty() {
        return Vec::new();
    }
    body.split(',')
        .filter_map(|part| {
            let part = part.trim();
            if let Some(colon) = part.find(':') {
                let name = part[..colon].trim().to_string();
                let ty = part[colon + 1..].trim().to_string();
                if name.is_empty() || ty.is_empty() {
                    None
                } else {
                    Some(PropEntry { name, r#type: ty })
                }
            } else {
                None
            }
        })
        .collect()
}

fn a11y_findings(view_name: &str, props: &[PropEntry]) -> Vec<A11yFinding> {
    let mut out = Vec::new();
    let lower = view_name.to_lowercase();
    if lower.contains("image") || lower.contains("icon") {
        let has_label = props
            .iter()
            .any(|p| p.name == "alt" || p.name == "label" || p.name == "aria_label");
        if !has_label {
            out.push(A11yFinding {
                severity: "warning",
                message: format!(
                    "view `{view_name}` looks visual but exposes no `alt`/`label`/`aria_label` prop"
                ),
            });
        }
    }
    if lower.contains("form") {
        let has_submit_label = props
            .iter()
            .any(|p| p.name == "submit_label" || p.name == "action_label");
        if !has_submit_label {
            out.push(A11yFinding {
                severity: "info",
                message: format!(
                    "form view `{view_name}` should expose a `submit_label` prop for screen readers"
                ),
            });
        }
    }
    out
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
    fn extracts_props_from_view_signature() {
        let module = module_for("module demo\nview ProductCard(product: Product, badge: Str)");
        let manifest = build_ui_manifest(&module);
        let card = manifest.views.iter().find(|v| v.name == "ProductCard");
        assert!(card.is_some());
        let card_props_len = manifest
            .views
            .iter()
            .find(|v| v.name == "ProductCard")
            .map(|v| v.props.len())
            .unwrap_or(0);
        assert_eq!(card_props_len, 2);
    }

    #[test]
    fn flags_visual_view_missing_label() {
        let module = module_for("module demo\nview HeroImage(src: Str)");
        let manifest = build_ui_manifest(&module);
        let any_finding = manifest
            .views
            .iter()
            .any(|v| !v.accessibility_findings.is_empty());
        assert!(any_finding);
    }

    #[test]
    fn no_finding_when_visual_view_has_alt() {
        let module = module_for("module demo\nview HeroImage(src: Str, alt: Str)");
        let manifest = build_ui_manifest(&module);
        let any_finding = manifest.views.iter().any(|v| {
            v.accessibility_findings
                .iter()
                .any(|f| f.severity == "warning")
        });
        assert!(!any_finding);
    }
}
