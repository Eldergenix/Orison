//! Dependency resolver.
//!
//! For the bootstrap, only local-path dependencies are followed. Registry-
//! style dependencies (plain version strings) are recorded as
//! `unresolved-bootstrap` placeholders so audit and SBOM can still surface
//! them without failing the build.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use crate::manifest::{DepSpec, Manifest, ManifestError};
use crate::version::{parse_constraint, parse_version, satisfies};
use crate::version::{Version, VersionConstraint};
use crate::version_solver::{solve, DependencyGraph, PackageId, SolverError};

/// One node in the resolved graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedNode {
    /// Logical package name (stable identifier).
    pub name: String,
    /// Resolved version. Empty string when the dependency could not be
    /// resolved in bootstrap mode.
    pub version: String,
    /// Resolved source descriptor: `"path+<abs-or-rel>"` or
    /// `"unresolved-bootstrap"`.
    pub source: String,
    /// Capability names this node declares.
    pub capabilities: Vec<String>,
    /// Direct dependency names (already deduplicated and sorted).
    pub dependencies: Vec<String>,
}

/// Output of [`resolve`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedGraph {
    /// Name of the root package.
    pub root: String,
    /// All nodes keyed by name. Sorted for determinism.
    pub nodes: BTreeMap<String, ResolvedNode>,
}

/// Resolve error category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveErrorKind {
    /// Cyclic dependency detected. The vector lists the cycle members in
    /// traversal order.
    Cycle(Vec<String>),
    /// A path dependency pointed at a missing or unreadable manifest.
    PathManifest {
        /// The dependency name in the manifest.
        name: String,
        /// The path that could not be loaded.
        path: String,
        /// Underlying manifest error message.
        message: String,
    },
    /// A path dep version pin disagreed with the dependency's own manifest.
    VersionMismatch {
        /// Dependency name.
        name: String,
        /// Version pinned by the consumer.
        expected: String,
        /// Version reported by the dependency manifest.
        actual: String,
    },
}

impl fmt::Display for ResolveErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveErrorKind::Cycle(path) => write!(f, "dependency cycle: {}", path.join(" -> ")),
            ResolveErrorKind::PathManifest { name, path, message } => {
                write!(f, "path dependency `{name}` ({path}): {message}")
            }
            ResolveErrorKind::VersionMismatch { name, expected, actual } => write!(
                f,
                "dependency `{name}` version mismatch: manifest pin `{expected}` vs path manifest `{actual}`"
            ),
        }
    }
}

/// Resolve error wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveError {
    /// The error category.
    pub kind: ResolveErrorKind,
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)
    }
}

impl std::error::Error for ResolveError {}

impl From<ManifestError> for ResolveError {
    fn from(value: ManifestError) -> Self {
        ResolveError {
            kind: ResolveErrorKind::PathManifest {
                name: String::new(),
                path: String::new(),
                message: value.to_string(),
            },
        }
    }
}

/// Resolve `manifest`'s dependency graph rooted at `root` (the directory
/// containing the manifest). Path dependencies are followed recursively;
/// registry dependencies become `unresolved-bootstrap` leaves.
pub fn resolve(manifest: &Manifest, root: &Path) -> Result<ResolvedGraph, ResolveError> {
    let mut nodes: BTreeMap<String, ResolvedNode> = BTreeMap::new();
    let mut stack: Vec<String> = Vec::new();
    let mut visited: BTreeSet<String> = BTreeSet::new();

    let root_name = manifest.package.name.clone();
    walk(
        manifest,
        root,
        &root_name,
        &mut nodes,
        &mut stack,
        &mut visited,
    )?;
    Ok(ResolvedGraph {
        root: root_name,
        nodes,
    })
}

fn walk(
    manifest: &Manifest,
    base_dir: &Path,
    name: &str,
    nodes: &mut BTreeMap<String, ResolvedNode>,
    stack: &mut Vec<String>,
    visited: &mut BTreeSet<String>,
) -> Result<(), ResolveError> {
    if stack.iter().any(|n| n == name) {
        let mut cycle = stack.clone();
        cycle.push(name.to_string());
        return Err(ResolveError {
            kind: ResolveErrorKind::Cycle(cycle),
        });
    }
    if visited.contains(name) {
        return Ok(());
    }
    stack.push(name.to_string());

    let mut direct_deps: Vec<String> = Vec::new();

    for (dep_name, dep_spec) in &manifest.dependencies {
        direct_deps.push(dep_name.clone());
        match dep_spec {
            DepSpec::Version(_) => {
                // Bootstrap: record an unresolved leaf and continue.
                nodes
                    .entry(dep_name.clone())
                    .or_insert_with(|| ResolvedNode {
                        name: dep_name.clone(),
                        version: String::new(),
                        source: "unresolved-bootstrap".to_string(),
                        capabilities: Vec::new(),
                        dependencies: Vec::new(),
                    });
            }
            DepSpec::Path { path, version } => {
                let dep_dir = base_dir.join(path);
                let dep_manifest_path = dep_dir.join("ori.toml");
                let dep_manifest = match Manifest::from_path(&dep_manifest_path) {
                    Ok(m) => m,
                    Err(err) => {
                        stack.pop();
                        return Err(ResolveError {
                            kind: ResolveErrorKind::PathManifest {
                                name: dep_name.clone(),
                                path: dep_manifest_path.display().to_string(),
                                message: err.to_string(),
                            },
                        });
                    }
                };
                if let Some(pin) = version {
                    // Compare against the dep manifest version using the
                    // new version-constraint matcher. Falls back to a string
                    // compare if either side fails to parse — that keeps the
                    // pre-version-solver tests green for non-semver strings.
                    let constraint_ok = match (
                        parse_constraint(pin),
                        parse_version(&dep_manifest.package.version),
                    ) {
                        (Ok(c), Ok(v)) => satisfies(&v, &c),
                        _ => pin == &dep_manifest.package.version,
                    };
                    if !constraint_ok {
                        stack.pop();
                        return Err(ResolveError {
                            kind: ResolveErrorKind::VersionMismatch {
                                name: dep_name.clone(),
                                expected: pin.clone(),
                                actual: dep_manifest.package.version.clone(),
                            },
                        });
                    }
                }
                // Record the dep first so cycles cycle properly.
                let resolved_source = format!("path+{}", dep_dir.display());
                let dep_capabilities = dep_manifest.capabilities.declared.clone();
                let dep_inner_names: Vec<String> =
                    dep_manifest.dependencies.keys().cloned().collect();
                let node = ResolvedNode {
                    name: dep_manifest.package.name.clone(),
                    version: dep_manifest.package.version.clone(),
                    source: resolved_source,
                    capabilities: dep_capabilities,
                    dependencies: dep_inner_names,
                };
                // Use the dependency manifest's declared name as the graph key
                // so siblings agree on identity (manifest `[dependencies]`
                // alias is recorded as a graph edge).
                let graph_name = node.name.clone();
                nodes.insert(graph_name.clone(), node);
                walk(&dep_manifest, &dep_dir, &graph_name, nodes, stack, visited)?;
            }
        }
    }

    direct_deps.sort();
    direct_deps.dedup();

    // Insert/update the current node.
    let entry = nodes
        .entry(name.to_string())
        .or_insert_with(|| ResolvedNode {
            name: name.to_string(),
            version: manifest.package.version.clone(),
            source: format!("path+{}", base_dir.display()),
            capabilities: manifest.capabilities.declared.clone(),
            dependencies: Vec::new(),
        });
    entry.dependencies = direct_deps;
    entry.capabilities = manifest.capabilities.declared.clone();
    entry.version = manifest.package.version.clone();

    visited.insert(name.to_string());
    stack.pop();
    Ok(())
}

/// Registry-aware resolve. Each entry of `candidates` lists the available
/// versions for that dependency name; the version solver picks the highest
/// version that satisfies every constraint. This is the planned migration
/// path away from the path-only resolver above: once the registry protocol
/// is implemented the CLI will populate `candidates` from
/// `/api/v1/packages/{name}/versions` and call this entry point instead of
/// [`resolve`].
///
/// The path-only [`resolve`] above remains the special case where each dep
/// has exactly one candidate — its on-disk version — so existing callers do
/// not need to change.
pub fn resolve_with_registry(
    manifest: &Manifest,
    candidates: &BTreeMap<String, Vec<Version>>,
) -> Result<BTreeMap<String, Version>, ResolveError> {
    let root_name = manifest.package.name.clone();
    let root_version = match parse_version(&manifest.package.version) {
        Ok(v) => v,
        Err(err) => {
            return Err(ResolveError {
                kind: ResolveErrorKind::PathManifest {
                    name: root_name,
                    path: String::new(),
                    message: format!("root version is not valid semver: {err}"),
                },
            });
        }
    };

    let mut graph = DependencyGraph::new();
    // Seed the root.
    graph
        .packages
        .insert(root_name.clone(), vec![root_version.clone()]);
    let mut root_deps: Vec<(String, VersionConstraint)> = Vec::new();
    for (name, dep_spec) in &manifest.dependencies {
        let constraint = match dep_spec.constraint() {
            Ok(Some(c)) => c,
            Ok(None) => VersionConstraint::Any,
            Err(err) => {
                return Err(ResolveError {
                    kind: ResolveErrorKind::PathManifest {
                        name: name.clone(),
                        path: String::new(),
                        message: format!("invalid version constraint: {err}"),
                    },
                });
            }
        };
        root_deps.push((name.clone(), constraint));
    }
    graph.dependencies.insert(
        PackageId {
            name: root_name.clone(),
            version: root_version,
        },
        root_deps,
    );
    // Copy the candidate set in so the solver sees them.
    for (name, versions) in candidates {
        graph
            .packages
            .entry(name.clone())
            .or_insert_with(|| versions.clone());
    }
    match solve(&graph, &root_name) {
        Ok(r) => Ok(r),
        Err(SolverError::Cycle(chain)) => Err(ResolveError {
            kind: ResolveErrorKind::Cycle(chain),
        }),
        Err(SolverError::NoMatchingVersion(name)) => Err(ResolveError {
            kind: ResolveErrorKind::PathManifest {
                name,
                path: String::new(),
                message: "no matching version".to_string(),
            },
        }),
        Err(SolverError::Conflict(chain)) => {
            let rendered: Vec<String> = chain.iter().map(ToString::to_string).collect();
            Err(ResolveError {
                kind: ResolveErrorKind::PathManifest {
                    name: rendered.join(" -> "),
                    path: String::new(),
                    message: "version conflict".to_string(),
                },
            })
        }
    }
}

#[cfg(test)]
#[allow(clippy::assertions_on_constants)]
mod registry_tests {
    use super::*;
    use crate::manifest::Manifest;

    #[test]
    fn resolve_with_registry_single_dep() {
        let text = r#"
schema = "ori.manifest.v1"

[package]
name = "root"
version = "1.0.0"
edition = "2027.1"

[dependencies]
a = "^1.0.0"
"#;
        let manifest = match Manifest::parse(text) {
            Ok(m) => m,
            Err(err) => {
                assert!(false, "parse failed: {err}");
                return;
            }
        };
        let mut candidates: BTreeMap<String, Vec<Version>> = BTreeMap::new();
        candidates.insert(
            "a".to_string(),
            vec![
                match parse_version("1.0.0") {
                    Ok(v) => v,
                    Err(_) => {
                        assert!(false, "parse");
                        return;
                    }
                },
                match parse_version("1.5.0") {
                    Ok(v) => v,
                    Err(_) => {
                        assert!(false, "parse");
                        return;
                    }
                },
                match parse_version("2.0.0") {
                    Ok(v) => v,
                    Err(_) => {
                        assert!(false, "parse");
                        return;
                    }
                },
            ],
        );
        let r = match resolve_with_registry(&manifest, &candidates) {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "resolve_with_registry failed: {err}");
                return;
            }
        };
        let got = match r.get("a") {
            Some(v) => v.clone(),
            None => {
                assert!(false, "no a");
                return;
            }
        };
        assert_eq!(got.major, 1);
        assert_eq!(got.minor, 5);
    }
}
