//! Runtime dispatch table for the Orison HTTP backend (M28 + M28b).
//!
//! `backend_dispatch` consumes the same surface AST as
//! [`crate::openapi`] but produces a *runtime* lookup keyed by
//! `(method, path)`. M28b extends the M28 dispatcher with:
//!
//! * Path-parameter patterns (`/users/:id`, `/orgs/:org/repos/:repo`).
//! * A before/after middleware chain with short-circuit semantics.
//! * A CORS preflight helper and middleware factory.
//! * A configurable trailing-slash policy (`Strict` / `Lenient`).
//!
//! ## Design
//!
//! * **Deterministic.** All maps are `BTreeMap` / `BTreeSet`; the JSON
//!   envelope produced by [`dispatch_report_json`] sorts every output
//!   so byte-for-byte stability is testable.
//! * **No panics.** Every public entry point returns a `Result` or
//!   builds a value.
//! * **Capability check.** Every effect on a route handler is treated
//!   as a capability the calling principal must hold. `http` is
//!   considered a transport-level effect (not a capability).

use crate::ast::{Module, SymbolKind};
use crate::json::to_json;
use serde::Serialize;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

/// Stable schema id for the dispatch-report envelope (M28).
pub const BACKEND_DISPATCH_SCHEMA: &str = "ori.backend_dispatch.v1";

/// Stable schema id for the dispatch-report envelope (M28b).
pub const BACKEND_DISPATCH_SCHEMA_V2: &str = "ori.backend_dispatch.v2";

/// Transport-level effects that are *not* treated as capabilities.
const TRANSPORT_EFFECTS: &[&str] = &["http"];

/// One route as understood by the runtime dispatcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RouteSpec {
    /// Stable handler symbol id (`sym:<module>.<name>`).
    pub symbol_id: String,
    /// HTTP method in upper-case (`GET`, `POST`, ...).
    pub method: String,
    /// Canonical request path (`/users` or `/users/:id`).
    pub path: String,
    /// Sorted effects copied from the handler symbol.
    pub effects: Vec<String>,
    /// `true` when at least one effect is treated as a capability.
    pub principal_required: bool,
}

/// Parsed view of a path pattern. Computed once at table-build time
/// so per-request matching is allocation-light.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PathPattern {
    segments: Vec<PathSegment>,
    trailing_slash: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PathSegment {
    Literal(String),
    Param(String),
}

impl PathPattern {
    fn parse(raw: &str) -> Self {
        let trailing_slash = raw.len() > 1 && raw.ends_with('/');
        let mut segments = Vec::new();
        for piece in raw.split('/') {
            if piece.is_empty() {
                continue;
            }
            if let Some(rest) = piece.strip_prefix(':') {
                segments.push(PathSegment::Param(rest.to_string()));
            } else {
                segments.push(PathSegment::Literal(piece.to_string()));
            }
        }
        Self {
            segments,
            trailing_slash,
        }
    }

    /// Canonical, parameter-name-agnostic shape used for conflict
    /// detection. `/users/:id` and `/users/:user_id` both collapse
    /// to `/users/:`.
    fn shape(&self) -> String {
        let mut out = String::with_capacity(self.segments.len() * 4);
        for seg in &self.segments {
            out.push('/');
            match seg {
                PathSegment::Literal(s) => out.push_str(s),
                PathSegment::Param(_) => out.push(':'),
            }
        }
        if out.is_empty() {
            out.push('/');
        }
        out
    }

    fn match_path(&self, request_path: &str) -> Option<BTreeMap<String, String>> {
        let req_trailing = request_path.len() > 1 && request_path.ends_with('/');
        if req_trailing != self.trailing_slash {
            return None;
        }
        let req_segments: Vec<&str> = request_path.split('/').filter(|s| !s.is_empty()).collect();
        if req_segments.len() != self.segments.len() {
            return None;
        }
        let mut params: BTreeMap<String, String> = BTreeMap::new();
        for (pat, got) in self.segments.iter().zip(req_segments.iter()) {
            match pat {
                PathSegment::Literal(lit) => {
                    if lit != got {
                        return None;
                    }
                }
                PathSegment::Param(name) => {
                    params.insert(name.clone(), (*got).to_string());
                }
            }
        }
        Some(params)
    }
}

/// Trailing-slash handling policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SlashPolicy {
    /// `/users` and `/users/` are distinct routes.
    #[default]
    Strict,
    /// `/users/` redirects to `/users` (HTTP 308).
    Lenient,
}

/// The full dispatch table.
#[derive(Debug, Clone, Default)]
pub struct DispatchTable {
    routes: BTreeMap<(String, String), RouteSpec>,
    patterns: BTreeMap<String, PathPattern>,
    /// Ordered middleware chain.
    pub middleware: Vec<Middleware>,
    /// Trailing-slash policy applied during [`dispatch`].
    pub slash_policy: SlashPolicy,
}

#[derive(Serialize)]
struct DispatchTableJson<'a> {
    routes: Vec<&'a RouteSpec>,
}

impl Serialize for DispatchTable {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let payload = DispatchTableJson {
            routes: self.routes.values().collect(),
        };
        payload.serialize(serializer)
    }
}

impl DispatchTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.routes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &RouteSpec> {
        self.routes.values()
    }

    pub fn get(&self, method: &str, path: &str) -> Option<&RouteSpec> {
        self.routes.get(&(method.to_string(), path.to_string()))
    }

    /// Collect every method registered for `path` (exact-string match).
    pub fn methods_for_path(&self, path: &str) -> Vec<String> {
        let mut methods: Vec<String> = self
            .routes
            .iter()
            .filter(|((_m, p), _)| p == path)
            .map(|((m, _p), _)| m.clone())
            .collect();
        methods.sort();
        methods.dedup();
        methods
    }

    pub fn insert(&mut self, route: RouteSpec) -> Option<RouteSpec> {
        let pattern = PathPattern::parse(&route.path);
        self.patterns.insert(route.path.clone(), pattern);
        let key = (route.method.clone(), route.path.clone());
        self.routes.insert(key, route)
    }

    /// Collect methods whose path *pattern* matches the literal
    /// request path. Used by [`dispatch`].
    fn methods_matching(&self, request_path: &str) -> Vec<String> {
        let mut methods: BTreeSet<String> = BTreeSet::new();
        for (method, path) in self.routes.keys() {
            if let Some(pattern) = self.patterns.get(path) {
                if pattern.match_path(request_path).is_some() {
                    methods.insert(method.clone());
                }
            } else if path == request_path {
                methods.insert(method.clone());
            }
        }
        methods.into_iter().collect()
    }

    /// Insert a route, rejecting clashes with already-registered
    /// patterns of the same shape (`/users/:id` vs `/users/:user_id`).
    pub fn insert_route(&mut self, route: RouteSpec) -> Result<(), DispatchError> {
        let new_pattern = PathPattern::parse(&route.path);
        let new_shape = new_pattern.shape();
        for (existing_method, existing_path) in self.routes.keys() {
            if existing_method != &route.method {
                continue;
            }
            if existing_path == &route.path {
                continue;
            }
            let existing_pattern = match self.patterns.get(existing_path) {
                Some(p) => p.clone(),
                None => PathPattern::parse(existing_path),
            };
            if existing_pattern.shape() == new_shape {
                return Err(DispatchError::ConflictingRoute {
                    existing: existing_path.clone(),
                    new: route.path.clone(),
                });
            }
        }
        self.patterns.insert(route.path.clone(), new_pattern);
        let key = (route.method.clone(), route.path.clone());
        self.routes.insert(key, route);
        Ok(())
    }

    pub fn with_middleware(mut self, mw: Middleware) -> Self {
        self.middleware.push(mw);
        self
    }

    pub fn with_slash_policy(mut self, policy: SlashPolicy) -> Self {
        self.slash_policy = policy;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Principal {
    pub id: String,
    pub capabilities: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub principal: Option<Principal>,
    pub body: Vec<u8>,
    pub headers: BTreeMap<String, String>,
    pub path_params: BTreeMap<String, String>,
}

impl Request {
    pub fn new(method: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            path: path.into(),
            principal: None,
            body: Vec::new(),
            headers: BTreeMap::new(),
            path_params: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    pub headers: BTreeMap<String, String>,
}

/// Middleware kind — `Before` runs before the handler, `After` after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MiddlewareKind {
    Before,
    After,
}

/// The decision a middleware returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MiddlewareOutcome {
    Continue,
    Short { response: Response },
}

pub type MiddlewareFn = fn(&Request, Option<&Response>) -> MiddlewareOutcome;

#[derive(Debug, Clone)]
pub struct Middleware {
    pub name: String,
    pub kind: MiddlewareKind,
    pub handler: MiddlewareFn,
}

impl PartialEq for Middleware {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.kind == other.kind
            && (self.handler as usize) == (other.handler as usize)
    }
}

impl Eq for Middleware {}

/// Lint diagnostics. R0050/R0051/R0052.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchDiagnostic {
    ConflictingRoute { existing: String, new: String },
    PathParamUnused { symbol_id: String, param: String },
    RouteMatchesNoMethod { path: String },
}

impl DispatchDiagnostic {
    pub fn rule(&self) -> &'static str {
        match self {
            DispatchDiagnostic::ConflictingRoute { .. } => "R0050",
            DispatchDiagnostic::PathParamUnused { .. } => "R0051",
            DispatchDiagnostic::RouteMatchesNoMethod { .. } => "R0052",
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            DispatchDiagnostic::ConflictingRoute { .. } => "conflicting_route",
            DispatchDiagnostic::PathParamUnused { .. } => "path_param_unused",
            DispatchDiagnostic::RouteMatchesNoMethod { .. } => "route_matches_no_method",
        }
    }
}

pub fn lint_dispatch_table(table: &DispatchTable) -> Vec<DispatchDiagnostic> {
    let mut out: Vec<DispatchDiagnostic> = Vec::new();
    for ((_method, path), route) in &table.routes {
        let pattern = match table.patterns.get(path) {
            Some(p) => p,
            None => continue,
        };
        for seg in &pattern.segments {
            if let PathSegment::Param(name) = seg {
                if !route.symbol_id.contains(name) {
                    out.push(DispatchDiagnostic::PathParamUnused {
                        symbol_id: route.symbol_id.clone(),
                        param: name.clone(),
                    });
                }
            }
        }
    }
    for path in table.patterns.keys() {
        let any = table.routes.keys().any(|(_m, p)| p == path);
        if !any {
            out.push(DispatchDiagnostic::RouteMatchesNoMethod { path: path.clone() });
        }
    }
    out.sort_by(|a, b| {
        let ra = a.rule();
        let rb = b.rule();
        ra.cmp(rb)
            .then_with(|| format!("{a:?}").cmp(&format!("{b:?}")))
    });
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchError {
    NotFound,
    MethodNotAllowed { allowed: Vec<String> },
    MissingPrincipal,
    MissingCapability { needed: String },
    ConflictingRoute { existing: String, new: String },
}

impl DispatchError {
    pub fn kind(&self) -> &'static str {
        match self {
            DispatchError::NotFound => "not_found",
            DispatchError::MethodNotAllowed { .. } => "method_not_allowed",
            DispatchError::MissingPrincipal => "missing_principal",
            DispatchError::MissingCapability { .. } => "missing_capability",
            DispatchError::ConflictingRoute { .. } => "conflicting_route",
        }
    }
}

pub fn build_dispatch_table(module: &Module) -> DispatchTable {
    let mut table = DispatchTable::new();
    let mut handlers: Vec<&crate::ast::Symbol> = module
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Function)
        .filter(|s| {
            s.effects
                .iter()
                .any(|e| e == "http" || e.starts_with("http."))
        })
        .collect();
    handlers.sort_by(|a, b| a.id.cmp(&b.id));
    for sym in handlers {
        let method = infer_method(sym.name.as_str(), &sym.effects).to_string();
        let path = infer_path(sym.name.as_str());
        let mut effects = sym.effects.clone();
        effects.sort();
        effects.dedup();
        let principal_required = effects
            .iter()
            .any(|e| !TRANSPORT_EFFECTS.contains(&e.as_str()));
        let route = RouteSpec {
            symbol_id: sym.id.clone(),
            method,
            path,
            effects,
            principal_required,
        };
        table.insert(route);
    }
    table
}

pub fn dispatch(table: &DispatchTable, req: &Request) -> Result<Response, DispatchError> {
    let method = req.method.to_uppercase();
    if let Some(redirect) = lenient_redirect(table, &req.path) {
        return Ok(redirect);
    }
    let methods = table.methods_matching(&req.path);
    if methods.is_empty() {
        return Err(DispatchError::NotFound);
    }
    let route = match find_matching_route(table, &method, &req.path) {
        Some(r) => r,
        None => {
            return Err(DispatchError::MethodNotAllowed { allowed: methods });
        }
    };
    let pattern = table.patterns.get(&route.path);
    let path_params = match pattern.and_then(|p| p.match_path(&req.path)) {
        Some(p) => p,
        None => req.path_params.clone(),
    };
    let mut working_req = req.clone();
    working_req.method = method;
    working_req.path_params = path_params;

    let mut short_response: Option<Response> = None;
    for mw in &table.middleware {
        if mw.kind != MiddlewareKind::Before {
            continue;
        }
        match (mw.handler)(&working_req, None) {
            MiddlewareOutcome::Continue => {}
            MiddlewareOutcome::Short { response } => {
                short_response = Some(response);
                break;
            }
        }
    }
    let mut response = if let Some(resp) = short_response {
        resp
    } else {
        if route.principal_required && working_req.principal.is_none() {
            return Err(DispatchError::MissingPrincipal);
        }
        if let Some(principal) = working_req.principal.as_ref() {
            for effect in &route.effects {
                if TRANSPORT_EFFECTS.contains(&effect.as_str()) {
                    continue;
                }
                if !principal.capabilities.contains(effect) {
                    return Err(DispatchError::MissingCapability {
                        needed: effect.clone(),
                    });
                }
            }
        }
        let body = format!(r#"{{"dispatched":"{}"}}"#, route.symbol_id).into_bytes();
        Response {
            status: 200,
            body,
            headers: BTreeMap::new(),
        }
    };
    for mw in table.middleware.iter().rev() {
        if mw.kind != MiddlewareKind::After {
            continue;
        }
        match (mw.handler)(&working_req, Some(&response)) {
            MiddlewareOutcome::Continue => {}
            MiddlewareOutcome::Short { response: new_resp } => {
                response = new_resp;
            }
        }
    }
    Ok(response)
}

fn lenient_redirect(table: &DispatchTable, path: &str) -> Option<Response> {
    if table.slash_policy != SlashPolicy::Lenient {
        return None;
    }
    if path.len() <= 1 || !path.ends_with('/') {
        return None;
    }
    let canonical = path.trim_end_matches('/').to_string();
    let exists = table.routes.keys().any(|(_m, p)| p == &canonical)
        || table
            .patterns
            .iter()
            .any(|(stored, pattern)| stored != path && pattern.match_path(&canonical).is_some());
    if !exists {
        return None;
    }
    let mut headers = BTreeMap::new();
    headers.insert("Location".to_string(), canonical);
    Some(Response {
        status: 308,
        body: Vec::new(),
        headers,
    })
}

fn find_matching_route<'a>(
    table: &'a DispatchTable,
    method: &str,
    request_path: &str,
) -> Option<&'a RouteSpec> {
    if let Some(route) = table
        .routes
        .get(&(method.to_string(), request_path.to_string()))
    {
        return Some(route);
    }
    let mut best: Option<(&RouteSpec, usize)> = None;
    for ((m, path), route) in &table.routes {
        if m != method {
            continue;
        }
        let pattern = match table.patterns.get(path) {
            Some(p) => p,
            None => continue,
        };
        if pattern.match_path(request_path).is_some() {
            let param_count = pattern
                .segments
                .iter()
                .filter(|s| matches!(s, PathSegment::Param(_)))
                .count();
            let take = match best {
                None => true,
                Some((_, count)) => param_count < count,
            };
            if take {
                best = Some((route, param_count));
            }
        }
    }
    best.map(|(r, _)| r)
}

#[derive(Debug, Serialize)]
struct DispatchReport<'a> {
    schema: &'static str,
    route_count: usize,
    routes: Vec<&'a RouteSpec>,
}

pub fn dispatch_report_json(table: &DispatchTable) -> String {
    let routes: Vec<&RouteSpec> = table.routes.values().collect();
    let report = DispatchReport {
        schema: BACKEND_DISPATCH_SCHEMA,
        route_count: routes.len(),
        routes,
    };
    to_json(&report)
}

#[derive(Debug, Serialize)]
struct DispatchReportV2<'a> {
    schema: &'static str,
    route_count: usize,
    routes: Vec<&'a RouteSpec>,
    middleware: Vec<MiddlewareJson>,
    slash_policy: &'static str,
}

#[derive(Debug, Serialize)]
struct MiddlewareJson {
    name: String,
    kind: &'static str,
}

pub fn dispatch_report_json_v2(table: &DispatchTable) -> String {
    let routes: Vec<&RouteSpec> = table.routes.values().collect();
    let mut middleware: Vec<MiddlewareJson> = table
        .middleware
        .iter()
        .map(|mw| MiddlewareJson {
            name: mw.name.clone(),
            kind: match mw.kind {
                MiddlewareKind::Before => "before",
                MiddlewareKind::After => "after",
            },
        })
        .collect();
    middleware.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.kind.cmp(b.kind)));
    let slash_policy = match table.slash_policy {
        SlashPolicy::Strict => "strict",
        SlashPolicy::Lenient => "lenient",
    };
    let report = DispatchReportV2 {
        schema: BACKEND_DISPATCH_SCHEMA_V2,
        route_count: routes.len(),
        routes,
        middleware,
        slash_policy,
    };
    to_json(&report)
}

fn infer_method(name: &str, effects: &[String]) -> &'static str {
    let lower = name.to_lowercase();
    if lower.starts_with("get_") || lower.starts_with("list_") || lower == "index" {
        "GET"
    } else if lower.starts_with("post_") || lower.starts_with("create_") {
        "POST"
    } else if lower.starts_with("put_") || lower.starts_with("update_") {
        "PUT"
    } else if lower.starts_with("delete_") || lower.starts_with("remove_") {
        "DELETE"
    } else if lower.starts_with("patch_") {
        "PATCH"
    } else if effects.iter().any(|e| e == "db.write") {
        "POST"
    } else {
        "GET"
    }
}

fn infer_path(name: &str) -> String {
    let trimmed = name
        .trim_start_matches("get_")
        .trim_start_matches("list_")
        .trim_start_matches("post_")
        .trim_start_matches("put_")
        .trim_start_matches("patch_")
        .trim_start_matches("delete_")
        .trim_start_matches("create_")
        .trim_start_matches("update_")
        .trim_start_matches("remove_");
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", trimmed.replace('_', "/"))
    }
}

// =========================================================================
// CORS helpers
// =========================================================================

/// Build a 204 preflight response for an allowed origin, or 403 if not.
pub fn preflight_response(
    origin: &str,
    allowed_origins: &[String],
    allowed_methods: &[String],
) -> Response {
    let origin_allowed =
        allowed_origins.iter().any(|o| o == origin) || allowed_origins.iter().any(|o| o == "*");
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    headers.insert("Vary".to_string(), "Origin".to_string());
    if !origin_allowed {
        return Response {
            status: 403,
            body: Vec::new(),
            headers,
        };
    }
    headers.insert(
        "Access-Control-Allow-Origin".to_string(),
        origin.to_string(),
    );
    let mut methods = allowed_methods.to_vec();
    methods.sort();
    methods.dedup();
    headers.insert(
        "Access-Control-Allow-Methods".to_string(),
        methods.join(","),
    );
    Response {
        status: 204,
        body: Vec::new(),
        headers,
    }
}

thread_local! {
    static CORS_ALLOWED_ORIGINS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static CORS_ALLOWED_METHODS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn cors_before_handler(req: &Request, _resp: Option<&Response>) -> MiddlewareOutcome {
    if !req.method.eq_ignore_ascii_case("OPTIONS") {
        return MiddlewareOutcome::Continue;
    }
    let origin = req
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("Origin"))
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    let origins = CORS_ALLOWED_ORIGINS.with(|cell| cell.borrow().clone());
    let methods = CORS_ALLOWED_METHODS.with(|cell| cell.borrow().clone());
    let response = preflight_response(&origin, &origins, &methods);
    MiddlewareOutcome::Short { response }
}

fn cors_after_handler(req: &Request, resp: Option<&Response>) -> MiddlewareOutcome {
    let base = match resp {
        Some(r) => r.clone(),
        None => return MiddlewareOutcome::Continue,
    };
    let origin = match req
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("Origin"))
    {
        Some((_, v)) => v.clone(),
        None => return MiddlewareOutcome::Continue,
    };
    let origins = CORS_ALLOWED_ORIGINS.with(|cell| cell.borrow().clone());
    let allowed = origins.iter().any(|o| o == &origin) || origins.iter().any(|o| o == "*");
    if !allowed {
        return MiddlewareOutcome::Continue;
    }
    let mut headers = base.headers.clone();
    headers.insert("Access-Control-Allow-Origin".to_string(), origin);
    headers.insert("Vary".to_string(), "Origin".to_string());
    MiddlewareOutcome::Short {
        response: Response {
            status: base.status,
            body: base.body.clone(),
            headers,
        },
    }
}

pub fn cors_middleware(allowed_origins: Vec<String>, allowed_methods: Vec<String>) -> Middleware {
    let mut origins = allowed_origins;
    origins.sort();
    origins.dedup();
    let mut methods = allowed_methods;
    methods.sort();
    methods.dedup();
    CORS_ALLOWED_ORIGINS.with(|cell| *cell.borrow_mut() = origins);
    CORS_ALLOWED_METHODS.with(|cell| *cell.borrow_mut() = methods);
    Middleware {
        name: "cors".to_string(),
        kind: MiddlewareKind::Before,
        handler: cors_before_handler,
    }
}

pub fn cors_after_middleware() -> Middleware {
    Middleware {
        name: "cors".to_string(),
        kind: MiddlewareKind::After,
        handler: cors_after_handler,
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

    fn module_from_file(rel_path: &str) -> Module {
        let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = match crate_dir.ancestors().nth(2) {
            Some(p) => p.to_path_buf(),
            None => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "could not derive workspace root from crate dir");
                }
                return Module::new("demo", "/missing");
            }
        };
        let path = workspace_root.join(rel_path);
        let path_str = path.to_string_lossy().to_string();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "failed to read {path_str}: {err}");
                }
                String::new()
            }
        };
        parse_source(&SourceFile::new(&path_str, &text)).module
    }

    fn demo_module() -> Module {
        module_for(
            "module demo\nservice Api\n\
             fn get_users() -> List[User] uses http, db.read\n\
             fn post_users(u: User) -> User uses http, db.write\n\
             fn get_status() -> Int uses http\n",
        )
    }

    fn principal_with(caps: &[&str]) -> Principal {
        let mut set = BTreeSet::new();
        for cap in caps {
            set.insert((*cap).to_string());
        }
        Principal {
            id: "user:alice".to_string(),
            capabilities: set,
        }
    }

    fn route(symbol_id: &str, method: &str, path: &str, effects: &[&str]) -> RouteSpec {
        let mut effects_v: Vec<String> = effects.iter().map(|s| s.to_string()).collect();
        effects_v.sort();
        effects_v.dedup();
        let principal_required = effects_v
            .iter()
            .any(|e| !TRANSPORT_EFFECTS.contains(&e.as_str()));
        RouteSpec {
            symbol_id: symbol_id.to_string(),
            method: method.to_string(),
            path: path.to_string(),
            effects: effects_v,
            principal_required,
        }
    }

    // M28 backward-compat tests.

    #[test]
    fn builds_table_from_users_ori() {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest_dir
            .ancestors()
            .nth(2)
            .map(std::path::Path::to_path_buf)
            .unwrap_or(manifest_dir);
        let path = workspace_root.join("examples/fullstack/users.ori");
        let module = module_from_file(path.to_string_lossy().as_ref());
        let table = build_dispatch_table(&module);
        assert!(table.is_empty());
        let json = dispatch_report_json(&table);
        assert!(json.contains("\"schema\":\"ori.backend_dispatch.v1\""));
        assert!(json.contains("\"route_count\":0"));
    }

    #[test]
    fn path_miss_returns_not_found() {
        let table = build_dispatch_table(&demo_module());
        let mut req = Request::new("GET", "/nope");
        req.principal = Some(principal_with(&["db.read"]));
        assert_eq!(dispatch(&table, &req), Err(DispatchError::NotFound));
    }

    #[test]
    fn wrong_method_returns_method_not_allowed() {
        let table = build_dispatch_table(&demo_module());
        let mut req = Request::new("DELETE", "/users");
        req.principal = Some(principal_with(&["db.read", "db.write"]));
        match dispatch(&table, &req) {
            Err(DispatchError::MethodNotAllowed { allowed }) => {
                assert_eq!(allowed, vec!["GET".to_string(), "POST".to_string()]);
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected MethodNotAllowed, got {other:?}");
                }
            }
        }
    }

    #[test]
    fn missing_principal_returns_missing_principal() {
        let table = build_dispatch_table(&demo_module());
        let req = Request::new("GET", "/users");
        assert_eq!(dispatch(&table, &req), Err(DispatchError::MissingPrincipal));
    }

    #[test]
    fn missing_capability_returns_missing_capability() {
        let table = build_dispatch_table(&demo_module());
        let mut req = Request::new("GET", "/users");
        req.principal = Some(principal_with(&[]));
        match dispatch(&table, &req) {
            Err(DispatchError::MissingCapability { needed }) => {
                assert_eq!(needed, "db.read");
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected MissingCapability(db.read), got {other:?}");
                }
            }
        }
    }

    #[test]
    fn valid_request_returns_200_with_symbol_id() {
        let table = build_dispatch_table(&demo_module());
        let mut req = Request::new("GET", "/users");
        req.principal = Some(principal_with(&["db.read"]));
        let resp = match dispatch(&table, &req) {
            Ok(r) => r,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected Ok, got {err:?}");
                }
                return;
            }
        };
        assert_eq!(resp.status, 200);
        let body = String::from_utf8_lossy(&resp.body).into_owned();
        assert!(body.contains("\"dispatched\""));
        assert!(body.contains("sym:demo.get_users"));
        assert!(resp.headers.is_empty());
    }

    #[test]
    fn dispatch_report_json_parses_with_schema_field() {
        let table = build_dispatch_table(&demo_module());
        let json = dispatch_report_json(&table);
        let value: serde_json::Value = match serde_json::from_str(&json) {
            Ok(v) => v,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "report JSON did not parse: {err}");
                }
                return;
            }
        };
        assert_eq!(
            value.get("schema").and_then(|v| v.as_str()),
            Some("ori.backend_dispatch.v1")
        );
        let count = value
            .get("route_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        assert_eq!(count, 3);
    }

    #[test]
    fn dispatch_report_is_deterministic() {
        let module = demo_module();
        let json_a = dispatch_report_json(&build_dispatch_table(&module));
        let json_b = dispatch_report_json(&build_dispatch_table(&module));
        assert_eq!(json_a, json_b);
    }

    #[test]
    fn transport_only_route_does_not_require_principal() {
        let table = build_dispatch_table(&demo_module());
        let req = Request::new("GET", "/status");
        let resp = match dispatch(&table, &req) {
            Ok(r) => r,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "transport-only route should dispatch, got {err:?}");
                }
                return;
            }
        };
        assert_eq!(resp.status, 200);
    }

    #[test]
    fn methods_for_path_is_sorted_and_unique() {
        let table = build_dispatch_table(&demo_module());
        let methods = table.methods_for_path("/users");
        assert_eq!(methods, vec!["GET".to_string(), "POST".to_string()]);
    }

    #[test]
    fn method_is_case_insensitive_on_input() {
        let table = build_dispatch_table(&demo_module());
        let req = Request::new("get", "/status");
        assert!(dispatch(&table, &req).is_ok());
    }

    #[test]
    fn missing_capability_reports_first_alpha_effect() {
        let module =
            module_for("module demo\nfn post_orders(o: Order) -> Order uses http, db.write\n");
        let table = build_dispatch_table(&module);
        let mut req = Request::new("POST", "/orders");
        req.principal = Some(principal_with(&[]));
        match dispatch(&table, &req) {
            Err(DispatchError::MissingCapability { needed }) => {
                assert_eq!(needed, "db.write");
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected MissingCapability(db.write), got {other:?}");
                }
            }
        }
    }

    #[test]
    fn dispatch_error_kind_strings() {
        assert_eq!(DispatchError::NotFound.kind(), "not_found");
        assert_eq!(
            DispatchError::MethodNotAllowed { allowed: vec![] }.kind(),
            "method_not_allowed"
        );
        assert_eq!(DispatchError::MissingPrincipal.kind(), "missing_principal");
        assert_eq!(
            DispatchError::MissingCapability {
                needed: "db.read".to_string()
            }
            .kind(),
            "missing_capability"
        );
        assert_eq!(
            DispatchError::ConflictingRoute {
                existing: "/a".to_string(),
                new: "/b".to_string(),
            }
            .kind(),
            "conflicting_route"
        );
    }

    #[test]
    fn empty_module_yields_empty_table() {
        let module = module_for("module demo\nfn _hidden() -> Unit\n");
        let table = build_dispatch_table(&module);
        assert!(table.is_empty());
        let req = Request::new("GET", "/anything");
        assert_eq!(dispatch(&table, &req), Err(DispatchError::NotFound));
    }

    #[test]
    fn route_spec_principal_required_reflects_capabilities() {
        let module = module_for(
            "module demo\n\
             fn get_open() -> Int uses http\n\
             fn get_secured() -> Int uses http, db.read\n",
        );
        let table = build_dispatch_table(&module);
        let open = match table.get("GET", "/open") {
            Some(r) => r,
            None => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "missing /open route");
                }
                return;
            }
        };
        let secured = match table.get("GET", "/secured") {
            Some(r) => r,
            None => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "missing /secured route");
                }
                return;
            }
        };
        assert!(!open.principal_required);
        assert!(secured.principal_required);
    }

    // M28b path-parameter tests.

    #[test]
    fn path_param_populates_request_via_pattern() {
        let mut table = DispatchTable::new();
        let r = route("sym:demo.get_users_id", "GET", "/users/:id", &["http"]);
        match table.insert_route(r) {
            Ok(()) => {}
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "insert_route failed: {err:?}");
                }
                return;
            }
        }
        fn snapshot(req: &Request, _r: Option<&Response>) -> MiddlewareOutcome {
            let id = req.path_params.get("id").cloned().unwrap_or_default();
            let body = format!(r#"{{"captured":"{id}"}}"#).into_bytes();
            let mut headers = BTreeMap::new();
            headers.insert("X-Captured-Id".to_string(), id);
            MiddlewareOutcome::Short {
                response: Response {
                    status: 200,
                    body,
                    headers,
                },
            }
        }
        table.middleware.push(Middleware {
            name: "snapshot".to_string(),
            kind: MiddlewareKind::Before,
            handler: snapshot,
        });
        let req = Request::new("GET", "/users/42");
        let resp = match dispatch(&table, &req) {
            Ok(r) => r,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected Ok, got {err:?}");
                }
                return;
            }
        };
        assert_eq!(resp.status, 200);
        assert_eq!(resp.headers.get("X-Captured-Id"), Some(&"42".to_string()));
        let body = String::from_utf8_lossy(&resp.body).into_owned();
        assert!(body.contains(r#""captured":"42""#));
    }

    #[test]
    fn conflicting_path_params_are_rejected() {
        let mut table = DispatchTable::new();
        let r1 = route("sym:demo.h1", "GET", "/users/:id", &["http"]);
        match table.insert_route(r1) {
            Ok(()) => {}
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "first insert failed: {err:?}");
                }
                return;
            }
        }
        let r2 = route("sym:demo.h2", "GET", "/users/:user_id", &["http"]);
        match table.insert_route(r2) {
            Err(DispatchError::ConflictingRoute { existing, new }) => {
                assert_eq!(existing, "/users/:id");
                assert_eq!(new, "/users/:user_id");
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected ConflictingRoute, got {other:?}");
                }
            }
        }
    }

    #[test]
    fn before_middleware_short_circuits_handler() {
        let mut table = DispatchTable::new();
        let _ = table.insert_route(route("sym:demo.get_x", "GET", "/x", &["http"]));
        fn gate(_req: &Request, _r: Option<&Response>) -> MiddlewareOutcome {
            MiddlewareOutcome::Short {
                response: Response {
                    status: 401,
                    body: b"blocked".to_vec(),
                    headers: BTreeMap::new(),
                },
            }
        }
        table.middleware.push(Middleware {
            name: "gate".to_string(),
            kind: MiddlewareKind::Before,
            handler: gate,
        });
        let req = Request::new("GET", "/x");
        let resp = match dispatch(&table, &req) {
            Ok(r) => r,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected short-circuit Ok, got {err:?}");
                }
                return;
            }
        };
        assert_eq!(resp.status, 401);
        assert_eq!(String::from_utf8_lossy(&resp.body), "blocked");
    }

    #[test]
    fn after_middleware_mutates_response_headers() {
        let mut table = DispatchTable::new();
        let _ = table.insert_route(route("sym:demo.get_y", "GET", "/y", &["http"]));
        fn tag(_req: &Request, resp: Option<&Response>) -> MiddlewareOutcome {
            let base = match resp {
                Some(r) => r.clone(),
                None => return MiddlewareOutcome::Continue,
            };
            let mut headers = base.headers.clone();
            headers.insert("X-After-Hook".to_string(), "yes".to_string());
            MiddlewareOutcome::Short {
                response: Response {
                    status: base.status,
                    body: base.body.clone(),
                    headers,
                },
            }
        }
        table.middleware.push(Middleware {
            name: "tag".to_string(),
            kind: MiddlewareKind::After,
            handler: tag,
        });
        let req = Request::new("GET", "/y");
        let resp = match dispatch(&table, &req) {
            Ok(r) => r,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected Ok, got {err:?}");
                }
                return;
            }
        };
        assert!(resp.headers.contains_key("X-After-Hook"));
        assert_eq!(resp.headers.get("X-After-Hook"), Some(&"yes".to_string()));
    }

    #[test]
    fn cors_preflight_returns_204_with_headers() {
        let origins = vec!["https://app.example".to_string()];
        let methods = vec!["GET".to_string(), "POST".to_string()];
        let resp = preflight_response("https://app.example", &origins, &methods);
        assert_eq!(resp.status, 204);
        assert_eq!(
            resp.headers.get("Access-Control-Allow-Origin"),
            Some(&"https://app.example".to_string())
        );
        assert_eq!(
            resp.headers.get("Access-Control-Allow-Methods"),
            Some(&"GET,POST".to_string())
        );
        assert_eq!(resp.headers.get("Vary"), Some(&"Origin".to_string()));
    }

    #[test]
    fn cors_preflight_disallowed_origin_returns_403() {
        let origins = vec!["https://app.example".to_string()];
        let methods = vec!["GET".to_string()];
        let resp = preflight_response("https://evil.example", &origins, &methods);
        assert_eq!(resp.status, 403);
        assert!(!resp.headers.contains_key("Access-Control-Allow-Origin"));
    }

    #[test]
    fn strict_slash_policy_keeps_routes_distinct() {
        let mut table = DispatchTable::new();
        let _ = table.insert_route(route("sym:demo.get_users", "GET", "/users", &["http"]));
        assert_eq!(table.slash_policy, SlashPolicy::Strict);
        let req = Request::new("GET", "/users/");
        assert_eq!(dispatch(&table, &req), Err(DispatchError::NotFound));
        let req2 = Request::new("GET", "/users");
        match dispatch(&table, &req2) {
            Ok(r) => assert_eq!(r.status, 200),
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "canonical /users failed: {err:?}");
                }
            }
        }
    }

    #[test]
    fn lenient_slash_policy_redirects_to_canonical() {
        let mut table = DispatchTable::new();
        let _ = table.insert_route(route("sym:demo.get_users", "GET", "/users", &["http"]));
        table = table.with_slash_policy(SlashPolicy::Lenient);
        let req = Request::new("GET", "/users/");
        let resp = match dispatch(&table, &req) {
            Ok(r) => r,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected 308, got {err:?}");
                }
                return;
            }
        };
        assert_eq!(resp.status, 308);
        assert_eq!(resp.headers.get("Location"), Some(&"/users".to_string()));
    }

    #[test]
    fn multiple_path_params_resolve_in_order() {
        let mut table = DispatchTable::new();
        let _ = table.insert_route(route(
            "sym:demo.get_orgs_org_repos_repo",
            "GET",
            "/orgs/:org/repos/:repo",
            &["http"],
        ));
        fn snapshot(req: &Request, _r: Option<&Response>) -> MiddlewareOutcome {
            let joined: Vec<String> = req
                .path_params
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            let body = joined.join("|").into_bytes();
            MiddlewareOutcome::Short {
                response: Response {
                    status: 200,
                    body,
                    headers: BTreeMap::new(),
                },
            }
        }
        table.middleware.push(Middleware {
            name: "snapshot".to_string(),
            kind: MiddlewareKind::Before,
            handler: snapshot,
        });
        let req = Request::new("GET", "/orgs/anthropic/repos/claude-code");
        let resp = match dispatch(&table, &req) {
            Ok(r) => r,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected Ok, got {err:?}");
                }
                return;
            }
        };
        let body = String::from_utf8_lossy(&resp.body).into_owned();
        assert_eq!(body, "org=anthropic|repo=claude-code");
    }

    #[test]
    fn dispatch_is_deterministic_across_repeats() {
        let mut table = DispatchTable::new();
        let _ = table.insert_route(route("sym:demo.get_x", "GET", "/x/:id", &["http"]));
        let req = Request::new("GET", "/x/abc");
        let r1 = dispatch(&table, &req);
        let r2 = dispatch(&table, &req);
        assert_eq!(r1, r2);
    }

    #[test]
    fn cors_middleware_short_circuits_options_preflight() {
        let mut table = DispatchTable::new();
        let _ = table.insert_route(route("sym:demo.get_x", "GET", "/x", &["http"]));
        let mw = cors_middleware(
            vec!["https://app.example".to_string()],
            vec!["GET".to_string(), "POST".to_string()],
        );
        let _ = table.insert_route(route("sym:demo.opt_x", "OPTIONS", "/x", &["http"]));
        table = table.with_middleware(mw);
        let mut req = Request::new("OPTIONS", "/x");
        req.headers
            .insert("Origin".to_string(), "https://app.example".to_string());
        let resp = match dispatch(&table, &req) {
            Ok(r) => r,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected Ok 204, got {err:?}");
                }
                return;
            }
        };
        assert_eq!(resp.status, 204);
        assert_eq!(
            resp.headers.get("Access-Control-Allow-Origin"),
            Some(&"https://app.example".to_string())
        );
    }

    #[test]
    fn lint_detects_unused_path_param() {
        let mut table = DispatchTable::new();
        let _ = table.insert_route(route(
            "sym:demo.handler_without_param",
            "GET",
            "/users/:id",
            &["http"],
        ));
        let diags = lint_dispatch_table(&table);
        assert!(diags
            .iter()
            .any(|d| d.rule() == "R0051" && d.kind() == "path_param_unused"));
    }

    #[test]
    fn dispatch_report_v2_includes_middleware_and_slash_policy() {
        let mut table = DispatchTable::new();
        let _ = table.insert_route(route("sym:demo.get_x", "GET", "/x", &["http"]));
        table = table
            .with_slash_policy(SlashPolicy::Lenient)
            .with_middleware(cors_middleware(
                vec!["*".to_string()],
                vec!["GET".to_string()],
            ));
        let json = dispatch_report_json_v2(&table);
        assert!(json.contains("\"schema\":\"ori.backend_dispatch.v2\""));
        assert!(json.contains("\"slash_policy\":\"lenient\""));
        assert!(json.contains("\"name\":\"cors\""));
    }
}
