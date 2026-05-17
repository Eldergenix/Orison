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
                    if pin != &dep_manifest.package.version {
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
