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

// Three identifier occurrences of `hello` (definition, recursive call,
// trailing reference) plus one inside a string literal which must NOT be
// touched.
const RENAME_SOURCE: &str = "module sample.greeter\n\nfn hello() -> Unit:\n  let msg = \"hello world\"\n  hello()\n  hello\n";

#[test]
fn rename_emits_workspace_edit_for_identifier_only() {
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
                "text": RENAME_SOURCE
            }
        }
    })));
    // Position the cursor inside `hello` on the function definition (line 2,
    // 0-based, starting at column 3 = the 'h' of `hello`).
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/rename",
        "params": {
            "textDocument": {"uri": "file:///greeter.ori"},
            "position": {"line": 2, "character": 4},
            "newName": "greet"
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

    let rename = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("rename response present");
    let changes = rename["result"]["changes"]
        .as_object()
        .expect("workspace edit changes map");
    let edits = changes
        .get("file:///greeter.ori")
        .and_then(Value::as_array)
        .expect("edits for greeter.ori");

    // Expect exactly three edits — the two function-name occurrences and the
    // call site, but NOT the literal substring inside `"hello world"`.
    assert_eq!(
        edits.len(),
        3,
        "expected 3 identifier edits (zero inside string literals), got {edits:?}"
    );

    for edit in edits {
        assert_eq!(edit["newText"], "greet");
        // None of the edits may overlap the string literal range. The string
        // sits on line 3 (0-based), starting after column 12.
        let start_line = edit["range"]["start"]["line"].as_u64().unwrap_or(0);
        let start_char = edit["range"]["start"]["character"].as_u64().unwrap_or(0);
        if start_line == 3 {
            assert!(
                start_char < 12,
                "edit at line 3 column {start_char} would overlap string literal"
            );
        }
    }

    // Apply the edits manually and verify the literal is preserved.
    let mut text = RENAME_SOURCE.to_string();
    // Apply in reverse order by byte position to keep offsets stable.
    let mut sorted: Vec<&Value> = edits.iter().collect();
    sorted.sort_by(|a, b| {
        let key = |edit: &&Value| {
            (
                edit["range"]["start"]["line"].as_u64().unwrap_or(0),
                edit["range"]["start"]["character"].as_u64().unwrap_or(0),
            )
        };
        key(b).cmp(&key(a))
    });
    for edit in sorted {
        text = apply_edit(&text, edit);
    }
    assert!(
        text.contains("\"hello world\""),
        "string literal must remain untouched, got: {text}"
    );
    assert!(text.contains("fn greet()"));
    assert!(text.contains("  greet()\n"));
    assert!(text.contains("  greet\n"));
}

#[test]
fn rename_returns_empty_workspace_edit_when_new_name_unchanged() {
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
                "text": RENAME_SOURCE
            }
        }
    })));
    input.extend_from_slice(&framed(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/rename",
        "params": {
            "textDocument": {"uri": "file:///greeter.ori"},
            "position": {"line": 2, "character": 4},
            "newName": "hello"
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

    let rename = messages
        .iter()
        .find(|m| m["id"] == 2)
        .expect("rename response present");
    let changes = rename["result"]["changes"]
        .as_object()
        .expect("workspace edit changes map");
    assert!(changes.is_empty(), "no edits when newName == oldName");
}

fn apply_edit(text: &str, edit: &Value) -> String {
    let start_line = edit["range"]["start"]["line"].as_u64().unwrap_or(0) as usize;
    let start_char = edit["range"]["start"]["character"].as_u64().unwrap_or(0) as usize;
    let end_line = edit["range"]["end"]["line"].as_u64().unwrap_or(0) as usize;
    let end_char = edit["range"]["end"]["character"].as_u64().unwrap_or(0) as usize;
    let new_text = edit["newText"].as_str().unwrap_or("");
    let start_byte = position_to_byte(text, start_line, start_char);
    let end_byte = position_to_byte(text, end_line, end_char);
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..start_byte]);
    out.push_str(new_text);
    out.push_str(&text[end_byte..]);
    out
}

fn position_to_byte(text: &str, line: usize, character: usize) -> usize {
    let mut current_line = 0usize;
    let mut current_char = 0usize;
    for (idx, ch) in text.char_indices() {
        if current_line == line && current_char == character {
            return idx;
        }
        if ch == '\n' {
            current_line += 1;
            current_char = 0;
        } else {
            current_char += 1;
        }
    }
    text.len()
}
