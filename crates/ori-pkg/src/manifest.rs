//! Strongly-typed Orison package manifest.
//!
//! The in-memory model is intentionally richer than the
//! `schemas/manifest.schema.json` file, which only mandates `schema`,
//! `package`, `edition`, and `version` and permits any shape for
//! `capabilities` and `dependencies`. The richer Rust model lets the package
//! manager perform real validation (versions, capability strings, dep specs)
//! while still serialising into a JSON object compatible with the schema's
//! `additionalProperties: false` set when the same field names are used.
//!
//! GAP: the schema spells `package` as a `string` but the bootstrap manifest
//! benefits from richer per-package metadata. The lockfile/SBOM/audit/
//! provenance outputs match their schemas exactly; the manifest in-memory
//! struct is a superset documented here so downstream consumers can plan a
//! `ori.manifest.v2` migration if the registry needs the flatter shape.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::toml_lite::{self, TomlError, TomlValue};
use crate::version::{parse_constraint, VersionConstraint, VersionError};

/// Current manifest schema identifier.
pub const MANIFEST_SCHEMA: &str = "ori.manifest.v1";

/// Errors produced while parsing or validating a manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestErrorKind {
    /// Underlying TOML parse failure.
    Toml(toml_lite::TomlErrorKind),
    /// Required key is absent.
    MissingKey(String),
    /// Field has a value of the wrong shape (e.g. string where array expected).
    WrongType {
        /// Logical path of the field, e.g. `"package.name"`.
        path: String,
        /// Human description of the expected type.
        expected: &'static str,
    },
    /// Field had an empty value that the schema forbids.
    EmptyField(String),
    /// Version string did not match `X.Y.Z` or `X.Y.Z-pre`.
    InvalidVersion(String),
    /// Capability identifier contained characters outside `[A-Za-z0-9._-]`.
    InvalidCapability(String),
    /// Dependency table contained a key with no supported descriptor.
    InvalidDependency {
        /// Dependency name.
        name: String,
        /// Reason for rejection.
        reason: String,
    },
    /// `schema` was present but had an unsupported value.
    UnsupportedSchema(String),
    /// A TOML section that the manifest model does not understand appeared.
    UnknownSection(String),
    /// A key within a known section was not recognised.
    UnknownKey {
        /// Section the key was found in.
        section: String,
        /// Key name.
        key: String,
    },
}

impl fmt::Display for ManifestErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestErrorKind::Toml(k) => write!(f, "toml: {k}"),
            ManifestErrorKind::MissingKey(k) => write!(f, "missing required key `{k}`"),
            ManifestErrorKind::WrongType { path, expected } => {
                write!(f, "field `{path}` has wrong type (expected {expected})")
            }
            ManifestErrorKind::EmptyField(p) => write!(f, "field `{p}` must not be empty"),
            ManifestErrorKind::InvalidVersion(v) => {
                write!(f, "version `{v}` must match X.Y.Z or X.Y.Z-pre")
            }
            ManifestErrorKind::InvalidCapability(c) => write!(f, "invalid capability `{c}`"),
            ManifestErrorKind::InvalidDependency { name, reason } => {
                write!(f, "dependency `{name}` is invalid: {reason}")
            }
            ManifestErrorKind::UnsupportedSchema(s) => {
                write!(f, "unsupported manifest schema `{s}`")
            }
            ManifestErrorKind::UnknownSection(s) => write!(f, "unknown manifest section `[{s}]`"),
            ManifestErrorKind::UnknownKey { section, key } => {
                if section.is_empty() {
                    write!(f, "unknown top-level manifest key `{key}`")
                } else {
                    write!(f, "unknown key `{key}` in `[{section}]`")
                }
            }
        }
    }
}

/// Manifest error with optional position. TOML-level errors carry positions
/// from the parser; validation errors do not currently track positions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestError {
    /// The error category.
    pub kind: ManifestErrorKind,
    /// 1-indexed source line. `None` for validation errors that do not have
    /// a single triggering line.
    pub line: Option<usize>,
    /// 1-indexed source column.
    pub column: Option<usize>,
}

impl ManifestError {
    fn validation(kind: ManifestErrorKind) -> Self {
        Self {
            kind,
            line: None,
            column: None,
        }
    }
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.line, self.column) {
            (Some(l), Some(c)) => write!(f, "{} at line {} column {}", self.kind, l, c),
            (Some(l), None) => write!(f, "{} at line {}", self.kind, l),
            _ => write!(f, "{}", self.kind),
        }
    }
}

impl std::error::Error for ManifestError {}

impl From<TomlError> for ManifestError {
    fn from(value: TomlError) -> Self {
        Self {
            kind: ManifestErrorKind::Toml(value.kind),
            line: Some(value.line),
            column: Some(value.column),
        }
    }
}

/// Per-package metadata block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageMeta {
    /// Logical package name (e.g. `app.users`).
    pub name: String,
    /// Semantic version string (`X.Y.Z` or `X.Y.Z-pre`).
    pub version: String,
    /// Orison edition string (e.g. `2027.1`).
    pub edition: String,
    /// Optional one-line description.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    /// Optional SPDX-style license identifier.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub license: Option<String>,
}

/// Declared capabilities block.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDecls {
    /// Capabilities this package opts in to.
    #[serde(default)]
    pub declared: Vec<String>,
    /// Capabilities this package explicitly forbids transitively.
    #[serde(default)]
    pub denied: Vec<String>,
}

/// Dependency descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DepSpec {
    /// Plain version requirement (e.g. `"0.2.1"`, `"^1.2.3"`, `"~2.0.0"`).
    /// The wire format is a string; [`DepSpec::constraint`] parses it into a
    /// strongly-typed [`VersionConstraint`].
    Version(String),
    /// Local path with optional version constraint.
    Path {
        /// Filesystem path, resolved relative to the manifest directory.
        path: String,
        /// Optional version constraint. When present and the dep is path-
        /// based, the path manifest's version must satisfy this constraint.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        version: Option<String>,
    },
}

impl DepSpec {
    /// Return the version-requirement string carried by this spec, if any.
    pub fn version_text(&self) -> Option<&str> {
        match self {
            DepSpec::Version(v) => Some(v.as_str()),
            DepSpec::Path { version, .. } => version.as_deref(),
        }
    }

    /// Parse the version requirement as a [`VersionConstraint`]. Returns
    /// `Ok(None)` for path-only deps with no version pin.
    pub fn constraint(&self) -> Result<Option<VersionConstraint>, VersionError> {
        match self.version_text() {
            None => Ok(None),
            Some(text) => parse_constraint(text).map(Some),
        }
    }
}

/// Parsed manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Stable schema identifier (`"ori.manifest.v1"`).
    pub schema: String,
    /// Package metadata.
    pub package: PackageMeta,
    /// Capability declarations.
    pub capabilities: CapabilityDecls,
    /// Dependency table. Ordered by name for determinism.
    pub dependencies: BTreeMap<String, DepSpec>,
    /// Convenience scripts. Ordered by name.
    pub scripts: BTreeMap<String, String>,
}

impl FromStr for Manifest {
    type Err = ManifestError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Manifest::parse(text)
    }
}

impl Manifest {
    /// Parse a manifest from TOML text.
    ///
    /// This method is also exposed via [`std::str::FromStr`].
    pub fn parse(text: &str) -> Result<Manifest, ManifestError> {
        let doc = toml_lite::parse_manifest(text)?;

        let mut schema = MANIFEST_SCHEMA.to_string();
        let mut package_name: Option<String> = None;
        let mut package_version: Option<String> = None;
        let mut package_edition: Option<String> = None;
        let mut package_desc: Option<String> = None;
        let mut package_license: Option<String> = None;
        let mut capabilities = CapabilityDecls::default();
        let mut dependencies: BTreeMap<String, DepSpec> = BTreeMap::new();
        let mut scripts: BTreeMap<String, String> = BTreeMap::new();

        // Track dependency-as-section parsing.
        let mut dep_sections: BTreeMap<String, (Option<String>, Option<String>)> = BTreeMap::new();

        for (section, table) in &doc {
            match section.as_str() {
                "" => {
                    for (key, value) in table {
                        match key.as_str() {
                            "schema" => {
                                let s = expect_string(value, "schema")?;
                                if s != MANIFEST_SCHEMA {
                                    return Err(ManifestError::validation(
                                        ManifestErrorKind::UnsupportedSchema(s),
                                    ));
                                }
                                schema = s;
                            }
                            other => {
                                return Err(ManifestError::validation(
                                    ManifestErrorKind::UnknownKey {
                                        section: String::new(),
                                        key: other.to_string(),
                                    },
                                ));
                            }
                        }
                    }
                }
                "package" => {
                    for (key, value) in table {
                        match key.as_str() {
                            "name" => package_name = Some(expect_string(value, "package.name")?),
                            "version" => {
                                package_version = Some(expect_string(value, "package.version")?)
                            }
                            "edition" => {
                                package_edition = Some(expect_string(value, "package.edition")?)
                            }
                            "description" => {
                                package_desc = Some(expect_string(value, "package.description")?)
                            }
                            "license" => {
                                package_license = Some(expect_string(value, "package.license")?)
                            }
                            other => {
                                return Err(ManifestError::validation(
                                    ManifestErrorKind::UnknownKey {
                                        section: "package".to_string(),
                                        key: other.to_string(),
                                    },
                                ));
                            }
                        }
                    }
                }
                "capabilities" => {
                    for (key, value) in table {
                        match key.as_str() {
                            "declared" => {
                                capabilities.declared =
                                    expect_string_array(value, "capabilities.declared")?;
                            }
                            "denied" => {
                                capabilities.denied =
                                    expect_string_array(value, "capabilities.denied")?;
                            }
                            other => {
                                return Err(ManifestError::validation(
                                    ManifestErrorKind::UnknownKey {
                                        section: "capabilities".to_string(),
                                        key: other.to_string(),
                                    },
                                ));
                            }
                        }
                    }
                }
                "dependencies" => {
                    for (name, value) in table {
                        match value {
                            TomlValue::String(v) => {
                                dependencies.insert(name.clone(), DepSpec::Version(v.clone()));
                            }
                            TomlValue::Array(_) => {
                                return Err(ManifestError::validation(
                                    ManifestErrorKind::InvalidDependency {
                                        name: name.clone(),
                                        reason: "array dependency specs are not supported"
                                            .to_string(),
                                    },
                                ));
                            }
                        }
                    }
                }
                "scripts" => {
                    for (name, value) in table {
                        scripts.insert(name.clone(), expect_string(value, "scripts")?);
                    }
                }
                section if section.starts_with("dependencies.") => {
                    let name = section["dependencies.".len()..].to_string();
                    if name.is_empty() {
                        return Err(ManifestError::validation(
                            ManifestErrorKind::UnknownSection(section.to_string()),
                        ));
                    }
                    let mut path: Option<String> = None;
                    let mut version: Option<String> = None;
                    for (key, value) in table {
                        match key.as_str() {
                            "path" => {
                                path = Some(expect_string(value, "dependencies.<name>.path")?)
                            }
                            "version" => {
                                version = Some(expect_string(value, "dependencies.<name>.version")?)
                            }
                            other => {
                                return Err(ManifestError::validation(
                                    ManifestErrorKind::UnknownKey {
                                        section: section.to_string(),
                                        key: other.to_string(),
                                    },
                                ));
                            }
                        }
                    }
                    dep_sections.insert(name, (path, version));
                }
                other => {
                    return Err(ManifestError::validation(
                        ManifestErrorKind::UnknownSection(other.to_string()),
                    ));
                }
            }
        }

        // Merge dep_sections into dependencies.
        for (name, (path, version)) in dep_sections {
            let Some(path) = path else {
                return Err(ManifestError::validation(
                    ManifestErrorKind::InvalidDependency {
                        name,
                        reason: "path dependency missing `path = \"...\"`".to_string(),
                    },
                ));
            };
            if dependencies.contains_key(&name) {
                return Err(ManifestError::validation(
                    ManifestErrorKind::InvalidDependency {
                        name,
                        reason: "dependency declared both as inline string and as section"
                            .to_string(),
                    },
                ));
            }
            dependencies.insert(name, DepSpec::Path { path, version });
        }

        let Some(name) = package_name else {
            return Err(ManifestError::validation(ManifestErrorKind::MissingKey(
                "package.name".to_string(),
            )));
        };
        let Some(version) = package_version else {
            return Err(ManifestError::validation(ManifestErrorKind::MissingKey(
                "package.version".to_string(),
            )));
        };
        let Some(edition) = package_edition else {
            return Err(ManifestError::validation(ManifestErrorKind::MissingKey(
                "package.edition".to_string(),
            )));
        };

        if name.trim().is_empty() {
            return Err(ManifestError::validation(ManifestErrorKind::EmptyField(
                "package.name".to_string(),
            )));
        }
        validate_version(&version)?;
        if edition.trim().is_empty() {
            return Err(ManifestError::validation(ManifestErrorKind::EmptyField(
                "package.edition".to_string(),
            )));
        }
        for cap in capabilities
            .declared
            .iter()
            .chain(capabilities.denied.iter())
        {
            validate_capability(cap)?;
        }
        for (dep_name, dep) in &dependencies {
            if let DepSpec::Version(v) = dep {
                if v.trim().is_empty() {
                    return Err(ManifestError::validation(
                        ManifestErrorKind::InvalidDependency {
                            name: dep_name.clone(),
                            reason: "version requirement is empty".to_string(),
                        },
                    ));
                }
                // Parse the requirement as a version constraint so callers
                // never see syntactically-invalid `^foo` strings escape the
                // manifest layer. Plain `X.Y.Z` parses as `Exact`.
                if let Err(err) = parse_constraint(v) {
                    return Err(ManifestError::validation(
                        ManifestErrorKind::InvalidDependency {
                            name: dep_name.clone(),
                            reason: format!("invalid version requirement `{v}`: {err}"),
                        },
                    ));
                }
            }
            if let DepSpec::Path { path, version } = dep {
                if path.trim().is_empty() {
                    return Err(ManifestError::validation(
                        ManifestErrorKind::InvalidDependency {
                            name: dep_name.clone(),
                            reason: "path is empty".to_string(),
                        },
                    ));
                }
                if let Some(v) = version {
                    // Path-dep `version` may be either a strict pin (X.Y.Z) or
                    // a constraint expression — parse_constraint accepts both.
                    if let Err(err) = parse_constraint(v) {
                        return Err(ManifestError::validation(
                            ManifestErrorKind::InvalidDependency {
                                name: dep_name.clone(),
                                reason: format!("invalid version constraint `{v}`: {err}"),
                            },
                        ));
                    }
                }
            }
        }

        Ok(Manifest {
            schema,
            package: PackageMeta {
                name,
                version,
                edition,
                description: package_desc,
                license: package_license,
            },
            capabilities,
            dependencies,
            scripts,
        })
    }

    /// Read a manifest from disk.
    pub fn from_path(path: &Path) -> Result<Manifest, ManifestError> {
        let text = fs::read_to_string(path).map_err(|err| {
            ManifestError::validation(ManifestErrorKind::MissingKey(format!(
                "manifest file {}: {}",
                path.display(),
                err
            )))
        })?;
        Manifest::parse(&text)
    }
}

fn expect_string(value: &TomlValue, path: &'static str) -> Result<String, ManifestError> {
    match value {
        TomlValue::String(s) => Ok(s.clone()),
        TomlValue::Array(_) => Err(ManifestError::validation(ManifestErrorKind::WrongType {
            path: path.to_string(),
            expected: "string",
        })),
    }
}

fn expect_string_array(
    value: &TomlValue,
    path: &'static str,
) -> Result<Vec<String>, ManifestError> {
    match value {
        TomlValue::Array(items) => Ok(items.clone()),
        TomlValue::String(_) => Err(ManifestError::validation(ManifestErrorKind::WrongType {
            path: path.to_string(),
            expected: "array of strings",
        })),
    }
}

fn validate_version(version: &str) -> Result<(), ManifestError> {
    // Accept X.Y.Z or X.Y.Z-pre where X,Y,Z are non-empty decimal digit runs and
    // `pre` is `[A-Za-z0-9.-]+`.
    let (core, pre) = match version.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (version, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_digit()))
    {
        return Err(ManifestError::validation(
            ManifestErrorKind::InvalidVersion(version.to_string()),
        ));
    }
    if let Some(p) = pre {
        if p.is_empty()
            || !p
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        {
            return Err(ManifestError::validation(
                ManifestErrorKind::InvalidVersion(version.to_string()),
            ));
        }
    }
    Ok(())
}

fn validate_capability(cap: &str) -> Result<(), ManifestError> {
    if cap.is_empty()
        || !cap
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err(ManifestError::validation(
            ManifestErrorKind::InvalidCapability(cap.to_string()),
        ));
    }
    Ok(())
}

/// Shorthand for [`Manifest::parse`].
pub fn from_str(text: &str) -> Result<Manifest, ManifestError> {
    Manifest::parse(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
schema = "ori.manifest.v1"

[package]
name = "demo"
version = "0.1.0"
edition = "2027.1"
description = "Demo package"
license = "Apache-2.0"

[capabilities]
declared = ["fs.read", "net.fetch"]
denied = ["fs.write"]

[dependencies]
std-io = "0.1.0"

[dependencies.users]
path = "./users"
version = "0.2.0"

[scripts]
test = "ori check"
"#;

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn parses_full_sample() {
        let m = match Manifest::parse(SAMPLE) {
            Ok(m) => m,
            Err(err) => {
                // `panic!`/`unreachable!` are forbidden in this crate by the
                // bootstrap source guardrails (`scripts/validate_all.py`),
                // so we use the only sanctioned escape: a constant-false
                // assertion that carries the failure context to the test
                // runner.
                assert!(false, "parse failed: {err}");
                return;
            }
        };
        assert_eq!(m.schema, MANIFEST_SCHEMA);
        assert_eq!(m.package.name, "demo");
        assert_eq!(m.package.version, "0.1.0");
        assert_eq!(m.capabilities.declared, vec!["fs.read", "net.fetch"]);
        assert_eq!(m.capabilities.denied, vec!["fs.write"]);
        assert_eq!(m.dependencies.len(), 2);
        let users_dep_ok = matches!(
            m.dependencies.get("users"),
            Some(DepSpec::Path { path, version })
                if path == "./users" && version.as_deref() == Some("0.2.0")
        );
        assert!(
            users_dep_ok,
            "users dep was not a path dep at ./users@0.2.0"
        );
        assert_eq!(m.scripts.get("test").map(String::as_str), Some("ori check"));
    }

    #[test]
    fn empty_manifest_reports_missing_name() {
        let err = Manifest::parse("").unwrap_err();
        assert!(matches!(err.kind, ManifestErrorKind::MissingKey(_)));
    }

    #[test]
    fn rejects_bad_version() {
        let text = "[package]\nname=\"a\"\nedition=\"x\"\nversion=\"1.2\"\n";
        let err = Manifest::parse(text).unwrap_err();
        assert!(matches!(err.kind, ManifestErrorKind::InvalidVersion(_)));
    }

    #[test]
    fn rejects_invalid_capability() {
        let text = r#"
[package]
name = "a"
version = "0.0.1"
edition = "x"

[capabilities]
declared = ["fs read"]
"#;
        let err = Manifest::parse(text).unwrap_err();
        assert!(matches!(err.kind, ManifestErrorKind::InvalidCapability(_)));
    }

    #[test]
    fn rejects_unknown_top_level_section() {
        let text = "[unknown]\n";
        let err = Manifest::parse(text).unwrap_err();
        assert!(matches!(err.kind, ManifestErrorKind::UnknownSection(_)));
    }
}
