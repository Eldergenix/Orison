//! `textDocument/prepareCallHierarchy` + incoming/outgoing call coverage.
//!
//! The fixture below contains three functions — `helper`, `worker`, and
//! `main` — so we can validate both the prepare hand-shake and the
//! resolver follow-up requests against deterministic data.

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
    "module callhier.demo\n",
    "\n",
    "fn helper() -> Unit:\n",
    "  return Unit\n",
    "\n",
    "fn worker() -> Unit:\n",
    "  helper()\n",
    "  helper()\n",
    "  return Unit\n",
    "\n",
    "fn main() -> Unit:\n",
    "  worker()\n",
    "  helper()\n",
    "  return Unit\n",
);

fn open_input() -> Vec<u8> {
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
                "uri": "file:///callhier.ori",
                "languageId": "orison",
                "version": 1,
                "text": SOURCE,
            }
        }
    })));
    input
}

#[test]
fn prepare_call_hierarchy_returns_function_under_cursor() {
    let mut input = open_input();
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": {"uri": "file:///callhier.ori"},
            "position": {"line": 2, "character": 4}
        }
    })));
    input.extend_from_slice(&framed(&json!({"jsonrpc": "2.0", "method": "exit"})));
    let mut output: Vec<u8> = Vec::new();
    Server::new()
        .run(Cursor::new(input), &mut output)
        .expect("server run");
    let messages = read_all_messages(output);
    let resp = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("prepareCallHierarchy response present");
    let items = resp["result"].as_array().expect("array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "helper");
    // Range and selectionRange are equal in our implementation.
    assert_eq!(items[0]["range"], items[0]["selectionRange"]);
}

#[test]
fn incoming_calls_lists_each_caller_with_call_counts() {
    let mut input = open_input();
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "helper",
                "kind": 12,
                "uri": "file:///callhier.ori",
                "range": {"start": {"line": 2, "character": 0}, "end": {"line": 2, "character": 11}},
                "selectionRange": {"start": {"line": 2, "character": 0}, "end": {"line": 2, "character": 11}}
            }
        }
    })));
    input.extend_from_slice(&framed(&json!({"jsonrpc": "2.0", "method": "exit"})));
    let mut output: Vec<u8> = Vec::new();
    Server::new()
        .run(Cursor::new(input), &mut output)
        .expect("server run");
    let messages = read_all_messages(output);
    let resp = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("incomingCalls response present");
    let calls = resp["result"].as_array().expect("array");
    // Both `worker` (2 calls) and `main` (1 call) call `helper`.
    let worker_call = calls
        .iter()
        .find(|c| c["from"]["name"] == "worker")
        .expect("worker caller present");
    let worker_ranges = worker_call["fromRanges"]
        .as_array()
        .expect("fromRanges array");
    assert_eq!(
        worker_ranges.len(),
        2,
        "expected 2 calls from worker to helper, got {:#?}",
        worker_ranges
    );
    let main_call = calls
        .iter()
        .find(|c| c["from"]["name"] == "main")
        .expect("main caller present");
    let main_ranges = main_call["fromRanges"]
        .as_array()
        .expect("fromRanges array");
    assert_eq!(main_ranges.len(), 1, "expected 1 call from main to helper");
}

#[test]
fn outgoing_calls_reports_callees_with_counts() {
    let mut input = open_input();
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "main",
                "kind": 12,
                "uri": "file:///callhier.ori",
                "range": {"start": {"line": 11, "character": 0}, "end": {"line": 11, "character": 7}},
                "selectionRange": {"start": {"line": 11, "character": 0}, "end": {"line": 11, "character": 7}}
            }
        }
    })));
    input.extend_from_slice(&framed(&json!({"jsonrpc": "2.0", "method": "exit"})));
    let mut output: Vec<u8> = Vec::new();
    Server::new()
        .run(Cursor::new(input), &mut output)
        .expect("server run");
    let messages = read_all_messages(output);
    let resp = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("outgoingCalls response present");
    let calls = resp["result"].as_array().expect("array");
    let names: Vec<String> = calls
        .iter()
        .map(|c| c["to"]["name"].as_str().unwrap_or("").to_string())
        .collect();
    assert!(
        names.contains(&"worker".to_string()),
        "expected worker callee in {names:?}"
    );
    assert!(
        names.contains(&"helper".to_string()),
        "expected helper callee in {names:?}"
    );
    // Outgoing list is sorted by (uri, line, char) so the order is stable.
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "outgoing list must be deterministic");
}
