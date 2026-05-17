//! Typed High-level Intermediate Representation (bootstrap subset).
//!
//! The bootstrap HIR captures only what the surface parser can faithfully
//! recover from a `.ori` source: items (functions, types, services, views,
//! ...) with parameters and return types, plus the effect set declared on
//! each function. It is intentionally lossy with respect to expression
//! bodies because the bootstrap parser does not yet read function bodies.
//!
//! Even at this fidelity HIR is useful: it is what downstream IRs and the
//! interpreter lower from, and it gives agent tools a stable shape for
//! "explain this symbol" requests.

use crate::ast::{Module, SymbolKind};
use crate::node_id::NodeId;
use serde::{Deserialize, Serialize};

/// One HIR item, derived from a parsed [`crate::ast::Symbol`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirItem {
    /// Stable symbol id (`sym:module.name`).
    pub id: String,
    /// Optional stable node id when known.
    pub node_id: Option<NodeId>,
    /// Symbol kind string (matches [`crate::ast::SymbolKind::as_str`]).
    pub kind: String,
    /// Declared name.
    pub name: String,
    /// Parameter list parsed from the signature.
    pub params: Vec<HirParam>,
    /// Declared return type (defaults to `Unit` when absent).
    pub return_type: String,
    /// Sorted effect list copied from the source symbol.
    pub effects: Vec<String>,
}

/// One parameter in an [`HirItem`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirParam {
    /// Binding name.
    pub name: String,
    /// Declared type, verbatim from the signature.
    pub r#type: String,
}

/// HIR view of an entire module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirModule {
    /// Fully qualified module name.
    pub module: String,
    /// One [`HirItem`] per non-module symbol, in source order.
    pub items: Vec<HirItem>,
}

/// Lower a parsed [`Module`] to [`HirModule`].
pub fn lower_module(module: &Module) -> HirModule {
    let items = module
        .symbols
        .iter()
        .filter(|s| s.kind != SymbolKind::Module)
        .map(|s| HirItem {
            id: s.id.clone(),
            node_id: None,
            kind: s.kind.as_str().to_string(),
            name: s.name.clone(),
            params: parse_params(&s.signature),
            return_type: parse_return(&s.signature),
            effects: s.effects.clone(),
        })
        .collect();
    HirModule {
        module: module.name.clone(),
        items,
    }
}

fn parse_params(signature: &str) -> Vec<HirParam> {
    let open = match signature.find('(') {
        Some(idx) => idx,
        None => return Vec::new(),
    };
    let close = match signature[open..].find(')') {
        Some(off) => open + off,
        None => return Vec::new(),
    };
    let body = &signature[open + 1..close];
    if body.trim().is_empty() {
        return Vec::new();
    }
    body.split(',')
        .filter_map(|part| {
            let part = part.trim();
            if let Some(colon) = part.find(':') {
                let name = part[..colon].trim().to_string();
                let ty = part[colon + 1..].trim().to_string();
                if name.is_empty() || ty.is_empty() {
                    None
                } else {
                    Some(HirParam { name, r#type: ty })
                }
            } else {
                None
            }
        })
        .collect()
}

fn parse_return(signature: &str) -> String {
    if let Some(idx) = signature.find("->") {
        let after = signature[idx + 2..].trim();
        let cutoff = after.find(" uses ").unwrap_or(after.len());
        after[..cutoff].trim().to_string()
    } else {
        "Unit".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_source;
    use crate::source::SourceFile;

    fn module_for(text: &str) -> Module {
        parse_source(&SourceFile::new("/t.ori", text)).module
    }

    #[test]
    fn lowers_function_with_params_and_return() {
        let module = module_for("module demo\nfn add(a: Int, b: Int) -> Int");
        let hir = lower_module(&module);
        let item_count = hir.items.iter().filter(|i| i.name == "add").count();
        assert_eq!(item_count, 1);
        let add = hir.items.iter().find(|i| i.name == "add");
        assert!(add.is_some());
        let add_params_len = hir
            .items
            .iter()
            .find(|i| i.name == "add")
            .map(|i| i.params.len())
            .unwrap_or(0);
        assert_eq!(add_params_len, 2);
        let add_return = hir
            .items
            .iter()
            .find(|i| i.name == "add")
            .map(|i| i.return_type.clone())
            .unwrap_or_default();
        assert_eq!(add_return, "Int");
    }

    #[test]
    fn strips_uses_from_return_type() {
        let module = module_for("module demo\nfn ping() -> Bool uses net.outbound");
        let hir = lower_module(&module);
        let ret = hir
            .items
            .iter()
            .find(|i| i.name == "ping")
            .map(|i| i.return_type.clone())
            .unwrap_or_default();
        assert_eq!(ret, "Bool");
    }
}
