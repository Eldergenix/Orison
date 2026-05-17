//! Convenience helpers for looking up symbols by id or name.

use crate::ast::{Module, Symbol};

/// Look up a symbol by either its stable id (`sym:foo.bar`) or its bare
/// name. Returns the first match in declaration order.
pub fn find_symbol<'a>(module: &'a Module, id_or_name: &str) -> Option<&'a Symbol> {
    module
        .symbols
        .iter()
        .find(|sym| sym.id == id_or_name || sym.name == id_or_name)
}

/// Count exported symbols (everything except the synthetic module symbol
/// and `_`-prefixed names).
pub fn public_symbol_count(module: &Module) -> usize {
    module.exported_symbols().count()
}
