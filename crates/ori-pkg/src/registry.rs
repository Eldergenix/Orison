//! Local-filesystem-backed Orison registry protocol stub (milestone M10).
//!
//! Real networked registries require a server, authentication, signature
//! verification, and a distribution protocol — none of which belong in the
//! bootstrap. This module instead implements the smallest workable shape of
//! that protocol against a local directory, so downstream commands
//! (`ori publish`, `ori fetch`, `ori registry list`, `ori registry yank`)
//! can be exercised end-to-end without a network or a daemon.
//!
//! Layout under `root`:
//!
//! ```text
//! root/
//!   index/
//!     {name}.json        -> { schema, name, versions[] }
//!   packages/
//!     {name}-{version}.ori-pkg
//!   provenance/
//!     {name}-{version}.json
//!   yanked.log           -> append-only log of yank events
//! ```
//!
//! Concurrency: this stub does **not** use file locking. Two simultaneous
//! `publish` calls for the same `{name, version}` may race in a way that
//! leaves the index in a partially updated state. That is acceptable for the
//! bootstrap because the registry is intentionally a single-user, local
//! filesystem mock; a real implementation would replace this module with a
//! transactional backend. The decision is documented here so reviewers do not
//! have to rediscover it.
//!
//! All JSON output is generated through `serde_json` against typed structs to
//! satisfy `MEMORY.md` decision D011 (contract JSON must be typed).

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::manifest::Manifest;
use crate::provenance::{BOOTSTRAP_SIGNATURE, PROVENANCE_SCHEMA};

/// Stable schema identifier for [`PublishReceipt`].
pub const PUBLISH_RECEIPT_SCHEMA: &str = "ori.publish_receipt.v1";

/// Stable schema identifier for the per-package index file.
pub const REGISTRY_INDEX_SCHEMA: &str = "ori.registry_index.v1";

/// Stable schema identifier for [`registry list`](LocalRegistry::list) output.
pub const REGISTRY_LIST_SCHEMA: &str = "ori.registry_list.v1";

/// Errors produced by [`LocalRegistry`] operations.
#[derive(Debug)]
pub enum RegistryError {
    /// The requested `{name, version}` is already published.
    AlreadyExists(String, String),
    /// The requested `{name, version}` was never published.
    NotFound,
    /// The requested `{name, version}` was yanked; the inner string is the
    /// most recent yank reason.
    Yanked(String),
    /// Underlying filesystem failure.
    Io(io::Error),
    /// Caller supplied input the registry refuses to accept (e.g. empty
    /// tarball bytes, blank package name).
    Invalid(String),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::AlreadyExists(name, version) => {
                write!(
                    f,
                    "package `{name}` version `{version}` is already published"
                )
            }
            RegistryError::NotFound => write!(f, "package not found in registry"),
            RegistryError::Yanked(reason) => write!(f, "package version was yanked: {reason}"),
            RegistryError::Io(err) => write!(f, "registry io error: {err}"),
            RegistryError::Invalid(msg) => write!(f, "invalid registry input: {msg}"),
        }
    }
}

impl std::error::Error for RegistryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RegistryError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for RegistryError {
    fn from(value: io::Error) -> Self {
        RegistryError::Io(value)
    }
}

/// Receipt returned by [`LocalRegistry::publish`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishReceipt {
    /// Stable schema identifier (`"ori.publish_receipt.v1"`).
    pub schema: &'static str,
    /// Package name.
    pub name: String,
    /// Package version.
    pub version: String,
    /// Size of the published tarball in bytes.
    pub bytes: usize,
    /// FNV-1a 64-bit hex digest of the tarball bytes (bootstrap stand-in for
    /// a real SHA-256 once artifact hashing lands).
    pub checksum: String,
    /// Unix-epoch seconds at which the publish completed. Tests treat this
    /// as the only non-deterministic field of the receipt.
    pub published_at_unix_seconds: u64,
}

/// One entry returned by [`LocalRegistry::list`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageEntry {
    /// Package name.
    pub name: String,
    /// Version string.
    pub version: String,
    /// Tarball size in bytes.
    pub bytes: usize,
    /// FNV-1a hex checksum of the tarball.
    pub checksum: String,
    /// Whether the most recent yank entry for this `{name, version}` is
    /// active (i.e. the package is currently yanked).
    pub yanked: bool,
}

/// On-disk per-package index file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct IndexFile {
    schema: String,
    name: String,
    /// Map keyed by version → tarball metadata. `BTreeMap` keeps the
    /// serialised JSON deterministic.
    versions: BTreeMap<String, IndexEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct IndexEntry {
    bytes: usize,
    checksum: String,
    published_at_unix_seconds: u64,
}

/// Provenance document written alongside each published tarball.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredProvenance {
    schema: String,
    package: String,
    version: String,
    source_uri: String,
    commit: Option<String>,
    built_by: String,
    build_time: String,
    signature: String,
    verified: bool,
    notes: Option<String>,
}

/// Local filesystem-backed Orison registry.
pub struct LocalRegistry {
    /// Root directory under which `index/`, `packages/`, and `provenance/`
    /// live.
    pub root: PathBuf,
}

impl LocalRegistry {
    /// Construct a registry handle rooted at `root`. Call [`Self::init`]
    /// before publishing if the directory layout may not yet exist.
    pub fn new<P: Into<PathBuf>>(root: P) -> Self {
        Self { root: root.into() }
    }

    /// Idempotently create the `index/`, `packages/`, and `provenance/`
    /// subdirectories under [`Self::root`]. Existing files are left intact;
    /// this method only ever creates the directories themselves.
    pub fn init(&self) -> io::Result<()> {
        fs::create_dir_all(&self.root)?;
        fs::create_dir_all(self.index_dir())?;
        fs::create_dir_all(self.packages_dir())?;
        fs::create_dir_all(self.provenance_dir())?;
        Ok(())
    }

    /// Publish a tarball for `manifest.package.name @ manifest.package.version`.
    ///
    /// Returns [`RegistryError::Invalid`] if `tarball_bytes` is empty or the
    /// manifest's name/version are blank. Returns
    /// [`RegistryError::AlreadyExists`] if the package+version is already on
    /// disk.
    pub fn publish(
        &self,
        manifest: &Manifest,
        tarball_bytes: &[u8],
    ) -> Result<PublishReceipt, RegistryError> {
        let name = manifest.package.name.trim();
        let version = manifest.package.version.trim();
        if name.is_empty() {
            return Err(RegistryError::Invalid(
                "manifest.package.name is empty".to_string(),
            ));
        }
        if version.is_empty() {
            return Err(RegistryError::Invalid(
                "manifest.package.version is empty".to_string(),
            ));
        }
        if tarball_bytes.is_empty() {
            return Err(RegistryError::Invalid(
                "tarball is empty; refusing to publish zero bytes".to_string(),
            ));
        }

        self.init()?;

        let tarball_path = self.tarball_path(name, version);
        if tarball_path.exists() {
            return Err(RegistryError::AlreadyExists(
                name.to_string(),
                version.to_string(),
            ));
        }

        let checksum = fnv1a_hex(tarball_bytes);
        let published_at = unix_now_seconds();

        // Write tarball, then provenance, then update the index. Failures
        // partway through leave a recoverable state because `publish` is
        // idempotent on re-attempt only if the tarball isn't yet present.
        fs::write(&tarball_path, tarball_bytes)?;

        let provenance = StoredProvenance {
            schema: PROVENANCE_SCHEMA.to_string(),
            package: name.to_string(),
            version: version.to_string(),
            source_uri: format!("local-registry+{}", self.root.display()),
            commit: None,
            built_by: "ori-cli@bootstrap".to_string(),
            build_time: "1970-01-01T00:00:00Z".to_string(),
            signature: BOOTSTRAP_SIGNATURE.to_string(),
            verified: true,
            notes: Some("self-attested by local registry stub".to_string()),
        };
        let provenance_json = serde_json::to_string_pretty(&provenance).map_err(|err| {
            RegistryError::Invalid(format!("could not serialise provenance: {err}"))
        })?;
        fs::write(self.provenance_path(name, version), provenance_json)?;

        self.upsert_index(
            name,
            version,
            IndexEntry {
                bytes: tarball_bytes.len(),
                checksum: checksum.clone(),
                published_at_unix_seconds: published_at,
            },
        )?;

        Ok(PublishReceipt {
            schema: PUBLISH_RECEIPT_SCHEMA,
            name: name.to_string(),
            version: version.to_string(),
            bytes: tarball_bytes.len(),
            checksum,
            published_at_unix_seconds: published_at,
        })
    }

    /// Read the published tarball bytes for `{name, version}`.
    ///
    /// Returns [`RegistryError::Yanked`] (with the most recent reason) if the
    /// package has been yanked since publish.
    pub fn fetch(&self, name: &str, version: &str) -> Result<Vec<u8>, RegistryError> {
        if let Some(reason) = self.latest_yank_reason(name, version)? {
            return Err(RegistryError::Yanked(reason));
        }
        let path = self.tarball_path(name, version);
        match fs::read(&path) {
            Ok(bytes) => Ok(bytes),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Err(RegistryError::NotFound),
            Err(err) => Err(RegistryError::Io(err)),
        }
    }

    /// List every published package known to this registry. The result is
    /// sorted by `(name, version)` to keep CLI output deterministic.
    pub fn list(&self) -> Result<Vec<PackageEntry>, RegistryError> {
        let index_dir = self.index_dir();
        if !index_dir.exists() {
            return Ok(Vec::new());
        }
        let mut entries: Vec<PackageEntry> = Vec::new();
        for dir_entry in fs::read_dir(&index_dir)? {
            let dir_entry = dir_entry?;
            let path = dir_entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let text = fs::read_to_string(&path)?;
            let index: IndexFile = serde_json::from_str(&text).map_err(|err| {
                RegistryError::Invalid(format!("corrupt index file {}: {err}", path.display()))
            })?;
            for (version, meta) in &index.versions {
                let yanked = self.latest_yank_reason(&index.name, version)?.is_some();
                entries.push(PackageEntry {
                    name: index.name.clone(),
                    version: version.clone(),
                    bytes: meta.bytes,
                    checksum: meta.checksum.clone(),
                    yanked,
                });
            }
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.version.cmp(&b.version)));
        Ok(entries)
    }

    /// Mark `{name, version}` as yanked. Subsequent [`Self::fetch`] calls
    /// return [`RegistryError::Yanked`]. The reason is sanitised before
    /// being written to the on-disk log: control characters (`\n`, `\r`,
    /// `\0`) are stripped so a single yank cannot inject extra log lines.
    pub fn yank(&self, name: &str, version: &str, reason: &str) -> Result<(), RegistryError> {
        if !self.tarball_path(name, version).exists() {
            return Err(RegistryError::NotFound);
        }
        self.init()?;
        let sanitized = sanitize_reason(reason);
        let line = format!(
            "{ts}\t{name}\t{version}\t{sanitized}\n",
            ts = unix_now_seconds(),
        );
        let log_path = self.yanked_log_path();
        // Open in append mode so concurrent yanks at least don't clobber
        // each other (filesystem-level append is atomic on common UNIXes
        // for writes under PIPE_BUF). Race-loss for the index update is
        // documented at the top of the module.
        use std::io::Write as _;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;
        file.write_all(line.as_bytes())?;
        Ok(())
    }

    // ---- path helpers --------------------------------------------------

    fn index_dir(&self) -> PathBuf {
        self.root.join("index")
    }

    fn packages_dir(&self) -> PathBuf {
        self.root.join("packages")
    }

    fn provenance_dir(&self) -> PathBuf {
        self.root.join("provenance")
    }

    fn tarball_path(&self, name: &str, version: &str) -> PathBuf {
        self.packages_dir()
            .join(format!("{name}-{version}.ori-pkg"))
    }

    fn provenance_path(&self, name: &str, version: &str) -> PathBuf {
        self.provenance_dir().join(format!("{name}-{version}.json"))
    }

    fn index_path(&self, name: &str) -> PathBuf {
        self.index_dir().join(format!("{name}.json"))
    }

    fn yanked_log_path(&self) -> PathBuf {
        self.root.join("yanked.log")
    }

    fn upsert_index(
        &self,
        name: &str,
        version: &str,
        entry: IndexEntry,
    ) -> Result<(), RegistryError> {
        let path = self.index_path(name);
        let mut index = if path.exists() {
            let text = fs::read_to_string(&path)?;
            let parsed: IndexFile = serde_json::from_str(&text).map_err(|err| {
                RegistryError::Invalid(format!("corrupt index file {}: {err}", path.display()))
            })?;
            parsed
        } else {
            IndexFile {
                schema: REGISTRY_INDEX_SCHEMA.to_string(),
                name: name.to_string(),
                versions: BTreeMap::new(),
            }
        };
        index.versions.insert(version.to_string(), entry);
        let serialised = serde_json::to_string_pretty(&index).map_err(|err| {
            RegistryError::Invalid(format!("could not serialise index file: {err}"))
        })?;
        fs::write(&path, serialised)?;
        Ok(())
    }

    /// Return the most recent yank reason for `{name, version}`, or `None`
    /// if the package has not been yanked. The log is small in practice;
    /// scanning it linearly is the cheapest correct option for the bootstrap.
    fn latest_yank_reason(
        &self,
        name: &str,
        version: &str,
    ) -> Result<Option<String>, RegistryError> {
        let path = self.yanked_log_path();
        if !path.exists() {
            return Ok(None);
        }
        let text = fs::read_to_string(&path)?;
        let mut latest: Option<String> = None;
        for line in text.lines() {
            let fields: Vec<&str> = line.splitn(4, '\t').collect();
            if fields.len() != 4 {
                continue;
            }
            if fields[1] == name && fields[2] == version {
                latest = Some(fields[3].to_string());
            }
        }
        Ok(latest)
    }
}

/// FNV-1a 64-bit digest rendered as 16 hex characters. Matches the algorithm
/// used by [`crate::lockfile`] so checksums are consistent across modules.
pub fn fnv1a_hex(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn unix_now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sanitize_reason(reason: &str) -> String {
    let trimmed = reason.trim();
    let cleaned: String = trimmed
        .chars()
        .filter(|c| *c != '\n' && *c != '\r' && *c != '\0' && *c != '\t')
        .collect();
    if cleaned.is_empty() {
        "(no reason given)".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
#[allow(clippy::assertions_on_constants)]
mod tests {
    use super::*;
    use std::env;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    // Uniquify temp directory names by mixing pid, wall-clock nanos, and a
    // process-local counter so parallel tests never collide even if the
    // clock has poor resolution.
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id() as u128;
        let count = COUNTER.fetch_add(1, Ordering::Relaxed) as u128;
        let dir = env::temp_dir().join(format!("ori-registry-{label}-{pid}-{nanos}-{count}"));
        if let Err(err) = fs::create_dir_all(&dir) {
            // panic! is forbidden — surface the failure as an assertion.
            let _ = err;
            assert!(false, "could not create temp dir {}", dir.display());
        }
        dir
    }

    fn sample_manifest(name: &str, version: &str) -> Manifest {
        let text = format!(
            "schema = \"ori.manifest.v1\"\n\
             [package]\n\
             name = \"{name}\"\n\
             version = \"{version}\"\n\
             edition = \"2027.1\"\n"
        );
        match Manifest::parse(&text) {
            Ok(m) => m,
            Err(err) => {
                assert!(false, "manifest parse failed: {err}");
                // Unreachable in practice; assert!(false) aborts the test.
                Manifest {
                    schema: String::new(),
                    package: crate::manifest::PackageMeta {
                        name: String::new(),
                        version: String::new(),
                        edition: String::new(),
                        description: None,
                        license: None,
                    },
                    capabilities: crate::manifest::CapabilityDecls::default(),
                    dependencies: BTreeMap::new(),
                    scripts: BTreeMap::new(),
                }
            }
        }
    }

    #[test]
    fn init_creates_expected_subdirs() {
        let root = temp_dir("init");
        let reg = LocalRegistry::new(&root);
        if let Err(err) = reg.init() {
            assert!(false, "init failed: {err}");
        }
        assert!(root.join("index").is_dir(), "index/ missing");
        assert!(root.join("packages").is_dir(), "packages/ missing");
        assert!(root.join("provenance").is_dir(), "provenance/ missing");
    }

    #[test]
    fn init_is_idempotent() {
        let root = temp_dir("init-idempotent");
        let reg = LocalRegistry::new(&root);
        // Pre-create one subdir with a sentinel file to prove init doesn't
        // clobber it.
        if let Err(err) = fs::create_dir_all(root.join("packages")) {
            assert!(false, "setup failed: {err}");
        }
        let sentinel = root.join("packages").join("sentinel.txt");
        if let Err(err) = fs::write(&sentinel, b"keep me") {
            assert!(false, "setup failed: {err}");
        }
        if let Err(err) = reg.init() {
            assert!(false, "first init failed: {err}");
        }
        if let Err(err) = reg.init() {
            assert!(false, "second init failed: {err}");
        }
        assert!(sentinel.exists(), "init() overwrote an existing file");
    }

    #[test]
    fn publish_then_fetch_roundtrips() {
        let root = temp_dir("publish-fetch");
        let reg = LocalRegistry::new(&root);
        let manifest = sample_manifest("app.users", "0.1.0");
        let bytes = b"orison-tarball-bytes" as &[u8];
        let receipt = match reg.publish(&manifest, bytes) {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "publish failed: {err}");
                return;
            }
        };
        assert_eq!(receipt.name, "app.users");
        assert_eq!(receipt.version, "0.1.0");
        assert_eq!(receipt.bytes, bytes.len());
        let fetched = match reg.fetch("app.users", "0.1.0") {
            Ok(b) => b,
            Err(err) => {
                assert!(false, "fetch failed: {err}");
                return;
            }
        };
        assert_eq!(fetched, bytes);
    }

    #[test]
    fn duplicate_publish_returns_already_exists() {
        let root = temp_dir("dup-publish");
        let reg = LocalRegistry::new(&root);
        let manifest = sample_manifest("dup", "0.1.0");
        let bytes = b"bytes" as &[u8];
        if let Err(err) = reg.publish(&manifest, bytes) {
            assert!(false, "first publish failed: {err}");
        }
        match reg.publish(&manifest, bytes) {
            Err(RegistryError::AlreadyExists(name, version)) => {
                assert_eq!(name, "dup");
                assert_eq!(version, "0.1.0");
            }
            Err(other) => assert!(false, "expected AlreadyExists, got {other}"),
            Ok(_) => assert!(false, "expected AlreadyExists, got Ok"),
        }
    }

    #[test]
    fn list_returns_sorted_published_packages() {
        let root = temp_dir("list");
        let reg = LocalRegistry::new(&root);
        let pairs = [("zeta", "0.1.0"), ("alpha", "0.2.0"), ("alpha", "0.1.0")];
        for (name, version) in pairs.iter() {
            let manifest = sample_manifest(name, version);
            if let Err(err) = reg.publish(&manifest, b"x") {
                assert!(false, "publish {name}@{version} failed: {err}");
            }
        }
        let listed = match reg.list() {
            Ok(l) => l,
            Err(err) => {
                assert!(false, "list failed: {err}");
                return;
            }
        };
        let names: Vec<(String, String)> = listed
            .iter()
            .map(|e| (e.name.clone(), e.version.clone()))
            .collect();
        assert_eq!(
            names,
            vec![
                ("alpha".to_string(), "0.1.0".to_string()),
                ("alpha".to_string(), "0.2.0".to_string()),
                ("zeta".to_string(), "0.1.0".to_string()),
            ]
        );
    }

    #[test]
    fn yank_then_fetch_returns_yanked() {
        let root = temp_dir("yank-fetch");
        let reg = LocalRegistry::new(&root);
        let manifest = sample_manifest("yk", "0.1.0");
        if let Err(err) = reg.publish(&manifest, b"bytes") {
            assert!(false, "publish failed: {err}");
        }
        if let Err(err) = reg.yank("yk", "0.1.0", "compromised key") {
            assert!(false, "yank failed: {err}");
        }
        match reg.fetch("yk", "0.1.0") {
            Err(RegistryError::Yanked(reason)) => {
                assert_eq!(reason, "compromised key");
            }
            other => assert!(false, "expected Yanked, got {:?}", other.is_ok()),
        }
        // list() should still surface the package, but mark it yanked.
        let listed = match reg.list() {
            Ok(l) => l,
            Err(err) => {
                assert!(false, "list failed: {err}");
                return;
            }
        };
        assert_eq!(listed.len(), 1);
        assert!(listed[0].yanked);
    }

    #[test]
    fn fetch_nonexistent_returns_not_found() {
        let root = temp_dir("notfound");
        let reg = LocalRegistry::new(&root);
        if let Err(err) = reg.init() {
            assert!(false, "init failed: {err}");
        }
        match reg.fetch("ghost", "0.0.1") {
            Err(RegistryError::NotFound) => {}
            other => assert!(false, "expected NotFound, got {:?}", other.is_ok()),
        }
    }

    #[test]
    fn receipts_are_deterministic_modulo_timestamp() {
        let root_a = temp_dir("det-a");
        let root_b = temp_dir("det-b");
        let manifest = sample_manifest("det", "0.1.0");
        let bytes = b"determinism" as &[u8];

        let reg_a = LocalRegistry::new(&root_a);
        let reg_b = LocalRegistry::new(&root_b);

        let ra = match reg_a.publish(&manifest, bytes) {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "publish A failed: {err}");
                return;
            }
        };
        let rb = match reg_b.publish(&manifest, bytes) {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "publish B failed: {err}");
                return;
            }
        };
        assert_eq!(ra.schema, rb.schema);
        assert_eq!(ra.name, rb.name);
        assert_eq!(ra.version, rb.version);
        assert_eq!(ra.bytes, rb.bytes);
        assert_eq!(ra.checksum, rb.checksum);
        // The published_at timestamp is allowed to differ; everything else
        // must match byte-for-byte.
    }

    #[test]
    fn checksum_matches_fnv1a_of_bytes() {
        let root = temp_dir("checksum");
        let reg = LocalRegistry::new(&root);
        let manifest = sample_manifest("ck", "0.1.0");
        let bytes = b"hello, registry";
        let expected = fnv1a_hex(bytes);
        let receipt = match reg.publish(&manifest, bytes) {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "publish failed: {err}");
                return;
            }
        };
        assert_eq!(receipt.checksum, expected);
        assert_eq!(receipt.checksum.len(), 16);
        assert!(receipt.checksum.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn empty_tarball_is_rejected() {
        let root = temp_dir("empty");
        let reg = LocalRegistry::new(&root);
        let manifest = sample_manifest("empty", "0.1.0");
        match reg.publish(&manifest, &[]) {
            Err(RegistryError::Invalid(msg)) => {
                assert!(msg.contains("empty"), "wrong invalid msg: {msg}");
            }
            other => assert!(false, "expected Invalid, got {:?}", other.is_ok()),
        }
    }

    #[test]
    fn yank_reason_strips_control_chars() {
        let root = temp_dir("sanitize");
        let reg = LocalRegistry::new(&root);
        let manifest = sample_manifest("san", "0.1.0");
        if let Err(err) = reg.publish(&manifest, b"x") {
            assert!(false, "publish failed: {err}");
        }
        let nasty = "bad\nreason\rwith\0nuls\t";
        if let Err(err) = reg.yank("san", "0.1.0", nasty) {
            assert!(false, "yank failed: {err}");
        }
        match reg.fetch("san", "0.1.0") {
            Err(RegistryError::Yanked(reason)) => {
                assert!(!reason.contains('\n'));
                assert!(!reason.contains('\r'));
                assert!(!reason.contains('\0'));
                assert!(!reason.contains('\t'));
                assert_eq!(reason, "badreasonwithnuls");
            }
            other => assert!(false, "expected Yanked, got {:?}", other.is_ok()),
        }
    }
}
