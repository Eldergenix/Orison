//! Top-level compiler facade.
//!
//! `Compiler` is the single public entrypoint orchestrating parse and
//! style checks for the bootstrap. Heavy-weight analysis passes (type
//! checker, borrow checker, effect propagation, ...) live in dedicated
//! modules and are run on demand by higher layers.

use crate::ast::Module;
use crate::capsule::module_capsule_json;
use crate::diagnostic::Diagnostic;
use crate::parser::parse_source;
use crate::source::{SourceFile, Span};
use std::io;

/// Compilation mode requested by the caller. Currently informational —
/// the bootstrap always runs the same passes regardless.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileMode {
    /// `ori check`: parse and validate but emit no artifacts.
    Check,
    /// `ori build --target dev`: debug-friendly artifacts.
    BuildDev,
    /// `ori build --target release`: optimised artifacts.
    BuildRelease,
}

/// Bundle of artefacts produced by [`Compiler::check_source`].
#[derive(Debug, Clone)]
pub struct CompileResult {
    /// Parsed module surface AST.
    pub module: Module,
    /// Diagnostics emitted by the parse and style passes.
    pub diagnostics: Vec<Diagnostic>,
}

/// Zero-sized type that exposes the bootstrap compiler's public API as
/// associated functions.
pub struct Compiler;

impl Compiler {
    /// Load `path` from disk and run the bootstrap check pipeline.
    pub fn check_file(path: &str) -> io::Result<CompileResult> {
        let source = SourceFile::from_path(path)?;
        Ok(Self::check_source(source))
    }

    /// Run the bootstrap check pipeline against an in-memory source file.
    pub fn check_source(source: SourceFile) -> CompileResult {
        let mut output = parse_source(&source);
        add_style_diagnostics(&source, &mut output.diagnostics);
        CompileResult {
            module: output.module,
            diagnostics: output.diagnostics,
        }
    }

    /// Apply the minimal whitespace canonicalisation used by `ori fmt`:
    /// CRLF to LF and trailing whitespace stripped from every line.
    pub fn format_source(text: &str) -> String {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let mut out = String::new();
        for line in normalized.lines() {
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }

    /// Render every diagnostic as one JSON object per line, joined by `\n`.
    /// Matches the format consumed by `ori check --json`.
    pub fn diagnostics_json_lines(result: &CompileResult) -> String {
        result
            .diagnostics
            .iter()
            .map(Diagnostic::to_json)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Produce the `ori.capsule.v1` JSON envelope for `result.module`.
    pub fn capsule_json(result: &CompileResult) -> String {
        module_capsule_json(&result.module)
    }
}

fn add_style_diagnostics(source: &SourceFile, diagnostics: &mut Vec<Diagnostic>) {
    for (line_idx, line) in source.text.lines().enumerate() {
        if let Some(column_idx) = line.find('\t') {
            diagnostics.push(
                Diagnostic::warning(
                    "W9001",
                    "tabs are discouraged; use spaces",
                    Span::new(
                        source.path.clone(),
                        line_idx + 1,
                        column_idx + 1,
                        line_idx + 1,
                        column_idx + 2,
                    ),
                )
                .with_agent_summary("Run `ori fmt` to normalize indentation.")
                .with_docs(vec!["doc:style.indentation".to_string()]),
            );
        }
    }
}
