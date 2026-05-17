//! In-memory workspace tracking for LSP-tracked documents.
//!
//! The server keeps the most recently synchronised text for every open URI so
//! it can re-run the compiler whenever a request arrives. The state is owned
//! by [`crate::server::Server`] and is not shared across threads — the LSP
//! base protocol guarantees in-order delivery on a single stream.

use std::collections::BTreeMap;

/// One synchronised text document.
#[derive(Debug, Clone)]
pub struct DocumentState {
    /// URI the client opened this document under.
    pub uri: String,
    /// Most recently synchronised full text.
    pub text: String,
    /// Client-supplied version counter, propagated through diagnostics.
    pub version: i64,
}

impl DocumentState {
    /// Construct a new tracked document.
    pub fn new(uri: impl Into<String>, text: impl Into<String>, version: i64) -> Self {
        Self {
            uri: uri.into(),
            text: text.into(),
            version,
        }
    }
}

/// Collection of all documents the server currently knows about.
///
/// Backed by a [`BTreeMap`] so iteration is deterministic across runs — both
/// `workspace/symbol` results and any other workspace-wide enumeration depend
/// on a stable order.
#[derive(Debug, Default)]
pub struct WorkspaceState {
    documents: BTreeMap<String, DocumentState>,
}

impl WorkspaceState {
    /// Create an empty workspace.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a document or replaces an existing entry with the same URI.
    pub fn open(&mut self, uri: impl Into<String>, text: impl Into<String>, version: i64) {
        let uri = uri.into();
        self.documents
            .insert(uri.clone(), DocumentState::new(uri, text, version));
    }

    /// Updates the text of an open document. Creates the document if it is
    /// not yet open — clients that send `didChange` before `didOpen` are
    /// non-compliant, but we tolerate it to avoid losing edits.
    pub fn update(&mut self, uri: &str, new_text: impl Into<String>, version: i64) {
        match self.documents.get_mut(uri) {
            Some(doc) => {
                doc.text = new_text.into();
                doc.version = version;
            }
            None => {
                self.documents
                    .insert(uri.to_string(), DocumentState::new(uri, new_text, version));
            }
        }
    }

    /// Removes a document from the workspace, returning the previous state.
    pub fn close(&mut self, uri: &str) -> Option<DocumentState> {
        self.documents.remove(uri)
    }

    /// Returns the tracked document for the given URI, if any.
    pub fn get(&self, uri: &str) -> Option<&DocumentState> {
        self.documents.get(uri)
    }

    /// Iterates over every open document in URI order. Used by workspace-wide
    /// LSP requests such as `workspace/symbol`,
    /// `textDocument/definition`, and `textDocument/references`.
    pub fn iter(&self) -> impl Iterator<Item = &DocumentState> {
        self.documents.values()
    }

    /// Iterate `(uri, document)` pairs in lexicographic URI order. Callers
    /// that need a stable ordering (responses, snapshots, tests) should use
    /// this in preference to [`Self::iter`].
    pub fn iter_sorted(&self) -> impl Iterator<Item = (&str, &DocumentState)> {
        self.documents.iter().map(|(uri, doc)| (uri.as_str(), doc))
    }

    /// Number of currently open documents.
    pub fn len(&self) -> usize {
        self.documents.len()
    }

    /// `true` when no document is currently tracked.
    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }
}
