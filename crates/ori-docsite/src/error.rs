//! Error types for the docsite generator.

#![allow(missing_docs)]

use std::fmt;
use std::io;
use std::path::PathBuf;

/// Errors returned by [`crate::build_site`].
///
/// All variants are `non_exhaustive` over wire compatibility so we can add new
/// failure modes without breaking downstream consumers. The bootstrap CLI uses
/// `Display` to produce human-readable error reports.
#[derive(Debug)]
pub enum SiteError {
    /// The input directory does not exist or is not a directory.
    InputNotADirectory(PathBuf),
    /// Failed to walk a directory while collecting markdown sources.
    Walk { path: PathBuf, source: io::Error },
    /// Failed to read a markdown file.
    Read { path: PathBuf, source: io::Error },
    /// Failed to create the output directory or a parent thereof.
    CreateDir { path: PathBuf, source: io::Error },
    /// Failed to write a rendered HTML or CSS asset.
    Write { path: PathBuf, source: io::Error },
    /// A source path is outside the input directory (defensive check).
    PathEscape { path: PathBuf },
    /// A source path contained a non-UTF-8 component which we cannot render.
    NonUtf8Path { path: PathBuf },
}

impl fmt::Display for SiteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SiteError::InputNotADirectory(p) => {
                write!(f, "input is not a directory: {}", p.display())
            }
            SiteError::Walk { path, source } => {
                write!(f, "failed to scan directory {}: {}", path.display(), source)
            }
            SiteError::Read { path, source } => {
                write!(f, "failed to read {}: {}", path.display(), source)
            }
            SiteError::CreateDir { path, source } => {
                write!(
                    f,
                    "failed to create directory {}: {}",
                    path.display(),
                    source
                )
            }
            SiteError::Write { path, source } => {
                write!(f, "failed to write {}: {}", path.display(), source)
            }
            SiteError::PathEscape { path } => {
                write!(
                    f,
                    "source path escapes input directory: {}",
                    path.display()
                )
            }
            SiteError::NonUtf8Path { path } => {
                write!(f, "non-UTF-8 path component in {}", path.display())
            }
        }
    }
}

impl std::error::Error for SiteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SiteError::Walk { source, .. }
            | SiteError::Read { source, .. }
            | SiteError::CreateDir { source, .. }
            | SiteError::Write { source, .. } => Some(source),
            _ => None,
        }
    }
}
