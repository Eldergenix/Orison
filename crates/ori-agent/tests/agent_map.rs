use ori_agent::{agent_map_json, explain_symbol_json, AgentMapOptions};
use ori_compiler::{Compiler, SourceFile};

#[test]
fn map_contains_schema_and_symbol() {
    let source = SourceFile::new(
        "hello.ori",
        "module hello\nfn main() -> Unit:\n  return Unit\n",
    );
    let result = Compiler::check_source(source);
    let map = agent_map_json(&result, AgentMapOptions { budget: 1000 });
    let value: serde_json::Value = serde_json::from_str(&map).expect("agent map JSON");
    assert_eq!(value["schema"], "ori.agent_map.v1");
    assert_eq!(value["symbols"][1]["id"], "sym:hello.main");
}

#[test]
fn explain_symbol_returns_symbol_card() {
    let source = SourceFile::new(
        "hello.ori",
        "module hello\nfn main() -> Unit:\n  return Unit\n",
    );
    let result = Compiler::check_source(source);
    let card = explain_symbol_json(&result, "main");
    let value: serde_json::Value = serde_json::from_str(&card).expect("symbol card JSON");
    assert_eq!(value["schema"], "ori.symbol_card.v1");
    assert_eq!(value["found"].as_bool(), Some(true));
    assert_eq!(value["id"], "sym:hello.main");
}
