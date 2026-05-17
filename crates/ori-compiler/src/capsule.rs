//! Module capsule (`ori.capsule.v1`) generator.
//!
//! The capsule is the per-module summary handed to agents: a stable JSON
//! envelope listing exported symbols, signatures, invariants, and a recommended
//! context window.

use crate::ast::{Module, Symbol};
use crate::json::to_json;
use serde::Serialize;

/// Stable schema identifier for the capsule envelope.
pub const CAPSULE_SCHEMA: &str = "ori.capsule.v1";

/// Number of symbols included in `agent.recommended_context`. Picked to fit
/// the default agent map budget without truncation while staying small enough
/// to keep prompts compact.
const RECOMMENDED_CONTEXT_LIMIT: usize = 12;

/// FNV-1a 64-bit offset basis. See <http://www.isthe.com/chongo/tech/comp/fnv/>.
const FNV1A_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime.
const FNV1A_PRIME: u64 = 0x100_0000_01b3;

#[derive(Debug, Serialize)]
struct Capsule<'a> {
    schema: &'static str,
    module: &'a str,
    path: &'a str,
    hash: String,
    exports: Vec<CapsuleSymbol<'a>>,
    imports: &'a [String],
    invariants: Vec<String>,
    agent: CapsuleAgent,
}

#[derive(Debug, Serialize)]
struct CapsuleSymbol<'a> {
    id: &'a str,
    kind: &'static str,
    name: &'a str,
    signature: &'a str,
    effects: &'a [String],
    calls: Vec<String>,
    tests: Vec<String>,
    summary: String,
}

#[derive(Debug, Serialize)]
struct CapsuleAgent {
    token_summary: String,
    recommended_context: Vec<String>,
}

/// Build the `ori.capsule.v1` JSON envelope for `module`.
pub fn module_capsule_json(module: &Module) -> String {
    let exports = module
        .exported_symbols()
        .map(symbol_json)
        .collect::<Vec<_>>();
    let recommended_context = module
        .exported_symbols()
        .take(RECOMMENDED_CONTEXT_LIMIT)
        .map(|symbol| symbol.id.clone())
        .collect::<Vec<_>>();
    let capsule = Capsule {
        schema: CAPSULE_SCHEMA,
        module: &module.name,
        path: &module.path,
        hash: format!("fnv1a:{:016x}", module_hash(module)),
        exports,
        imports: &module.imports,
        invariants: vec![
            "No null values; use Option[T].".to_string(),
            "No exceptions; use Result[T, E].".to_string(),
        ],
        agent: CapsuleAgent {
            token_summary: format!(
                "Module {} with {} exported symbols and {} imports.",
                module.name,
                module.exported_symbols().count(),
                module.imports.len()
            ),
            recommended_context,
        },
    };
    to_json(&capsule)
}

fn symbol_json(symbol: &Symbol) -> CapsuleSymbol<'_> {
    CapsuleSymbol {
        id: &symbol.id,
        kind: symbol.kind.as_str(),
        name: &symbol.name,
        signature: &symbol.signature,
        effects: &symbol.effects,
        calls: Vec::new(),
        tests: Vec::new(),
        summary: format!(
            "{} `{}` declared in this module.",
            symbol.kind.as_str(),
            symbol.name
        ),
    }
}

fn module_hash(module: &Module) -> u64 {
    let mut input = String::new();
    input.push_str(&module.name);
    for import in &module.imports {
        input.push('|');
        input.push_str(import);
    }
    for symbol in module.exported_symbols() {
        input.push('|');
        input.push_str(&symbol.id);
        input.push(':');
        input.push_str(&symbol.signature);
    }
    stable_hash(&input)
}

fn stable_hash(input: &str) -> u64 {
    let mut hash = FNV1A_OFFSET_BASIS;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV1A_PRIME);
    }
    hash
}
