use ori_compiler::patch::check_patch_json;
use ori_compiler::{Compiler, SourceFile};

#[test]
fn parses_module_imports_and_symbols() {
    let source = SourceFile::new(
        "hello.ori",
        "module hello\nimport std.json\nfn main() -> Unit:\n  return Unit\n",
    );
    let result = Compiler::check_source(source);
    assert_eq!(result.module.name, "hello");
    assert_eq!(result.module.imports, vec!["std.json"]);
    assert!(result
        .module
        .symbols
        .iter()
        .any(|symbol| symbol.id == "sym:hello.main" && symbol.signature == "fn main() -> Unit"));
}

#[test]
fn emits_null_diagnostic_outside_strings_and_comments() {
    let source = SourceFile::new(
        "bad.ori",
        "module bad\n// null in comments is not a value\nfn main() -> Unit:\n  let s = \"null\"\n  let x = null\n",
    );
    let result = Compiler::check_source(source);
    let null_errors = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.id == "E0100")
        .count();
    assert_eq!(null_errors, 1);
}

#[test]
fn diagnostic_json_lines_are_valid_json() {
    let source = SourceFile::new(
        "bad.ori",
        "module bad\nfn main() -> Unit:\n  let x = null\n",
    );
    let result = Compiler::check_source(source);
    let lines = Compiler::diagnostics_json_lines(&result);
    for line in lines.lines() {
        let value: serde_json::Value = serde_json::from_str(line).expect("diagnostic JSON");
        assert_eq!(value["schema"], "ori.diagnostic.v1");
    }
}

#[test]
fn capsule_json_is_valid_and_omits_module_from_exports() {
    let source = SourceFile::new(
        "hello.ori",
        "module hello\nfn main() -> Unit:\n  return Unit\n",
    );
    let result = Compiler::check_source(source);
    let value: serde_json::Value =
        serde_json::from_str(&Compiler::capsule_json(&result)).expect("capsule JSON");
    assert_eq!(value["schema"], "ori.capsule.v1");
    assert_eq!(value["exports"][0]["id"], "sym:hello.main");
}

#[test]
fn patch_checker_rejects_unknown_operations() {
    let patch = r#"{
      "schema": "ori.patch.v1",
      "intent": "exercise validation",
      "operations": [{ "op": "rewrite_everything" }],
      "tests": { "run": ["ori test --changed"], "expected": "pass" }
    }"#;
    let result = check_patch_json("patch.json", patch);
    assert!(!result.valid);
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.id == "P1002"));
}

#[test]
fn patch_checker_accepts_structural_patch() {
    let patch = r#"{
      "schema": "ori.patch.v1",
      "intent": "Add missing database error match arm",
      "operations": [{
        "op": "insert_match_arm",
        "target": "node:match:42",
        "pattern": "Err(Db(err))",
        "body": "render_db_error(err)"
      }],
      "tests": { "run": ["ori test --changed"], "expected": "pass" }
    }"#;
    let result = check_patch_json("patch.json", patch);
    assert!(result.valid, "{:?}", result.diagnostics);
}

#[test]
fn formatter_trims_trailing_whitespace() {
    let formatted = Compiler::format_source("module hello   \n");
    assert_eq!(formatted, "module hello\n");
}
