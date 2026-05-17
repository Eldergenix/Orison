//! Source files, positions and spans.
//!
//! Lines and columns are always **1-based** throughout the compiler. The LSP
//! adapter in `ori-lsp` converts to the 0-based form mandated by the LSP
//! specification at the protocol boundary; consumers inside this crate must
//! not introduce their own zero-base conversions.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::Path;

/// 1-based source coordinate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    /// 1-based line number.
    pub line: usize,
    /// 1-based column number (in source bytes, since the bootstrap lexer
    /// only accepts ASCII).
    pub column: usize,
}

impl Position {
    /// Construct a position from explicit 1-based coordinates.
    pub fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

/// Inclusive range over a single file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    /// File path the span refers to.
    pub file: String,
    /// Inclusive start coordinate.
    pub start: Position,
    /// Inclusive end coordinate (`end >= start`).
    pub end: Position,
}

impl Span {
    /// Construct a span from explicit start/end 1-based coordinates.
    pub fn new(
        file: impl Into<String>,
        start_line: usize,
        start_column: usize,
        end_line: usize,
        end_column: usize,
    ) -> Self {
        Self {
            file: file.into(),
            start: Position::new(start_line, start_column),
            end: Position::new(end_line, end_column),
        }
    }

    /// Construct a zero-width span at `(line, column)`.
    pub fn point(file: impl Into<String>, line: usize, column: usize) -> Self {
        Self::new(file, line, column, line, column)
    }

    /// Construct a placeholder span at `(1, 1)`. Used by synthetic symbols.
    pub fn dummy(file: impl Into<String>) -> Self {
        Self::new(file, 1, 1, 1, 1)
    }
}

/// In-memory representation of a single Orison source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// On-disk path or pseudo-path used in diagnostics.
    pub path: String,
    /// Source text (UTF-8, ASCII-only enforced by the lexer).
    pub text: String,
}

impl SourceFile {
    /// Construct a source file from a path and pre-loaded text.
    pub fn new(path: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            text: text.into(),
        }
    }

    /// Read the given filesystem path into a [`SourceFile`].
    pub fn from_path(path: impl AsRef<Path>) -> io::Result<Self> {
        let path_ref = path.as_ref();
        let text = fs::read_to_string(path_ref)?;
        Ok(Self::new(path_ref.to_string_lossy().to_string(), text))
    }
}
