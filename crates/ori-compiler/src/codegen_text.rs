//! Textual pseudo-IR emitter (bootstrap stand-in for native AOT).
//!
//! This module produces a deterministic, human-readable text artefact that
//! *looks like* LLVM IR but is intentionally not consumable by any LLVM
//! toolchain. The goal is to prove the codegen pipeline shape (`MirModule
//! -> textual artefact`) without taking on a native-codegen dependency in
//! the bootstrap. A later M10 pass can swap this implementation for a real
//! LLVM/MIR backend without changing the call sites.
//!
//! Output schema:
//!   - `; ModuleID = '<module>'` (header line)
//!   - `; ori.codegen_text.v1` (schema tag for downstream tools)
//!   - one blank separator line
//!   - per function, in source order: `define i32 @<name>() {`, `entry:`,
//!     `  ret i32 0`, `}`, followed by one blank line.
//!
//! The trailing newline is always present so file writers do not need to
//! special-case it.

use crate::mir::MirModule;

/// Emit deterministic textual IR for the given MIR module.
pub fn emit_textual_ir(mir: &MirModule) -> String {
    let mut out = String::new();
    out.push_str("; ModuleID = '");
    out.push_str(&mir.module);
    out.push_str("'\n");
    out.push_str("; ori.codegen_text.v1\n");
    out.push('\n');

    for func in &mir.functions {
        out.push_str("define i32 @");
        out.push_str(&func.name);
        out.push_str("() {\n");
        out.push_str("entry:\n");
        out.push_str("  ret i32 0\n");
        out.push_str("}\n");
        out.push('\n');
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::HirParam;
    use crate::mir::{MirBlock, MirFunction, MirInstruction};

    fn mir(name: &str, funcs: Vec<MirFunction>) -> MirModule {
        MirModule {
            module: name.to_string(),
            functions: funcs,
        }
    }

    fn func(name: &str) -> MirFunction {
        MirFunction {
            name: name.to_string(),
            params: Vec::<HirParam>::new(),
            return_type: "Int".to_string(),
            blocks: vec![MirBlock {
                id: 0,
                instructions: vec![MirInstruction {
                    op: "const_default".to_string(),
                    args: vec!["Int".to_string()],
                    result: Some(format!("%ret:{name}")),
                }],
            }],
        }
    }

    #[test]
    fn empty_module_emits_header_only() {
        let text = emit_textual_ir(&mir("empty", Vec::new()));
        let expected = "; ModuleID = 'empty'\n; ori.codegen_text.v1\n\n";
        assert_eq!(text, expected);
    }

    #[test]
    fn single_function_emits_define_block() {
        let text = emit_textual_ir(&mir("demo", vec![func("main")]));
        let expected_lines: Vec<&str> = vec![
            "; ModuleID = 'demo'",
            "; ori.codegen_text.v1",
            "",
            "define i32 @main() {",
            "entry:",
            "  ret i32 0",
            "}",
            "",
        ];
        let expected = format!("{}\n", expected_lines.join("\n"));
        assert_eq!(text, expected);
    }

    #[test]
    fn multiple_functions_preserve_source_order() {
        let text = emit_textual_ir(&mir("multi", vec![func("alpha"), func("beta")]));
        let alpha_idx = text.find("@alpha");
        let beta_idx = text.find("@beta");
        assert!(alpha_idx.is_some());
        assert!(beta_idx.is_some());
        assert!(alpha_idx < beta_idx, "alpha must appear before beta");
    }

    #[test]
    fn output_is_deterministic() {
        let m = mir("repeat", vec![func("a"), func("b")]);
        assert_eq!(emit_textual_ir(&m), emit_textual_ir(&m));
    }

    #[test]
    fn output_includes_schema_tag() {
        let text = emit_textual_ir(&mir("any", Vec::new()));
        assert!(text.contains("ori.codegen_text.v1"));
    }

    #[test]
    fn output_always_ends_with_newline() {
        let with_funcs = emit_textual_ir(&mir("a", vec![func("f")]));
        let without_funcs = emit_textual_ir(&mir("a", Vec::new()));
        assert!(with_funcs.ends_with('\n'));
        assert!(without_funcs.ends_with('\n'));
    }
}
