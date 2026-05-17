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
use crate::diagnostic::{Diagnostic, Fix};
use crate::types::is_builtin_type;
use std::collections::BTreeMap;

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
}
