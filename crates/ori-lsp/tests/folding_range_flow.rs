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

const FOLDABLE: &str = concat!(
    "module fold.demo\n",
    "\n",
    "fn classify(n: Int) -> Unit:\n",
    "  match n:\n",
    "    case 0:\n",
    "      return Unit\n",
    "    case _:\n",
    "      return Unit\n",
);

#[test]
fn folding_range_capability_advertised() {
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
        messages[0]["result"]["capabilities"]["foldingRangeProvider"],
        true
    );
}

#[test]
fn folding_range_returns_fn_and_match_ranges() {
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
                "uri": "file:///fold.ori",
                "languageId": "orison",
                "version": 1,
                "text": FOLDABLE,
            }
        }
    })));
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/foldingRange",
        "params": {"textDocument": {"uri": "file:///fold.ori"}}
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
        .expect("foldingRange response");
    let ranges = response["result"].as_array().expect("range array");
    assert!(!ranges.is_empty());
    // The `fn classify` header sits on line index 2 and the body extends
    // through to the last indented line.
    let has_fn = ranges
        .iter()
        .any(|r| r["startLine"] == 2 && r["endLine"].as_u64().unwrap() >= 5);
    assert!(has_fn, "expected fn body to be foldable");
    let has_match = ranges
        .iter()
        .any(|r| r["startLine"] == 3 && r["kind"] == "region");
    assert!(has_match, "expected match block to be foldable as region");
}

#[test]
fn folding_range_returns_empty_for_unknown_document() {
    let mut input = Vec::new();
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    })));
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/foldingRange",
        "params": {"textDocument": {"uri": "file:///never-opened.ori"}}
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
        .expect("response present");
    let ranges = response["result"].as_array().expect("range array");
    assert!(ranges.is_empty());
}
