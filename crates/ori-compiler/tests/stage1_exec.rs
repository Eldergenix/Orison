//! Stage 1 self-hosting *execution* tests.
//!
//! Complements `stage1_parity.rs` (which proves the prototype parses
//! clean and exports the required surface symbols) by actually running
//! the Stage 1 `compiler/stage1/{lexer,parser}.ori` modules against
//! synthetic Orison inputs and asserting on the returned runtime
//! [`Value`].
//!
//! ## Why these tests live here, not in `stage1_parity.rs`
//!
//! `stage1_parity.rs` enforces the *shape* gate (zero errors, expected
//! symbols, byte-stable JSON). This file enforces the *behavioural*
//! gate: that the bodies the prototype ships today actually execute
//! against the bootstrap interpreter (`exec_program`) and produce the
//! `Token` / `ModuleDecl` records `docs/compiler/SELF_HOSTING.md`
//! §"Stage 1 prototype status" promises. The two suites are run in
//! the same CI lane so a regression in either gate is caught
//! immediately.
//!
//! ## What stays out of scope
//!
//! General-purpose tokenisation / parsing of arbitrary `.ori` sources
//! remains the Rust bootstrap's job until the M27 runtime primitives
//! tracked in SELF_HOSTING.md §3.2 land (lambdas inside top-level
//! bodies, non-destructive `list.head`/`list.tail`, runtime
//! string-to-int conversion). These tests exercise only the fixture
//! envelope the prototype documents — anything outside that envelope
//! is the Rust bootstrap's contract.

use std::collections::BTreeMap;
use std::path::PathBuf;

use ori_compiler::body::parse_module_bodies;
use ori_compiler::interp_exec::{exec_program, Value};
use ori_compiler::parser::parse_source;
use ori_compiler::{Compiler, SourceFile};

/// Locate `compiler/stage1/<file>` relative to the workspace root
/// regardless of which crate directory `cargo test` is invoked from.
/// Mirrors the helper in `stage1_parity.rs` so the two suites pick up
/// identical source artefacts.
fn stage1_path(file: &str) -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("compiler")
        .join("stage1")
        .join(file)
}

/// Load a Stage 1 `.ori` file from disk and validate that the bootstrap
/// parser reports zero errors. Bails the test (via `panic!`) with the
/// diagnostic dump on failure so the regression surface is obvious.
fn load_stage1(file: &str) -> SourceFile {
    let path = stage1_path(file);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    let logical = format!("compiler/stage1/{file}");
    let source = SourceFile::new(logical, text);
    let result = Compiler::check_source(source.clone());
    let errors: Vec<_> = result.diagnostics.iter().filter(|d| d.is_error()).collect();
    assert!(
        errors.is_empty(),
        "compiler/stage1/{file} should compile clean before exec: {:#?}",
        errors,
    );
    source
}

/// Run the named entry function in `file` with `args`, returning the
/// produced [`Value`]. Bails with a descriptive message on any
/// runtime error so the failure shows the stable `R####` code.
fn run_stage1(file: &str, entry: &str, args: Vec<Value>) -> Value {
    let source = load_stage1(file);
    let module = parse_source(&source).module;
    let bodies = parse_module_bodies(&source);
    match exec_program(&module, &bodies, entry, args) {
        Ok(value) => value,
        Err(err) => panic!(
            "exec_program({file}, {entry}) failed: {} {}",
            err.code, err.message
        ),
    }
}

/// Convenience: extract a string field from a record value or panic
/// with a clear error message naming the missing field.
fn record_str(value: &Value, field: &str) -> String {
    let Value::Record(fields) = value else {
        panic!("expected Record, got {:?}", value.type_tag());
    };
    match fields.get(field) {
        Some(Value::Str(s)) => s.clone(),
        Some(other) => panic!("field `{field}` expected Str, got {:?}", other.type_tag()),
        None => panic!(
            "missing field `{field}` in record (have: {:?})",
            fields.keys().collect::<Vec<_>>()
        ),
    }
}

/// Convenience: extract a `List[Value]` payload or panic.
fn list_items(value: &Value) -> &[Value] {
    let Value::List(items) = value else {
        panic!("expected List, got {:?}", value.type_tag());
    };
    items.as_slice()
}

/// True when the record carries `tag == expected`. Used by the lexer
/// tests to inspect Token records by tag without unwrapping a chain
/// of variant constructors (the bootstrap interpreter does not yet
/// preserve constructor tags through `Construct`, so the Stage 1
/// prototype materialises tokens as records).
fn record_has_tag(value: &Value, expected: &str) -> bool {
    match value {
        Value::Record(fields) => matches!(fields.get("tag"), Some(Value::Str(s)) if s == expected),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Lexer execution tests
// ---------------------------------------------------------------------------

#[test]
fn stage1_lexer_tokenizes_module_header() {
    // The Stage 1 lexer operates at line granularity: a trailing `\n`
    // emits an explicit `Newline` token after the classified head. For
    // `module greeter\n` we expect a `Module` record carrying
    // `name == "greeter"` followed by a `Newline` record.
    let result = run_stage1(
        "lexer.ori",
        "lex",
        vec![Value::Str("module greeter\n".to_string())],
    );
    let tokens = list_items(&result);
    assert_eq!(
        tokens.len(),
        2,
        "expected exactly 2 tokens for `module greeter\\n`, got {:?}",
        tokens,
    );
    assert!(
        record_has_tag(&tokens[0], "Module"),
        "first token should be Module, got {:?}",
        tokens[0],
    );
    assert_eq!(
        record_str(&tokens[0], "name"),
        "greeter",
        "Module token should carry the parsed name",
    );
    assert!(
        record_has_tag(&tokens[1], "Newline"),
        "second token should be Newline, got {:?}",
        tokens[1],
    );
}

#[test]
fn stage1_lexer_handles_strings() {
    // A bare `"hello"` line classifies as a `Str` token whose `value`
    // strips both surrounding double-quotes. No trailing `\n` in the
    // input → no Newline emitted.
    let result = run_stage1(
        "lexer.ori",
        "lex",
        vec![Value::Str("\"hello\"".to_string())],
    );
    let tokens = list_items(&result);
    assert_eq!(
        tokens.len(),
        1,
        "expected 1 token for `\"hello\"`, got {:?}",
        tokens
    );
    assert!(
        record_has_tag(&tokens[0], "Str"),
        "expected Str token, got {:?}",
        tokens[0],
    );
    assert_eq!(record_str(&tokens[0], "value"), "hello");
}

#[test]
fn stage1_lexer_handles_integers() {
    // `42` classifies as an `Int` token. The lexeme is captured
    // verbatim because the bootstrap interpreter has no runtime
    // string-to-int conversion (M27-deferred, see SELF_HOSTING.md
    // §3.2); the assertion below mirrors that contract.
    let result = run_stage1("lexer.ori", "lex", vec![Value::Str("42".to_string())]);
    let tokens = list_items(&result);
    assert_eq!(
        tokens.len(),
        1,
        "expected 1 token for `42`, got {:?}",
        tokens
    );
    assert!(
        record_has_tag(&tokens[0], "Int"),
        "expected Int token, got {:?}",
        tokens[0],
    );
    assert_eq!(record_str(&tokens[0], "lexeme"), "42");
}

// ---------------------------------------------------------------------------
// Parser execution tests
// ---------------------------------------------------------------------------

/// Unwrap an `Ok_(inner)` value or panic with the wrapped `Err_`
/// payload printed so the failure surface is human-readable.
fn assert_ok(value: Value) -> Value {
    match value {
        Value::Ok_(inner) => *inner,
        Value::Err_(inner) => panic!("expected Ok, got Err({inner:?})"),
        other => panic!("expected Ok, got {:?}", other.type_tag()),
    }
}

/// Pull the `name`, `imports`, `items` triple out of a parsed
/// `ModuleDecl` record, panicking on any missing field.
fn unpack_module_decl(value: &Value) -> (String, &Vec<Value>, &Vec<Value>) {
    let Value::Record(fields) = value else {
        panic!("expected ModuleDecl Record, got {:?}", value.type_tag());
    };
    let name = match fields.get("name") {
        Some(Value::Str(s)) => s.clone(),
        other => panic!("ModuleDecl.name should be Str, got {other:?}"),
    };
    let imports = match fields.get("imports") {
        Some(Value::List(items)) => items,
        other => panic!("ModuleDecl.imports should be List, got {other:?}"),
    };
    let items = match fields.get("items") {
        Some(Value::List(items)) => items,
        other => panic!("ModuleDecl.items should be List, got {other:?}"),
    };
    (name, imports, items)
}

#[test]
fn stage1_parser_parses_empty_module() {
    // `module a` with no imports and no items round-trips to a
    // `ModuleDecl { name: "a", imports: [], items: [] }`.
    let raw = run_stage1(
        "parser.ori",
        "parse_module",
        vec![Value::Str("module a".to_string())],
    );
    let ok = assert_ok(raw);
    let (name, imports, items) = unpack_module_decl(&ok);
    assert_eq!(name, "a");
    assert!(imports.is_empty(), "expected no imports, got {imports:?}");
    assert!(items.is_empty(), "expected no items, got {items:?}");
}

#[test]
fn stage1_parser_parses_imports() {
    // Two `uses` lines after the module header → imports.len() == 2.
    // The Stage 1 prototype recognises the exact `(b, c.d)` fixture
    // documented in `parser.ori::collect_imports`; this test pins the
    // contract so a regression in the fixture envelope is caught.
    let raw = run_stage1(
        "parser.ori",
        "parse_module",
        vec![Value::Str("module a\nuses b\nuses c.d".to_string())],
    );
    let ok = assert_ok(raw);
    let (name, imports, items) = unpack_module_decl(&ok);
    assert_eq!(name, "a");
    assert_eq!(imports.len(), 2, "expected 2 imports, got {imports:?}",);
    assert!(items.is_empty(), "expected no items, got {items:?}");
    // Spot-check the import payloads — the prototype stores them as
    // Str so the assertion is direct.
    let import_strings: Vec<String> = imports
        .iter()
        .map(|v| match v {
            Value::Str(s) => s.clone(),
            other => panic!("import should be Str, got {other:?}"),
        })
        .collect();
    assert_eq!(import_strings, vec!["b".to_string(), "c.d".to_string()]);
}

#[test]
fn stage1_parser_parses_fn_decl() {
    // A module with a single `fn greet() -> Str` produces exactly one
    // `Function`-tagged `ItemDecl`. The Stage 1 prototype stores the
    // verbatim header in `signature`; the assertion below pins the
    // tag and the extracted `name`, leaving `signature` content as a
    // free-form payload to keep the test resilient against future
    // header-normalisation tweaks.
    let raw = run_stage1(
        "parser.ori",
        "parse_module",
        vec![Value::Str("module a\nfn greet() -> Str".to_string())],
    );
    let ok = assert_ok(raw);
    let (name, imports, items) = unpack_module_decl(&ok);
    assert_eq!(name, "a");
    assert!(imports.is_empty(), "expected no imports, got {imports:?}");
    assert_eq!(items.len(), 1, "expected 1 item, got {items:?}");
    assert!(
        record_has_tag(&items[0], "Function"),
        "expected Function item, got {:?}",
        items[0],
    );
    let item_name = record_str(&items[0], "name");
    assert_eq!(
        item_name, "greet",
        "expected item name `greet`, got `{item_name}`",
    );
}

// ---------------------------------------------------------------------------
// Determinism gate — run twice, expect byte-identical outputs.
// ---------------------------------------------------------------------------

#[test]
fn stage1_exec_is_deterministic_across_runs() {
    // Two runs over the same input must produce identical `Value`
    // structures, including the `BTreeMap` ordering inside `Record`.
    // This is the §5.4 determinism gate translated into runtime form
    // — `stage1_byte_stable_across_runs` covers it at the AST level.
    let first = run_stage1(
        "parser.ori",
        "parse_module",
        vec![Value::Str("module a\nuses b\nuses c.d".to_string())],
    );
    let second = run_stage1(
        "parser.ori",
        "parse_module",
        vec![Value::Str("module a\nuses b\nuses c.d".to_string())],
    );
    assert_eq!(first, second, "Stage 1 parse_module must be deterministic");
}

// ---------------------------------------------------------------------------
// Convenience: silence the `unused_imports` lint on `BTreeMap`.
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn _btreemap_use() -> BTreeMap<String, Value> {
    BTreeMap::new()
}
