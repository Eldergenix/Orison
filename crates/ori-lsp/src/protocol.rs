//! Typed JSON-RPC and LSP message shapes.
//!
//! Only the subset of fields needed by [`crate::server::Server`] are modelled.
//! All structures derive [`serde::Serialize`] / [`serde::Deserialize`] and use
//! `camelCase` field names to match the LSP wire format.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC message ids may be either a string or a number. We keep the raw
/// `serde_json::Value` so we can echo whatever the client sent back without
/// having to guess the original encoding.
pub type RequestId = Value;

/// Standard JSON-RPC error codes used by this server.
pub mod error_codes {
    /// Invalid JSON was received by the server.
    pub const PARSE_ERROR: i32 = -32700;
    /// The JSON sent is not a valid Request object.
    pub const INVALID_REQUEST: i32 = -32600;
    /// The method does not exist or is not available.
    pub const METHOD_NOT_FOUND: i32 = -32601;
    /// Invalid method parameter(s).
    pub const INVALID_PARAMS: i32 = -32602;
    /// Internal JSON-RPC error.
    pub const INTERNAL_ERROR: i32 = -32603;
}

/// A JSON-RPC 2.0 request envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestMessage {
    pub jsonrpc: String,
    pub id: RequestId,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// A JSON-RPC 2.0 notification envelope (no `id`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationMessage {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// A JSON-RPC 2.0 response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseMessage {
    pub jsonrpc: String,
    pub id: RequestId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
}

/// A JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseError {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// `initialize` request params. Only fields we care about are decoded; the
/// rest are tolerated through `serde`'s default deny-unknown-off behaviour.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_id: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_uri: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_info: Option<Value>,
}

/// `initialize` response result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub capabilities: ServerCapabilities,
    pub server_info: ServerInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

/// Subset of `ServerCapabilities` published by this server.
///
/// `text_document_sync = 1` corresponds to the LSP enum value for full
/// document synchronisation; we never request incremental changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    pub text_document_sync: i32,
    pub hover_provider: bool,
    pub code_action_provider: bool,
    pub completion_provider: CompletionOptions,
    pub rename_provider: bool,
    pub workspace_symbol_provider: bool,
    pub document_symbol_provider: bool,
    pub definition_provider: bool,
    pub references_provider: bool,
}

impl Default for ServerCapabilities {
    fn default() -> Self {
        Self {
            text_document_sync: 1,
            hover_provider: true,
            code_action_provider: true,
            completion_provider: CompletionOptions::default(),
            rename_provider: true,
            workspace_symbol_provider: true,
            document_symbol_provider: true,
            definition_provider: true,
            references_provider: true,
        }
    }
}

/// `completionProvider` server capability advertisement.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionOptions {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trigger_characters: Vec<String>,
}

impl Default for CompletionOptions {
    fn default() -> Self {
        Self {
            trigger_characters: vec![".".to_string(), ":".to_string()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocumentItem {
    pub uri: String,
    #[serde(default)]
    pub language_id: String,
    #[serde(default)]
    pub version: i64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocumentIdentifier {
    pub uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionedTextDocumentIdentifier {
    pub uri: String,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DidOpenTextDocumentParams {
    pub text_document: TextDocumentItem,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DidChangeTextDocumentParams {
    pub text_document: VersionedTextDocumentIdentifier,
    pub content_changes: Vec<TextDocumentContentChangeEvent>,
}

/// Full-document change event. We only support the spec variant where `range`
/// is omitted because the server advertises `textDocumentSync = 1` (Full).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocumentContentChangeEvent {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DidCloseTextDocumentParams {
    pub text_document: TextDocumentIdentifier,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

/// LSP severity codes. Stored as `i32` on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "i32", from = "i32")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

impl From<DiagnosticSeverity> for i32 {
    fn from(value: DiagnosticSeverity) -> Self {
        match value {
            DiagnosticSeverity::Error => 1,
            DiagnosticSeverity::Warning => 2,
            DiagnosticSeverity::Information => 3,
            DiagnosticSeverity::Hint => 4,
        }
    }
}

impl From<i32> for DiagnosticSeverity {
    fn from(value: i32) -> Self {
        match value {
            1 => DiagnosticSeverity::Error,
            2 => DiagnosticSeverity::Warning,
            3 => DiagnosticSeverity::Information,
            4 => DiagnosticSeverity::Hint,
            _ => DiagnosticSeverity::Information,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub range: Range,
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub source: String,
    pub message: String,
    /// Optional opaque payload preserved so the code-action handler can
    /// recover the original compiler fix without re-running the parser. The
    /// LSP spec allows arbitrary JSON here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishDiagnosticsParams {
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<i64>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoverParams {
    pub text_document: TextDocumentIdentifier,
    pub position: Position,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hover {
    pub contents: MarkupContent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkupContent {
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeActionParams {
    pub text_document: TextDocumentIdentifier,
    pub range: Range,
    pub context: CodeActionContext,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeActionContext {
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeAction {
    pub title: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
    /// Reference to the Patch IR document that materializes this action. The
    /// LSP `data` field is opaque; the client passes it back to the server
    /// during `codeAction/resolve` flows, which we will support in a later
    /// task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// `textDocument/completion` request params.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionParams {
    pub text_document: TextDocumentIdentifier,
    pub position: Position,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<CompletionContext>,
}

/// Trigger metadata supplied by clients alongside a completion request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionContext {
    #[serde(default)]
    pub trigger_kind: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_character: Option<String>,
}

/// Result of `textDocument/completion`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionList {
    pub is_incomplete: bool,
    pub items: Vec<CompletionItem>,
}

/// A single completion suggestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionItem {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<CompletionItemKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation: Option<MarkupContent>,
}

/// LSP `CompletionItemKind` subset used by the bootstrap server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "i32", from = "i32")]
pub enum CompletionItemKind {
    Function,
    Field,
    Variable,
    Class,
    Module,
    Property,
    Keyword,
    Snippet,
}

impl From<CompletionItemKind> for i32 {
    fn from(value: CompletionItemKind) -> Self {
        match value {
            CompletionItemKind::Function => 3,
            CompletionItemKind::Field => 5,
            CompletionItemKind::Variable => 6,
            CompletionItemKind::Class => 7,
            CompletionItemKind::Module => 9,
            CompletionItemKind::Property => 10,
            CompletionItemKind::Keyword => 14,
            CompletionItemKind::Snippet => 15,
        }
    }
}

impl From<i32> for CompletionItemKind {
    fn from(value: i32) -> Self {
        match value {
            3 => CompletionItemKind::Function,
            5 => CompletionItemKind::Field,
            6 => CompletionItemKind::Variable,
            7 => CompletionItemKind::Class,
            9 => CompletionItemKind::Module,
            10 => CompletionItemKind::Property,
            14 => CompletionItemKind::Keyword,
            15 => CompletionItemKind::Snippet,
            _ => CompletionItemKind::Variable,
        }
    }
}

/// `textDocument/rename` request params.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameParams {
    pub text_document: TextDocumentIdentifier,
    pub position: Position,
    pub new_name: String,
}

/// A textual edit applied to a single document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextEdit {
    pub range: Range,
    pub new_text: String,
}

/// Workspace-wide edit. Only the `changes` map is populated by this server;
/// `documentChanges` is intentionally omitted because the bootstrap client
/// does not advertise versioned document support.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEdit {
    #[serde(default)]
    pub changes: BTreeMap<String, Vec<TextEdit>>,
}

/// `workspace/symbol` request params.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSymbolParams {
    #[serde(default)]
    pub query: String,
}

/// `textDocument/documentSymbol` request params.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSymbolParams {
    pub text_document: TextDocumentIdentifier,
}

/// `textDocument/definition` request params. Modelled on
/// `TextDocumentPositionParams` from the LSP specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefinitionParams {
    pub text_document: TextDocumentIdentifier,
    pub position: Position,
}

/// `textDocument/references` request params.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceParams {
    pub text_document: TextDocumentIdentifier,
    pub position: Position,
    #[serde(default)]
    pub context: ReferenceContext,
}

/// `ReferenceContext` from the LSP spec.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceContext {
    #[serde(default)]
    pub include_declaration: bool,
}

/// LSP `Location` — a `Range` inside a document identified by `uri`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub uri: String,
    pub range: Range,
}

/// LSP `SymbolKind` subset used by the workspace/document symbol responses.
/// Wire format is an integer per the LSP specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "i32", from = "i32")]
pub enum SymbolKind {
    Module,
    Class,
    Property,
    Function,
    Variable,
}

impl From<SymbolKind> for i32 {
    fn from(value: SymbolKind) -> Self {
        match value {
            SymbolKind::Module => 2,
            SymbolKind::Class => 5,
            SymbolKind::Property => 7,
            SymbolKind::Function => 12,
            SymbolKind::Variable => 13,
        }
    }
}

impl From<i32> for SymbolKind {
    fn from(value: i32) -> Self {
        match value {
            2 => SymbolKind::Module,
            5 => SymbolKind::Class,
            7 => SymbolKind::Property,
            12 => SymbolKind::Function,
            13 => SymbolKind::Variable,
            _ => SymbolKind::Variable,
        }
    }
}

/// LSP `SymbolInformation` shape used as the flat result form for both
/// `workspace/symbol` and `textDocument/documentSymbol`. The richer hierarchical
/// `DocumentSymbol` form is intentionally not modelled here — clients that do
/// not understand `SymbolInformation` still render this view per the spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolInformation {
    pub name: String,
    pub kind: SymbolKind,
    pub location: Location,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_name: Option<String>,
}

/// Helper that constructs a notification envelope around an arbitrary
/// serializable params value.
pub fn notification<P: Serialize>(method: &str, params: &P) -> serde_json::Result<Vec<u8>> {
    let value = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    });
    serde_json::to_vec(&value)
}

/// Helper that constructs a success response envelope.
pub fn success_response<R: Serialize>(id: &RequestId, result: &R) -> serde_json::Result<Vec<u8>> {
    let value = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    });
    serde_json::to_vec(&value)
}

/// Helper that constructs a `null` result success response.
pub fn null_response(id: &RequestId) -> serde_json::Result<Vec<u8>> {
    let value = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": Value::Null,
    });
    serde_json::to_vec(&value)
}

/// Helper that constructs an error response envelope.
pub fn error_response(id: &RequestId, code: i32, message: &str) -> serde_json::Result<Vec<u8>> {
    let value = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        },
    });
    serde_json::to_vec(&value)
}
