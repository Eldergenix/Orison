//! Wasm-component manifest extraction (bootstrap).
//!
//! The bootstrap does not yet emit real WebAssembly. It does, however, emit
//! a stable manifest describing the *intended* component interface for a
//! module: exported functions, imported names, and the union of effects. We
//! treat this manifest as the authoritative contract surface that a later
//! codegen pass must honour.

use crate::ast::{Module, SymbolKind};
use crate::json::to_json;
use serde::Serialize;
use std::collections::BTreeSet;

/// Stable schema id for the wasm-component manifest envelope.
pub const WASM_COMPONENT_SCHEMA: &str = "ori.wasm_component.v1";
/// Build target name reported by the bootstrap (`wasm32-component`).
pub const WASM_BUILD_TARGET: &str = "wasm32-component";

/// Wasm-component manifest envelope.
#[derive(Debug, Serialize)]
pub struct WasmComponentManifest {
    /// Stable schema identifier.
    pub schema: &'static str,
    /// Module name.
    pub module: String,
    /// Optional world identifier.
    pub world: Option<String>,
    /// Exported symbols.
    pub exports: Vec<WasmExport>,
    /// Imported names.
    pub imports: Vec<WasmImport>,
    /// Union of effects observed on every symbol.
    pub capabilities: Vec<String>,
    /// Build target string surfaced in the manifest.
    pub build_target: &'static str,
}

/// One exported wasm-component name.
#[derive(Debug, Serialize)]
pub struct WasmExport {
    /// Export name.
    pub name: String,
    /// Symbol kind string.
    pub kind: String,
    /// Reconstructed signature.
    pub signature: String,
}

/// One imported wasm-component name.
#[derive(Debug, Serialize)]
pub struct WasmImport {
    /// Import name (mirrors the source module path).
    pub name: String,
    /// Import kind (always `module` in the bootstrap).
    pub kind: String,
    /// Source module path.
    pub source: String,
}

impl WasmComponentManifest {
    /// Render the manifest as canonical JSON.
    pub fn to_json(&self) -> String {
        to_json(self)
    }
}

/// Extract the wasm-component manifest for `module`.
pub fn build_wasm_component_manifest(module: &Module) -> WasmComponentManifest {
    let exports: Vec<WasmExport> = module
        .exported_symbols()
        .filter(|s| {
            matches!(
                s.kind,
                SymbolKind::Function | SymbolKind::Query | SymbolKind::Service
            )
        })
        .map(|s| WasmExport {
            name: s.name.clone(),
            kind: s.kind.as_str().to_string(),
            signature: s.signature.clone(),
        })
        .collect();

    let imports: Vec<WasmImport> = module
        .imports
        .iter()
        .map(|imp| WasmImport {
            name: imp.clone(),
            kind: "module".to_string(),
            source: imp.clone(),
        })
        .collect();

    let mut caps: BTreeSet<String> = BTreeSet::new();
    for sym in &module.symbols {
        for eff in &sym.effects {
            caps.insert(eff.clone());
        }
    }

    WasmComponentManifest {
        schema: WASM_COMPONENT_SCHEMA,
        module: module.name.clone(),
        world: Some(format!("{}-world", module.name.replace('.', "-"))),
        exports,
        imports,
        capabilities: caps.into_iter().collect(),
        build_target: WASM_BUILD_TARGET,
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
    fn collects_exports_and_imports() {
        let module = module_for(
            "module demo.a\nimport std.json\nfn hello() -> Unit uses log\nfn list() -> List[Str] uses db.read",
        );
        let manifest = build_wasm_component_manifest(&module);
        assert_eq!(manifest.module, "demo.a");
        let names: Vec<_> = manifest.exports.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"hello"));
        assert!(names.contains(&"list"));
        let import_names: Vec<_> = manifest.imports.iter().map(|i| i.name.as_str()).collect();
        assert!(import_names.contains(&"std.json"));
        assert!(manifest.capabilities.contains(&"log".to_string()));
        assert!(manifest.capabilities.contains(&"db.read".to_string()));
    }

    #[test]
    fn manifest_serialises_with_schema() {
        let module = module_for("module demo\nfn x() -> Unit");
        let json = build_wasm_component_manifest(&module).to_json();
        assert!(json.contains("\"schema\":\"ori.wasm_component.v1\""));
        assert!(json.contains("\"build_target\":\"wasm32-component\""));
    }
}
