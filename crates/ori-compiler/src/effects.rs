//! Static list of effects recognised by the bootstrap.
//!
//! Capability tokens (declared via `capability X` in source) are detected by
//! their leading uppercase character — the same rule the spec uses to
//! distinguish a named capability from a built-in effect.

/// Built-in effect identifiers. Capability names declared in source are
/// matched separately by [`is_known_effect_or_capability`].
pub const KNOWN_EFFECTS: &[&str] = &[
    "fs.read",
    "fs.write",
    "net.inbound",
    "net.outbound",
    "db.read",
    "db.write",
    "env.read",
    "process.spawn",
    "crypto",
    "time",
    "random",
    "ui",
    "gpu",
    "unsafe",
    "http",
    "db",
    "fs",
    "net",
    "auth",
    "mail.send",
];

/// Returns `true` when `effect` is either a built-in effect name or a
/// capability identifier (first character ASCII upper-case).
pub fn is_known_effect_or_capability(effect: &str) -> bool {
    KNOWN_EFFECTS.contains(&effect)
        || effect
            .chars()
            .next()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false)
}
