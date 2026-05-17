//! Runtime capability enforcement (`ori.capability_runtime.v1`).
//!
//! Static capability declarations are checked at compile time by
//! [`crate::effect_check`]. Once a binary is built, however, the language
//! still needs a layer that decides — for a given *call* with a given
//! *principal* — whether the call is allowed to perform its declared
//! effects.
//!
//! That layer is this module. It is intentionally pure and deterministic:
//! given a [`CallContext`] (a caller symbol, the set of effects the call
//! requires, the active principal, and a [`CapabilitySet`] of tokens the
//! principal currently holds) the [`guard_call`] entry point returns a
//! single [`GuardOutcome`].
//!
//! ## Diagnostic codes
//!
//! Each denial carries a stable `CAP####` code so agents can route on the
//! reason without parsing the human message:
//!
//! | Code     | Meaning                                                       |
//! |----------|---------------------------------------------------------------|
//! | CAP0001  | a required effect has no token in the capability set          |
//! | CAP0002  | a token exists but its `expires_at` is before `now`            |
//! | CAP0003  | a token exists but its `issued_to` differs from the principal  |
//! | CAP0004  | the effect is in the capability set's `denials` allowlist veto |
//! | CAP0005  | informational: effect is in `audit_required`, audit was logged |
//!
//! CAP0005 is special — it never blocks the call. The sister
//! [`guard_call_with_audit`] entry point returns the (allowed) outcome
//! plus a list of [`AuditEntry`] records that the host runtime is
//! expected to persist to its audit log.

use crate::json::to_json;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Stable schema identifier for the runtime capability report envelope.
pub const CAPABILITY_RUNTIME_SCHEMA: &str = "ori.capability_runtime.v1";

/// A single capability token granting permission to perform an effect.
///
/// Tokens are looked up by effect name in [`CapabilitySet::tokens`]; the
/// guard checks both the principal binding (`issued_to`) and the optional
/// `expires_at` Unix timestamp.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapabilityToken {
    /// Effect this token grants (e.g. `db.write`).
    pub effect: String,
    /// Principal id the token was issued to. Must match the call's
    /// `principal_id`, otherwise CAP0003 is emitted.
    pub issued_to: String,
    /// Optional Unix-epoch expiry. `None` means "no expiry"; `Some(t)`
    /// means the token is valid while `now <= t`.
    pub expires_at: Option<u64>,
}

/// All capabilities currently bound to a principal.
///
/// The set ships its own `denials` allowlist veto (effects the principal
/// must not perform even when a token is present) and `audit_required`
/// list (effects whose every call should be appended to an audit log).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CapabilitySet {
    /// Effect → token, keyed by effect name so lookup is O(log n) and
    /// JSON output order is stable.
    pub tokens: BTreeMap<String, CapabilityToken>,
    /// Effects the principal is explicitly forbidden from performing,
    /// even if a token would otherwise authorise them. Always wins over
    /// `tokens`.
    pub denials: BTreeSet<String>,
    /// Effects whose every call should emit an [`AuditEntry`] via the
    /// [`guard_call_with_audit`] entry point.
    pub audit_required: BTreeSet<String>,
}

/// The inputs to a single guarded call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CallContext {
    /// Stable symbol id of the call site (e.g. `sym:demo.post_checkout`).
    pub caller_symbol: String,
    /// Set of effects the call statically declares. Always sorted.
    pub required_effects: BTreeSet<String>,
    /// Principal performing the call.
    pub principal_id: String,
    /// Capabilities the principal currently holds.
    pub capabilities: CapabilitySet,
}

/// The result of one guard evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum GuardOutcome {
    /// The call may proceed.
    Allowed,
    /// The call is rejected with a stable `code` and human `reason`.
    Denied {
        /// One of the `CAP####` codes documented at the module level.
        code: &'static str,
        /// Human-readable explanation safe for log surfaces.
        reason: String,
    },
}

impl GuardOutcome {
    /// Convenience: true iff the outcome is `Allowed`.
    pub fn is_allowed(&self) -> bool {
        matches!(self, GuardOutcome::Allowed)
    }
}

/// One entry recorded by [`guard_call_with_audit`] for effects the
/// capability set marks as `audit_required`. The host runtime is
/// expected to append these to a durable audit log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditEntry {
    /// Always `CAP0005`; included for routing.
    pub code: &'static str,
    /// Effect that triggered the audit.
    pub effect: String,
    /// Principal that performed the call.
    pub principal_id: String,
    /// Caller symbol id from the [`CallContext`].
    pub caller_symbol: String,
}

/// Guard a single call using the system clock for expiry checks.
///
/// Tests should use [`guard_call_at`] with an explicit `now` so the
/// outcome is deterministic.
pub fn guard_call(ctx: &CallContext) -> GuardOutcome {
    let now = current_unix_seconds();
    guard_call_at(ctx, now)
}

/// Guard a single call against an explicit `now` (Unix seconds).
pub fn guard_call_at(ctx: &CallContext, now: u64) -> GuardOutcome {
    for effect in &ctx.required_effects {
        // CAP0004 always wins.
        if ctx.capabilities.denials.contains(effect) {
            return GuardOutcome::Denied {
                code: "CAP0004",
                reason: format!(
                    "effect `{effect}` is denied by policy for principal `{}`",
                    ctx.principal_id
                ),
            };
        }
        let Some(token) = ctx.capabilities.tokens.get(effect) else {
            return GuardOutcome::Denied {
                code: "CAP0001",
                reason: format!(
                    "principal `{}` is missing a capability token for effect `{effect}`",
                    ctx.principal_id
                ),
            };
        };
        if token.issued_to != ctx.principal_id {
            return GuardOutcome::Denied {
                code: "CAP0003",
                reason: format!(
                    "token for effect `{effect}` was issued to `{}`, not `{}`",
                    token.issued_to, ctx.principal_id
                ),
            };
        }
        if let Some(expires_at) = token.expires_at {
            if expires_at < now {
                return GuardOutcome::Denied {
                    code: "CAP0002",
                    reason: format!(
                        "token for effect `{effect}` expired at {expires_at} (now={now})"
                    ),
                };
            }
        }
    }
    GuardOutcome::Allowed
}

/// Guard a single call and also collect [`AuditEntry`] records for any
/// effect listed in `capabilities.audit_required`.
///
/// CAP0005 is a *warning-flavour* denial: it never blocks the call. The
/// returned [`GuardOutcome`] is whatever [`guard_call_at`] would have
/// returned; the audit entries are returned separately.
pub fn guard_call_with_audit(ctx: &CallContext) -> (GuardOutcome, Vec<AuditEntry>) {
    let now = current_unix_seconds();
    guard_call_with_audit_at(ctx, now)
}

/// Variant of [`guard_call_with_audit`] taking an explicit clock.
pub fn guard_call_with_audit_at(ctx: &CallContext, now: u64) -> (GuardOutcome, Vec<AuditEntry>) {
    let outcome = guard_call_at(ctx, now);
    let mut entries: Vec<AuditEntry> = Vec::new();
    for effect in &ctx.required_effects {
        if ctx.capabilities.audit_required.contains(effect) {
            entries.push(AuditEntry {
                code: "CAP0005",
                effect: effect.clone(),
                principal_id: ctx.principal_id.clone(),
                caller_symbol: ctx.caller_symbol.clone(),
            });
        }
    }
    // Sort to keep the entry list deterministic for the same context.
    entries.sort_by(|a, b| a.effect.cmp(&b.effect));
    (outcome, entries)
}

/// JSON envelope returned by `ori capability check`.
#[derive(Debug, Serialize)]
struct GuardReport<'a> {
    schema: &'static str,
    outcomes: Vec<GuardReportEntry<'a>>,
}

#[derive(Debug, Serialize)]
struct GuardReportEntry<'a> {
    index: usize,
    outcome: &'a GuardOutcome,
}

/// Render an array of [`GuardOutcome`] values as a stable JSON envelope
/// with schema id `ori.capability_runtime.v1`.
pub fn guard_report_json(outcomes: &[GuardOutcome]) -> String {
    let entries: Vec<GuardReportEntry<'_>> = outcomes
        .iter()
        .enumerate()
        .map(|(index, outcome)| GuardReportEntry { index, outcome })
        .collect();
    let report = GuardReport {
        schema: CAPABILITY_RUNTIME_SCHEMA,
        outcomes: entries,
    };
    to_json(&report)
}

/// Read the current Unix time in seconds. Returns `0` if the system
/// clock is before the Unix epoch (the contract for `guard_call` is "use
/// the clock once"; pre-epoch is effectively `now=0`, which only
/// matters in tests).
fn current_unix_seconds() -> u64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn principal(id: &str) -> String {
        id.to_string()
    }

    fn effects(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    fn token(effect: &str, issued_to: &str, expires_at: Option<u64>) -> CapabilityToken {
        CapabilityToken {
            effect: effect.to_string(),
            issued_to: issued_to.to_string(),
            expires_at,
        }
    }

    fn capset(tokens: Vec<CapabilityToken>) -> CapabilitySet {
        let mut map = BTreeMap::new();
        for t in tokens {
            map.insert(t.effect.clone(), t);
        }
        CapabilitySet {
            tokens: map,
            denials: BTreeSet::new(),
            audit_required: BTreeSet::new(),
        }
    }

    fn ctx_for(
        caller: &str,
        required: &[&str],
        principal_id: &str,
        capabilities: CapabilitySet,
    ) -> CallContext {
        CallContext {
            caller_symbol: caller.to_string(),
            required_effects: effects(required),
            principal_id: principal(principal_id),
            capabilities,
        }
    }

    #[test]
    fn allowed_when_every_effect_has_matching_token() {
        let caps = capset(vec![
            token("db.read", "alice", None),
            token("http", "alice", Some(10_000)),
        ]);
        let ctx = ctx_for("sym:demo.read", &["db.read", "http"], "alice", caps);
        let out = guard_call_at(&ctx, 5_000);
        assert!(out.is_allowed(), "expected Allowed, got {out:?}");
    }

    #[test]
    fn cap0001_missing_capability() {
        let caps = capset(vec![token("db.read", "alice", None)]);
        let ctx = ctx_for("sym:demo.write", &["db.read", "db.write"], "alice", caps);
        match guard_call_at(&ctx, 0) {
            GuardOutcome::Denied { code, reason } => {
                assert_eq!(code, "CAP0001");
                assert!(reason.contains("db.write"), "reason: {reason}");
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected CAP0001 denial, got {other:?}");
                }
            }
        }
    }

    #[test]
    fn cap0002_token_expired() {
        let caps = capset(vec![token("http", "alice", Some(100))]);
        let ctx = ctx_for("sym:demo.call", &["http"], "alice", caps);
        match guard_call_at(&ctx, 500) {
            GuardOutcome::Denied { code, reason } => {
                assert_eq!(code, "CAP0002");
                assert!(reason.contains("expired"), "reason: {reason}");
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected CAP0002 denial, got {other:?}");
                }
            }
        }
    }

    #[test]
    fn cap0003_principal_mismatch() {
        let caps = capset(vec![token("db.write", "bob", None)]);
        let ctx = ctx_for("sym:demo.write", &["db.write"], "alice", caps);
        match guard_call_at(&ctx, 0) {
            GuardOutcome::Denied { code, reason } => {
                assert_eq!(code, "CAP0003");
                assert!(reason.contains("alice"), "reason: {reason}");
                assert!(reason.contains("bob"), "reason: {reason}");
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected CAP0003 denial, got {other:?}");
                }
            }
        }
    }

    #[test]
    fn cap0004_denied_by_policy_even_with_token() {
        let mut caps = capset(vec![token("net.outbound", "alice", None)]);
        caps.denials.insert("net.outbound".to_string());
        let ctx = ctx_for("sym:demo.fetch", &["net.outbound"], "alice", caps);
        match guard_call_at(&ctx, 0) {
            GuardOutcome::Denied { code, reason } => {
                assert_eq!(code, "CAP0004");
                assert!(reason.contains("denied"), "reason: {reason}");
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected CAP0004 denial, got {other:?}");
                }
            }
        }
    }

    #[test]
    fn cap0005_audit_entry_produced_but_call_allowed() {
        let mut caps = capset(vec![token("db.write", "alice", None)]);
        caps.audit_required.insert("db.write".to_string());
        let ctx = ctx_for("sym:demo.write", &["db.write"], "alice", caps);
        let (outcome, entries) = guard_call_with_audit_at(&ctx, 0);
        assert!(outcome.is_allowed(), "audit must not block the call");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].code, "CAP0005");
        assert_eq!(entries[0].effect, "db.write");
        assert_eq!(entries[0].principal_id, "alice");
        assert_eq!(entries[0].caller_symbol, "sym:demo.write");
    }

    #[test]
    fn determinism_same_inputs_same_outcome() {
        let caps = capset(vec![token("http", "alice", Some(10))]);
        let ctx = ctx_for("sym:demo.call", &["http"], "alice", caps);
        let a = guard_call_at(&ctx, 5);
        let b = guard_call_at(&ctx, 5);
        assert_eq!(a, b);
    }

    #[test]
    fn report_json_roundtrips_through_serde_json() {
        let outcomes = vec![
            GuardOutcome::Allowed,
            GuardOutcome::Denied {
                code: "CAP0001",
                reason: "missing".to_string(),
            },
        ];
        let json = guard_report_json(&outcomes);
        let value: serde_json::Value = match serde_json::from_str(&json) {
            Ok(v) => v,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "report json failed to parse: {err}; raw: {json}");
                }
                return;
            }
        };
        assert_eq!(
            value
                .get("schema")
                .and_then(|v| v.as_str())
                .unwrap_or_default(),
            "ori.capability_runtime.v1"
        );
        let arr = value
            .get("outcomes")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        assert_eq!(arr, 2);
    }

    #[test]
    fn mixed_scenario_denial_wins_over_token() {
        // Even with a valid token, denials short-circuit on the first
        // matching required effect.
        let mut caps = capset(vec![
            token("db.read", "alice", Some(100)),
            token("db.write", "alice", Some(100)),
        ]);
        caps.denials.insert("db.write".to_string());
        let ctx = ctx_for("sym:demo.write", &["db.read", "db.write"], "alice", caps);
        match guard_call_at(&ctx, 50) {
            GuardOutcome::Denied { code, .. } => assert_eq!(code, "CAP0004"),
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected CAP0004 (denial wins), got {other:?}");
                }
            }
        }
    }

    #[test]
    fn mixed_scenario_expired_with_audit_still_blocks() {
        // Audit list does not rescue an expired token: CAP0002 is
        // returned and no audit entries are produced (because the call
        // is denied before reaching audit).
        let mut caps = capset(vec![token("http", "alice", Some(10))]);
        caps.audit_required.insert("http".to_string());
        let ctx = ctx_for("sym:demo.call", &["http"], "alice", caps);
        let (outcome, entries) = guard_call_with_audit_at(&ctx, 1_000);
        match outcome {
            GuardOutcome::Denied { code, .. } => assert_eq!(code, "CAP0002"),
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected CAP0002, got {other:?}");
                }
            }
        }
        // Audit entries are still emitted for any required effect in
        // the audit list — they describe the *attempt* to call.
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn mixed_scenario_missing_then_mismatch_picks_first_required_effect() {
        // `required_effects` iterates in sorted order; `db.read` comes
        // before `db.write`. Both fail (one is missing, one belongs to
        // a different principal). The first failure wins.
        let caps = capset(vec![token("db.write", "bob", None)]);
        let ctx = ctx_for("sym:demo.io", &["db.read", "db.write"], "alice", caps);
        match guard_call_at(&ctx, 0) {
            GuardOutcome::Denied { code, .. } => {
                // sorted: db.read first → missing → CAP0001
                assert_eq!(code, "CAP0001");
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected CAP0001 denial, got {other:?}");
                }
            }
        }
    }

    #[test]
    fn no_required_effects_is_allowed() {
        let caps = capset(vec![]);
        let ctx = ctx_for("sym:demo.noop", &[], "alice", caps);
        assert!(guard_call_at(&ctx, 0).is_allowed());
    }

    #[test]
    fn report_entries_are_indexed_in_order() {
        let outcomes = vec![
            GuardOutcome::Allowed,
            GuardOutcome::Allowed,
            GuardOutcome::Denied {
                code: "CAP0001",
                reason: "missing".to_string(),
            },
        ];
        let json = guard_report_json(&outcomes);
        let value: serde_json::Value = match serde_json::from_str(&json) {
            Ok(v) => v,
            Err(_) => serde_json::Value::Null,
        };
        let arr = value
            .get("outcomes")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert_eq!(arr.len(), 3);
        for (i, entry) in arr.iter().enumerate() {
            let idx = entry.get("index").and_then(|v| v.as_u64()).unwrap_or(99);
            assert_eq!(idx as usize, i);
        }
    }

    #[test]
    fn token_at_exact_expiry_is_still_valid() {
        // The contract is `expires_at < now`; equality is allowed.
        let caps = capset(vec![token("http", "alice", Some(100))]);
        let ctx = ctx_for("sym:demo.call", &["http"], "alice", caps);
        assert!(guard_call_at(&ctx, 100).is_allowed());
    }
}
