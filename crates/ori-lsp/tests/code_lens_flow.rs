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
    "module callers.demo\n",
    "\n",
    "fn helper() -> Unit:\n",
    "  return Unit\n",
    "\n",
    "fn main() -> Unit:\n",
    "  helper()\n",
    "  helper()\n",
    "  return Unit\n",
);

#[test]
fn code_lens_capability_advertised() {
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
    let caps = &messages[0]["result"]["capabilities"];
    assert_eq!(caps["codeLensProvider"]["resolveProvider"], false);
}

#[test]
fn code_lens_counts_callers_for_each_function() {
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
                "uri": "file:///callers.ori",
                "languageId": "orison",
                "version": 1,
                "text": SOURCE,
            }
        }
    })));
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/codeLens",
        "params": {"textDocument": {"uri": "file:///callers.ori"}}
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
        .expect("codeLens response");
    let lenses = response["result"].as_array().expect("lens array");
    assert_eq!(lenses.len(), 2);
    let helper = lenses
        .iter()
        .find(|l| l["data"]["symbol"] == "sym:callers.demo.helper")
        .expect("helper lens present");
    assert_eq!(helper["command"]["title"], "2 callers");
    let main = lenses
        .iter()
        .find(|l| l["data"]["symbol"] == "sym:callers.demo.main")
        .expect("main lens present");
    assert_eq!(main["command"]["title"], "0 callers");
}

#[test]
fn code_lens_resolve_returns_lens_unchanged() {
    let lens = json!({
        "range": {"start": {"line": 2, "character": 0}, "end": {"line": 2, "character": 11}},
        "command": {
            "title": "1 callers",
            "command": "ori.lsp.codeLens.showCallers",
            "arguments": []
        },
        "data": {"schema": "ori.lsp.code_lens.v1", "symbol": "sym:demo.x", "callers": 1}
    });

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
        "method": "codeLens/resolve",
        "params": lens.clone(),
    })));
    input.extend_from_slice(&framed(&json!({"jsonrpc": "2.0", "method": "exit"})));

    let mut output: Vec<u8> = Vec::new();
    Server::new()
        .run(Cursor::new(input), &mut output)
        .expect("server run");
    let messages = read_all_messages(output);
    let resolved = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("resolve response");
    assert_eq!(resolved["result"]["command"]["title"], "1 callers");
    assert_eq!(resolved["result"]["data"]["callers"], 1);
}
