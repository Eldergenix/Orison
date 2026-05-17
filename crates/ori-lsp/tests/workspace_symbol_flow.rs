//! `workspace/symbol` end-to-end coverage.
//!
//! Opens two documents, dispatches a substring query against the workspace,
//! and asserts that hits from both documents are returned sorted alongside the
//! spec-compliant integer `SymbolKind` payload.

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

const DOC_A: &str =
    "module sample.alpha\n\nfn alpha_one() -> Unit:\n  return Unit\n\nfn shared() -> Unit:\n  return Unit\n";
const DOC_B: &str =
    "module sample.beta\n\nfn beta_one() -> Unit:\n  return Unit\n\nfn shared_beta() -> Unit:\n  return Unit\n";

fn build_open_pair_input(query: &str) -> Vec<u8> {
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
                "uri": "file:///alpha.ori",
                "languageId": "orison",
                "version": 1,
                "text": DOC_A
            }
        }
    })));
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": "file:///beta.ori",
                "languageId": "orison",
                "version": 1,
                "text": DOC_B
            }
        }
    })));
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "workspace/symbol",
        "params": {"query": query}
    })));
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));
    input
}

#[test]
fn initialize_advertises_workspace_and_document_symbol_capabilities() {
    let mut input = Vec::new();
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    })));
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));

    let mut output: Vec<u8> = Vec::new();
    Server::new()
        .run(Cursor::new(input), &mut output)
        .expect("server run");
    let messages = read_all_messages(output);
    let caps = &messages[0]["result"]["capabilities"];
    assert_eq!(caps["workspaceSymbolProvider"], true);
    assert_eq!(caps["documentSymbolProvider"], true);
    assert_eq!(caps["definitionProvider"], true);
    assert_eq!(caps["referencesProvider"], true);
}

#[test]
fn workspace_symbol_returns_hits_from_both_documents_sorted() {
    let input = build_open_pair_input("shared");

    let mut output: Vec<u8> = Vec::new();
    Server::new()
        .run(Cursor::new(input), &mut output)
        .expect("server run");
    let messages = read_all_messages(output);

    let symbol_resp = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("workspace/symbol response present");
    let items = symbol_resp["result"]
        .as_array()
        .expect("result is symbol array");
    let names: Vec<&str> = items.iter().filter_map(|i| i["name"].as_str()).collect();

    assert!(
        names.contains(&"shared"),
        "expected `shared` from alpha.ori in {names:?}"
    );
    assert!(
        names.contains(&"shared_beta"),
        "expected `shared_beta` from beta.ori in {names:?}"
    );

    // Sorted alphabetically, the senior-review quality gate.
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "workspace/symbol must be sorted by name");

    // `SymbolKind` must be the LSP integer per spec, not a string.
    for item in items {
        assert!(
            item["kind"].is_number(),
            "SymbolKind must be wire-encoded as an integer: {item:?}"
        );
    }
}

#[test]
fn workspace_symbol_empty_query_returns_all_symbols_capped_at_100() {
    let input = build_open_pair_input("");

    let mut output: Vec<u8> = Vec::new();
    Server::new()
        .run(Cursor::new(input), &mut output)
        .expect("server run");
    let messages = read_all_messages(output);

    let symbol_resp = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("workspace/symbol response present");
    let items = symbol_resp["result"]
        .as_array()
        .expect("result is symbol array");
    let names: Vec<&str> = items.iter().filter_map(|i| i["name"].as_str()).collect();

    // Two functions per document, four symbols total.
    assert_eq!(
        names.len(),
        4,
        "all symbols (no module entries) in {names:?}"
    );
    assert!(items.len() <= 100, "spec cap honoured");
    assert!(names.contains(&"alpha_one"));
    assert!(names.contains(&"beta_one"));
    assert!(names.contains(&"shared"));
    assert!(names.contains(&"shared_beta"));
}

#[test]
fn workspace_symbol_query_is_case_insensitive() {
    let input = build_open_pair_input("ALPHA");

    let mut output: Vec<u8> = Vec::new();
    Server::new()
        .run(Cursor::new(input), &mut output)
        .expect("server run");
    let messages = read_all_messages(output);

    let symbol_resp = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("workspace/symbol response present");
    let items = symbol_resp["result"]
        .as_array()
        .expect("result is symbol array");
    let names: Vec<&str> = items.iter().filter_map(|i| i["name"].as_str()).collect();
    assert!(
        names.contains(&"alpha_one"),
        "case-insensitive match should hit `alpha_one`: {names:?}"
    );
}
