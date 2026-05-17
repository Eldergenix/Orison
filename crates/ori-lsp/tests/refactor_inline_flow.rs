//! `workspace/executeCommand` coverage for `ori.refactor.inline`.

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
//   0: module inline.demo
//   1: <blank>
//   2: fn helper() -> Unit:
//   3:   return Unit
//   4: <blank>
//   5: fn caller() -> Unit:
//   6:   helper()
//   7:   return Unit
const SOURCE: &str = concat!(
    "module inline.demo\n",
    "\n",
    "fn helper() -> Unit:\n",
    "  return Unit\n",
    "\n",
    "fn caller() -> Unit:\n",
    "  helper()\n",
    "  return Unit\n",
);

fn run_inline(uri: &str, text: &str, line: u32, character: u32) -> Vec<Value> {
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
        "id": 2,
        "method": "workspace/executeCommand",
        "params": {
            "command": "ori.refactor.inline",
            "arguments": [{
                "textDocument": {"uri": uri},
                "position": {"line": line, "character": character}
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
fn inline_happy_path_replaces_call_with_body() {
    // Cursor sits on the `helper` identifier inside `helper()` on line 6.
    let messages = run_inline("file:///inline.ori", SOURCE, 6, 2);
    let resp = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("response present");
    let edits = resp["result"]["changes"]["file:///inline.ori"]
        .as_array()
        .expect("changes array");
    assert_eq!(edits.len(), 1, "inline produces exactly one edit");
    let new_text = edits[0]["newText"].as_str().unwrap_or("");
    assert!(
        new_text.contains("return Unit"),
        "inlined text must include helper body, got {new_text:?}"
    );
}

#[test]
fn inline_recursive_function_is_rejected() {
    let recursive = concat!(
        "module rec.demo\n",
        "\n",
        "fn loop_forever() -> Unit:\n",
        "  loop_forever()\n",
        "  return Unit\n",
        "\n",
        "fn caller() -> Unit:\n",
        "  loop_forever()\n",
        "  return Unit\n",
    );
    // Click on the `loop_forever()` call inside caller() — line 7, col 2.
    let messages = run_inline("file:///rec.ori", recursive, 7, 2);
    let resp = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("response present");
    assert!(
        resp.get("error").is_some(),
        "expected INVALID_PARAMS rejection, got {resp:#?}"
    );
    let message = resp["error"]["message"].as_str().unwrap_or("");
    assert!(
        message.contains("recursive"),
        "error must mention recursion, got {message:?}"
    );
}

#[test]
fn inline_multi_statement_body_preserves_all_statements() {
    let multi = concat!(
        "module multi.demo\n",
        "\n",
        "fn helper() -> Unit:\n",
        "  let x = 1\n",
        "  let y = 2\n",
        "  return Unit\n",
        "\n",
        "fn caller() -> Unit:\n",
        "  helper()\n",
        "  return Unit\n",
    );
    // Click on the `helper()` call inside caller — line 8, col 2.
    let messages = run_inline("file:///multi.ori", multi, 8, 2);
    let resp = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("response present");
    let edits = resp["result"]["changes"]["file:///multi.ori"]
        .as_array()
        .expect("changes array");
    assert_eq!(edits.len(), 1);
    let new_text = edits[0]["newText"].as_str().unwrap_or("");
    assert!(
        new_text.contains("let x = 1"),
        "expected let x in {new_text:?}"
    );
    assert!(
        new_text.contains("let y = 2"),
        "expected let y in {new_text:?}"
    );
    assert!(
        new_text.contains("return Unit"),
        "expected return in {new_text:?}"
    );
}

#[test]
fn inline_emits_exactly_one_edit_for_a_single_call_site() {
    let messages = run_inline("file:///inline.ori", SOURCE, 6, 2);
    let resp = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("response present");
    let changes = &resp["result"]["changes"];
    let uris: Vec<&str> = changes
        .as_object()
        .expect("object")
        .keys()
        .map(|s| s.as_str())
        .collect();
    assert_eq!(
        uris,
        vec!["file:///inline.ori"],
        "edits must be scoped to the host document"
    );
    let edits = changes[uris[0]].as_array().expect("array");
    assert_eq!(edits.len(), 1, "inline emits exactly one TextEdit per call");
}
