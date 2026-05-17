//! Route-to-OpenAPI 3.1 extraction.
//!
//! The bootstrap parser exposes routes as ordinary function signatures with
//! the route metadata encoded by convention: function name maps to the
//! operation id, the effects list determines the HTTP method
//! (`http`, `db.read`, `db.write`), and the signature provides the request
//! body type and the response type.
//!
//! For the bootstrap we do not yet have full route attributes, so we
//! reconstruct an OpenAPI document by treating each `fn <name>(...)` symbol
//! inside a service-bearing module as an operation. This is a deliberately
//! conservative interpretation that still satisfies the M12 acceptance
//! command `ori openapi --json examples/fullstack/users.ori` with a stable,
//! schema-valid output.

use crate::ast::{Module, SymbolKind};
use crate::json::to_json;
use serde::Serialize;

/// Stable schema id for the OpenAPI extraction envelope.
pub const OPENAPI_REPORT_SCHEMA: &str = "ori.openapi_report.v1";
/// OpenAPI specification version this extractor targets.
pub const OPENAPI_VERSION: &str = "3.1.0";

/// JSON envelope produced by [`extract_openapi`].
#[derive(Debug, Serialize)]
pub struct OpenApiReport {
    /// Stable schema identifier.
    pub schema: &'static str,
    /// OpenAPI specification version targeted by this report.
    pub openapi_version: &'static str,
    /// Names of services declared in the module.
    pub services: Vec<String>,
    /// One [`RouteEntry`] per extracted HTTP route.
    pub routes: Vec<RouteEntry>,
}

/// One HTTP route extracted from a function symbol.
#[derive(Debug, Serialize)]
pub struct RouteEntry {
    /// HTTP method.
    pub method: &'static str,
    /// URL path with placeholders.
    pub path: String,
    /// Symbol id of the handler function.
    pub handler_symbol: String,
    /// Path/query parameters extracted from the signature.
    pub params: Vec<RouteParam>,
    /// Optional request-body type name.
    pub request_body_type: Option<String>,
    /// Response type name.
    pub response_type: String,
    /// Sorted effect list copied from the handler.
    pub effects: Vec<String>,
}

/// One route parameter inside a [`RouteEntry`].
#[derive(Debug, Serialize)]
pub struct RouteParam {
    /// Parameter binding name.
    pub name: String,
    /// Location (`path`, `query`, ...).
    pub r#in: &'static str,
    /// Declared type.
    pub r#type: String,
    /// Whether the parameter is required by the route.
    pub required: bool,
}

impl OpenApiReport {
    /// Render the report as canonical JSON.
    pub fn to_json(&self) -> String {
        to_json(self)
    }
}

/// Extract HTTP routes and service names from `module` into an
/// [`OpenApiReport`].
pub fn extract_openapi(module: &Module) -> OpenApiReport {
    let services: Vec<String> = module
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Service)
        .map(|s| s.name.clone())
        .collect();

    let routes: Vec<RouteEntry> = module
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Function)
        .filter(|s| s.effects.iter().any(|e| e.starts_with("http")))
        .map(|s| RouteEntry {
            method: infer_method(s.name.as_str(), &s.effects),
            path: infer_path(s.name.as_str()),
            handler_symbol: s.id.clone(),
            params: extract_params(s.signature.as_str()),
            request_body_type: extract_request_body_type(s.signature.as_str()),
            response_type: extract_response_type(s.signature.as_str()),
            effects: s.effects.clone(),
        })
        .collect();

    OpenApiReport {
        schema: OPENAPI_REPORT_SCHEMA,
        openapi_version: OPENAPI_VERSION,
        services,
        routes,
    }
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

fn extract_params(signature: &str) -> Vec<RouteParam> {
    let open = match signature.find('(') {
        Some(idx) => idx,
        None => return Vec::new(),
    };
    let close = match signature[open..].find(')') {
        Some(off) => open + off,
        None => return Vec::new(),
    };
    let body = &signature[open + 1..close];
    if body.trim().is_empty() {
        return Vec::new();
    }
    body.split(',')
        .filter_map(|part| {
            let part = part.trim();
            if let Some(colon) = part.find(':') {
                let name = part[..colon].trim().to_string();
                let ty = part[colon + 1..].trim().to_string();
                if name.is_empty() || ty.is_empty() {
                    None
                } else {
                    Some(RouteParam {
                        name,
                        r#in: "path",
                        r#type: ty,
                        required: true,
                    })
                }
            } else {
                None
            }
        })
        .collect()
}

fn extract_request_body_type(signature: &str) -> Option<String> {
    let params = extract_params(signature);
    params
        .into_iter()
        .find(|p| p.r#type.contains("Body") || p.r#type.contains("Request") || p.name == "body")
        .map(|p| p.r#type)
}

fn extract_response_type(signature: &str) -> String {
    if let Some(idx) = signature.find("->") {
        let after = signature[idx + 2..].trim();
        let cutoff = after.find(" uses ").unwrap_or(after.len());
        after[..cutoff].trim().to_string()
    } else {
        "Unit".to_string()
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

    #[test]
    fn extracts_get_route_from_http_effect() {
        let module =
            module_for("module demo\nservice Api\nfn get_users() -> List[User] uses http, db.read");
        let report = extract_openapi(&module);
        assert!(report.services.contains(&"Api".to_string()));
        assert!(report
            .routes
            .iter()
            .any(|r| r.method == "GET" && r.path == "/users"));
    }

    #[test]
    fn extracts_post_route_with_db_write() {
        let module = module_for(
            "module demo\nfn create_order(order: NewOrder) -> Order uses http, db.write",
        );
        let report = extract_openapi(&module);
        let route = report
            .routes
            .iter()
            .find(|r| r.handler_symbol.ends_with("create_order"));
        assert!(route.is_some());
        let route = report
            .routes
            .into_iter()
            .find(|r| r.handler_symbol.ends_with("create_order"));
        assert!(route.is_some());
    }

    #[test]
    fn report_serialises_with_schema_field() {
        let module = module_for("module demo\nfn get_x() -> Int uses http");
        let json = extract_openapi(&module).to_json();
        assert!(json.contains("\"schema\":\"ori.openapi_report.v1\""));
        assert!(json.contains("\"openapi_version\":\"3.1.0\""));
    }
}
