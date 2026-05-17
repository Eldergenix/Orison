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
use ori_compiler::ast::{Symbol, SymbolKind};
use ori_compiler::body::parse_module_bodies;
use ori_compiler::design_tokens::{
    check_module as design_check_module, report_to_diagnostics as design_report_to_diagnostics,
    TokenSet,
};
use ori_compiler::effect_check::{build_capability_manifest, effect_diagnostics};
use ori_compiler::effect_propagate::{
    build_effect_graph, propagate_effects, propagation_diagnostics,
};
use ori_compiler::exhaustive::check_module_matches_with_source;
use ori_compiler::interp_exec::{exec_program, Value as RuntimeValue};
use ori_compiler::mobile::{build_mobile_manifest, validate_manifest as validate_mobile_manifest};
use ori_compiler::openapi::extract_openapi;
use ori_compiler::parser::parse_source;
use ori_compiler::patch::check_patch_json;
use ori_compiler::patch_apply::apply_patch;
use ori_compiler::preproc::{preprocess, PreprocessConfig};
use ori_compiler::resolver::resolve as resolve_modules;
use ori_compiler::source::SourceFile;
use ori_compiler::sql_check::check_module_queries;
use ori_compiler::type_check::type_check_module;
use ori_compiler::ui_check::build_ui_manifest;
use ori_compiler::wasm_component::build_wasm_component_manifest;
use ori_compiler::{borrow, Compiler, Diagnostic};
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

// ---------------------------------------------------------------------------
// M34 per-diagnostic fixture coverage (resolver, effect, exhaustive, borrow,
// sql, design, mobile, preproc, async, runtime, patch IR).
//
// Each test below follows the same shape: load a small fixture, drive the
// relevant analysis pass, filter diagnostics down to the targeted id, then
// compare against the recorded `.expected.jsonl`. Filtering keeps the
// fixture scoped to the single diagnostic being tested — other passes may
// add adjacent findings (style warnings, broader type checks) over time
// without invalidating the contract for this id.
// ---------------------------------------------------------------------------

/// Filter `diags` down to the diagnostics whose `id` exactly matches `id` and
/// emit them as the JSONL the existing fixtures expect (one object per line,
/// trailing newline).
fn diagnostics_filtered_jsonl(diags: &[Diagnostic], id: &str) -> String {
    let scoped: Vec<Diagnostic> = diags.iter().filter(|d| d.id == id).cloned().collect();
    diagnostics_to_jsonl(&scoped)
}

/// Compare an in-memory diagnostic JSONL string against a recorded fixture
/// (with bless support). This is the structural twin of
/// [`assert_diagnostic_fixture`] but accepts an already-computed JSONL so
/// non-`ori check` passes can use it.
fn assert_jsonl_fixture(label: &str, expected_rel: &str, actual_jsonl: &str) {
    let expected_path = workspace_root().join(expected_rel);

    if bless_mode() {
        write_text(&expected_path, actual_jsonl);
        return;
    }

    let expected_text = read_text(&expected_path);
    let actual_values = parse_jsonl(actual_jsonl);
    let expected_values = parse_jsonl(&expected_text);
    assert_json_eq(
        label,
        &Value::Array(actual_values),
        &Value::Array(expected_values),
    );
}

/// Validate every recorded line under `rel` against the diagnostic schema
/// shape. Same contract as [`validate_each_recorded_diagnostic`] but for
/// fixtures stored outside `tests/golden/diagnostics/` whose ids may use
/// the wider `^[A-Z]{1,4}[0-9]{4}$` family (e.g. `MOB0001`). The check is
/// skipped automatically when `skip_id_pattern` is true.
fn validate_each_recorded_diagnostic_loose(rel: &str, skip_id_pattern: bool) {
    let text = read_text(&workspace_root().join(rel));
    let values = parse_jsonl(&text);
    if values.is_empty() {
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(false, "{rel}: expected at least one diagnostic line");
        }
    }
    for v in &values {
        validate_diagnostic_schema_loose(rel, v, skip_id_pattern);
    }
}

/// Loose schema validation: same required fields as
/// [`validate_diagnostic_schema`] but optionally tolerates the wider id
/// shape used by the `MOB####` / `PRE####` (or other multi-letter) families.
fn validate_diagnostic_schema_loose(label: &str, line: &Value, skip_id_pattern: bool) {
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
    for key in [
        "schema", "id", "level", "message", "span", "expected", "found", "fixes", "agent",
    ] {
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
    if !skip_id_pattern {
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
    }
}

// --- Resolver: E0211, E0220, E0230 ----------------------------------------

#[test]
fn golden_e0220_unresolved_import() {
    let source = load_source("tests/golden/diagnostics/unresolved_import.ori");
    let module = parse_source(&source).module;
    let res = resolve_modules(&[module]);
    let actual = diagnostics_filtered_jsonl(&res.diagnostics, "E0220");
    assert_jsonl_fixture(
        "tests/golden/diagnostics/unresolved_import.expected.jsonl",
        "tests/golden/diagnostics/unresolved_import.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic("tests/golden/diagnostics/unresolved_import.expected.jsonl");
}

#[test]
fn golden_e0230_module_cycle() {
    let source_a = load_source("tests/golden/diagnostics/cycle_a.ori");
    let source_b = load_source("tests/golden/diagnostics/cycle_b.ori");
    let module_a = parse_source(&source_a).module;
    let module_b = parse_source(&source_b).module;
    let res = resolve_modules(&[module_a, module_b]);
    let actual = diagnostics_filtered_jsonl(&res.diagnostics, "E0230");
    assert_jsonl_fixture(
        "tests/golden/diagnostics/cycle.expected.jsonl",
        "tests/golden/diagnostics/cycle.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic("tests/golden/diagnostics/cycle.expected.jsonl");
}

#[test]
fn golden_e0211_cross_module_duplicate() {
    let source_a = load_source("tests/golden/diagnostics/cross_module_dup_a.ori");
    let source_b = load_source("tests/golden/diagnostics/cross_module_dup_b.ori");
    let module_a = parse_source(&source_a).module;
    let module_b = parse_source(&source_b).module;
    let res = resolve_modules(&[module_a, module_b]);
    let actual = diagnostics_filtered_jsonl(&res.diagnostics, "E0211");
    assert_jsonl_fixture(
        "tests/golden/diagnostics/cross_module_dup.expected.jsonl",
        "tests/golden/diagnostics/cross_module_dup.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic("tests/golden/diagnostics/cross_module_dup.expected.jsonl");
}

// --- Effect check / propagation: E0410, E0420 -----------------------------

#[test]
fn golden_e0410_effect_undeclared() {
    let source = load_source("tests/golden/diagnostics/effect_undeclared.ori");
    let module = parse_source(&source).module;
    let declared_policy = vec!["net.outbound".to_string()];
    let diags = effect_diagnostics(&module, &declared_policy);
    let actual = diagnostics_filtered_jsonl(&diags, "E0410");
    assert_jsonl_fixture(
        "tests/golden/diagnostics/effect_undeclared.expected.jsonl",
        "tests/golden/diagnostics/effect_undeclared.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic("tests/golden/diagnostics/effect_undeclared.expected.jsonl");
}

#[test]
fn golden_e0420_effect_propagation() {
    let source = load_source("tests/golden/diagnostics/effect_propagation.ori");
    let module = parse_source(&source).module;
    let bodies = parse_module_bodies(&source);
    let mut graph = build_effect_graph(&module, &bodies);
    let _ = propagate_effects(&mut graph);
    let diags = propagation_diagnostics(&module, &graph);
    let actual = diagnostics_filtered_jsonl(&diags, "E0420");
    assert_jsonl_fixture(
        "tests/golden/diagnostics/effect_propagation.expected.jsonl",
        "tests/golden/diagnostics/effect_propagation.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic("tests/golden/diagnostics/effect_propagation.expected.jsonl");
}

// --- Exhaustive: E0540 -----------------------------------------------------

#[test]
fn golden_e0540_exhaustive_missing_arm() {
    let source = load_source("tests/golden/diagnostics/exhaustive_missing_arm.ori");
    let module = parse_source(&source).module;
    let bodies = parse_module_bodies(&source);
    let diags = check_module_matches_with_source(&source, &module, &bodies);
    let actual = diagnostics_filtered_jsonl(&diags, "E0540");
    assert_jsonl_fixture(
        "tests/golden/diagnostics/exhaustive_missing_arm.expected.jsonl",
        "tests/golden/diagnostics/exhaustive_missing_arm.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic(
        "tests/golden/diagnostics/exhaustive_missing_arm.expected.jsonl",
    );
}

// --- Borrow checker: B0010, B0011, B0020, B0030, B0040, B0050 -------------

fn borrow_diagnostics(rel: &str) -> Vec<Diagnostic> {
    let source = load_source(rel);
    let module = parse_source(&source).module;
    borrow::borrow_check_module(&module)
}

#[test]
fn golden_b0010_mut_alias() {
    let diags = borrow_diagnostics("tests/golden/diagnostics/borrow_mut_alias.ori");
    let actual = diagnostics_filtered_jsonl(&diags, "B0010");
    assert_jsonl_fixture(
        "tests/golden/diagnostics/borrow_mut_alias.expected.jsonl",
        "tests/golden/diagnostics/borrow_mut_alias.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic("tests/golden/diagnostics/borrow_mut_alias.expected.jsonl");
}

#[test]
fn golden_b0011_mut_shared_conflict() {
    let diags = borrow_diagnostics("tests/golden/diagnostics/borrow_mut_shared_conflict.ori");
    let actual = diagnostics_filtered_jsonl(&diags, "B0011");
    assert_jsonl_fixture(
        "tests/golden/diagnostics/borrow_mut_shared_conflict.expected.jsonl",
        "tests/golden/diagnostics/borrow_mut_shared_conflict.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic(
        "tests/golden/diagnostics/borrow_mut_shared_conflict.expected.jsonl",
    );
}

#[test]
fn golden_b0020_newtype_confusion() {
    let diags = borrow_diagnostics("tests/golden/diagnostics/borrow_newtype_confusion.ori");
    let actual = diagnostics_filtered_jsonl(&diags, "B0020");
    assert_jsonl_fixture(
        "tests/golden/diagnostics/borrow_newtype_confusion.expected.jsonl",
        "tests/golden/diagnostics/borrow_newtype_confusion.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic(
        "tests/golden/diagnostics/borrow_newtype_confusion.expected.jsonl",
    );
}

#[test]
fn golden_b0030_shared_over_unique() {
    let diags = borrow_diagnostics("tests/golden/diagnostics/borrow_shared_over_unique.ori");
    let actual = diagnostics_filtered_jsonl(&diags, "B0030");
    assert_jsonl_fixture(
        "tests/golden/diagnostics/borrow_shared_over_unique.expected.jsonl",
        "tests/golden/diagnostics/borrow_shared_over_unique.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic(
        "tests/golden/diagnostics/borrow_shared_over_unique.expected.jsonl",
    );
}

#[test]
fn golden_b0040_unsafe_rejected() {
    let diags = borrow_diagnostics("tests/golden/diagnostics/borrow_unsafe.ori");
    let actual = diagnostics_filtered_jsonl(&diags, "B0040");
    assert_jsonl_fixture(
        "tests/golden/diagnostics/borrow_unsafe.expected.jsonl",
        "tests/golden/diagnostics/borrow_unsafe.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic("tests/golden/diagnostics/borrow_unsafe.expected.jsonl");
}

#[test]
fn golden_b0050_dangling_borrow() {
    let diags = borrow_diagnostics("tests/golden/diagnostics/borrow_dangling.ori");
    let actual = diagnostics_filtered_jsonl(&diags, "B0050");
    assert_jsonl_fixture(
        "tests/golden/diagnostics/borrow_dangling.expected.jsonl",
        "tests/golden/diagnostics/borrow_dangling.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic("tests/golden/diagnostics/borrow_dangling.expected.jsonl");
}

// --- SQL: Q0010, Q0020 ----------------------------------------------------

#[test]
fn golden_q0010_unknown_column_type() {
    let source = load_source("tests/golden/diagnostics/sql_unknown_type.ori");
    let module = parse_source(&source).module;
    let diags = check_module_queries(&module);
    let actual = diagnostics_filtered_jsonl(&diags, "Q0010");
    assert_jsonl_fixture(
        "tests/golden/diagnostics/sql_unknown_type.expected.jsonl",
        "tests/golden/diagnostics/sql_unknown_type.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic("tests/golden/diagnostics/sql_unknown_type.expected.jsonl");
}

#[test]
fn golden_q0020_duplicate_shape() {
    // Two source files in the same module deliver two query symbols with
    // distinct ids — the parser dedups within one file but a downstream
    // merge can re-introduce the pair, which is exactly what Q0020 guards.
    let source_a = load_source("tests/golden/diagnostics/sql_query_shape_a.ori");
    let source_b = load_source("tests/golden/diagnostics/sql_query_shape_b.ori");
    let module_a = parse_source(&source_a).module;
    let module_b = parse_source(&source_b).module;
    let mut merged = module_a.clone();
    for sym in module_b.symbols.into_iter() {
        // Re-id the second file's symbols so the parser's per-file dedupe
        // contract is preserved: same name, distinct sym ids.
        let new_id = format!("{}#b", sym.id);
        merged.symbols.push(Symbol { id: new_id, ..sym });
    }
    let diags = check_module_queries(&merged);
    let actual = diagnostics_filtered_jsonl(&diags, "Q0020");
    assert_jsonl_fixture(
        "tests/golden/diagnostics/sql_query_shape.expected.jsonl",
        "tests/golden/diagnostics/sql_query_shape.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic("tests/golden/diagnostics/sql_query_shape.expected.jsonl");
}

// --- Design tokens: D0010, D0020 ------------------------------------------

/// Shared TokenSet used by the design fixtures. Defined inline so the
/// expected JSONL stays self-describing without an extra `tokens.toml`
/// shipped alongside the fixture.
fn sample_design_tokens() -> TokenSet {
    let text = "[colors]\nprimary = \"#3366ff\"\ndanger = \"#cc0033\"\n";
    match TokenSet::from_toml_subset(text) {
        Ok(set) => set,
        Err(err) => {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "sample tokens failed to parse: {err}");
            }
            TokenSet::default()
        }
    }
}

/// Load a fixture but stamp the `Module.path` with the workspace-absolute
/// location so `design_tokens::check_module` (which re-reads the source
/// from disk to scan view bodies) can find it from any CWD.
fn load_source_with_abs_path(rel: &str) -> SourceFile {
    let abs = workspace_root().join(rel);
    let text = read_text(&abs);
    SourceFile::new(abs.to_string_lossy().to_string(), text)
}

#[test]
fn golden_d0010_unknown_design_token() {
    // The design pass scans the view body via `fs::read_to_string(module.path)`
    // to follow `tokens.<category>.<key>` references, so the module must
    // carry an absolute path here.
    let source = load_source_with_abs_path("tests/golden/diagnostics/design_unknown_token.ori");
    let module = parse_source(&source).module;
    let report = design_check_module(&module, &sample_design_tokens());
    let mut diags = design_report_to_diagnostics(&module, &report);
    // Rewrite the absolute `span.file` to the workspace-relative path so
    // the recorded fixture stays portable across machines.
    for d in diags.iter_mut() {
        d.span.file = "tests/golden/diagnostics/design_unknown_token.ori".to_string();
    }
    let actual = diagnostics_filtered_jsonl(&diags, "D0010");
    assert_jsonl_fixture(
        "tests/golden/diagnostics/design_unknown_token.expected.jsonl",
        "tests/golden/diagnostics/design_unknown_token.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic(
        "tests/golden/diagnostics/design_unknown_token.expected.jsonl",
    );
}

#[test]
fn golden_d0020_raw_color_literal() {
    let source = load_source_with_abs_path("tests/golden/diagnostics/design_raw_color.ori");
    let module = parse_source(&source).module;
    let report = design_check_module(&module, &sample_design_tokens());
    let mut diags = design_report_to_diagnostics(&module, &report);
    for d in diags.iter_mut() {
        d.span.file = "tests/golden/diagnostics/design_raw_color.ori".to_string();
    }
    let actual = diagnostics_filtered_jsonl(&diags, "D0020");
    assert_jsonl_fixture(
        "tests/golden/diagnostics/design_raw_color.expected.jsonl",
        "tests/golden/diagnostics/design_raw_color.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic("tests/golden/diagnostics/design_raw_color.expected.jsonl");
}

// --- Mobile validation: MOB0001, MOB0002, MOB0003 -------------------------
// The mobile-pass diagnostic ids use the wider `MOB####` shape rather than
// the single-letter pattern enforced for `tests/golden/diagnostics/*`, so
// fixtures live under `tests/golden/mobile/` and we skip the strict id
// pattern check (the schema regex itself allows them).

#[test]
fn golden_mob0001_unsupported_permission() {
    let source = load_source("tests/golden/mobile/mob0001_unsupported_permission.ori");
    let module = parse_source(&source).module;
    let manifest = build_mobile_manifest(&module, "com.example.demo", &["android"]);
    let diags = validate_mobile_manifest(
        &manifest,
        "tests/golden/mobile/mob0001_unsupported_permission.ori",
    );
    let actual = diagnostics_filtered_jsonl(&diags, "MOB0001");
    assert_jsonl_fixture(
        "tests/golden/mobile/mob0001_unsupported_permission.expected.jsonl",
        "tests/golden/mobile/mob0001_unsupported_permission.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic_loose(
        "tests/golden/mobile/mob0001_unsupported_permission.expected.jsonl",
        true,
    );
}

#[test]
fn golden_mob0002_unsupported_platform() {
    let source = load_source("tests/golden/mobile/mob0002_unsupported_platform.ori");
    let module = parse_source(&source).module;
    let manifest = build_mobile_manifest(&module, "com.example.demo", &["web"]);
    let diags = validate_mobile_manifest(
        &manifest,
        "tests/golden/mobile/mob0002_unsupported_platform.ori",
    );
    let actual = diagnostics_filtered_jsonl(&diags, "MOB0002");
    assert_jsonl_fixture(
        "tests/golden/mobile/mob0002_unsupported_platform.expected.jsonl",
        "tests/golden/mobile/mob0002_unsupported_platform.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic_loose(
        "tests/golden/mobile/mob0002_unsupported_platform.expected.jsonl",
        true,
    );
}

#[test]
fn golden_mob0003_invalid_app_id() {
    let source = load_source("tests/golden/mobile/mob0003_invalid_app_id.ori");
    let module = parse_source(&source).module;
    let manifest = build_mobile_manifest(&module, "oneword", &["ios"]);
    let diags =
        validate_mobile_manifest(&manifest, "tests/golden/mobile/mob0003_invalid_app_id.ori");
    let actual = diagnostics_filtered_jsonl(&diags, "MOB0003");
    assert_jsonl_fixture(
        "tests/golden/mobile/mob0003_invalid_app_id.expected.jsonl",
        "tests/golden/mobile/mob0003_invalid_app_id.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic_loose(
        "tests/golden/mobile/mob0003_invalid_app_id.expected.jsonl",
        true,
    );
}

// --- Preprocessor: P3010, P3020, P3030 ------------------------------------

#[test]
fn golden_p3010_env_not_allowed() {
    let text = read_text(&workspace_root().join("tests/golden/preproc/p3010_env_not_allowed.ori"));
    let (_out, diags) = preprocess(&text, &PreprocessConfig::default());
    let actual = diagnostics_filtered_jsonl(&diags, "P3010");
    assert_jsonl_fixture(
        "tests/golden/preproc/p3010_env_not_allowed.expected.jsonl",
        "tests/golden/preproc/p3010_env_not_allowed.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic("tests/golden/preproc/p3010_env_not_allowed.expected.jsonl");
}

#[test]
fn golden_p3020_const_undeclared() {
    let text = read_text(&workspace_root().join("tests/golden/preproc/p3020_const_undeclared.ori"));
    let (_out, diags) = preprocess(&text, &PreprocessConfig::default());
    let actual = diagnostics_filtered_jsonl(&diags, "P3020");
    assert_jsonl_fixture(
        "tests/golden/preproc/p3020_const_undeclared.expected.jsonl",
        "tests/golden/preproc/p3020_const_undeclared.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic("tests/golden/preproc/p3020_const_undeclared.expected.jsonl");
}

#[test]
fn golden_p3030_marker_in_string() {
    let text = read_text(&workspace_root().join("tests/golden/preproc/p3030_marker_in_string.ori"));
    // Allow USER so the marker reaches the string-literal escape check
    // instead of being intercepted by the env allow-list gate.
    let mut config = PreprocessConfig::default();
    config.allow_env.push("USER".to_string());
    let (_out, diags) = preprocess(&text, &config);
    let actual = diagnostics_filtered_jsonl(&diags, "P3030");
    assert_jsonl_fixture(
        "tests/golden/preproc/p3030_marker_in_string.expected.jsonl",
        "tests/golden/preproc/p3030_marker_in_string.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic("tests/golden/preproc/p3030_marker_in_string.expected.jsonl");
}

// --- Async runtime: A0001, A0002, A0003 -----------------------------------

fn runtime_error_to_json(
    rel_source: &str,
    err: &ori_compiler::interp_exec::RuntimeError,
) -> String {
    let value = serde_json::json!({
        "schema": "ori.runtime.v1",
        "source": rel_source,
        "code": err.code,
        "message": err.message,
        "observed_effects": err.observed_effects,
    });
    format!("{value}\n")
}

#[test]
fn golden_a0001_scheduler_overflow() {
    use ori_compiler::async_runtime::{diagnostics_for, run_to_completion, Scheduler};
    use ori_compiler::interp_exec::Value as V;
    let mut s = Scheduler::new();
    for v in 0..5 {
        let _ = s.spawn(V::Int(v));
    }
    let _ = run_to_completion(&mut s, 2);
    let diags = diagnostics_for(&s, 2, 2);
    let actual = diagnostics_filtered_jsonl(&diags, "A0001");
    assert_jsonl_fixture(
        "tests/golden/async/a0001_scheduler_overflow.expected.jsonl",
        "tests/golden/async/a0001_scheduler_overflow.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic_loose(
        "tests/golden/async/a0001_scheduler_overflow.expected.jsonl",
        false,
    );
}

#[test]
fn golden_a0002_deadlock() {
    use ori_compiler::async_runtime::{diagnostics_for, run_to_completion, Scheduler};
    use ori_compiler::interp_exec::Value as V;
    let mut s = Scheduler::new();
    let id_a = s.spawn(V::Unit);
    let id_b = s.spawn(V::Unit);
    s.park(id_a, V::Unit);
    s.park(id_b, V::Unit);
    let _ = run_to_completion(&mut s, 16);
    let diags = diagnostics_for(&s, 0, 16);
    let actual = diagnostics_filtered_jsonl(&diags, "A0002");
    assert_jsonl_fixture(
        "tests/golden/async/a0002_deadlock.expected.jsonl",
        "tests/golden/async/a0002_deadlock.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic_loose(
        "tests/golden/async/a0002_deadlock.expected.jsonl",
        false,
    );
}

#[test]
fn golden_a0003_future_leak() {
    use ori_compiler::async_runtime::{diagnostics_for, Scheduler};
    use ori_compiler::interp_exec::Value as V;
    let mut s = Scheduler::new();
    let _id = s.spawn(V::Unit);
    // Force the scheduler into the leaked state: queue is empty, the future
    // was never resumed nor parked, but its id is still allocated.
    s.queue.clear();
    let diags = diagnostics_for(&s, 0, 8);
    let actual = diagnostics_filtered_jsonl(&diags, "A0003");
    assert_jsonl_fixture(
        "tests/golden/async/a0003_future_leak.expected.jsonl",
        "tests/golden/async/a0003_future_leak.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic_loose(
        "tests/golden/async/a0003_future_leak.expected.jsonl",
        false,
    );
}

// --- Runtime / interpreter errors: R0001, R0002, R0003, R0004, R0005 ------
// The interpreter surfaces these as `RuntimeError`, not `Diagnostic`. We
// pin a small JSON envelope around the (code, message, observed_effects)
// triple so the contract is still byte-stable across runs.

fn exec_fixture(
    rel_source: &str,
    entry: &str,
) -> Result<RuntimeValue, ori_compiler::interp_exec::RuntimeError> {
    let source = load_source(rel_source);
    let module = parse_source(&source).module;
    let bodies = parse_module_bodies(&source);
    exec_program(&module, &bodies, entry, Vec::new())
}

fn assert_runtime_fixture(rel_source: &str, entry: &str, expected_rel: &str) {
    let err = match exec_fixture(rel_source, entry) {
        Ok(value) => {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(
                    false,
                    "{rel_source}: expected runtime error, got value {value:?}"
                );
            }
            return;
        }
        Err(err) => err,
    };
    let actual = runtime_error_to_json(rel_source, &err);
    let expected_path = workspace_root().join(expected_rel);

    if bless_mode() {
        write_text(&expected_path, &actual);
        return;
    }
    let expected_text = read_text(&expected_path);
    let actual_value = parse_json(actual.trim(), expected_rel);
    let expected_value = parse_json(expected_text.trim(), expected_rel);
    assert_json_eq(expected_rel, &actual_value, &expected_value);
}

#[test]
fn golden_r0001_entry_missing() {
    assert_runtime_fixture(
        "tests/golden/runtime/r0001_entry_missing.ori",
        "missing",
        "tests/golden/runtime/r0001_entry_missing.expected.json",
    );
}

#[test]
fn golden_r0002_unknown_name() {
    assert_runtime_fixture(
        "tests/golden/runtime/r0002_unknown_name.ori",
        "main",
        "tests/golden/runtime/r0002_unknown_name.expected.json",
    );
}

#[test]
fn golden_r0003_arity_mismatch() {
    assert_runtime_fixture(
        "tests/golden/runtime/r0003_arity.ori",
        "main",
        "tests/golden/runtime/r0003_arity.expected.json",
    );
}

#[test]
fn golden_r0004_try_on_non_result() {
    assert_runtime_fixture(
        "tests/golden/runtime/r0004_try_on_non_result.ori",
        "main",
        "tests/golden/runtime/r0004_try_on_non_result.expected.json",
    );
}

#[test]
fn golden_r0005_stack_overflow() {
    assert_runtime_fixture(
        "tests/golden/runtime/r0005_stack_overflow.ori",
        "main",
        "tests/golden/runtime/r0005_stack_overflow.expected.json",
    );
}

// --- Patch IR: P0000, P0001, P0002, P0003, P0004, P0005, P1000, P1001,
//                P1002, P1003, P1010 ---------------------------------------

fn patch_check_fixture(rel_patch: &str) -> Vec<Diagnostic> {
    let text = read_text(&workspace_root().join(rel_patch));
    let result = check_patch_json(rel_patch, &text);
    result.diagnostics
}

fn assert_patch_check_fixture(rel_patch: &str, target_id: &str, expected_rel: &str) {
    let diags = patch_check_fixture(rel_patch);
    let actual = diagnostics_filtered_jsonl(&diags, target_id);
    assert_jsonl_fixture(expected_rel, expected_rel, &actual);
    validate_each_recorded_diagnostic_loose(expected_rel, false);
}

#[test]
fn golden_p0000_invalid_json() {
    assert_patch_check_fixture(
        "tests/golden/patch/p0000_invalid_json.json",
        "P0000",
        "tests/golden/patch/p0000_invalid_json.expected.jsonl",
    );
}

#[test]
fn golden_p0001_wrong_schema() {
    assert_patch_check_fixture(
        "tests/golden/patch/p0001_wrong_schema.json",
        "P0001",
        "tests/golden/patch/p0001_wrong_schema.expected.jsonl",
    );
}

#[test]
fn golden_p0002_no_operations() {
    assert_patch_check_fixture(
        "tests/golden/patch/p0002_no_operations.json",
        "P0002",
        "tests/golden/patch/p0002_no_operations.expected.jsonl",
    );
}

#[test]
fn golden_p0003_root_not_object() {
    assert_patch_check_fixture(
        "tests/golden/patch/p0003_root_not_object.json",
        "P0003",
        "tests/golden/patch/p0003_root_not_object.expected.jsonl",
    );
}

#[test]
fn golden_p0004_empty_intent() {
    assert_patch_check_fixture(
        "tests/golden/patch/p0004_empty_intent.json",
        "P0004",
        "tests/golden/patch/p0004_empty_intent.expected.jsonl",
    );
}

#[test]
fn golden_p0005_operations_not_array() {
    assert_patch_check_fixture(
        "tests/golden/patch/p0005_operations_not_array.json",
        "P0005",
        "tests/golden/patch/p0005_operations_not_array.expected.jsonl",
    );
}

#[test]
fn golden_p1000_op_not_object() {
    assert_patch_check_fixture(
        "tests/golden/patch/p1000_op_not_object.json",
        "P1000",
        "tests/golden/patch/p1000_op_not_object.expected.jsonl",
    );
}

#[test]
fn golden_p1001_op_missing_kind() {
    assert_patch_check_fixture(
        "tests/golden/patch/p1001_op_missing_kind.json",
        "P1001",
        "tests/golden/patch/p1001_op_missing_kind.expected.jsonl",
    );
}

#[test]
fn golden_p1002_unknown_op() {
    assert_patch_check_fixture(
        "tests/golden/patch/p1002_unknown_op.json",
        "P1002",
        "tests/golden/patch/p1002_unknown_op.expected.jsonl",
    );
}

#[test]
fn golden_p1003_rename_missing_args() {
    assert_patch_check_fixture(
        "tests/golden/patch/p1003_rename_missing_args.json",
        "P1003",
        "tests/golden/patch/p1003_rename_missing_args.expected.jsonl",
    );
}

#[test]
fn golden_p1010_stale_target() {
    // P1010 is emitted by the apply engine when an operation references a
    // node id absent from the live CST. Drive the applier directly against
    // the paired .ori source so the diagnostic carries the source path.
    let source = load_source("tests/golden/patch/p1010_stale_target.ori");
    let patch_text =
        read_text(&workspace_root().join("tests/golden/patch/p1010_stale_target.json"));
    let report = apply_patch(&source, &patch_text, true);
    let actual = diagnostics_filtered_jsonl(&report.diagnostics, "P1010");
    assert_jsonl_fixture(
        "tests/golden/patch/p1010_stale_target.expected.jsonl",
        "tests/golden/patch/p1010_stale_target.expected.jsonl",
        &actual,
    );
    validate_each_recorded_diagnostic_loose(
        "tests/golden/patch/p1010_stale_target.expected.jsonl",
        false,
    );
}

// ---------------------------------------------------------------------------
// Coverage notes (subsystems not exercised here)
// ---------------------------------------------------------------------------
// The following diagnostic families are owned by subsystems that the
// bootstrap CLI does not currently expose as standalone passes, so the
// conformance suite intentionally does not bind a fixture for them:
//
//   * Runtime-only follow-ups beyond R0005 (panic conversion paths) live
//     entirely inside `interp_exec` and have no observable surface beyond
//     the five entry points above; broaden once a dedicated `ori exec`
//     subcommand stabilises.
//   * Wasm component runtime errors (W2*) and incremental cache eviction
//     warnings (I0*) require multi-build orchestration that the
//     conformance harness cannot model without a fixture sandbox.
//
// Re-evaluate whenever a new public pass entry point is added.
