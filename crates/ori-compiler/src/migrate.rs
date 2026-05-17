//! Edition migration planner.
//!
//! Edition transitions in Orison are described as a set of *migration
//! candidates*: deterministic, mechanical rewrites that move source from one
//! edition to the next. The planner inspects the parsed `Module`s and emits
//! a [`MigrationReport`] listing every candidate it would apply, but never
//! mutates source files itself - the report is the contract the CLI prints
//! and downstream tooling consumes.
//!
//! The bootstrap supports two transitions:
//!
//! * `2027.1 -> 2028.1`: deprecates the coarse `fs` effect in favour of the
//!   `fs.read` / `fs.write` split, and renames the historical
//!   `app.service` module to the canonical plural `app.services`.

use crate::ast::Module;
use serde::Serialize;

/// Schema identifier embedded in every JSON migration report.
pub const MIGRATION_REPORT_SCHEMA: &str = "ori.migration_report.v1";

/// Structured outcome of running [`plan_migration`].
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MigrationReport {
    pub schema: &'static str,
    pub from: String,
    pub to: String,
    pub candidates: Vec<MigrationCandidate>,
    pub applied: bool,
}

impl MigrationReport {
    pub fn to_json(&self) -> String {
        crate::json::to_json(self)
    }
}

/// One rewrite the planner would perform.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MigrationCandidate {
    pub kind: String,
    pub target: String,
    pub from_form: String,
    pub to_form: String,
    pub rationale: String,
}

/// Structured error used when the planner refuses a transition.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MigrationError {
    pub schema: &'static str,
    pub from: String,
    pub to: String,
    pub message: String,
}

impl MigrationError {
    pub fn to_json(&self) -> String {
        crate::json::to_json(self)
    }
}

/// Build a [`MigrationReport`] for migrating `modules` from `from_edition` to
/// `to_edition`.
///
/// Identical editions are a deterministic no-op. Unknown edition pairs
/// return a report with a single `unsupported_edition` candidate so the CLI
/// can surface a structured error without panicking.
pub fn plan_migration(modules: &[Module], from_edition: &str, to_edition: &str) -> MigrationReport {
    let mut candidates: Vec<MigrationCandidate> = Vec::new();

    if from_edition == to_edition {
        return finalise(from_edition, to_edition, candidates);
    }

    match (from_edition, to_edition) {
        ("2027.1", "2028.1") => {
            candidates.extend(plan_2027_1_to_2028_1(modules));
        }
        _ => {
            candidates.push(MigrationCandidate {
                kind: "unsupported_edition".to_string(),
                target: format!("edition:{from_edition}->{to_edition}"),
                from_form: from_edition.to_string(),
                to_form: to_edition.to_string(),
                rationale: format!(
                    "no known migration recipe from edition `{from_edition}` to `{to_edition}`"
                ),
            });
        }
    }

    finalise(from_edition, to_edition, candidates)
}

/// Convenience constructor: build a structured error for unknown edition
/// pairs. CLI callers can return this verbatim when they want hard-fail
/// behavior instead of an in-report `unsupported_edition` candidate.
pub fn unsupported_edition_error(from_edition: &str, to_edition: &str) -> MigrationError {
    MigrationError {
        schema: MIGRATION_REPORT_SCHEMA,
        from: from_edition.to_string(),
        to: to_edition.to_string(),
        message: format!(
            "no known migration recipe from edition `{from_edition}` to `{to_edition}`"
        ),
    }
}

fn finalise(
    from_edition: &str,
    to_edition: &str,
    mut candidates: Vec<MigrationCandidate>,
) -> MigrationReport {
    candidates.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| a.target.cmp(&b.target))
            .then_with(|| a.from_form.cmp(&b.from_form))
    });
    MigrationReport {
        schema: MIGRATION_REPORT_SCHEMA,
        from: from_edition.to_string(),
        to: to_edition.to_string(),
        candidates,
        applied: false,
    }
}

fn plan_2027_1_to_2028_1(modules: &[Module]) -> Vec<MigrationCandidate> {
    let mut out: Vec<MigrationCandidate> = Vec::new();

    for module in modules {
        if module.name == "app.service" {
            out.push(MigrationCandidate {
                kind: "rename_module".to_string(),
                target: format!("mod:{}", module.name),
                from_form: "module app.service".to_string(),
                to_form: "module app.services".to_string(),
                rationale: "module `app.service` was renamed to plural `app.services` for consistency with other domain layers"
                    .to_string(),
            });
        }

        for import in &module.imports {
            if import == "app.service" {
                out.push(MigrationCandidate {
                    kind: "rename_module".to_string(),
                    target: format!("import:{}:{import}", module.name),
                    from_form: "import app.service".to_string(),
                    to_form: "import app.services".to_string(),
                    rationale: "callers must follow the `app.service` -> `app.services` rename"
                        .to_string(),
                });
            }
        }

        for symbol in &module.symbols {
            if symbol.effects.iter().any(|effect| effect == "fs") {
                out.push(MigrationCandidate {
                    kind: "deprecate_effect".to_string(),
                    target: symbol.id.clone(),
                    from_form: "uses fs".to_string(),
                    to_form: "uses fs.read, fs.write".to_string(),
                    rationale:
                        "coarse `fs` effect was split into `fs.read` and `fs.write`; declare only the narrower effect actually used"
                            .to_string(),
                });
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Module, Symbol, SymbolKind};
    use crate::source::Span;

    fn module_with_fs_effect() -> Module {
        let mut module = Module::new("app.service", "/app/service.ori");
        module.imports.push("std.json".to_string());
        module.symbols.push(Symbol {
            id: "sym:app.service.write_log".to_string(),
            name: "write_log".to_string(),
            kind: SymbolKind::Function,
            signature: "fn write_log(line: Str) -> Unit".to_string(),
            effects: vec!["fs".to_string()],
            span: Span::new("/app/service.ori", 1, 1, 1, 2),
        });
        module
    }

    fn consumer_of_app_service() -> Module {
        let mut module = Module::new("app.main", "/app/main.ori");
        module.imports.push("app.service".to_string());
        module
    }

    #[test]
    fn identical_edition_is_a_no_op() {
        let modules = vec![module_with_fs_effect()];
        let report = plan_migration(&modules, "2028.1", "2028.1");
        assert_eq!(report.from, "2028.1");
        assert_eq!(report.to, "2028.1");
        assert!(report.candidates.is_empty());
        assert!(!report.applied);
    }

    #[test]
    fn known_migration_produces_candidates() {
        let modules = vec![module_with_fs_effect(), consumer_of_app_service()];
        let report = plan_migration(&modules, "2027.1", "2028.1");
        assert_eq!(report.schema, MIGRATION_REPORT_SCHEMA);
        assert!(report
            .candidates
            .iter()
            .any(|c| c.kind == "deprecate_effect" && c.target == "sym:app.service.write_log"));
        assert!(report
            .candidates
            .iter()
            .any(|c| c.kind == "rename_module" && c.target == "mod:app.service"));
        assert!(report
            .candidates
            .iter()
            .any(|c| c.kind == "rename_module" && c.target.starts_with("import:")));
        // Sorted deterministically.
        let mut sorted = report.candidates.clone();
        sorted.sort_by(|a, b| {
            a.kind
                .cmp(&b.kind)
                .then_with(|| a.target.cmp(&b.target))
                .then_with(|| a.from_form.cmp(&b.from_form))
        });
        assert_eq!(sorted, report.candidates);
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn unknown_edition_pair_records_unsupported_candidate() {
        let modules = vec![module_with_fs_effect()];
        let report = plan_migration(&modules, "1999.0", "2099.0");
        assert_eq!(report.candidates.len(), 1);
        let Some(candidate) = report.candidates.first() else {
            assert!(false, "expected at least one candidate");
            return;
        };
        assert_eq!(candidate.kind, "unsupported_edition");
        assert!(candidate.rationale.contains("no known migration recipe"));
    }

    #[test]
    fn unsupported_edition_error_carries_schema_and_message() {
        let err = unsupported_edition_error("1999.0", "2099.0");
        assert_eq!(err.schema, MIGRATION_REPORT_SCHEMA);
        assert_eq!(err.from, "1999.0");
        assert_eq!(err.to, "2099.0");
        assert!(err.message.contains("no known migration recipe"));
    }

    #[test]
    fn report_serialises_with_schema_field() {
        let modules = vec![module_with_fs_effect()];
        let report = plan_migration(&modules, "2027.1", "2028.1");
        let json = report.to_json();
        assert!(json.contains("\"schema\":\"ori.migration_report.v1\""));
        assert!(json.contains("\"applied\":false"));
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn schema_constant_is_stable() {
        if MIGRATION_REPORT_SCHEMA != "ori.migration_report.v1" {
            assert!(false, "MIGRATION_REPORT_SCHEMA must remain stable");
        }
    }
}
