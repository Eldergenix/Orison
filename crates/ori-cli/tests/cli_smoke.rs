//! End-to-end smoke tests for the `ori` CLI.
//!
//! Every subcommand surfaced by `print_help` should have at least one test
//! here that:
//!   1. invokes it with `--json`,
//!   2. asserts exit code 0 (or a documented non-zero such as 1 for the
//!      `doctor`-with-warnings case),
//!   3. parses the output as JSON, and
//!   4. asserts the envelope's `schema` field matches the published
//!      `ori.<name>.v1` id from `schemas/<name>.schema.json`.
//!
//! The tests build the release binary once (cargo's caching makes this
//! cheap after the first run) and re-use it for every case. They depend
//! on the example apps under `examples/` and the local manifest at
//! `ori.toml`, both committed to the repository.

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Locate the workspace root by walking up from `CARGO_MANIFEST_DIR`. We
/// stop at the first ancestor that contains the workspace `Cargo.toml`
/// declaring the top-level `[workspace]` block.
fn workspace_root() -> PathBuf {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for ancestor in crate_dir.ancestors() {
        let cargo_toml = ancestor.join("Cargo.toml");
        if cargo_toml.exists() {
            if let Ok(text) = std::fs::read_to_string(&cargo_toml) {
                if text.contains("[workspace]") {
                    return ancestor.to_path_buf();
                }
            }
        }
    }
    crate_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or(crate_dir)
}

/// Path to the release `ori` binary. Tests build it on demand via `cargo
/// build --release -p ori` if it does not already exist.
fn ori_binary() -> PathBuf {
    let root = workspace_root();
    let bin = root.join("target").join("release").join("ori");
    if !bin.exists() {
        // Build once. cargo's caching means subsequent test runs skip this.
        let status = Command::new(env!("CARGO"))
            .arg("build")
            .arg("--release")
            .arg("-p")
            .arg("ori")
            .current_dir(&root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if status.is_err() || !bin.exists() {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "could not build release `ori` binary");
            }
        }
    }
    bin
}

/// Run `ori <args>` from the workspace root and return (exit_code,
/// stdout_text). Stdout is interpreted as UTF-8; non-UTF-8 bytes
/// produce a clean assertion failure rather than panicking.
fn run(args: &[&str]) -> (i32, String) {
    let bin = ori_binary();
    let output = Command::new(&bin)
        .args(args)
        .current_dir(workspace_root())
        .output();
    let output = match output {
        Ok(o) => o,
        Err(err) => {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "failed to spawn `ori`: {err}");
            }
            return (255, String::new());
        }
    };
    let exit = output.status.code().unwrap_or(-1);
    let text = match String::from_utf8(output.stdout) {
        Ok(s) => s,
        Err(err) => {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "ori produced non-UTF8 stdout: {err}");
            }
            String::new()
        }
    };
    (exit, text)
}

/// Parse `text` as one JSON object and assert its `schema` field equals
/// `expected_schema`. Returns the parsed value for the caller to inspect
/// further if needed.
fn assert_envelope(text: &str, expected_schema: &str) -> serde_json::Value {
    let value: serde_json::Value = match serde_json::from_str(text.trim()) {
        Ok(v) => v,
        Err(err) => {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected JSON, got parse error: {err}; raw: {text}");
            }
            return serde_json::Value::Null;
        }
    };
    let schema = value.get("schema").and_then(|v| v.as_str()).unwrap_or("");
    if schema != expected_schema {
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(
                false,
                "expected schema `{expected_schema}`, got `{schema}` in: {text}"
            );
        }
    }
    value
}

// ---- Per-subcommand cases ----

#[test]
fn doctor_emits_ori_doctor_v1() {
    let (code, out) = run(&["doctor"]);
    assert_eq!(code, 0, "doctor exit code");
    let v = assert_envelope(&out, "ori.doctor.v1");
    let schemas = v
        .get("schema_versions")
        .and_then(|v| v.as_object())
        .map(|m| m.len())
        .unwrap_or(0);
    assert!(
        schemas >= 30,
        "doctor should advertise >= 30 schemas, got {schemas}"
    );
}

#[test]
fn check_clean_demo_storefront_api() {
    let (code, out) = run(&["check", "--json", "examples/demo_store/src/api.ori"]);
    assert_eq!(code, 0, "check exit code on clean source");
    assert!(out.is_empty(), "clean source should emit no diagnostics");
}

#[test]
fn check_bad_null_emits_e0100() {
    let (code, out) = run(&["check", "--json", "examples/bad_null.ori"]);
    // E-level diagnostics → exit 1.
    assert_eq!(code, 1, "check exit code on bad source");
    let first = out.lines().next().unwrap_or("");
    let v: serde_json::Value = serde_json::from_str(first).unwrap_or(serde_json::Value::Null);
    assert_eq!(v.get("id").and_then(|v| v.as_str()), Some("E0100"));
}

#[test]
fn capsule_demo_storefront_api() {
    let (code, out) = run(&["capsule", "--json", "examples/demo_store/src/api.ori"]);
    assert_eq!(code, 0);
    let v = assert_envelope(&out, "ori.capsule.v1");
    let exports = v
        .get("exports")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    assert!(exports >= 3, "expected ≥3 exports, got {exports}");
}

#[test]
fn agent_map_budget_respected() {
    let (code, out) = run(&[
        "agent",
        "map",
        "--budget",
        "300",
        "--json",
        "examples/demo_store/src/api.ori",
    ]);
    assert_eq!(code, 0);
    let v = assert_envelope(&out, "ori.agent_map.v1");
    let used = v.get("used_estimate").and_then(|v| v.as_u64()).unwrap_or(0);
    assert!(used <= 400, "budget=300 should keep used ≲ 400, got {used}");
    let truncated = v
        .get("truncated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(truncated, "budget=300 should truncate the medium file");
}

#[test]
fn agent_diagnose_reports_status() {
    let (code, out) = run(&["agent", "diagnose", "--json", "examples/bad_null.ori"]);
    // diagnose itself exits 0 even when the inspected file has errors.
    assert_eq!(code, 0);
    let v = assert_envelope(&out, "ori.agent_diagnose.v1");
    let status = v
        .get("overall_status")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert_eq!(status, "error");
}

#[test]
fn openapi_extracts_routes() {
    let (code, out) = run(&["openapi", "--json", "examples/demo_store/src/api.ori"]);
    assert_eq!(code, 0);
    let v = assert_envelope(&out, "ori.openapi_report.v1");
    let routes = v
        .get("routes")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    assert_eq!(routes, 3, "demo storefront API should expose 3 routes");
}

#[test]
fn ui_manifest_lists_views() {
    let (code, out) = run(&["ui", "--json", "examples/demo_store/src/ui.ori"]);
    assert_eq!(code, 0);
    let v = assert_envelope(&out, "ori.ui_manifest.v1");
    let views = v
        .get("views")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    assert_eq!(views, 5, "demo storefront UI should expose 5 views");
}

#[test]
fn wasm_component_manifest_is_byte_stable() {
    let (code1, out1) = run(&["wasm", "--json", "examples/demo_store/src/api.ori"]);
    let (code2, out2) = run(&["wasm", "--json", "examples/demo_store/src/api.ori"]);
    assert_eq!(code1, 0);
    assert_eq!(code2, 0);
    assert_eq!(out1, out2, "wasm manifest must be byte-deterministic");
    assert_envelope(&out1, "ori.wasm_component.v1");
}

#[test]
fn capability_policy_diff() {
    let (code, out) = run(&[
        "capability",
        "--policy",
        "http,db.read,db.write",
        "--json",
        "examples/demo_store/src/api.ori",
    ]);
    assert_eq!(code, 0);
    let v = assert_envelope(&out, "ori.capability.v1");
    let undeclared = v
        .pointer("/policy/undeclared")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(99);
    assert_eq!(undeclared, 0, "policy matches usage exactly");
}

#[test]
fn patch_check_rejects_unknown_op() {
    use std::io::Write;
    let path = std::env::temp_dir().join(format!(
        "ori_cli_smoke_patch_{}_{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    {
        let mut f = match std::fs::File::create(&path) {
            Ok(f) => f,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "could not create temp file: {err}");
                }
                return;
            }
        };
        let _ = f.write_all(
            br#"{"schema":"ori.patch.v1","intent":"x","operations":[{"op":"unknown_op"}]}"#,
        );
    }
    let path_str = path.to_string_lossy().to_string();
    let (code, out) = run(&["patch", "check", "--json", &path_str]);
    let _ = std::fs::remove_file(&path);
    // Invalid patch → exit 1.
    assert_eq!(code, 1, "patch check should reject unknown op with exit 1");
    let v = assert_envelope(&out, "ori.patch_check.v1");
    let valid = v.get("valid").and_then(|v| v.as_bool()).unwrap_or(true);
    assert!(!valid, "patch should be marked invalid");
}

#[test]
fn run_executes_hello_world() {
    let (code, out) = run(&["run", "examples/hello.ori"]);
    assert_eq!(code, 0);
    assert!(out.contains("status: ok"), "raw output: {out}");
    assert!(out.contains("entry:  main"));
}

#[test]
fn bench_emits_at_least_seven_suites() {
    let (code, out) = run(&["bench", "--samples", "3", "--json"]);
    assert_eq!(code, 0);
    let v = assert_envelope(&out, "ori.benchmark.v1");
    let suites = v
        .get("suites")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    assert!(suites >= 7, "expected ≥7 benchmark suites, got {suites}");
}

#[test]
fn coverage_reports_for_demo() {
    let (code, out) = run(&["coverage", "--json", "examples/demo_store"]);
    assert_eq!(code, 0);
    let v = assert_envelope(&out, "ori.coverage_report.v1");
    let total = v
        .get("total_functions")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert!(total > 0, "should detect functions in demo_store");
}

#[test]
fn db_check_reports_migration_order() {
    let (code, out) = run(&[
        "db",
        "check",
        "--json",
        "examples/demo_store/src/catalog.ori",
    ]);
    assert_eq!(code, 0);
    let v = assert_envelope(&out, "ori.db_check.v1");
    let ordered = v
        .pointer("/migrations/ordered")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    assert_eq!(ordered, 1, "demo catalog has exactly one migration");
}

#[test]
fn package_check_reports_self_manifest() {
    let (code, out) = run(&["package", "check", "--json"]);
    assert_eq!(code, 0);
    let v = assert_envelope(&out, "ori.package_check.v1");
    let pkg = v
        .pointer("/manifest/package/name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert_eq!(pkg, "orison.bootstrap");
}

#[test]
fn sbom_emits_at_least_one_component() {
    let (code, out) = run(&["sbom", "--json"]);
    assert_eq!(code, 0);
    let v = assert_envelope(&out, "ori.sbom.v1");
    let components = v
        .get("components")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    assert!(components >= 1, "sbom must include the root package");
}

#[test]
fn audit_summary_shape() {
    let (code, out) = run(&["audit", "--json"]);
    assert_eq!(code, 0);
    let v = assert_envelope(&out, "ori.audit_report.v1");
    let summary = v.get("summary").and_then(|v| v.as_object());
    assert!(summary.is_some(), "audit must include a summary object");
}

#[test]
fn docs_agent_format_is_markdown() {
    let (code, out) = run(&[
        "docs",
        "--format",
        "agent",
        "--budget",
        "1000",
        "examples/demo_store/src",
    ]);
    assert_eq!(code, 0);
    assert!(
        out.starts_with("# "),
        "docs (agent format) should be markdown: {}",
        out.lines().next().unwrap_or("")
    );
}

#[test]
fn agent_telemetry_normalises_envelope() {
    use std::io::Write;
    let path = std::env::temp_dir().join(format!(
        "ori_cli_smoke_telemetry_{}_{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    {
        let mut f = match std::fs::File::create(&path) {
            Ok(f) => f,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "could not create temp file: {err}");
                }
                return;
            }
        };
        // Intentionally use wrong totals so we can verify the CLI
        // recomputes them.
        let _ = f.write_all(
            br#"{"schema":"ori.model_loop_telemetry.v1","session_id":"smoke","model_id":"m","iterations":[{"iteration":1,"started_at":0,"completed_at":250,"edits_proposed":3,"edits_accepted":2,"edits_rejected":1,"tokens_in":100,"tokens_out":200,"budget_remaining":9700,"diagnostics_before":5,"diagnostics_after":1}],"totals":{"iterations":42,"wall_ms":42,"edits_proposed":42,"edits_accepted":42,"edits_rejected":42,"tokens_in":42,"tokens_out":42,"diagnostics_resolved":42}}"#,
        );
    }
    let path_str = path.to_string_lossy().to_string();
    let (code, out) = run(&["agent", "telemetry", "--in", &path_str, "--json"]);
    let _ = std::fs::remove_file(&path);
    assert_eq!(code, 0, "telemetry should succeed; stdout: {out}");
    let v = assert_envelope(&out, "ori.model_loop_telemetry.v1");
    let wall = v
        .pointer("/totals/wall_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert_eq!(wall, 250, "wall_ms should be recomputed from iterations");
    let resolved = v
        .pointer("/totals/diagnostics_resolved")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    assert_eq!(resolved, 4, "5 -> 1 should report 4 resolved");
}

#[test]
fn migrate_reports_no_candidates_when_already_current() {
    let (code, out) = run(&[
        "migrate",
        "--from",
        "2027.1",
        "--to",
        "2028.1",
        "--dry-run",
        "--json",
        "examples/demo_store/src",
    ]);
    assert_eq!(code, 0);
    let v = assert_envelope(&out, "ori.migration_report.v1");
    let dry = v.get("applied").and_then(|v| v.as_bool()).unwrap_or(true);
    assert!(!dry, "dry-run must not report applied");
}

#[test]
fn ui_render_dry_run_emits_ori_ui_render_v1() {
    // Pick any view symbol from the demo storefront. The dry-run path
    // renders an empty-prop placeholder so the envelope must come back
    // with the published schema id and a non-zero node count.
    let (code, out) = run(&[
        "ui",
        "render",
        "--dry-run",
        "--module",
        "examples/demo_store/src/ui.ori",
        "--view",
        "ProductDetail",
    ]);
    assert_eq!(code, 0, "ui render dry-run exit code (stdout: {out})");
    let v = assert_envelope(&out, "ori.ui_render.v1");
    let node_count = v
        .pointer("/stats/node_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert!(node_count >= 1, "expected ≥1 node, got {node_count}");
    let root_kind = v
        .pointer("/root/kind")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert_eq!(root_kind, "ProductDetail");
}

#[test]
fn serve_dry_run_emits_ori_backend_dispatch_v1() {
    // The demo storefront exposes three http-annotated handlers
    // (get_products, get_product, post_checkout); the dispatch
    // envelope must enumerate them.
    let (code, out) = run(&[
        "serve",
        "--dry-run",
        "--module",
        "examples/demo_store/src/api.ori",
    ]);
    assert_eq!(code, 0, "serve --dry-run exit code (stdout: {out})");
    let v = assert_envelope(&out, "ori.backend_dispatch.v1");
    let count = v.get("route_count").and_then(|v| v.as_u64()).unwrap_or(0);
    assert_eq!(count, 3, "demo storefront should expose 3 dispatch routes");
    let routes = v
        .get("routes")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    assert_eq!(routes, 3, "routes array length must match route_count");
}

#[test]
fn serve_dry_run_envelope_is_byte_stable() {
    // Two consecutive invocations of the same dry-run dispatcher
    // must produce identical bytes — the determinism contract from
    // the M28 milestone.
    let args = [
        "serve",
        "--dry-run",
        "--module",
        "examples/demo_store/src/api.ori",
    ];
    let (code1, out1) = run(&args);
    let (code2, out2) = run(&args);
    assert_eq!(code1, 0);
    assert_eq!(code2, 0);
    assert_eq!(out1, out2, "dispatch envelope must be byte-deterministic");
    assert_envelope(&out1, "ori.backend_dispatch.v1");
}

/// `ori capability check --dry-run` walks every route/service symbol in
/// the module, builds a `CallContext` per symbol from the principal and
/// the `--has` effects, and emits a single `ori.capability_runtime.v1`
/// envelope. With only `http` held the storefront routes that require
/// `db.read`/`db.write` must come back as CAP0001 denials.
#[test]
fn capability_check_dry_run_emits_runtime_envelope() {
    let (code, out) = run(&[
        "capability",
        "check",
        "--dry-run",
        "--module",
        "examples/demo_store/src/api.ori",
        "--principal",
        "alice",
        "--has",
        "http",
    ]);
    assert_eq!(code, 0, "capability check exit code");
    let v = assert_envelope(&out, "ori.capability_runtime.v1");
    let outcomes = v
        .get("outcomes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        !outcomes.is_empty(),
        "expected at least one route outcome, got: {out}"
    );
    let denied = outcomes
        .iter()
        .filter_map(|entry| entry.pointer("/outcome/code").and_then(|c| c.as_str()))
        .any(|c| c == "CAP0001");
    assert!(denied, "expected at least one CAP0001 denial in: {out}");
}
