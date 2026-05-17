//! `textDocument/definition` end-to-end coverage.
//!
//! Opens a document containing a call site for `f()` and asserts that the
//! definition request maps back to the line where `fn f` is declared. The
//! assertions explicitly verify the 0-based vs 1-based span translation so a
//! regression in that axis would fail loudly.

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

// 0-based view:
//   line 0: "module sample.calls"
//   line 1: ""
//   line 2: "fn f() -> Unit:"             <- declaration of `f`
//   line 3: "  return Unit"
//   line 4: ""
//   line 5: "fn caller() -> Unit:"
//   line 6: "  f()"                        <- call site, `f` at column 2
//   line 7: "  return Unit"
const SOURCE: &str =
    "module sample.calls\n\nfn f() -> Unit:\n  return Unit\n\nfn caller() -> Unit:\n  f()\n  return Unit\n";

#[test]
fn definition_returns_location_of_first_matching_symbol() {
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
                "uri": "file:///calls.ori",
                "languageId": "orison",
                "version": 1,
                "text": SOURCE
            }
        }
    })));
    // Cursor lands on the `f` at line 6, column 2 (0-based).
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/definition",
        "params": {
            "textDocument": {"uri": "file:///calls.ori"},
            "position": {"line": 6, "character": 2}
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

    let resp = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("definition response present");
    let loc = &resp["result"];
    assert_eq!(loc["uri"], "file:///calls.ori");

    // The compiler reports the symbol span as 1-based; the LSP layer is
    // 0-based. `fn f` lives on source line 3 (1-based) which is line 2
    // (0-based). The pinned assertion guards the off-by-one conversion.
    let start_line = loc["range"]["start"]["line"].as_u64().expect("start.line");
    let start_char = loc["range"]["start"]["character"]
        .as_u64()
        .expect("start.character");
    assert_eq!(
        start_line, 2,
        "expected 0-based line 2 for `fn f` declaration (got {start_line})"
    );
    assert_eq!(
        start_char, 0,
        "expected 0-based column 0 for `fn f` declaration (got {start_char})"
    );

    let end_line = loc["range"]["end"]["line"].as_u64().expect("end.line");
    let end_char = loc["range"]["end"]["character"]
        .as_u64()
        .expect("end.character");
    assert_eq!(end_line, 2, "definition span sits on one line");
    // The compiler returns 1-based end column inclusive of the last char of
    // `f`; the LSP `Range.end` is 0-based and exclusive — for "f" that means
    // column 4 (covering the `fn f` span).
    assert!(end_char >= start_char, "end column must not precede start");
}

#[test]
fn definition_returns_null_when_identifier_unknown() {
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
                "uri": "file:///calls.ori",
                "languageId": "orison",
                "version": 1,
                "text": SOURCE
            }
        }
    })));
    // Cursor on whitespace at end-of-document: no identifier under it.
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/definition",
        "params": {
            "textDocument": {"uri": "file:///calls.ori"},
            "position": {"line": 1, "character": 0}
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
    let resp = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("definition response present");
    assert_eq!(
        resp["result"],
        Value::Null,
        "no identifier => null Location"
    );
}
