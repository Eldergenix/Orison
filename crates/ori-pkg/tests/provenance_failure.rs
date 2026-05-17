//! Provenance verifier failure modes.
//!
//! Three negative paths are exercised:
//!
//! * The provenance JSON omits the `signature` field entirely — unsigned.
//! * The provenance JSON sets `signature` to the bootstrap magic
//!   (`BOOTSTRAP_SIGNATURE`) which is the only currently accepted value —
//!   verified must be `true` to keep the positive control on the test.
//! * The provenance JSON sets `signature` to something else — accepted by
//!   the schema but rejected by the bootstrap policy.
//!
//! In every failure case the verifier must return `verified=false` AND a
//! non-empty `notes` field so downstream tooling (the supply-chain dashboard
//! and `ori publish`) can surface a human-readable reason.

use ori_pkg::provenance::{verify_provenance, BOOTSTRAP_SIGNATURE, PROVENANCE_SCHEMA};

fn make_payload(signature_field: Option<&str>) -> String {
    match signature_field {
        Some(sig) => format!(
            r#"{{
  "package": "demo",
  "version": "0.1.0",
  "source_uri": "git+https://example.test/demo",
  "built_by": "ori-cli@0.1.1",
  "build_time": "2026-05-16T00:00:00Z",
  "signature": "{sig}"
}}"#
        ),
        None => r#"{
  "package": "demo",
  "version": "0.1.0",
  "source_uri": "git+https://example.test/demo",
  "built_by": "ori-cli@0.1.1",
  "build_time": "2026-05-16T00:00:00Z"
}"#
        .to_string(),
    }
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn missing_signature_is_unverified_with_notes() {
    let result = verify_provenance(&make_payload(None));
    assert_eq!(result.schema, PROVENANCE_SCHEMA, "schema header drift");
    assert!(
        !result.verified,
        "unsigned provenance must NOT be marked verified, got {result:?}"
    );
    assert_eq!(
        result.signature, None,
        "unsigned input must preserve signature=None"
    );
    let notes = result.notes.as_deref().unwrap_or("");
    assert!(
        !notes.is_empty(),
        "notes must be populated for the unsigned path"
    );
    assert!(
        notes.contains("unsigned"),
        "notes should explain the unsigned failure mode, got {notes:?}"
    );
    // Package/version must round-trip even on failure so the dashboard can
    // group findings by artifact.
    assert_eq!(result.package, "demo");
    assert_eq!(result.version, "0.1.0");
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn bootstrap_signature_is_verified() {
    // Positive control: the failure tests are only meaningful if the
    // accepting path still accepts.
    let result = verify_provenance(&make_payload(Some(BOOTSTRAP_SIGNATURE)));
    assert!(
        result.verified,
        "bootstrap magic signature must verify, got {result:?}"
    );
    assert_eq!(
        result.signature.as_deref(),
        Some(BOOTSTRAP_SIGNATURE),
        "verified signature must round-trip"
    );
    let notes = result.notes.as_deref().unwrap_or("");
    assert!(
        notes.contains("bootstrap"),
        "verified notes should mention bootstrap self-attestation, got {notes:?}"
    );
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn unrecognised_signature_is_rejected_with_notes() {
    let result = verify_provenance(&make_payload(Some("sigstore:unknown-issuer")));
    assert!(
        !result.verified,
        "unrecognised signature must NOT verify, got {result:?}"
    );
    assert_eq!(
        result.signature.as_deref(),
        Some("sigstore:unknown-issuer"),
        "signature value must be echoed back so audit logs preserve it"
    );
    let notes = result.notes.as_deref().unwrap_or("");
    assert!(
        !notes.is_empty(),
        "notes must be populated on policy rejection"
    );
    assert!(
        notes.contains("bootstrap policy"),
        "notes should reference the bootstrap policy mismatch, got {notes:?}"
    );
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn malformed_json_is_rejected_with_parse_note() {
    let result = verify_provenance("{not json");
    assert!(
        !result.verified,
        "malformed JSON must NOT verify, got {result:?}"
    );
    let notes = result.notes.as_deref().unwrap_or("");
    assert!(
        notes.contains("parse error"),
        "notes should describe the parse failure, got {notes:?}"
    );
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn missing_required_field_is_rejected_with_field_note() {
    // No `package` — the verifier should call this out by name.
    let payload = r#"{
  "version": "0.1.0",
  "source_uri": "git+https://example.test/demo",
  "signature": "self-attested:bootstrap"
}"#;
    let result = verify_provenance(payload);
    assert!(
        !result.verified,
        "missing required field must NOT verify, got {result:?}"
    );
    let notes = result.notes.as_deref().unwrap_or("");
    assert!(
        notes.contains("package"),
        "notes should name the missing field, got {notes:?}"
    );
}
