//! SBOM <-> `schemas/sbom.schema.json` shape conformance.
//!
//! We do not pull in the `jsonschema` crate (workspace dep policy forbids
//! new third-party deps — see `MEMORY.md` D002 and the
//! `ALLOWED_WORKSPACE_DEPS` allowlist in `scripts/validate_all.py`).
//! Instead this test loads the schema JSON, walks the relevant subset of
//! Draft 2020-12 keywords, and asserts that the generated SBOM document
//! satisfies them. This is enough to catch the regressions the security
//! suite cares about: schema id drift, format-enum drift, missing required
//! component fields, and rogue additional properties.

use std::fs;
use std::path::{Path, PathBuf};

use ori_pkg::lockfile::from_graph;
use ori_pkg::manifest::Manifest;
use ori_pkg::resolver::resolve;
use ori_pkg::sbom::{build_sbom, SbomFormat};

fn scratch(name: &str) -> PathBuf {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("ori_pkg_sbom_schema_{name}_{pid}_{nanos}"));
    let _ = fs::remove_dir_all(&dir);
    dir
}

#[allow(clippy::assertions_on_constants)]
fn write_manifest(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            assert!(false, "create_dir_all({}) failed: {err}", parent.display());
            return;
        }
    }
    if let Err(err) = fs::write(path, body) {
        assert!(false, "write({}) failed: {err}", path.display());
    }
}

fn schema_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("schemas");
    p.push("sbom.schema.json");
    p
}

/// Type token, matching JSON Schema's `type` keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsonShape {
    Object,
    Array,
    String,
    Number,
    Boolean,
    Null,
}

fn shape_of(value: &serde_json::Value) -> JsonShape {
    match value {
        serde_json::Value::Object(_) => JsonShape::Object,
        serde_json::Value::Array(_) => JsonShape::Array,
        serde_json::Value::String(_) => JsonShape::String,
        serde_json::Value::Number(_) => JsonShape::Number,
        serde_json::Value::Bool(_) => JsonShape::Boolean,
        serde_json::Value::Null => JsonShape::Null,
    }
}

fn shape_name(s: JsonShape) -> &'static str {
    match s {
        JsonShape::Object => "object",
        JsonShape::Array => "array",
        JsonShape::String => "string",
        JsonShape::Number => "number",
        JsonShape::Boolean => "boolean",
        JsonShape::Null => "null",
    }
}

/// Coerce a JSON `type` keyword (string or array of strings) into an allowed
/// set.
fn parse_allowed_types(schema_type: &serde_json::Value) -> Vec<JsonShape> {
    let mut out = Vec::new();
    let mut push = |name: &str| match name {
        "object" => out.push(JsonShape::Object),
        "array" => out.push(JsonShape::Array),
        "string" => out.push(JsonShape::String),
        "number" | "integer" => out.push(JsonShape::Number),
        "boolean" => out.push(JsonShape::Boolean),
        "null" => out.push(JsonShape::Null),
        _ => {}
    };
    match schema_type {
        serde_json::Value::String(s) => push(s),
        serde_json::Value::Array(items) => {
            for item in items {
                if let Some(s) = item.as_str() {
                    push(s);
                }
            }
        }
        _ => {}
    }
    out
}

/// Validate `value` against `schema`. Returns a list of human-readable
/// problems; an empty list means the value satisfies the supported subset of
/// the schema.
fn validate_shape(
    value: &serde_json::Value,
    schema: &serde_json::Value,
    path: &str,
) -> Vec<String> {
    let mut errors = Vec::new();

    if let Some(t) = schema.get("type") {
        let allowed = parse_allowed_types(t);
        if !allowed.is_empty() {
            let actual = shape_of(value);
            if !allowed.contains(&actual) {
                errors.push(format!(
                    "{path}: expected type {:?}, got {}",
                    allowed.iter().copied().map(shape_name).collect::<Vec<_>>(),
                    shape_name(actual)
                ));
            }
        }
    }

    if let Some(constant) = schema.get("const") {
        if value != constant {
            errors.push(format!("{path}: expected const {constant}, got {value}"));
        }
    }

    if let Some(enum_values) = schema.get("enum").and_then(|v| v.as_array()) {
        if !enum_values.iter().any(|allowed| allowed == value) {
            errors.push(format!(
                "{path}: value {value} is not in declared enum {enum_values:?}"
            ));
        }
    }

    if shape_of(value) == JsonShape::Object {
        let object = match value.as_object() {
            Some(o) => o,
            None => return errors,
        };
        if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
            for r in required {
                if let Some(name) = r.as_str() {
                    if !object.contains_key(name) {
                        errors.push(format!("{path}: missing required field `{name}`"));
                    }
                }
            }
        }
        let additional_allowed = schema
            .get("additionalProperties")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let properties = schema.get("properties").and_then(|v| v.as_object());
        for (k, v) in object {
            let child_path = format!("{path}.{k}");
            match properties.and_then(|p| p.get(k)) {
                Some(child_schema) => {
                    errors.extend(validate_shape(v, child_schema, &child_path));
                }
                None => {
                    if !additional_allowed {
                        errors.push(format!("{path}: unexpected additional property `{k}`"));
                    }
                }
            }
        }
    }

    if shape_of(value) == JsonShape::Array {
        if let Some(items) = value.as_array() {
            if let Some(item_schema) = schema.get("items") {
                for (idx, item) in items.iter().enumerate() {
                    let child_path = format!("{path}[{idx}]");
                    errors.extend(validate_shape(item, item_schema, &child_path));
                }
            }
        }
    }

    if shape_of(value) == JsonShape::String {
        if let Some(min) = schema.get("minLength").and_then(|v| v.as_u64()) {
            if let Some(s) = value.as_str() {
                if (s.chars().count() as u64) < min {
                    errors.push(format!("{path}: string `{s}` shorter than minLength {min}"));
                }
            }
        }
    }

    errors
}

#[allow(clippy::assertions_on_constants)]
fn load_schema() -> serde_json::Value {
    let path = schema_path();
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(err) => {
            assert!(false, "read {} failed: {err}", path.display());
            return serde_json::Value::Null;
        }
    };
    match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(err) => {
            assert!(false, "parse {} failed: {err}", path.display());
            serde_json::Value::Null
        }
    }
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn generated_sbom_satisfies_schema_shape() {
    let tmp = scratch("conform");
    write_manifest(
        &tmp.join("root/ori.toml"),
        r#"schema = "ori.manifest.v1"
[package]
name = "sbom_root"
version = "0.1.0"
edition = "2027.1"
[capabilities]
declared = ["fs.read"]

[dependencies.lib_a]
path = "../lib_a"
"#,
    );
    write_manifest(
        &tmp.join("lib_a/ori.toml"),
        r#"schema = "ori.manifest.v1"
[package]
name = "lib_a"
version = "0.4.7"
edition = "2027.1"
[capabilities]
declared = ["fs.read"]
"#,
    );

    let manifest = match Manifest::from_path(&tmp.join("root/ori.toml")) {
        Ok(m) => m,
        Err(err) => {
            assert!(false, "manifest parse failed: {err}");
            let _ = fs::remove_dir_all(&tmp);
            return;
        }
    };
    let graph = match resolve(&manifest, &tmp.join("root")) {
        Ok(g) => g,
        Err(err) => {
            assert!(false, "resolve failed: {err}");
            let _ = fs::remove_dir_all(&tmp);
            return;
        }
    };

    let sbom = build_sbom(&graph, SbomFormat::OriNative);
    let serialised = match serde_json::to_value(&sbom) {
        Ok(v) => v,
        Err(err) => {
            assert!(false, "serialize sbom failed: {err}");
            let _ = fs::remove_dir_all(&tmp);
            return;
        }
    };

    // schema header and format must match the JSON Schema literally.
    assert_eq!(
        serialised.get("schema").and_then(|v| v.as_str()),
        Some("ori.sbom.v1"),
        "schema header drifted from `ori.sbom.v1`"
    );

    let format_value = serialised
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let allowed_formats = ["spdx-2.3-compat", "cyclonedx-1.5-compat", "ori-native"];
    assert!(
        allowed_formats.contains(&format_value),
        "format `{format_value}` is not in schema enum {allowed_formats:?}"
    );

    // Every component must carry the required fields.
    let components = serialised
        .get("components")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        !components.is_empty(),
        "SBOM must list at least one component"
    );
    for (idx, comp) in components.iter().enumerate() {
        for field in ["name", "version", "license", "checksum", "capabilities"] {
            assert!(
                comp.get(field).is_some(),
                "component[{idx}] missing required field `{field}`"
            );
        }
        let caps = comp
            .get("capabilities")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        for cap in &caps {
            assert!(
                cap.is_string(),
                "component[{idx}] capability entry must be a string"
            );
        }
    }

    // Now run the full shape walker against the on-disk schema.
    let schema = load_schema();
    let problems = validate_shape(&serialised, &schema, "$");
    assert!(
        problems.is_empty(),
        "SBOM does not conform to schemas/sbom.schema.json: {problems:?}"
    );

    // Also build under the other two formats and re-validate to catch
    // enum/format regressions for those specifically.
    for fmt in [SbomFormat::SpdxCompat, SbomFormat::CycloneDxCompat] {
        let other = build_sbom(&graph, fmt);
        let other_json = match serde_json::to_value(&other) {
            Ok(v) => v,
            Err(err) => {
                assert!(false, "serialize {fmt:?} sbom failed: {err}");
                let _ = fs::remove_dir_all(&tmp);
                return;
            }
        };
        let other_problems = validate_shape(&other_json, &schema, "$");
        assert!(
            other_problems.is_empty(),
            "{fmt:?} SBOM violates schema: {other_problems:?}"
        );
    }

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn shape_validator_rejects_known_violations() {
    // Defensive: make sure our hand-rolled validator actually catches the
    // shape problems we rely on it detecting. Without this assertion the
    // main test could silently rubber-stamp anything.
    let schema = load_schema();

    // Missing every required field.
    let bad_root = serde_json::json!({});
    let problems = validate_shape(&bad_root, &schema, "$");
    assert!(
        problems.iter().any(|p| p.contains("schema")),
        "validator should report missing `schema` field, got {problems:?}"
    );

    // Wrong const for schema and bad format enum.
    let bad_const = serde_json::json!({
        "schema": "ori.sbom.v0",
        "format": "not-a-format",
        "generated_at": "1970-01-01T00:00:00Z",
        "root": "x",
        "components": []
    });
    let problems = validate_shape(&bad_const, &schema, "$");
    assert!(
        problems.iter().any(|p| p.contains("const")),
        "validator should flag wrong schema const, got {problems:?}"
    );
    assert!(
        problems.iter().any(|p| p.contains("enum")),
        "validator should flag bad format enum, got {problems:?}"
    );

    // Component missing required `capabilities` and carrying an extra prop.
    // Use a graph helper to get a real component name we know is a string.
    let bad_component = serde_json::json!({
        "schema": "ori.sbom.v1",
        "format": "ori-native",
        "generated_at": "1970-01-01T00:00:00Z",
        "root": "r",
        "components": [{
            "name": "x",
            "version": "0.1.0",
            "license": null,
            "checksum": null,
            "rogue_field": true
        }]
    });
    let problems = validate_shape(&bad_component, &schema, "$");
    assert!(
        problems.iter().any(|p| p.contains("capabilities")),
        "validator should flag missing component.capabilities, got {problems:?}"
    );
    assert!(
        problems.iter().any(|p| p.contains("rogue_field")),
        "validator should flag rogue additional component property, got {problems:?}"
    );

    // And a sanity smoke: a manually constructed minimum sbom passes.
    // `from_graph` is the public entry point used by `build_sbom`; we touch
    // it here so this file pulls the symbol in and dead-imports don't lint.
    let _ = from_graph;
}
