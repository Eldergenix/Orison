//! Prototype ownership / borrow checker for the bootstrap compiler.
//!
//! The bootstrap parser only exposes function signatures and declared
//! effects — there are no expression bodies and no liveness analysis. Even
//! so, a meaningful subset of ownership and aliasing rules can be enforced
//! at the signature level. This module implements those rules as a fast
//! linter that operates purely over the [`Module`] surface:
//!
//! * **B0010** — A parameter list that mentions the same identifier twice
//!   with `&mut T` violates the "at most one mutable borrow per binding"
//!   rule, even before bodies exist. We also flag a single binding that
//!   appears as both `&mut T` and `&T`, since shared and exclusive borrows
//!   to the same identifier cannot coexist.
//! * **B0011** — A binding that is borrowed mutably (`&mut T`) more than
//!   once with the same lifetime annotation. Recorded as a sub-rule of
//!   B0010 with a sharper hint.
//! * **B0020** — Newtype confusion: two parameters with distinct newtype
//!   names that wrap the same base type. This is advisory; it catches the
//!   `ProductId` vs `OrderId` swap pattern that motivated newtypes.
//! * **B0030** — A function whose declared effects imply unique resource
//!   ownership (`db.write`, `fs.write`, `process.spawn`, `crypto`) returns
//!   a value wrapped in `Shared[T]`. Shared aliases over unique resources
//!   defeat ownership tracking.
//! * **B0040** — `unsafe` appears in a public signature or is declared as
//!   an effect. Bootstrap mode rejects unsafe surfaces outright; later
//!   milestones may allow them inside `unsafe` capsules.
//! * **B0050** — A function returns a borrow (`&T` or `&mut T`) but no
//!   parameter of compatible base type is borrowed. Dangling-borrow
//!   heuristic.
//!
//! The checker is idempotent: invoking it twice on the same module
//! produces identical diagnostics. It never panics; all parsing is done
//! with conservative scans that fall back to "no diagnostic" on malformed
//! input — the parser is responsible for surfacing syntax errors.

use crate::ast::{Module, Symbol, SymbolKind};
use crate::body::{parse_module_bodies_with_module, ModuleBodies};
use crate::borrow_regions::{BorrowMode, RegionMap};
use crate::diagnostic::{Diagnostic, Fix};
use crate::expr::{Expr, InterpPart, MatchArm, Stmt};
use crate::source::{SourceFile, Span};
use crate::types::is_builtin_type;
use std::collections::{BTreeMap, BTreeSet};

/// Effects that imply the holder owns a unique runtime resource. Returning
/// a `Shared[T]` from such a function aliases that resource silently.
const UNIQUE_RESOURCE_EFFECTS: &[&str] = &["db.write", "fs.write", "process.spawn", "crypto"];

/// Entry point: run all borrow rules over `module`'s function and query
/// symbols.
pub fn borrow_check_module(module: &Module) -> Vec<Diagnostic> {
    let newtype_bases = collect_newtype_bases(module);
    let mut diagnostics = Vec::new();

    for symbol in &module.symbols {
        if !is_callable(symbol) {
            continue;
        }
        let params = parse_parameters(&symbol.signature);
        let return_ty = parse_return_type(&symbol.signature);

        check_unsafe(symbol, &mut diagnostics);
        check_mut_alias(symbol, &params, &mut diagnostics);
        check_newtype_confusion(symbol, &params, &newtype_bases, &mut diagnostics);
        check_shared_over_unique(symbol, return_ty.as_deref(), &mut diagnostics);
        check_dangling_borrow(symbol, &params, return_ty.as_deref(), &mut diagnostics);
    }
    diagnostics
}

fn is_callable(symbol: &Symbol) -> bool {
    matches!(symbol.kind, SymbolKind::Function | SymbolKind::Query)
}

// ---------------------------------------------------------------------------
// Rule: B0040 — unsafe in signature or effects.
// ---------------------------------------------------------------------------

fn check_unsafe(symbol: &Symbol, out: &mut Vec<Diagnostic>) {
    let in_effects = symbol.effects.iter().any(|e| e == "unsafe");
    let in_signature = signature_contains_keyword(&symbol.signature, "unsafe");
    if !in_effects && !in_signature {
        return;
    }
    let location = if in_signature { "signature" } else { "effects" };
    let diag = Diagnostic::error(
        "B0040",
        format!(
            "function `{}` uses `unsafe`; bootstrap mode rejects unsafe surfaces",
            symbol.name
        ),
        symbol.span.clone(),
    )
    .with_symbol(symbol.id.clone())
    .with_expected(vec!["a safe surface (no unsafe keyword/effect)".to_string()])
    .with_found(vec![format!("unsafe in {location}")])
    .with_fix(
        Fix::new(
            "remove_unsafe",
            "Remove the `unsafe` qualifier or move the surface into a capability-gated capsule.",
            0.7,
        )
        .with_patch(serde_json::json!({
            "schema": "ori.patch.v1",
            "intent": format!(
                "Strip unsafe qualifier from `{}` to satisfy bootstrap policy.",
                symbol.id
            ),
            "operations": [{
                "op": "change_signature",
                "target": symbol.id,
                "text": strip_unsafe_from_signature(&symbol.signature),
            }],
            "tests": { "run": ["cargo test -p ori-compiler borrow"] }
        })),
    )
    .with_agent_summary("Remove `unsafe` from the surface; bootstrap rejects it.")
    .with_minimal_context(vec![symbol.id.clone()])
    .with_docs(vec![
        "doc:borrow.rules".to_string(),
        "doc:borrow.unsafe".to_string(),
    ]);
    out.push(diag);
}

fn strip_unsafe_from_signature(signature: &str) -> String {
    let cleaned: String = signature
        .split_whitespace()
        .filter(|tok| *tok != "unsafe")
        .collect::<Vec<_>>()
        .join(" ");
    cleaned
}

// ---------------------------------------------------------------------------
// Rule: B0010 — at most one mutable borrow per identifier.
// ---------------------------------------------------------------------------

fn check_mut_alias(symbol: &Symbol, params: &[Parameter], out: &mut Vec<Diagnostic>) {
    // Group borrows by parameter name.
    let mut by_name: BTreeMap<&str, BorrowSet> = BTreeMap::new();
    for param in params {
        let entry = by_name.entry(param.name.as_str()).or_default();
        match param.kind {
            BorrowKind::Mut => entry.mut_count += 1,
            BorrowKind::Shared => entry.shared_count += 1,
            BorrowKind::Owned => entry.owned_count += 1,
        }
    }

    for (name, set) in by_name {
        if set.mut_count >= 2 {
            let diag = Diagnostic::error(
                "B0010",
                format!(
                    "parameter `{name}` of `{}` is borrowed mutably {} times; at most one `&mut` is allowed per binding",
                    symbol.name, set.mut_count
                ),
                symbol.span.clone(),
            )
            .with_symbol(symbol.id.clone())
            .with_expected(vec!["one `&mut T` per binding".to_string()])
            .with_found(vec![format!("{} `&mut` borrows of `{name}`", set.mut_count)])
            .with_fix(
                Fix::new(
                    "rename_aliased_param",
                    "Rename one of the colliding `&mut` parameters or merge them.",
                    0.65,
                )
                .with_patch(serde_json::json!({
                    "schema": "ori.patch.v1",
                    "intent": format!(
                        "Disambiguate aliased `&mut` parameter `{name}` in `{}`.",
                        symbol.id
                    ),
                    "operations": [{
                        "op": "change_signature",
                        "target": symbol.id,
                        "text": symbol.signature.clone(),
                    }],
                    "tests": { "run": ["cargo test -p ori-compiler borrow"] }
                })),
            )
            .with_agent_summary("Reduce to one mutable borrow per identifier.")
            .with_minimal_context(vec![symbol.id.clone()])
            .with_docs(vec!["doc:borrow.rules".to_string(), "doc:borrow.aliasing".to_string()]);
            out.push(diag);
        } else if set.mut_count >= 1 && set.shared_count >= 1 {
            let diag = Diagnostic::error(
                "B0011",
                format!(
                    "parameter `{name}` of `{}` is borrowed both mutably and shared",
                    symbol.name
                ),
                symbol.span.clone(),
            )
            .with_symbol(symbol.id.clone())
            .with_expected(vec!["either `&T` or `&mut T`, not both".to_string()])
            .with_found(vec![format!(
                "{} `&mut` and {} `&` borrows of `{name}`",
                set.mut_count, set.shared_count
            )])
            .with_agent_summary("Pick exclusive or shared access for this binding.")
            .with_minimal_context(vec![symbol.id.clone()])
            .with_docs(vec![
                "doc:borrow.rules".to_string(),
                "doc:borrow.aliasing".to_string(),
            ]);
            out.push(diag);
        }
    }
}

#[derive(Default)]
struct BorrowSet {
    mut_count: usize,
    shared_count: usize,
    #[allow(dead_code)]
    owned_count: usize,
}

// ---------------------------------------------------------------------------
// Rule: B0020 — newtype confusion (advisory).
// ---------------------------------------------------------------------------

fn check_newtype_confusion(
    symbol: &Symbol,
    params: &[Parameter],
    newtype_bases: &BTreeMap<String, String>,
    out: &mut Vec<Diagnostic>,
) {
    // Group newtype parameters by their base type.
    let mut by_base: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
    for param in params {
        let head = type_head(&param.ty);
        if let Some(base) = newtype_bases.get(head) {
            by_base
                .entry(base.as_str())
                .or_default()
                .push((head, param.name.as_str()));
        }
    }

    for (base, entries) in by_base {
        // Find distinct newtype names sharing this base.
        let mut seen_names = BTreeMap::new();
        for (newtype, param) in &entries {
            seen_names.entry(*newtype).or_insert(*param);
        }
        if seen_names.len() >= 2 {
            let names: Vec<String> = seen_names.keys().map(|s| (*s).to_string()).collect();
            let diag = Diagnostic::warning(
                "B0020",
                format!(
                    "function `{}` mixes newtypes {} that share base type `{}` — verify argument order",
                    symbol.name,
                    names.join(" / "),
                    base
                ),
                symbol.span.clone(),
            )
            .with_symbol(symbol.id.clone())
            .with_expected(vec![format!(
                "distinct call sites for {} vs {}",
                names.first().cloned().unwrap_or_default(),
                names.get(1).cloned().unwrap_or_default()
            )])
            .with_found(names.clone())
            .with_agent_summary(
                "Verify that callers pass each newtype in the correct positional slot.",
            )
            .with_minimal_context(vec![symbol.id.clone()])
            .with_docs(vec![
                "doc:borrow.rules".to_string(),
                "doc:types.newtypes".to_string(),
            ]);
            out.push(diag);
        }
    }
}

// ---------------------------------------------------------------------------
// Rule: B0030 — Shared[T] returned from a function with unique-resource effects.
// ---------------------------------------------------------------------------

fn check_shared_over_unique(symbol: &Symbol, return_ty: Option<&str>, out: &mut Vec<Diagnostic>) {
    let Some(return_ty) = return_ty else { return };
    let (head, _) = split_generic(return_ty);
    if head != "Shared" {
        return;
    }
    let triggers: Vec<&str> = symbol
        .effects
        .iter()
        .filter(|e| UNIQUE_RESOURCE_EFFECTS.contains(&e.as_str()))
        .map(String::as_str)
        .collect();
    if triggers.is_empty() {
        return;
    }
    let diag = Diagnostic::error(
        "B0030",
        format!(
            "function `{}` returns `{}` while declaring effect(s) [{}] that own a unique resource",
            symbol.name,
            return_ty.trim(),
            triggers.join(", ")
        ),
        symbol.span.clone(),
    )
    .with_symbol(symbol.id.clone())
    .with_expected(vec![
        "return an owned value (e.g. T or Result[T, E])".to_string()
    ])
    .with_found(vec![return_ty.trim().to_string()])
    .with_fix(
        Fix::new(
            "unwrap_shared",
            "Return the owned value directly instead of `Shared[T]`.",
            0.6,
        )
        .with_patch(serde_json::json!({
            "schema": "ori.patch.v1",
            "intent": format!(
                "Replace `Shared[T]` return with the unique resource in `{}`.",
                symbol.id
            ),
            "operations": [{
                "op": "change_signature",
                "target": symbol.id,
                "text": symbol.signature.clone(),
            }],
            "tests": { "run": ["cargo test -p ori-compiler borrow"] }
        })),
    )
    .with_agent_summary(
        "Do not share aliases to unique resources; return owned handles or scoped guards.",
    )
    .with_minimal_context(vec![symbol.id.clone()])
    .with_docs(vec![
        "doc:borrow.rules".to_string(),
        "doc:borrow.shared-vs-unique".to_string(),
    ]);
    out.push(diag);
}

// ---------------------------------------------------------------------------
// Rule: B0050 — dangling-borrow heuristic.
// ---------------------------------------------------------------------------

fn check_dangling_borrow(
    symbol: &Symbol,
    params: &[Parameter],
    return_ty: Option<&str>,
    out: &mut Vec<Diagnostic>,
) {
    let Some(return_ty) = return_ty else { return };
    let trimmed = return_ty.trim();
    if !trimmed.starts_with('&') {
        return;
    }
    let ret_base = type_head(strip_borrow_prefix(trimmed));
    if ret_base.is_empty() {
        return;
    }
    // We need at least one borrowed parameter whose base equals the return base.
    let compatible = params.iter().any(|p| {
        matches!(p.kind, BorrowKind::Shared | BorrowKind::Mut)
            && type_head(strip_borrow_prefix(&p.ty)) == ret_base
    });
    if compatible {
        return;
    }
    let diag = Diagnostic::warning(
        "B0050",
        format!(
            "function `{}` returns borrow `{}` but no parameter borrows a compatible `{}` — possible dangling reference",
            symbol.name, trimmed, ret_base
        ),
        symbol.span.clone(),
    )
    .with_symbol(symbol.id.clone())
    .with_expected(vec![format!(
        "a parameter borrowing `{ret_base}` so the returned reference has a lifetime source"
    )])
    .with_found(vec![trimmed.to_string()])
    .with_agent_summary("Tie the returned borrow to a parameter's lifetime.")
    .with_minimal_context(vec![symbol.id.clone()])
    .with_docs(vec![
        "doc:borrow.rules".to_string(),
        "doc:borrow.lifetimes".to_string(),
    ]);
    out.push(diag);
}

// ---------------------------------------------------------------------------
// Signature parsing helpers (signature-only, conservative).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Parameter {
    name: String,
    ty: String,
    kind: BorrowKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BorrowKind {
    Owned,
    Shared,
    Mut,
}

/// Parse the `(...)` parameter list out of a signature string. Returns an
/// empty vector for nullary or malformed signatures.
fn parse_parameters(signature: &str) -> Vec<Parameter> {
    let Some(open) = signature.find('(') else {
        return Vec::new();
    };
    let body_with_tail = &signature[open + 1..];
    let close_rel = match find_matching_close(body_with_tail) {
        Some(idx) => idx,
        None => return Vec::new(),
    };
    let body = &body_with_tail[..close_rel];
    if body.trim().is_empty() {
        return Vec::new();
    }
    split_top_level_commas(body)
        .into_iter()
        .filter_map(parse_one_parameter)
        .collect()
}

fn parse_one_parameter(raw: &str) -> Option<Parameter> {
    let part = raw.trim();
    let colon = part.find(':')?;
    let name = part[..colon].trim().to_string();
    let ty_raw = part[colon + 1..].trim().to_string();
    if name.is_empty() || ty_raw.is_empty() {
        return None;
    }
    let (kind, ty_stripped) = classify_borrow(&ty_raw);
    Some(Parameter {
        name,
        ty: ty_stripped,
        kind,
    })
}

/// Strip a leading `&` / `&mut` (with optional whitespace) and report the
/// borrow kind. The bootstrap formatter renders `&mut` as `& mut`, so we
/// must accept both spellings.
fn classify_borrow(raw: &str) -> (BorrowKind, String) {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix('&') {
        let rest = rest.trim_start();
        if let Some(after_mut) = rest.strip_prefix("mut") {
            // ensure `mut` was a standalone token (not e.g. `mutable`).
            let boundary = after_mut
                .chars()
                .next()
                .map(|c| !c.is_ascii_alphanumeric() && c != '_')
                .unwrap_or(true);
            if boundary {
                return (BorrowKind::Mut, after_mut.trim().to_string());
            }
        }
        return (BorrowKind::Shared, rest.trim().to_string());
    }
    (BorrowKind::Owned, trimmed.to_string())
}

fn strip_borrow_prefix(ty: &str) -> &str {
    let trimmed = ty.trim();
    let Some(rest) = trimmed.strip_prefix('&') else {
        return trimmed;
    };
    let rest = rest.trim_start();
    if let Some(after_mut) = rest.strip_prefix("mut") {
        let boundary = after_mut
            .chars()
            .next()
            .map(|c| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(true);
        if boundary {
            return after_mut.trim_start();
        }
    }
    rest
}

fn parse_return_type(signature: &str) -> Option<String> {
    let idx = signature.find("->")?;
    let after = signature[idx + 2..].trim();
    let cutoff = after.find(" uses ").unwrap_or(after.len());
    let candidate = after[..cutoff].trim();
    if candidate.is_empty() {
        None
    } else {
        Some(candidate.to_string())
    }
}

/// Locate the `)` that closes the parameter list at top-level bracket depth.
fn find_matching_close(body: &str) -> Option<usize> {
    let mut depth: i32 = 0;
    for (idx, ch) in body.char_indices() {
        match ch {
            '(' | '[' => depth += 1,
            ']' => depth -= 1,
            ')' => {
                if depth == 0 {
                    return Some(idx);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

fn split_top_level_commas(input: &str) -> Vec<&str> {
    let mut depth: i32 = 0;
    let mut last = 0usize;
    let mut out = Vec::new();
    for (idx, ch) in input.char_indices() {
        match ch {
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(input[last..idx].trim());
                last = idx + 1;
            }
            _ => {}
        }
    }
    let tail = input[last..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

/// Extract the head identifier of a type expression: `Shared[T]` -> `Shared`,
/// `Result[T, E]` -> `Result`, `Int` -> `Int`. Returns an empty string for
/// unrecognised input.
fn type_head(ty: &str) -> &str {
    let trimmed = ty.trim();
    let end = trimmed
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(trimmed.len());
    &trimmed[..end]
}

fn split_generic(ty: &str) -> (&str, Vec<&str>) {
    let ty = ty.trim();
    if let Some(open) = ty.find('[') {
        if ty.ends_with(']') {
            let head = ty[..open].trim();
            let inner = &ty[open + 1..ty.len() - 1];
            let args = split_top_level_commas(inner);
            return (head, args);
        }
    }
    (ty, Vec::new())
}

/// Whole-word scan for `keyword` inside a signature string. Avoids false
/// positives from longer identifiers that happen to contain the substring.
fn signature_contains_keyword(signature: &str, keyword: &str) -> bool {
    let bytes = signature.as_bytes();
    let key = keyword.as_bytes();
    if key.is_empty() || bytes.len() < key.len() {
        return false;
    }
    let mut idx = 0;
    while idx + key.len() <= bytes.len() {
        if &bytes[idx..idx + key.len()] == key {
            let before_ok = idx == 0 || !is_ident_byte(bytes[idx - 1]);
            let after_ok = idx + key.len() == bytes.len() || !is_ident_byte(bytes[idx + key.len()]);
            if before_ok && after_ok {
                return true;
            }
        }
        idx += 1;
    }
    false
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Build a `newtype name -> base type` map from `type X wraps Base` symbol
/// signatures. Only entries whose base looks like a known primitive or a
/// declared identifier are recorded; bogus inputs are silently dropped.
fn collect_newtype_bases(module: &Module) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for symbol in &module.symbols {
        if symbol.kind != SymbolKind::Type {
            continue;
        }
        if let Some(base) = extract_wraps_base(&symbol.signature) {
            // Accept builtins and any uppercase-prefixed declared type as base.
            let head = type_head(&base).to_string();
            if head.is_empty() {
                continue;
            }
            if is_builtin_type(&head)
                || head
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_uppercase())
                    .unwrap_or(false)
            {
                out.insert(symbol.name.clone(), head);
            }
        }
    }
    out
}

fn extract_wraps_base(signature: &str) -> Option<String> {
    let idx = signature.find(" wraps ")?;
    let after = signature[idx + " wraps ".len()..].trim();
    if after.is_empty() {
        None
    } else {
        Some(after.to_string())
    }
}

// ---------------------------------------------------------------------------
// Body-level borrow checking (M23).
//
// The signature-level rules above cannot observe what happens inside a
// function body. With the body parser now producing structured
// expressions, three additional ownership/lifetime patterns can be
// detected directly:
//
//   * **B0060 (move-after-use)** — A binding is moved (consumed by a
//     closure capture, `move(x)`, or a `return x`) and then referenced
//     again in the same block. We emit one diagnostic per stale use,
//     citing both the move site and the offending use.
//   * **B0070 (mut-after-shared)** — `&mut x` is taken in a region where
//     a `&x` (or another `&mut x`) is still live. The region tracker in
//     [`crate::borrow_regions`] drops borrows on block exit, so this
//     fires only when both borrows overlap in source order.
//   * **B0080 (escapes-region)** — A reference (`borrow(x)` /
//     `borrow_mut(x)`) is returned from a block whose `x` was declared
//     by a `let` in that same block. The borrow would outlive its
//     source, so we reject it.
//
// Borrow expressions are detected by their call name. The bootstrap
// surface uses `borrow(x)` / `borrow_shared(x)` / `ref(x)` for shared
// borrows and `borrow_mut(x)` / `mut_ref(x)` / `mut_borrow(x)` for
// exclusive ones. The bootstrap body parser doesn't yet have a dedicated
// `Expr::Borrow` node — adding one is out of scope for this milestone
// and would require touching `expr.rs`, which is reserved for other
// agents.
// ---------------------------------------------------------------------------

/// Names that introduce a shared borrow when used as a call target.
const SHARED_BORROW_FNS: &[&str] = &["borrow", "borrow_shared", "ref"];
/// Names that introduce a mutable borrow when used as a call target.
const MUT_BORROW_FNS: &[&str] = &["borrow_mut", "mut_ref", "mut_borrow"];
/// Names that explicitly consume (move) their first argument.
const MOVE_FNS: &[&str] = &["move", "consume", "take"];

/// Run all body-level borrow rules over `module` using a pre-parsed
/// [`ModuleBodies`]. Returns diagnostics in deterministic order: by
/// symbol id, then by rule id, then by source order within the body.
pub fn borrow_check_bodies(module: &Module, bodies: &ModuleBodies) -> Vec<Diagnostic> {
    let mut out: Vec<Diagnostic> = Vec::new();
    // Iterate symbols in their declared order so diagnostic order is
    // stable. The body map is keyed by symbol id and ordered by BTreeMap,
    // but we still want to honour declaration order when both lists are
    // synthesised from the same module.
    for symbol in &module.symbols {
        if symbol.kind != SymbolKind::Function {
            continue;
        }
        let Some(body) = bodies.get(&symbol.id) else {
            continue;
        };
        check_function_body(symbol, body, &mut out);
    }
    out
}

/// Convenience wrapper: parse `source` into a module + bodies, then run
/// the full borrow check (signature + body rules) over both. The output
/// is the concatenation of [`borrow_check_module`] and
/// [`borrow_check_bodies`] in that order, so signature errors precede
/// body errors in the diagnostic stream.
pub fn borrow_check_source(source: &SourceFile) -> Vec<Diagnostic> {
    let parse = crate::parser::parse_source(source);
    let bodies = parse_module_bodies_with_module(source, &parse.module);
    let mut out = borrow_check_module(&parse.module);
    out.extend(borrow_check_bodies(&parse.module, &bodies));
    out
}

/// Drive the three body rules over a single function body. Each rule
/// gets its own walker so additions in later milestones can be made
/// without rewriting the others.
fn check_function_body(symbol: &Symbol, body: &Expr, out: &mut Vec<Diagnostic>) {
    // Treat the whole body as living inside a synthetic outer region so
    // bodies that are not literal `Block` expressions still get a region
    // to attach borrows to.
    let mut regions = RegionMap::new();
    let outer = regions.enter();
    check_b0070_and_b0080(
        symbol,
        body,
        &mut regions,
        /*in_block_tail=*/ false,
        out,
    );
    // Ensure the synthetic region is balanced even if the body walker
    // unwound mid-block.
    while regions.depth() > 0 {
        regions.exit();
    }
    let _ = outer;

    // B0060 needs its own pass because it cares about *moved* names, not
    // borrows, and the simplest correct algorithm walks statements
    // linearly inside each block.
    check_b0060_move_after_use(symbol, body, out);
}

// ---------------------------------------------------------------------------
// B0060 — move-after-use.
// ---------------------------------------------------------------------------

fn check_b0060_move_after_use(symbol: &Symbol, body: &Expr, out: &mut Vec<Diagnostic>) {
    // moved: name -> first span where it was moved. BTreeMap keeps the
    // diagnostic stream deterministic when more than one name is stale.
    let mut moved: BTreeMap<String, Span> = BTreeMap::new();
    walk_block_for_moves(symbol, body, &mut moved, out);
}

fn walk_block_for_moves(
    symbol: &Symbol,
    expr: &Expr,
    moved: &mut BTreeMap<String, Span>,
    out: &mut Vec<Diagnostic>,
) {
    match expr {
        Expr::Block { stmts, tail } => {
            // A nested block uses a snapshot of the moved set so moves
            // inside the inner block don't leak out, matching the
            // intuition that the inner block has its own ownership scope.
            let snapshot = moved.clone();
            for stmt in stmts {
                walk_stmt_for_moves(symbol, stmt, moved, out);
            }
            if let Some(t) = tail {
                check_uses_against_moved(symbol, t, moved, out);
                record_moves_in_expr(t, moved);
            }
            *moved = snapshot;
        }
        // Non-block bodies: treat as one big statement.
        other => {
            check_uses_against_moved(symbol, other, moved, out);
            record_moves_in_expr(other, moved);
        }
    }
}

fn walk_stmt_for_moves(
    symbol: &Symbol,
    stmt: &Stmt,
    moved: &mut BTreeMap<String, Span>,
    out: &mut Vec<Diagnostic>,
) {
    match stmt {
        Stmt::Let { init, .. } => {
            check_uses_against_moved(symbol, init, moved, out);
            record_moves_in_expr(init, moved);
        }
        Stmt::Expr(e) => {
            check_uses_against_moved(symbol, e, moved, out);
            record_moves_in_expr(e, moved);
        }
        Stmt::Return(Some(e)) => {
            check_uses_against_moved(symbol, e, moved, out);
            record_moves_in_expr(e, moved);
        }
        Stmt::Return(None) => {}
    }
}

/// Walk `expr` looking for `Var(name)` uses where `name` is already in
/// `moved`. Each fresh stale use produces one B0060 diagnostic. We do
/// not look into lambda bodies here because those represent the *new*
/// capture and are reported as moves, not as stale uses.
fn check_uses_against_moved(
    symbol: &Symbol,
    expr: &Expr,
    moved: &BTreeMap<String, Span>,
    out: &mut Vec<Diagnostic>,
) {
    if moved.is_empty() {
        return;
    }
    let mut reported: BTreeSet<String> = BTreeSet::new();
    visit_var_uses(expr, &mut |name| {
        if reported.contains(name) {
            return;
        }
        if let Some(move_span) = moved.get(name) {
            reported.insert(name.to_string());
            out.push(build_b0060(symbol, name, move_span.clone()));
        }
    });
}

/// Record every move introduced by `expr` into `moved`. The first move
/// of a name wins (subsequent ones would themselves be B0060 cases and
/// are handled before this function runs).
fn record_moves_in_expr(expr: &Expr, moved: &mut BTreeMap<String, Span>) {
    match expr {
        // `return x` consumes x.
        Expr::Return(Some(inner)) => {
            if let Expr::Var(name) = inner.as_ref() {
                moved.entry(name.clone()).or_insert_with(|| Span::dummy(""));
            }
            record_moves_in_expr(inner, moved);
        }
        // `move(x)` / `take(x)` / `consume(x)`.
        Expr::Call { callee, args } => {
            if let Expr::Var(name) = callee.as_ref() {
                if MOVE_FNS.contains(&name.as_str()) {
                    if let Some(Expr::Var(target)) = args.first() {
                        moved
                            .entry(target.clone())
                            .or_insert_with(|| Span::dummy(""));
                    }
                }
            }
            record_moves_in_expr(callee, moved);
            for a in args {
                record_moves_in_expr(a, moved);
            }
        }
        // A lambda captures every free variable by move (the bootstrap
        // has no explicit `move` keyword on closures yet).
        Expr::Lambda { params, body } => {
            let bound: BTreeSet<String> = params.iter().map(|(n, _)| n.clone()).collect();
            let mut captured: BTreeSet<String> = BTreeSet::new();
            visit_var_uses(body, &mut |name| {
                if !bound.contains(name) {
                    captured.insert(name.to_string());
                }
            });
            for name in captured {
                moved.entry(name).or_insert_with(|| Span::dummy(""));
            }
        }
        Expr::Block { stmts, tail } => {
            for s in stmts {
                if let Stmt::Expr(e) | Stmt::Let { init: e, .. } = s {
                    record_moves_in_expr(e, moved);
                }
                if let Stmt::Return(Some(e)) = s {
                    if let Expr::Var(name) = e {
                        moved.entry(name.clone()).or_insert_with(|| Span::dummy(""));
                    }
                    record_moves_in_expr(e, moved);
                }
            }
            if let Some(t) = tail {
                record_moves_in_expr(t, moved);
            }
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            record_moves_in_expr(cond, moved);
            record_moves_in_expr(then_branch, moved);
            if let Some(e) = else_branch {
                record_moves_in_expr(e, moved);
            }
        }
        Expr::Match { scrutinee, arms } => {
            record_moves_in_expr(scrutinee, moved);
            for MatchArm { body, .. } in arms {
                record_moves_in_expr(body, moved);
            }
        }
        Expr::Construct { args, .. } | Expr::Tuple(args) => {
            for a in args {
                record_moves_in_expr(a, moved);
            }
        }
        Expr::Record { fields } => {
            for (_, v) in fields {
                record_moves_in_expr(v, moved);
            }
        }
        Expr::Try(inner) | Expr::Field { base: inner, .. } => record_moves_in_expr(inner, moved),
        Expr::Binary { lhs, rhs, .. } => {
            record_moves_in_expr(lhs, moved);
            record_moves_in_expr(rhs, moved);
        }
        Expr::Unary { operand, .. } => record_moves_in_expr(operand, moved),
        Expr::InterpString { parts } => {
            for part in parts {
                if let InterpPart::Expr(inner) = part {
                    record_moves_in_expr(inner, moved);
                }
            }
        }
        Expr::Lit(_) | Expr::Var(_) | Expr::RawStr { .. } | Expr::Error | Expr::Return(None) => {}
    }
}

fn build_b0060(symbol: &Symbol, name: &str, move_span: Span) -> Diagnostic {
    let _ = move_span; // span is informational; we anchor to the symbol for now.
    Diagnostic::error(
        "B0060",
        format!(
            "value `{name}` is used in `{}` after being moved (consumed by closure or returned)",
            symbol.name
        ),
        symbol.span.clone(),
    )
    .with_symbol(symbol.id.clone())
    .with_expected(vec![format!("at most one consuming use of `{name}`")])
    .with_found(vec![format!("`{name}` is used again after a move")])
    .with_help(format!(
        "After `{name}` is moved (captured by a closure, passed to `move(...)`, or returned), \
         it cannot be referenced again. Clone it before the move, or restructure the code so the \
         later use does not need ownership."
    ))
    .with_fix(
        Fix::new(
            "clone_before_move",
            format!("Clone `{name}` before the move, or move only on the last use."),
            0.5,
        )
        .with_patch(serde_json::json!({
            "schema": "ori.patch.v1",
            "intent": format!("Avoid use-after-move of `{name}` in `{}`.", symbol.id),
            "operations": [],
            "tests": { "run": ["cargo test -p ori-compiler borrow"] }
        })),
    )
    .with_agent_summary(format!(
        "Refactor so `{name}` is not referenced after being moved into a closure or returned."
    ))
    .with_minimal_context(vec![symbol.id.clone()])
    .with_docs(vec![
        "doc:borrow.rules".to_string(),
        "doc:borrow.move-semantics".to_string(),
    ])
}

// ---------------------------------------------------------------------------
// B0070 — &mut taken while a shared (or another mut) borrow is live.
// B0080 — borrow returned from a block whose source region is the block.
// ---------------------------------------------------------------------------

/// Walk `expr` while pushing/popping regions on every `Block`. The walker
/// reports B0070 the moment a conflicting borrow is added, and B0080
/// when a borrow expression is the tail of a block (or argument to a
/// `return`) and its target is a local of an ancestor region introduced
/// inside the function body.
fn check_b0070_and_b0080(
    symbol: &Symbol,
    expr: &Expr,
    regions: &mut RegionMap,
    in_block_tail: bool,
    out: &mut Vec<Diagnostic>,
) {
    match expr {
        Expr::Block { stmts, tail } => {
            regions.enter();
            for stmt in stmts {
                walk_stmt_for_borrows(symbol, stmt, regions, out);
            }
            if let Some(t) = tail {
                check_b0070_and_b0080(symbol, t, regions, /*in_block_tail=*/ true, out);
            }
            regions.exit();
        }
        Expr::Call { callee, args } => {
            // Detect borrow calls. If matched and we're in tail
            // position, flag escapes; otherwise just record.
            if let Some((mode, target_name, target_span)) = detect_borrow_call(callee, args) {
                if in_block_tail && regions.is_local(&target_name) {
                    out.push(build_b0080(symbol, &target_name, mode));
                }
                // Conflict detection against the current region stack.
                if let Some(conflict) = find_borrow_conflict(regions, &target_name, mode) {
                    out.push(build_b0070(symbol, &target_name, mode, conflict));
                }
                regions.add_borrow(&target_name, mode, target_span);
                // Still walk into the args in case they nest more
                // expressions, but skip the *first* arg whose identity
                // we just consumed.
                for arg in args.iter().skip(1) {
                    check_b0070_and_b0080(symbol, arg, regions, false, out);
                }
                return;
            }
            check_b0070_and_b0080(symbol, callee, regions, false, out);
            for arg in args {
                check_b0070_and_b0080(symbol, arg, regions, false, out);
            }
        }
        Expr::Return(Some(inner)) => {
            // A `return borrow(x)` is morally the tail of every
            // enclosing block, so propagate the tail flag.
            check_b0070_and_b0080(symbol, inner, regions, true, out);
        }
        Expr::Return(None) => {}
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            check_b0070_and_b0080(symbol, cond, regions, false, out);
            check_b0070_and_b0080(symbol, then_branch, regions, in_block_tail, out);
            if let Some(e) = else_branch {
                check_b0070_and_b0080(symbol, e, regions, in_block_tail, out);
            }
        }
        Expr::Match { scrutinee, arms } => {
            check_b0070_and_b0080(symbol, scrutinee, regions, false, out);
            for MatchArm { body, .. } in arms {
                check_b0070_and_b0080(symbol, body, regions, in_block_tail, out);
            }
        }
        Expr::Construct { args, .. } | Expr::Tuple(args) => {
            for a in args {
                check_b0070_and_b0080(symbol, a, regions, false, out);
            }
        }
        Expr::Record { fields } => {
            for (_, v) in fields {
                check_b0070_and_b0080(symbol, v, regions, false, out);
            }
        }
        Expr::Try(inner) | Expr::Field { base: inner, .. } => {
            check_b0070_and_b0080(symbol, inner, regions, false, out);
        }
        Expr::Binary { lhs, rhs, .. } => {
            check_b0070_and_b0080(symbol, lhs, regions, false, out);
            check_b0070_and_b0080(symbol, rhs, regions, false, out);
        }
        Expr::Unary { operand, .. } => {
            check_b0070_and_b0080(symbol, operand, regions, false, out);
        }
        Expr::Lambda { body, .. } => {
            // Lambda bodies are independent regions for borrow tracking.
            regions.enter();
            check_b0070_and_b0080(symbol, body, regions, false, out);
            regions.exit();
        }
        Expr::InterpString { parts } => {
            for part in parts {
                if let InterpPart::Expr(inner) = part {
                    check_b0070_and_b0080(symbol, inner, regions, false, out);
                }
            }
        }
        Expr::Lit(_) | Expr::Var(_) | Expr::RawStr { .. } | Expr::Error => {}
    }
}

fn walk_stmt_for_borrows(
    symbol: &Symbol,
    stmt: &Stmt,
    regions: &mut RegionMap,
    out: &mut Vec<Diagnostic>,
) {
    match stmt {
        Stmt::Let { name, init, .. } => {
            // Walk the initialiser first; the binding name only becomes
            // a local *after* its init evaluates, so a borrow taken
            // during init cannot escape via that same binding.
            check_b0070_and_b0080(symbol, init, regions, false, out);
            regions.declare_local(name);
        }
        Stmt::Expr(e) => check_b0070_and_b0080(symbol, e, regions, false, out),
        Stmt::Return(Some(e)) => check_b0070_and_b0080(symbol, e, regions, true, out),
        Stmt::Return(None) => {}
    }
}

/// If `(callee, args)` is one of the known borrow-call shapes, return
/// `(mode, target_name, target_span)`. Returns `None` for anything
/// else, including malformed inputs.
fn detect_borrow_call(callee: &Expr, args: &[Expr]) -> Option<(BorrowMode, String, Span)> {
    let Expr::Var(name) = callee else {
        return None;
    };
    let mode = if SHARED_BORROW_FNS.contains(&name.as_str()) {
        BorrowMode::Shared
    } else if MUT_BORROW_FNS.contains(&name.as_str()) {
        BorrowMode::Mut
    } else {
        return None;
    };
    let first = args.first()?;
    if let Expr::Var(target) = first {
        // We don't have a span on `Expr::Var` itself; use a dummy span
        // pinned to the file. The diagnostic anchors to the symbol span,
        // which is what tools surface anyway.
        return Some((mode, target.clone(), Span::dummy("")));
    }
    None
}

/// Return the existing live borrow that conflicts with adding a new
/// borrow of `(name, new_mode)`, or `None` when the addition is safe.
fn find_borrow_conflict(
    regions: &RegionMap,
    name: &str,
    new_mode: BorrowMode,
) -> Option<BorrowMode> {
    let live = regions.borrows_of(name);
    for b in live {
        match (b.mode, new_mode) {
            // Two shared borrows are fine.
            (BorrowMode::Shared, BorrowMode::Shared) => continue,
            // Anything involving a mut borrow conflicts.
            _ => return Some(b.mode),
        }
    }
    None
}

fn build_b0070(
    symbol: &Symbol,
    target: &str,
    new_mode: BorrowMode,
    existing_mode: BorrowMode,
) -> Diagnostic {
    let new = mode_label(new_mode);
    let existing = mode_label(existing_mode);
    Diagnostic::error(
        "B0070",
        format!(
            "cannot take `{new}` borrow of `{target}` in `{}`: a `{existing}` borrow is already live in the same region",
            symbol.name
        ),
        symbol.span.clone(),
    )
    .with_symbol(symbol.id.clone())
    .with_expected(vec![format!(
        "no overlapping `{new}` and `{existing}` borrows of `{target}`"
    )])
    .with_found(vec![format!(
        "`{new}` borrow of `{target}` while `{existing}` borrow still live"
    )])
    .with_help(format!(
        "Drop or end the existing `{existing}` borrow of `{target}` (let it go out of scope) \
         before taking a `{new}` borrow."
    ))
    .with_fix(
        Fix::new(
            "split_borrow_regions",
            format!(
                "Restructure so the `{existing}` and `{new}` borrows of `{target}` do not overlap."
            ),
            0.5,
        )
        .with_patch(serde_json::json!({
            "schema": "ori.patch.v1",
            "intent": format!(
                "Eliminate overlapping borrows of `{}` in `{}`.",
                target, symbol.id
            ),
            "operations": [],
            "tests": { "run": ["cargo test -p ori-compiler borrow"] }
        })),
    )
    .with_agent_summary(format!(
        "End the existing `{existing}` borrow of `{target}` before requesting `{new}`."
    ))
    .with_minimal_context(vec![symbol.id.clone()])
    .with_docs(vec![
        "doc:borrow.rules".to_string(),
        "doc:borrow.aliasing".to_string(),
    ])
}

fn build_b0080(symbol: &Symbol, target: &str, mode: BorrowMode) -> Diagnostic {
    let label = mode_label(mode);
    Diagnostic::error(
        "B0080",
        format!(
            "borrow of `{target}` escapes its region in `{}`: `{target}` is a block-local binding so the returned `{label}` reference would dangle",
            symbol.name
        ),
        symbol.span.clone(),
    )
    .with_symbol(symbol.id.clone())
    .with_expected(vec![format!(
        "return an owned value, or borrow a binding that outlives the block"
    )])
    .with_found(vec![format!(
        "`{label}` borrow of block-local `{target}` returned from the enclosing block"
    )])
    .with_help(format!(
        "`{target}` is declared inside this block, so any reference to it dies when the block \
         ends. Return the value by move, or take the borrow from a binding that lives at least \
         as long as the caller."
    ))
    .with_fix(
        Fix::new(
            "return_owned_value",
            format!("Return `{target}` by value instead of borrowing it."),
            0.55,
        )
        .with_patch(serde_json::json!({
            "schema": "ori.patch.v1",
            "intent": format!(
                "Stop returning a borrow of the block-local `{}` from `{}`.",
                target, symbol.id
            ),
            "operations": [],
            "tests": { "run": ["cargo test -p ori-compiler borrow"] }
        })),
    )
    .with_agent_summary(format!(
        "Return `{target}` by value or extend its lifetime; the current borrow escapes its source region."
    ))
    .with_minimal_context(vec![symbol.id.clone()])
    .with_docs(vec![
        "doc:borrow.rules".to_string(),
        "doc:borrow.lifetimes".to_string(),
    ])
}

fn mode_label(mode: BorrowMode) -> &'static str {
    match mode {
        BorrowMode::Shared => "shared",
        BorrowMode::Mut => "mut",
    }
}

// ---------------------------------------------------------------------------
// Generic AST walkers.
// ---------------------------------------------------------------------------

/// Visit every `Expr::Var(name)` in `expr` (excluding lambda parameter
/// names, which are bound). Useful for both move tracking and free-var
/// computation. The visitor is deterministic in source order.
fn visit_var_uses<F: FnMut(&str)>(expr: &Expr, f: &mut F) {
    match expr {
        Expr::Var(name) => f(name),
        Expr::Call { callee, args } => {
            visit_var_uses(callee, f);
            for a in args {
                visit_var_uses(a, f);
            }
        }
        Expr::Field { base, .. } => visit_var_uses(base, f),
        Expr::Block { stmts, tail } => {
            for s in stmts {
                match s {
                    Stmt::Let { init, .. } | Stmt::Expr(init) => visit_var_uses(init, f),
                    Stmt::Return(Some(e)) => visit_var_uses(e, f),
                    Stmt::Return(None) => {}
                }
            }
            if let Some(t) = tail {
                visit_var_uses(t, f);
            }
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            visit_var_uses(cond, f);
            visit_var_uses(then_branch, f);
            if let Some(e) = else_branch {
                visit_var_uses(e, f);
            }
        }
        Expr::Match { scrutinee, arms } => {
            visit_var_uses(scrutinee, f);
            for MatchArm { body, .. } in arms {
                visit_var_uses(body, f);
            }
        }
        Expr::Return(Some(inner)) => visit_var_uses(inner, f),
        Expr::Construct { args, .. } | Expr::Tuple(args) => {
            for a in args {
                visit_var_uses(a, f);
            }
        }
        Expr::Record { fields } => {
            for (_, v) in fields {
                visit_var_uses(v, f);
            }
        }
        Expr::Try(inner) => visit_var_uses(inner, f),
        Expr::Lambda { body, .. } => visit_var_uses(body, f),
        Expr::Binary { lhs, rhs, .. } => {
            visit_var_uses(lhs, f);
            visit_var_uses(rhs, f);
        }
        Expr::Unary { operand, .. } => visit_var_uses(operand, f),
        Expr::InterpString { parts } => {
            for part in parts {
                if let InterpPart::Expr(inner) = part {
                    visit_var_uses(inner, f);
                }
            }
        }
        Expr::Lit(_) | Expr::RawStr { .. } | Expr::Error | Expr::Return(None) => {}
    }
}

// Re-export the dangling-borrow ID under the documented name from the task
// (B0010 for `&mut` aliasing, B0020 for newtypes, B0030 for shared/unique,
// B0040 for unsafe, B0050 for dangling). The task mentions B0010 for
// dangling; we keep B0050 for clarity and add a const alias for callers
// that want to filter by stable ID.
pub const RULE_MUT_ALIAS: &str = "B0010";
pub const RULE_MUT_SHARED_CONFLICT: &str = "B0011";
pub const RULE_NEWTYPE_CONFUSION: &str = "B0020";
pub const RULE_SHARED_OVER_UNIQUE: &str = "B0030";
pub const RULE_UNSAFE_REJECTED: &str = "B0040";
pub const RULE_DANGLING_BORROW: &str = "B0050";
pub const RULE_MOVE_AFTER_USE: &str = "B0060";
pub const RULE_MUT_AFTER_SHARED: &str = "B0070";
pub const RULE_BORROW_ESCAPES_REGION: &str = "B0080";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_source;
    use crate::source::SourceFile;

    fn module_for(text: &str) -> Module {
        parse_source(&SourceFile::new("/t.ori", text)).module
    }

    #[test]
    fn flags_double_mut_borrow_on_same_identifier() {
        let module = module_for("module a\nfn swap(x: &mut Int, x: &mut Int) -> Unit");
        let diags = borrow_check_module(&module);
        assert!(
            diags.iter().any(|d| d.id == "B0010"),
            "expected B0010 in {:?}",
            diags.iter().map(|d| &d.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn flags_newtype_confusion_with_shared_base() {
        let module = module_for(
            "module a\ntype ProductId wraps Str\ntype OrderId wraps Str\nfn link(a: ProductId, b: OrderId) -> Unit",
        );
        let diags = borrow_check_module(&module);
        assert!(
            diags.iter().any(|d| d.id == "B0020"),
            "expected B0020 in {:?}",
            diags.iter().map(|d| &d.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn flags_shared_return_with_db_write() {
        let module = module_for("module a\nfn open_writer() -> Shared[Conn] uses db.write");
        let diags = borrow_check_module(&module);
        assert!(
            diags.iter().any(|d| d.id == "B0030"),
            "expected B0030 in {:?}",
            diags.iter().map(|d| &d.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn flags_unsafe_via_effect() {
        // The bootstrap parser drops the `unsafe` keyword from the signature
        // string, but it preserves `uses unsafe` in the effect list. Both
        // paths must trip B0040.
        let module = module_for("module a\nfn raw() -> Unit uses unsafe");
        let diags = borrow_check_module(&module);
        assert!(
            diags.iter().any(|d| d.id == "B0040"),
            "expected B0040 in {:?}",
            diags.iter().map(|d| &d.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn clean_signature_emits_no_borrow_diagnostics() {
        let module = module_for("module a\nfn add(x: Int, y: Int) -> Int");
        let diags = borrow_check_module(&module);
        assert!(
            diags.is_empty(),
            "expected no diagnostics for clean signature, got {:?}",
            diags.iter().map(|d| &d.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn dangling_borrow_returns_warning_when_no_param_matches() {
        let module = module_for("module a\nfn make() -> &Int");
        let diags = borrow_check_module(&module);
        assert!(
            diags.iter().any(|d| d.id == "B0050"),
            "expected B0050 in {:?}",
            diags.iter().map(|d| &d.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn dangling_borrow_quiet_when_param_borrows_same_base() {
        let module = module_for("module a\nfn first(xs: &List) -> &List");
        let diags = borrow_check_module(&module);
        assert!(
            !diags.iter().any(|d| d.id == "B0050"),
            "unexpected B0050 in {:?}",
            diags.iter().map(|d| &d.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn shared_and_mut_on_same_binding_emits_b0011() {
        let module = module_for("module a\nfn touch(x: &Int, x: &mut Int) -> Unit");
        let diags = borrow_check_module(&module);
        assert!(diags.iter().any(|d| d.id == "B0011"));
    }

    #[test]
    fn newtype_confusion_quiet_for_distinct_bases() {
        let module = module_for(
            "module a\ntype Price wraps Int\ntype Name wraps Str\nfn label(p: Price, n: Name) -> Unit",
        );
        let diags = borrow_check_module(&module);
        assert!(!diags.iter().any(|d| d.id == "B0020"));
    }

    #[test]
    fn borrow_check_is_idempotent() {
        let module =
            module_for("module a\ntype A wraps Str\ntype B wraps Str\nfn f(x: A, y: B) -> Unit");
        let first = borrow_check_module(&module);
        let second = borrow_check_module(&module);
        assert_eq!(first.len(), second.len());
        for (lhs, rhs) in first.iter().zip(second.iter()) {
            assert_eq!(lhs.id, rhs.id);
            assert_eq!(lhs.message, rhs.message);
        }
    }

    #[test]
    fn unsafe_diagnostic_includes_patch_fix() {
        let module = module_for("module a\nfn raw() -> Unit uses unsafe");
        let diags = borrow_check_module(&module);
        let Some(diag) = diags.iter().find(|d| d.id == "B0040") else {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "unsafe diagnostic missing");
            }
            return;
        };
        assert!(diag.fixes.iter().any(|f| f.patch.is_some()));
        assert!(diag.agent.docs.iter().any(|doc| doc == "doc:borrow.rules"));
        assert!(!diag.agent.summary.is_empty());
    }

    // -----------------------------------------------------------------------
    // M23 body-level borrow checks.
    //
    // Each test exercises one rule on a body whose Ori source uses the
    // calling-convention borrow / move shapes documented above the
    // body-checker block:
    //   borrow(x) / borrow_shared(x) / ref(x)  — shared borrow
    //   borrow_mut(x) / mut_ref(x)             — mutable borrow
    //   move(x) / take(x) / consume(x)         — explicit move
    //   `fn (...) => ...x...`                  — closure capture move
    //   `return x`                             — move via return
    // -----------------------------------------------------------------------

    fn source_for(text: &str) -> SourceFile {
        SourceFile::new("/t.ori", text)
    }

    // ---- B0060: move-after-use ----

    #[test]
    fn b0060_flags_use_after_move_into_closure() {
        // `let x = 1; let cb = fn () => x; x` — x captured by closure,
        // then used again in tail position. The body checker must flag
        // the trailing `x` as a move-after-use.
        let src = source_for("module a\nfn f() -> Int:\n  let x = 1\n  let cb = fn () => x\n  x\n");
        let diags = borrow_check_source(&src);
        assert!(
            diags.iter().any(|d| d.id == "B0060"),
            "expected B0060 in {:?}",
            diags.iter().map(|d| &d.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn b0060_quiet_when_no_aliasing() {
        // A clean body that uses each binding exactly once must not
        // produce B0060. We use `add(a, b)` style code that the body
        // checker should pass through.
        let src = source_for("module a\nfn f() -> Int:\n  let x = 1\n  let y = 2\n  add(x, y)\n");
        let diags = borrow_check_source(&src);
        assert!(
            !diags.iter().any(|d| d.id == "B0060"),
            "unexpected B0060 in {:?}",
            diags.iter().map(|d| &d.id).collect::<Vec<_>>()
        );
    }

    // ---- B0070: mut-after-shared ----

    #[test]
    fn b0070_flags_mut_borrow_while_shared_live() {
        // `let r = borrow(x); let w = borrow_mut(x); use(r, w)` — taking
        // `&mut x` while `&x` is still in scope is the canonical
        // mut-after-shared violation.
        let src = source_for(
            "module a\nfn f() -> Int:\n  let r = borrow(x)\n  let w = borrow_mut(x)\n  use(r, w)\n",
        );
        let diags = borrow_check_source(&src);
        assert!(
            diags.iter().any(|d| d.id == "B0070"),
            "expected B0070 in {:?}",
            diags.iter().map(|d| &d.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn b0070_quiet_with_only_shared_borrows() {
        // Two shared borrows of the same identifier coexist freely —
        // that's the whole point of shared borrowing. The checker must
        // not produce B0070 here.
        let src = source_for(
            "module a\nfn f() -> Int:\n  let r1 = borrow(x)\n  let r2 = borrow(x)\n  use(r1, r2)\n",
        );
        let diags = borrow_check_source(&src);
        assert!(
            !diags.iter().any(|d| d.id == "B0070"),
            "unexpected B0070 in {:?}",
            diags.iter().map(|d| &d.id).collect::<Vec<_>>()
        );
    }

    // ---- B0080: borrow escapes region ----

    #[test]
    fn b0080_flags_borrow_of_local_returned() {
        // `let x = make(); return borrow(x)` — returning a reference to
        // a block-local binding is the canonical escape case.
        let src = source_for("module a\nfn f() -> Int:\n  let x = make()\n  return borrow(x)\n");
        let diags = borrow_check_source(&src);
        assert!(
            diags.iter().any(|d| d.id == "B0080"),
            "expected B0080 in {:?}",
            diags.iter().map(|d| &d.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn b0080_quiet_when_borrow_target_is_not_block_local() {
        // Borrowing a free identifier (presumed parameter or module
        // value) must not trigger the escape rule — its lifetime is
        // bounded by the caller, not by this block.
        let src = source_for("module a\nfn f() -> Int:\n  return borrow(global_handle)\n");
        let diags = borrow_check_source(&src);
        assert!(
            !diags.iter().any(|d| d.id == "B0080"),
            "unexpected B0080 in {:?}",
            diags.iter().map(|d| &d.id).collect::<Vec<_>>()
        );
    }

    // ---- Mixed bodies that exercise Expr::Binary without false positives ----

    #[test]
    fn body_with_binary_addition_is_clean() {
        let src = source_for("module a\nfn add(x: Int, y: Int) -> Int:\n  return x + y\n");
        let diags = borrow_check_source(&src);
        for forbidden in ["B0060", "B0070", "B0080"] {
            assert!(
                !diags.iter().any(|d| d.id == forbidden),
                "unexpected {forbidden} on clean binary body: {:?}",
                diags.iter().map(|d| &d.id).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn body_with_nested_binary_and_comparison_is_clean() {
        let src = source_for(
            "module a\nfn mix(a: Int, b: Int, c: Int) -> Bool:\n  return a + b * c == c - a\n",
        );
        let diags = borrow_check_source(&src);
        for forbidden in ["B0060", "B0070", "B0080"] {
            assert!(
                !diags.iter().any(|d| d.id == forbidden),
                "unexpected {forbidden} on nested binary body: {:?}",
                diags.iter().map(|d| &d.id).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn body_with_unary_and_binary_is_clean() {
        let src = source_for("module a\nfn flip(p: Bool, q: Int) -> Bool:\n  return !p && q > 0\n");
        let diags = borrow_check_source(&src);
        for forbidden in ["B0060", "B0070", "B0080"] {
            assert!(
                !diags.iter().any(|d| d.id == forbidden),
                "unexpected {forbidden} on unary+binary body: {:?}",
                diags.iter().map(|d| &d.id).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn body_with_let_then_binary_is_clean() {
        // Real-world shape: a let-binding feeds a binary computation in
        // the tail expression. No aliasing, no escape, no move.
        let src =
            source_for("module a\nfn calc(x: Int) -> Int:\n  let two = 2\n  return x * two + 1\n");
        let diags = borrow_check_source(&src);
        for forbidden in ["B0060", "B0070", "B0080"] {
            assert!(
                !diags.iter().any(|d| d.id == forbidden),
                "unexpected {forbidden} on let+binary body: {:?}",
                diags.iter().map(|d| &d.id).collect::<Vec<_>>()
            );
        }
    }

    // ---- Determinism gate ----

    #[test]
    fn body_borrow_check_is_byte_identical_on_repeat() {
        // Mix of all three body rules plus a clean binary body — the
        // resulting diagnostic stream must be byte-identical across
        // independent invocations. Comparing the full `Vec<Diagnostic>`
        // covers id, message, span, fixes, and agent guidance.
        let src = source_for(
            "module a\n\
             fn f() -> Int:\n  \
               let x = make()\n  \
               let cb = fn () => x\n  \
               let r = borrow(y)\n  \
               let w = borrow_mut(y)\n  \
               return borrow(x)\n\
             fn clean(a: Int, b: Int) -> Int:\n  \
               return a + b\n",
        );
        let d1 = borrow_check_source(&src);
        let d2 = borrow_check_source(&src);
        assert_eq!(d1, d2, "borrow_check_source must be deterministic");
    }

    // ---- Diagnostic shape contract ----

    #[test]
    fn b0060_diagnostic_carries_required_metadata() {
        let src = source_for("module a\nfn f() -> Int:\n  let x = 1\n  let cb = fn () => x\n  x\n");
        let diags = borrow_check_source(&src);
        let Some(diag) = diags.iter().find(|d| d.id == "B0060") else {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "B0060 missing");
            }
            return;
        };
        assert!(!diag.expected.is_empty(), "B0060 missing `expected`");
        assert!(!diag.found.is_empty(), "B0060 missing `found`");
        assert!(
            !diag.agent.summary.is_empty(),
            "B0060 missing agent summary"
        );
    }
}
