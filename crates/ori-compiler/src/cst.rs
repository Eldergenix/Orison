//! Error-tolerant concrete syntax tree.
//!
//! The bootstrap CST is intentionally small: it groups the existing token
//! stream into top-level "items" (module, import, fn, type, service, view,
//! actor, query, migration, capability) plus interleaved trivia (comments,
//! blank lines). Each item carries a stable [`NodeId`] so that structural
//! patches (`patch_apply`) target nodes by identity rather than by line/col.
//!
//! When the parser cannot recognise an item it records the offending token
//! range as a `CstNodeKind::Error` so downstream tools can still surface the
//! file structure for partial editing.

use crate::lexer::{lex, Token, TokenKind};
use crate::node_id::{make_node_id, NodeId};
use crate::source::{SourceFile, Span};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CstNodeKind {
    Module,
    Import,
    Function,
    Type,
    Service,
    View,
    Actor,
    Query,
    Migration,
    Capability,
    Comment,
    BlankLine,
    Error,
}

impl CstNodeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            CstNodeKind::Module => "module",
            CstNodeKind::Import => "import",
            CstNodeKind::Function => "fn",
            CstNodeKind::Type => "type",
            CstNodeKind::Service => "service",
            CstNodeKind::View => "view",
            CstNodeKind::Actor => "actor",
            CstNodeKind::Query => "query",
            CstNodeKind::Migration => "migration",
            CstNodeKind::Capability => "capability",
            CstNodeKind::Comment => "comment",
            CstNodeKind::BlankLine => "blank",
            CstNodeKind::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CstNode {
    pub id: NodeId,
    pub kind: CstNodeKind,
    pub name: String,
    pub signature: String,
    pub span: Span,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cst {
    pub module_name: String,
    pub path: String,
    pub nodes: Vec<CstNode>,
}

impl Cst {
    pub fn find(&self, id: &str) -> Option<&CstNode> {
        self.nodes.iter().find(|node| node.id.as_str() == id)
    }
}

/// Parse a source file into an error-tolerant CST. Comments and blank lines
/// are preserved as `Comment` / `BlankLine` nodes so the formatter can
/// reconstruct the original layout.
pub fn parse_cst(source: &SourceFile) -> Cst {
    let tokens = lex(source);
    let module_name = detect_module_name(source, &tokens);
    let lines: Vec<&str> = source.text.lines().collect();

    let mut nodes: Vec<CstNode> = Vec::new();
    let mut sibling_counters: std::collections::BTreeMap<&'static str, usize> =
        std::collections::BTreeMap::new();

    // First pass: emit trivia (comments + blank lines) keyed by line number.
    for (idx, line) in lines.iter().enumerate() {
        let line_no = idx + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            nodes.push(CstNode {
                id: make_node_id(&module_name, None, "blank", "", line_no, ""),
                kind: CstNodeKind::BlankLine,
                name: String::new(),
                signature: String::new(),
                span: Span::new(source.path.clone(), line_no, 1, line_no, 1),
                text: String::new(),
            });
            continue;
        }
        if trimmed.starts_with("//") {
            nodes.push(CstNode {
                id: make_node_id(&module_name, None, "comment", "", line_no, trimmed),
                kind: CstNodeKind::Comment,
                name: String::new(),
                signature: String::new(),
                span: Span::new(source.path.clone(), line_no, 1, line_no, line.len() + 1),
                text: (*line).to_string(),
            });
        }
    }

    // Second pass: walk tokens to find item-introducing keywords.
    let mut i = 0usize;
    while i < tokens.len() {
        let token = &tokens[i];
        let kind = item_kind_for(token);
        if let Some(kind) = kind {
            let line = token.span.start.line;
            let (name, signature) = collect_item(&tokens, i);
            let key = kind.as_str();
            let sibling = sibling_counters
                .entry(key)
                .and_modify(|n| *n += 1)
                .or_insert(0);
            let id = make_node_id(&module_name, None, key, &name, *sibling, &signature);
            let text = lines
                .get(line.saturating_sub(1))
                .copied()
                .unwrap_or("")
                .to_string();
            nodes.push(CstNode {
                id,
                kind,
                name,
                signature,
                span: Span::new(
                    source.path.clone(),
                    line,
                    token.span.start.column,
                    line,
                    text.len() + 1,
                ),
                text,
            });
            // Skip rest of this line
            while i < tokens.len() && tokens[i].span.start.line == line {
                i += 1;
            }
            continue;
        }
        i += 1;
    }

    // Sort by start line so the CST mirrors source order.
    nodes.sort_by_key(|node| node.span.start.line);

    Cst {
        module_name,
        path: source.path.clone(),
        nodes,
    }
}

fn detect_module_name(source: &SourceFile, tokens: &[Token]) -> String {
    for (idx, token) in tokens.iter().enumerate() {
        if token.kind == TokenKind::Keyword && token.lexeme == "module" {
            let mut name = String::new();
            let mut expect_ident = true;
            let mut j = idx + 1;
            while j < tokens.len() {
                let t = &tokens[j];
                match (t.kind, t.lexeme.as_str(), expect_ident) {
                    (TokenKind::Ident, _, true) => {
                        name.push_str(&t.lexeme);
                        expect_ident = false;
                    }
                    (TokenKind::Symbol, ".", false) => {
                        name.push('.');
                        expect_ident = true;
                    }
                    _ => break,
                }
                j += 1;
            }
            if !name.is_empty() && !name.ends_with('.') {
                return name;
            }
        }
    }
    module_name_from_path(&source.path)
}

fn module_name_from_path(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    let stem = file.strip_suffix(".ori").unwrap_or(file);
    stem.replace('-', "_")
}

fn item_kind_for(token: &Token) -> Option<CstNodeKind> {
    if token.kind != TokenKind::Keyword {
        return None;
    }
    Some(match token.lexeme.as_str() {
        "module" => CstNodeKind::Module,
        "import" => CstNodeKind::Import,
        "fn" => CstNodeKind::Function,
        "type" => CstNodeKind::Type,
        "service" => CstNodeKind::Service,
        "view" => CstNodeKind::View,
        "actor" => CstNodeKind::Actor,
        "query" => CstNodeKind::Query,
        "migration" => CstNodeKind::Migration,
        "capability" => CstNodeKind::Capability,
        _ => return None,
    })
}

fn collect_item(tokens: &[Token], start: usize) -> (String, String) {
    let line = tokens[start].span.start.line;
    let mut name = String::new();
    if let Some(next) = tokens.get(start + 1) {
        if matches!(next.kind, TokenKind::Ident) && next.span.start.line == line {
            name = next.lexeme.clone();
        }
    }
    let mut parts = Vec::new();
    let mut i = start;
    while i < tokens.len() && tokens[i].span.start.line == line && tokens[i].kind != TokenKind::Eof
    {
        parts.push(tokens[i].lexeme.clone());
        i += 1;
    }
    let signature = compact_signature(&parts);
    (name, signature)
}

fn compact_signature(parts: &[String]) -> String {
    let mut out = String::new();
    for part in parts {
        let no_space_before = matches!(part.as_str(), ")" | "]" | "," | ":" | "." | "(" | "[");
        let no_space_after_prev = out.ends_with('(') || out.ends_with('[') || out.ends_with('.');
        if !out.is_empty() && !no_space_before && !no_space_after_prev {
            out.push(' ');
        }
        out.push_str(part);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cst_for(text: &str) -> Cst {
        parse_cst(&SourceFile::new("/t.ori", text))
    }

    #[test]
    fn picks_up_module_and_items() {
        let cst = cst_for("module demo.a\nimport std.json\nfn hello() -> Unit\ntype Foo");
        assert_eq!(cst.module_name, "demo.a");
        let kinds: Vec<_> = cst
            .nodes
            .iter()
            .map(|node| node.kind)
            .filter(|k| !matches!(k, CstNodeKind::BlankLine | CstNodeKind::Comment))
            .collect();
        assert!(kinds.contains(&CstNodeKind::Module));
        assert!(kinds.contains(&CstNodeKind::Import));
        assert!(kinds.contains(&CstNodeKind::Function));
        assert!(kinds.contains(&CstNodeKind::Type));
    }

    #[test]
    fn comments_are_preserved() {
        let cst = cst_for("module a\n// note\nfn f() -> Unit");
        assert!(cst.nodes.iter().any(|n| n.kind == CstNodeKind::Comment));
    }

    #[test]
    fn blank_lines_are_preserved() {
        let cst = cst_for("module a\n\nfn f() -> Unit");
        assert!(cst.nodes.iter().any(|n| n.kind == CstNodeKind::BlankLine));
    }

    #[test]
    fn node_ids_are_stable_for_same_input() {
        let cst_a = cst_for("module a\nfn f() -> Unit\nfn g() -> Int");
        let cst_b = cst_for("module a\nfn f() -> Unit\nfn g() -> Int");
        let ids_a: Vec<_> = cst_a.nodes.iter().map(|n| n.id.clone()).collect();
        let ids_b: Vec<_> = cst_b.nodes.iter().map(|n| n.id.clone()).collect();
        assert_eq!(ids_a, ids_b);
    }

    #[test]
    fn node_id_for_fn_survives_adding_unrelated_blank_line() {
        let cst_a = cst_for("module a\nfn keep() -> Unit");
        let cst_b = cst_for("module a\n\n\nfn keep() -> Unit");
        let id_a = cst_a
            .nodes
            .iter()
            .find(|n| n.kind == CstNodeKind::Function && n.name == "keep")
            .map(|n| n.id.clone());
        let id_b = cst_b
            .nodes
            .iter()
            .find(|n| n.kind == CstNodeKind::Function && n.name == "keep")
            .map(|n| n.id.clone());
        assert!(id_a.is_some(), "fn `keep` not found in cst_a");
        assert_eq!(id_a, id_b);
    }

    #[test]
    fn duplicate_names_get_distinct_ids() {
        let cst = cst_for("module a\nfn dup() -> Unit\nfn dup() -> Int");
        let dups: Vec<_> = cst
            .nodes
            .iter()
            .filter(|n| n.kind == CstNodeKind::Function && n.name == "dup")
            .collect();
        assert_eq!(dups.len(), 2);
        assert_ne!(dups[0].id, dups[1].id);
    }
}
