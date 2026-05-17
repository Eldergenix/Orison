//! Stage 1 self-hosting parity tests.
//!
//! These tests assert that the Orison-in-Orison front-end prototype under
//! `compiler/stage1/` parses clean against the Rust bootstrap and that it
//! declares the surface symbols required by `docs/compiler/SELF_HOSTING.md`
//! ("Stage 1 prototype status"). Bodies are out of scope at Stage 1; only
//! the shape of the declarations and the determinism of structured output
//! are checked here.

use std::path::PathBuf;

use ori_compiler::ast::SymbolKind;
use ori_compiler::json::to_json;
use ori_compiler::{Compiler, SourceFile};

/// Locate `compiler/stage1/<file>` relative to the workspace root regardless
/// of which crate directory `cargo test` is invoked from.
fn stage1_path(file: &str) -> PathBuf {
    // CARGO_MANIFEST_DIR points at `crates/ori-compiler/`; the workspace
    // root is two parents up.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("compiler")
        .join("stage1")
        .join(file)
}

fn load_stage1(file: &str) -> SourceFile {
    let path = stage1_path(file);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    // Use the project-relative path as the pseudo-path so diagnostic
    // envelopes match what `ori check compiler/stage1/<file>` would emit.
    let logical = format!("compiler/stage1/{file}");
    SourceFile::new(logical, text)
}

#[test]
fn stage1_parser_module_parses() {
    let result = Compiler::check_source(load_stage1("parser.ori"));
    let errors: Vec<_> = result.diagnostics.iter().filter(|d| d.is_error()).collect();
    assert!(
        errors.is_empty(),
        "compiler/stage1/parser.ori produced errors: {:#?}",
        errors,
    );
    assert_eq!(result.module.name, "compiler.stage1.parser");
}

#[test]
fn stage1_lexer_module_parses() {
    let result = Compiler::check_source(load_stage1("lexer.ori"));
    let errors: Vec<_> = result.diagnostics.iter().filter(|d| d.is_error()).collect();
    assert!(
        errors.is_empty(),
        "compiler/stage1/lexer.ori produced errors: {:#?}",
        errors,
    );
    assert_eq!(result.module.name, "compiler.stage1.lexer");
}

#[test]
fn stage1_modules_declare_expected_symbols() {
    let parser = Compiler::check_source(load_stage1("parser.ori"));
    let lexer = Compiler::check_source(load_stage1("lexer.ori"));

    let parser_names: Vec<(&str, SymbolKind)> = parser
        .module
        .exported_symbols()
        .map(|s| (s.name.as_str(), s.kind))
        .collect();
    let lexer_names: Vec<(&str, SymbolKind)> = lexer
        .module
        .exported_symbols()
        .map(|s| (s.name.as_str(), s.kind))
        .collect();

    assert!(
        parser_names.contains(&("ModuleDecl", SymbolKind::Type)),
        "parser.ori should declare type ModuleDecl; saw {:?}",
        parser_names,
    );
    assert!(
        parser_names.contains(&("ItemDecl", SymbolKind::Type)),
        "parser.ori should declare type ItemDecl; saw {:?}",
        parser_names,
    );
    assert!(
        parser_names.contains(&("parse_module", SymbolKind::Function)),
        "parser.ori should declare fn parse_module; saw {:?}",
        parser_names,
    );
    assert!(
        lexer_names.contains(&("Token", SymbolKind::Type)),
        "lexer.ori should declare type Token; saw {:?}",
        lexer_names,
    );
    assert!(
        lexer_names.contains(&("lex", SymbolKind::Function)),
        "lexer.ori should declare fn lex; saw {:?}",
        lexer_names,
    );
}

#[test]
fn stage1_byte_stable_across_runs() {
    let first = Compiler::check_source(load_stage1("parser.ori"));
    let second = Compiler::check_source(load_stage1("parser.ori"));
    let first_json = to_json(&first.module);
    let second_json = to_json(&second.module);
    assert_eq!(
        first_json, second_json,
        "ModuleDecl JSON must round-trip byte-identical across runs",
    );

    // Also verify the lexer prototype is deterministic.
    let lex_first = Compiler::check_source(load_stage1("lexer.ori"));
    let lex_second = Compiler::check_source(load_stage1("lexer.ori"));
    assert_eq!(
        to_json(&lex_first.module),
        to_json(&lex_second.module),
        "lexer Module JSON must round-trip byte-identical across runs",
    );
}
