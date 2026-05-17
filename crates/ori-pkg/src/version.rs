//! SemVer 2.0.0 parser and constraint matcher.
//!
//! Implements [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html)
//! for the package manager's version-aware resolver. The implementation is
//! intentionally allocation-light and validates every character against the
//! spec rules, including:
//!
//! * Numeric identifiers MUST NOT contain leading zeros (`01.2.3` is invalid).
//! * Pre-release identifiers are dot-separated ASCII alphanumerics plus `-`.
//!   Numeric pre-release identifiers MUST NOT have leading zeros.
//! * Build metadata is dot-separated `[0-9A-Za-z-]+` segments, ignored for
//!   precedence (a hard SemVer 2.0.0 rule).
//! * A version with no pre-release has higher precedence than the same core
//!   version with one (e.g. `2.0.0` > `2.0.0-alpha`).
//!
//! The matcher implements a small fragment of common range syntaxes:
//!
//! * `1.2.3` or `=1.2.3` — exact.
//! * `>=1.2.3`, `<1.2.3` — half-open.
//! * `>=1.2.3, <2.0.0` — explicit range.
//! * `^1.2.3` — Cargo-style caret: lock the left-most non-zero element.
//! * `~1.2.3` — tilde: lock through the minor.
//! * `*` or empty — any version.
//!
//! `parse_constraint` accepts whitespace around tokens and a single comma
//! between the two halves of an explicit range. Anything else is rejected
//! with a structured [`VersionError`].

use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};

/// A parsed SemVer 2.0.0 version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Version {
    /// Major number (`X` in `X.Y.Z`).
    pub major: u32,
    /// Minor number (`Y`).
    pub minor: u32,
    /// Patch number (`Z`).
    pub patch: u32,
    /// Optional pre-release suffix (`-alpha.1`), without the leading `-`.
    pub pre: Option<String>,
    /// Optional build metadata (`+build.7`), without the leading `+`. Build
    /// metadata is ignored for precedence per SemVer 2.0.0 §10.
    pub build: Option<String>,
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        // SemVer §10: build metadata MUST be ignored when determining version
        // precedence. Therefore equality is based on (major, minor, patch, pre)
        // only.
        self.major == other.major
            && self.minor == other.minor
            && self.patch == other.patch
            && self.pre == other.pre
    }
}

impl Eq for Version {}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.major.cmp(&other.major) {
            Ordering::Equal => {}
            ord => return ord,
        }
        match self.minor.cmp(&other.minor) {
            Ordering::Equal => {}
            ord => return ord,
        }
        match self.patch.cmp(&other.patch) {
            Ordering::Equal => {}
            ord => return ord,
        }
        match (self.pre.as_deref(), other.pre.as_deref()) {
            (None, None) => Ordering::Equal,
            // SemVer §11: a version without a pre-release outranks the same
            // core version with one.
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(a), Some(b)) => compare_prerelease(a, b),
        }
    }
}

impl std::hash::Hash for Version {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.major.hash(state);
        self.minor.hash(state);
        self.patch.hash(state);
        self.pre.hash(state);
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        if let Some(build) = &self.build {
            write!(f, "+{build}")?;
        }
        Ok(())
    }
}

/// Categorised parse error returned by [`parse_version`] and
/// [`parse_constraint`]. All variants carry enough context to render a useful
/// diagnostic without panicking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionError {
    /// Input string was empty.
    Empty,
    /// Core version did not have three dot-separated numeric parts.
    BadCore(String),
    /// A numeric identifier was empty, non-numeric, or had a leading zero.
    BadNumeric(String),
    /// A numeric identifier overflowed `u32`.
    NumericOverflow(String),
    /// A pre-release identifier was empty or contained disallowed characters.
    BadPrerelease(String),
    /// Build metadata segment was empty or contained disallowed characters.
    BadBuildMetadata(String),
    /// A constraint operator was unrecognised.
    BadOperator(String),
    /// An explicit range was malformed (e.g. only one half).
    BadRange(String),
}

impl fmt::Display for VersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VersionError::Empty => f.write_str("version string is empty"),
            VersionError::BadCore(s) => write!(f, "invalid core version `{s}` (expected X.Y.Z)"),
            VersionError::BadNumeric(s) => {
                write!(f, "invalid numeric identifier `{s}` (no leading zeros)")
            }
            VersionError::NumericOverflow(s) => write!(f, "numeric identifier `{s}` overflows u32"),
            VersionError::BadPrerelease(s) => write!(f, "invalid pre-release identifier `{s}`"),
            VersionError::BadBuildMetadata(s) => write!(f, "invalid build metadata segment `{s}`"),
            VersionError::BadOperator(s) => write!(f, "unknown constraint operator `{s}`"),
            VersionError::BadRange(s) => write!(f, "invalid version range `{s}`"),
        }
    }
}

impl std::error::Error for VersionError {}

/// A version constraint that a candidate [`Version`] can be tested against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionConstraint {
    /// Match exactly one version (build metadata ignored).
    Exact(Version),
    /// `>= v` (no upper bound).
    GreaterEq(Version),
    /// `< v`.
    LessThan(Version),
    /// Cargo-style `^1.2.3` (= `>=1.2.3, <2.0.0`; for `^0.x` see [`caret_upper_bound`]).
    Caret(Version),
    /// `~1.2.3` (= `>=1.2.3, <1.3.0`).
    Tilde(Version),
    /// Half-open `[low, high)`.
    Range {
        /// Inclusive lower bound.
        low: Version,
        /// Exclusive upper bound.
        high: Version,
    },
    /// `*` — accept any version.
    Any,
}

/// Parse a SemVer 2.0.0 string.
pub fn parse_version(s: &str) -> Result<Version, VersionError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(VersionError::Empty);
    }

    // Split off build metadata first because `-` may legitimately appear inside
    // build metadata (after a `+`) but not the other way around.
    let (core_and_pre, build) = match s.find('+') {
        Some(idx) => {
            let (left, right) = s.split_at(idx);
            (left, Some(&right[1..]))
        }
        None => (s, None),
    };

    let (core, pre) = match core_and_pre.find('-') {
        Some(idx) => {
            let (left, right) = core_and_pre.split_at(idx);
            (left, Some(&right[1..]))
        }
        None => (core_and_pre, None),
    };

    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3 {
        return Err(VersionError::BadCore(core.to_string()));
    }
    let major = parse_numeric(parts[0])?;
    let minor = parse_numeric(parts[1])?;
    let patch = parse_numeric(parts[2])?;

    let pre_owned = match pre {
        Some(p) => Some(validate_prerelease(p)?.to_string()),
        None => None,
    };
    let build_owned = match build {
        Some(b) => Some(validate_build(b)?.to_string()),
        None => None,
    };

    Ok(Version {
        major,
        minor,
        patch,
        pre: pre_owned,
        build: build_owned,
    })
}

fn parse_numeric(part: &str) -> Result<u32, VersionError> {
    if part.is_empty() {
        return Err(VersionError::BadNumeric(part.to_string()));
    }
    if !part.chars().all(|c| c.is_ascii_digit()) {
        return Err(VersionError::BadNumeric(part.to_string()));
    }
    if part.len() > 1 && part.starts_with('0') {
        return Err(VersionError::BadNumeric(part.to_string()));
    }
    part.parse::<u32>()
        .map_err(|_| VersionError::NumericOverflow(part.to_string()))
}

fn validate_prerelease(pre: &str) -> Result<&str, VersionError> {
    if pre.is_empty() {
        return Err(VersionError::BadPrerelease(pre.to_string()));
    }
    for ident in pre.split('.') {
        if ident.is_empty() {
            return Err(VersionError::BadPrerelease(pre.to_string()));
        }
        let is_numeric = ident.chars().all(|c| c.is_ascii_digit());
        if is_numeric {
            // Numeric pre-release identifiers must not have leading zeros
            // (SemVer §9).
            if ident.len() > 1 && ident.starts_with('0') {
                return Err(VersionError::BadPrerelease(ident.to_string()));
            }
        } else {
            for ch in ident.chars() {
                if !(ch.is_ascii_alphanumeric() || ch == '-') {
                    return Err(VersionError::BadPrerelease(ident.to_string()));
                }
            }
        }
    }
    Ok(pre)
}

fn validate_build(build: &str) -> Result<&str, VersionError> {
    if build.is_empty() {
        return Err(VersionError::BadBuildMetadata(build.to_string()));
    }
    for ident in build.split('.') {
        if ident.is_empty() {
            return Err(VersionError::BadBuildMetadata(build.to_string()));
        }
        for ch in ident.chars() {
            if !(ch.is_ascii_alphanumeric() || ch == '-') {
                return Err(VersionError::BadBuildMetadata(ident.to_string()));
            }
        }
    }
    Ok(build)
}

fn compare_prerelease(a: &str, b: &str) -> Ordering {
    // SemVer §11: identifiers are compared left-to-right. Numeric identifiers
    // are compared numerically; alphanumerics lexically; numeric < alphanumeric
    // for the same position; a longer set of identifiers outranks a shorter
    // set if all preceding ones are equal.
    let mut a_iter = a.split('.');
    let mut b_iter = b.split('.');
    loop {
        match (a_iter.next(), b_iter.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let xn = x.parse::<u64>().ok();
                let yn = y.parse::<u64>().ok();
                let ord = match (xn, yn) {
                    (Some(xv), Some(yv)) => xv.cmp(&yv),
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => x.cmp(y),
                };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

/// Parse a constraint expression.
pub fn parse_constraint(s: &str) -> Result<VersionConstraint, VersionError> {
    let s = s.trim();
    if s.is_empty() || s == "*" {
        return Ok(VersionConstraint::Any);
    }

    if let Some(rest) = s.strip_prefix("^") {
        let v = parse_version(rest.trim())?;
        return Ok(VersionConstraint::Caret(v));
    }
    if let Some(rest) = s.strip_prefix("~") {
        let v = parse_version(rest.trim())?;
        return Ok(VersionConstraint::Tilde(v));
    }
    if let Some(rest) = s.strip_prefix(">=") {
        // Could be ">=X, <Y" — peek for a comma.
        if let Some((lo, hi)) = split_range(rest) {
            let low = parse_version(lo.trim().trim_start_matches(">=").trim())?;
            let high = parse_upper(hi.trim())?;
            return Ok(VersionConstraint::Range { low, high });
        }
        let v = parse_version(rest.trim())?;
        return Ok(VersionConstraint::GreaterEq(v));
    }
    if let Some(rest) = s.strip_prefix("<") {
        let v = parse_version(rest.trim())?;
        return Ok(VersionConstraint::LessThan(v));
    }
    if let Some(rest) = s.strip_prefix("=") {
        let v = parse_version(rest.trim())?;
        return Ok(VersionConstraint::Exact(v));
    }
    if s.starts_with('>') {
        // Bare `>` (strict greater) is not in the supported set.
        return Err(VersionError::BadOperator(s.to_string()));
    }

    // Bare version string → exact match.
    let v = parse_version(s)?;
    Ok(VersionConstraint::Exact(v))
}

fn split_range(rest: &str) -> Option<(&str, &str)> {
    rest.find(',').map(|i| (&rest[..i], &rest[i + 1..]))
}

fn parse_upper(s: &str) -> Result<Version, VersionError> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("<") {
        parse_version(rest.trim())
    } else {
        Err(VersionError::BadRange(s.to_string()))
    }
}

/// Return the exclusive upper bound implied by `^v` per Cargo's rules:
/// the next version that increments the left-most non-zero component.
pub fn caret_upper_bound(v: &Version) -> Version {
    let (major, minor, patch) = if v.major > 0 {
        (v.major + 1, 0, 0)
    } else if v.minor > 0 {
        (0, v.minor + 1, 0)
    } else {
        (0, 0, v.patch + 1)
    };
    Version {
        major,
        minor,
        patch,
        pre: None,
        build: None,
    }
}

/// Return the exclusive upper bound implied by `~v` (bump the minor).
pub fn tilde_upper_bound(v: &Version) -> Version {
    Version {
        major: v.major,
        minor: v.minor + 1,
        patch: 0,
        pre: None,
        build: None,
    }
}

/// Test whether `version` satisfies `constraint`.
pub fn satisfies(version: &Version, constraint: &VersionConstraint) -> bool {
    match constraint {
        VersionConstraint::Any => true,
        VersionConstraint::Exact(v) => version == v,
        VersionConstraint::GreaterEq(v) => version >= v,
        VersionConstraint::LessThan(v) => version < v,
        VersionConstraint::Caret(v) => {
            let upper = caret_upper_bound(v);
            version >= v && version < &upper
        }
        VersionConstraint::Tilde(v) => {
            let upper = tilde_upper_bound(v);
            version >= v && version < &upper
        }
        VersionConstraint::Range { low, high } => version >= low && version < high,
    }
}

#[cfg(test)]
#[allow(clippy::assertions_on_constants)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        match parse_version(s) {
            Ok(v) => v,
            Err(err) => {
                assert!(false, "parse_version({s:?}) failed: {err}");
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

    #[test]
    fn parse_simple_core() {
        let p = v("1.2.3");
        assert_eq!(p.major, 1);
        assert_eq!(p.minor, 2);
        assert_eq!(p.patch, 3);
        assert!(p.pre.is_none());
        assert!(p.build.is_none());
    }

    #[test]
    fn parse_rejects_empty() {
        assert_eq!(parse_version(""), Err(VersionError::Empty));
        assert_eq!(parse_version("   "), Err(VersionError::Empty));
    }

    #[test]
    fn parse_rejects_two_part_version() {
        match parse_version("1.2") {
            Err(VersionError::BadCore(_)) => {}
            other => assert!(false, "expected BadCore, got {other:?}"),
        }
    }

    #[test]
    fn parse_rejects_leading_zero() {
        match parse_version("01.2.3") {
            Err(VersionError::BadNumeric(_)) => {}
            other => assert!(false, "expected BadNumeric, got {other:?}"),
        }
    }

    #[test]
    fn parse_zero_is_allowed() {
        let p = v("0.0.0");
        assert_eq!(p.major, 0);
    }

    #[test]
    fn parse_prerelease_simple() {
        let p = v("1.0.0-alpha");
        assert_eq!(p.pre.as_deref(), Some("alpha"));
    }

    #[test]
    fn parse_prerelease_dotted() {
        let p = v("1.0.0-alpha.1.beta");
        assert_eq!(p.pre.as_deref(), Some("alpha.1.beta"));
    }

    #[test]
    fn parse_prerelease_rejects_leading_zero_numeric_id() {
        match parse_version("1.0.0-alpha.01") {
            Err(VersionError::BadPrerelease(_)) => {}
            other => assert!(false, "expected BadPrerelease, got {other:?}"),
        }
    }

    #[test]
    fn parse_build_metadata() {
        let p = v("1.0.0+build.7");
        assert_eq!(p.build.as_deref(), Some("build.7"));
    }

    #[test]
    fn parse_pre_and_build() {
        let p = v("1.0.0-alpha.1+build.7-foo");
        assert_eq!(p.pre.as_deref(), Some("alpha.1"));
        assert_eq!(p.build.as_deref(), Some("build.7-foo"));
    }

    #[test]
    fn parse_rejects_empty_pre() {
        match parse_version("1.0.0-") {
            Err(VersionError::BadPrerelease(_)) => {}
            other => assert!(false, "expected BadPrerelease, got {other:?}"),
        }
    }

    #[test]
    fn parse_rejects_empty_build() {
        match parse_version("1.0.0+") {
            Err(VersionError::BadBuildMetadata(_)) => {}
            other => assert!(false, "expected BadBuildMetadata, got {other:?}"),
        }
    }

    #[test]
    fn build_metadata_is_ignored_for_equality() {
        assert_eq!(v("1.0.0+a"), v("1.0.0+b"));
    }

    #[test]
    fn ordering_core_versions() {
        assert!(v("1.0.0") < v("1.0.1"));
        assert!(v("1.0.1") < v("1.1.0"));
        assert!(v("1.1.0") < v("2.0.0"));
    }

    #[test]
    fn ordering_pre_release_outranked_by_release() {
        assert!(v("2.0.0-alpha") < v("2.0.0"));
    }

    #[test]
    fn ordering_pre_release_alpha_vs_alpha1() {
        assert!(v("2.0.0-alpha") < v("2.0.0-alpha.1"));
    }

    #[test]
    fn ordering_pre_release_alpha1_vs_beta() {
        assert!(v("2.0.0-alpha.1") < v("2.0.0-beta"));
    }

    #[test]
    fn ordering_numeric_vs_alpha_pre_release() {
        assert!(v("1.0.0-1") < v("1.0.0-alpha"));
    }

    #[test]
    fn ordering_chain_from_spec() {
        // 1.0.0 < 1.0.1 < 1.1.0 < 2.0.0-alpha < 2.0.0-alpha.1 < 2.0.0-beta < 2.0.0
        let chain = [
            "1.0.0",
            "1.0.1",
            "1.1.0",
            "2.0.0-alpha",
            "2.0.0-alpha.1",
            "2.0.0-beta",
            "2.0.0",
        ];
        for window in chain.windows(2) {
            let a = v(window[0]);
            let b = v(window[1]);
            assert!(a < b, "{a} should be < {b}");
        }
    }

    #[test]
    fn caret_constraint_above_one() {
        let c = match parse_constraint("^1.2.3") {
            Ok(c) => c,
            Err(err) => {
                assert!(false, "parse failed: {err}");
                return;
            }
        };
        assert!(satisfies(&v("1.2.3"), &c));
        assert!(satisfies(&v("1.9.0"), &c));
        assert!(!satisfies(&v("2.0.0"), &c));
        assert!(!satisfies(&v("1.2.2"), &c));
    }

    #[test]
    fn caret_constraint_below_one() {
        let c = match parse_constraint("^0.2.3") {
            Ok(c) => c,
            Err(err) => {
                assert!(false, "parse failed: {err}");
                return;
            }
        };
        assert!(satisfies(&v("0.2.3"), &c));
        assert!(satisfies(&v("0.2.9"), &c));
        assert!(!satisfies(&v("0.3.0"), &c));
    }

    #[test]
    fn caret_constraint_zero_zero_patch() {
        let c = match parse_constraint("^0.0.3") {
            Ok(c) => c,
            Err(err) => {
                assert!(false, "parse failed: {err}");
                return;
            }
        };
        assert!(satisfies(&v("0.0.3"), &c));
        assert!(!satisfies(&v("0.0.4"), &c));
    }

    #[test]
    fn tilde_constraint_locks_minor() {
        let c = match parse_constraint("~1.2.3") {
            Ok(c) => c,
            Err(err) => {
                assert!(false, "parse failed: {err}");
                return;
            }
        };
        assert!(satisfies(&v("1.2.3"), &c));
        assert!(satisfies(&v("1.2.9"), &c));
        assert!(!satisfies(&v("1.3.0"), &c));
        assert!(!satisfies(&v("1.2.2"), &c));
    }

    #[test]
    fn exact_constraint_bare_version() {
        let c = match parse_constraint("1.0.0") {
            Ok(c) => c,
            Err(err) => {
                assert!(false, "parse failed: {err}");
                return;
            }
        };
        assert!(satisfies(&v("1.0.0"), &c));
        assert!(!satisfies(&v("1.0.1"), &c));
    }

    #[test]
    fn explicit_range_overlap() {
        let c = match parse_constraint(">=1.2.0, <2.0.0") {
            Ok(c) => c,
            Err(err) => {
                assert!(false, "parse failed: {err}");
                return;
            }
        };
        assert!(satisfies(&v("1.2.0"), &c));
        assert!(satisfies(&v("1.9.9"), &c));
        assert!(!satisfies(&v("2.0.0"), &c));
        assert!(!satisfies(&v("1.1.9"), &c));
    }

    #[test]
    fn greater_eq_constraint() {
        let c = match parse_constraint(">=2.0.0") {
            Ok(c) => c,
            Err(err) => {
                assert!(false, "parse failed: {err}");
                return;
            }
        };
        assert!(satisfies(&v("2.0.0"), &c));
        assert!(satisfies(&v("99.0.0"), &c));
        assert!(!satisfies(&v("1.9.9"), &c));
    }

    #[test]
    fn less_than_constraint() {
        let c = match parse_constraint("<2.0.0") {
            Ok(c) => c,
            Err(err) => {
                assert!(false, "parse failed: {err}");
                return;
            }
        };
        assert!(satisfies(&v("1.99.9"), &c));
        assert!(!satisfies(&v("2.0.0"), &c));
    }

    #[test]
    fn any_constraint_accepts_everything() {
        let c = match parse_constraint("*") {
            Ok(c) => c,
            Err(err) => {
                assert!(false, "parse failed: {err}");
                return;
            }
        };
        assert!(satisfies(&v("0.0.1"), &c));
        assert!(satisfies(&v("99.99.99-alpha"), &c));
    }

    #[test]
    fn bad_operator_is_rejected() {
        match parse_constraint(">1.0.0") {
            Err(VersionError::BadOperator(_)) => {}
            other => assert!(false, "expected BadOperator, got {other:?}"),
        }
    }

    #[test]
    fn display_round_trips_simple() {
        let p = v("1.2.3");
        assert_eq!(format!("{p}"), "1.2.3");
        let p = v("1.0.0-alpha+build");
        assert_eq!(format!("{p}"), "1.0.0-alpha+build");
    }

    #[test]
    fn caret_upper_bound_rules() {
        assert_eq!(caret_upper_bound(&v("1.2.3")), v("2.0.0"));
        assert_eq!(caret_upper_bound(&v("0.2.3")), v("0.3.0"));
        assert_eq!(caret_upper_bound(&v("0.0.3")), v("0.0.4"));
    }

    #[test]
    fn tilde_upper_bound_rule() {
        assert_eq!(tilde_upper_bound(&v("1.2.3")), v("1.3.0"));
        assert_eq!(tilde_upper_bound(&v("0.2.3")), v("0.3.0"));
    }
}
