//! Stable node identifiers for the concrete syntax tree.
//!
//! IDs are derived from a small structural fingerprint so that they remain
//! stable across whitespace/comment edits and across changes to unrelated
//! parts of the same file. The format is `node:<module>.<kind>.<name>.<disc>`
//! where `disc` disambiguates duplicates by salted FNV-1a hash of
//! `(parent_id, sibling_index, signature)`.

use serde::{Deserialize, Serialize};

/// FNV-1a 64-bit offset basis. See <http://www.isthe.com/chongo/tech/comp/fnv/>.
const FNV1A_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime.
const FNV1A_PRIME: u64 = 0x100_0000_01b3;

/// Stable, human-readable identifier for an AST/CST node.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub String);

impl NodeId {
    /// Construct a new id from any string-like value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Return the underlying string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// FNV-1a 64-bit hash, used everywhere structural hashes are needed in the
/// bootstrap compiler. Deterministic, dependency-free, fast enough for the
/// edit-check-repair loop.
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = FNV1A_OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV1A_PRIME);
    }
    hash
}

/// Hash a list of string fragments joined by `|`. Useful for building
/// structural fingerprints that should be insensitive to formatting.
pub fn fnv1a_64_str(parts: &[&str]) -> u64 {
    let mut buffer = String::new();
    for (idx, part) in parts.iter().enumerate() {
        if idx > 0 {
            buffer.push('|');
        }
        buffer.push_str(part);
    }
    fnv1a_64(buffer.as_bytes())
}

/// Construct a NodeId from a parent fingerprint, kind, name, sibling index,
/// and signature. The encoded form is human-readable so agents can paste IDs
/// across tools.
pub fn make_node_id(
    module: &str,
    parent: Option<&NodeId>,
    kind: &str,
    name: &str,
    sibling_index: usize,
    signature: &str,
) -> NodeId {
    let parent_str = parent.map(NodeId::as_str).unwrap_or("");
    let disc = fnv1a_64_str(&[
        parent_str,
        kind,
        name,
        &sibling_index.to_string(),
        signature,
    ]);
    NodeId::new(format!("node:{module}.{kind}.{name}.{disc:016x}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_for_same_inputs() {
        let id_a = make_node_id("demo", None, "fn", "hello", 0, "() -> Unit");
        let id_b = make_node_id("demo", None, "fn", "hello", 0, "() -> Unit");
        assert_eq!(id_a, id_b);
    }

    #[test]
    fn ids_differ_for_sibling_index() {
        let id_a = make_node_id("demo", None, "fn", "f", 0, "() -> Unit");
        let id_b = make_node_id("demo", None, "fn", "f", 1, "() -> Unit");
        assert_ne!(id_a, id_b);
    }

    #[test]
    fn ids_differ_for_signature() {
        let id_a = make_node_id("demo", None, "fn", "f", 0, "() -> Unit");
        let id_b = make_node_id("demo", None, "fn", "f", 0, "() -> Int");
        assert_ne!(id_a, id_b);
    }

    #[test]
    fn fnv1a_is_deterministic() {
        assert_eq!(fnv1a_64(b"orison"), fnv1a_64(b"orison"));
        assert_ne!(fnv1a_64(b"orison"), fnv1a_64(b"orisons"));
    }
}
