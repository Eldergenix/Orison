//! Schema-validation gate for every shipping CLI envelope.
//!
//! For each subcommand the bootstrap CLI publishes a `--json` envelope for,
//! this test:
//!   1. invokes the subcommand and captures its stdout,
//!   2. writes the captured JSON to a temp file,
//!   3. runs `ori schema validate --in <tempfile>` against the published
//!      schemas under `./schemas/`,
//!   4. asserts the returned `errors` array is empty.
//!
//! The test depends on `cli_smoke.rs` only by mirroring its set of
//! envelopes; both files build the same release binary on demand.

use std::env;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

// ---------------------------------------------------------------------------
// Plumbing: build the release binary, run it, write temp files. Mirrors the
// helpers in `cli_smoke.rs` so the two files stay independent.
// ---------------------------------------------------------------------------

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

fn ori_binary() -> PathBuf {
    let root = workspace_root();
    let bin = root.join("target").join("release").join("ori");
    if !bin.exists() {
        let _ = Command::new(env!("CARGO"))
            .arg("build")
            .arg("--release")
            .arg("-p")
            .arg("ori")
            .current_dir(&root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    if !bin.exists() {
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(false, "could not build release `ori` binary at {bin:?}");
        }
    }
    bin
}

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
    let text = String::from_utf8(output.stdout).unwrap_or_default();
    (exit, text)
}

fn temp_file(prefix: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    env::temp_dir().join(format!(
        "ori_schema_validation_{prefix}_{}_{nanos}.json",
        std::process::id()
    ))
}

fn write_file(path: &Path, contents: &str) {
    let mut f = match std::fs::File::create(path) {
        Ok(f) => f,
        Err(err) => {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "could not create temp file {path:?}: {err}");
            }
            return;
        }
    };
    let _ = f.write_all(contents.as_bytes());
}

/// Run `ori schema validate --in <file>` against the shipped schemas and
/// return `(ok, errors_json_text)`. A non-empty `errors` array fails the
/// gate.
fn validate_envelope_text(envelope_text: &str, label: &str) {
    let path = temp_file(label);
    write_file(&path, envelope_text);
    let path_str = path.to_string_lossy().to_string();
    let (code, out) = run(&[
        "schema",
        "validate",
        "--in",
        &path_str,
        "--schemas",
        "./schemas",
    ]);
    let _ = std::fs::remove_file(&path);

    let report: serde_json::Value = match serde_json::from_str(out.trim()) {
        Ok(v) => v,
        Err(err) => {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(
                    false,
                    "validator produced non-JSON output for {label}: {err}; raw: {out}"
                );
            }
            return;
        }
    };
    let schema = report.get("schema").and_then(|v| v.as_str()).unwrap_or("");
    assert_eq!(
        schema, "ori.schema_validation.v1",
        "validator envelope schema mismatch for {label}: {report}"
    );
    let errors = report
        .get("errors")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if !errors.is_empty() {
        let printed = serde_json::to_string_pretty(&errors).unwrap_or_default();
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(
                false,
                "envelope for {label} failed schema validation (exit {code}):\n{printed}\n\nraw envelope was:\n{envelope_text}"
            );
        }
    }
    assert_eq!(
        code, 0,
        "validator should exit 0 on clean validation: {out}"
    );
}

/// Capture the stdout of `args` and validate it. Used for the most common
/// case where the command emits a single JSON envelope.
fn capture_and_validate(label: &str, args: &[&str]) {
    let (code, out) = run(args);
    assert!(
        code == 0 || code == 1,
        "{label} unexpected exit code {code}; stdout: {out}"
    );
    let trimmed = out.trim();
    assert!(!trimmed.is_empty(), "{label} produced no stdout");
    validate_envelope_text(trimmed, label);
}

// ---------------------------------------------------------------------------
// One test per shipping envelope. Each test exercises exactly one CLI
// subcommand and validates its JSON envelope against the corresponding
// `schemas/<name>.schema.json` file.
// ---------------------------------------------------------------------------

#[test]
fn validates_doctor_envelope() {
    capture_and_validate("doctor", &["doctor"]);
}

#[test]
fn validates_check_diagnostic_envelope() {
    // `ori check --json` emits one ori.diagnostic.v1 envelope per line for
    // each diagnostic. We validate the first one against
    // `schemas/diagnostic.schema.json`.
    let (code, out) = run(&["check", "--json", "examples/bad_null.ori"]);
    assert_eq!(code, 1, "check should exit 1 with diagnostics");
    let first = out.lines().next().unwrap_or("");
    assert!(!first.is_empty(), "check should emit at least one envelope");
    validate_envelope_text(first, "check_diagnostic");
}

#[test]
fn validates_agent_map_envelope() {
    capture_and_validate(
        "agent_map",
        &[
            "agent",
            "map",
            "--budget",
            "2000",
            "--json",
            "examples/demo_store/src/api.ori",
        ],
    );
}

#[test]
fn validates_agent_diagnose_envelope() {
    capture_and_validate(
        "agent_diagnose",
        &["agent", "diagnose", "--json", "examples/bad_null.ori"],
    );
}

#[test]
fn validates_agent_telemetry_envelope() {
    let path = temp_file("telemetry_in");
    let input = r#"{"schema":"ori.model_loop_telemetry.v1","session_id":"smoke","model_id":"m","iterations":[{"iteration":1,"started_at":0,"completed_at":250,"edits_proposed":3,"edits_accepted":2,"edits_rejected":1,"tokens_in":100,"tokens_out":200,"budget_remaining":9700,"diagnostics_before":5,"diagnostics_after":1}],"totals":{"iterations":42,"wall_ms":42,"edits_proposed":42,"edits_accepted":42,"edits_rejected":42,"tokens_in":42,"tokens_out":42,"diagnostics_resolved":42}}"#;
    write_file(&path, input);
    let path_str = path.to_string_lossy().to_string();
    let (code, out) = run(&["agent", "telemetry", "--in", &path_str, "--json"]);
    let _ = std::fs::remove_file(&path);
    assert_eq!(code, 0, "agent telemetry exit code; stdout: {out}");
    validate_envelope_text(out.trim(), "agent_telemetry");
}

#[test]
fn validates_capsule_envelope() {
    capture_and_validate(
        "capsule",
        &["capsule", "--json", "examples/demo_store/src/api.ori"],
    );
}

#[test]
fn validates_openapi_envelope() {
    capture_and_validate(
        "openapi",
        &["openapi", "--json", "examples/demo_store/src/api.ori"],
    );
}

#[test]
fn validates_ui_manifest_envelope() {
    capture_and_validate(
        "ui_manifest",
        &["ui", "--json", "examples/demo_store/src/ui.ori"],
    );
}

#[test]
fn validates_ui_render_envelope() {
    capture_and_validate(
        "ui_render",
        &[
            "ui",
            "render",
            "--dry-run",
            "--module",
            "examples/demo_store/src/ui.ori",
            "--view",
            "ProductDetail",
        ],
    );
}

#[test]
fn validates_serve_dispatch_envelope() {
    capture_and_validate(
        "serve_dispatch",
        &[
            "serve",
            "--dry-run",
            "--module",
            "examples/demo_store/src/api.ori",
        ],
    );
}

#[test]
fn validates_capability_runtime_envelope() {
    capture_and_validate(
        "capability_runtime",
        &[
            "capability",
            "check",
            "--dry-run",
            "--module",
            "examples/demo_store/src/api.ori",
            "--principal",
            "alice",
            "--has",
            "http",
        ],
    );
}

#[test]
fn validates_patch_check_envelope() {
    let path = temp_file("patch_in");
    let input = r#"{"schema":"ori.patch.v1","intent":"x","operations":[{"op":"unknown_op"}]}"#;
    write_file(&path, input);
    let path_str = path.to_string_lossy().to_string();
    let (code, out) = run(&["patch", "check", "--json", &path_str]);
    let _ = std::fs::remove_file(&path);
    // Invalid patch is expected; CLI exits 1 but the envelope still
    // conforms to ori.patch_check.v1.
    assert_eq!(code, 1, "patch check exit code; stdout: {out}");
    validate_envelope_text(out.trim(), "patch_check");
}

#[test]
fn validates_package_check_envelope() {
    capture_and_validate("package_check", &["package", "check", "--json"]);
}

#[test]
fn validates_sbom_envelope() {
    capture_and_validate("sbom", &["sbom", "--json"]);
}

#[test]
fn validates_audit_envelope() {
    capture_and_validate("audit", &["audit", "--json"]);
}

#[test]
fn validates_bench_envelope() {
    capture_and_validate("bench", &["bench", "--samples", "1", "--json"]);
}

#[test]
fn validates_coverage_envelope() {
    capture_and_validate("coverage", &["coverage", "--json", "examples/demo_store"]);
}

#[test]
fn validates_migrate_envelope() {
    capture_and_validate(
        "migrate",
        &[
            "migrate",
            "--from",
            "2027.1",
            "--to",
            "2028.1",
            "--dry-run",
            "--json",
            "examples/demo_store/src",
        ],
    );
}

#[test]
fn validates_docs_envelope() {
    capture_and_validate(
        "docs",
        &[
            "docs",
            "--format",
            "agent",
            "--budget",
            "1000",
            "--json",
            "examples/demo_store/src",
        ],
    );
}

#[test]
fn validates_build_mobile_envelope() {
    capture_and_validate(
        "build_mobile",
        &[
            "build",
            "--target",
            "mobile",
            "--json",
            "examples/demo_store/src/api.ori",
        ],
    );
}
