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

fn run(uri: &str, text: &str, id: i64) -> Vec<Value> {
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
                "uri": uri,
                "languageId": "orison",
                "version": 1,
                "text": text,
            }
        }
    })));
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "textDocument/formatting",
        "params": {
            "textDocument": {"uri": uri},
            "options": {"tabSize": 2, "insertSpaces": true},
        }
    })));
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "method": "exit",
    })));
    let mut output: Vec<u8> = Vec::new();
    Server::new()
        .run(Cursor::new(input), &mut output)
        .expect("server run");
    read_all_messages(output)
}

#[test]
fn formatting_capability_advertised() {
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
        messages[0]["result"]["capabilities"]["documentFormattingProvider"],
        true
    );
}

#[test]
fn formatting_returns_full_replace_when_input_has_trailing_whitespace() {
    let dirty = "module fmt.demo  \n\nfn main() -> Unit:\n  return Unit \n";
    let messages = run("file:///fmt.ori", dirty, 2);
    let response = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("formatting response present");
    let edits = response["result"].as_array().expect("edits array");
    assert_eq!(edits.len(), 1, "single full-document edit");
    let edit = &edits[0];
    assert_eq!(edit["range"]["start"]["line"], 0);
    assert_eq!(edit["range"]["start"]["character"], 0);
    let new_text = edit["newText"].as_str().expect("newText is string");
    assert!(!new_text.contains("  \n"));
    assert!(!new_text.contains(" \n"));
}

#[test]
fn formatting_returns_empty_when_input_already_clean() {
    let clean = "module fmt.clean\n\nfn main() -> Unit:\n  return Unit\n";
    let messages = run("file:///fmt-clean.ori", clean, 3);
    let response = messages
        .iter()
        .find(|m| m["id"] == 3)
        .expect("formatting response present");
    let edits = response["result"].as_array().expect("edits array");
    assert!(edits.is_empty(), "no edits for already-formatted source");
}
