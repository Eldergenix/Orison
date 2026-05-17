//! Provenance verifier.
//!
//! Output matches `schemas/provenance.schema.json`. Real signature
//! verification (sigstore / cosign / in-toto) is out of scope for the
//! bootstrap. The stub accepts a single magic signature, `self-attested:bootstrap`,
//! and otherwise reports `verified: false` with a structured reason.

use serde::{Deserialize, Serialize};

/// Schema identifier.
pub const PROVENANCE_SCHEMA: &str = "ori.provenance.v1";

/// Sentinel signature that the bootstrap considers valid.
pub const BOOTSTRAP_SIGNATURE: &str = "self-attested:bootstrap";

/// Verifier output. This is also the document persisted alongside artifacts,
/// so it includes every field required by the schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceVerification {
    /// Schema identifier (`ori.provenance.v1`).
    pub schema: String,
    /// Package name.
    pub package: String,
    /// Package version.
    pub version: String,
    /// Where the source was fetched from.
    pub source_uri: String,
    /// Optional VCS commit identifier.
    pub commit: Option<String>,
    /// Builder identifier (e.g. `ori-cli@0.1.1`).
    pub built_by: String,
    /// Build time in RFC3339.
    pub build_time: String,
    /// Signature string (or `null` when unsigned).
    pub signature: Option<String>,
    /// Verification result.
    pub verified: bool,
    /// Human-readable note (always populated in bootstrap output).
    pub notes: Option<String>,
}

/// Verify a provenance document supplied as JSON text.
///
/// Behaviour:
///
/// * Required fields missing → `verified=false`, with a `notes` value
///   listing the first missing field.
/// * Signature equals [`BOOTSTRAP_SIGNATURE`] → `verified=true`.
/// * Anything else → `verified=false` with a generic note.
pub fn verify_provenance(prov_json: &str) -> ProvenanceVerification {
    let raw: serde_json::Value = match serde_json::from_str(prov_json) {
        Ok(v) => v,
        Err(err) => {
            return failure(
                "",
                "",
                "",
                None,
                None,
                None,
                Some(format!("provenance JSON parse error: {err}")),
            );
        }
    };
    let pkg = raw
        .get("package")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let version = raw
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let source_uri = raw
        .get("source_uri")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let commit = raw
        .get("commit")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let built_by = raw
        .get("built_by")
        .and_then(|v| v.as_str())
        .unwrap_or("ori-cli@bootstrap")
        .to_string();
    let build_time = raw
        .get("build_time")
        .and_then(|v| v.as_str())
        .unwrap_or("1970-01-01T00:00:00Z")
        .to_string();
    let signature = raw
        .get("signature")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    for (label, value) in [
        ("package", pkg.as_str()),
        ("version", version.as_str()),
        ("source_uri", source_uri.as_str()),
    ] {
        if value.is_empty() {
            return failure(
                &pkg,
                &version,
                &source_uri,
                commit.clone(),
                signature.clone(),
                Some(built_by.clone()),
                Some(format!("missing required field `{label}`")),
            );
        }
    }

    let (verified, notes) = match signature.as_deref() {
        Some(s) if s == BOOTSTRAP_SIGNATURE => (
            true,
            Some("verified by bootstrap self-attestation".to_string()),
        ),
        Some(_) => (
            false,
            Some("signature does not match bootstrap policy".to_string()),
        ),
        None => (false, Some("provenance is unsigned".to_string())),
    };

    ProvenanceVerification {
        schema: PROVENANCE_SCHEMA.to_string(),
        package: pkg,
        version,
        source_uri,
        commit,
        built_by,
        build_time,
        signature,
        verified,
        notes,
    }
}

#[allow(clippy::too_many_arguments)]
fn failure(
    pkg: &str,
    version: &str,
    source_uri: &str,
    commit: Option<String>,
    signature: Option<String>,
    built_by: Option<String>,
    notes: Option<String>,
) -> ProvenanceVerification {
    ProvenanceVerification {
        schema: PROVENANCE_SCHEMA.to_string(),
        package: pkg.to_string(),
        version: version.to_string(),
        source_uri: source_uri.to_string(),
        commit,
        built_by: built_by.unwrap_or_else(|| "ori-cli@bootstrap".to_string()),
        build_time: "1970-01-01T00:00:00Z".to_string(),
        signature,
        verified: false,
        notes,
    }
}
