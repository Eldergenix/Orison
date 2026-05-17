//! Expression-level type inference for function bodies.
//!
//! This module is a deliberately small, additive layer that sits on top of
//! the body parser in [`crate::expr`] and the (signature-only) type checker
//! in [`crate::type_check`]. It walks each function body, threads a lexical
//! [`TypeEnv`] through `let` bindings and `if`/`match` branches, and infers
//! a [`TypeRef`] for every expression in the bootstrap subset of the
//! grammar.
//!
//! The inference is intentionally monotonic: when a sub-expression
//! produces [`TypeRef::Unknown`] (because it is a recovery node, an
//! unknown identifier, or otherwise unresolved) the parent **does not**
//! get polluted unless an actual conflict is detected. Conflicts are
//! reported as warnings — never errors — because the surrounding pipeline
//! is still proving out expression-level information and we do not want
//! these signals to gate the rest of the compiler.
//!
//! Diagnostic IDs in the `W0530`..`W0599` range belong to this module.
//!
//! Inference rules (kept tight on purpose):
//!
//! * `Lit(Int)` → `Int`
//! * `Lit(Float)` → `Float64`
//! * `Lit(Str)` → `Str`
//! * `Lit(Bool)` → `Bool`
//! * `Lit(Unit)` → `Unit`
//! * `Var(name)` → environment lookup; unknown ⇒ `Unknown` + `W0530`.
//! * `Block` → fold over statements (let binds into a child env),
//!   tail expression type or `Unit`.
//! * `Call(callee, args)` → if `callee` is `Var(name)`, look up the
//!   function symbol in the AST module and return its declared return
//!   type. Unknown callee ⇒ `Unknown` + `W0531`.
//! * `If(_, then, else)` → unify branches; mismatch ⇒ `W0541`.
//! * `Match(_, arms)` → unify all arm bodies; mismatch ⇒ `W0541`.
//! * `Return(Some(e))` → infer the inner expression and compare it
//!   against the declared return type (`W0540`).
//! * `Construct("Ok"|"Err"|"Some"|"None", args)` → produce
//!   `Result[_, _]` / `Option[_]` with the inferred argument type
//!   filling the relevant slot.
//! * `Try(e)` → if `e: Result[T, _]` return `T`; else `Unknown`.
//!
//! Everything outside this subset returns `Unknown` without diagnostics so
//! the inference pass is safe to run incrementally as the grammar grows.

use crate::ast::{Module, SymbolKind};
use crate::body::ModuleBodies;
use crate::diagnostic::Diagnostic;
use crate::expr::{Expr, Literal, MatchArm, Stmt};
use crate::source::Span;
use crate::types::TypeRef;
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Type environment
// ---------------------------------------------------------------------------

/// Lexical type environment used during expression inference. Each `Block`
/// pushes a child scope so that `let` bindings shadow outer ones without
/// mutating the surrounding environment.
///
/// Lookup is `O(depth)` which is fine for the bootstrap subset; nothing in
/// the grammar produces deeply-nested scopes today.
#[derive(Debug, Clone, Default)]
pub struct TypeEnv {
    pub parent: Option<Box<TypeEnv>>,
    pub bindings: BTreeMap<String, TypeRef>,
}

impl TypeEnv {
    /// A fresh, empty environment with no parent scope.
    pub fn new() -> Self {
        Self {
            parent: None,
            bindings: BTreeMap::new(),
        }
    }

    /// Create a child environment that delegates unresolved lookups to
    /// `parent`. Bindings introduced in the child are invisible to the
    /// parent on purpose.
    pub fn with_parent(parent: TypeEnv) -> Self {
        Self {
            parent: Some(Box::new(parent)),
            bindings: BTreeMap::new(),
        }
    }

    /// Resolve `name` walking parent scopes. Returns `None` when the name
    /// is not in scope anywhere.
    pub fn lookup(&self, name: &str) -> Option<TypeRef> {
        if let Some(ty) = self.bindings.get(name) {
            return Some(ty.clone());
        }
        match &self.parent {
            Some(parent) => parent.lookup(name),
            None => None,
        }
    }

    /// Introduce a binding in the current scope. Overwrites any prior
    /// binding for `name` at this level (so explicit shadowing inside the
    /// same block is allowed).
    pub fn bind(&mut self, name: impl Into<String>, ty: TypeRef) {
        self.bindings.insert(name.into(), ty);
    }
}

// ---------------------------------------------------------------------------
// Module-level entry point
// ---------------------------------------------------------------------------

/// Infer types for every function body in `module` and emit `W0540`
/// diagnostics whenever the inferred body type disagrees with the declared
/// return type.
///
/// This is intentionally a warning, not an error: the inference rules
/// implemented here cover the bootstrap subset only, so the parent
/// pipeline must remain green when a body's type cannot be fully proven.
pub fn check_module_bodies(module: &Module, bodies: &ModuleBodies) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let function_returns = collect_function_returns(module);

    for symbol in &module.symbols {
        if symbol.kind != SymbolKind::Function {
            continue;
        }
        let Some(body) = bodies.get(&symbol.id) else {
            continue;
        };

        let declared_return = parse_return_type(&symbol.signature);
        let mut env = TypeEnv::new();
        // Inject function parameters so identifiers referenced in the body
        // resolve to the right declared type.
        for (name, ty) in parse_parameters(&symbol.signature) {
            env.bind(name, ty);
        }

        let ctx = InferContext {
            function_returns: &function_returns,
            declared_return: declared_return.clone(),
            symbol_id: symbol.id.clone(),
            symbol_span: symbol.span.clone(),
        };

        let (inferred, mut diags) = infer_expr_with_ctx(body, &env, &ctx);
        diagnostics.append(&mut diags);

        if let Some(diag) =
            mismatch_with_declared_return(&declared_return, &inferred, &symbol.id, &symbol.span)
        {
            diagnostics.push(diag);
        }
    }

    diagnostics
}

// ---------------------------------------------------------------------------
// Stand-alone inference (matches the public API requested by the spec)
// ---------------------------------------------------------------------------

/// Infer the type of a single expression against `env`. Returns the
/// inferred [`TypeRef`] together with any diagnostics produced along the
/// way. Suitable for unit tests and ad-hoc callers that do not have a
/// surrounding module.
pub fn infer_expr(expr: &Expr, env: &TypeEnv) -> (TypeRef, Vec<Diagnostic>) {
    let empty: BTreeMap<String, TypeRef> = BTreeMap::new();
    let ctx = InferContext {
        function_returns: &empty,
        declared_return: TypeRef::Unknown,
        symbol_id: String::new(),
        symbol_span: Span::dummy("<expr>"),
    };
    infer_expr_with_ctx(expr, env, &ctx)
}

// ---------------------------------------------------------------------------
// Internal context + recursion
// ---------------------------------------------------------------------------

struct InferContext<'a> {
    /// Map from bare function name → declared return type, used to resolve
    /// `Call(Var(name), _)` without re-parsing signatures every time.
    function_returns: &'a BTreeMap<String, TypeRef>,
    /// Declared return type of the enclosing function, used by `Return`.
    declared_return: TypeRef,
    /// Symbol id of the enclosing function, attached to every diagnostic.
    symbol_id: String,
    /// Span of the enclosing function's name. Used as a stable fallback
    /// span for body-level diagnostics — the parser-produced expressions
    /// do not yet carry per-node spans.
    symbol_span: Span,
}

fn infer_expr_with_ctx(
    expr: &Expr,
    env: &TypeEnv,
    ctx: &InferContext<'_>,
) -> (TypeRef, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let ty = infer_expr_inner(expr, env, ctx, &mut diags);
    (ty, diags)
}

fn infer_expr_inner(
    expr: &Expr,
    env: &TypeEnv,
    ctx: &InferContext<'_>,
    diags: &mut Vec<Diagnostic>,
) -> TypeRef {
    match expr {
        Expr::Lit(lit) => infer_literal(lit),

        Expr::Var(name) => match env.lookup(name) {
            Some(ty) => ty,
            None => {
                diags.push(unknown_identifier_diag(name, ctx));
                TypeRef::Unknown
            }
        },

        Expr::Block { stmts, tail } => infer_block(stmts, tail.as_deref(), env, ctx, diags),

        Expr::Call { callee, args } => infer_call(callee, args, env, ctx, diags),

        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            // Walk the condition for side-effect diagnostics; the boolean
            // shape itself isn't enforced yet so we ignore the inferred type.
            let _cond_ty = infer_expr_inner(cond, env, ctx, diags);
            let then_ty = infer_expr_inner(then_branch, env, ctx, diags);
            match else_branch {
                Some(else_expr) => {
                    let else_ty = infer_expr_inner(else_expr, env, ctx, diags);
                    unify_branches(&then_ty, &else_ty, "if branches", ctx, diags)
                }
                None => {
                    // An `if` without `else` evaluates to `Unit` in
                    // statement position; we surface the then-branch type
                    // only when it is itself Unit so we don't pollute.
                    if matches!(then_ty, TypeRef::Unknown) {
                        TypeRef::Unknown
                    } else {
                        TypeRef::Primitive("Unit".to_string())
                    }
                }
            }
        }

        Expr::Match { scrutinee, arms } => {
            let _scrutinee_ty = infer_expr_inner(scrutinee, env, ctx, diags);
            infer_match_arms(arms, env, ctx, diags)
        }

        Expr::Return(value) => {
            // The return *expression* itself contributes `Never` semantics,
            // but we already track the declared return via `ctx`. We model
            // its visible type as the declared return so callers don't see
            // a spurious mismatch on `return`-tail bodies.
            if let Some(inner) = value {
                let inner_ty = infer_expr_inner(inner, env, ctx, diags);
                if let Some(diag) = mismatch_with_declared_return(
                    &ctx.declared_return,
                    &inner_ty,
                    &ctx.symbol_id,
                    &ctx.symbol_span,
                ) {
                    diags.push(diag);
                }
            }
            ctx.declared_return.clone()
        }

        Expr::Construct { variant, args } => infer_construct(variant, args, env, ctx, diags),

        Expr::Try(inner) => {
            let inner_ty = infer_expr_inner(inner, env, ctx, diags);
            match inner_ty {
                TypeRef::Generic { name, args } if name == "Result" => {
                    args.into_iter().next().unwrap_or(TypeRef::Unknown)
                }
                _ => TypeRef::Unknown,
            }
        }

        // The remaining variants are outside the bootstrap inference
        // subset. They walk children for diagnostic side-effects but do
        // not produce a concrete type.
        Expr::Field { base, .. } => {
            let _ = infer_expr_inner(base, env, ctx, diags);
            TypeRef::Unknown
        }
        Expr::Tuple(parts) => {
            for part in parts {
                let _ = infer_expr_inner(part, env, ctx, diags);
            }
            TypeRef::Unknown
        }
        Expr::Record { fields } => {
            for (_, value) in fields {
                let _ = infer_expr_inner(value, env, ctx, diags);
            }
            TypeRef::Unknown
        }
        Expr::Lambda { body, .. } => {
            let _ = infer_expr_inner(body, env, ctx, diags);
            TypeRef::Unknown
        }
        Expr::Error => TypeRef::Unknown,
    }
}

fn infer_literal(lit: &Literal) -> TypeRef {
    match lit {
        Literal::Int(_) => TypeRef::Primitive("Int".to_string()),
        Literal::Float(_) => TypeRef::Primitive("Float64".to_string()),
        Literal::Str(_) => TypeRef::Primitive("Str".to_string()),
        Literal::Bool(_) => TypeRef::Primitive("Bool".to_string()),
        Literal::Unit => TypeRef::Primitive("Unit".to_string()),
    }
}

fn infer_block(
    stmts: &[Stmt],
    tail: Option<&Expr>,
    env: &TypeEnv,
    ctx: &InferContext<'_>,
    diags: &mut Vec<Diagnostic>,
) -> TypeRef {
    let mut child = TypeEnv::with_parent(env.clone());
    for stmt in stmts {
        match stmt {
            Stmt::Let { name, ty, init } => {
                let init_ty = infer_expr_inner(init, &child, ctx, diags);
                // Prefer the explicit annotation when present; fall back to
                // the inferred type, and finally `Unknown` so monotonicity
                // is preserved.
                let bound_ty = match ty {
                    Some(annot) if !matches!(annot, TypeRef::Unknown) => annot.clone(),
                    _ => init_ty,
                };
                child.bind(name.clone(), bound_ty);
            }
            Stmt::Expr(e) => {
                let _ = infer_expr_inner(e, &child, ctx, diags);
            }
            Stmt::Return(value) => {
                if let Some(inner) = value {
                    let inner_ty = infer_expr_inner(inner, &child, ctx, diags);
                    if let Some(diag) = mismatch_with_declared_return(
                        &ctx.declared_return,
                        &inner_ty,
                        &ctx.symbol_id,
                        &ctx.symbol_span,
                    ) {
                        diags.push(diag);
                    }
                }
            }
        }
    }
    match tail {
        Some(expr) => infer_expr_inner(expr, &child, ctx, diags),
        None => TypeRef::Primitive("Unit".to_string()),
    }
}

fn infer_call(
    callee: &Expr,
    args: &[Expr],
    env: &TypeEnv,
    ctx: &InferContext<'_>,
    diags: &mut Vec<Diagnostic>,
) -> TypeRef {
    // Walk argument expressions first so we capture diagnostics from them
    // regardless of whether the callee resolves.
    for arg in args {
        let _ = infer_expr_inner(arg, env, ctx, diags);
    }
    match callee {
        Expr::Var(name) => match ctx.function_returns.get(name) {
            Some(ret) => ret.clone(),
            None => {
                diags.push(unknown_callee_diag(name, ctx));
                TypeRef::Unknown
            }
        },
        other => {
            // We don't yet model first-class function values; walk the
            // callee for diagnostics but return `Unknown` without polluting.
            let _ = infer_expr_inner(other, env, ctx, diags);
            TypeRef::Unknown
        }
    }
}

fn infer_match_arms(
    arms: &[MatchArm],
    env: &TypeEnv,
    ctx: &InferContext<'_>,
    diags: &mut Vec<Diagnostic>,
) -> TypeRef {
    let mut current: Option<TypeRef> = None;
    for arm in arms {
        let body_ty = infer_expr_inner(&arm.body, env, ctx, diags);
        current = Some(match current {
            None => body_ty,
            Some(prev) => unify_branches(&prev, &body_ty, "match arms", ctx, diags),
        });
    }
    current.unwrap_or(TypeRef::Unknown)
}

fn infer_construct(
    variant: &str,
    args: &[Expr],
    env: &TypeEnv,
    ctx: &InferContext<'_>,
    diags: &mut Vec<Diagnostic>,
) -> TypeRef {
    // Always walk the arguments so nested diagnostics surface.
    let arg_types: Vec<TypeRef> = args
        .iter()
        .map(|a| infer_expr_inner(a, env, ctx, diags))
        .collect();

    match variant {
        "Ok" => {
            let payload = arg_types.into_iter().next().unwrap_or(TypeRef::Unknown);
            TypeRef::Generic {
                name: "Result".to_string(),
                args: vec![payload, TypeRef::Unknown],
            }
        }
        "Err" => {
            let payload = arg_types.into_iter().next().unwrap_or(TypeRef::Unknown);
            TypeRef::Generic {
                name: "Result".to_string(),
                args: vec![TypeRef::Unknown, payload],
            }
        }
        "Some" => {
            let payload = arg_types.into_iter().next().unwrap_or(TypeRef::Unknown);
            TypeRef::Generic {
                name: "Option".to_string(),
                args: vec![payload],
            }
        }
        "None" => TypeRef::Generic {
            name: "Option".to_string(),
            args: vec![TypeRef::Unknown],
        },
        _ => TypeRef::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Unification & diagnostics
// ---------------------------------------------------------------------------

/// Combine two branch types into a single type. `Unknown` is treated as a
/// no-op so it never pollutes a concrete result. Genuine mismatches emit
/// `W0541` and resolve to `Unknown`.
fn unify_branches(
    left: &TypeRef,
    right: &TypeRef,
    label: &str,
    ctx: &InferContext<'_>,
    diags: &mut Vec<Diagnostic>,
) -> TypeRef {
    match (left, right) {
        (TypeRef::Unknown, other) | (other, TypeRef::Unknown) => other.clone(),
        (a, b) if a == b => a.clone(),
        (a, b) => {
            diags.push(
                Diagnostic::warning(
                    "W0541",
                    format!(
                        "incompatible types in {label}: `{}` vs `{}`",
                        a.display(),
                        b.display()
                    ),
                    ctx.symbol_span.clone(),
                )
                .with_symbol(ctx.symbol_id.clone())
                .with_expected(vec![a.display()])
                .with_found(vec![b.display()])
                .with_agent_summary(
                    "Make all branches produce the same type, or insert an explicit conversion.",
                )
                .with_docs(vec!["doc:types.unification".to_string()]),
            );
            TypeRef::Unknown
        }
    }
}

fn mismatch_with_declared_return(
    declared: &TypeRef,
    inferred: &TypeRef,
    symbol_id: &str,
    span: &Span,
) -> Option<Diagnostic> {
    if matches!(declared, TypeRef::Unknown) || matches!(inferred, TypeRef::Unknown) {
        return None;
    }
    if return_types_compatible(declared, inferred) {
        return None;
    }
    Some(
        Diagnostic::warning(
            "W0540",
            format!(
                "function body produces `{}` but signature declares `{}`",
                inferred.display(),
                declared.display()
            ),
            span.clone(),
        )
        .with_symbol(symbol_id.to_string())
        .with_expected(vec![declared.display()])
        .with_found(vec![inferred.display()])
        .with_agent_summary(
            "Adjust the body or the return type so the signature and the value agree.",
        )
        .with_docs(vec!["doc:types.return".to_string()]),
    )
}

/// Compatibility relation between declared and inferred return types.
/// Unknown is treated as compatible with anything to preserve
/// monotonicity. For generic head-equal types we recurse on arguments,
/// treating `Unknown` argument slots as wildcards (since `Ok(x)` produces
/// `Result[_, _]` until we infer the error arm).
fn return_types_compatible(declared: &TypeRef, inferred: &TypeRef) -> bool {
    match (declared, inferred) {
        (TypeRef::Unknown, _) | (_, TypeRef::Unknown) => true,
        (TypeRef::Primitive(a), TypeRef::Primitive(b)) | (TypeRef::Named(a), TypeRef::Named(b)) => {
            a == b
        }
        (TypeRef::Primitive(a), TypeRef::Named(b)) | (TypeRef::Named(a), TypeRef::Primitive(b)) => {
            a == b
        }
        (TypeRef::Generic { name: na, args: aa }, TypeRef::Generic { name: nb, args: ab }) => {
            if na != nb || aa.len() != ab.len() {
                return false;
            }
            aa.iter()
                .zip(ab.iter())
                .all(|(x, y)| return_types_compatible(x, y))
        }
        _ => false,
    }
}

fn unknown_identifier_diag(name: &str, ctx: &InferContext<'_>) -> Diagnostic {
    Diagnostic::warning(
        "W0530",
        format!("unknown identifier `{name}`"),
        ctx.symbol_span.clone(),
    )
    .with_symbol(ctx.symbol_id.clone())
    .with_expected(vec!["a binding in scope or a declared symbol".to_string()])
    .with_found(vec![name.to_string()])
    .with_agent_summary("Declare the identifier or bring it into scope before referencing it.")
    .with_docs(vec!["doc:types.scope".to_string()])
}

fn unknown_callee_diag(name: &str, ctx: &InferContext<'_>) -> Diagnostic {
    Diagnostic::warning(
        "W0531",
        format!("call target `{name}` is not a known function in this module"),
        ctx.symbol_span.clone(),
    )
    .with_symbol(ctx.symbol_id.clone())
    .with_expected(vec!["a function declared in the current module".to_string()])
    .with_found(vec![name.to_string()])
    .with_agent_summary("Define or import the function before calling it.")
    .with_docs(vec!["doc:types.calls".to_string()])
}

// ---------------------------------------------------------------------------
// Signature helpers (kept local to avoid cross-module coupling)
// ---------------------------------------------------------------------------

/// Build a lookup table of declared function return types keyed by bare
/// function name. Used to resolve `Call(Var(name), _)` without re-parsing
/// the whole signature for every call site.
fn collect_function_returns(module: &Module) -> BTreeMap<String, TypeRef> {
    let mut out = BTreeMap::new();
    for symbol in &module.symbols {
        if symbol.kind != SymbolKind::Function {
            continue;
        }
        let ret = parse_return_type(&symbol.signature);
        out.insert(symbol.name.clone(), ret);
    }
    out
}

/// Extract the textual return type from a compacted signature and parse it
/// into a [`TypeRef`]. Mirrors the logic in `hir::parse_return` but yields
/// a structured type instead of a string.
fn parse_return_type(signature: &str) -> TypeRef {
    let after = match signature.find("->") {
        Some(idx) => &signature[idx + 2..],
        None => return TypeRef::Primitive("Unit".to_string()),
    };
    let trimmed = after.trim();
    let cutoff = trimmed.find(" uses ").unwrap_or(trimmed.len());
    let ret = trimmed[..cutoff].trim();
    parse_type_ref_from_text(ret)
}

/// Parse a textual type into a [`TypeRef`]. Supports primitives, named
/// user types, and a single level of generic arguments (sufficient for the
/// bootstrap subset).
fn parse_type_ref_from_text(text: &str) -> TypeRef {
    let text = text.trim();
    if text.is_empty() {
        return TypeRef::Unknown;
    }
    if let Some(open) = text.find('[') {
        if text.ends_with(']') {
            let head = text[..open].trim().to_string();
            let inner = &text[open + 1..text.len() - 1];
            let args = split_top_level_commas(inner)
                .into_iter()
                .map(parse_type_ref_from_text)
                .collect();
            return TypeRef::Generic { name: head, args };
        }
    }
    if crate::types::is_builtin_type(text) {
        TypeRef::Primitive(text.to_string())
    } else {
        TypeRef::Named(text.to_string())
    }
}

fn split_top_level_commas(inner: &str) -> Vec<&str> {
    let mut depth = 0i32;
    let mut last = 0usize;
    let mut out = Vec::new();
    for (idx, ch) in inner.char_indices() {
        match ch {
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(inner[last..idx].trim());
                last = idx + 1;
            }
            _ => {}
        }
    }
    let tail = inner[last..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

/// Extract `(name, type)` pairs for every parameter declared in a compact
/// signature like `fn add(a: Int, b: Int) -> Int`. Best-effort: returns an
/// empty list if the parameter block can't be located.
fn parse_parameters(signature: &str) -> Vec<(String, TypeRef)> {
    let open = match signature.find('(') {
        Some(idx) => idx,
        None => return Vec::new(),
    };
    let close = match matching_paren(signature, open) {
        Some(idx) => idx,
        None => return Vec::new(),
    };
    let inner = &signature[open + 1..close];
    if inner.trim().is_empty() {
        return Vec::new();
    }
    split_top_level_commas(inner)
        .into_iter()
        .filter_map(|part| {
            let mut split = part.splitn(2, ':');
            let name = split.next().map(str::trim).unwrap_or("");
            let ty = split.next().map(str::trim).unwrap_or("");
            if name.is_empty() {
                None
            } else {
                Some((name.to_string(), parse_type_ref_from_text(ty)))
            }
        })
        .collect()
}

fn matching_paren(text: &str, open_idx: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if open_idx >= bytes.len() || bytes[open_idx] != b'(' {
        return None;
    }
    let mut depth = 0i32;
    for (i, b) in bytes.iter().enumerate().skip(open_idx) {
        match *b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::parse_module_bodies;
    use crate::expr::{Expr, Literal, MatchArm, Pattern, Stmt};
    use crate::parser::parse_source;
    use crate::source::SourceFile;

    fn must_be(ty: &TypeRef, expected: &str) {
        #[allow(clippy::assertions_on_constants)]
        {
            if ty.display() != expected {
                assert!(false, "expected type `{expected}`, got `{}`", ty.display());
            }
        }
    }

    // ---- literals ----

    #[test]
    fn int_literal_infers_int() {
        let env = TypeEnv::new();
        let (ty, diags) = infer_expr(&Expr::Lit(Literal::Int(7)), &env);
        must_be(&ty, "Int");
        assert!(diags.is_empty());
    }

    #[test]
    fn float_literal_infers_float64() {
        let env = TypeEnv::new();
        let (ty, _) = infer_expr(&Expr::Lit(Literal::Float(1.0)), &env);
        must_be(&ty, "Float64");
    }

    #[test]
    fn string_literal_infers_str() {
        let env = TypeEnv::new();
        let (ty, _) = infer_expr(&Expr::Lit(Literal::Str("hi".into())), &env);
        must_be(&ty, "Str");
    }

    #[test]
    fn bool_literal_infers_bool() {
        let env = TypeEnv::new();
        let (ty, _) = infer_expr(&Expr::Lit(Literal::Bool(true)), &env);
        must_be(&ty, "Bool");
    }

    #[test]
    fn unit_literal_infers_unit() {
        let env = TypeEnv::new();
        let (ty, _) = infer_expr(&Expr::Lit(Literal::Unit), &env);
        must_be(&ty, "Unit");
    }

    // ---- env / vars ----

    #[test]
    fn var_lookup_uses_env() {
        let mut env = TypeEnv::new();
        env.bind("x", TypeRef::Primitive("Int".into()));
        let (ty, diags) = infer_expr(&Expr::Var("x".into()), &env);
        must_be(&ty, "Int");
        assert!(diags.is_empty());
    }

    #[test]
    fn unknown_identifier_emits_w0530() {
        let env = TypeEnv::new();
        let (ty, diags) = infer_expr(&Expr::Var("nope".into()), &env);
        assert!(matches!(ty, TypeRef::Unknown));
        assert!(diags.iter().any(|d| d.id == "W0530"));
    }

    #[test]
    fn child_env_shadows_parent() {
        let mut outer = TypeEnv::new();
        outer.bind("x", TypeRef::Primitive("Int".into()));
        let mut inner = TypeEnv::with_parent(outer);
        inner.bind("x", TypeRef::Primitive("Str".into()));
        let (ty, _) = infer_expr(&Expr::Var("x".into()), &inner);
        must_be(&ty, "Str");
    }

    // ---- blocks / let ----

    #[test]
    fn block_binds_let_and_infers_tail() {
        let env = TypeEnv::new();
        let block = Expr::Block {
            stmts: vec![Stmt::Let {
                name: "x".into(),
                ty: None,
                init: Expr::Lit(Literal::Int(1)),
            }],
            tail: Some(Box::new(Expr::Var("x".into()))),
        };
        let (ty, diags) = infer_expr(&block, &env);
        must_be(&ty, "Int");
        assert!(diags.is_empty());
    }

    #[test]
    fn block_without_tail_is_unit() {
        let env = TypeEnv::new();
        let block = Expr::Block {
            stmts: vec![Stmt::Expr(Expr::Lit(Literal::Int(1)))],
            tail: None,
        };
        let (ty, _) = infer_expr(&block, &env);
        must_be(&ty, "Unit");
    }

    // ---- if / match ----

    #[test]
    fn if_with_matching_branches_unifies() {
        let env = TypeEnv::new();
        let expr = Expr::If {
            cond: Box::new(Expr::Lit(Literal::Bool(true))),
            then_branch: Box::new(Expr::Lit(Literal::Int(1))),
            else_branch: Some(Box::new(Expr::Lit(Literal::Int(2)))),
        };
        let (ty, diags) = infer_expr(&expr, &env);
        must_be(&ty, "Int");
        assert!(!diags.iter().any(|d| d.id == "W0541"));
    }

    #[test]
    fn if_with_mismatched_branches_emits_w0541() {
        let env = TypeEnv::new();
        let expr = Expr::If {
            cond: Box::new(Expr::Lit(Literal::Bool(true))),
            then_branch: Box::new(Expr::Lit(Literal::Int(1))),
            else_branch: Some(Box::new(Expr::Lit(Literal::Str("oops".into())))),
        };
        let (_, diags) = infer_expr(&expr, &env);
        assert!(diags.iter().any(|d| d.id == "W0541"));
    }

    #[test]
    fn match_arms_unify() {
        let env = TypeEnv::new();
        let expr = Expr::Match {
            scrutinee: Box::new(Expr::Lit(Literal::Int(0))),
            arms: vec![
                MatchArm {
                    pattern: Pattern::Wildcard,
                    body: Expr::Lit(Literal::Int(1)),
                },
                MatchArm {
                    pattern: Pattern::Wildcard,
                    body: Expr::Lit(Literal::Int(2)),
                },
            ],
        };
        let (ty, diags) = infer_expr(&expr, &env);
        must_be(&ty, "Int");
        assert!(diags.is_empty());
    }

    #[test]
    fn match_arms_mismatch_emits_w0541() {
        let env = TypeEnv::new();
        let expr = Expr::Match {
            scrutinee: Box::new(Expr::Lit(Literal::Int(0))),
            arms: vec![
                MatchArm {
                    pattern: Pattern::Wildcard,
                    body: Expr::Lit(Literal::Int(1)),
                },
                MatchArm {
                    pattern: Pattern::Wildcard,
                    body: Expr::Lit(Literal::Str("x".into())),
                },
            ],
        };
        let (_, diags) = infer_expr(&expr, &env);
        assert!(diags.iter().any(|d| d.id == "W0541"));
    }

    // ---- constructors / try ----

    #[test]
    fn ok_construct_produces_result_with_payload() {
        let env = TypeEnv::new();
        let expr = Expr::Construct {
            variant: "Ok".into(),
            args: vec![Expr::Lit(Literal::Int(1))],
        };
        let (ty, _) = infer_expr(&expr, &env);
        must_be(&ty, "Result[Int, _]");
    }

    #[test]
    fn err_construct_produces_result_with_error() {
        let env = TypeEnv::new();
        let expr = Expr::Construct {
            variant: "Err".into(),
            args: vec![Expr::Lit(Literal::Str("nope".into()))],
        };
        let (ty, _) = infer_expr(&expr, &env);
        must_be(&ty, "Result[_, Str]");
    }

    #[test]
    fn some_and_none_produce_option() {
        let env = TypeEnv::new();
        let (some_ty, _) = infer_expr(
            &Expr::Construct {
                variant: "Some".into(),
                args: vec![Expr::Lit(Literal::Int(1))],
            },
            &env,
        );
        must_be(&some_ty, "Option[Int]");
        let (none_ty, _) = infer_expr(
            &Expr::Construct {
                variant: "None".into(),
                args: vec![],
            },
            &env,
        );
        must_be(&none_ty, "Option[_]");
    }

    #[test]
    fn try_on_result_returns_payload() {
        let mut env = TypeEnv::new();
        env.bind(
            "r",
            TypeRef::Generic {
                name: "Result".into(),
                args: vec![
                    TypeRef::Primitive("Int".into()),
                    TypeRef::Named("MyErr".into()),
                ],
            },
        );
        let (ty, _) = infer_expr(&Expr::Try(Box::new(Expr::Var("r".into()))), &env);
        must_be(&ty, "Int");
    }

    #[test]
    fn try_on_non_result_is_unknown() {
        let env = TypeEnv::new();
        let (ty, _) = infer_expr(&Expr::Try(Box::new(Expr::Lit(Literal::Int(1)))), &env);
        assert!(matches!(ty, TypeRef::Unknown));
    }

    // ---- calls / module-level checks ----

    #[test]
    fn call_to_known_function_returns_declared_type() {
        let s = SourceFile::new(
            "/t.ori",
            "module a\nfn helper() -> Int:\n  return 1\nfn user() -> Int:\n  return helper()\n",
        );
        let module = parse_source(&s).module;
        let bodies = parse_module_bodies(&s);
        let diags = check_module_bodies(&module, &bodies);
        // No mismatch — `helper()` resolves to `Int`, matching `user`'s
        // declared `Int` return.
        assert!(!diags.iter().any(|d| d.id == "W0540"));
        // And no unknown-callee diagnostic for `helper`.
        assert!(!diags.iter().any(|d| d.id == "W0531"));
    }

    #[test]
    fn unknown_callee_emits_w0531() {
        let s = SourceFile::new(
            "/t.ori",
            "module a\nfn user() -> Int:\n  return missing()\n",
        );
        let module = parse_source(&s).module;
        let bodies = parse_module_bodies(&s);
        let diags = check_module_bodies(&module, &bodies);
        assert!(diags.iter().any(|d| d.id == "W0531"));
    }

    #[test]
    fn return_mismatch_emits_w0540() {
        let s = SourceFile::new("/t.ori", "module a\nfn f() -> Int:\n  return \"oops\"\n");
        let module = parse_source(&s).module;
        let bodies = parse_module_bodies(&s);
        let diags = check_module_bodies(&module, &bodies);
        assert!(diags.iter().any(|d| d.id == "W0540"));
    }

    #[test]
    fn return_match_does_not_emit_w0540() {
        let s = SourceFile::new("/t.ori", "module a\nfn f() -> Int:\n  return 42\n");
        let module = parse_source(&s).module;
        let bodies = parse_module_bodies(&s);
        let diags = check_module_bodies(&module, &bodies);
        assert!(!diags.iter().any(|d| d.id == "W0540"));
    }

    #[test]
    fn parameters_are_in_scope_inside_body() {
        let s = SourceFile::new("/t.ori", "module a\nfn id(x: Int) -> Int:\n  return x\n");
        let module = parse_source(&s).module;
        let bodies = parse_module_bodies(&s);
        let diags = check_module_bodies(&module, &bodies);
        // `x` must resolve via the parameter binding — no unknown-id
        // and no return mismatch.
        assert!(!diags.iter().any(|d| d.id == "W0530"));
        assert!(!diags.iter().any(|d| d.id == "W0540"));
    }

    // ---- monotonicity ----

    #[test]
    fn unknown_does_not_pollute_concrete_branch() {
        let env = TypeEnv::new();
        let expr = Expr::If {
            cond: Box::new(Expr::Lit(Literal::Bool(true))),
            // `nope` is unknown ⇒ Unknown
            then_branch: Box::new(Expr::Var("nope".into())),
            // concrete Int
            else_branch: Some(Box::new(Expr::Lit(Literal::Int(1)))),
        };
        let (ty, diags) = infer_expr(&expr, &env);
        // Unknown unifies with Int → Int, not a mismatch.
        must_be(&ty, "Int");
        assert!(!diags.iter().any(|d| d.id == "W0541"));
        // The unknown identifier still surfaces its own diagnostic.
        assert!(diags.iter().any(|d| d.id == "W0530"));
    }
}
