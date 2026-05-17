//! `workspace/executeCommand` dispatcher coverage.
//!
//! The dispatch arm must route to the right refactor and surface the
//! advertised commands inside the initialize result.

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

#[test]
fn execute_command_advertises_and_rejects_unknown() {
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
        "method": "workspace/executeCommand",
        "params": {
            "command": "ori.does.not.exist",
            "arguments": []
        }
    })));
    input.extend_from_slice(&framed(&json!({"jsonrpc": "2.0", "method": "exit"})));
    let mut output: Vec<u8> = Vec::new();
    Server::new()
        .run(Cursor::new(input), &mut output)
        .expect("server run");
    let messages = read_all_messages(output);

    // Initialize result advertises both refactor commands and the three
    // new boolean providers.
    let init = messages
        .iter()
        .find(|m| m["id"] == 1)
        .expect("initialize response present");
    let caps = &init["result"]["capabilities"];
    assert_eq!(caps["callHierarchyProvider"], true);
    assert_eq!(caps["typeHierarchyProvider"], true);
    let commands = caps["executeCommandProvider"]["commands"]
        .as_array()
        .expect("executeCommandProvider.commands array");
    let labels: Vec<&str> = commands.iter().filter_map(|v| v.as_str()).collect();
    assert!(labels.contains(&"ori.refactor.extractFunction"));
    assert!(labels.contains(&"ori.refactor.inline"));

    // Unknown command dispatch must surface METHOD_NOT_FOUND, not a
    // success response that silently swallows the request.
    let exec = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("executeCommand response present");
    assert_eq!(
        exec["error"]["code"], -32601,
        "unknown command must produce METHOD_NOT_FOUND"
    );
}
