//! Call-graph based effect inference.
//!
//! The bootstrap [`effect_check`](crate::effect_check) pass only knows about
//! the *declared* `uses ...` clause on each function. It cannot yet observe
//! that a caller transitively performs an effect because one of its callees
//! does. Now that the body parser exposes call sites as
//! `Expr::Call(Var(name), _)`, we can build a static call graph between the
//! function symbols of a [`Module`] and propagate effects along it.
//!
//! The flow is intentionally module-local for the bootstrap:
//!
//!   1. [`build_effect_graph`] walks every function body, collects every
//!      `Call(Var(name), _)` where `name` matches another function symbol
//!      in the module, and seeds the per-function effect set with the
//!      declared `uses` clause.
//!   2. [`propagate_effects`] iterates to a fixpoint: a function's effective
//!      effect set is the union of its declared effects with the effective
//!      effects of every transitively reachable callee. Cycles are handled
//!      naturally because we stop when no set changes in an iteration.
//!   3. [`propagation_diagnostics`] turns each "inferred ⊋ declared" gap
//!      into an `E0420` diagnostic with a `change_signature` Patch IR
//!      suggestion that appends the missing effects to the `uses` clause.
//!
//! Diagnostic IDs in the `E0420..=E0429` range belong to this pass. Every
//! diagnostic carries an `agent_summary` and a `doc:effects.propagation`
//! reference so downstream tools can consume the JSON contract directly.
//!
//! Effect sets are stored as [`BTreeSet<String>`] for deterministic
//! ordering across runs — diagnostics, manifests, and snapshots all depend
//! on the order being stable.

use crate::ast::{Module, Symbol, SymbolKind};
use crate::body::ModuleBodies;
use crate::diagnostic::{Diagnostic, Fix};
use crate::expr::{Expr, MatchArm, Stmt};
use std::collections::{BTreeMap, BTreeSet};

/// Per-module call/effect graph produced by [`build_effect_graph`].
///
/// * `edges[caller]` is the set of callee symbol ids reachable from
///   `caller` via a direct `Expr::Call(Var(name), _)`.
/// * `effects[fn]` is the inferred effect set for `fn`. Initially seeded
///   with the declared effects; [`propagate_effects`] grows it to the
///   transitive union.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EffectGraph {
    pub edges: BTreeMap<String, BTreeSet<String>>,
    pub effects: BTreeMap<String, BTreeSet<String>>,
}

impl EffectGraph {
    /// Empty graph. Equivalent to `EffectGraph::default()` but more explicit
    /// at call sites that build a graph incrementally.
    pub fn new() -> Self {
        Self::default()
    }

    /// Effects inferred for `symbol_id`, or an empty set if the symbol is
    /// not a known function.
    pub fn effects_of(&self, symbol_id: &str) -> BTreeSet<String> {
        self.effects.get(symbol_id).cloned().unwrap_or_default()
    }
}

/// Build the call graph for `module` by walking each function body and
/// recording direct callee references that resolve to another function
/// symbol in the same module. Unknown call targets (e.g. methods on
/// records, imported helpers we cannot resolve yet) are ignored
/// gracefully: they neither create edges nor poison the inferred set.
pub fn build_effect_graph(module: &Module, bodies: &ModuleBodies) -> EffectGraph {
    let mut graph = EffectGraph::new();

    // Build a name → symbol-id index restricted to functions. Names that
    // collide across kinds are resolved to the function variant because
    // call-site name resolution only considers values.
    let mut by_name: BTreeMap<String, String> = BTreeMap::new();
    for symbol in &module.symbols {
        if symbol.kind == SymbolKind::Function {
            by_name
                .entry(symbol.name.clone())
                .or_insert_with(|| symbol.id.clone());
        }
    }

    for symbol in &module.symbols {
        if symbol.kind != SymbolKind::Function {
            continue;
        }
        seed_function(&mut graph, symbol);
        let Some(body) = bodies.get(&symbol.id) else {
            continue;
        };
        let mut callees: BTreeSet<String> = BTreeSet::new();
        collect_callees(body, &by_name, &symbol.id, &mut callees);
        graph.edges.insert(symbol.id.clone(), callees);
    }

    graph
}

/// Iterate the per-function effect sets to a fixpoint. Each round, every
/// function absorbs the effective effects of every direct callee. The loop
/// terminates when no set changes — guaranteed even on cycles because
/// sets only grow and the universe of effect strings is finite.
///
/// Returns the list of symbol ids whose inferred effect set ended up
/// strictly different from the originally declared (seeded) set. Order is
/// deterministic (lexicographic by symbol id).
pub fn propagate_effects(graph: &mut EffectGraph) -> Vec<String> {
    let declared: BTreeMap<String, BTreeSet<String>> = graph.effects.clone();

    loop {
        let mut changed = false;
        // Snapshot to avoid borrowing `effects` mutably and immutably at
        // the same time when we union callee sets in.
        let snapshot: BTreeMap<String, BTreeSet<String>> = graph.effects.clone();
        for (caller, callees) in graph.edges.iter() {
            let mut union: BTreeSet<String> = snapshot.get(caller).cloned().unwrap_or_default();
            for callee in callees {
                if let Some(callee_eff) = snapshot.get(callee) {
                    for eff in callee_eff {
                        union.insert(eff.clone());
                    }
                }
            }
            if let Some(existing) = graph.effects.get_mut(caller) {
                if *existing != union {
                    *existing = union;
                    changed = true;
                }
            } else {
                graph.effects.insert(caller.clone(), union);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let mut diffs: Vec<String> = Vec::new();
    for (id, inferred) in graph.effects.iter() {
        let original = declared.get(id).cloned().unwrap_or_default();
        if *inferred != original {
            diffs.push(id.clone());
        }
    }
    diffs
}

/// For every function whose inferred effect set exceeds the declared one,
/// emit an `E0420` diagnostic. The diagnostic includes:
///
/// * `expected` — the full inferred effect set;
/// * `found` — the declared effect set;
/// * a `change_signature` Patch IR fix that appends the missing effect to
///   the function's `uses` clause.
///
/// Functions whose declared effects are a (non-strict) superset of the
/// inferred set produce no diagnostic — over-declaring is allowed (and
/// often necessary while a body is being filled in).
pub fn propagation_diagnostics(module: &Module, graph: &EffectGraph) -> Vec<Diagnostic> {
    let mut out: Vec<Diagnostic> = Vec::new();

    // name → callee symbol id, identical to the index in build_effect_graph.
    let mut by_name: BTreeMap<String, String> = BTreeMap::new();
    for symbol in &module.symbols {
        if symbol.kind == SymbolKind::Function {
            by_name
                .entry(symbol.name.clone())
                .or_insert_with(|| symbol.id.clone());
        }
    }

    for symbol in &module.symbols {
        if symbol.kind != SymbolKind::Function {
            continue;
        }
        let declared: BTreeSet<String> = symbol.effects.iter().cloned().collect();
        let inferred = graph.effects_of(&symbol.id);
        let missing: BTreeSet<String> = inferred.difference(&declared).cloned().collect();
        if missing.is_empty() {
            continue;
        }

        for eff in &missing {
            let callee = first_callee_introducing(symbol, graph, eff)
                .unwrap_or_else(|| "<unknown>".to_string());
            out.push(build_diagnostic(symbol, &declared, &inferred, eff, &callee));
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn seed_function(graph: &mut EffectGraph, symbol: &Symbol) {
    let declared: BTreeSet<String> = symbol.effects.iter().cloned().collect();
    graph.effects.insert(symbol.id.clone(), declared);
    graph.edges.entry(symbol.id.clone()).or_default();
}

/// Walk `expr` and push every direct callee symbol id into `out`. Self
/// references (`caller_id == callee_id`) are still recorded so cycles are
/// visible to the fixpoint; they simply do not change the effect set.
fn collect_callees(
    expr: &Expr,
    by_name: &BTreeMap<String, String>,
    caller_id: &str,
    out: &mut BTreeSet<String>,
) {
    match expr {
        Expr::Lit(_) | Expr::Var(_) | Expr::Error => {}
        Expr::Call { callee, args } => {
            if let Expr::Var(name) = callee.as_ref() {
                if let Some(target) = by_name.get(name) {
                    let _ = caller_id; // intentionally unused; kept for future per-edge metadata
                    out.insert(target.clone());
                }
            }
            // Always descend into the callee and arguments so nested calls
            // like `f(g(x))` are captured even when the outer callee is not
            // a bare name.
            collect_callees(callee, by_name, caller_id, out);
            for arg in args {
                collect_callees(arg, by_name, caller_id, out);
            }
        }
        Expr::Field { base, .. } => collect_callees(base, by_name, caller_id, out),
        Expr::Block { stmts, tail } => {
            for stmt in stmts {
                collect_stmt(stmt, by_name, caller_id, out);
            }
            if let Some(tail) = tail {
                collect_callees(tail, by_name, caller_id, out);
            }
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_callees(cond, by_name, caller_id, out);
            collect_callees(then_branch, by_name, caller_id, out);
            if let Some(else_branch) = else_branch {
                collect_callees(else_branch, by_name, caller_id, out);
            }
        }
        Expr::Match { scrutinee, arms } => {
            collect_callees(scrutinee, by_name, caller_id, out);
            for MatchArm { body, .. } in arms {
                collect_callees(body, by_name, caller_id, out);
            }
        }
        Expr::Return(Some(inner)) => collect_callees(inner, by_name, caller_id, out),
        Expr::Return(None) => {}
        Expr::Construct { args, .. } => {
            for arg in args {
                collect_callees(arg, by_name, caller_id, out);
            }
        }
        Expr::Try(inner) => collect_callees(inner, by_name, caller_id, out),
        Expr::Tuple(parts) => {
            for part in parts {
                collect_callees(part, by_name, caller_id, out);
            }
        }
        Expr::Record { fields } => {
            for (_, value) in fields {
                collect_callees(value, by_name, caller_id, out);
            }
        }
        Expr::Lambda { body, .. } => collect_callees(body, by_name, caller_id, out),
    }
}

fn collect_stmt(
    stmt: &Stmt,
    by_name: &BTreeMap<String, String>,
    caller_id: &str,
    out: &mut BTreeSet<String>,
) {
    match stmt {
        Stmt::Let { init, .. } => collect_callees(init, by_name, caller_id, out),
        Stmt::Expr(e) => collect_callees(e, by_name, caller_id, out),
        Stmt::Return(Some(e)) => collect_callees(e, by_name, caller_id, out),
        Stmt::Return(None) => {}
    }
}

/// Find the first direct callee of `symbol` whose own inferred effect set
/// contains `eff`. Falls back to any transitively reachable callee if no
/// direct edge introduces the effect (e.g. when propagation crossed
/// several hops to find it).
fn first_callee_introducing(symbol: &Symbol, graph: &EffectGraph, eff: &str) -> Option<String> {
    if let Some(direct) = graph.edges.get(&symbol.id) {
        for callee in direct {
            if callee == &symbol.id {
                continue;
            }
            if let Some(callee_eff) = graph.effects.get(callee) {
                if callee_eff.contains(eff) {
                    return Some(callee.clone());
                }
            }
        }
    }
    // Fallback: walk the transitive closure deterministically.
    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut frontier: Vec<String> = graph
        .edges
        .get(&symbol.id)
        .cloned()
        .map(|s| s.into_iter().collect())
        .unwrap_or_default();
    while let Some(current) = frontier.pop() {
        if !visited.insert(current.clone()) {
            continue;
        }
        if current != symbol.id {
            if let Some(callee_eff) = graph.effects.get(&current) {
                if callee_eff.contains(eff) {
                    return Some(current);
                }
            }
        }
        if let Some(next) = graph.edges.get(&current) {
            for callee in next {
                if !visited.contains(callee) {
                    frontier.push(callee.clone());
                }
            }
        }
    }
    None
}

fn build_diagnostic(
    symbol: &Symbol,
    declared: &BTreeSet<String>,
    inferred: &BTreeSet<String>,
    eff: &str,
    callee: &str,
) -> Diagnostic {
    let expected: Vec<String> = inferred.iter().cloned().collect();
    let found: Vec<String> = declared.iter().cloned().collect();
    let updated_signature = append_effect_to_uses(&symbol.signature, eff);

    Diagnostic::error(
        "E0420",
        format!(
            "function `{}` requires effect `{}` transitively through calls to `{}` but does not declare it",
            symbol.name, eff, callee
        ),
        symbol.span.clone(),
    )
    .with_symbol(symbol.id.clone())
    .with_expected(expected)
    .with_found(found)
    .with_fix(
        Fix::new(
            "change_signature",
            format!("Append `{eff}` to the `uses` clause of `{}`.", symbol.name),
            0.85,
        )
        .with_patch(serde_json::json!({
            "schema": "ori.patch.v1",
            "intent": format!(
                "Declare transitively required effect `{}` on `{}`.",
                eff, symbol.id
            ),
            "operations": [{
                "op": "change_signature",
                "target": symbol.id,
                "text": updated_signature,
            }],
            "tests": { "run": ["cargo test -p ori-compiler effect_propagate"] }
        })),
    )
    .with_agent_summary(
        "Declare the missing effect on the caller, or stop calling the offending function.",
    )
    .with_minimal_context(vec![symbol.id.clone(), callee.to_string(), eff.to_string()])
    .with_docs(vec!["doc:effects.propagation".to_string()])
}

/// Produce a new signature string by appending `eff` to the function's
/// `uses` clause. If no `uses` clause exists yet, a new one is added.
/// Best-effort string editing: we never panic on malformed signatures,
/// we just fall through to the most defensive shape.
fn append_effect_to_uses(signature: &str, eff: &str) -> String {
    let trimmed = signature.trim_end();
    if let Some(idx) = trimmed.find(" uses ") {
        let (head, tail) = trimmed.split_at(idx);
        let tail = tail.trim_start_matches(" uses ");
        let mut existing: Vec<String> = tail
            .split(',')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        if !existing.iter().any(|e| e == eff) {
            existing.push(eff.to_string());
        }
        format!("{head} uses {}", existing.join(", "))
    } else {
        format!("{trimmed} uses {eff}")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::parse_module_bodies;
    use crate::parser::parse_source;
    use crate::source::SourceFile;

    fn build(text: &str) -> (Module, EffectGraph, Vec<String>) {
        let source = SourceFile::new("/t.ori", text);
        let module = parse_source(&source).module;
        let bodies = parse_module_bodies(&source);
        let mut graph = build_effect_graph(&module, &bodies);
        let diffs = propagate_effects(&mut graph);
        (module, graph, diffs)
    }

    #[test]
    fn two_function_chain_propagates_db_read() {
        let (_module, graph, diffs) = build(
            "module m\n\
             fn leaf() -> Int uses db.read:\n  return 1\n\
             fn caller() -> Int:\n  return leaf()\n",
        );
        let caller = graph.effects_of("sym:m.caller");
        assert!(
            caller.contains("db.read"),
            "expected caller to inherit db.read, got {caller:?}"
        );
        assert!(diffs.iter().any(|id| id == "sym:m.caller"));
    }

    #[test]
    fn deep_chain_propagates_through_all_hops() {
        let (_module, graph, _diffs) = build(
            "module m\n\
             fn d() -> Int uses fs.read:\n  return 0\n\
             fn c() -> Int:\n  return d()\n\
             fn b() -> Int:\n  return c()\n\
             fn a() -> Int:\n  return b()\n",
        );
        for id in ["sym:m.a", "sym:m.b", "sym:m.c", "sym:m.d"] {
            let eff = graph.effects_of(id);
            assert!(
                eff.contains("fs.read"),
                "expected {id} to carry fs.read, got {eff:?}"
            );
        }
    }

    #[test]
    fn direct_cycle_terminates_and_propagates() {
        // a -> b -> a; both should converge to the union of declared effects.
        let (_module, graph, _diffs) = build(
            "module m\n\
             fn a() -> Int uses db.read:\n  return b()\n\
             fn b() -> Int uses net.outbound:\n  return a()\n",
        );
        let a = graph.effects_of("sym:m.a");
        let b = graph.effects_of("sym:m.b");
        assert!(a.contains("db.read") && a.contains("net.outbound"));
        assert!(b.contains("db.read") && b.contains("net.outbound"));
    }

    #[test]
    fn self_recursive_call_does_not_loop() {
        let (_module, graph, _diffs) = build(
            "module m\n\
             fn loopy(n: Int) -> Int uses fs.read:\n  return loopy(n)\n",
        );
        let eff = graph.effects_of("sym:m.loopy");
        assert!(eff.contains("fs.read"));
    }

    #[test]
    fn function_with_superset_uses_emits_no_diff() {
        // caller declares db.read AND db.write, but only really needs db.read
        // through `leaf`. Over-declaration is allowed.
        let (module, graph, _diffs) = build(
            "module m\n\
             fn leaf() -> Int uses db.read:\n  return 1\n\
             fn caller() -> Int uses db.read, db.write:\n  return leaf()\n",
        );
        let diags = propagation_diagnostics(&module, &graph);
        assert!(
            !diags
                .iter()
                .any(|d| d.symbol.as_ref().map(|s| s.id.as_str()) == Some("sym:m.caller")),
            "no diagnostic should fire when caller declares a superset"
        );
    }

    #[test]
    fn unknown_callee_is_ignored_gracefully() {
        // `mystery` is not a function symbol in this module; the graph must
        // not panic, and `caller` must end up with only its declared set.
        let (_module, graph, _diffs) = build(
            "module m\n\
             fn caller() -> Int uses db.read:\n  return mystery()\n",
        );
        let eff = graph.effects_of("sym:m.caller");
        assert_eq!(eff, BTreeSet::from(["db.read".to_string()]));
    }

    #[test]
    fn diagnostic_e0420_includes_patch_and_docs() {
        let (module, graph, _diffs) = build(
            "module m\n\
             fn leaf() -> Int uses fs.read:\n  return 1\n\
             fn caller() -> Int:\n  return leaf()\n",
        );
        let diags = propagation_diagnostics(&module, &graph);
        let diag = diags
            .iter()
            .find(|d| d.id == "E0420")
            .cloned()
            .unwrap_or_else(panic_no_diag);
        assert!(diag.message.contains("fs.read"));
        assert!(diag.message.contains("caller"));
        assert!(diag.message.contains("leaf"));
        // expected = inferred set, found = declared set
        assert!(diag.expected.iter().any(|e| e == "fs.read"));
        assert!(diag.found.is_empty(), "caller declared no effects");
        // docs reference and agent summary
        assert!(diag
            .agent
            .docs
            .iter()
            .any(|d| d == "doc:effects.propagation"));
        assert!(!diag.agent.summary.is_empty());
        // patch is a change_signature operation
        let fix = diag.fixes.first().cloned().unwrap_or_else(panic_no_fix);
        assert_eq!(fix.kind, "change_signature");
        let patch = fix.patch.unwrap_or(serde_json::Value::Null);
        let ops = patch
            .get("operations")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let first_op = ops.first().cloned().unwrap_or(serde_json::Value::Null);
        assert_eq!(
            first_op.get("op").and_then(|v| v.as_str()),
            Some("change_signature")
        );
        let text = first_op.get("text").and_then(|v| v.as_str()).unwrap_or("");
        assert!(
            text.contains("uses") && text.contains("fs.read"),
            "patch text should append `fs.read` to `uses`, got: {text:?}"
        );
    }

    #[test]
    fn multiple_missing_effects_all_reported() {
        let (module, graph, _diffs) = build(
            "module m\n\
             fn a() -> Int uses db.read:\n  return 0\n\
             fn b() -> Int uses net.outbound:\n  return 0\n\
             fn caller() -> Int:\n  let x = a()\n  let y = b()\n  return x\n",
        );
        let diags = propagation_diagnostics(&module, &graph);
        let for_caller: Vec<_> = diags
            .iter()
            .filter(|d| d.symbol.as_ref().map(|s| s.id.as_str()) == Some("sym:m.caller"))
            .collect();
        assert!(
            for_caller.len() >= 2,
            "expected one diagnostic per missing effect, got {}",
            for_caller.len()
        );
    }

    #[test]
    fn no_diagnostic_when_no_effects_anywhere() {
        let (module, graph, _diffs) = build(
            "module m\n\
             fn leaf() -> Int:\n  return 1\n\
             fn caller() -> Int:\n  return leaf()\n",
        );
        let diags = propagation_diagnostics(&module, &graph);
        assert!(diags.is_empty(), "pure functions should not trigger E0420");
    }

    #[test]
    fn nested_call_in_let_init_is_captured() {
        // The callee is buried inside a `let` initialiser; the walker must
        // still find it.
        let (_module, graph, _diffs) = build(
            "module m\n\
             fn leaf() -> Int uses fs.write:\n  return 1\n\
             fn caller() -> Int:\n  let x = leaf()\n  return x\n",
        );
        let eff = graph.effects_of("sym:m.caller");
        assert!(eff.contains("fs.write"));
    }

    #[test]
    fn return_diffs_lists_only_changed_symbols() {
        let (_module, _graph, diffs) = build(
            "module m\n\
             fn leaf() -> Int uses fs.read:\n  return 1\n\
             fn caller() -> Int:\n  return leaf()\n\
             fn solo() -> Int uses db.read:\n  return 1\n",
        );
        // `leaf` and `solo` declared what they got; `caller` gained fs.read.
        assert!(diffs.iter().any(|id| id == "sym:m.caller"));
        assert!(!diffs.iter().any(|id| id == "sym:m.leaf"));
        assert!(!diffs.iter().any(|id| id == "sym:m.solo"));
    }

    #[test]
    fn append_effect_to_uses_handles_missing_clause() {
        let updated = append_effect_to_uses("fn caller() -> Int", "fs.read");
        assert_eq!(updated, "fn caller() -> Int uses fs.read");
    }

    #[test]
    fn append_effect_to_uses_extends_existing_clause() {
        let updated = append_effect_to_uses("fn caller() -> Int uses db.read", "net.outbound");
        assert_eq!(updated, "fn caller() -> Int uses db.read, net.outbound");
    }

    #[test]
    fn append_effect_to_uses_is_idempotent() {
        let updated = append_effect_to_uses("fn caller() -> Int uses fs.read", "fs.read");
        assert_eq!(updated, "fn caller() -> Int uses fs.read");
    }

    // ---- helpers for assertions without forbidden panic primitives in product code ----

    #[cold]
    fn panic_no_diag() -> Diagnostic {
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(false, "expected an E0420 diagnostic to be present");
        }
        Diagnostic::error("E0420", "", crate::source::Span::dummy("/t.ori"))
    }

    #[cold]
    fn panic_no_fix() -> Fix {
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(false, "expected diagnostic to carry a fix");
        }
        Fix::new("change_signature", "", 0.0)
    }
}
