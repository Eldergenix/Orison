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

const SAMPLE: &str = "module sample.tokens\n\nfn greet() -> Unit:\n  let n = 42\n  return Unit\n";

fn send_request(input: &mut Vec<u8>, id: i64, method: &str, params: Value) {
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    })));
}

fn send_notif(input: &mut Vec<u8>, method: &str, params: Value) {
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    })));
}

fn open_and_run(uri: &str, text: &str, requests: Vec<(i64, &str, Value)>) -> Vec<Value> {
    let mut input = Vec::new();
    send_request(&mut input, 1, "initialize", json!({}));
    send_notif(
        &mut input,
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": uri,
                "languageId": "orison",
                "version": 1,
                "text": text,
            }
        }),
    );
    for (id, method, params) in requests {
        send_request(&mut input, id, method, params);
    }
    send_notif(&mut input, "exit", json!({}));
    let mut output: Vec<u8> = Vec::new();
    Server::new()
        .run(Cursor::new(input), &mut output)
        .expect("server run");
    read_all_messages(output)
}

#[test]
fn semantic_tokens_advertised_in_capabilities() {
    let messages = open_and_run(
        "file:///tokens.ori",
        SAMPLE,
        vec![(99, "shutdown", json!(null))],
    );
    let init = &messages[0];
    let caps = &init["result"]["capabilities"];
    assert_eq!(caps["semanticTokensProvider"]["full"], true);
    let legend = caps["semanticTokensProvider"]["legend"]["tokenTypes"]
        .as_array()
        .expect("legend.tokenTypes is array");
    let names: Vec<&str> = legend.iter().filter_map(Value::as_str).collect();
    assert!(names.contains(&"keyword"));
    assert!(names.contains(&"function"));
    assert!(names.contains(&"comment"));
}

#[test]
fn semantic_tokens_full_emits_delta_encoded_runs() {
    let messages = open_and_run(
        "file:///tokens.ori",
        SAMPLE,
        vec![(
            2,
            "textDocument/semanticTokens/full",
            json!({"textDocument": {"uri": "file:///tokens.ori"}}),
        )],
    );
    let response = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("semantic tokens response present");
    let data = response["result"]["data"]
        .as_array()
        .expect("data array present");
    assert!(!data.is_empty());
    // Spec: every run is exactly 5 numbers.
    assert_eq!(data.len() % 5, 0);
    // The first token's deltaLine must be the absolute line (no previous
    // token), so the first 5-tuple's first slot is the line of `module`.
    assert_eq!(data[0], json!(0));
}

#[test]
fn semantic_tokens_returns_empty_for_unknown_document() {
    let messages = open_and_run(
        "file:///tokens.ori",
        SAMPLE,
        vec![(
            3,
            "textDocument/semanticTokens/full",
            json!({"textDocument": {"uri": "file:///does-not-exist.ori"}}),
        )],
    );
    let response = messages
        .iter()
        .find(|m| m["id"] == 3)
        .expect("response present");
    let data = response["result"]["data"]
        .as_array()
        .expect("data array present");
    assert!(data.is_empty());
}
