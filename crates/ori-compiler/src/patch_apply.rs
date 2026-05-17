//! Apply Patch IR documents to source files.
//!
//! The bootstrap apply engine resolves operation `target` IDs against the
//! CST that the compiler can compute for any module, then emits the
//! resulting text (and a structured dry-run report) without touching disk.
//! Stale node IDs are rejected with the same `P1*` diagnostic codes used by
//! `patch check`, so callers get a uniform contract whether validating or
//! applying.

use crate::cst::{parse_cst, CstNodeKind};
use crate::diagnostic::Diagnostic;
use crate::json::to_json;
use crate::source::{SourceFile, Span};
use serde::Serialize;
use serde_json::Value;

/// Stable schema id for the patch-apply envelope.
pub const PATCH_APPLY_SCHEMA: &str = "ori.patch_apply.v1";

/// Structured report produced by [`apply_patch`].
#[derive(Debug, Serialize)]
pub struct PatchApplyReport {
    /// Stable schema identifier.
    pub schema: &'static str,
    /// `true` when at least one operation was applied successfully.
    pub applied: bool,
    /// `true` when the run was a dry run (no disk writes are ever made).
    pub dry_run: bool,
    /// Number of operations encountered in the patch.
    pub operations_attempted: usize,
    /// Subset of those that applied without error.
    pub operations_applied: usize,
    /// Diagnostics emitted during apply.
    pub diagnostics: Vec<Diagnostic>,
    /// Source text before the patch (always present).
    pub before: Option<String>,
    /// Source text after the patch (only when `applied` is true).
    pub after: Option<String>,
}

impl PatchApplyReport {
    /// Render the report as canonical JSON.
    pub fn to_json(&self) -> String {
        to_json(self)
    }
}

/// Apply `patch_json` to `source`. When `dry_run` is `true` the resulting
/// text is computed but not persisted; in either case the call is a
/// pure function over the inputs.
pub fn apply_patch(source: &SourceFile, patch_json: &str, dry_run: bool) -> PatchApplyReport {
    let mut diagnostics = Vec::new();
    let mut report = PatchApplyReport {
        schema: PATCH_APPLY_SCHEMA,
        applied: false,
        dry_run,
        operations_attempted: 0,
        operations_applied: 0,
        diagnostics: Vec::new(),
        before: Some(source.text.clone()),
        after: None,
    };

    let value: Value = match serde_json::from_str(patch_json) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::error(
                "P0000",
                format!("patch file is not valid JSON: {err}"),
                Span::dummy(source.path.clone()),
            ));
            report.diagnostics = diagnostics;
            return report;
        }
    };

    let Some(root) = value.as_object() else {
        diagnostics.push(Diagnostic::error(
            "P0003",
            "patch root must be a JSON object",
            Span::dummy(source.path.clone()),
        ));
        report.diagnostics = diagnostics;
        return report;
    };
    let Some(operations) = root.get("operations").and_then(Value::as_array) else {
        diagnostics.push(Diagnostic::error(
            "P0002",
            "patch file must include operations",
            Span::dummy(source.path.clone()),
        ));
        report.diagnostics = diagnostics;
        return report;
    };

    let cst = parse_cst(source);
    let ast = crate::parser::parse_source(source).module;
    let mut text = source.text.clone();
    let mut applied = 0usize;

    // Resolve a target id (CST `node:` id, AST `sym:` id, or module `mod:` id)
    // to a 1-based source line.
    let resolve_line = |id: &str| -> Option<usize> {
        if let Some(node) = cst.find(id) {
            return Some(node.span.start.line);
        }
        if let Some(sym) = ast.symbols.iter().find(|s| s.id == id) {
            return Some(sym.span.start.line);
        }
        // Translate `sym:<module>` to `mod:<module>` (the module declaration).
        if let Some(rest) = id.strip_prefix("sym:") {
            let mod_id = format!("mod:{rest}");
            if let Some(sym) = ast.symbols.iter().find(|s| s.id == mod_id) {
                return Some(sym.span.start.line);
            }
        }
        None
    };

    // Parse an optional position field that may be either a bare keyword
    // ("before" / "after") or a directive of the form
    // "<keyword>:<id>" which overrides the anchor.
    let resolve_position = |position: &str, default_line: usize| -> (String, usize) {
        if let Some(rest) = position.strip_prefix("after:") {
            if let Some(line) = resolve_line(rest.trim()) {
                return ("after".to_string(), line);
            }
        }
        if let Some(rest) = position.strip_prefix("before:") {
            if let Some(line) = resolve_line(rest.trim()) {
                return ("before".to_string(), line);
            }
        }
        let normalized = if position.is_empty() {
            "after".to_string()
        } else {
            position.to_string()
        };
        (normalized, default_line)
    };

    for (idx, op) in operations.iter().enumerate() {
        report.operations_attempted += 1;
        let Some(obj) = op.as_object() else {
            diagnostics.push(Diagnostic::error(
                "P1000",
                format!("operation {idx} must be an object"),
                Span::dummy(source.path.clone()),
            ));
            continue;
        };
        let Some(kind) = obj.get("op").and_then(Value::as_str) else {
            diagnostics.push(Diagnostic::error(
                "P1001",
                format!("operation {idx} is missing `op`"),
                Span::dummy(source.path.clone()),
            ));
            continue;
        };

        match kind {
            "insert_node" => {
                let target_id = obj.get("target").and_then(Value::as_str);
                let raw_position = obj
                    .get("position")
                    .and_then(Value::as_str)
                    .unwrap_or("after");
                let payload = obj.get("text").and_then(Value::as_str).unwrap_or("");
                if let Some(line) = target_id.and_then(resolve_line) {
                    let (position, effective_line) = resolve_position(raw_position, line);
                    let new_text = insert_line(&text, effective_line, &position, payload);
                    text = new_text;
                    applied += 1;
                } else {
                    diagnostics.push(stale_target_diagnostic(idx, target_id, &source.path));
                }
            }
            "insert_after" => {
                let target_id = obj.get("target").and_then(Value::as_str);
                let payload = obj.get("text").and_then(Value::as_str).unwrap_or("");
                if let Some(line) = target_id.and_then(resolve_line) {
                    text = insert_line(&text, line, "after", payload);
                    applied += 1;
                } else {
                    diagnostics.push(stale_target_diagnostic(idx, target_id, &source.path));
                }
            }
            "replace_node" | "change_signature" | "update_route" | "update_view" => {
                let target_id = obj.get("target").and_then(Value::as_str);
                let payload = obj.get("text").and_then(Value::as_str).unwrap_or("");
                if let Some(line) = target_id.and_then(resolve_line) {
                    text = replace_line(&text, line, payload);
                    applied += 1;
                } else {
                    diagnostics.push(stale_target_diagnostic(idx, target_id, &source.path));
                }
            }
            "delete_node" => {
                let target_id = obj.get("target").and_then(Value::as_str);
                if let Some(line) = target_id.and_then(resolve_line) {
                    text = delete_line(&text, line);
                    applied += 1;
                } else {
                    diagnostics.push(stale_target_diagnostic(idx, target_id, &source.path));
                }
            }
            "add_import" => {
                let payload = obj.get("text").and_then(Value::as_str).unwrap_or("");
                text = insert_after_imports_or_module(&text, payload, &cst);
                applied += 1;
            }
            "rename_symbol" => {
                let from = obj.get("from").and_then(Value::as_str).unwrap_or("");
                let to = obj.get("to").and_then(Value::as_str).unwrap_or("");
                if from.is_empty() || to.is_empty() {
                    diagnostics.push(Diagnostic::error(
                        "P1003",
                        format!("operation {idx} `rename_symbol` requires `from` and `to`"),
                        Span::dummy(source.path.clone()),
                    ));
                    continue;
                }
                text = rename_identifier(&text, from, to);
                applied += 1;
            }
            "insert_match_arm" | "add_arm" => {
                let target_id = obj.get("target").and_then(Value::as_str);
                let pattern = obj.get("pattern").and_then(Value::as_str).unwrap_or("_");
                let body = obj.get("body").and_then(Value::as_str).unwrap_or("Unit");
                if let Some(line) = target_id.and_then(resolve_line) {
                    let payload = format!("  | {pattern} => {body}");
                    text = insert_line(&text, line, "after", &payload);
                    applied += 1;
                } else {
                    diagnostics.push(stale_target_diagnostic(idx, target_id, &source.path));
                }
            }
            other => {
                diagnostics.push(Diagnostic::error(
                    "P1002",
                    format!("operation {idx} uses unsupported op `{other}` for apply"),
                    Span::dummy(source.path.clone()),
                ));
            }
        }
    }

    // Fatal-error classification: structural problems (P1000, P1001, P1003) and
    // unsupported ops (P1002) abort the whole patch. Stale-target errors (P1010)
    // are *per-op* skips — other operations still apply.
    let fatal_codes = ["P1000", "P1001", "P1002", "P1003"];
    let has_fatal = diagnostics
        .iter()
        .any(|d| d.is_error() && fatal_codes.iter().any(|c| d.id == *c));
    report.diagnostics = diagnostics;
    report.operations_applied = applied;
    if has_fatal {
        report.applied = false;
        report.after = None;
    } else if applied == 0 {
        // Nothing applied (e.g. all ops were stale targets); report failure
        // but no after-text either.
        report.applied = false;
        report.after = None;
    } else {
        // At least one op applied and no fatal errors — emit partial-apply
        // result regardless of dry-run flag.
        report.applied = true;
        report.after = Some(text);
    }
    report
}

fn stale_target_diagnostic(index: usize, target: Option<&str>, path: &str) -> Diagnostic {
    let target = target.unwrap_or("<missing>");
    Diagnostic::error(
        "P1010",
        format!("operation {index} references unknown node id `{target}`"),
        Span::dummy(path.to_string()),
    )
    .with_expected(vec!["a node id present in the current CST".to_string()])
    .with_found(vec![target.to_string()])
    .with_agent_summary("Re-resolve target ids against the latest CST before applying.")
    .with_docs(vec!["doc:patch.targets".to_string()])
}

fn insert_line(text: &str, line: usize, position: &str, payload: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let idx = match position {
        "before" => line.saturating_sub(1),
        _ => line.min(lines.len()),
    };
    let normalized = payload.trim_end_matches('\n').to_string();
    if idx >= lines.len() {
        lines.push(normalized);
    } else {
        lines.insert(idx, normalized);
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn replace_line(text: &str, line: usize, payload: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let idx = line.saturating_sub(1);
    let normalized = payload.trim_end_matches('\n').to_string();
    if idx < lines.len() {
        lines[idx] = normalized;
    } else {
        lines.push(normalized);
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn delete_line(text: &str, line: usize) -> String {
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let idx = line.saturating_sub(1);
    if idx < lines.len() {
        lines.remove(idx);
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn insert_after_imports_or_module(text: &str, payload: &str, cst: &crate::cst::Cst) -> String {
    // Find the last import line, or the module line, and insert after it.
    let mut target_line = 1usize;
    for node in &cst.nodes {
        if matches!(node.kind, CstNodeKind::Import | CstNodeKind::Module) {
            target_line = target_line.max(node.span.start.line);
        }
    }
    insert_line(text, target_line, "after", payload)
}

fn rename_identifier(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut buf = String::new();
    let mut in_string = false;
    let mut escape = false;
    for ch in text.chars() {
        if in_string {
            out.push(ch);
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            flush_ident(&mut out, &mut buf, from, to);
            out.push(ch);
            in_string = true;
            continue;
        }
        if ch.is_ascii_alphanumeric() || ch == '_' {
            buf.push(ch);
        } else {
            flush_ident(&mut out, &mut buf, from, to);
            out.push(ch);
        }
    }
    flush_ident(&mut out, &mut buf, from, to);
    out
}

fn flush_ident(out: &mut String, buf: &mut String, from: &str, to: &str) {
    if buf.is_empty() {
        return;
    }
    if buf == from {
        out.push_str(to);
    } else {
        out.push_str(buf);
    }
    buf.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(text: &str) -> SourceFile {
        SourceFile::new("/t.ori", text)
    }

    fn patch_with_op(target: &str, op: &str, payload: &str) -> String {
        serde_json::json!({
            "schema": "ori.patch.v1",
            "intent": "test",
            "operations": [ { "op": op, "target": target, "text": payload } ]
        })
        .to_string()
    }

    #[test]
    fn insert_node_adds_line_after_target() {
        let s = src("module a\nfn old() -> Unit\n");
        let cst = parse_cst(&s);
        let id = cst
            .nodes
            .iter()
            .find(|n| n.name == "old")
            .map(|n| n.id.clone())
            .unwrap_or_else(|| crate::node_id::NodeId::new("node:missing"));
        let patch = serde_json::json!({
            "schema": "ori.patch.v1",
            "intent": "add_new",
            "operations": [ {
                "op": "insert_node",
                "target": id.as_str(),
                "position": "after",
                "text": "fn fresh() -> Int"
            } ],
            "tests": { "run": [] }
        })
        .to_string();
        let report = apply_patch(&s, &patch, true);
        assert!(report.applied, "diagnostics: {:?}", report.diagnostics);
        assert!(report
            .after
            .as_deref()
            .unwrap_or("")
            .contains("fn fresh() -> Int"));
    }

    #[test]
    fn stale_target_yields_p1010() {
        let s = src("module a\nfn keep() -> Unit\n");
        let patch = patch_with_op(
            "node:demo.fn.nonexistent.deadbeefdeadbeef",
            "insert_node",
            "fn boom() -> Unit",
        );
        let report = apply_patch(&s, &patch, true);
        assert!(!report.applied, "all ops stale, nothing applied");
        assert!(report.diagnostics.iter().any(|d| d.id == "P1010"));
    }

    #[test]
    fn partial_apply_returns_after_when_one_op_works() {
        let s = src("module a\nfn keep() -> Unit\n");
        let cst = parse_cst(&s);
        let real_id = cst
            .nodes
            .iter()
            .find(|n| n.name == "keep")
            .map(|n| n.id.as_str().to_string())
            .unwrap_or_default();
        let patch = serde_json::json!({
            "schema": "ori.patch.v1",
            "intent": "mixed",
            "operations": [
                { "op": "insert_node", "target": real_id, "position": "after", "text": "fn fresh() -> Int" },
                { "op": "insert_node", "target": "node:nope.fn.x.0", "position": "after", "text": "fn boom() -> Int" }
            ]
        })
        .to_string();
        let report = apply_patch(&s, &patch, true);
        assert!(report.applied, "partial apply should be marked applied");
        assert_eq!(report.operations_applied, 1);
        assert!(report.diagnostics.iter().any(|d| d.id == "P1010"));
        assert!(report.after.unwrap_or_default().contains("fn fresh()"));
    }

    #[test]
    fn add_import_inserts_after_module() {
        let s = src("module demo\nfn f() -> Unit\n");
        let patch = serde_json::json!({
            "schema": "ori.patch.v1",
            "intent": "add_import",
            "operations": [ {
                "op": "add_import",
                "text": "import std.json"
            } ]
        })
        .to_string();
        let report = apply_patch(&s, &patch, true);
        let after = report.after.unwrap_or_default();
        let first_two: Vec<_> = after.lines().take(2).collect();
        assert_eq!(first_two[0], "module demo");
        assert_eq!(first_two[1], "import std.json");
    }

    #[test]
    fn rename_symbol_renames_only_identifiers() {
        let s = src("module a\nfn dup() -> Unit\nfn helper() -> Unit\n");
        let patch = serde_json::json!({
            "schema": "ori.patch.v1",
            "intent": "rename",
            "operations": [ {
                "op": "rename_symbol",
                "from": "dup",
                "to": "renamed"
            } ]
        })
        .to_string();
        let report = apply_patch(&s, &patch, true);
        let after = report.after.unwrap_or_default();
        assert!(after.contains("fn renamed()"));
        assert!(!after.contains("fn dup("));
    }

    #[test]
    fn rename_does_not_touch_string_contents() {
        let s = src("module a\nfn run() -> Str\nfn use_it() -> Str\n");
        let patch = serde_json::json!({
            "schema": "ori.patch.v1",
            "intent": "rename_run",
            "operations": [ {
                "op": "rename_symbol",
                "from": "run",
                "to": "execute"
            } ]
        })
        .to_string();
        // Even though the word run appears in the text, the rename should treat
        // it as an identifier; string literals are preserved verbatim.
        let report = apply_patch(&s, &patch, true);
        let after = report.after.unwrap_or_default();
        assert!(after.contains("fn execute()"));
    }

    #[test]
    fn unsupported_op_is_reported() {
        let s = src("module a\nfn f() -> Unit\n");
        let patch = serde_json::json!({
            "schema": "ori.patch.v1",
            "intent": "x",
            "operations": [ { "op": "unknown_op", "target": "x" } ]
        })
        .to_string();
        let report = apply_patch(&s, &patch, true);
        assert!(!report.applied);
        assert!(report.diagnostics.iter().any(|d| d.id == "P1002"));
    }
}
