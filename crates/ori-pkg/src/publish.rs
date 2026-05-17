//! `ori publish` workflow (M37).
//!
//! Wires the manifest parser, the capability auditor, and the local
//! [`LocalRegistry`](crate::registry::LocalRegistry) stub together into a
//! single end-to-end `publish` operation. The workflow:
//!
//! 1. Loads and validates the manifest.
//! 2. Verifies the declared `[capabilities].declared` policy by running the
//!    in-tree audit and treating any AUD0001 (`error`) finding as a hard
//!    capability-policy failure.
//! 3. Ensures the requested registry directory exists (the bootstrap
//!    registry is a local filesystem stub; see `registry.rs`).
//! 4. Builds a deterministic [`PublishReceipt`] including a tiny stub
//!    signature (`fnv1a:<hex>`) over `manifest_hash || version || name`.
//! 5. Writes the package to the registry (skipped on `--dry-run`).
//!
//! Every failure mode is mapped to a stable diagnostic id (`PUB0001`
//! through `PUB0005`) so callers (CLI, tests, future LSP integration) can
//! react programmatically.
//!
//! The receipt embedded in [`PublishOutcome`] is the bootstrap-compatible
//! `ori.publish_receipt.v1` shape (extended with `manifest_hash`,
//! `source_count`, `bytes_estimate`, and `signature`); the outer envelope
//! is published as `ori.publish_outcome.v1`. The pre-existing
//! `ori.publish_receipt.v1` schema was not modified — extending it would
//! require a v2 schema and the bootstrap intentionally avoids that churn.
//!
//! Determinism: every iteration uses ordered collections, every
//! serialization is performed via typed `serde` structs, and the
//! signature is content-only so two identical inputs always yield byte-
//! identical receipts (timestamps live one level up in the outcome).

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::audit::{run_audit, AuditSeverity};
use crate::manifest::{Manifest, ManifestError, ManifestErrorKind};
use crate::registry::{fnv1a_hex, LocalRegistry, PackageEntry, RegistryError};
use crate::resolver::resolve;

/// Schema id for the publish outcome envelope written to stdout/JSON.
pub const PUBLISH_OUTCOME_SCHEMA: &str = "ori.publish_outcome.v1";

/// Prefix used by the bootstrap stub signature. Real cryptographic signing
/// (e.g. cosign / sigstore / sigsum) is post-1.0; this prefix is the
/// migration anchor: every future signer must produce strings prefixed by
/// the algorithm name, and `fnv1a:` is reserved for the bootstrap stub.
pub const SIGNATURE_PREFIX: &str = "fnv1a:";

/// Request to publish a manifest into a (local) registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishRequest {
    /// Filesystem path to the manifest. Both `path/to/dir` (resolved to
    /// `dir/ori.toml`) and `path/to/ori.toml` are accepted by the CLI;
    /// the library expects the resolved file path.
    pub manifest_path: PathBuf,
    /// Registry endpoint. The bootstrap accepts a filesystem path; future
    /// real registry URLs (`https://…`) will be tagged here too.
    pub registry: String,
    /// Optional human-readable tag for the publish (e.g. `"beta"`). The
    /// bootstrap registry stores it inside the outcome but never as part
    /// of the package identifier itself.
    pub tag: Option<String>,
}

/// Status of a single publish attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PublishStatus {
    /// Publish succeeded (or was simulated under `--dry-run`).
    Accepted,
    /// Publish was intentionally not executed. The bootstrap uses this
    /// for `--dry-run`.
    Skipped {
        /// Why the publish was skipped (machine-friendly short string).
        reason: String,
    },
    /// Publish failed but produced a structured diagnostic. The outer
    /// `publish()` function still returns `Err(PublishError)` in this
    /// case; this variant is reserved for callers that need to embed a
    /// rejection envelope inside a non-error JSON document.
    Rejected {
        /// Stable diagnostic id (e.g. `PUB0002`).
        code: String,
        /// Human-readable reason.
        reason: String,
    },
}

/// Receipt fields persisted alongside an accepted publish.
///
/// The schema id matches `crate::registry::PUBLISH_RECEIPT_SCHEMA` so the
/// shared `ori.publish_receipt.v1` contract still validates the first six
/// fields. Additional fields (`manifest_hash`, `source_count`,
/// `bytes_estimate`, `signature`) live on the outer envelope as siblings —
/// they are *not* serialised into the receipt at the registry layer to
/// avoid breaking the v1 schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishReceipt {
    /// FNV-1a 64-bit hex digest of the normalised manifest bytes
    /// (`schema|name|version|edition|caps...|deps...`). The normalisation
    /// guarantees byte-for-byte stability across whitespace-only edits to
    /// the on-disk manifest, which is what every downstream consumer
    /// needs for caching keys.
    pub manifest_hash: String,
    /// Number of source files counted under the manifest root (a coarse
    /// approximation of "what's in the package"). The bootstrap is
    /// deliberately conservative: it counts `.ori` files only.
    pub source_count: usize,
    /// Conservative bytes-on-the-wire estimate. The bootstrap simply
    /// sums the byte length of every `.ori` file plus the manifest text
    /// itself; a real tarball pipeline will replace this.
    pub bytes_estimate: u64,
    /// Bootstrap stub signature, `fnv1a:<hex>`. See [`SIGNATURE_PREFIX`].
    /// [`verify_signature`] round-trips this value against the same
    /// inputs used to compute it.
    pub signature: String,
}

/// Full outcome envelope written by `ori publish`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishOutcome {
    /// Stable schema id (`"ori.publish_outcome.v1"`).
    pub schema: &'static str,
    /// Status of the publish.
    pub status: PublishStatus,
    /// Package name.
    pub package: String,
    /// Package version.
    pub version: String,
    /// Unix-epoch seconds at which the publish completed.
    pub published_at: u64,
    /// Deterministic receipt fields (see [`PublishReceipt`]).
    pub receipt: PublishReceipt,
    /// Optional registry tag (mirrors [`PublishRequest::tag`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Registry endpoint the publish targeted.
    pub registry: String,
    /// Non-fatal warnings collected during the publish run. Ordered for
    /// determinism.
    pub warnings: Vec<String>,
}

/// Diagnostic-bearing failure type for [`publish`].
///
/// Variant identifiers map 1:1 to the published `PUB0001`–`PUB0005` codes.
#[derive(Debug)]
pub enum PublishError {
    /// `PUB0001` — manifest missing required fields or otherwise rejected
    /// by [`Manifest::parse`].
    ManifestInvalid {
        /// Human-readable detail (the first manifest validation error).
        detail: String,
    },
    /// `PUB0002` — `{name, version}` already exists in the registry.
    VersionConflict {
        /// Package name.
        name: String,
        /// Conflicting version.
        version: String,
    },
    /// `PUB0003` — the manifest's declared capability policy does not
    /// audit-clean (one or more `error`-severity findings).
    CapabilityPolicyFailed {
        /// First failing audit message.
        detail: String,
    },
    /// `PUB0004` — the registry endpoint cannot be reached. For the
    /// bootstrap this means the local directory does not exist (and was
    /// not the publish's job to create — `init` happens only on a fresh
    /// `--registry` target).
    RegistryUnreachable {
        /// Path or URL of the registry.
        registry: String,
        /// OS-level reason (`io::Error`'s display form, or a short string
        /// for non-IO failures).
        detail: String,
    },
    /// `PUB0005` — the receipt signature failed to verify on round-trip.
    /// Should be unreachable in the bootstrap (the signer and verifier
    /// share the same input) and indicates a deeper invariant violation.
    SignatureInvalid {
        /// Human-readable detail.
        detail: String,
    },
}

impl PublishError {
    /// Return the stable diagnostic id (`PUB0001`-`PUB0005`).
    pub fn code(&self) -> &'static str {
        match self {
            PublishError::ManifestInvalid { .. } => "PUB0001",
            PublishError::VersionConflict { .. } => "PUB0002",
            PublishError::CapabilityPolicyFailed { .. } => "PUB0003",
            PublishError::RegistryUnreachable { .. } => "PUB0004",
            PublishError::SignatureInvalid { .. } => "PUB0005",
        }
    }
}

impl fmt::Display for PublishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PublishError::ManifestInvalid { detail } => {
                write!(f, "PUB0001 manifest_invalid: {detail}")
            }
            PublishError::VersionConflict { name, version } => {
                write!(
                    f,
                    "PUB0002 version_conflict: {name}@{version} is already published"
                )
            }
            PublishError::CapabilityPolicyFailed { detail } => {
                write!(f, "PUB0003 capability_policy_failed: {detail}")
            }
            PublishError::RegistryUnreachable { registry, detail } => {
                write!(
                    f,
                    "PUB0004 registry_unreachable: {registry} is unreachable: {detail}"
                )
            }
            PublishError::SignatureInvalid { detail } => {
                write!(f, "PUB0005 signature_invalid: {detail}")
            }
        }
    }
}

impl std::error::Error for PublishError {}

impl From<ManifestError> for PublishError {
    fn from(value: ManifestError) -> Self {
        PublishError::ManifestInvalid {
            detail: format!("{value}"),
        }
    }
}

/// Result of one publish attempt. `Ok` covers both accepted and
/// intentionally-skipped publishes (the latter via
/// [`PublishStatus::Skipped`]); `Err` covers diagnostics PUB0001-PUB0005.
pub type PublishResult = Result<PublishOutcome, PublishError>;

/// Run the publish workflow (`--dry-run`-aware via [`publish_with_options`]).
///
/// Equivalent to `publish_with_options(req, false)`.
pub fn publish(req: &PublishRequest) -> PublishResult {
    publish_with_options(req, false)
}

/// Run the publish workflow. When `dry_run` is `true`, every check runs
/// and the receipt is computed, but no bytes are written to the registry.
pub fn publish_with_options(req: &PublishRequest, dry_run: bool) -> PublishResult {
    // 1. Load + validate manifest. ManifestError → PUB0001.
    let manifest_text = match fs::read_to_string(&req.manifest_path) {
        Ok(text) => text,
        Err(err) => {
            return Err(PublishError::ManifestInvalid {
                detail: format!(
                    "could not read manifest at {}: {}",
                    req.manifest_path.display(),
                    err
                ),
            });
        }
    };
    let manifest = match Manifest::parse(&manifest_text) {
        Ok(m) => m,
        Err(err) => return Err(missing_field_error(&err)),
    };

    // 2. Capability policy check (PUB0003). Runs the in-tree auditor and
    //    fails on any AUD0001-class finding.
    let manifest_root = req
        .manifest_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut warnings: Vec<String> = Vec::new();
    match resolve(&manifest, &manifest_root) {
        Ok(graph) => {
            let report = run_audit(&manifest, &graph);
            for finding in &report.findings {
                match finding.severity {
                    AuditSeverity::Error => {
                        return Err(PublishError::CapabilityPolicyFailed {
                            detail: format!("{}: {}", finding.id, finding.message),
                        });
                    }
                    AuditSeverity::Warn => {
                        warnings.push(format!("{}: {}", finding.id, finding.message));
                    }
                    AuditSeverity::Info => {
                        // Info findings (e.g. unused declared capability)
                        // are intentionally silent during publish to avoid
                        // signal fatigue. They surface via `ori audit`.
                    }
                }
            }
        }
        Err(err) => {
            // Resolver failures are not capability-policy failures per
            // se — they are surfaced as warnings so a missing path-dep
            // does not block a bootstrap publish.
            warnings.push(format!("resolver: {err}"));
        }
    }
    warnings.sort();

    // 3. Registry reachability (PUB0004). The bootstrap registry is a
    //    local directory; "unreachable" means "doesn't exist".
    let registry_path = PathBuf::from(&req.registry);
    if !registry_path.exists() {
        return Err(PublishError::RegistryUnreachable {
            registry: req.registry.clone(),
            detail: "directory does not exist".to_string(),
        });
    }
    if !registry_path.is_dir() {
        return Err(PublishError::RegistryUnreachable {
            registry: req.registry.clone(),
            detail: "registry path is not a directory".to_string(),
        });
    }

    // 4. Compute the deterministic manifest hash + receipt. The signature
    //    is verified before we return so PUB0005 fires deterministically
    //    if the signer/verifier disagree.
    let manifest_hash = compute_manifest_hash(&manifest);
    let (source_count, bytes_estimate) = count_sources(&manifest_root, &manifest_text);
    let signature = sign(
        &manifest_hash,
        &manifest.package.version,
        &manifest.package.name,
    );
    if !verify_signature(
        &manifest_hash,
        &manifest.package.version,
        &manifest.package.name,
        &signature,
    ) {
        return Err(PublishError::SignatureInvalid {
            detail: "fnv1a stub signature did not round-trip".to_string(),
        });
    }
    let receipt = PublishReceipt {
        manifest_hash,
        source_count,
        bytes_estimate,
        signature,
    };

    // 5. Reject obvious version conflicts before writing. The registry's
    //    `publish` also catches this, but mapping it to PUB0002 here lets
    //    the caller get a structured diagnostic.
    let registry = LocalRegistry::new(&registry_path);
    if !dry_run {
        // The registry's `list` returns Ok(empty) for a fresh registry,
        // which is what we want.
        match registry.list() {
            Ok(entries) => {
                if entries.iter().any(|e| {
                    e.name == manifest.package.name && e.version == manifest.package.version
                }) {
                    return Err(PublishError::VersionConflict {
                        name: manifest.package.name.clone(),
                        version: manifest.package.version.clone(),
                    });
                }
            }
            Err(err) => {
                warnings.push(format!("could not pre-list registry: {err}"));
            }
        }
    }

    let published_at = unix_now_seconds();
    let (status, final_warnings) = if dry_run {
        let mut w = warnings.clone();
        w.push("dry-run: nothing was written to the registry".to_string());
        w.sort();
        (
            PublishStatus::Skipped {
                reason: "dry-run".to_string(),
            },
            w,
        )
    } else {
        // Synthesize tarball bytes deterministically from the receipt.
        // The bootstrap registry only stores `bytes`; a real registry
        // would receive an actual tarball over the wire.
        let tarball_bytes = synthesize_tarball(&receipt, &manifest);
        match registry.publish(&manifest, &tarball_bytes) {
            Ok(_real_receipt) => (PublishStatus::Accepted, warnings),
            Err(RegistryError::AlreadyExists(name, version)) => {
                return Err(PublishError::VersionConflict { name, version });
            }
            Err(RegistryError::Io(err)) => {
                return Err(PublishError::RegistryUnreachable {
                    registry: req.registry.clone(),
                    detail: err.to_string(),
                });
            }
            Err(RegistryError::Invalid(msg)) => {
                return Err(PublishError::ManifestInvalid { detail: msg });
            }
            Err(other) => {
                return Err(PublishError::CapabilityPolicyFailed {
                    detail: format!("registry rejected publish: {other}"),
                });
            }
        }
    };

    Ok(PublishOutcome {
        schema: PUBLISH_OUTCOME_SCHEMA,
        status,
        package: manifest.package.name.clone(),
        version: manifest.package.version.clone(),
        published_at,
        receipt,
        tag: req.tag.clone(),
        registry: req.registry.clone(),
        warnings: final_warnings,
    })
}

/// Serialise an outcome to compact JSON. Wraps `serde_json::to_string`
/// and returns a sentinel envelope on failure (which should be
/// unreachable because every field is a primitive). Output is single-
/// line so the CLI can stream it.
pub fn outcome_json(o: &PublishOutcome) -> String {
    match serde_json::to_string(o) {
        Ok(s) => s,
        Err(err) => {
            format!("{{\"schema\":\"{PUBLISH_OUTCOME_SCHEMA}\",\"error\":\"serialise: {err}\"}}")
        }
    }
}

/// Compute the canonical manifest hash used to derive the signature.
///
/// The hash inputs are explicitly normalised so whitespace-only manifest
/// edits do not invalidate the receipt and so two identical manifests
/// always hash identically regardless of section ordering on disk.
pub fn compute_manifest_hash(manifest: &Manifest) -> String {
    let mut buf = String::new();
    buf.push_str(&manifest.schema);
    buf.push('|');
    buf.push_str(&manifest.package.name);
    buf.push('|');
    buf.push_str(&manifest.package.version);
    buf.push('|');
    buf.push_str(&manifest.package.edition);
    buf.push('|');
    let mut caps: Vec<&str> = manifest
        .capabilities
        .declared
        .iter()
        .map(String::as_str)
        .collect();
    caps.sort();
    for cap in &caps {
        buf.push_str(cap);
        buf.push(',');
    }
    buf.push('|');
    let mut denied: Vec<&str> = manifest
        .capabilities
        .denied
        .iter()
        .map(String::as_str)
        .collect();
    denied.sort();
    for cap in &denied {
        buf.push_str(cap);
        buf.push(',');
    }
    buf.push('|');
    let mut deps: BTreeMap<&str, String> = BTreeMap::new();
    for (name, spec) in &manifest.dependencies {
        let label = match spec.version_text() {
            Some(v) => v.to_string(),
            None => String::new(),
        };
        deps.insert(name.as_str(), label);
    }
    for (name, label) in &deps {
        buf.push_str(name);
        buf.push('=');
        buf.push_str(label);
        buf.push(';');
    }
    fnv1a_hex(buf.as_bytes())
}

/// Verify a `fnv1a:<hex>` signature against the same inputs that
/// [`sign`] would have used. Returns `false` for any string that is not
/// prefixed with [`SIGNATURE_PREFIX`] or whose digest does not match.
pub fn verify_signature(manifest_hash: &str, version: &str, name: &str, signature: &str) -> bool {
    let Some(digest) = signature.strip_prefix(SIGNATURE_PREFIX) else {
        return false;
    };
    let expected = sign_digest(manifest_hash, version, name);
    digest == expected
}

fn sign(manifest_hash: &str, version: &str, name: &str) -> String {
    format!(
        "{SIGNATURE_PREFIX}{}",
        sign_digest(manifest_hash, version, name)
    )
}

fn sign_digest(manifest_hash: &str, version: &str, name: &str) -> String {
    let mut buf = String::new();
    buf.push_str(manifest_hash);
    buf.push('|');
    buf.push_str(version);
    buf.push('|');
    buf.push_str(name);
    fnv1a_hex(buf.as_bytes())
}

fn missing_field_error(err: &ManifestError) -> PublishError {
    let detail = match &err.kind {
        ManifestErrorKind::MissingKey(k) => format!("missing required field `{k}`"),
        ManifestErrorKind::EmptyField(k) => format!("required field `{k}` is empty"),
        other => format!("{other}"),
    };
    PublishError::ManifestInvalid { detail }
}

fn count_sources(root: &Path, manifest_text: &str) -> (usize, u64) {
    let mut count = 0usize;
    let mut bytes: u64 = manifest_text.len() as u64;
    walk_ori_sources(root, &mut count, &mut bytes);
    (count, bytes)
}

/// Recursively walk `root` counting `.ori` files. Errors at any point
/// are silently absorbed because the bootstrap publish should never fail
/// over a transient filesystem race when computing a size estimate;
/// callers see the conservative number and proceed.
fn walk_ori_sources(root: &Path, count: &mut usize, bytes: &mut u64) {
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut files: Vec<PathBuf> = Vec::new();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        } else {
            files.push(path);
        }
    }
    files.sort();
    dirs.sort();
    for path in &files {
        if path.extension().and_then(|s| s.to_str()) == Some("ori") {
            *count += 1;
            if let Ok(meta) = fs::metadata(path) {
                *bytes += meta.len();
            }
        }
    }
    for dir in &dirs {
        walk_ori_sources(dir, count, bytes);
    }
}

fn synthesize_tarball(receipt: &PublishReceipt, manifest: &Manifest) -> Vec<u8> {
    // The registry only stores bytes for accounting; a real publish
    // would push a real tarball. Synthesising a small content-addressed
    // payload keeps the bootstrap deterministic and audit-friendly.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"ori-bootstrap-tarball/v1\n");
    bytes.extend_from_slice(manifest.package.name.as_bytes());
    bytes.push(b'@');
    bytes.extend_from_slice(manifest.package.version.as_bytes());
    bytes.push(b'\n');
    bytes.extend_from_slice(receipt.manifest_hash.as_bytes());
    bytes.push(b'\n');
    bytes.extend_from_slice(receipt.signature.as_bytes());
    bytes.push(b'\n');
    bytes
}

fn unix_now_seconds() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        Err(_) => 0,
    }
}

/// Apply a yank against `registry`. Returns the up-to-date package list
/// (the same shape that `LocalRegistry::list` returns). Surfaced through
/// `crate::publish` so callers can pair publish + yank without reaching
/// into the lower-level registry module directly.
pub fn yank(registry: &str, name: &str, version: &str) -> Result<Vec<PackageEntry>, PublishError> {
    let registry_path = PathBuf::from(registry);
    if !registry_path.exists() {
        return Err(PublishError::RegistryUnreachable {
            registry: registry.to_string(),
            detail: "directory does not exist".to_string(),
        });
    }
    let reg = LocalRegistry::new(&registry_path);
    match reg.yank(name, version, "yanked via ori registry yank") {
        Ok(()) => {}
        Err(RegistryError::NotFound) => {
            return Err(PublishError::ManifestInvalid {
                detail: format!("{name}@{version} was never published"),
            });
        }
        Err(RegistryError::Io(err)) => {
            return Err(PublishError::RegistryUnreachable {
                registry: registry.to_string(),
                detail: io_detail(&err),
            });
        }
        Err(other) => {
            return Err(PublishError::CapabilityPolicyFailed {
                detail: format!("yank failed: {other}"),
            });
        }
    }
    match reg.list() {
        Ok(entries) => Ok(entries
            .into_iter()
            .filter(|e| e.name == name)
            .collect::<Vec<_>>()),
        Err(err) => Err(PublishError::ManifestInvalid {
            detail: format!("could not list registry after yank: {err}"),
        }),
    }
}

fn io_detail(err: &io::Error) -> String {
    err.to_string()
}

#[cfg(test)]
#[allow(clippy::assertions_on_constants)]
mod tests {
    use super::*;
    use std::env;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_root(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id() as u128;
        let count = COUNTER.fetch_add(1, Ordering::Relaxed) as u128;
        let dir = env::temp_dir().join(format!("ori-publish-{label}-{pid}-{nanos}-{count}"));
        if let Err(err) = fs::create_dir_all(&dir) {
            assert!(false, "could not create temp dir {}: {err}", dir.display());
        }
        dir
    }

    fn write_manifest(dir: &Path, name: &str, version: &str) -> PathBuf {
        write_manifest_with_caps(dir, name, version, &[])
    }

    fn write_manifest_with_caps(
        dir: &Path,
        name: &str,
        version: &str,
        declared: &[&str],
    ) -> PathBuf {
        let caps_line = if declared.is_empty() {
            "declared = []\n".to_string()
        } else {
            let joined: Vec<String> = declared.iter().map(|c| format!("\"{c}\"")).collect();
            format!("declared = [{}]\n", joined.join(", "))
        };
        let text = format!(
            "schema = \"ori.manifest.v1\"\n\
             [package]\n\
             name = \"{name}\"\n\
             version = \"{version}\"\n\
             edition = \"2027.1\"\n\
             [capabilities]\n\
             {caps_line}\
             denied = []\n"
        );
        let path = dir.join("ori.toml");
        if let Err(err) = fs::write(&path, text) {
            assert!(false, "could not write manifest: {err}");
        }
        path
    }

    fn ensure_registry(label: &str) -> PathBuf {
        let registry = temp_root(&format!("reg-{label}"));
        // Pre-create the layout that LocalRegistry::init would build so
        // the publish workflow's PUB0004 reachability check passes.
        if let Err(err) = LocalRegistry::new(&registry).init() {
            assert!(false, "could not init registry: {err}");
        }
        registry
    }

    #[test]
    fn successful_publish_to_temp_registry() {
        let manifest_dir = temp_root("ok-manifest");
        let manifest_path = write_manifest(&manifest_dir, "app.greet", "0.1.0");
        let registry = ensure_registry("ok");
        let req = PublishRequest {
            manifest_path,
            registry: registry.to_string_lossy().to_string(),
            tag: Some("first".to_string()),
        };
        let outcome = match publish(&req) {
            Ok(o) => o,
            Err(err) => {
                assert!(false, "publish failed: {err}");
                return;
            }
        };
        assert_eq!(outcome.schema, PUBLISH_OUTCOME_SCHEMA);
        assert_eq!(outcome.package, "app.greet");
        assert_eq!(outcome.version, "0.1.0");
        assert!(matches!(outcome.status, PublishStatus::Accepted));
        assert_eq!(outcome.tag.as_deref(), Some("first"));
        assert!(outcome.receipt.signature.starts_with(SIGNATURE_PREFIX));
        assert!(outcome.receipt.bytes_estimate >= 1);
    }

    #[test]
    fn pub0001_on_missing_manifest_fields() {
        let manifest_dir = temp_root("bad-manifest");
        // Manifest with no [package] section -> missing name/version/edition.
        let path = manifest_dir.join("ori.toml");
        if let Err(err) = fs::write(&path, "schema = \"ori.manifest.v1\"\n") {
            assert!(false, "could not write manifest: {err}");
        }
        let registry = ensure_registry("pub0001");
        let req = PublishRequest {
            manifest_path: path,
            registry: registry.to_string_lossy().to_string(),
            tag: None,
        };
        match publish(&req) {
            Err(PublishError::ManifestInvalid { detail }) => {
                assert!(
                    detail.contains("package.name") || detail.contains("missing"),
                    "unexpected detail: {detail}"
                );
            }
            other => {
                let ok = other.is_ok();
                assert!(false, "expected PUB0001, got ok={ok}");
            }
        }
    }

    #[test]
    fn pub0002_on_duplicate_publish() {
        let manifest_dir = temp_root("dup-manifest");
        let manifest_path = write_manifest(&manifest_dir, "app.dup", "0.2.0");
        let registry = ensure_registry("pub0002");
        let req = PublishRequest {
            manifest_path,
            registry: registry.to_string_lossy().to_string(),
            tag: None,
        };
        if let Err(err) = publish(&req) {
            assert!(false, "first publish failed: {err}");
        }
        match publish(&req) {
            Err(PublishError::VersionConflict { name, version }) => {
                assert_eq!(name, "app.dup");
                assert_eq!(version, "0.2.0");
            }
            Err(other) => {
                let code = other.code();
                assert!(false, "expected PUB0002, got {code}");
            }
            Ok(_) => assert!(false, "expected PUB0002, got Ok"),
        }
    }

    #[test]
    fn pub0004_when_registry_dir_missing() {
        let manifest_dir = temp_root("unreachable");
        let manifest_path = write_manifest(&manifest_dir, "app.unreach", "0.1.0");
        let phantom = env::temp_dir().join(format!(
            "ori-publish-phantom-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        let req = PublishRequest {
            manifest_path,
            registry: phantom.to_string_lossy().to_string(),
            tag: None,
        };
        match publish(&req) {
            Err(PublishError::RegistryUnreachable { registry, .. }) => {
                assert_eq!(registry, phantom.to_string_lossy().to_string());
            }
            other => {
                let ok = other.is_ok();
                assert!(false, "expected PUB0004, got ok={ok}");
            }
        }
    }

    #[test]
    fn outcome_json_contains_schema_field() {
        let manifest_dir = temp_root("json-schema");
        let manifest_path = write_manifest(&manifest_dir, "app.json", "0.1.0");
        let registry = ensure_registry("json");
        let req = PublishRequest {
            manifest_path,
            registry: registry.to_string_lossy().to_string(),
            tag: None,
        };
        let outcome = match publish(&req) {
            Ok(o) => o,
            Err(err) => {
                assert!(false, "publish failed: {err}");
                return;
            }
        };
        let json = outcome_json(&outcome);
        assert!(
            json.contains("\"schema\":\"ori.publish_outcome.v1\""),
            "bad json: {json}"
        );
        assert!(
            json.contains("\"signature\":\"fnv1a:"),
            "missing signature: {json}"
        );
    }

    #[test]
    fn determinism_same_input_same_receipt_twice() {
        let manifest_dir = temp_root("det-manifest");
        let manifest_path = write_manifest(&manifest_dir, "app.det", "0.3.0");
        let registry_a = ensure_registry("det-a");
        let registry_b = ensure_registry("det-b");
        let req_a = PublishRequest {
            manifest_path: manifest_path.clone(),
            registry: registry_a.to_string_lossy().to_string(),
            tag: None,
        };
        let req_b = PublishRequest {
            manifest_path,
            registry: registry_b.to_string_lossy().to_string(),
            tag: None,
        };
        let oa = match publish(&req_a) {
            Ok(o) => o,
            Err(err) => {
                assert!(false, "publish A failed: {err}");
                return;
            }
        };
        let ob = match publish(&req_b) {
            Ok(o) => o,
            Err(err) => {
                assert!(false, "publish B failed: {err}");
                return;
            }
        };
        // The receipt portion is the deterministic surface; everything
        // about it must round-trip byte-for-byte.
        assert_eq!(oa.receipt, ob.receipt);
        assert_eq!(oa.package, ob.package);
        assert_eq!(oa.version, ob.version);
    }

    #[test]
    fn signature_roundtrips_through_verify_helper() {
        let manifest_dir = temp_root("sig-manifest");
        let manifest_path = write_manifest(&manifest_dir, "app.sig", "1.0.0");
        let registry = ensure_registry("sig");
        let req = PublishRequest {
            manifest_path,
            registry: registry.to_string_lossy().to_string(),
            tag: None,
        };
        let outcome = match publish(&req) {
            Ok(o) => o,
            Err(err) => {
                assert!(false, "publish failed: {err}");
                return;
            }
        };
        assert!(verify_signature(
            &outcome.receipt.manifest_hash,
            &outcome.version,
            &outcome.package,
            &outcome.receipt.signature,
        ));
        // Tampering with any input invalidates the signature.
        assert!(!verify_signature(
            &outcome.receipt.manifest_hash,
            &outcome.version,
            "wrong.name",
            &outcome.receipt.signature,
        ));
        assert!(!verify_signature(
            &outcome.receipt.manifest_hash,
            "9.9.9",
            &outcome.package,
            &outcome.receipt.signature,
        ));
        // Non-fnv1a signatures are rejected outright.
        assert!(!verify_signature(
            &outcome.receipt.manifest_hash,
            &outcome.version,
            &outcome.package,
            "sha256:deadbeef",
        ));
    }

    #[test]
    fn dry_run_does_not_write_to_registry() {
        let manifest_dir = temp_root("dry-manifest");
        let manifest_path = write_manifest(&manifest_dir, "app.dry", "0.1.0");
        let registry = ensure_registry("dry");
        let req = PublishRequest {
            manifest_path,
            registry: registry.to_string_lossy().to_string(),
            tag: None,
        };
        let outcome = match publish_with_options(&req, true) {
            Ok(o) => o,
            Err(err) => {
                assert!(false, "dry-run publish failed: {err}");
                return;
            }
        };
        match &outcome.status {
            PublishStatus::Skipped { reason } => assert_eq!(reason, "dry-run"),
            other => {
                let kind = match other {
                    PublishStatus::Accepted => "accepted",
                    PublishStatus::Rejected { .. } => "rejected",
                    PublishStatus::Skipped { .. } => "skipped",
                };
                assert!(false, "expected Skipped, got {kind}");
            }
        }
        // Listing the registry must return zero entries.
        let reg = LocalRegistry::new(&registry);
        match reg.list() {
            Ok(entries) => assert!(
                entries.is_empty(),
                "dry-run wrote {} entries",
                entries.len()
            ),
            Err(err) => assert!(false, "list failed: {err}"),
        }
    }

    #[test]
    fn dry_run_then_real_publish_still_works() {
        let manifest_dir = temp_root("dry-then-real-manifest");
        let manifest_path = write_manifest(&manifest_dir, "app.combo", "0.1.0");
        let registry = ensure_registry("combo");
        let req = PublishRequest {
            manifest_path,
            registry: registry.to_string_lossy().to_string(),
            tag: None,
        };
        let dry = match publish_with_options(&req, true) {
            Ok(o) => o,
            Err(err) => {
                assert!(false, "dry-run failed: {err}");
                return;
            }
        };
        let real = match publish_with_options(&req, false) {
            Ok(o) => o,
            Err(err) => {
                assert!(false, "real publish failed: {err}");
                return;
            }
        };
        // Receipt must match between dry-run and real publish (content
        // addressed by the same manifest).
        assert_eq!(dry.receipt, real.receipt);
        assert!(matches!(real.status, PublishStatus::Accepted));
    }

    #[test]
    fn yank_marks_package_yanked_in_listing() {
        let manifest_dir = temp_root("yank-manifest");
        let manifest_path = write_manifest(&manifest_dir, "app.yk", "0.1.0");
        let registry = ensure_registry("yank-flow");
        let req = PublishRequest {
            manifest_path,
            registry: registry.to_string_lossy().to_string(),
            tag: None,
        };
        if let Err(err) = publish(&req) {
            assert!(false, "publish failed: {err}");
        }
        let entries = match yank(&req.registry, "app.yk", "0.1.0") {
            Ok(v) => v,
            Err(err) => {
                assert!(false, "yank failed: {err}");
                return;
            }
        };
        assert_eq!(entries.len(), 1);
        assert!(entries[0].yanked, "package should be yanked");
        assert_eq!(entries[0].name, "app.yk");
        assert_eq!(entries[0].version, "0.1.0");
    }

    #[test]
    fn manifest_hash_is_stable_across_whitespace_variants() {
        let a = match Manifest::parse(
            "schema = \"ori.manifest.v1\"\n[package]\nname = \"x\"\nversion = \"0.1.0\"\nedition = \"2027.1\"\n",
        ) {
            Ok(m) => m,
            Err(err) => {
                assert!(false, "parse A failed: {err}");
                return;
            }
        };
        let b = match Manifest::parse(
            "schema = \"ori.manifest.v1\"\n\n[package]\n  name = \"x\"\n  version = \"0.1.0\"\n  edition = \"2027.1\"\n",
        ) {
            Ok(m) => m,
            Err(err) => {
                assert!(false, "parse B failed: {err}");
                return;
            }
        };
        assert_eq!(compute_manifest_hash(&a), compute_manifest_hash(&b));
    }
}
