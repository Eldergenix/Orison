//! Minimal interpreter for `ori run`.
//!
//! The bootstrap interpreter executes a tiny, deterministic operational
//! model that is enough for the demo storefront to "boot": it walks a
//! module's declared functions, looks up an `entry` (`boot` by default), and
//! emits a structured `RunReport` describing what would have run with which
//! effects. It does *not* execute user expression bodies — those land when
//! the parser learns to recover bodies.
//!
//! Even at this fidelity the report gives agents a stable JSON contract for
//! "what does running this module mean today" and lets the demo guide's
//! `ori run examples/hello.ori` acceptance command produce useful output.

use crate::ast::{Module, SymbolKind};
use crate::json::to_json;
use serde::Serialize;
use std::collections::BTreeSet;

/// Stable schema identifier for the run envelope.
pub const RUN_REPORT_SCHEMA: &str = "ori.run.v1";

/// JSON envelope produced by [`run_module`].
#[derive(Debug, Serialize)]
pub struct RunReport {
    /// Stable schema identifier.
    pub schema: &'static str,
    /// Module that was executed.
    pub module: String,
    /// Entry function actually targeted.
    pub entry: String,
    /// Outcome of the run (`ok`, `missing_entry`, ...).
    pub status: &'static str,
    /// Effects observed while walking the call graph.
    pub effects_observed: Vec<String>,
    /// Step-by-step trace for agent consumption.
    pub trace: Vec<RunStep>,
    /// Concatenated stdout text printed by the report (not the program).
    pub stdout: String,
}

/// One entry in [`RunReport::trace`].
#[derive(Debug, Serialize)]
pub struct RunStep {
    /// Symbol id this step refers to.
    pub symbol_id: String,
    /// Step kind (`enter`, `call_candidate`, ...).
    pub kind: String,
    /// Human-readable message.
    pub message: String,
}

impl RunReport {
    /// Render the report as a canonical JSON string.
    pub fn to_json(&self) -> String {
        to_json(self)
    }
}

/// Walk `module`, locate the requested entry function, and produce a
/// [`RunReport`]. When `entry` is `None` the bootstrap tries `main`, `boot`,
/// `run`, `start` in order.
pub fn run_module(module: &Module, entry: Option<&str>) -> RunReport {
    // Try the explicit entry first; otherwise fall back to the conventional
    // entrypoints in priority order.
    let candidates: Vec<&str> = match entry {
        Some(name) => vec![name],
        None => vec!["main", "boot", "run", "start"],
    };
    let entry_sym = candidates.iter().find_map(|name| {
        module
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Function && s.name == *name)
    });
    let entry_name = entry_sym
        .map(|s| s.name.as_str())
        .unwrap_or_else(|| candidates.first().copied().unwrap_or("main"));

    let mut trace = Vec::new();
    let mut effects: BTreeSet<String> = BTreeSet::new();
    let mut stdout = String::new();

    if let Some(sym) = entry_sym {
        trace.push(RunStep {
            symbol_id: sym.id.clone(),
            kind: "enter".to_string(),
            message: format!("entered `{}`", sym.name),
        });
        for eff in &sym.effects {
            effects.insert(eff.clone());
        }
        stdout.push_str(&format!("ori run: executed `{}` (no body)\n", sym.name));

        // Walk callees as best-effort by name: any function whose signature
        // mentions another declared function name is reported as a callee
        // candidate. Bootstrap-quality only.
        let names: BTreeSet<&str> = module
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function && s.name != sym.name)
            .map(|s| s.name.as_str())
            .collect();
        for callee_name in names {
            if sym.signature.contains(callee_name) {
                if let Some(callee) = module.symbols.iter().find(|s| s.name == callee_name) {
                    trace.push(RunStep {
                        symbol_id: callee.id.clone(),
                        kind: "call_candidate".to_string(),
                        message: format!("name-based callee candidate `{}`", callee.name),
                    });
                    for eff in &callee.effects {
                        effects.insert(eff.clone());
                    }
                }
            }
        }

        RunReport {
            schema: RUN_REPORT_SCHEMA,
            module: module.name.clone(),
            entry: sym.name.clone(),
            status: "ok",
            effects_observed: effects.into_iter().collect(),
            trace,
            stdout,
        }
    } else {
        RunReport {
            schema: RUN_REPORT_SCHEMA,
            module: module.name.clone(),
            entry: entry_name.to_string(),
            status: "missing_entry",
            effects_observed: Vec::new(),
            trace: Vec::new(),
            stdout: format!(
                "ori run: module `{}` does not declare entry function `{}`\n",
                module.name, entry_name
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_source;
    use crate::source::SourceFile;

    fn module_for(text: &str) -> Module {
        parse_source(&SourceFile::new("/t.ori", text)).module
    }

    #[test]
    fn run_reports_missing_entry() {
        let module = module_for("module a\nfn helper() -> Unit");
        let report = run_module(&module, None);
        assert_eq!(report.status, "missing_entry");
    }

    #[test]
    fn run_records_observed_effects_from_entry() {
        let module = module_for("module a\nfn main() -> Unit uses log, fs.read");
        let report = run_module(&module, None);
        assert_eq!(report.status, "ok");
        assert!(report.effects_observed.contains(&"log".to_string()));
        assert!(report.effects_observed.contains(&"fs.read".to_string()));
    }

    #[test]
    fn run_falls_back_to_boot_when_main_absent() {
        let module = module_for("module a\nfn boot() -> Unit uses ui");
        let report = run_module(&module, None);
        assert_eq!(report.status, "ok");
        assert_eq!(report.entry, "boot");
    }

    #[test]
    fn run_can_target_alternate_entry() {
        let module = module_for("module a\nfn launch() -> Unit uses ui");
        let report = run_module(&module, Some("launch"));
        assert_eq!(report.entry, "launch");
        assert!(report.effects_observed.contains(&"ui".to_string()));
    }
}
