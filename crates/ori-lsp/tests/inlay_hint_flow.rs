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

const SOURCE: &str = concat!(
    "module hints.demo\n",
    "\n",
    "fn main() -> Unit:\n",
    "  let answer = 42\n",
    "  let greeting = \"hi\"\n",
    "  return Unit\n",
);

#[test]
fn inlay_hint_capability_advertised() {
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
        messages[0]["result"]["capabilities"]["inlayHintProvider"],
        true
    );
}

#[test]
fn inlay_hint_emits_one_hint_per_unannotated_let() {
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
                "uri": "file:///hints.ori",
                "languageId": "orison",
                "version": 1,
                "text": SOURCE,
            }
        }
    })));
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/inlayHint",
        "params": {
            "textDocument": {"uri": "file:///hints.ori"},
            "range": {
                "start": {"line": 0, "character": 0},
                "end": {"line": 99, "character": 0}
            }
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
        .expect("inlay hint response");
    let hints = response["result"].as_array().expect("hint array");
    assert_eq!(hints.len(), 2);

    // The `answer` binding sits on line 3 (0-based). The hint position
    // points one column past the binding name.
    let answer = &hints[0];
    assert_eq!(answer["position"]["line"], 3);
    assert!(answer["label"].as_str().unwrap().starts_with(": "));
    assert_eq!(answer["kind"], 1);
}

#[test]
fn inlay_hint_filters_to_request_range() {
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
                "uri": "file:///hints.ori",
                "languageId": "orison",
                "version": 1,
                "text": SOURCE,
            }
        }
    })));
    // Only the first let line (line index 3) is in the requested range.
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/inlayHint",
        "params": {
            "textDocument": {"uri": "file:///hints.ori"},
            "range": {
                "start": {"line": 3, "character": 0},
                "end": {"line": 3, "character": 99}
            }
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
        .expect("inlay hint response");
    let hints = response["result"].as_array().expect("hint array");
    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0]["position"]["line"], 3);
}
