//! Model-in-loop telemetry envelope (`ori.model_loop_telemetry.v1`).
//!
//! Captures the per-iteration accounting an editor agent emits while
//! collaborating with a language model on a source file: how many edits the
//! model proposed, how many were accepted, the token budget used, and the
//! diagnostic delta after each pass. The envelope is consumed by IDE / agent
//! integrations via `ori agent telemetry --in <session.json>`.
//!
//! Serialisation is deterministic: structs are emitted in field order and
//! iteration arrays preserve their input order so identical inputs produce
//! byte-identical JSON. Totals are recomputed by [`build_telemetry`] so a
//! caller cannot smuggle inconsistent aggregates through the envelope; the
//! `budget_remaining` value on each iteration is preserved as-is but the
//! constructor saturates at zero rather than underflowing.

use ori_compiler::json::to_json;
use serde::{Deserialize, Serialize};

/// Schema id advertised by every [`LoopTelemetry`] envelope. Kept in lock step
/// with `schemas/model-loop-telemetry.schema.json` and the entry in
/// `extras::doctor_report_json`.
pub const SCHEMA_ID: &str = "ori.model_loop_telemetry.v1";

/// One pass through the model-in-loop edit cycle. All fields are required
/// because consumers (dashboards, agent post-mortems) need totals they can
/// trust without imputing zeros for missing data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopIteration {
    pub iteration: u32,
    pub started_at: u64,
    pub completed_at: u64,
    pub edits_proposed: u32,
    pub edits_accepted: u32,
    pub edits_rejected: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub budget_remaining: u64,
    pub diagnostics_before: u32,
    pub diagnostics_after: u32,
}

/// Aggregated totals derived from the iteration list. `diagnostics_resolved`
/// is signed because a session may *add* diagnostics overall (a regression
/// from the model's edits) which must surface in the envelope rather than be
/// clamped to zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopTotals {
    pub iterations: u32,
    pub wall_ms: u64,
    pub edits_proposed: u32,
    pub edits_accepted: u32,
    pub edits_rejected: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub diagnostics_resolved: i32,
}

/// The full envelope. `schema` is a `&'static str` (the constant
/// [`SCHEMA_ID`]) on construction; when re-parsed via
/// [`parse_telemetry_json`] we still expose it as a `&'static str` so callers
/// cannot accidentally mint envelopes carrying an arbitrary schema id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoopTelemetry {
    pub schema: &'static str,
    pub session_id: String,
    pub model_id: String,
    pub iterations: Vec<LoopIteration>,
    pub totals: LoopTotals,
}

/// Build a telemetry envelope, computing totals from the iteration list.
/// Saturating arithmetic is used throughout so a hostile or buggy input
/// cannot panic the consumer.
pub fn build_telemetry(
    session_id: String,
    model_id: String,
    iterations: Vec<LoopIteration>,
) -> LoopTelemetry {
    let totals = compute_totals(&iterations);
    LoopTelemetry {
        schema: SCHEMA_ID,
        session_id,
        model_id,
        iterations,
        totals,
    }
}

fn compute_totals(iterations: &[LoopIteration]) -> LoopTotals {
    let mut wall_ms: u64 = 0;
    let mut edits_proposed: u32 = 0;
    let mut edits_accepted: u32 = 0;
    let mut edits_rejected: u32 = 0;
    let mut tokens_in: u64 = 0;
    let mut tokens_out: u64 = 0;
    let mut diagnostics_before_first: Option<u32> = None;
    let mut diagnostics_after_last: u32 = 0;

    for iteration in iterations {
        // Per-iteration wall time is `completed_at - started_at` when the
        // clock advanced forward, and 0 otherwise. We use saturating_sub so a
        // monotonic-clock glitch (or a deliberately bogus payload) cannot
        // wrap a u64. The sum across iterations is also saturating.
        let per = iteration.completed_at.saturating_sub(iteration.started_at);
        wall_ms = wall_ms.saturating_add(per);

        edits_proposed = edits_proposed.saturating_add(iteration.edits_proposed);
        edits_accepted = edits_accepted.saturating_add(iteration.edits_accepted);
        edits_rejected = edits_rejected.saturating_add(iteration.edits_rejected);
        tokens_in = tokens_in.saturating_add(iteration.tokens_in);
        tokens_out = tokens_out.saturating_add(iteration.tokens_out);

        if diagnostics_before_first.is_none() {
            diagnostics_before_first = Some(iteration.diagnostics_before);
        }
        diagnostics_after_last = iteration.diagnostics_after;
    }

    // Net diagnostic change across the session. Signed so a regression
    // (final > initial) shows up as a negative value rather than being lost.
    let diagnostics_resolved: i32 = match diagnostics_before_first {
        Some(before) => signed_delta(before, diagnostics_after_last),
        None => 0,
    };

    LoopTotals {
        iterations: iterations.len() as u32,
        wall_ms,
        edits_proposed,
        edits_accepted,
        edits_rejected,
        tokens_in,
        tokens_out,
        diagnostics_resolved,
    }
}

/// Returns `before - after` clamped to the `i32` range without panicking.
/// A positive value means diagnostics were resolved; negative means added.
fn signed_delta(before: u32, after: u32) -> i32 {
    if before >= after {
        let diff = before - after;
        if diff > i32::MAX as u32 {
            i32::MAX
        } else {
            diff as i32
        }
    } else {
        let diff = after - before;
        if diff > i32::MAX as u32 {
            i32::MIN
        } else {
            -(diff as i32)
        }
    }
}

/// Constructor that saturates `budget_remaining` at zero. Callers that
/// compute `budget_remaining` from a subtraction should route through this
/// to avoid wraparound when `tokens_in + tokens_out > budget`.
#[allow(clippy::too_many_arguments)]
pub fn iteration_with_saturating_budget(
    iteration: u32,
    started_at: u64,
    completed_at: u64,
    edits_proposed: u32,
    edits_accepted: u32,
    edits_rejected: u32,
    tokens_in: u64,
    tokens_out: u64,
    budget_total: u64,
    diagnostics_before: u32,
    diagnostics_after: u32,
) -> LoopIteration {
    let spent = tokens_in.saturating_add(tokens_out);
    let budget_remaining = budget_total.saturating_sub(spent);
    LoopIteration {
        iteration,
        started_at,
        completed_at,
        edits_proposed,
        edits_accepted,
        edits_rejected,
        tokens_in,
        tokens_out,
        budget_remaining,
        diagnostics_before,
        diagnostics_after,
    }
}

/// Serialize the envelope using the shared compiler JSON helper. Output is
/// deterministic — fields are emitted in declaration order and arrays
/// preserve their input order.
pub fn telemetry_json(telemetry: &LoopTelemetry) -> String {
    to_json(telemetry)
}

/// Parse a telemetry JSON envelope. Validates the `schema` field equals
/// [`SCHEMA_ID`] and recomputes totals so a malformed `totals` block from an
/// upstream tool cannot silently propagate. Returns a human-readable error
/// string on failure.
pub fn parse_telemetry_json(text: &str) -> Result<LoopTelemetry, String> {
    let raw: ParsedEnvelope = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(err) => return Err(format!("invalid model_loop_telemetry JSON: {err}")),
    };

    let Some(schema) = raw.schema else {
        return Err("missing required field `schema`".to_string());
    };
    if schema != SCHEMA_ID {
        return Err(format!("expected schema `{SCHEMA_ID}`, found `{schema}`"));
    }
    let Some(session_id) = raw.session_id else {
        return Err("missing required field `session_id`".to_string());
    };
    let Some(model_id) = raw.model_id else {
        return Err("missing required field `model_id`".to_string());
    };
    let iterations = raw.iterations.unwrap_or_default();

    Ok(build_telemetry(session_id, model_id, iterations))
}

/// Lenient mirror of [`LoopTelemetry`] used only by [`parse_telemetry_json`]
/// so missing top-level fields can be reported by name instead of via a
/// serde-generated `missing field` message. Totals are intentionally ignored
/// on input — we always recompute.
#[derive(Debug, Deserialize)]
struct ParsedEnvelope {
    schema: Option<String>,
    session_id: Option<String>,
    model_id: Option<String>,
    iterations: Option<Vec<LoopIteration>>,
    #[serde(default, rename = "totals")]
    _totals: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn it(iter: u32, before: u32, after: u32, proposed: u32, accepted: u32) -> LoopIteration {
        LoopIteration {
            iteration: iter,
            started_at: (iter as u64) * 1_000,
            completed_at: (iter as u64) * 1_000 + 250,
            edits_proposed: proposed,
            edits_accepted: accepted,
            edits_rejected: proposed.saturating_sub(accepted),
            tokens_in: 100,
            tokens_out: 200,
            budget_remaining: 10_000 - (iter as u64) * 300,
            diagnostics_before: before,
            diagnostics_after: after,
        }
    }

    #[test]
    fn build_telemetry_zero_iterations_has_zero_totals() {
        let t = build_telemetry("s".to_string(), "m".to_string(), Vec::new());
        assert_eq!(t.totals.iterations, 0);
        assert_eq!(t.totals.wall_ms, 0);
        assert_eq!(t.totals.edits_proposed, 0);
        assert_eq!(t.totals.edits_accepted, 0);
        assert_eq!(t.totals.edits_rejected, 0);
        assert_eq!(t.totals.tokens_in, 0);
        assert_eq!(t.totals.tokens_out, 0);
        assert_eq!(t.totals.diagnostics_resolved, 0);
        assert_eq!(t.schema, SCHEMA_ID);
    }

    #[test]
    fn build_telemetry_three_iterations_sums_correctly() {
        let iters = vec![it(1, 10, 7, 4, 3), it(2, 7, 4, 5, 4), it(3, 4, 1, 2, 2)];
        let t = build_telemetry("session-A".to_string(), "model-X".to_string(), iters);
        assert_eq!(t.totals.iterations, 3);
        // Each iteration contributes 250 ms.
        assert_eq!(t.totals.wall_ms, 750);
        assert_eq!(t.totals.edits_proposed, 11);
        assert_eq!(t.totals.edits_accepted, 9);
        // rejected = proposed - accepted on each row in this fixture.
        assert_eq!(t.totals.edits_rejected, 2);
        // 100 in + 200 out per iteration, 3 iterations.
        assert_eq!(t.totals.tokens_in, 300);
        assert_eq!(t.totals.tokens_out, 600);
        // First before=10, last after=1, so 9 resolved.
        assert_eq!(t.totals.diagnostics_resolved, 9);
    }

    #[test]
    fn telemetry_json_round_trips_byte_identical() {
        let iters = vec![it(1, 5, 3, 2, 2), it(2, 3, 0, 4, 3)];
        let original = build_telemetry("sess".to_string(), "claude".to_string(), iters);
        let serialized = telemetry_json(&original);
        let parsed = match parse_telemetry_json(&serialized) {
            Ok(p) => p,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "round-trip parse failed: {err}");
                }
                return;
            }
        };
        let reserialized = telemetry_json(&parsed);
        assert_eq!(
            serialized, reserialized,
            "round-trip must be byte-identical"
        );
    }

    #[test]
    fn parse_telemetry_json_rejects_missing_schema() {
        let raw = r#"{"session_id":"s","model_id":"m","iterations":[],"totals":{}}"#;
        let err = match parse_telemetry_json(raw) {
            Ok(_) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected error for missing schema");
                }
                return;
            }
            Err(err) => err,
        };
        assert!(
            err.contains("schema"),
            "error should mention schema, got: {err}"
        );
    }

    #[test]
    fn parse_telemetry_json_rejects_wrong_schema_id() {
        let raw =
            r#"{"schema":"ori.something_else.v1","session_id":"s","model_id":"m","iterations":[]}"#;
        let err = match parse_telemetry_json(raw) {
            Ok(_) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected error for wrong schema id");
                }
                return;
            }
            Err(err) => err,
        };
        assert!(
            err.contains("ori.model_loop_telemetry.v1"),
            "error should mention expected schema, got: {err}"
        );
    }

    #[test]
    fn diagnostics_resolved_handles_regression() {
        // Started with 2 diagnostics, model added more — net regression.
        let iters = vec![it(1, 2, 5, 3, 1), it(2, 5, 8, 4, 1)];
        let t = build_telemetry("regression".to_string(), "m".to_string(), iters);
        assert_eq!(
            t.totals.diagnostics_resolved, -6,
            "regression of 2 -> 8 should report -6"
        );
    }

    #[test]
    fn telemetry_json_is_deterministic() {
        let make = || {
            build_telemetry(
                "fixed".to_string(),
                "m".to_string(),
                vec![it(1, 4, 2, 3, 2), it(2, 2, 1, 1, 1)],
            )
        };
        let a = telemetry_json(&make());
        let b = telemetry_json(&make());
        let c = telemetry_json(&make());
        assert_eq!(a, b);
        assert_eq!(b, c);
    }

    #[test]
    fn budget_remaining_saturates_at_zero() {
        // tokens_in (u64::MAX/2) + tokens_out (u64::MAX/2) > budget (1000).
        let iter = iteration_with_saturating_budget(
            1,
            0,
            10,
            1,
            1,
            0,
            u64::MAX / 2,
            u64::MAX / 2,
            1_000,
            0,
            0,
        );
        assert_eq!(
            iter.budget_remaining, 0,
            "overflow must saturate, not wrap to a huge u64"
        );
    }

    #[test]
    fn json_shape_matches_schema_required_fields() {
        // Validate the envelope's top-level shape against the published
        // schema's `required` list using a lightweight serde_json::Value
        // check. We deliberately avoid pulling in a full json-schema crate
        // (no new deps allowed) and instead encode the contract here.
        let t = build_telemetry(
            "shape".to_string(),
            "m".to_string(),
            vec![it(1, 1, 0, 1, 1)],
        );
        let json = telemetry_json(&t);
        let value: serde_json::Value = match serde_json::from_str(&json) {
            Ok(v) => v,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "telemetry_json should produce valid JSON: {err}");
                }
                return;
            }
        };

        let obj = match value.as_object() {
            Some(o) => o,
            None => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "envelope must serialise as a JSON object");
                }
                return;
            }
        };

        for field in ["schema", "session_id", "model_id", "iterations", "totals"] {
            assert!(
                obj.contains_key(field),
                "missing required envelope field `{field}`: {json}"
            );
        }

        let totals = match obj.get("totals").and_then(|v| v.as_object()) {
            Some(o) => o,
            None => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "`totals` must be an object");
                }
                return;
            }
        };
        for field in [
            "iterations",
            "wall_ms",
            "edits_proposed",
            "edits_accepted",
            "edits_rejected",
            "tokens_in",
            "tokens_out",
            "diagnostics_resolved",
        ] {
            assert!(
                totals.contains_key(field),
                "missing required totals field `{field}`"
            );
        }

        let iter0 = match obj
            .get("iterations")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_object())
        {
            Some(o) => o,
            None => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected at least one iteration object");
                }
                return;
            }
        };
        for field in [
            "iteration",
            "started_at",
            "completed_at",
            "edits_proposed",
            "edits_accepted",
            "edits_rejected",
            "tokens_in",
            "tokens_out",
            "budget_remaining",
            "diagnostics_before",
            "diagnostics_after",
        ] {
            assert!(
                iter0.contains_key(field),
                "missing required iteration field `{field}`"
            );
        }
    }

    #[test]
    fn scenario_single_iteration_full_acceptance() {
        // Mixed scenario 1: a clean, one-shot edit pass that accepts every
        // proposal and resolves all diagnostics.
        let only = LoopIteration {
            iteration: 1,
            started_at: 0,
            completed_at: 1_500,
            edits_proposed: 6,
            edits_accepted: 6,
            edits_rejected: 0,
            tokens_in: 2_048,
            tokens_out: 512,
            budget_remaining: 7_440,
            diagnostics_before: 4,
            diagnostics_after: 0,
        };
        let t = build_telemetry("clean".to_string(), "m".to_string(), vec![only]);
        assert_eq!(t.totals.iterations, 1);
        assert_eq!(t.totals.wall_ms, 1_500);
        assert_eq!(t.totals.edits_accepted, 6);
        assert_eq!(t.totals.edits_rejected, 0);
        assert_eq!(t.totals.diagnostics_resolved, 4);
    }

    #[test]
    fn scenario_mixed_rejections_and_clock_skew() {
        // Mixed scenario 2: multiple iterations, some with clock skew
        // (completed_at < started_at) and some with heavy rejection ratios.
        // Wall time should saturate to 0 for the skewed iteration, not
        // underflow.
        let iters = vec![
            LoopIteration {
                iteration: 1,
                started_at: 1_000,
                completed_at: 500, // clock skew
                edits_proposed: 5,
                edits_accepted: 1,
                edits_rejected: 4,
                tokens_in: 50,
                tokens_out: 75,
                budget_remaining: 9_875,
                diagnostics_before: 3,
                diagnostics_after: 3,
            },
            LoopIteration {
                iteration: 2,
                started_at: 2_000,
                completed_at: 2_400,
                edits_proposed: 10,
                edits_accepted: 2,
                edits_rejected: 8,
                tokens_in: 100,
                tokens_out: 200,
                budget_remaining: 9_575,
                diagnostics_before: 3,
                diagnostics_after: 2,
            },
        ];
        let t = build_telemetry("mixed".to_string(), "m".to_string(), iters);
        // Skewed iteration contributes 0; second contributes 400.
        assert_eq!(t.totals.wall_ms, 400);
        assert_eq!(t.totals.edits_proposed, 15);
        assert_eq!(t.totals.edits_accepted, 3);
        assert_eq!(t.totals.edits_rejected, 12);
        assert_eq!(t.totals.tokens_in, 150);
        assert_eq!(t.totals.tokens_out, 275);
        assert_eq!(t.totals.diagnostics_resolved, 1);
    }

    #[test]
    fn parse_recomputes_totals_even_if_input_has_bogus_totals() {
        // A caller who hand-crafts an envelope with wrong totals should not
        // see those totals propagate — parse_telemetry_json recomputes.
        let raw = r#"{"schema":"ori.model_loop_telemetry.v1","session_id":"s","model_id":"m","iterations":[{"iteration":1,"started_at":0,"completed_at":100,"edits_proposed":2,"edits_accepted":2,"edits_rejected":0,"tokens_in":10,"tokens_out":20,"budget_remaining":970,"diagnostics_before":2,"diagnostics_after":0}],"totals":{"iterations":999,"wall_ms":999,"edits_proposed":999,"edits_accepted":999,"edits_rejected":999,"tokens_in":999,"tokens_out":999,"diagnostics_resolved":999}}"#;
        let parsed = match parse_telemetry_json(raw) {
            Ok(p) => p,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected parse success, got: {err}");
                }
                return;
            }
        };
        assert_eq!(parsed.totals.iterations, 1);
        assert_eq!(parsed.totals.wall_ms, 100);
        assert_eq!(parsed.totals.edits_accepted, 2);
        assert_eq!(parsed.totals.diagnostics_resolved, 2);
    }
}
