//! Mobile-target manifest generation (M15 bootstrap).
//!
//! Given a parsed [`Module`], derive a [`MobileManifest`] suitable for
//! mobile (`ios`, `android`) deployment. The manifest mirrors
//! `schemas/mobile-manifest.schema.json`. Permissions are auto-derived
//! from declared effects; capabilities are a sorted, de-duplicated union
//! of every effect declared on every symbol.
//!
//! The companion [`check_permission_required`] helper exposes a small
//! per-platform table of permissions that the bootstrap considers
//! required (i.e. the OS would otherwise refuse the capability). The
//! [`validate_manifest`] entry point surfaces three diagnostics:
//!
//! * `MOB0001` — platform does not support a derived permission
//! * `MOB0002` — unsupported platform (not `ios` / `android`)
//! * `MOB0003` — invalid app id (not in reverse-DNS form)
//!
//! The implementation is intentionally panic-free: all fallible work is
//! expressed through [`Result`]/[`Option`] and never via
//! `unwrap`/`expect`/`panic!`.

use crate::ast::{Module, SymbolKind};
use crate::diagnostic::Diagnostic;
use crate::json::to_json;
use crate::mobile_ui_ir::{build_native_ui_manifest, NativeUiKind, NativeUiManifest};
use crate::source::Span;
use serde::Serialize;
use std::collections::BTreeSet;

/// Maximum length (in chars, not bytes) of a permission justification.
const JUSTIFICATION_MAX_CHARS: usize = 200;

/// Supported mobile platforms — the authoritative allow-list used by the
/// CLI flag parser and by [`validate_manifest`].
pub const SUPPORTED_PLATFORMS: &[&str] = &["ios", "android"];

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct MobileManifest {
    pub schema: &'static str,
    pub app_id: String,
    pub platforms: Vec<String>,
    pub permissions: Vec<Permission>,
    pub capabilities: Vec<String>,
    pub entrypoints: Vec<String>,
    /// Optional native UI binding manifest. Skipped from JSON when `None`
    /// so the existing `ori.mobile_manifest.v1` shape is unchanged for
    /// callers that do not request a native UI target.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub native_ui_manifest: Option<NativeUiManifest>,
}

#[derive(Debug, Serialize, PartialEq, Eq, Clone)]
pub struct Permission {
    pub key: String,
    pub justification: String,
}

impl MobileManifest {
    pub fn to_json(&self) -> String {
        to_json(self)
    }
}

/// Map a single declared effect name onto a derived permission key. Returns
/// `None` for effect names that should be silently skipped (currently never
/// — unknown effects pass through as their own key).
fn effect_to_permission_key(effect: &str) -> &str {
    match effect {
        "net.outbound" | "net.inbound" | "http" | "net" => "network",
        "fs.read" | "fs.write" | "fs" => "storage",
        "db.read" | "db.write" | "db" => "database",
        "ui" => "display",
        "time" => "clock",
        "crypto" => "keychain",
        "auth" => "authentication",
        other => other,
    }
}

/// Truncate justification to [`JUSTIFICATION_MAX_CHARS`] chars, replacing
/// the tail with an ellipsis when truncation actually shortens the string.
fn truncate_justification(input: String) -> String {
    let mut byte_cutoff: Option<usize> = None;
    for (count, (idx, _ch)) in input.char_indices().enumerate() {
        if count == JUSTIFICATION_MAX_CHARS {
            byte_cutoff = Some(idx);
            break;
        }
    }
    match byte_cutoff {
        Some(cut) => {
            let mut out = String::with_capacity(cut + 3);
            out.push_str(&input[..cut]);
            // Replace the last 3 chars with an ellipsis when room allows so
            // truncation is visually obvious.
            if out.len() >= 3 {
                let trim_start = out
                    .char_indices()
                    .rev()
                    .nth(2)
                    .map(|(i, _)| i)
                    .unwrap_or(out.len());
                out.truncate(trim_start);
                out.push_str("...");
            }
            out
        }
        None => input,
    }
}

/// Build a justification string from the originating effect, then truncate
/// to the configured maximum.
fn auto_justification(effect: &str) -> String {
    truncate_justification(format!("auto-derived from declared effect {effect}"))
}

/// Build a mobile manifest from a parsed module. Permissions are
/// de-duplicated case-sensitively by key and ordered by first appearance
/// for deterministic output.
pub fn build_mobile_manifest(module: &Module, app_id: &str, platforms: &[&str]) -> MobileManifest {
    // Collect effects in order of first appearance so the derived
    // permission list is deterministic, then dedupe case-sensitively on
    // permission key.
    let mut seen_keys: BTreeSet<String> = BTreeSet::new();
    let mut permissions: Vec<Permission> = Vec::new();
    let mut capabilities: BTreeSet<String> = BTreeSet::new();
    let mut entrypoints: BTreeSet<String> = BTreeSet::new();

    for sym in &module.symbols {
        // Entrypoints: every exported function / service / query is a
        // candidate mobile entrypoint. The set is sorted (BTreeSet) so the
        // output is stable across runs.
        if matches!(
            sym.kind,
            SymbolKind::Function | SymbolKind::Service | SymbolKind::Query | SymbolKind::View
        ) && !sym.name.starts_with('_')
        {
            entrypoints.insert(sym.id.clone());
        }
        for eff in &sym.effects {
            capabilities.insert(eff.clone());
            let key = effect_to_permission_key(eff).to_string();
            if seen_keys.insert(key.clone()) {
                permissions.push(Permission {
                    key,
                    justification: auto_justification(eff),
                });
            }
        }
    }

    // Platforms: preserve caller order but dedupe case-sensitively. The
    // schema requires uniqueItems.
    let mut platform_seen: BTreeSet<String> = BTreeSet::new();
    let mut platforms_out: Vec<String> = Vec::new();
    for plat in platforms {
        let key = (*plat).to_string();
        if platform_seen.insert(key.clone()) {
            platforms_out.push(key);
        }
    }

    MobileManifest {
        schema: "ori.mobile_manifest.v1",
        app_id: app_id.to_string(),
        platforms: platforms_out,
        permissions,
        capabilities: capabilities.into_iter().collect(),
        entrypoints: entrypoints.into_iter().collect(),
        native_ui_manifest: None,
    }
}

/// Build a mobile manifest with an optional embedded native UI manifest.
///
/// Passing `None` produces the same JSON shape as [`build_mobile_manifest`]
/// (the `native_ui_manifest` field is skipped from serialisation). Passing
/// `Some(kind)` runs [`build_native_ui_manifest`] over the module and embeds
/// the result so consumers can drive native bindings from a single artefact.
pub fn build_mobile_manifest_with_ui(
    module: &Module,
    app_id: &str,
    platforms: &[&str],
    ui_kind: Option<NativeUiKind>,
) -> MobileManifest {
    let mut manifest = build_mobile_manifest(module, app_id, platforms);
    manifest.native_ui_manifest = ui_kind.map(|kind| build_native_ui_manifest(module, kind));
    manifest
}

/// Is the given permission *required* on the given platform (i.e. would
/// the OS otherwise refuse to grant the capability silently)? The table
/// is intentionally a small hardcoded subset suitable for the bootstrap.
pub fn check_permission_required(platform: &str, permission_key: &str) -> bool {
    let ios_required: &[&str] = &[
        "network",
        "storage",
        "display",
        "authentication",
        "keychain",
    ];
    let android_required: &[&str] = &[
        "network",
        "storage",
        "database",
        "display",
        "authentication",
    ];
    match platform {
        "ios" => ios_required.contains(&permission_key),
        "android" => android_required.contains(&permission_key),
        _ => false,
    }
}

/// Is the given permission *supported* (i.e. understood) on the platform?
/// Anything not in this table is rejected with [`MOB0001`].
fn permission_supported(platform: &str, permission_key: &str) -> bool {
    let ios_supported: &[&str] = &[
        "network",
        "storage",
        "display",
        "authentication",
        "keychain",
        "clock",
        "database",
    ];
    // Android intentionally does NOT list `keychain`: keystore-style
    // secret storage is provided by a different surface and callers who
    // declared `crypto` must either target iOS or opt in via an
    // Android-specific capability.
    let android_supported: &[&str] = &[
        "network",
        "storage",
        "display",
        "authentication",
        "clock",
        "database",
    ];
    match platform {
        "ios" => ios_supported.contains(&permission_key),
        "android" => android_supported.contains(&permission_key),
        _ => false,
    }
}

fn is_supported_platform(platform: &str) -> bool {
    SUPPORTED_PLATFORMS.contains(&platform)
}

/// Reverse-DNS form (`com.example.app`) is required for portability with
/// both Apple's CFBundleIdentifier and Android's package id rules. We
/// require at least 2 dot-separated segments, each non-empty, starting
/// with a letter, and containing only ASCII letters, digits, and `_`.
fn is_valid_app_id(app_id: &str) -> bool {
    if app_id.is_empty() {
        return false;
    }
    let segments: Vec<&str> = app_id.split('.').collect();
    if segments.len() < 2 {
        return false;
    }
    for seg in &segments {
        if seg.is_empty() {
            return false;
        }
        let mut chars = seg.chars();
        let first = match chars.next() {
            Some(c) => c,
            None => return false,
        };
        if !first.is_ascii_alphabetic() {
            return false;
        }
        for ch in chars {
            if !(ch.is_ascii_alphanumeric() || ch == '_') {
                return false;
            }
        }
    }
    true
}

/// Validate a built manifest, emitting `MOB0001` / `MOB0002` / `MOB0003`
/// diagnostics. The span is a dummy span tagged with the module path so
/// callers can still attribute findings to a source file.
pub fn validate_manifest(manifest: &MobileManifest, source_path: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let span = Span::dummy(source_path.to_string());

    if !is_valid_app_id(&manifest.app_id) {
        diagnostics.push(
            Diagnostic::error(
                "MOB0003",
                format!(
                    "invalid app id `{}` (expected reverse-DNS form like `com.example.app`)",
                    manifest.app_id
                ),
                span.clone(),
            )
            .with_expected(vec!["com.example.app".to_string()])
            .with_found(vec![manifest.app_id.clone()])
            .with_agent_summary("Use a reverse-DNS app id with at least two segments.")
            .with_docs(vec!["doc:mobile.app-id".to_string()]),
        );
    }

    for plat in &manifest.platforms {
        if !is_supported_platform(plat) {
            diagnostics.push(
                Diagnostic::error(
                    "MOB0002",
                    format!("unsupported platform `{plat}` (expected `ios` or `android`)"),
                    span.clone(),
                )
                .with_expected(
                    SUPPORTED_PLATFORMS
                        .iter()
                        .map(|p| (*p).to_string())
                        .collect(),
                )
                .with_found(vec![plat.clone()])
                .with_agent_summary("Restrict --platforms to ios and/or android.")
                .with_docs(vec!["doc:mobile.platforms".to_string()]),
            );
        }
    }

    for plat in &manifest.platforms {
        if !is_supported_platform(plat) {
            continue;
        }
        for perm in &manifest.permissions {
            if !permission_supported(plat, &perm.key) {
                diagnostics.push(
                    Diagnostic::error(
                        "MOB0001",
                        format!(
                            "platform `{plat}` does not support permission `{}`",
                            perm.key
                        ),
                        span.clone(),
                    )
                    .with_expected(vec![format!("a permission supported by {plat}")])
                    .with_found(vec![perm.key.clone()])
                    .with_agent_summary(
                        "Remove the unsupported effect or target a different platform.",
                    )
                    .with_docs(vec!["doc:mobile.permissions".to_string()]),
                );
            }
        }
    }

    diagnostics
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
    fn net_outbound_produces_network_permission() {
        let module = module_for("module demo\nfn ping() -> Unit uses net.outbound");
        let m = build_mobile_manifest(&module, "com.example.demo", &["ios"]);
        assert!(m.permissions.iter().any(|p| p.key == "network"));
    }

    #[test]
    fn ui_in_view_produces_display_permission() {
        let module = module_for("module demo\nview Hero(text: Str) uses ui");
        let m = build_mobile_manifest(&module, "com.example.demo", &["ios"]);
        assert!(m.permissions.iter().any(|p| p.key == "display"));
    }

    #[test]
    fn capabilities_are_sorted() {
        let module = module_for(
            "module demo\nfn a() -> Unit uses ui\nfn b() -> Unit uses net.outbound\nfn c() -> Unit uses db.read",
        );
        let m = build_mobile_manifest(&module, "com.example.demo", &["ios", "android"]);
        let mut sorted = m.capabilities.clone();
        sorted.sort();
        assert_eq!(
            m.capabilities, sorted,
            "capabilities must be sorted ascending"
        );
    }

    #[test]
    fn multiple_effects_produce_distinct_permissions() {
        let module = module_for(
            "module demo\nfn a() -> Unit uses net.outbound\nfn b() -> Unit uses ui\nfn c() -> Unit uses db.read",
        );
        let m = build_mobile_manifest(&module, "com.example.demo", &["android"]);
        let keys: Vec<&str> = m.permissions.iter().map(|p| p.key.as_str()).collect();
        assert!(keys.contains(&"network"));
        assert!(keys.contains(&"display"));
        assert!(keys.contains(&"database"));
        // Deduplicated: no key appears twice.
        let mut sorted_keys = keys.clone();
        sorted_keys.sort_unstable();
        let original_len = sorted_keys.len();
        sorted_keys.dedup();
        assert_eq!(original_len, sorted_keys.len());
    }

    #[test]
    fn duplicate_effects_are_deduplicated() {
        let module = module_for(
            "module demo\nfn a() -> Unit uses net.outbound\nfn b() -> Unit uses net.outbound, http",
        );
        let m = build_mobile_manifest(&module, "com.example.demo", &["ios"]);
        let network_count = m.permissions.iter().filter(|p| p.key == "network").count();
        assert_eq!(network_count, 1);
    }

    #[test]
    fn unknown_effect_produces_auto_derived_permission() {
        // "Telemetry" begins with uppercase so the warning gate accepts it
        // as a capability identifier; the mobile pass treats unknown
        // effects as their own permission key.
        let module = module_for("module demo\nfn t() -> Unit uses Telemetry");
        let m = build_mobile_manifest(&module, "com.example.demo", &["ios"]);
        let matched: Vec<&Permission> = m
            .permissions
            .iter()
            .filter(|p| p.key == "Telemetry")
            .collect();
        assert_eq!(
            matched.len(),
            1,
            "expected exactly one Telemetry permission"
        );
        if let Some(perm) = matched.first() {
            assert!(perm.justification.starts_with("auto-derived from declared"));
        }
    }

    #[test]
    fn ios_and_android_are_both_supported() {
        let module = module_for("module demo\nfn a() -> Unit uses net.outbound");
        let m = build_mobile_manifest(&module, "com.example.demo", &["ios", "android"]);
        let diags = validate_manifest(&m, "/t.ori");
        assert!(diags.iter().all(|d| d.id != "MOB0002"));
        assert!(m.platforms.contains(&"ios".to_string()));
        assert!(m.platforms.contains(&"android".to_string()));
    }

    #[test]
    fn mob0001_fires_for_unsupported_permission_on_platform() {
        // `clock` is supported (so we instead test a permission iOS does
        // not support). The `keychain` permission is supported by iOS but
        // NOT by android — see `permission_supported`.
        let module = module_for("module demo\nfn t() -> Unit uses crypto");
        let m = build_mobile_manifest(&module, "com.example.demo", &["android"]);
        let diags = validate_manifest(&m, "/t.ori");
        assert!(
            diags.iter().any(|d| d.id == "MOB0001"),
            "expected MOB0001 for android+keychain, got: {diags:?}"
        );
    }

    #[test]
    fn mob0002_fires_for_unknown_platform() {
        let module = module_for("module demo\nfn a() -> Unit uses net.outbound");
        let m = build_mobile_manifest(&module, "com.example.demo", &["bsd"]);
        let diags = validate_manifest(&m, "/t.ori");
        assert!(diags.iter().any(|d| d.id == "MOB0002"));
    }

    #[test]
    fn mob0003_fires_for_non_reverse_dns_app_id() {
        let module = module_for("module demo\nfn a() -> Unit");
        let m = build_mobile_manifest(&module, "notreversedns", &["ios"]);
        let diags = validate_manifest(&m, "/t.ori");
        assert!(diags.iter().any(|d| d.id == "MOB0003"));
    }

    #[test]
    fn justification_is_truncated_to_max_chars() {
        // Effect name pushed past 200 chars to force truncation.
        let long_effect = "a".repeat(300);
        let perm_key = effect_to_permission_key(&long_effect).to_string();
        assert_eq!(perm_key, long_effect);
        let just = auto_justification(&long_effect);
        let char_count = just.chars().count();
        assert!(
            char_count <= JUSTIFICATION_MAX_CHARS,
            "justification has {char_count} chars; expected <= {JUSTIFICATION_MAX_CHARS}"
        );
    }

    #[test]
    fn manifest_serialises_with_schema() {
        let module = module_for("module demo\nfn a() -> Unit");
        let json = build_mobile_manifest(&module, "com.example.demo", &["ios"]).to_json();
        assert!(json.contains("\"schema\":\"ori.mobile_manifest.v1\""));
        assert!(json.contains("\"app_id\":\"com.example.demo\""));
    }

    #[test]
    fn default_manifest_omits_native_ui_manifest_field() {
        let module = module_for("module demo\nview Hero(text: Str) uses ui");
        let json = build_mobile_manifest(&module, "com.example.demo", &["ios"]).to_json();
        assert!(
            !json.contains("native_ui_manifest"),
            "default manifest must omit the optional native_ui_manifest field"
        );
    }

    #[test]
    fn with_ui_kind_embeds_native_ui_manifest() {
        let module = module_for("module demo\nview Hero(text: Str)");
        let manifest = build_mobile_manifest_with_ui(
            &module,
            "com.example.demo",
            &["ios"],
            Some(NativeUiKind::iOS_UIKit),
        );
        assert!(manifest.native_ui_manifest.is_some());
        let json = manifest.to_json();
        assert!(json.contains("\"native_ui_manifest\""));
        assert!(json.contains("\"target\":\"ios-uikit\""));
    }

    #[test]
    fn with_none_ui_kind_omits_native_ui_manifest_field() {
        let module = module_for("module demo\nview Hero(text: Str)");
        let manifest = build_mobile_manifest_with_ui(&module, "com.example.demo", &["ios"], None);
        assert!(manifest.native_ui_manifest.is_none());
        let json = manifest.to_json();
        assert!(!json.contains("native_ui_manifest"));
    }
}
