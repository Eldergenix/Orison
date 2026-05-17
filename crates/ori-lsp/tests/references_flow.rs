//! `textDocument/references` end-to-end coverage.
//!
//! Opens a document with `f` used three times as an identifier and one extra
//! occurrence inside a string literal. The server must return exactly three
//! locations, deduplicated and excluding the string-literal hit.

use std::io::Cursor;

use ori_lsp::codec::{read_message, write_message};
use ori_lsp::Server;
use serde_json::{json, Value};

fn framed(payload: &Value) -> Vec<u8> {
    let bytes = serde_json::to_vec(payload).expect("encode");
    let mut buf = Vec::new();
    write_message(&mut buf, &bytes).expect("frame");
    buf
}

fn read_all_messages(bytes: Vec<u8>) -> Vec<Value> {
    let mut reader = Cursor::new(bytes);
    let mut out = Vec::new();
    while let Some(payload) = read_message(&mut reader).expect("decode frame") {
        let value: Value = serde_json::from_slice(&payload).expect("decode body");
        out.push(value);
    }
    out
}

// `f` appears three times as an identifier (decl, recursive call, trailing
// reference) plus once inside a string literal which must NOT be matched.
//
// 0-based view:
//   line 0: "module sample.refs"
//   line 1: ""
//   line 2: "fn f() -> Unit:"      <- declaration occurrence #1
//   line 3: "  let s = \"f only inside string\""
//   line 4: "  f()"                <- call occurrence #2
//   line 5: "  f"                  <- reference occurrence #3
const SOURCE: &str =
    "module sample.refs\n\nfn f() -> Unit:\n  let s = \"f only inside string\"\n  f()\n  f\n";

fn run_references(include_declaration: bool, cursor_line: u32, cursor_char: u32) -> Vec<Value> {
    let mut input = Vec::new();
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    })));
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": "file:///refs.ori",
                "languageId": "orison",
                "version": 1,
                "text": SOURCE
            }
        }
    })));
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/references",
        "params": {
            "textDocument": {"uri": "file:///refs.ori"},
            "position": {"line": cursor_line, "character": cursor_char},
            "context": {"includeDeclaration": include_declaration}
        }
    })));
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));

    let mut output: Vec<u8> = Vec::new();
    Server::new()
        .run(Cursor::new(input), &mut output)
        .expect("server run");
    read_all_messages(output)
}

#[test]
fn references_with_declaration_returns_three_hits_excluding_string_literal() {
    // Cursor lands on `f` of the call site `f()`.
    let messages = run_references(true, 4, 2);
    let resp = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("references response present");
    let locations = resp["result"]
        .as_array()
        .expect("references returns Location[]");

    assert_eq!(
        locations.len(),
        3,
        "expected exactly 3 identifier occurrences (string literal excluded), got {:#?}",
        locations
    );

    // No location may sit inside the string-literal line at column >= 10
    // (the literal `"f only..."` begins at column 10 on 0-based line 3).
    for loc in locations {
        let line = loc["range"]["start"]["line"].as_u64().unwrap_or(0);
        let character = loc["range"]["start"]["character"].as_u64().unwrap_or(0);
        if line == 3 {
            assert!(
                character < 10,
                "reference inside string literal at line 3 column {character}"
            );
        }
    }

    // Dedup invariant: no two locations may share the same start position.
    let mut seen: std::collections::HashSet<(u64, u64)> = std::collections::HashSet::new();
    for loc in locations {
        let line = loc["range"]["start"]["line"].as_u64().unwrap_or(0);
        let character = loc["range"]["start"]["character"].as_u64().unwrap_or(0);
        assert!(
            seen.insert((line, character)),
            "duplicate Location at line {line} column {character}"
        );
    }
}

#[test]
fn references_without_declaration_strips_definition_span() {
    let messages = run_references(false, 4, 2);
    let resp = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("references response present");
    let locations = resp["result"]
        .as_array()
        .expect("references returns Location[]");

    // Definition is `fn f` on line 2 (0-based) starting at column 0. With
    // `includeDeclaration = false`, only the two callsite uses survive.
    assert_eq!(
        locations.len(),
        2,
        "expected 2 references when declaration is excluded, got {:#?}",
        locations
    );
    for loc in locations {
        let line = loc["range"]["start"]["line"].as_u64().unwrap_or(0);
        assert_ne!(line, 2, "declaration line 2 must be stripped");
    }
}
