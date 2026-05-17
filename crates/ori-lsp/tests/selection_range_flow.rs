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

const SOURCE: &str = "module sel.demo\n\nfn greet() -> Unit:\n  return Unit\n";

#[test]
fn selection_range_capability_advertised() {
    let mut input = Vec::new();
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    })));
    input.extend_from_slice(&framed(&json!({"jsonrpc": "2.0", "method": "exit"})));
    let mut output: Vec<u8> = Vec::new();
    Server::new()
        .run(Cursor::new(input), &mut output)
        .expect("server run");
    let messages = read_all_messages(output);
    assert_eq!(
        messages[0]["result"]["capabilities"]["selectionRangeProvider"],
        true
    );
}

#[test]
fn selection_range_nests_identifier_within_line_within_module() {
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
                "uri": "file:///sel.ori",
                "languageId": "orison",
                "version": 1,
                "text": SOURCE,
            }
        }
    })));
    // Cursor sits inside the `greet` identifier on line 2 (0-based).
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/selectionRange",
        "params": {
            "textDocument": {"uri": "file:///sel.ori"},
            "positions": [{"line": 2, "character": 4}]
        }
    })));
    input.extend_from_slice(&framed(&json!({"jsonrpc": "2.0", "method": "exit"})));

    let mut output: Vec<u8> = Vec::new();
    Server::new()
        .run(Cursor::new(input), &mut output)
        .expect("server run");
    let messages = read_all_messages(output);
    let response = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("selection range response");
    let ranges = response["result"].as_array().expect("range array");
    assert_eq!(ranges.len(), 1);
    let inner = &ranges[0];
    // Inner range should hit the identifier.
    assert_eq!(inner["range"]["start"]["line"], 2);
    let parent = &inner["parent"];
    assert!(!parent.is_null(), "selection range nests at least once");
    // The very outermost frame is always the whole document.
    let mut depth = 0;
    let mut current = inner.clone();
    while !current["parent"].is_null() {
        current = current["parent"].clone();
        depth += 1;
    }
    assert!(depth >= 2, "expected ≥3 nesting levels, got {depth} levels");
}

#[test]
fn selection_range_handles_multiple_positions() {
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
                "uri": "file:///sel.ori",
                "languageId": "orison",
                "version": 1,
                "text": SOURCE,
            }
        }
    })));
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/selectionRange",
        "params": {
            "textDocument": {"uri": "file:///sel.ori"},
            "positions": [
                {"line": 0, "character": 7},
                {"line": 2, "character": 4}
            ]
        }
    })));
    input.extend_from_slice(&framed(&json!({"jsonrpc": "2.0", "method": "exit"})));

    let mut output: Vec<u8> = Vec::new();
    Server::new()
        .run(Cursor::new(input), &mut output)
        .expect("server run");
    let messages = read_all_messages(output);
    let response = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("selection range response");
    let ranges = response["result"].as_array().expect("range array");
    assert_eq!(ranges.len(), 2);
}
