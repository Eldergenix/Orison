//! Surface AST used by every public structured-output envelope.
//!
//! This module deliberately keeps the AST flat and serde-friendly: the
//! capsule, agent map, LSP, and IDE integrations all consume these shapes
//! directly, so any field change here is a schema break.

use crate::source::Span;
use serde::{Deserialize, Serialize};

/// Discriminator for the kind of declaration a [`Symbol`] represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    /// The synthetic module-level symbol every [`Module`] starts with.
    Module,
    /// A function declaration.
    Function,
    /// A type declaration (record, sum, alias, or wrapper).
    Type,
    /// A service declaration (collection of HTTP/RPC routes).
    Service,
    /// A view declaration (UI tree fragment).
    View,
    /// An actor declaration (single-writer state machine).
    Actor,
    /// A SQL/data query declaration.
    Query,
    /// A schema migration declaration.
    Migration,
    /// A capability declaration (named effect permission).
    Capability,
    /// Catch-all kind for declarations the parser could not classify; should
    /// never be emitted by a clean parse.
    Unknown,
}

impl SymbolKind {
    /// Return the canonical snake_case string used in JSON envelopes.
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Module => "module",
            SymbolKind::Function => "function",
            SymbolKind::Type => "type",
            SymbolKind::Service => "service",
            SymbolKind::View => "view",
            SymbolKind::Actor => "actor",
            SymbolKind::Query => "query",
            SymbolKind::Migration => "migration",
            SymbolKind::Capability => "capability",
            SymbolKind::Unknown => "unknown",
        }
    }
}

/// One declared symbol surfaced through the agent ABI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    /// Stable identifier (`mod:foo` or `sym:foo.bar`).
    pub id: String,
    /// User-facing name; matches the source identifier.
    pub name: String,
    /// Kind discriminator.
    pub kind: SymbolKind,
    /// Single-line signature reconstructed from the source.
    pub signature: String,
    /// Sorted effects the symbol participates in (`db.read`, `http`, ...).
    pub effects: Vec<String>,
    /// Source span of the declaration.
    pub span: Span,
}

/// Top-level container of every parsed declaration in one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Module {
    /// Fully qualified module name (e.g. `store.users`).
    pub name: String,
    /// On-disk path the module was read from.
    pub path: String,
    /// Imports declared at the top of the module, in source order.
    pub imports: Vec<String>,
    /// Declared symbols, beginning with the synthetic module symbol.
    pub symbols: Vec<Symbol>,
}

impl Module {
    /// Construct an empty module with a synthetic module symbol installed.
    pub fn new(name: impl Into<String>, path: impl Into<String>) -> Self {
        let name = name.into();
        let path = path.into();
        let module_symbol = Symbol {
            id: format!("mod:{name}"),
            name: name.clone(),
            kind: SymbolKind::Module,
            signature: format!("module {name}"),
            effects: Vec::new(),
            span: Span::dummy(path.clone()),
        };
        Self {
            name,
            path,
            imports: Vec::new(),
            symbols: vec![module_symbol],
        }
    }

    /// Yield every symbol that should be visible to consumers of the module:
    /// the synthetic module entry is skipped and conventional `_`-prefixed
    /// names are treated as private.
    pub fn exported_symbols(&self) -> impl Iterator<Item = &Symbol> {
        self.symbols
            .iter()
            .filter(|symbol| symbol.kind != SymbolKind::Module && !symbol.name.starts_with('_'))
    }
}
