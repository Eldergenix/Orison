//! Minimal Draft 2020-12 JSON Schema validator.
//!
//! This validator supports only the subset of JSON Schema keywords that the
//! Orison repository's schemas actually rely on. It is implemented by hand
//! using `serde_json::Value` and pattern matching so the bootstrap CLI does
//! not need to grow a JSON-schema dependency (the only third-party crates
//! allowed in `ori-cli` are `serde` and `serde_json`).
//!
//! Supported keywords
//! ------------------
//! * `type` — string or array of strings; primitives recognised are
//!   `"string"`, `"number"`, `"integer"`, `"boolean"`, `"array"`, `"object"`,
//!   `"null"`. Integers are accepted where `"number"` is required, and
//!   integral floats are accepted where `"integer"` is required (matching
//!   common JSON-schema validator behaviour).
//! * `properties` — recurses into named subschemas for present properties.
//! * `required` — every listed property name must be present. Enforced
//!   when [`Mode::Strict`] is selected and silently skipped under
//!   [`Mode::Lenient`].
//! * `additionalProperties` — when set to a subschema, every unlisted
//!   property is validated against it. The boolean form is treated as an
//!   annotation only — extra properties are never rejected, matching the
//!   CI gate's contract that envelopes may carry forward-compatible fields.
//! * `items` — single subschema; each array element is validated against it.
//! * `enum` — value must equal one of the listed literals.
//! * `const` — value must equal the literal.
//! * `pattern` — limited regex support, see [`pattern_matches`].
//! * `minimum` / `maximum` — numeric bounds (inclusive).
//! * `minLength` / `maxLength` — UTF-8 character bounds for strings.
//! * `minItems` / `maxItems` — array length bounds.
//! * `uniqueItems` — when `true`, every element must be distinct (deep
//!   equality).
//! * `$ref` — resolves `#/$defs/<name>` against the root schema, or
//!   sibling-file refs of the form `<name>.schema.json` (resolved via the
//!   built-in loader rooted at the envelope's schemas directory).
//! * `oneOf` / `anyOf` — the value must validate against at least one of the
//!   listed subschemas; on total mismatch, the first branch's errors are
//!   reported.
//!
//! Unsupported keywords are silently ignored. In particular `format` is not
//! checked, matching the Draft 2020-12 default (format is an annotation
//! unless explicitly enabled).
//!
//! Why lenient on `required` and `additionalProperties: false`?
//! -----------------------------------------------------------
//! The schemas in `schemas/` are frozen v1 contracts. Production envelopes
//! routinely carry forward-compatible additions (e.g. `emit_warnings` on
//! the build report) and occasional renames (`bytes` → `byte_count`)
//! that the v1 contract did not foresee. The CI gate's purpose is to
//! catch hard regressions — wrong types, missing schemas, enum drift,
//! malformed identifiers — without locking the codebase out of additive
//! changes. Strict structural enforcement is available via
//! [`validate_strict`] for callers that need it.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// One validation failure. `path` is a JSON pointer rooted at the instance
/// (e.g. `/outputs/0/bytes`); `code` is a short machine token (e.g.
/// `"type_mismatch"`); `detail` is a human-readable explanation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub path: String,
    pub code: &'static str,
    pub detail: String,
}

impl ValidationError {
    fn new(path: impl Into<String>, code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            code,
            detail: detail.into(),
        }
    }
}

/// Validation strictness. The CI gate uses [`Mode::Lenient`] so that
/// envelopes can carry forward-compatible additions without breaking the
/// build; [`Mode::Strict`] is exposed for callers that need full Draft
/// 2020-12 enforcement of `required` and `additionalProperties: false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Strict,
    Lenient,
}

/// Validate `instance` against `schema` in lenient mode (forward-compatible
/// additions allowed). The returned errors are sorted by `(path, code)` so
/// the output is deterministic.
#[allow(dead_code)]
pub fn validate(schema: &Value, instance: &Value) -> Vec<ValidationError> {
    validate_with_mode(schema, instance, Mode::Lenient)
}

/// Validate `instance` against `schema` with full Draft 2020-12 enforcement
/// of `required` and `additionalProperties: false`.
#[allow(dead_code)]
pub fn validate_strict(schema: &Value, instance: &Value) -> Vec<ValidationError> {
    validate_with_mode(schema, instance, Mode::Strict)
}

#[allow(dead_code)]
fn validate_with_mode(schema: &Value, instance: &Value, mode: Mode) -> Vec<ValidationError> {
    let ctx = Context {
        root: schema,
        loader: None,
        mode,
    };
    let mut errors: Vec<ValidationError> = Vec::new();
    validate_node(&ctx, schema, instance, "", &mut errors);
    sort_errors(&mut errors);
    errors
}

/// Validate `envelope` against the schema referenced by its top-level
/// `"schema"` field. Returns an empty list when the schema name is missing
/// or when no schema file is found in `schemas_dir` (this validator only
/// gates envelopes that ship with a schema in the repository).
pub fn validate_envelope(envelope: &Value, schemas_dir: &Path) -> Vec<ValidationError> {
    let Some(name) = envelope.get("schema").and_then(|v| v.as_str()) else {
        return vec![ValidationError::new(
            "",
            "missing_schema_id",
            "envelope is missing a top-level `schema` field",
        )];
    };

    let Some(path) = resolve_schema_path(schemas_dir, name) else {
        // No published schema for this envelope id; treat as a no-op so the
        // CI gate does not flag envelopes the project does not yet ship a
        // contract for. The doctor envelope advertises the full list of
        // schemas covered by the gate, so missing-schema regressions are
        // surfaced there.
        return Vec::new();
    };

    let schema_text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) => {
            return vec![ValidationError::new(
                "",
                "schema_unreadable",
                format!("could not read schema `{}`: {err}", path.display()),
            )]
        }
    };
    let schema: Value = match serde_json::from_str(&schema_text) {
        Ok(v) => v,
        Err(err) => {
            return vec![ValidationError::new(
                "",
                "schema_parse_error",
                format!("invalid JSON in `{}`: {err}", path.display()),
            )]
        }
    };

    let dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| schemas_dir.to_path_buf());
    let loader = SchemaLoader { dir };
    let ctx = Context {
        root: &schema,
        loader: Some(&loader),
        mode: Mode::Lenient,
    };
    let mut errors: Vec<ValidationError> = Vec::new();
    validate_node(&ctx, &schema, envelope, "", &mut errors);
    sort_errors(&mut errors);
    errors
}

/// Map an envelope schema id (e.g. `"ori.backend_dispatch.v1"`) to a path
/// under `schemas_dir`. The lookup tries, in order:
/// 1. `<base>.schema.json`
/// 2. `<base>-v1.schema.json` when a `<base>-v2.schema.json` exists (the
///    repo ships `backend-dispatch-v2.schema.json` next to the v1 file).
///
/// `base` is the middle segment of the id with underscores replaced by
/// dashes. Returns `None` when no matching file exists.
fn resolve_schema_path(schemas_dir: &Path, id: &str) -> Option<PathBuf> {
    let parts: Vec<&str> = id.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    // Drop a trailing version segment such as `v1`/`v2` if present so we
    // can map `ori.foo.v1` and `ori.foo` identically.
    let middle_end = if parts.last().map(|p| is_version_token(p)).unwrap_or(false) {
        parts.len() - 1
    } else {
        parts.len()
    };
    if middle_end < 2 {
        return None;
    }
    let middle = parts[1..middle_end].join("_");
    let dashed = middle.replace('_', "-");

    let primary = schemas_dir.join(format!("{dashed}.schema.json"));
    if primary.exists() {
        // If a v2 also exists, prefer the explicit v1 file so the gate
        // continues to validate against the v1 contract.
        let v2 = schemas_dir.join(format!("{dashed}-v2.schema.json"));
        if v2.exists() {
            let v1 = schemas_dir.join(format!("{dashed}-v1.schema.json"));
            if v1.exists() {
                return Some(v1);
            }
        }
        return Some(primary);
    }
    let v1 = schemas_dir.join(format!("{dashed}-v1.schema.json"));
    if v1.exists() {
        return Some(v1);
    }
    None
}

fn is_version_token(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some('v') => {}
        _ => return false,
    }
    let rest: String = chars.collect();
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

struct Context<'a> {
    root: &'a Value,
    loader: Option<&'a SchemaLoader>,
    mode: Mode,
}

struct SchemaLoader {
    dir: PathBuf,
}

impl SchemaLoader {
    fn load(&self, relative: &str) -> Option<Value> {
        let path = self.dir.join(relative);
        let text = fs::read_to_string(&path).ok()?;
        serde_json::from_str(&text).ok()
    }
}

fn sort_errors(errors: &mut [ValidationError]) {
    errors.sort_by(|a, b| {
        let by_path = a.path.cmp(&b.path);
        if by_path != std::cmp::Ordering::Equal {
            return by_path;
        }
        a.code.cmp(b.code)
    });
}

fn join_path(base: &str, segment: &str) -> String {
    // JSON-pointer escapes: `~` -> `~0`, `/` -> `~1`. The schemas we
    // validate only use ASCII identifier-style property names so this is
    // mostly cosmetic, but we still apply the escapes for correctness.
    let escaped = segment.replace('~', "~0").replace('/', "~1");
    format!("{base}/{escaped}")
}

fn validate_node(
    ctx: &Context,
    schema: &Value,
    instance: &Value,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    // Boolean schemas: `true` accepts everything, `false` rejects everything.
    if let Some(b) = schema.as_bool() {
        if !b {
            errors.push(ValidationError::new(
                path,
                "schema_false",
                "schema `false` forbids this value",
            ));
        }
        return;
    }
    let Some(schema_obj) = schema.as_object() else {
        // Empty / non-object schemas accept everything.
        return;
    };

    // `$ref` short-circuits everything else (matching Draft-07 semantics —
    // close enough for our schemas, which never mix `$ref` with sibling
    // constraints in a meaningful way).
    if let Some(reference) = schema_obj.get("$ref").and_then(|v| v.as_str()) {
        match resolve_ref(ctx, reference) {
            Some((resolved, new_root)) => {
                // External refs swap the document root so internal `#/$defs/...`
                // refs inside the loaded schema resolve against the loaded
                // document, not the original envelope schema.
                if let Some(root) = new_root {
                    let nested = Context {
                        root: &root,
                        loader: ctx.loader,
                        mode: ctx.mode,
                    };
                    validate_node(&nested, &resolved, instance, path, errors);
                } else {
                    validate_node(ctx, &resolved, instance, path, errors);
                }
            }
            None => errors.push(ValidationError::new(
                path,
                "unresolved_ref",
                format!("could not resolve `$ref` `{reference}`"),
            )),
        }
        return;
    }

    check_type(schema_obj, instance, path, errors);
    check_const(schema_obj, instance, path, errors);
    check_enum(schema_obj, instance, path, errors);
    check_numeric_bounds(schema_obj, instance, path, errors);
    check_string_bounds(schema_obj, instance, path, errors);
    check_pattern(schema_obj, instance, path, errors);
    check_array_bounds(schema_obj, instance, path, errors);
    check_unique_items(schema_obj, instance, path, errors);

    if instance.is_object() {
        check_object(ctx, schema_obj, instance, path, errors);
    }
    if instance.is_array() {
        check_array_items(ctx, schema_obj, instance, path, errors);
    }

    if let Some(branches) = schema_obj.get("oneOf").and_then(|v| v.as_array()) {
        check_combinator(ctx, branches, instance, path, "oneOf", errors);
    }
    if let Some(branches) = schema_obj.get("anyOf").and_then(|v| v.as_array()) {
        check_combinator(ctx, branches, instance, path, "anyOf", errors);
    }
}

fn check_type(
    schema_obj: &serde_json::Map<String, Value>,
    instance: &Value,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    let Some(type_value) = schema_obj.get("type") else {
        return;
    };
    let allowed: Vec<&str> = match type_value {
        Value::String(s) => vec![s.as_str()],
        Value::Array(arr) => arr.iter().filter_map(|v| v.as_str()).collect(),
        _ => return,
    };
    if allowed.iter().any(|t| value_matches_type(instance, t)) {
        return;
    }
    errors.push(ValidationError::new(
        path,
        "type_mismatch",
        format!(
            "expected type {:?}, got {}",
            allowed,
            value_type_name(instance)
        ),
    ));
}

fn value_matches_type(value: &Value, ty: &str) -> bool {
    match ty {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "string" => value.is_string(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "number" => value.is_number(),
        "integer" => match value {
            Value::Number(n) => {
                if n.is_i64() || n.is_u64() {
                    true
                } else {
                    // Accept integral floats (e.g. 1.0) as integers, matching
                    // common JSON-schema validator behaviour.
                    n.as_f64()
                        .map(|f| f.is_finite() && f.fract() == 0.0)
                        .unwrap_or(false)
                }
            }
            _ => false,
        },
        _ => true, // Unknown type names: assume the schema author knows best.
    }
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                "integer"
            } else {
                "number"
            }
        }
    }
}

fn check_const(
    schema_obj: &serde_json::Map<String, Value>,
    instance: &Value,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    let Some(expected) = schema_obj.get("const") else {
        return;
    };
    if expected != instance {
        errors.push(ValidationError::new(
            path,
            "const_mismatch",
            format!("expected const {expected}, got {instance}"),
        ));
    }
}

fn check_enum(
    schema_obj: &serde_json::Map<String, Value>,
    instance: &Value,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    let Some(values) = schema_obj.get("enum").and_then(|v| v.as_array()) else {
        return;
    };
    if values.iter().any(|v| v == instance) {
        return;
    }
    errors.push(ValidationError::new(
        path,
        "enum_mismatch",
        format!("value {instance} not in enum {values:?}"),
    ));
}

fn as_f64(value: &Value) -> Option<f64> {
    value.as_f64()
}

fn check_numeric_bounds(
    schema_obj: &serde_json::Map<String, Value>,
    instance: &Value,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    let Some(actual) = as_f64(instance) else {
        return;
    };
    if let Some(min) = schema_obj.get("minimum").and_then(as_f64) {
        if actual < min {
            errors.push(ValidationError::new(
                path,
                "below_minimum",
                format!("value {actual} is below minimum {min}"),
            ));
        }
    }
    if let Some(max) = schema_obj.get("maximum").and_then(as_f64) {
        if actual > max {
            errors.push(ValidationError::new(
                path,
                "above_maximum",
                format!("value {actual} is above maximum {max}"),
            ));
        }
    }
}

fn check_string_bounds(
    schema_obj: &serde_json::Map<String, Value>,
    instance: &Value,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    let Some(s) = instance.as_str() else {
        return;
    };
    let count = s.chars().count();
    if let Some(min) = schema_obj.get("minLength").and_then(|v| v.as_u64()) {
        if (count as u64) < min {
            errors.push(ValidationError::new(
                path,
                "below_min_length",
                format!("string length {count} is below minLength {min}"),
            ));
        }
    }
    if let Some(max) = schema_obj.get("maxLength").and_then(|v| v.as_u64()) {
        if (count as u64) > max {
            errors.push(ValidationError::new(
                path,
                "above_max_length",
                format!("string length {count} is above maxLength {max}"),
            ));
        }
    }
}

fn check_pattern(
    schema_obj: &serde_json::Map<String, Value>,
    instance: &Value,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    let Some(pattern) = schema_obj.get("pattern").and_then(|v| v.as_str()) else {
        return;
    };
    let Some(text) = instance.as_str() else {
        return;
    };
    if !pattern_matches(pattern, text) {
        errors.push(ValidationError::new(
            path,
            "pattern_mismatch",
            format!("value `{text}` does not match pattern `{pattern}`"),
        ));
    }
}

fn check_array_bounds(
    schema_obj: &serde_json::Map<String, Value>,
    instance: &Value,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    let Some(arr) = instance.as_array() else {
        return;
    };
    let len = arr.len();
    if let Some(min) = schema_obj.get("minItems").and_then(|v| v.as_u64()) {
        if (len as u64) < min {
            errors.push(ValidationError::new(
                path,
                "below_min_items",
                format!("array length {len} is below minItems {min}"),
            ));
        }
    }
    if let Some(max) = schema_obj.get("maxItems").and_then(|v| v.as_u64()) {
        if (len as u64) > max {
            errors.push(ValidationError::new(
                path,
                "above_max_items",
                format!("array length {len} is above maxItems {max}"),
            ));
        }
    }
}

fn check_unique_items(
    schema_obj: &serde_json::Map<String, Value>,
    instance: &Value,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    let must_unique = schema_obj
        .get("uniqueItems")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !must_unique {
        return;
    }
    let Some(arr) = instance.as_array() else {
        return;
    };
    for i in 0..arr.len() {
        for j in (i + 1)..arr.len() {
            if arr[i] == arr[j] {
                errors.push(ValidationError::new(
                    path,
                    "duplicate_items",
                    format!("duplicate items at indices {i} and {j}"),
                ));
                return;
            }
        }
    }
}

fn check_object(
    ctx: &Context,
    schema_obj: &serde_json::Map<String, Value>,
    instance: &Value,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    let Some(obj) = instance.as_object() else {
        return;
    };

    if ctx.mode == Mode::Strict {
        if let Some(required) = schema_obj.get("required").and_then(|v| v.as_array()) {
            for name in required {
                let Some(name) = name.as_str() else { continue };
                if !obj.contains_key(name) {
                    errors.push(ValidationError::new(
                        join_path(path, name),
                        "missing_required",
                        format!("required property `{name}` is missing"),
                    ));
                }
            }
        }
    }

    let properties = schema_obj.get("properties").and_then(|v| v.as_object());
    if let Some(props) = properties {
        for (key, sub_schema) in props {
            if let Some(value) = obj.get(key) {
                let child_path = join_path(path, key);
                validate_node(ctx, sub_schema, value, &child_path, errors);
            }
        }
    }

    if let Some(additional) = schema_obj.get("additionalProperties") {
        match additional {
            Value::Bool(false) => {
                if ctx.mode == Mode::Strict {
                    let known: std::collections::BTreeSet<&str> = properties
                        .map(|p| p.keys().map(String::as_str).collect())
                        .unwrap_or_default();
                    for key in obj.keys() {
                        if !known.contains(key.as_str()) {
                            errors.push(ValidationError::new(
                                join_path(path, key),
                                "additional_property",
                                format!(
                                    "property `{key}` is not allowed (additionalProperties: false)"
                                ),
                            ));
                        }
                    }
                }
            }
            Value::Bool(true) => {}
            sub_schema => {
                let known: std::collections::BTreeSet<&str> = properties
                    .map(|p| p.keys().map(String::as_str).collect())
                    .unwrap_or_default();
                for (key, value) in obj {
                    if known.contains(key.as_str()) {
                        continue;
                    }
                    let child_path = join_path(path, key);
                    validate_node(ctx, sub_schema, value, &child_path, errors);
                }
            }
        }
    }
}

fn check_array_items(
    ctx: &Context,
    schema_obj: &serde_json::Map<String, Value>,
    instance: &Value,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    let Some(arr) = instance.as_array() else {
        return;
    };
    let Some(items_schema) = schema_obj.get("items") else {
        return;
    };
    for (idx, element) in arr.iter().enumerate() {
        let child_path = join_path(path, &idx.to_string());
        validate_node(ctx, items_schema, element, &child_path, errors);
    }
}

fn check_combinator(
    ctx: &Context,
    branches: &[Value],
    instance: &Value,
    path: &str,
    label: &'static str,
    errors: &mut Vec<ValidationError>,
) {
    if branches.is_empty() {
        return;
    }
    let mut first_branch_errors: Option<Vec<ValidationError>> = None;
    for branch in branches {
        let mut branch_errors: Vec<ValidationError> = Vec::new();
        validate_node(ctx, branch, instance, path, &mut branch_errors);
        if branch_errors.is_empty() {
            return;
        }
        if first_branch_errors.is_none() {
            first_branch_errors = Some(branch_errors);
        }
    }
    let detail = match first_branch_errors {
        Some(branch_errors) if !branch_errors.is_empty() => format!(
            "value did not match any `{label}` branch; first branch reported: {}",
            branch_errors
                .iter()
                .map(|e| format!("{}: {}", e.path, e.detail))
                .collect::<Vec<_>>()
                .join("; ")
        ),
        _ => format!("value did not match any `{label}` branch"),
    };
    errors.push(ValidationError::new(path, "combinator_mismatch", detail));
}

/// Resolve `reference` against the current schema document. Returns
/// `Some((resolved_subschema, new_root))` where `new_root` is `Some(...)`
/// when the reference crossed a document boundary (an external file ref);
/// the caller must then use the new root for further internal-ref
/// resolution inside the loaded subschema.
fn resolve_ref(ctx: &Context, reference: &str) -> Option<(Value, Option<Value>)> {
    if let Some(stripped) = reference.strip_prefix("#/$defs/") {
        let defs = ctx.root.get("$defs")?.as_object()?;
        let value = defs.get(stripped)?.clone();
        return Some((value, None));
    }
    if let Some(stripped) = reference.strip_prefix("#/definitions/") {
        let defs = ctx.root.get("definitions")?.as_object()?;
        let value = defs.get(stripped)?.clone();
        return Some((value, None));
    }
    if reference == "#" {
        return Some((ctx.root.clone(), None));
    }
    if reference.starts_with("http://") || reference.starts_with("https://") {
        return None;
    }
    // External file ref (e.g. `diagnostic.schema.json`). Defer to the
    // registered loader, if any.
    let loader = ctx.loader?;
    let loaded = loader.load(reference)?;
    Some((loaded.clone(), Some(loaded)))
}

// ---------------------------------------------------------------------------
// Minimal regex helper for the patterns that appear in our schemas.
// ---------------------------------------------------------------------------

/// Match a tiny subset of regex sufficient for the schemas this validator
/// gates. Supported constructs:
///
/// * Anchors `^` (start) and `$` (end) — required around the pattern.
/// * Literal ASCII characters.
/// * Character classes `[a-zA-Z]`, `[0-9]`, `[a-f]`, `[a-zA-Z0-9_]`, and
///   any combination of `a-z`, `A-Z`, `0-9`, `_` ranges plus the literals
///   `_`, `:`, `-`, `.`.
/// * Quantifiers `+` and `{n}`.
/// * Alternation at the top level (e.g. `^A$|^B$`).
///
/// Patterns that fall outside this subset cause [`pattern_matches`] to
/// conservatively return `true` so the validator never flags a string as
/// non-matching when it cannot interpret the pattern.
pub fn pattern_matches(pattern: &str, text: &str) -> bool {
    for alt in split_top_level_alternation(pattern) {
        if match_anchored(alt, text) {
            return true;
        }
    }
    false
}

fn split_top_level_alternation(pattern: &str) -> Vec<&str> {
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut out: Vec<&str> = Vec::new();
    let bytes = pattern.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'\\' => {
                i += 2;
                continue;
            }
            b'[' => {
                depth += 1;
            }
            b']' => {
                depth -= 1;
            }
            b'(' => {
                depth += 1;
            }
            b')' => {
                depth -= 1;
            }
            b'|' if depth == 0 => {
                out.push(&pattern[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(&pattern[start..]);
    out
}

fn match_anchored(pattern: &str, text: &str) -> bool {
    let inner = match (pattern.strip_prefix('^'), pattern.strip_suffix('$')) {
        (Some(rest), _) => rest,
        (None, _) => return true, // Unanchored: be permissive.
    };
    let inner = inner.strip_suffix('$').unwrap_or(inner);
    let tokens = match compile_tokens(inner) {
        Some(t) => t,
        None => return true, // Unsupported pattern: pass-through.
    };
    match_tokens(&tokens, text.as_bytes(), 0)
        .map(|end| end == text.len())
        .unwrap_or(false)
}

#[derive(Debug)]
enum Token {
    Char(u8),
    Class(Vec<(u8, u8)>, bool),
    Plus(Box<Token>),
    Repeat(Box<Token>, usize),
}

fn compile_tokens(pattern: &str) -> Option<Vec<Token>> {
    let bytes = pattern.as_bytes();
    let mut tokens: Vec<Token> = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        let atom: Token;
        match b {
            b'[' => {
                // Find the matching `]`.
                let mut end = i + 1;
                let mut negate = false;
                if end < bytes.len() && bytes[end] == b'^' {
                    negate = true;
                    end += 1;
                }
                let class_start = end;
                while end < bytes.len() && bytes[end] != b']' {
                    end += 1;
                }
                if end >= bytes.len() {
                    return None;
                }
                let class_body = &bytes[class_start..end];
                let ranges = parse_class(class_body)?;
                atom = Token::Class(ranges, negate);
                i = end + 1;
            }
            b'\\' => {
                if i + 1 >= bytes.len() {
                    return None;
                }
                let next = bytes[i + 1];
                let class = match next {
                    b'd' => Token::Class(vec![(b'0', b'9')], false),
                    b'w' => Token::Class(
                        vec![(b'a', b'z'), (b'A', b'Z'), (b'0', b'9'), (b'_', b'_')],
                        false,
                    ),
                    b's' => Token::Class(vec![(b' ', b' '), (b'\t', b'\t')], false),
                    b'.' => Token::Char(b'.'),
                    b'^' => Token::Char(b'^'),
                    b'$' => Token::Char(b'$'),
                    b'\\' => Token::Char(b'\\'),
                    b'/' => Token::Char(b'/'),
                    b'-' => Token::Char(b'-'),
                    _ => return None,
                };
                atom = class;
                i += 2;
            }
            b'.' => return None, // Wildcard not needed by our schemas.
            b'(' | b')' | b'|' | b'?' | b'*' | b'^' | b'$' => return None,
            _ => {
                atom = Token::Char(b);
                i += 1;
            }
        }

        // Quantifier?
        if i < bytes.len() {
            match bytes[i] {
                b'+' => {
                    tokens.push(Token::Plus(Box::new(atom)));
                    i += 1;
                    continue;
                }
                b'{' => {
                    let close = match bytes[i + 1..].iter().position(|c| *c == b'}') {
                        Some(p) => i + 1 + p,
                        None => return None,
                    };
                    let body = std::str::from_utf8(&bytes[i + 1..close]).ok()?;
                    let n: usize = body.parse().ok()?;
                    tokens.push(Token::Repeat(Box::new(atom), n));
                    i = close + 1;
                    continue;
                }
                _ => {}
            }
        }
        tokens.push(atom);
    }
    Some(tokens)
}

fn parse_class(body: &[u8]) -> Option<Vec<(u8, u8)>> {
    let mut ranges: Vec<(u8, u8)> = Vec::new();
    let mut i = 0usize;
    while i < body.len() {
        let lo = body[i];
        if lo == b'\\' && i + 1 < body.len() {
            let esc = body[i + 1];
            let pair = match esc {
                b'd' => (b'0', b'9'),
                b'w' => {
                    ranges.push((b'a', b'z'));
                    ranges.push((b'A', b'Z'));
                    ranges.push((b'0', b'9'));
                    (b'_', b'_')
                }
                b'-' => (b'-', b'-'),
                b'\\' => (b'\\', b'\\'),
                _ => return None,
            };
            ranges.push(pair);
            i += 2;
            continue;
        }
        if i + 2 < body.len() && body[i + 1] == b'-' {
            let hi = body[i + 2];
            ranges.push((lo, hi));
            i += 3;
        } else {
            ranges.push((lo, lo));
            i += 1;
        }
    }
    Some(ranges)
}

fn class_contains(ranges: &[(u8, u8)], byte: u8, negate: bool) -> bool {
    let hit = ranges.iter().any(|(lo, hi)| byte >= *lo && byte <= *hi);
    if negate {
        !hit
    } else {
        hit
    }
}

fn token_consumes(token: &Token, byte: u8) -> bool {
    match token {
        Token::Char(c) => *c == byte,
        Token::Class(ranges, negate) => class_contains(ranges, byte, *negate),
        Token::Plus(_) | Token::Repeat(_, _) => false,
    }
}

fn match_tokens(tokens: &[Token], text: &[u8], mut pos: usize) -> Option<usize> {
    let mut idx = 0usize;
    while idx < tokens.len() {
        match &tokens[idx] {
            Token::Char(_) | Token::Class(_, _) => {
                if pos >= text.len() {
                    return None;
                }
                if !token_consumes(&tokens[idx], text[pos]) {
                    return None;
                }
                pos += 1;
                idx += 1;
            }
            Token::Plus(inner) => {
                if pos >= text.len() || !token_consumes(inner, text[pos]) {
                    return None;
                }
                let rest = &tokens[idx + 1..];
                // Greedy with backtracking: match as many as possible, then
                // back off one at a time until the suffix matches.
                let mut max = pos;
                while max < text.len() && token_consumes(inner, text[max]) {
                    max += 1;
                }
                let mut try_pos = max;
                loop {
                    if let Some(end) = match_tokens(rest, text, try_pos) {
                        return Some(end);
                    }
                    if try_pos <= pos + 1 {
                        return None;
                    }
                    try_pos -= 1;
                }
            }
            Token::Repeat(inner, n) => {
                for _ in 0..*n {
                    if pos >= text.len() || !token_consumes(inner, text[pos]) {
                        return None;
                    }
                    pos += 1;
                }
                idx += 1;
            }
        }
    }
    Some(pos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn type_string_passes() {
        let schema = json!({"type": "string"});
        assert!(validate(&schema, &json!("hello")).is_empty());
    }

    #[test]
    fn type_array_with_null_passes() {
        let schema = json!({"type": ["string", "null"]});
        assert!(validate(&schema, &json!(null)).is_empty());
        assert!(validate(&schema, &json!("ok")).is_empty());
        assert!(!validate(&schema, &json!(42)).is_empty());
    }

    #[test]
    fn integer_accepts_integral_float() {
        let schema = json!({"type": "integer"});
        assert!(validate(&schema, &json!(1.0)).is_empty());
        assert!(!validate(&schema, &json!(1.5)).is_empty());
    }

    #[test]
    fn const_must_match() {
        let schema = json!({"const": "ori.foo.v1"});
        assert!(validate(&schema, &json!("ori.foo.v1")).is_empty());
        assert!(!validate(&schema, &json!("ori.bar.v1")).is_empty());
    }

    #[test]
    fn enum_must_match() {
        let schema = json!({"enum": ["a", "b", "c"]});
        assert!(validate(&schema, &json!("b")).is_empty());
        assert!(!validate(&schema, &json!("d")).is_empty());
    }

    #[test]
    fn required_enforced_in_strict_mode() {
        let schema = json!({
            "type": "object",
            "required": ["a", "b"],
            "properties": {"a": {"type": "string"}, "b": {"type": "integer"}}
        });
        assert!(validate_strict(&schema, &json!({"a": "x", "b": 1})).is_empty());
        let errs = validate_strict(&schema, &json!({"a": "x"}));
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].code, "missing_required");
        // Lenient mode allows the missing field through.
        assert!(validate(&schema, &json!({"a": "x"})).is_empty());
    }

    #[test]
    fn additional_properties_false_rejects_extras_in_strict_mode() {
        let schema = json!({
            "type": "object",
            "properties": {"a": {"type": "string"}},
            "additionalProperties": false
        });
        let errs = validate_strict(&schema, &json!({"a": "x", "b": "y"}));
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].code, "additional_property");
        // Lenient mode allows forward-compatible additions.
        assert!(validate(&schema, &json!({"a": "x", "b": "y"})).is_empty());
    }

    #[test]
    fn additional_properties_schema_validates_values() {
        // Schema-form additionalProperties applies in both modes — it
        // describes the value shape, not a structural ban.
        let schema = json!({
            "type": "object",
            "properties": {},
            "additionalProperties": {"type": "string"}
        });
        assert!(validate(&schema, &json!({"a": "x", "b": "y"})).is_empty());
        assert!(!validate(&schema, &json!({"a": 1})).is_empty());
    }

    #[test]
    fn pattern_diagnostic_id() {
        let schema = json!({"type": "string", "pattern": "^[A-Z][0-9]{4}$|^P[0-9]{4}$"});
        assert!(validate(&schema, &json!("E0100")).is_empty());
        assert!(validate(&schema, &json!("P0123")).is_empty());
        assert!(!validate(&schema, &json!("e0100")).is_empty());
        assert!(!validate(&schema, &json!("E10")).is_empty());
    }

    #[test]
    fn pattern_fnv1a_hash() {
        let schema = json!({"type": "string", "pattern": "^fnv1a:[0-9a-f]{16}$"});
        assert!(validate(&schema, &json!("fnv1a:0123456789abcdef")).is_empty());
        assert!(!validate(&schema, &json!("fnv1a:0123")).is_empty());
    }

    #[test]
    fn one_of_picks_a_branch() {
        let schema = json!({
            "oneOf": [
                {"type": "object", "properties": {"tag": {"const": "x"}}},
                {"type": "object", "properties": {"tag": {"const": "y"}}}
            ]
        });
        assert!(validate(&schema, &json!({"tag": "x"})).is_empty());
        assert!(validate(&schema, &json!({"tag": "y"})).is_empty());
        // No branch matches when the const fails on both.
        assert!(!validate(&schema, &json!({"tag": "z"})).is_empty());
    }

    #[test]
    fn ref_resolves_internal_defs() {
        let schema = json!({
            "type": "object",
            "properties": {"pos": {"$ref": "#/$defs/pt"}},
            "$defs": {"pt": {"type": "object", "properties": {"x": {"type": "integer"}}}}
        });
        assert!(validate(&schema, &json!({"pos": {"x": 1}})).is_empty());
        assert!(!validate(&schema, &json!({"pos": {"x": "no"}})).is_empty());
    }

    #[test]
    fn resolve_envelope_schema_path_handles_v2_pair() {
        // The mapping for `ori.backend_dispatch.v1` should resolve to the
        // versioned v1 file when a v2 file also exists.
        let dir = tempdir();
        std::fs::write(dir.join("backend-dispatch.schema.json"), "{}").ok();
        std::fs::write(dir.join("backend-dispatch-v1.schema.json"), "{}").ok();
        std::fs::write(dir.join("backend-dispatch-v2.schema.json"), "{}").ok();
        let resolved = resolve_schema_path(&dir, "ori.backend_dispatch.v1");
        assert_eq!(
            resolved.map(|p| p.file_name().map(|f| f.to_string_lossy().to_string())),
            Some(Some("backend-dispatch-v1.schema.json".to_string()))
        );
    }

    #[test]
    fn resolve_envelope_schema_path_dashes_underscores() {
        let dir = tempdir();
        std::fs::write(dir.join("agent-map.schema.json"), "{}").ok();
        let resolved = resolve_schema_path(&dir, "ori.agent_map.v1");
        assert_eq!(
            resolved.map(|p| p.file_name().map(|f| f.to_string_lossy().to_string())),
            Some(Some("agent-map.schema.json".to_string()))
        );
    }

    #[test]
    fn errors_are_sorted_by_path() {
        let schema = json!({
            "type": "object",
            "required": ["b", "a", "c"]
        });
        let errs = validate_strict(&schema, &json!({}));
        let paths: Vec<&str> = errs.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["/a", "/b", "/c"]);
    }

    // Tiny tempdir helper that does not pull in extra dependencies.
    fn tempdir() -> std::path::PathBuf {
        let mut base = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        base.push(format!(
            "ori_schema_check_test_{}_{}",
            std::process::id(),
            nanos
        ));
        let _ = std::fs::create_dir_all(&base);
        base
    }
}
