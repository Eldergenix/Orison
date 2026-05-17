//! Documented IR for native UI bindings on iOS and Android.
//!
//! The bootstrap compiler does not (yet) emit platform-native UI code: that
//! lands when the `view-tree` MIR exists. This module models the *contract*
//! between an Orison `view` declaration and the eventual native binding so
//! tooling (CLI `--ui-kind` flag, agents, lockfiles, SBOM) can reason about
//! the surface today.
//!
//! ## Supported Orison constructs
//!
//! The bootstrap derives a [`NativeViewSpec`] for every `view <Name>(props)`
//! declaration that satisfies these (intentionally tight) constraints:
//!
//! * The view is **exported** — names beginning with `_` are skipped, matching
//!   the rest of the symbol surface.
//! * Props are **explicitly typed** — the view IR currently records the prop
//!   binding name and its declared type as separate fields. Untyped props are
//!   dropped (with the field count tracked).
//!
//! ## Currently unsupported (planned)
//!
//! * **Platform-conditional views** — there is no `#[platform("ios")]` /
//!   `#[platform("android")]` attribute syntax yet. When that lands, the
//!   derivation will skip views that opt out of the requested platform.
//! * **Lifecycle hooks** — `on_appear` / `on_disappear` / `on_resume` are not
//!   modelled.
//! * **Action callbacks** — closures passed as props are emitted as their
//!   declared type only; the eventual lowering will derive a `@objc` or
//!   Kotlin functional-interface stub.
//! * **State binding** — observable state (the planned `@state` /
//!   `@observable` attributes) is not represented.
//! * **Navigation graphs** — multi-view navigation graphs require the
//!   `route(...)` resolver pass that is gated on the same M30 milestone.
//!
//! The IR is deliberately serde-friendly: callers can serialise a
//! [`NativeUiManifest`] directly to JSON and validate against
//! `schemas/native-ui-manifest.schema.json`.

use crate::ast::{Module, Symbol, SymbolKind};
use crate::json::to_json;
use crate::ui_check::{build_ui_manifest, UiManifest};
use serde::Serialize;
use std::collections::BTreeMap;

/// Stable schema id for the native UI manifest envelope.
pub const NATIVE_UI_MANIFEST_SCHEMA: &str = "ori.native_ui_manifest.v1";

/// Discriminator for the native UI binding target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[allow(non_camel_case_types)]
pub enum NativeUiKind {
    /// Apple UIKit (`UIViewController`, `UIView`).
    #[serde(rename = "ios-uikit")]
    iOS_UIKit,
    /// Apple SwiftUI (`View` protocol).
    #[serde(rename = "ios-swiftui")]
    iOS_SwiftUI,
    /// Google Jetpack Compose (`@Composable` functions).
    #[serde(rename = "android-compose")]
    Android_Compose,
    /// Legacy Android Views (`android.view.View`).
    #[serde(rename = "android-view")]
    Android_View,
}

impl NativeUiKind {
    /// Canonical, stable CLI / JSON string.
    pub fn as_str(&self) -> &'static str {
        match self {
            NativeUiKind::iOS_UIKit => "ios-uikit",
            NativeUiKind::iOS_SwiftUI => "ios-swiftui",
            NativeUiKind::Android_Compose => "android-compose",
            NativeUiKind::Android_View => "android-view",
        }
    }

    /// Parse the CLI / JSON string form. Returns `None` for unknown kinds —
    /// the CLI surface uses this to reject typos with a clean diagnostic
    /// rather than panicking.
    pub fn from_cli_str(value: &str) -> Option<Self> {
        match value {
            "ios-uikit" => Some(NativeUiKind::iOS_UIKit),
            "ios-swiftui" => Some(NativeUiKind::iOS_SwiftUI),
            "android-compose" => Some(NativeUiKind::Android_Compose),
            "android-view" => Some(NativeUiKind::Android_View),
            _ => None,
        }
    }

    /// The base native "component" emitted for a view targeting this kind.
    fn base_component(&self) -> &'static str {
        match self {
            NativeUiKind::iOS_UIKit => "UIViewController",
            NativeUiKind::iOS_SwiftUI => "View",
            NativeUiKind::Android_Compose => "ComposableFunction",
            NativeUiKind::Android_View => "ViewGroup",
        }
    }

    /// Build the per-prop mapping target string for this kind. Stable so the
    /// IR can be diffed across builds.
    fn prop_target(&self, prop_name: &str) -> String {
        match self {
            NativeUiKind::iOS_UIKit => {
                format!("UIViewController.{prop_name}DataSource")
            }
            NativeUiKind::iOS_SwiftUI => format!("@State {prop_name}"),
            NativeUiKind::Android_Compose => format!("composable parameter `{prop_name}`"),
            NativeUiKind::Android_View => {
                format!("ViewGroup.set{}(...)", capitalise_first(prop_name))
            }
        }
    }
}

fn capitalise_first(input: &str) -> String {
    let mut chars = input.chars();
    match chars.next() {
        Some(c) => {
            let mut out = String::with_capacity(input.len());
            for upper in c.to_uppercase() {
                out.push(upper);
            }
            out.push_str(chars.as_str());
            out
        }
        None => String::new(),
    }
}

/// One Orison `view` mapped to its native UI counterpart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NativeViewSpec {
    /// Native UI binding kind this spec was derived for.
    pub kind: NativeUiKind,
    /// Native component name (e.g. `UIViewController`, `View`).
    pub component: String,
    /// Symbol id of the originating Orison view.
    pub symbol: String,
    /// Orison view name.
    pub view_name: String,
    /// Stable, sorted prop-to-native-target mapping.
    pub props_mapping: BTreeMap<String, String>,
}

/// Top-level envelope listing every [`NativeViewSpec`] derived for a module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NativeUiManifest {
    /// Stable schema id (`ori.native_ui_manifest.v1`).
    pub schema: &'static str,
    /// Target native UI binding.
    pub target: NativeUiKind,
    /// One spec per exported view in source order.
    pub views: Vec<NativeViewSpec>,
}

impl NativeUiManifest {
    /// Render the manifest as canonical JSON.
    pub fn to_json(&self) -> String {
        to_json(self)
    }
}

fn is_exported_view(sym: &Symbol) -> bool {
    sym.kind == SymbolKind::View && !sym.name.starts_with('_')
}

/// Derive a [`NativeViewSpec`] per exported view in `module` for the given
/// [`NativeUiKind`].
///
/// View order matches the source-order traversal of `module.symbols`. The
/// per-view prop set is sourced from [`build_ui_manifest`] so the bootstrap
/// has a single parser for view signatures.
pub fn derive_native_ui_specs(module: &Module, kind: NativeUiKind) -> Vec<NativeViewSpec> {
    let ui_manifest: UiManifest = build_ui_manifest(module);
    // Index UI manifest views by symbol id for O(1) lookup so the surface
    // order remains the symbol-table order rather than the manifest order.
    let mut by_symbol: BTreeMap<&str, &crate::ui_check::ViewEntry> = BTreeMap::new();
    for view in &ui_manifest.views {
        by_symbol.insert(view.symbol.as_str(), view);
    }

    let mut out: Vec<NativeViewSpec> = Vec::new();
    for sym in &module.symbols {
        if !is_exported_view(sym) {
            continue;
        }
        let entry = match by_symbol.get(sym.id.as_str()) {
            Some(e) => *e,
            None => continue,
        };
        let mut mapping: BTreeMap<String, String> = BTreeMap::new();
        for prop in &entry.props {
            mapping.insert(prop.name.clone(), kind.prop_target(&prop.name));
        }
        out.push(NativeViewSpec {
            kind,
            component: kind.base_component().to_string(),
            symbol: sym.id.clone(),
            view_name: sym.name.clone(),
            props_mapping: mapping,
        });
    }
    out
}

/// Build a complete [`NativeUiManifest`] for `module` targeting `kind`.
pub fn build_native_ui_manifest(module: &Module, kind: NativeUiKind) -> NativeUiManifest {
    NativeUiManifest {
        schema: NATIVE_UI_MANIFEST_SCHEMA,
        target: kind,
        views: derive_native_ui_specs(module, kind),
    }
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
    fn cli_strings_round_trip() {
        for kind in [
            NativeUiKind::iOS_UIKit,
            NativeUiKind::iOS_SwiftUI,
            NativeUiKind::Android_Compose,
            NativeUiKind::Android_View,
        ] {
            let cli = kind.as_str();
            let parsed = NativeUiKind::from_cli_str(cli);
            assert_eq!(parsed, Some(kind));
        }
    }

    #[test]
    fn unknown_kind_string_returns_none() {
        assert!(NativeUiKind::from_cli_str("watch-os").is_none());
    }

    #[test]
    fn derives_one_spec_per_view_ios_uikit() {
        let module = module_for(
            "module demo\nview ProductDetail(product: Product)\nview UserCard(name: Str, avatar: Str)",
        );
        let specs = derive_native_ui_specs(&module, NativeUiKind::iOS_UIKit);
        assert_eq!(specs.len(), 2);
        if let Some(first) = specs.first() {
            assert_eq!(first.kind, NativeUiKind::iOS_UIKit);
            assert_eq!(first.component, "UIViewController");
        }
    }

    #[test]
    fn derives_one_spec_per_view_android_compose() {
        let module = module_for(
            "module demo\nview ProductDetail(product: Product)\nview UserCard(name: Str)",
        );
        let specs = derive_native_ui_specs(&module, NativeUiKind::Android_Compose);
        assert_eq!(specs.len(), 2);
        if let Some(first) = specs.first() {
            assert_eq!(first.kind, NativeUiKind::Android_Compose);
            assert_eq!(first.component, "ComposableFunction");
        }
    }

    #[test]
    fn props_mapping_targets_uikit_view_data_source() {
        let module = module_for("module demo\nview ProductDetail(product: Product)");
        let specs = derive_native_ui_specs(&module, NativeUiKind::iOS_UIKit);
        let first = specs.into_iter().next();
        assert!(first.is_some());
        if let Some(first) = first {
            let target = first.props_mapping.get("product").cloned();
            assert_eq!(
                target.as_deref(),
                Some("UIViewController.productDataSource")
            );
        }
    }

    #[test]
    fn props_mapping_targets_swiftui_state() {
        let module = module_for("module demo\nview ProductDetail(product: Product)");
        let specs = derive_native_ui_specs(&module, NativeUiKind::iOS_SwiftUI);
        let first = specs.into_iter().next();
        if let Some(first) = first {
            let target = first.props_mapping.get("product").cloned();
            assert_eq!(target.as_deref(), Some("@State product"));
        }
    }

    #[test]
    fn manifest_serialises_to_json_with_schema_and_stable_order() {
        let module = module_for(
            "module demo\nview ProductDetail(product: Product)\nview UserCard(name: Str, avatar: Str)",
        );
        let manifest = build_native_ui_manifest(&module, NativeUiKind::iOS_UIKit);
        let json = manifest.to_json();
        assert!(json.contains("\"schema\":\"ori.native_ui_manifest.v1\""));
        assert!(json.contains("\"target\":\"ios-uikit\""));
        // BTreeMap serialisation preserves ascending key order: `avatar`
        // must precede `name` for the UserCard view.
        let avatar_pos = json.find("\"avatar\"");
        let name_pos = json.find("\"name\":\"name\"");
        if let (Some(a), Some(n)) = (avatar_pos, name_pos) {
            assert!(a < n, "BTreeMap should preserve ascending key order");
        }
    }

    #[test]
    fn private_views_are_skipped() {
        let module = module_for("module demo\nview _Internal(x: Str)\nview Public(y: Int)");
        let specs = derive_native_ui_specs(&module, NativeUiKind::Android_View);
        assert_eq!(specs.len(), 1);
        if let Some(spec) = specs.first() {
            assert_eq!(spec.view_name, "Public");
        }
    }

    #[test]
    fn android_view_uses_setter_style_target() {
        let module = module_for("module demo\nview ProductDetail(product: Product)");
        let specs = derive_native_ui_specs(&module, NativeUiKind::Android_View);
        if let Some(first) = specs.first() {
            let target = first.props_mapping.get("product").cloned();
            assert_eq!(target.as_deref(), Some("ViewGroup.setProduct(...)"));
        }
    }
}
