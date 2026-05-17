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

const COMPLETION_SOURCE: &str =
    "module sample.greeter\n\nfn hello() -> Unit:\n  return Unit\n\nfn farewell() -> Unit:\n  return Unit\n";

#[test]
fn initialize_advertises_completion_and_rename_capabilities() {
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
    assert_eq!(caps["renameProvider"], true);
    let triggers = caps["completionProvider"]["triggerCharacters"]
        .as_array()
        .expect("triggerCharacters array");
    let strings: Vec<&str> = triggers.iter().filter_map(Value::as_str).collect();
    assert!(strings.contains(&"."));
    assert!(strings.contains(&":"));
}

#[test]
fn completion_returns_symbols_and_keywords() {
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
                "uri": "file:///greeter.ori",
                "languageId": "orison",
                "version": 1,
                "text": COMPLETION_SOURCE
            }
        }
    })));
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/completion",
        "params": {
            "textDocument": {"uri": "file:///greeter.ori"},
            "position": {"line": 6, "character": 0}
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

    let completion = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("completion response present");
    let items = completion["result"]["items"]
        .as_array()
        .expect("items array");

    let labels: Vec<&str> = items.iter().filter_map(|i| i["label"].as_str()).collect();
    assert!(
        labels.contains(&"hello"),
        "expected symbol `hello` in {labels:?}"
    );
    assert!(
        labels.contains(&"farewell"),
        "expected symbol `farewell` in {labels:?}"
    );
    assert!(
        labels.contains(&"fn"),
        "expected keyword `fn` in {labels:?}"
    );

    // Sorted alphabetically.
    let mut sorted = labels.clone();
    sorted.sort();
    assert_eq!(labels, sorted, "completion items must be sorted");

    // Each symbol entry advertises a kind matching the LSP enum.
    let hello = items
        .iter()
        .find(|i| i["label"] == "hello")
        .expect("hello item");
    assert_eq!(hello["kind"], 3); // Function
    assert!(hello["detail"].as_str().is_some());

    // Smoke metric for the report.
    assert!(
        items.len() >= 10,
        "expected a useful completion list, got {}",
        items.len()
    );
}
