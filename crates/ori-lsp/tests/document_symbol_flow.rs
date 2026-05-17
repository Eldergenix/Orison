//! `textDocument/documentSymbol` end-to-end coverage.
//!
//! Opens a document containing a function, a service, and a type and asserts
//! the returned flat `SymbolInformation` list carries the correct LSP
//! `SymbolKind` integers per the spec.

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

const SOURCE: &str =
    "module sample.mixed\n\nfn hello() -> Unit:\n  return Unit\n\ntype User\n\nservice Greeter\n";

#[test]
fn document_symbol_returns_flat_list_with_correct_kinds() {
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
                "uri": "file:///mixed.ori",
                "languageId": "orison",
                "version": 1,
                "text": SOURCE
            }
        }
    })));
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/documentSymbol",
        "params": {
            "textDocument": {"uri": "file:///mixed.ori"}
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
    let messages = read_all_messages(output);

    let symbol_resp = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("documentSymbol response present");
    let items = symbol_resp["result"]
        .as_array()
        .expect("documentSymbol returns array");

    // Flat shape: no `children` field per item.
    for item in items {
        assert!(
            item.get("children").is_none(),
            "expected flat SymbolInformation list, item has children: {item}"
        );
        assert!(
            item["location"]["uri"].as_str() == Some("file:///mixed.ori"),
            "location URI must match the requested document: {item}"
        );
        assert!(
            item["kind"].is_number(),
            "SymbolKind must be the LSP integer per spec: {item}"
        );
    }

    let by_name: std::collections::HashMap<&str, &Value> = items
        .iter()
        .filter_map(|i| i["name"].as_str().map(|n| (n, i)))
        .collect();

    let hello = by_name.get("hello").expect("hello function present");
    assert_eq!(
        hello["kind"], 12,
        "fn should map to SymbolKind::Function=12"
    );

    let user = by_name.get("User").expect("User type present");
    assert_eq!(user["kind"], 5, "type should map to SymbolKind::Class=5");

    let greeter = by_name.get("Greeter").expect("Greeter service present");
    assert_eq!(
        greeter["kind"], 5,
        "service should map to SymbolKind::Class=5"
    );
}
