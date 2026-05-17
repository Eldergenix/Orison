//! `ori-pkg` is the Orison bootstrap package manager.
//!
//! This crate intentionally has no third-party dependencies beyond `serde` and
//! `serde_json`. See `MEMORY.md` decision D002. It exposes the manifest parser,
//! lockfile builder, dependency resolver, SBOM generator, audit runner, and
//! provenance verifier required by `ori-cli`.
//!
//! All public JSON output is produced through typed `serde` serialization to
//! satisfy `MEMORY.md` decision D011 (contract JSON must be typed). Map fields
//! that become serialized JSON use `BTreeMap` to guarantee deterministic
//! ordering across runs.
//!
//! The TOML subset accepted by [`toml_lite::parse_manifest`] is documented in
//! that module. It is deliberately small enough to be hand-implemented without
//! pulling in the `toml` crate.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod audit;
pub mod error;
pub mod lockfile;
pub mod manifest;
pub mod provenance;
pub mod registry;
pub mod resolver;
pub mod sandbox;
pub mod sbom;
pub mod toml_lite;
pub mod version;
pub mod version_solver;

pub use audit::{run_audit, AuditFinding, AuditReport, AuditSeverity, AuditSummary};
pub use error::PkgError;
pub use lockfile::{build_lockfile, LockedPackage, Lockfile};
pub use manifest::{
    CapabilityDecls, DepSpec, Manifest, ManifestError, ManifestErrorKind, PackageMeta,
};
pub use provenance::{verify_provenance, ProvenanceVerification};
pub use registry::{
    fnv1a_hex, LocalRegistry, PackageEntry, PublishReceipt, RegistryError, PUBLISH_RECEIPT_SCHEMA,
    REGISTRY_INDEX_SCHEMA, REGISTRY_LIST_SCHEMA,
};
pub use resolver::{resolve, ResolveError, ResolveErrorKind, ResolvedGraph, ResolvedNode};
pub use sandbox::{
    check_env_read, check_fs_read, check_fs_write, check_network, default_policy, path_allowed,
    run_in_sandbox, validate_policy, PolicyViolation, SandboxError, SandboxPolicy, SandboxResult,
    DEFAULT_TIMEOUT_SECONDS, SANDBOX_RESULT_SCHEMA,
};
pub use sbom::{build_sbom, Sbom, SbomComponent, SbomFormat};
pub use toml_lite::{parse_manifest, TomlError, TomlErrorKind, TomlValue};
pub use version::{
    caret_upper_bound, parse_constraint, parse_version, satisfies, tilde_upper_bound, Version,
    VersionConstraint, VersionError,
};
pub use version_solver::{solve, DependencyGraph, PackageId, SolverError};
