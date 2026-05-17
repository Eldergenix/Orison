//! Diagnostic data model.
//!
//! The [`Diagnostic`] shape is the load-bearing contract between the
//! compiler, IDE, and agent. Every analysis pass emits these structs and the
//! JSON envelope (`ori.diagnostic.v1`) is verified against
//! `schemas/diagnostic.schema.json`. Mutating the public fields is a schema
//! break; add new optional fields instead.

use crate::json::to_json;
use crate::source::Span;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stable schema identifier for [`Diagnostic`].
pub const DIAGNOSTIC_SCHEMA: &str = "ori.diagnostic.v1";

/// Severity discriminator for a [`Diagnostic`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticLevel {
    /// Compilation cannot succeed; CLI exit code reflects this.
    Error,
    /// Soft failure; surfaced but does not fail the build.
    Warning,
    /// Informational guidance, never gates CI.
    Info,
}

impl DiagnosticLevel {
    /// Return the canonical lowercase string used in JSON envelopes.
    pub fn as_str(&self) -> &'static str {
        match self {
            DiagnosticLevel::Error => "error",
            DiagnosticLevel::Warning => "warning",
            DiagnosticLevel::Info => "info",
        }
    }
}

/// Suggested fix attached to a [`Diagnostic`]. The optional patch payload is
/// a free-form JSON envelope so machine-applied repairs can carry whatever
/// fields the relevant patch family expects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fix {
    /// Short category (`replace`, `add_import`, ...).
    pub kind: String,
    /// Human-readable explanation.
    pub description: String,
    /// Confidence in `[0.0, 1.0]`. Constructor clamps to range.
    pub confidence: f32,
    /// Optional structured patch payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch: Option<Value>,
}

impl Fix {
    /// Build a fix with the given description and confidence. Confidence is
    /// clamped into `[0.0, 1.0]` so out-of-range inputs cannot pollute JSON.
    pub fn new(kind: impl Into<String>, description: impl Into<String>, confidence: f32) -> Self {
        Self {
            kind: kind.into(),
            description: description.into(),
            confidence: confidence.clamp(0.0, 1.0),
            patch: None,
        }
    }

    /// Attach a parsed patch JSON value.
    pub fn with_patch(mut self, patch: Value) -> Self {
        self.patch = Some(patch);
        self
    }

    /// Attach a patch supplied as raw JSON text. Invalid JSON is preserved
    /// verbatim as a string so the diagnostic still serialises cleanly.
    pub fn with_patch_json(mut self, patch_json: impl Into<String>) -> Self {
        let text = patch_json.into();
        let value = match serde_json::from_str::<Value>(&text) {
            Ok(value) => value,
            Err(_) => Value::String(text),
        };
        self.patch = Some(value);
        self
    }

    /// Render the fix as a canonical JSON string.
    pub fn to_json(&self) -> String {
        to_json(self)
    }
}

/// Optional reference to the symbol a diagnostic is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticSymbol {
    /// Stable symbol id (`sym:module.name`).
    pub id: String,
}

/// Agent-facing guidance bundled with each diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentHint {
    /// One-line natural-language summary.
    pub summary: String,
    /// Symbol ids the agent should hydrate before attempting a fix.
    pub minimal_context: Vec<String>,
    /// Documentation links/anchors relevant to the diagnostic.
    pub docs: Vec<String>,
}

impl AgentHint {
    /// Build an empty hint with all collection fields initialised.
    pub fn empty() -> Self {
        Self {
            summary: String::new(),
            minimal_context: Vec::new(),
            docs: Vec::new(),
        }
    }
}

/// A single diagnostic surfaced by any compiler pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Stable schema identifier (`"ori.diagnostic.v1"`).
    pub schema: String,
    /// Diagnostic id (`E0042`, `W3010`, ...).
    pub id: String,
    /// Severity.
    pub level: DiagnosticLevel,
    /// Human-readable message; should follow the
    /// `expected X, got Y` convention where applicable.
    pub message: String,
    /// Source span the diagnostic refers to.
    pub span: Span,
    /// Optional symbol id the diagnostic is about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<DiagnosticSymbol>,
    /// Tokens the analysis pass expected.
    pub expected: Vec<String>,
    /// Tokens the analysis pass actually found.
    pub found: Vec<String>,
    /// Suggested machine-applicable fixes.
    pub fixes: Vec<Fix>,
    /// Agent-specific guidance.
    pub agent: AgentHint,
}

impl Diagnostic {
    /// Construct a diagnostic with arbitrary level. Most callers should use
    /// [`Self::error`], [`Self::warning`], or [`Self::info`].
    pub fn new(
        id: impl Into<String>,
        level: DiagnosticLevel,
        message: impl Into<String>,
        span: Span,
    ) -> Self {
        Self {
            schema: DIAGNOSTIC_SCHEMA.to_string(),
            id: id.into(),
            level,
            message: message.into(),
            span,
            symbol: None,
            expected: Vec::new(),
            found: Vec::new(),
            fixes: Vec::new(),
            agent: AgentHint::empty(),
        }
    }

    /// Construct an error-level diagnostic.
    pub fn error(id: impl Into<String>, message: impl Into<String>, span: Span) -> Self {
        Self::new(id, DiagnosticLevel::Error, message, span)
    }

    /// Construct a warning-level diagnostic.
    pub fn warning(id: impl Into<String>, message: impl Into<String>, span: Span) -> Self {
        Self::new(id, DiagnosticLevel::Warning, message, span)
    }

    /// Construct an informational diagnostic.
    pub fn info(id: impl Into<String>, message: impl Into<String>, span: Span) -> Self {
        Self::new(id, DiagnosticLevel::Info, message, span)
    }

    /// `true` for error-level diagnostics.
    pub fn is_error(&self) -> bool {
        self.level == DiagnosticLevel::Error
    }

    /// Attach a symbol id reference.
    pub fn with_symbol(mut self, symbol: impl Into<String>) -> Self {
        self.symbol = Some(DiagnosticSymbol { id: symbol.into() });
        self
    }

    /// Replace the `expected` token list.
    pub fn with_expected(mut self, expected: Vec<String>) -> Self {
        self.expected = expected;
        self
    }

    /// Replace the `found` token list.
    pub fn with_found(mut self, found: Vec<String>) -> Self {
        self.found = found;
        self
    }

    /// Append a single [`Fix`] suggestion.
    pub fn with_fix(mut self, fix: Fix) -> Self {
        self.fixes.push(fix);
        self
    }

    /// Replace the agent-facing summary.
    pub fn with_agent_summary(mut self, summary: impl Into<String>) -> Self {
        self.agent.summary = summary.into();
        self
    }

    /// Attach longer-form help text. The bootstrap diagnostic shape has
    /// no dedicated `help` field today, so help is routed through the
    /// agent summary — keeping the on-the-wire JSON contract stable
    /// while still letting callers express richer guidance.
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        let text = help.into();
        if self.agent.summary.is_empty() {
            self.agent.summary = text;
        } else {
            self.agent.summary.push(' ');
            self.agent.summary.push_str(&text);
        }
        self
    }

    /// Replace the agent-facing minimal-context list.
    pub fn with_minimal_context(mut self, context: Vec<String>) -> Self {
        self.agent.minimal_context = context;
        self
    }

    /// Replace the agent-facing docs list.
    pub fn with_docs(mut self, docs: Vec<String>) -> Self {
        self.agent.docs = docs;
        self
    }

    /// Render the diagnostic as a canonical JSON string.
    pub fn to_json(&self) -> String {
        to_json(self)
    }
}
