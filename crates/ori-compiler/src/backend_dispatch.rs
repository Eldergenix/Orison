//! Runtime dispatch table for the Orison HTTP backend (M28).
//!
//! `backend_dispatch` consumes the same surface AST as
//! [`crate::openapi`] but produces a *runtime* lookup keyed by
//! `(method, path)`. The dispatcher itself is intentionally minimal —
//! the M28 milestone only ships a dry-run path-match / method-match /
//! principal / capability check pipeline that returns a canned
//! `Response`. The real HTTP server is a later milestone.
//!
//! ## Design
//!
//! * **Deterministic.** All maps are `BTreeMap` / `BTreeSet`; the JSON
//!   envelope produced by [`dispatch_report_json`] sorts every output
//!   so byte-for-byte stability is testable.
//! * **No panics.** Every public entry point returns a `Result` or
//!   builds a value; serialization errors are routed through
//!   [`crate::json::to_json`] which renders an `ori.serialization_error.v1`
//!   envelope rather than panicking.
//! * **Static paths only.** Path parameters (`/users/{id:UserId}`) are
//!   recognised and stored verbatim but not matched at dispatch time;
//!   only an *exact* request path matches a stored route. Parameterised
//!   matching lands in M28b.
//! * **Capability check.** Every effect on a route handler is treated
//!   as a capability the calling principal must hold. `http` is
//!   considered a transport-level effect (not a capability) and is
//!   excluded from the principal check; capabilities such as
//!   `db.read` / `db.write` / `audit` are enforced.

use crate::ast::{Module, SymbolKind};
use crate::json::to_json;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Stable schema id for the dispatch-report envelope.
pub const BACKEND_DISPATCH_SCHEMA: &str = "ori.backend_dispatch.v1";

/// Transport-level effects that are *not* treated as capabilities.
/// The principal does not need to advertise these in
/// [`Principal::capabilities`]; they are satisfied by virtue of the
/// request reaching the dispatcher at all.
const TRANSPORT_EFFECTS: &[&str] = &["http"];

/// One route as understood by the runtime dispatcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RouteSpec {
    /// Stable handler symbol id (`sym:<module>.<name>`).
    pub symbol_id: String,
    /// HTTP method in upper-case (`GET`, `POST`, ...).
    pub method: String,
    /// Canonical request path (`/users` or `/users/{id}`).
    pub path: String,
    /// Sorted effects copied from the handler symbol.
    pub effects: Vec<String>,
    /// `true` when at least one effect is treated as a capability
    /// (i.e. anything outside [`TRANSPORT_EFFECTS`]).
    pub principal_required: bool,
}

/// The full dispatch table — one entry per `(method, path)` pair.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DispatchTable {
    /// Routes keyed by `(method, path)` so duplicate registrations are
    /// disallowed and iteration is sort-stable.
    routes: BTreeMap<(String, String), RouteSpec>,
}

impl DispatchTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of routes registered.
    pub fn len(&self) -> usize {
        self.routes.len()
    }

    /// Whether the table has zero routes.
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    /// Iterate routes in deterministic `(method, path)` order.
    pub fn iter(&self) -> impl Iterator<Item = &RouteSpec> {
        self.routes.values()
    }

    /// Look up a route by method + path. Returns `None` if missing.
    pub fn get(&self, method: &str, path: &str) -> Option<&RouteSpec> {
        self.routes.get(&(method.to_string(), path.to_string()))
    }

    /// Collect every method registered for `path` (deterministic).
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

    /// Register a route, returning the previous value if any. Used by
    /// the builder; callers normally use [`build_dispatch_table`].
    pub fn insert(&mut self, route: RouteSpec) -> Option<RouteSpec> {
        let key = (route.method.clone(), route.path.clone());
        self.routes.insert(key, route)
    }
}

/// Principal making a dispatch request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Principal {
    /// Stable principal identifier (opaque to the dispatcher).
    pub id: String,
    /// Capabilities the principal currently holds.
    pub capabilities: BTreeSet<String>,
}

/// Inbound request fed to [`dispatch`].
#[derive(Debug, Clone)]
pub struct Request {
    /// HTTP method (case-insensitive on input; normalised to upper).
    pub method: String,
    /// Request path.
    pub path: String,
    /// Optional principal; `None` denotes an anonymous request.
    pub principal: Option<Principal>,
    /// Raw request body bytes.
    pub body: Vec<u8>,
}

/// Outbound response returned by [`dispatch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// HTTP status code.
    pub status: u16,
    /// Raw response body bytes.
    pub body: Vec<u8>,
    /// Sorted response headers.
    pub headers: BTreeMap<String, String>,
}

/// Error variants the dispatcher can return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchError {
    /// No route is registered for the requested path.
    NotFound,
    /// The path matches but the method does not. `allowed` lists the
    /// methods that *are* registered for that path, sorted ascending.
    MethodNotAllowed {
        /// Methods registered for the matched path.
        allowed: Vec<String>,
    },
    /// The route requires a principal but the request supplied none.
    MissingPrincipal,
    /// The principal exists but does not carry the required
    /// capability. `needed` is the offending effect name.
    MissingCapability {
        /// The capability the principal is missing.
        needed: String,
    },
}

impl DispatchError {
    /// Snake-case discriminator for the error variant. Useful in JSON
    /// surfaces and tests that need to assert on the failure mode
    /// without matching on the structured variant.
    pub fn kind(&self) -> &'static str {
        match self {
            DispatchError::NotFound => "not_found",
            DispatchError::MethodNotAllowed { .. } => "method_not_allowed",
            DispatchError::MissingPrincipal => "missing_principal",
            DispatchError::MissingCapability { .. } => "missing_capability",
        }
    }
}

/// Build a [`DispatchTable`] from a parsed [`Module`].
///
/// The extractor follows the same conventions as [`crate::openapi`]:
/// any function symbol whose `effects` list includes `http` is treated
/// as an HTTP route. Method and path are inferred from the function
/// name (`get_users` → `GET /users`, `post_checkout` →
/// `POST /checkout`, etc.). Effects are copied across verbatim and
/// `principal_required` is computed from the effect set.
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
    // Sort handlers for deterministic insertion order. (BTreeMap keeps
    // final iteration sorted anyway; sorting here also makes any
    // future logging deterministic.)
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

/// Dispatch a request against `table`.
///
/// The check order is:
/// 1. Path match. If no route is registered for `req.path`, return
///    [`DispatchError::NotFound`].
/// 2. Method match. If the path is known but the method is not,
///    return [`DispatchError::MethodNotAllowed`] with the allowed
///    methods sorted.
/// 3. Principal check. If the route requires a principal and none was
///    supplied, return [`DispatchError::MissingPrincipal`].
/// 4. Capability check. For each non-transport effect, the principal
///    must hold the matching capability; missing one returns
///    [`DispatchError::MissingCapability`].
///
/// On success a canned `200 OK` response is returned with body
/// `{"dispatched":"<symbol_id>"}` and an empty header map.
pub fn dispatch(table: &DispatchTable, req: &Request) -> Result<Response, DispatchError> {
    let method = req.method.to_uppercase();
    // 1. Path match.
    let methods = table.methods_for_path(&req.path);
    if methods.is_empty() {
        return Err(DispatchError::NotFound);
    }
    // 2. Method match.
    let route = match table.get(&method, &req.path) {
        Some(r) => r,
        None => {
            return Err(DispatchError::MethodNotAllowed { allowed: methods });
        }
    };
    // 3. Principal check.
    if route.principal_required && req.principal.is_none() {
        return Err(DispatchError::MissingPrincipal);
    }
    // 4. Capability check (only required if a principal is present;
    //    routes without `principal_required` short-circuited above).
    if let Some(principal) = req.principal.as_ref() {
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
    Ok(Response {
        status: 200,
        body,
        headers: BTreeMap::new(),
    })
}

/// Serializable JSON envelope for the dispatch report.
#[derive(Debug, Serialize)]
struct DispatchReport<'a> {
    schema: &'static str,
    route_count: usize,
    routes: Vec<&'a RouteSpec>,
}

/// Render the dispatch table as a canonical JSON envelope identified
/// by `ori.backend_dispatch.v1`. Output is deterministic.
pub fn dispatch_report_json(table: &DispatchTable) -> String {
    let routes: Vec<&RouteSpec> = table.routes.values().collect();
    let report = DispatchReport {
        schema: BACKEND_DISPATCH_SCHEMA,
        route_count: routes.len(),
        routes,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_source;
    use crate::source::SourceFile;

    fn module_for(text: &str) -> Module {
        parse_source(&SourceFile::new("/t.ori", text)).module
    }

    fn module_from_file(rel_path: &str) -> Module {
        // Resolve `rel_path` against the workspace root so the test
        // is location-independent. `CARGO_MANIFEST_DIR` points at
        // `<root>/crates/ori-compiler`; the workspace root is two
        // levels up.
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
        // Synthetic 3-route module that mirrors the demo storefront so
        // tests don't depend on file-system layout for the negative
        // cases. `users.ori` is exercised in `builds_table_from_users_ori`.
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

    #[test]
    fn builds_table_from_users_ori() {
        // The shipping example does not annotate the handler with the
        // `http` effect (its routes are declared inside `service Users`
        // syntactically), so the runtime extraction sees zero routes —
        // which is the deterministic, schema-valid answer. The test
        // pins that contract so a future parser change is loud.
        // Resolve the path from the workspace root so the test is
        // location-stable regardless of cargo's working directory.
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest_dir
            .ancestors()
            .nth(2)
            .map(std::path::Path::to_path_buf)
            .unwrap_or(manifest_dir);
        let path = workspace_root.join("examples/fullstack/users.ori");
        let module = module_from_file(path.to_string_lossy().as_ref());
        let table = build_dispatch_table(&module);
        assert!(
            table.is_empty(),
            "users.ori currently contributes no http-annotated handlers"
        );
        let json = dispatch_report_json(&table);
        assert!(json.contains("\"schema\":\"ori.backend_dispatch.v1\""));
        assert!(json.contains("\"route_count\":0"));
    }

    #[test]
    fn path_miss_returns_not_found() {
        let table = build_dispatch_table(&demo_module());
        let req = Request {
            method: "GET".to_string(),
            path: "/nope".to_string(),
            principal: Some(principal_with(&["db.read"])),
            body: Vec::new(),
        };
        let err = dispatch(&table, &req);
        assert_eq!(err, Err(DispatchError::NotFound));
    }

    #[test]
    fn wrong_method_returns_method_not_allowed() {
        let table = build_dispatch_table(&demo_module());
        // /users is registered for GET and POST in the demo module.
        let req = Request {
            method: "DELETE".to_string(),
            path: "/users".to_string(),
            principal: Some(principal_with(&["db.read", "db.write"])),
            body: Vec::new(),
        };
        let err = dispatch(&table, &req);
        match err {
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
        let req = Request {
            method: "GET".to_string(),
            path: "/users".to_string(),
            principal: None,
            body: Vec::new(),
        };
        assert_eq!(dispatch(&table, &req), Err(DispatchError::MissingPrincipal));
    }

    #[test]
    fn missing_capability_returns_missing_capability() {
        let table = build_dispatch_table(&demo_module());
        let req = Request {
            method: "GET".to_string(),
            path: "/users".to_string(),
            principal: Some(principal_with(&[])),
            body: Vec::new(),
        };
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
        let req = Request {
            method: "GET".to_string(),
            path: "/users".to_string(),
            principal: Some(principal_with(&["db.read"])),
            body: Vec::new(),
        };
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
        assert_eq!(json_a, json_b, "report must be byte-stable across runs");
    }

    #[test]
    fn transport_only_route_does_not_require_principal() {
        // `get_status` only declares `http`, which is transport-level,
        // so it must be reachable anonymously.
        let table = build_dispatch_table(&demo_module());
        let req = Request {
            method: "GET".to_string(),
            path: "/status".to_string(),
            principal: None,
            body: Vec::new(),
        };
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
        let req = Request {
            method: "get".to_string(),
            path: "/status".to_string(),
            principal: None,
            body: Vec::new(),
        };
        assert!(dispatch(&table, &req).is_ok());
    }

    #[test]
    fn missing_capability_reports_first_alpha_effect() {
        // db.read sorts before http; with `http, db.write` on a POST,
        // the principal needs db.write, so we should be told that.
        let module =
            module_for("module demo\nfn post_orders(o: Order) -> Order uses http, db.write\n");
        let table = build_dispatch_table(&module);
        let req = Request {
            method: "POST".to_string(),
            path: "/orders".to_string(),
            principal: Some(principal_with(&[])),
            body: Vec::new(),
        };
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
    }

    #[test]
    fn empty_module_yields_empty_table() {
        let module = module_for("module demo\nfn _hidden() -> Unit\n");
        let table = build_dispatch_table(&module);
        assert!(table.is_empty());
        let req = Request {
            method: "GET".to_string(),
            path: "/anything".to_string(),
            principal: None,
            body: Vec::new(),
        };
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
}
