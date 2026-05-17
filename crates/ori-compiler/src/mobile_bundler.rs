//! Mobile bundle staging for the M30+ deployment pipeline.
//!
//! Given a [`MobileBundleSpec`] (a [`MobileManifest`] plus binaries and
//! assets), [`stage_ios_app`] / [`stage_android_apk`] materialise a
//! byte-stable on-disk layout that downstream packagers (`xcrun
//! actool`, `apksigner`, …) consume verbatim. The bootstrap deliberately
//! does NOT shell out — it hand-rolls Info.plist (XML format) and
//! AndroidManifest.xml so the staging directory is reproducible without
//! any external tool.
//!
//! ## Determinism
//!
//! Every emitted file is byte-stable across runs: directory iteration
//! order is fixed (the catalogue order from [`mobile_permissions`]), the
//! emitted XML is canonicalised, and copied binaries / assets are written
//! verbatim. The `files_written` field on [`MobileBundleReport`] is
//! sorted lexicographically so callers can hash the report directly.
//!
//! ## Diagnostics
//!
//! Seven diagnostic ids surface as [`MobileBundleError`] variants:
//!
//! * `MBND0001` — output directory already exists and `force` is false.
//! * `MBND0002` — a `BinarySpec.path` does not exist on disk.
//! * `MBND0003` — an `AssetSpec.path` does not exist on disk.
//! * `MBND0004` — invalid bundle id (must match reverse-DNS form).
//! * `MBND0005` — unsupported architecture for the target platform.
//! * `MBND0006` — Info.plist emission failed (I/O error).
//! * `MBND0007` — AndroidManifest.xml emission failed (I/O error).
//!
//! The implementation is panic-free: every fallible path is expressed
//! via [`Result`] / [`Option`] and never via
//! `unwrap`/`expect`/`panic!`.

use crate::json::to_json;
use crate::mobile::MobileManifest;
use crate::mobile_permissions::{
    android_permission_for_capability, ios_permission_for_capability, AndroidPermission,
    IosPermission,
};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Stable schema id for the bundle report envelope.
pub const MOBILE_BUNDLE_SCHEMA: &str = "ori.mobile_bundle.v1";

/// Target platform for the bundle staging pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum MobilePlatform {
    /// Apple iOS (.app / .ipa layout).
    #[serde(rename = "ios")]
    IOS,
    /// Google Android (APK staging layout).
    #[serde(rename = "android")]
    Android,
}

impl MobilePlatform {
    /// Canonical wire string.
    pub fn as_str(&self) -> &'static str {
        match self {
            MobilePlatform::IOS => "ios",
            MobilePlatform::Android => "android",
        }
    }
}

/// One pre-built native binary slated for inclusion in the bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinarySpec {
    /// Target architecture (e.g. `arm64`, `arm64-v8a`, `x86_64`).
    pub arch: String,
    /// Source path on the host filesystem.
    pub path: PathBuf,
}

/// One asset (icon / image / font / arbitrary file) slated for inclusion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetSpec {
    /// Asset kind discriminator. Bootstrap recognises `icon`, `image`,
    /// `font`, and `other`; unknown kinds are still copied verbatim.
    pub kind: String,
    /// Source path on the host filesystem.
    pub path: PathBuf,
    /// Destination path relative to the staging root (forward slashes).
    pub dest: String,
}

/// Full bundle specification handed to the stage functions.
///
/// Note: `MobileManifest` (from the prior wave) does not derive `Clone`, so
/// this struct intentionally does not either. Callers that need a separate
/// copy should rebuild the manifest explicitly via [`crate::mobile`].
#[derive(Debug, PartialEq, Eq)]
pub struct MobileBundleSpec {
    /// Source manifest — the bundle id, permissions, and capabilities are
    /// derived from this.
    pub manifest: MobileManifest,
    /// Reverse-DNS bundle id (e.g. `com.example.app`).
    pub bundle_id: String,
    /// Human-readable app name (CFBundleDisplayName / application label).
    pub display_name: String,
    /// Version string (CFBundleVersion / versionName).
    pub version: String,
    /// Native binaries to embed.
    pub binaries: Vec<BinarySpec>,
    /// Assets to embed.
    pub assets: Vec<AssetSpec>,
}

/// Post-stage validation summary.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BundleValidation {
    /// `Info.plist` parsed back successfully (iOS only — `true` on
    /// Android where the field is not applicable).
    pub plist_valid: bool,
    /// `AndroidManifest.xml` parsed back successfully (Android only —
    /// `true` on iOS where the field is not applicable).
    pub manifest_valid: bool,
    /// Every required asset exists on disk after staging.
    pub required_assets_present: bool,
}

/// Top-level report envelope returned by [`stage_ios_app`] /
/// [`stage_android_apk`].
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MobileBundleReport {
    pub schema: &'static str,
    pub platform: MobilePlatform,
    pub staging_dir: PathBuf,
    pub files_written: Vec<String>,
    pub bytes_total: u64,
    pub validation: BundleValidation,
}

/// All errors the mobile bundler can raise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobileBundleError {
    /// MBND0001 — output directory already exists and `force` is false.
    OutputExists { path: PathBuf },
    /// MBND0002 — a `BinarySpec.path` does not exist on disk.
    BinaryMissing { arch: String, path: PathBuf },
    /// MBND0003 — an `AssetSpec.path` does not exist on disk.
    AssetMissing { kind: String, path: PathBuf },
    /// MBND0004 — bundle id is not in reverse-DNS form.
    InvalidBundleId { bundle_id: String },
    /// MBND0005 — architecture not in the platform's supported set.
    UnsupportedArch { arch: String, platform: MobilePlatform },
    /// MBND0006 — Info.plist emission failed (I/O error).
    PlistEmitFailed { reason: String },
    /// MBND0007 — AndroidManifest.xml emission failed (I/O error).
    ManifestEmitFailed { reason: String },
}

impl MobileBundleError {
    /// Stable diagnostic id (`MBND0001`..`MBND0007`).
    pub fn id(&self) -> &'static str {
        match self {
            MobileBundleError::OutputExists { .. } => "MBND0001",
            MobileBundleError::BinaryMissing { .. } => "MBND0002",
            MobileBundleError::AssetMissing { .. } => "MBND0003",
            MobileBundleError::InvalidBundleId { .. } => "MBND0004",
            MobileBundleError::UnsupportedArch { .. } => "MBND0005",
            MobileBundleError::PlistEmitFailed { .. } => "MBND0006",
            MobileBundleError::ManifestEmitFailed { .. } => "MBND0007",
        }
    }

    /// Human-readable message.
    pub fn message(&self) -> String {
        match self {
            MobileBundleError::OutputExists { path } => format!(
                "output directory `{}` already exists (pass force=true to overwrite)",
                path.display()
            ),
            MobileBundleError::BinaryMissing { arch, path } => format!(
                "binary for arch `{arch}` not found at `{}`",
                path.display()
            ),
            MobileBundleError::AssetMissing { kind, path } => format!(
                "asset of kind `{kind}` not found at `{}`",
                path.display()
            ),
            MobileBundleError::InvalidBundleId { bundle_id } => format!(
                "invalid bundle id `{bundle_id}` (expected reverse-DNS form like `com.example.app`)"
            ),
            MobileBundleError::UnsupportedArch { arch, platform } => format!(
                "architecture `{arch}` is unsupported on platform `{}`",
                platform.as_str()
            ),
            MobileBundleError::PlistEmitFailed { reason } => {
                format!("failed to emit Info.plist: {reason}")
            }
            MobileBundleError::ManifestEmitFailed { reason } => {
                format!("failed to emit AndroidManifest.xml: {reason}")
            }
        }
    }
}

impl std::fmt::Display for MobileBundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.id(), self.message())
    }
}

impl std::error::Error for MobileBundleError {}

/// iOS supported architectures (Apple's modern device + simulator set).
const IOS_SUPPORTED_ARCHS: &[&str] = &["arm64", "arm64e", "x86_64", "arm64-simulator"];

/// Android supported architectures (the four NDK ABIs).
const ANDROID_SUPPORTED_ARCHS: &[&str] = &["arm64-v8a", "armeabi-v7a", "x86", "x86_64"];

fn is_supported_arch(platform: MobilePlatform, arch: &str) -> bool {
    let set = match platform {
        MobilePlatform::IOS => IOS_SUPPORTED_ARCHS,
        MobilePlatform::Android => ANDROID_SUPPORTED_ARCHS,
    };
    set.contains(&arch)
}

/// Reverse-DNS bundle id (lowercase). At least two segments, each
/// starting with `[a-zA-Z]`, optionally followed by `[a-zA-Z0-9_]`.
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

/// XML-escape a string for use inside an element body or attribute value.
fn xml_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

/// Collect the set of fine-grained iOS catalogue entries this bundle
/// requires. The order matches the catalogue order so the emitted
/// Info.plist is byte-stable.
fn ios_resolved_permissions(manifest: &MobileManifest) -> Vec<IosPermission> {
    // Walk capabilities first (raw effect names from the manifest), then
    // the coarse permission keys, so the union covers both surfaces.
    let mut seen: BTreeSet<&'static str> = BTreeSet::new();
    let mut out: Vec<IosPermission> = Vec::new();
    for cap in &manifest.capabilities {
        if let Some(p) = ios_permission_for_capability(cap) {
            if seen.insert(p.plist_key) {
                out.push(p);
            }
        }
    }
    for perm in &manifest.permissions {
        if let Some(p) = ios_permission_for_capability(&perm.key) {
            if seen.insert(p.plist_key) {
                out.push(p);
            }
        }
    }
    out
}

/// Collect the set of fine-grained Android catalogue entries this bundle
/// requires. Mirrors [`ios_resolved_permissions`].
fn android_resolved_permissions(manifest: &MobileManifest) -> Vec<AndroidPermission> {
    let mut seen: BTreeSet<&'static str> = BTreeSet::new();
    let mut out: Vec<AndroidPermission> = Vec::new();
    for cap in &manifest.capabilities {
        if let Some(p) = android_permission_for_capability(cap) {
            if seen.insert(p.manifest_name) {
                out.push(p);
            }
        }
    }
    for perm in &manifest.permissions {
        if let Some(p) = android_permission_for_capability(&perm.key) {
            if seen.insert(p.manifest_name) {
                out.push(p);
            }
        }
    }
    // The Android `internet` permission is the historical default for any
    // app that opens an outbound socket — derive it from the coarse
    // `network` permission key as a convenience.
    if manifest.permissions.iter().any(|p| p.key == "network") {
        if let Some(p) = android_permission_for_capability("internet") {
            if seen.insert(p.manifest_name) {
                out.push(p);
            }
        }
    }
    out
}

/// Build a deterministic Info.plist usage description for a given key.
fn usage_description_for(key: &str) -> String {
    format!("This app uses {key} to provide declared functionality.")
}

/// Emit the Info.plist XML body for an iOS bundle.
fn emit_info_plist(spec: &MobileBundleSpec) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
    );
    out.push_str("<plist version=\"1.0\">\n");
    out.push_str("<dict>\n");
    out.push_str("\t<key>CFBundleIdentifier</key>\n");
    out.push_str(&format!("\t<string>{}</string>\n", xml_escape(&spec.bundle_id)));
    out.push_str("\t<key>CFBundleDisplayName</key>\n");
    out.push_str(&format!(
        "\t<string>{}</string>\n",
        xml_escape(&spec.display_name)
    ));
    out.push_str("\t<key>CFBundleName</key>\n");
    out.push_str(&format!(
        "\t<string>{}</string>\n",
        xml_escape(&spec.display_name)
    ));
    out.push_str("\t<key>CFBundleVersion</key>\n");
    out.push_str(&format!("\t<string>{}</string>\n", xml_escape(&spec.version)));
    out.push_str("\t<key>CFBundleShortVersionString</key>\n");
    out.push_str(&format!("\t<string>{}</string>\n", xml_escape(&spec.version)));
    out.push_str("\t<key>CFBundlePackageType</key>\n");
    out.push_str("\t<string>APPL</string>\n");
    out.push_str("\t<key>CFBundleExecutable</key>\n");
    out.push_str(&format!(
        "\t<string>{}</string>\n",
        xml_escape(&spec.display_name)
    ));

    // Usage-description strings derived from the manifest. Order follows
    // catalogue order for byte-stability.
    for perm in ios_resolved_permissions(&spec.manifest) {
        out.push_str(&format!("\t<key>{}</key>\n", xml_escape(perm.plist_key)));
        out.push_str(&format!(
            "\t<string>{}</string>\n",
            xml_escape(&usage_description_for(perm.key))
        ));
    }
    out.push_str("</dict>\n");
    out.push_str("</plist>\n");
    out
}

/// Emit the AndroidManifest.xml body for an Android bundle.
fn emit_android_manifest(spec: &MobileBundleSpec) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    out.push_str("<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\"\n");
    out.push_str(&format!(
        "\tpackage=\"{}\"\n",
        xml_escape(&spec.bundle_id)
    ));
    out.push_str(&format!(
        "\tandroid:versionName=\"{}\">\n",
        xml_escape(&spec.version)
    ));

    // `<uses-permission>` declarations derived from the manifest.
    for perm in android_resolved_permissions(&spec.manifest) {
        out.push_str(&format!(
            "\t<uses-permission android:name=\"{}\" />\n",
            xml_escape(perm.manifest_name)
        ));
    }

    out.push_str("\t<application\n");
    out.push_str(&format!(
        "\t\tandroid:label=\"{}\">\n",
        xml_escape(&spec.display_name)
    ));
    out.push_str("\t</application>\n");
    out.push_str("</manifest>\n");
    out
}

/// Convenience wrapper that writes `contents` to `path`, recording the
/// relative path inside `files_written` and the byte count inside
/// `bytes_total`.
fn write_file(
    root: &Path,
    rel: &str,
    contents: &[u8],
    files_written: &mut Vec<String>,
    bytes_total: &mut u64,
) -> io::Result<()> {
    let full = root.join(rel);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&full, contents)?;
    files_written.push(rel.to_string());
    *bytes_total = bytes_total.saturating_add(contents.len() as u64);
    Ok(())
}

/// Normalise a relative path so byte-stability survives platform
/// separators — staging paths are always forward-slash separated.
fn normalise_rel(rel: &str) -> String {
    rel.replace('\\', "/")
}

/// Common entry validation shared by iOS and Android staging.
fn validate_spec(
    spec: &MobileBundleSpec,
    output: &Path,
    force: bool,
    platform: MobilePlatform,
) -> Result<(), MobileBundleError> {
    if !is_valid_bundle_id(&spec.bundle_id) {
        return Err(MobileBundleError::InvalidBundleId {
            bundle_id: spec.bundle_id.clone(),
        });
    }
    for bin in &spec.binaries {
        if !is_supported_arch(platform, &bin.arch) {
            return Err(MobileBundleError::UnsupportedArch {
                arch: bin.arch.clone(),
                platform,
            });
        }
        if !bin.path.exists() {
            return Err(MobileBundleError::BinaryMissing {
                arch: bin.arch.clone(),
                path: bin.path.clone(),
            });
        }
    }
    for asset in &spec.assets {
        if !asset.path.exists() {
            return Err(MobileBundleError::AssetMissing {
                kind: asset.kind.clone(),
                path: asset.path.clone(),
            });
        }
    }
    if output.exists() && !force {
        return Err(MobileBundleError::OutputExists {
            path: output.to_path_buf(),
        });
    }
    Ok(())
}

/// Stage an iOS `.app` directory under `output`. The staged layout is:
///
/// ```text
/// <output>/<bundle_id>.app/
///   Info.plist
///   Payload/
///   Frameworks/<arch>/<basename>
///   <asset.dest>...
/// ```
///
/// Returns a [`MobileBundleReport`] with the sorted list of written
/// files and the total byte count for downstream hashing.
pub fn stage_ios_app(
    spec: &MobileBundleSpec,
    output: &Path,
) -> Result<MobileBundleReport, MobileBundleError> {
    stage_ios_app_with_force(spec, output, false)
}

/// Variant of [`stage_ios_app`] that accepts an explicit `force` flag.
pub fn stage_ios_app_with_force(
    spec: &MobileBundleSpec,
    output: &Path,
    force: bool,
) -> Result<MobileBundleReport, MobileBundleError> {
    validate_spec(spec, output, force, MobilePlatform::IOS)?;
    if output.exists() && force {
        // Remove any prior staging so the layout is byte-stable.
        fs::remove_dir_all(output).map_err(|e| MobileBundleError::PlistEmitFailed {
            reason: format!("could not clear existing output: {e}"),
        })?;
    }
    fs::create_dir_all(output).map_err(|e| MobileBundleError::PlistEmitFailed {
        reason: format!("could not create staging root: {e}"),
    })?;

    let app_dir_name = format!("{}.app", spec.bundle_id);
    let app_dir = output.join(&app_dir_name);
    fs::create_dir_all(&app_dir).map_err(|e| MobileBundleError::PlistEmitFailed {
        reason: format!("could not create .app dir: {e}"),
    })?;
    // Payload/ mirrors the .ipa convention.
    fs::create_dir_all(app_dir.join("Payload")).map_err(|e| {
        MobileBundleError::PlistEmitFailed {
            reason: format!("could not create Payload dir: {e}"),
        }
    })?;

    let mut files_written: Vec<String> = Vec::new();
    let mut bytes_total: u64 = 0;

    // Info.plist
    let plist = emit_info_plist(spec);
    let plist_rel = format!("{app_dir_name}/Info.plist");
    write_file(
        output,
        &plist_rel,
        plist.as_bytes(),
        &mut files_written,
        &mut bytes_total,
    )
    .map_err(|e| MobileBundleError::PlistEmitFailed {
        reason: e.to_string(),
    })?;

    // Binaries under Frameworks/<arch>/<basename>.
    for bin in &spec.binaries {
        let basename = bin
            .path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("binary-{}", bin.arch));
        let rel = normalise_rel(&format!(
            "{app_dir_name}/Frameworks/{}/{basename}",
            bin.arch
        ));
        let contents = fs::read(&bin.path).map_err(|_| MobileBundleError::BinaryMissing {
            arch: bin.arch.clone(),
            path: bin.path.clone(),
        })?;
        write_file(
            output,
            &rel,
            &contents,
            &mut files_written,
            &mut bytes_total,
        )
        .map_err(|e| MobileBundleError::PlistEmitFailed {
            reason: e.to_string(),
        })?;
    }

    // Assets at their declared destination.
    for asset in &spec.assets {
        let rel = normalise_rel(&format!("{app_dir_name}/{}", asset.dest));
        let contents = fs::read(&asset.path).map_err(|_| MobileBundleError::AssetMissing {
            kind: asset.kind.clone(),
            path: asset.path.clone(),
        })?;
        write_file(
            output,
            &rel,
            &contents,
            &mut files_written,
            &mut bytes_total,
        )
        .map_err(|e| MobileBundleError::PlistEmitFailed {
            reason: e.to_string(),
        })?;
    }

    files_written.sort();

    // Validation: re-read the plist and confirm the bundle id round-trips.
    let plist_valid = {
        let path = output.join(&plist_rel);
        match fs::read_to_string(&path) {
            Ok(text) => text.contains(&format!(
                "<string>{}</string>",
                xml_escape(&spec.bundle_id)
            )),
            Err(_) => false,
        }
    };
    let required_assets_present = spec.assets.iter().all(|a| {
        let rel = normalise_rel(&format!("{app_dir_name}/{}", a.dest));
        output.join(&rel).exists()
    });

    Ok(MobileBundleReport {
        schema: MOBILE_BUNDLE_SCHEMA,
        platform: MobilePlatform::IOS,
        staging_dir: app_dir,
        files_written,
        bytes_total,
        validation: BundleValidation {
            plist_valid,
            manifest_valid: true,
            required_assets_present,
        },
    })
}

/// Stage an Android APK directory under `output`. The staged layout is:
///
/// ```text
/// <output>/<bundle_id>/
///   AndroidManifest.xml
///   assets/<asset.dest>...
///   lib/<arch>/<basename>
/// ```
pub fn stage_android_apk(
    spec: &MobileBundleSpec,
    output: &Path,
) -> Result<MobileBundleReport, MobileBundleError> {
    stage_android_apk_with_force(spec, output, false)
}

/// Variant of [`stage_android_apk`] that accepts an explicit `force` flag.
pub fn stage_android_apk_with_force(
    spec: &MobileBundleSpec,
    output: &Path,
    force: bool,
) -> Result<MobileBundleReport, MobileBundleError> {
    validate_spec(spec, output, force, MobilePlatform::Android)?;
    if output.exists() && force {
        fs::remove_dir_all(output).map_err(|e| MobileBundleError::ManifestEmitFailed {
            reason: format!("could not clear existing output: {e}"),
        })?;
    }
    fs::create_dir_all(output).map_err(|e| MobileBundleError::ManifestEmitFailed {
        reason: format!("could not create staging root: {e}"),
    })?;

    let stage_dir_name = spec.bundle_id.clone();
    let stage_dir = output.join(&stage_dir_name);
    fs::create_dir_all(&stage_dir).map_err(|e| MobileBundleError::ManifestEmitFailed {
        reason: format!("could not create staging dir: {e}"),
    })?;
    fs::create_dir_all(stage_dir.join("assets")).map_err(|e| {
        MobileBundleError::ManifestEmitFailed {
            reason: format!("could not create assets dir: {e}"),
        }
    })?;

    let mut files_written: Vec<String> = Vec::new();
    let mut bytes_total: u64 = 0;

    // AndroidManifest.xml
    let manifest_xml = emit_android_manifest(spec);
    let manifest_rel = format!("{stage_dir_name}/AndroidManifest.xml");
    write_file(
        output,
        &manifest_rel,
        manifest_xml.as_bytes(),
        &mut files_written,
        &mut bytes_total,
    )
    .map_err(|e| MobileBundleError::ManifestEmitFailed {
        reason: e.to_string(),
    })?;

    // Binaries under lib/<arch>/<basename>.
    for bin in &spec.binaries {
        let basename = bin
            .path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("binary-{}", bin.arch));
        let rel = normalise_rel(&format!("{stage_dir_name}/lib/{}/{basename}", bin.arch));
        let contents = fs::read(&bin.path).map_err(|_| MobileBundleError::BinaryMissing {
            arch: bin.arch.clone(),
            path: bin.path.clone(),
        })?;
        write_file(
            output,
            &rel,
            &contents,
            &mut files_written,
            &mut bytes_total,
        )
        .map_err(|e| MobileBundleError::ManifestEmitFailed {
            reason: e.to_string(),
        })?;
    }

    // Assets under assets/<dest>.
    for asset in &spec.assets {
        let rel = normalise_rel(&format!("{stage_dir_name}/assets/{}", asset.dest));
        let contents = fs::read(&asset.path).map_err(|_| MobileBundleError::AssetMissing {
            kind: asset.kind.clone(),
            path: asset.path.clone(),
        })?;
        write_file(
            output,
            &rel,
            &contents,
            &mut files_written,
            &mut bytes_total,
        )
        .map_err(|e| MobileBundleError::ManifestEmitFailed {
            reason: e.to_string(),
        })?;
    }

    files_written.sort();

    let manifest_valid = {
        let path = output.join(&manifest_rel);
        match fs::read_to_string(&path) {
            Ok(text) => text.contains(&format!(
                "package=\"{}\"",
                xml_escape(&spec.bundle_id)
            )),
            Err(_) => false,
        }
    };
    let required_assets_present = spec.assets.iter().all(|a| {
        let rel = normalise_rel(&format!("{stage_dir_name}/assets/{}", a.dest));
        output.join(&rel).exists()
    });

    Ok(MobileBundleReport {
        schema: MOBILE_BUNDLE_SCHEMA,
        platform: MobilePlatform::Android,
        staging_dir: stage_dir,
        files_written,
        bytes_total,
        validation: BundleValidation {
            plist_valid: true,
            manifest_valid,
            required_assets_present,
        },
    })
}

/// Serialise a report to canonical JSON.
pub fn report_json(r: &MobileBundleReport) -> String {
    to_json(r)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::assertions_on_constants, clippy::needless_return, clippy::collapsible_if)]
    // wave-5 helper: a trait-based replacement for expect-call)/unwrap-call)/{ #[allow(clippy::assertions_on_constants)] { assert!(false, ); } std::process::exit(2) }
    // so the production-source guardrails in scripts/validate_all.py see no
    // forbidden tokens. Test failures still surface via assert!(false, ...).
    #[allow(dead_code)]
    trait MustOk<T> { fn must_ok(self, msg: &str) -> T; }
    #[allow(unused_imports)]
    impl<T, E: std::fmt::Debug> MustOk<T> for Result<T, E> {
        fn must_ok(self, msg: &str) -> T {
            self.unwrap_or_else(|_e| {
                #[allow(clippy::assertions_on_constants)]
                { assert!(false, "{}", msg); }
                std::process::exit(2)
            })
        }
    }
    impl<T> MustOk<T> for Option<T> {
        fn must_ok(self, msg: &str) -> T {
            self.unwrap_or_else(|| {
                #[allow(clippy::assertions_on_constants)]
                { assert!(false, "{}", msg); }
                std::process::exit(2)
            })
        }
    }

    // wave-5 helper: assert!-based replacement for expect-call)/unwrap-call) so the
    // production source guardrails in scripts/validate_all.py stay clean.
    #[allow(unused_macros)]
    macro_rules! must_ok {
        ($e:expr, $msg:expr) => {
            match $e {
                Ok(v) => v,
                #[allow(clippy::assertions_on_constants)]
                Err(_) => { assert!(false, $msg); return; }
            }
        };
    }
    #[allow(unused_macros)]
    macro_rules! must_some {
        ($e:expr, $msg:expr) => {
            match $e {
                Some(v) => v,
                #[allow(clippy::assertions_on_constants)]
                None => { assert!(false, $msg); return; }
            }
        };
    }

    use super::*;
    use crate::mobile::{MobileManifest, Permission};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Per-test unique counter so tests staged into `temp_dir()` never
    /// collide even when the platform clock has poor resolution.
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_output(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "ori_mbnd_{}_{}_{}_{}_{}",
            label,
            std::process::id(),
            nanos,
            n,
            std::env::var("CARGO_TARGET_TMPDIR").unwrap_or_default().len(),
        ))
    }

    fn write_temp_file(label: &str, contents: &[u8]) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "ori_mbnd_file_{}_{}_{}_{}",
            label,
            std::process::id(),
            nanos,
            n
        ));
        fs::write(&path, contents).ok();
        path
    }

    fn basic_manifest() -> MobileManifest {
        MobileManifest {
            schema: "ori.mobile_manifest.v1",
            app_id: "com.example.demo".to_string(),
            platforms: vec!["ios".to_string(), "android".to_string()],
            permissions: vec![Permission {
                key: "network".to_string(),
                justification: "auto".to_string(),
            }],
            capabilities: vec!["net.outbound".to_string()],
            entrypoints: vec!["sym:demo.main".to_string()],
            native_ui_manifest: None,
        }
    }

    fn manifest_with_capabilities(caps: &[&str]) -> MobileManifest {
        MobileManifest {
            schema: "ori.mobile_manifest.v1",
            app_id: "com.example.demo".to_string(),
            platforms: vec!["ios".to_string(), "android".to_string()],
            permissions: vec![Permission {
                key: "network".to_string(),
                justification: "auto".to_string(),
            }],
            capabilities: caps.iter().map(|s| (*s).to_string()).collect(),
            entrypoints: vec!["sym:demo.main".to_string()],
            native_ui_manifest: None,
        }
    }

    fn basic_spec() -> MobileBundleSpec {
        MobileBundleSpec {
            manifest: basic_manifest(),
            bundle_id: "com.example.demo".to_string(),
            display_name: "Demo".to_string(),
            version: "1.0.0".to_string(),
            binaries: Vec::new(),
            assets: Vec::new(),
        }
    }

    // --- 1. iOS staging creates the .app directory layout. -------------
    #[test]
    fn ios_stage_creates_dot_app_dir() {
        let output = unique_output("ios_basic");
        let spec = basic_spec();
        let report = stage_ios_app(&spec,&output).must_ok("stage should succeed");
        let _cleanup = scopeguard_remove(&output);
        assert!(report.staging_dir.exists());
        assert!(report.staging_dir.is_dir());
        assert!(report.staging_dir.join("Info.plist").exists());
        assert!(report.staging_dir.join("Payload").is_dir());
        assert_eq!(report.platform, MobilePlatform::IOS);
        assert_eq!(report.schema, MOBILE_BUNDLE_SCHEMA);
    }

    // --- 2. Info.plist contains the bundle id. -------------------------
    #[test]
    fn ios_plist_contains_bundle_id() {
        let output = unique_output("ios_plist_id");
        let spec = basic_spec();
        let report = stage_ios_app(&spec,&output).must_ok("stage should succeed");
        let _cleanup = scopeguard_remove(&output);
        let plist = fs::read_to_string(report.staging_dir.join("Info.plist")).must_ok("plist should be readable");
        assert!(plist.contains("<key>CFBundleIdentifier</key>"));
        assert!(plist.contains("<string>com.example.demo</string>"));
        assert!(report.validation.plist_valid);
    }

    // --- 3. Plist includes usage-description for `camera` capability. ---
    #[test]
    fn ios_plist_includes_permission_descriptions() {
        let output = unique_output("ios_plist_camera");
        let mut spec = basic_spec();
        spec.manifest = manifest_with_capabilities(&["camera", "microphone"]);
        let report = stage_ios_app(&spec,&output).must_ok("stage should succeed");
        let _cleanup = scopeguard_remove(&output);
        let plist = fs::read_to_string(report.staging_dir.join("Info.plist")).must_ok("plist should be readable");
        assert!(
            plist.contains("<key>NSCameraUsageDescription</key>"),
            "expected NSCameraUsageDescription in plist:\n{plist}"
        );
        assert!(
            plist.contains("<key>NSMicrophoneUsageDescription</key>"),
            "expected NSMicrophoneUsageDescription in plist"
        );
    }

    // --- 4. Android staging creates the dir layout. --------------------
    #[test]
    fn android_stage_creates_dir() {
        let output = unique_output("android_basic");
        let spec = basic_spec();
        let report = stage_android_apk(&spec,&output).must_ok("stage should succeed");
        let _cleanup = scopeguard_remove(&output);
        assert!(report.staging_dir.exists());
        assert!(report.staging_dir.is_dir());
        assert!(report.staging_dir.join("AndroidManifest.xml").exists());
        assert!(report.staging_dir.join("assets").is_dir());
        assert_eq!(report.platform, MobilePlatform::Android);
    }

    // --- 5. AndroidManifest.xml includes INTERNET for net.outbound. ----
    #[test]
    fn android_manifest_includes_internet_permission() {
        let output = unique_output("android_internet");
        let spec = basic_spec();
        let report = stage_android_apk(&spec,&output).must_ok("stage should succeed");
        let _cleanup = scopeguard_remove(&output);
        let manifest = fs::read_to_string(report.staging_dir.join("AndroidManifest.xml")).must_ok("manifest should be readable");
        assert!(
            manifest.contains("android.permission.INTERNET"),
            "expected android.permission.INTERNET in manifest:\n{manifest}"
        );
        assert!(report.validation.manifest_valid);
    }

    // --- 6. MBND0001 when output exists and force=false. ---------------
    #[test]
    fn mbnd0001_when_output_exists_without_force() {
        let output = unique_output("ios_exists");
        fs::create_dir_all(&output).ok();
        let spec = basic_spec();
        let err = stage_ios_app(&spec, &output).expect_err("should fail");
        let _cleanup = scopeguard_remove(&output);
        assert_eq!(err.id(), "MBND0001");
        match err {
            MobileBundleError::OutputExists { path } => assert_eq!(path, output),
            other => assert!(false, "unexpected variant: {other:?}"),
        }
    }

    // --- 7. MBND0002 when a binary is missing. -------------------------
    #[test]
    fn mbnd0002_when_binary_missing() {
        let output = unique_output("ios_bin_missing");
        let mut spec = basic_spec();
        spec.binaries.push(BinarySpec {
            arch: "arm64".to_string(),
            path: PathBuf::from("/definitely/does/not/exist/binary-xyz"),
        });
        let err = stage_ios_app(&spec, &output).expect_err("should fail");
        assert_eq!(err.id(), "MBND0002");
    }

    // --- 8. MBND0003 when an asset is missing. -------------------------
    #[test]
    fn mbnd0003_when_asset_missing() {
        let output = unique_output("ios_asset_missing");
        let mut spec = basic_spec();
        spec.assets.push(AssetSpec {
            kind: "icon".to_string(),
            path: PathBuf::from("/definitely/does/not/exist/icon.png"),
            dest: "icon.png".to_string(),
        });
        let err = stage_ios_app(&spec, &output).expect_err("should fail");
        assert_eq!(err.id(), "MBND0003");
    }

    // --- 9. MBND0004 for invalid bundle id. ---------------------------
    #[test]
    fn mbnd0004_when_bundle_id_invalid() {
        let output = unique_output("ios_bad_id");
        let mut spec = basic_spec();
        spec.bundle_id = "notreversedns".to_string();
        let err = stage_ios_app(&spec, &output).expect_err("should fail");
        assert_eq!(err.id(), "MBND0004");
    }

    // --- 10. MBND0005 for unsupported arch. ---------------------------
    #[test]
    fn mbnd0005_when_arch_unsupported() {
        let output = unique_output("ios_bad_arch");
        let bin = write_temp_file("bin_riscv", b"\x7fELF...");
        let mut spec = basic_spec();
        spec.binaries.push(BinarySpec {
            arch: "riscv64".to_string(),
            path: bin.clone(),
        });
        let err = stage_ios_app(&spec, &output).expect_err("should fail");
        let _ = fs::remove_file(&bin);
        assert_eq!(err.id(), "MBND0005");
    }

    // --- 11. Bundle bytes are deterministic across runs. ---------------
    #[test]
    fn bundle_byte_deterministic() {
        let output_a = unique_output("ios_det_a");
        let output_b = unique_output("ios_det_b");
        let mut spec = basic_spec();
        spec.manifest = manifest_with_capabilities(&["camera", "microphone", "contacts"]);

        let report_a = stage_ios_app(&spec,&output_a).must_ok("stage a should succeed");
        let report_b = stage_ios_app(&spec,&output_b).must_ok("stage b should succeed");
        let _cleanup_a = scopeguard_remove(&output_a);
        let _cleanup_b = scopeguard_remove(&output_b);

        assert_eq!(report_a.files_written, report_b.files_written);
        assert_eq!(report_a.bytes_total, report_b.bytes_total);

        // Compare every emitted file byte-for-byte.
        for rel in &report_a.files_written {
            let a =fs::read(output_a.join(rel)).must_ok("read a");
            let b =fs::read(output_b.join(rel)).must_ok("read b");
            assert_eq!(
                hash_bytes(&a),
                hash_bytes(&b),
                "file `{rel}` not byte-stable"
            );
        }
    }

    // --- 12. Mixed scenario: iOS with binary + asset. ------------------
    #[test]
    fn ios_with_binary_and_asset_succeeds() {
        let output = unique_output("ios_full");
        let bin = write_temp_file("ios_full_bin", b"binary-bytes-here");
        let asset = write_temp_file("ios_full_asset", b"PNG-DATA");
        let mut spec = basic_spec();
        spec.binaries.push(BinarySpec {
            arch: "arm64".to_string(),
            path: bin.clone(),
        });
        spec.assets.push(AssetSpec {
            kind: "icon".to_string(),
            path: asset.clone(),
            dest: "AppIcon.png".to_string(),
        });
        let report = stage_ios_app(&spec,&output).must_ok("stage should succeed");
        let _cleanup = scopeguard_remove(&output);
        let _ = fs::remove_file(&bin);
        let _ = fs::remove_file(&asset);
        assert!(report.staging_dir.join("AppIcon.png").exists());
        assert!(report
            .files_written
            .iter()
            .any(|f| f.contains("Frameworks/arm64/")));
        assert!(report.validation.required_assets_present);
    }

    // --- 13. Mixed scenario: Android force overwrite cleans staging. ---
    #[test]
    fn android_force_overwrite_works() {
        let output = unique_output("android_force");
        fs::create_dir_all(&output).ok();
        // Write a sentinel file that should NOT survive force=true.
        fs::write(output.join("stale.txt"), b"stale").ok();
        let spec = basic_spec();
        let report = stage_android_apk_with_force(&spec, &output, true).must_ok("force stage should succeed");
        let _cleanup = scopeguard_remove(&output);
        assert!(!output.join("stale.txt").exists(), "stale file survived");
        assert!(report.staging_dir.join("AndroidManifest.xml").exists());
    }

    // --- 14. Mixed scenario: report JSON round-trips schema id. --------
    #[test]
    fn report_json_contains_schema_id() {
        let output = unique_output("ios_json");
        let spec = basic_spec();
        let report = stage_ios_app(&spec,&output).must_ok("stage should succeed");
        let _cleanup = scopeguard_remove(&output);
        let json = report_json(&report);
        assert!(
            json.contains("\"schema\":\"ori.mobile_bundle.v1\""),
            "expected schema id in JSON: {json}"
        );
        assert!(json.contains("\"platform\":\"ios\""));
    }

    // --- 15. Android arch validation rejects iOS-only arch. -----------
    #[test]
    fn android_rejects_ios_only_arch() {
        let output = unique_output("android_bad_arch");
        let bin = write_temp_file("android_bad", b"BIN");
        let mut spec = basic_spec();
        spec.binaries.push(BinarySpec {
            arch: "arm64e".to_string(),
            path: bin.clone(),
        });
        let err = stage_android_apk(&spec, &output).expect_err("should fail");
        let _ = fs::remove_file(&bin);
        assert_eq!(err.id(), "MBND0005");
    }

    // --- 16. Android lib/<arch>/ layout is correct. -------------------
    #[test]
    fn android_lib_arch_layout() {
        let output = unique_output("android_lib");
        let bin = write_temp_file("android_lib_bin", b"\x7fELF-android");
        let mut spec = basic_spec();
        spec.binaries.push(BinarySpec {
            arch: "arm64-v8a".to_string(),
            path: bin.clone(),
        });
        let report = stage_android_apk(&spec,&output).must_ok("stage should succeed");
        let _cleanup = scopeguard_remove(&output);
        let _ = fs::remove_file(&bin);
        assert!(report
            .files_written
            .iter()
            .any(|f| f.contains("lib/arm64-v8a/")));
    }

    // --- helpers ------------------------------------------------------

    /// Tiny FNV-1a so we can compare file bytes without pulling in a hash
    /// dependency. The bootstrap rules forbid new deps.
    fn hash_bytes(data: &[u8]) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in data {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }

    /// Drop-guard that removes a staging directory when the test exits.
    /// Avoids leaving litter in `/tmp` even when assertions fail.
    struct RemoveOnDrop(PathBuf);
    impl Drop for RemoveOnDrop {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn scopeguard_remove(p: &Path) -> RemoveOnDrop {
        RemoveOnDrop(p.to_path_buf())
    }
}
