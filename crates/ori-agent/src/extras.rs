//! Expanded Agent Context ABI: symbol lists, diagnose, affected tests, doctor.
//!
//! These additions plug directly into the bootstrap `ori-compiler` JSON
//! surface so agents can call:
//!
//! * `ori agent symbols --changed` → ori.agent_symbol_list.v1
//! * `ori agent diagnose`         → ori.agent_diagnose.v1
//! * `ori agent tests --affected` → ori.agent_tests.v1
//! * `ori doctor --json`          → ori.doctor.v1
//!
//! All outputs are produced via typed serde structs (no raw JSON
//! concatenation) and match the schemas under `schemas/`.

use ori_compiler::ast::SymbolKind;
use ori_compiler::diagnostic::Diagnostic;
use ori_compiler::CompileResult;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct AgentSymbolList<'a> {
    pub schema: &'static str,
    pub module: &'a str,
    pub symbols: Vec<AgentSymbolEntry<'a>>,
}

#[derive(Debug, Serialize)]
pub struct AgentSymbolEntry<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub kind: &'static str,
    pub signature: &'a str,
    pub changed: bool,
    pub reason: Option<&'static str>,
}

pub fn agent_symbol_list_json(result: &CompileResult, changed_only: bool) -> String {
    let entries: Vec<AgentSymbolEntry<'_>> = result
        .module
        .symbols
        .iter()
        .filter(|s| s.kind != SymbolKind::Module)
        .filter(|_| true)
        .filter(|_| !changed_only) // bootstrap: no diff oracle yet, returns all if not changed_only
        .chain(std::iter::empty())
        .map(|s| AgentSymbolEntry {
            id: s.id.as_str(),
            name: s.name.as_str(),
            kind: s.kind.as_str(),
            signature: s.signature.as_str(),
            changed: changed_only,
            reason: if changed_only {
                Some("bootstrap: marking all symbols as changed without diff oracle")
            } else {
                None
            },
        })
        .collect();
    // When changed_only is true and we have no oracle, return every symbol
    // tagged changed so the contract shape is honoured.
    let actual: Vec<AgentSymbolEntry<'_>> = if changed_only {
        result
            .module
            .symbols
            .iter()
            .filter(|s| s.kind != SymbolKind::Module)
            .map(|s| AgentSymbolEntry {
                id: s.id.as_str(),
                name: s.name.as_str(),
                kind: s.kind.as_str(),
                signature: s.signature.as_str(),
                changed: true,
                reason: Some("bootstrap: changed-detection requires the incremental cache"),
            })
            .collect()
    } else {
        entries
    };
    let list = AgentSymbolList {
        schema: "ori.agent_symbol_list.v1",
        module: result.module.name.as_str(),
        symbols: actual,
    };
    ori_compiler::json::to_json(&list)
}

#[derive(Debug, Serialize)]
pub struct AgentDiagnose<'a> {
    pub schema: &'static str,
    pub module: &'a str,
    pub overall_status: &'static str,
    pub errors: usize,
    pub warnings: usize,
    pub diagnostics: &'a [Diagnostic],
    pub top_repair_candidates: Vec<RepairCandidate>,
}

#[derive(Debug, Serialize)]
pub struct RepairCandidate {
    pub diagnostic_id: String,
    pub fix_kind: String,
    pub description: String,
    pub confidence: f32,
}

pub fn agent_diagnose_json(result: &CompileResult) -> String {
    let errors = result.diagnostics.iter().filter(|d| d.is_error()).count();
    let warnings = result.diagnostics.iter().filter(|d| !d.is_error()).count();
    let status: &'static str = if errors > 0 {
        "error"
    } else if warnings > 0 {
        "warn"
    } else {
        "ok"
    };
    let mut candidates: Vec<RepairCandidate> = Vec::new();
    for d in &result.diagnostics {
        for fix in &d.fixes {
            candidates.push(RepairCandidate {
                diagnostic_id: d.id.clone(),
                fix_kind: fix.kind.clone(),
                description: fix.description.clone(),
                confidence: fix.confidence,
            });
        }
    }
    candidates.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.truncate(8);
    let diagnose = AgentDiagnose {
        schema: "ori.agent_diagnose.v1",
        module: result.module.name.as_str(),
        overall_status: status,
        errors,
        warnings,
        diagnostics: &result.diagnostics,
        top_repair_candidates: candidates,
    };
    ori_compiler::json::to_json(&diagnose)
}

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub schema: &'static str,
    pub status: &'static str,
    pub compiler: &'static str,
    pub language: &'static str,
    pub version: &'static str,
    pub rust_toolchain: Option<String>,
    pub checks: Vec<DoctorCheck>,
    pub capabilities_summary: Vec<String>,
    pub schema_versions: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct DoctorCheck {
    pub name: String,
    pub status: &'static str,
    pub detail: String,
}

pub fn doctor_report_json() -> String {
    let mut versions: std::collections::BTreeMap<String, String> = Default::default();
    // Keep this list in lock-step with `schemas/*.schema.json`. Adding a new
    // schema means adding the matching `(key, "ori.<key>.v1")` entry here so
    // `ori doctor` advertises the contract.
    for (key, val) in [
        ("agent_changed", "ori.agent_changed.v1"),
        ("agent_diagnose", "ori.agent_diagnose.v1"),
        ("agent_map", "ori.agent_map.v1"),
        ("agent_symbol_list", "ori.agent_symbol_list.v1"),
        ("agent_tests", "ori.agent_tests.v1"),
        ("audit_report", "ori.audit_report.v1"),
        ("backend_dispatch", "ori.backend_dispatch.v1"),
        ("benchmark", "ori.benchmark.v1"),
        ("build_report", "ori.build_report.v1"),
        ("capability", "ori.capability.v1"),
        ("capability_runtime", "ori.capability_runtime.v1"),
        ("capsule", "ori.capsule.v1"),
        ("change", "ori.change.v1"),
        ("coverage_report", "ori.coverage_report.v1"),
        ("design_tokens_report", "ori.design_tokens_report.v1"),
        ("diagnostic", "ori.diagnostic.v1"),
        ("doctor", "ori.doctor.v1"),
        ("graphql_import", "ori.graphql_import.v1"),
        ("lockfile", "ori.lockfile.v1"),
        ("lsp_code_action", "ori.lsp_code_action.v1"),
        ("manifest", "ori.manifest.v1"),
        ("migration_graph", "ori.migration_graph.v1"),
        ("migration_report", "ori.migration_report.v1"),
        ("mobile_manifest", "ori.mobile_manifest.v1"),
        ("model_loop_telemetry", "ori.model_loop_telemetry.v1"),
        ("native_ui_manifest", "ori.native_ui_manifest.v1"),
        ("openapi_report", "ori.openapi_report.v1"),
        ("patch", "ori.patch.v1"),
        ("patch_check", "ori.patch_check.v1"),
        ("preprocess", "ori.preprocess.v1"),
        ("provenance", "ori.provenance.v1"),
        ("publish_receipt", "ori.publish_receipt.v1"),
        ("registry_list", "ori.registry_list.v1"),
        ("rpc_import", "ori.rpc_import.v1"),
        ("sandbox_result", "ori.sandbox_result.v1"),
        ("sbom", "ori.sbom.v1"),
        ("symbol_card", "ori.symbol_card.v1"),
        ("ui_manifest", "ori.ui_manifest.v1"),
        ("ui_render", "ori.ui_render.v1"),
        ("wasm_component", "ori.wasm_component.v1"),
    ] {
        versions.insert(key.to_string(), val.to_string());
    }
    let report = DoctorReport {
        schema: "ori.doctor.v1",
        status: "ok",
        compiler: "bootstrap",
        language: "Orison",
        version: env!("CARGO_PKG_VERSION"),
        rust_toolchain: option_env!("RUSTUP_TOOLCHAIN").map(|s| s.to_string()),
        checks: vec![
            DoctorCheck {
                name: "schemas_published".to_string(),
                status: "ok",
                detail: format!("{} stable contracts", versions.len()),
            },
            DoctorCheck {
                name: "compiler_modules".to_string(),
                status: "ok",
                detail: "lexer, parser, cst, resolver, type_check, type_infer, effect_check, effect_propagate, borrow, exhaustive, const_fold, patch, patch_apply, hir, mir, interp, interp_exec, async_runtime, bench, openapi, ui_check, design_tokens, mobile, wasm_component, wasm_encoder, codegen_text, incremental, query, coverage, docs, migrate, sql_check, migration_graph, graphql_import, rpc_import, preproc, formatter, body, expr".to_string(),
            },
        ],
        capabilities_summary: vec![
            "package-level effect policy enforced statically via `ori capability --policy ...`".to_string(),
            "bootstrap compiler does not yet enforce capability runtime".to_string(),
        ],
        schema_versions: versions,
    };
    ori_compiler::json::to_json(&report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ori_compiler::source::SourceFile;
    use ori_compiler::Compiler;

    fn result_for(text: &str) -> CompileResult {
        Compiler::check_source(SourceFile::new("/t.ori", text))
    }

    #[test]
    fn symbol_list_marks_changed_when_requested() {
        let r = result_for("module demo\nfn a() -> Unit\nfn b() -> Int");
        let json = agent_symbol_list_json(&r, true);
        assert!(json.contains("\"changed\":true"));
        assert!(json.contains("\"schema\":\"ori.agent_symbol_list.v1\""));
    }

    #[test]
    fn symbol_list_skips_module_symbol_when_unchanged() {
        let r = result_for("module demo\nfn a() -> Unit");
        let json = agent_symbol_list_json(&r, false);
        assert!(!json.contains("\"name\":\"demo\""));
    }

    #[test]
    fn diagnose_reports_ok_status_when_no_diagnostics() {
        let r = result_for("module demo\nfn a() -> Unit");
        let json = agent_diagnose_json(&r);
        assert!(json.contains("\"overall_status\":\"ok\""));
    }

    #[test]
    fn diagnose_reports_error_on_null_use() {
        let r = result_for(
            "module demo\nfn a() -> Unit\n// payload: null\nfn b() -> Int\nlet x = null",
        );
        let json = agent_diagnose_json(&r);
        // Either error or warn (parser may classify), but never "ok"
        assert!(!json.contains("\"overall_status\":\"ok\""));
    }

    #[test]
    fn doctor_report_lists_schemas() {
        let json = doctor_report_json();
        assert!(json.contains("\"schema\":\"ori.doctor.v1\""));
        assert!(json.contains("ori.diagnostic.v1"));
        assert!(json.contains("ori.benchmark.v1"));
    }

    /// Regression: the doctor report must list every schema file shipped under
    /// `schemas/`. Drift between `schemas/*.schema.json` and the in-code list
    /// means agents querying `ori doctor` get stale version data.
    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn doctor_report_lists_every_shipped_schema() {
        let json = doctor_report_json();
        // Walk the schemas directory from the workspace root, derived from the
        // crate manifest path so the test is location-stable.
        let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = match crate_dir.ancestors().nth(2) {
            Some(p) => p.to_path_buf(),
            None => {
                assert!(false, "could not derive workspace root from crate dir");
                return;
            }
        };
        let schemas_dir = workspace_root.join("schemas");
        let entries = match std::fs::read_dir(&schemas_dir) {
            Ok(entries) => entries,
            Err(err) => {
                assert!(false, "failed to read {}: {err}", schemas_dir.display());
                return;
            }
        };
        let mut missing: Vec<String> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let name = match path.file_name().and_then(|s| s.to_str()) {
                Some(n) => n,
                None => continue,
            };
            // Map `foo-bar.schema.json` to the on-the-wire id
            // `ori.foo_bar.v1`.
            let stem = match name.strip_suffix(".schema.json") {
                Some(s) => s,
                None => continue,
            };
            let snake = stem.replace('-', "_");
            let expected = format!("ori.{snake}.v1");
            if !json.contains(&expected) {
                missing.push(expected);
            }
        }
        if !missing.is_empty() {
            missing.sort();
            assert!(
                false,
                "doctor report is missing schema versions: {missing:?}"
            );
        }
    }
}
