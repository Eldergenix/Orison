//! Agent-facing helpers built on top of the bootstrap compiler.
//!
//! This crate produces the stable JSON envelopes consumed by `ori agent`
//! sub-commands and by IDE/agent integrations. Output is generated through
//! typed `serde` structs so the schema version (`ori.agent_map.v1`,
//! `ori.symbol_card.v1`, ...) stays in lock step with the schemas under
//! `schemas/`.

pub mod extras;
pub mod model_loop;

pub use extras::{
    agent_diagnose_json, agent_symbol_list_json, doctor_report_json, AgentDiagnose,
    AgentSymbolEntry, AgentSymbolList, DoctorReport, RepairCandidate,
};
pub use model_loop::{
    build_telemetry, iteration_with_saturating_budget, parse_telemetry_json, telemetry_json,
    LoopIteration, LoopTelemetry, LoopTotals,
};

use ori_compiler::json::to_json;
use ori_compiler::symbols::find_symbol;
use ori_compiler::CompileResult;
use serde::Serialize;

/// Configuration knobs for [`agent_map_json`].
#[derive(Debug, Clone, Copy)]
pub struct AgentMapOptions {
    /// Approximate maximum number of bytes the JSON envelope should occupy
    /// before further symbols are skipped and `truncated` is set to true.
    pub budget: usize,
}

impl Default for AgentMapOptions {
    fn default() -> Self {
        Self { budget: 4000 }
    }
}

#[derive(Debug, Serialize)]
struct AgentMap<'a> {
    schema: &'static str,
    module: &'a str,
    budget: usize,
    used_estimate: usize,
    truncated: bool,
    imports: &'a [String],
    symbols: Vec<AgentMapSymbol<'a>>,
    diagnostic_count: usize,
    error_count: usize,
    warning_count: usize,
}

#[derive(Debug, Serialize)]
struct AgentMapSymbol<'a> {
    id: &'a str,
    kind: &'static str,
    name: &'a str,
    signature: &'a str,
    effects: &'a [String],
}

#[derive(Debug, Serialize)]
struct SymbolCard<'a> {
    schema: &'static str,
    found: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    signature: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    effects: Option<&'a [String]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_span: Option<SourceSpan<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    query: Option<&'a str>,
    module: &'a str,
}

#[derive(Debug, Serialize)]
struct SourceSpan<'a> {
    file: &'a str,
    start_line: usize,
    end_line: usize,
}

/// Produce the `ori.agent_map.v1` JSON envelope for `result`. Symbols are
/// emitted in declaration order and the list is truncated once
/// [`AgentMapOptions::budget`] is exhausted.
pub fn agent_map_json(result: &CompileResult, options: AgentMapOptions) -> String {
    let budget = options.budget.max(1);
    let mut used = 0usize;
    let mut symbols = Vec::new();
    let mut truncated = false;

    for symbol in &result.module.symbols {
        let estimated =
            symbol.signature.len() + symbol.id.len() + symbol.effects.join(",").len() + 48;
        if used + estimated > budget && !symbols.is_empty() {
            truncated = true;
            break;
        }
        used += estimated;
        symbols.push(AgentMapSymbol {
            id: &symbol.id,
            kind: symbol.kind.as_str(),
            name: &symbol.name,
            signature: &symbol.signature,
            effects: &symbol.effects,
        });
    }

    let error_count = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.is_error())
        .count();
    let warning_count = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.level.as_str() == "warning")
        .count();
    let map = AgentMap {
        schema: "ori.agent_map.v1",
        module: &result.module.name,
        budget,
        used_estimate: used,
        truncated,
        imports: &result.module.imports,
        symbols,
        diagnostic_count: result.diagnostics.len(),
        error_count,
        warning_count,
    };
    to_json(&map)
}

/// Produce the `ori.symbol_card.v1` JSON envelope for the symbol matching
/// `id_or_name`. When no matching symbol exists the envelope is still emitted
/// with `found: false` and the original `query` preserved so callers can
/// distinguish a typo from a missing symbol.
pub fn explain_symbol_json(result: &CompileResult, id_or_name: &str) -> String {
    if let Some(symbol) = find_symbol(&result.module, id_or_name) {
        let card = SymbolCard {
            schema: "ori.symbol_card.v1",
            found: true,
            id: Some(&symbol.id),
            name: Some(&symbol.name),
            kind: Some(symbol.kind.as_str()),
            signature: Some(&symbol.signature),
            effects: Some(&symbol.effects),
            source_span: Some(SourceSpan {
                file: &symbol.span.file,
                start_line: symbol.span.start.line,
                end_line: symbol.span.end.line,
            }),
            summary: Some(format!(
                "{} `{}` in module `{}`.",
                symbol.kind.as_str(),
                symbol.name,
                result.module.name
            )),
            query: None,
            module: &result.module.name,
        };
        to_json(&card)
    } else {
        let card = SymbolCard {
            schema: "ori.symbol_card.v1",
            found: false,
            id: None,
            name: None,
            kind: None,
            signature: None,
            effects: None,
            source_span: None,
            summary: None,
            query: Some(id_or_name),
            module: &result.module.name,
        };
        to_json(&card)
    }
}
