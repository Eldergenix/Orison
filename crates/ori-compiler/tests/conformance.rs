//! Conformance suite: drives the compiler library against the curated
//! golden fixtures under `<workspace>/tests/golden/` and asserts the
//! produced JSON matches the recorded `.expected.*` files byte-for-byte
//! (after parsing into `serde_json::Value` so whitespace and key ordering
//! are normalised by the structural comparison).
//!
//! Fixture layout:
//!   tests/golden/parser/*.ori                    -- source-only; checked
//!                                                   by parse-shape assertions.
//!   tests/golden/diagnostics/*.ori +
//!     tests/golden/diagnostics/*.expected.jsonl  -- one diagnostic per line.
//!   tests/golden/{capsule,agent_map,openapi,
//!                 ui,wasm,capability}/*.expected.json
//!                                                -- single JSON document.
//!
//! Volatile fields (timestamps, content hashes) are stripped before
//! comparison; the helpers below document each stripped field next to the
//! call site so reviewers can audit what we deliberately ignore.
//!
//! Re-blessing: set `ORI_CONFORMANCE_BLESS=1` and re-run
//! `cargo test -p ori-compiler conformance`. The test will overwrite the
//! `.expected.*` files with current output. Without that env var the test
//! never mutates fixtures.

#![allow(clippy::needless_collect)]

use ori_agent::{agent_map_json, AgentMapOptions};
use ori_compiler::ast::SymbolKind;
use ori_compiler::effect_check::build_capability_manifest;
use ori_compiler::openapi::extract_openapi;
use ori_compiler::source::SourceFile;
use ori_compiler::type_check::type_check_module;
use ori_compiler::ui_check::build_ui_manifest;
use ori_compiler::wasm_component::build_wasm_component_manifest;
use ori_compiler::{Compiler, Diagnostic};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Workspace + bless plumbing
// ---------------------------------------------------------------------------

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is `<workspace>/crates/ori-compiler`. Walk two
    // levels up to land at the workspace root.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    match manifest.parent().and_then(Path::parent) {
        Some(root) => root.to_path_buf(),
        None => {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "CARGO_MANIFEST_DIR has no two-level parent");
            }
            unreachable!()
        }
    }
}

fn bless_mode() -> bool {
    matches!(std::env::var("ORI_CONFORMANCE_BLESS").as_deref(), Ok("1"))
}

fn read_text(path: &Path) -> String {
    match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "failed to read {}: {err}", path.display());
            }
            unreachable!()
        }
    }
}

fn write_text(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "failed to create {}: {err}", parent.display());
            }
        }
    }
    if let Err(err) = fs::write(path, text) {
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(false, "failed to write {}: {err}", path.display());
        }
    }
}

fn parse_json(text: &str, where_: &str) -> Value {
    match serde_json::from_str::<Value>(text) {
        Ok(v) => v,
        Err(err) => {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(
                    false,
                    "failed to parse JSON from {where_}: {err}\n--- text ---\n{text}"
                );
            }
            unreachable!()
        }
    }
}

/// Compare two JSON values structurally. On mismatch, print a diff-friendly
/// dump of both sides and fail the test.
fn assert_json_eq(label: &str, actual: &Value, expected: &Value) {
    if actual == expected {
        return;
    }
    let actual_pretty = serde_json::to_string_pretty(actual).unwrap_or_else(|_| actual.to_string());
    let expected_pretty =
        serde_json::to_string_pretty(expected).unwrap_or_else(|_| expected.to_string());
    #[allow(clippy::assertions_on_constants)]
    {
        assert!(
            false,
            "conformance mismatch [{label}]\n\n--- expected ---\n{expected_pretty}\n\n--- actual ---\n{actual_pretty}\n\nRe-bless with: ORI_CONFORMANCE_BLESS=1 cargo test -p ori-compiler conformance"
        );
    }
}

/// Load a fixture source file at `<workspace>/<rel>` and produce a
/// `SourceFile` whose `path` is the workspace-relative string (so spans
/// in the recorded JSON stay stable across machines).
fn load_source(rel: &str) -> SourceFile {
    let abs = workspace_root().join(rel);
    let text = read_text(&abs);
    SourceFile::new(rel.to_string(), text)
}

// ---------------------------------------------------------------------------
// Volatile-field strippers (documented per snapshot type)
// ---------------------------------------------------------------------------

/// Capsule snapshots embed a content hash (`fnv1a:...`) keyed on parsed
/// signatures, which is stable today but is conceptually a build artefact.
/// We normalise it to `"<stripped>"` so the snapshot is robust to future
/// hash algorithm changes.
fn strip_volatile_capsule(value: &mut Value) {
    if let Some(obj) = value.as_object_mut() {
        if obj.contains_key("hash") {
            obj.insert("hash".to_string(), Value::String("<stripped>".to_string()));
        }
    }
}

/// `agent_map` snapshots contain `used_estimate`, which depends on the
/// exact byte length of signatures the parser produced. That value is
/// already deterministic given the source, so we keep it; nothing is
/// stripped today, but the call site is reserved for future drift.
fn strip_volatile_agent_map(_value: &mut Value) {}

/// OpenAPI / UI / wasm / capability reports are content-derived and do
/// not embed timestamps. The functions are kept as explicit no-ops so the
/// list of "what we deliberately do not strip" is auditable.
fn strip_volatile_openapi(_value: &mut Value) {}
fn strip_volatile_ui(_value: &mut Value) {}
fn strip_volatile_wasm(_value: &mut Value) {}
fn strip_volatile_capability(_value: &mut Value) {}

// ---------------------------------------------------------------------------
// Diagnostic JSONL helpers
// ---------------------------------------------------------------------------

fn diagnostics_to_jsonl(diags: &[Diagnostic]) -> String {
    let mut out = String::new();
    for (idx, diag) in diags.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        out.push_str(&diag.to_json());
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

fn parse_jsonl(text: &str) -> Vec<Value> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| parse_json(line, "jsonl line"))
        .collect()
}

/// Run `Compiler::check_source` (which emits parser + style diagnostics)
/// and merge in the type-check pass so the conformance suite covers
/// W0501 / W0510 as well. Effect-policy diagnostics are not merged here:
/// the policy is fixture-specific (`capability/*.expected.json` exercises
/// those).
fn full_diagnostics(source: SourceFile) -> Vec<Diagnostic> {
    let result = Compiler::check_source(source);
    let mut diags = result.diagnostics.clone();
    diags.extend(type_check_module(&result.module));
    diags
}

fn assert_diagnostic_fixture(rel_source: &str, expected_rel: &str) {
    let source = load_source(rel_source);
    let diags = full_diagnostics(source);
    let actual_jsonl = diagnostics_to_jsonl(&diags);
    let expected_path = workspace_root().join(expected_rel);

    if bless_mode() {
        write_text(&expected_path, &actual_jsonl);
        return;
    }

    let expected_text = read_text(&expected_path);
    let actual_values = parse_jsonl(&actual_jsonl);
    let expected_values = parse_jsonl(&expected_text);
    assert_json_eq(
        expected_rel,
        &Value::Array(actual_values),
        &Value::Array(expected_values),
    );
}

fn assert_json_fixture(
    label: &str,
    expected_rel: &str,
    mut actual: Value,
    strip: impl Fn(&mut Value),
) {
    strip(&mut actual);
    let expected_path = workspace_root().join(expected_rel);

    if bless_mode() {
        let pretty = match serde_json::to_string_pretty(&actual) {
            Ok(s) => s,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "failed to pretty-print {label}: {err}");
                }
                unreachable!()
            }
        };
        let mut blessed = pretty;
        blessed.push('\n');
        write_text(&expected_path, &blessed);
        return;
    }

    let expected_text = read_text(&expected_path);
    let mut expected = parse_json(&expected_text, expected_rel);
    strip(&mut expected);
    assert_json_eq(label, &actual, &expected);
}

// ---------------------------------------------------------------------------
// Diagnostic schema sanity check
// ---------------------------------------------------------------------------

/// Light structural check that each fixture line obeys the
/// `diagnostic.schema.json` v1 shape. Not a full JSON Schema validator —
/// the workspace has no JSON Schema runtime dependency — but it catches
/// missing required fields / wrong types, which is the failure mode the
/// schema is meant to prevent.
fn validate_diagnostic_schema(label: &str, line: &Value) {
    let obj = match line.as_object() {
        Some(obj) => obj,
        None => {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "{label}: diagnostic entry is not an object");
            }
            unreachable!()
        }
    };
    let required = [
        ("schema", true),
        ("id", true),
        ("level", true),
        ("message", true),
        ("span", true),
        ("expected", true),
        ("found", true),
        ("fixes", true),
        ("agent", true),
    ];
    for (key, _) in required {
        if !obj.contains_key(key) {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "{label}: required field `{key}` missing");
            }
        }
    }
    if obj.get("schema").and_then(Value::as_str) != Some("ori.diagnostic.v1") {
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(false, "{label}: schema is not ori.diagnostic.v1");
        }
    }
    let id = obj.get("id").and_then(Value::as_str).unwrap_or("");
    let id_ok = id.len() == 5
        && id
            .chars()
            .next()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false)
        && id.chars().skip(1).all(|c| c.is_ascii_digit());
    if !id_ok {
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(
                false,
                "{label}: diagnostic id `{id}` does not match `^[A-Z][0-9]{{4}}$`"
            );
        }
    }
    let level = obj.get("level").and_then(Value::as_str).unwrap_or("");
    if !matches!(level, "error" | "warning" | "info") {
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(false, "{label}: level `{level}` is not error|warning|info");
        }
    }
    let span_ok = obj
        .get("span")
        .and_then(Value::as_object)
        .map(|s| s.contains_key("file") && s.contains_key("start") && s.contains_key("end"))
        .unwrap_or(false);
    if !span_ok {
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(false, "{label}: span missing file/start/end");
        }
    }
}

// ---------------------------------------------------------------------------
// Parser fixtures (source-only). Each fixture asserts the parser produced
// the expected set of top-level symbol kinds — the contract a downstream
// consumer (capsule, agent_map, openapi, ...) relies on.
// ---------------------------------------------------------------------------

#[test]
fn parser_hello_has_function() {
    let source = load_source("tests/golden/parser/hello.ori");
    let result = Compiler::check_source(source);
    assert!(result.module.name == "parser.hello", "module name");
    assert!(
        result
            .module
            .symbols
            .iter()
            .any(|s| s.kind == SymbolKind::Function && s.name == "greet"),
        "expected fn `greet`"
    );
}

#[test]
fn parser_full_signatures_have_effects_and_returns() {
    let source = load_source("tests/golden/parser/full_signatures.ori");
    let result = Compiler::check_source(source);
    let fetch = match result.module.symbols.iter().find(|s| s.name == "fetch") {
        Some(symbol) => symbol,
        None => {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "expected to find fn `fetch` in full_signatures.ori");
            }
            unreachable!()
        }
    };
    assert!(fetch.signature.contains("->"), "fetch missing return type");
    assert!(
        fetch.effects.iter().any(|e| e == "net.outbound")
            && fetch.effects.iter().any(|e| e == "fs.read"),
        "fetch effects: {:?}",
        fetch.effects
    );
}

#[test]
fn parser_variants_indexed_as_type() {
    let source = load_source("tests/golden/parser/variants.ori");
    let result = Compiler::check_source(source);
    assert!(
        result
            .module
            .symbols
            .iter()
            .any(|s| s.kind == SymbolKind::Type && s.name == "X"),
        "expected type `X`"
    );
}

#[test]
fn parser_records_indexed_as_types() {
    let source = load_source("tests/golden/parser/records.ori");
    let result = Compiler::check_source(source);
    let names: Vec<_> = result
        .module
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Type)
        .map(|s| s.name.as_str())
        .collect();
    assert!(names.contains(&"Point"), "Point missing in {names:?}");
    assert!(names.contains(&"Person"), "Person missing in {names:?}");
}

#[test]
fn parser_services_indexed_as_service() {
    let source = load_source("tests/golden/parser/services.ori");
    let result = Compiler::check_source(source);
    assert!(
        result
            .module
            .symbols
            .iter()
            .any(|s| s.kind == SymbolKind::Service && s.name == "Y"),
        "expected service `Y`"
    );
}

#[test]
fn parser_views_indexed_as_view() {
    let source = load_source("tests/golden/parser/views.ori");
    let result = Compiler::check_source(source);
    assert!(
        result
            .module
            .symbols
            .iter()
            .any(|s| s.kind == SymbolKind::View && s.name == "Z"),
        "expected view `Z`"
    );
}

// ---------------------------------------------------------------------------
// Diagnostic fixtures
// ---------------------------------------------------------------------------

#[test]
fn diagnostics_null_jsonl_matches_recorded_fixture() {
    // The legacy `null.jsonl` fixture was recorded from a run against
    // `examples/bad_null.ori`; reproduce the same input so the file path
    // embedded in `span.file` lines up.
    let source = load_source("examples/bad_null.ori");
    let diags = Compiler::check_source(source).diagnostics;
    // Filter to just the null diagnostic (style passes can add extras
    // over time; the fixture is scoped to the null finding).
    let null_diags: Vec<_> = diags.into_iter().filter(|d| d.id == "E0100").collect();
    let actual_jsonl = diagnostics_to_jsonl(&null_diags);

    let expected_text = read_text(&workspace_root().join("tests/golden/diagnostics/null.jsonl"));
    let actual_values = parse_jsonl(&actual_jsonl);
    let expected_values = parse_jsonl(&expected_text);
    for v in &actual_values {
        validate_diagnostic_schema("tests/golden/diagnostics/null.jsonl", v);
    }
    for v in &expected_values {
        validate_diagnostic_schema("tests/golden/diagnostics/null.jsonl (recorded)", v);
    }
    assert_json_eq(
        "tests/golden/diagnostics/null.jsonl",
        &Value::Array(actual_values),
        &Value::Array(expected_values),
    );
}

#[test]
fn diagnostics_missing_module_e0001() {
    assert_diagnostic_fixture(
        "tests/golden/diagnostics/missing_module.ori",
        "tests/golden/diagnostics/missing_module.expected.jsonl",
    );
    validate_each_recorded_diagnostic("tests/golden/diagnostics/missing_module.expected.jsonl");
}

#[test]
fn diagnostics_duplicate_symbol_e0201() {
    assert_diagnostic_fixture(
        "tests/golden/diagnostics/duplicate_symbol.ori",
        "tests/golden/diagnostics/duplicate_symbol.expected.jsonl",
    );
    validate_each_recorded_diagnostic("tests/golden/diagnostics/duplicate_symbol.expected.jsonl");
}

#[test]
fn diagnostics_unknown_effect_w0401() {
    assert_diagnostic_fixture(
        "tests/golden/diagnostics/unknown_effect.ori",
        "tests/golden/diagnostics/unknown_effect.expected.jsonl",
    );
    validate_each_recorded_diagnostic("tests/golden/diagnostics/unknown_effect.expected.jsonl");
}

#[test]
fn diagnostics_missing_return_type_w0301() {
    assert_diagnostic_fixture(
        "tests/golden/diagnostics/missing_return_type.ori",
        "tests/golden/diagnostics/missing_return_type.expected.jsonl",
    );
    validate_each_recorded_diagnostic(
        "tests/golden/diagnostics/missing_return_type.expected.jsonl",
    );
}

#[test]
fn diagnostics_unknown_type_w0501() {
    assert_diagnostic_fixture(
        "tests/golden/diagnostics/unknown_type.ori",
        "tests/golden/diagnostics/unknown_type.expected.jsonl",
    );
    validate_each_recorded_diagnostic("tests/golden/diagnostics/unknown_type.expected.jsonl");
}

#[test]
fn diagnostics_bare_result_w0510() {
    assert_diagnostic_fixture(
        "tests/golden/diagnostics/bare_result.ori",
        "tests/golden/diagnostics/bare_result.expected.jsonl",
    );
    validate_each_recorded_diagnostic("tests/golden/diagnostics/bare_result.expected.jsonl");
}

fn validate_each_recorded_diagnostic(rel: &str) {
    let text = read_text(&workspace_root().join(rel));
    let values = parse_jsonl(&text);
    if values.is_empty() {
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(false, "{rel}: expected at least one diagnostic line");
        }
    }
    for v in &values {
        validate_diagnostic_schema(rel, v);
    }
}

// ---------------------------------------------------------------------------
// Capsule / agent_map / openapi / ui / wasm / capability snapshots
// ---------------------------------------------------------------------------

#[test]
fn capsule_store_users_snapshot() {
    let source = load_source("examples/fullstack/users.ori");
    let result = Compiler::check_source(source);
    let json = Compiler::capsule_json(&result);
    let value = parse_json(&json, "capsule_json");
    assert_json_fixture(
        "capsule/store_users",
        "tests/golden/capsule/store_users.expected.json",
        value,
        strip_volatile_capsule,
    );
}

#[test]
fn agent_map_store_users_snapshot() {
    let source = load_source("examples/fullstack/users.ori");
    let result = Compiler::check_source(source);
    let json = agent_map_json(&result, AgentMapOptions { budget: 2000 });
    let value = parse_json(&json, "agent_map_json");
    assert_json_fixture(
        "agent_map/store_users",
        "tests/golden/agent_map/store_users.expected.json",
        value,
        strip_volatile_agent_map,
    );
}

#[test]
fn openapi_demo_store_api_snapshot() {
    let source = load_source("examples/demo_store/src/api.ori");
    let result = Compiler::check_source(source);
    let report = extract_openapi(&result.module);
    let value = parse_json(&report.to_json(), "openapi");
    assert_json_fixture(
        "openapi/demo_store_api",
        "tests/golden/openapi/demo_store_api.expected.json",
        value,
        strip_volatile_openapi,
    );
}

#[test]
fn ui_demo_store_snapshot() {
    let source = load_source("examples/demo_store/src/ui.ori");
    let result = Compiler::check_source(source);
    let manifest = build_ui_manifest(&result.module);
    let value = parse_json(&manifest.to_json(), "ui");
    assert_json_fixture(
        "ui/demo_store_ui",
        "tests/golden/ui/demo_store_ui.expected.json",
        value,
        strip_volatile_ui,
    );
}

#[test]
fn wasm_demo_store_api_snapshot() {
    let source = load_source("examples/demo_store/src/api.ori");
    let result = Compiler::check_source(source);
    let manifest = build_wasm_component_manifest(&result.module);
    let value = parse_json(&manifest.to_json(), "wasm");
    assert_json_fixture(
        "wasm/demo_store_api",
        "tests/golden/wasm/demo_store_api.expected.json",
        value,
        strip_volatile_wasm,
    );
}

#[test]
fn capability_demo_store_api_snapshot() {
    let source = load_source("examples/demo_store/src/api.ori");
    let result = Compiler::check_source(source);
    let policy = vec![
        "http".to_string(),
        "db.read".to_string(),
        "db.write".to_string(),
    ];
    let manifest = build_capability_manifest(&result.module, &policy);
    let value = parse_json(&manifest.to_json(), "capability");
    assert_json_fixture(
        "capability/demo_store_api",
        "tests/golden/capability/demo_store_api.expected.json",
        value,
        strip_volatile_capability,
    );
}
