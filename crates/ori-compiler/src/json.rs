use serde::Serialize;

/// Serialize public compiler and agent contracts.
///
/// Public CLI contracts must never panic. If serialization fails because a future
/// custom serializer is fallible, emit a small valid JSON error object instead
/// of crashing the compiler process.
pub fn to_json<T: Serialize>(value: &T) -> String {
    match serde_json::to_string(value) {
        Ok(serialized) => serialized,
        Err(err) => serialization_error_json(&err.to_string()),
    }
}

/// Pretty-printing variant of [`to_json`]. Serialisation failures are
/// rendered as a `ori.serialization_error.v1` JSON object so callers never
/// see a panic.
pub fn to_pretty_json<T: Serialize>(value: &T) -> String {
    match serde_json::to_string_pretty(value) {
        Ok(serialized) => serialized,
        Err(err) => serialization_error_json(&err.to_string()),
    }
}

fn serialization_error_json(message: &str) -> String {
    let escaped = message
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    format!(
        "{{\"schema\":\"ori.serialization_error.v1\",\"level\":\"error\",\"message\":\"{escaped}\"}}"
    )
}
