//! Desktop-target manifest generation (M30 bootstrap).
//!
//! Given a parsed [`Module`], derive a [`DesktopManifest`] suitable for
//! macOS / Linux / Windows deployment. The manifest mirrors
//! `schemas/desktop-manifest.schema.json` and the embedded report mirrors
//! the `ori.desktop_build_report.v1` envelope.
//!
//! This pass deliberately mirrors the M15 mobile pipeline (`mobile.rs`):
//! capabilities are derived from the union of every effect declared on
//! every symbol; an enum of three platforms keeps the build target
//! authoritatively typed.
//!
//! Five diagnostic ids surface as [`DesktopError`] variants:
//!
//! * `DSK0001` — invalid bundle id (must match reverse-DNS form).
//! * `DSK0002` — unsupported platform (defensive — the enum prevents most
//!   misuse, but the variant is preserved for future external callers).
//! * `DSK0003` — capability declared on the module is unavailable on the
//!   requested platform (e.g. iOS-only `keychain`).
//! * `DSK0004` — binary target unsupported on the requested platform
//!   (e.g. `riscv64` on macOS).
//! * `DSK0005` — `product_name` is empty.
//!
//! The implementation is panic-free: every fallible path is expressed via
//! [`Result`] / [`Option`].

use crate::ast::{Module, SymbolKind};
use crate::json::to_json;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Supported desktop platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DesktopPlatform {
    #[serde(rename = "macos")]
    MacOS,
    #[serde(rename = "linux")]
    Linux,
    #[serde(rename = "windows")]
    Windows,
}

impl DesktopPlatform {
    /// Canonical wire-format string for the platform.
    pub fn as_str(&self) -> &'static str {
        match self {
            DesktopPlatform::MacOS => "macos",
            DesktopPlatform::Linux => "linux",
            DesktopPlatform::Windows => "windows",
        }
    }

    /// Parse a CLI-form platform string.
    pub fn from_cli_str(s: &str) -> Option<Self> {
        match s {
            "macos" | "mac" | "darwin" => Some(DesktopPlatform::MacOS),
            "linux" => Some(DesktopPlatform::Linux),
            "windows" | "win" => Some(DesktopPlatform::Windows),
            _ => None,
        }
    }
}

/// The desktop manifest shape consumed by downstream packagers.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DesktopManifest {
    pub schema: &'static str,
    pub platform: DesktopPlatform,
    pub bundle_id: String,
    pub product_name: String,
    pub version: String,
    pub binary_targets: Vec<String>,
    pub entitlements: BTreeMap<String, String>,
    pub linux_categories: Vec<String>,
    pub windows_subsystem: Option<String>,
    pub capabilities_required: BTreeSet<String>,
}

/// One artefact produced by the desktop build pipeline.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DesktopArtefact {
    pub path: String,
    pub kind: String,
    pub bytes_estimate: u64,
}

/// Top-level build report envelope.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DesktopBuildReport {
    pub schema: &'static str,
    pub manifest: DesktopManifest,
    pub artefacts: Vec<DesktopArtefact>,
    pub warnings: Vec<String>,
}

/// All errors the desktop pass can raise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopError {
    /// DSK0001 — bundle id is not in reverse-DNS form.
    BundleIdInvalid { bundle_id: String },
    /// DSK0002 — caller passed an unrecognised platform.
    UnsupportedPlatform { platform: String },
    /// DSK0003 — capability is iOS/Android only and not available on the
    /// requested desktop platform.
    CapabilityUnavailableOnPlatform {
        capability: String,
        platform: DesktopPlatform,
    },
    /// DSK0004 — binary target architecture is unsupported on this
    /// platform.
    BinaryTargetUnsupported {
        target: String,
        platform: DesktopPlatform,
    },
    /// DSK0005 — `product_name` is empty.
    MissingProductName,
}

impl DesktopError {
    /// Stable diagnostic id (`DSK0001`..`DSK0005`).
    pub fn id(&self) -> &'static str {
        match self {
            DesktopError::BundleIdInvalid { .. } => "DSK0001",
            DesktopError::UnsupportedPlatform { .. } => "DSK0002",
            DesktopError::CapabilityUnavailableOnPlatform { .. } => "DSK0003",
            DesktopError::BinaryTargetUnsupported { .. } => "DSK0004",
            DesktopError::MissingProductName => "DSK0005",
        }
    }

    /// Human-readable message.
    pub fn message(&self) -> String {
        match self {
            DesktopError::BundleIdInvalid { bundle_id } => format!(
                "invalid bundle id `{bundle_id}` (expected reverse-DNS form like `com.example.app`)"
            ),
            DesktopError::UnsupportedPlatform { platform } => format!(
                "unsupported desktop platform `{platform}` (expected `macos`, `linux`, or `windows`)"
            ),
            DesktopError::CapabilityUnavailableOnPlatform {
                capability,
                platform,
            } => format!(
                "capability `{capability}` is not available on platform `{}`",
                platform.as_str()
            ),
            DesktopError::BinaryTargetUnsupported { target, platform } => format!(
                "binary target `{target}` is unsupported on platform `{}`",
                platform.as_str()
            ),
            DesktopError::MissingProductName => {
                "product_name is empty (a non-empty product_name is required)".to_string()
            }
        }
    }
}

impl std::fmt::Display for DesktopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.id(), self.message())
    }
}

impl std::error::Error for DesktopError {}

/// Reverse-DNS bundle id (lowercase). At least two segments, each starting
/// with `[a-z]`, optionally followed by `[a-z0-9-]`.
///
/// Equivalent regex: `^[a-z][a-z0-9-]*(\.[a-z][a-z0-9-]*)+$`.
fn is_valid_bundle_id(bundle_id: &str) -> bool {
    if bundle_id.is_empty() {
        return false;
    }
    let segments: Vec<&str> = bundle_id.split('.').collect();
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
        if !(first.is_ascii_lowercase()) {
            return false;
        }
        for ch in chars {
            let ok = ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-';
            if !ok {
                return false;
            }
        }
    }
    true
}

/// Per-platform allow-lists for binary target triples.
fn supported_binary_targets(platform: DesktopPlatform) -> &'static [&'static str] {
    match platform {
        DesktopPlatform::MacOS => &["x86_64", "aarch64"],
        DesktopPlatform::Linux => &["x86_64", "aarch64", "i686"],
        DesktopPlatform::Windows => &["x86_64", "aarch64"],
    }
}

/// Capabilities that are *exclusive* to mobile platforms — these cannot be
/// declared on a desktop target. Anything not in this list passes through.
fn capability_is_mobile_only(capability: &str) -> bool {
    matches!(
        capability,
        // iOS/Android specific surfaces.
        "ios.push"
            | "android.push"
            | "ios.haptics"
            | "android.haptics"
            | "ios.biometrics"
            | "android.biometrics"
            | "mobile.camera"
            | "mobile.location"
            | "mobile.contacts"
            | "mobile.calendar"
            | "mobile.bluetooth"
            | "mobile.sensors"
    )
}

/// Collect the sorted union of every effect declared on every non-`_`
/// symbol in the module.
fn collect_capabilities(module: &Module) -> BTreeSet<String> {
    let mut capabilities: BTreeSet<String> = BTreeSet::new();
    for sym in &module.symbols {
        if sym.kind == SymbolKind::Module {
            continue;
        }
        for eff in &sym.effects {
            capabilities.insert(eff.clone());
        }
    }
    capabilities
}

/// Derive a default product_name from a module name (`store.users` →
/// `users`). Returns an empty string when the module name is empty so
/// `DSK0005` can fire for genuinely missing names.
fn default_product_name(module_name: &str) -> String {
    match module_name.rsplit('.').next() {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => String::new(),
    }
}

/// Derive a default bundle id from a module name (`store.users` →
/// `app.store.users`).
fn default_bundle_id(module_name: &str) -> String {
    if module_name.is_empty() {
        return "app".to_string();
    }
    format!("app.{module_name}")
}

/// Build a [`DesktopManifest`] from a parsed module for the given
/// [`DesktopPlatform`]. The function pre-fills sensible per-platform
/// defaults (Windows subsystem = `console`, etc.) and then runs every
/// shape rule. Any rule failure yields a [`DesktopError`].
pub fn build_manifest(
    module: &Module,
    platform: DesktopPlatform,
) -> Result<DesktopManifest, DesktopError> {
    let product_name = default_product_name(&module.name);
    let bundle_id = default_bundle_id(&module.name);
    let capabilities = collect_capabilities(module);

    // Default binary targets: a single 64-bit target for every platform so
    // bootstrap users have a reasonable starting point.
    let binary_targets: Vec<String> = vec!["x86_64".to_string()];

    let windows_subsystem = if platform == DesktopPlatform::Windows {
        Some("console".to_string())
    } else {
        None
    };

    let manifest = DesktopManifest {
        schema: "ori.desktop_manifest.v1",
        platform,
        bundle_id,
        product_name,
        version: "0.0.0".to_string(),
        binary_targets,
        entitlements: BTreeMap::new(),
        linux_categories: Vec::new(),
        windows_subsystem,
        capabilities_required: capabilities,
    };

    validate_manifest(&manifest)?;
    Ok(manifest)
}

/// Apply every per-platform shape rule and surface the first violation.
fn validate_manifest(manifest: &DesktopManifest) -> Result<(), DesktopError> {
    if manifest.product_name.is_empty() {
        return Err(DesktopError::MissingProductName);
    }
    if !is_valid_bundle_id(&manifest.bundle_id) {
        return Err(DesktopError::BundleIdInvalid {
            bundle_id: manifest.bundle_id.clone(),
        });
    }
    let allowed = supported_binary_targets(manifest.platform);
    for target in &manifest.binary_targets {
        if !allowed.contains(&target.as_str()) {
            return Err(DesktopError::BinaryTargetUnsupported {
                target: target.clone(),
                platform: manifest.platform,
            });
        }
    }
    for cap in &manifest.capabilities_required {
        if capability_is_mobile_only(cap) {
            return Err(DesktopError::CapabilityUnavailableOnPlatform {
                capability: cap.clone(),
                platform: manifest.platform,
            });
        }
    }
    Ok(())
}

/// Convenience constructor for direct callers that want to assemble a
/// manifest by hand and still get validation. The bootstrap CLI does not
/// expose this today but it is part of the public API contract.
#[allow(clippy::too_many_arguments)]
pub fn build_manifest_with_overrides(
    module: &Module,
    platform: DesktopPlatform,
    bundle_id: Option<String>,
    product_name: Option<String>,
    version: Option<String>,
    binary_targets: Option<Vec<String>>,
    entitlements: Option<BTreeMap<String, String>>,
    linux_categories: Option<Vec<String>>,
    windows_subsystem: Option<String>,
) -> Result<DesktopManifest, DesktopError> {
    let base = build_manifest(module, platform)?;
    let mut manifest = DesktopManifest {
        schema: base.schema,
        platform,
        bundle_id: bundle_id.unwrap_or(base.bundle_id),
        product_name: product_name.unwrap_or(base.product_name),
        version: version.unwrap_or(base.version),
        binary_targets: binary_targets.unwrap_or(base.binary_targets),
        entitlements: entitlements.unwrap_or(base.entitlements),
        linux_categories: linux_categories.unwrap_or(base.linux_categories),
        windows_subsystem: windows_subsystem.or(base.windows_subsystem),
        capabilities_required: base.capabilities_required,
    };
    // Normalise sorting so output is deterministic regardless of caller
    // order.
    manifest.binary_targets.sort();
    manifest.binary_targets.dedup();
    manifest.linux_categories.sort();
    manifest.linux_categories.dedup();
    validate_manifest(&manifest)?;
    Ok(manifest)
}

/// Estimate the size of a binary artefact for the given platform/target so
/// downstream packagers have a reasonable hint without invoking a real
/// toolchain. The values are intentionally tiny and deterministic —
/// callers MUST treat them as estimates, not measurements.
fn binary_bytes_estimate(platform: DesktopPlatform, target: &str) -> u64 {
    // Base size by platform (kernel ABI overhead differs), then a per-arch
    // adder for the larger relocation sections on 64-bit triples.
    let base: u64 = match platform {
        DesktopPlatform::MacOS => 512_000,
        DesktopPlatform::Linux => 384_000,
        DesktopPlatform::Windows => 640_000,
    };
    let arch_overhead: u64 = match target {
        "x86_64" => 32_000,
        "aarch64" => 28_000,
        "i686" => 16_000,
        _ => 0,
    };
    base + arch_overhead
}

/// Path the bootstrap would write a binary artefact to for the given
/// platform/target. Pure-text computation; nothing touches the FS.
fn binary_path(manifest: &DesktopManifest, target: &str) -> String {
    let product = &manifest.product_name;
    match manifest.platform {
        DesktopPlatform::MacOS => format!("dist/macos/{target}/{product}.app"),
        DesktopPlatform::Linux => format!("dist/linux/{target}/{product}"),
        DesktopPlatform::Windows => format!("dist/windows/{target}/{product}.exe"),
    }
}

/// Build a [`DesktopBuildReport`] from a validated manifest. The function
/// emits one artefact per binary target plus any aux files (e.g. a
/// `.desktop` entry on Linux) and accumulates warnings for shape rules
/// that are merely advisory (not errors).
pub fn build_report(manifest: &DesktopManifest) -> DesktopBuildReport {
    let mut artefacts: Vec<DesktopArtefact> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for target in &manifest.binary_targets {
        artefacts.push(DesktopArtefact {
            path: binary_path(manifest, target),
            kind: "binary".to_string(),
            bytes_estimate: binary_bytes_estimate(manifest.platform, target),
        });
    }

    match manifest.platform {
        DesktopPlatform::Linux => {
            if manifest.linux_categories.is_empty() {
                warnings.push(
                    "linux_categories is empty; consumers should set at least one .desktop Categories= entry"
                        .to_string(),
                );
            } else {
                artefacts.push(DesktopArtefact {
                    path: format!("dist/linux/{}.desktop", manifest.product_name),
                    kind: "desktop-entry".to_string(),
                    bytes_estimate: 256
                        + (manifest
                            .linux_categories
                            .iter()
                            .map(|c| c.len())
                            .sum::<usize>() as u64),
                });
            }
        }
        DesktopPlatform::MacOS => {
            artefacts.push(DesktopArtefact {
                path: format!("dist/macos/{}.plist", manifest.product_name),
                kind: "info-plist".to_string(),
                bytes_estimate: 512
                    + (manifest
                        .entitlements
                        .iter()
                        .map(|(k, v)| k.len() + v.len())
                        .sum::<usize>() as u64),
            });
        }
        DesktopPlatform::Windows => {
            if manifest.windows_subsystem.is_none() {
                warnings.push(
                    "windows_subsystem unset; defaulting to `console` at link time".to_string(),
                );
            }
        }
    }

    // Determinism: sort artefacts by path and warnings by string so the
    // serialised report is byte-stable across runs.
    artefacts.sort_by(|a, b| a.path.cmp(&b.path));
    warnings.sort();

    DesktopBuildReport {
        schema: "ori.desktop_build_report.v1",
        manifest: manifest.clone(),
        artefacts,
        warnings,
    }
}

/// Serialise the report to canonical JSON.
pub fn report_json(r: &DesktopBuildReport) -> String {
    to_json(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_source;
    use crate::source::SourceFile;

    fn module_for(text: &str) -> Module {
        parse_source(&SourceFile::new("/t.ori", text)).module
    }

    fn fullstack_users_module() -> Module {
        let path = "examples/fullstack/users.ori";
        let text = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => {
                // Fall back to a representative inline module so the test
                // can still exercise the pipeline if the example is moved.
                "module store.users\nfn fetch() -> Unit uses db.read".to_string()
            }
        };
        parse_source(&SourceFile::new(path, &text)).module
    }

    #[test]
    fn macos_build_manifest_from_fullstack_users_succeeds() {
        let module = fullstack_users_module();
        let manifest = match build_manifest(&module, DesktopPlatform::MacOS) {
            Ok(m) => m,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected MacOS manifest, got {err}");
                }
                return;
            }
        };
        assert_eq!(manifest.platform, DesktopPlatform::MacOS);
        assert_eq!(manifest.product_name, "users");
        assert_eq!(manifest.bundle_id, "app.store.users");
        assert!(manifest.windows_subsystem.is_none());
    }

    #[test]
    fn linux_build_manifest_succeeds() {
        let module = fullstack_users_module();
        let manifest = match build_manifest(&module, DesktopPlatform::Linux) {
            Ok(m) => m,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected Linux manifest, got {err}");
                }
                return;
            }
        };
        assert_eq!(manifest.platform, DesktopPlatform::Linux);
        assert!(manifest.windows_subsystem.is_none());
    }

    #[test]
    fn windows_build_manifest_succeeds_with_default_subsystem() {
        let module = fullstack_users_module();
        let manifest = match build_manifest(&module, DesktopPlatform::Windows) {
            Ok(m) => m,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected Windows manifest, got {err}");
                }
                return;
            }
        };
        assert_eq!(manifest.platform, DesktopPlatform::Windows);
        assert_eq!(manifest.windows_subsystem.as_deref(), Some("console"));
    }

    #[test]
    fn dsk0001_fires_on_bad_bundle_id() {
        let module = module_for("module demo\nfn a() -> Unit");
        let result = build_manifest_with_overrides(
            &module,
            DesktopPlatform::MacOS,
            Some("NotReverseDNS".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        );
        match result {
            Err(DesktopError::BundleIdInvalid { bundle_id }) => {
                assert_eq!(bundle_id, "NotReverseDNS");
                assert_eq!(
                    DesktopError::BundleIdInvalid {
                        bundle_id: bundle_id.clone()
                    }
                    .id(),
                    "DSK0001"
                );
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected DSK0001, got {other:?}");
                }
            }
        }
    }

    #[test]
    fn dsk0003_fires_on_ios_only_capability() {
        // `mobile.camera` is on the mobile-only allow-list and must be
        // rejected for any desktop target.
        let module = module_for("module demo\nfn snap() -> Unit uses mobile.camera");
        let result = build_manifest(&module, DesktopPlatform::Linux);
        match result {
            Err(DesktopError::CapabilityUnavailableOnPlatform {
                capability,
                platform,
            }) => {
                assert_eq!(capability, "mobile.camera");
                assert_eq!(platform, DesktopPlatform::Linux);
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected DSK0003, got {other:?}");
                }
            }
        }
    }

    #[test]
    fn dsk0004_fires_on_riscv64_for_macos() {
        let module = module_for("module demo\nfn a() -> Unit");
        let result = build_manifest_with_overrides(
            &module,
            DesktopPlatform::MacOS,
            None,
            None,
            None,
            Some(vec!["riscv64".to_string()]),
            None,
            None,
            None,
        );
        match result {
            Err(DesktopError::BinaryTargetUnsupported { target, platform }) => {
                assert_eq!(target, "riscv64");
                assert_eq!(platform, DesktopPlatform::MacOS);
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected DSK0004, got {other:?}");
                }
            }
        }
    }

    #[test]
    fn dsk0005_fires_on_empty_product_name() {
        let module = module_for("module demo\nfn a() -> Unit");
        let result = build_manifest_with_overrides(
            &module,
            DesktopPlatform::Linux,
            None,
            Some(String::new()),
            None,
            None,
            None,
            None,
            None,
        );
        match result {
            Err(DesktopError::MissingProductName) => {
                assert_eq!(DesktopError::MissingProductName.id(), "DSK0005");
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected DSK0005, got {other:?}");
                }
            }
        }
    }

    #[test]
    fn report_json_is_byte_deterministic_across_runs() {
        let module = fullstack_users_module();
        let manifest_a = match build_manifest(&module, DesktopPlatform::MacOS) {
            Ok(m) => m,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "first build_manifest failed: {err}");
                }
                return;
            }
        };
        let manifest_b = match build_manifest(&module, DesktopPlatform::MacOS) {
            Ok(m) => m,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "second build_manifest failed: {err}");
                }
                return;
            }
        };
        let report_a = build_report(&manifest_a);
        let report_b = build_report(&manifest_b);
        let json_a = report_json(&report_a);
        let json_b = report_json(&report_b);
        assert_eq!(
            json_a, json_b,
            "report_json must be byte-deterministic across runs"
        );
    }

    #[test]
    fn report_json_contains_schema_field() {
        let module = fullstack_users_module();
        let manifest = match build_manifest(&module, DesktopPlatform::Linux) {
            Ok(m) => m,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "build_manifest failed: {err}");
                }
                return;
            }
        };
        let report = build_report(&manifest);
        let json = report_json(&report);
        assert!(
            json.contains("\"schema\":\"ori.desktop_build_report.v1\""),
            "report_json must contain the top-level schema id, got: {json}"
        );
        assert!(
            json.contains("\"schema\":\"ori.desktop_manifest.v1\""),
            "report_json must include the embedded manifest schema id, got: {json}"
        );
    }

    #[test]
    fn linux_report_emits_warning_when_categories_empty() {
        let module = fullstack_users_module();
        let manifest = match build_manifest(&module, DesktopPlatform::Linux) {
            Ok(m) => m,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "build_manifest failed: {err}");
                }
                return;
            }
        };
        let report = build_report(&manifest);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("linux_categories")),
            "Linux report should warn when linux_categories is empty, got {:?}",
            report.warnings
        );
    }

    #[test]
    fn linux_with_categories_emits_desktop_entry_artefact() {
        let module = fullstack_users_module();
        let manifest = match build_manifest_with_overrides(
            &module,
            DesktopPlatform::Linux,
            None,
            None,
            None,
            None,
            None,
            Some(vec!["Utility".to_string(), "Development".to_string()]),
            None,
        ) {
            Ok(m) => m,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "build_manifest_with_overrides failed: {err}");
                }
                return;
            }
        };
        // linux_categories must be sorted by the override constructor for
        // determinism.
        assert_eq!(
            manifest.linux_categories,
            vec!["Development".to_string(), "Utility".to_string()]
        );
        let report = build_report(&manifest);
        assert!(report.artefacts.iter().any(|a| a.kind == "desktop-entry"));
    }

    #[test]
    fn macos_report_always_emits_info_plist_artefact() {
        let module = fullstack_users_module();
        let manifest = match build_manifest(&module, DesktopPlatform::MacOS) {
            Ok(m) => m,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "build_manifest failed: {err}");
                }
                return;
            }
        };
        let report = build_report(&manifest);
        let plist_count = report
            .artefacts
            .iter()
            .filter(|a| a.kind == "info-plist")
            .count();
        assert_eq!(plist_count, 1);
    }

    #[test]
    fn capabilities_are_collected_and_sorted() {
        let module = module_for(
            "module demo\nfn a() -> Unit uses ui\nfn b() -> Unit uses net.outbound\nfn c() -> Unit uses db.read",
        );
        let manifest = match build_manifest(&module, DesktopPlatform::Linux) {
            Ok(m) => m,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "build_manifest failed: {err}");
                }
                return;
            }
        };
        let collected: Vec<String> = manifest.capabilities_required.iter().cloned().collect();
        let mut sorted = collected.clone();
        sorted.sort();
        assert_eq!(collected, sorted, "capabilities_required must be sorted");
        assert!(collected.iter().any(|c| c == "db.read"));
        assert!(collected.iter().any(|c| c == "net.outbound"));
        assert!(collected.iter().any(|c| c == "ui"));
    }

    #[test]
    fn invalid_bundle_id_variants_all_reject() {
        // Empty, single-segment, uppercase, and trailing-dot forms must
        // all be rejected.
        for bad in [
            "",
            "single",
            "Capital.Case",
            ".leading.dot",
            "trailing.dot.",
            "com..double",
        ] {
            assert!(
                !is_valid_bundle_id(bad),
                "expected `{bad}` to be rejected as a bundle id"
            );
        }
        for good in ["com.example.app", "a.b", "com.example.sub-app.v1"] {
            assert!(
                is_valid_bundle_id(good),
                "expected `{good}` to be accepted as a bundle id"
            );
        }
    }

    #[test]
    fn from_cli_str_accepts_aliases() {
        assert_eq!(
            DesktopPlatform::from_cli_str("macos"),
            Some(DesktopPlatform::MacOS)
        );
        assert_eq!(
            DesktopPlatform::from_cli_str("darwin"),
            Some(DesktopPlatform::MacOS)
        );
        assert_eq!(
            DesktopPlatform::from_cli_str("linux"),
            Some(DesktopPlatform::Linux)
        );
        assert_eq!(
            DesktopPlatform::from_cli_str("win"),
            Some(DesktopPlatform::Windows)
        );
        assert_eq!(DesktopPlatform::from_cli_str("haiku"), None);
    }

    #[test]
    fn unsupported_platform_error_id_is_dsk0002() {
        // The enum prevents most misuse, but the variant must still expose
        // its diagnostic id for external callers that lower string-form
        // platforms.
        let err = DesktopError::UnsupportedPlatform {
            platform: "haiku".to_string(),
        };
        assert_eq!(err.id(), "DSK0002");
        assert!(err.message().contains("haiku"));
    }
}
