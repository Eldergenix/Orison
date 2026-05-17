//! Mid-level IR (bootstrap subset).
//!
//! The bootstrap MIR is a thin wrapper around HIR plus a `body` field that
//! is currently a single opaque `Constant` instruction per function. The
//! shape is the contract: future lowering passes can fill the instruction
//! lists without changing the surface JSON used by tests and agents.

use crate::hir::{HirModule, HirParam};
use serde::{Deserialize, Serialize};

/// One MIR instruction. Bootstrap MIR only emits placeholder `const_default`
/// instructions; the shape is the contract, not the contents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MirInstruction {
    /// Opcode name.
    pub op: String,
    /// Operand list (encoded as strings in the bootstrap MIR).
    pub args: Vec<String>,
    /// Optional SSA name receiving the result.
    pub result: Option<String>,
}

/// One basic block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MirBlock {
    /// Sequential block id, unique per function.
    pub id: usize,
    /// Instructions in execution order.
    pub instructions: Vec<MirInstruction>,
}

/// One MIR function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MirFunction {
    /// Function name.
    pub name: String,
    /// Parameters copied from HIR.
    pub params: Vec<HirParam>,
    /// Declared return type.
    pub return_type: String,
    /// One or more basic blocks (entry block has id 0).
    pub blocks: Vec<MirBlock>,
}

/// MIR view of a whole module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MirModule {
    /// Module name.
    pub module: String,
    /// Functions, in HIR order.
    pub functions: Vec<MirFunction>,
}

/// Lower an [`HirModule`] to [`MirModule`].
pub fn lower_module(hir: &HirModule) -> MirModule {
    let functions = hir
        .items
        .iter()
        .filter(|i| i.kind == "function")
        .map(|i| {
            let placeholder = MirInstruction {
                op: "const_default".to_string(),
                args: vec![i.return_type.clone()],
                result: Some(format!("%ret:{}", i.name)),
            };
            MirFunction {
                name: i.name.clone(),
                params: i.params.clone(),
                return_type: i.return_type.clone(),
                blocks: vec![MirBlock {
                    id: 0,
                    instructions: vec![placeholder],
                }],
            }
        })
        .collect();
    MirModule {
        module: hir.module.clone(),
        functions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::lower_module as lower_hir;
    use crate::parser::parse_source;
    use crate::source::SourceFile;

    #[test]
    fn lowers_single_function_to_mir() {
        let module = parse_source(&SourceFile::new("/t.ori", "module a\nfn f() -> Int")).module;
        let mir = lower_module(&lower_hir(&module));
        assert_eq!(mir.module, "a");
        let f = mir.functions.iter().find(|f| f.name == "f");
        assert!(f.is_some());
        let blocks_len = mir
            .functions
            .iter()
            .find(|f| f.name == "f")
            .map(|f| f.blocks.len())
            .unwrap_or(0);
        assert_eq!(blocks_len, 1);
    }
}
