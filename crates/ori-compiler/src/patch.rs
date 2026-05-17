//! Patch IR JSON schema validator (`ori patch check`).
//!
//! Reads a serialised Patch IR document and produces structured diagnostics
//! describing any schema, intent, or per-operation issues. The check is
//! purely structural: it does not apply the patch (see [`crate::patch_apply`]
//! for that).

use crate::diagnostic::{Diagnostic, DiagnosticLevel, Fix};
use crate::json::to_json;
use crate::source::Span;
use serde::Serialize;
use serde_json::Value;

/// Stable schema id for the patch-check envelope.
pub const PATCH_CHECK_SCHEMA: &str = "ori.patch_check.v1";
/// Expected `schema` value inside a Patch IR document.
pub const PATCH_SCHEMA: &str = "ori.patch.v1";

/// Outcome of [`check_patch_json`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PatchCheckResult {
    /// `true` when no error-level diagnostics were produced.
    pub valid: bool,
    /// Per-issue diagnostics, in encounter order.
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Serialize)]
struct PatchCheckReport<'a> {
    schema: &'static str,
    valid: bool,
    diagnostics: &'a [Diagnostic],
}

impl PatchCheckResult {
    /// Render the result as the canonical `ori.patch_check.v1` JSON envelope.
    pub fn to_json(&self) -> String {
        let report = PatchCheckReport {
            schema: PATCH_CHECK_SCHEMA,
            valid: self.valid,
            diagnostics: &self.diagnostics,
        };
        to_json(&report)
    }
}

/// Validate a Patch IR document loaded from `path` (used only for
/// diagnostic spans). `text` is the raw JSON contents.
pub fn check_patch_json(path: &str, text: &str) -> PatchCheckResult {
    let mut diagnostics = Vec::new();
    let value = match serde_json::from_str::<Value>(text) {
        Ok(value) => value,
        Err(err) => {
            diagnostics.push(
                Diagnostic::error(
                    "P0000",
                    format!("patch file is not valid JSON: {err}"),
                    Span::dummy(path.to_string()),
                )
                .with_expected(vec!["valid JSON object".to_string()])
                .with_agent_summary("Parse the patch as JSON before validating Patch IR semantics.")
                .with_docs(vec!["doc:patch.schema".to_string()]),
            );
            return finish(diagnostics);
        }
    };

    let Some(root) = value.as_object() else {
        diagnostics.push(
            Diagnostic::error(
                "P0003",
                "patch root must be a JSON object",
                Span::dummy(path.to_string()),
            )
            .with_expected(vec!["object".to_string()])
            .with_found(vec![json_type_name(&value).to_string()])
            .with_agent_summary(
                "Use a JSON object with schema, intent, operations, and optional tests.",
            )
            .with_docs(vec!["doc:patch.schema".to_string()]),
        );
        return finish(diagnostics);
    };

    match root.get("schema").and_then(Value::as_str) {
        Some(PATCH_SCHEMA) => {}
        Some(other) => diagnostics.push(
            Diagnostic::error(
                "P0001",
                "patch file must declare schema `ori.patch.v1`",
                Span::dummy(path.to_string()),
            )
            .with_expected(vec!["ori.patch.v1".to_string()])
            .with_found(vec![other.to_string()])
            .with_fix(Fix::new(
                "set_patch_schema",
                "Set `schema` to `ori.patch.v1`.",
                0.93,
            ))
            .with_agent_summary("Use the supported patch schema version.")
            .with_docs(vec!["doc:patch.schema".to_string()]),
        ),
        None => diagnostics.push(
            Diagnostic::error(
                "P0001",
                "patch file must declare schema `ori.patch.v1`",
                Span::dummy(path.to_string()),
            )
            .with_expected(vec!["schema: ori.patch.v1".to_string()])
            .with_agent_summary("Add the patch schema version before applying structural changes.")
            .with_docs(vec!["doc:patch.schema".to_string()]),
        ),
    }

    match root.get("intent").and_then(Value::as_str) {
        Some(intent) if !intent.trim().is_empty() => {}
        _ => diagnostics.push(
            Diagnostic::error(
                "P0004",
                "patch file must include a non-empty intent",
                Span::dummy(path.to_string()),
            )
            .with_expected(vec!["intent: non-empty string".to_string()])
            .with_agent_summary("Describe the intended change so humans and agents can audit it.")
            .with_docs(vec!["doc:patch.intent".to_string()]),
        ),
    }

    match root.get("operations") {
        Some(Value::Array(operations)) if !operations.is_empty() => {
            for (index, operation) in operations.iter().enumerate() {
                validate_operation(path, index, operation, &mut diagnostics);
            }
        }
        Some(Value::Array(_)) => diagnostics.push(
            Diagnostic::error(
                "P0002",
                "patch file must include at least one operation",
                Span::dummy(path.to_string()),
            )
            .with_expected(vec!["operations: non-empty array".to_string()])
            .with_agent_summary(
                "Add at least one structural operation for the patch to do useful work.",
            )
            .with_docs(vec!["doc:patch.operations".to_string()]),
        ),
        Some(other) => diagnostics.push(
            Diagnostic::error(
                "P0005",
                "patch operations must be an array",
                Span::dummy(path.to_string()),
            )
            .with_expected(vec!["array".to_string()])
            .with_found(vec![json_type_name(other).to_string()])
            .with_agent_summary("Represent operations as an array of Patch IR operation objects.")
            .with_docs(vec!["doc:patch.operations".to_string()]),
        ),
        None => diagnostics.push(
            Diagnostic::error(
                "P0002",
                "patch file must include operations",
                Span::dummy(path.to_string()),
            )
            .with_expected(vec!["operations: [...]".to_string()])
            .with_agent_summary("Add a non-empty operations array.")
            .with_docs(vec!["doc:patch.operations".to_string()]),
        ),
    }

    if !root.contains_key("tests") {
        diagnostics.push(
            Diagnostic::warning(
                "P0100",
                "patch file does not declare validation tests",
                Span::dummy(path.to_string()),
            )
            .with_expected(vec!["tests.run".to_string()])
            .with_agent_summary("Declare the tests expected to validate this patch.")
            .with_docs(vec!["doc:patch.tests".to_string()]),
        );
    }

    finish(diagnostics)
}

fn validate_operation(
    path: &str,
    index: usize,
    operation: &Value,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(object) = operation.as_object() else {
        diagnostics.push(
            Diagnostic::error(
                "P1000",
                format!("operation {index} must be an object"),
                Span::dummy(path.to_string()),
            )
            .with_expected(vec!["operation object".to_string()])
            .with_found(vec![json_type_name(operation).to_string()])
            .with_agent_summary("Each Patch IR operation must be an object with an `op` field.")
            .with_docs(vec!["doc:patch.operations".to_string()]),
        );
        return;
    };

    let Some(op) = object.get("op").and_then(Value::as_str) else {
        diagnostics.push(
            Diagnostic::error(
                "P1001",
                format!("operation {index} is missing `op`"),
                Span::dummy(path.to_string()),
            )
            .with_expected(vec!["op".to_string()])
            .with_agent_summary("Add an operation kind.")
            .with_docs(vec!["doc:patch.operations".to_string()]),
        );
        return;
    };

    if !is_known_operation(op) {
        diagnostics.push(
            Diagnostic::error(
                "P1002",
                format!("operation {index} uses unknown op `{op}`"),
                Span::dummy(path.to_string()),
            )
            .with_expected(
                KNOWN_OPERATIONS
                    .iter()
                    .map(|name| (*name).to_string())
                    .collect(),
            )
            .with_found(vec![op.to_string()])
            .with_agent_summary("Use a supported Patch IR operation kind.")
            .with_docs(vec!["doc:patch.operations".to_string()]),
        );
        return;
    }

    for field in required_fields(op) {
        if !object.contains_key(*field) {
            diagnostics.push(
                Diagnostic::error(
                    "P1003",
                    format!("operation {index} `{op}` is missing required field `{field}`"),
                    Span::dummy(path.to_string()),
                )
                .with_expected(vec![field.to_string()])
                .with_agent_summary("Add the required field for this operation kind.")
                .with_docs(vec!["doc:patch.operations".to_string()]),
            );
        }
    }
}

fn finish(diagnostics: Vec<Diagnostic>) -> PatchCheckResult {
    let valid = diagnostics
        .iter()
        .all(|diagnostic| diagnostic.level != DiagnosticLevel::Error);
    PatchCheckResult { valid, diagnostics }
}

const KNOWN_OPERATIONS: &[&str] = &[
    "replace_node",
    "insert_node",
    "delete_node",
    "rename_symbol",
    "add_import",
    "remove_import",
    "change_signature",
    "insert_match_arm",
    "add_field",
    "remove_field",
    "add_protocol_impl",
    "update_route",
    "update_view",
    "add_test",
];

fn is_known_operation(op: &str) -> bool {
    KNOWN_OPERATIONS.contains(&op)
}

fn required_fields(op: &str) -> &'static [&'static str] {
    match op {
        "replace_node" => &["target"],
        "insert_node" => &["target", "position"],
        "delete_node" => &["target"],
        "rename_symbol" => &["from", "to"],
        "add_import" => &["text"],
        "remove_import" => &["target"],
        "change_signature" => &["target", "text"],
        "insert_match_arm" => &["target", "pattern", "body"],
        "add_field" => &["target", "text"],
        "remove_field" => &["target"],
        "add_protocol_impl" => &["target", "text"],
        "update_route" => &["target", "text"],
        "update_view" => &["target", "text"],
        "add_test" => &["text"],
        _ => &[],
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
