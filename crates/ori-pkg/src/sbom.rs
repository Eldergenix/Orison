//! SBOM (Software Bill of Materials) generator.
//!
//! Output matches `schemas/sbom.schema.json`. The `format` field uses the
//! enum string the schema requires (`ori-native`, `spdx-2.3-compat`, or
//! `cyclonedx-1.5-compat`). The bootstrap implementation populates a single
//! native shape regardless of `format` — proper SPDX/CycloneDX export will
//! arrive with the registry milestone.

use serde::{Deserialize, Serialize};

use crate::lockfile::from_graph;
use crate::resolver::ResolvedGraph;

/// Stable schema identifier.
pub const SBOM_SCHEMA: &str = "ori.sbom.v1";

/// Requested SBOM format. The bootstrap emits the same body for every choice
/// but tags it accurately so downstream tooling knows what was requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SbomFormat {
    /// Native Orison shape (default).
    #[serde(rename = "ori-native")]
    OriNative,
    /// SPDX 2.3 compatible mode (bootstrap stub).
    #[serde(rename = "spdx-2.3-compat")]
    SpdxCompat,
    /// CycloneDX 1.5 compatible mode (bootstrap stub).
    #[serde(rename = "cyclonedx-1.5-compat")]
    CycloneDxCompat,
}

impl SbomFormat {
    /// Parse from CLI string representation.
    pub fn from_cli(value: &str) -> Option<Self> {
        match value {
            "ori-native" => Some(Self::OriNative),
            "spdx-2.3-compat" | "spdx" => Some(Self::SpdxCompat),
            "cyclonedx-1.5-compat" | "cyclonedx" => Some(Self::CycloneDxCompat),
            _ => None,
        }
    }
}

/// One SBOM component entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SbomComponent {
    /// Component name.
    pub name: String,
    /// Resolved version.
    pub version: String,
    /// SPDX-style license identifier, or `null` when unknown.
    pub license: Option<String>,
    /// Checksum string, or `null` when unavailable.
    pub checksum: Option<String>,
    /// Capability identifiers the component declares.
    pub capabilities: Vec<String>,
}

/// SBOM document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sbom {
    /// Schema identifier.
    pub schema: String,
    /// Format requested by the caller.
    pub format: SbomFormat,
    /// RFC3339 generation timestamp.
    pub generated_at: String,
    /// Name of the root package.
    pub root: String,
    /// Component list sorted by `name` for determinism.
    pub components: Vec<SbomComponent>,
}

/// Build an SBOM from a resolved graph. The `generated_at` timestamp is
/// fixed at the Unix epoch in the bootstrap to keep golden outputs
/// reproducible; real builds will inject the wall clock through the build
/// context.
pub fn build_sbom(graph: &ResolvedGraph, format: SbomFormat) -> Sbom {
    let lock = from_graph(graph);
    let mut components: Vec<SbomComponent> = lock
        .packages
        .into_iter()
        .map(|p| SbomComponent {
            name: p.name,
            version: p.version,
            license: None,
            checksum: Some(format!("sha256-bootstrap:{}", p.checksum)),
            capabilities: p.capabilities,
        })
        .collect();
    components.sort_by(|a, b| a.name.cmp(&b.name));
    Sbom {
        schema: SBOM_SCHEMA.to_string(),
        format,
        generated_at: "1970-01-01T00:00:00Z".to_string(),
        root: graph.root.clone(),
        components,
    }
}
