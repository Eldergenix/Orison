//! Tree-walking interpreter for Orison bootstrap programs.
//!
//! Unlike [`crate::interp`] which only reports the *observed* effects of a
//! module's entrypoint, this module actually executes parsed function bodies
//! ([`crate::expr::Expr`]) over a small dynamic value lattice.
//!
//! Scope of bootstrap fidelity:
//!   * No first-class closures, no method dispatch, no field assignment.
//!   * `Call(Var(name), args)` looks up a top-level function in the env.
//!   * `Match` supports `Pattern::Literal`, `Pattern::Binding`, and
//!     `Pattern::Wildcard`. Constructor patterns aren't matched.
//!   * `Try` operates on `Ok_/Err_` constructors only.
//!
//! Errors are surfaced as [`RuntimeError`] values with stable codes so the
//! CLI and downstream agents can map them deterministically:
//!   * `R0001` — entrypoint not found.
//!   * `R0002` — name lookup failed (unknown variable or unknown call target).
//!   * `R0003` — arity mismatch when calling a function.
//!   * `R0004` — type mismatch (`if`-cond not Bool, `try` on non-Result, etc.).
//!   * `R0005` — call stack exceeded the recursion cap.
//!
//! Effects from the entry function's declared `uses ...` clause are mirrored
//! into [`RuntimeError::observed_effects`] so a failed run still tells the
//! caller "what capabilities the program would have touched".

use crate::ast::{Module, SymbolKind};
use crate::body::ModuleBodies;
use crate::expr::{Expr, Literal, MatchArm, Pattern, Stmt};
use std::collections::BTreeMap;

// Conservative cap: every call_function frame in debug profile carries a
// large Env clone plus the recursive eval_expr stack underneath, so the
// raw OS stack runs out well before pure call-frame counting would. 64
// is well inside the safe window on every supported toolchain (verified
// with `cargo test` in dev/debug mode where each frame is 4-8x the size
// of release).
const MAX_CALL_DEPTH: usize = 64;

/// Dynamic value lattice. Kept intentionally narrow so the bootstrap
/// interpreter has a single canonical representation for each AST shape.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Unit,
    None_,
    Some_(Box<Value>),
    Ok_(Box<Value>),
    Err_(Box<Value>),
    List(Vec<Value>),
    Record(BTreeMap<String, Value>),
    /// First-class lambda value, captured at evaluation time. Used by
    /// the `list.map` / `list.filter` builtins so callers can pass
    /// `fn (x) => …` literals to higher-order list combinators. Lambdas
    /// here are *not* closures — they do not capture the surrounding
    /// lexical scope, mirroring the bootstrap's existing scoping rules.
    Lambda {
        params: Vec<String>,
        body: Expr,
    },
    /// In-process cache value backing the `cache.*` builtins. The
    /// `entries` list is FIFO-ordered: `put` appends to the tail; when
    /// the cache is at `capacity`, the head is evicted first. This
    /// choice keeps iteration deterministic without needing a sidecar
    /// access-recency table — sufficient for the bootstrap's needs.
    Cache {
        capacity: usize,
        entries: Vec<(Value, Value)>,
    },
}

impl Value {
    /// Short tag used in error messages so the user sees something like
    /// "expected Bool, got Int" without us serialising the whole value.
    pub fn type_tag(&self) -> &'static str {
        match self {
            Value::Int(_) => "Int",
            Value::Float(_) => "Float",
            Value::Bool(_) => "Bool",
            Value::Str(_) => "Str",
            Value::Unit => "Unit",
            Value::None_ => "None",
            Value::Some_(_) => "Some",
            Value::Ok_(_) => "Ok",
            Value::Err_(_) => "Err",
            Value::List(_) => "List",
            Value::Record(_) => "Record",
            Value::Lambda { .. } => "Lambda",
            Value::Cache { .. } => "Cache",
        }
    }
}

/// Top-level function in the runtime image. Stored on the root [`Env`].
#[derive(Debug, Clone)]
pub struct FunctionDef {
    pub name: String,
    pub params: Vec<String>,
    pub body: Expr,
    pub effects: Vec<String>,
}

/// Lexical environment. A child frame's parent chain is only consulted for
/// variable lookups; functions live exclusively on the root environment so
/// callees never accidentally close over local bindings.
#[derive(Debug, Clone, Default)]
pub struct Env {
    pub parent: Option<Box<Env>>,
    pub bindings: BTreeMap<String, Value>,
    pub functions: BTreeMap<String, FunctionDef>,
}

impl Env {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a fresh child environment that delegates name lookups upward.
    pub fn child_of(parent: Env) -> Self {
        Self {
            parent: Some(Box::new(parent)),
            bindings: BTreeMap::new(),
            functions: BTreeMap::new(),
        }
    }

    pub fn bind(&mut self, name: impl Into<String>, value: Value) {
        self.bindings.insert(name.into(), value);
    }

    pub fn lookup(&self, name: &str) -> Option<&Value> {
        if let Some(value) = self.bindings.get(name) {
            return Some(value);
        }
        match &self.parent {
            Some(parent) => parent.lookup(name),
            None => None,
        }
    }

    /// Locate a function by name. Functions are only registered on the
    /// outermost frame, so we walk to the root.
    pub fn lookup_function(&self, name: &str) -> Option<&FunctionDef> {
        if let Some(def) = self.functions.get(name) {
            return Some(def);
        }
        match &self.parent {
            Some(parent) => parent.lookup_function(name),
            None => None,
        }
    }
}

/// Failure mode for [`exec_program`]. The `code`/`message` pair is stable;
/// `observed_effects` mirrors the entry function's declared capabilities so
/// callers can still report "what the program would have done".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError {
    pub code: String,
    pub message: String,
    pub observed_effects: Vec<String>,
}

impl RuntimeError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            observed_effects: Vec::new(),
        }
    }

    fn with_effects(mut self, effects: Vec<String>) -> Self {
        self.observed_effects = effects;
        self
    }
}

/// Internal control-flow signal. `Return` propagates upward through
/// statement and expression evaluators until the surrounding function call
/// unwraps it. `TryProp` carries the `Err(...)` value of a `?` operator
/// that fired on an `Err_`.
#[derive(Debug, Clone)]
enum EvalFlow {
    Value(Value),
    Return(Value),
    TryProp(Value),
    Err(RuntimeError),
}

/// Public entry point. Builds an [`Env`] from `module`/`bodies`, resolves
/// `entry`, and runs it with the provided `args`.
pub fn exec_program(
    module: &Module,
    bodies: &ModuleBodies,
    entry: &str,
    args: Vec<Value>,
) -> Result<Value, RuntimeError> {
    let mut env = Env::new();
    for symbol in &module.symbols {
        if symbol.kind != SymbolKind::Function {
            continue;
        }
        let Some(body) = bodies.get(&symbol.id) else {
            continue;
        };
        let params = extract_param_names(&symbol.signature);
        env.functions.insert(
            symbol.name.clone(),
            FunctionDef {
                name: symbol.name.clone(),
                params,
                body: body.clone(),
                effects: symbol.effects.clone(),
            },
        );
    }

    let entry_effects: Vec<String> = module
        .symbols
        .iter()
        .find(|s| s.kind == SymbolKind::Function && s.name == entry)
        .map(|s| s.effects.clone())
        .unwrap_or_default();

    let Some(entry_def) = env.lookup_function(entry).cloned() else {
        return Err(RuntimeError::new(
            "R0001",
            format!(
                "entry function `{entry}` not found in module `{}`",
                module.name
            ),
        )
        .with_effects(entry_effects));
    };

    if entry_def.params.len() != args.len() {
        return Err(RuntimeError::new(
            "R0003",
            format!(
                "entry `{}` expects {} argument(s), got {}",
                entry_def.name,
                entry_def.params.len(),
                args.len()
            ),
        )
        .with_effects(entry_effects));
    }

    match call_function(&env, &entry_def, args, 0) {
        Ok(value) => Ok(value),
        Err(err) => Err(err.with_effects(entry_effects)),
    }
}

/// Invoke a function by value. Creates a fresh child env, binds parameters,
/// evaluates the body, and unwraps `Return` into the call's value.
fn call_function(
    root: &Env,
    def: &FunctionDef,
    args: Vec<Value>,
    depth: usize,
) -> Result<Value, RuntimeError> {
    if depth >= MAX_CALL_DEPTH {
        return Err(RuntimeError::new(
            "R0005",
            format!(
                "call stack exceeded {MAX_CALL_DEPTH} frames while invoking `{}`",
                def.name
            ),
        ));
    }
    if def.params.len() != args.len() {
        return Err(RuntimeError::new(
            "R0003",
            format!(
                "function `{}` expects {} argument(s), got {}",
                def.name,
                def.params.len(),
                args.len()
            ),
        ));
    }
    let mut call_env = Env::child_of(root.clone());
    for (name, value) in def.params.iter().zip(args.into_iter()) {
        call_env.bind(name.clone(), value);
    }
    match eval_expr(&def.body, &mut call_env, depth + 1) {
        EvalFlow::Value(v) | EvalFlow::Return(v) => Ok(v),
        EvalFlow::TryProp(err_payload) => Ok(Value::Err_(Box::new(err_payload))),
        EvalFlow::Err(err) => Err(err),
    }
}

/// Evaluate an arbitrary expression. The [`EvalFlow`] return type lets us
/// propagate early `return` and `?` outcomes without losing per-frame state.
fn eval_expr(expr: &Expr, env: &mut Env, depth: usize) -> EvalFlow {
    match expr {
        Expr::Lit(lit) => EvalFlow::Value(eval_literal(lit)),

        Expr::Var(name) => {
            // The bootstrap lexer doesn't carve `true`/`false` into bool
            // literals — they reach us as bare identifiers. Honour them at
            // lookup time so `if true: ... else: ...` works as expected.
            if name == "true" {
                return EvalFlow::Value(Value::Bool(true));
            }
            if name == "false" {
                return EvalFlow::Value(Value::Bool(false));
            }
            match env.lookup(name) {
                Some(value) => EvalFlow::Value(value.clone()),
                None => EvalFlow::Err(RuntimeError::new("R0002", format!("unknown name `{name}`"))),
            }
        }

        Expr::Block { stmts, tail } => eval_block(stmts, tail.as_deref(), env, depth),

        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => eval_if(cond, then_branch, else_branch.as_deref(), env, depth),

        Expr::Match { scrutinee, arms } => eval_match(scrutinee, arms, env, depth),

        Expr::Return(value) => match value {
            Some(inner) => match eval_expr(inner, env, depth) {
                EvalFlow::Value(v) => EvalFlow::Return(v),
                EvalFlow::Return(v) => EvalFlow::Return(v),
                other => other,
            },
            None => EvalFlow::Return(Value::Unit),
        },

        Expr::Call { callee, args } => eval_call(callee, args, env, depth),

        Expr::Construct { variant, args } => eval_construct(variant, args, env, depth),

        Expr::Try(inner) => match eval_expr(inner, env, depth) {
            EvalFlow::Value(Value::Ok_(v)) => EvalFlow::Value(*v),
            EvalFlow::Value(Value::Err_(v)) => EvalFlow::TryProp(*v),
            EvalFlow::Value(other) => EvalFlow::Err(RuntimeError::new(
                "R0004",
                format!("`?` requires Ok/Err, got {}", other.type_tag()),
            )),
            other => other,
        },

        Expr::Field { base, name } => eval_field(base, name, env, depth),

        Expr::Tuple(_) => EvalFlow::Err(RuntimeError::new(
            "R0004",
            "tuple literals are not supported in the bootstrap interpreter",
        )),

        Expr::Record { fields } => eval_record(fields, env, depth),

        Expr::Lambda { params, body } => {
            // Lambdas evaluate to a `Value::Lambda` immediately. They do
            // not capture the surrounding env; callers must pass any
            // required bindings explicitly via parameters.
            let names: Vec<String> = params.iter().map(|(n, _)| n.clone()).collect();
            EvalFlow::Value(Value::Lambda {
                params: names,
                body: (**body).clone(),
            })
        }

        // Extended string literals (M21b) are not yet evaluated. We
        // surface a structured runtime error so callers can distinguish
        // "parser saw it" from "interpreter ran it".
        Expr::InterpString { .. } => EvalFlow::Err(RuntimeError::new(
            "R0004",
            "interpolated string literals are not supported in the bootstrap interpreter",
        )),
        Expr::RawStr { text, .. } => EvalFlow::Value(Value::Str(text.clone())),

        Expr::Error => EvalFlow::Err(RuntimeError::new(
            "R0004",
            "encountered Expr::Error recovery node during execution",
        )),

        // Operator forms (M21a) parse but are not yet executable in the
        // bootstrap interpreter; surface a structured runtime error so the
        // contract surface stays panic-free.
        Expr::Binary { .. } => EvalFlow::Err(RuntimeError::new(
            "R0004",
            "binary operators are not supported in the bootstrap interpreter",
        )),
        Expr::Unary { .. } => EvalFlow::Err(RuntimeError::new(
            "R0004",
            "unary operators are not supported in the bootstrap interpreter",
        )),
    }
}

fn eval_literal(lit: &Literal) -> Value {
    match lit {
        Literal::Int(i) => Value::Int(*i),
        Literal::Float(f) => Value::Float(*f),
        Literal::Bool(b) => Value::Bool(*b),
        Literal::Str(s) => Value::Str(s.clone()),
        Literal::Unit => Value::Unit,
    }
}

fn eval_block(stmts: &[Stmt], tail: Option<&Expr>, env: &mut Env, depth: usize) -> EvalFlow {
    for stmt in stmts {
        match stmt {
            Stmt::Let { name, init, .. } => {
                let value = match eval_expr(init, env, depth) {
                    EvalFlow::Value(v) => v,
                    other => return other,
                };
                env.bind(name.clone(), value);
            }
            Stmt::Expr(expr) => match eval_expr(expr, env, depth) {
                EvalFlow::Value(_) => {}
                other => return other,
            },
            Stmt::Return(value) => {
                let result = match value {
                    Some(expr) => match eval_expr(expr, env, depth) {
                        EvalFlow::Value(v) => v,
                        EvalFlow::Return(v) => v,
                        other => return other,
                    },
                    None => Value::Unit,
                };
                return EvalFlow::Return(result);
            }
        }
    }
    match tail {
        Some(expr) => eval_expr(expr, env, depth),
        None => EvalFlow::Value(Value::Unit),
    }
}

fn eval_if(
    cond: &Expr,
    then_branch: &Expr,
    else_branch: Option<&Expr>,
    env: &mut Env,
    depth: usize,
) -> EvalFlow {
    let cond_value = match eval_expr(cond, env, depth) {
        EvalFlow::Value(v) => v,
        other => return other,
    };
    match cond_value {
        Value::Bool(true) => eval_expr(then_branch, env, depth),
        Value::Bool(false) => match else_branch {
            Some(branch) => eval_expr(branch, env, depth),
            None => EvalFlow::Value(Value::Unit),
        },
        other => EvalFlow::Err(RuntimeError::new(
            "R0004",
            format!("`if` condition must be Bool, got {}", other.type_tag()),
        )),
    }
}

fn eval_match(scrutinee: &Expr, arms: &[MatchArm], env: &mut Env, depth: usize) -> EvalFlow {
    let value = match eval_expr(scrutinee, env, depth) {
        EvalFlow::Value(v) => v,
        other => return other,
    };
    for arm in arms {
        if let Some(bindings) = pattern_match(&arm.pattern, &value) {
            let mut arm_env = Env::child_of(env.clone());
            for (name, bound) in bindings {
                arm_env.bind(name, bound);
            }
            return eval_expr(&arm.body, &mut arm_env, depth);
        }
    }
    EvalFlow::Err(RuntimeError::new(
        "R0004",
        format!("no match arm matched value of type {}", value.type_tag()),
    ))
}

/// Try to match `value` against `pattern`. Returns the bindings introduced
/// by the arm on success, or `None` to signal "try the next arm". Only the
/// shapes documented at the module header are honoured; richer patterns
/// always return `None`.
fn pattern_match(pattern: &Pattern, value: &Value) -> Option<Vec<(String, Value)>> {
    match pattern {
        Pattern::Wildcard => Some(Vec::new()),
        Pattern::Binding(name) => Some(vec![(name.clone(), value.clone())]),
        Pattern::Literal(lit) => {
            if literal_eq(lit, value) {
                Some(Vec::new())
            } else {
                None
            }
        }
        Pattern::Constructor { .. } => None,
    }
}

fn literal_eq(lit: &Literal, value: &Value) -> bool {
    match (lit, value) {
        (Literal::Int(a), Value::Int(b)) => a == b,
        (Literal::Float(a), Value::Float(b)) => a == b,
        (Literal::Bool(a), Value::Bool(b)) => a == b,
        (Literal::Str(a), Value::Str(b)) => a == b,
        (Literal::Unit, Value::Unit) => true,
        _ => false,
    }
}

fn eval_call(callee: &Expr, args: &[Expr], env: &mut Env, depth: usize) -> EvalFlow {
    // First: check for namespaced builtins like `str.len(s)`. These
    // parse as `Call { callee: Field { base: Var("str"), name: "len" }, args }`.
    // We dispatch *before* evaluating the base so `str`/`list`/`cache`/`url`
    // do not need to exist as bindings in the runtime env.
    if let Expr::Field { base, name: method } = callee {
        if let Expr::Var(ns) = base.as_ref() {
            if is_builtin_namespace(ns) {
                let mut evaluated: Vec<Value> = Vec::with_capacity(args.len());
                for arg in args {
                    match eval_expr(arg, env, depth) {
                        EvalFlow::Value(v) => evaluated.push(v),
                        other => return other,
                    }
                }
                return dispatch_builtin(ns, method, evaluated, depth);
            }
        }
    }

    let Expr::Var(name) = callee else {
        return EvalFlow::Err(RuntimeError::new(
            "R0004",
            "only direct function calls (`name(...)`) are supported",
        ));
    };

    let def = match env.lookup_function(name) {
        Some(def) => def.clone(),
        None => {
            return EvalFlow::Err(RuntimeError::new(
                "R0002",
                format!("unknown call target `{name}`"),
            ));
        }
    };

    if def.params.len() != args.len() {
        return EvalFlow::Err(RuntimeError::new(
            "R0003",
            format!(
                "function `{}` expects {} argument(s), got {}",
                def.name,
                def.params.len(),
                args.len()
            ),
        ));
    }

    let mut evaluated: Vec<Value> = Vec::with_capacity(args.len());
    for arg in args {
        match eval_expr(arg, env, depth) {
            EvalFlow::Value(v) => evaluated.push(v),
            other => return other,
        }
    }

    // Functions execute against the root env so they cannot see locals
    // from the caller. We rebuild a root view from the call site's chain.
    let root_view = root_view(env);
    match call_function(&root_view, &def, evaluated, depth) {
        Ok(value) => EvalFlow::Value(value),
        Err(err) => EvalFlow::Err(err),
    }
}

/// Build a synthetic root [`Env`] containing only the function table from
/// the chain rooted at `env`. Used so that a callee never sees the
/// caller's local bindings.
fn root_view(env: &Env) -> Env {
    let mut functions: BTreeMap<String, FunctionDef> = BTreeMap::new();
    collect_functions(env, &mut functions);
    Env {
        parent: None,
        bindings: BTreeMap::new(),
        functions,
    }
}

fn collect_functions(env: &Env, out: &mut BTreeMap<String, FunctionDef>) {
    for (name, def) in &env.functions {
        // Inner shadowings would normally win; for the bootstrap each
        // function name is unique at the root so this is academic.
        out.entry(name.clone()).or_insert_with(|| def.clone());
    }
    if let Some(parent) = &env.parent {
        collect_functions(parent, out);
    }
}

fn eval_construct(variant: &str, args: &[Expr], env: &mut Env, depth: usize) -> EvalFlow {
    let mut evaluated: Vec<Value> = Vec::with_capacity(args.len());
    for arg in args {
        match eval_expr(arg, env, depth) {
            EvalFlow::Value(v) => evaluated.push(v),
            other => return other,
        }
    }
    match variant {
        "Ok" => match evaluated.into_iter().next() {
            Some(value) => EvalFlow::Value(Value::Ok_(Box::new(value))),
            None => EvalFlow::Err(RuntimeError::new(
                "R0003",
                "constructor `Ok` requires exactly 1 argument, got 0",
            )),
        },
        "Err" => match evaluated.into_iter().next() {
            Some(value) => EvalFlow::Value(Value::Err_(Box::new(value))),
            None => EvalFlow::Err(RuntimeError::new(
                "R0003",
                "constructor `Err` requires exactly 1 argument, got 0",
            )),
        },
        "Some" => match evaluated.into_iter().next() {
            Some(value) => EvalFlow::Value(Value::Some_(Box::new(value))),
            None => EvalFlow::Err(RuntimeError::new(
                "R0003",
                "constructor `Some` requires exactly 1 argument, got 0",
            )),
        },
        "None" => {
            if evaluated.is_empty() {
                EvalFlow::Value(Value::None_)
            } else {
                EvalFlow::Err(RuntimeError::new(
                    "R0003",
                    "constructor `None` takes no arguments",
                ))
            }
        }
        "Unit" => {
            if evaluated.is_empty() {
                EvalFlow::Value(Value::Unit)
            } else {
                EvalFlow::Err(RuntimeError::new(
                    "R0003",
                    "constructor `Unit` takes no arguments",
                ))
            }
        }
        "List" => EvalFlow::Value(Value::List(evaluated)),
        _ => {
            // Tagged record construction: `Url { scheme: "https", ... }`
            // parses as `Construct { variant: "Url", args: [Record{..}] }`.
            // We treat the inner Record as the value and drop the tag —
            // the bootstrap doesn't model nominal types yet, so structural
            // equality is the contract callers rely on.
            if evaluated.len() == 1 {
                if let Value::Record(_) = &evaluated[0] {
                    if let Some(v) = evaluated.into_iter().next() {
                        return EvalFlow::Value(v);
                    }
                    return EvalFlow::Value(Value::Record(BTreeMap::new()));
                }
            }
            if evaluated.is_empty() {
                return EvalFlow::Value(Value::Record(BTreeMap::new()));
            }
            // Variant with positional payloads but no record body: bundle
            // them under synthetic `_0`/`_1`/... keys so callers like
            // `Invalid(reason)` still produce something usable.
            let mut fields = BTreeMap::new();
            for (idx, v) in evaluated.into_iter().enumerate() {
                fields.insert(format!("_{idx}"), v);
            }
            EvalFlow::Value(Value::Record(fields))
        }
    }
}

/// Evaluate a record literal: walk each `(name, expr)` field, evaluate
/// the right-hand expression, and build a deterministic `BTreeMap`.
fn eval_record(fields: &[(String, Expr)], env: &mut Env, depth: usize) -> EvalFlow {
    let mut out: BTreeMap<String, Value> = BTreeMap::new();
    for (name, expr) in fields {
        let value = match eval_expr(expr, env, depth) {
            EvalFlow::Value(v) => v,
            other => return other,
        };
        out.insert(name.clone(), value);
    }
    EvalFlow::Value(Value::Record(out))
}

/// Evaluate field access. Only `Value::Record` carries fields; everything
/// else surfaces a structured runtime error so the namespace dispatch in
/// `eval_call` can still intercept `str.len(...)`-style calls upstream.
fn eval_field(base: &Expr, name: &str, env: &mut Env, depth: usize) -> EvalFlow {
    let base_value = match eval_expr(base, env, depth) {
        EvalFlow::Value(v) => v,
        other => return other,
    };
    match base_value {
        Value::Record(fields) => match fields.get(name) {
            Some(v) => EvalFlow::Value(v.clone()),
            None => EvalFlow::Err(RuntimeError::new(
                "R0004",
                format!("record has no field `{name}`"),
            )),
        },
        other => EvalFlow::Err(RuntimeError::new(
            "R0004",
            format!("field access requires Record, got {}", other.type_tag()),
        )),
    }
}

/// Pull parameter *names* out of a function signature string of the form
/// `fn name(p1: T1, p2: T2) -> R uses ...`. We only need names: types are
/// not enforced by the bootstrap interpreter.
fn extract_param_names(signature: &str) -> Vec<String> {
    let Some(open) = signature.find('(') else {
        return Vec::new();
    };
    let after_open = &signature[open + 1..];
    let Some(close) = after_open.find(')') else {
        return Vec::new();
    };
    let inside = &after_open[..close];
    let mut out: Vec<String> = Vec::new();
    for raw in inside.split(',') {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let name = match trimmed.find(':') {
            Some(idx) => trimmed[..idx].trim(),
            None => trimmed,
        };
        if name.is_empty() {
            continue;
        }
        out.push(name.to_string());
    }
    out
}

// ---------------------------------------------------------------------------
// Builtin namespace dispatch
//
// Calls like `str.len(s)`, `list.map(xs, f)`, `cache.put(c, k, v)`, and
// `url.append_query(u, k, v)` are intercepted in `eval_call` before
// normal name resolution runs. The receiver name (`str`, `list`,
// `cache`, `url`, plus the `string` alias for `str`) is treated as a
// builtin namespace, never a real binding.
//
// All Unicode case conversions are intentionally ASCII-only in the
// bootstrap. A future iteration can swap `make_ascii_lowercase` for a
// `unicode_normalization`-aware fold without changing the surface.
// ---------------------------------------------------------------------------

fn is_builtin_namespace(name: &str) -> bool {
    matches!(name, "str" | "string" | "list" | "cache" | "url")
}

fn dispatch_builtin(ns: &str, method: &str, args: Vec<Value>, depth: usize) -> EvalFlow {
    match ns {
        "str" | "string" => dispatch_str_builtin(method, args),
        "list" => dispatch_list_builtin(method, args, depth),
        "cache" => dispatch_cache_builtin(method, args),
        "url" => dispatch_url_builtin(method, args),
        other => EvalFlow::Err(RuntimeError::new(
            "R0004",
            format!("unknown builtin namespace `{other}`"),
        )),
    }
}

fn builtin_arity_err(ns: &str, method: &str, expected: usize, got: usize) -> EvalFlow {
    EvalFlow::Err(RuntimeError::new(
        "R0003",
        format!("builtin `{ns}.{method}` expects {expected} argument(s), got {got}"),
    ))
}

fn builtin_type_err(ns: &str, method: &str, expected: &str, got: &str) -> EvalFlow {
    EvalFlow::Err(RuntimeError::new(
        "R0004",
        format!("builtin `{ns}.{method}` expected {expected}, got {got}"),
    ))
}

fn dispatch_str_builtin(method: &str, args: Vec<Value>) -> EvalFlow {
    match method {
        "len" => {
            if args.len() != 1 {
                return builtin_arity_err("str", "len", 1, args.len());
            }
            match &args[0] {
                Value::Str(s) => EvalFlow::Value(Value::Int(s.chars().count() as i64)),
                other => builtin_type_err("str", "len", "Str", other.type_tag()),
            }
        }
        "contains" => {
            if args.len() != 2 {
                return builtin_arity_err("str", "contains", 2, args.len());
            }
            match (&args[0], &args[1]) {
                (Value::Str(s), Value::Str(needle)) => {
                    EvalFlow::Value(Value::Bool(s.contains(needle.as_str())))
                }
                (a, b) => builtin_type_err(
                    "str",
                    "contains",
                    "(Str, Str)",
                    &format!("({}, {})", a.type_tag(), b.type_tag()),
                ),
            }
        }
        "starts_with" => {
            if args.len() != 2 {
                return builtin_arity_err("str", "starts_with", 2, args.len());
            }
            match (&args[0], &args[1]) {
                (Value::Str(s), Value::Str(prefix)) => {
                    EvalFlow::Value(Value::Bool(s.starts_with(prefix.as_str())))
                }
                (a, b) => builtin_type_err(
                    "str",
                    "starts_with",
                    "(Str, Str)",
                    &format!("({}, {})", a.type_tag(), b.type_tag()),
                ),
            }
        }
        "ends_with" => {
            if args.len() != 2 {
                return builtin_arity_err("str", "ends_with", 2, args.len());
            }
            match (&args[0], &args[1]) {
                (Value::Str(s), Value::Str(suffix)) => {
                    EvalFlow::Value(Value::Bool(s.ends_with(suffix.as_str())))
                }
                (a, b) => builtin_type_err(
                    "str",
                    "ends_with",
                    "(Str, Str)",
                    &format!("({}, {})", a.type_tag(), b.type_tag()),
                ),
            }
        }
        "split" => {
            if args.len() != 2 {
                return builtin_arity_err("str", "split", 2, args.len());
            }
            match (&args[0], &args[1]) {
                (Value::Str(s), Value::Str(sep)) => {
                    let parts: Vec<Value> = if sep.is_empty() {
                        // Split-on-empty in Rust emits a piece per
                        // boundary; we collapse to a single-element
                        // list so stdlib callers see a deterministic
                        // identity transform.
                        vec![Value::Str(s.clone())]
                    } else {
                        s.split(sep.as_str())
                            .map(|piece| Value::Str(piece.to_string()))
                            .collect()
                    };
                    EvalFlow::Value(Value::List(parts))
                }
                (a, b) => builtin_type_err(
                    "str",
                    "split",
                    "(Str, Str)",
                    &format!("({}, {})", a.type_tag(), b.type_tag()),
                ),
            }
        }
        "to_lower" => {
            if args.len() != 1 {
                return builtin_arity_err("str", "to_lower", 1, args.len());
            }
            match &args[0] {
                // ASCII-only: the bootstrap intentionally avoids pulling
                // in a Unicode case-fold dependency. Non-ASCII bytes
                // round-trip unchanged. Document this in stdlib docs.
                Value::Str(s) => EvalFlow::Value(Value::Str(s.to_ascii_lowercase())),
                other => builtin_type_err("str", "to_lower", "Str", other.type_tag()),
            }
        }
        "to_upper" => {
            if args.len() != 1 {
                return builtin_arity_err("str", "to_upper", 1, args.len());
            }
            match &args[0] {
                Value::Str(s) => EvalFlow::Value(Value::Str(s.to_ascii_uppercase())),
                other => builtin_type_err("str", "to_upper", "Str", other.type_tag()),
            }
        }
        "join" => {
            if args.len() != 2 {
                return builtin_arity_err("str", "join", 2, args.len());
            }
            match (&args[0], &args[1]) {
                (Value::List(items), Value::Str(sep)) => {
                    let mut pieces: Vec<String> = Vec::with_capacity(items.len());
                    for item in items {
                        match item {
                            Value::Str(s) => pieces.push(s.clone()),
                            other => {
                                return builtin_type_err(
                                    "str",
                                    "join",
                                    "List[Str]",
                                    &format!("List containing {}", other.type_tag()),
                                );
                            }
                        }
                    }
                    EvalFlow::Value(Value::Str(pieces.join(sep.as_str())))
                }
                (a, b) => builtin_type_err(
                    "str",
                    "join",
                    "(List[Str], Str)",
                    &format!("({}, {})", a.type_tag(), b.type_tag()),
                ),
            }
        }
        other => EvalFlow::Err(RuntimeError::new(
            "R0002",
            format!("unknown str builtin `str.{other}`"),
        )),
    }
}

fn dispatch_list_builtin(method: &str, args: Vec<Value>, depth: usize) -> EvalFlow {
    match method {
        "len" => {
            if args.len() != 1 {
                return builtin_arity_err("list", "len", 1, args.len());
            }
            match &args[0] {
                Value::List(items) => EvalFlow::Value(Value::Int(items.len() as i64)),
                other => builtin_type_err("list", "len", "List", other.type_tag()),
            }
        }
        "is_empty" => {
            if args.len() != 1 {
                return builtin_arity_err("list", "is_empty", 1, args.len());
            }
            match &args[0] {
                Value::List(items) => EvalFlow::Value(Value::Bool(items.is_empty())),
                other => builtin_type_err("list", "is_empty", "List", other.type_tag()),
            }
        }
        "push" => {
            if args.len() != 2 {
                return builtin_arity_err("list", "push", 2, args.len());
            }
            let mut iter = args.into_iter();
            let (Some(first), Some(item)) = (iter.next(), iter.next()) else {
                return builtin_arity_err("list", "push", 2, 0);
            };
            match first {
                Value::List(mut items) => {
                    items.push(item);
                    EvalFlow::Value(Value::List(items))
                }
                other => builtin_type_err("list", "push", "List", other.type_tag()),
            }
        }
        "pop" => {
            if args.len() != 1 {
                return builtin_arity_err("list", "pop", 1, args.len());
            }
            let mut iter = args.into_iter();
            let Some(first) = iter.next() else {
                return builtin_arity_err("list", "pop", 1, 0);
            };
            match first {
                // "Last" here is the tail of the list (LIFO pop) — chosen
                // because `push` is append-to-tail, so pop/push round-trip
                // the same element.
                Value::List(mut items) => match items.pop() {
                    Some(v) => EvalFlow::Value(Value::Some_(Box::new(v))),
                    None => EvalFlow::Value(Value::None_),
                },
                other => builtin_type_err("list", "pop", "List", other.type_tag()),
            }
        }
        "map" => {
            if args.len() != 2 {
                return builtin_arity_err("list", "map", 2, args.len());
            }
            let mut iter = args.into_iter();
            let (Some(list_val), Some(f_val)) = (iter.next(), iter.next()) else {
                return builtin_arity_err("list", "map", 2, 0);
            };
            let items = match list_val {
                Value::List(items) => items,
                other => return builtin_type_err("list", "map", "List", other.type_tag()),
            };
            let (params, body) = match f_val {
                Value::Lambda { params, body } => (params, body),
                other => return builtin_type_err("list", "map", "Lambda", other.type_tag()),
            };
            if params.len() != 1 {
                return EvalFlow::Err(RuntimeError::new(
                    "R0003",
                    format!("list.map expects a 1-arg lambda, got {}-arg", params.len()),
                ));
            }
            let mut out: Vec<Value> = Vec::with_capacity(items.len());
            for item in items {
                let mut frame = Env::new();
                frame.bind(params[0].clone(), item);
                match eval_expr(&body, &mut frame, depth + 1) {
                    EvalFlow::Value(v) | EvalFlow::Return(v) => out.push(v),
                    EvalFlow::Err(e) => return EvalFlow::Err(e),
                    EvalFlow::TryProp(v) => {
                        return EvalFlow::Value(Value::Err_(Box::new(v)));
                    }
                }
            }
            EvalFlow::Value(Value::List(out))
        }
        "filter" => {
            if args.len() != 2 {
                return builtin_arity_err("list", "filter", 2, args.len());
            }
            let mut iter = args.into_iter();
            let (Some(list_val), Some(f_val)) = (iter.next(), iter.next()) else {
                return builtin_arity_err("list", "filter", 2, 0);
            };
            let items = match list_val {
                Value::List(items) => items,
                other => return builtin_type_err("list", "filter", "List", other.type_tag()),
            };
            let (params, body) = match f_val {
                Value::Lambda { params, body } => (params, body),
                other => return builtin_type_err("list", "filter", "Lambda", other.type_tag()),
            };
            if params.len() != 1 {
                return EvalFlow::Err(RuntimeError::new(
                    "R0003",
                    format!(
                        "list.filter expects a 1-arg lambda, got {}-arg",
                        params.len()
                    ),
                ));
            }
            let mut out: Vec<Value> = Vec::new();
            for item in items {
                let mut frame = Env::new();
                frame.bind(params[0].clone(), item.clone());
                match eval_expr(&body, &mut frame, depth + 1) {
                    EvalFlow::Value(Value::Bool(true)) | EvalFlow::Return(Value::Bool(true)) => {
                        out.push(item);
                    }
                    EvalFlow::Value(Value::Bool(false)) | EvalFlow::Return(Value::Bool(false)) => {}
                    EvalFlow::Value(other) | EvalFlow::Return(other) => {
                        return builtin_type_err(
                            "list",
                            "filter",
                            "Bool from predicate",
                            other.type_tag(),
                        );
                    }
                    EvalFlow::Err(e) => return EvalFlow::Err(e),
                    EvalFlow::TryProp(v) => {
                        return EvalFlow::Value(Value::Err_(Box::new(v)));
                    }
                }
            }
            EvalFlow::Value(Value::List(out))
        }
        other => EvalFlow::Err(RuntimeError::new(
            "R0002",
            format!("unknown list builtin `list.{other}`"),
        )),
    }
}

fn dispatch_cache_builtin(method: &str, args: Vec<Value>) -> EvalFlow {
    match method {
        "new" => {
            if args.len() != 1 {
                return builtin_arity_err("cache", "new", 1, args.len());
            }
            match &args[0] {
                Value::Int(cap) if *cap >= 0 => EvalFlow::Value(Value::Cache {
                    capacity: *cap as usize,
                    entries: Vec::new(),
                }),
                Value::Int(neg) => EvalFlow::Err(RuntimeError::new(
                    "R0004",
                    format!("cache.new capacity must be non-negative, got {neg}"),
                )),
                other => builtin_type_err("cache", "new", "Int", other.type_tag()),
            }
        }
        "len" => {
            if args.len() != 1 {
                return builtin_arity_err("cache", "len", 1, args.len());
            }
            match &args[0] {
                Value::Cache { entries, .. } => EvalFlow::Value(Value::Int(entries.len() as i64)),
                other => builtin_type_err("cache", "len", "Cache", other.type_tag()),
            }
        }
        "get" => {
            if args.len() != 2 {
                return builtin_arity_err("cache", "get", 2, args.len());
            }
            match &args[0] {
                Value::Cache { entries, .. } => {
                    for (k, v) in entries {
                        if k == &args[1] {
                            return EvalFlow::Value(Value::Some_(Box::new(v.clone())));
                        }
                    }
                    EvalFlow::Value(Value::None_)
                }
                other => builtin_type_err("cache", "get", "Cache", other.type_tag()),
            }
        }
        "put" => {
            if args.len() != 3 {
                return builtin_arity_err("cache", "put", 3, args.len());
            }
            let mut iter = args.into_iter();
            let Some(first) = iter.next() else {
                return builtin_arity_err("cache", "put", 3, 0);
            };
            let Some(key) = iter.next() else {
                return builtin_arity_err("cache", "put", 3, 1);
            };
            let Some(value) = iter.next() else {
                return builtin_arity_err("cache", "put", 3, 2);
            };
            match first {
                Value::Cache {
                    capacity,
                    mut entries,
                } => {
                    // Update-in-place if key exists, preserving its
                    // FIFO position so iteration order stays stable.
                    let mut updated = false;
                    for (k, v) in entries.iter_mut() {
                        if k == &key {
                            *v = value.clone();
                            updated = true;
                            break;
                        }
                    }
                    if !updated {
                        if capacity == 0 {
                            // Zero-capacity caches accept nothing; this
                            // mirrors the "Full" variant in std.cache.
                            return EvalFlow::Value(Value::Cache { capacity, entries });
                        }
                        if entries.len() >= capacity {
                            // FIFO eviction: drop the oldest entry.
                            let _ = entries.remove(0);
                        }
                        entries.push((key, value));
                    }
                    EvalFlow::Value(Value::Cache { capacity, entries })
                }
                other => builtin_type_err("cache", "put", "Cache", other.type_tag()),
            }
        }
        "clear" => {
            if args.len() != 1 {
                return builtin_arity_err("cache", "clear", 1, args.len());
            }
            let mut iter = args.into_iter();
            let Some(first) = iter.next() else {
                return builtin_arity_err("cache", "clear", 1, 0);
            };
            match first {
                Value::Cache { capacity, .. } => EvalFlow::Value(Value::Cache {
                    capacity,
                    entries: Vec::new(),
                }),
                other => builtin_type_err("cache", "clear", "Cache", other.type_tag()),
            }
        }
        other => EvalFlow::Err(RuntimeError::new(
            "R0002",
            format!("unknown cache builtin `cache.{other}`"),
        )),
    }
}

fn dispatch_url_builtin(method: &str, args: Vec<Value>) -> EvalFlow {
    match method {
        // `url.parse(s)` returns a Result-tagged Record:
        //   Ok({ scheme, host, path }) on a recognised scheme,
        //   Err({ kind: "Invalid"|"UnsupportedScheme", reason|scheme })
        // The scheme/host/path split honours the URL syntax most callers
        // need (`scheme://host[:port]/path?query`).
        "parse" => {
            if args.len() != 1 {
                return builtin_arity_err("url", "parse", 1, args.len());
            }
            match &args[0] {
                Value::Str(s) => parse_url_str(s),
                other => builtin_type_err("url", "parse", "Str", other.type_tag()),
            }
        }
        "scheme_of" => {
            if args.len() != 1 {
                return builtin_arity_err("url", "scheme_of", 1, args.len());
            }
            match &args[0] {
                Value::Str(s) => EvalFlow::Value(Value::Str(extract_scheme(s))),
                other => builtin_type_err("url", "scheme_of", "Str", other.type_tag()),
            }
        }
        "host_of" => {
            if args.len() != 1 {
                return builtin_arity_err("url", "host_of", 1, args.len());
            }
            match &args[0] {
                Value::Str(s) => EvalFlow::Value(Value::Str(extract_host(s))),
                other => builtin_type_err("url", "host_of", "Str", other.type_tag()),
            }
        }
        "path_of" => {
            if args.len() != 1 {
                return builtin_arity_err("url", "path_of", 1, args.len());
            }
            match &args[0] {
                Value::Str(s) => EvalFlow::Value(Value::Str(extract_path(s))),
                other => builtin_type_err("url", "path_of", "Str", other.type_tag()),
            }
        }
        // `url.append_query(url, key, value)` accepts either a Str URL
        // or a Record URL (with a `scheme`/`host`/`path` shape from
        // `parse`). Appends `?key=value` if no query is present, else
        // `&key=value`. No escaping is performed in the bootstrap.
        "append_query" => {
            if args.len() != 3 {
                return builtin_arity_err("url", "append_query", 3, args.len());
            }
            let key = match &args[1] {
                Value::Str(s) => s.clone(),
                other => {
                    return builtin_type_err("url", "append_query", "Str key", other.type_tag())
                }
            };
            let value = match &args[2] {
                Value::Str(s) => s.clone(),
                other => {
                    return builtin_type_err("url", "append_query", "Str value", other.type_tag());
                }
            };
            match &args[0] {
                Value::Str(s) => {
                    let sep = if s.contains('?') { '&' } else { '?' };
                    EvalFlow::Value(Value::Str(format!("{s}{sep}{key}={value}")))
                }
                Value::Record(fields) => {
                    // Append to the `path` field so the next round-trip
                    // through `parse` finds the same query suffix.
                    let mut new_fields = fields.clone();
                    let path_str = match new_fields.get("path") {
                        Some(Value::Str(p)) => p.clone(),
                        _ => String::new(),
                    };
                    let sep = if path_str.contains('?') { '&' } else { '?' };
                    let new_path = format!("{path_str}{sep}{key}={value}");
                    new_fields.insert("path".to_string(), Value::Str(new_path));
                    EvalFlow::Value(Value::Record(new_fields))
                }
                other => {
                    builtin_type_err("url", "append_query", "Str or Url Record", other.type_tag())
                }
            }
        }
        other => EvalFlow::Err(RuntimeError::new(
            "R0002",
            format!("unknown url builtin `url.{other}`"),
        )),
    }
}

/// Build a `Result[Url, UrlError]`-shaped value from a URL string. The
/// returned `Ok` payload is a plain Record with `scheme`/`host`/`path`
/// fields so callers can `.scheme`, `.host`, `.path` straight off it.
fn parse_url_str(s: &str) -> EvalFlow {
    if s.is_empty() {
        let mut err = BTreeMap::new();
        err.insert("kind".to_string(), Value::Str("Invalid".to_string()));
        err.insert(
            "reason".to_string(),
            Value::Str("input must not be empty".to_string()),
        );
        return EvalFlow::Value(Value::Err_(Box::new(Value::Record(err))));
    }
    let scheme = extract_scheme(s);
    if scheme.is_empty() {
        let mut err = BTreeMap::new();
        err.insert(
            "kind".to_string(),
            Value::Str("UnsupportedScheme".to_string()),
        );
        err.insert("scheme".to_string(), Value::Str(s.to_string()));
        return EvalFlow::Value(Value::Err_(Box::new(Value::Record(err))));
    }
    if !matches!(scheme.as_str(), "http" | "https") {
        let mut err = BTreeMap::new();
        err.insert(
            "kind".to_string(),
            Value::Str("UnsupportedScheme".to_string()),
        );
        err.insert("scheme".to_string(), Value::Str(scheme));
        return EvalFlow::Value(Value::Err_(Box::new(Value::Record(err))));
    }
    let host = extract_host(s);
    let path = extract_path(s);
    let mut rec = BTreeMap::new();
    rec.insert("scheme".to_string(), Value::Str(scheme));
    rec.insert("host".to_string(), Value::Str(host));
    rec.insert("path".to_string(), Value::Str(path));
    EvalFlow::Value(Value::Ok_(Box::new(Value::Record(rec))))
}

/// Return the `scheme` portion of `s` (everything before `://`), or an
/// empty string when no scheme marker is present.
fn extract_scheme(s: &str) -> String {
    match s.find("://") {
        Some(idx) => s[..idx].to_string(),
        None => String::new(),
    }
}

/// Return the `host` portion (between `://` and the first `/`, `?`, or
/// `#`). Strips the port if present so callers compare hosts directly.
fn extract_host(s: &str) -> String {
    let after_scheme = match s.find("://") {
        Some(idx) => &s[idx + 3..],
        None => s,
    };
    let host_with_port: &str = match after_scheme.find(['/', '?', '#']) {
        Some(idx) => &after_scheme[..idx],
        None => after_scheme,
    };
    match host_with_port.find(':') {
        Some(idx) => host_with_port[..idx].to_string(),
        None => host_with_port.to_string(),
    }
}

/// Return the `path` portion (everything from the first `/` after the
/// authority to the end). Includes any query/fragment so they survive a
/// round-trip through `append_query`.
fn extract_path(s: &str) -> String {
    let after_scheme = match s.find("://") {
        Some(idx) => &s[idx + 3..],
        None => s,
    };
    match after_scheme.find('/') {
        Some(idx) => after_scheme[idx..].to_string(),
        None => String::new(),
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

    fn compile(text: &str) -> (Module, ModuleBodies) {
        let source = SourceFile::new("/t.ori", text);
        let module = parse_source(&source).module;
        let bodies = parse_module_bodies(&source);
        (module, bodies)
    }

    fn fail(msg: &str) -> ! {
        // Tests intentionally avoid `unwrap`/`expect`; this helper centralises
        // the only sanctioned panic pattern (assert!(false, ...)).
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(false, "{msg}");
        }
        unreachable!()
    }

    #[test]
    fn integer_literal_main_returns_it() {
        let (module, bodies) = compile("module a\nfn main() -> Int:\n  return 42\n");
        match exec_program(&module, &bodies, "main", Vec::new()) {
            Ok(Value::Int(42)) => {}
            Ok(other) => fail(&format!("expected Int(42), got {other:?}")),
            Err(err) => fail(&format!("unexpected runtime error: {err:?}")),
        }
    }

    #[test]
    fn empty_body_main_returns_unit() {
        // `fn main() -> Unit:` with no statements parses to a Unit literal.
        let (module, bodies) =
            compile("module a\nfn main() -> Unit:\nfn helper() -> Int:\n  return 1\n");
        match exec_program(&module, &bodies, "main", Vec::new()) {
            Ok(Value::Unit) => {}
            Ok(other) => fail(&format!("expected Unit, got {other:?}")),
            Err(err) => fail(&format!("unexpected runtime error: {err:?}")),
        }
    }

    #[test]
    fn function_calling_another_function() {
        let text = "module a\n\
                    fn inner() -> Int:\n  return 7\n\
                    fn main() -> Int:\n  return inner()\n";
        let (module, bodies) = compile(text);
        match exec_program(&module, &bodies, "main", Vec::new()) {
            Ok(Value::Int(7)) => {}
            Ok(other) => fail(&format!("expected Int(7), got {other:?}")),
            Err(err) => fail(&format!("unexpected runtime error: {err:?}")),
        }
    }

    #[test]
    fn if_else_picks_correct_branch() {
        // `cond` is `true` → take the then branch.
        let text = "module a\nfn main() -> Int:\n  if true: return 1 else: return 2\n";
        let (module, bodies) = compile(text);
        match exec_program(&module, &bodies, "main", Vec::new()) {
            Ok(Value::Int(1)) => {}
            Ok(other) => fail(&format!("expected Int(1), got {other:?}")),
            Err(err) => fail(&format!("unexpected runtime error: {err:?}")),
        }
        let text_false = "module a\nfn main() -> Int:\n  if false: return 1 else: return 2\n";
        let (module2, bodies2) = compile(text_false);
        match exec_program(&module2, &bodies2, "main", Vec::new()) {
            Ok(Value::Int(2)) => {}
            Ok(other) => fail(&format!("expected Int(2), got {other:?}")),
            Err(err) => fail(&format!("unexpected runtime error: {err:?}")),
        }
    }

    #[test]
    fn match_literal_patterns_pick_correct_arm() {
        // A match over an Int that selects the second arm.
        let text = "module a\nfn main() -> Int:\n  match 2 | 1 => 10 | 2 => 20 | _ => 99\n";
        let (module, bodies) = compile(text);
        match exec_program(&module, &bodies, "main", Vec::new()) {
            Ok(Value::Int(20)) => {}
            Ok(other) => fail(&format!("expected Int(20), got {other:?}")),
            Err(err) => fail(&format!("unexpected runtime error: {err:?}")),
        }
    }

    #[test]
    fn try_unwraps_ok_and_propagates_err() {
        // Ok branch: `?` unwraps inner.
        let text_ok = "module a\nfn main() -> Int:\n  let x = Ok(5)?\n  return x\n";
        let (module, bodies) = compile(text_ok);
        match exec_program(&module, &bodies, "main", Vec::new()) {
            Ok(Value::Int(5)) => {}
            Ok(other) => fail(&format!("expected Int(5), got {other:?}")),
            Err(err) => fail(&format!("unexpected runtime error: {err:?}")),
        }
        // Err branch: `?` short-circuits and the function returns Err(_).
        let text_err = "module a\nfn main() -> Int:\n  let x = Err(9)?\n  return x\n";
        let (module2, bodies2) = compile(text_err);
        match exec_program(&module2, &bodies2, "main", Vec::new()) {
            Ok(Value::Err_(inner)) => {
                if *inner != Value::Int(9) {
                    fail(&format!("expected Err(9), got Err({inner:?})"));
                }
            }
            Ok(other) => fail(&format!("expected Err(9), got {other:?}")),
            Err(err) => fail(&format!("unexpected runtime error: {err:?}")),
        }
    }

    #[test]
    fn construct_roundtrip_ok_err_some_none() {
        // Build each constructor and assert structural equality.
        let cases = [
            (
                "module a\nfn main() -> Int:\n  return Ok(1)\n",
                Value::Ok_(Box::new(Value::Int(1))),
            ),
            (
                "module a\nfn main() -> Int:\n  return Err(2)\n",
                Value::Err_(Box::new(Value::Int(2))),
            ),
            (
                "module a\nfn main() -> Int:\n  return Some(3)\n",
                Value::Some_(Box::new(Value::Int(3))),
            ),
            ("module a\nfn main() -> Int:\n  return None\n", Value::None_),
        ];
        for (text, want) in cases.iter() {
            let (module, bodies) = compile(text);
            match exec_program(&module, &bodies, "main", Vec::new()) {
                Ok(got) => {
                    if got != *want {
                        fail(&format!("for {text:?}: expected {want:?}, got {got:?}"));
                    }
                }
                Err(err) => fail(&format!("unexpected runtime error: {err:?}")),
            }
        }
    }

    #[test]
    fn unknown_call_returns_r0002() {
        let text = "module a\nfn main() -> Int:\n  return ghost()\n";
        let (module, bodies) = compile(text);
        match exec_program(&module, &bodies, "main", Vec::new()) {
            Err(err) => {
                if err.code != "R0002" {
                    fail(&format!("expected R0002, got {}", err.code));
                }
            }
            Ok(value) => fail(&format!("expected R0002, got Ok({value:?})")),
        }
    }

    #[test]
    fn missing_entry_returns_r0001() {
        let text = "module a\nfn helper() -> Int:\n  return 1\n";
        let (module, bodies) = compile(text);
        match exec_program(&module, &bodies, "main", Vec::new()) {
            Err(err) => {
                if err.code != "R0001" {
                    fail(&format!("expected R0001, got {}", err.code));
                }
            }
            Ok(value) => fail(&format!("expected R0001, got Ok({value:?})")),
        }
    }

    #[test]
    fn arity_mismatch_returns_r0003() {
        // `inner` takes one parameter; `main` calls it with two.
        let text = "module a\n\
                    fn inner(x: Int) -> Int:\n  return x\n\
                    fn main() -> Int:\n  return inner(1, 2)\n";
        let (module, bodies) = compile(text);
        match exec_program(&module, &bodies, "main", Vec::new()) {
            Err(err) => {
                if err.code != "R0003" {
                    fail(&format!("expected R0003, got {}", err.code));
                }
            }
            Ok(value) => fail(&format!("expected R0003, got Ok({value:?})")),
        }
    }

    #[test]
    fn type_mismatch_in_if_returns_r0004() {
        let text = "module a\nfn main() -> Int:\n  if 42: return 1 else: return 2\n";
        let (module, bodies) = compile(text);
        match exec_program(&module, &bodies, "main", Vec::new()) {
            Err(err) => {
                if err.code != "R0004" {
                    fail(&format!("expected R0004, got {}", err.code));
                }
            }
            Ok(value) => fail(&format!("expected R0004, got Ok({value:?})")),
        }
    }

    #[test]
    fn recursion_cap_returns_r0005() {
        // Direct recursion without a base case must hit the depth cap.
        let text = "module a\nfn loop_fn() -> Int:\n  return loop_fn()\n";
        let (module, bodies) = compile(text);
        match exec_program(&module, &bodies, "loop_fn", Vec::new()) {
            Err(err) => {
                if err.code != "R0005" {
                    fail(&format!("expected R0005, got {}", err.code));
                }
            }
            Ok(value) => fail(&format!("expected R0005, got Ok({value:?})")),
        }
    }

    #[test]
    fn entry_arity_mismatch_returns_r0003() {
        // `main()` takes no args, but we pass one.
        let text = "module a\nfn main() -> Int:\n  return 1\n";
        let (module, bodies) = compile(text);
        match exec_program(&module, &bodies, "main", vec![Value::Int(1)]) {
            Err(err) => {
                if err.code != "R0003" {
                    fail(&format!("expected R0003, got {}", err.code));
                }
            }
            Ok(value) => fail(&format!("expected R0003, got Ok({value:?})")),
        }
    }

    #[test]
    fn observed_effects_mirror_uses_clause_on_error() {
        // The entry declares `uses log`; even though it errors, the
        // observed_effects list should carry the declared capability.
        let text = "module a\nfn main() -> Int uses log:\n  return ghost()\n";
        let (module, bodies) = compile(text);
        match exec_program(&module, &bodies, "main", Vec::new()) {
            Err(err) => {
                if err.code != "R0002" {
                    fail(&format!("expected R0002, got {}", err.code));
                }
                if !err.observed_effects.iter().any(|e| e == "log") {
                    fail(&format!(
                        "expected `log` in observed_effects, got {:?}",
                        err.observed_effects
                    ));
                }
            }
            Ok(value) => fail(&format!("expected R0002, got Ok({value:?})")),
        }
    }

    #[test]
    fn extract_param_names_basic() {
        let names = extract_param_names("fn f(x: Int, y: Bool) -> Int");
        assert_eq!(names, vec!["x".to_string(), "y".to_string()]);
    }

    #[test]
    fn extract_param_names_no_params() {
        let names = extract_param_names("fn f() -> Int");
        assert!(names.is_empty());
    }

    // -----------------------------------------------------------------
    // Builtin dispatch tests (str / list / cache / url namespaces).
    //
    // Each test asserts that a `name(...)` call routed through the new
    // `dispatch_*_builtin` arms produces the documented value. The tests
    // use small `module a` programs so the parser drives the same code
    // path as a real stdlib body.
    // -----------------------------------------------------------------

    fn run_str(text: &str) -> Result<Value, RuntimeError> {
        let (module, bodies) = compile(text);
        exec_program(&module, &bodies, "main", Vec::new())
    }

    fn assert_value(text: &str, want: Value) {
        match run_str(text) {
            Ok(got) => {
                if got != want {
                    fail(&format!("for {text:?}: expected {want:?}, got {got:?}"));
                }
            }
            Err(err) => fail(&format!("unexpected runtime error for {text:?}: {err:?}")),
        }
    }

    // ---- str.* builtins ----

    #[test]
    fn builtin_str_len_counts_chars() {
        // Ascii: 5 chars. The builtin counts Unicode scalars, not bytes.
        assert_value(
            "module a\nfn main() -> Int:\n  return str.len(\"hello\")\n",
            Value::Int(5),
        );
    }

    #[test]
    fn builtin_str_contains_returns_bool() {
        assert_value(
            "module a\nfn main() -> Bool:\n  return str.contains(\"hello world\", \"world\")\n",
            Value::Bool(true),
        );
        assert_value(
            "module a\nfn main() -> Bool:\n  return str.contains(\"hello\", \"xyz\")\n",
            Value::Bool(false),
        );
    }

    #[test]
    fn builtin_str_starts_ends_with() {
        assert_value(
            "module a\nfn main() -> Bool:\n  return str.starts_with(\"hello\", \"he\")\n",
            Value::Bool(true),
        );
        assert_value(
            "module a\nfn main() -> Bool:\n  return str.ends_with(\"hello\", \"lo\")\n",
            Value::Bool(true),
        );
        assert_value(
            "module a\nfn main() -> Bool:\n  return str.starts_with(\"hello\", \"lo\")\n",
            Value::Bool(false),
        );
    }

    #[test]
    fn builtin_str_split_returns_list() {
        assert_value(
            "module a\nfn main() -> Int:\n  return str.split(\"a,b,c\", \",\")\n",
            Value::List(vec![
                Value::Str("a".into()),
                Value::Str("b".into()),
                Value::Str("c".into()),
            ]),
        );
    }

    #[test]
    fn builtin_str_case_conversion() {
        assert_value(
            "module a\nfn main() -> Str:\n  return str.to_lower(\"HeLLo\")\n",
            Value::Str("hello".into()),
        );
        assert_value(
            "module a\nfn main() -> Str:\n  return str.to_upper(\"hello\")\n",
            Value::Str("HELLO".into()),
        );
    }

    #[test]
    fn builtin_str_join_concatenates() {
        // Build a List of Str literals via the existing `List` constructor.
        let text = "module a\nfn main() -> Str:\n  let parts = List(\"a\", \"b\", \"c\")\n  return str.join(parts, \"-\")\n";
        assert_value(text, Value::Str("a-b-c".into()));
    }

    #[test]
    fn builtin_string_alias_routes_to_str_namespace() {
        // `string.len(...)` is treated as an alias for `str.len(...)`
        // so existing stdlib bodies referencing `string.*` keep working.
        assert_value(
            "module a\nfn main() -> Int:\n  return string.len(\"ori\")\n",
            Value::Int(3),
        );
    }

    // ---- list.* builtins ----

    #[test]
    fn builtin_list_len_and_is_empty() {
        let text = "module a\nfn main() -> Int:\n  let xs = List(1, 2, 3)\n  return list.len(xs)\n";
        assert_value(text, Value::Int(3));
        let text_empty =
            "module a\nfn main() -> Bool:\n  let xs = List()\n  return list.is_empty(xs)\n";
        assert_value(text_empty, Value::Bool(true));
    }

    #[test]
    fn builtin_list_push_pop_roundtrip() {
        // push then pop returns Some(item) of the same value.
        let text = "module a\nfn main() -> Int:\n  let xs = List(1, 2)\n  let ys = list.push(xs, 9)\n  return list.len(ys)\n";
        assert_value(text, Value::Int(3));

        let text_pop =
            "module a\nfn main() -> Int:\n  let xs = List(1, 2, 7)\n  return list.pop(xs)\n";
        assert_value(text_pop, Value::Some_(Box::new(Value::Int(7))));

        let text_empty = "module a\nfn main() -> Int:\n  let xs = List()\n  return list.pop(xs)\n";
        assert_value(text_empty, Value::None_);
    }

    #[test]
    fn builtin_list_map_applies_lambda() {
        // `fn (x) => x` echoes inputs.
        let text = "module a\nfn main() -> Int:\n  let xs = List(1, 2, 3)\n  return list.map(xs, fn (x) => x)\n";
        assert_value(
            text,
            Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
        );
    }

    #[test]
    fn builtin_list_filter_keeps_true_predicate_items() {
        // Predicate that always returns true: keeps every element.
        let text = "module a\nfn main() -> Int:\n  let xs = List(10, 20)\n  return list.filter(xs, fn (x) => true)\n";
        assert_value(text, Value::List(vec![Value::Int(10), Value::Int(20)]));

        // Predicate that always returns false: drops every element.
        let text_none = "module a\nfn main() -> Int:\n  let xs = List(10, 20)\n  return list.filter(xs, fn (x) => false)\n";
        assert_value(text_none, Value::List(vec![]));
    }

    // ---- cache.* builtins ----

    #[test]
    fn builtin_cache_new_len_clear() {
        let text = "module a\nfn main() -> Int:\n  let c = cache.new(4)\n  return cache.len(c)\n";
        assert_value(text, Value::Int(0));

        let text_clear = "module a\nfn main() -> Int:\n  let c = cache.new(4)\n  let c2 = cache.put(c, \"k\", 1)\n  let c3 = cache.clear(c2)\n  return cache.len(c3)\n";
        assert_value(text_clear, Value::Int(0));
    }

    #[test]
    fn builtin_cache_put_and_get() {
        let text = "module a\nfn main() -> Int:\n  let c = cache.new(4)\n  let c2 = cache.put(c, \"k\", 42)\n  return cache.get(c2, \"k\")\n";
        assert_value(text, Value::Some_(Box::new(Value::Int(42))));

        let text_miss =
            "module a\nfn main() -> Int:\n  let c = cache.new(4)\n  return cache.get(c, \"k\")\n";
        assert_value(text_miss, Value::None_);
    }

    #[test]
    fn builtin_cache_evicts_fifo_when_full() {
        // Capacity 2: insert k1, k2, k3 — k1 should be evicted.
        let text = "module a\nfn main() -> Int:\n  let c = cache.new(2)\n  let a = cache.put(c, \"k1\", 1)\n  let b = cache.put(a, \"k2\", 2)\n  let d = cache.put(b, \"k3\", 3)\n  return cache.get(d, \"k1\")\n";
        assert_value(text, Value::None_);

        let text_k3 = "module a\nfn main() -> Int:\n  let c = cache.new(2)\n  let a = cache.put(c, \"k1\", 1)\n  let b = cache.put(a, \"k2\", 2)\n  let d = cache.put(b, \"k3\", 3)\n  return cache.get(d, \"k3\")\n";
        assert_value(text_k3, Value::Some_(Box::new(Value::Int(3))));
    }

    // ---- url.* builtins ----

    #[test]
    fn builtin_url_parse_http_populates_fields() {
        // Ok arm: scheme/host/path are all populated.
        let text = "module a\nfn main() -> Str:\n  return url.parse(\"http://example.com/path\")\n";
        let mut want_rec = BTreeMap::new();
        want_rec.insert("scheme".into(), Value::Str("http".into()));
        want_rec.insert("host".into(), Value::Str("example.com".into()));
        want_rec.insert("path".into(), Value::Str("/path".into()));
        assert_value(text, Value::Ok_(Box::new(Value::Record(want_rec))));
    }

    #[test]
    fn builtin_url_parse_https_with_port_and_query() {
        let text = "module a\nfn main() -> Str:\n  return url.parse(\"https://example.com:443/path?q=1\")\n";
        let mut want_rec = BTreeMap::new();
        want_rec.insert("scheme".into(), Value::Str("https".into()));
        want_rec.insert("host".into(), Value::Str("example.com".into()));
        want_rec.insert("path".into(), Value::Str("/path?q=1".into()));
        assert_value(text, Value::Ok_(Box::new(Value::Record(want_rec))));
    }

    #[test]
    fn builtin_url_parse_empty_returns_err_invalid() {
        let text = "module a\nfn main() -> Str:\n  return url.parse(\"\")\n";
        let mut want_err = BTreeMap::new();
        want_err.insert("kind".into(), Value::Str("Invalid".into()));
        want_err.insert(
            "reason".into(),
            Value::Str("input must not be empty".into()),
        );
        assert_value(text, Value::Err_(Box::new(Value::Record(want_err))));
    }

    #[test]
    fn builtin_url_append_query_str_form() {
        assert_value(
            "module a\nfn main() -> Str:\n  return url.append_query(\"http://x/y\", \"a\", \"1\")\n",
            Value::Str("http://x/y?a=1".into()),
        );
        assert_value(
            "module a\nfn main() -> Str:\n  return url.append_query(\"http://x/y?b=2\", \"a\", \"1\")\n",
            Value::Str("http://x/y?b=2&a=1".into()),
        );
    }

    #[test]
    fn builtin_url_helpers_scheme_host_path() {
        assert_value(
            "module a\nfn main() -> Str:\n  return url.scheme_of(\"https://x.example/y\")\n",
            Value::Str("https".into()),
        );
        assert_value(
            "module a\nfn main() -> Str:\n  return url.host_of(\"https://x.example:8080/y\")\n",
            Value::Str("x.example".into()),
        );
        assert_value(
            "module a\nfn main() -> Str:\n  return url.path_of(\"https://x.example/y/z?q=1\")\n",
            Value::Str("/y/z?q=1".into()),
        );
    }

    // ---- Error surface coverage ----

    #[test]
    fn builtin_unknown_method_returns_r0002() {
        let text = "module a\nfn main() -> Int:\n  return str.bogus(\"x\")\n";
        match run_str(text) {
            Err(err) if err.code == "R0002" => {}
            other => fail(&format!("expected R0002, got {other:?}")),
        }
    }

    #[test]
    fn builtin_wrong_arity_returns_r0003() {
        let text = "module a\nfn main() -> Int:\n  return str.len(\"a\", \"b\")\n";
        match run_str(text) {
            Err(err) if err.code == "R0003" => {}
            other => fail(&format!("expected R0003, got {other:?}")),
        }
    }

    #[test]
    fn builtin_wrong_type_returns_r0004() {
        // str.len on an Int should be a type error.
        let text = "module a\nfn main() -> Int:\n  return str.len(7)\n";
        match run_str(text) {
            Err(err) if err.code == "R0004" => {}
            other => fail(&format!("expected R0004, got {other:?}")),
        }
    }

    // ---- Stdlib integration tests: parse small .ori snippets that
    //      exercise the new builtins end-to-end through the same parser
    //      path the real stdlib uses. ----

    #[test]
    fn stdlib_integration_str_len_via_module() {
        // Mirrors how `core.string.len` will body-delegate to `str.len`.
        let text = "module core.string\nfn len(s: Str) -> Int:\n  return str.len(s)\nfn main() -> Int:\n  return len(\"orison\")\n";
        let (module, bodies) = compile(text);
        match exec_program(&module, &bodies, "main", Vec::new()) {
            Ok(Value::Int(6)) => {}
            other => fail(&format!("expected Int(6), got {other:?}")),
        }
    }

    #[test]
    fn stdlib_integration_list_map_via_module() {
        let text = "module core.list\nfn double(xs: List[Int]) -> List[Int]:\n  return list.map(xs, fn (x) => x)\nfn main() -> Int:\n  let xs = List(1, 2, 3)\n  return list.len(double(xs))\n";
        let (module, bodies) = compile(text);
        match exec_program(&module, &bodies, "main", Vec::new()) {
            Ok(Value::Int(3)) => {}
            other => fail(&format!("expected Int(3), got {other:?}")),
        }
    }

    #[test]
    fn stdlib_integration_cache_roundtrip_via_module() {
        let text = "module std.cache\nfn warm() -> Int:\n  let c = cache.new(8)\n  let c2 = cache.put(c, \"a\", 1)\n  return cache.get(c2, \"a\")\nfn main() -> Int:\n  return warm()\n";
        let (module, bodies) = compile(text);
        match exec_program(&module, &bodies, "main", Vec::new()) {
            Ok(Value::Some_(inner)) if *inner == Value::Int(1) => {}
            other => fail(&format!("expected Some(1), got {other:?}")),
        }
    }

    #[test]
    fn stdlib_integration_url_helpers_via_module() {
        let text = "module std.url\nfn scheme(u: Str) -> Str:\n  return url.scheme_of(u)\nfn main() -> Str:\n  return scheme(\"https://example.com/y\")\n";
        let (module, bodies) = compile(text);
        match exec_program(&module, &bodies, "main", Vec::new()) {
            Ok(Value::Str(s)) if s == "https" => {}
            other => fail(&format!("expected Str(\"https\"), got {other:?}")),
        }
    }

    #[test]
    fn stdlib_integration_str_join_via_module() {
        let text = "module core.string\nfn join(xs: List[Str], sep: Str) -> Str:\n  return str.join(xs, sep)\nfn main() -> Str:\n  let xs = List(\"a\", \"b\")\n  return join(xs, \":\")\n";
        let (module, bodies) = compile(text);
        match exec_program(&module, &bodies, "main", Vec::new()) {
            Ok(Value::Str(s)) if s == "a:b" => {}
            other => fail(&format!("expected Str(\"a:b\"), got {other:?}")),
        }
    }

    #[test]
    fn stdlib_integration_url_append_query_via_module() {
        let text = "module std.url\nfn add(u: Str, k: Str, v: Str) -> Str:\n  return url.append_query(u, k, v)\nfn main() -> Str:\n  return add(\"http://x/y\", \"q\", \"1\")\n";
        let (module, bodies) = compile(text);
        match exec_program(&module, &bodies, "main", Vec::new()) {
            Ok(Value::Str(s)) if s == "http://x/y?q=1" => {}
            other => fail(&format!("expected Str(\"http://x/y?q=1\"), got {other:?}")),
        }
    }
}
