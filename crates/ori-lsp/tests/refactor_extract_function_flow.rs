//! `workspace/executeCommand` coverage for `ori.refactor.extractFunction`.

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

// 0-based lines:
//   0: module extract.demo
//   1: <blank>
//   2: fn host() -> Unit:
//   3:   let x = 1
//   4:   let y = 2
//   5:   return Unit
const SOURCE: &str = concat!(
    "module extract.demo\n",
    "\n",
    "fn host() -> Unit:\n",
    "  let x = 1\n",
    "  let y = 2\n",
    "  return Unit\n",
);

fn run_extract(name: &str, start: (u32, u32), end: (u32, u32)) -> Vec<Value> {
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
                "uri": "file:///extract.ori",
                "languageId": "orison",
                "version": 1,
                "text": SOURCE,
            }
        }
    })));
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "workspace/executeCommand",
        "params": {
            "command": "ori.refactor.extractFunction",
            "arguments": [{
                "textDocument": {"uri": "file:///extract.ori"},
                "range": {
                    "start": {"line": start.0, "character": start.1},
                    "end": {"line": end.0, "character": end.1}
                },
                "name": name
            }]
        }
    })));
    input.extend_from_slice(&framed(&json!({"jsonrpc": "2.0", "method": "exit"})));
    let mut output: Vec<u8> = Vec::new();
    Server::new()
        .run(Cursor::new(input), &mut output)
        .expect("server run");
    read_all_messages(output)
}

#[test]
fn extract_function_happy_path_produces_two_edits() {
    let messages = run_extract("extracted", (3, 0), (4, 11));
    let resp = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("response present");
    let edits = resp["result"]["changes"]["file:///extract.ori"]
        .as_array()
        .expect("changes array");
    // One replacement at the selection + one append of the new function.
    assert_eq!(edits.len(), 2, "expected two edits, got {:#?}", edits);
    // The append edit must mention `fn extracted(` somewhere in new_text.
    let appends: Vec<&Value> = edits
        .iter()
        .filter(|e| {
            e["newText"]
                .as_str()
                .map(|s| s.contains("fn extracted("))
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(appends.len(), 1, "expected exactly one new function edit");
}

#[test]
fn extract_function_across_blocks_is_rejected() {
    // Range crosses outside the host function body: end line is past EOF.
    let messages = run_extract("doomed", (3, 0), (99, 0));
    let resp = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("response present");
    assert!(
        resp.get("error").is_some(),
        "expected INVALID_PARAMS error, got {resp:#?}"
    );
    assert_eq!(resp["error"]["code"], -32602);
}

#[test]
fn extract_function_name_collision_is_rejected() {
    // `host` already exists in the module.
    let messages = run_extract("host", (3, 0), (4, 11));
    let resp = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("response present");
    assert!(
        resp.get("error").is_some(),
        "expected INVALID_PARAMS error, got {resp:#?}"
    );
    let message = resp["error"]["message"].as_str().unwrap_or("");
    assert!(
        message.contains("host"),
        "error must mention the colliding name, got {message:?}"
    );
}

#[test]
fn extract_function_edits_are_deterministic_across_invocations() {
    let first = run_extract("worker", (3, 0), (4, 11));
    let second = run_extract("worker", (3, 0), (4, 11));
    let edits_a = &first.iter().find(|m| m["id"] == 2).expect("present a")["result"];
    let edits_b = &second.iter().find(|m| m["id"] == 2).expect("present b")["result"];
    assert_eq!(
        edits_a, edits_b,
        "identical extractFunction requests must produce byte-identical WorkspaceEdit"
    );
}
