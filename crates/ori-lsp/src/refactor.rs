//! Workspace refactors plus call-hierarchy and type-hierarchy helpers.
//!
//! This module is **pure**: every public function takes its inputs (typically
//! one or more `(uri, text)` pairs and a request payload) and returns a
//! deterministic answer. No global state and no I/O is performed here. The
//! [`crate::server`] handlers thread the request through the right helper and
//! serialise the result.
//!
//! ## Determinism rules
//!
//! * Every result list is sorted by `(uri, line, character)` before return.
//! * `WorkspaceEdit::changes` is a `BTreeMap` so URIs are in lexicographic
//!   order; the inner `Vec<TextEdit>` is sorted bottom-up so callers can
//!   apply edits without re-computing positions.
//! * Names that disambiguate two equal positions (e.g. variant constructors
//!   on the same line) sort by lexicographic identifier.
//!
//! ## Schemas
//!
//! Wire shapes are taken straight from LSP §3.18 (call hierarchy) and §3.18
//! (type hierarchy). We model the minimum the spec requires for round-trip
//! interoperability; richer fields like `tags` are intentionally omitted
//! because the bootstrap server has no source of truth for them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use ori_compiler::ast::{Module, Symbol, SymbolKind as CompilerSymbolKind};
use ori_compiler::body::parse_module_bodies_with_module;
use ori_compiler::compiler::Compiler;
use ori_compiler::expr::{Expr, InterpPart, MatchArm, Stmt};
use ori_compiler::source::SourceFile;

use crate::diagnostics::span_to_range;
use crate::protocol::{
    Position, Range, SymbolKind as LspSymbolKind, TextDocumentIdentifier, TextEdit, WorkspaceEdit,
};

// ---------------------------------------------------------------------------
// Call hierarchy
// ---------------------------------------------------------------------------

/// `textDocument/prepareCallHierarchy` request params.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallHierarchyPrepareParams {
    pub text_document: TextDocumentIdentifier,
    pub position: Position,
}

/// One node in the call hierarchy graph. Mirrors LSP `CallHierarchyItem`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallHierarchyItem {
    pub name: String,
    pub kind: LspSymbolKind,
    pub uri: String,
    pub range: Range,
    pub selection_range: Range,
    /// Stable identifier so resolve handlers can round-trip. We piggy-back
    /// the compiler symbol id (`sym:<module>.<name>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// `callHierarchy/incomingCalls` request params.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallHierarchyIncomingCallsParams {
    pub item: CallHierarchyItem,
}

/// `callHierarchy/outgoingCalls` request params.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallHierarchyOutgoingCallsParams {
    pub item: CallHierarchyItem,
}

/// LSP `CallHierarchyIncomingCall`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallHierarchyIncomingCall {
    pub from: CallHierarchyItem,
    pub from_ranges: Vec<Range>,
}

/// LSP `CallHierarchyOutgoingCall`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallHierarchyOutgoingCall {
    pub to: CallHierarchyItem,
    pub from_ranges: Vec<Range>,
}

/// View of a single workspace document used by every helper in this module.
/// The server passes a borrowed slice so we never need to clone the text.
#[derive(Debug, Clone)]
pub struct WorkspaceDoc<'a> {
    pub uri: &'a str,
    pub text: &'a str,
}

/// `prepareCallHierarchy` implementation. Returns the function symbol at the
/// cursor position, or an empty list if the cursor does not point to a
/// function declaration in the requested document.
pub fn prepare_call_hierarchy(
    docs: &[WorkspaceDoc<'_>],
    uri: &str,
    position: Position,
) -> Vec<CallHierarchyItem> {
    let Some(doc) = docs.iter().find(|d| d.uri == uri) else {
        return Vec::new();
    };
    let result = Compiler::check_source(SourceFile::new(doc.uri, doc.text));
    let line_one = (position.line as usize).saturating_add(1);
    let column_one = (position.character as usize).saturating_add(1);

    // Prefer an exact span hit; fall back to a name match against the
    // identifier under the cursor so clicks anywhere inside the function
    // header land on the symbol.
    let mut hits: Vec<CallHierarchyItem> = Vec::new();
    for symbol in &result.module.symbols {
        if symbol.kind != CompilerSymbolKind::Function {
            continue;
        }
        if span_contains(&symbol.span, line_one, column_one) {
            hits.push(symbol_to_item(uri, symbol));
        }
    }
    if hits.is_empty() {
        if let Some(name) = identifier_at(doc.text, position) {
            for symbol in &result.module.symbols {
                if symbol.kind == CompilerSymbolKind::Function && symbol.name == name {
                    hits.push(symbol_to_item(uri, symbol));
                }
            }
        }
    }
    hits.sort_by_key(item_sort_key);
    hits.dedup_by(|a, b| a.uri == b.uri && a.range.start.line == b.range.start.line);
    hits
}

/// `callHierarchy/incomingCalls` implementation. Walks every function body
/// across the workspace and reports each call site that targets `item.name`.
pub fn incoming_calls(
    docs: &[WorkspaceDoc<'_>],
    item: &CallHierarchyItem,
) -> Vec<CallHierarchyIncomingCall> {
    let mut by_caller: BTreeMap<(String, String), CallHierarchyIncomingCall> = BTreeMap::new();

    for doc in docs.iter() {
        let source = SourceFile::new(doc.uri, doc.text);
        let result = Compiler::check_source(source.clone());
        let bodies = parse_module_bodies_with_module(&source, &result.module);
        for caller in &result.module.symbols {
            if caller.kind != CompilerSymbolKind::Function {
                continue;
            }
            let Some(body) = bodies.get(&caller.id) else {
                continue;
            };
            let mut call_count: usize = 0;
            count_calls_to(body, &item.name, &mut call_count);
            if call_count == 0 {
                continue;
            }
            let from_item = symbol_to_item(doc.uri, caller);
            // We have no source spans for the call sites at the expression
            // level; report the caller's selection range repeatedly so
            // tools that key off `from_ranges.len()` see the right count.
            let mut from_ranges: Vec<Range> = Vec::with_capacity(call_count);
            for _ in 0..call_count {
                from_ranges.push(from_item.selection_range);
            }
            let key = (doc.uri.to_string(), caller.id.clone());
            by_caller.insert(
                key,
                CallHierarchyIncomingCall {
                    from: from_item,
                    from_ranges,
                },
            );
        }
    }

    let mut out: Vec<CallHierarchyIncomingCall> = by_caller.into_values().collect();
    out.sort_by(|a, b| {
        a.from
            .uri
            .cmp(&b.from.uri)
            .then_with(|| item_sort_key(&a.from).cmp(&item_sort_key(&b.from)))
    });
    out
}

/// `callHierarchy/outgoingCalls` implementation. Lists every function called
/// from inside `item`'s body. The walk is restricted to the document that
/// hosts `item` because Orison's bootstrap parser keeps function bodies
/// scoped to their declaring module.
pub fn outgoing_calls(
    docs: &[WorkspaceDoc<'_>],
    item: &CallHierarchyItem,
) -> Vec<CallHierarchyOutgoingCall> {
    let Some(doc) = docs.iter().find(|d| d.uri == item.uri) else {
        return Vec::new();
    };
    let source = SourceFile::new(doc.uri, doc.text);
    let result = Compiler::check_source(source.clone());
    let bodies = parse_module_bodies_with_module(&source, &result.module);
    let Some(symbol) = result
        .module
        .symbols
        .iter()
        .find(|s| s.kind == CompilerSymbolKind::Function && s.name == item.name)
    else {
        return Vec::new();
    };
    let Some(body) = bodies.get(&symbol.id) else {
        return Vec::new();
    };

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    collect_call_names(body, &mut counts);

    // Resolve each callee name against every workspace document so calls
    // to other modules still resolve (best-effort).
    let mut out: Vec<CallHierarchyOutgoingCall> = Vec::new();
    for (name, count) in counts {
        if name == item.name {
            // Recursive call — represent it but keep the caller item.
        }
        let mut found = false;
        for d in docs.iter() {
            let r = Compiler::check_source(SourceFile::new(d.uri, d.text));
            for s in &r.module.symbols {
                if s.kind == CompilerSymbolKind::Function && s.name == name {
                    let to = symbol_to_item(d.uri, s);
                    let from_ranges = vec![to.selection_range; count];
                    out.push(CallHierarchyOutgoingCall { to, from_ranges });
                    found = true;
                }
            }
            if found {
                break;
            }
        }
    }
    out.sort_by(|a, b| {
        a.to.uri
            .cmp(&b.to.uri)
            .then_with(|| item_sort_key(&a.to).cmp(&item_sort_key(&b.to)))
    });
    out
}

// ---------------------------------------------------------------------------
// Type hierarchy
// ---------------------------------------------------------------------------

/// `textDocument/prepareTypeHierarchy` request params.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeHierarchyPrepareParams {
    pub text_document: TextDocumentIdentifier,
    pub position: Position,
}

/// `typeHierarchy/supertypes` request params.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeHierarchySupertypesParams {
    pub item: TypeHierarchyItem,
}

/// `typeHierarchy/subtypes` request params.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeHierarchySubtypesParams {
    pub item: TypeHierarchyItem,
}

/// One node in the type hierarchy graph. Mirrors LSP `TypeHierarchyItem`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeHierarchyItem {
    pub name: String,
    pub kind: LspSymbolKind,
    pub uri: String,
    pub range: Range,
    pub selection_range: Range,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Opaque payload that round-trips through resolve calls. Stores
    /// `{ "kind": "variant"|"type", "owner": "<type-name>" }`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// `prepareTypeHierarchy` implementation. Returns either the type symbol at
/// the cursor or, if the cursor sits on a variant constructor, the variant
/// itself.
pub fn prepare_type_hierarchy(
    docs: &[WorkspaceDoc<'_>],
    uri: &str,
    position: Position,
) -> Vec<TypeHierarchyItem> {
    let Some(doc) = docs.iter().find(|d| d.uri == uri) else {
        return Vec::new();
    };
    let result = Compiler::check_source(SourceFile::new(doc.uri, doc.text));
    let line_one = (position.line as usize).saturating_add(1);
    let column_one = (position.character as usize).saturating_add(1);

    // Hit-test type symbols first.
    let mut out: Vec<TypeHierarchyItem> = Vec::new();
    for symbol in &result.module.symbols {
        if symbol.kind != CompilerSymbolKind::Type {
            continue;
        }
        if span_contains(&symbol.span, line_one, column_one) {
            out.push(type_symbol_to_item(uri, symbol));
        }
    }
    if !out.is_empty() {
        out.sort_by(type_item_sort_key);
        return out;
    }

    // No type span hit — see if the cursor is on a constructor inside any
    // type declaration. Walk the variants for every type symbol and check
    // their token positions.
    let needle = identifier_at(doc.text, position);
    if let Some(name) = needle {
        for symbol in &result.module.symbols {
            if symbol.kind != CompilerSymbolKind::Type {
                continue;
            }
            for variant in find_variants(doc.text, symbol) {
                if variant.name == name {
                    out.push(variant.to_item(uri, &symbol.name));
                }
            }
        }
    }

    out.sort_by(type_item_sort_key);
    out
}

/// `typeHierarchy/supertypes` — for a variant constructor, return the
/// enclosing type. For a type itself, return nothing (Orison's bootstrap
/// grammar has no inheritance relation between types).
pub fn supertypes(docs: &[WorkspaceDoc<'_>], item: &TypeHierarchyItem) -> Vec<TypeHierarchyItem> {
    let owner = item
        .data
        .as_ref()
        .and_then(|v| v.get("owner").and_then(Value::as_str).map(str::to_string));
    let Some(owner) = owner else {
        return Vec::new();
    };
    let mut out: Vec<TypeHierarchyItem> = Vec::new();
    for doc in docs.iter() {
        let result = Compiler::check_source(SourceFile::new(doc.uri, doc.text));
        for symbol in &result.module.symbols {
            if symbol.kind == CompilerSymbolKind::Type && symbol.name == owner {
                out.push(type_symbol_to_item(doc.uri, symbol));
            }
        }
    }
    out.sort_by(type_item_sort_key);
    out
}

/// `typeHierarchy/subtypes` — for a variant type, return its constructors.
/// For a constructor we have nothing to return.
pub fn subtypes(docs: &[WorkspaceDoc<'_>], item: &TypeHierarchyItem) -> Vec<TypeHierarchyItem> {
    // Subtypes only make sense for a type, not a variant constructor.
    let is_variant = item
        .data
        .as_ref()
        .and_then(|v| v.get("kind").and_then(Value::as_str))
        .map(|k| k == "variant")
        .unwrap_or(false);
    if is_variant {
        return Vec::new();
    }
    let Some(doc) = docs.iter().find(|d| d.uri == item.uri) else {
        return Vec::new();
    };
    let result = Compiler::check_source(SourceFile::new(doc.uri, doc.text));
    let mut out: Vec<TypeHierarchyItem> = Vec::new();
    for symbol in &result.module.symbols {
        if symbol.kind == CompilerSymbolKind::Type && symbol.name == item.name {
            for variant in find_variants(doc.text, symbol) {
                out.push(variant.to_item(doc.uri, &symbol.name));
            }
        }
    }
    out.sort_by(type_item_sort_key);
    out
}

// ---------------------------------------------------------------------------
// Execute command — extractFunction / inline
// ---------------------------------------------------------------------------

/// Identifier for `workspace/executeCommand`.
pub const COMMAND_EXTRACT_FUNCTION: &str = "ori.refactor.extractFunction";
/// Identifier for `workspace/executeCommand`.
pub const COMMAND_INLINE: &str = "ori.refactor.inline";

/// Outcome of a refactor request. `Edits` is returned to the client as a
/// `WorkspaceEdit`; `Rejected` translates to an `INVALID_PARAMS` error.
#[derive(Debug, Clone)]
pub enum RefactorOutcome {
    Edits(WorkspaceEdit),
    Rejected(String),
}

/// Arguments for `ori.refactor.extractFunction`. The client supplies the
/// document URI, the range of text to extract, and the new function name.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractFunctionArgs {
    pub text_document: TextDocumentIdentifier,
    pub range: Range,
    pub name: String,
}

/// Arguments for `ori.refactor.inline`. `position` points at a call site;
/// the refactor replaces that call with the body of the target function.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineArgs {
    pub text_document: TextDocumentIdentifier,
    pub position: Position,
}

/// Execute `ori.refactor.extractFunction`. The selection must:
///   * cover at least one non-blank line,
///   * stay within the body of a single function (start and end line both
///     inside the same function body),
///   * not collide with an existing symbol name.
pub fn extract_function(doc: &WorkspaceDoc<'_>, args: &ExtractFunctionArgs) -> RefactorOutcome {
    if !is_valid_identifier(&args.name) {
        return RefactorOutcome::Rejected(format!(
            "new function name `{}` is not a valid identifier",
            args.name
        ));
    }
    let result = Compiler::check_source(SourceFile::new(doc.uri, doc.text));
    if result
        .module
        .symbols
        .iter()
        .any(|s| s.kind != CompilerSymbolKind::Module && s.name == args.name)
    {
        return RefactorOutcome::Rejected(format!(
            "symbol `{}` already exists in the document",
            args.name
        ));
    }
    // Range must enclose at least one line of code.
    if args.range.start.line > args.range.end.line {
        return RefactorOutcome::Rejected("range start is after range end".to_string());
    }
    // The host function: start and end must fall inside the same function
    // body. We approximate "function body" as "inside the source span
    // of one function symbol".
    let host = find_enclosing_function(&result.module, args.range.start, args.range.end);
    let Some(host) = host else {
        return RefactorOutcome::Rejected(
            "selection must fall within a single function body".to_string(),
        );
    };
    let lines: Vec<&str> = doc.text.split('\n').collect();
    let start_line = args.range.start.line as usize;
    let end_line = args.range.end.line as usize;
    if start_line >= lines.len() {
        return RefactorOutcome::Rejected("selection start is past the end of file".to_string());
    }
    if end_line >= lines.len() {
        return RefactorOutcome::Rejected("selection end is past the end of file".to_string());
    }
    let host_indent = host_body_indent(&lines, &host);
    let mut selected: Vec<&str> = Vec::with_capacity(end_line - start_line + 1);
    for line in &lines[start_line..=end_line] {
        selected.push(line);
    }
    if selected.iter().all(|s| s.trim().is_empty()) {
        return RefactorOutcome::Rejected("selection is empty".to_string());
    }

    let mut edits: Vec<TextEdit> = Vec::new();

    // 1) Replace the selected lines with a call. The call's indent matches
    //    the original first line's indent.
    let first_indent_len = leading_space_len(lines[start_line]);
    let call_indent: String = " ".repeat(first_indent_len);
    let new_text = format!("{call_indent}{}()", args.name);
    // We need the end position to be the end of the last line of the
    // selection so we drop the original code completely.
    let end_char = lines.get(end_line).map(|l| l.chars().count()).unwrap_or(0) as u32;
    edits.push(TextEdit {
        range: Range {
            start: Position {
                line: start_line as u32,
                character: 0,
            },
            end: Position {
                line: end_line as u32,
                character: end_char,
            },
        },
        new_text,
    });

    // 2) Append a new function after the host function. We compute the
    //    insertion position as the end of the host's last body line.
    let host_end_line = function_body_end_line(&lines, &host);
    let insertion_line = host_end_line + 1;
    let mut new_fn = String::new();
    new_fn.push('\n');
    new_fn.push_str(&format!("fn {}() -> Unit:\n", args.name));
    let inner_indent = " ".repeat(host_indent + 2);
    for line in &selected {
        let trimmed = strip_common_indent(line, host_indent);
        new_fn.push_str(&inner_indent);
        new_fn.push_str(trimmed);
        new_fn.push('\n');
    }
    let insertion_position = Position {
        line: insertion_line as u32,
        character: 0,
    };
    edits.push(TextEdit {
        range: Range {
            start: insertion_position,
            end: insertion_position,
        },
        new_text: new_fn,
    });

    let mut changes: BTreeMap<String, Vec<TextEdit>> = BTreeMap::new();
    edits.sort_by(|a, b| {
        // Sort *bottom-up* (largest start position first) so applying the
        // edits in order does not shift positions of earlier edits. This
        // is the LSP-recommended deterministic order.
        (b.range.start.line, b.range.start.character)
            .cmp(&(a.range.start.line, a.range.start.character))
    });
    changes.insert(doc.uri.to_string(), edits);
    RefactorOutcome::Edits(WorkspaceEdit { changes })
}

/// Execute `ori.refactor.inline`. The position must sit on an identifier
/// that names a function in the current document; the function must not
/// recurse (self-call) and must have a body the inliner can extract.
pub fn inline(doc: &WorkspaceDoc<'_>, args: &InlineArgs) -> RefactorOutcome {
    let Some(name) = identifier_at(doc.text, args.position) else {
        return RefactorOutcome::Rejected("no identifier at position".to_string());
    };
    let source = SourceFile::new(doc.uri, doc.text);
    let result = Compiler::check_source(source.clone());
    let Some(target) = result
        .module
        .symbols
        .iter()
        .find(|s| s.kind == CompilerSymbolKind::Function && s.name == name)
    else {
        return RefactorOutcome::Rejected(format!("no function named `{name}`"));
    };
    let bodies = parse_module_bodies_with_module(&source, &result.module);
    let Some(body) = bodies.get(&target.id) else {
        return RefactorOutcome::Rejected(format!("function `{name}` has no body to inline"));
    };
    let mut recurses = false;
    let mut count: usize = 0;
    count_calls_to(body, &name, &mut count);
    if count > 0 {
        recurses = true;
    }
    if recurses {
        return RefactorOutcome::Rejected(format!("function `{name}` is recursive"));
    }

    // Extract the body text from the source — we replace the call site with
    // the trimmed body lines. The body is the source between the `:` that
    // introduces the function and the start of the next item.
    let lines: Vec<&str> = doc.text.split('\n').collect();
    let body_text = extract_body_text(&lines, target);
    if body_text.trim().is_empty() {
        return RefactorOutcome::Rejected(format!("function `{name}` has no body to inline"));
    }

    // Locate the call site under the cursor. We replace it with the body.
    let Some(call_range) = call_site_range(doc.text, args.position, &name) else {
        return RefactorOutcome::Rejected("no call site at position".to_string());
    };

    // Convert the multi-line body into a one-line replacement when the
    // call sits on a single line, otherwise preserve newlines and indent
    // the continuation lines.
    let call_line = call_range.start.line as usize;
    let call_indent_len = leading_space_len(lines.get(call_line).copied().unwrap_or(""));
    let call_indent: String = " ".repeat(call_indent_len);
    let body_lines: Vec<&str> = body_text.split('\n').collect();
    let mut replacement = String::new();
    for (i, raw) in body_lines.iter().enumerate() {
        let trimmed = raw.trim_end();
        if trimmed.is_empty() {
            if i + 1 < body_lines.len() {
                // Preserve blank separator lines between statements.
                if i > 0 {
                    replacement.push('\n');
                }
            }
            continue;
        }
        // Strip the body's own leading indent (typically 2 spaces) and
        // re-indent at the call site's level.
        let stripped = trimmed.trim_start();
        if i > 0 {
            replacement.push('\n');
            replacement.push_str(&call_indent);
        }
        replacement.push_str(stripped);
    }

    let edit = TextEdit {
        range: call_range,
        new_text: replacement,
    };
    let mut changes: BTreeMap<String, Vec<TextEdit>> = BTreeMap::new();
    changes.insert(doc.uri.to_string(), vec![edit]);
    RefactorOutcome::Edits(WorkspaceEdit { changes })
}

// ---------------------------------------------------------------------------
// Helpers — symbol mapping
// ---------------------------------------------------------------------------

fn symbol_to_item(uri: &str, symbol: &Symbol) -> CallHierarchyItem {
    let range = span_to_range(&symbol.span);
    CallHierarchyItem {
        name: symbol.name.clone(),
        kind: LspSymbolKind::Function,
        uri: uri.to_string(),
        range,
        selection_range: range,
        detail: Some(symbol.id.clone()),
    }
}

fn type_symbol_to_item(uri: &str, symbol: &Symbol) -> TypeHierarchyItem {
    let range = span_to_range(&symbol.span);
    TypeHierarchyItem {
        name: symbol.name.clone(),
        kind: LspSymbolKind::Class,
        uri: uri.to_string(),
        range,
        selection_range: range,
        detail: Some(symbol.id.clone()),
        data: Some(serde_json::json!({"kind": "type", "owner": symbol.name})),
    }
}

fn item_sort_key(item: &CallHierarchyItem) -> (String, u32, u32) {
    (
        item.uri.clone(),
        item.selection_range.start.line,
        item.selection_range.start.character,
    )
}

fn type_item_sort_key(a: &TypeHierarchyItem, b: &TypeHierarchyItem) -> std::cmp::Ordering {
    a.uri
        .cmp(&b.uri)
        .then_with(|| {
            (
                a.selection_range.start.line,
                a.selection_range.start.character,
            )
                .cmp(&(
                    b.selection_range.start.line,
                    b.selection_range.start.character,
                ))
        })
        .then_with(|| a.name.cmp(&b.name))
}

// ---------------------------------------------------------------------------
// Helpers — call counting
// ---------------------------------------------------------------------------

fn count_calls_to(expr: &Expr, target: &str, counter: &mut usize) {
    match expr {
        Expr::Call { callee, args } => {
            if let Expr::Var(name) = callee.as_ref() {
                if name == target {
                    *counter += 1;
                }
            }
            count_calls_to(callee, target, counter);
            for arg in args {
                count_calls_to(arg, target, counter);
            }
        }
        Expr::Field { base, .. } => count_calls_to(base, target, counter),
        Expr::Block { stmts, tail } => {
            for stmt in stmts {
                match stmt {
                    Stmt::Let { init, .. } => count_calls_to(init, target, counter),
                    Stmt::Expr(e) => count_calls_to(e, target, counter),
                    Stmt::Return(Some(e)) => count_calls_to(e, target, counter),
                    Stmt::Return(None) => {}
                }
            }
            if let Some(e) = tail {
                count_calls_to(e, target, counter);
            }
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            count_calls_to(cond, target, counter);
            count_calls_to(then_branch, target, counter);
            if let Some(e) = else_branch {
                count_calls_to(e, target, counter);
            }
        }
        Expr::Match { scrutinee, arms } => {
            count_calls_to(scrutinee, target, counter);
            for MatchArm { body, .. } in arms {
                count_calls_to(body, target, counter);
            }
        }
        Expr::Return(Some(e)) => count_calls_to(e, target, counter),
        Expr::Construct { args, .. } => {
            for a in args {
                count_calls_to(a, target, counter);
            }
        }
        Expr::Try(e) => count_calls_to(e, target, counter),
        Expr::Tuple(items) => {
            for i in items {
                count_calls_to(i, target, counter);
            }
        }
        Expr::Record { fields } => {
            for (_, e) in fields {
                count_calls_to(e, target, counter);
            }
        }
        Expr::Lambda { body, .. } => count_calls_to(body, target, counter),
        Expr::Binary { lhs, rhs, .. } => {
            count_calls_to(lhs, target, counter);
            count_calls_to(rhs, target, counter);
        }
        Expr::Unary { operand, .. } => count_calls_to(operand, target, counter),
        Expr::InterpString { parts } => {
            for part in parts {
                if let InterpPart::Expr(inner) = part {
                    count_calls_to(inner, target, counter);
                }
            }
        }
        Expr::Lit(_) | Expr::Var(_) | Expr::Return(None) | Expr::RawStr { .. } | Expr::Error => {}
    }
}

fn collect_call_names(expr: &Expr, out: &mut BTreeMap<String, usize>) {
    match expr {
        Expr::Call { callee, args } => {
            if let Expr::Var(name) = callee.as_ref() {
                *out.entry(name.clone()).or_insert(0) += 1;
            }
            collect_call_names(callee, out);
            for arg in args {
                collect_call_names(arg, out);
            }
        }
        Expr::Field { base, .. } => collect_call_names(base, out),
        Expr::Block { stmts, tail } => {
            for stmt in stmts {
                match stmt {
                    Stmt::Let { init, .. } => collect_call_names(init, out),
                    Stmt::Expr(e) => collect_call_names(e, out),
                    Stmt::Return(Some(e)) => collect_call_names(e, out),
                    Stmt::Return(None) => {}
                }
            }
            if let Some(e) = tail {
                collect_call_names(e, out);
            }
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_call_names(cond, out);
            collect_call_names(then_branch, out);
            if let Some(e) = else_branch {
                collect_call_names(e, out);
            }
        }
        Expr::Match { scrutinee, arms } => {
            collect_call_names(scrutinee, out);
            for MatchArm { body, .. } in arms {
                collect_call_names(body, out);
            }
        }
        Expr::Return(Some(e)) => collect_call_names(e, out),
        Expr::Construct { args, .. } => {
            for a in args {
                collect_call_names(a, out);
            }
        }
        Expr::Try(e) => collect_call_names(e, out),
        Expr::Tuple(items) => {
            for i in items {
                collect_call_names(i, out);
            }
        }
        Expr::Record { fields } => {
            for (_, e) in fields {
                collect_call_names(e, out);
            }
        }
        Expr::Lambda { body, .. } => collect_call_names(body, out),
        Expr::Binary { lhs, rhs, .. } => {
            collect_call_names(lhs, out);
            collect_call_names(rhs, out);
        }
        Expr::Unary { operand, .. } => collect_call_names(operand, out),
        Expr::InterpString { parts } => {
            for part in parts {
                if let InterpPart::Expr(inner) = part {
                    collect_call_names(inner, out);
                }
            }
        }
        Expr::Lit(_) | Expr::Var(_) | Expr::Return(None) | Expr::RawStr { .. } | Expr::Error => {}
    }
}

// ---------------------------------------------------------------------------
// Helpers — variant extraction
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct VariantHit {
    name: String,
    line: u32,
    character: u32,
}

impl VariantHit {
    fn to_item(&self, uri: &str, owner: &str) -> TypeHierarchyItem {
        let range = Range {
            start: Position {
                line: self.line,
                character: self.character,
            },
            end: Position {
                line: self.line,
                character: self
                    .character
                    .saturating_add(self.name.chars().count() as u32),
            },
        };
        TypeHierarchyItem {
            name: self.name.clone(),
            kind: LspSymbolKind::Class,
            uri: uri.to_string(),
            range,
            selection_range: range,
            detail: Some(format!("variant of {owner}")),
            data: Some(serde_json::json!({"kind": "variant", "owner": owner})),
        }
    }
}

/// Scan the source after the type declaration looking for `|` lines and
/// extract the variant constructor names. The walk stops at the first
/// line whose indentation matches or is shallower than the declaration's
/// own indent (i.e. the next top-level item).
fn find_variants(text: &str, symbol: &Symbol) -> Vec<VariantHit> {
    let mut out: Vec<VariantHit> = Vec::new();
    let lines: Vec<&str> = text.split('\n').collect();
    // 1-based → 0-based.
    let start_line = symbol.span.start.line.saturating_sub(1);
    if start_line >= lines.len() {
        return out;
    }
    let header_indent = leading_space_len(lines[start_line]);
    // Walk forward from the line after the header.
    for (offset, raw) in lines.iter().enumerate().skip(start_line + 1) {
        let trimmed = raw.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        let indent = leading_space_len(raw);
        if indent <= header_indent && !trimmed.trim_start().starts_with('|') {
            break;
        }
        let stripped = trimmed.trim_start();
        if let Some(rest) = stripped.strip_prefix('|') {
            let name = rest.trim_start();
            // Capture only the bare constructor name, up to `(` or whitespace.
            let mut end = 0usize;
            for (i, ch) in name.char_indices() {
                if !(ch.is_ascii_alphanumeric() || ch == '_') {
                    end = i;
                    break;
                }
                end = i + ch.len_utf8();
            }
            if end == 0 {
                continue;
            }
            let ident = &name[..end];
            if ident.is_empty() {
                continue;
            }
            // Column of the constructor name within the line.
            let pipe_col = indent;
            // skip `|` and any spaces after it
            let after_pipe = trimmed
                .chars()
                .skip(pipe_col + 1)
                .take_while(|c| *c == ' ')
                .count();
            let name_col = pipe_col + 1 + after_pipe;
            out.push(VariantHit {
                name: ident.to_string(),
                line: offset as u32,
                character: name_col as u32,
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Helpers — refactor mechanics
// ---------------------------------------------------------------------------

fn is_valid_identifier(candidate: &str) -> bool {
    let mut chars = candidate.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn leading_space_len(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ').count()
}

fn strip_common_indent(line: &str, base: usize) -> &str {
    let prefix = leading_space_len(line);
    let drop = prefix.min(base + 2);
    &line[drop..]
}

fn span_contains(span: &ori_compiler::source::Span, line: usize, column: usize) -> bool {
    if line < span.start.line || line > span.end.line {
        return false;
    }
    if line == span.start.line && column < span.start.column {
        return false;
    }
    if line == span.end.line && column > span.end.column {
        return false;
    }
    true
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn identifier_at(text: &str, position: Position) -> Option<String> {
    let line = text.lines().nth(position.line as usize)?;
    let column = position.character as usize;
    let bytes = line.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let len = bytes.len();
    let probe = column.min(len.saturating_sub(1));
    if !is_ident_byte(bytes[probe]) && (column == 0 || !is_ident_byte(bytes[column - 1])) {
        return None;
    }
    let anchor = probe.min(len - 1);
    let mut start = anchor;
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = anchor;
    while end < len && is_ident_byte(bytes[end]) {
        end += 1;
    }
    if start == end {
        return None;
    }
    Some(line[start..end].to_string())
}

/// Find a function symbol whose body encloses both `start` and `end`. We
/// approximate the function body as `[span.start.line+1 .. next_item_line)`
/// because the bootstrap parser only stores the header span on the symbol.
fn find_enclosing_function(module: &Module, start: Position, end: Position) -> Option<Symbol> {
    let funcs: Vec<&Symbol> = module
        .symbols
        .iter()
        .filter(|s| s.kind == CompilerSymbolKind::Function)
        .collect();
    let lines = function_boundaries(&funcs);
    for (idx, symbol) in funcs.iter().enumerate() {
        let header_line = symbol.span.start.line.saturating_sub(1) as u32;
        let body_end_line = lines[idx] as u32;
        if start.line > header_line && end.line <= body_end_line {
            return Some((*symbol).clone());
        }
    }
    None
}

fn function_boundaries(funcs: &[&Symbol]) -> Vec<usize> {
    // For each function index, return the 0-based line of the *last* line
    // that still belongs to its body. We approximate this as one line before
    // the next function's header, or `usize::MAX` for the final entry.
    let mut out = Vec::with_capacity(funcs.len());
    for i in 0..funcs.len() {
        if let Some(next) = funcs.get(i + 1) {
            out.push(next.span.start.line.saturating_sub(2));
        } else {
            out.push(usize::MAX);
        }
    }
    out
}

fn function_body_end_line(lines: &[&str], host: &Symbol) -> usize {
    // Walk from the header line forward until we hit a line whose indent is
    // less than or equal to the header indent and is non-blank.
    let header_idx = host.span.start.line.saturating_sub(1);
    if header_idx >= lines.len() {
        return lines.len().saturating_sub(1);
    }
    let header_indent = leading_space_len(lines[header_idx]);
    let mut last_body = header_idx;
    for (i, line) in lines.iter().enumerate().skip(header_idx + 1) {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        let indent = leading_space_len(line);
        if indent <= header_indent {
            break;
        }
        last_body = i;
    }
    last_body
}

fn host_body_indent(lines: &[&str], host: &Symbol) -> usize {
    let header_idx = host.span.start.line.saturating_sub(1);
    if header_idx >= lines.len() {
        return 0;
    }
    let header_indent = leading_space_len(lines[header_idx]);
    // The body conventionally indents 2 spaces past the header.
    for line in lines.iter().skip(header_idx + 1) {
        if line.trim().is_empty() {
            continue;
        }
        let indent = leading_space_len(line);
        if indent > header_indent {
            return indent;
        }
        break;
    }
    header_indent + 2
}

fn extract_body_text(lines: &[&str], target: &Symbol) -> String {
    let header_idx = target.span.start.line.saturating_sub(1);
    if header_idx >= lines.len() {
        return String::new();
    }
    let body_end = function_body_end_line(lines, target);
    if body_end <= header_idx {
        return String::new();
    }
    lines[(header_idx + 1)..=body_end].join("\n")
}

fn call_site_range(text: &str, position: Position, name: &str) -> Option<Range> {
    let lines: Vec<&str> = text.split('\n').collect();
    let line_idx = position.line as usize;
    let line = lines.get(line_idx)?;
    // Locate the identifier under or around the cursor.
    let bytes = line.as_bytes();
    let column = position.character as usize;
    let probe = column.min(bytes.len().saturating_sub(1));
    if bytes.is_empty() {
        return None;
    }
    let probe = if is_ident_byte(bytes[probe]) {
        probe
    } else if column > 0 && is_ident_byte(bytes[column - 1]) {
        column - 1
    } else {
        return None;
    };
    let mut start = probe;
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = probe;
    while end < bytes.len() && is_ident_byte(bytes[end]) {
        end += 1;
    }
    let ident = &line[start..end];
    if ident != name {
        return None;
    }
    // Find the closing `)` so the replacement covers `name(...)`.
    let mut cursor = end;
    while cursor < bytes.len() && (bytes[cursor] == b' ' || bytes[cursor] == b'\t') {
        cursor += 1;
    }
    if cursor >= bytes.len() || bytes[cursor] != b'(' {
        return None;
    }
    let mut depth: i32 = 0;
    let mut close = cursor;
    while close < bytes.len() {
        match bytes[close] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        close += 1;
    }
    if close >= bytes.len() {
        return None;
    }
    Some(Range {
        start: Position {
            line: line_idx as u32,
            character: start as u32,
        },
        end: Position {
            line: line_idx as u32,
            character: (close + 1) as u32,
        },
    })
}
