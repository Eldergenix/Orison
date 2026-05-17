//! Top-level error type for `ori-pkg`.

use std::fmt;
use std::io;

use crate::manifest::ManifestError;
use crate::resolver::ResolveError;

/// Aggregate error type for high-level package operations exposed through the
/// CLI. Individual modules expose more specific error types when callers need
/// to inspect failure causes programmatically.
#[derive(Debug)]
pub enum PkgError {
    /// The package manifest failed to parse or validate.
    Manifest(ManifestError),
    /// Resolving the dependency graph failed.
    Resolve(ResolveError),
    /// Reading a file from disk failed.
    Io(String, io::Error),
}

impl fmt::Display for PkgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PkgError::Manifest(err) => write!(f, "manifest error: {err}"),
            PkgError::Resolve(err) => write!(f, "resolve error: {err}"),
            PkgError::Io(path, err) => write!(f, "io error reading {path}: {err}"),
        }
    }
}

impl std::error::Error for PkgError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PkgError::Manifest(err) => Some(err),
            PkgError::Resolve(err) => Some(err),
            PkgError::Io(_, err) => Some(err),
        }
    }
}

impl From<ManifestError> for PkgError {
    fn from(value: ManifestError) -> Self {
        PkgError::Manifest(value)
    }
}

impl From<ResolveError> for PkgError {
    fn from(value: ResolveError) -> Self {
        PkgError::Resolve(value)
    }
}
