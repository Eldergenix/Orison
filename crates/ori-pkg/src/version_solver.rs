//! PubGrub-inspired backtracking version solver.
//!
//! This is the bootstrap version solver used by `ori-pkg` once the resolver has
//! discovered the set of candidate versions for each dependency name. The
//! algorithm is a deliberately small, deterministic backtracking search rather
//! than a full PubGrub implementation — the bootstrap policy forbids any
//! cryptography or external dependencies, and the dependency graphs we have to
//! solve at this stage are tiny (low single digits of packages). The interface
//! is shaped so a PubGrub-style implementation can replace the body later
//! without breaking callers.
//!
//! ## Conflict-resolution semantics
//!
//! The resolver picks, for each dependency name, the **highest version that
//! satisfies every constraint imposed on that name by any package already in
//! the partial resolution**. When two candidate versions are equally good
//! (which is never in pure semver but can happen if the candidate list
//! contains duplicates) the lexicographically smaller `Version` wins so that
//! identical inputs always produce identical outputs.
//!
//! ## Termination
//!
//! The search visits each `(package, version)` at most once per backtrack
//! branch. We track the names currently on the activation stack to detect
//! dependency cycles deterministically; cycles return [`SolverError::Cycle`]
//! instead of looping. Impossible constraints — e.g. one dependency requires
//! `^1` while another requires `^2` — surface as [`SolverError::Conflict`]
//! after exhausting the candidate list.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::version::{satisfies, Version, VersionConstraint};

/// A `(name, version)` pair. Used for conflict reporting and as the key into
/// [`DependencyGraph::dependencies`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PackageId {
    /// Package name.
    pub name: String,
    /// Resolved version.
    pub version: Version,
}

impl fmt::Display for PackageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.name, self.version)
    }
}

/// All inputs needed by [`solve`].
#[derive(Debug, Clone, Default)]
pub struct DependencyGraph {
    /// For each package name, the candidate versions available in the
    /// registry. The solver does not assume any particular ordering; it
    /// re-sorts internally.
    pub packages: BTreeMap<String, Vec<Version>>,
    /// For each concrete `{name, version}`, the dependencies of that release.
    pub dependencies: BTreeMap<PackageId, Vec<(String, VersionConstraint)>>,
}

impl DependencyGraph {
    /// Construct an empty graph. Use the public fields to populate it.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Failure modes for [`solve`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SolverError {
    /// The named package has no candidate version that satisfies the union of
    /// all constraints. The string is the package name.
    NoMatchingVersion(String),
    /// A constraint conflict was detected and could not be resolved by
    /// backtracking. The list shows the package versions that participated in
    /// the conflict, in selection order.
    Conflict(Vec<PackageId>),
    /// A dependency cycle was detected. The vector lists the names of the
    /// cycle's members in traversal order.
    Cycle(Vec<String>),
}

impl fmt::Display for SolverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SolverError::NoMatchingVersion(name) => {
                write!(f, "no matching version found for `{name}`")
            }
            SolverError::Conflict(chain) => {
                let rendered: Vec<String> = chain.iter().map(ToString::to_string).collect();
                write!(f, "version conflict: {}", rendered.join(" -> "))
            }
            SolverError::Cycle(chain) => write!(f, "dependency cycle: {}", chain.join(" -> ")),
        }
    }
}

impl std::error::Error for SolverError {}

/// Solve the dependency graph rooted at `root`.
///
/// Returns a map of every transitively-required package name to the chosen
/// [`Version`]. The map is keyed by [`BTreeMap`] so ordering is deterministic.
/// The same `(graph, root)` always produces the same map.
pub fn solve(
    graph: &DependencyGraph,
    root: &str,
) -> Result<BTreeMap<String, Version>, SolverError> {
    if !graph.packages.contains_key(root) {
        return Err(SolverError::NoMatchingVersion(root.to_string()));
    }

    let mut resolved: BTreeMap<String, Version> = BTreeMap::new();
    let mut active_stack: Vec<String> = Vec::new();
    // Constraints stack — accumulated per package name as we descend. We
    // expose the current effective constraint to the recursion via `lookup`.
    let mut constraints: BTreeMap<String, Vec<VersionConstraint>> = BTreeMap::new();
    constraints
        .entry(root.to_string())
        .or_default()
        .push(VersionConstraint::Any);

    let mut conflict_chain: Vec<PackageId> = Vec::new();
    let ok = assign(
        graph,
        root,
        &mut resolved,
        &mut active_stack,
        &mut constraints,
        &mut conflict_chain,
    )?;
    if !ok {
        return Err(SolverError::Conflict(conflict_chain));
    }
    Ok(resolved)
}

/// Attempt to assign a version to `name`. Returns `Ok(true)` on success,
/// `Ok(false)` if every candidate was tried and none worked (the caller may
/// backtrack), and `Err(_)` for hard failures (cycle, missing package).
fn assign(
    graph: &DependencyGraph,
    name: &str,
    resolved: &mut BTreeMap<String, Version>,
    active_stack: &mut Vec<String>,
    constraints: &mut BTreeMap<String, Vec<VersionConstraint>>,
    conflict_chain: &mut Vec<PackageId>,
) -> Result<bool, SolverError> {
    // Cycle detection: a name reappearing on the active stack would loop.
    if active_stack.iter().any(|n| n == name) {
        let mut cycle = active_stack.clone();
        cycle.push(name.to_string());
        return Err(SolverError::Cycle(cycle));
    }

    let candidates = match graph.packages.get(name) {
        Some(v) => v.clone(),
        None => return Err(SolverError::NoMatchingVersion(name.to_string())),
    };
    if candidates.is_empty() {
        // Hard failure: the package itself has no candidates at all. Distinct
        // from "constraints exhausted the candidates" — that case is signalled
        // by `Ok(false)` below so callers may backtrack.
        return Err(SolverError::NoMatchingVersion(name.to_string()));
    }

    let effective = constraints.get(name).cloned().unwrap_or_default();

    // Try the highest version first; ties broken by lexicographic order of the
    // serialised version string (BTreeSet) so duplicates collapse and ordering
    // is deterministic.
    let mut ordered: Vec<Version> = candidates
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    ordered.sort();
    ordered.reverse();

    // Filter by effective constraints up front so impossible cases short-circuit.
    let mut viable: Vec<Version> = ordered
        .into_iter()
        .filter(|v| effective.iter().all(|c| satisfies(v, c)))
        .collect();

    if viable.is_empty() {
        // Constraints eliminated every candidate. We treat this as a soft
        // failure (`Ok(false)`) so the caller can backtrack and try a
        // different version for the package that introduced the constraint.
        // The outer `solve` converts a top-level `Ok(false)` into a
        // structured `Conflict` for the user.
        return Ok(false);
    }

    // If we already have a resolved version for this name, lock to it (acts as
    // a unifier across diamond deps).
    if let Some(existing) = resolved.get(name).cloned() {
        if !effective.iter().all(|c| satisfies(&existing, c)) {
            return Ok(false);
        }
        viable = vec![existing];
    }

    active_stack.push(name.to_string());
    for choice in viable {
        let pid = PackageId {
            name: name.to_string(),
            version: choice.clone(),
        };
        resolved.insert(name.to_string(), choice.clone());
        conflict_chain.push(pid.clone());

        // Apply this version's dependency constraints.
        let deps = graph.dependencies.get(&pid).cloned().unwrap_or_default();
        for (dep_name, _) in &deps {
            // Make sure the dep name exists in the candidate set; absence is a
            // hard error so users see misconfiguration immediately.
            if !graph.packages.contains_key(dep_name) {
                active_stack.pop();
                return Err(SolverError::NoMatchingVersion(dep_name.clone()));
            }
        }
        // Push constraints in deterministic order (BTreeMap iter is sorted).
        let mut pushed: Vec<String> = Vec::new();
        for (dep_name, constraint) in &deps {
            constraints
                .entry(dep_name.clone())
                .or_default()
                .push(constraint.clone());
            pushed.push(dep_name.clone());
        }

        let mut all_ok = true;
        for (dep_name, _) in &deps {
            match assign(
                graph,
                dep_name,
                resolved,
                active_stack,
                constraints,
                conflict_chain,
            )? {
                true => {}
                false => {
                    all_ok = false;
                    break;
                }
            }
        }

        // Undo constraint pushes.
        for dep_name in pushed.iter().rev() {
            if let Some(stack) = constraints.get_mut(dep_name) {
                let _ = stack.pop();
                if stack.is_empty() {
                    let _ = constraints.remove(dep_name);
                }
            }
        }

        if all_ok {
            active_stack.pop();
            return Ok(true);
        }

        // Backtrack: pop conflict_chain entry, unresolve, try next candidate.
        let _ = conflict_chain.pop();
        let _ = resolved.remove(name);
    }

    active_stack.pop();
    Ok(false)
}

#[cfg(test)]
#[allow(clippy::assertions_on_constants)]
mod tests {
    use super::*;
    use crate::version::{parse_constraint, parse_version};

    fn ver(s: &str) -> Version {
        match parse_version(s) {
            Ok(v) => v,
            Err(err) => {
                assert!(false, "bad version `{s}`: {err}");
                Version {
                    major: 0,
                    minor: 0,
                    patch: 0,
                    pre: None,
                    build: None,
                }
            }
        }
    }

    fn con(s: &str) -> VersionConstraint {
        match parse_constraint(s) {
            Ok(c) => c,
            Err(err) => {
                assert!(false, "bad constraint `{s}`: {err}");
                VersionConstraint::Any
            }
        }
    }

    fn assert_resolution(actual: &BTreeMap<String, Version>, expected: &[(&str, &str)]) {
        for (k, v) in expected {
            let got = match actual.get(*k) {
                Some(g) => g.clone(),
                None => {
                    assert!(false, "missing resolution for {k}");
                    return;
                }
            };
            assert_eq!(got, ver(v), "wrong version for {k}");
        }
        assert_eq!(actual.len(), expected.len(), "resolution size mismatch");
    }

    #[test]
    fn root_only_no_deps() {
        let mut g = DependencyGraph::new();
        g.packages.insert("a".into(), vec![ver("1.0.0")]);
        let r = match solve(&g, "a") {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "solve failed: {err}");
                return;
            }
        };
        assert_resolution(&r, &[("a", "1.0.0")]);
    }

    #[test]
    fn picks_highest_compatible_version() {
        let mut g = DependencyGraph::new();
        g.packages
            .insert("a".into(), vec![ver("1.0.0"), ver("1.1.0"), ver("1.2.0")]);
        let r = match solve(&g, "a") {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "solve failed: {err}");
                return;
            }
        };
        assert_resolution(&r, &[("a", "1.2.0")]);
    }

    #[test]
    fn root_with_single_dep() {
        let mut g = DependencyGraph::new();
        g.packages.insert("root".into(), vec![ver("1.0.0")]);
        g.packages.insert("b".into(), vec![ver("0.1.0")]);
        g.dependencies.insert(
            PackageId {
                name: "root".into(),
                version: ver("1.0.0"),
            },
            vec![("b".into(), con("^0.1.0"))],
        );
        let r = match solve(&g, "root") {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "solve failed: {err}");
                return;
            }
        };
        assert_resolution(&r, &[("root", "1.0.0"), ("b", "0.1.0")]);
    }

    #[test]
    fn transitive_dep_is_pulled_in() {
        let mut g = DependencyGraph::new();
        g.packages.insert("root".into(), vec![ver("1.0.0")]);
        g.packages.insert("a".into(), vec![ver("1.0.0")]);
        g.packages.insert("b".into(), vec![ver("2.0.0")]);
        g.dependencies.insert(
            PackageId {
                name: "root".into(),
                version: ver("1.0.0"),
            },
            vec![("a".into(), con("^1.0.0"))],
        );
        g.dependencies.insert(
            PackageId {
                name: "a".into(),
                version: ver("1.0.0"),
            },
            vec![("b".into(), con("^2.0.0"))],
        );
        let r = match solve(&g, "root") {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "solve failed: {err}");
                return;
            }
        };
        assert_resolution(&r, &[("root", "1.0.0"), ("a", "1.0.0"), ("b", "2.0.0")]);
    }

    #[test]
    fn conflict_between_two_incompatible_deps() {
        // root depends on a (which needs c@^1) and b (which needs c@^2).
        let mut g = DependencyGraph::new();
        g.packages.insert("root".into(), vec![ver("1.0.0")]);
        g.packages.insert("a".into(), vec![ver("1.0.0")]);
        g.packages.insert("b".into(), vec![ver("1.0.0")]);
        g.packages
            .insert("c".into(), vec![ver("1.0.0"), ver("2.0.0")]);
        g.dependencies.insert(
            PackageId {
                name: "root".into(),
                version: ver("1.0.0"),
            },
            vec![("a".into(), con("^1.0.0")), ("b".into(), con("^1.0.0"))],
        );
        g.dependencies.insert(
            PackageId {
                name: "a".into(),
                version: ver("1.0.0"),
            },
            vec![("c".into(), con("^1.0.0"))],
        );
        g.dependencies.insert(
            PackageId {
                name: "b".into(),
                version: ver("1.0.0"),
            },
            vec![("c".into(), con("^2.0.0"))],
        );
        match solve(&g, "root") {
            // Either signal is correct: with no alternative versions of `a`
            // and `b` left to try, the solver exhausts and reports a
            // `Conflict`. A future PubGrub-style implementation may instead
            // report `NoMatchingVersion(c)` once incompatibility tracking is
            // added; both shapes are documented as acceptable.
            Err(SolverError::Conflict(_)) => {}
            Err(SolverError::NoMatchingVersion(name)) => {
                assert_eq!(name, "c");
            }
            Err(other) => assert!(
                false,
                "expected Conflict or NoMatchingVersion(c), got {other}"
            ),
            Ok(r) => assert!(false, "expected conflict, got resolution {r:?}"),
        }
    }

    #[test]
    fn backtracks_when_first_choice_blocks_dep() {
        // root depends on a@*. a has two versions; only a@1.0.0 has a dep on
        // b@^1 (which exists); a@2.0.0 has a dep on b@^9 (which does not).
        // Highest-first selection picks a@2.0.0 first → fails → backtrack to
        // a@1.0.0 → success.
        let mut g = DependencyGraph::new();
        g.packages.insert("root".into(), vec![ver("1.0.0")]);
        g.packages
            .insert("a".into(), vec![ver("1.0.0"), ver("2.0.0")]);
        g.packages.insert("b".into(), vec![ver("1.0.0")]);
        g.dependencies.insert(
            PackageId {
                name: "root".into(),
                version: ver("1.0.0"),
            },
            vec![("a".into(), con("*"))],
        );
        g.dependencies.insert(
            PackageId {
                name: "a".into(),
                version: ver("1.0.0"),
            },
            vec![("b".into(), con("^1.0.0"))],
        );
        g.dependencies.insert(
            PackageId {
                name: "a".into(),
                version: ver("2.0.0"),
            },
            vec![("b".into(), con("^9.0.0"))],
        );
        let r = match solve(&g, "root") {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "expected backtrack to succeed: {err}");
                return;
            }
        };
        assert_resolution(&r, &[("root", "1.0.0"), ("a", "1.0.0"), ("b", "1.0.0")]);
    }

    #[test]
    fn diamond_dep_unifies() {
        // root → a, root → b; both depend on c@^1.0.0.
        let mut g = DependencyGraph::new();
        g.packages.insert("root".into(), vec![ver("1.0.0")]);
        g.packages.insert("a".into(), vec![ver("1.0.0")]);
        g.packages.insert("b".into(), vec![ver("1.0.0")]);
        g.packages
            .insert("c".into(), vec![ver("1.0.0"), ver("1.1.0")]);
        g.dependencies.insert(
            PackageId {
                name: "root".into(),
                version: ver("1.0.0"),
            },
            vec![("a".into(), con("^1.0.0")), ("b".into(), con("^1.0.0"))],
        );
        g.dependencies.insert(
            PackageId {
                name: "a".into(),
                version: ver("1.0.0"),
            },
            vec![("c".into(), con("^1.0.0"))],
        );
        g.dependencies.insert(
            PackageId {
                name: "b".into(),
                version: ver("1.0.0"),
            },
            vec![("c".into(), con("^1.0.0"))],
        );
        let r = match solve(&g, "root") {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "solve failed: {err}");
                return;
            }
        };
        // c should be the same version in both branches; highest is 1.1.0.
        assert_resolution(
            &r,
            &[
                ("root", "1.0.0"),
                ("a", "1.0.0"),
                ("b", "1.0.0"),
                ("c", "1.1.0"),
            ],
        );
    }

    #[test]
    fn cycle_is_detected() {
        let mut g = DependencyGraph::new();
        g.packages.insert("a".into(), vec![ver("1.0.0")]);
        g.packages.insert("b".into(), vec![ver("1.0.0")]);
        g.dependencies.insert(
            PackageId {
                name: "a".into(),
                version: ver("1.0.0"),
            },
            vec![("b".into(), con("*"))],
        );
        g.dependencies.insert(
            PackageId {
                name: "b".into(),
                version: ver("1.0.0"),
            },
            vec![("a".into(), con("*"))],
        );
        match solve(&g, "a") {
            Err(SolverError::Cycle(chain)) => {
                assert_eq!(
                    chain,
                    vec!["a".to_string(), "b".to_string(), "a".to_string()]
                );
            }
            other => assert!(false, "expected Cycle, got {other:?}"),
        }
    }

    #[test]
    fn missing_root_returns_no_matching_version() {
        let g = DependencyGraph::new();
        match solve(&g, "ghost") {
            Err(SolverError::NoMatchingVersion(name)) => assert_eq!(name, "ghost"),
            other => assert!(false, "expected NoMatchingVersion, got {other:?}"),
        }
    }

    #[test]
    fn missing_transitive_dep_returns_no_matching_version() {
        let mut g = DependencyGraph::new();
        g.packages.insert("a".into(), vec![ver("1.0.0")]);
        g.dependencies.insert(
            PackageId {
                name: "a".into(),
                version: ver("1.0.0"),
            },
            vec![("missing".into(), con("*"))],
        );
        match solve(&g, "a") {
            Err(SolverError::NoMatchingVersion(name)) => assert_eq!(name, "missing"),
            other => assert!(false, "expected NoMatchingVersion, got {other:?}"),
        }
    }

    #[test]
    fn deterministic_for_repeated_runs() {
        let mut g = DependencyGraph::new();
        g.packages.insert("root".into(), vec![ver("1.0.0")]);
        g.packages.insert(
            "a".into(),
            vec![ver("1.0.0"), ver("1.1.0"), ver("1.2.0"), ver("2.0.0")],
        );
        g.dependencies.insert(
            PackageId {
                name: "root".into(),
                version: ver("1.0.0"),
            },
            vec![("a".into(), con("^1.0.0"))],
        );
        let mut last: Option<BTreeMap<String, Version>> = None;
        for _ in 0..5 {
            let r = match solve(&g, "root") {
                Ok(r) => r,
                Err(err) => {
                    assert!(false, "solve failed: {err}");
                    return;
                }
            };
            if let Some(prev) = &last {
                assert_eq!(prev, &r, "non-deterministic resolution");
            }
            last = Some(r);
        }
    }

    #[test]
    fn empty_candidate_list_is_no_matching_version() {
        let mut g = DependencyGraph::new();
        g.packages.insert("a".into(), Vec::new());
        match solve(&g, "a") {
            Err(SolverError::NoMatchingVersion(name)) => assert_eq!(name, "a"),
            other => assert!(false, "expected NoMatchingVersion, got {other:?}"),
        }
    }

    #[test]
    fn constraint_eliminates_all_candidates() {
        let mut g = DependencyGraph::new();
        g.packages.insert("root".into(), vec![ver("1.0.0")]);
        g.packages
            .insert("a".into(), vec![ver("0.1.0"), ver("0.2.0")]);
        g.dependencies.insert(
            PackageId {
                name: "root".into(),
                version: ver("1.0.0"),
            },
            vec![("a".into(), con("^1.0.0"))],
        );
        match solve(&g, "root") {
            // Constraint exhaustion at the leaf surfaces as a `Conflict` once
            // no parent version is left to backtrack to. `NoMatchingVersion`
            // is reserved for "the package does not exist in the registry".
            Err(SolverError::Conflict(_)) => {}
            Err(SolverError::NoMatchingVersion(name)) => assert_eq!(name, "a"),
            other => assert!(
                false,
                "expected Conflict or NoMatchingVersion, got {other:?}"
            ),
        }
    }

    #[test]
    fn ties_break_by_version_ordering() {
        // If two candidates compare equal (e.g. build metadata differs) the
        // BTreeSet collapses them so the result is deterministic.
        let mut g = DependencyGraph::new();
        g.packages.insert(
            "a".into(),
            vec![ver("1.0.0+a"), ver("1.0.0+b"), ver("1.0.0+a")],
        );
        let r = match solve(&g, "a") {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "solve failed: {err}");
                return;
            }
        };
        let got = match r.get("a") {
            Some(v) => v.clone(),
            None => {
                assert!(false, "missing a");
                return;
            }
        };
        assert_eq!(got.major, 1);
        assert_eq!(got.minor, 0);
        assert_eq!(got.patch, 0);
    }

    #[test]
    fn package_id_display() {
        let p = PackageId {
            name: "a".into(),
            version: ver("1.2.3"),
        };
        assert_eq!(format!("{p}"), "a@1.2.3");
    }

    #[test]
    fn solver_error_display() {
        let e = SolverError::NoMatchingVersion("x".into());
        assert!(format!("{e}").contains("x"));
        let e = SolverError::Cycle(vec!["a".into(), "b".into(), "a".into()]);
        assert!(format!("{e}").contains("a -> b -> a"));
    }
}
