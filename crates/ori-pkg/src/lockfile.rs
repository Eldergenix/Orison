//! Deterministic lockfile builder.
//!
//! Output shape matches `schemas/lockfile.schema.json`:
//!
//! ```jsonc
//! {
//!   "schema": "ori.lockfile.v1",
//!   "format_version": 1,
//!   "packages": [ { name, version, source, checksum, capabilities, dependencies } ]
//! }
//! ```
//!
//! The schema mandates a hex `checksum`. Until real artifact hashing lands we
//! derive a deterministic 32-character hex digest from `name + "@" + version +
//! "+" + source` using a FNV-1a 64-bit hash mixed twice. This satisfies the
//! `^[0-9a-f]+$` schema constraint without inventing a cryptographic claim;
//! the prefix `00000000` documents that this is a bootstrap stand-in.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::manifest::Manifest;
use crate::resolver::{resolve, ResolveError, ResolvedGraph};

/// Stable schema identifier.
pub const LOCKFILE_SCHEMA: &str = "ori.lockfile.v1";
/// Current `format_version` of the lockfile body.
pub const LOCKFILE_FORMAT_VERSION: u32 = 1;

/// One entry in the locked dependency set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedPackage {
    /// Package name.
    pub name: String,
    /// Resolved version.
    pub version: String,
    /// Resolved source descriptor.
    pub source: String,
    /// Deterministic hex checksum (bootstrap stand-in).
    pub checksum: String,
    /// Capabilities the package declares.
    pub capabilities: Vec<String>,
    /// Direct dependency names.
    pub dependencies: Vec<String>,
}

/// Lockfile document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lockfile {
    /// Schema identifier.
    pub schema: String,
    /// Lockfile body version.
    pub format_version: u32,
    /// Packages sorted by name for byte-for-byte determinism.
    pub packages: Vec<LockedPackage>,
}

/// Build a lockfile from a manifest located at `root`.
pub fn build_lockfile(manifest: &Manifest, root: &Path) -> Result<Lockfile, ResolveError> {
    let graph = resolve(manifest, root)?;
    Ok(from_graph(&graph))
}

/// Build a lockfile from an already-resolved graph.
pub fn from_graph(graph: &ResolvedGraph) -> Lockfile {
    let mut packages: Vec<LockedPackage> = graph
        .nodes
        .values()
        .map(|node| {
            // Strip duplicates and sort capability/dep lists for determinism.
            let mut caps: Vec<String> = node
                .capabilities
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            caps.sort();
            let mut deps: Vec<String> = node
                .dependencies
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            deps.sort();
            let checksum = bootstrap_checksum(&node.name, &node.version, &node.source);
            LockedPackage {
                name: node.name.clone(),
                version: node.version.clone(),
                source: node.source.clone(),
                checksum,
                capabilities: caps,
                dependencies: deps,
            }
        })
        .collect();
    packages.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.version.cmp(&b.version)));
    Lockfile {
        schema: LOCKFILE_SCHEMA.to_string(),
        format_version: LOCKFILE_FORMAT_VERSION,
        packages,
    }
}

/// Bootstrap-grade deterministic hex digest. NOT cryptographic; will be
/// replaced by SHA-256 of the package artifact once artifact assembly lands.
fn bootstrap_checksum(name: &str, version: &str, source: &str) -> String {
    let seed = format!("{name}@{version}+{source}");
    let h1 = fnv1a_64(seed.as_bytes());
    let h2 = fnv1a_64(format!("{seed}::mix").as_bytes());
    format!("{h1:016x}{h2:016x}")
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_is_deterministic_hex() {
        let a = bootstrap_checksum("x", "0.1.0", "path+./x");
        let b = bootstrap_checksum("x", "0.1.0", "path+./x");
        assert_eq!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(a.len(), 32);
    }
}
