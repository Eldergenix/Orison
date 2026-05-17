//! `textDocument/prepareTypeHierarchy` + supertypes/subtypes coverage.

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

// `Color` is a simple variant type with 3 constructors. Span info points
// at the type-line `type Color =`; variants begin on the next lines.
//
// 0-based lines:
//   0: module typehier.demo
//   1: <blank>
//   2: type Color =
//   3:   | Red
//   4:   | Green
//   5:   | Blue
//   6: <blank>
//   7: fn use_color() -> Unit:
//   8:   return Unit
const SOURCE: &str = concat!(
    "module typehier.demo\n",
    "\n",
    "type Color =\n",
    "  | Red\n",
    "  | Green\n",
    "  | Blue\n",
    "\n",
    "fn use_color() -> Unit:\n",
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
                "uri": "file:///types.ori",
                "languageId": "orison",
                "version": 1,
                "text": SOURCE,
            }
        }
    })));
    input
}

#[test]
fn prepare_type_hierarchy_returns_type_under_cursor() {
    let mut input = open_input();
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": {"uri": "file:///types.ori"},
            "position": {"line": 2, "character": 5}
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
        .expect("prepareTypeHierarchy response present");
    let items = resp["result"].as_array().expect("array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Color");
    assert_eq!(items[0]["data"]["kind"], "type");
}

#[test]
fn type_hierarchy_subtypes_returns_constructors() {
    let mut input = open_input();
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "typeHierarchy/subtypes",
        "params": {
            "item": {
                "name": "Color",
                "kind": 5,
                "uri": "file:///types.ori",
                "range": {"start": {"line": 2, "character": 0}, "end": {"line": 2, "character": 10}},
                "selectionRange": {"start": {"line": 2, "character": 0}, "end": {"line": 2, "character": 10}},
                "data": {"kind": "type", "owner": "Color"}
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
        .expect("subtypes response present");
    let variants = resp["result"].as_array().expect("array");
    assert_eq!(variants.len(), 3, "expected three variant constructors");
    let names: Vec<&str> = variants
        .iter()
        .map(|v| v["name"].as_str().unwrap_or(""))
        .collect();
    assert!(names.contains(&"Red"));
    assert!(names.contains(&"Green"));
    assert!(names.contains(&"Blue"));
    // Variants must declare the owner so supertypes can roundtrip.
    for v in variants {
        assert_eq!(v["data"]["kind"], "variant");
        assert_eq!(v["data"]["owner"], "Color");
    }
}

#[test]
fn type_hierarchy_supertypes_for_variant_returns_owner() {
    let mut input = open_input();
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "typeHierarchy/supertypes",
        "params": {
            "item": {
                "name": "Red",
                "kind": 5,
                "uri": "file:///types.ori",
                "range": {"start": {"line": 3, "character": 4}, "end": {"line": 3, "character": 7}},
                "selectionRange": {"start": {"line": 3, "character": 4}, "end": {"line": 3, "character": 7}},
                "data": {"kind": "variant", "owner": "Color"}
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
        .expect("supertypes response present");
    let parents = resp["result"].as_array().expect("array");
    assert_eq!(parents.len(), 1);
    assert_eq!(parents[0]["name"], "Color");
}
