//! LSP server event loop.
//!
//! `Server::run` reads framed JSON-RPC messages from a `Read` source until
//! the client sends `shutdown` followed by `exit`. The server owns a
//! [`WorkspaceState`] and dispatches the LSP messages listed in the crate
//! documentation. Errors are surfaced as JSON-RPC error responses where the
//! protocol requires a reply; transport-level errors propagate out of
//! `run` so the caller can decide whether to retry.

use std::io::{self, BufReader, Read, Write};

use ori_compiler::ast::{Symbol, SymbolKind as CompilerSymbolKind};
use ori_compiler::body::parse_module_bodies_with_module;
use ori_compiler::compiler::{CompileResult, Compiler};
use ori_compiler::expr::{Expr, MatchArm, Stmt};
use ori_compiler::lexer::{lex_with_diagnostics, TokenKind};
use ori_compiler::source::SourceFile;
use ori_compiler::type_infer::{infer_expr, TypeEnv};
use serde::Serialize;
use serde_json::Value;

use crate::codec::{read_message, write_message};
use crate::diagnostics::{span_to_range, to_lsp_diagnostics};
use crate::protocol::{
    error_codes, error_response, notification, null_response, success_response, CodeAction,
    CodeActionParams, CodeLens, CodeLensParams, Command, CompletionItem, CompletionItemKind,
    CompletionList, CompletionParams, DefinitionParams, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DocumentFormattingParams,
    DocumentSymbolParams, FoldingRange, FoldingRangeParams, Hover, HoverParams, InitializeParams,
    InitializeResult, InlayHint, InlayHintParams, Location, MarkupContent, NotificationMessage,
    Position, PublishDiagnosticsParams, Range, ReferenceParams, RenameParams, RequestId,
    RequestMessage, SelectionRange, SelectionRangeParams, SemanticTokens, SemanticTokensParams,
    ServerCapabilities, ServerInfo, SymbolInformation, SymbolKind as LspSymbolKind, TextEdit,
    WorkspaceEdit, WorkspaceSymbolParams,
};
use crate::state::WorkspaceState;

/// Maximum number of symbols returned from `workspace/symbol`, per the
/// senior-review quality gate. Matches the guidance in TASKS.md.
const WORKSPACE_SYMBOL_LIMIT: usize = 100;

/// Orison keywords surfaced as completion candidates. Kept alphabetised so
/// the completion-list ordering is stable across invocations.
const ORISON_KEYWORDS: &[&str] = &[
    "actor",
    "capability",
    "case",
    "else",
    "false",
    "fn",
    "for",
    "if",
    "import",
    "let",
    "match",
    "migration",
    "module",
    "none",
    "query",
    "return",
    "service",
    "type",
    "view",
    "while",
];

/// Orison LSP server.
#[derive(Debug, Default)]
pub struct Server {
    state: WorkspaceState,
    /// `true` after the client has issued `shutdown`. A subsequent `exit`
    /// notification terminates the run loop cleanly; any other request
    /// received in this state is answered with `InvalidRequest`.
    shutting_down: bool,
}

impl Server {
    /// Construct a fresh server with an empty workspace.
    pub fn new() -> Self {
        Self::default()
    }

    /// Drive the server with the supplied transports until `exit` is
    /// received or the input stream is exhausted.
    pub fn run<R: Read, W: Write>(mut self, reader: R, writer: W) -> io::Result<()> {
        let mut reader = BufReader::new(reader);
        let mut writer = writer;
        loop {
            let payload = match read_message(&mut reader)? {
                Some(bytes) => bytes,
                None => return Ok(()),
            };

            // Decide whether this is a request, a notification, or invalid
            // JSON. We use `serde_json::Value` for routing and then deserialize
            // the typed envelope on the matching branch so a single bad
            // message cannot corrupt the loop state.
            let value: Value = match serde_json::from_slice(&payload) {
                Ok(value) => value,
                Err(_) => {
                    let body =
                        error_response(&Value::Null, error_codes::PARSE_ERROR, "invalid json")
                            .map_err(io_other)?;
                    write_message(&mut writer, &body)?;
                    continue;
                }
            };

            let has_id = value.get("id").is_some_and(|id| !id.is_null());

            if has_id {
                let request: RequestMessage = match serde_json::from_value(value.clone()) {
                    Ok(req) => req,
                    Err(err) => {
                        let id = value.get("id").cloned().unwrap_or(Value::Null);
                        let body = error_response(
                            &id,
                            error_codes::INVALID_REQUEST,
                            &format!("invalid request envelope: {err}"),
                        )
                        .map_err(io_other)?;
                        write_message(&mut writer, &body)?;
                        continue;
                    }
                };
                if self.handle_request(&mut writer, request)? {
                    return Ok(());
                }
            } else {
                let notification: NotificationMessage = match serde_json::from_value(value) {
                    Ok(notif) => notif,
                    Err(_) => {
                        // Notifications never get a reply. Drop malformed ones.
                        continue;
                    }
                };
                if self.handle_notification(&mut writer, notification)? {
                    return Ok(());
                }
            }
        }
    }

    /// Returns `Ok(true)` when the server should exit.
    fn handle_request<W: Write>(
        &mut self,
        writer: &mut W,
        request: RequestMessage,
    ) -> io::Result<bool> {
        if self.shutting_down && request.method != "shutdown" && request.method != "exit" {
            let body = error_response(
                &request.id,
                error_codes::INVALID_REQUEST,
                "server has been shut down",
            )
            .map_err(io_other)?;
            write_message(writer, &body)?;
            return Ok(false);
        }

        match request.method.as_str() {
            "initialize" => self.on_initialize(writer, &request)?,
            "shutdown" => self.on_shutdown(writer, &request.id)?,
            "textDocument/hover" => self.on_hover(writer, &request)?,
            "textDocument/codeAction" => self.on_code_action(writer, &request)?,
            "textDocument/completion" => self.on_completion(writer, &request)?,
            "textDocument/rename" => self.on_rename(writer, &request)?,
            "workspace/symbol" => self.on_workspace_symbol(writer, &request)?,
            "textDocument/documentSymbol" => self.on_document_symbol(writer, &request)?,
            "textDocument/definition" => self.on_definition(writer, &request)?,
            "textDocument/references" => self.on_references(writer, &request)?,
            "textDocument/semanticTokens/full" => self.on_semantic_tokens_full(writer, &request)?,
            "textDocument/formatting" => self.on_formatting(writer, &request)?,
            "textDocument/codeLens" => self.on_code_lens(writer, &request)?,
            "codeLens/resolve" => self.on_code_lens_resolve(writer, &request)?,
            "textDocument/inlayHint" => self.on_inlay_hint(writer, &request)?,
            "textDocument/foldingRange" => self.on_folding_range(writer, &request)?,
            "textDocument/selectionRange" => self.on_selection_range(writer, &request)?,
            other => {
                let body = error_response(
                    &request.id,
                    error_codes::METHOD_NOT_FOUND,
                    &format!("method not implemented: {other}"),
                )
                .map_err(io_other)?;
                write_message(writer, &body)?;
            }
        }
        Ok(false)
    }

    /// Returns `Ok(true)` when the server should exit.
    fn handle_notification<W: Write>(
        &mut self,
        writer: &mut W,
        notification: NotificationMessage,
    ) -> io::Result<bool> {
        match notification.method.as_str() {
            "initialized" => Ok(false),
            "exit" => Ok(true),
            "textDocument/didOpen" => {
                self.on_did_open(writer, notification.params)?;
                Ok(false)
            }
            "textDocument/didChange" => {
                self.on_did_change(writer, notification.params)?;
                Ok(false)
            }
            "textDocument/didClose" => {
                self.on_did_close(writer, notification.params)?;
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    // ---------- request handlers ----------

    fn on_initialize<W: Write>(
        &mut self,
        writer: &mut W,
        request: &RequestMessage,
    ) -> io::Result<()> {
        // Decode params for validation; we currently do not key off any
        // client capability so the value is intentionally unused.
        if let Some(params) = request.params.clone() {
            let _: InitializeParams =
                serde_json::from_value(params).unwrap_or(InitializeParams::default());
        }
        let result = InitializeResult {
            capabilities: ServerCapabilities::default(),
            server_info: ServerInfo {
                name: "ori-lsp".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        };
        write_typed_response(writer, &request.id, &result)
    }

    fn on_shutdown<W: Write>(&mut self, writer: &mut W, id: &RequestId) -> io::Result<()> {
        self.shutting_down = true;
        let body = null_response(id).map_err(io_other)?;
        write_message(writer, &body)
    }

    fn on_hover<W: Write>(&mut self, writer: &mut W, request: &RequestMessage) -> io::Result<()> {
        let Some(params_value) = request.params.clone() else {
            return write_invalid_params(writer, &request.id, "hover requires params");
        };
        let params: HoverParams = match serde_json::from_value(params_value) {
            Ok(params) => params,
            Err(err) => {
                return write_invalid_params(
                    writer,
                    &request.id,
                    &format!("invalid hover params: {err}"),
                );
            }
        };

        let Some(document) = self.state.get(&params.text_document.uri).cloned() else {
            let body = null_response(&request.id).map_err(io_other)?;
            return write_message(writer, &body);
        };

        let result = Compiler::check_source(SourceFile::new(&document.uri, &document.text));
        let Some(symbol) = symbol_at(&result, params.position) else {
            let body = null_response(&request.id).map_err(io_other)?;
            return write_message(writer, &body);
        };

        let markdown = render_symbol_markdown(symbol);
        let hover = Hover {
            contents: MarkupContent {
                kind: "markdown".to_string(),
                value: markdown,
            },
            range: Some(crate::diagnostics::span_to_range(&symbol.span)),
        };
        write_typed_response(writer, &request.id, &hover)
    }

    fn on_code_action<W: Write>(
        &mut self,
        writer: &mut W,
        request: &RequestMessage,
    ) -> io::Result<()> {
        let Some(params_value) = request.params.clone() else {
            return write_invalid_params(writer, &request.id, "codeAction requires params");
        };
        let params: CodeActionParams = match serde_json::from_value(params_value) {
            Ok(params) => params,
            Err(err) => {
                return write_invalid_params(
                    writer,
                    &request.id,
                    &format!("invalid codeAction params: {err}"),
                );
            }
        };

        let mut actions: Vec<CodeAction> = Vec::new();
        for diagnostic in &params.context.diagnostics {
            let Some(data) = diagnostic.data.clone() else {
                continue;
            };
            let Some(fixes) = data.get("fixes").and_then(Value::as_array) else {
                continue;
            };
            for (idx, fix) in fixes.iter().enumerate() {
                let description = fix
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("Apply suggested fix")
                    .to_string();
                let kind = fix
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("patch")
                    .to_string();
                let patch_ref = patch_ir_ref(&diagnostic.code, idx);
                let action = CodeAction {
                    title: format!("{description} ({kind})"),
                    kind: "quickfix".to_string(),
                    diagnostics: vec![diagnostic.clone()],
                    data: Some(serde_json::json!({
                        "schema": "ori.lsp.code_action.v1",
                        "patchRef": patch_ref,
                        "diagnosticCode": diagnostic.code,
                        "fix": fix,
                    })),
                };
                actions.push(action);
            }
        }
        write_typed_response(writer, &request.id, &actions)
    }

    fn on_completion<W: Write>(
        &mut self,
        writer: &mut W,
        request: &RequestMessage,
    ) -> io::Result<()> {
        let Some(params_value) = request.params.clone() else {
            return write_invalid_params(writer, &request.id, "completion requires params");
        };
        let params: CompletionParams = match serde_json::from_value(params_value) {
            Ok(params) => params,
            Err(err) => {
                return write_invalid_params(
                    writer,
                    &request.id,
                    &format!("invalid completion params: {err}"),
                );
            }
        };

        let mut items: Vec<CompletionItem> = Vec::new();
        if let Some(document) = self.state.get(&params.text_document.uri).cloned() {
            let result = Compiler::check_source(SourceFile::new(&document.uri, &document.text));
            for symbol in result.module.exported_symbols() {
                items.push(CompletionItem {
                    label: symbol.name.clone(),
                    kind: Some(symbol_to_completion_kind(symbol.kind)),
                    detail: Some(symbol.signature.clone()),
                    documentation: Some(MarkupContent {
                        kind: "markdown".to_string(),
                        value: render_symbol_markdown(symbol),
                    }),
                });
            }
        }
        for keyword in ORISON_KEYWORDS {
            items.push(CompletionItem {
                label: (*keyword).to_string(),
                kind: Some(CompletionItemKind::Keyword),
                detail: Some("Orison keyword".to_string()),
                documentation: None,
            });
        }
        items.sort_by(|a, b| a.label.cmp(&b.label));
        items.dedup_by(|a, b| a.label == b.label);

        let list = CompletionList {
            is_incomplete: false,
            items,
        };
        write_typed_response(writer, &request.id, &list)
    }

    fn on_rename<W: Write>(&mut self, writer: &mut W, request: &RequestMessage) -> io::Result<()> {
        let Some(params_value) = request.params.clone() else {
            return write_invalid_params(writer, &request.id, "rename requires params");
        };
        let params: RenameParams = match serde_json::from_value(params_value) {
            Ok(params) => params,
            Err(err) => {
                return write_invalid_params(
                    writer,
                    &request.id,
                    &format!("invalid rename params: {err}"),
                );
            }
        };

        let new_name = params.new_name.trim();
        if new_name.is_empty() || !is_valid_identifier(new_name) {
            return write_invalid_params(
                writer,
                &request.id,
                "rename newName must be a non-empty identifier",
            );
        }

        let Some(document) = self.state.get(&params.text_document.uri).cloned() else {
            let body = null_response(&request.id).map_err(io_other)?;
            return write_message(writer, &body);
        };

        let Some(old_name) = identifier_at(&document.text, params.position) else {
            let body = null_response(&request.id).map_err(io_other)?;
            return write_message(writer, &body);
        };

        if old_name == new_name {
            let edit = WorkspaceEdit::default();
            return write_typed_response(writer, &request.id, &edit);
        }

        let edits = identifier_edits(&document.text, &old_name, new_name);
        let mut changes: std::collections::BTreeMap<String, Vec<TextEdit>> =
            std::collections::BTreeMap::new();
        changes.insert(document.uri.clone(), edits);
        let edit = WorkspaceEdit { changes };
        write_typed_response(writer, &request.id, &edit)
    }

    fn on_workspace_symbol<W: Write>(
        &mut self,
        writer: &mut W,
        request: &RequestMessage,
    ) -> io::Result<()> {
        // Per the LSP spec, the `query` field is required but an empty string
        // means "return everything". We honour that contract and only cap the
        // response size.
        let params: WorkspaceSymbolParams = match request.params.clone() {
            Some(value) => serde_json::from_value(value).unwrap_or_default(),
            None => WorkspaceSymbolParams::default(),
        };
        let needle = params.query.to_ascii_lowercase();

        let mut symbols: Vec<SymbolInformation> = Vec::new();
        for document in self.state.iter() {
            let result = Compiler::check_source(SourceFile::new(&document.uri, &document.text));
            for symbol in &result.module.symbols {
                if symbol.kind == CompilerSymbolKind::Module {
                    continue;
                }
                if !needle.is_empty() && !symbol.name.to_ascii_lowercase().contains(&needle) {
                    continue;
                }
                symbols.push(compiler_symbol_to_information(&document.uri, symbol));
            }
        }
        // Stable ordering so test assertions are deterministic across hash-map
        // iteration orders.
        symbols.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then_with(|| a.location.uri.cmp(&b.location.uri))
        });
        symbols.truncate(WORKSPACE_SYMBOL_LIMIT);
        write_typed_response(writer, &request.id, &symbols)
    }

    fn on_document_symbol<W: Write>(
        &mut self,
        writer: &mut W,
        request: &RequestMessage,
    ) -> io::Result<()> {
        let Some(params_value) = request.params.clone() else {
            return write_invalid_params(writer, &request.id, "documentSymbol requires params");
        };
        let params: DocumentSymbolParams = match serde_json::from_value(params_value) {
            Ok(params) => params,
            Err(err) => {
                return write_invalid_params(
                    writer,
                    &request.id,
                    &format!("invalid documentSymbol params: {err}"),
                );
            }
        };

        let Some(document) = self.state.get(&params.text_document.uri).cloned() else {
            // Returning an empty list keeps the editor's outline view clean
            // rather than producing a null that some clients treat as an error.
            let empty: Vec<SymbolInformation> = Vec::new();
            return write_typed_response(writer, &request.id, &empty);
        };

        let result = Compiler::check_source(SourceFile::new(&document.uri, &document.text));
        let mut symbols: Vec<SymbolInformation> = result
            .module
            .symbols
            .iter()
            .filter(|symbol| symbol.kind != CompilerSymbolKind::Module)
            .map(|symbol| compiler_symbol_to_information(&document.uri, symbol))
            .collect();
        symbols.sort_by(|a, b| {
            (
                a.location.range.start.line,
                a.location.range.start.character,
            )
                .cmp(&(
                    b.location.range.start.line,
                    b.location.range.start.character,
                ))
        });
        write_typed_response(writer, &request.id, &symbols)
    }

    fn on_definition<W: Write>(
        &mut self,
        writer: &mut W,
        request: &RequestMessage,
    ) -> io::Result<()> {
        let Some(params_value) = request.params.clone() else {
            return write_invalid_params(writer, &request.id, "definition requires params");
        };
        let params: DefinitionParams = match serde_json::from_value(params_value) {
            Ok(params) => params,
            Err(err) => {
                return write_invalid_params(
                    writer,
                    &request.id,
                    &format!("invalid definition params: {err}"),
                );
            }
        };

        let Some(document) = self.state.get(&params.text_document.uri).cloned() else {
            let body = null_response(&request.id).map_err(io_other)?;
            return write_message(writer, &body);
        };
        let Some(needle) = identifier_at(&document.text, params.position) else {
            let body = null_response(&request.id).map_err(io_other)?;
            return write_message(writer, &body);
        };

        // Walk every document, preferring an exact name match. The first hit
        // wins — that matches the spec's "return one or more Locations" with
        // the simplest deterministic ordering. Documents are visited in URI
        // order so the result does not depend on hash-map iteration order.
        let mut docs: Vec<_> = self.state.iter().collect();
        docs.sort_by(|a, b| a.uri.cmp(&b.uri));
        for doc in docs {
            let result = Compiler::check_source(SourceFile::new(&doc.uri, &doc.text));
            if let Some(symbol) = result
                .module
                .symbols
                .iter()
                .filter(|s| s.kind != CompilerSymbolKind::Module)
                .find(|s| s.name == needle)
            {
                let location = Location {
                    uri: doc.uri.clone(),
                    range: span_to_range(&symbol.span),
                };
                return write_typed_response(writer, &request.id, &location);
            }
        }

        let body = null_response(&request.id).map_err(io_other)?;
        write_message(writer, &body)
    }

    fn on_references<W: Write>(
        &mut self,
        writer: &mut W,
        request: &RequestMessage,
    ) -> io::Result<()> {
        let Some(params_value) = request.params.clone() else {
            return write_invalid_params(writer, &request.id, "references requires params");
        };
        let params: ReferenceParams = match serde_json::from_value(params_value) {
            Ok(params) => params,
            Err(err) => {
                return write_invalid_params(
                    writer,
                    &request.id,
                    &format!("invalid references params: {err}"),
                );
            }
        };

        let Some(document) = self.state.get(&params.text_document.uri).cloned() else {
            let empty: Vec<Location> = Vec::new();
            return write_typed_response(writer, &request.id, &empty);
        };
        let Some(needle) = identifier_at(&document.text, params.position) else {
            let empty: Vec<Location> = Vec::new();
            return write_typed_response(writer, &request.id, &empty);
        };

        let mut docs: Vec<_> = self.state.iter().collect();
        docs.sort_by(|a, b| a.uri.cmp(&b.uri));

        let mut locations: Vec<Location> = Vec::new();
        for doc in docs {
            let ranges = identifier_occurrences(&doc.text, &needle);
            for range in ranges {
                locations.push(Location {
                    uri: doc.uri.clone(),
                    range,
                });
            }
        }

        // Deduplicate per (uri, range). Two compiler passes or callers asking
        // the same query repeatedly must never see the same location twice.
        locations.sort_by(|a, b| {
            a.uri.cmp(&b.uri).then_with(|| {
                (
                    a.range.start.line,
                    a.range.start.character,
                    a.range.end.line,
                    a.range.end.character,
                )
                    .cmp(&(
                        b.range.start.line,
                        b.range.start.character,
                        b.range.end.line,
                        b.range.end.character,
                    ))
            })
        });
        locations.dedup_by(|a, b| {
            a.uri == b.uri
                && a.range.start.line == b.range.start.line
                && a.range.start.character == b.range.start.character
                && a.range.end.line == b.range.end.line
                && a.range.end.character == b.range.end.character
        });

        // `includeDeclaration = false` strips the symbol's own declaration
        // span from the reply. The compiler reports the declaration span as
        // `(keyword..end_of_name)` so a textual identifier occurrence of the
        // *name* sits inside but does not share its start position — match by
        // containment rather than equality.
        if !params.context.include_declaration {
            let declaration_ranges: Vec<(String, Range)> = self
                .state
                .iter()
                .flat_map(|doc| {
                    let result = Compiler::check_source(SourceFile::new(&doc.uri, &doc.text));
                    result
                        .module
                        .symbols
                        .iter()
                        .filter(|s| s.kind != CompilerSymbolKind::Module && s.name == needle)
                        .map(|s| (doc.uri.clone(), span_to_range(&s.span)))
                        .collect::<Vec<_>>()
                })
                .collect();
            locations.retain(|loc| {
                !declaration_ranges
                    .iter()
                    .any(|(uri, range)| uri == &loc.uri && range_contains(range, &loc.range))
            });
        }

        write_typed_response(writer, &request.id, &locations)
    }

    fn on_semantic_tokens_full<W: Write>(
        &mut self,
        writer: &mut W,
        request: &RequestMessage,
    ) -> io::Result<()> {
        let Some(params_value) = request.params.clone() else {
            return write_invalid_params(writer, &request.id, "semanticTokens requires params");
        };
        let params: SemanticTokensParams = match serde_json::from_value(params_value) {
            Ok(params) => params,
            Err(err) => {
                return write_invalid_params(
                    writer,
                    &request.id,
                    &format!("invalid semanticTokens params: {err}"),
                );
            }
        };
        let Some(document) = self.state.get(&params.text_document.uri).cloned() else {
            let empty = SemanticTokens::default();
            return write_typed_response(writer, &request.id, &empty);
        };
        let tokens = compute_semantic_tokens(&document.uri, &document.text);
        write_typed_response(writer, &request.id, &tokens)
    }

    fn on_formatting<W: Write>(
        &mut self,
        writer: &mut W,
        request: &RequestMessage,
    ) -> io::Result<()> {
        let Some(params_value) = request.params.clone() else {
            return write_invalid_params(writer, &request.id, "formatting requires params");
        };
        let params: DocumentFormattingParams = match serde_json::from_value(params_value) {
            Ok(params) => params,
            Err(err) => {
                return write_invalid_params(
                    writer,
                    &request.id,
                    &format!("invalid formatting params: {err}"),
                );
            }
        };
        let Some(document) = self.state.get(&params.text_document.uri).cloned() else {
            let empty: Vec<TextEdit> = Vec::new();
            return write_typed_response(writer, &request.id, &empty);
        };
        let formatted = Compiler::format_source(&document.text);
        if formatted == document.text {
            let empty: Vec<TextEdit> = Vec::new();
            return write_typed_response(writer, &request.id, &empty);
        }
        let edit = TextEdit {
            range: full_document_range(&document.text),
            new_text: formatted,
        };
        write_typed_response(writer, &request.id, &vec![edit])
    }

    fn on_code_lens<W: Write>(
        &mut self,
        writer: &mut W,
        request: &RequestMessage,
    ) -> io::Result<()> {
        let Some(params_value) = request.params.clone() else {
            return write_invalid_params(writer, &request.id, "codeLens requires params");
        };
        let params: CodeLensParams = match serde_json::from_value(params_value) {
            Ok(params) => params,
            Err(err) => {
                return write_invalid_params(
                    writer,
                    &request.id,
                    &format!("invalid codeLens params: {err}"),
                );
            }
        };

        // Count callers across the entire workspace. Each `Call(Var(name))`
        // anywhere in any function body that matches a function symbol in
        // the requested document contributes one. This deliberately walks
        // every open document so workspace-wide call sites are reflected.
        let Some(document) = self.state.get(&params.text_document.uri).cloned() else {
            let empty: Vec<CodeLens> = Vec::new();
            return write_typed_response(writer, &request.id, &empty);
        };
        let source = SourceFile::new(&document.uri, &document.text);
        let result = Compiler::check_source(source.clone());
        let function_names: std::collections::BTreeSet<String> = result
            .module
            .symbols
            .iter()
            .filter(|s| s.kind == CompilerSymbolKind::Function)
            .map(|s| s.name.clone())
            .collect();
        let mut caller_counts: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for name in &function_names {
            caller_counts.insert(name.clone(), 0);
        }

        let mut documents: Vec<_> = self.state.iter().collect();
        documents.sort_by(|a, b| a.uri.cmp(&b.uri));
        for doc in documents {
            let doc_source = SourceFile::new(&doc.uri, &doc.text);
            let doc_result = Compiler::check_source(doc_source.clone());
            let bodies = parse_module_bodies_with_module(&doc_source, &doc_result.module);
            for symbol in &doc_result.module.symbols {
                if symbol.kind != CompilerSymbolKind::Function {
                    continue;
                }
                let Some(body) = bodies.get(&symbol.id) else {
                    continue;
                };
                collect_call_targets(body, &function_names, &mut caller_counts);
            }
        }

        let mut lenses: Vec<CodeLens> = Vec::new();
        for symbol in &result.module.symbols {
            if symbol.kind != CompilerSymbolKind::Function {
                continue;
            }
            let count = caller_counts.get(&symbol.name).copied().unwrap_or(0);
            let title = format!("{count} callers");
            let range = span_to_range(&symbol.span);
            lenses.push(CodeLens {
                range,
                command: Some(Command {
                    title: title.clone(),
                    command: "ori.lsp.codeLens.showCallers".to_string(),
                    arguments: vec![serde_json::json!({
                        "uri": document.uri,
                        "symbol": symbol.id,
                    })],
                }),
                data: Some(serde_json::json!({
                    "schema": "ori.lsp.code_lens.v1",
                    "symbol": symbol.id,
                    "callers": count,
                })),
            });
        }
        // Stable, line-then-column ordering so test assertions don't rely on
        // hash-map iteration.
        lenses.sort_by(|a, b| {
            (a.range.start.line, a.range.start.character)
                .cmp(&(b.range.start.line, b.range.start.character))
        });
        write_typed_response(writer, &request.id, &lenses)
    }

    fn on_code_lens_resolve<W: Write>(
        &mut self,
        writer: &mut W,
        request: &RequestMessage,
    ) -> io::Result<()> {
        // Every lens emitted by `on_code_lens` is already fully populated,
        // so `resolve` is the identity transform. We still accept the
        // request to advertise spec-compliant behaviour for clients that
        // always send it through.
        let Some(params_value) = request.params.clone() else {
            return write_invalid_params(writer, &request.id, "codeLens/resolve requires params");
        };
        let lens: CodeLens = match serde_json::from_value(params_value) {
            Ok(lens) => lens,
            Err(err) => {
                return write_invalid_params(
                    writer,
                    &request.id,
                    &format!("invalid codeLens/resolve params: {err}"),
                );
            }
        };
        write_typed_response(writer, &request.id, &lens)
    }

    fn on_inlay_hint<W: Write>(
        &mut self,
        writer: &mut W,
        request: &RequestMessage,
    ) -> io::Result<()> {
        let Some(params_value) = request.params.clone() else {
            return write_invalid_params(writer, &request.id, "inlayHint requires params");
        };
        let params: InlayHintParams = match serde_json::from_value(params_value) {
            Ok(params) => params,
            Err(err) => {
                return write_invalid_params(
                    writer,
                    &request.id,
                    &format!("invalid inlayHint params: {err}"),
                );
            }
        };
        let Some(document) = self.state.get(&params.text_document.uri).cloned() else {
            let empty: Vec<InlayHint> = Vec::new();
            return write_typed_response(writer, &request.id, &empty);
        };
        let mut hints = compute_inlay_hints(&document.uri, &document.text);
        hints.retain(|hint| position_in_range(&params.range, &hint.position));
        write_typed_response(writer, &request.id, &hints)
    }

    fn on_folding_range<W: Write>(
        &mut self,
        writer: &mut W,
        request: &RequestMessage,
    ) -> io::Result<()> {
        let Some(params_value) = request.params.clone() else {
            return write_invalid_params(writer, &request.id, "foldingRange requires params");
        };
        let params: FoldingRangeParams = match serde_json::from_value(params_value) {
            Ok(params) => params,
            Err(err) => {
                return write_invalid_params(
                    writer,
                    &request.id,
                    &format!("invalid foldingRange params: {err}"),
                );
            }
        };
        let Some(document) = self.state.get(&params.text_document.uri).cloned() else {
            let empty: Vec<FoldingRange> = Vec::new();
            return write_typed_response(writer, &request.id, &empty);
        };
        let ranges = compute_folding_ranges(&document.text);
        write_typed_response(writer, &request.id, &ranges)
    }

    fn on_selection_range<W: Write>(
        &mut self,
        writer: &mut W,
        request: &RequestMessage,
    ) -> io::Result<()> {
        let Some(params_value) = request.params.clone() else {
            return write_invalid_params(writer, &request.id, "selectionRange requires params");
        };
        let params: SelectionRangeParams = match serde_json::from_value(params_value) {
            Ok(params) => params,
            Err(err) => {
                return write_invalid_params(
                    writer,
                    &request.id,
                    &format!("invalid selectionRange params: {err}"),
                );
            }
        };
        let Some(document) = self.state.get(&params.text_document.uri).cloned() else {
            let empty: Vec<SelectionRange> = Vec::new();
            return write_typed_response(writer, &request.id, &empty);
        };
        let source = SourceFile::new(&document.uri, &document.text);
        let result = Compiler::check_source(source);
        let mut out: Vec<SelectionRange> = Vec::with_capacity(params.positions.len());
        for position in &params.positions {
            out.push(build_selection_range(&document.text, &result, *position));
        }
        write_typed_response(writer, &request.id, &out)
    }

    // ---------- notification handlers ----------

    fn on_did_open<W: Write>(&mut self, writer: &mut W, params: Option<Value>) -> io::Result<()> {
        let Some(value) = params else {
            return Ok(());
        };
        let Ok(parsed) = serde_json::from_value::<DidOpenTextDocumentParams>(value) else {
            return Ok(());
        };
        let uri = parsed.text_document.uri.clone();
        let version = parsed.text_document.version;
        self.state
            .open(&uri, parsed.text_document.text.clone(), version);
        self.publish_diagnostics(writer, &uri, Some(version), &parsed.text_document.text)
    }

    fn on_did_change<W: Write>(&mut self, writer: &mut W, params: Option<Value>) -> io::Result<()> {
        let Some(value) = params else {
            return Ok(());
        };
        let Ok(parsed) = serde_json::from_value::<DidChangeTextDocumentParams>(value) else {
            return Ok(());
        };
        let Some(latest) = parsed.content_changes.into_iter().last() else {
            return Ok(());
        };
        let uri = parsed.text_document.uri.clone();
        let version = parsed.text_document.version;
        self.state.update(&uri, latest.text.clone(), version);
        self.publish_diagnostics(writer, &uri, Some(version), &latest.text)
    }

    fn on_did_close<W: Write>(&mut self, writer: &mut W, params: Option<Value>) -> io::Result<()> {
        let Some(value) = params else {
            return Ok(());
        };
        let Ok(parsed) = serde_json::from_value::<DidCloseTextDocumentParams>(value) else {
            return Ok(());
        };
        self.state.close(&parsed.text_document.uri);
        // Per LSP spec, send an empty diagnostic list so the editor clears
        // previously published markers for the closed document.
        let body = notification(
            "textDocument/publishDiagnostics",
            &PublishDiagnosticsParams {
                uri: parsed.text_document.uri,
                version: None,
                diagnostics: Vec::new(),
            },
        )
        .map_err(io_other)?;
        write_message(writer, &body)
    }

    fn publish_diagnostics<W: Write>(
        &self,
        writer: &mut W,
        uri: &str,
        version: Option<i64>,
        text: &str,
    ) -> io::Result<()> {
        let result = Compiler::check_source(SourceFile::new(uri, text));
        let diagnostics = to_lsp_diagnostics(&result.diagnostics);
        let params = PublishDiagnosticsParams {
            uri: uri.to_string(),
            version,
            diagnostics,
        };
        let body = notification("textDocument/publishDiagnostics", &params).map_err(io_other)?;
        write_message(writer, &body)
    }
}

fn write_typed_response<W: Write, R: Serialize>(
    writer: &mut W,
    id: &RequestId,
    result: &R,
) -> io::Result<()> {
    let body = success_response(id, result).map_err(io_other)?;
    write_message(writer, &body)
}

fn write_invalid_params<W: Write>(writer: &mut W, id: &RequestId, message: &str) -> io::Result<()> {
    let body = error_response(id, error_codes::INVALID_PARAMS, message).map_err(io_other)?;
    write_message(writer, &body)
}

fn io_other(err: serde_json::Error) -> io::Error {
    io::Error::other(err)
}

/// Locate the symbol whose span contains the LSP position. Lines and columns
/// are converted to 1-based form so they line up with the compiler's spans.
fn symbol_at(result: &CompileResult, position: Position) -> Option<&Symbol> {
    let target_line = (position.line as usize).saturating_add(1);
    let target_column = (position.character as usize).saturating_add(1);
    result
        .module
        .symbols
        .iter()
        .filter(|symbol| symbol.kind != CompilerSymbolKind::Module)
        .find(|symbol| span_contains(&symbol.span, target_line, target_column))
        .or_else(|| {
            // Fall back to the module symbol so hover still produces useful
            // content when the cursor is outside any specific declaration.
            result.module.symbols.first()
        })
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

fn render_symbol_markdown(symbol: &Symbol) -> String {
    let mut out = String::new();
    out.push_str("**");
    out.push_str(&symbol.id);
    out.push_str("**\n\n");
    out.push_str("```orison\n");
    out.push_str(&symbol.signature);
    out.push('\n');
    out.push_str("```\n");
    if !symbol.effects.is_empty() {
        out.push_str("\n_effects:_ ");
        out.push_str(&symbol.effects.join(", "));
        out.push('\n');
    }
    out.push_str(&format!("\n_kind:_ `{}`\n", symbol.kind.as_str()));
    out
}

fn patch_ir_ref(diagnostic_code: &str, index: usize) -> String {
    format!("patch:diag/{diagnostic_code}/fix/{index}")
}

fn symbol_to_completion_kind(kind: CompilerSymbolKind) -> CompletionItemKind {
    match kind {
        CompilerSymbolKind::Function | CompilerSymbolKind::Query => CompletionItemKind::Function,
        CompilerSymbolKind::Type => CompletionItemKind::Class,
        CompilerSymbolKind::Service
        | CompilerSymbolKind::View
        | CompilerSymbolKind::Actor
        | CompilerSymbolKind::Migration
        | CompilerSymbolKind::Capability => CompletionItemKind::Class,
        CompilerSymbolKind::Module => CompletionItemKind::Module,
        CompilerSymbolKind::Unknown => CompletionItemKind::Variable,
    }
}

/// `true` when the start position of `inner` falls inside the half-open
/// `[outer.start, outer.end)` range. Compares (line, character) tuples in
/// lexicographic order.
fn range_contains(outer: &Range, inner: &Range) -> bool {
    let start = (inner.start.line, inner.start.character);
    let outer_start = (outer.start.line, outer.start.character);
    let outer_end = (outer.end.line, outer.end.character);
    start >= outer_start && start < outer_end
}

fn compiler_kind_to_lsp(kind: CompilerSymbolKind) -> LspSymbolKind {
    match kind {
        CompilerSymbolKind::Module => LspSymbolKind::Module,
        CompilerSymbolKind::Function | CompilerSymbolKind::Query => LspSymbolKind::Function,
        CompilerSymbolKind::Type => LspSymbolKind::Class,
        CompilerSymbolKind::Service
        | CompilerSymbolKind::View
        | CompilerSymbolKind::Actor
        | CompilerSymbolKind::Migration
        | CompilerSymbolKind::Capability => LspSymbolKind::Class,
        CompilerSymbolKind::Unknown => LspSymbolKind::Variable,
    }
}

fn compiler_symbol_to_information(uri: &str, symbol: &Symbol) -> SymbolInformation {
    SymbolInformation {
        name: symbol.name.clone(),
        kind: compiler_kind_to_lsp(symbol.kind),
        location: Location {
            uri: uri.to_string(),
            range: span_to_range(&symbol.span),
        },
        container_name: None,
    }
}

/// Walk `text` and return one `Range` per identifier-token occurrence of
/// `needle` outside string literals and `#` line comments. Byte offsets are
/// safe because the bootstrap parser only accepts ASCII.
fn identifier_occurrences(text: &str, needle: &str) -> Vec<Range> {
    let mut out: Vec<Range> = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let bytes = text.as_bytes();
    let mut line: u32 = 0;
    let mut column: u32 = 0;
    let mut in_string = false;
    let mut in_line_comment = false;
    let mut escape = false;
    let mut ident_start: Option<(u32, u32, usize)> = None;

    for (idx, &byte) in bytes.iter().enumerate() {
        let ch = byte as char;

        if in_line_comment {
            if ch == '\n' {
                in_line_comment = false;
            }
            advance(&mut line, &mut column, ch);
            continue;
        }

        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            advance(&mut line, &mut column, ch);
            continue;
        }

        if ch == '"' {
            flush_pending_occurrence(&mut out, &mut ident_start, line, column, text, needle);
            in_string = true;
            advance(&mut line, &mut column, ch);
            continue;
        }

        if ch == '#' {
            flush_pending_occurrence(&mut out, &mut ident_start, line, column, text, needle);
            in_line_comment = true;
            advance(&mut line, &mut column, ch);
            continue;
        }

        if is_ident_byte(byte) {
            if ident_start.is_none() {
                ident_start = Some((line, column, idx));
            }
        } else {
            flush_pending_occurrence(&mut out, &mut ident_start, line, column, text, needle);
        }
        advance(&mut line, &mut column, ch);
    }
    flush_pending_occurrence(&mut out, &mut ident_start, line, column, text, needle);
    out
}

fn flush_pending_occurrence(
    out: &mut Vec<Range>,
    ident_start: &mut Option<(u32, u32, usize)>,
    end_line: u32,
    end_column: u32,
    text: &str,
    needle: &str,
) {
    let Some((start_line, start_column, start_idx)) = ident_start.take() else {
        return;
    };
    let end_idx = position_to_byte_offset(text, end_line, end_column).unwrap_or(text.len());
    let slice = text.get(start_idx..end_idx).unwrap_or("");
    if slice == needle {
        out.push(Range {
            start: Position {
                line: start_line,
                character: start_column,
            },
            end: Position {
                line: end_line,
                character: end_column,
            },
        });
    }
}

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

/// Returns the identifier touched by the given LSP position, if any. Lines
/// are 0-based; columns are UTF-16 code unit offsets which we approximate as
/// byte offsets given the bootstrap parser only accepts ASCII source.
fn identifier_at(text: &str, position: Position) -> Option<String> {
    let line = text.lines().nth(position.line as usize)?;
    let column = position.character as usize;
    let bytes = line.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let len = bytes.len();
    // Cursor can sit just past the final character; clamp so we still pick
    // up identifiers at end-of-line.
    let probe = column.min(len.saturating_sub(1));
    if !is_ident_byte(bytes[probe]) {
        // Try the byte to the left in case the cursor is at the right edge.
        if column == 0 || !is_ident_byte(bytes[column - 1]) {
            return None;
        }
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

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Produce a list of [`TextEdit`]s that rename every identifier-token
/// occurrence of `from` to `to` outside of string literals. Mirrors the
/// algorithm used by `patch_apply::rename_identifier` so the LSP rename and
/// the compiler patch flow stay consistent.
fn identifier_edits(text: &str, from: &str, to: &str) -> Vec<TextEdit> {
    let mut edits: Vec<TextEdit> = Vec::new();
    let mut line: u32 = 0;
    let mut column: u32 = 0;
    let mut in_string = false;
    let mut escape = false;
    let mut ident_start: Option<(u32, u32, usize)> = None;
    let bytes = text.as_bytes();

    for (idx, &byte) in bytes.iter().enumerate() {
        let ch = byte as char;
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            advance(&mut line, &mut column, ch);
            continue;
        }
        if ch == '"' {
            flush_pending_edit(&mut edits, &mut ident_start, text, line, column, from, to);
            in_string = true;
            advance(&mut line, &mut column, ch);
            continue;
        }
        if is_ident_byte(byte) {
            if ident_start.is_none() {
                ident_start = Some((line, column, idx));
            }
        } else {
            flush_pending_edit(&mut edits, &mut ident_start, text, line, column, from, to);
        }
        advance(&mut line, &mut column, ch);
    }
    flush_pending_edit(&mut edits, &mut ident_start, text, line, column, from, to);
    edits
}

fn advance(line: &mut u32, column: &mut u32, ch: char) {
    if ch == '\n' {
        *line = line.saturating_add(1);
        *column = 0;
    } else {
        *column = column.saturating_add(1);
    }
}

fn flush_pending_edit(
    edits: &mut Vec<TextEdit>,
    ident_start: &mut Option<(u32, u32, usize)>,
    text: &str,
    end_line: u32,
    end_column: u32,
    from: &str,
    to: &str,
) {
    let Some((start_line, start_column, start_idx)) = ident_start.take() else {
        return;
    };
    let end_idx = position_to_byte_offset(text, end_line, end_column).unwrap_or(text.len());
    let slice = text.get(start_idx..end_idx).unwrap_or("");
    if slice == from {
        edits.push(TextEdit {
            range: Range {
                start: Position {
                    line: start_line,
                    character: start_column,
                },
                end: Position {
                    line: end_line,
                    character: end_column,
                },
            },
            new_text: to.to_string(),
        });
    }
}

fn position_to_byte_offset(text: &str, line: u32, column: u32) -> Option<usize> {
    let mut current_line: u32 = 0;
    let mut current_column: u32 = 0;
    for (idx, ch) in text.char_indices() {
        if current_line == line && current_column == column {
            return Some(idx);
        }
        if ch == '\n' {
            current_line = current_line.saturating_add(1);
            current_column = 0;
        } else {
            current_column = current_column.saturating_add(1);
        }
    }
    if current_line == line && current_column == column {
        return Some(text.len());
    }
    None
}

// ---------- semantic tokens ----------

/// Index of `name` inside the legend advertised by
/// [`crate::protocol::SEMANTIC_TOKEN_TYPES`]. Returns `12` (operator) for
/// any unknown name so the encoder never panics on an unmapped category.
fn semantic_token_type_index(name: &str) -> u32 {
    use crate::protocol::SEMANTIC_TOKEN_TYPES;
    SEMANTIC_TOKEN_TYPES
        .iter()
        .position(|candidate| *candidate == name)
        .map(|n| n as u32)
        .unwrap_or(12)
}

/// Compute the LSP-encoded semantic-token stream for `text`. Comments are
/// not produced by the lexer, so we scan for `//` line comments separately
/// and merge them in source order.
fn compute_semantic_tokens(uri: &str, text: &str) -> SemanticTokens {
    let source = SourceFile::new(uri, text);
    let result = Compiler::check_source(source.clone());
    let function_names: std::collections::BTreeSet<String> = result
        .module
        .symbols
        .iter()
        .filter(|s| s.kind == CompilerSymbolKind::Function)
        .map(|s| s.name.clone())
        .collect();
    let type_names: std::collections::BTreeSet<String> = result
        .module
        .symbols
        .iter()
        .filter(|s| s.kind == CompilerSymbolKind::Type)
        .map(|s| s.name.clone())
        .collect();
    let (tokens, _diags) = lex_with_diagnostics(&source);

    // Collected as (line, col, length, tokenType) using 0-based coords.
    let mut runs: Vec<(u32, u32, u32, u32)> = Vec::new();

    for tok in &tokens {
        if matches!(tok.kind, TokenKind::Eof) {
            continue;
        }
        let line = zero_based_u32(tok.span.start.line);
        let column = zero_based_u32(tok.span.start.column);
        let length = lexeme_token_length(&tok.kind, &tok.lexeme);
        let category = match tok.kind {
            TokenKind::Keyword => Some("keyword"),
            TokenKind::Number => Some("number"),
            TokenKind::String => Some("string"),
            TokenKind::Symbol => Some("operator"),
            TokenKind::Ident => {
                if type_names.contains(&tok.lexeme)
                    || ori_compiler::types::is_builtin_type(&tok.lexeme)
                {
                    Some("type")
                } else if function_names.contains(&tok.lexeme) {
                    Some("function")
                } else if tok
                    .lexeme
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_uppercase())
                {
                    Some("type")
                } else {
                    Some("variable")
                }
            }
            TokenKind::Eof => None,
        };
        let Some(name) = category else { continue };
        let ty = semantic_token_type_index(name);
        runs.push((line, column, length, ty));
    }

    // Scan for `//` line comments. The lexer drops them, so we recover them
    // here and emit one run per comment fragment (a comment terminates at
    // the newline, so it is always a single-line run).
    let bytes = text.as_bytes();
    let mut line: u32 = 0;
    let mut column: u32 = 0;
    let mut idx = 0usize;
    let mut in_string = false;
    let mut escape = false;
    while idx < bytes.len() {
        let ch = bytes[idx] as char;
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            if ch == '\n' {
                line = line.saturating_add(1);
                column = 0;
            } else {
                column = column.saturating_add(1);
            }
            idx += 1;
            continue;
        }
        if ch == '"' {
            in_string = true;
            column = column.saturating_add(1);
            idx += 1;
            continue;
        }
        if ch == '/' && idx + 1 < bytes.len() && bytes[idx + 1] == b'/' {
            let start_col = column;
            let comment_line = line;
            let mut length: u32 = 0;
            while idx < bytes.len() && bytes[idx] != b'\n' {
                length = length.saturating_add(1);
                idx += 1;
                column = column.saturating_add(1);
            }
            runs.push((
                comment_line,
                start_col,
                length,
                semantic_token_type_index("comment"),
            ));
            continue;
        }
        if ch == '\n' {
            line = line.saturating_add(1);
            column = 0;
        } else {
            column = column.saturating_add(1);
        }
        idx += 1;
    }

    // Sort by absolute (line, column) so the delta encoding below sees
    // every run in source order.
    runs.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));

    // LSP §3.17 expects each 5-tuple to be relative to the previous one:
    // `deltaLine`, `deltaStart`, `length`, `tokenType`, `tokenModifiers`.
    let mut data: Vec<u32> = Vec::with_capacity(runs.len() * 5);
    let mut prev_line: u32 = 0;
    let mut prev_col: u32 = 0;
    for (line, column, length, ty) in runs {
        let delta_line = line.saturating_sub(prev_line);
        let delta_start = if delta_line == 0 {
            column.saturating_sub(prev_col)
        } else {
            column
        };
        data.push(delta_line);
        data.push(delta_start);
        data.push(length);
        data.push(ty);
        data.push(0); // tokenModifiers
        prev_line = line;
        prev_col = column;
    }
    SemanticTokens { data }
}

fn lexeme_token_length(kind: &TokenKind, lexeme: &str) -> u32 {
    let base = lexeme.chars().count() as u32;
    match kind {
        // String spans include the surrounding double quotes in source,
        // but the lexeme strips them. Add them back so the highlight covers
        // the full literal.
        TokenKind::String => base.saturating_add(2),
        _ => base.max(1),
    }
}

fn zero_based_u32(one_based: usize) -> u32 {
    u32::try_from(one_based.saturating_sub(1)).unwrap_or(u32::MAX)
}

// ---------- code lens helpers ----------

fn collect_call_targets(
    expr: &Expr,
    names: &std::collections::BTreeSet<String>,
    counts: &mut std::collections::BTreeMap<String, usize>,
) {
    match expr {
        Expr::Call { callee, args } => {
            if let Expr::Var(name) = callee.as_ref() {
                if names.contains(name) {
                    *counts.entry(name.clone()).or_insert(0) += 1;
                }
            }
            collect_call_targets(callee, names, counts);
            for arg in args {
                collect_call_targets(arg, names, counts);
            }
        }
        Expr::Field { base, .. } => collect_call_targets(base, names, counts),
        Expr::Block { stmts, tail } => {
            for stmt in stmts {
                match stmt {
                    Stmt::Let { init, .. } => collect_call_targets(init, names, counts),
                    Stmt::Expr(e) => collect_call_targets(e, names, counts),
                    Stmt::Return(Some(e)) => collect_call_targets(e, names, counts),
                    Stmt::Return(None) => {}
                }
            }
            if let Some(e) = tail {
                collect_call_targets(e, names, counts);
            }
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_call_targets(cond, names, counts);
            collect_call_targets(then_branch, names, counts);
            if let Some(e) = else_branch {
                collect_call_targets(e, names, counts);
            }
        }
        Expr::Match { scrutinee, arms } => {
            collect_call_targets(scrutinee, names, counts);
            for MatchArm { body, .. } in arms {
                collect_call_targets(body, names, counts);
            }
        }
        Expr::Return(Some(e)) => collect_call_targets(e, names, counts),
        Expr::Construct { args, .. } => {
            for arg in args {
                collect_call_targets(arg, names, counts);
            }
        }
        Expr::Try(e) => collect_call_targets(e, names, counts),
        Expr::Tuple(items) => {
            for it in items {
                collect_call_targets(it, names, counts);
            }
        }
        Expr::Record { fields } => {
            for (_, e) in fields {
                collect_call_targets(e, names, counts);
            }
        }
        Expr::Lambda { body, .. } => collect_call_targets(body, names, counts),
        Expr::Binary { lhs, rhs, .. } => {
            collect_call_targets(lhs, names, counts);
            collect_call_targets(rhs, names, counts);
        }
        Expr::Unary { operand, .. } => collect_call_targets(operand, names, counts),
        Expr::InterpString { parts } => {
            for part in parts {
                if let ori_compiler::expr::InterpPart::Expr(inner) = part {
                    collect_call_targets(inner, names, counts);
                }
            }
        }
        Expr::Lit(_) | Expr::Var(_) | Expr::Return(None) | Expr::RawStr { .. } | Expr::Error => {}
    }
}

// ---------- inlay hints ----------

/// Compute one inlay hint per `let` binding that lacks an explicit type
/// annotation. The hint is placed immediately after the binding name.
fn compute_inlay_hints(uri: &str, text: &str) -> Vec<InlayHint> {
    let source = SourceFile::new(uri, text);
    let parse = Compiler::check_source(source.clone());
    let bodies = parse_module_bodies_with_module(&source, &parse.module);
    let mut hints: Vec<InlayHint> = Vec::new();

    for symbol in &parse.module.symbols {
        if symbol.kind != CompilerSymbolKind::Function {
            continue;
        }
        let Some(body) = bodies.get(&symbol.id) else {
            continue;
        };
        let mut let_bindings: Vec<(String, TypeEnv)> = Vec::new();
        collect_let_bindings(body, &mut TypeEnv::new(), &mut let_bindings);
        // Each entry stays paired with the env that was visible after the
        // binding ran. We re-scan the source text to map the binding name
        // to a source position, picking each occurrence in order so shadowed
        // bindings still get individual hints.
        let mut occurrences = find_let_positions(text);
        for (name, env_after) in &let_bindings {
            let Some(position) = occurrences.next_for(name) else {
                continue;
            };
            let ty_label = env_after
                .lookup(name)
                .map(|t| t.display())
                .unwrap_or_else(|| "_".to_string());
            hints.push(InlayHint {
                position,
                label: format!(": {ty_label}"),
                kind: 1,
                padding_left: Some(false),
                padding_right: Some(true),
            });
        }
    }

    hints.sort_by(|a, b| {
        (a.position.line, a.position.character).cmp(&(b.position.line, b.position.character))
    });
    hints
}

/// Walk `expr` and record every `Stmt::Let { ty: None, .. }` together with
/// a snapshot of the environment **after** the binding has been bound.
fn collect_let_bindings(expr: &Expr, env: &mut TypeEnv, out: &mut Vec<(String, TypeEnv)>) {
    match expr {
        Expr::Block { stmts, tail } => {
            for stmt in stmts {
                match stmt {
                    Stmt::Let { name, ty, init } => {
                        let inferred = if let Some(declared) = ty {
                            declared.clone()
                        } else {
                            let (inferred, _diags) = infer_expr(init, env);
                            inferred
                        };
                        env.bind(name.clone(), inferred.clone());
                        if ty.is_none() {
                            out.push((name.clone(), env.clone()));
                        }
                    }
                    Stmt::Expr(e) => collect_let_bindings(e, env, out),
                    Stmt::Return(Some(e)) => collect_let_bindings(e, env, out),
                    Stmt::Return(None) => {}
                }
            }
            if let Some(t) = tail {
                collect_let_bindings(t, env, out);
            }
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_let_bindings(cond, env, out);
            let mut branch_env = env.clone();
            collect_let_bindings(then_branch, &mut branch_env, out);
            if let Some(e) = else_branch {
                let mut else_env = env.clone();
                collect_let_bindings(e, &mut else_env, out);
            }
        }
        Expr::Match { scrutinee, arms } => {
            collect_let_bindings(scrutinee, env, out);
            for MatchArm { body, .. } in arms {
                let mut arm_env = env.clone();
                collect_let_bindings(body, &mut arm_env, out);
            }
        }
        Expr::Call { callee, args } => {
            collect_let_bindings(callee, env, out);
            for arg in args {
                collect_let_bindings(arg, env, out);
            }
        }
        Expr::Field { base, .. } => collect_let_bindings(base, env, out),
        Expr::Return(Some(e)) => collect_let_bindings(e, env, out),
        Expr::Construct { args, .. } => {
            for arg in args {
                collect_let_bindings(arg, env, out);
            }
        }
        Expr::Try(e) => collect_let_bindings(e, env, out),
        Expr::Tuple(items) => {
            for it in items {
                collect_let_bindings(it, env, out);
            }
        }
        Expr::Record { fields } => {
            for (_, e) in fields {
                collect_let_bindings(e, env, out);
            }
        }
        Expr::Lambda { body, .. } => collect_let_bindings(body, env, out),
        Expr::Binary { lhs, rhs, .. } => {
            collect_let_bindings(lhs, env, out);
            collect_let_bindings(rhs, env, out);
        }
        Expr::Unary { operand, .. } => collect_let_bindings(operand, env, out),
        Expr::InterpString { parts } => {
            for part in parts {
                if let ori_compiler::expr::InterpPart::Expr(inner) = part {
                    collect_let_bindings(inner, env, out);
                }
            }
        }
        Expr::Lit(_) | Expr::Var(_) | Expr::Return(None) | Expr::RawStr { .. } | Expr::Error => {}
    }
}

/// Cursor that records every `let <name>` occurrence in source order so
/// callers can map AST-level `Let` bindings to source positions. The
/// position returned points one column past the name.
struct LetPositionCursor {
    positions: Vec<(String, Position)>,
    counters: std::collections::BTreeMap<String, usize>,
}

impl LetPositionCursor {
    fn next_for(&mut self, name: &str) -> Option<Position> {
        let counter = self.counters.entry(name.to_string()).or_insert(0);
        let mut found_idx: Option<usize> = None;
        let mut seen = 0usize;
        for (idx, (candidate, _)) in self.positions.iter().enumerate() {
            if candidate == name {
                if seen == *counter {
                    found_idx = Some(idx);
                    break;
                }
                seen += 1;
            }
        }
        let position = found_idx.map(|idx| self.positions[idx].1);
        *counter += 1;
        position
    }
}

fn find_let_positions(text: &str) -> LetPositionCursor {
    let mut positions: Vec<(String, Position)> = Vec::new();
    let bytes = text.as_bytes();
    let mut line: u32 = 0;
    let mut column: u32 = 0;
    let mut in_string = false;
    let mut escape = false;
    let mut in_line_comment = false;
    let mut idx = 0usize;

    while idx < bytes.len() {
        let ch = bytes[idx] as char;
        if in_line_comment {
            if ch == '\n' {
                in_line_comment = false;
                line = line.saturating_add(1);
                column = 0;
            } else {
                column = column.saturating_add(1);
            }
            idx += 1;
            continue;
        }
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            if ch == '\n' {
                line = line.saturating_add(1);
                column = 0;
            } else {
                column = column.saturating_add(1);
            }
            idx += 1;
            continue;
        }
        if ch == '"' {
            in_string = true;
            column = column.saturating_add(1);
            idx += 1;
            continue;
        }
        if ch == '/' && idx + 1 < bytes.len() && bytes[idx + 1] == b'/' {
            in_line_comment = true;
            column = column.saturating_add(1);
            idx += 1;
            continue;
        }
        // Match a `let` keyword at a word boundary.
        if ch == 'l' && idx + 3 <= bytes.len() && &bytes[idx..idx + 3] == b"let" {
            let before_ok = idx == 0 || !is_ident_byte(bytes[idx - 1]);
            let after_ok = idx + 3 == bytes.len() || !is_ident_byte(bytes[idx + 3]);
            if before_ok && after_ok {
                // Skip past `let` and any whitespace, then capture the
                // identifier that follows.
                let mut cursor = idx + 3;
                let mut cur_col = column + 3;
                while cursor < bytes.len() {
                    let c = bytes[cursor];
                    if c == b' ' || c == b'\t' {
                        cursor += 1;
                        cur_col = cur_col.saturating_add(1);
                    } else {
                        break;
                    }
                }
                if cursor < bytes.len() && is_ident_byte(bytes[cursor]) {
                    let name_start = cursor;
                    while cursor < bytes.len() && is_ident_byte(bytes[cursor]) {
                        cursor += 1;
                        cur_col = cur_col.saturating_add(1);
                    }
                    let name = text.get(name_start..cursor).unwrap_or("").to_string();
                    positions.push((
                        name,
                        Position {
                            line,
                            character: cur_col,
                        },
                    ));
                    column = cur_col;
                    idx = cursor;
                    continue;
                }
            }
        }
        if ch == '\n' {
            line = line.saturating_add(1);
            column = 0;
        } else {
            column = column.saturating_add(1);
        }
        idx += 1;
    }
    LetPositionCursor {
        positions,
        counters: std::collections::BTreeMap::new(),
    }
}

fn position_in_range(range: &Range, position: &Position) -> bool {
    let start = (range.start.line, range.start.character);
    let end = (range.end.line, range.end.character);
    let pos = (position.line, position.character);
    pos >= start && pos <= end
}

fn full_document_range(text: &str) -> Range {
    let mut line: u32 = 0;
    let mut column: u32 = 0;
    for ch in text.chars() {
        if ch == '\n' {
            line = line.saturating_add(1);
            column = 0;
        } else {
            column = column.saturating_add(1);
        }
    }
    Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line,
            character: column,
        },
    }
}

// ---------- folding ranges ----------

/// Compute folding ranges for `text`. Orison's surface syntax is
/// indentation-based: a `Block`, `Match`, `If/else`, or multi-line record
/// literal opens with a header line ending in `:` or `{` and runs until
/// the indentation returns to (or below) the header's level. We use this
/// heuristic so the folding works without requiring per-node spans.
fn compute_folding_ranges(text: &str) -> Vec<FoldingRange> {
    let lines: Vec<&str> = text.split('\n').collect();
    let line_count = lines.len();
    let mut ranges: Vec<FoldingRange> = Vec::new();

    fn indent_of(line: &str) -> Option<usize> {
        let trimmed = line.trim_end();
        if trimmed.is_empty() || trimmed.trim_start().starts_with("//") {
            None
        } else {
            Some(line.len() - line.trim_start().len())
        }
    }

    for (idx, raw) in lines.iter().enumerate() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        let opens_block = line.ends_with(':') || line.ends_with('{');
        let opens_record = trimmed.contains('=') && line.ends_with('{');
        let opens_match = trimmed.starts_with("match ") && line.ends_with(':');
        let opens_if =
            (trimmed.starts_with("if ") || trimmed == "else" || trimmed.starts_with("else "))
                && line.ends_with(':');
        if !(opens_block || opens_record || opens_match || opens_if) {
            continue;
        }
        let Some(header_indent) = indent_of(raw) else {
            continue;
        };
        let mut end_line = idx;
        for (j, candidate) in lines.iter().enumerate().take(line_count).skip(idx + 1) {
            match indent_of(candidate) {
                Some(child_indent) if child_indent > header_indent => {
                    end_line = j;
                }
                Some(_) => break,
                None => {
                    // Blank/comment lines do not close a block; continue.
                }
            }
        }
        if end_line > idx {
            let kind = if opens_match {
                Some("region".to_string())
            } else {
                None
            };
            ranges.push(FoldingRange {
                start_line: idx as u32,
                end_line: end_line as u32,
                kind,
            });
        }
    }

    ranges.sort_by(|a, b| {
        a.start_line
            .cmp(&b.start_line)
            .then(a.end_line.cmp(&b.end_line))
    });
    ranges.dedup_by(|a, b| a.start_line == b.start_line && a.end_line == b.end_line);
    ranges
}

// ---------- selection ranges ----------

fn build_selection_range(text: &str, result: &CompileResult, position: Position) -> SelectionRange {
    // Outer-to-inner stack of candidate ranges. We always include the
    // module range as the outermost frame so even a click on whitespace
    // returns something useful.
    let mut frames: Vec<Range> = Vec::new();
    frames.push(full_document_range(text));

    // Enclosing function/type/service symbol, if any.
    let line_one_based = (position.line as usize).saturating_add(1);
    let column_one_based = (position.character as usize).saturating_add(1);
    let mut best_symbol_range: Option<Range> = None;
    for symbol in &result.module.symbols {
        if symbol.kind == CompilerSymbolKind::Module {
            continue;
        }
        if span_contains(&symbol.span, line_one_based, column_one_based) {
            let candidate = span_to_range(&symbol.span);
            best_symbol_range = match best_symbol_range {
                Some(existing) if range_area(&existing) <= range_area(&candidate) => Some(existing),
                _ => Some(candidate),
            };
        }
    }
    if let Some(range) = best_symbol_range {
        frames.push(range);
    }

    // The line the cursor sits on.
    let line_range = line_range_at(text, position.line);
    frames.push(line_range);

    // The identifier (if any) under the cursor.
    if let Some(ident_range) = identifier_range_at(text, position) {
        frames.push(ident_range);
    }

    // Compose into a nested SelectionRange. The LSP spec returns the
    // *smallest* enclosing range at the top, with `parent` linking outward.
    // We accumulate frames in outer→inner order; walking in that order with
    // each new frame becoming the *child* (so the latest, innermost frame
    // ends up as the outermost variable) produces the wanted nesting when
    // we then return the last `current` that contains every parent below.
    let mut current: Option<SelectionRange> = None;
    for range in frames.into_iter() {
        let next = SelectionRange {
            range,
            parent: None,
        };
        match current.take() {
            Some(parent) => {
                let mut new_inner = next;
                new_inner.parent = Some(Box::new(parent));
                current = Some(new_inner);
            }
            None => current = Some(next),
        }
    }
    current.unwrap_or_else(|| SelectionRange {
        range: Range::default(),
        parent: None,
    })
}

fn range_area(range: &Range) -> u64 {
    let line_diff = u64::from(range.end.line.saturating_sub(range.start.line));
    if line_diff == 0 {
        u64::from(range.end.character.saturating_sub(range.start.character))
    } else {
        line_diff
            .saturating_mul(10_000)
            .saturating_add(u64::from(range.end.character))
    }
}

fn line_range_at(text: &str, line: u32) -> Range {
    let lines: Vec<&str> = text.split('\n').collect();
    let idx = (line as usize).min(lines.len().saturating_sub(1));
    let length = lines.get(idx).map(|s| s.chars().count()).unwrap_or(0) as u32;
    Range {
        start: Position { line, character: 0 },
        end: Position {
            line,
            character: length,
        },
    }
}

fn identifier_range_at(text: &str, position: Position) -> Option<Range> {
    let line = text.lines().nth(position.line as usize)?;
    let bytes = line.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let len = bytes.len();
    let probe = (position.character as usize).min(len.saturating_sub(1));
    if !is_ident_byte(bytes[probe]) && (probe == 0 || !is_ident_byte(bytes[probe - 1])) {
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
    Some(Range {
        start: Position {
            line: position.line,
            character: start as u32,
        },
        end: Position {
            line: position.line,
            character: end as u32,
        },
    })
}
