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

const MAX_CALL_DEPTH: usize = 256;

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

        Expr::Field { .. } => EvalFlow::Err(RuntimeError::new(
            "R0004",
            "field access is not supported in the bootstrap interpreter",
        )),

        Expr::Tuple(_) => EvalFlow::Err(RuntimeError::new(
            "R0004",
            "tuple literals are not supported in the bootstrap interpreter",
        )),

        Expr::Record { .. } => EvalFlow::Err(RuntimeError::new(
            "R0004",
            "record literals are not supported in the bootstrap interpreter",
        )),

        Expr::Lambda { .. } => EvalFlow::Err(RuntimeError::new(
            "R0004",
            "first-class lambdas are not supported in the bootstrap interpreter",
        )),

        Expr::Error => EvalFlow::Err(RuntimeError::new(
            "R0004",
            "encountered Expr::Error recovery node during execution",
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
        other => EvalFlow::Err(RuntimeError::new(
            "R0004",
            format!("unknown constructor `{other}` in bootstrap interpreter"),
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
}
