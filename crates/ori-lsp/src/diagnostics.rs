//! Translation between Orison compiler diagnostics and LSP diagnostics.
//!
//! The compiler reports positions as 1-based `(line, column)` pairs (see
//! `crates/ori-compiler/src/source.rs`). LSP positions are 0-based UTF-16
//! offsets. We do not yet model UTF-16 offsets — Orison source is ASCII for
//! the bootstrap parser — but we do translate the 1-based to 0-based shift
//! and saturate at zero so a span landing on the very first column does not
//! underflow.

use ori_compiler::diagnostic::{Diagnostic as OriDiagnostic, DiagnosticLevel, Fix};
use ori_compiler::source::Span;
use serde_json::json;

use crate::protocol::{Diagnostic, DiagnosticSeverity, Position, Range};

/// Source name attached to every emitted LSP diagnostic.
pub const DIAGNOSTIC_SOURCE: &str = "ori";

/// Converts a single compiler diagnostic to its LSP wire form.
pub fn to_lsp_diagnostic(diagnostic: &OriDiagnostic) -> Diagnostic {
    let severity = match diagnostic.level {
        DiagnosticLevel::Error => DiagnosticSeverity::Error,
        DiagnosticLevel::Warning => DiagnosticSeverity::Warning,
        DiagnosticLevel::Info => DiagnosticSeverity::Information,
    };
    Diagnostic {
        range: span_to_range(&diagnostic.span),
        severity,
        code: diagnostic.id.clone(),
        source: DIAGNOSTIC_SOURCE.to_string(),
        message: diagnostic.message.clone(),
        data: encode_fixes(&diagnostic.fixes),
    }
}

/// Converts a slice of compiler diagnostics.
pub fn to_lsp_diagnostics(diagnostics: &[OriDiagnostic]) -> Vec<Diagnostic> {
    diagnostics.iter().map(to_lsp_diagnostic).collect()
}

/// Translates a compiler [`Span`] (1-based, inclusive) to an LSP [`Range`]
/// (0-based, end-exclusive).
pub fn span_to_range(span: &Span) -> Range {
    Range {
        start: Position {
            line: zero_based(span.start.line),
            character: zero_based(span.start.column),
        },
        end: Position {
            line: zero_based(span.end.line),
            character: zero_based(span.end.column),
        },
    }
}

fn zero_based(one_based: usize) -> u32 {
    let adjusted = one_based.saturating_sub(1);
    u32::try_from(adjusted).unwrap_or(u32::MAX)
}

/// Encodes the compiler fix list into the LSP diagnostic `data` field. The
/// payload is intentionally a stable JSON shape so editors can match it
/// against `schemas/lsp-code-action.schema.json` once the schema lands.
fn encode_fixes(fixes: &[Fix]) -> Option<serde_json::Value> {
    if fixes.is_empty() {
        return None;
    }
    let encoded: Vec<serde_json::Value> = fixes
        .iter()
        .map(|fix| {
            json!({
                "kind": fix.kind,
                "description": fix.description,
                "confidence": fix.confidence,
                "patch": fix.patch.clone(),
            })
        })
        .collect();
    Some(json!({
        "schema": "ori.lsp.fixes.v1",
        "fixes": encoded,
    }))
}
