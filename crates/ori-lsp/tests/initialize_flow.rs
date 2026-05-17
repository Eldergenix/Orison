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
fn responds_to_initialize_with_capabilities() {
    let mut input = Vec::new();
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "processId": null,
            "rootUri": null,
            "capabilities": {}
        }
    })));
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "shutdown"
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
    assert_eq!(messages.len(), 2, "initialize + shutdown responses");

    let init = &messages[0];
    assert_eq!(init["id"], 1);
    let caps = &init["result"]["capabilities"];
    assert_eq!(caps["textDocumentSync"], 1);
    assert_eq!(caps["hoverProvider"], true);
    assert_eq!(caps["codeActionProvider"], true);
    assert_eq!(init["result"]["serverInfo"]["name"], "ori-lsp");

    let shutdown = &messages[1];
    assert_eq!(shutdown["id"], 2);
    assert_eq!(shutdown["result"], Value::Null);
}

#[test]
fn publishes_diagnostics_on_did_open() {
    let mut input = Vec::new();
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    })));
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    })));
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": "file:///bad.ori",
                "languageId": "orison",
                "version": 1,
                "text": "module bad.null_example\n\nfn main() -> Unit:\n  let user = null\n  return Unit\n"
            }
        }
    })));
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "shutdown"
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

    let publish = messages
        .iter()
        .find(|m| m["method"] == "textDocument/publishDiagnostics")
        .expect("publishDiagnostics present");
    assert_eq!(publish["params"]["uri"], "file:///bad.ori");
    let diagnostics = publish["params"]["diagnostics"]
        .as_array()
        .expect("diagnostic array");
    assert!(diagnostics.iter().any(|d| d["code"] == "E0100"));
}

#[test]
fn supports_string_ids() {
    let mut input = Vec::new();
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": "init-token",
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
    assert_eq!(messages[0]["id"], "init-token");
}

#[test]
fn rejects_invalid_json_with_parse_error() {
    // Build a frame with garbage payload manually.
    let payload = b"\x7bnot json";
    let header = format!("Content-Length: {}\r\n\r\n", payload.len());
    let mut input = Vec::new();
    input.extend_from_slice(header.as_bytes());
    input.extend_from_slice(payload);
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));

    let mut output: Vec<u8> = Vec::new();
    Server::new()
        .run(Cursor::new(input), &mut output)
        .expect("server run");
    let messages = read_all_messages(output);
    let err = &messages[0];
    assert_eq!(err["error"]["code"], -32700);
}

#[test]
fn code_action_returns_quickfix_for_fixable_diagnostic() {
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
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": {"uri": "file:///bad.ori"},
            "range": {
                "start": {"line": 3, "character": 13},
                "end": {"line": 3, "character": 17}
            },
            "context": {
                "diagnostics": [{
                    "range": {
                        "start": {"line": 3, "character": 13},
                        "end": {"line": 3, "character": 17}
                    },
                    "severity": 1,
                    "code": "E0100",
                    "source": "ori",
                    "message": "`null` is not part of Orison",
                    "data": {
                        "schema": "ori.lsp.fixes.v1",
                        "fixes": [{
                            "kind": "replace_null",
                            "description": "Replace `null` with `None`",
                            "confidence": 0.82,
                            "patch": null
                        }]
                    }
                }]
            }
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

    let action_response = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("code action response present");
    let actions = action_response["result"].as_array().expect("actions array");
    assert_eq!(actions.len(), 1);
    let action = &actions[0];
    assert_eq!(action["kind"], "quickfix");
    assert_eq!(action["data"]["patchRef"], "patch:diag/E0100/fix/0");
    assert_eq!(action["data"]["schema"], "ori.lsp.code_action.v1");
}
